//! WANDERLUST: the web, on the machine.
//!
//! A real browser underneath - Chromium renders the page, runs its scripts and
//! plays its videos - and CRead on top: the page is a picture at emulated
//! resolution with the CRT pass over it, the top bar shows where you are, and
//! the prompt is the shell's. Nothing of Chromium's own window is ever seen.
//!
//! The program owns the page and its gestures. It never touches the vault:
//! the browser's profile and downloads live outside it, so cookies and logins
//! never sync.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use cread_core::command::Command;
use cread_core::scrollbar::{self, Scroll};
use cread_core::{
    Cell, Frame, KeyPress, Outcome, PointerEvent, PointerKind, Program, Raster, Screen, TopBar,
    Waker,
};
use cread_web::address;
use cread_web::{Engine, Event, Mouse, Options, Request};

/// The stage under the top bar: 80 columns by 23 rows. A frame runs round its
/// edge; its right side doubles as the scrollbar.
pub const STAGE_COLS: u32 = 80;
pub const STAGE_ROWS: u32 = 23;
/// The page inside the frame, in cells and in pixels.
const PAGE_COLS: u32 = STAGE_COLS - 2;
const PAGE_ROWS: u32 = STAGE_ROWS - 2;
pub const PAGE_W: u32 = PAGE_COLS * 8;
pub const PAGE_H: u32 = PAGE_ROWS * 16;
/// Where the page starts inside the stage: one cell in from the frame.
const PAGE_X: i32 = 8;
const PAGE_Y: i32 = 16;

/// Page pixels per wheel line. The shell sends three lines a notch, so one
/// notch scrolls about three lines of text.
const WHEEL_LINE: i32 = 20;
/// Presses closer than this in time and space count as a double click.
const DOUBLE_CLICK: Duration = Duration::from_millis(400);
const DOUBLE_CLICK_SLOP: i32 = 4;
/// How long a notice stays in the status line.
const NOTICE: Duration = Duration::from_secs(4);
/// A page that sent a picture this recently is moving (a video, an
/// animation): the shell keeps its frame clock and the screensaver waits.
const MOVING: Duration = Duration::from_secs(1);

/// Where the browser keeps its things.
#[derive(Clone, Debug)]
pub struct Dirs {
    /// Cookies, history, logins: `~/.local/share/cread/browser`.
    pub profile: PathBuf,
    /// `~/Downloads`. Not in the vault, never synced.
    pub downloads: PathBuf,
}

impl Dirs {
    /// The usual places. A snap Chromium cannot see hidden folders, so its
    /// profile goes into the snap's own folder instead.
    pub fn for_binary(binary: Option<&std::path::Path>) -> Self {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/tmp"));
        let snap = binary.is_some_and(|b| b.starts_with("/snap"));
        let profile = if snap {
            home.join("snap/chromium/common/cread-browser")
        } else {
            std::env::var_os("XDG_DATA_HOME")
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .unwrap_or_else(|| home.join(".local/share"))
                .join("cread/browser")
        };
        Self {
            profile,
            downloads: home.join("Downloads"),
        }
    }
}

pub struct Wanderlust {
    binary: Option<PathBuf>,
    dirs: Dirs,
    engine: Option<Engine>,
    waker: Option<Waker>,
    /// Why there is no browser: not installed, or it stopped.
    trouble: Option<String>,
    frame: Option<Frame>,
    frame_at: Option<Instant>,
    url: String,
    title: String,
    loading: bool,
    scroll: Scroll,
    notice: Option<(String, Instant)>,
    /// Buttons held, as CDP's bit mask.
    buttons: u8,
    last_press: Option<(Instant, i32, i32, u8)>,
    clicks: u32,
    page: u8,
    ink: u8,
    dim: u8,
    accent: u8,
}

impl Default for Wanderlust {
    fn default() -> Self {
        Self::new()
    }
}

impl Wanderlust {
    pub fn new() -> Self {
        let binary = cread_web::find_binary();
        let dirs = Dirs::for_binary(binary.as_deref());
        Self::with(binary, dirs)
    }

    /// A browser with a chosen binary and folders (tests, screenshots).
    pub fn with(binary: Option<PathBuf>, dirs: Dirs) -> Self {
        Self {
            binary,
            dirs,
            engine: None,
            waker: None,
            trouble: None,
            frame: None,
            frame_at: None,
            url: String::new(),
            title: String::new(),
            loading: false,
            scroll: Scroll::default(),
            notice: None,
            buttons: 0,
            last_press: None,
            clicks: 0,
            page: 0,
            ink: 7,
            dim: 8,
            accent: 15,
        }
    }

    /// Go to what the writer typed: an address, or words to search for.
    pub fn go(&mut self, typed: &str) {
        let url = address::resolve(typed);
        self.send(Request::Navigate(url));
    }

    /// The page as it was last drawn, if any.
    pub fn url(&self) -> &str {
        &self.url
    }

    fn send(&mut self, request: Request) {
        if self.engine.is_none() {
            self.start();
        }
        if let Some(engine) = self.engine.as_ref() {
            engine.send(request);
        }
    }

    fn start(&mut self) {
        let Some(binary) = self.binary.clone() else {
            self.trouble = Some("Chromium is not installed: sudo apt install chromium".into());
            return;
        };
        let waker = self
            .waker
            .clone()
            .unwrap_or_else(|| std::sync::Arc::new(|| {}));
        let opts = Options {
            width: PAGE_W,
            height: PAGE_H,
            profile: self.dirs.profile.clone(),
            downloads: self.dirs.downloads.clone(),
        };
        match Engine::start(&binary, opts, waker) {
            Ok(engine) => {
                self.engine = Some(engine);
                self.trouble = None;
                self.loading = true;
            }
            Err(e) => self.trouble = Some(e),
        }
    }

    fn say(&mut self, text: impl Into<String>) {
        self.notice = Some((text.into(), Instant::now()));
    }

    fn mouse(&mut self, mouse: Mouse) {
        if let Some(engine) = self.engine.as_ref() {
            engine.send(Request::Mouse(mouse));
        }
    }

    /// Count a press as the first, second or third click of a run.
    fn click_count(&mut self, x: i32, y: i32, button: u8) -> u32 {
        let now = Instant::now();
        let repeat = self.last_press.is_some_and(|(at, px, py, pb)| {
            pb == button
                && now.duration_since(at) <= DOUBLE_CLICK
                && (px - x).abs() <= DOUBLE_CLICK_SLOP
                && (py - y).abs() <= DOUBLE_CLICK_SLOP
        });
        self.clicks = if repeat { (self.clicks % 3) + 1 } else { 1 };
        self.last_press = Some((now, x, y, button));
        self.clicks
    }

    /// A thin line round the page, the way CRead frames its windows.
    fn draw_frame(&self, screen: &mut Screen) {
        let (right, bottom) = (STAGE_COLS - 1, STAGE_ROWS - 1);
        // Ink, not the dim slot: on the paper theme a dim line vanishes
        // against a white page.
        let line = |ch| Cell::new(ch, self.ink, self.page);
        screen.put(0, 0, line('\u{250c}'));
        screen.put(right, 0, line('\u{2510}'));
        screen.put(0, bottom, line('\u{2514}'));
        screen.put(right, bottom, line('\u{2518}'));
        for x in 1..right {
            screen.put(x, 0, line('\u{2500}'));
            screen.put(x, bottom, line('\u{2500}'));
        }
        for y in 1..bottom {
            screen.put(0, y, line('\u{2502}'));
            screen.put(right, y, line('\u{2502}'));
        }
    }

    fn draw_door(&self, screen: &mut Screen) {
        let centre = |screen: &mut Screen, y: u32, text: &str, fg: u8| {
            let width = text.chars().count() as u32;
            let x = 1 + PAGE_COLS.saturating_sub(width) / 2;
            screen.put_str(x, y, text, fg, self.page);
        };
        let top = 1 + PAGE_ROWS / 2 - 3;
        centre(screen, top, "W A N D E R L U S T", self.accent);
        centre(
            screen,
            top + 1,
            "the whole wide world, on a quiet screen.",
            self.dim,
        );
        let line = match (&self.trouble, self.engine.is_some()) {
            (Some(trouble), _) => trouble.clone(),
            (None, true) => "dialling in ...".to_string(),
            (None, false) => {
                "Esc, then: web <address>   or   web <words to search for>".to_string()
            }
        };
        centre(screen, top + 4, &line, self.ink);
    }
}

impl Program for Wanderlust {
    fn title(&self) -> &str {
        "WANDERLUST"
    }

    fn key(&mut self, key: KeyPress) -> Outcome {
        if self.engine.is_some() {
            self.send(Request::Key(key));
        }
        Outcome::Continue
    }

    fn pointer(&mut self, event: PointerEvent) -> Outcome {
        let PointerEvent { x, y, kind } = event;
        let (x, y) = (x - PAGE_X, y - PAGE_Y);
        let inside = (0..PAGE_W as i32).contains(&x) && (0..PAGE_H as i32).contains(&y);
        let (x, y) = (x.clamp(0, PAGE_W as i32 - 1), y.clamp(0, PAGE_H as i32 - 1));
        match kind {
            PointerKind::Move => {
                if inside || self.buttons != 0 {
                    self.mouse(Mouse::Move {
                        x,
                        y,
                        buttons: self.buttons,
                    });
                }
            }
            PointerKind::Down(button) if inside => {
                let clicks = self.click_count(x, y, button);
                self.buttons |= cread_web::input::button_bit(button);
                self.mouse(Mouse::Press {
                    x,
                    y,
                    button,
                    clicks,
                    buttons: self.buttons,
                });
            }
            PointerKind::Down(_) => {}
            PointerKind::Up(button) => {
                // A release always reaches the page, or a drag stays stuck.
                let bit = cread_web::input::button_bit(button);
                if self.buttons & bit != 0 {
                    self.buttons &= !bit;
                    self.mouse(Mouse::Release {
                        x,
                        y,
                        button,
                        clicks: self.clicks.max(1),
                        buttons: self.buttons,
                    });
                }
            }
            PointerKind::Wheel(lines) if inside => self.mouse(Mouse::Wheel {
                x,
                y,
                delta: wheel_delta(lines),
            }),
            PointerKind::Wheel(_) => {}
        }
        Outcome::Continue
    }

    fn draw(&mut self, screen: &mut Screen, _raster: &mut Raster) {
        if self.frame.is_none() {
            for y in 1..=PAGE_ROWS {
                for x in 1..=PAGE_COLS {
                    screen.put(x, y, Cell::blank(self.ink, self.page));
                }
            }
            self.draw_door(screen);
        }
        self.draw_frame(screen);
        // The right edge is the scrollbar: its track is the frame's own line,
        // so a page that fits shows a plain frame.
        scrollbar::draw_scrollbar(
            screen,
            STAGE_COLS - 1,
            1,
            PAGE_ROWS,
            self.scroll,
            self.ink,
            self.accent,
            self.page,
        );
    }

    fn wants_frames(&self) -> bool {
        self.frame_at.is_some_and(|at| at.elapsed() < MOVING)
    }

    fn frame(&self) -> Option<(i32, i32, &Frame)> {
        self.frame.as_ref().map(|frame| (PAGE_X, PAGE_Y, frame))
    }

    fn set_waker(&mut self, waker: Waker) {
        self.waker = Some(waker);
    }

    fn poll(&mut self) -> bool {
        let mut changed = false;
        let mut events = Vec::new();
        if let Some(engine) = self.engine.as_ref() {
            while let Some(event) = engine.try_recv() {
                events.push(event);
            }
        }
        for event in events {
            changed = true;
            match event {
                Event::Frame(frame) => {
                    self.frame = Some(frame);
                    self.frame_at = Some(Instant::now());
                }
                Event::Page { url, title } => {
                    self.url = url;
                    self.title = title;
                }
                Event::Loading(on) => self.loading = on,
                Event::Scroll(scroll) => self.scroll = scroll,
                Event::Notice(text) => self.say(text),
                Event::Exited(why) => {
                    self.engine = None;
                    self.loading = false;
                    self.trouble = Some(format!("{why} - reload to start it again"));
                    self.say(format!("{why} - reload to start it again"));
                }
            }
        }
        if self
            .notice
            .as_ref()
            .is_some_and(|(_, at)| at.elapsed() >= NOTICE)
        {
            self.notice = None;
            changed = true;
        }
        changed
    }

    fn command(&mut self, command: &Command) -> Result<String, String> {
        match command {
            Command::Web(typed) => {
                self.go(typed.as_deref().unwrap_or(""));
                Ok(String::new())
            }
            Command::Back => {
                self.send(Request::Back);
                Ok(String::new())
            }
            Command::Forward => {
                self.send(Request::Forward);
                Ok(String::new())
            }
            Command::Reload => {
                if self.engine.is_none() && !self.url.is_empty() {
                    // The browser stopped: start it again where it was.
                    let url = self.url.clone();
                    self.send(Request::Navigate(url));
                } else {
                    self.send(Request::Reload);
                }
                Ok(String::new())
            }
            _ => Err("not in the browser".into()),
        }
    }

    fn status(&self) -> String {
        match &self.notice {
            Some((text, _)) => text.clone(),
            None if self.loading => "loading".into(),
            None => String::new(),
        }
    }

    fn top_bar(&self) -> Option<TopBar> {
        // Until a page names itself Chromium reports its address as the title;
        // the location already says that.
        let bare = self
            .url
            .split_once("://")
            .map_or(self.url.as_str(), |(_, rest)| rest);
        let title = if self.title.is_empty() || self.title == self.url || self.title == bare {
            "WANDERLUST".to_string()
        } else {
            self.title.clone()
        };
        Some(TopBar {
            title,
            state: if self.loading {
                "loading".into()
            } else {
                String::new()
            },
            location: location(&self.url),
        })
    }

    fn paste(&mut self, text: &str) -> bool {
        if self.engine.is_none() {
            return false;
        }
        self.send(Request::Text(text.to_string()));
        true
    }

    fn palette(&mut self, background: u8, ink: u8) {
        self.page = background;
        self.ink = ink;
    }

    fn chrome(&mut self, dim: u8, accent: u8) {
        self.dim = dim;
        self.accent = accent;
    }
}

/// The shell counts a wheel turned up (away from you) as positive lines; a
/// page scrolls down for a positive delta. Hence the sign.
fn wheel_delta(lines: i32) -> i32 {
    -lines * WHEEL_LINE
}

/// What the bar says about where the page is from. The scheme stays visible:
/// whether a page is encrypted is not something to hide behind an icon the
/// font does not have.
pub fn location(url: &str) -> String {
    if url.is_empty() {
        return String::new();
    }
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("https://") || lower.starts_with("http://") {
        let scheme = if address::is_secure(url) {
            "https"
        } else {
            "http"
        };
        return format!("{scheme}://{}", address::host(url));
    }
    // data:, about:, chrome-error: - name the kind, not the whole payload.
    let kind = url.split(':').next().unwrap_or("");
    format!("{kind}:")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn browser() -> Wanderlust {
        Wanderlust::with(
            None,
            Dirs {
                profile: "/nonexistent/profile".into(),
                downloads: "/nonexistent/downloads".into(),
            },
        )
    }

    #[test]
    fn the_bar_shows_the_real_host_and_the_scheme() {
        assert_eq!(
            location("https://de.wikipedia.org/wiki/X"),
            "https://de.wikipedia.org"
        );
        assert_eq!(location("http://neverssl.com/"), "http://neverssl.com");
        assert_eq!(
            location("https://user@bank.example.evil.com/"),
            "https://bank.example.evil.com"
        );
        assert_eq!(location("data:text/html,<h1>long</h1>"), "data:");
        assert_eq!(location(""), "");
    }

    #[test]
    fn without_chromium_the_door_says_how_to_get_it() {
        let mut web = browser();
        web.go("example.com");
        assert!(web.engine.is_none());
        assert!(
            web.trouble
                .as_deref()
                .unwrap()
                .contains("apt install chromium")
        );
        let mut screen = Screen::new(80, 25);
        let mut raster = Raster::new(640, 368);
        web.draw(&mut screen, &mut raster);
        let text: String = (0..80)
            .filter_map(|x| screen.get(x, 1 + PAGE_ROWS / 2 + 1).map(|c| c.ch))
            .collect();
        assert!(text.contains("apt install chromium"), "{text}");
    }

    #[test]
    fn clicks_in_a_row_count_up() {
        let mut web = browser();
        assert_eq!(web.click_count(10, 10, 0), 1);
        assert_eq!(web.click_count(11, 10, 0), 2);
        assert_eq!(web.click_count(11, 11, 0), 3);
        assert_eq!(web.click_count(11, 11, 0), 1);
        assert_eq!(web.click_count(100, 11, 0), 1);
        assert_eq!(web.click_count(100, 11, 1), 1);
    }

    #[test]
    fn the_top_bar_never_hides_the_location() {
        let mut web = browser();
        web.url = "https://example.com/a".into();
        web.title = "Example".into();
        let bar = web.top_bar().unwrap();
        assert_eq!(bar.title, "Example");
        assert_eq!(bar.location, "https://example.com");
    }

    #[test]
    fn the_page_sits_inside_a_frame() {
        let mut web = browser();
        let mut screen = Screen::new(80, 25);
        let mut raster = Raster::new(640, 368);
        web.draw(&mut screen, &mut raster);
        assert_eq!(screen.get(0, 0).unwrap().ch, '\u{250c}');
        assert_eq!(screen.get(79, 22).unwrap().ch, '\u{2518}');
        assert_eq!(screen.get(79, 5).unwrap().ch, '\u{2502}');
        assert_eq!((PAGE_W, PAGE_H), (624, 336));
        web.frame = Some(Frame {
            width: PAGE_W,
            height: PAGE_H,
            rgba: vec![0; (PAGE_W * PAGE_H * 4) as usize],
        });
        assert_eq!(web.frame().map(|(x, y, _)| (x, y)), Some((8, 16)));
    }

    #[test]
    fn the_wheel_turned_up_scrolls_the_page_up() {
        assert!(wheel_delta(3) < 0, "up on the wheel is up on the page");
        assert!(wheel_delta(-3) > 0);
        assert_eq!(wheel_delta(-3), 60);
    }
}
