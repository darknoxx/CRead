//! Window lifetime, frame loop and input.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use cread_core::{
    Editor, ScaleMode, Screen, Stats, Theme, Viewport,
    command::{self, Command, CommandError},
    resolution::target_line_px_for_scale_factor,
    storage,
};
use cread_render::{CrtSettings, Font, FrameStatus, Framebuffer, Renderer, raster};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey};
use winit::window::{Fullscreen, Window, WindowId};

use crate::cli::Options;
use crate::ui;
use cread_sync::{Config as SyncConfig, Event as SyncEvent, Report as SyncReport, Worker};

const WELCOME: &str = include_str!("welcome.md");

/// How long the editor waits after the last keystroke before writing to disk.
const AUTOSAVE_DELAY: Duration = Duration::from_millis(1500);

/// Frame budget while typing. The flicker is quantised to 24 Hz anyway.
const ACTIVE_FPS: u32 = 30;
/// While idle the screen is static; we only redraw for the cursor blink.
const IDLE_FPS: u32 = 2;
/// How long after the last keystroke we stay on the fast path.
const ACTIVE_WINDOW: Duration = Duration::from_millis(1200);

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
    title_hint: Option<&'static str>,
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
            None => self.title_hint.unwrap_or("untitled").to_string(),
        }
    }
}

#[derive(Clone)]
enum Action {
    Open(PathBuf),
    NewFile,
}

struct Palette {
    input: String,
    entries: Vec<(String, Action)>,
    filtered: Vec<usize>,
    selected: usize,
    title: String,
}

impl Palette {
    fn empty() -> Self {
        Self {
            input: String::new(),
            entries: Vec::new(),
            filtered: Vec::new(),
            selected: 0,
            title: "OPEN".into(),
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Typing into the document.
    Write,
    /// A command line at the bottom, like a shell.
    Action,
    /// File picker.
    Palette,
    /// WebDAV settings.
    Settings,
    /// Full-screen help or about.
    Help,
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
    crt_base: CrtSettings,

    start: Instant,
    last_frame: Instant,
    next_frame: Instant,
    last_input: Instant,
    fps: f32,
    dirty: bool,
    blink: bool,
    mods: ModifiersState,
    typewriter: bool,

    doc: Document,
    /// False on the start screen: no file is open yet.
    has_document: bool,
    mode: Mode,
    palette: Palette,
    form: FormState,
    status: String,
    screenshot_done: bool,

    /// Action-mode command line.
    command: String,
    command_error: String,
    history: Vec<String>,
    history_pos: Option<usize>,
    /// Pending yes/no question, e.g. before deleting a file.
    confirm: Option<String>,
    help_about: bool,

    sync_cfg: SyncConfig,
    sync: Option<Worker>,
    sync_label: String,
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
        let crt = opts.crt;
        let theme = opts.theme;
        let (doc, has_document) = Self::initial_document(&opts);
        let sync_cfg = SyncConfig::load();
        Self {
            opts,
            window: None,
            renderer: None,
            adapter: String::from("unknown"),
            font: Font::new(),
            screen: Screen::new(80, 25),
            fb: Framebuffer::new(640, 400),
            vp: None,
            theme,
            crt,
            crt_base: crt,
            start: Instant::now(),
            last_frame: Instant::now(),
            next_frame: Instant::now(),
            last_input: Instant::now(),
            fps: 60.0,
            dirty: true,
            blink: true,
            mods: ModifiersState::empty(),
            typewriter: false,
            doc,
            has_document,
            mode: if has_document {
                Mode::Write
            } else {
                Mode::Action
            },
            palette: Palette::empty(),
            form: FormState::from_config(&sync_cfg),
            status: String::new(),
            screenshot_done: false,
            command: String::new(),
            command_error: String::new(),
            history: Vec::new(),
            history_pos: None,
            confirm: None,
            help_about: false,
            sync_label: initial_sync_label(&sync_cfg),
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
                    title_hint: Some("welcome.md"),
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
        let vp = Viewport::compute(w, h, mode);
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
        self.autosave_if_due();

        let Some(vp) = self.vp else {
            return;
        };
        if self.renderer.is_none() {
            return;
        }

        let time = self.start.elapsed().as_secs_f32();
        let blink = ((time * 1.6) as u32).is_multiple_of(2);
        if blink != self.blink {
            self.blink = blink;
            self.dirty = true;
        }

        let mut crt = self.crt;
        if self.theme.is_light() {
            crt.bloom = 0.0;
            crt.scanline *= 0.4;
            crt.flicker *= 0.4;
        }

        if self.dirty {
            if self.screen.cols() != vp.cols() || self.screen.rows() != vp.rows() {
                self.screen = Screen::new(vp.cols(), vp.rows());
            }
            if self.fb.width() != vp.virt_w || self.fb.height() != vp.virt_h {
                self.fb.resize(vp.virt_w, vp.virt_h);
            }
            self.compose();
            self.fb.draw_screen(&self.screen, &self.theme, &self.font);
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

        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;
        if dt > 0.0 && dt < 1.0 {
            self.fps = self.fps * 0.9 + (1.0 / dt) * 0.1;
        }

        let fps = if now.duration_since(self.last_input) < ACTIVE_WINDOW {
            ACTIVE_FPS
        } else {
            IDLE_FPS
        };
        self.next_frame = now + Duration::from_millis(1000 / fps as u64);
    }

    /// Build the cell grid for the current state.
    fn compose(&mut self) {
        let mut screen = std::mem::replace(&mut self.screen, Screen::new(1, 1));
        if self.mode == Mode::Help {
            ui::draw_help(&mut screen, &self.theme, self.help_about);
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
            if self.has_document {
                // Left empty on purpose: the prompt then shows how to leave.
                String::new()
            } else {
                "new  empty document     open  file list".to_string()
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

        if !self.has_document {
            ui::draw_welcome(&mut screen, &self.theme, &self.command, &hint, &error);
        } else {
            let (text_width, margin) = ui::layout(cols);
            let height = ui::text_height(rows);

            let anchor = self.typewriter.then_some(height / 2);
            self.doc.editor.scroll_to_cursor(text_width, height, anchor);

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
            let sync = if self.doc.path.is_none() {
                String::new()
            } else if self.doc.editor.modified() {
                "unsaved".to_string()
            } else {
                "saved".to_string()
            };
            let sync = if self.sync_cfg.enabled && !self.sync_label.is_empty() {
                format!("{sync} \u{00b7} {}", self.sync_label)
            } else {
                sync
            };

            let bottom = if self.mode == Mode::Action {
                ui::Bottom::Prompt {
                    input: &self.command,
                    hint: &hint,
                    error: &error,
                }
            } else {
                ui::Bottom::Status {
                    message: &self.status,
                }
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
                modified: self.doc.editor.modified(),
                lines: self.doc.editor.lines(),
                rows: &visible,
                cursor: cursor_cell,
                margin,
                stats,
                cursor_pos: (cursor.line, cursor.col),
                sync: &sync,
                bottom,
                theme_name: self.theme.name,
                in_fence,
                cursor_on: self.blink,
            };
            ui::draw_document(&mut screen, &self.theme, &view);
        }

        match self.mode {
            Mode::Palette => {
                let items = self.palette.items();
                let view = ui::PaletteView {
                    title: &self.palette.title,
                    input: &self.palette.input,
                    items: &items,
                    selected: self.palette.selected,
                    hint: "Enter select   Esc close",
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
                SyncEvent::Started => self.sync_label = "sync ...".into(),
                SyncEvent::Done(report) => {
                    self.sync_label = format!("sync {}", report_word(&report));
                    if report.conflicts > 0 {
                        self.status =
                            format!("sync: {} - conflict copy kept", report_word(&report));
                    }
                    if report.downloaded > 0 {
                        self.reload_after_sync();
                    }
                }
                SyncEvent::Failed(e) => {
                    self.sync_label = "sync failed".into();
                    self.status = format!("sync: {e}");
                }
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
        self.status = "updated from server".into();
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
                self.sync_label = initial_sync_label(&cfg);
                self.start_worker();
                if let Some(worker) = self.sync.as_ref() {
                    worker.configure(cfg);
                }
                self.to_write();
                self.status = if self.sync_cfg.enabled {
                    "sync on".into()
                } else {
                    "sync off".into()
                };
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
        let path = match self.doc.path.clone() {
            Some(path) => path,
            None => self.free_untitled_path(),
        };
        self.save_to(&path)?;
        Ok(if self.doc.path.as_deref() == Some(path.as_path()) {
            format!("saved {}", self.doc.title())
        } else {
            format!("saved as {}", path.display())
        })
    }

    /// Start a fresh file in the vault, named `untitled.md`, `untitled-2.md`, …
    fn new_document(&mut self) -> Result<String, String> {
        let path = self.free_untitled_path();
        self.doc = Document {
            path: None,
            title_hint: None,
            editor: Editor::new(),
            last_edit: Instant::now(),
        };
        self.has_document = true;
        self.save_to(&path)?;
        Ok(format!("created {}", path.display()))
    }

    fn save_as(&mut self, name: &str) -> Result<String, String> {
        let path = self.document_path(name);
        self.save_to(&path)?;
        Ok(format!("saved as {}", path.display()))
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
        let target = self.document_path(name);
        match self.doc.path.clone().filter(|p| p.is_file()) {
            Some(old) => {
                std::fs::rename(&old, &target).map_err(|e| format!("rename failed: {e}"))?;
                self.doc.path = Some(target.clone());
                self.dirty = true;
                Ok(format!("renamed to {}", target.display()))
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
        self.doc = Document {
            path: None,
            title_hint: None,
            editor: Editor::new(),
            last_edit: Instant::now(),
        };
        self.has_document = false;
        self.dirty = true;
        Ok(format!("deleted {}", path.display()))
    }

    // ── palette ─────────────────────────────────────────────────────────────

    fn open_palette(&mut self) {
        let dir = self.opts.vault.clone().unwrap_or_else(storage::vault_dir);
        let mut entries = vec![("+ new document".to_string(), Action::NewFile)];
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
            Err(e) => self.status = format!("cannot list {}: {e}", dir.display()),
        }

        self.palette = Palette {
            input: String::new(),
            entries,
            filtered: Vec::new(),
            selected: 0,
            title: "OPEN".into(),
        };
        self.palette.refilter();
        self.mode = Mode::Palette;
        self.dirty = true;
    }

    fn palette_accept(&mut self) {
        let Some(action) = self.palette.current().cloned() else {
            return;
        };
        match action {
            Action::Open(path) => match self.open_document(path.clone()) {
                Ok(()) => self.status = format!("opened {}", path.display()),
                Err(e) => self.status = e,
            },
            Action::NewFile => match self.new_document() {
                Ok(msg) => self.status = msg,
                Err(e) => self.status = e,
            },
        }
        self.to_write();
    }

    // ── input ───────────────────────────────────────────────────────────────

    fn on_key(&mut self, el: &ActiveEventLoop, event: KeyEvent) {
        if event.state != ElementState::Pressed {
            return;
        }
        self.last_input = Instant::now();
        match self.mode {
            Mode::Write => self.on_key_write(el, event),
            Mode::Action => self.on_key_action(el, event),
            Mode::Palette => self.on_key_palette(event),
            Mode::Settings => self.on_key_settings(event),
            Mode::Help => self.to_write(),
        }
    }

    /// Write mode: the document is live, editing keys only.
    fn on_key_write(&mut self, _el: &ActiveEventLoop, event: KeyEvent) {
        let ctrl = self.mods.control_key() || self.mods.super_key();
        let code = match event.physical_key {
            PhysicalKey::Code(code) => Some(code),
            PhysicalKey::Unidentified(_) => None,
        };

        if ctrl {
            // Undo is an editing gesture, not a command, so it keeps its key.
            match code {
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
                    self.doc.editor.move_word_left();
                    self.poke();
                }
                Some(KeyCode::ArrowRight) => {
                    self.doc.editor.move_word_right();
                    self.poke();
                }
                Some(KeyCode::Home) => {
                    self.doc.editor.move_doc_start();
                    self.poke();
                }
                Some(KeyCode::End) => {
                    self.doc.editor.move_doc_end();
                    self.poke();
                }
                _ => {}
            }
            return;
        }

        match &event.logical_key {
            Key::Named(NamedKey::Escape) => self.to_action(),
            Key::Named(NamedKey::ArrowLeft) => {
                self.doc.editor.move_left();
                self.doc.editor.break_undo_group();
                self.poke();
            }
            Key::Named(NamedKey::ArrowRight) => {
                self.doc.editor.move_right();
                self.doc.editor.break_undo_group();
                self.poke();
            }
            Key::Named(NamedKey::ArrowUp) => self.move_vertical(-1),
            Key::Named(NamedKey::ArrowDown) => self.move_vertical(1),
            Key::Named(NamedKey::Home) => {
                self.doc.editor.move_home();
                self.poke();
            }
            Key::Named(NamedKey::End) => {
                self.doc.editor.move_end();
                self.poke();
            }
            Key::Named(NamedKey::PageUp) => {
                let h = ui::text_height(self.screen.rows()) as isize;
                self.move_vertical(-h);
            }
            Key::Named(NamedKey::PageDown) => {
                let h = ui::text_height(self.screen.rows()) as isize;
                self.move_vertical(h);
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
                        self.status = msg;
                        self.to_write();
                    }
                    Err(e) => self.command_error = e,
                }
            } else {
                self.confirm = None;
                self.status = "cancelled".into();
                self.to_write();
            }
            self.poke();
            return;
        }

        match &event.logical_key {
            Key::Named(NamedKey::Escape) => {
                if self.command.is_empty() {
                    self.to_write();
                } else {
                    self.command.clear();
                    self.command_error.clear();
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
                self.history_step(-1);
                self.poke();
            }
            Key::Named(NamedKey::ArrowDown) => {
                self.history_step(1);
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

    fn submit_command(&mut self, el: &ActiveEventLoop) {
        let line = self.command.trim().to_string();
        let command = match command::parse(&line) {
            Ok(command) => command,
            Err(CommandError::Empty) => {
                self.to_write();
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
                    self.status = message;
                }
                self.command.clear();
                self.command_error.clear();
                // Commands that opened another screen keep it.
                if self.mode == Mode::Action && self.confirm.is_none() {
                    self.to_write();
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
            Command::Save => self.save(),
            Command::SaveAs(name) => self.save_as(&name),
            Command::Open(None) => {
                self.open_palette();
                Ok(String::new())
            }
            Command::Open(Some(name)) => {
                let path = self
                    .find_document(&name)
                    .ok_or_else(|| format!("no such document: {name}"))?;
                self.open_document(path)?;
                Ok(format!("opened {}", self.doc.title()))
            }
            Command::New => self.new_document(),
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
                self.dirty = true;
                Ok(format!("theme: {}", theme.name))
            }
            Command::Scanlines(want) => {
                let on = apply_switch(&mut self.crt.scanline, self.crt_base.scanline, want);
                self.dirty = true;
                Ok(format!("scanlines: {}", on_off(on)))
            }
            Command::Bloom(want) => {
                let on = apply_switch(&mut self.crt.bloom, self.crt_base.bloom, want);
                self.dirty = true;
                Ok(format!("bloom: {}", on_off(on)))
            }
            Command::Flicker(want) => {
                let on = apply_switch(&mut self.crt.flicker, self.crt_base.flicker, want);
                self.dirty = true;
                Ok(format!("flicker: {}", on_off(on)))
            }
            Command::Typewriter(want) => {
                self.typewriter = want.unwrap_or(!self.typewriter);
                self.dirty = true;
                Ok(format!("typewriter: {}", on_off(self.typewriter)))
            }
            Command::Fullscreen(on) => {
                self.set_fullscreen(on);
                Ok(if on { "fullscreen" } else { "windowed" }.to_string())
            }
            Command::Undo => {
                if self.doc.editor.undo() {
                    self.dirty = true;
                    Ok("undone".into())
                } else {
                    Err("nothing to undo".into())
                }
            }
            Command::Redo => {
                if self.doc.editor.redo() {
                    self.dirty = true;
                    Ok("redone".into())
                } else {
                    Err("nothing to redo".into())
                }
            }
            Command::Settings => {
                self.open_settings();
                Ok(String::new())
            }
            Command::Help => {
                self.help_about = false;
                self.mode = Mode::Help;
                self.dirty = true;
                Ok(String::new())
            }
            Command::About => {
                self.help_about = true;
                self.mode = Mode::Help;
                self.dirty = true;
                Ok(String::new())
            }
            Command::Quit => {
                if self.has_document {
                    self.save()?;
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

    /// Cheap "something changed, redraw soon" for non-document keys.
    fn poke(&mut self) {
        self.last_input = Instant::now();
        self.next_frame = self.last_input;
        self.dirty = true;
    }

    fn on_key_palette(&mut self, event: KeyEvent) {
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

    fn move_vertical(&mut self, delta: isize) {
        let (text_width, _) = ui::layout(self.screen.cols());
        for _ in 0..delta.unsigned_abs() {
            if delta < 0 {
                self.doc.editor.move_up(text_width);
            } else {
                self.doc.editor.move_down(text_width);
            }
        }
        self.doc.editor.break_undo_group();
        self.dirty = true;
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
    }
}

fn initial_sync_label(cfg: &SyncConfig) -> String {
    if !cfg.is_configured() {
        "webdav off".into()
    } else if cfg.enabled {
        "sync idle".into()
    } else {
        "sync off".into()
    }
}

fn report_word(report: &SyncReport) -> String {
    report.summary()
}
