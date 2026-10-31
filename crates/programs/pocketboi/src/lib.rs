//! Pocketboi: a Game Boy in a window.
//!
//! The emulation is Mizu's; this crate is the cartridge shelf and the picture.
//! Mizu gives a 160x144 RGB frame per tick, which we reduce to the four shades
//! of the machine and blow up by two into the emulated raster.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use cread_core::program::{KeyPress, Outcome, PointerEvent, Program};
use cread_core::{Cell, Raster, Screen};
use gilrs::{Axis, Button, EventType, Gilrs};
use mizu_core::{GameBoy, JoypadButton};
use rodio::buffer::SamplesBuffer;
use rodio::queue::{SourcesQueueInput, queue};
use rodio::{OutputStream, OutputStreamBuilder, Sink};

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
struct Game {
    gb: GameBoy,
    title: String,
    /// Set when the emulator gave up: the picture freezes, the shell lives.
    fault: Option<String>,
}

/// A game pad, polled once per frame. No thread: the emulation loop is
/// already running at 60 Hz, so asking gilrs "anything new?" costs nothing.
struct Pad {
    gilrs: Gilrs,
    /// Which directions the left stick is holding down, so it only reports a
    /// change when it crosses the dead zone.
    stick: [bool; 4],
}

impl Pad {
    fn open() -> Option<Self> {
        let gilrs = Gilrs::new().ok()?;
        Some(Self {
            gilrs,
            stick: [false; 4],
        })
    }

    /// Any pad plugged in right now.
    fn connected(&self) -> bool {
        self.gilrs.gamepads().next().is_some()
    }

    fn poll(&mut self, gb: &mut GameBoy) {
        while let Some(event) = self.gilrs.next_event() {
            match event.event {
                EventType::ButtonPressed(button, _) => {
                    if let Some(button) = pad_button(button) {
                        gb.press_joypad(button);
                    }
                }
                EventType::ButtonReleased(button, _) => {
                    if let Some(button) = pad_button(button) {
                        gb.release_joypad(button);
                    }
                }
                EventType::AxisChanged(axis, value, _) => self.axis(gb, axis, value),
                _ => {}
            }
        }
    }

    fn axis(&mut self, gb: &mut GameBoy, axis: Axis, value: f32) {
        const DEAD: f32 = 0.5;
        let (slot, positive, negative) = match axis {
            Axis::LeftStickX => (0, JoypadButton::Right, JoypadButton::Left),
            Axis::LeftStickY => (1, JoypadButton::Down, JoypadButton::Up),
            _ => return,
        };
        let held = value.abs() > DEAD;
        if held != self.stick[slot] {
            self.stick[slot] = held;
            if held {
                gb.press_joypad(if value > 0.0 { positive } else { negative });
            } else {
                gb.release_joypad(positive);
                gb.release_joypad(negative);
            }
        }
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
struct Sound {
    _stream: OutputStream,
    sink: Sink,
    input: Arc<SourcesQueueInput>,
    /// The device's own rate. We convert to it ourselves, so rodio has no
    /// reason to resample our chunks one by one.
    rate: u32,
    /// Input samples per output sample (88.2 kHz over the device rate).
    step: f64,
    /// Where the next output sample sits in the incoming stream, carried from
    /// one frame to the next so the seam is continuous.
    pos: f64,
    /// The last sample of the previous frame, for the same reason.
    tail: f32,
    muted: bool,
}

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
        // keep_alive: the stream stays open between frames instead of
        // clicking as the queue empties.
        let (input, output) = queue(true);
        let sink = Sink::connect_new(stream.mixer());
        sink.set_volume(0.6);
        sink.append(output);
        let rate = stream.config().sample_rate();
        Some(Self {
            _stream: stream,
            sink,
            input,
            rate,
            step: SAMPLE_RATE as f64 / rate as f64,
            pos: 0.0,
            tail: 0.0,
            muted: false,
        })
    }

    fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
        self.sink.set_volume(if muted { 0.0 } else { 0.6 });
    }

    /// One frame of Game Boy audio, resampled to the device's rate.
    ///
    /// The resampling is ours and carries its phase across frames: handing
    /// rodio a fresh 88.2 kHz buffer every 16 ms made it resample each chunk
    /// from scratch, and the seams were audible as a fine crackle.
    fn push(&mut self, input: &[f32]) {
        if input.is_empty() || self.muted {
            return;
        }
        let mut src = Vec::with_capacity(input.len() + 1);
        src.push(self.tail);
        src.extend_from_slice(input);
        let last = src.len() - 1;
        let mut pos = self.pos;
        let mut out = Vec::with_capacity((last as f64 / self.step) as usize + 2);
        while pos < last as f64 {
            let i = pos.floor() as usize;
            let frac = (pos - i as f64) as f32;
            out.push(src[i] + (src[i + 1] - src[i]) * frac);
            pos += self.step;
        }
        self.pos = pos - last as f64;
        self.tail = src[last];
        if !out.is_empty() {
            self.input.append(SamplesBuffer::new(1, self.rate, out));
        }
    }
}

pub struct Pocketboi {
    dir: PathBuf,
    entries: Vec<Entry>,
    selected: usize,
    game: Option<Game>,
    sound: Option<Sound>,
    sound_failed: bool,
    /// Silence, kept here as well so it survives the sound being closed and
    /// stays testable without a device.
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
            sound: None,
            sound_failed: false,
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
        match GameBoy::builder(path).build() {
            Ok(gb) => {
                let title = gb.game_title().trim().to_string();
                self.game = Some(Game {
                    gb,
                    title: if title.is_empty() {
                        path.file_stem()
                            .map(|s| s.to_string_lossy().into_owned())
                            .unwrap_or_else(|| "cartridge".into())
                    } else {
                        title
                    },
                    fault: None,
                });
                Ok(())
            }
            Err(e) => Err(format!("{e}")),
        }
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
        if self.game.is_none() {
            return;
        }
        // The speaker is opened on the first frame, not on the shelf.
        if self.sound.is_none() && !self.sound_failed {
            self.sound = Sound::open();
            self.sound_failed = self.sound.is_none();
            if let Some(sound) = self.sound.as_mut() {
                sound.set_muted(self.muted);
            }
        }
        if self.pad.is_none() && !self.pad_failed {
            self.pad = Pad::open();
            self.pad_failed = self.pad.is_none();
        }
        {
            let Some(game) = self.game.as_mut() else {
                return;
            };
            if let Some(pad) = self.pad.as_mut() {
                pad.poll(&mut game.gb);
            }
            if game.fault.is_none() {
                // A malformed cartridge must not take CRead down with it: the
                // emulator is allowed exactly one panic, and it costs the game.
                let stepped = catch_unwind(AssertUnwindSafe(|| game.gb.clock_for_frame()));
                match stepped {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => game.fault = Some(format!("{e:?}")),
                    Err(_) => game.fault = Some("the emulator crashed".into()),
                }
            }
            if let Some(sound) = self.sound.as_mut() {
                let samples = game.gb.audio_buffers().all().to_vec();
                sound.push(&samples);
            }
            paint(&game.gb, raster, PALETTES[self.palette].1, self.ink);
        }
        let pad = self.pad.as_ref().is_some_and(Pad::connected);
        draw_legend(screen, self.ink, pad);
    }

    fn status(&self) -> String {
        let base = match &self.game {
            Some(game) => match &game.fault {
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
fn paint(gb: &GameBoy, raster: &mut Raster, shades: [u8; 4], ink: u8) {
    // The shell already filled the raster with the page colour; leave it.
    let frame = gb.screen_buffer();
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
                && let Some(game) = self.game.as_mut()
            {
                game.gb.press_joypad(button);
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
        if let Some(game) = self.game.as_mut()
            && let Some(button) = button(key)
        {
            game.gb.release_joypad(button);
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
                if let Some(sound) = self.sound.as_mut() {
                    sound.set_muted(self.muted);
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
            self.sound = None;
            self.rescan();
            return true;
        }
        false
    }

    fn wants_frames(&self) -> bool {
        self.game.as_ref().is_some_and(|game| game.fault.is_none())
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
