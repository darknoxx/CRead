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
mod frame;
mod machine;
mod manual;
mod note;
mod persistence;
mod pointer;
mod power;
mod programs;
mod riff;
mod search;
mod setup;
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

    // ── frame ───────────────────────────────────────────────────────────────

    // ── persistence ─────────────────────────────────────────────────────────

    // ── browser ─────────────────────────────────────────────────────────────

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

    // ── programs ────────────────────────────────────────────────────────────

    // ── riff ────────────────────────────────────────────────────────────────

    // ── software ────────────────────────────────────────────────────────────

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
