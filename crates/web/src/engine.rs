//! The browser engine: a headless Chromium driven over a private pipe.
//!
//! Two threads. The reader turns the pipe into messages; the worker owns the
//! write end, answers the UI's requests and the browser's events, and decodes
//! frames. Both feed one inbox, so there are no locks. The UI never blocks: it
//! drops [`Request`]s in and picks [`Event`]s up when the [`Waker`] rang.

use std::collections::HashMap;
use std::io::{PipeWriter, Read, Write};
use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use base64::Engine as _;
use command_fds::{CommandFdExt, FdMapping};
use cread_core::scrollbar::Scroll;
use cread_core::{Frame, KeyPress, Waker};
use serde_json::{Value, json};

use crate::cdp::{Decoder, encode};
use crate::input::{Mouse, key_events, mouse_event};

/// How the engine is set up.
#[derive(Clone, Debug)]
pub struct Options {
    /// The page size in pixels: exactly what the shell shows, never scaled.
    pub width: u32,
    pub height: u32,
    /// Cookies, history and logins. Outside the vault, so they never sync.
    pub profile: PathBuf,
    /// Where downloads land. Also outside the vault.
    pub downloads: PathBuf,
}

/// What the UI asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    Navigate(String),
    Back,
    Forward,
    Reload,
    Key(KeyPress),
    /// Text inserted at the caret, as a paste does.
    Text(String),
    Mouse(Mouse),
}

/// What the engine reports.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// A new picture of the page.
    Frame(Frame),
    /// The page's address or title changed.
    Page {
        url: String,
        title: String,
    },
    Loading(bool),
    /// Where the page is scrolled, in CSS pixels.
    Scroll(Scroll),
    /// One line for the status bar: a download, a dialog, an error.
    Notice(String),
    /// The browser is gone; the text says why.
    Exited(String),
}

enum Inbox {
    Ui(Request),
    Cdp(Value),
    /// The read end saw EOF: Chromium closed the pipe.
    Gone,
    Close,
}

/// The running browser.
pub struct Engine {
    tx: Sender<Inbox>,
    rx: Receiver<Event>,
    worker: Option<JoinHandle<()>>,
}

/// Chromium, if this machine has one. `CREAD_CHROMIUM` names another binary.
pub fn find_binary() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("CREAD_CHROMIUM") {
        return Some(PathBuf::from(path));
    }
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        for name in ["chromium", "chromium-browser"] {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// The command line. The sandbox stays on: there is no `--no-sandbox` here,
/// and there must never be one.
pub fn arguments(opts: &Options) -> Vec<String> {
    vec![
        "--headless=new".into(),
        "--remote-debugging-pipe".into(),
        format!("--user-data-dir={}", opts.profile.display()),
        format!("--window-size={},{}", opts.width, opts.height),
        "--force-device-scale-factor=1".into(),
        "--hide-scrollbars".into(),
        "--no-first-run".into(),
        "--no-default-browser-check".into(),
        "about:blank".into(),
    ]
}

impl Engine {
    /// Start Chromium and the worker. Fails when there is no browser to start.
    pub fn start(binary: &Path, opts: Options, waker: Waker) -> Result<Self, String> {
        std::fs::create_dir_all(&opts.profile)
            .map_err(|e| format!("cannot create {}: {e}", opts.profile.display()))?;

        let (to_chrome_r, to_chrome_w) = std::io::pipe().map_err(|e| format!("pipe: {e}"))?;
        let (from_chrome_r, from_chrome_w) = std::io::pipe().map_err(|e| format!("pipe: {e}"))?;
        let child = {
            let mut command = Command::new(binary);
            command
                .args(arguments(&opts))
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            command
                .fd_mappings(vec![
                    FdMapping {
                        parent_fd: OwnedFd::from(to_chrome_r),
                        child_fd: 3,
                    },
                    FdMapping {
                        parent_fd: OwnedFd::from(from_chrome_w),
                        child_fd: 4,
                    },
                ])
                .map_err(|e| format!("pipe: {e}"))?;
            command
                .spawn()
                .map_err(|e| format!("cannot start {}: {e}", binary.display()))?
            // `command` drops here and closes our copies of the child's ends,
            // so EOF on the read end really means the browser is gone.
        };

        let (tx, inbox) = mpsc::channel();
        let (events, rx) = mpsc::channel();

        let reader_tx = tx.clone();
        let mut pipe = from_chrome_r;
        thread::spawn(move || {
            let mut decoder = Decoder::default();
            let mut buf = vec![0u8; 1 << 16];
            loop {
                match pipe.read(&mut buf) {
                    Ok(0) | Err(_) => {
                        let _ = reader_tx.send(Inbox::Gone);
                        return;
                    }
                    Ok(n) => {
                        for message in decoder.push(&buf[..n]) {
                            if reader_tx.send(Inbox::Cdp(message)).is_err() {
                                return;
                            }
                        }
                    }
                }
            }
        });

        let worker = thread::spawn(move || {
            let mut worker = Worker {
                child,
                out: to_chrome_w,
                events,
                waker,
                opts,
                next_id: 0,
                pending: HashMap::new(),
                target: None,
                session: None,
                queued: Vec::new(),
                last_frame: None,
                sharp_due: false,
                shooting: false,
                metrics_at: None,
                downloads: HashMap::new(),
            };
            worker.run(inbox);
        });

        Ok(Self {
            tx,
            rx,
            worker: Some(worker),
        })
    }

    pub fn send(&self, request: Request) {
        let _ = self.tx.send(Inbox::Ui(request));
    }

    pub fn try_recv(&self) -> Option<Event> {
        self.rx.try_recv().ok()
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.tx.send(Inbox::Close);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// What a reply is for.
enum Pending {
    Targets,
    Attach,
    History(i64),
    Navigate,
    TargetInfo,
    Screenshot,
    Metrics,
    Close,
}

/// After the last moving frame, how long to wait before fetching a sharp one.
const SETTLE: Duration = Duration::from_millis(250);
/// Scroll position is asked for at most this often while frames stream in.
const METRICS_EVERY: Duration = Duration::from_millis(150);

struct Worker {
    child: Child,
    out: PipeWriter,
    events: Sender<Event>,
    waker: Waker,
    opts: Options,
    next_id: u64,
    pending: HashMap<u64, Pending>,
    target: Option<String>,
    session: Option<String>,
    /// Requests that came in before the page was attached.
    queued: Vec<Request>,
    last_frame: Option<Instant>,
    /// Moving frames are JPEG; once the page is still, one PNG replaces the
    /// last of them so text is sharp again.
    sharp_due: bool,
    /// A sharp frame is on its way. Taking it makes the compositor draw once
    /// more, and the screencast reports that echo; it must not count as
    /// movement, or the two would ping-pong forever.
    shooting: bool,
    metrics_at: Option<Instant>,
    /// Download guid to file name.
    downloads: HashMap<String, String>,
}

impl Worker {
    fn run(&mut self, inbox: Receiver<Inbox>) {
        self.call(
            None,
            "Target.setDiscoverTargets",
            json!({ "discover": true }),
            None,
        );
        self.call(None, "Target.getTargets", json!({}), Some(Pending::Targets));

        loop {
            match inbox.recv_timeout(Duration::from_millis(100)) {
                Ok(Inbox::Ui(request)) => {
                    if self.session.is_some() {
                        self.request(request);
                    } else {
                        self.queued.push(request);
                    }
                }
                Ok(Inbox::Cdp(message)) => self.message(message),
                Ok(Inbox::Gone) => {
                    let status = self.child.wait().ok();
                    self.emit(Event::Exited(match status {
                        Some(status) if !status.success() => {
                            format!("the browser stopped ({status})")
                        }
                        _ => "the browser stopped".into(),
                    }));
                    return;
                }
                Ok(Inbox::Close) | Err(RecvTimeoutError::Disconnected) => {
                    self.shutdown(&inbox);
                    return;
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
            self.maybe_sharpen();
        }
    }

    fn emit(&self, event: Event) {
        if self.events.send(event).is_ok() {
            (self.waker)();
        }
    }

    /// Send a command. Replies we do not care about are simply dropped.
    fn call(
        &mut self,
        session: Option<&str>,
        method: &str,
        params: Value,
        pending: Option<Pending>,
    ) {
        self.next_id += 1;
        let id = self.next_id;
        if let Some(pending) = pending {
            self.pending.insert(id, pending);
        }
        let bytes = encode(id, method, params, session);
        // A failed write means the browser is gone; the reader reports it.
        let _ = self.out.write_all(&bytes);
    }

    /// A command to the page.
    fn page(&mut self, method: &str, params: Value, pending: Option<Pending>) {
        let session = self.session.clone();
        self.call(session.as_deref(), method, params, pending);
    }

    fn request(&mut self, request: Request) {
        match request {
            Request::Navigate(url) => self.page(
                "Page.navigate",
                json!({ "url": url }),
                Some(Pending::Navigate),
            ),
            Request::Back => self.page(
                "Page.getNavigationHistory",
                json!({}),
                Some(Pending::History(-1)),
            ),
            Request::Forward => self.page(
                "Page.getNavigationHistory",
                json!({}),
                Some(Pending::History(1)),
            ),
            Request::Reload => self.page("Page.reload", json!({}), None),
            Request::Key(key) => {
                for event in key_events(key).unwrap_or_default() {
                    self.page("Input.dispatchKeyEvent", event, None);
                }
            }
            Request::Text(text) => self.page("Input.insertText", json!({ "text": text }), None),
            Request::Mouse(mouse) => {
                self.page("Input.dispatchMouseEvent", mouse_event(mouse), None)
            }
        }
    }

    fn message(&mut self, message: Value) {
        if let Some(id) = message.get("id").and_then(Value::as_u64) {
            if let Some(pending) = self.pending.remove(&id) {
                self.reply(pending, &message);
            }
            return;
        }
        let method = message["method"].as_str().unwrap_or("");
        let params = &message["params"];
        match method {
            "Page.screencastFrame" => self.screencast_frame(params),
            "Target.targetInfoChanged" => self.target_changed(&params["targetInfo"]),
            "Page.frameStartedLoading" | "Page.frameStoppedLoading" => {
                if params["frameId"].as_str() == self.target.as_deref() {
                    let stopped = method == "Page.frameStoppedLoading";
                    self.emit(Event::Loading(!stopped));
                    if stopped {
                        self.ask_target_info();
                    }
                }
            }
            // Titles change without a target event often enough (and pages
            // that navigate by script never load): ask when it may have.
            "Page.navigatedWithinDocument" | "Page.loadEventFired" => self.ask_target_info(),
            "Page.javascriptDialogOpening" => {
                // A headless page has nobody to click OK: accept, and say what
                // it wanted, or the page hangs on the dialog.
                self.page(
                    "Page.handleJavaScriptDialog",
                    json!({ "accept": true }),
                    None,
                );
                let text = params["message"].as_str().unwrap_or("").trim().to_string();
                if !text.is_empty() {
                    self.emit(Event::Notice(format!("page says: {text}")));
                }
            }
            "Browser.downloadWillBegin" => {
                let guid = params["guid"].as_str().unwrap_or("").to_string();
                let name = params["suggestedFilename"]
                    .as_str()
                    .unwrap_or("file")
                    .to_string();
                self.emit(Event::Notice(format!("downloading {name}")));
                self.downloads.insert(guid, name);
            }
            "Browser.downloadProgress" => {
                let guid = params["guid"].as_str().unwrap_or("");
                match params["state"].as_str() {
                    Some("completed") => {
                        let name = self.downloads.remove(guid).unwrap_or_default();
                        let notice = format!("saved {name} in {}", self.opts.downloads.display());
                        self.emit(Event::Notice(notice));
                    }
                    Some("canceled") => {
                        let name = self.downloads.remove(guid).unwrap_or_default();
                        self.emit(Event::Notice(format!("download of {name} stopped")));
                    }
                    _ => {}
                }
            }
            "Inspector.targetCrashed" => self.emit(Event::Notice(
                "the page crashed - reload to try again".into(),
            )),
            _ => {}
        }
    }

    fn reply(&mut self, pending: Pending, message: &Value) {
        let result = &message["result"];
        match pending {
            Pending::Targets => {
                let page = result["targetInfos"]
                    .as_array()
                    .and_then(|targets| targets.iter().find(|t| t["type"] == "page"))
                    .and_then(|t| t["targetId"].as_str())
                    .map(str::to_string);
                match page {
                    Some(id) => {
                        self.target = Some(id.clone());
                        self.call(
                            None,
                            "Target.attachToTarget",
                            json!({ "targetId": id, "flatten": true }),
                            Some(Pending::Attach),
                        );
                    }
                    None => self.emit(Event::Notice("the browser has no page".into())),
                }
            }
            Pending::Attach => {
                let Some(session) = result["sessionId"].as_str() else {
                    self.emit(Event::Notice("cannot attach to the page".into()));
                    return;
                };
                self.session = Some(session.to_string());
                self.setup();
                for request in std::mem::take(&mut self.queued) {
                    self.request(request);
                }
            }
            Pending::History(delta) => {
                let index = result["currentIndex"].as_i64().unwrap_or(0) + delta;
                let entry = result["entries"]
                    .as_array()
                    .and_then(|entries| entries.get(usize::try_from(index).ok()?))
                    .and_then(|entry| entry["id"].as_i64());
                match entry {
                    Some(id) => self.page(
                        "Page.navigateToHistoryEntry",
                        json!({ "entryId": id }),
                        None,
                    ),
                    None => self.emit(Event::Notice(
                        if delta < 0 {
                            "no page to go back to"
                        } else {
                            "no page to go forward to"
                        }
                        .into(),
                    )),
                }
            }
            Pending::Navigate => {
                if let Some(error) = result["errorText"].as_str()
                    && !error.is_empty()
                {
                    self.emit(Event::Notice(
                        error.trim_start_matches("net::").to_lowercase(),
                    ));
                }
            }
            Pending::Screenshot => {
                self.shooting = false;
                if let Some(frame) = result["data"].as_str().and_then(decode_png) {
                    self.emit(Event::Frame(frame));
                }
                self.ask_metrics();
            }
            Pending::Metrics => {
                let viewport = &result["cssLayoutViewport"];
                let content = &result["cssContentSize"];
                let to_usize = |v: &Value| v.as_f64().unwrap_or(0.0).max(0.0).round() as usize;
                self.emit(Event::Scroll(Scroll {
                    first: to_usize(&viewport["pageY"]),
                    visible: to_usize(&viewport["clientHeight"]),
                    total: to_usize(&content["height"]),
                }));
            }
            Pending::TargetInfo => {
                let info = result["targetInfo"].clone();
                self.target_changed(&info);
            }
            Pending::Close => {}
        }
    }

    /// Everything a fresh page needs once it is attached.
    fn setup(&mut self) {
        let (width, height) = (self.opts.width, self.opts.height);
        self.page("Page.enable", json!({}), None);
        self.page(
            "Emulation.setDeviceMetricsOverride",
            json!({ "width": width, "height": height, "deviceScaleFactor": 1, "mobile": false }),
            None,
        );
        let _ = std::fs::create_dir_all(&self.opts.downloads);
        let downloads = self.opts.downloads.display().to_string();
        self.call(
            None,
            "Browser.setDownloadBehavior",
            json!({ "behavior": "allow", "downloadPath": downloads, "eventsEnabled": true }),
            None,
        );
        self.page(
            "Page.startScreencast",
            json!({
                "format": "jpeg", "quality": 85,
                "maxWidth": width, "maxHeight": height, "everyNthFrame": 1,
            }),
            None,
        );
    }

    fn screencast_frame(&mut self, params: &Value) {
        if let Some(id) = params["sessionId"].as_i64() {
            // Unacknowledged, the browser stops sending after a few frames.
            self.page("Page.screencastFrameAck", json!({ "sessionId": id }), None);
        }
        if self.shooting {
            return;
        }
        if let Some(frame) = params["data"].as_str().and_then(decode_jpeg) {
            self.emit(Event::Frame(frame));
        }
        self.last_frame = Some(Instant::now());
        self.sharp_due = true;
        if self
            .metrics_at
            .is_none_or(|at| at.elapsed() >= METRICS_EVERY)
        {
            self.ask_metrics();
        }
    }

    fn ask_target_info(&mut self) {
        if let Some(id) = self.target.clone() {
            self.call(
                None,
                "Target.getTargetInfo",
                json!({ "targetId": id }),
                Some(Pending::TargetInfo),
            );
        }
    }

    fn ask_metrics(&mut self) {
        self.metrics_at = Some(Instant::now());
        self.page("Page.getLayoutMetrics", json!({}), Some(Pending::Metrics));
    }

    fn maybe_sharpen(&mut self) {
        if self.sharp_due && self.last_frame.is_some_and(|at| at.elapsed() >= SETTLE) {
            self.sharp_due = false;
            self.shooting = true;
            self.page(
                "Page.captureScreenshot",
                json!({ "format": "png", "captureBeyondViewport": false }),
                Some(Pending::Screenshot),
            );
        }
    }

    fn target_changed(&mut self, info: &Value) {
        let id = info["targetId"].as_str().unwrap_or("");
        let url = info["url"].as_str().unwrap_or("");
        if Some(id) == self.target.as_deref() {
            let title = info["title"].as_str().unwrap_or("").to_string();
            self.emit(Event::Page {
                url: url.to_string(),
                title,
            });
            return;
        }
        // One page at a time: a link that wants a new window opens here.
        let ours = self
            .target
            .as_deref()
            .is_some_and(|t| info["openerId"] == t);
        if ours && info["type"] == "page" && !url.is_empty() && url != "about:blank" {
            let url = url.to_string();
            self.call(None, "Target.closeTarget", json!({ "targetId": id }), None);
            self.page(
                "Page.navigate",
                json!({ "url": url }),
                Some(Pending::Navigate),
            );
        }
    }

    /// Ask the browser to close, then make sure it did.
    fn shutdown(&mut self, inbox: &Receiver<Inbox>) {
        self.call(None, "Browser.close", json!({}), Some(Pending::Close));
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return;
            }
            let _ = inbox.recv_timeout(Duration::from_millis(50));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn decode_base64(data: &str) -> Option<Vec<u8>> {
    base64::engine::general_purpose::STANDARD.decode(data).ok()
}

fn decode_jpeg(data: &str) -> Option<Frame> {
    use zune_core::colorspace::ColorSpace;
    use zune_core::options::DecoderOptions;
    let bytes = decode_base64(data)?;
    let options = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGBA);
    let mut decoder =
        zune_jpeg::JpegDecoder::new_with_options(std::io::Cursor::new(&bytes), options);
    let rgba = decoder.decode().ok()?;
    let info = decoder.info()?;
    let (width, height) = (u32::from(info.width), u32::from(info.height));
    (rgba.len() == (width * height * 4) as usize).then_some(Frame {
        width,
        height,
        rgba,
    })
}

fn decode_png(data: &str) -> Option<Frame> {
    let bytes = decode_base64(data)?;
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info().ok()?;
    let mut buffer = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buffer).ok()?;
    let data = &buffer[..info.buffer_size()];
    let rgba = match info.color_type {
        png::ColorType::Rgba => data.to_vec(),
        png::ColorType::Rgb => data
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2], 0xff])
            .collect(),
        _ => return None,
    };
    Some(Frame {
        width: info.width,
        height: info.height,
        rgba,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sandbox_is_never_switched_off() {
        let opts = Options {
            width: 632,
            height: 368,
            profile: "/p".into(),
            downloads: "/d".into(),
        };
        let args = arguments(&opts);
        assert!(!args.iter().any(|a| a.contains("no-sandbox")));
        assert!(args.contains(&"--remote-debugging-pipe".to_string()));
        // A port would let any process on the machine drive the browser.
        assert!(
            !args
                .iter()
                .any(|a| a.starts_with("--remote-debugging-port"))
        );
        assert!(args.contains(&"--window-size=632,368".to_string()));
        assert!(args.contains(&"--user-data-dir=/p".to_string()));
    }

    #[test]
    fn a_png_frame_decodes() {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, 2, 1);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&[255, 0, 0, 0, 0, 255]).unwrap();
        }
        let data = base64::engine::general_purpose::STANDARD.encode(&bytes);
        let frame = decode_png(&data).unwrap();
        assert_eq!((frame.width, frame.height), (2, 1));
        assert_eq!(frame.rgba, vec![255, 0, 0, 255, 0, 0, 255, 255]);
    }

    #[test]
    fn garbage_is_not_a_frame() {
        assert!(decode_jpeg("bm90IGEganBlZw==").is_none());
        assert!(decode_png("!!").is_none());
    }

    /// Drives a real Chromium. Run with `cargo test -p cread-web -- --ignored`
    /// on a machine that has one.
    #[test]
    #[ignore]
    fn a_real_browser_renders_and_closes() {
        let binary = find_binary().expect("no chromium on PATH");
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap();
        // The snap cannot see hidden folders or the real /tmp.
        let base = home.join("snap/chromium/common/cread-engine-test");
        let opts = Options {
            width: 632,
            height: 368,
            profile: base.join("profile"),
            downloads: base.join("downloads"),
        };
        let (wake_tx, wake_rx) = mpsc::channel();
        let wake_tx = std::sync::Mutex::new(wake_tx);
        let waker: Waker = std::sync::Arc::new(move || {
            let _ = wake_tx.lock().unwrap().send(());
        });
        let started = Instant::now();
        let engine = Engine::start(&binary, opts, waker).unwrap();
        engine.send(Request::Navigate(
            "data:text/html,<title>Hallo</title><h1>Wanderlust</h1>".into(),
        ));
        let (mut frame, mut url, mut frames) = (None, String::new(), 0);
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline && (frame.is_none() || !url.starts_with("data:")) {
            let _ = wake_rx.recv_timeout(Duration::from_millis(200));
            while let Some(event) = engine.try_recv() {
                match event {
                    Event::Frame(f) => {
                        frames += 1;
                        frame = Some(f);
                    }
                    Event::Page { url: u, .. } => url = u,
                    Event::Exited(why) => panic!("{why}"),
                    _ => {}
                }
            }
        }
        eprintln!("first page after {:?}, {frames} frames", started.elapsed());
        let frame = frame.expect("no frame");
        assert_eq!((frame.width, frame.height), (632, 368));
        // A still page settles: one sharp frame, then silence.
        std::thread::sleep(Duration::from_secs(2));
        while engine.try_recv().is_some() {}
        std::thread::sleep(Duration::from_secs(2));
        let mut later = 0;
        while let Some(event) = engine.try_recv() {
            if matches!(event, Event::Frame(_)) {
                later += 1;
            }
        }
        assert_eq!(later, 0, "a still page keeps sending frames");
        let closing = Instant::now();
        drop(engine);
        eprintln!("closed in {:?}", closing.elapsed());
        assert!(closing.elapsed() < Duration::from_secs(3));
    }
}
