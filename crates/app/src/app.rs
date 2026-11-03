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
mod power;
mod sync;
mod update;
mod webdav;

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

/// One selectable point in the manual's contents.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ManualJump {
    /// Source line of the contents entry, for the cursor row.
    line: usize,
    /// What the contents line says (`1. Starting`, `A — Commands …`).
    label: String,
    /// Source line of the heading Enter jumps to.
    target: usize,
}

/// Build the contents menu from the Markdown itself.
///
/// Only real list entries count: the bold `Part …` headings are skipped, as
/// are the Contents heading and the rule that closes the block. The matching
/// heading is the first one after Contents whose text ends with the entry's
/// label, so `Starting` finds `Step 1: Starting` and `A — Commands …` finds
/// `Appendix A — Commands …`.
fn manual_jumps() -> Vec<ManualJump> {
    let lines: Vec<&str> = MANUAL.lines().collect();
    let Some(contents) = lines.iter().position(|line| line.trim() == "## Contents") else {
        return Vec::new();
    };
    let end = lines[contents + 1..]
        .iter()
        .position(|line| line.trim() == "---")
        .map(|offset| contents + 1 + offset)
        .unwrap_or(lines.len());

    let mut entries = Vec::new();
    for (offset, line) in lines[contents + 1..end].iter().enumerate() {
        let text = line.trim();
        let label = if let Some((number, rest)) = text.split_once(". ")
            && number.chars().all(|ch| ch.is_ascii_digit())
            && !rest.is_empty()
        {
            rest
        } else if let Some(rest) = text.strip_prefix("- ")
            && !rest.is_empty()
        {
            rest
        } else {
            continue;
        };
        entries.push((contents + 1 + offset, text.to_string(), label.to_string()));
    }

    let mut jumps = Vec::new();
    for (line, label, search) in entries {
        let wanted = normalize_manual(&search);
        let target = lines
            .iter()
            .enumerate()
            .skip(end + 1)
            .find(|(_, line)| {
                manual_heading(line).is_some_and(|heading| {
                    let heading = normalize_manual(heading);
                    heading == wanted || heading.ends_with(&format!(" {wanted}"))
                })
            })
            .map(|(line, _)| line);
        if let Some(target) = target {
            jumps.push(ManualJump {
                line,
                label,
                target,
            });
        }
    }
    jumps
}

fn manual_heading(line: &str) -> Option<&str> {
    let line = line.trim_start();
    let hashes = line.chars().take_while(|ch| *ch == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    Some(line[hashes..].trim())
}

fn normalize_manual(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

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

    /// Open the built-in manual, rendered for the current width.
    pub(super) fn open_manual(&mut self, back: Mode) {
        let (width, _) = ui::layout(self.screen.cols());
        self.manual_jumps = manual_jumps();
        self.manual_choice = 0;
        self.manual_nav = !self.manual_jumps.is_empty();
        self.manual_top = 0;
        self.render_manual(width);
        // Land on Contents, not on the cover: the first entry is then visible
        // with the cursor already on it. On a very short screen this scrolls
        // just enough to keep the cursor on screen.
        self.manual_top = self.manual_contents_row().unwrap_or(0);
        self.keep_manual_selection_visible();
        self.manual_back = back;
        self.mode = Mode::Manual;
        self.dirty = true;
    }

    /// Render the embedded manual at `width`. The scroll position survives a
    /// resize as far as the new layout still has that row.
    fn render_manual(&mut self, width: usize) {
        if self.manual_width != width || self.manual_rows.is_empty() {
            let lines: Vec<String> = MANUAL.lines().map(str::to_string).collect();
            self.manual_rows = cread_core::markdown::render(&lines, width);
            self.manual_width = width;
        }
        let max = self
            .manual_rows
            .len()
            .saturating_sub(ui::text_height(self.screen.rows()));
        self.manual_top = self.manual_top.min(max);
    }

    fn leave_manual(&mut self) {
        self.mode = self.manual_back;
        self.dirty = true;
    }

    fn manual_contents_row(&self) -> Option<usize> {
        let source = MANUAL
            .lines()
            .position(|line| line.trim() == "## Contents")?;
        self.manual_rows
            .iter()
            .position(|row| row.source_line == source)
    }

    /// The rendered rows where the cursor currently sits (a wrapped entry may
    /// occupy more than one row).
    fn manual_selection_span(&self) -> Option<(usize, usize)> {
        let jump = self.manual_jumps.get(self.manual_choice)?;
        let first = self
            .manual_rows
            .iter()
            .position(|row| row.source_line == jump.line)?;
        let last = self.manual_rows[first..]
            .iter()
            .position(|row| row.source_line != jump.line)
            .map(|offset| first + offset)
            .unwrap_or(self.manual_rows.len());
        Some((first, last))
    }

    fn manual_selection_row(&self) -> Option<usize> {
        self.manual_selection_span().map(|(first, _)| first)
    }

    /// Where Enter should put the top of the page.
    fn manual_target_row(&self) -> Option<usize> {
        let jump = self.manual_jumps.get(self.manual_choice)?;
        self.manual_rows
            .iter()
            .position(|row| row.source_line == jump.target)
    }

    fn keep_manual_selection_visible(&mut self) {
        let Some(row) = self.manual_selection_row() else {
            return;
        };
        let height = ui::text_height(self.screen.rows());
        if height < 4 {
            // The first compose can run before the viewport is known; do not
            // scroll to a menu entry in a one-row screen.
            return;
        }
        if row < self.manual_top {
            self.manual_top = row;
        } else if row >= self.manual_top + height {
            self.manual_top = row.saturating_sub(height.saturating_sub(1));
        }
    }

    fn scroll_manual(&mut self, delta: isize) {
        let height = ui::text_height(self.screen.rows());
        let max = self.manual_rows.len().saturating_sub(height) as isize;
        self.manual_top = (self.manual_top as isize + delta).clamp(0, max.max(0)) as usize;
        // Scrolling away leaves the Contents menu; landing back on the heading
        // puts the cursor on the first entry again.
        let at_contents =
            !self.manual_jumps.is_empty() && Some(self.manual_top) == self.manual_contents_row();
        if at_contents && !self.manual_nav {
            self.manual_choice = 0;
        }
        self.manual_nav = at_contents;
        self.poke();
    }

    fn select_manual(&mut self, delta: isize) {
        if self.manual_jumps.is_empty() {
            self.scroll_manual(delta);
            return;
        }
        let last = self.manual_jumps.len() - 1;
        self.manual_choice = (self.manual_choice as isize + delta).clamp(0, last as isize) as usize;
        self.manual_nav = true;
        self.keep_manual_selection_visible();
        self.poke();
    }

    fn enter_manual_selection(&mut self) {
        let Some(row) = self.manual_target_row() else {
            return;
        };
        self.manual_top = row;
        self.manual_nav = false;
        self.poke();
    }

    fn on_key_manual(&mut self, event: KeyEvent) {
        let page = ui::text_height(self.screen.rows()) as isize;
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => self.leave_manual(),
            Key::Named(NamedKey::ArrowDown) if self.manual_nav => self.select_manual(1),
            Key::Named(NamedKey::ArrowUp) if self.manual_nav => self.select_manual(-1),
            Key::Named(NamedKey::ArrowDown) => self.scroll_manual(1),
            Key::Named(NamedKey::ArrowUp) => self.scroll_manual(-1),
            Key::Named(NamedKey::Enter) if self.manual_nav => self.enter_manual_selection(),
            Key::Named(NamedKey::PageDown) | Key::Named(NamedKey::Space) => {
                self.scroll_manual(page.saturating_sub(1))
            }
            Key::Named(NamedKey::PageUp) => self.scroll_manual(-page.saturating_sub(1)),
            Key::Named(NamedKey::Home) => {
                self.manual_top = self.manual_contents_row().unwrap_or(0);
                self.manual_choice = 0;
                self.manual_nav = !self.manual_jumps.is_empty();
                self.keep_manual_selection_visible();
                self.poke();
            }
            Key::Named(NamedKey::End) => self.scroll_manual(isize::MAX / 2),
            _ => {}
        }
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

    /// A kiosk starts without a pointer. It appears with the first real move
    /// (or click), so a machine with no mouse never shows an arrow.
    fn hide_pointer(&mut self) {
        if !self.opts.kiosk || !self.pointer_visible {
            return;
        }
        if let Some(window) = self.window.as_ref() {
            window.set_cursor_visible(false);
        }
        self.pointer_visible = false;
    }

    fn show_pointer(&mut self) {
        if !self.opts.kiosk || self.pointer_visible {
            return;
        }
        if let Some(window) = self.window.as_ref() {
            window.set_cursor_visible(true);
        }
        self.pointer_visible = true;
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
        if self.mode == Mode::Manual {
            self.scroll_manual(-lines * 3);
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
        self.clipboard
            .as_mut()
            .ok_or_else(|| "clipboard is not available".to_string())
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

    /// Leaving the reader returns to its cover, where the logo and the reading
    /// position are the next thing one expects to see.
    fn leave_scripture(&mut self) {
        self.close_program();
        self.picture_path = None;
        self.program_prompt = false;
        self.program_point = (-1, -1);
        self.home = Home::Scripture;
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
        // Loading is its own destination; anything else drops back to the
        // document or the home screen.
        if self.mode != Mode::Loading {
            self.to_write();
        }
    }

    /// Esc: put the cursor back where the search started.
    fn cancel_search(&mut self) {
        if let Some(search) = self.search.take() {
            self.doc.editor.set_cursor(search.origin);
        }
        // Loading is its own destination; anything else drops back to the
        // document or the home screen.
        if self.mode != Mode::Loading {
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
mod tests {
    use super::boot::BootItem;
    use super::*;

    fn app() -> App {
        let mut app = App::new(Options::default());
        // The developer's own preferences live on this machine; a test wants a
        // fresh shelf and must not write over them.
        app.prefs.installed.clear();
        app.persist = false;
        app
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
        let mut app = App::new(Options {
            vault: Some(dir.to_path_buf()),
            ..Options::default()
        });
        app.prefs.installed.clear();
        app.persist = false;
        app
    }

    #[test]
    fn autosave_is_off_until_it_is_asked_for() {
        let dir = vault("autosave");
        let mut app = app_in(&dir);
        app.new_document().unwrap();
        let path = app.doc.path.clone().unwrap();
        app.mode = Mode::Write;
        app.doc.editor.insert_text("typed but not saved");
        // Pretend the last keystroke was long ago.
        app.doc.last_edit = Instant::now() - AUTOSAVE_DELAY * 2;

        app.autosave_if_due();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "", "still empty");

        app.autosave = true;
        app.autosave_if_due();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "typed but not saved"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn opening_another_document_never_discards_unsaved_text() {
        let dir = vault("open-unsaved");
        let mut app = app_in(&dir);
        app.new_document().unwrap();
        app.doc.editor.insert_text("not saved yet");
        std::fs::create_dir_all(dir.join("Documents")).unwrap();
        std::fs::write(dir.join("Documents/other.md"), "other").unwrap();

        let error = app
            .open_document(dir.join("Documents/other.md"))
            .unwrap_err();
        assert!(error.contains("unsaved"), "{error}");
        assert_eq!(app.doc.editor.text(), "not saved yet");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn saveas_refuses_to_overwrite_an_existing_document() {
        let dir = vault("saveas-existing");
        let mut app = app_in(&dir);
        app.new_document().unwrap();
        app.doc.editor.insert_text("mine");
        std::fs::create_dir_all(dir.join("Documents")).unwrap();
        std::fs::write(dir.join("Documents/other.md"), "theirs").unwrap();

        let error = app.save_as("other").unwrap_err();
        assert!(error.contains("already exists"), "{error}");
        assert_eq!(
            std::fs::read_to_string(dir.join("Documents/other.md")).unwrap(),
            "theirs"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_without_a_document_does_not_create_untitled() {
        let dir = vault("save-no-doc");
        let mut app = app_in(&dir);
        let error = app.save().unwrap_err();
        assert!(error.contains("no document"), "{error}");
        assert!(!dir.join("Documents/untitled.md").exists());
        let _ = std::fs::remove_dir_all(&dir);
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
    fn reopening_riff_with_a_disc_goes_straight_to_the_player() {
        let dir = vault("riff-reopen");
        let mut app = app_in(&dir);
        let index = PROGRAMS.iter().position(|p| p.id == "riff").expect("known");

        app.pending = Some(Pending::Program(index));
        app.finish_loading();
        assert_eq!(app.home, Home::Riff);
        assert_eq!(app.mode, Mode::Action, "an empty tray shows the front door");

        app.riff.playlist =
            music::Playlist::load(vec![music::Track::from_path(dir.join("01.wav"))], 0);
        app.pending = Some(Pending::Program(index));
        app.finish_loading();
        assert_eq!(app.mode, Mode::Riff, "the disc is still in: the player");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn riff_says_what_it_is_missing_and_insert_only_opens_inside_riff() {
        let dir = vault("riff-empty");
        let mut app = app_in(&dir);
        app.prefs.riff_last.clear();
        // Not installed yet: the deck refuses to even talk.
        assert!(app.run_riff(RiffAction::Status).is_err());
        app.prefs.installed.push("riff".into());
        assert!(app.riff_status().contains("no disc"));
        let error = app.run_riff(RiffAction::Play(None)).unwrap_err();
        assert!(error.contains("no music"), "{error}");
        let error = app.run_riff(RiffAction::Insert).unwrap_err();
        assert!(error.contains("open Riff"), "{error}");

        app.home = Home::Riff;
        app.mode = Mode::Action;
        assert!(app.run_riff(RiffAction::Insert).is_ok());
        assert_eq!(app.mode, Mode::RiffInsert);
        assert!(app.riff.explorer.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn inserting_takes_the_highlighted_folder_not_just_the_open_one() {
        let dir = vault("riff-target");
        let music = dir.join("music");
        std::fs::create_dir_all(music.join("Album")).unwrap();
        std::fs::write(music.join("loose.wav"), b"x").unwrap();

        let mut app = app_in(&dir);
        let mut browser = Browser::music(music.clone());
        browser.refresh().unwrap();
        app.riff.explorer = Some(browser);
        assert_eq!(
            app.explorer_insert_target(),
            Some(music.join("Album")),
            "the cursor is on a folder"
        );

        // On a track row, the folder itself is inserted.
        app.riff.explorer.as_mut().unwrap().selected = 1;
        assert_eq!(app.explorer_insert_target(), Some(music));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_explorer_walks_folders_without_leaving_music() {
        let dir = vault("riff-explorer");
        let music = dir.join("music");
        std::fs::create_dir_all(music.join("Album")).unwrap();
        std::fs::write(music.join("Album/01.wav"), b"x").unwrap();
        std::fs::write(music.join("loose.wav"), b"x").unwrap();
        std::fs::write(music.join("cover.png"), b"x").unwrap();

        let mut app = app_in(&dir);
        app.home = Home::Riff;
        app.mode = Mode::Action;
        let mut browser = Browser::music(music.clone());
        browser.refresh().unwrap();
        app.riff.explorer = Some(browser);
        app.mode = Mode::RiffInsert;
        assert_eq!(
            app.riff.explorer.as_ref().unwrap().entries.len(),
            2,
            "the album and one track, no '..' at the music root"
        );
        assert!(browser::is_folder(
            &app.riff.explorer.as_ref().unwrap().entries[0].1
        ));
        assert!(
            matches!(
                app.riff.explorer.as_ref().unwrap().entries[1].1,
                Action::PlayTrack(_)
            ),
            "a playable file becomes a track, not an open action"
        );

        app.explorer_enter();
        assert!(app.riff.explorer.as_ref().unwrap().dir().ends_with("Album"));
        assert_eq!(
            app.riff.explorer.as_ref().unwrap().entries.len(),
            2,
            "'..' and the track inside"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_arrow_keys_move_the_music_list_not_the_vault() {
        let dir = vault("riff-move");
        let music = dir.join("music");
        std::fs::create_dir_all(music.join("Album")).unwrap();
        std::fs::write(music.join("loose.wav"), b"x").unwrap();
        std::fs::write(music.join("zeta.wav"), b"x").unwrap();

        let mut app = app_in(&dir);
        app.mode = Mode::RiffInsert;
        let mut browser = Browser::music(music.clone());
        browser.refresh().unwrap();
        app.riff.explorer = Some(browser);

        // The vault's slot is a different browser and must stay out of it.
        app.browser.selected = 7;
        app.explorer_move(1);
        assert_eq!(app.riff.explorer.as_ref().unwrap().selected, 1);
        assert_eq!(
            app.browser.selected, 7,
            "the music list has its own highlight"
        );

        app.explorer_move(1);
        assert_eq!(app.riff.explorer.as_ref().unwrap().selected, 2);
        app.explorer_move(1);
        assert_eq!(
            app.riff.explorer.as_ref().unwrap().selected,
            2,
            "clamped at the end"
        );
        app.explorer_move(-5);
        assert_eq!(app.riff.explorer.as_ref().unwrap().selected, 0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_launch_bar_is_lumpy_but_never_goes_back() {
        assert_eq!(boot_progress(0.0), 0.0);
        assert_eq!(
            boot_progress(BOOT_SECONDS * 0.92),
            1.0,
            "full before the end"
        );
        assert_eq!(boot_progress(BOOT_SECONDS * 1.5), 1.0, "and stays full");

        // It holds still at times: that pause is the whole point.
        assert_eq!(
            boot_progress(BOOT_SECONDS * 0.18),
            boot_progress(BOOT_SECONDS * 0.28),
            "a seek pause"
        );
        // Then jumps to the next step in one frame.
        assert!(
            boot_progress(BOOT_SECONDS * 0.34) > boot_progress(BOOT_SECONDS * 0.32),
            "the bar moves in jumps"
        );

        // Monotonic, and never past 1.
        let mut last = 0.0;
        for step in 0..=400 {
            let value = boot_progress(step as f32 * BOOT_SECONDS / 400.0);
            assert!(value >= last, "went backwards at {step}");
            assert!((0.0..=1.0).contains(&value));
            last = value;
        }
    }

    #[test]
    fn leaving_software_leaves_the_row_highlighted() {
        let dir = vault("software-cursor");
        let mut app = app_in(&dir);
        // Nothing installed: Inspiro, the rule, SOFTWARE, DOCUMENTS.
        app.boot_selected = 2;
        assert_eq!(app.boot_items()[2], BootItem::Software);

        app.open_software();
        assert_eq!(app.mode, Mode::Software);
        app.to_write();
        assert_eq!(app.mode, Mode::Action, "back on the boot screen");
        assert_eq!(app.boot_selected, 2, "the cursor stays on SOFTWARE");

        app.compose();
        let text: Vec<String> = (0..app.screen.rows())
            .map(|y| {
                (0..app.screen.cols())
                    .map(|x| app.screen.get(x, y).unwrap().ch)
                    .collect()
            })
            .collect();
        let row = text
            .iter()
            .position(|line| line.contains("SOFTWARE"))
            .expect("SOFTWARE row");
        let highlighted = (0..app.screen.cols()).any(|x| {
            app.screen
                .get(x, row as u32)
                .is_some_and(|cell| cell.bg == app.theme.fg)
        });
        assert!(highlighted, "row {row} lost its cursor: {:?}", text[row]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn installing_keeps_the_cursor_on_the_row_it_was_on() {
        let dir = vault("software-cursor-moved");
        let mut app = app_in(&dir);
        // The boot screen with the cursor on SOFTWARE.
        app.boot_selected = 2;
        assert_eq!(app.boot_items()[2], BootItem::Software);

        // Install Softhand from the shelf: a row is inserted above SOFTWARE.
        app.open_software();
        app.software_selected = 1;
        app.software_activate();
        app.finish_loading();
        // Still on the shelf, and the cursor there is unaffected.
        assert_eq!(app.mode, Mode::Software);

        app.to_write();
        assert_eq!(
            app.boot_items().get(app.boot_selected),
            Some(&BootItem::Software),
            "the cursor followed the row, not the index"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn software_installs_and_removes_modules() {
        let dir = vault("software");
        let mut app = app_in(&dir);
        // Fresh: only the system and the shelf.
        let labels = |app: &App| -> Vec<String> {
            app.boot_rows().into_iter().map(|row| row.label).collect()
        };
        assert_eq!(
            labels(&app),
            vec![
                "A: INSPIRO",
                "",
                "SOFTWARE",
                "THE VAULT",
                "CALENDAR",
                "SETTINGS"
            ]
        );

        // Row 0 is Inspiro, the system: it refuses to go.
        app.software_selected = 0;
        app.software_activate();
        assert_eq!(app.status, "Inspiro is the system - it stays");
        assert_eq!(app.mode, Mode::Action, "no bar for a refusal");

        // Enter on Softhand starts the bar; the bar is what finishes the job.
        app.software_selected = 1;
        app.software_activate();
        assert_eq!(app.mode, Mode::Loading);
        assert!(!app.is_installed("softhand"), "not until the bar is done");
        app.finish_loading();
        assert_eq!(app.mode, Mode::Software, "back to the shelf");
        assert!(app.is_installed("softhand"));
        assert_eq!(app.prefs.installed, vec!["softhand"]);
        assert_eq!(
            labels(&app),
            vec![
                "A: INSPIRO",
                "B: SOFTHAND",
                "",
                "SOFTWARE",
                "THE VAULT",
                "CALENDAR",
                "SETTINGS"
            ]
        );

        // And removing it takes the row away again.
        app.software_selected = 1;
        app.software_activate();
        app.finish_loading();
        assert!(!app.is_installed("softhand"));
        assert_eq!(app.prefs.installed.len(), 0);
        assert_eq!(
            labels(&app),
            vec![
                "A: INSPIRO",
                "",
                "SOFTWARE",
                "THE VAULT",
                "CALENDAR",
                "SETTINGS"
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn drive_letters_follow_the_order_modules_were_installed() {
        let dir = vault("letters");
        let mut app = app_in(&dir);
        // Riff first, then Softhand: Riff is B, Softhand is C.
        app.prefs.installed = vec!["riff".into(), "softhand".into()];
        let labels: Vec<String> = app.boot_rows().into_iter().map(|row| row.label).collect();
        assert_eq!(
            labels,
            vec![
                "A: INSPIRO",
                "B: RIFF",
                "C: SOFTHAND",
                "",
                "SOFTWARE",
                "THE VAULT",
                "CALENDAR",
                "SETTINGS"
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn calendar_loads_the_month_and_the_day_list() {
        let dir = vault("calendar");
        let cal_dir = storage::calendar_in(&dir);
        std::fs::create_dir_all(&cal_dir).unwrap();
        let today = chrono::Local::now().date_naive();
        let event = cread_core::calendar::Event {
            title: "Doctor".into(),
            date: today,
            time: Some(chrono::NaiveTime::from_hms_opt(10, 0, 0).unwrap()),
            remind: Some(15),
            repeat: cread_core::calendar::Repeat::Never,
            deleted: false,
            notes: String::new(),
        };
        std::fs::write(cal_dir.join("doctor.md"), event.to_markdown()).unwrap();

        let mut app = app_in(&dir);
        app.open_calendar();
        assert_eq!(app.mode, Mode::Calendar);
        assert!(
            app.calendar
                .events
                .iter()
                .any(|entry| entry.event.title == "Doctor")
        );
        app.compose();
        let text = rows_as_text(&app.screen);
        assert!(text.contains("CALENDAR"));
        assert!(text.contains("Doctor"));
        assert!(text.contains("10:00"));
        assert!(text.contains("[GRID]"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn all_day_appointments_sort_before_timed_ones() {
        let dir = vault("calendar-all-day-first");
        let mut app = app_in(&dir);
        app.open_calendar();
        let today = chrono::Local::now().date_naive();
        let last_week = today - chrono::Duration::days(7);
        let timed = cread_core::calendar::Event {
            title: "Timed".into(),
            date: last_week,
            time: chrono::NaiveTime::from_hms_opt(9, 0, 0),
            remind: None,
            repeat: cread_core::calendar::Repeat::Weekly,
            deleted: false,
            notes: String::new(),
        };
        let all_day = cread_core::calendar::Event {
            title: "All day".into(),
            date: today,
            time: None,
            remind: None,
            repeat: cread_core::calendar::Repeat::Never,
            deleted: false,
            notes: String::new(),
        };
        app.calendar.events = vec![
            cread_core::calendar::Entry {
                path: dir.join("timed.md"),
                event: timed,
            },
            cread_core::calendar::Entry {
                path: dir.join("all-day.md"),
                event: all_day,
            },
        ];
        app.compose();
        let text = rows_as_text(&app.screen);
        let all_day = text.find("All day").expect("all-day title");
        let timed = text.find("09:00").expect("timed title");
        assert!(all_day < timed, "all-day row must come first:\n{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unsaved_calendar_form_is_marked_and_warns_before_discarding() {
        let dir = vault("calendar-unsaved");
        let mut app = app_in(&dir);
        app.open_calendar();
        app.new_event_form();
        {
            let form = app.calendar_form.as_mut().unwrap();
            form.title = "Draft".into();
            form.dirty = true;
        }
        app.compose();
        assert!(rows_as_text(&app.screen).contains("UNSAVED"));
        app.calendar_form.as_mut().unwrap().confirm_discard = true;
        app.compose();
        assert!(rows_as_text(&app.screen).contains("DISCARD EVENT?"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn calendar_edit_shows_a_visible_cursor() {
        let dir = vault("calendar-cursor");
        let mut app = app_in(&dir);
        app.open_calendar();
        app.new_event_form();
        {
            let form = app.calendar_form.as_mut().unwrap();
            form.title = "Abc".into();
            form.editing = true;
        }
        app.compose();
        let text = rows_as_text(&app.screen);
        assert!(text.contains('\u{2588}'), "the block cursor: {text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn calendar_form_writes_a_markdown_event() {
        let dir = vault("calendar-form");
        let mut app = app_in(&dir);
        app.open_calendar();
        app.new_event_form();
        let today = chrono::Local::now().date_naive();
        {
            let form = app.calendar_form.as_mut().unwrap();
            form.title = "Checkup".into();
            form.date = today.format("%Y-%m-%d").to_string();
            form.time = "10:30".into();
            form.repeat = "weekly".into();
            form.remind = "10".into();
        }
        app.save_event_form();
        assert_eq!(app.mode, Mode::Calendar);
        assert!(app.calendar_form.is_none());
        let entries = cread_core::calendar::load_events(&storage::calendar_in(&dir));
        let saved = entries
            .iter()
            .find(|entry| entry.event.title == "Checkup")
            .expect("the event file");
        assert_eq!(saved.event.time, chrono::NaiveTime::from_hms_opt(10, 30, 0));
        assert_eq!(saved.event.remind, Some(10));
        assert_eq!(saved.event.repeat, cread_core::calendar::Repeat::Weekly);
        assert!(app.unsynced);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn deleting_a_calendar_event_leaves_a_tombstone() {
        let dir = vault("calendar-delete");
        let cal_dir = storage::calendar_in(&dir);
        std::fs::create_dir_all(&cal_dir).unwrap();
        let today = chrono::Local::now().date_naive();
        let event = cread_core::calendar::Event {
            title: "Old".into(),
            date: today,
            time: None,
            remind: None,
            repeat: cread_core::calendar::Repeat::Never,
            deleted: false,
            notes: String::new(),
        };
        let path = cal_dir.join("old.md");
        std::fs::write(&path, event.to_markdown()).unwrap();

        let mut app = app_in(&dir);
        app.open_calendar();
        app.calendar.selected = today;
        app.calendar.selected_event = 0;
        app.delete_selected_event();
        let entries = cread_core::calendar::load_events(&cal_dir);
        assert!(entries.iter().any(|entry| entry.event.deleted));
        assert!(
            !entries
                .iter()
                .any(|entry| entry.event.title == "Old" && !entry.event.deleted)
        );
        assert!(app.unsynced);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_due_calendar_reminder_opens_in_front() {
        let dir = vault("calendar-reminder");
        let mut app = app_in(&dir);
        app.open_calendar();
        app.beep_on = false;
        let now = chrono::Local::now();
        app.reminder = Some(ReminderDue {
            at: now.naive_local() - chrono::Duration::minutes(1),
            key: "test".into(),
            title: "Doctor".into(),
            when: "today".into(),
            date: now.date_naive(),
            path: std::path::PathBuf::new(),
        });
        app.pump_reminders();
        assert_eq!(app.mode, Mode::Reminder);
        assert_eq!(app.reminder_title, "Doctor");
        assert!(app.reminder_fired.contains("test"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_fired_weekly_reminder_advances_to_next_week() {
        let dir = vault("calendar-weekly-reminder");
        let mut app = app_in(&dir);
        app.open_calendar();
        let at = chrono::Local::now().naive_local() + chrono::Duration::hours(2);
        let path = dir.join("Calendar/weekly.md");
        let event = cread_core::calendar::Event {
            title: "Weekly".into(),
            date: at.date(),
            time: Some(at.time()),
            remind: Some(0),
            repeat: cread_core::calendar::Repeat::Weekly,
            deleted: false,
            notes: String::new(),
        };
        let entry = cread_core::calendar::Entry {
            path: path.clone(),
            event,
        };
        let key = format!("{}|{}", path.display(), at.format("%Y-%m-%d %H:%M"));
        app.calendar.events = vec![entry];
        app.reminder_fired.insert(key);
        app.schedule_reminder();
        assert_eq!(
            app.reminder.as_ref().map(|due| due.at),
            Some(at + chrono::Duration::weeks(1))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_reminder_opened_over_the_calendar_can_leave_the_calendar() {
        let dir = vault("calendar-reminder-back");
        let mut app = app_in(&dir);
        app.open_calendar();
        let back = app.calendar_back;
        app.reminder_back = Mode::Calendar;
        app.open_calendar_at(chrono::Local::now().date_naive(), dir.join("event.md"));
        app.leave_calendar();
        assert_ne!(app.mode, Mode::Calendar);
        assert_ne!(app.mode, Mode::CalendarEdit);
        assert_eq!(app.mode, back);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn american_poker_opens_on_its_own_front_door_and_returns_there() {
        let dir = vault("poker-front");
        let mut app = app_in(&dir);
        let index = PROGRAMS
            .iter()
            .position(|program| program.id == "poker")
            .expect("the poker module");

        app.pending = Some(Pending::Program(index));
        app.finish_loading();
        assert_eq!(app.home, Home::Poker);
        assert_eq!(app.mode, Mode::Action, "the logo is the front door");

        app.start_poker();
        assert_eq!(app.program_id, Some("poker"));
        assert_eq!(app.mode, Mode::Program);

        app.leave_poker();
        assert!(app.program.is_none());
        assert_eq!(app.home, Home::Poker);
        assert_eq!(app.mode, Mode::Action);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scripture_is_installable_and_opens_from_its_cover() {
        let dir = vault("scripture-front");
        let mut app = app_in(&dir);
        let index = PROGRAMS
            .iter()
            .position(|program| program.id == "scripture")
            .expect("the scripture module");
        let row = app
            .software_rows()
            .into_iter()
            .find(|row| row.name == "SCRIPTURE")
            .expect("SCRIPTURE on the shelf");
        assert_eq!(row.value, "install");
        app.prefs.installed.push("scripture".into());
        assert!(
            app.boot_items()
                .iter()
                .any(|item| matches!(item, BootItem::Program(i) if *i == index)),
            "the installed module appears on the boot screen"
        );

        app.pending = Some(Pending::Program(index));
        app.finish_loading();
        assert_eq!(app.home, Home::Scripture);
        assert_eq!(app.mode, Mode::Action, "the logo is the front door");

        app.compose();
        assert!(app.scripture_at.is_some(), "the cover logo is placed");
        assert!(rows_as_text(&app.screen).contains("PRESS ENTER TO READ"));

        app.start_scripture();
        assert_eq!(app.program_id, Some("scripture"));
        assert_eq!(app.mode, Mode::Program);
        app.compose();
        let text = rows_as_text(&app.screen);
        assert!(text.contains("SCRIPTURE"));
        assert!(text.contains("GENESIS 1/50"));

        app.leave_scripture();
        assert!(app.program.is_none());
        assert_eq!(app.home, Home::Scripture);
        assert_eq!(app.mode, Mode::Action);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_software_screen_lists_the_system_and_the_modules() {
        let dir = vault("software-rows");
        let mut app = app_in(&dir);
        let rows = app.software_rows();
        assert_eq!(rows[0].name, "INSPIRO");
        assert_eq!(rows[0].value, "system");
        let softhand = rows.iter().find(|row| row.name == "SOFTHAND").unwrap();
        assert_eq!(softhand.value, "install");
        app.prefs.installed.push("softhand".into());
        let rows = app.software_rows();
        let softhand = rows.iter().find(|row| row.name == "SOFTHAND").unwrap();
        assert_eq!(softhand.value, "installed");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_arrow_keys_skip_the_rule() {
        let dir = vault("boot-nav");
        let mut app = app_in(&dir);
        assert_eq!(app.boot_selected, 0, "starts on INSPIRO");
        app.boot_step(1);
        assert_eq!(app.boot_selected, 2, "over the rule, onto SOFTWARE");
        app.boot_step(1);
        assert_eq!(app.boot_selected, 3, "onto THE VAULT");
        app.boot_step(1);
        assert_eq!(app.boot_selected, 4, "onto CALENDAR");
        app.boot_step(1);
        assert_eq!(app.boot_selected, 5, "onto SETTINGS");
        app.boot_step(1);
        assert_eq!(app.boot_selected, 5, "the end is the end");
        app.boot_step(-1);
        assert_eq!(app.boot_selected, 4);
        app.boot_step(-1);
        assert_eq!(app.boot_selected, 3);
        app.boot_step(-1);
        assert_eq!(app.boot_selected, 2);
        app.boot_step(-1);
        assert_eq!(app.boot_selected, 0, "back over the rule");
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
        // A pointer position in real pixels lands on the raster pixel under it,
        // wherever the stage sits: the window origin is the stage origin.
        let (ox, oy) = app.program_origin();
        assert_eq!(ox, 0, "a 640-wide stage needs no horizontal offset");
        app.pointer = ((320.0 + ox as f64) * scale, (200.0 + oy as f64) * scale);
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
        assert_eq!(app.mode, Mode::Browser);
        assert!(app.browser.entries.is_empty(), "nothing saved yet");

        app.new_picture();
        app.save_picture(None).unwrap();
        app.leave_to_boot();
        app.home = Home::SoftHand;
        app.open_pictures();
        assert_eq!(app.browser.entries.len(), 1);
        assert!(matches!(app.browser.entries[0].1, Action::OpenPicture(_)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn settings_offers_an_update_check() {
        let dir = vault("update-row");
        let app = app_in(&dir);
        let rows = app.setup_rows();
        let row = |name: &str| {
            rows.iter()
                .position(|row| row.name == name)
                .unwrap_or_else(|| panic!("no {name} row"))
        };
        assert!(rows.iter().any(|row| row.rule), "groups are separated");
        assert_eq!(rows[row("UPDATE")].value, ui::VERSION);
        assert!(row("INFO") < row("UPDATE"));
        assert!(row("UPDATE") < row("SOFTWARE"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn settings_has_the_screensaver_switches() {
        let dir = vault("screensaver-rows");
        let mut app = app_in(&dir);
        let index = |app: &App, name: &str| {
            app.setup_rows()
                .iter()
                .position(|row| row.name == name)
                .unwrap_or_else(|| panic!("no {name} row"))
        };
        assert_eq!(app.setup_rows()[index(&app, "SCREENSAVER")].value, "on");
        assert_eq!(app.setup_rows()[index(&app, "IDLE")].value, "5 min");

        app.setup_selected = index(&app, "SCREENSAVER");
        app.setup_activate();
        assert!(!app.screensaver_on);
        assert_eq!(app.setup_rows()[index(&app, "SCREENSAVER")].value, "off");

        app.setup_selected = index(&app, "IDLE");
        app.setup_activate();
        assert_eq!(app.screensaver_minutes, 10);
        assert_eq!(app.setup_rows()[index(&app, "IDLE")].value, "10 min");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn programs_are_centred_on_a_larger_framebuffer() {
        let dir = vault("stage");
        let mut app = app_in(&dir);

        // A 160x50 grid is what a 100%-scaled 3072x1920 panel ends up with.
        assert_eq!(app.program_stage(160, 50), (40, 12), "a game is centred");
        app.new_picture();
        assert_eq!(
            app.program_stage(160, 50),
            (40, 13),
            "Softhand sits below its bar"
        );
        assert_eq!(app.program_stage(80, 25), (0, 1), "the classic grid");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_pointer_follows_the_centred_stage() {
        let dir = vault("stage-pointer");
        let mut app = app_in(&dir);
        app.new_picture();
        let vp = Viewport::compute(3072, 1920, ScaleMode::DEFAULT).unwrap();
        app.vp = Some(vp);
        let (ox, oy) = app.program_origin();
        assert!(ox > 0, "the stage is not in the corner: {ox}");
        assert!(oy > 0);

        let scale = vp.scale as f64;
        app.pointer = (
            vp.offset_x as f64 + (ox as f64 + 10.0) * scale,
            vp.offset_y as f64 + (oy as f64 + 10.0) * scale,
        );
        assert_eq!(app.raster_point(), (10, 10));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_settings_bar_has_no_subtitle() {
        let dir = vault("settings-bar");
        let mut app = app_in(&dir);
        app.open_setup();
        app.compose();
        let bar = rows_as_text(&app.screen)
            .lines()
            .next()
            .unwrap()
            .to_string();
        assert!(bar.contains("SETTINGS"), "{bar:?}");
        assert!(!bar.contains("machine"), "no subtitle: {bar:?}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_game_prompt_offers_only_what_a_game_can_do() {
        let dir = vault("boi-prompt");
        let mut app = app_in(&dir);
        app.program = Some(Box::new(cread_pocketboi::Pocketboi::new(&dir)));
        app.program_id = Some("pocketboi");
        app.mode = Mode::Program;
        app.program_prompt = true;
        app.command.clear();
        app.compose();

        let text = rows_as_text(&app.screen);
        assert!(text.contains("mute") && text.contains("exit"), "{text}");
        assert!(
            !text.contains("save   open"),
            "a game has nothing to save: {text}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn opening_a_document_from_a_program_hands_the_screen_to_the_editor() {
        let dir = vault("open-from-program");
        std::fs::create_dir_all(dir.join("Documents")).unwrap();
        std::fs::write(dir.join("Documents/notiz.md"), "hallo").unwrap();

        let mut app = app_in(&dir);
        app.program = Some(Box::new(cread_pocketboi::Pocketboi::new(&dir)));
        app.program_id = Some("pocketboi");
        app.mode = Mode::Program;

        app.start_loading(Pending::Document(dir.join("Documents/notiz.md")));
        app.finish_loading();

        assert!(app.program.is_none(), "the game is not in front any more");
        assert!(app.program_id.is_none());
        assert_eq!(app.mode, Mode::Write);
        assert!(app.has_document);
        assert_eq!(app.doc.editor.text(), "hallo");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn info_returns_to_settings() {
        let dir = vault("info-back");
        let mut app = app_in(&dir);
        app.open_setup();
        app.setup_selected = app
            .setup_entries()
            .iter()
            .position(|(row, _)| row.name == "INFO")
            .expect("the info row");
        app.setup_activate();
        assert_eq!(app.mode, Mode::Help);
        assert!(app.help_about);

        app.leave_help();
        assert_eq!(app.mode, Mode::Setup, "back where INFO was opened");

        // `about` from the prompt still returns to the work it covered.
        app.help_from_setup = false;
        app.mode = Mode::Write;
        app.has_document = true;
        app.leave_help();
        assert_eq!(app.mode, Mode::Write);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_settings_switch_the_clock_and_date_formats() {
        let dir = vault("format-rows");
        let mut app = app_in(&dir);
        let index = |app: &App, name: &str| {
            app.setup_rows()
                .iter()
                .position(|row| row.name == name)
                .unwrap_or_else(|| panic!("no {name} row"))
        };

        assert_eq!(app.setup_rows()[index(&app, "TIME")].value, "24 hours");
        assert_eq!(app.setup_rows()[index(&app, "DATE")].value, "DD.MM.YY");

        app.setup_selected = index(&app, "TIME");
        app.setup_activate();
        assert!(!app.clock_24h);
        assert_eq!(app.setup_rows()[index(&app, "TIME")].value, "12 hours");

        app.setup_selected = index(&app, "DATE");
        app.setup_activate();
        assert!(!app.date_day_first);
        assert_eq!(app.setup_rows()[index(&app, "DATE")].value, "MM/DD/YY");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_settings_arrows_skip_the_rules() {
        let dir = vault("settings-rules");
        let mut app = app_in(&dir);
        app.open_setup();
        assert_eq!(app.setup_rows()[app.setup_selected].name, "WEBDAV");

        app.setup_step(1);
        assert_eq!(app.setup_rows()[app.setup_selected].name, "THEME");
        app.setup_step(-1);
        assert_eq!(app.setup_rows()[app.setup_selected].name, "WEBDAV");

        for _ in 0..40 {
            app.setup_step(1);
        }
        assert_eq!(app.setup_rows()[app.setup_selected].name, "USER MANUAL");
        assert!(
            !app.setup_rows()[app.setup_selected].rule,
            "never lands on a rule"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn autosave_says_so_while_the_document_is_clean() {
        let dir = vault("autosave-bar");
        let mut app = app_in(&dir);
        app.new_document().unwrap();
        let bar = |app: &mut App| {
            app.compose();
            rows_as_text(&app.screen)
                .lines()
                .next()
                .unwrap()
                .to_string()
        };

        let line = bar(&mut app);
        assert!(line.contains(" saved"), "a fresh file is saved: {line}");
        assert!(
            !line.contains("autosave on"),
            "autosave is off, not just missing from the state: {line}"
        );

        app.autosave = true;
        let line = bar(&mut app);
        assert!(line.contains("autosave on"), "the mode is visible: {line}");
        assert!(!line.contains("saved"), "{line}");

        app.doc.editor.insert_text("x");
        let line = bar(&mut app);
        assert!(
            line.contains(" modified"),
            "unwritten text still warns, even with autosave on: {line}"
        );
        assert!(!line.contains("autosave on"), "{line}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_settings_show_the_disk_and_keep_the_manual_last() {
        const GIB: u64 = 1024 * 1024 * 1024;
        let dir = vault("storage-row");
        let mut app = app_in(&dir);
        app.open_setup();

        let rows = app.setup_rows();
        let storage = rows
            .iter()
            .position(|row| row.name == "STORAGE")
            .expect("the storage row");
        let network = rows
            .iter()
            .position(|row| row.name == "NETWORK")
            .expect("the network row");
        assert_eq!(storage + 1, network, "network follows storage");
        assert_eq!(network + 1, rows.len() - 1, "just before the manual");
        assert_eq!(rows.last().unwrap().name, "USER MANUAL");
        assert!(app.space.is_some(), "the vault's filesystem is readable");

        app.space = Some(space::Space {
            total: 100 * GIB,
            used: 50 * GIB,
            free: 50 * GIB,
        });
        app.compose();
        let text = rows_as_text(&app.screen);
        assert!(text.contains("STORAGE"), "{text}");
        assert!(text.contains("50%"), "{text}");
        assert!(text.contains('\u{2588}'), "the bar has filled cells");
        assert!(text.contains('\u{2591}'), "and empty cells");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_about_screen_shows_the_machine() {
        let dir = vault("about-machine");
        let mut app = app_in(&dir);
        app.refresh_machine();
        app.help_about = true;
        app.mode = Mode::Help;

        app.compose();
        let text = rows_as_text(&app.screen);
        assert!(text.contains("CRead 0."), "the version title: {text}");
        assert!(text.contains("Network"), "the local address line: {text}");
        assert!(text.contains("Kernel") || text.contains("System"), "{text}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_settings_open_the_manual_and_esc_brings_them_back() {
        let dir = vault("manual");
        let mut app = app_in(&dir);
        app.open_setup();
        let index = app
            .setup_rows()
            .iter()
            .position(|row| row.name == "USER MANUAL")
            .expect("the manual row");
        assert_eq!(
            index,
            app.setup_rows().len() - 1,
            "the manual is the last point"
        );

        app.setup_selected = index;
        app.setup_activate();
        assert_eq!(app.mode, Mode::Manual);

        app.compose();
        let text = rows_as_text(&app.screen);
        assert!(text.contains("Contents"), "the contents are on screen");
        assert!(text.contains("Starting"), "the first point is visible");
        assert!(text.contains("USER MANUAL"), "the bar says where we are");
        let column: String = (2..app.screen.rows() - 2)
            .map(|y| app.screen.get(app.screen.cols() - 1, y).unwrap().ch)
            .collect();
        assert!(
            column.contains('\u{2588}'),
            "the manual scrolls: {column:?}"
        );

        app.scroll_manual(40);
        assert!(app.manual_top > 0, "it scrolls");

        app.leave_manual();
        assert_eq!(app.mode, Mode::Setup, "back to the settings");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn switching_sync_says_so_and_returns_to_the_form() {
        let dir = vault("sync-notice");
        let mut app = app_in(&dir);
        app.sync_cfg = SyncConfig {
            url: "https://example.com/dav".into(),
            enabled: false,
            ..SyncConfig::default()
        };
        app.open_webdav();

        app.toggle_sync();
        assert_eq!(app.mode, Mode::Notice);
        assert_eq!(app.notice_text, "SYNC ON");
        assert!(app.sync_cfg.enabled, "the switch is real");
        app.compose();
        assert!(rows_as_text(&app.screen).contains("SYNC ON"));
        app.on_key_notice();
        assert_eq!(app.mode, Mode::Webdav, "back to the form");

        app.toggle_sync();
        assert_eq!(app.notice_text, "SYNC OFF");
        assert!(!app.sync_cfg.enabled);

        // A switch without a server stays off and says why.
        app.sync_cfg.url.clear();
        app.sync_cfg.enabled = false;
        app.open_webdav();
        app.toggle_sync();
        assert_eq!(app.mode, Mode::Webdav, "the form stays open");
        assert!(!app.sync_cfg.enabled);
        assert!(app.form.note.contains("URL"), "{}", app.form.note);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_narrowest_grid_still_draws_the_screens() {
        let dir = vault("narrow-grid");
        let mut app = app_in(&dir);
        app.screen = Screen::new(40, 12);

        app.mode = Mode::Write;
        app.compose();

        app.open_manual(Mode::Write);
        app.compose();
        assert!(!app.manual_rows.is_empty());

        app.open_notice("SYNC ON", Mode::Write);
        app.compose();

        app.quit = Some(QuitPrompt {
            yes: true,
            from: Mode::Write,
        });
        app.mode = Mode::Quit;
        app.compose();

        app.open_setup();
        app.compose();
        app.help_all = true;
        app.mode = Mode::Help;
        app.compose();
        app.open_software();
        app.compose();
        app.open_webdav();
        app.compose();
        app.open_browser();
        app.compose();
        app.mode = Mode::View;
        app.compose();
        app.mode = Mode::Search;
        app.compose();
        app.mode = Mode::Loading;
        app.compose();
        app.mode = Mode::Checking;
        app.compose();
        app.start_screensaver();
        app.compose();

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn shutdown_asks_first_and_can_be_cancelled() {
        let dir = vault("shutdown");
        let mut app = app_in(&dir);
        app.mode = Mode::Action;

        app.cmd_shutdown().unwrap();
        assert_eq!(app.mode, Mode::Power);
        assert_eq!(app.power.map(|prompt| prompt.yes), Some(false));

        app.compose();
        assert!(rows_as_text(&app.screen).contains("POWER OFF"));

        app.cancel_power();
        assert_eq!(app.mode, Mode::Action);
        assert!(app.power.is_none());

        app.cmd_reboot().unwrap();
        app.compose();
        assert!(rows_as_text(&app.screen).contains("REBOOT"));
        app.cancel_power();

        app.cmd_backtolinux().unwrap();
        app.compose();
        assert!(rows_as_text(&app.screen).contains("LEAVE CREAD"));
        app.cancel_power();

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn manual_contents_entries_become_jump_targets() {
        let jumps = manual_jumps();
        assert_eq!(
            jumps.len(),
            27,
            "8 quick-start steps, 14 chapters, 5 appendices"
        );
        assert_eq!(jumps[0].label, "1. Starting");
        assert!(jumps.iter().all(|jump| !jump.label.starts_with("Part")));
        assert!(jumps.iter().all(|jump| {
            MANUAL
                .lines()
                .nth(jump.target)
                .is_some_and(|line| manual_heading(line).is_some())
        }));
        let first = MANUAL.lines().nth(jumps[0].target).unwrap();
        assert!(first.contains("Step 1: Starting"), "{first}");

        let last = jumps.last().unwrap();
        assert!(last.label.contains("About this program"));
        let appendix = MANUAL.lines().nth(last.target).unwrap();
        assert!(appendix.contains("Appendix E"), "{appendix}");
    }

    #[test]
    fn enter_jumps_the_manual_to_the_selected_contents_entry() {
        let dir = vault("manual-jump");
        let mut app = app_in(&dir);
        app.open_manual(Mode::Write);
        assert!(app.manual_nav);
        assert_eq!(app.manual_choice, 0);
        assert_eq!(
            app.manual_selection_row()
                .map(|row| app.manual_rows[row].source_line),
            Some(app.manual_jumps[0].line)
        );

        app.select_manual(1);
        assert_eq!(app.manual_choice, 1);
        app.enter_manual_selection();
        assert!(!app.manual_nav);
        assert_eq!(
            app.manual_rows[app.manual_top].source_line,
            app.manual_jumps[1].target
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_manual_returns_to_where_it_was_opened() {
        let dir = vault("manual-command");
        let mut app = app_in(&dir);
        app.mode = Mode::Write;
        app.has_document = true;

        app.open_manual(Mode::Write);
        assert_eq!(app.mode, Mode::Manual);
        app.leave_manual();
        assert_eq!(app.mode, Mode::Write);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn softhand_shows_its_tool_keys_under_the_colours() {
        let dir = vault("softhand-legend");
        let mut app = app_in(&dir);
        app.new_picture();
        app.screen = Screen::new(80, 32);
        app.compose();
        let text = rows_as_text(&app.screen);
        assert!(text.contains("B pencil"), "the legend is there: {text}");
        assert!(text.contains("E eraser"));
        assert!(text.contains("I pick"));

        app.screen = Screen::new(80, 25);
        app.compose();
        assert!(
            !rows_as_text(&app.screen).contains("B pencil"),
            "25 rows leave no space, and the palette is not covered"
        );

        app.screen = Screen::new(80, 26);
        app.compose();
        assert!(
            rows_as_text(&app.screen).contains("B pencil"),
            "one free row is enough for the legend"
        );

        app.program_prompt = true;
        app.compose();
        assert!(
            !rows_as_text(&app.screen).contains("B pencil"),
            "the prompt takes the last row when it needs it"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_long_document_has_a_scrollbar_on_the_right() {
        let dir = vault("scrollbar");
        let mut app = app_in(&dir);
        app.new_document().unwrap();
        app.doc.editor = Editor::from_text(&"line\n".repeat(60));
        app.compose();
        let column: String = (2..app.screen.rows() - 2)
            .map(|y| app.screen.get(app.screen.cols() - 1, y).unwrap().ch)
            .collect();
        assert!(column.contains('\u{2588}'), "there is a thumb: {column:?}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn softhand_wears_the_same_bar_as_the_editor() {
        let dir = vault("softhand-bar");
        let mut app = app_in(&dir);
        app.new_picture();
        app.vp = Some(Viewport::compute(1280, 800, ScaleMode::DEFAULT).unwrap());
        app.compose();
        let bar = rows_as_text(&app.screen)
            .lines()
            .next()
            .unwrap()
            .to_string();
        assert!(bar.contains("untitled.png"), "{bar}");
        assert!(bar.contains("modified"), "{bar}");

        app.save_picture(None).unwrap();
        app.compose();
        let bar = rows_as_text(&app.screen)
            .lines()
            .next()
            .unwrap()
            .to_string();
        assert!(bar.contains("saved"), "the state follows the save: {bar}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_screensaver_rests_after_the_configured_idle_time() {
        let dir = vault("screensaver-idle");
        let mut app = app_in(&dir);
        assert!(app.screensaver_on, "on out of the box");
        assert_eq!(app.screensaver_minutes, 5);
        assert!(!app.screensaver_due(), "fresh input");

        app.last_input = Instant::now() - Duration::from_secs(5 * 60 + 1);
        assert!(app.screensaver_due());
        app.start_screensaver();
        assert_eq!(app.mode, Mode::Screensaver);

        assert!(app.wake_screensaver(), "a key wakes it");
        assert_eq!(app.mode, Mode::Action);
        assert!(!app.screensaver_due(), "and the clock waits again");

        app.screensaver_on = false;
        app.last_input = Instant::now() - Duration::from_secs(60 * 60);
        assert!(!app.screensaver_due(), "switched off in settings");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_screensaver_returns_to_the_screen_it_covered() {
        let dir = vault("screensaver-back");
        let mut app = app_in(&dir);
        app.new_document().unwrap();
        app.mode = Mode::Write;

        app.start_screensaver();
        assert_eq!(app.screensaver_back, Mode::Write);
        assert!(app.wake_screensaver());
        assert_eq!(app.mode, Mode::Write, "the document, not the boot screen");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_screensaver_draws_the_clock_and_no_page() {
        let dir = vault("screensaver-draw");
        let mut app = app_in(&dir);
        app.start_screensaver();
        app.compose();

        assert!(
            app.screensaver_raster
                .pixels()
                .iter()
                .any(|index| *index != 0),
            "the clock was drawn"
        );
        assert_eq!(
            app.screensaver_raster.get(0, 0),
            Some(0),
            "and the page is black"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_restart_uses_the_path_captured_before_the_update() {
        let dir = vault("restart-path");
        let mut app = app_in(&dir);
        // `install` renames the running binary to `cread.old`; from then on
        // `/proc/self/exe` follows the inode and `current_exe()` names the old
        // file. The restart must use what was captured at startup.
        let captured = PathBuf::from("/tmp/cread-captured");
        app.self_exe = Some(captured.clone());
        assert_eq!(app.restart_target(), Some(captured));
        assert_ne!(
            app.restart_target(),
            std::env::current_exe().ok(),
            "not wherever the test binary happens to be"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn asking_for_an_update_shows_the_progress_window() {
        let dir = vault("update-window");
        let mut app = app_in(&dir);
        // Nothing listens there: the worker fails at once, no network needed.
        app.update_api = "http://127.0.0.1:1/releases".into();
        app.open_setup();
        app.setup_selected = app
            .setup_rows()
            .iter()
            .position(|row| row.name == "UPDATE")
            .expect("the update row");
        app.setup_activate();

        assert_eq!(app.mode, Mode::Checking);
        app.compose();
        let text = rows_as_text(&app.screen);
        assert!(text.contains("Checking github"), "{text}");

        // Joins the worker before the test binary goes away.
        app.updater = None;
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_update_question_names_the_version() {
        let dir = vault("update-prompt");
        let mut app = app_in(&dir);
        app.update_offer = Some(cread_update::Offer {
            version: "0.99.0".into(),
            tarball: "https://example.org/a.tar.gz".into(),
            checksum: "https://example.org/a.sha256".into(),
            signature: "https://example.org/a.minisig".into(),
        });
        app.update_status = UpdateStatus::Available("0.99.0".into());
        app.update_yes = true;
        app.mode = Mode::UpdatePrompt;

        app.compose();
        let text = rows_as_text(&app.screen);
        assert!(text.contains("UPDATE AVAILABLE"), "{text}");
        assert!(text.contains("0.99.0 is available"), "{text}");
        assert!(text.contains("[ Yes ]"), "{text}");

        // The settings row remembers what the check found.
        let rows = app.setup_rows();
        let update = rows
            .iter()
            .position(|row| row.name == "UPDATE")
            .expect("the update row");
        assert_eq!(rows[update].value, "0.99.0 available");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn open_starts_in_the_room_the_program_lives_in() {
        let dir = vault("open-rooms");
        std::fs::create_dir_all(dir.join("Documents/kapitel")).unwrap();
        std::fs::write(dir.join("Documents/notiz.md"), "x").unwrap();
        std::fs::create_dir_all(dir.join("Pictures")).unwrap();
        std::fs::write(dir.join("Pictures/a.png"), "x").unwrap();
        std::fs::write(dir.join("Pictures/notes.md"), "x").unwrap();

        let mut app = app_in(&dir);
        app.home = Home::Inspiro;
        app.new_document().unwrap();
        app.cmd_open(None).unwrap();

        assert_eq!(app.mode, Mode::Browser);
        assert_eq!(app.browser.root, dir.join("Documents"));
        assert_eq!(app.browser.title, "DOCUMENTS");
        let names = app.browser.items();
        assert!(names.contains(&"notiz.md".to_string()), "{names:?}");
        assert!(names.contains(&"kapitel/".to_string()), "{names:?}");
        assert!(
            !names.contains(&"+ sync now".to_string()),
            "the whole-vault row is not the editor's"
        );

        // Softhand starts in Pictures, and a text file is not a painting.
        app.close_program();
        app.has_document = false;
        app.home = Home::SoftHand;
        app.cmd_open(None).unwrap();
        assert_eq!(app.browser.root, dir.join("Pictures"));
        assert_eq!(app.browser.title, "PICTURES");
        let names = app.browser.items();
        assert!(names.contains(&"a.png".to_string()), "{names:?}");
        assert!(!names.contains(&"notes.md".to_string()), "{names:?}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_vault_row_opens_every_room() {
        let dir = vault("open-vault");
        std::fs::create_dir_all(dir.join("Documents")).unwrap();
        std::fs::create_dir_all(dir.join("Roms")).unwrap();

        let mut app = app_in(&dir);
        app.has_document = false;
        app.home = Home::Boot;
        app.cmd_open(None).unwrap();

        assert_eq!(app.browser.root, dir, "the whole machine");
        assert_eq!(app.browser.title, "THE VAULT");
        let names = app.browser.items();
        assert!(names.contains(&"Documents/".to_string()), "{names:?}");
        assert!(names.contains(&"Roms/".to_string()), "{names:?}");
        assert!(names.contains(&"+ sync now".to_string()), "{names:?}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_softhand_is_softhand() {
        let dir = vault("not-softhand");
        let mut app = app_in(&dir);
        app.home = Home::SoftHand;
        app.new_picture();
        assert!(app.in_softhand(), "the paint canvas");

        // A game opened from the vault: the front door is long gone.
        app.home = Home::Boot;
        app.program_id = Some("pocketboi");
        assert!(
            !app.in_softhand(),
            "a game must not answer to Softhand's save/open/new"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_dialog_stands_on_its_own_page() {
        let dir = vault("dialog-page");
        let mut app = app_in(&dir);

        // Over the boot screen: no drive row may peek out around the frame.
        app.home = Home::Boot;
        app.has_document = false;
        app.open_browser();
        app.compose();
        let text = rows_as_text(&app.screen);
        assert!(text.contains("THE VAULT"), "the window is there");
        assert!(!text.contains("A: INSPIRO"), "no boot row behind it");
        assert!(!text.contains("SOFTWARE"), "no shelf row behind it");

        // Over a document: the text stays out of sight as well.
        app.new_document().unwrap();
        app.doc
            .editor
            .insert_text("dieser satz darf nicht durchscheinen");
        app.open_browser();
        app.compose();
        let text = rows_as_text(&app.screen);
        assert!(text.contains("THE VAULT"));
        assert!(
            !text.contains("durchscheinen"),
            "the document is not drawn under a dialog"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_vault_browser_walks_folders_and_opens_the_right_program() {
        let dir = vault("vault-browser");
        let mut app = app_in(&dir);
        std::fs::create_dir_all(dir.join("Documents/kapitel")).unwrap();
        std::fs::write(dir.join("Documents/kapitel/eins.md"), "hi").unwrap();
        std::fs::create_dir_all(dir.join("Roms")).unwrap();
        std::fs::write(dir.join("Roms/game.gb"), b"rom").unwrap();
        std::fs::create_dir_all(dir.join("Pictures")).unwrap();
        std::fs::write(dir.join("Pictures/a.png"), b"png").unwrap();

        app.open_browser();
        let names = app.browser.items();
        assert!(names.contains(&"Documents/".to_string()), "{names:?}");
        assert!(names.contains(&"Pictures/".to_string()), "{names:?}");
        assert!(names.contains(&"Roms/".to_string()), "{names:?}");
        assert_eq!(app.browser.title, "THE VAULT");

        app.browser_enter(PathBuf::from("Roms"));
        assert_eq!(app.browser.items(), vec!["..", "game.gb"]);
        assert_eq!(app.browser.title, "THE VAULT /Roms");
        app.browser.selected = 1;
        match app.browser.current() {
            Some(Action::OpenRom(path)) => assert!(path.ends_with("Roms/game.gb")),
            _ => panic!("a cartridge should open Pocketboi"),
        }

        app.browser_enter(PathBuf::from("Documents"));
        assert_eq!(app.browser.items(), vec!["..", "kapitel/"]);
        app.browser_enter(PathBuf::from("Documents/kapitel"));
        assert_eq!(app.browser.items(), vec!["..", "eins.md"]);

        assert!(app.browser_up(), "up from a folder");
        assert_eq!(app.browser.path, PathBuf::from("Documents"));
        assert!(app.browser_up());
        assert_eq!(app.browser.path, PathBuf::new());
        assert!(!app.browser_up(), "the root is the top");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn saveas_only_knows_documents() {
        let dir = vault("saveas");
        let mut app = app_in(&dir);
        app.new_document().unwrap();
        assert!(
            app.doc
                .path
                .as_ref()
                .unwrap()
                .starts_with(dir.join("Documents")),
            "a new document is born in Documents/"
        );

        app.doc.editor.insert_text("kapitel eins");
        app.save_as("kapitel/eins").unwrap();
        assert!(dir.join("Documents/kapitel/eins.md").is_file());
        assert!(!dir.join("kapitel/eins.md").exists(), "not in the root");

        // Dots and absolute paths do not escape the folder.
        app.save_as("../../ausbruch").unwrap();
        assert!(dir.join("Documents/ausbruch.md").is_file());

        // A bare name still finds a nested document.
        let found = app.find_document("eins").unwrap();
        assert_eq!(found, dir.join("Documents/kapitel/eins.md"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn loose_documents_move_in_when_the_vault_is_first_used() {
        let dir = vault("migrate-app");
        std::fs::write(dir.join("alt.md"), "von früher").unwrap();
        let mut app = app_in(&dir);
        app.prepare_vault();
        assert!(dir.join("Documents/alt.md").is_file());
        assert!(!dir.join("alt.md").exists());
        assert!(app.unsynced, "a moved file has not reached the server");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_new_document_is_announced_as_a_draft_to_sync() {
        let dir = vault("draft-marker");
        let mut app = app_in(&dir);
        app.new_document().unwrap();

        let draft = cread_sync::draft(&dir).expect("the app said which file is the draft");
        assert!(draft.starts_with("Documents/untitled"), "{draft}");
        assert!(dir.join(&draft).is_file());

        app.save_as("kapitel/eins").unwrap();
        assert!(
            cread_sync::draft(&dir).is_none(),
            "naming the document ends the draft"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_explicit_save_keeps_the_placeholder_as_a_document() {
        let dir = vault("draft-save");
        let mut app = app_in(&dir);
        app.new_document().unwrap();
        let path = app.doc.path.clone().unwrap();
        app.doc.editor.insert_text("behalten");

        app.save().unwrap();
        assert!(path.is_file(), "the file stays");
        assert!(app.scratch.is_none(), "but it is not a draft any more");
        assert!(cread_sync::draft(&dir).is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn saveas_removes_the_untitled_placeholder() {
        let dir = vault("saveas-scratch");
        let mut app = app_in(&dir);
        app.autosave = true;
        app.mode = Mode::Write;
        app.new_document().unwrap();
        let scratch = app.doc.path.clone().unwrap();
        assert!(scratch.is_file(), "the placeholder exists from the start");

        // Autosave fills the placeholder, as it would while typing.
        app.doc.editor.insert_text("kapitel eins");
        app.doc.last_edit = Instant::now() - AUTOSAVE_DELAY * 2;
        app.autosave_if_due();
        assert_eq!(std::fs::read_to_string(&scratch).unwrap(), "kapitel eins");

        let message = app.save_as("kapitel/eins").unwrap();
        assert!(dir.join("Documents/kapitel/eins.md").is_file());
        assert!(!scratch.exists(), "the placeholder is gone: {message}");
        assert!(app.scratch.is_none(), "and not remembered for later");
        assert!(
            message.contains("removed"),
            "the status line says what happened: {message}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn saveas_keeps_the_placeholder_when_it_cannot_write() {
        let dir = vault("saveas-fails");
        let mut app = app_in(&dir);
        app.new_document().unwrap();
        let scratch = app.doc.path.clone().unwrap();

        assert!(app.save_as(".").is_err());
        assert!(scratch.is_file(), "nothing was written, nothing removed");
        assert!(app.scratch.is_some(), "still a placeholder");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_name_that_folds_away_is_refused_not_redirected() {
        let dir = vault("fold-name");
        let mut app = app_in(&dir);
        app.new_document().unwrap();
        app.doc.editor.insert_text("x");
        for bad in [".", "..", "../.."] {
            assert!(app.save_as(bad).is_err(), "{bad:?} must not name a file");
        }
        assert!(
            !dir.join("Documents.md").exists(),
            "no document next to the folder"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rename_refuses_to_overwrite_and_stays_in_its_folder() {
        let dir = vault("rename-guard");
        let mut app = app_in(&dir);
        std::fs::create_dir_all(dir.join("Documents/kapitel")).unwrap();
        std::fs::write(dir.join("Documents/kapitel/eins.md"), "eins").unwrap();
        std::fs::write(dir.join("Documents/kapitel/belegt.md"), "belegt").unwrap();
        app.open_document(dir.join("Documents/kapitel/eins.md"))
            .unwrap();

        let err = app.rename_current("belegt").unwrap_err();
        assert!(err.contains("already exists"), "{err}");
        assert_eq!(
            std::fs::read_to_string(dir.join("Documents/kapitel/belegt.md")).unwrap(),
            "belegt",
            "the other document is untouched"
        );
        assert!(dir.join("Documents/kapitel/eins.md").is_file());

        app.rename_current("zwei").unwrap();
        assert!(
            dir.join("Documents/kapitel/zwei.md").is_file(),
            "a rename stays in its folder"
        );
        assert!(!dir.join("Documents/zwei.md").exists());
        assert!(app.unsynced, "a rename is a local change");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn deleting_a_document_counts_as_a_change() {
        let dir = vault("delete-counts");
        let mut app = app_in(&dir);
        app.new_document().unwrap();
        app.doc.editor.insert_text("weg damit");
        app.save().unwrap();
        app.unsynced = false;
        app.delete_current().unwrap();
        assert!(app.unsynced, "the server still has the file");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_bare_name_never_escapes_the_vault_root() {
        let dir = vault("find-escape");
        let app = app_in(&dir);
        let outside = dir
            .parent()
            .unwrap()
            .join(format!("cread-aussen-{}.md", std::process::id()));
        std::fs::write(&outside, "geheim").unwrap();

        for name in ["../cread-aussen", "../../cread-aussen"] {
            assert!(
                app.find_document(name).is_none(),
                "{name:?} must not reach outside the vault"
            );
        }
        let _ = std::fs::remove_file(&outside);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn quitting_asks_before_leaving_unsynced_work() {
        let dir = vault("quit-prompt");
        let mut app = app_in(&dir);
        app.sync_cfg = SyncConfig {
            url: "https://example.org/dav".into(),
            enabled: true,
            ..SyncConfig::default()
        };
        app.sync = Some(Worker::spawn(
            dir.clone(),
            app.sync_cfg.clone(),
            Duration::from_secs(3600),
        ));
        assert!(!app.wants_sync_before_quitting(), "nothing written yet");

        app.unsynced = true;
        assert!(app.wants_sync_before_quitting());
        assert!(app.ask_quit(Mode::Action));
        assert_eq!(app.mode, Mode::Quit);
        assert_eq!(
            app.quit,
            Some(QuitPrompt {
                yes: true,
                from: Mode::Action
            })
        );

        app.compose();
        let text = rows_as_text(&app.screen);
        assert!(text.contains("SYNC TO SERVER?"), "{text}");
        assert!(text.contains("Yes") && text.contains("No"), "{text}");

        // Sync off means no question, ever.
        app.sync_cfg.enabled = false;
        app.mode = Mode::Action;
        app.quit = None;
        assert!(!app.ask_quit(Mode::Action));

        // A failed last round keeps the app open and the data unsynced.
        app.sync_cfg.url = "http://127.0.0.1:1".into();
        app.sync_cfg.enabled = true;
        assert!(app.sync_before_quit().is_err());
        assert!(app.unsynced, "nothing reached the server");
        assert!(app.sync.is_some(), "the worker is running again");

        app.sync = None;
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn open_on_the_front_door_shows_the_browser() {
        let dir = vault("front-door-open");
        let mut app = app_in(&dir);
        app.home = Home::Inspiro;
        app.mode = Mode::Action;
        app.compose();
        assert!(!rows_as_text(&app.screen).contains("+ new document"));

        // 'open' is a command line away, and the front door composed its own
        // screen; it used to return from compose before the browser was drawn.
        app.command = "open".into();
        app.open_browser();
        app.compose();
        let text = rows_as_text(&app.screen);
        assert!(text.contains("+ new document"), "browser is on screen");
        assert!(text.contains("Enter open"), "with its hint");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_inspiro_logo_belongs_to_its_own_screen_only() {
        let mut app = app();
        app.home = Home::Inspiro;
        app.mode = Mode::Action;
        app.compose();
        assert!(app.inspiro_at.is_some(), "front door shows its logo");
        assert!(app.splash_at.is_none(), "and not the boot logo");

        // Help, the browser and the boot screen all draw over the same grid, so
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
        app.mode = Mode::Browser;
        app.compose();
        assert!(app.splash_at.is_none());
    }
}
