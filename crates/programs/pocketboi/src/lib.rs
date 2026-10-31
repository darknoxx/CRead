//! Pocketboi: a Game Boy in a window.
//!
//! The emulation is Mizu's; this crate is the cartridge shelf and the picture.
//! Mizu gives a 160x144 RGB frame per tick, which we reduce to the four shades
//! of the machine and blow up by two into the emulated raster.

use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cread_core::program::{KeyPress, Outcome, PointerEvent, Program};
use cread_core::{Cell, Raster, Screen};
use gilrs::{Axis, Button, EventType, Gilrs};
use mizu_core::{GameBoy, JoypadButton};
use rodio::{OutputStream, OutputStreamBuilder, Sink, Source};

/// What Mizu's APU actually emits.
///
/// Its source says 44.1 kHz, but a frame yields 1476 samples and a frame is
/// 59.7 Hz, so the rate is 88.2 kHz. Playing those samples at 44.1 kHz made
/// everything an octave low and half speed, with the queue growing behind.
/// Measured, not guessed.
const SAMPLE_RATE: u32 = 88_200;

/// The machine's screen, in its own pixels.
pub const GB_W: u32 = 160;
pub const GB_H: u32 = 144;
/// Integer zoom: 160x144 fits twice over in 640x384.
const ZOOM: u32 = 2;
/// The four shades, lightest first, as palette indices.
///
/// Grey is the default: the green of a DMG is charming for five minutes and
/// tiring for an hour.
const PALETTES: [(&str, [u8; 4]); 3] = [
    ("grey", [15, 7, 8, 0]),
    ("green", [15, 10, 2, 0]),
    ("amber", [15, 14, 6, 0]),
];

/// One row of the cartridge shelf.
struct Entry {
    name: String,
    path: PathBuf,
    folder: bool,
}

/// A running cartridge.
/// What the emulation thread publishes and the screen reads.
struct Shared {
    /// The latest 160x144 RGB frame.
    frame: Mutex<Vec<u8>>,
    /// Which pad buttons are held, in [`BUTTONS`] order.
    buttons: Mutex<[bool; 8]>,
    muted: AtomicBool,
    /// Set when the emulator gave up: the picture freezes, the shell lives.
    fault: Mutex<Option<String>>,
}

/// A cartridge running on its own thread at the machine's own rate.
///
/// It used to be stepped from the drawing code, which tied the sound to the
/// compositor: a slow frame was a hole in the audio. The thread keeps its own
/// clock, fills the audio ring and publishes the picture; the screen only ever
/// copies the latest frame.
struct Machine {
    title: String,
    shared: Arc<Shared>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Machine {
    /// Load the cartridge (in the thread, because the emulator is not `Send`)
    /// and let it run at the machine's own pace.
    fn start(rom: &Path, muted: bool) -> Result<Self, String> {
        let shared = Arc::new(Shared {
            frame: Mutex::new(vec![0; (GB_W * GB_H * 3) as usize]),
            buttons: Mutex::new([false; 8]),
            muted: AtomicBool::new(muted),
            fault: Mutex::new(None),
        });
        let stop = Arc::new(AtomicBool::new(false));
        let (ready, loaded) = mpsc::channel();
        let thread_shared = shared.clone();
        let thread_stop = stop.clone();
        let rom = rom.to_path_buf();
        let thread = std::thread::spawn(move || {
            let mut gb = match GameBoy::builder(&rom).build() {
                Ok(gb) => {
                    let _ = ready.send(Ok(gb.game_title().trim().to_string()));
                    gb
                }
                Err(e) => {
                    let _ = ready.send(Err(format!("{e}")));
                    return;
                }
            };
            let mut sound = Sound::open();
            if let Some(sound) = sound.as_mut() {
                sound.set_muted(thread_shared.muted.load(Ordering::Relaxed));
            }
            // One emulated frame in wall time: 59.7275 Hz.
            let period = Duration::from_micros(16_734);
            let mut next = std::time::Instant::now();
            while !thread_stop.load(Ordering::Relaxed) {
                if let Ok(buttons) = thread_shared.buttons.lock() {
                    for index in 0..8 {
                        if buttons[index] {
                            gb.press_joypad(button_at(index));
                        } else {
                            gb.release_joypad(button_at(index));
                        }
                    }
                }
                let stepped = catch_unwind(AssertUnwindSafe(|| gb.clock_for_frame()));
                match stepped {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => {
                        *thread_shared
                            .fault
                            .lock()
                            .unwrap_or_else(|p| p.into_inner()) = Some(format!("{e:?}"));
                        break;
                    }
                    Err(_) => {
                        *thread_shared
                            .fault
                            .lock()
                            .unwrap_or_else(|p| p.into_inner()) =
                            Some("the emulator crashed".into());
                        break;
                    }
                }
                if let Some(sound) = sound.as_mut() {
                    let samples = gb.audio_buffers().all().to_vec();
                    sound.push(&samples);
                    let muted = thread_shared.muted.load(Ordering::Relaxed);
                    if muted != sound.is_muted() {
                        sound.set_muted(muted);
                    }
                }
                if let Ok(mut frame) = thread_shared.frame.lock() {
                    frame.copy_from_slice(gb.screen_buffer());
                }
                next += period;
                let now = std::time::Instant::now();
                if next > now {
                    std::thread::sleep(next - now);
                } else {
                    next = now;
                }
            }
        });
        let title = match loaded.recv() {
            Ok(Ok(title)) => title,
            Ok(Err(e)) => {
                stop.store(true, Ordering::Relaxed);
                return Err(e);
            }
            Err(_) => {
                stop.store(true, Ordering::Relaxed);
                return Err("the emulation thread did not start".into());
            }
        };
        Ok(Self {
            title,
            shared,
            stop,
            thread: Some(thread),
        })
    }

    fn fault(&self) -> Option<String> {
        self.shared
            .fault
            .lock()
            .ok()
            .and_then(|fault| fault.clone())
    }

    /// Which buttons are down, for the key and pad handlers.
    fn set_button(&self, button: JoypadButton, down: bool) {
        if let (Ok(mut buttons), Some(index)) = (self.buttons(), button_index(&button)) {
            buttons[index] = down;
        }
    }

    fn buttons(&self) -> Result<std::sync::MutexGuard<'_, [bool; 8]>, ()> {
        self.shared.buttons.lock().map_err(|_| ())
    }
}

impl Drop for Machine {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The order [`Shared::buttons`] uses. `JoypadButton` is not `Copy`, so the
/// table is a function rather than a constant.
/// Feed one stick reading into the button table.
///
/// gilrs reports the stick's vertical axis positive *upwards*, which is the
/// opposite of the screen's y. Getting this wrong makes the pad fight the
/// player: up becomes down.
fn apply_axis(shared: &Shared, stick: &mut [i8; 4], axis: Axis, value: f32) {
    const DEAD: f32 = 0.5;
    let (slot, positive, negative) = match axis {
        Axis::LeftStickX => (0, JoypadButton::Right, JoypadButton::Left),
        Axis::LeftStickY => (1, JoypadButton::Up, JoypadButton::Down),
        _ => return,
    };
    let lean = if value > DEAD {
        1
    } else if value < -DEAD {
        -1
    } else {
        0
    };
    if lean == stick[slot] {
        return;
    }
    stick[slot] = lean;
    let Ok(mut buttons) = shared.buttons.lock() else {
        return;
    };
    let mut set = |button: JoypadButton, down: bool| {
        if let Some(index) = button_index(&button) {
            buttons[index] = down;
        }
    };
    set(positive, lean > 0);
    set(negative, lean < 0);
}

fn button_at(index: usize) -> JoypadButton {
    match index {
        0 => JoypadButton::Up,
        1 => JoypadButton::Down,
        2 => JoypadButton::Left,
        3 => JoypadButton::Right,
        4 => JoypadButton::A,
        5 => JoypadButton::B,
        6 => JoypadButton::Start,
        _ => JoypadButton::Select,
    }
}

fn button_index(button: &JoypadButton) -> Option<usize> {
    (0..8)
        .find(|index| std::mem::discriminant(&button_at(*index)) == std::mem::discriminant(button))
}

/// A game pad, polled once per frame. No thread: the emulation loop is
/// already running at 60 Hz, so asking gilrs "anything new?" costs nothing.
struct Pad {
    gilrs: Gilrs,
    /// Which way the stick leans on each axis: -1, 0 or 1. A flag was not
    /// enough — flicking from up to down kept "up" pressed.
    stick: [i8; 4],
}

impl Pad {
    fn open() -> Option<Self> {
        let gilrs = Gilrs::new().ok()?;
        Some(Self {
            gilrs,
            stick: [0; 4],
        })
    }

    /// Any pad plugged in right now.
    fn connected(&self) -> bool {
        self.gilrs.gamepads().next().is_some()
    }

    fn poll(&mut self, shared: &Shared) {
        while let Some(event) = self.gilrs.next_event() {
            match event.event {
                EventType::ButtonPressed(button, _) => {
                    if let (Some(button), Ok(mut buttons)) =
                        (pad_button(button), shared.buttons.lock())
                        && let Some(index) = button_index(&button)
                    {
                        buttons[index] = true;
                    }
                }
                EventType::ButtonReleased(button, _) => {
                    if let (Some(button), Ok(mut buttons)) =
                        (pad_button(button), shared.buttons.lock())
                        && let Some(index) = button_index(&button)
                    {
                        buttons[index] = false;
                    }
                }
                EventType::AxisChanged(axis, value, _) => self.axis(shared, axis, value),
                _ => {}
            }
        }
    }

    fn axis(&mut self, shared: &Shared, axis: Axis, value: f32) {
        apply_axis(shared, &mut self.stick, axis, value);
    }
}

/// The pad's buttons. A SNES-style pad reports its east and south buttons, and
/// on this one the east button is the one marked B.
fn pad_button(button: Button) -> Option<JoypadButton> {
    Some(match button {
        // On the pad in hand the right-hand button is B, the lower one is A.
        Button::East => JoypadButton::B,
        Button::South => JoypadButton::A,
        Button::Start => JoypadButton::Start,
        Button::Select => JoypadButton::Select,
        Button::DPadUp => JoypadButton::Up,
        Button::DPadDown => JoypadButton::Down,
        Button::DPadLeft => JoypadButton::Left,
        Button::DPadRight => JoypadButton::Right,
        _ => return None,
    })
}

/// The machine's speaker, in the shell's own audio device.
/// A source that never ends: it plays whatever the emulator has produced and
/// silence when it has nothing.
///
/// Feeding rodio one short source per frame made the mixer switch sources
/// sixteen times a second, and every switch was a place for a sample to go
/// missing. One stream, fed through a ring, has no seams to miss.
struct Speaker {
    /// What the emulator has produced and the speaker has not taken yet.
    shared: Arc<Mutex<VecDeque<f32>>>,
    /// The block the speaker is reading from, so it does not touch the lock
    /// once per sample — that was 48 000 locks a second, each one a chance to
    /// make the callback late.
    local: VecDeque<f32>,
    rate: u32,
}

impl Iterator for Speaker {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.local.is_empty()
            && let Ok(mut queue) = self.shared.lock()
        {
            std::mem::swap(&mut self.local, &mut *queue);
        }
        Some(self.local.pop_front().unwrap_or(0.0))
    }
}

impl Source for Speaker {
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> u16 {
        1
    }

    fn sample_rate(&self) -> u32 {
        self.rate
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

/// Linear resampling from the APU's rate to the device's, with the phase and
/// the last sample carried across calls so consecutive frames join seamlessly.
struct Resampler {
    /// Input samples per output sample.
    step: f64,
    pos: f64,
    tail: f32,
    /// The first chunk has no previous sample to join to.
    primed: bool,
}

impl Resampler {
    fn new(input_rate: u32, output_rate: u32) -> Self {
        Self {
            step: input_rate as f64 / output_rate.max(1) as f64,
            pos: 0.0,
            tail: 0.0,
            primed: false,
        }
    }

    fn feed(&mut self, input: &[f32], out: &mut Vec<f32>) {
        if input.is_empty() {
            return;
        }
        if !self.primed {
            // Nothing to join to yet: start at this sample rather than ramping
            // up from zero, and skip the duplicate it would produce.
            self.primed = true;
            self.tail = input[0];
            self.pos = 1.0;
        }
        let mut src = Vec::with_capacity(input.len() + 1);
        src.push(self.tail);
        src.extend_from_slice(input);
        let last = src.len() - 1;
        let mut pos = self.pos;
        while pos < last as f64 {
            let i = pos.floor() as usize;
            let frac = (pos - i as f64) as f32;
            out.push(src[i] + (src[i + 1] - src[i]) * frac);
            pos += self.step;
        }
        self.pos = pos - last as f64;
        self.tail = src[last];
    }
}

struct Sound {
    _stream: OutputStream,
    sink: Sink,
    samples: Arc<Mutex<VecDeque<f32>>>,
    resampler: Resampler,
    /// The device's own rate, for the ring's cap.
    rate: u32,
    muted: bool,
}

/// How much audio waits in the ring: enough to ride out a late frame, little
/// enough that a button press is heard when it happens.
const TARGET_SECONDS: f32 = 0.040;

/// Open the output the way a desktop expects it.
///
/// Not `open_default_stream`: that one falls back to the first *hardware*
/// device it can find when the default refuses to open, which is how a game
/// ends up on the speakers while the system is set to headphones. The sound
/// server's own PCM (`pipewire`, `pulse`) is the one that follows the system
/// default and moves a stream when you switch output, so it is asked for by
/// name; plain `default` is the next choice, hardware never.
pub fn open_output() -> Option<OutputStream> {
    use rodio::cpal::traits::{DeviceTrait, HostTrait};
    use rodio::{Device, cpal};
    let host = cpal::default_host();
    let mut fallback: Option<Device> = None;
    if let Ok(devices) = host.output_devices() {
        for device in devices {
            let Ok(name) = device.name() else { continue };
            match name.to_ascii_lowercase().as_str() {
                "pipewire" | "pulse" => {
                    return OutputStreamBuilder::from_device(device)
                        .ok()?
                        .open_stream_or_fallback()
                        .ok();
                }
                "default" if fallback.is_none() => fallback = Some(device),
                _ => {}
            }
        }
    }
    let device = fallback.or_else(|| host.default_output_device())?;
    OutputStreamBuilder::from_device(device)
        .ok()?
        .open_stream_or_fallback()
        .ok()
}

impl Sound {
    fn open() -> Option<Self> {
        let stream = open_output()?;
        let rate = stream.config().sample_rate();
        let samples = Arc::new(Mutex::new(VecDeque::with_capacity(
            (TARGET_SECONDS * 2.0 * rate as f32) as usize,
        )));
        let sink = Sink::connect_new(stream.mixer());
        sink.set_volume(0.6);
        // A buffer that starts empty has no margin: the very first late frame
        // starves it and every one after that ticked. It starts full of
        // silence instead, so the 40 ms are actually there to absorb jitter.
        if let Ok(mut queue) = samples.lock() {
            queue.extend(std::iter::repeat_n(
                0.0,
                (TARGET_SECONDS * rate as f32) as usize,
            ));
        }
        sink.append(Speaker {
            shared: samples.clone(),
            local: VecDeque::new(),
            rate,
        });
        Some(Self {
            _stream: stream,
            sink,
            samples,
            resampler: Resampler::new(SAMPLE_RATE, rate),
            rate,
            muted: false,
        })
    }

    fn is_muted(&self) -> bool {
        self.muted
    }

    fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
        self.sink.set_volume(if muted { 0.0 } else { 0.6 });
        if muted {
            self.samples.lock().map(|mut q| q.clear()).ok();
        }
    }

    /// One frame of Game Boy audio, resampled to the device's rate and left in
    /// the ring for the speaker.
    ///
    /// The resampling is ours and carries its phase across frames: handing
    /// rodio a fresh 88.2 kHz buffer every 16 ms made it resample each chunk
    /// from scratch, and the seams were audible as a fine crackle.
    fn push(&mut self, input: &[f32]) {
        if input.is_empty() || self.muted {
            return;
        }
        let mut out = Vec::new();
        self.resampler.feed(input, &mut out);
        if out.is_empty() {
            return;
        }
        let cap = (TARGET_SECONDS * 2.0 * self.rate as f32) as usize;
        if let Ok(mut queue) = self.samples.lock() {
            // A ring that only ever grows is latency: when we get ahead, drop
            // what will not be heard anyway.
            if queue.len() + out.len() > cap {
                let overflow = queue.len() + out.len() - cap;
                for _ in 0..overflow.min(queue.len()) {
                    queue.pop_front();
                }
            }
            queue.extend(out);
        }
    }
}

pub struct Pocketboi {
    dir: PathBuf,
    entries: Vec<Entry>,
    selected: usize,
    game: Option<Machine>,
    /// Silence, as a flag the emulation thread reads each frame.
    muted: bool,
    pad: Option<Pad>,
    pad_failed: bool,
    /// The shell's page and ink, so the module does not paint a black box of
    /// its own over a paper screen.
    paper: u8,
    ink: u8,
    /// Which of [`PALETTES`] the picture is painted in.
    palette: usize,
}

impl Pocketboi {
    /// Opens on the cartridge shelf inside the vault.
    pub fn new(vault: &Path) -> Self {
        let dir = vault.join("Roms");
        // Make the shelf visible instead of leaving the user to guess the path.
        let _ = std::fs::create_dir_all(&dir);
        let mut boi = Self {
            entries: Vec::new(),
            selected: 0,
            game: None,
            muted: false,
            pad: None,
            pad_failed: false,
            paper: 0,
            ink: 15,
            palette: 0,
            dir,
        };
        boi.rescan();
        boi
    }

    /// Load a cartridge directly, skipping the shelf.
    pub fn play(&mut self, path: &Path) -> Result<(), String> {
        // Stop whatever was running first: one machine at a time.
        self.game = None;
        let machine = Machine::start(path, self.muted)?;
        self.game = Some(machine);
        Ok(())
    }

    fn rescan(&mut self) {
        self.entries = shelf(&self.dir);
        self.selected = self.selected.min(self.entries.len().saturating_sub(1));
    }

    fn enter(&mut self) {
        let Some(entry) = self.entries.get(self.selected) else {
            return;
        };
        let (path, folder) = (entry.path.clone(), entry.folder);
        if folder {
            self.dir = path;
            self.selected = 0;
            self.rescan();
            return;
        }
        let name = self.entries[self.selected].name.clone();
        if let Err(e) = self.play(&path) {
            // A cartridge that will not load never reaches the emulator.
            self.entries[self.selected].name = format!("{name} ({e})");
        }
    }

    fn up(&mut self) {
        let parent = self.dir.parent().map(Path::to_path_buf);
        if let Some(parent) = parent {
            self.dir = parent;
            self.selected = 0;
            self.rescan();
        }
    }

    /// Step one frame and paint it.
    fn run(&mut self, screen: &mut Screen, raster: &mut Raster) {
        if self.pad.is_none() && !self.pad_failed {
            self.pad = Pad::open();
            self.pad_failed = self.pad.is_none();
        }
        if let Some(machine) = self.game.as_mut()
            && let Some(pad) = self.pad.as_mut()
        {
            pad.poll(&machine.shared);
        }
        if let Some(machine) = self.game.as_ref() {
            paint(&machine.shared, raster, PALETTES[self.palette].1, self.ink);
        }
        let pad = self.pad.as_ref().is_some_and(Pad::connected);
        draw_legend(screen, self.ink, pad);
    }

    fn status(&self) -> String {
        let base = match &self.game {
            Some(game) => match game.fault() {
                Some(fault) => format!("{} — stopped: {fault}", game.title),
                None => game.title.clone(),
            },
            None => "insert a cartridge".into(),
        };
        if self.muted {
            format!("{base} · muted")
        } else {
            base
        }
    }
}

/// Folders and cartridges in one folder.
fn shelf(dir: &Path) -> Vec<Entry> {
    let mut folders = Vec::new();
    let mut carts = Vec::new();
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    for entry in read.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            folders.push(Entry {
                name,
                path,
                folder: true,
            });
        } else if is_cartridge(&path) {
            carts.push(Entry {
                name,
                path,
                folder: false,
            });
        }
    }
    folders.sort_by_key(|entry| entry.name.to_lowercase());
    carts.sort_by_key(|entry| entry.name.to_lowercase());
    folders.extend(carts);
    folders
}

fn is_cartridge(path: &Path) -> bool {
    path.extension()
        .map(|ext| {
            let ext = ext.to_string_lossy().to_ascii_lowercase();
            ext == "gb" || ext == "gbc"
        })
        .unwrap_or(false)
}

/// The frame, four shades deep, blown up by two and centred.
fn paint(shared: &Shared, raster: &mut Raster, shades: [u8; 4], ink: u8) {
    // The shell already filled the raster with the page colour; leave it.
    let Ok(frame) = shared.frame.lock() else {
        return;
    };
    let frame = frame.as_slice();
    let width = raster.width();
    let x0 = (width.saturating_sub(GB_W * ZOOM)) / 2;
    // Slightly above the middle: the key legend needs a row of its own below.
    let y0 = (raster.height().saturating_sub(GB_H * ZOOM)) / 2 - 12;
    // A case around the glass: four pixels of air, then a two-pixel bezel.
    let left = x0 as i32 - 6;
    let top = y0 as i32 - 6;
    let right = x0 as i32 + (GB_W * ZOOM) as i32 + 5;
    let bottom = y0 as i32 + (GB_H * ZOOM) as i32 + 5;
    raster.rect((left, top), (right, bottom), 2, ink, false);

    // The pixel buffer is RGB, 3 bytes per pixel.
    let row_len = GB_W as usize * 3;
    if frame.len() < row_len * GB_H as usize {
        return;
    }
    for y in 0..GB_H {
        let row = &frame[y as usize * row_len..][..row_len];
        for x in 0..GB_W {
            let p = &row[x as usize * 3..];
            let luma = (p[0] as u32 * 30 + p[1] as u32 * 59 + p[2] as u32 * 11) / 100;
            let shade = match luma {
                0..=63 => 3,
                64..=127 => 2,
                128..=191 => 1,
                _ => 0,
            };
            let index = shades[shade];
            for dy in 0..ZOOM {
                for dx in 0..ZOOM {
                    raster.set(
                        x0 as i32 + x as i32 * ZOOM as i32 + dx as i32,
                        y0 as i32 + y as i32 * ZOOM as i32 + dy as i32,
                        index,
                    );
                }
            }
        }
    }
}

/// Keys the pad answers to.
fn button(key: KeyPress) -> Option<JoypadButton> {
    Some(match key {
        KeyPress::Up => JoypadButton::Up,
        KeyPress::Down => JoypadButton::Down,
        KeyPress::Left => JoypadButton::Left,
        KeyPress::Right => JoypadButton::Right,
        KeyPress::Enter | KeyPress::ControlRight => JoypadButton::Start,
        KeyPress::ControlLeft => JoypadButton::Select,
        KeyPress::Char(' ') => JoypadButton::A,
        KeyPress::Char(c) if c.eq_ignore_ascii_case(&'c') => JoypadButton::B,
        _ => return None,
    })
}

impl Program for Pocketboi {
    fn title(&self) -> &str {
        "POCKETBOI"
    }

    fn status(&self) -> String {
        self.status()
    }

    fn key(&mut self, key: KeyPress) -> Outcome {
        if self.game.is_some() {
            if let KeyPress::Char(c) = key
                && c.eq_ignore_ascii_case(&'p')
            {
                self.palette = (self.palette + 1) % PALETTES.len();
                return Outcome::Continue;
            }
            if let Some(button) = button(key)
                && let Some(game) = self.game.as_ref()
            {
                game.set_button(button, true);
            }
            return Outcome::Continue;
        }
        match key {
            KeyPress::Up => self.selected = self.selected.saturating_sub(1),
            KeyPress::Down => {
                self.selected = (self.selected + 1).min(self.entries.len().saturating_sub(1));
            }
            KeyPress::Enter => self.enter(),
            KeyPress::Backspace | KeyPress::Left => self.up(),
            _ => {}
        }
        Outcome::Continue
    }

    fn key_released(&mut self, key: KeyPress) {
        if let Some(game) = self.game.as_ref()
            && let Some(button) = button(key)
        {
            game.set_button(button, false);
        }
    }

    fn pointer(&mut self, _event: PointerEvent) -> Outcome {
        Outcome::Continue
    }

    fn palette(&mut self, background: u8, ink: u8) {
        self.paper = background;
        self.ink = ink;
    }

    fn command(&mut self, command: &cread_core::Command) -> Result<String, String> {
        match command {
            cread_core::Command::Mute => {
                self.muted = !self.muted;
                if let Some(game) = self.game.as_ref() {
                    game.shared.muted.store(self.muted, Ordering::Relaxed);
                }
                Ok(if self.muted { "muted" } else { "sound on" }.into())
            }
            _ => Err("POCKETBOI does not know that command".into()),
        }
    }

    fn exit(&mut self) -> bool {
        // Back to the shelf first; the shell only gets the screen once the
        // cartridge is out.
        if self.game.take().is_some() {
            self.rescan();
            return true;
        }
        false
    }

    fn wants_frames(&self) -> bool {
        self.game
            .as_ref()
            .is_some_and(|game| game.fault().is_none())
    }

    fn draw(&mut self, screen: &mut Screen, raster: &mut Raster) {
        // The cells are transparent: the picture is pixels, not text.
        screen.fill(Cell::TRANSPARENT);
        if self.game.is_some() {
            self.run(screen, raster);
            return;
        }
        draw_shelf(screen, self);
    }
}

/// The key legend, under the machine's screen: there is room for it.
///
/// The keys are chosen to sit in the same place on every layout: Space is a
/// thumb, C is next to it, and the two Controls are the small buttons.
fn draw_legend(screen: &mut Screen, ink: u8, pad: bool) {
    let line = "arrows move    C B    Space A    Enter/R-Ctrl start    L-Ctrl select";
    let width = line.chars().count() as u32;
    let x = screen.cols().saturating_sub(width) / 2;
    screen.put_str(x, 21, line, ink, Cell::BG_TRANSPARENT);

    if pad {
        let line = "Gamepad recognized! Lean back and relax.";
        let width = line.chars().count() as u32;
        let x = screen.cols().saturating_sub(width) / 2;
        screen.put_str(x, 23, line, ink, Cell::BG_TRANSPARENT);
    }
}

fn draw_shelf(screen: &mut Screen, boi: &Pocketboi) {
    let cols = screen.cols();
    let (paper, ink) = (boi.paper, boi.ink);
    screen.put_str(2, 1, "P O C K E T B O I", ink, Cell::BG_TRANSPARENT);
    let dir = boi.dir.display().to_string();
    screen.put_str_ellipsis(
        2,
        3,
        &dir,
        cols.saturating_sub(4),
        ink,
        Cell::BG_TRANSPARENT,
    );

    let first = 5;
    let last = screen.rows().saturating_sub(2);
    for (i, entry) in boi.entries.iter().enumerate() {
        let y = first + i as u32;
        if y >= last {
            break;
        }
        let selected = i == boi.selected;
        let (fg, bg) = if selected {
            (paper, ink)
        } else {
            (ink, Cell::BG_TRANSPARENT)
        };
        if selected {
            for x in 1..cols.saturating_sub(1) {
                screen.put(x, y, Cell::blank(paper, ink));
            }
        }
        let mark = if entry.folder { '\u{25ba}' } else { ' ' };
        screen.put_str(2, y, &format!("{mark} {}", entry.name), fg, bg);
    }
    if boi.entries.is_empty() {
        screen.put_str(
            4,
            first,
            "no cartridges yet - drop .gb or .gbc files in this folder",
            ink,
            Cell::BG_TRANSPARENT,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "cread-pocketboi-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_shelf_lists_folders_then_cartridges_and_ignores_the_rest() {
        let vault = temp("shelf");
        let roms = vault.join("Roms");
        std::fs::create_dir_all(roms.join("Homebrew")).unwrap();
        for name in ["Tetris.gb", "Pokemon.GBC", "notes.txt", "cover.png"] {
            std::fs::write(roms.join(name), b"x").unwrap();
        }
        let listed = shelf(&roms);
        let names: Vec<&str> = listed.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["Homebrew", "Pokemon.GBC", "Tetris.gb"]);
        assert!(listed[0].folder);
        let _ = std::fs::remove_dir_all(&vault);
    }

    #[test]
    fn opening_the_module_creates_the_shelf_in_the_vault() {
        let vault = temp("create");
        let boi = Pocketboi::new(&vault);
        assert!(vault.join("Roms").is_dir());
        assert!(boi.entries.is_empty());
        assert_eq!(boi.status(), "insert a cartridge");
        let _ = std::fs::remove_dir_all(&vault);
    }

    #[test]
    fn the_arrow_keys_walk_the_shelf_and_backspace_goes_up() {
        let vault = temp("walk");
        let roms = vault.join("Roms");
        std::fs::create_dir_all(roms.join("Homebrew")).unwrap();
        std::fs::write(roms.join("a.gb"), b"x").unwrap();
        let mut boi = Pocketboi::new(&vault);
        assert_eq!(boi.selected, 0);
        boi.key(KeyPress::Down);
        assert_eq!(boi.selected, 1);
        boi.key(KeyPress::Down);
        assert_eq!(boi.selected, 1, "stops at the end");
        boi.key(KeyPress::Up);
        assert_eq!(boi.selected, 0);
        // Into the folder, then back out.
        boi.key(KeyPress::Enter);
        assert!(boi.dir.ends_with("Homebrew"));
        boi.key(KeyPress::Backspace);
        assert!(boi.dir.ends_with("Roms"));
        let _ = std::fs::remove_dir_all(&vault);
    }

    #[test]
    fn a_pad_key_maps_to_the_right_button() {
        fn is(key: KeyPress, want: JoypadButton) -> bool {
            matches!((button(key), want), (Some(got), want) if std::mem::discriminant(&got) == std::mem::discriminant(&want))
        }
        assert!(is(KeyPress::Up, JoypadButton::Up));
        assert!(is(KeyPress::Enter, JoypadButton::Start));
        assert!(is(KeyPress::ControlRight, JoypadButton::Start));
        assert!(is(KeyPress::ControlLeft, JoypadButton::Select));
        assert!(is(KeyPress::Char('c'), JoypadButton::B));
        assert!(is(KeyPress::Char(' '), JoypadButton::A));
        assert!(button(KeyPress::Char('q')).is_none());
    }

    #[test]
    fn the_resampler_lands_on_the_device_rate_and_joins_frames() {
        // 88.2 kHz in, 48 kHz out: two thirds of the samples survive.
        let mut resampler = Resampler::new(88_200, 48_000);
        let mut total = 0;
        let mut out = Vec::new();
        for _ in 0..60 {
            out.clear();
            resampler.feed(&vec![0.5; 1476], &mut out);
            total += out.len();
        }
        let expected = 60.0 * 1476.0 * 48_000.0 / 88_200.0;
        assert!(
            (total as f64 - expected).abs() < 2.0 * 60.0,
            "got {total}, expected about {expected:.0}"
        );

        // A constant signal stays constant across the seams: no step at the
        // frame boundary, which is what a fresh phase every 16 ms produced.
        let mut resampler = Resampler::new(88_200, 48_000);
        let mut out = Vec::new();
        for _ in 0..10 {
            resampler.feed(&[0.25; 1476], &mut out);
        }
        assert!(out.iter().all(|s| (*s - 0.25).abs() < 1e-6));
    }

    #[test]
    fn a_rate_that_already_matches_passes_samples_through() {
        let mut resampler = Resampler::new(48_000, 48_000);
        let mut out = Vec::new();
        resampler.feed(&[0.1, 0.2, 0.3, 0.4], &mut out);
        assert_eq!(out.len(), 3, "one sample is held back for the next seam");
        resampler.feed(&[0.5], &mut out);
        assert_eq!(out.len(), 4);
        assert!((out[3] - 0.4).abs() < 1e-6);
    }

    #[test]
    fn the_stick_points_where_the_thumb_goes() {
        let shared = Shared {
            frame: Mutex::new(Vec::new()),
            buttons: Mutex::new([false; 8]),
            muted: AtomicBool::new(false),
            fault: Mutex::new(None),
        };
        let mut stick = [0i8; 4];
        let held = |shared: &Shared, button: JoypadButton| {
            let index = button_index(&button).unwrap();
            shared.buttons.lock().unwrap()[index]
        };

        // Positive Y is up, as gilrs reports it.
        apply_axis(&shared, &mut stick, Axis::LeftStickY, 1.0);
        assert!(held(&shared, JoypadButton::Up), "stick up is up");
        assert!(!held(&shared, JoypadButton::Down));

        apply_axis(&shared, &mut stick, Axis::LeftStickY, -1.0);
        assert!(held(&shared, JoypadButton::Down), "stick down is down");
        assert!(!held(&shared, JoypadButton::Up));

        // Back to the middle: nothing held.
        apply_axis(&shared, &mut stick, Axis::LeftStickY, 0.0);
        assert!(!held(&shared, JoypadButton::Up));
        assert!(!held(&shared, JoypadButton::Down));

        // Horizontal stays as it was: right is positive.
        apply_axis(&shared, &mut stick, Axis::LeftStickX, 1.0);
        assert!(held(&shared, JoypadButton::Right));
        apply_axis(&shared, &mut stick, Axis::LeftStickX, -1.0);
        assert!(held(&shared, JoypadButton::Left));
    }

    #[test]
    fn mute_toggles_and_says_so() {
        let mut boi = Pocketboi::new(&temp("mute"));
        assert!(!boi.status().contains("muted"));
        assert_eq!(boi.command(&cread_core::Command::Mute).unwrap(), "muted");
        assert!(boi.status().contains("muted"));
        assert_eq!(boi.command(&cread_core::Command::Mute).unwrap(), "sound on");
        assert!(!boi.status().contains("muted"));
    }

    #[test]
    fn a_pad_gets_a_line_of_its_own_under_the_keys() {
        let mut screen = Screen::new(80, 25);
        draw_legend(&mut screen, 0, true);
        let text: Vec<String> = (21..24)
            .map(|y| (0..80).map(|x| screen.get(x, y).unwrap().ch).collect())
            .collect();
        assert!(text[0].contains("Space A"), "{:?}", text[0]);
        assert!(text[1].trim().is_empty(), "a row of air: {:?}", text[1]);
        assert!(text[2].contains("Gamepad recognized!"), "{:?}", text[2]);

        let mut screen = Screen::new(80, 25);
        draw_legend(&mut screen, 0, false);
        let text: Vec<String> = (21..24)
            .map(|y| (0..80).map(|x| screen.get(x, y).unwrap().ch).collect())
            .collect();
        assert!(!text[2].contains("Gamepad"), "no pad, no line");
    }

    #[test]
    fn exit_steps_back_to_the_shelf_before_it_leaves_the_module() {
        let vault = temp("exit");
        let roms = vault.join("Roms");
        std::fs::create_dir_all(&roms).unwrap();
        std::fs::copy("/tmp/opencode/test.gb", roms.join("test.gb")).ok();
        let mut boi = Pocketboi::new(&vault);
        if boi.play(&roms.join("test.gb")).is_err() {
            // No cartridge to load in this environment: nothing to step out of.
            return;
        }
        assert!(boi.wants_frames());
        assert!(boi.exit(), "one level: back to the shelf");
        assert!(boi.game.is_none());
        assert!(!boi.wants_frames());
        assert!(!boi.exit(), "nothing left: the shell takes over");
        let _ = std::fs::remove_dir_all(&vault);
    }

    #[test]
    fn the_shelf_leaves_the_page_colour_alone() {
        let mut raster = Raster::new(640, 384);
        raster.fill_clear();
        // No game: the module draws text cells, not a black box.
        let mut boi = Pocketboi::new(&temp("paint"));
        boi.palette(15, 0);
        let mut screen = Screen::new(80, 25);
        boi.draw(&mut screen, &mut raster);
        assert_eq!(raster.get(0, 0), Some(cread_core::raster::CLEAR));
        assert_eq!(raster.get(639, 383), Some(cread_core::raster::CLEAR));
        // The title row carries ink the shell can see.
        assert!(screen.get(2, 1).is_some_and(|c| c.ch != '\0'));
    }
}
