//! Window lifetime, frame loop and input.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use cread_core::{
    Cell, Editor, KeyPress, Position, Prefs, Raster, ScaleMode, Screen, Stats, Theme, Viewport,
    command::{self, Command, CommandError},
    disk::Disk,
    picture,
    program::{Outcome, PointerEvent, PointerKind, Program},
    resolution::target_line_px_for_scale_factor,
    storage,
};
use cread_render::{
    CrtSettings, Face, Font, FrameStatus, Framebuffer, Renderer, Splash, SplashStyle, raster,
};
use cread_softhand::{CANVAS_W, SURFACE_H as SOFTHAND_H, Softhand};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey};
use winit::window::{Fullscreen, Window, WindowId};

use crate::cli::Options;
use crate::ui;
use cread_sync::{Config as SyncConfig, Event as SyncEvent, Report as SyncReport, Worker};

const WELCOME: &str = include_str!("welcome.md");

/// How long the editor waits after the last keystroke before writing to disk.
const AUTOSAVE_DELAY: Duration = Duration::from_millis(1500);

/// Cursor blink, in toggles per second.
const CURSOR_BLINK_HZ: f32 = 1.6;
/// How long the fake loading bar runs. Long enough to read, short enough not to
/// annoy; any key skips it.
const LOADING_SECONDS: f32 = 1.4;
/// Scanline drift, in emulated rows per second: a full 480-line screen every
/// twelve seconds. Fast enough to see, slow enough not to pull the eye.
const ROLL_SPEED: f32 = 40.0;
/// While the raster drifts we need a frame clock again; this is the budget.
const ANIMATION_FPS: u32 = 60;
/// How long a status message stays on the line before the mode hint returns.
const STATUS_TTL: Duration = Duration::from_secs(4);
/// Launch: the monitor warms up, a bar runs, the monitor syncs once more, then
/// the picture holds. Short enough to read as a PC starting rather than a wait.
const BOOT_FLICKER_SECONDS: f32 = 0.7;
const BOOT_LOADING_SECONDS: f32 = 1.0;
/// What the bar says while the "system" starts.
const BOOT_LABEL: &str = "Starting CRead OS...";
/// (seconds into the phase, brightness) — a monitor looking for a signal.
const BOOT_WARMUP_STEPS: &[(f32, f32)] = &[
    (0.00, 0.0),
    (0.08, 0.9),
    (0.14, 0.0),
    (0.20, 0.5),
    (0.26, 0.0),
    (0.34, 1.0),
    (0.42, 0.2),
    (0.50, 1.0),
    (0.58, 0.45),
    (0.62, 1.0),
];

pub fn run(opts: Options) -> Result<(), String> {
    let event_loop = EventLoop::new().map_err(|e| format!("event loop: {e}"))?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = App::new(opts);
    event_loop
        .run_app(&mut app)
        .map_err(|e| format!("event loop: {e}"))
}

struct Document {
    path: Option<PathBuf>,
    /// Opened from a diskette: shown in the reader, never written back.
    read_only: bool,
    /// Shown when there is no path yet, e.g. the built-in demo document.
    title_hint: Option<String>,
    editor: Editor,
    last_edit: Instant,
}

impl Document {
    fn title(&self) -> String {
        match &self.path {
            Some(p) => p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "untitled".into()),
            None => self.title_hint.clone().unwrap_or_else(|| "untitled".into()),
        }
    }
}

#[derive(Clone)]
enum Action {
    Open(PathBuf),
    OpenPicture(PathBuf),
    NewFile,
    SyncNow,
}

/// What the file palette is currently doing.
#[derive(Clone, PartialEq, Eq)]
enum PaletteMode {
    Browse,
    /// `delete x.md?` waiting for y/n.
    ConfirmDelete(PathBuf),
    /// Renaming `old` as it is typed.
    Rename {
        old: PathBuf,
        new: String,
    },
}

struct Palette {
    input: String,
    entries: Vec<(String, Action)>,
    filtered: Vec<usize>,
    selected: usize,
    title: String,
    mode: PaletteMode,
    /// Pictures cannot be renamed or deleted from here, so the hint differs.
    pictures: bool,
}

impl Palette {
    fn empty() -> Self {
        Self {
            input: String::new(),
            entries: Vec::new(),
            filtered: Vec::new(),
            selected: 0,
            title: "OPEN".into(),
            mode: PaletteMode::Browse,
            pictures: false,
        }
    }

    fn refilter(&mut self) {
        let needle = self.input.to_lowercase();
        self.filtered = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, (name, _))| needle.is_empty() || name.to_lowercase().contains(&needle))
            .map(|(i, _)| i)
            .collect();
        self.selected = self.selected.min(self.filtered.len().saturating_sub(1));
    }

    fn items(&self) -> Vec<String> {
        self.filtered
            .iter()
            .map(|i| self.entries[*i].0.clone())
            .collect()
    }

    fn current(&self) -> Option<&Action> {
        self.filtered
            .get(self.selected)
            .map(|i| &self.entries[*i].1)
    }
}

/// An active incremental search.
struct Search {
    /// Where the cursor was when the search started, for Esc to restore.
    origin: Position,
    query: String,
    matches: Vec<(Position, usize)>,
    current: usize,
}

/// Where the launch sequence is: flicker, bar, flicker, picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BootPhase {
    /// An empty screen stuttering on: the monitor, before there is a picture.
    Warmup,
    /// The bar, still on an empty screen.
    Loading,
}

/// What just happened at a phase boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BootEvent {
    /// Warm-up over: the bar starts.
    Loading,
    /// Bar full: beep once and draw the real screen.
    Ready,
}

struct BootState {
    phase: BootPhase,
    started: Instant,
}

impl BootState {
    fn new() -> Self {
        Self {
            phase: BootPhase::Warmup,
            started: Instant::now(),
        }
    }

    fn elapsed(&self) -> f32 {
        self.started.elapsed().as_secs_f32()
    }

    fn brightness(&self) -> f32 {
        match self.phase {
            BootPhase::Warmup => level_at(BOOT_WARMUP_STEPS, self.elapsed()),
            BootPhase::Loading => 1.0,
        }
    }

    /// Bar fill, 0 to 1, done before the wait is over so it never stalls short.
    fn progress(&self) -> f32 {
        (self.elapsed() / (BOOT_LOADING_SECONDS * 0.8))
            .powf(0.8)
            .clamp(0.0, 1.0)
    }

    fn tick(&mut self) -> Option<BootEvent> {
        let event = boot_advance(self.phase, self.elapsed())?;
        if event == BootEvent::Loading {
            self.phase = BootPhase::Loading;
            self.started = Instant::now();
        }
        Some(event)
    }
}

/// The phase machine on its own, so the order is testable without a clock.
fn boot_advance(phase: BootPhase, elapsed: f32) -> Option<BootEvent> {
    let done = match phase {
        BootPhase::Warmup => elapsed >= BOOT_FLICKER_SECONDS,
        BootPhase::Loading => elapsed >= BOOT_LOADING_SECONDS,
    };
    if !done {
        return None;
    }
    Some(match phase {
        BootPhase::Warmup => BootEvent::Loading,
        BootPhase::Loading => BootEvent::Ready,
    })
}

/// Brightness from a step table at `seconds`.
fn level_at(steps: &[(f32, f32)], seconds: f32) -> f32 {
    steps
        .iter()
        .rev()
        .find(|(from, _)| seconds >= *from)
        .map(|(_, level)| *level)
        .unwrap_or(0.0)
}

/// A diskette found in the vault, parsed or not — a broken disk still shows up
/// on the boot screen so its owner can see what went wrong.
struct DiskEntry {
    /// File name without extension, for the label when the disk is broken.
    stem: String,
    disk: Option<Disk>,
    error: Option<String>,
}

/// Which home screen the shell is showing when nothing is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Home {
    /// The boot screen: drives and diskettes.
    Boot,
    /// Inspiro's own front door: logo, tagline, new/open.
    Inspiro,
    /// Softhand's front door: same shape, its own logo and tagline.
    SoftHand,
}

/// What happens when the loading bar finishes.
enum Pending {
    Inspiro,
    /// Index into [`PROGRAMS`]: load that program's front door.
    Program(usize),
    NewPicture,
    Picture(PathBuf),
    Document(PathBuf),
    NewDocument,
}

/// One row of the boot screen.
#[derive(Clone, Copy)]
enum BootItem {
    /// The vault: the first row, always.
    Documents,
    /// The rule between the vault and the modules. Never selectable.
    Rule,
    /// The editor, the first module.
    Inspiro,
    /// Index into [`PROGRAMS`].
    Program(usize),
    Disk(usize),
}

impl BootItem {
    fn selectable(self) -> bool {
        !matches!(self, BootItem::Rule)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    /// Typing into the document.
    Write,
    /// The `/` prompt at the bottom.
    Search,
    /// Reader mode: markup stripped, not editable.
    View,
    /// Browsing the files on a diskette.
    DiskBrowser,
    /// A fake loading bar before a program appears.
    Loading,
    /// A command line at the bottom, like a shell.
    Action,
    /// File picker.
    Palette,
    /// WebDAV settings.
    Settings,
    /// Full-screen help or about.
    Help,
    /// A program owns the screen.
    Program,
}

/// A program compiled into this build.
pub struct ProgramInfo {
    pub id: &'static str,
    /// What the boot row and the loading bar call it.
    pub entry: &'static str,
    pub tagline: &'static str,
    /// Installed programs always have a row. The others need a diskette whose
    /// header says `program=<id>`; the code is in the binary either way, the
    /// diskette is only what puts the drive on the shelf.
    pub installed: bool,
}

pub const PROGRAMS: &[ProgramInfo] = &[ProgramInfo {
    id: "softhand",
    entry: "SOFTHAND",
    tagline: "16 colors, endless possibilities.",
    installed: true,
}];

fn program_index_by_id(id: &str) -> Option<usize> {
    PROGRAMS.iter().position(|program| program.id == id)
}

/// The home screen a program opens onto.
fn program_home(index: usize) -> Home {
    match PROGRAMS.get(index).map(|program| program.id) {
        Some("softhand") => Home::SoftHand,
        _ => Home::Boot,
    }
}

/// Append one drive row with the next free letter.
fn push_boot_row(
    rows: &mut Vec<ui::BootItem>,
    letter: &mut u8,
    label: String,
    detail: String,
    broken: bool,
) {
    rows.push(ui::BootItem {
        label: format!("{}: {label}", *letter as char),
        detail,
        broken,
        rule: false,
    });
    *letter += 1;
}

struct App {
    opts: Options,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    adapter: String,

    font: Font,
    screen: Screen,
    fb: Framebuffer,
    vp: Option<Viewport>,

    theme: Theme,
    crt: CrtSettings,
    prefs: Prefs,

    start: Instant,
    last_frame: Instant,
    next_frame: Instant,
    dirty: bool,
    blink: bool,
    mods: ModifiersState,
    typewriter: bool,

    doc: Document,
    /// Diskettes in `<vault>/Disks`, refreshed when the boot screen appears.
    disks: Vec<DiskEntry>,
    /// Set when the boot screen should re-read the disk folder.
    disks_dirty: bool,
    boot_selected: usize,
    /// Index into `disks` while the browser is open.
    disk_open: usize,
    disk_selected: usize,
    splash: Option<Splash>,
    /// Where the logo was blitted this frame: x, y and the integer divisor.
    splash_at: Option<(i32, i32, u32)>,
    /// Same, for the Inspiro front door.
    inspiro_at: Option<(i32, i32, u32)>,
    /// The file `new` wrote to reserve a name. Dropped again when the document
    /// is left without a single character in it, so the vault does not silt up
    /// with empty `untitled.md` files.
    scratch: Option<PathBuf>,
    search: Option<Search>,
    last_search: Option<String>,
    /// Reader-mode rows, computed on entering the view and on resize.
    view_rows: Vec<cread_core::markdown::Rendered>,
    view_top: usize,
    view_width: usize,
    /// Last known pointer position in physical pixels.
    pointer: (f64, f64),
    dragging: bool,
    /// False after a wheel scroll, until the next key or click: the view must
    /// not snap back to the cursor while you are just looking around.
    follow_cursor: bool,
    /// Kept alive for the whole run: on Wayland the clipboard is served by the
    /// owning process, so dropping the handle would lose the copied text.
    clipboard: Option<arboard::Clipboard>,
    /// False on the start screen: no file is open yet.
    has_document: bool,
    /// Which home screen to show while nothing is open.
    home: Home,
    pending: Option<Pending>,
    loading_started: Instant,
    inspiro_logo: Option<Splash>,
    softhand_logo: Option<Splash>,
    /// Where the Softhand logo sits this frame.
    softhand_at: Option<(i32, i32, u32)>,
    /// The running program, if any.
    program: Option<Box<dyn Program>>,
    /// Pixel layer handed to programs.
    raster: Raster,
    /// File the current picture came from or was last saved to.
    picture_path: Option<PathBuf>,
    /// The program's command line is open (Esc between drawing and typing).
    program_prompt: bool,
    /// Last raster pixel the pointer reported, so sub-pixel motion at scale 2 or
    /// 3 does not redraw the world.
    program_point: (i32, i32),
    mode: Mode,
    palette: Palette,
    form: FormState,
    status: String,
    /// When the current status message was set; it fades back to the hint.
    status_at: Instant,
    screenshot_done: bool,

    /// Action-mode command line.
    command: String,
    command_error: String,
    history: Vec<String>,
    history_pos: Option<usize>,
    /// Pending yes/no question, e.g. before deleting a file.
    confirm: Option<String>,
    help_about: bool,
    /// Highlighted row on the help screen.
    help_selected: usize,
    /// `help all` shows the whole vocabulary instead of this screen's list.
    help_all: bool,
    /// The launch sequence, while it runs.
    boot: Option<BootState>,
    /// The startup beep, mirrored from the preferences.
    beep_on: bool,

    sync_cfg: SyncConfig,
    sync: Option<Worker>,
}

/// Editable copy of the WebDAV settings, shown as a form.
struct FormState {
    url: String,
    username: String,
    password: String,
    remote_dir: String,
    enabled: bool,
    focus: usize,
    note: String,
}

impl FormState {
    fn from_config(cfg: &SyncConfig) -> Self {
        Self {
            url: cfg.url.clone(),
            username: cfg.username.clone(),
            password: cfg.password.clone(),
            remote_dir: cfg.remote_dir.clone(),
            enabled: cfg.enabled,
            focus: 0,
            note: String::new(),
        }
    }

    fn to_config(&self) -> SyncConfig {
        SyncConfig {
            url: self.url.trim().to_string(),
            username: self.username.trim().to_string(),
            password: self.password.clone(),
            remote_dir: self.remote_dir.trim().trim_matches('/').to_string(),
            enabled: self.enabled,
        }
    }

    fn field_count() -> usize {
        5
    }

    fn fields(&self) -> Vec<ui::FormField> {
        vec![
            ui::FormField {
                label: "server".into(),
                value: self.url.clone(),
                secret: false,
            },
            ui::FormField {
                label: "user".into(),
                value: self.username.clone(),
                secret: false,
            },
            ui::FormField {
                label: "password".into(),
                value: self.password.clone(),
                secret: true,
            },
            ui::FormField {
                label: "folder".into(),
                value: self.remote_dir.clone(),
                secret: false,
            },
            ui::FormField {
                label: "sync".into(),
                value: if self.enabled {
                    "on".into()
                } else {
                    "off".into()
                },
                secret: false,
            },
        ]
    }

    fn text_field_mut(&mut self) -> Option<&mut String> {
        match self.focus {
            0 => Some(&mut self.url),
            1 => Some(&mut self.username),
            2 => Some(&mut self.password),
            3 => Some(&mut self.remote_dir),
            _ => None,
        }
    }
}

impl App {
    fn new(opts: Options) -> Self {
        let prefs = Prefs::load();
        // Command line wins over saved preferences, which win over defaults.
        let mut crt = if opts.crt_explicit {
            opts.crt
        } else {
            CrtSettings {
                scanline: if prefs.scanlines {
                    CrtSettings::default().scanline
                } else {
                    0.0
                },
                bloom: if prefs.bloom {
                    CrtSettings::default().bloom
                } else {
                    0.0
                },
                roll: if prefs.roll { ROLL_SPEED } else { 0.0 },
                ..CrtSettings::default()
            }
        };
        if opts.crt_explicit {
            crt.brightness = opts.crt.brightness;
        }
        let theme = opts.theme.unwrap_or_else(|| prefs.theme());
        let typewriter = prefs.typewriter;
        let beep_on = prefs.beep;
        let (doc, has_document) = Self::initial_document(&opts);
        let sync_cfg = SyncConfig::load();
        Self {
            opts,
            window: None,
            renderer: None,
            adapter: String::from("unknown"),
            font: Font::with_face(Face::by_name(&prefs.font).unwrap_or_default()),
            screen: Screen::new(80, 25),
            fb: Framebuffer::new(640, 400),
            vp: None,
            theme,
            crt,
            prefs,
            start: Instant::now(),
            last_frame: Instant::now(),
            next_frame: Instant::now(),
            dirty: true,
            blink: true,
            mods: ModifiersState::empty(),
            typewriter,
            doc,
            disks: Vec::new(),
            disks_dirty: true,
            boot_selected: 0,
            disk_open: 0,
            disk_selected: 0,
            splash: Splash::from_png(include_bytes!("../../../assets/splash.png"))
                .map_err(|e| eprintln!("cread: {e}"))
                .ok(),
            splash_at: None,
            inspiro_at: None,
            scratch: None,
            search: None,
            last_search: None,
            view_rows: Vec::new(),
            view_top: 0,
            view_width: 0,
            pointer: (0.0, 0.0),
            dragging: false,
            follow_cursor: true,
            clipboard: None,
            has_document,
            home: Home::Boot,
            pending: None,
            loading_started: Instant::now(),
            inspiro_logo: Splash::from_png(include_bytes!("../../../assets/inspiro.png"))
                .map_err(|e| eprintln!("cread: inspiro logo: {e}"))
                .ok(),
            softhand_logo: Splash::from_png(include_bytes!("../../../assets/softhand.png"))
                .map_err(|e| eprintln!("cread: softhand logo: {e}"))
                .ok(),
            softhand_at: None,
            program: None,
            raster: Raster::new(CANVAS_W, SOFTHAND_H),
            picture_path: None,
            program_prompt: false,
            program_point: (-1, -1),
            mode: if has_document {
                Mode::Write
            } else {
                Mode::Action
            },
            palette: Palette::empty(),
            form: FormState::from_config(&sync_cfg),
            status: String::new(),
            status_at: Instant::now(),
            screenshot_done: false,
            command: String::new(),
            command_error: String::new(),
            history: Vec::new(),
            history_pos: None,
            confirm: None,
            help_about: false,
            help_selected: 0,
            help_all: false,
            boot: None,
            beep_on,
            sync_cfg,
            sync: None,
        }
    }

    /// Returns the buffer and whether it counts as an open document.
    ///
    /// A file argument or `--demo` opens straight into write mode; otherwise we
    /// start on the greeting screen. Silently resuming the last file makes it
    /// too easy to type into the wrong document.
    fn initial_document(opts: &Options) -> (Document, bool) {
        if let Some(path) = &opts.file {
            match storage::load(path) {
                Ok(text) => {
                    return (
                        Document {
                            path: Some(path.clone()),
                            read_only: false,
                            title_hint: None,
                            editor: Editor::from_text(&text),
                            last_edit: Instant::now(),
                        },
                        true,
                    );
                }
                Err(e) => eprintln!("cread: cannot open {}: {e}", path.display()),
            }
        }

        if opts.demo {
            return (
                Document {
                    path: None,
                    read_only: false,
                    title_hint: Some("welcome.md".into()),
                    editor: Editor::from_text(WELCOME),
                    last_edit: Instant::now(),
                },
                true,
            );
        }

        (
            Document {
                path: None,
                read_only: false,
                title_hint: None,
                editor: Editor::new(),
                last_edit: Instant::now(),
            },
            false,
        )
    }

    fn backends(&self) -> wgpu::Backends {
        match self.opts.backend.as_deref() {
            Some("vulkan") => wgpu::Backends::VULKAN,
            Some("gl") => wgpu::Backends::GL,
            _ => wgpu::Backends::all(),
        }
    }

    fn recompute_viewport(&mut self) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        let (w, h) = self.target_phys_size(window);
        let mode = match self.opts.scale {
            Some(scale) => ScaleMode::Fixed(scale),
            None => ScaleMode::Auto {
                target_line_px: target_line_px_for_scale_factor(window.scale_factor()),
            },
        };
        let vp = Viewport::compute_for(w, h, mode, self.font.cell_w(), self.font.cell_h());
        if vp != self.vp {
            self.vp = vp;
            if let (Some(vp), Some(renderer)) = (vp, self.renderer.as_mut()) {
                renderer.resize(vp.phys_w, vp.phys_h);
            }
            self.dirty = true;
        }
    }

    fn target_phys_size(&self, window: &Window) -> (u32, u32) {
        if self.opts.screenshot.is_some()
            && let Some(size) = self.opts.size
        {
            return size;
        }
        let size = window.inner_size();
        (size.width.max(1), size.height.max(1))
    }

    // ── frame ───────────────────────────────────────────────────────────────

    fn frame(&mut self, el: &ActiveEventLoop) {
        self.pump_sync();
        // Status messages are transient: after a moment the line goes back to
        // telling you how to change modes.
        if self.mode == Mode::Loading
            && self.loading_started.elapsed().as_secs_f32() >= LOADING_SECONDS
        {
            self.finish_loading();
        }
        if !self.status.is_empty() && self.status_at.elapsed() >= STATUS_TTL {
            self.status.clear();
            self.dirty = true;
        }
        self.autosave_if_due();

        let Some(vp) = self.vp else {
            return;
        };
        if self.renderer.is_none() {
            return;
        }

        let frame_start = Instant::now();
        let time = self.start.elapsed().as_secs_f32();
        let blink = ((time * CURSOR_BLINK_HZ) as u32).is_multiple_of(2);
        if blink != self.blink {
            self.blink = blink;
            self.dirty = true;
        }

        if let Some(boot) = self.boot.as_mut() {
            match boot.tick() {
                Some(BootEvent::Loading) => {}
                Some(BootEvent::Ready) => {
                    // The bar is full: the speaker says so, then the picture
                    // appears. Nothing of the boot screen is drawn before.
                    self.beep();
                    self.boot = None;
                    self.dirty = true;
                }
                None => {}
            }
            // The bar has to move every frame; the flickers only change the
            // brightness, which is a uniform.
            if self
                .boot
                .as_ref()
                .is_some_and(|b| b.phase == BootPhase::Loading)
            {
                self.dirty = true;
            }
        }

        let mut crt = self.crt;
        if let Some(level) = self.boot.as_ref().map(BootState::brightness) {
            crt.brightness *= level;
        }
        if self.theme.is_light() {
            // Glow on paper is ink bleed: keep a trace, not a haze.
            crt.bloom *= 0.3;
            crt.scanline *= 0.4;
        }

        if self.dirty {
            if self.screen.cols() != vp.cols() || self.screen.rows() != vp.rows() {
                self.screen = Screen::new(vp.cols(), vp.rows());
            }
            if self.fb.width() != vp.virt_w || self.fb.height() != vp.virt_h {
                self.fb.resize(vp.virt_w, vp.virt_h);
            }
            self.compose();
            if self.program.is_some() {
                self.fb
                    .draw_program(&self.screen, &self.raster, &self.theme, &self.font);
            } else {
                self.fb.draw_screen(&self.screen, &self.theme, &self.font);
            }
            if let (Some(splash), Some((x, y, divisor))) = (self.splash.as_ref(), self.splash_at) {
                // The artwork is black on transparency, so the shape comes from
                // the alpha channel and the ink follows the theme: dark on
                // paper, light on the dark schemes.
                let style = SplashStyle::Ink(self.theme.color(self.theme.fg));
                self.fb.draw_splash(splash, x, y, divisor, &style);
            }
            if let (Some(logo), Some((x, y, divisor))) =
                (self.inspiro_logo.as_ref(), self.inspiro_at)
            {
                let style = SplashStyle::Ink(self.theme.color(self.theme.fg));
                self.fb.draw_splash(logo, x, y, divisor, &style);
            }
            if let (Some(logo), Some((x, y, divisor))) =
                (self.softhand_logo.as_ref(), self.softhand_at)
            {
                let style = SplashStyle::Ink(self.theme.color(self.theme.fg));
                self.fb.draw_splash(logo, x, y, divisor, &style);
            }
            if let Some(renderer) = self.renderer.as_mut() {
                renderer.upload(&self.fb);
            }
            self.dirty = false;
        }

        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };

        if let Some(path) = self.opts.screenshot.clone() {
            let ready = self.start.elapsed() >= Duration::from_millis(self.opts.screenshot_delay);
            if !self.screenshot_done && ready {
                let (w, h) = (vp.phys_w, vp.phys_h);
                match renderer.render_offscreen(&vp, time, &crt, w, h) {
                    Ok((data, w, h)) => {
                        if let Err(e) = raster::write_png(&path, w, h, &data) {
                            eprintln!("cread: cannot write {}: {e}", path.display());
                        } else {
                            println!(
                                "cread: wrote {} ({}x{}, {}x scale)",
                                path.display(),
                                w,
                                h,
                                vp.scale
                            );
                        }
                    }
                    Err(e) => eprintln!("cread: screenshot failed: {e}"),
                }
                self.screenshot_done = true;
                el.exit();
            } else if !ready {
                // Keep the loop alive so background work can finish first.
                self.next_frame = Instant::now() + Duration::from_millis(100);
            }
            return;
        }

        if let Some(window) = self.window.as_ref() {
            window.pre_present_notify();
        }
        match renderer.render(&vp, time, &crt) {
            Ok(FrameStatus::Presented) => {}
            Ok(FrameStatus::Reconfigure) => renderer.resize(vp.phys_w, vp.phys_h),
            Ok(FrameStatus::Skipped) => {}
            Err(e) => {
                eprintln!("cread: render failed: {e}");
                el.exit();
            }
        }

        self.last_frame = Instant::now();

        // Nothing animates any more except the cursor, so we do not run a
        // frame clock at all: sleep until the blink is due and redraw then.
        if self.boot.is_some() || self.mode == Mode::Loading {
            self.next_frame = frame_start + Duration::from_millis(1000 / ANIMATION_FPS as u64);
        } else if crt.roll > 0.0 && crt.scanline > 0.0 {
            // The raster moves, so this is the one case that needs a clock.
            self.next_frame = frame_start + Duration::from_millis(1000 / ANIMATION_FPS as u64);
        } else {
            let phase = (time * CURSOR_BLINK_HZ) as u32;
            let next_toggle = (phase as f32 + 1.0) / CURSOR_BLINK_HZ;
            let wait = (next_toggle - time).clamp(0.005, 1.0);
            self.next_frame = frame_start + Duration::from_secs_f32(wait);
        }
    }

    /// Build the cell grid for the current state.
    fn compose(&mut self) {
        let mut screen = std::mem::replace(&mut self.screen, Screen::new(1, 1));
        // Only the screen composed below may blit a logo. Clearing both here is
        // what keeps the Inspiro artwork from floating over the help, the
        // palette or the boot screen.
        self.splash_at = None;
        self.inspiro_at = None;
        self.softhand_at = None;
        if let Some(boot) = self.boot.as_ref() {
            match boot.phase {
                BootPhase::Warmup => {
                    // An empty screen stuttering on, not the boot screen yet.
                    screen.fill(Cell::blank(self.theme.fg, self.theme.bg));
                }
                BootPhase::Loading => {
                    ui::draw_loading(&mut screen, &self.theme, BOOT_LABEL, boot.progress());
                }
            }
            self.screen = screen;
            return;
        }
        if self.mode == Mode::Loading {
            let elapsed = self.loading_started.elapsed().as_secs_f32();
            // The meter is full before the wait ends, so the last thing you see
            // is a complete bar rather than a stalled one.
            let progress = (elapsed / (LOADING_SECONDS * 0.78))
                .powf(0.8)
                .clamp(0.0, 1.0);
            let label = match &self.pending {
                Some(Pending::Inspiro) => "Loading INSPIRO...".to_string(),
                Some(Pending::Program(index)) => format!(
                    "Loading {}...",
                    PROGRAMS.get(*index).map(|p| p.entry).unwrap_or("program")
                ),
                Some(Pending::NewPicture) => "New picture...".to_string(),
                Some(Pending::Picture(_)) => "Loading picture...".to_string(),
                Some(Pending::Document(_)) => "Loading document...".to_string(),
                Some(Pending::NewDocument) => "New document...".to_string(),
                None => "Loading...".to_string(),
            };
            ui::draw_loading(&mut screen, &self.theme, &label, progress);
            self.screen = screen;
            return;
        }
        if self.mode == Mode::DiskBrowser {
            self.splash_at = None;
            if let Some(entry) = self.disks.get(self.disk_open)
                && let Some(disk) = &entry.disk
            {
                let files: Vec<String> = disk.files.iter().map(|f| f.name.clone()).collect();
                let view = ui::DiskView {
                    title: &format!("{}:", disk.name),
                    about: &disk.about,
                    files: &files,
                    selected: self.disk_selected,
                };
                ui::draw_disk(&mut screen, &self.theme, &view);
            }
            self.screen = screen;
            return;
        }
        if self.mode == Mode::Help {
            self.splash_at = None;
            let specs: Vec<&command::Spec> = if self.help_about {
                Vec::new()
            } else if self.help_all {
                command::SPECS.iter().collect()
            } else {
                command::specs_for(self.help_scope())
            };
            ui::draw_help(
                &mut screen,
                &self.theme,
                self.help_about,
                &specs,
                self.help_selected,
            );
            self.screen = screen;
            return;
        }

        let cols = screen.cols();
        let rows = screen.rows();

        // A pending yes/no question takes over the prompt line.
        let question = self.confirm.clone().unwrap_or_default();
        let hint = if !question.is_empty() {
            String::new()
        } else if self.command.is_empty() {
            if self.program.is_some() {
                "save   open   undo   exit".to_string()
            } else if self.has_document {
                // Left empty on purpose: the prompt then shows how to leave.
                String::new()
            } else if matches!(self.home, Home::SoftHand | Home::Inspiro) {
                "new   open   help for commands".to_string()
            } else {
                "↑↓ select   Enter run   help for commands".to_string()
            }
        } else {
            command::complete(&self.command)
                .map(|c| c.hint)
                .unwrap_or_default()
        };
        let error = if question.is_empty() {
            self.command_error.clone()
        } else {
            question
        };

        if self.program.is_some() {
            let mut raster = std::mem::replace(&mut self.raster, Raster::new(1, 1));
            if let Some(program) = self.program.as_mut() {
                program.draw(&mut screen, &mut raster);
            }
            self.raster = raster;

            let title = self
                .program
                .as_ref()
                .map(|p| p.title().to_string())
                .unwrap_or_default();
            let state = self
                .program
                .as_ref()
                .map(|p| p.status())
                .unwrap_or_default();
            let status = if state.is_empty() {
                "Esc commands".to_string()
            } else {
                format!("{state}    Esc commands")
            };
            let prompt = self.program_prompt.then_some((
                self.command.as_str(),
                hint.as_str(),
                error.as_str(),
            ));
            ui::draw_program_line(&mut screen, &self.theme, &title, &status, prompt);
        } else if self.mode == Mode::View {
            let (width, _) = ui::layout(cols);
            if width != self.view_width {
                self.view_rows = cread_core::markdown::render(self.doc.editor.lines(), width);
                self.view_width = width;
            }
            let stats = self.doc.editor.stats();
            let state = if self.doc.editor.modified() {
                "modified"
            } else if self.doc.path.is_some() {
                "saved"
            } else {
                ""
            };
            let title = self.doc.title();
            let location = self.location();
            let view = ui::ReaderView {
                title: &title,
                state,
                location: &location,
                rows: &self.view_rows,
                top: self.view_top,
                words: stats.words,
                hint: if self.doc.read_only {
                    "Esc back to the diskette"
                } else {
                    "Esc back to write mode"
                },
            };
            ui::draw_reader(&mut screen, &self.theme, &view);
        } else if !self.has_document {
            if self.disks_dirty {
                self.refresh_disks();
                self.disks_dirty = false;
            }
            let show_prompt = matches!(self.mode, Mode::Action | Mode::Search);

            // Pixel art shrinks by a whole factor at most, and only when the
            // raster genuinely cannot hold it.
            let max_w = cols * cread_core::FONT_W;
            let max_h = rows.saturating_sub(8) * cread_core::FONT_H;
            // An overlay (palette, settings) covers the front door's logo the
            // same way it covers the boot screen's.
            let overlay_open = matches!(self.mode, Mode::Palette | Mode::Settings);
            if self.home == Home::Inspiro {
                let title_row =
                    match front_door_layout(&self.inspiro_logo, max_w, max_h, cols, rows) {
                        Some((x, y, divisor, img_rows)) => {
                            self.inspiro_at = (!overlay_open).then_some((x, y, divisor));
                            rows.saturating_sub(img_rows + 1 + 3) / 2 + img_rows + 1
                        }
                        None => {
                            self.inspiro_at = None;
                            rows.saturating_sub(3) / 2
                        }
                    };
                ui::draw_inspiro(
                    &mut screen,
                    &self.theme,
                    title_row,
                    &self.command,
                    &hint,
                    &error,
                );
            } else if self.home == Home::SoftHand {
                let title_row =
                    match front_door_layout(&self.softhand_logo, max_w, max_h, cols, rows) {
                        Some((x, y, divisor, img_rows)) => {
                            self.softhand_at = (!overlay_open).then_some((x, y, divisor));
                            rows.saturating_sub(img_rows + 1 + 3) / 2 + img_rows + 1
                        }
                        None => {
                            self.softhand_at = None;
                            rows.saturating_sub(3) / 2
                        }
                    };
                ui::draw_front_door(
                    &mut screen,
                    &self.theme,
                    title_row,
                    "16 colors, endless possibilities.",
                    &self.command,
                    &hint,
                    &error,
                );
            } else {
                let items = self.boot_rows();
                let note = if self.disks.is_empty() {
                    format!("no disks - put .dsk files in {}", self.short_disk_dir())
                } else {
                    String::new()
                };

                // The whole group - logo, greeting, drives - is centred vertically,
                // like the old welcome screen was; pinning the logo to the top made
                // it look like a different screen. A long drive list shrinks the
                // logo by a whole step rather than losing rows off the bottom.
                let (divisor, img_rows) = boot_logo(
                    &self.splash,
                    max_w,
                    max_h,
                    rows,
                    items.len() as u32,
                    !note.is_empty(),
                );
                let gap = u32::from(img_rows > 0);
                let block =
                    img_rows + gap + 1 + 1 + items.len() as u32 + u32::from(!note.is_empty());
                let top = rows.saturating_sub(block) / 2;
                let title_row = top + img_rows + gap;

                self.splash_at = (img_rows > 0 && !overlay_open).then(|| {
                    let width = self
                        .splash
                        .as_ref()
                        .map(|s| (s.width() / divisor) as i32)
                        .unwrap_or(0);
                    let x = (cols * cread_core::FONT_W) as i32 / 2 - width / 2;
                    (x, (top * cread_core::FONT_H) as i32, divisor)
                });

                let view = ui::WelcomeView {
                    show_prompt,
                    title_row,
                    items: &items,
                    selected: self.boot_selected,
                    note: &note,
                    input: &self.command,
                    hint: &hint,
                    error: &error,
                };
                ui::draw_welcome(&mut screen, &self.theme, &view);
            }
        } else {
            // A document is open: the logo belongs to the start screen only.
            self.splash_at = None;
            let (text_width, margin) = ui::layout(cols);
            let height = ui::text_height(rows);

            if self.follow_cursor {
                let anchor = self.typewriter.then_some(height / 2);
                self.doc.editor.scroll_to_cursor(text_width, height, anchor);
            }

            let visible = self.doc.editor.visible_rows(text_width, height);
            let cursor = self.doc.editor.cursor();
            let cursor_cell = visible
                .iter()
                .position(|r| r.contains(cursor))
                .map(|i| (i, cursor.col - visible[i].start));

            let in_fence = self
                .doc
                .editor
                .lines()
                .iter()
                .take(self.doc.editor.top().line)
                .filter(|l| cread_core::markdown::is_fence_line(l))
                .count()
                % 2
                == 1;

            let stats: Stats = self.doc.editor.stats();
            let state = if self.doc.editor.modified() {
                "modified"
            } else if self.doc.path.is_some() {
                "saved"
            } else {
                ""
            };
            let location = self.location();
            let selection = self.doc.editor.selection();
            let (matches, current_match) = match &self.search {
                Some(search) => (
                    search.matches.as_slice(),
                    (!search.matches.is_empty()).then_some(search.current),
                ),
                None => (&[][..], None),
            };
            let message = if self.status_at.elapsed() < STATUS_TTL {
                self.status.as_str()
            } else {
                ""
            };

            let search_hint = self.search.as_ref().map(|s| {
                if s.matches.is_empty() {
                    "no matches   Esc cancel".to_string()
                } else {
                    format!(
                        "{}/{}   Enter keep   Esc cancel",
                        s.current + 1,
                        s.matches.len()
                    )
                }
            });
            let bottom = if self.mode == Mode::Search {
                ui::Bottom::Prompt {
                    sigil: "/",
                    input: self.search.as_ref().map(|s| s.query.as_str()).unwrap_or(""),
                    hint: search_hint.as_deref().unwrap_or(""),
                    error: "",
                }
            } else if self.mode == Mode::Action {
                ui::Bottom::Prompt {
                    sigil: ">",
                    input: &self.command,
                    hint: &hint,
                    error: &error,
                }
            } else {
                ui::Bottom::Status { message }
            };
            // The document cursor belongs to write mode; the prompt has its own.
            let cursor_cell = if self.mode == Mode::Write {
                cursor_cell
            } else {
                None
            };

            let title = self.doc.title();
            let view = ui::DocumentView {
                title: &title,
                state,
                location: &location,
                lines: self.doc.editor.lines(),
                rows: &visible,
                selection,
                matches,
                current_match,
                cursor: cursor_cell,
                margin,
                stats,
                cursor_pos: (cursor.line, cursor.col),
                bottom,
                in_fence,
                cursor_on: self.blink,
            };
            ui::draw_document(&mut screen, &self.theme, &view);
        }

        match self.mode {
            Mode::Palette => {
                let items = self.palette.items();
                let question;
                let (prompt, input, hint) = match &self.palette.mode {
                    PaletteMode::Browse if self.palette.pictures => {
                        (">", self.palette.input.as_str(), "Enter open   Esc back")
                    }
                    PaletteMode::Browse => (
                        ">",
                        self.palette.input.as_str(),
                        "Enter open   Ctrl+R rename   Ctrl+D delete",
                    ),
                    PaletteMode::ConfirmDelete(path) => {
                        question = format!("delete {}? y/n", self.doc_title_of(path));
                        ("?", "", question.as_str())
                    }
                    PaletteMode::Rename { new, .. } => {
                        ("name>", new.as_str(), "Enter rename   Esc cancel")
                    }
                };
                let view = ui::PaletteView {
                    title: &self.palette.title,
                    prompt,
                    input,
                    items: &items,
                    selected: self.palette.selected,
                    hint,
                };
                ui::draw_palette(&mut screen, &self.theme, &view);
            }
            Mode::Settings => {
                let fields = self.form.fields();
                let view = ui::FormView {
                    title: "WEBDAV",
                    fields: &fields,
                    focus: self.form.focus,
                    note: &self.form.note,
                    hint: "Tab next   Enter toggle/save   Esc cancel",
                };
                ui::draw_form(&mut screen, &self.theme, &view);
            }
            _ => {}
        }

        self.screen = screen;
    }

    // ── persistence ─────────────────────────────────────────────────────────

    fn autosave_if_due(&mut self) {
        if self.mode != Mode::Write {
            return;
        }
        if self.doc.read_only || !self.doc.editor.modified() {
            return;
        }
        if self.doc.last_edit.elapsed() < AUTOSAVE_DELAY {
            return;
        }
        let Some(path) = self.doc.path.clone() else {
            return;
        };
        if let Err(e) = self.save_to(&path) {
            self.status = e;
        }
    }

    fn pump_sync(&mut self) {
        let Some(worker) = self.sync.as_ref() else {
            return;
        };
        let mut events = Vec::new();
        while let Some(event) = worker.try_recv() {
            events.push(event);
        }
        for event in events {
            match event {
                SyncEvent::Started => self.set_status("sync ..."),
                SyncEvent::Done(report) => {
                    if report.conflicts > 0 {
                        self.set_status(format!(
                            "sync: {} - conflict copy kept",
                            report_word(&report)
                        ));
                    } else {
                        self.set_status(format!("sync: {}", report_word(&report)));
                    }
                    if report.downloaded > 0 {
                        self.reload_after_sync();
                    }
                }
                SyncEvent::Failed(e) => self.set_status(format!("sync failed: {e}")),
            }
            self.dirty = true;
        }
    }

    /// A sync round may have replaced the file on disk. Pull it in, unless the
    /// buffer holds edits that have not been written yet.
    fn reload_after_sync(&mut self) {
        let Some(path) = self.doc.path.clone() else {
            return;
        };
        if self.doc.editor.modified() {
            return;
        }
        let Ok(text) = storage::load(&path) else {
            return;
        };
        if text == self.doc.editor.text() {
            return;
        }
        let cursor = self.doc.editor.cursor();
        self.doc.editor = Editor::from_text(&text);
        self.doc.editor.set_cursor(cursor);
        self.set_status("updated from server");
        self.dirty = true;
    }

    fn start_worker(&mut self) {
        if self.sync.is_some() {
            return;
        }
        let vault = self.opts.vault.clone().unwrap_or_else(storage::vault_dir);
        self.sync = Some(Worker::spawn(
            vault,
            self.sync_cfg.clone(),
            Duration::from_secs(60),
        ));
    }

    fn request_sync(&mut self) {
        if let Some(worker) = self.sync.as_ref() {
            worker.request_sync();
        }
    }

    fn open_settings(&mut self) {
        self.form = FormState::from_config(&self.sync_cfg);
        self.form.note = format!("config: {}", SyncConfig::path().display());
        self.mode = Mode::Settings;
        self.dirty = true;
    }

    fn save_settings(&mut self) {
        let cfg = self.form.to_config();
        if cfg.enabled && !cfg.is_configured() {
            self.form.note = "a server URL is required".into();
            self.dirty = true;
            return;
        }
        match cfg.save() {
            Ok(()) => {
                self.sync_cfg = cfg.clone();
                self.start_worker();
                if let Some(worker) = self.sync.as_ref() {
                    worker.configure(cfg);
                }
                self.to_write();
                self.set_status(if self.sync_cfg.enabled {
                    "sync on"
                } else {
                    "sync off"
                });
            }
            Err(e) => self.form.note = format!("cannot write config: {e}"),
        }
        self.dirty = true;
    }

    fn save_to(&mut self, path: &Path) -> Result<(), String> {
        let text = self.doc.editor.text();
        storage::save_atomic(path, &text).map_err(|e| format!("save failed: {e}"))?;
        self.doc.editor.mark_saved();
        self.doc.path = Some(path.to_path_buf());
        if self.sync_cfg.enabled {
            self.request_sync();
        }
        self.dirty = true;
        Ok(())
    }

    fn save(&mut self) -> Result<String, String> {
        if self.doc.read_only {
            return Err("this came from a diskette - read only".into());
        }
        let path = match self.doc.path.clone() {
            Some(path) => path,
            None => self.free_untitled_path(),
        };
        self.save_to(&path)?;
        Ok(String::new())
    }

    /// Start a fresh file in the vault, named `untitled.md`, `untitled-2.md`, …
    fn new_document(&mut self) -> Result<String, String> {
        self.discard_scratch();
        let path = self.free_untitled_path();
        self.doc = Document {
            path: None,
            read_only: false,
            title_hint: None,
            editor: Editor::new(),
            last_edit: Instant::now(),
        };
        self.has_document = true;
        self.save_to(&path)?;
        self.scratch = Some(path);
        Ok(format!("created {}", self.doc.title()))
    }

    /// Delete the file `new` reserved if the document is being left empty.
    ///
    /// Both the buffer and the file on disk have to be blank: a scratch file
    /// that autosave filled is somebody's text, and an in-memory document with
    /// unsaved characters must keep its place on disk.
    fn discard_scratch(&mut self) -> Option<String> {
        let path = self.scratch.take()?;
        let same = self.doc.path.as_deref() == Some(path.as_path());
        let unsaved = same && !self.doc.editor.text().trim().is_empty();
        let blank_on_disk = std::fs::read_to_string(&path)
            .map(|text| text.trim().is_empty())
            .unwrap_or(false);
        if unsaved || !blank_on_disk {
            return None;
        }
        std::fs::remove_file(&path).ok()?;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.set_status(format!("removed empty {name}"));
        self.dirty = true;
        Some(name)
    }

    fn save_as(&mut self, name: &str) -> Result<String, String> {
        let path = self.document_path(name);
        self.save_to(&path)?;
        Ok(format!("saved as {}", self.doc.title()))
    }

    fn vault(&self) -> PathBuf {
        self.opts.vault.clone().unwrap_or_else(storage::vault_dir)
    }

    /// First free `untitled[-n].md` in the vault.
    fn free_untitled_path(&self) -> PathBuf {
        storage::free_untitled(&self.vault())
    }

    /// Resolve a user-supplied name inside the vault, adding `.md` when needed.
    fn document_path(&self, name: &str) -> PathBuf {
        let raw = PathBuf::from(name);
        let path = if raw.is_absolute() {
            raw
        } else {
            self.vault().join(raw)
        };
        if path.extension().is_some() {
            path
        } else {
            path.with_extension("md")
        }
    }

    /// Find an existing document by name, with or without the extension.
    fn find_document(&self, name: &str) -> Option<PathBuf> {
        let with_ext = self.document_path(name);
        if with_ext.is_file() {
            return Some(with_ext);
        }
        let bare = self.vault().join(name);
        bare.is_file().then_some(bare)
    }

    fn open_document(&mut self, path: PathBuf) -> Result<(), String> {
        self.discard_scratch();
        let text =
            storage::load(&path).map_err(|e| format!("cannot open {}: {e}", path.display()))?;
        self.doc = Document {
            path: Some(path.clone()),
            read_only: false,
            title_hint: None,
            editor: Editor::from_text(&text),
            last_edit: Instant::now(),
        };
        self.has_document = true;
        self.dirty = true;
        Ok(())
    }

    fn rename_current(&mut self, name: &str) -> Result<String, String> {
        let target = self.document_path(name);
        match self.doc.path.clone().filter(|p| p.is_file()) {
            Some(old) => {
                std::fs::rename(&old, &target).map_err(|e| format!("rename failed: {e}"))?;
                // A name of its own means it is no longer a placeholder.
                self.scratch = None;
                self.doc.path = Some(target.clone());
                self.dirty = true;
                Ok(format!("renamed to {}", self.doc.title()))
            }
            // Not on disk yet: fall back to a plain save under the new name.
            None => self.save_as(name),
        }
    }

    fn delete_current(&mut self) -> Result<String, String> {
        let Some(path) = self.doc.path.clone().filter(|p| p.is_file()) else {
            return Err("nothing to delete".into());
        };
        std::fs::remove_file(&path).map_err(|e| format!("delete failed: {e}"))?;
        if self.scratch.as_deref() == Some(path.as_path()) {
            self.scratch = None;
        }
        self.doc = Document {
            path: None,
            read_only: false,
            title_hint: None,
            editor: Editor::new(),
            last_edit: Instant::now(),
        };
        self.has_document = false;
        self.dirty = true;
        Ok(format!(
            "deleted {}",
            path.file_name()
                .map(|n| n.to_string_lossy())
                .unwrap_or_default()
        ))
    }

    // ── palette ─────────────────────────────────────────────────────────────

    fn open_palette(&mut self) {
        self.palette = Palette::empty();
        self.refresh_palette();
        self.mode = Mode::Palette;
        self.dirty = true;
    }

    fn refresh_palette(&mut self) {
        let dir = self.vault();
        let mut entries = vec![
            ("+ new document".to_string(), Action::NewFile),
            ("+ sync now".to_string(), Action::SyncNow),
        ];
        match storage::list_documents(&dir) {
            Ok(files) => {
                for path in files {
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    entries.push((name, Action::Open(path)));
                }
            }
            Err(e) => self.set_status(format!("cannot list {}: {e}", dir.display())),
        }
        self.palette.entries = entries;
        self.palette.refilter();
    }

    fn palette_target(&self) -> Option<PathBuf> {
        match self.palette.current()? {
            Action::Open(path) => Some(path.clone()),
            _ => None,
        }
    }

    fn doc_title_of(&self, path: &Path) -> String {
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    fn palette_delete(&mut self, path: PathBuf) {
        let name = self.doc_title_of(&path);
        if let Err(e) = std::fs::remove_file(&path) {
            self.set_status(format!("delete failed: {e}"));
            return;
        }
        if self.doc.path.as_deref() == Some(path.as_path()) {
            self.doc = Document {
                path: None,
                read_only: false,
                title_hint: None,
                editor: Editor::new(),
                last_edit: Instant::now(),
            };
            self.has_document = false;
        }
        self.set_status(format!("deleted {name}"));
        self.refresh_palette();
    }

    fn palette_rename(&mut self, old: PathBuf, new: &str) {
        let new = new.trim();
        if new.is_empty() {
            self.set_status("name cannot be empty");
            return;
        }
        let target = self.document_path(new);
        if target.exists() {
            self.set_status(format!("{} already exists", self.doc_title_of(&target)));
            return;
        }
        if let Err(e) = std::fs::rename(&old, &target) {
            self.set_status(format!("rename failed: {e}"));
            return;
        }
        if self.doc.path.as_deref() == Some(old.as_path()) {
            self.doc.path = Some(target.clone());
        }
        self.set_status(format!("renamed to {}", self.doc_title_of(&target)));
        self.refresh_palette();
    }

    fn palette_accept(&mut self) {
        let Some(action) = self.palette.current().cloned() else {
            return;
        };
        match action {
            Action::Open(path) => self.start_loading(Pending::Document(path)),
            Action::OpenPicture(path) => self.start_loading(Pending::Picture(path)),
            Action::NewFile => self.start_loading(Pending::NewDocument),
            Action::SyncNow => {
                if self.sync_cfg.enabled && self.sync.is_some() {
                    self.request_sync();
                    self.set_status("sync requested");
                } else {
                    self.set_status("webdav is off - try 'settings'");
                }
            }
        }
        // Loading and the browser are their own destinations; anything else
        // drops back to the document or the home screen.
        if !matches!(self.mode, Mode::Loading | Mode::DiskBrowser) {
            self.to_write();
        }
    }

    // ── input ───────────────────────────────────────────────────────────────

    fn on_key(&mut self, el: &ActiveEventLoop, event: KeyEvent) {
        if event.state != ElementState::Pressed {
            return;
        }
        if self.boot.take().is_some() {
            // Impatient: skip straight to the picture, and swallow the key.
            self.next_frame = Instant::now();
            self.dirty = true;
            return;
        }
        match self.mode {
            Mode::Write => self.on_key_write(el, event),
            Mode::Search => self.on_key_search(event),
            Mode::View => self.on_key_view(event),
            Mode::DiskBrowser => self.on_key_disk_browser(event),
            Mode::Loading => self.finish_loading(),
            Mode::Program => self.on_key_program(el, event),
            Mode::Action => self.on_key_action(el, event),
            Mode::Palette => self.on_key_palette(event),
            Mode::Settings => self.on_key_settings(event),
            Mode::Help => self.on_key_help(event),
        }

        // Single choke point: whatever the key changed, draw it now. Forgetting
        // this in one handler makes that movement feel like it lags a beat
        // behind the key repeat, because the next frame would otherwise wait
        // for the cursor blink.
        if self.dirty {
            self.next_frame = Instant::now();
        }
    }

    /// Write mode: the document is live, editing keys only.
    fn on_key_write(&mut self, _el: &ActiveEventLoop, event: KeyEvent) {
        self.follow_cursor = true;
        let ctrl = self.mods.control_key() || self.mods.super_key();
        let shift = self.mods.shift_key();
        let code = match event.physical_key {
            PhysicalKey::Code(code) => Some(code),
            PhysicalKey::Unidentified(_) => None,
        };

        if ctrl {
            // Undo is an editing gesture, not a command, so it keeps its key.
            match code {
                Some(KeyCode::KeyF) => self.start_search(None),
                Some(KeyCode::KeyA) => {
                    self.doc.editor.select_all();
                    self.poke();
                }
                Some(KeyCode::KeyC) => {
                    let result = self.copy_selection();
                    self.report(result);
                }
                Some(KeyCode::KeyX) => {
                    let result = self.cut_selection();
                    self.report(result);
                }
                Some(KeyCode::KeyV) => {
                    let result = self.paste();
                    self.report(result);
                }
                Some(KeyCode::KeyZ) => {
                    if self.mods.shift_key() {
                        self.doc.editor.redo();
                    } else {
                        self.doc.editor.undo();
                    }
                    self.touch();
                }
                Some(KeyCode::KeyY) => {
                    self.doc.editor.redo();
                    self.touch();
                }
                Some(KeyCode::ArrowLeft) => {
                    self.doc.editor.move_word_left(shift);
                    self.poke();
                }
                Some(KeyCode::ArrowRight) => {
                    self.doc.editor.move_word_right(shift);
                    self.poke();
                }
                Some(KeyCode::Home) => {
                    self.doc.editor.move_doc_start(shift);
                    self.poke();
                }
                Some(KeyCode::End) => {
                    self.doc.editor.move_doc_end(shift);
                    self.poke();
                }
                _ => {}
            }
            return;
        }

        match &event.logical_key {
            Key::Named(NamedKey::Escape) => self.to_action(),
            Key::Named(NamedKey::ArrowLeft) => {
                self.doc.editor.move_left(shift);
                self.doc.editor.break_undo_group();
                self.poke();
            }
            Key::Named(NamedKey::ArrowRight) => {
                self.doc.editor.move_right(shift);
                self.doc.editor.break_undo_group();
                self.poke();
            }
            Key::Named(NamedKey::F3) => self.repeat_search(!shift),
            Key::Named(NamedKey::ArrowUp) => self.move_vertical(-1, shift),
            Key::Named(NamedKey::ArrowDown) => self.move_vertical(1, shift),
            Key::Named(NamedKey::Home) => {
                self.doc.editor.move_home(shift);
                self.poke();
            }
            Key::Named(NamedKey::End) => {
                self.doc.editor.move_end(shift);
                self.poke();
            }
            Key::Named(NamedKey::PageUp) => {
                let h = ui::text_height(self.screen.rows()) as isize;
                self.move_vertical(-h, shift);
            }
            Key::Named(NamedKey::PageDown) => {
                let h = ui::text_height(self.screen.rows()) as isize;
                self.move_vertical(h, shift);
            }
            Key::Named(NamedKey::Backspace) => {
                self.doc.editor.backspace();
                self.touch();
            }
            Key::Named(NamedKey::Delete) => {
                self.doc.editor.delete_forward();
                self.touch();
            }
            Key::Named(NamedKey::Enter) => {
                self.doc.editor.insert_newline();
                self.touch();
            }
            Key::Named(NamedKey::Tab) => {
                self.doc.editor.insert_tab();
                self.touch();
            }
            Key::Named(NamedKey::Space) => {
                self.doc.editor.insert_char(' ');
                self.touch();
            }
            Key::Character(text) => {
                if self.mods.alt_key() {
                    return;
                }
                if text.chars().any(|c| !c.is_control()) {
                    self.doc.editor.insert_text(text);
                    self.touch();
                }
            }
            _ => {}
        }
    }

    /// Action mode: the document is frozen, keys go to the command line.
    fn on_key_action(&mut self, el: &ActiveEventLoop, event: KeyEvent) {
        if let Some(question) = self.confirm.clone() {
            let _ = question;
            if matches!(&event.logical_key, Key::Character(c) if c.eq_ignore_ascii_case("y")) {
                self.confirm = None;
                match self.delete_current() {
                    Ok(msg) => {
                        self.set_status(msg);
                        self.to_write();
                    }
                    Err(e) => self.command_error = e,
                }
            } else {
                self.confirm = None;
                self.set_status("cancelled");
                self.to_write();
            }
            self.poke();
            return;
        }

        match &event.logical_key {
            Key::Named(NamedKey::Escape) => {
                if !self.command.is_empty() {
                    self.command.clear();
                    self.command_error.clear();
                } else if !self.has_document && self.home != Home::Boot {
                    self.home = Home::Boot;
                    self.disks_dirty = true;
                } else if self.has_document {
                    self.to_write();
                }
                self.poke();
            }
            Key::Named(NamedKey::Enter) => self.submit_command(el),
            Key::Named(NamedKey::Tab) => {
                if let Some(completion) = command::complete(&self.command) {
                    self.command = completion.completed;
                }
                self.poke();
            }
            Key::Named(NamedKey::ArrowUp) => {
                if self.home == Home::Boot && !self.has_document && self.command.is_empty() {
                    self.boot_step(-1);
                } else {
                    self.history_step(-1);
                }
                self.poke();
            }
            Key::Named(NamedKey::ArrowDown) => {
                if self.home == Home::Boot && !self.has_document && self.command.is_empty() {
                    self.boot_step(1);
                } else {
                    self.history_step(1);
                }
                self.poke();
            }
            Key::Named(NamedKey::Backspace) => {
                self.command.pop();
                self.command_error.clear();
                self.poke();
            }
            Key::Named(NamedKey::Space) => {
                self.command.push(' ');
                self.command_error.clear();
                self.poke();
            }
            Key::Character(text) => {
                if self.mods.alt_key() {
                    return;
                }
                for ch in text.chars().filter(|c| !c.is_control()) {
                    self.command.push(ch);
                }
                self.command_error.clear();
                self.poke();
            }
            _ => {}
        }
    }

    /// Which command list `help` shows.
    fn help_scope(&self) -> command::Scope {
        if self.program.is_some() || self.home == Home::SoftHand {
            command::Scope::Program
        } else if self.has_document || self.home == Home::Inspiro {
            command::Scope::Editor
        } else {
            command::Scope::Boot
        }
    }

    /// Help: arrows walk the list, any other key goes back.
    fn on_key_help(&mut self, event: KeyEvent) {
        let specs = if self.help_all {
            command::SPECS.len()
        } else {
            command::specs_for(self.help_scope()).len()
        };
        match &event.logical_key {
            Key::Named(NamedKey::ArrowUp) if !self.help_about => {
                self.help_selected = self.help_selected.saturating_sub(1);
                self.poke();
            }
            Key::Named(NamedKey::ArrowDown) if !self.help_about => {
                self.help_selected = (self.help_selected + 1).min(specs.saturating_sub(1));
                self.poke();
            }
            _ => self.to_write(),
        }
    }

    /// A program's own keys: Esc opens the command line, everything else is
    /// the program's gesture.
    fn on_key_program(&mut self, el: &ActiveEventLoop, event: KeyEvent) {
        if self.program_prompt {
            // An empty Esc closes the prompt; otherwise this is the ordinary
            // command line, history and completion included.
            if matches!(&event.logical_key, Key::Named(NamedKey::Escape)) && self.command.is_empty()
            {
                self.program_prompt = false;
                self.poke();
                return;
            }
            self.on_key_action(el, event);
            return;
        }
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => {
                self.program_prompt = true;
                self.command.clear();
                self.command_error.clear();
                self.poke();
            }
            _ => {
                let Some(key) = self.program_key(&event) else {
                    return;
                };
                let mut done = false;
                if let Some(program) = self.program.as_mut() {
                    done = program.key(key) == Outcome::Exit;
                }
                if done {
                    self.leave_to_boot();
                }
                self.poke();
            }
        }
    }

    /// Translate a winit key into the program's vocabulary.
    fn program_key(&self, event: &KeyEvent) -> Option<KeyPress> {
        let ctrl = self.mods.control_key() || self.mods.super_key();
        Some(match &event.logical_key {
            Key::Character(text) => {
                let ch = text.chars().next()?;
                if ctrl {
                    KeyPress::Ctrl(ch.to_ascii_lowercase())
                } else {
                    KeyPress::Char(ch)
                }
            }
            Key::Named(NamedKey::Enter) => KeyPress::Enter,
            Key::Named(NamedKey::Backspace) => KeyPress::Backspace,
            Key::Named(NamedKey::Delete) => KeyPress::Delete,
            Key::Named(NamedKey::Tab) => KeyPress::Tab,
            Key::Named(NamedKey::ArrowLeft) => KeyPress::Left,
            Key::Named(NamedKey::ArrowRight) => KeyPress::Right,
            Key::Named(NamedKey::ArrowUp) => KeyPress::Up,
            Key::Named(NamedKey::ArrowDown) => KeyPress::Down,
            Key::Named(NamedKey::Home) => KeyPress::Home,
            Key::Named(NamedKey::End) => KeyPress::End,
            Key::Named(NamedKey::PageUp) => KeyPress::PageUp,
            Key::Named(NamedKey::PageDown) => KeyPress::PageDown,
            _ => return None,
        })
    }

    /// Pointer position in emulated raster pixels.
    fn raster_point(&self) -> (i32, i32) {
        match self.vp.as_ref() {
            Some(vp) => {
                let scale = vp.scale.max(1) as f64;
                (
                    ((self.pointer.0 - vp.offset_x as f64) / scale) as i32,
                    ((self.pointer.1 - vp.offset_y as f64) / scale) as i32,
                )
            }
            None => (self.pointer.0 as i32, self.pointer.1 as i32),
        }
    }

    /// Hand a pointer event to the running program, if it should get one.
    fn program_pointer(&mut self, kind: PointerKind) -> bool {
        if self.mode != Mode::Program || self.program_prompt {
            return false;
        }
        let (x, y) = self.raster_point();
        if matches!(kind, PointerKind::Move) && (x, y) == self.program_point {
            // Same emulated pixel: nothing on screen would change.
            return true;
        }
        self.program_point = (x, y);
        if let Some(program) = self.program.as_mut() {
            if program.pointer(PointerEvent { x, y, kind }) == Outcome::Exit {
                self.leave_to_boot();
            }
            self.poke();
        }
        true
    }

    fn submit_command(&mut self, el: &ActiveEventLoop) {
        let line = self.command.trim().to_string();
        let command = match command::parse(&line) {
            Ok(command) => command,
            Err(CommandError::Empty) => {
                if self.has_document {
                    self.to_write();
                } else {
                    self.boot_activate();
                }
                return;
            }
            Err(e) => {
                self.command_error = e.message();
                self.poke();
                return;
            }
        };

        if self.history.last() != Some(&line) {
            self.history.push(line);
            if self.history.len() > 200 {
                self.history.remove(0);
            }
        }
        self.history_pos = None;

        match self.run_command(el, command) {
            Ok(message) => {
                if !message.is_empty() {
                    self.set_status(message);
                }
                self.command.clear();
                self.command_error.clear();
                // Commands that opened another screen keep it.
                if self.mode == Mode::Action && self.confirm.is_none() {
                    self.to_write();
                }
                // A program's prompt closes again after a command, so you are
                // back to drawing without another keypress.
                if self.mode == Mode::Program {
                    self.program_prompt = false;
                }
            }
            Err(e) => {
                // Keep the typed line so it can be corrected.
                self.command_error = e;
            }
        }
        self.poke();
    }

    fn run_command(&mut self, el: &ActiveEventLoop, command: Command) -> Result<String, String> {
        match command {
            Command::Save => {
                if self.in_softhand() {
                    self.save_picture(None)
                } else {
                    self.save()
                }
            }
            Command::SaveAs(name) => {
                if self.in_softhand() {
                    self.save_picture(Some(&name))
                } else {
                    self.save_as(&name)
                }
            }
            Command::Open(None) => {
                if self.in_softhand() {
                    self.open_pictures();
                } else {
                    self.open_palette();
                }
                Ok(String::new())
            }
            Command::Open(Some(name)) => {
                if self.in_softhand() {
                    let path = picture::resolve(&self.vault(), &name);
                    if !path.is_file() {
                        return Err(format!("no such picture: {name}"));
                    }
                    self.start_loading(Pending::Picture(path));
                    return Ok(String::new());
                }
                let path = self
                    .find_document(&name)
                    .ok_or_else(|| format!("no such document: {name}"))?;
                self.start_loading(Pending::Document(path));
                Ok(String::new())
            }
            Command::New => {
                if self.in_softhand() {
                    self.start_loading(Pending::NewPicture);
                } else {
                    self.start_loading(Pending::NewDocument);
                }
                Ok(String::new())
            }
            Command::Rename(name) => self.rename_current(&name),
            Command::Delete => {
                if self.doc.path.is_none() {
                    return Err("nothing to delete".into());
                }
                self.confirm = Some(format!("delete {}? y/n", self.doc.title()));
                Ok(String::new())
            }
            Command::Sync => {
                if !self.sync_cfg.enabled || self.sync.is_none() {
                    return Err("webdav is off - try 'settings'".into());
                }
                self.request_sync();
                Ok("sync requested".into())
            }
            Command::Theme(None) => self.cycle_theme(1),
            Command::Theme(Some(arg)) if arg == "+" => self.cycle_theme(1),
            Command::Theme(Some(arg)) if arg == "-" => self.cycle_theme(-1),
            Command::Theme(Some(name)) => {
                let theme = Theme::by_name(&name).ok_or_else(|| {
                    let names: Vec<&str> = Theme::ALL.iter().map(|t| t.name).collect();
                    format!("unknown theme: {name} ({})", names.join(", "))
                })?;
                self.theme = theme;
                self.save_prefs();
                self.dirty = true;
                Ok(format!("theme: {}", theme.name))
            }
            Command::Scanlines { on, roll } => {
                // Bare `scanlines` switches them on rather than toggling: it is
                // the everyday "give me the CRT" gesture.
                let enable = on.unwrap_or(true) || roll;
                self.crt.scanline = if enable {
                    CrtSettings::default().scanline
                } else {
                    0.0
                };
                if roll {
                    self.crt.roll = ROLL_SPEED;
                }
                self.save_prefs();
                self.dirty = true;
                Ok(format!(
                    "scanlines: {}{}",
                    on_off(enable),
                    if roll { ", roll on" } else { "" }
                ))
            }
            Command::Bloom(want) => {
                let on = apply_switch(&mut self.crt.bloom, CrtSettings::default().bloom, want);
                self.save_prefs();
                self.dirty = true;
                Ok(format!("bloom: {}", on_off(on)))
            }
            Command::Roll(want) => {
                let on = want.unwrap_or(self.crt.roll <= 0.0);
                self.crt.roll = if on { ROLL_SPEED } else { 0.0 };
                if on && self.crt.scanline <= 0.0 {
                    // Drifting with no lines to drift is a no-op; turn them on.
                    self.crt.scanline = CrtSettings::default().scanline;
                }
                self.save_prefs();
                self.dirty = true;
                Ok(format!("scanline roll: {}", on_off(on)))
            }
            Command::Beep(want) => {
                self.beep_on = want.unwrap_or(!self.beep_on);
                self.prefs.beep = self.beep_on;
                self.save_prefs();
                self.beep();
                Ok(format!("beep: {}", on_off(self.beep_on)))
            }
            Command::Typewriter(want) => {
                self.typewriter = want.unwrap_or(!self.typewriter);
                self.save_prefs();
                self.dirty = true;
                Ok(format!("typewriter: {}", on_off(self.typewriter)))
            }
            Command::Font(want) => {
                let next = match want.as_deref() {
                    Some(name) => Face::by_name(name)
                        .ok_or_else(|| format!("unknown font: {name} (classic, thin)"))?,
                    None => match self.font.face() {
                        Face::Classic => Face::Thin,
                        Face::Thin => Face::Classic,
                    },
                };
                self.font = Font::with_face(next);
                self.prefs.font = next.name().to_string();
                self.save_prefs();
                // The cell height feeds the scale heuristic and the grid size.
                self.recompute_viewport();
                self.dirty = true;
                Ok(format!("font: {}", next.name()))
            }
            Command::Fullscreen(on) => {
                self.set_fullscreen(on);
                Ok(if on { "fullscreen" } else { "windowed" }.to_string())
            }
            Command::Undo => {
                let done = match self.program.as_mut() {
                    Some(program) => program.undo(),
                    None => self.doc.editor.undo(),
                };
                if done {
                    self.dirty = true;
                    Ok("undone".into())
                } else {
                    Err("nothing to undo".into())
                }
            }
            Command::Redo => {
                let done = match self.program.as_mut() {
                    Some(program) => program.redo(),
                    None => self.doc.editor.redo(),
                };
                if done {
                    self.dirty = true;
                    Ok("redone".into())
                } else {
                    Err("nothing to redo".into())
                }
            }
            Command::View => {
                if self.mode == Mode::View {
                    self.to_write();
                } else {
                    self.enter_view();
                }
                Ok(String::new())
            }
            Command::Find(query) => {
                self.start_search(query);
                Ok(String::new())
            }
            Command::Run(None) => {
                self.boot_activate();
                Ok(String::new())
            }
            Command::Run(Some(name)) => self.run_named(&name),
            Command::Dir => {
                self.disks_dirty = true;
                self.refresh_disks();
                let names: Vec<String> = self
                    .disks
                    .iter()
                    .map(|e| match &e.disk {
                        Some(d) => d.name.clone(),
                        None => format!("{} (bad)", e.stem),
                    })
                    .collect();
                Ok(if names.is_empty() {
                    format!("no disks in {}", self.disk_dir().display())
                } else {
                    format!("{} disk(s): {}", names.len(), names.join(", "))
                })
            }
            Command::Exit => {
                if self.has_document && self.doc.editor.modified() {
                    self.save()?;
                }
                // On the boot screen `exit` means what it means everywhere else:
                // stop the program. That is CRead itself.
                if !self.has_document && self.program.is_none() && self.home == Home::Boot {
                    el.exit();
                    return Ok(String::new());
                }
                self.leave_to_boot();
                Ok(String::new())
            }
            Command::Settings => {
                self.open_settings();
                Ok(String::new())
            }
            Command::Help(argument) => {
                self.help_about = false;
                self.help_all = argument
                    .as_deref()
                    .is_some_and(|a| a.eq_ignore_ascii_case("all"));
                self.help_selected = 0;
                self.mode = Mode::Help;
                self.dirty = true;
                Ok(String::new())
            }
            Command::Tool(_) | Command::Size(_) | Command::Colour(_) => {
                let Some(program) = self.program.as_mut() else {
                    return Err("only a program has tools".into());
                };
                program.command(&command)
            }
            Command::About => {
                self.help_about = true;
                self.mode = Mode::Help;
                self.dirty = true;
                Ok(String::new())
            }
            Command::ExitForce => {
                if self.has_document {
                    self.save()?;
                }
                // A picture that was never saved keeps no file behind: quitting
                // must not silently create one.
                if self.program.is_some() && self.picture_path.is_some() {
                    let _ = self.save_picture(None);
                }
                el.exit();
                Ok(String::new())
            }
        }
    }

    #[allow(clippy::wrong_self_convention)]
    fn to_action(&mut self) {
        self.mode = Mode::Action;
        self.command.clear();
        self.command_error.clear();
        self.history_pos = None;
        self.confirm = None;
        self.dirty = true;
    }

    #[allow(clippy::wrong_self_convention)]
    fn to_write(&mut self) {
        if self.program.is_some() {
            self.mode = Mode::Program;
            self.program_prompt = false;
            self.command.clear();
            self.command_error.clear();
            self.history_pos = None;
            self.confirm = None;
            self.dirty = true;
            return;
        }
        if self.doc.read_only {
            self.doc = Document {
                path: None,
                read_only: false,
                title_hint: None,
                editor: Editor::new(),
                last_edit: Instant::now(),
            };
            self.has_document = false;
            self.mode = Mode::DiskBrowser;
            self.command.clear();
            self.dirty = true;
            return;
        }
        self.mode = if self.has_document {
            Mode::Write
        } else {
            Mode::Action
        };
        self.command.clear();
        self.command_error.clear();
        self.history_pos = None;
        self.confirm = None;
        self.dirty = true;
    }

    fn history_step(&mut self, delta: isize) {
        if self.history.is_empty() {
            return;
        }
        let last = self.history.len() as isize - 1;
        let next = match self.history_pos {
            None => {
                if delta < 0 {
                    last
                } else {
                    return;
                }
            }
            Some(pos) => pos as isize + delta,
        };
        if next < 0 {
            self.history_pos = Some(0);
        } else if next > last {
            self.history_pos = None;
            self.command.clear();
            return;
        } else {
            self.history_pos = Some(next as usize);
        }
        if let Some(pos) = self.history_pos {
            self.command = self.history[pos].clone();
        }
    }

    // ── clipboard ───────────────────────────────────────────────────────────

    /// Persist the bits of UI state that should survive a restart.
    fn save_prefs(&mut self) {
        self.prefs.set(self.theme);
        self.prefs.font = self.font.face().name().to_string();
        self.prefs.typewriter = self.typewriter;
        self.prefs.scanlines = self.crt.scanline > 0.0;
        self.prefs.bloom = self.crt.bloom > 0.0;
        self.prefs.roll = self.crt.roll > 0.0;
        if let Err(e) = self.prefs.save() {
            eprintln!("cread: cannot save preferences: {e}");
        }
    }

    // ── mouse ───────────────────────────────────────────────────────────────

    /// Map a physical pixel to a document position, or `None` outside the text.
    fn position_at(&self, px: f64, py: f64) -> Option<Position> {
        let vp = self.vp?;
        let scale = vp.scale as f64;
        let vx = (px - vp.offset_x as f64) / scale;
        let vy = (py - vp.offset_y as f64) / scale;
        if vx < 0.0 || vy < 0.0 {
            return None;
        }

        let (text_width, margin) = ui::layout(self.screen.cols());
        let height = ui::text_height(self.screen.rows());

        // Two rows of chrome above the text.
        let row = (vy / self.font.cell_h() as f64) as usize;
        let row = row.checked_sub(2)?;
        if row >= height {
            return None;
        }
        // Clicking in the margin clamps to the start or end of the row instead
        // of doing nothing.
        let col = (vx / self.font.cell_w() as f64) as usize;
        let col_in_row = col.saturating_sub(margin).min(text_width);

        let rows = self.doc.editor.visible_rows(text_width, height);
        let visual = rows.get(row)?;
        let width = visual.text.chars().count();
        let doc_col = visual.start + col_in_row.min(width);
        Some(Position {
            line: visual.line,
            col: doc_col,
        })
    }

    fn on_mouse_press(&mut self) {
        if self.mode != Mode::Write {
            return;
        }
        let (px, py) = self.pointer;
        if let Some(pos) = self.position_at(px, py) {
            self.doc.editor.begin_selection(pos);
            self.dragging = true;
            self.follow_cursor = true;
            self.poke();
        }
    }

    fn on_mouse_release(&mut self) {
        self.dragging = false;
    }

    fn on_mouse_drag(&mut self) {
        if !self.dragging || self.mode != Mode::Write {
            return;
        }
        let (px, py) = self.pointer;
        // Dragging past the edges just clamps to the last computed position.
        if let Some(pos) = self.position_at(px, py) {
            self.doc.editor.drag_to(pos);
            self.poke();
        }
    }

    fn on_wheel(&mut self, lines: isize) {
        if self.mode == Mode::View {
            self.scroll_view(-lines);
            return;
        }
        if self.mode != Mode::Write {
            return;
        }
        self.doc.editor.scroll_lines(lines);
        self.follow_cursor = false;
        self.poke();
    }

    fn clipboard(&mut self) -> Result<&mut arboard::Clipboard, String> {
        if self.clipboard.is_none() {
            self.clipboard =
                Some(arboard::Clipboard::new().map_err(|e| format!("clipboard: {e}"))?);
        }
        Ok(self.clipboard.as_mut().expect("just created"))
    }

    fn copy_selection(&mut self) -> Result<String, String> {
        let text = self.doc.editor.selected_text();
        if text.is_empty() {
            return Err("nothing selected".into());
        }
        self.clipboard()?
            .set_text(text.clone())
            .map_err(|e| format!("clipboard: {e}"))?;
        Ok(format!("{} chars copied", text.chars().count()))
    }

    fn cut_selection(&mut self) -> Result<String, String> {
        let text = self.doc.editor.selected_text();
        if text.is_empty() {
            return Err("nothing selected".into());
        }
        self.clipboard()?
            .set_text(text.clone())
            .map_err(|e| format!("clipboard: {e}"))?;
        if let Some((start, end)) = self.doc.editor.selection() {
            self.doc.editor.delete_range(start, end);
        }
        self.touch();
        Ok(format!("{} chars cut", text.chars().count()))
    }

    fn paste(&mut self) -> Result<String, String> {
        let text = {
            let clipboard = self.clipboard()?;
            clipboard
                .get_text()
                .map_err(|e| format!("clipboard: {e}"))?
        };
        if text.is_empty() {
            return Err("clipboard is empty".into());
        }
        self.doc.editor.insert_text(&text);
        self.touch();
        Ok(format!("{} chars pasted", text.chars().count()))
    }

    /// Show a command result, or its error, on the status line.
    fn report(&mut self, result: Result<String, String>) {
        match result {
            Ok(message) => {
                self.set_status(message);
                self.dirty = true;
            }
            Err(e) => {
                self.set_status(e);
                self.dirty = true;
            }
        }
    }

    /// Show a short message on the status line for a few seconds.
    fn set_status(&mut self, message: impl Into<String>) {
        self.status = message.into();
        self.status_at = Instant::now();
        self.poke();
    }

    /// Cheap "something changed, redraw soon" for non-document keys.
    /// One PC speaker beep, if the preference is on.
    fn beep(&mut self) {
        if self.beep_on {
            crate::beep::play();
        }
    }

    fn poke(&mut self) {
        // Render as soon as the event loop comes back around.
        self.next_frame = Instant::now();
        self.dirty = true;
    }

    fn on_key_search(&mut self, event: KeyEvent) {
        let shift = self.mods.shift_key();
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => self.cancel_search(),
            Key::Named(NamedKey::Enter) => self.accept_search(),
            Key::Named(NamedKey::F3) => self.step_match(!shift),
            Key::Named(NamedKey::Backspace) => {
                if let Some(search) = self.search.as_mut() {
                    search.query.pop();
                }
                self.update_search();
            }
            Key::Named(NamedKey::Space) => {
                if let Some(search) = self.search.as_mut() {
                    search.query.push(' ');
                }
                self.update_search();
            }
            Key::Character(text) => {
                if self.mods.alt_key() {
                    return;
                }
                if let Some(search) = self.search.as_mut() {
                    for ch in text.chars().filter(|c| !c.is_control()) {
                        search.query.push(ch);
                    }
                }
                self.update_search();
            }
            _ => {}
        }
    }

    // ── loading ─────────────────────────────────────────────────────────────

    /// Show the fake progress bar, then do `next`.
    fn start_loading(&mut self, next: Pending) {
        self.pending = Some(next);
        self.loading_started = Instant::now();
        self.mode = Mode::Loading;
        self.dirty = true;
    }

    fn finish_loading(&mut self) {
        let Some(next) = self.pending.take() else {
            self.mode = Mode::Action;
            return;
        };
        match next {
            Pending::Inspiro => {
                self.home = Home::Inspiro;
                self.mode = Mode::Action;
                self.command.clear();
            }
            Pending::Program(index) => {
                self.home = program_home(index);
                self.mode = Mode::Action;
                self.command.clear();
            }
            Pending::NewPicture => {
                self.new_picture();
            }
            Pending::Picture(path) => match self.open_picture(path) {
                Ok(()) => {}
                Err(e) => {
                    self.set_status(e);
                    self.home = Home::SoftHand;
                    self.mode = Mode::Action;
                }
            },
            Pending::Document(path) => match self.open_document(path) {
                Ok(()) => self.mode = Mode::Write,
                Err(e) => {
                    self.set_status(e);
                    self.mode = Mode::Action;
                }
            },
            Pending::NewDocument => match self.new_document() {
                Ok(msg) => {
                    if !msg.is_empty() {
                        self.set_status(msg);
                    }
                    self.mode = Mode::Write;
                }
                Err(e) => {
                    self.set_status(e);
                    self.mode = Mode::Action;
                }
            },
        }
        self.dirty = true;
    }

    // ── programs ────────────────────────────────────────────────────────────

    /// Drop everything and stand on the boot screen again.
    fn leave_to_boot(&mut self) {
        self.program = None;
        self.picture_path = None;
        self.program_prompt = false;
        self.program_point = (-1, -1);
        self.discard_scratch();
        self.doc = Document {
            path: None,
            read_only: false,
            title_hint: None,
            editor: Editor::new(),
            last_edit: Instant::now(),
        };
        self.has_document = false;
        self.home = Home::Boot;
        self.disks_dirty = true;
        self.mode = Mode::Action;
        self.command.clear();
        self.command_error.clear();
        self.dirty = true;
    }

    /// A fresh canvas on an empty sheet.
    fn new_picture(&mut self) {
        self.program = Some(Box::new(Softhand::new()));
        self.picture_path = None;
        self.program_prompt = false;
        self.mode = Mode::Program;
        self.dirty = true;
    }

    fn open_picture(&mut self, path: PathBuf) -> Result<(), String> {
        let image = picture::load(&path)?;
        self.program = Some(Box::new(Softhand::with_picture(&image)));
        self.picture_path = Some(path.clone());
        self.program_prompt = false;
        self.mode = Mode::Program;
        self.dirty = true;
        self.set_status(format!(
            "opened {}",
            path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        ));
        Ok(())
    }

    /// Write the running program's picture. `None` keeps the current name, or
    /// picks the next free one.
    fn save_picture(&mut self, name: Option<&str>) -> Result<String, String> {
        let vault = self.vault();
        let path = match name {
            Some(name) => picture::resolve(&vault, name),
            None => match self.picture_path.clone() {
                Some(path) => path,
                None => picture::free_name(&vault),
            },
        };
        let Some(program) = self.program.as_ref() else {
            return Err("no picture open".into());
        };
        let Some(canvas) = program.picture() else {
            return Err("this program has no picture".into());
        };
        picture::save(&path, canvas)?;
        self.picture_path = Some(path.clone());
        Ok(format!(
            "saved {}",
            path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        ))
    }

    /// The picture picker: same list as the document palette, other files.
    fn open_pictures(&mut self) {
        self.palette = Palette::empty();
        self.palette.title = "PICTURES".into();
        self.palette.pictures = true;
        match picture::list(&self.vault()) {
            Ok(files) => {
                for path in files {
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    self.palette.entries.push((name, Action::OpenPicture(path)));
                }
            }
            Err(e) => self.set_status(format!("pictures: {e}")),
        }
        if self.palette.entries.is_empty() {
            self.set_status(format!(
                "no pictures in {}",
                picture::dir(&self.vault()).display()
            ));
        }
        self.palette.refilter();
        self.mode = Mode::Palette;
        self.dirty = true;
    }

    /// True while Softhand is the thing in front: its front door or its canvas.
    fn in_softhand(&self) -> bool {
        self.program.is_some() || (!self.has_document && self.home == Home::SoftHand)
    }

    // ── disks and the boot screen ───────────────────────────────────────────

    fn disk_dir(&self) -> PathBuf {
        storage::disk_dir(&self.vault())
    }

    /// The disk folder with `~` instead of the home directory.
    fn short_disk_dir(&self) -> String {
        let dir = self.disk_dir();
        match dir.strip_prefix(storage::home_dir()) {
            Ok(rest) => format!("~/{}", rest.display()),
            Err(_) => dir.display().to_string(),
        }
    }

    fn refresh_disks(&mut self) {
        let dir = self.disk_dir();
        self.disks = match storage::list_disks(&dir) {
            Ok(entries) => entries
                .into_iter()
                .map(|(path, text)| {
                    let stem = path
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    match text
                        .map_err(|e| e.to_string())
                        .and_then(|t| Disk::parse(&t).map_err(|e| e.to_string()))
                    {
                        Ok(disk)
                            if !disk.program.is_empty()
                                && program_index_by_id(&disk.program).is_none() =>
                        {
                            DiskEntry {
                                stem,
                                disk: None,
                                error: Some(format!(
                                    "needs program {}, which is not installed",
                                    disk.program
                                )),
                            }
                        }
                        Ok(disk) => DiskEntry {
                            stem,
                            disk: Some(disk),
                            error: None,
                        },
                        Err(error) => DiskEntry {
                            stem,
                            disk: None,
                            error: Some(error),
                        },
                    }
                })
                .collect(),
            Err(e) => {
                self.set_status(format!("cannot list disks: {e}"));
                Vec::new()
            }
        };
        if self.boot_selected >= self.boot_items().len() {
            self.boot_selected = 0;
        }
    }

    fn boot_items(&self) -> Vec<BootItem> {
        let mut items = vec![BootItem::Documents, BootItem::Rule, BootItem::Inspiro];
        for (index, program) in PROGRAMS.iter().enumerate() {
            if program.installed {
                items.push(BootItem::Program(index));
            }
        }
        items.extend((0..self.disks.len()).map(BootItem::Disk));
        items
    }

    /// The boot screen list as the UI wants it.
    fn boot_rows(&self) -> Vec<ui::BootItem> {
        let mut rows = Vec::new();
        let mut letter = b'A';
        push_boot_row(
            &mut rows,
            &mut letter,
            "DOCUMENTS".into(),
            "open a file from the vault".into(),
            false,
        );
        rows.push(ui::BootItem::rule());

        push_boot_row(
            &mut rows,
            &mut letter,
            "INSPIRO".into(),
            "where your ideas have room to grow".into(),
            false,
        );
        for program in PROGRAMS.iter().filter(|program| program.installed) {
            push_boot_row(
                &mut rows,
                &mut letter,
                program.entry.into(),
                program.tagline.into(),
                false,
            );
        }
        for entry in &self.disks {
            match &entry.disk {
                Some(disk) => {
                    let detail = if disk.about.is_empty() {
                        format!("{} file(s)", disk.files.len())
                    } else {
                        disk.about.clone()
                    };
                    push_boot_row(&mut rows, &mut letter, disk.name.clone(), detail, false)
                }
                None => push_boot_row(
                    &mut rows,
                    &mut letter,
                    entry.stem.to_uppercase(),
                    entry.error.clone().unwrap_or_default(),
                    true,
                ),
            }
        }
        rows
    }

    fn boot_activate(&mut self) {
        // Only the boot screen has a selectable list; on Inspiro's front door
        // an empty Enter has nothing to run.
        if self.home != Home::Boot {
            return;
        }
        let Some(item) = self.boot_items().get(self.boot_selected).copied() else {
            return;
        };
        match item {
            BootItem::Documents => self.open_palette(),
            BootItem::Rule => {}
            BootItem::Inspiro => self.start_loading(Pending::Inspiro),
            BootItem::Program(index) => self.start_loading(Pending::Program(index)),
            BootItem::Disk(index) => self.launch_disk(index),
        }
    }

    /// Move the boot selection, stepping over rows that are not selectable
    /// (the rule). Stops at both ends instead of wrapping.
    fn boot_step(&mut self, delta: isize) {
        let items = self.boot_items();
        let mut index = self.boot_selected as isize;
        loop {
            index += delta;
            if index < 0 || index >= items.len() as isize {
                return;
            }
            if items[index as usize].selectable() {
                self.boot_selected = index as usize;
                return;
            }
        }
    }

    /// Enter on a diskette: a program disk runs the program, a data disk opens
    /// the browser. Same letter, same row, different destination.
    fn launch_disk(&mut self, index: usize) {
        self.open_disk(index);
    }

    fn open_disk(&mut self, index: usize) {
        let Some(entry) = self.disks.get(index) else {
            return;
        };
        if let Some(error) = entry.error.clone() {
            let stem = entry.stem.clone();
            self.set_status(format!("{stem}: {error}"));
            return;
        }
        self.disk_open = index;
        self.disk_selected = 0;
        self.mode = Mode::DiskBrowser;
        self.dirty = true;
    }

    /// Run a disk by name, or one of the built-ins.
    fn run_named(&mut self, name: &str) -> Result<String, String> {
        let wanted = name.trim().to_ascii_lowercase();
        if matches!(wanted.as_str(), "editor" | "a") {
            self.new_document()?;
            return Ok(String::new());
        }
        if matches!(wanted.as_str(), "documents" | "files" | "b") {
            self.open_palette();
            return Ok(String::new());
        }
        if let Some(index) = program_index_by_id(&wanted) {
            if !PROGRAMS[index].installed {
                let mounted = self.disks.iter().any(|entry| {
                    entry
                        .disk
                        .as_ref()
                        .is_some_and(|disk| disk.program == wanted)
                });
                if !mounted {
                    return Err(format!("no diskette called {wanted} - put one in Disks"));
                }
            }
            self.start_loading(Pending::Program(index));
            return Ok(String::new());
        }
        let index = self
            .disks
            .iter()
            .position(|entry| {
                entry.stem.eq_ignore_ascii_case(&wanted)
                    || entry
                        .disk
                        .as_ref()
                        .is_some_and(|d| d.name.eq_ignore_ascii_case(&wanted))
            })
            .ok_or_else(|| format!("no disk called {name}"))?;
        self.open_disk(index);
        Ok(String::new())
    }

    fn open_disk_file(&mut self) {
        let Some(entry) = self.disks.get(self.disk_open) else {
            return;
        };
        let Some(disk) = &entry.disk else {
            return;
        };
        let Some(file) = disk.files.get(self.disk_selected) else {
            return;
        };
        let title = format!("{}:{}", disk.name, file.name);
        let text = file.text.clone();

        self.discard_scratch();
        self.doc = Document {
            path: None,
            read_only: true,
            title_hint: Some(title),
            editor: Editor::from_text(&text),
            last_edit: Instant::now(),
        };
        self.has_document = true;
        self.enter_view();
    }

    fn on_key_disk_browser(&mut self, event: KeyEvent) {
        let count = self
            .disks
            .get(self.disk_open)
            .and_then(|d| d.disk.as_ref())
            .map(|d| d.files.len())
            .unwrap_or(0);
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => {
                self.mode = Mode::Action;
                self.disks_dirty = true;
                self.dirty = true;
            }
            Key::Named(NamedKey::Enter) | Key::Named(NamedKey::Space) => self.open_disk_file(),
            Key::Named(NamedKey::ArrowDown) => {
                if self.disk_selected + 1 < count {
                    self.disk_selected += 1;
                }
                self.poke();
            }
            Key::Named(NamedKey::ArrowUp) => {
                self.disk_selected = self.disk_selected.saturating_sub(1);
                self.poke();
            }
            _ => {}
        }
    }

    // ── reader mode ─────────────────────────────────────────────────────────

    fn enter_view(&mut self) {
        if !self.has_document {
            return;
        }
        let (width, _) = ui::layout(self.screen.cols());
        self.view_rows = cread_core::markdown::render(self.doc.editor.lines(), width);
        self.view_width = width;

        // Open near the line being edited rather than at the top.
        let height = ui::text_height(self.screen.rows());
        let target = self.doc.editor.cursor().line;
        let index = self
            .view_rows
            .iter()
            .position(|row| row.source_line >= target)
            .unwrap_or(0);
        self.view_top = index.saturating_sub(height / 3);
        self.mode = Mode::View;
        self.dirty = true;
    }

    fn scroll_view(&mut self, delta: isize) {
        let height = ui::text_height(self.screen.rows());
        let max = self.view_rows.len().saturating_sub(height) as isize;
        self.view_top = (self.view_top as isize + delta).clamp(0, max.max(0)) as usize;
        self.poke();
    }

    fn on_key_view(&mut self, event: KeyEvent) {
        let page = ui::text_height(self.screen.rows()) as isize;
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => self.to_write(),
            Key::Named(NamedKey::ArrowDown) => self.scroll_view(1),
            Key::Named(NamedKey::ArrowUp) => self.scroll_view(-1),
            Key::Named(NamedKey::PageDown) | Key::Named(NamedKey::Space) => {
                self.scroll_view(page.saturating_sub(1))
            }
            Key::Named(NamedKey::PageUp) => self.scroll_view(-page.saturating_sub(1)),
            Key::Named(NamedKey::Home) => {
                self.view_top = 0;
                self.poke();
            }
            Key::Named(NamedKey::End) => {
                self.scroll_view(isize::MAX / 2);
            }
            _ => {}
        }
    }

    fn start_search(&mut self, query: Option<String>) {
        if !self.has_document {
            return;
        }
        self.search = Some(Search {
            origin: self.doc.editor.cursor(),
            query: query.unwrap_or_default(),
            matches: Vec::new(),
            current: 0,
        });
        self.mode = Mode::Search;
        self.update_search();
    }

    /// Recompute the hits and jump to the first one at or after the origin.
    fn update_search(&mut self) {
        let (matches, origin) = match &self.search {
            Some(search) => (self.doc.editor.find_all(&search.query), search.origin),
            None => return,
        };
        if let Some(search) = self.search.as_mut() {
            search.current = matches
                .iter()
                .position(|(pos, _)| *pos >= origin)
                .unwrap_or(0);
            search.matches = matches;
        }
        if let Some(pos) = self
            .search
            .as_ref()
            .and_then(|s| s.matches.get(s.current))
            .map(|(pos, _)| *pos)
        {
            self.doc.editor.set_cursor(pos);
        }
        self.dirty = true;
    }

    fn step_match(&mut self, forward: bool) {
        let Some(search) = self.search.as_mut() else {
            return;
        };
        if search.matches.is_empty() {
            return;
        }
        let len = search.matches.len();
        search.current = if forward {
            (search.current + 1) % len
        } else {
            (search.current + len - 1) % len
        };
        let pos = search.matches[search.current].0;
        self.doc.editor.set_cursor(pos);
        self.dirty = true;
    }

    /// Enter: keep the cursor where the search landed.
    fn accept_search(&mut self) {
        if let Some(search) = self.search.take() {
            if !search.query.is_empty() {
                self.last_search = Some(search.query.clone());
            }
            if let Some((pos, _)) = search.matches.get(search.current) {
                self.doc.editor.set_cursor(*pos);
            }
        }
        // Loading and the browser are their own destinations; anything else
        // drops back to the document or the home screen.
        if !matches!(self.mode, Mode::Loading | Mode::DiskBrowser) {
            self.to_write();
        }
    }

    /// Esc: put the cursor back where the search started.
    fn cancel_search(&mut self) {
        if let Some(search) = self.search.take() {
            self.doc.editor.set_cursor(search.origin);
        }
        // Loading and the browser are their own destinations; anything else
        // drops back to the document or the home screen.
        if !matches!(self.mode, Mode::Loading | Mode::DiskBrowser) {
            self.to_write();
        }
    }

    /// F3 outside search: jump to the next hit of the previous query.
    fn repeat_search(&mut self, forward: bool) {
        let Some(query) = self.last_search.clone() else {
            self.set_status("no previous search");
            return;
        };
        let matches = self.doc.editor.find_all(&query);
        if matches.is_empty() {
            self.set_status(format!("no matches for {query}"));
            return;
        }
        let cursor = self.doc.editor.cursor();
        let index = if forward {
            matches
                .iter()
                .position(|(pos, _)| *pos > cursor)
                .unwrap_or(0)
        } else {
            matches
                .iter()
                .rposition(|(pos, _)| *pos < cursor)
                .unwrap_or(matches.len() - 1)
        };
        self.doc.editor.set_cursor(matches[index].0);
        self.set_status(format!("{}/{} matches", index + 1, matches.len()));
    }

    fn on_key_palette(&mut self, event: KeyEvent) {
        // Modal sub-states first: they swallow everything.
        match self.palette.mode.clone() {
            PaletteMode::ConfirmDelete(path) => {
                if matches!(&event.logical_key, Key::Character(c) if c.eq_ignore_ascii_case("y")) {
                    self.palette.mode = PaletteMode::Browse;
                    self.palette_delete(path);
                } else {
                    self.palette.mode = PaletteMode::Browse;
                    self.set_status("cancelled");
                }
                self.poke();
                return;
            }
            PaletteMode::Rename { old, mut new } => {
                match &event.logical_key {
                    Key::Named(NamedKey::Escape) => {
                        self.palette.mode = PaletteMode::Browse;
                        self.set_status("cancelled");
                    }
                    Key::Named(NamedKey::Enter) => {
                        self.palette.mode = PaletteMode::Browse;
                        self.palette_rename(old, &new);
                    }
                    Key::Named(NamedKey::Backspace) => {
                        new.pop();
                        self.palette.mode = PaletteMode::Rename { old, new };
                    }
                    Key::Named(NamedKey::Space) => {
                        new.push(' ');
                        self.palette.mode = PaletteMode::Rename { old, new };
                    }
                    Key::Character(text) => {
                        for ch in text.chars().filter(|c| !c.is_control()) {
                            new.push(ch);
                        }
                        self.palette.mode = PaletteMode::Rename { old, new };
                    }
                    _ => {}
                }
                self.poke();
                return;
            }
            PaletteMode::Browse => {}
        }

        let ctrl = self.mods.control_key() || self.mods.super_key();
        if ctrl && let PhysicalKey::Code(code) = event.physical_key {
            {
                match code {
                    KeyCode::KeyD => {
                        if let Some(path) = self.palette_target() {
                            self.palette.mode = PaletteMode::ConfirmDelete(path);
                            self.poke();
                        }
                        return;
                    }
                    KeyCode::KeyR => {
                        if let Some(old) = self.palette_target() {
                            let new = self.doc_title_of(&old).trim_end_matches(".md").to_string();
                            self.palette.mode = PaletteMode::Rename { old, new };
                            self.poke();
                        }
                        return;
                    }
                    _ => {}
                }
            }
        }

        match &event.logical_key {
            Key::Named(NamedKey::Escape) => self.to_write(),
            Key::Named(NamedKey::Enter) => self.palette_accept(),
            Key::Named(NamedKey::ArrowUp) => {
                self.palette.selected = self.palette.selected.saturating_sub(1);
                self.poke();
            }
            Key::Named(NamedKey::ArrowDown) => {
                if self.palette.selected + 1 < self.palette.filtered.len() {
                    self.palette.selected += 1;
                }
                self.poke();
            }
            Key::Named(NamedKey::Backspace) => {
                self.palette.input.pop();
                self.palette.refilter();
                self.poke();
            }
            Key::Named(NamedKey::Space) => {
                self.palette.input.push(' ');
                self.palette.refilter();
                self.poke();
            }
            Key::Character(text) => {
                for ch in text.chars().filter(|c| !c.is_control()) {
                    self.palette.input.push(ch);
                }
                self.palette.refilter();
                self.poke();
            }
            _ => {}
        }
    }

    fn on_key_settings(&mut self, event: KeyEvent) {
        let ctrl = self.mods.control_key() || self.mods.super_key();
        match &event.logical_key {
            _ if ctrl
                && matches!(&event.logical_key, Key::Character(c) if c.eq_ignore_ascii_case("s")) =>
            {
                self.save_settings()
            }
            Key::Named(NamedKey::Escape) => self.to_write(),
            Key::Named(NamedKey::Enter) => {
                if self.form.focus == 4 {
                    self.form.enabled = !self.form.enabled;
                    self.poke();
                } else {
                    self.save_settings();
                }
            }
            Key::Named(NamedKey::Tab) | Key::Named(NamedKey::ArrowDown) => {
                self.form.focus = (self.form.focus + 1) % FormState::field_count();
                self.poke();
            }
            Key::Named(NamedKey::ArrowUp) => {
                self.form.focus =
                    (self.form.focus + FormState::field_count() - 1) % FormState::field_count();
                self.poke();
            }
            Key::Named(NamedKey::Backspace) => {
                if let Some(field) = self.form.text_field_mut() {
                    field.pop();
                }
                self.poke();
            }
            Key::Named(NamedKey::Space) => {
                if let Some(field) = self.form.text_field_mut() {
                    field.push(' ');
                }
                self.poke();
            }
            Key::Character(text) => {
                if let Some(field) = self.form.text_field_mut() {
                    for ch in text.chars().filter(|c| !c.is_control()) {
                        field.push(ch);
                    }
                }
                self.poke();
            }
            _ => {}
        }
    }

    fn move_vertical(&mut self, delta: isize, extend: bool) {
        let (text_width, _) = ui::layout(self.screen.cols());
        for _ in 0..delta.unsigned_abs() {
            if delta < 0 {
                self.doc.editor.move_up(text_width, extend);
            } else {
                self.doc.editor.move_down(text_width, extend);
            }
        }
        self.doc.editor.break_undo_group();
        // Without this the redraw waits for the next cursor blink, which makes
        // vertical movement feel like it lags a beat behind the key repeat.
        self.poke();
    }

    /// Mark the document edited and restart the autosave timer.
    fn touch(&mut self) {
        self.doc.last_edit = Instant::now();
        self.poke();
    }

    /// Folder the current document lives in, home abbreviated.
    fn location(&self) -> String {
        let dir = self
            .doc
            .path
            .as_ref()
            .and_then(|p| p.parent())
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| self.vault());
        let home = storage::home_dir();
        match dir.strip_prefix(&home) {
            Ok(rest) if rest.as_os_str().is_empty() => "~".to_string(),
            Ok(rest) => format!("~/{}", rest.display()),
            Err(_) => dir.display().to_string(),
        }
    }

    fn cycle_theme(&mut self, delta: isize) -> Result<String, String> {
        let len = Theme::ALL.len() as isize;
        let idx = Theme::ALL
            .iter()
            .position(|t| *t == self.theme)
            .unwrap_or(0) as isize;
        let next = (idx + delta).rem_euclid(len);
        self.theme = Theme::ALL[next as usize];
        self.save_prefs();
        self.dirty = true;
        Ok(format!("theme: {}", self.theme.name))
    }

    fn set_fullscreen(&mut self, on: bool) {
        if let Some(window) = self.window.as_ref() {
            window.set_fullscreen(on.then_some(Fullscreen::Borderless(None)));
        }
        self.dirty = true;
    }
}

/// Effect switches: `None` toggles, `Some` sets. Returns the resulting state.
fn apply_switch(slot: &mut f32, base: f32, want: Option<bool>) -> bool {
    let on = want.unwrap_or(*slot <= 0.0);
    *slot = if on { base.max(0.05) } else { 0.0 };
    on
}

fn on_off(value: bool) -> &'static str {
    if value { "on" } else { "off" }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let fullscreen = self.opts.fullscreen && self.opts.screenshot.is_none();
        let mut attrs = Window::default_attributes()
            .with_title("CRead")
            .with_fullscreen(fullscreen.then_some(Fullscreen::Borderless(None)));
        match (self.opts.screenshot.is_none(), self.opts.size) {
            (true, Some((w, h))) => {
                attrs = attrs.with_inner_size(winit::dpi::PhysicalSize::new(w, h));
            }
            (false, _) => {
                attrs = attrs.with_inner_size(winit::dpi::PhysicalSize::new(640, 400));
            }
            _ => {}
        }

        let window = match el.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                eprintln!("cread: cannot create window: {e}");
                el.exit();
                return;
            }
        };

        let mut renderer = match Renderer::new(window.clone(), self.backends()) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("cread: {e}");
                el.exit();
                return;
            }
        };

        let size = window.inner_size();
        renderer.resize(size.width.max(1), size.height.max(1));

        self.adapter = renderer.adapter_info().to_string();
        println!("cread: vault {}", self.vault().display());
        println!(
            "cread: {} · surface {:?}",
            self.adapter,
            renderer.surface_format()
        );

        self.renderer = Some(renderer);
        self.window = Some(window);
        self.recompute_viewport();
        self.dirty = true;

        // A screenshot wants the settled picture; `--boot` replays the flicker
        // so it can be looked at.
        if self.opts.boot || self.opts.screenshot.is_none() {
            self.boot = Some(BootState::new());
        }

        if self.sync_cfg.enabled {
            self.start_worker();
            self.request_sync();
        }

        match self.opts.show.as_str() {
            "palette" => self.open_palette(),
            "settings" => self.open_settings(),
            "help" => {
                self.mode = Mode::Help;
            }
            "view" => self.enter_view(),
            "inspiro" => {
                self.home = Home::Inspiro;
                self.mode = Mode::Action;
            }
            "softhand" => {
                self.home = Home::SoftHand;
                self.mode = Mode::Action;
            }
            "picture" => {
                self.new_picture();
                if !self.opts.command.is_empty() {
                    self.command = self.opts.command.clone();
                    self.program_prompt = true;
                }
            }
            "pictures" => self.open_pictures(),
            other if other.starts_with("picture:") => {
                let path = PathBuf::from(&other["picture:".len()..]);
                if let Err(e) = self.open_picture(path) {
                    eprintln!("cread: {e}");
                }
            }
            "loading" => self.start_loading(Pending::Inspiro),
            "disk" => {
                self.refresh_disks();
                let name = self.opts.command.clone();
                if let Err(e) = self.run_named(&name) {
                    eprintln!("cread: {e}");
                }
            }
            "diskfile" => {
                self.refresh_disks();
                let name = self.opts.command.clone();
                match self.run_named(&name) {
                    Ok(_) => self.open_disk_file(),
                    Err(e) => eprintln!("cread: {e}"),
                }
            }
            "search" => {
                let query = self.opts.command.clone();
                self.start_search((!query.is_empty()).then_some(query));
            }
            "welcome" => {
                self.has_document = false;
                self.mode = Mode::Action;
            }
            "action" => {
                self.to_action();
                self.command = self.opts.command.clone();
            }
            _ => {}
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::ModifiersChanged(mods) => self.mods = mods.state(),
            WindowEvent::Resized(size) => {
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.resize(size.width.max(1), size.height.max(1));
                }
                self.recompute_viewport();
            }
            WindowEvent::ScaleFactorChanged { .. } => self.recompute_viewport(),
            WindowEvent::CursorMoved { position, .. } => {
                self.pointer = (position.x, position.y);
                if !self.program_pointer(PointerKind::Move) {
                    self.on_mouse_drag();
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if self.program.is_some() {
                    let code = match button {
                        MouseButton::Left => Some(cread_core::program::BUTTON_LEFT),
                        MouseButton::Right => Some(cread_core::program::BUTTON_RIGHT),
                        MouseButton::Middle => Some(cread_core::program::BUTTON_MIDDLE),
                        _ => None,
                    };
                    if let Some(code) = code {
                        let kind = match state {
                            ElementState::Pressed => PointerKind::Down(code),
                            ElementState::Released => PointerKind::Up(code),
                        };
                        self.program_pointer(kind);
                    }
                } else {
                    match (button, state) {
                        (MouseButton::Left, ElementState::Pressed) => self.on_mouse_press(),
                        (MouseButton::Left, ElementState::Released) => self.on_mouse_release(),
                        _ => {}
                    }
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => (y * 3.0).round() as isize,
                    MouseScrollDelta::PixelDelta(pos) => (pos.y / 16.0).round() as isize,
                };
                if !self.program_pointer(PointerKind::Wheel(lines as i32)) {
                    self.on_wheel(-lines);
                }
            }
            WindowEvent::RedrawRequested => self.frame(el),
            WindowEvent::KeyboardInput { event, .. } => self.on_key(el, event),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        // Render on our own clock instead of as fast as the compositor asks: a
        // writing app should be close to idle when you are just reading. The
        // deadline matters - with a bare `Wait` the loop would sleep until the
        // next input event and the cursor would stop blinking.
        let now = Instant::now();
        if now >= self.next_frame {
            el.set_control_flow(ControlFlow::Wait);
            if let Some(window) = self.window.as_ref() {
                window.request_redraw();
            }
        } else {
            el.set_control_flow(ControlFlow::WaitUntil(self.next_frame));
        }
    }

    fn exiting(&mut self, _el: &ActiveEventLoop) {
        // Never lose the last keystrokes on quit.
        if self.doc.editor.modified() {
            let _ = self.save();
        }
        self.discard_scratch();
    }
}

/// Biggest whole divisor whose result fits, as (rows, divisor).
fn fit_logo(logo: &Option<Splash>, max_w: u32, max_h: u32) -> Option<(u32, u32)> {
    let splash = logo.as_ref()?;
    for divisor in 1..=8 {
        if splash.width() / divisor <= max_w && splash.height() / divisor <= max_h {
            return Some((
                (splash.height() / divisor).div_ceil(cread_core::FONT_H),
                divisor,
            ));
        }
    }
    None
}

/// Where a front door's logo goes: (x, y, divisor, rows it occupies).
fn front_door_layout(
    logo: &Option<Splash>,
    max_w: u32,
    max_h: u32,
    cols: u32,
    rows: u32,
) -> Option<(i32, i32, u32, u32)> {
    let (img_rows, divisor) = fit_logo(logo, max_w, max_h)?;
    let block = img_rows + 1 + 3;
    let top = rows.saturating_sub(block) / 2;
    let width = (logo.as_ref()?.width() / divisor) as i32;
    Some((
        (cols * cread_core::FONT_W) as i32 / 2 - width / 2,
        (top * cread_core::FONT_H) as i32,
        divisor,
        img_rows,
    ))
}

/// Logo divisor and row count for the boot screen, chosen so the whole group
/// (logo, greeting, drives) still fits above the prompt.
fn boot_logo(
    splash: &Option<Splash>,
    max_w: u32,
    max_h: u32,
    rows: u32,
    items: u32,
    note: bool,
) -> (u32, u32) {
    let Some(splash) = splash.as_ref() else {
        return (1, 0);
    };
    for divisor in 1..=8u32 {
        if splash.width() / divisor > max_w || splash.height() / divisor > max_h {
            continue;
        }
        let img_rows = (splash.height() / divisor).div_ceil(cread_core::FONT_H);
        let gap = u32::from(img_rows > 0);
        let block = img_rows + gap + 1 + 1 + items + u32::from(note);
        let top = rows.saturating_sub(block) / 2;
        let last_row = top + img_rows + gap + 2 + items.saturating_sub(1) + u32::from(note);
        // The list must end above the prompt line, which keeps three rows for
        // itself and a little air.
        if last_row < rows.saturating_sub(3) {
            return (divisor, img_rows);
        }
    }
    (1, 0)
}

fn report_word(report: &SyncReport) -> String {
    report.summary()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> App {
        App::new(Options::default())
    }

    fn rows_as_text(screen: &Screen) -> String {
        let mut out = String::new();
        for y in 0..screen.rows() {
            for x in 0..screen.cols() {
                out.push(screen.get(x, y).unwrap().ch);
            }
            out.push('\n');
        }
        out
    }

    fn vault(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cread-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn app_in(dir: &Path) -> App {
        App::new(Options {
            vault: Some(dir.to_path_buf()),
            ..Options::default()
        })
    }

    #[test]
    fn an_empty_scratch_document_is_removed_when_it_is_left() {
        let dir = vault("scratch-empty");
        let mut app = app_in(&dir);
        app.new_document().unwrap();
        let path = app.doc.path.clone().unwrap();
        assert!(path.is_file(), "new reserves a name straight away");

        app.discard_scratch();
        assert!(!path.exists(), "nothing was written, so nothing stays");

        // The name is free again, so the vault does not fill with untitled-n.
        app.new_document().unwrap();
        assert_eq!(app.doc.path.as_deref(), Some(path.as_path()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_scratch_document_that_got_text_stays() {
        let dir = vault("scratch-used");
        let mut app = app_in(&dir);
        app.new_document().unwrap();
        let path = app.doc.path.clone().unwrap();

        // Text only in the buffer: still the user's, keep the reservation.
        app.doc.editor.insert_text("hello");
        app.discard_scratch();
        assert!(path.is_file());

        // Text on disk, empty buffer: autosave wrote something earlier.
        app.doc.editor = Editor::new();
        std::fs::write(&path, "hello\n").unwrap();
        app.discard_scratch();
        assert!(path.is_file());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_launch_is_flicker_then_bar_then_the_picture() {
        use BootEvent::*;
        assert_eq!(boot_advance(BootPhase::Warmup, 0.3), None);
        assert_eq!(boot_advance(BootPhase::Warmup, 0.7), Some(Loading));
        assert_eq!(boot_advance(BootPhase::Loading, 0.9), None);
        assert_eq!(boot_advance(BootPhase::Loading, 1.0), Some(Ready));
    }

    #[test]
    fn the_warmup_flashes_black_and_bright_and_ends_lit() {
        assert_eq!(level_at(BOOT_WARMUP_STEPS, 0.0), 0.0);
        assert_eq!(level_at(BOOT_WARMUP_STEPS, 0.08), 0.9);
        assert_eq!(level_at(BOOT_WARMUP_STEPS, 0.15), 0.0);
        assert_eq!(level_at(BOOT_WARMUP_STEPS, 0.65), 1.0, "holds once warm");
    }

    #[test]
    fn the_boot_screen_is_documents_then_a_rule_then_the_modules() {
        let dir = vault("boot-order");
        std::fs::create_dir_all(dir.join("Disks")).unwrap();
        std::fs::write(
            dir.join("Disks/HELLO.DSK"),
            "CRead Disk 1\nname=HELLO\n\n---FILE A.TXT\nhi\n",
        )
        .unwrap();
        let mut app = app_in(&dir);
        app.refresh_disks();
        let rows = app.boot_rows();
        let labels: Vec<&str> = rows.iter().map(|row| row.label.as_str()).collect();
        assert_eq!(
            labels,
            vec!["A: DOCUMENTS", "", "B: INSPIRO", "C: SOFTHAND", "D: HELLO"]
        );
        assert!(
            rows[1].rule,
            "the rule separates the vault from the modules"
        );
        assert!(rows[2].detail.contains("your ideas"));
        assert!(rows[3].detail.contains("16 colors"));
        assert!(!rows[3].broken, "SOFTHAND is installed, no diskette needed");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_arrow_keys_skip_the_rule() {
        let dir = vault("boot-nav");
        let mut app = app_in(&dir);
        assert_eq!(app.boot_selected, 0, "starts on DOCUMENTS");
        app.boot_step(1);
        assert_eq!(app.boot_selected, 2, "over the rule, onto INSPIRO");
        app.boot_step(1);
        assert_eq!(app.boot_selected, 3, "onto SOFTHAND");
        app.boot_step(1);
        assert_eq!(app.boot_selected, 3, "no diskette, no row, no wrap");
        app.boot_step(-1);
        assert_eq!(app.boot_selected, 2);
        app.boot_step(-1);
        assert_eq!(app.boot_selected, 0, "back over the rule");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_disk_naming_an_unknown_program_says_so() {
        let dir = vault("unknown-program");
        std::fs::create_dir_all(dir.join("Disks")).unwrap();
        std::fs::write(
            dir.join("Disks/MAIL.DSK"),
            "CRead Disk 1\nname=MAIL\nprogram=mailbox\n",
        )
        .unwrap();
        let mut app = app_in(&dir);
        app.refresh_disks();
        let rows = app.boot_rows();
        let row = rows
            .iter()
            .find(|row| row.label.contains("MAIL"))
            .expect("listed");
        assert!(row.broken);
        assert!(row.detail.contains("not installed"), "{}", row.detail);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_disk_without_the_program_header_stays_a_data_disk() {
        let dir = vault("data-disk");
        std::fs::create_dir_all(dir.join("Disks")).unwrap();
        std::fs::write(
            dir.join("Disks/HELLO.DSK"),
            "CRead Disk 1\nname=HELLO\n\n---FILE A.TXT\nhi\n",
        )
        .unwrap();
        let mut app = app_in(&dir);
        app.refresh_disks();
        let rows = app.boot_rows();
        let row = rows
            .iter()
            .find(|row| row.label.contains("HELLO"))
            .expect("listed");
        assert!(row.label.ends_with("HELLO"));
        assert!(row.detail.contains("file"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn help_shows_the_commands_of_the_screen_you_are_on() {
        let dir = vault("help-scope");
        let mut app = app_in(&dir);
        assert_eq!(app.help_scope(), command::Scope::Boot);

        app.home = Home::Inspiro;
        assert_eq!(app.help_scope(), command::Scope::Editor);

        app.home = Home::SoftHand;
        assert_eq!(app.help_scope(), command::Scope::Program);

        app.new_picture();
        assert_eq!(app.help_scope(), command::Scope::Program);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn exit_leaves_a_program_and_the_boot_screen_is_next() {
        let dir = vault("leave-program");
        let mut app = app_in(&dir);
        app.home = Home::SoftHand;
        app.new_picture();
        assert_eq!(app.mode, Mode::Program);
        app.leave_to_boot();
        assert!(app.program.is_none());
        assert_eq!(app.mode, Mode::Action);
        assert_eq!(app.home, Home::Boot);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_click_on_the_canvas_reaches_the_program() {
        let dir = vault("pointer");
        let mut app = app_in(&dir);
        app.new_picture();
        let vp = Viewport::compute(1280, 800, ScaleMode::DEFAULT).unwrap();
        let scale = vp.scale as f64;
        app.vp = Some(vp);
        // A pointer position in real pixels lands on the raster pixel under it.
        app.pointer = (320.0 * scale, 200.0 * scale);
        assert_eq!(app.raster_point(), (320, 200));
        assert!(app.program_pointer(PointerKind::Down(cread_core::program::BUTTON_LEFT)));
        let canvas = app.program.as_ref().unwrap().picture().unwrap();
        assert_eq!(canvas.get(320, 200), Some(0), "pencil writes the colour");
        assert_eq!(canvas.get(10, 10), Some(cread_core::PAPER));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_pointer_is_ignored_while_the_prompt_is_open() {
        let dir = vault("pointer-prompt");
        let mut app = app_in(&dir);
        app.new_picture();
        app.program_prompt = true;
        app.vp = Viewport::compute(1280, 800, ScaleMode::DEFAULT);
        app.pointer = (200.0, 200.0);
        assert!(!app.program_pointer(PointerKind::Down(cread_core::program::BUTTON_LEFT)));
        let canvas = app.program.as_ref().unwrap().picture().unwrap();
        assert_eq!(canvas.get(100, 100), Some(cread_core::PAPER));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn saving_a_picture_writes_a_png_next_to_the_documents() {
        let dir = vault("save-picture");
        let mut app = app_in(&dir);
        app.new_picture();
        let message = app.save_picture(None).unwrap();
        assert!(message.starts_with("saved picture"), "{message}");
        let files = picture::list(&dir).unwrap();
        assert_eq!(files.len(), 1);
        let back = picture::load(&files[0]).unwrap();
        assert_eq!(back.width(), CANVAS_W);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_picture_picker_lists_saved_pictures() {
        let dir = vault("pictures");
        let mut app = app_in(&dir);
        app.home = Home::SoftHand;
        app.open_pictures();
        assert_eq!(app.mode, Mode::Palette);
        assert!(app.palette.entries.is_empty(), "nothing saved yet");

        app.new_picture();
        app.save_picture(None).unwrap();
        app.leave_to_boot();
        app.home = Home::SoftHand;
        app.open_pictures();
        assert_eq!(app.palette.entries.len(), 1);
        assert!(matches!(app.palette.entries[0].1, Action::OpenPicture(_)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn open_on_the_front_door_shows_the_palette() {
        let mut app = app();
        app.home = Home::Inspiro;
        app.mode = Mode::Action;
        app.compose();
        assert!(!rows_as_text(&app.screen).contains("+ new document"));

        // 'open' is a command line away, and the front door composed its own
        // screen; it used to return from compose before the palette was drawn.
        app.command = "open".into();
        app.open_palette();
        app.compose();
        let text = rows_as_text(&app.screen);
        assert!(text.contains("+ new document"), "palette is on screen");
        assert!(text.contains("Enter open"), "with its hint");
    }

    #[test]
    fn the_inspiro_logo_belongs_to_its_own_screen_only() {
        let mut app = app();
        app.home = Home::Inspiro;
        app.mode = Mode::Action;
        app.compose();
        assert!(app.inspiro_at.is_some(), "front door shows its logo");
        assert!(app.splash_at.is_none(), "and not the boot logo");

        // Help, the palette and the boot screen all draw over the same grid, so
        // a stale blit position floats on top of them.
        app.mode = Mode::Help;
        app.compose();
        assert!(app.inspiro_at.is_none());
        assert!(app.splash_at.is_none());

        app.home = Home::Boot;
        app.mode = Mode::Action;
        app.compose();
        assert!(app.inspiro_at.is_none());
        assert!(
            app.splash_at.is_some(),
            "the boot screen shows the boot logo"
        );

        // An overlay hides the boot logo while it is open.
        app.mode = Mode::Palette;
        app.compose();
        assert!(app.splash_at.is_none());
    }
}
