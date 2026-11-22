//! Window lifetime, frame loop and input.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

mod boot;
mod browser;
mod calendar;
mod commands;
mod compose;
mod machine;
mod manual;
mod note;
mod pointer;
mod power;
mod search;
mod sync;
mod update;
mod webdav;

use manual::ManualJump;

use browser::{Browser, BrowserKind, BrowserMode};
use webdav::FormState;

use calendar::{CalendarState, EventForm, ReminderDue};
use machine::Machine;
use power::PowerPrompt;
use sync::QuitPrompt;

use cread_core::{
    Editor, KeyPress, Position, Prefs, Raster, ScaleMode, Screen, Stats, Theme, Viewport,
    command::{self, Command, CommandError, RiffAction},
    music, picture,
    program::{Outcome, PointerEvent, PointerKind, Program},
    resolution::target_line_px_for_scale_factor,
    space, storage,
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

use crate::screensaver::next_idle_minutes;
use crate::ui;
use cread_sync::{Config as SyncConfig, Report as SyncReport, Worker};
use cread_update::Event as UpdateEvent;

const WELCOME: &str = include_str!("welcome.md");
/// The user manual, embedded so it is always there — like a built-in book.
const MANUAL: &str = include_str!("manual.md");

/// Programs are written for the classic 80x25 stage.
const STAGE_COLS: u32 = 80;
const STAGE_ROWS: u32 = 25;
/// Softhand paints a 23-row canvas and a one-row palette strip, so its stage is
/// one row shorter and the legend can sit underneath.
const SOFTHAND_ROWS: u32 = 24;

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
/// One program frame, in wall time: 59.7275 Hz. Pocketboi is clocked by this
/// instead of 60 Hz because its audio is tied to wall time and 0.45% more
/// samples per second is a queue that grows into a delay; US Poker uses the
/// same budget for the risk light.
const PROGRAM_FRAME: Duration = Duration::from_micros(16_734);
/// How long a status message stays on the line before the mode hint returns.
const STATUS_TTL: Duration = Duration::from_secs(4);
/// Launch: a bar runs before the first real frame, long enough to read as a
/// machine starting up. Any key skips it.
const BOOT_SECONDS: f32 = 1.5;
/// What the bar says while the "system" starts.
const BOOT_LABEL: &str = "Starting CRead OS...";
/// Installing a module takes a moment, and the moment is the point.
const SOFTWARE_SECONDS: f32 = 0.9;

/// (fraction of the wait, fraction of the bar) — a drive seeking: it moves in
/// jumps and then sits still for a moment before the next one.
const BOOT_CURVE: &[(f32, f32)] = &[
    (0.00, 0.00),
    (0.05, 0.03),
    (0.11, 0.03),
    (0.15, 0.12),
    (0.29, 0.12),
    (0.33, 0.20),
    (0.45, 0.21),
    (0.49, 0.38),
    (0.58, 0.38),
    (0.62, 0.55),
    (0.71, 0.56),
    (0.75, 0.72),
    (0.84, 0.72),
    (0.88, 0.89),
    (0.92, 1.00),
];

/// Bar fill at `seconds` in, 0 to 1.
///
/// Deliberately not a smooth ease: a real drive fills a bar in jumps with
/// pauses between them. Full a little before the wait ends, so it never stops
/// short of the right edge.
fn boot_progress(seconds: f32) -> f32 {
    let at = (seconds / BOOT_SECONDS).clamp(0.0, 1.0);
    BOOT_CURVE
        .iter()
        .rev()
        .find(|(from, _)| at >= *from)
        .map(|(_, level)| *level)
        .unwrap_or(0.0)
}

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
    OpenRom(PathBuf),
    /// A track in the music browser: load it into the deck.
    PlayTrack(PathBuf),
    EnterDir(PathBuf),
    Up,
    NewFile,
    SyncNow,
}

/// An active incremental search.
struct Search {
    /// Where the cursor was when the search started, for Esc to restore.
    origin: Position,
    query: String,
    matches: Vec<(Position, usize)>,
    current: usize,
}

/// What Riff is doing. Owned by the shell, so the music outlives the screen.
struct RiffState {
    playlist: music::Playlist,
    /// A track is loaded and not paused.
    playing: bool,
    /// Folder or file the playlist was inserted from.
    source: String,
    /// The music folder browser, while it is open.
    explorer: Option<Browser>,
    /// Last whole second drawn, so the counter does not redraw at 60 fps.
    second: u64,
}

impl RiffState {
    fn new() -> Self {
        Self {
            playlist: music::Playlist::default(),
            playing: false,
            source: String::new(),
            explorer: None,
            second: 0,
        }
    }
}

/// Which home screen the shell is showing when nothing is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Home {
    /// The boot screen: the vault, the modules and the shelf.
    Boot,
    /// Inspiro's own front door: logo, tagline, new/open.
    Inspiro,
    /// Softhand's front door: same shape, its own logo and tagline.
    SoftHand,
    /// Riff's front door.
    Riff,
    /// American Poker's front door.
    Poker,
    /// SCRIPTURE's front door: a quiet cover before the reader opens.
    Scripture,
}

/// What happens when the loading bar finishes.
enum Pending {
    Inspiro,
    /// Index into [`PROGRAMS`]: load that program's front door.
    Program(usize),
    NewPicture,
    Picture(PathBuf),
    /// Install or remove a module once the bar has run.
    Software {
        id: &'static str,
        install: bool,
    },
    /// A cartridge picked in the vault, played straight away.
    Rom(PathBuf),
    Document(PathBuf),
    NewDocument,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    /// Typing into the document.
    Write,
    /// The `/` prompt at the bottom.
    Search,
    /// Reader mode: markup stripped, not editable.
    View,
    /// A fake loading bar before a program appears.
    Loading,
    /// A command line at the bottom, like a shell.
    Action,
    /// The vault: folders, documents, pictures, cartridges.
    Browser,
    /// The WebDAV form.
    Webdav,
    /// Full-screen help or about.
    Help,
    /// A program owns the screen.
    Program,
    /// Riff playing, or its front door.
    Riff,
    /// The music folder browser.
    RiffInsert,
    /// Install and remove modules.
    Software,
    /// The system settings: WebDAV, theme, font, screen, info, update.
    Setup,
    /// A progress window while GitHub is asked, or the new binary is fetched.
    Checking,
    /// "0.22.0 is available. Install and restart?"
    UpdatePrompt,
    /// "Sync to server?" on the way out.
    Quit,
    /// "Save and power off / restart / leave for a console?"
    Power,
    /// The user manual: read-only Markdown with a scrollbar.
    Manual,
    /// The month grid and the day's appointments.
    Calendar,
    /// The new/edit event form.
    CalendarEdit,
    /// A due reminder; a modal over whatever was in front.
    Reminder,
    /// A small box confirming one thing, e.g. the sync switch.
    Notice,
    /// The clock, after a while without input.
    Screensaver,
}

/// A program compiled into this build.
pub struct ProgramInfo {
    pub id: &'static str,
    /// What the boot row and the loading bar call it.
    pub entry: &'static str,
    pub tagline: &'static str,
}

pub const PROGRAMS: &[ProgramInfo] = &[
    ProgramInfo {
        id: "softhand",
        entry: "SOFTHAND",
        tagline: "16 colors, endless possibilities.",
    },
    ProgramInfo {
        id: "riff",
        entry: "RIFF",
        tagline: "a Soundtrack for your life.",
    },
    ProgramInfo {
        id: "pocketboi",
        entry: "POCKETBOI",
        tagline: "get gaming, my friend!",
    },
    ProgramInfo {
        id: "poker",
        entry: "US POKER",
        tagline: "ten dollars and a dream.",
    },
    ProgramInfo {
        id: "scripture",
        entry: "SCRIPTURE",
        tagline: "a quiet place for the Word.",
    },
];

/// The home screen a program opens onto.
fn program_home(index: usize) -> Home {
    match PROGRAMS.get(index).map(|program| program.id) {
        Some("softhand") => Home::SoftHand,
        Some("riff") => Home::Riff,
        Some("poker") => Home::Poker,
        Some("scripture") => Home::Scripture,
        _ => Home::Boot,
    }
}

/// What Enter does on a SETTINGS row. Rules have no action and are skipped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SetupAction {
    Webdav,
    Theme,
    Font,
    Screen,
    Screensaver,
    Idle,
    Time,
    Date,
    Info,
    Update,
    Software,
    Storage,
    Network,
    Manual,
}

/// What the last check found, remembered for the settings row.
#[derive(Clone, Debug, PartialEq, Eq)]
enum UpdateStatus {
    Unknown,
    UpToDate,
    Available(String),
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
    /// Highlighted row on the SOFTWARE screen.
    software_selected: usize,
    /// Highlighted row on the SETTINGS screen.
    setup_selected: usize,
    /// Last storage reading, taken when SETTINGS opens or STORAGE is activated.
    space: Option<space::Space>,
    /// Board, CPU, memory and local address, read when SETTINGS or INFO opens.
    machine: Option<Machine>,
    /// The layout was checked (and loose documents moved) once already.
    vault_ready: bool,
    /// Written to disk since the last successful sync in this session.
    unsynced: bool,
    /// The "sync before quitting?" dialog while it is open.
    quit: Option<QuitPrompt>,
    /// The save-and-leave dialog while it is open.
    power: Option<PowerPrompt>,

    boot_selected: usize,

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
    /// False when a kiosk has hidden the OS pointer until it is moved.
    pointer_visible: bool,
    /// winit sends a synthetic CursorMoved on pointer enter; that one must not
    /// count as "the mouse was moved".
    skip_pointer_motion: bool,
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
    /// The floating quick note, if one is open.
    note: Option<note::QuickNote>,
    loading_started: Instant,
    inspiro_logo: Option<Splash>,
    softhand_logo: Option<Splash>,
    /// Where the Softhand logo sits this frame.
    softhand_at: Option<(i32, i32, u32)>,
    riff_logo: Option<Splash>,
    riff_at: Option<(i32, i32, u32)>,
    poker_logo: Option<Splash>,
    poker_at: Option<(i32, i32, u32)>,
    scripture_logo: Option<Splash>,
    scripture_at: Option<(i32, i32, u32)>,
    /// The screensaver is switched on in SETTINGS.
    screensaver_on: bool,
    /// Minutes of silence before it starts.
    screensaver_minutes: u32,
    /// The bars show 24-hour time and German dates out of the box.
    clock_24h: bool,
    date_day_first: bool,
    /// Last keyboard or pointer activity; the screensaver counts from here.
    last_input: Instant,
    /// Where a key should return to after the screensaver.
    screensaver_back: Mode,
    /// Write preferences to disk. Off in tests, so they never touch the real
    /// config of whoever runs them.
    persist: bool,
    /// The clock, drawn full-screen and outside the theme.
    screensaver_raster: Raster,
    manual_rows: Vec<cread_core::markdown::Rendered>,
    manual_top: usize,
    manual_width: usize,
    manual_back: Mode,
    manual_jumps: Vec<ManualJump>,
    manual_choice: usize,
    /// Up/Down belong to the Contents menu, not the scrollbar.
    manual_nav: bool,
    notice_text: String,
    notice_back: Mode,
    pub(super) calendar: CalendarState,
    calendar_back: Mode,
    /// The appointment a second `D` will delete. A single keypress only arms.
    calendar_delete_armed: Option<PathBuf>,
    pub(super) calendar_form: Option<EventForm>,
    pub(super) reminder: Option<ReminderDue>,
    reminder_back: Mode,
    pub(super) reminder_title: String,
    pub(super) reminder_when: String,
    reminder_date: chrono::NaiveDate,
    reminder_path: PathBuf,
    reminder_fired: HashSet<String>,
    /// The running program, if any.
    program: Option<Box<dyn Program>>,
    /// Which module the program belongs to. The shell needs this where the
    /// trait only says "some program": `open`, `save` and `new` mean different
    /// rooms for Softhand and Pocketboi.
    program_id: Option<&'static str>,
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
    browser: Browser,
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
    /// INFO was opened from SETTINGS, so Esc belongs back there.
    help_from_setup: bool,
    /// Highlighted row on the help screen.
    help_selected: usize,
    /// `help all` shows the whole vocabulary instead of this screen's list.
    help_all: bool,
    /// The launch bar, while it runs.
    boot_at: Option<Instant>,
    /// The music output, opened on first use.
    audio: Option<crate::audio::Audio>,
    riff: RiffState,
    /// Loudness, mirrored from the preferences.
    volume: u8,
    /// The startup beep, mirrored from the preferences.
    beep_on: bool,
    /// Write the document while typing; off unless asked for.
    autosave: bool,

    sync_cfg: SyncConfig,
    sync: Option<Worker>,
    /// The update worker, started on the first check and kept for the session.
    updater: Option<cread_update::Worker>,
    /// Where the release list is read from; tests point it at a dead address.
    update_api: String,
    /// This executable, captured at startup. It must not be asked for again
    /// after an update: renaming the running binary makes `/proc/self/exe`
    /// follow the inode, so `current_exe()` would then name `cread.old`.
    self_exe: Option<PathBuf>,
    update_started: Instant,
    /// The window is fetching and installing, not asking.
    update_installing: bool,
    update_status: UpdateStatus,
    update_offer: Option<cread_update::Offer>,
    update_yes: bool,
    /// The arguments this session was started with, for the restart.
    argv: Vec<std::ffi::OsString>,
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
        let autosave = prefs.autosave;
        let volume = prefs.volume;
        let (doc, has_document) = Self::initial_document(&opts);
        let sync_cfg = SyncConfig::load();
        let screensaver_on = prefs.screensaver;
        let screensaver_minutes = prefs.screensaver_minutes.max(1);
        let clock_24h = prefs.clock_24h;
        let date_day_first = prefs.date_day_first;
        // The browser needs its root before the first frame, not when it opens.
        let vault = opts.vault.clone().unwrap_or_else(storage::vault_dir);
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
            software_selected: 0,
            setup_selected: 0,
            space: None,
            machine: None,
            vault_ready: false,
            unsynced: false,
            quit: None,
            power: None,

            boot_selected: 0,

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
            pointer_visible: true,
            skip_pointer_motion: false,
            dragging: false,
            follow_cursor: true,
            clipboard: None,
            has_document,
            home: Home::Boot,
            pending: None,
            note: None,
            loading_started: Instant::now(),
            inspiro_logo: Splash::from_png(include_bytes!("../../../assets/inspiro.png"))
                .map_err(|e| eprintln!("cread: inspiro logo: {e}"))
                .ok(),
            softhand_logo: Splash::from_png(include_bytes!("../../../assets/softhand.png"))
                .map_err(|e| eprintln!("cread: softhand logo: {e}"))
                .ok(),
            softhand_at: None,
            riff_logo: Splash::from_png(include_bytes!("../../../assets/riff.png"))
                .map_err(|e| eprintln!("cread: riff logo: {e}"))
                .ok(),
            riff_at: None,
            poker_logo: Splash::from_png(include_bytes!("../../../assets/american_logo.png"))
                .map_err(|e| eprintln!("cread: poker logo: {e}"))
                .ok(),
            poker_at: None,
            scripture_logo: Splash::from_png(include_bytes!("../../../assets/scripture.png"))
                .map_err(|e| eprintln!("cread: scripture logo: {e}"))
                .ok(),
            scripture_at: None,
            screensaver_on,
            screensaver_minutes,
            clock_24h,
            date_day_first,
            last_input: Instant::now(),
            screensaver_back: Mode::Action,
            persist: true,
            screensaver_raster: Raster::new(1, 1),
            manual_rows: Vec::new(),
            manual_top: 0,
            manual_width: 0,
            manual_back: Mode::Setup,
            manual_jumps: Vec::new(),
            manual_choice: 0,
            manual_nav: true,
            notice_text: String::new(),
            notice_back: Mode::Setup,
            calendar: CalendarState::today(),
            calendar_back: Mode::Action,
            calendar_delete_armed: None,
            calendar_form: None,
            reminder: None,
            reminder_back: Mode::Action,
            reminder_title: String::new(),
            reminder_when: String::new(),
            reminder_date: chrono::Local::now().date_naive(),
            reminder_path: PathBuf::new(),
            reminder_fired: HashSet::new(),
            program: None,
            program_id: None,
            raster: Raster::new(CANVAS_W, SOFTHAND_H),
            picture_path: None,
            program_prompt: false,
            program_point: (-1, -1),
            mode: if has_document {
                Mode::Write
            } else {
                Mode::Action
            },
            browser: Browser::vault(vault),
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
            help_from_setup: false,
            help_selected: 0,
            help_all: false,
            boot_at: None,
            audio: None,
            riff: RiffState::new(),
            volume,
            beep_on,
            autosave,
            sync_cfg,
            sync: None,
            updater: None,
            update_api: std::env::var("CREAD_UPDATE_API")
                .unwrap_or_else(|_| cread_update::LATEST.to_string()),
            self_exe: std::env::current_exe().ok(),
            update_started: Instant::now(),
            update_installing: false,
            update_status: UpdateStatus::Unknown,
            update_offer: None,
            update_yes: true,
            argv: std::env::args_os().collect(),
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
            // A resize is a new picture, not just a dirty flag: schedule the
            // frame or the old surface can stay on screen until the cursor blink.
            self.poke();
            if let Some(window) = self.window.as_ref() {
                window.request_redraw();
            }
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
        self.pump_update(el);
        self.pump_reminders();
        // An idle machine rests: the screensaver counts from the last key or
        // pointer event, and never starts over a progress window.
        if self.screensaver_due() {
            self.start_screensaver();
        }
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
        self.riff_tick();

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

        // A running cartridge, or a live risk ladder, redraws every frame.
        if self
            .program
            .as_ref()
            .is_some_and(|program| program.wants_frames())
        {
            self.dirty = true;
        }

        if self.mode == Mode::Checking {
            // The bar moves while the worker is busy; nothing else redraws it.
            self.dirty = true;
        }
        if self.mode == Mode::Screensaver {
            // The second hand is the only thing that moves.
            self.dirty = true;
        }

        if let Some(started) = self.boot_at {
            if started.elapsed().as_secs_f32() >= BOOT_SECONDS {
                // The bar is full: the speaker says so, then the picture
                // appears. Nothing of the boot screen was drawn before it.
                self.beep();
                self.boot_at = None;
            }
            // The bar has to move every frame.
            self.dirty = true;
        }

        let mut crt = self.crt;
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
            if self.mode == Mode::Screensaver {
                // The clock owns every pixel and ignores the theme; the CRT pass
                // runs over the result like over any other picture.
                self.fb.draw_canvas(&self.screensaver_raster, [0, 0, 0]);
            } else if self.program.is_some() {
                let (x, y) = self.program_origin();
                self.fb
                    .draw_program_at(&self.screen, &self.raster, &self.theme, &self.font, x, y);
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
            if let (Some(logo), Some((x, y, divisor))) = (self.riff_logo.as_ref(), self.riff_at) {
                let style = SplashStyle::Ink(self.theme.color(self.theme.fg));
                self.fb.draw_splash(logo, x, y, divisor, &style);
            }
            if let (Some(logo), Some((x, y, divisor))) = (self.poker_logo.as_ref(), self.poker_at) {
                let style = SplashStyle::Ink(self.theme.color(self.theme.fg));
                self.fb.draw_splash(logo, x, y, divisor, &style);
            }
            if let (Some(logo), Some((x, y, divisor))) =
                (self.scripture_logo.as_ref(), self.scripture_at)
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
        if self.mode == Mode::Screensaver {
            // One tick a second: the second hand steps, it does not sweep.
            self.next_frame = frame_start + Duration::from_secs(1);
        } else if self.boot_at.is_some() || matches!(self.mode, Mode::Loading | Mode::Checking) {
            self.next_frame = frame_start + Duration::from_secs_f64(1.0 / ANIMATION_FPS as f64);
        } else if self
            .program
            .as_ref()
            .is_some_and(|program| program.wants_frames())
        {
            self.next_frame = frame_start + PROGRAM_FRAME;
        } else if crt.roll > 0.0 && crt.scanline > 0.0 {
            // The raster moves, so this is the one case that needs a full clock.
            self.next_frame = frame_start + Duration::from_secs_f64(1.0 / ANIMATION_FPS as f64);
        } else if self.riff.playing {
            // No raster movement: the tape counter and the end of a track only
            // need twice a second.
            self.next_frame = frame_start + Duration::from_millis(500);
        } else {
            let phase = (time * CURSOR_BLINK_HZ) as u32;
            let next_toggle = (phase as f32 + 1.0) / CURSOR_BLINK_HZ;
            let wait = (next_toggle - time).clamp(0.005, 1.0);
            self.next_frame = frame_start + Duration::from_secs_f32(wait);
        }
        if let Some(reminder) = self.next_reminder_instant()
            && reminder < self.next_frame
        {
            self.next_frame = reminder;
        }
    }

    // ── persistence ─────────────────────────────────────────────────────────

    fn autosave_if_due(&mut self) {
        if !self.autosave {
            return;
        }
        if self.mode != Mode::Write {
            return;
        }
        if !self.doc.editor.modified() {
            return;
        }
        if self.doc.last_edit.elapsed() < AUTOSAVE_DELAY {
            return;
        }
        let Some(path) = self.doc.path.clone() else {
            return;
        };
        if let Err(e) = self.save_to(&path) {
            self.set_status(e);
        }
    }

    fn save_to(&mut self, path: &Path) -> Result<(), String> {
        let text = self.doc.editor.text();
        storage::save_atomic(path, &text).map_err(|e| format!("save failed: {e}"))?;
        self.unsynced = true;
        self.doc.editor.mark_saved();
        self.doc.path = Some(path.to_path_buf());
        if self.sync_cfg.enabled {
            self.request_sync();
        }
        self.dirty = true;
        Ok(())
    }

    fn save(&mut self) -> Result<String, String> {
        if !self.has_document {
            return Err("no document - use new or open".into());
        }
        let path = match self.doc.path.clone() {
            Some(path) => path,
            None => self.free_untitled_path(),
        };
        self.save_to(&path)?;
        // `save` on the placeholder is the writer keeping that name: it is a
        // document now, not a draft the editor made up.
        if self.scratch.as_deref() == Some(path.as_path()) {
            self.scratch = None;
            self.forget_draft();
        }
        Ok(String::new())
    }

    /// Start a fresh file in the vault, named `untitled.md`, `untitled-2.md`, …
    fn new_document(&mut self) -> Result<String, String> {
        if self.has_document && self.doc.editor.modified() {
            return Err("unsaved changes - save first".into());
        }
        self.prepare_vault();
        self.discard_scratch();
        let path = self.free_untitled_path();
        self.doc = Document {
            path: None,
            title_hint: None,
            editor: Editor::new(),
            last_edit: Instant::now(),
        };
        self.has_document = true;
        self.scratch = Some(path.clone());
        self.remember_draft();
        self.save_to(&path)?;
        Ok(format!("created {}", self.doc.title()))
    }

    /// Delete the file `new` reserved if the document is being left empty.
    ///
    /// Both the buffer and the file on disk have to be blank: a scratch file
    /// that autosave filled is somebody's text, and an in-memory document with
    /// unsaved characters must keep its place on disk.
    fn discard_scratch(&mut self) -> Option<String> {
        let path = self.scratch.take()?;
        // Whatever happens next, this file is no longer "the draft".
        self.forget_draft();
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
        if !self.has_document {
            return Err("no document - use new or open".into());
        }
        self.prepare_vault();
        let path = self.document_path(name)?;
        if path.exists() && self.doc.path.as_deref() != Some(path.as_path()) {
            return Err(format!("{} already exists", self.doc_title_of(&path)));
        }
        self.save_to(&path)?;
        match self.drop_scratch(&path) {
            Some(removed) => Ok(format!("saved as {} ({removed} removed)", self.doc.title())),
            None => Ok(format!("saved as {}", self.doc.title())),
        }
    }

    /// The untitled placeholder has done its job once the text has a name.
    ///
    /// Autosave may have filled it, but `saveas` just wrote the same buffer to
    /// the chosen path, so keeping the placeholder would only leave a second
    /// copy of the document in the vault - and on the server.
    fn drop_scratch(&mut self, kept: &Path) -> Option<String> {
        self.forget_draft();
        let scratch = self.scratch.take()?;
        if scratch == kept {
            // Saved under its own placeholder name: it is a named file now.
            return None;
        }
        std::fs::remove_file(&scratch).ok()?;
        self.unsynced = true;
        self.dirty = true;
        Some(scratch.file_name()?.to_string_lossy().into_owned())
    }

    fn vault(&self) -> PathBuf {
        self.opts.vault.clone().unwrap_or_else(storage::vault_dir)
    }

    /// Tell the sync engine which file is the editor's unnamed draft, so the
    /// server copy can follow it when the draft gets a real name.
    fn remember_draft(&self) {
        let vault = self.vault();
        let name = self
            .scratch
            .as_ref()
            .and_then(|path| path.strip_prefix(&vault).ok())
            .map(|path| path.to_string_lossy().into_owned());
        let _ = cread_sync::set_draft(&vault, name.as_deref());
    }

    fn forget_draft(&self) {
        let _ = cread_sync::set_draft(&self.vault(), None);
    }

    /// Check the vault layout once: a document belongs in `Documents/`, so
    /// anything still sitting in the root is moved there before we list it.
    fn prepare_vault(&mut self) {
        if self.vault_ready {
            return;
        }
        self.vault_ready = true;
        let vault = self.vault();
        // The shelf has three rooms; an empty one still belongs to the layout,
        // so the browser and the server both show it.
        for dir in [
            storage::documents_in(&vault),
            storage::pictures_in(&vault),
            storage::roms_in(&vault),
            storage::calendar_in(&vault),
            storage::notes_in(&vault),
        ] {
            let _ = std::fs::create_dir_all(dir);
        }
        // A draft the app marked before a restart has outlived its session:
        // it belongs to the writer now, so it stops being a draft.
        let _ = cread_sync::promote_draft(&vault);
        let report = storage::migrate_documents(&vault);
        if report.moved > 0 || report.duplicates > 0 {
            self.unsynced = true;
            let mut message = format!("moved {} into Documents/", report.moved);
            if report.duplicates > 0 {
                message = format!("{message} ({} already there)", report.duplicates);
            }
            self.set_status(message);
        }
        if !report.kept.is_empty() {
            self.set_status(format!(
                "{} kept in the vault root: name already taken",
                report.kept.join(", ")
            ));
        }
    }

    /// Where documents live: `<vault>/Documents`, never the root.
    fn documents(&self) -> PathBuf {
        storage::documents_in(&self.vault())
    }

    /// First free `untitled[-n].md` in Documents.
    fn free_untitled_path(&self) -> PathBuf {
        storage::free_untitled(&self.documents())
    }

    /// Resolve a user-supplied name inside Documents, adding `.md` when needed.
    ///
    /// Subfolders are fine (`saveas kapitel/eins`), `..` and absolute paths are
    /// not: a document has exactly one home. A name that folds away to nothing
    /// (`.`, `..`, an empty string) is refused instead of becoming
    /// `<vault>/Documents.md`, a file next to the folder.
    fn document_path(&self, name: &str) -> Result<PathBuf, String> {
        let mut relative = PathBuf::new();
        for part in Path::new(name).components() {
            if let std::path::Component::Normal(part) = part {
                relative.push(part);
            }
        }
        if relative.as_os_str().is_empty() {
            return Err("a document needs a name".into());
        }
        let path = self.documents().join(relative);
        Ok(match path.extension() {
            Some(ext) if !ext.is_empty() => path,
            _ => path.with_extension("md"),
        })
    }

    /// The path `old` gets when renamed to `name`: same folder, same extension
    /// when none was typed. One rule for the command and the browser.
    fn rename_target(&self, old: &Path, name: &str) -> Result<PathBuf, String> {
        let name = Path::new(name)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if name.is_empty() {
            return Err("a name is needed".into());
        }
        let parent = old
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.vault());
        let mut target = parent.join(&name);
        if target.extension().is_none() {
            target.set_extension(old.extension().unwrap_or_else(|| "md".as_ref()));
        }
        Ok(target)
    }

    /// Find an existing document by name, with or without the extension.
    ///
    /// A bare name is looked up anywhere under Documents, so
    /// `open kapitel` finds `Documents/kapitel/eins.md`. The vault root is the
    /// last resort, for a file the migration had to leave behind.
    fn find_document(&self, name: &str) -> Option<PathBuf> {
        if let Ok(named) = self.document_path(name)
            && named.is_file()
        {
            return Some(named);
        }
        let wanted = Path::new(name)
            .file_name()?
            .to_string_lossy()
            .to_lowercase();
        let with_ext = if wanted.ends_with(".md") {
            wanted.clone()
        } else {
            format!("{wanted}.md")
        };
        let mut matches: Vec<PathBuf> = storage::walk_files(&self.documents())
            .into_iter()
            .filter(|path| storage::is_document(path))
            .filter(|path| {
                path.file_name().is_some_and(|n| {
                    let n = n.to_string_lossy().to_lowercase();
                    n == wanted || n == with_ext
                })
            })
            .collect();
        matches.sort();
        if let Some(path) = matches.into_iter().next() {
            return Some(path);
        }
        // The vault root is the last resort, for a file the migration had to
        // leave behind - and only by its bare name, never by a path.
        let leaf = Path::new(name).file_name()?;
        let legacy = self.vault().join(leaf);
        if legacy.is_file() {
            return Some(legacy);
        }
        let legacy_md = legacy.with_extension("md");
        legacy_md.is_file().then_some(legacy_md)
    }

    fn open_document(&mut self, path: PathBuf) -> Result<(), String> {
        if self.has_document && self.doc.editor.modified() {
            return Err("unsaved changes - save first".into());
        }
        self.discard_scratch();
        let text =
            storage::load(&path).map_err(|e| format!("cannot open {}: {e}", path.display()))?;
        self.doc = Document {
            path: Some(path.clone()),
            title_hint: None,
            editor: Editor::from_text(&text),
            last_edit: Instant::now(),
        };
        self.has_document = true;
        self.dirty = true;
        Ok(())
    }

    fn rename_current(&mut self, name: &str) -> Result<String, String> {
        match self.doc.path.clone().filter(|p| p.is_file()) {
            Some(old) => {
                let target = self.rename_target(&old, name)?;
                if target != old && target.exists() {
                    return Err(format!("{} already exists", self.doc_title_of(&target)));
                }
                std::fs::rename(&old, &target).map_err(|e| format!("rename failed: {e}"))?;
                self.unsynced = true;
                // A name of its own means it is no longer a placeholder.
                self.scratch = None;
                self.forget_draft();
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
        self.unsynced = true;
        if self.scratch.as_deref() == Some(path.as_path()) {
            self.scratch = None;
        }
        self.doc = Document {
            path: None,
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

    // ── browser ─────────────────────────────────────────────────────────────

    fn doc_title_of(&self, path: &Path) -> String {
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    // ── input ───────────────────────────────────────────────────────────────

    fn on_key(&mut self, el: &ActiveEventLoop, event: KeyEvent) {
        if event.state == ElementState::Released {
            // A game pad needs the key going up again; everything else ignores it.
            // Releases reach the program even while the command prompt is
            // open. Otherwise a held pad button stays pressed forever when the
            // prompt was opened between press and release.
            if self.mode == Mode::Program
                && let Some(key) = self.program_key(&event)
                && let Some(program) = self.program.as_mut()
            {
                program.key_released(key);
            }
            return;
        }
        if event.state != ElementState::Pressed {
            return;
        }
        // The screensaver belongs to the machine: the first key brings back the
        // screen and is swallowed, so it does not also type a character.
        if self.wake_screensaver() {
            self.next_frame = Instant::now();
            return;
        }
        self.last_input = Instant::now();
        if self.note.is_some() {
            self.on_key_note(event);
            return;
        }
        if self.boot_at.take().is_some() {
            // Impatient: skip straight to the picture, and swallow the key.
            self.next_frame = Instant::now();
            self.dirty = true;
            return;
        }
        match self.mode {
            Mode::Write => self.on_key_write(el, event),
            Mode::Search => self.on_key_search(event),
            Mode::View => self.on_key_view(event),
            Mode::Loading => self.finish_loading(),
            Mode::Program => self.on_key_program(el, event),
            Mode::Riff => self.on_key_riff(el, event),
            Mode::RiffInsert => self.on_key_riff_insert(event),
            Mode::Software => self.on_key_software(event),
            Mode::Setup => self.on_key_setup(event),
            Mode::Quit => self.on_key_quit(el, event),
            Mode::Power => self.on_key_power(el, event),
            Mode::Checking => self.on_key_checking(event),
            Mode::UpdatePrompt => self.on_key_update_prompt(event),
            Mode::Manual => self.on_key_manual(event),
            Mode::Calendar => self.on_key_calendar(el, event),
            Mode::CalendarEdit => self.on_key_calendar_edit(event),
            Mode::Reminder => self.on_key_reminder(event),
            Mode::Notice => self.on_key_notice(),
            Mode::Screensaver => {
                // Waking happens above; nothing else reaches this screen.
            }
            Mode::Action => self.on_key_action(el, event),
            Mode::Browser => self.on_key_browser(event),
            Mode::Webdav => self.on_key_webdav(event),
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

        // AltGr arrives as Ctrl+Alt on many X11/Wayland layouts. That is a
        // printable character, not a command chord.
        let altgr = self.mods.alt_key() && self.mods.control_key();
        if ctrl && !altgr {
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
                if self.mods.alt_key() && !self.mods.control_key() {
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
            Key::Named(NamedKey::Escape) => self.leave_help(),
            // Any other key leaves the screen the same way Esc does; this
            // keeps a later `help` from returning to SETTINGS by mistake.
            _ => self.leave_help(),
        }
    }

    /// Leave the help/about screen: back to SETTINGS when it was opened from
    /// there, otherwise back to the work it covered.
    fn leave_help(&mut self) {
        if self.help_from_setup {
            self.help_from_setup = false;
            self.mode = Mode::Setup;
        } else {
            self.to_write();
        }
        self.dirty = true;
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
                let consumed = self
                    .program
                    .as_mut()
                    .is_some_and(|program| program.escape());
                if consumed {
                    self.poke();
                    return;
                }
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
                    if self.program_id == Some("poker") {
                        self.leave_poker();
                    } else if self.program_id == Some("scripture") {
                        self.leave_scripture();
                    } else {
                        self.leave_to_boot();
                    }
                }
                self.poke();
            }
        }
    }

    /// Riff's own keys: space, arrows, and a few letters.
    fn on_key_riff(&mut self, el: &ActiveEventLoop, event: KeyEvent) {
        if self.program_prompt {
            if matches!(&event.logical_key, Key::Named(NamedKey::Escape)) && self.command.is_empty()
            {
                self.program_prompt = false;
                self.poke();
                return;
            }
            self.on_key_action(el, event);
            return;
        }
        let outcome = match &event.logical_key {
            Key::Named(NamedKey::Escape) => {
                self.program_prompt = true;
                self.command.clear();
                self.command_error.clear();
                self.poke();
                return;
            }
            Key::Named(NamedKey::Space) => Some(self.run_riff(RiffAction::Pause)),
            Key::Named(NamedKey::ArrowLeft) => Some(self.run_riff(RiffAction::Prev)),
            Key::Named(NamedKey::ArrowRight) => Some(self.run_riff(RiffAction::Next)),
            Key::Named(NamedKey::ArrowUp) => {
                self.volume_step(5);
                None
            }
            Key::Named(NamedKey::ArrowDown) => {
                self.volume_step(-5);
                None
            }
            Key::Character(text) if text.eq_ignore_ascii_case("i") => {
                Some(self.run_riff(RiffAction::Insert))
            }
            Key::Character(text) if text.eq_ignore_ascii_case("s") => {
                Some(self.run_riff(RiffAction::Stop))
            }
            Key::Character(text) if text.eq_ignore_ascii_case("r") => {
                Some(self.run_riff(RiffAction::Random(None)))
            }
            _ => None,
        };
        match outcome {
            Some(Ok(message)) if !message.is_empty() => self.set_status(message),
            Some(Err(e)) => self.set_status(e),
            _ => {}
        }
        self.poke();
    }

    /// The insert browser: arrows, enter, backspace, i.
    fn on_key_riff_insert(&mut self, event: KeyEvent) {
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => {
                self.riff.explorer = None;
                self.close_explorer_into_player();
            }
            Key::Named(NamedKey::Enter) => self.explorer_enter(),
            Key::Named(NamedKey::ArrowUp) => self.explorer_move(-1),
            Key::Named(NamedKey::ArrowDown) => self.explorer_move(1),
            Key::Named(NamedKey::Backspace) | Key::Named(NamedKey::ArrowLeft) => {
                self.explorer_up();
            }
            Key::Character(text) if text.eq_ignore_ascii_case("i") => {
                if let Err(e) = self.explorer_insert_folder() {
                    self.set_status(e);
                }
            }
            _ => {}
        }
        self.poke();
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
            // The space bar arrives as a named key, not as the character " ".
            Key::Named(NamedKey::Space) => KeyPress::Char(' '),
            Key::Named(NamedKey::Control) => match event.physical_key {
                PhysicalKey::Code(KeyCode::ControlLeft) => KeyPress::ControlLeft,
                PhysicalKey::Code(KeyCode::ControlRight) => KeyPress::ControlRight,
                _ => return None,
            },
            _ => return None,
        })
    }

    /// Pointer position in emulated raster pixels.
    fn raster_point(&self) -> (i32, i32) {
        let (ox, oy) = self.program_origin();
        match self.vp.as_ref() {
            Some(vp) => {
                let scale = vp.scale.max(1) as f64;
                (
                    ((self.pointer.0 - vp.offset_x as f64) / scale) as i32 - ox,
                    ((self.pointer.1 - vp.offset_y as f64) / scale) as i32 - oy,
                )
            }
            None => (self.pointer.0 as i32 - ox, self.pointer.1 as i32 - oy),
        }
    }

    /// The stage's pixel origin: the cell origin from `program_stage`, in the
    /// framebuffer's pixels.
    fn program_origin(&self) -> (i32, i32) {
        let (cols, rows) = match self.vp.as_ref() {
            Some(vp) => (
                vp.virt_w / cread_core::FONT_W,
                vp.virt_h / cread_core::FONT_H,
            ),
            None => (self.screen.cols(), self.screen.rows()),
        };
        let (x, y) = self.program_stage(cols, rows);
        (
            (x * cread_core::FONT_W) as i32,
            (y * cread_core::FONT_H) as i32,
        )
    }

    /// Hand a pointer event to the running program, if it should get one.
    fn program_pointer(&mut self, kind: PointerKind) -> bool {
        if self.mode != Mode::Program {
            return false;
        }
        // The prompt takes presses, but a release still has to reach the
        // program or a brush stays glued to the canvas.
        if self.program_prompt && !matches!(kind, PointerKind::Up(_)) {
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
                } else if self.home == Home::Poker {
                    self.start_poker();
                } else if self.home == Home::Scripture {
                    self.start_scripture();
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
                if matches!(self.mode, Mode::Program | Mode::Riff) {
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

    #[allow(clippy::wrong_self_convention)]
    /// Has the machine been quiet long enough to rest?
    fn screensaver_due(&self) -> bool {
        self.screensaver_on
            && self.note.is_none()
            && !matches!(self.mode, Mode::Screensaver | Mode::Reminder)
            && self.boot_at.is_none()
            && !matches!(self.mode, Mode::Loading | Mode::Checking)
            // A running cartridge keeps playing while the clock covers it, and
            // the waking key never reaches the pad. Do not rest on a live game.
            && !(self.mode == Mode::Program
                && self
                    .program
                    .as_ref()
                    .is_some_and(|program| program.wants_frames()))
            && self.last_input.elapsed()
                >= Duration::from_secs(u64::from(self.screensaver_minutes) * 60)
    }

    /// Where a program's classic 640x400 stage sits in the current grid.
    ///
    /// Programs are written for 80x25 cells; a bigger framebuffer gets the whole
    /// stage centred instead of pinned to the top-left. Softhand is 24 rows tall
    /// and wears the top bar, so its stage is centred below that bar.
    fn program_stage(&self, cols: u32, rows: u32) -> (u32, u32) {
        let stage_rows = self.program_stage_height();
        let top = if self.program_id == Some("softhand") {
            1
        } else {
            0
        };
        (
            cols.saturating_sub(STAGE_COLS) / 2,
            top + rows.saturating_sub(top + stage_rows) / 2,
        )
    }

    /// Rows the current program's stage occupies, strip and legend included.
    fn program_stage_height(&self) -> u32 {
        if self.program_id == Some("softhand") {
            SOFTHAND_ROWS
        } else {
            STAGE_ROWS
        }
    }

    /// The binary to start after an update. Captured at startup on purpose:
    /// `install` renames the running binary to `cread.old`, and from then on
    /// `/proc/self/exe` follows the inode - `current_exe()` would return the
    /// old file and the restart would run the previous version.
    fn restart_target(&self) -> Option<PathBuf> {
        self.self_exe
            .clone()
            .or_else(|| std::env::current_exe().ok())
    }

    /// A key woke the clock. True when it did, so the key is swallowed instead
    /// of also typing or opening something.
    fn wake_screensaver(&mut self) -> bool {
        if self.mode != Mode::Screensaver {
            return false;
        }
        self.mode = self.screensaver_back;
        self.last_input = Instant::now();
        self.dirty = true;
        true
    }

    /// A small confirmation box; any key closes it and returns.
    pub(super) fn open_notice(&mut self, message: impl Into<String>, back: Mode) {
        self.notice_text = message.into();
        self.notice_back = back;
        self.mode = Mode::Notice;
        self.dirty = true;
    }

    fn on_key_notice(&mut self) {
        self.mode = self.notice_back;
        self.dirty = true;
    }

    /// Hide the screen behind the clock. Any key brings back what was there.
    pub(super) fn start_screensaver(&mut self) {
        if self.mode == Mode::Screensaver {
            return;
        }
        self.screensaver_back = self.mode;
        self.mode = Mode::Screensaver;
        self.last_input = Instant::now();
        self.hide_pointer();
        self.dirty = true;
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
        if matches!(self.mode, Mode::RiffInsert) {
            self.riff.explorer = None;
            self.mode = Mode::Riff;
            self.program_prompt = false;
            self.dirty = true;
            return;
        }
        if self.mode == Mode::Riff
            || (!self.has_document && self.home == Home::Riff && self.mode == Mode::Action)
        {
            self.mode = Mode::Riff;
            self.program_prompt = false;
            self.command.clear();
            self.command_error.clear();
            self.dirty = true;
            return;
        }
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
        self.prefs.screensaver = self.screensaver_on;
        self.prefs.screensaver_minutes = self.screensaver_minutes;
        self.prefs.clock_24h = self.clock_24h;
        self.prefs.date_day_first = self.date_day_first;
        if !self.persist {
            return;
        }
        if let Err(e) = self.prefs.save() {
            eprintln!("cread: cannot save preferences: {e}");
        }
    }

    // ── mouse ───────────────────────────────────────────────────────────────

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

    fn beep_twice(&mut self) {
        if self.beep_on {
            crate::beep::play_twice();
        }
    }

    fn poke(&mut self) {
        // Render as soon as the event loop comes back around.
        self.next_frame = Instant::now();
        self.dirty = true;
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
                let id = PROGRAMS.get(index).map(|p| p.id).unwrap_or_default();
                if id == "pocketboi" {
                    self.program = Some(Box::new(cread_pocketboi::Pocketboi::new(&self.vault())));
                    self.program_id = Some("pocketboi");
                    self.mode = Mode::Program;
                    self.program_prompt = false;
                    self.command.clear();
                    self.dirty = true;
                    return;
                }
                self.home = program_home(index);
                // A disc is still in the tray: Riff opens on the player, not on
                // the front door it was left from.
                if self.home == Home::Riff && !self.riff.playlist.is_empty() {
                    self.mode = Mode::Riff;
                } else {
                    self.mode = Mode::Action;
                }
                self.command.clear();
            }
            Pending::NewPicture => {
                self.new_picture();
            }
            Pending::Rom(path) => {
                if !self.is_installed("pocketboi") {
                    self.set_status("pocketboi is not installed - see SOFTWARE");
                    self.mode = Mode::Browser;
                    return;
                }
                let mut boi = cread_pocketboi::Pocketboi::new(&self.vault());
                match boi.play(&path) {
                    Ok(()) => {
                        self.program = Some(Box::new(boi));
                        self.program_id = Some("pocketboi");
                        self.mode = Mode::Program;
                        self.program_prompt = false;
                        self.command.clear();
                        self.dirty = true;
                    }
                    Err(e) => {
                        self.set_status(e);
                        self.mode = Mode::Browser;
                    }
                }
            }
            Pending::Picture(path) => {
                // Whatever program was in front lets go first, so a cartridge
                // writes its battery before the painting takes the screen.
                self.close_program();
                match self.open_picture(path) {
                    Ok(()) => {}
                    Err(e) => {
                        self.set_status(e);
                        self.home = Home::SoftHand;
                        self.mode = Mode::Action;
                    }
                }
            }
            Pending::Document(path) => match self.open_document(path) {
                Ok(()) => {
                    // A document belongs to the editor, not to whatever program
                    // happened to be in front when `open` was typed.
                    self.close_program();
                    self.mode = Mode::Write;
                }
                Err(e) => {
                    self.set_status(e);
                    self.mode = Mode::Action;
                }
            },
            Pending::Software { id, install } => {
                self.apply_software(id, install);
                self.mode = Mode::Software;
            }
            Pending::NewDocument => match self.new_document() {
                Ok(msg) => {
                    self.close_program();
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

    /// Open the machine from its front door.
    fn start_poker(&mut self) {
        let poker = if self.persist {
            cread_poker::Poker::new()
        } else {
            // The window and exit paths cash out in Drop; tests must never
            // write the real wallet from a temporary app.
            let path =
                std::env::temp_dir().join(format!("cread-poker-app-{}.save", std::process::id()));
            cread_poker::Poker::new_at(path)
        };
        self.show_poker(poker);
    }

    fn show_poker(&mut self, program: cread_poker::Poker) {
        self.program = Some(Box::new(program));
        self.program_id = Some("poker");
        self.home = Home::Poker;
        self.mode = Mode::Program;
        self.program_prompt = false;
        self.command.clear();
        self.command_error.clear();
        self.dirty = true;
    }

    /// Leaving the machine returns to its own front door, not the boot list:
    /// the wallet and highscore are the next thing the player wants to see.
    fn leave_poker(&mut self) {
        self.close_program();
        self.picture_path = None;
        self.program_prompt = false;
        self.program_point = (-1, -1);
        self.mode = Mode::Action;
        self.command.clear();
        self.command_error.clear();
        self.dirty = true;
    }

    /// Open the Bible reader from its cover.
    fn start_scripture(&mut self) {
        let scripture = if self.persist && self.opts.screenshot.is_none() {
            cread_scripture::Scripture::new()
        } else {
            let path = std::env::temp_dir()
                .join(format!("cread-scripture-app-{}.state", std::process::id()));
            cread_scripture::Scripture::new_at(path, false)
        };
        self.program = Some(Box::new(scripture));
        self.program_id = Some("scripture");
        self.home = Home::Scripture;
        self.mode = Mode::Program;
        self.program_prompt = false;
        self.command.clear();
        self.command_error.clear();
        self.dirty = true;
    }

    /// Leaving the reader returns to the CRead welcome screen, like every other
    /// module. The cover is only the way in, never the way out.
    fn leave_scripture(&mut self) {
        self.close_program();
        self.picture_path = None;
        self.program_prompt = false;
        self.program_point = (-1, -1);
        self.home = Home::Boot;
        self.mode = Mode::Action;
        self.command.clear();
        self.command_error.clear();
        self.dirty = true;
    }

    /// Drop everything and stand on the boot screen again.
    fn leave_to_boot(&mut self) {
        self.close_program();
        self.picture_path = None;
        self.program_prompt = false;
        self.program_point = (-1, -1);
        self.discard_scratch();
        self.doc = Document {
            path: None,
            title_hint: None,
            editor: Editor::new(),
            last_edit: Instant::now(),
        };
        self.has_document = false;
        self.home = Home::Boot;
        self.mode = Mode::Action;
        self.command.clear();
        self.command_error.clear();
        self.dirty = true;
    }

    /// Drop the open program. Dropping is also when a cartridge writes its
    /// battery save, so the shell marks the vault as changed first.
    fn close_program(&mut self) {
        if self
            .program
            .as_ref()
            .is_some_and(|program| program.wrote_files())
        {
            self.unsynced = true;
        }
        self.program = None;
        self.program_id = None;
    }

    /// A fresh canvas on an empty sheet.
    fn new_picture(&mut self) {
        self.program = Some(Box::new(Softhand::new()));
        self.program_id = Some("softhand");
        self.picture_path = None;
        self.program_prompt = false;
        self.mode = Mode::Program;
        self.dirty = true;
    }

    fn open_picture(&mut self, path: PathBuf) -> Result<(), String> {
        let image = picture::load(&path)?;
        self.program = Some(Box::new(Softhand::with_picture(&image)));
        self.program_id = Some("softhand");
        let resized = self
            .program
            .as_ref()
            .is_some_and(|program| program.modified());
        // A cropped or padded source is a new canvas, not the file on disk.
        // Keeping the old path would overwrite the original on quit.
        self.picture_path = (!resized).then_some(path.clone());
        self.program_prompt = false;
        self.mode = Mode::Program;
        self.dirty = true;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.set_status(if resized {
            format!("opened {name} (resized copy - save as new)")
        } else {
            format!("opened {name}")
        });
        Ok(())
    }

    /// Write the running program's picture. `None` keeps the current name, or
    /// picks the next free one.
    fn save_picture(&mut self, name: Option<&str>) -> Result<String, String> {
        let vault = self.vault();
        let path = match name {
            Some(name) => {
                let path = picture::resolve(&vault, name)?;
                if path.exists() && self.picture_path.as_deref() != Some(path.as_path()) {
                    return Err(format!(
                        "{} already exists",
                        path.file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default()
                    ));
                }
                path
            }
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
        if let Some(program) = self.program.as_mut() {
            program.mark_saved();
        }
        self.unsynced = true;
        self.picture_path = Some(path.clone());
        Ok(format!(
            "saved {}",
            path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        ))
    }

    /// True while Softhand is the thing in front: its front door or its canvas.
    fn in_softhand(&self) -> bool {
        self.program_id == Some("softhand") || (!self.has_document && self.home == Home::SoftHand)
    }

    // ── riff ────────────────────────────────────────────────────────────────

    /// The output device, opened the first time music is asked for.
    fn audio(&mut self) -> Result<&mut crate::audio::Audio, String> {
        if self.audio.is_none() {
            self.audio = Some(crate::audio::Audio::open(self.volume)?);
        }
        self.audio
            .as_mut()
            .ok_or_else(|| "audio output is not available".to_string())
    }

    /// Start whatever the playlist points at.
    fn riff_start_current(&mut self) -> Result<String, String> {
        let Some(track) = self.riff.playlist.current().cloned() else {
            return Err("no music inserted".into());
        };
        let volume = self.volume;
        {
            let audio = self.audio()?;
            audio.set_volume(volume);
            audio.play(&track.path)?;
        }
        self.riff.playing = true;
        Ok(format!("\u{266b} {}", track.title))
    }

    /// Fill an empty playlist from the folder or file inserted last time.
    fn riff_ensure_loaded(&mut self) -> Result<(), String> {
        if !self.riff.playlist.is_empty() {
            return Ok(());
        }
        let last = self.prefs.riff_last.clone();
        if last.is_empty() {
            return Err("no music - open Riff and insert a folder".into());
        }
        let path = PathBuf::from(&last);
        if !path.exists() {
            return Err(format!("the inserted music is gone: {last}"));
        }
        let tracks = if path.is_dir() {
            music::tracks_in(&path)
        } else {
            vec![music::Track::from_path(path.clone())]
        };
        if tracks.is_empty() {
            return Err(format!("no music in {last}"));
        }
        self.riff.playlist = music::Playlist::load(tracks, 0);
        self.riff.playlist.set_shuffle(self.prefs.riff_random);
        self.riff.source = last;
        Ok(())
    }

    /// Put a folder or a single file into the player and start it at `index`.
    fn riff_load(&mut self, source: &Path, index: usize) -> Result<String, String> {
        let tracks = if source.is_dir() {
            music::tracks_in(source)
        } else {
            vec![music::Track::from_path(source.to_path_buf())]
        };
        if tracks.is_empty() {
            return Err(format!("no music in {}", source.display()));
        }
        self.riff.playlist = music::Playlist::load(tracks, index);
        self.riff.playlist.set_shuffle(self.prefs.riff_random);
        self.riff.source = source.display().to_string();
        self.prefs.riff_last = self.riff.source.clone();
        self.save_prefs();
        self.riff_start_current()
    }

    fn run_riff(&mut self, action: RiffAction) -> Result<String, String> {
        if !self.is_installed("riff") {
            return Err("Riff is not installed - open SOFTWARE".into());
        }
        match action {
            RiffAction::Insert => {
                if !self.riff_is_front() {
                    return Err("open Riff to insert a disc".into());
                }
                self.open_explorer();
                Ok(String::new())
            }
            RiffAction::Status => Ok(self.riff_status()),
            RiffAction::Volume(want) => {
                let next = want.unwrap_or(self.volume);
                self.set_volume(next);
                Ok(format!("volume {}", self.volume))
            }
            RiffAction::Random(want) => {
                let on = want.unwrap_or(!self.riff.playlist.shuffle());
                self.riff.playlist.set_shuffle(on);
                self.prefs.riff_random = on;
                self.save_prefs();
                Ok(if on {
                    "random on".into()
                } else {
                    "random off".to_string()
                })
            }
            RiffAction::Pause => {
                let Some(audio) = self.audio.as_mut() else {
                    return Err("nothing playing".into());
                };
                if audio.finished() {
                    return Err("nothing playing".into());
                }
                if audio.is_paused() {
                    audio.resume();
                    self.riff.playing = true;
                    Ok("playing".into())
                } else {
                    audio.pause();
                    self.riff.playing = false;
                    Ok("paused".into())
                }
            }
            RiffAction::Stop => {
                if let Some(audio) = self.audio.as_mut() {
                    audio.stop();
                }
                self.riff.playing = false;
                Ok("stopped".into())
            }
            RiffAction::Play(name) => {
                self.riff_ensure_loaded()?;
                if let Some(name) = name {
                    let needle = name.to_lowercase();
                    let found = self
                        .riff
                        .playlist
                        .tracks()
                        .iter()
                        .position(|track| track.title.to_lowercase().contains(&needle))
                        .ok_or_else(|| format!("no track called {name}"))?;
                    self.riff.playlist.select(found);
                    return self.riff_start_current();
                }
                let idle = self
                    .audio
                    .as_ref()
                    .map(|audio| audio.finished() || audio.is_paused())
                    .unwrap_or(true);
                if self.riff.playing && !idle {
                    return Ok("already playing".into());
                }
                self.riff_start_current()
            }
            step @ (RiffAction::Next | RiffAction::Prev) => {
                self.riff_ensure_loaded()?;
                let forward = matches!(step, RiffAction::Next);
                let moved = if forward {
                    self.riff.playlist.next()
                } else {
                    self.riff.playlist.prev()
                };
                if !moved {
                    return Err(if forward {
                        "end of the disc".into()
                    } else {
                        "start of the disc".into()
                    });
                }
                self.riff_start_current()
            }
        }
    }

    /// `♪ Morgenlicht · 00:42` while a track is loaded, for the status bars of
    /// the other screens. Riff's player shows this itself, so it is left out
    /// there.
    fn now_playing(&self) -> Option<String> {
        if self.mode == Mode::Riff {
            return None;
        }
        let track = self.riff.playlist.current()?;
        let audio = self.audio.as_ref()?;
        if audio.finished() {
            return None;
        }
        Some(format!(
            "\u{266a} {} \u{00b7} {}",
            track.title,
            crate::audio::clock(audio.position())
        ))
    }

    fn riff_status(&self) -> String {
        match self.riff.playlist.current() {
            Some(track) => {
                let (at, of) = self.riff.playlist.position();
                let state = if self.riff.playing {
                    "playing"
                } else {
                    "paused"
                };
                format!("\u{266b} {} ({at} of {of}) - {state}", track.title)
            }
            None => "riff: no disc inserted".into(),
        }
    }

    fn set_volume(&mut self, volume: u8) {
        self.volume = volume.min(100);
        if let Some(audio) = self.audio.as_mut() {
            audio.set_volume(self.volume);
        }
        self.prefs.volume = self.volume;
        self.save_prefs();
        self.dirty = true;
    }

    fn volume_step(&mut self, by: i32) {
        let next = (self.volume as i32 + by).clamp(0, 100) as u8;
        self.set_volume(next);
    }

    /// True while Riff is the thing in front.
    fn riff_is_front(&self) -> bool {
        matches!(self.mode, Mode::Riff | Mode::RiffInsert)
            || (!self.has_document && self.home == Home::Riff && self.mode == Mode::Action)
    }

    fn open_explorer(&mut self) {
        let mut browser = Browser::music(music::music_dir());
        if let Err(e) = browser.refresh() {
            self.set_status(e);
        }
        self.riff.explorer = Some(browser);
        self.mode = Mode::RiffInsert;
        self.program_prompt = false;
        self.dirty = true;
    }

    fn explorer_move(&mut self, delta: isize) {
        let Some(browser) = self.riff.explorer.as_mut() else {
            return;
        };
        browser.step(delta);
        self.poke();
    }

    /// Enter in the music browser: the same accept the vault uses, on the
    /// browser the explorer owns. It swaps into the shell's slot so there is
    /// one implementation, then puts it back if the screen is still the list.
    fn explorer_enter(&mut self) {
        let Some(browser) = self.riff.explorer.take() else {
            return;
        };
        let vault_root = self.vault();
        self.browser = browser;
        self.browser_accept();
        if self.mode == Mode::RiffInsert {
            let browser = std::mem::replace(&mut self.browser, Browser::vault(vault_root));
            self.riff.explorer = Some(browser);
        } else {
            // The track took over; do not leave the music shelf in the slot the
            // vault browser expects.
            self.browser = Browser::vault(vault_root);
        }
    }

    fn explorer_up(&mut self) {
        if let Some(browser) = self.riff.explorer.as_mut()
            && browser.path.as_os_str().is_empty()
        {
            // At the music root: nowhere to go, and never into the home folder.
            return;
        }
        if let Some(browser) = self.riff.explorer.as_mut() {
            browser.path = browser
                .path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_default();
            browser.selected = 0;
            if let Err(e) = browser.refresh() {
                self.set_status(e);
            }
        }
        self.poke();
    }

    /// What `i` would insert: the highlighted folder if the cursor is on one,
    /// otherwise the folder being browsed.
    fn explorer_insert_target(&self) -> Option<PathBuf> {
        let browser = self.riff.explorer.as_ref()?;
        Some(match browser.current()? {
            Action::EnterDir(relative) => browser.root.join(relative),
            _ => browser.dir(),
        })
    }

    fn explorer_insert_folder(&mut self) -> Result<(), String> {
        let Some(dir) = self.explorer_insert_target() else {
            return Ok(());
        };
        let message = self.riff_load(&dir, 0)?;
        self.close_explorer_into_player();
        self.set_status(message);
        Ok(())
    }

    fn close_explorer_into_player(&mut self) {
        self.riff.explorer = None;
        self.mode = Mode::Riff;
        self.program_prompt = false;
        self.dirty = true;
    }

    /// One second of music time: advance at the end of a track, redraw the
    /// counter while playing.
    fn riff_tick(&mut self) -> bool {
        if !self.riff.playing {
            return false;
        }
        if self.audio.as_ref().is_none_or(|audio| !audio.finished()) {
            let second = self
                .audio
                .as_ref()
                .map(|audio| audio.position().as_secs())
                .unwrap_or(0);
            if second != self.riff.second {
                self.riff.second = second;
                self.dirty = true;
            }
            return true;
        }
        if self.riff.playlist.next() {
            let _ = self.riff_start_current();
        } else {
            self.riff.playing = false;
            if let Some(audio) = self.audio.as_mut() {
                audio.stop();
            }
            self.set_status("end of the disc");
        }
        true
    }

    // ── software ────────────────────────────────────────────────────────────

    fn is_installed(&self, id: &str) -> bool {
        self.prefs.installed.iter().any(|have| have == id)
    }

    /// One row per module for the SOFTWARE screen.
    fn software_rows(&self) -> Vec<ui::MenuItem> {
        let mut rows = vec![ui::MenuItem::row(
            "INSPIRO",
            "where your ideas have room to grow.",
            "system",
        )];
        for program in PROGRAMS {
            rows.push(ui::MenuItem::row(
                program.entry,
                program.tagline,
                if self.is_installed(program.id) {
                    "installed"
                } else {
                    "install"
                },
            ));
        }
        rows
    }

    /// The system settings, in the order they belong in: sync first, then the
    /// look, then the clock, then the machine's own business. Rules split the
    /// groups; they are never selectable.
    fn setup_entries(&self) -> Vec<(ui::MenuItem, Option<SetupAction>)> {
        use SetupAction as Action;
        let sync = if !self.sync_cfg.enabled {
            "off"
        } else if self.sync_cfg.url.is_empty() {
            "no server"
        } else {
            "on"
        };
        let fullscreen = self
            .window
            .as_ref()
            .map(|window| window.fullscreen().is_some())
            .unwrap_or(false);
        let update = match &self.update_status {
            UpdateStatus::Unknown => ui::VERSION.to_string(),
            UpdateStatus::UpToDate => "up to date".to_string(),
            UpdateStatus::Available(version) => format!("{version} available"),
        };
        vec![
            (
                ui::MenuItem::row("WEBDAV", "sync the whole vault", sync),
                Some(Action::Webdav),
            ),
            (ui::MenuItem::rule(), None),
            (
                ui::MenuItem::row("THEME", "colour scheme", self.theme.name),
                Some(Action::Theme),
            ),
            (
                ui::MenuItem::row("FONT", "bitmap face", self.font.face().name()),
                Some(Action::Font),
            ),
            (
                ui::MenuItem::row(
                    "SCREEN",
                    "window or whole screen",
                    if fullscreen { "fullscreen" } else { "window" },
                ),
                Some(Action::Screen),
            ),
            (
                ui::MenuItem::row(
                    "SCREENSAVER",
                    "clock when idle",
                    if self.screensaver_on { "on" } else { "off" },
                ),
                Some(Action::Screensaver),
            ),
            (
                ui::MenuItem::row(
                    "IDLE",
                    "until it starts",
                    format!("{} min", self.screensaver_minutes),
                ),
                Some(Action::Idle),
            ),
            (ui::MenuItem::rule(), None),
            (
                ui::MenuItem::row(
                    "TIME",
                    "clock format",
                    if self.clock_24h {
                        "24 hours"
                    } else {
                        "12 hours"
                    },
                ),
                Some(Action::Time),
            ),
            (
                ui::MenuItem::row(
                    "DATE",
                    "date format",
                    if self.date_day_first {
                        "DD.MM.YY"
                    } else {
                        "MM/DD/YY"
                    },
                ),
                Some(Action::Date),
            ),
            (ui::MenuItem::rule(), None),
            (
                ui::MenuItem::row("INFO", "CRead OS and device info", ui::VERSION),
                Some(Action::Info),
            ),
            (
                ui::MenuItem::row("UPDATE", "check github for a new version", update),
                Some(Action::Update),
            ),
            (
                ui::MenuItem::row("SOFTWARE", "install and remove modules", ""),
                Some(Action::Software),
            ),
            (
                ui::MenuItem::row(
                    "STORAGE",
                    storage_detail(self.space),
                    storage_value(self.space),
                ),
                Some(Action::Storage),
            ),
            (
                ui::MenuItem::row(
                    "NETWORK",
                    "local address",
                    self.machine
                        .as_ref()
                        .map(Machine::ip_label)
                        .unwrap_or_else(|| "no address".into()),
                ),
                Some(Action::Network),
            ),
            (
                ui::MenuItem::row("USER MANUAL", "read the manual", ""),
                Some(Action::Manual),
            ),
        ]
    }

    fn setup_rows(&self) -> Vec<ui::MenuItem> {
        self.setup_entries()
            .into_iter()
            .map(|(row, _)| row)
            .collect()
    }

    /// Move the highlight, skipping the rules.
    fn setup_step(&mut self, delta: isize) {
        let entries = self.setup_entries();
        let mut index = self.setup_selected as isize;
        loop {
            index += delta;
            if index < 0 || index >= entries.len() as isize {
                return;
            }
            if entries[index as usize].1.is_some() {
                self.setup_selected = index as usize;
                self.poke();
                return;
            }
        }
    }

    fn open_software(&mut self) {
        self.mode = Mode::Software;
        self.software_selected = self.software_selected.min(PROGRAMS.len());
        self.command.clear();
        self.program_prompt = false;
        self.dirty = true;
    }

    fn open_setup(&mut self) {
        self.refresh_space();
        self.refresh_machine();
        self.mode = Mode::Setup;
        let entries = self.setup_entries();
        if self.setup_selected >= entries.len() || entries[self.setup_selected].1.is_none() {
            self.setup_selected = entries
                .iter()
                .position(|(_, action)| action.is_some())
                .unwrap_or(0);
        }
        self.command.clear();
        self.program_prompt = false;
        self.dirty = true;
    }

    fn refresh_machine(&mut self) {
        self.machine = Some(Machine::read());
    }

    fn refresh_space(&mut self) {
        let vault = self.vault();
        self.space = space::at(&vault).or_else(|| space::at(std::path::Path::new("/")));
    }

    /// What Enter does on the SETTINGS screen.
    fn setup_activate(&mut self) {
        let Some(action) = self
            .setup_entries()
            .get(self.setup_selected)
            .and_then(|(_, action)| *action)
        else {
            return;
        };
        match action {
            SetupAction::Webdav => {
                // WebDAV is a form, not a value in a row.
                self.open_webdav();
            }
            SetupAction::Theme => {
                let _ = self.cycle_theme(1);
            }
            SetupAction::Font => {
                let next = match self.font.face() {
                    Face::Classic => Face::Thin,
                    Face::Thin => Face::Classic,
                };
                self.font = Font::with_face(next);
                self.prefs.font = next.name().to_string();
                self.save_prefs();
                self.dirty = true;
                self.set_status(format!("font: {}", next.name()));
            }
            SetupAction::Screen => {
                if let Some(window) = self.window.as_ref() {
                    let going_fullscreen = window.fullscreen().is_none();
                    window.set_fullscreen(going_fullscreen.then_some(Fullscreen::Borderless(None)));
                    self.set_status(if going_fullscreen {
                        "fullscreen"
                    } else {
                        "windowed"
                    });
                }
            }
            SetupAction::Screensaver => {
                self.screensaver_on = !self.screensaver_on;
                self.save_prefs();
                self.set_status(format!("screensaver: {}", on_off(self.screensaver_on)));
                self.dirty = true;
            }
            SetupAction::Idle => {
                self.screensaver_minutes = next_idle_minutes(self.screensaver_minutes);
                self.save_prefs();
                self.set_status(format!(
                    "screensaver after {} min",
                    self.screensaver_minutes
                ));
                self.dirty = true;
            }
            SetupAction::Time => {
                self.clock_24h = !self.clock_24h;
                self.save_prefs();
                self.set_status(if self.clock_24h {
                    "clock: 24 hours"
                } else {
                    "clock: 12 hours"
                });
                self.dirty = true;
            }
            SetupAction::Date => {
                self.date_day_first = !self.date_day_first;
                self.save_prefs();
                self.set_status(if self.date_day_first {
                    "date: DD.MM.YY"
                } else {
                    "date: MM/DD/YY"
                });
                self.dirty = true;
            }
            SetupAction::Info => {
                self.help_about = true;
                self.help_from_setup = true;
                self.mode = Mode::Help;
                self.dirty = true;
            }
            SetupAction::Update => self.start_update_check(),
            SetupAction::Software => self.open_software(),
            SetupAction::Storage => {
                self.refresh_space();
                self.dirty = true;
            }
            SetupAction::Network => {
                self.refresh_machine();
                self.dirty = true;
            }
            SetupAction::Manual => self.open_manual(Mode::Setup),
        }
    }

    fn on_key_setup(&mut self, event: KeyEvent) {
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => self.to_write(),
            Key::Named(NamedKey::Enter) => self.setup_activate(),
            Key::Named(NamedKey::ArrowUp) => self.setup_step(-1),
            Key::Named(NamedKey::ArrowDown) => self.setup_step(1),
            _ => {}
        }
    }

    /// Enter on the SOFTWARE screen: install or remove, with the bar in between.
    fn software_activate(&mut self) {
        // Row 0 is Inspiro, the system; it cannot be removed.
        if self.software_selected == 0 {
            self.set_status("Inspiro is the system - it stays");
            return;
        }
        let Some(program) = PROGRAMS.get(self.software_selected - 1) else {
            return;
        };
        let install = !self.is_installed(program.id);
        self.start_loading(Pending::Software {
            id: program.id,
            install,
        });
    }

    /// The change itself, once the bar has run.
    fn apply_software(&mut self, id: &'static str, install: bool) {
        let Some(program) = PROGRAMS.iter().find(|program| program.id == id) else {
            return;
        };
        // A row is about to appear or vanish above the cursor: remember which
        // row it was on, not where, and find it again afterwards.
        let selected = self.boot_items().get(self.boot_selected).copied();
        if install {
            if !self.is_installed(id) {
                self.prefs.installed.push(id.to_string());
            }
        } else {
            if program.id == "riff" {
                // The deck is the shell's; removing the player silences it.
                if let Some(audio) = self.audio.as_mut() {
                    audio.stop();
                }
                self.riff.playing = false;
            }
            self.prefs.installed.retain(|have| have != id);
        }
        self.save_prefs();
        self.set_status(format!(
            "{} {}",
            if install { "installed" } else { "removed" },
            program.entry
        ));
        match selected.and_then(|item| self.boot_items().iter().position(|have| *have == item)) {
            Some(index) => self.boot_selected = index,
            None => {
                if self.boot_selected >= self.boot_items().len() {
                    self.boot_selected = 0;
                }
            }
        }
    }

    fn on_key_software(&mut self, event: KeyEvent) {
        match &event.logical_key {
            Key::Named(NamedKey::Escape) | Key::Named(NamedKey::Enter) => {
                if matches!(&event.logical_key, Key::Named(NamedKey::Escape)) {
                    self.to_write();
                } else {
                    self.software_activate();
                }
            }
            Key::Named(NamedKey::ArrowUp) => {
                self.software_selected = self.software_selected.saturating_sub(1);
                self.poke();
            }
            Key::Named(NamedKey::ArrowDown) => {
                let last = PROGRAMS.len();
                self.software_selected = (self.software_selected + 1).min(last);
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
        abbreviate_home(&dir)
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
        self.hide_pointer();
        self.recompute_viewport();
        self.dirty = true;

        // A screenshot wants the settled picture; `--boot` replays the flicker
        // so it can be looked at.
        if self.opts.boot || self.opts.screenshot.is_none() {
            self.boot_at = Some(Instant::now());
        }
        if self.opts.music {
            let _ = self
                .riff_ensure_loaded()
                .and_then(|()| self.riff_start_current());
        }

        // Prepare the vault before the worker's first round: creating the four
        // rooms and promoting a draft are writes the sync must not race.
        self.prepare_vault();
        self.reload_calendar();

        if self.sync_cfg.enabled {
            self.start_worker();
            self.request_sync();
        }

        match self.opts.show.as_str() {
            "palette" => self.open_browser(),
            "settings" => self.open_setup(),
            "webdav" => self.open_webdav(),
            "vault" => self.open_browser(),
            "screensaver" => self.start_screensaver(),
            "about" => {
                self.refresh_machine();
                self.help_about = true;
                self.mode = Mode::Help;
            }
            "updatecheck" => {
                self.update_started = Instant::now();
                self.update_installing = false;
                self.mode = Mode::Checking;
            }
            "updateprompt" => {
                self.update_offer = Some(cread_update::Offer {
                    version: "0.23.0".into(),
                    tarball: "https://example.org/cread.tar.gz".into(),
                    checksum: "https://example.org/cread.sha256".into(),
                    signature: "https://example.org/cread.minisig".into(),
                });
                self.update_status = UpdateStatus::Available("0.23.0".into());
                self.mode = Mode::UpdatePrompt;
            }
            "documents" => self.open_documents(),
            "shutdown" => {
                let _ = self.cmd_shutdown();
            }
            "reboot" => {
                let _ = self.cmd_reboot();
            }
            "backtolinux" => {
                let _ = self.cmd_backtolinux();
            }
            "quit" => {
                self.unsynced = true;
                self.quit = Some(QuitPrompt {
                    yes: true,
                    from: Mode::Action,
                });
                self.mode = Mode::Quit;
            }
            "help" => {
                self.mode = Mode::Help;
            }
            "manual" => self.open_manual(self.mode),
            "calendar" => self.open_calendar(),
            "note" => {
                let _ = self.open_note();
            }
            "calendaredit" => {
                self.open_calendar();
                self.new_event_form();
            }
            "reminder" => {
                self.open_calendar();
                self.reminder_title = "Demo appointment".into();
                self.reminder_when = "Today · 10:30".into();
                self.reminder_date = chrono::Local::now().date_naive();
                self.reminder_back = self.mode;
                self.mode = Mode::Reminder;
            }
            "notice" => {
                self.open_webdav();
                self.open_notice("SYNC ON", Mode::Webdav);
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
            "riff" => {
                self.home = Home::Riff;
                self.mode = Mode::Action;
            }
            "poker" => {
                self.home = Home::Poker;
                self.mode = Mode::Action;
            }
            "scripture" => {
                self.home = Home::Scripture;
                self.mode = Mode::Action;
            }
            "scriptureread" => self.start_scripture(),
            "scripturegrid" => {
                self.start_scripture();
                if let Some(program) = self.program.as_mut() {
                    program.key(KeyPress::Char('c'));
                }
            }
            "pokerplay" => self.start_poker(),
            "pokerhand" => self.show_poker(cread_poker::Poker::demo_hand()),
            "pokerrisk" => self.show_poker(cread_poker::Poker::demo_risk()),
            "pokerover" => self.show_poker(cread_poker::Poker::demo_bust()),
            "riffplay" => {
                if let Err(e) = self
                    .riff_ensure_loaded()
                    .and_then(|()| self.riff_start_current())
                {
                    eprintln!("cread: {e}");
                }
                self.mode = Mode::Riff;
                if !self.opts.command.is_empty() {
                    self.command = self.opts.command.clone();
                    self.program_prompt = true;
                }
            }
            "riffinsert" => self.open_explorer(),
            "pocketboi" => {
                self.program = Some(Box::new(cread_pocketboi::Pocketboi::new(&self.vault())));
                self.program_id = Some("pocketboi");
                self.mode = Mode::Program;
            }
            other if other.starts_with("pocketboi:") => {
                let rom = PathBuf::from(&other["pocketboi:".len()..]);
                let mut boi = cread_pocketboi::Pocketboi::new(&self.vault());
                match boi.play(&rom) {
                    Ok(()) => {
                        self.program = Some(Box::new(boi));
                        self.program_id = Some("pocketboi");
                        self.mode = Mode::Program;
                    }
                    Err(e) => eprintln!("cread: {e}"),
                }
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
            "software" => self.open_software(),
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
            WindowEvent::CloseRequested => {
                // The exit box is already the question; closing the window
                // while it is up means "leave now", exactly as documented.
                if self.mode == Mode::Quit {
                    el.exit();
                    return;
                }
                if self.note.is_some()
                    && let Err(e) = self.save_note()
                {
                    self.set_status(e);
                    self.poke();
                    return;
                }
                // The event form has its own save path; never throw it away
                // because somebody clicked the title-bar close button.
                if self.calendar_form.as_ref().is_some_and(|form| form.dirty) {
                    self.mode = Mode::CalendarEdit;
                    self.set_status("unsaved appointment - SAVE or Esc first");
                    self.poke();
                    return;
                }
                // The window close button is a quit like any other: save what
                // is unsaved, then give sync a chance to catch up.
                if self.has_document
                    && self.doc.editor.modified()
                    && let Err(e) = self.save()
                {
                    self.set_status(e);
                    self.poke();
                    return;
                }
                if self
                    .program
                    .as_ref()
                    .is_some_and(|program| program.modified())
                    && self.picture_path.is_some()
                    && let Err(e) = self.save_picture(None)
                {
                    self.set_status(e);
                    self.poke();
                    return;
                }
                let from = self.mode;
                if !self.ask_quit(from) {
                    el.exit();
                }
            }
            WindowEvent::ModifiersChanged(mods) => self.mods = mods.state(),
            WindowEvent::Focused(false) => {
                // A button released outside the window never reaches us, and
                // modifiers from the old focus must not stay latched.
                self.dragging = false;
                self.mods = ModifiersState::empty();
                self.dirty = true;
                self.poke();
            }
            WindowEvent::Resized(size) => {
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.resize(size.width.max(1), size.height.max(1));
                }
                self.recompute_viewport();
            }
            WindowEvent::ScaleFactorChanged { .. } => self.recompute_viewport(),
            WindowEvent::CursorEntered { .. } => {
                if self.opts.kiosk {
                    self.skip_pointer_motion = true;
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let synthetic = std::mem::take(&mut self.skip_pointer_motion);
                self.last_input = Instant::now();
                if self.mode == Mode::Screensaver {
                    return;
                }
                if !synthetic {
                    self.show_pointer();
                }
                self.pointer = (position.x, position.y);
                if !self.program_pointer(PointerKind::Move) {
                    self.on_mouse_drag();
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                self.last_input = Instant::now();
                if self.mode == Mode::Screensaver {
                    return;
                }
                self.show_pointer();
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
                self.last_input = Instant::now();
                if self.mode == Mode::Screensaver {
                    return;
                }
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
        self.quit = None;
        // Never lose the last keystrokes on quit.
        if self.note.is_some() {
            let _ = self.save_note();
        }
        if self.doc.editor.modified() {
            let _ = self.save();
        }
        self.discard_scratch();
    }
}

/// `/home/you/Music` -> `~/Music`, so a bar does not waste half its width.
fn abbreviate_home(path: &Path) -> String {
    let home = storage::home_dir();
    match path.strip_prefix(&home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
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

/// The SETTINGS row's left-hand text: used and total, or a dash.
fn storage_detail(space: Option<space::Space>) -> String {
    space
        .map(|space| space.used_total())
        .unwrap_or_else(|| "no reading".into())
}

/// The SETTINGS row's right-hand text: the bar and the percentage.
fn storage_value(space: Option<space::Space>) -> String {
    space
        .map(|space| format!("{} {}%", space.bar(), space.percent()))
        .unwrap_or_else(|| "\u{2014}".into())
}

#[cfg(test)]
mod tests;
