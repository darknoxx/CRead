//! Window lifetime, frame loop and input.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use cread_core::{
    resolution::target_line_px_for_scale_factor, storage, Editor, ScaleMode, Screen, Stats, Theme,
    Viewport,
};
use cread_render::{raster, CrtSettings, Font, FrameStatus, Framebuffer, Renderer};
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
    event_loop.run_app(&mut app).map_err(|e| format!("event loop: {e}"))
}

struct Document {
    path: Option<PathBuf>,
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
            None => "welcome.md".into(),
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

#[derive(PartialEq, Eq)]
enum Mode {
    Editor,
    Palette,
    Settings,
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
    mode: Mode,
    palette: Palette,
    form: FormState,
    status: String,
    screenshot_done: bool,

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
            ui::FormField { label: "server".into(), value: self.url.clone(), secret: false },
            ui::FormField { label: "user".into(), value: self.username.clone(), secret: false },
            ui::FormField { label: "password".into(), value: self.password.clone(), secret: true },
            ui::FormField { label: "folder".into(), value: self.remote_dir.clone(), secret: false },
            ui::FormField {
                label: "sync".into(),
                value: if self.enabled { "on".into() } else { "off".into() },
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
        let doc = Self::initial_document(&opts);
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
            mode: Mode::Editor,
            palette: Palette::empty(),
            form: FormState::from_config(&sync_cfg),
            status: String::new(),
            screenshot_done: false,
            sync_label: initial_sync_label(&sync_cfg),
            sync_cfg,
            sync: None,
        }
    }

    fn initial_document(opts: &Options) -> Document {
        if let Some(path) = &opts.file {
            match storage::load(path) {
                Ok(text) => {
                    return Document {
                        path: Some(path.clone()),
                        editor: Editor::from_text(&text),
                        last_edit: Instant::now(),
                    };
                }
                Err(e) => eprintln!("cread: cannot open {}: {e}", path.display()),
            }
        }

        if !opts.demo
            && let Some(path) = last_opened_file()
                && let Ok(text) = storage::load(&path) {
                    return Document {
                        path: Some(path),
                        editor: Editor::from_text(&text),
                        last_edit: Instant::now(),
                    };
                }

        Document {
            path: None,
            editor: Editor::from_text(WELCOME),
            last_edit: Instant::now(),
        }
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
            && let Some(size) = self.opts.size {
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
        let cols = self.screen.cols();
        let rows = self.screen.rows();
        let (text_width, margin) = ui::layout(cols);
        let height = ui::text_height(rows);

        let anchor = self.typewriter.then_some(height / 2);
        self.doc
            .editor
            .scroll_to_cursor(text_width, height, anchor);

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

        let mut screen = std::mem::replace(&mut self.screen, Screen::new(1, 1));

        // The document is always there; overlays are painted on top of it, like
        // a TSR popping up over a DOS application.
        {
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
                status: &self.status,
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
            Mode::Editor => {}
        }

        self.screen = screen;
    }

    // ── persistence ─────────────────────────────────────────────────────────

    fn autosave_if_due(&mut self) {
        if self.mode != Mode::Editor {
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
        if self.save_to(&path) {
            self.status = "autosaved".into();
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
                        self.status = format!("sync: {} - conflict copy kept", report_word(&report));
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
                self.mode = Mode::Editor;
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

    fn save_to(&mut self, path: &Path) -> bool {
        let text = self.doc.editor.text();
        match storage::save_atomic(path, &text) {
            Ok(()) => {
                self.doc.editor.mark_saved();
                self.doc.path = Some(path.to_path_buf());
                remember_last_file(path);
                if self.sync_cfg.enabled {
                    self.request_sync();
                }
                self.dirty = true;
                true
            }
            Err(e) => {
                self.status = format!("save failed: {e}");
                self.dirty = true;
                false
            }
        }
    }

    fn save(&mut self) {
        if let Some(path) = self.doc.path.clone() {
            if self.save_to(&path) {
                self.status = format!("saved {}", self.doc.title());
            }
            return;
        }

        // First save of the welcome buffer: pick a free name in the vault.
        let dir = self.opts.vault.clone().unwrap_or_else(storage::vault_dir);
        let mut path = dir.join("untitled.md");
        let mut n = 2;
        while path.exists() {
            path = dir.join(format!("untitled-{n}.md"));
            n += 1;
        }
        if self.save_to(&path) {
            self.status = format!("saved as {}", path.display());
        }
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
            Action::Open(path) => {
                match storage::load(&path) {
                    Ok(text) => {
                        self.doc = Document {
                            path: Some(path.clone()),
                            editor: Editor::from_text(&text),
                            last_edit: Instant::now(),
                        };
                        remember_last_file(&path);
                        self.status = format!("opened {}", path.display());
                    }
                    Err(e) => self.status = format!("cannot open: {e}"),
                }
            }
            Action::NewFile => {
                let dir = self.opts.vault.clone().unwrap_or_else(storage::vault_dir);
                let mut path = dir.join("untitled.md");
                let mut n = 2;
                while path.exists() {
                    path = dir.join(format!("untitled-{n}.md"));
                    n += 1;
                }
                self.doc = Document {
                    path: None,
                    editor: Editor::new(),
                    last_edit: Instant::now(),
                };
                if self.save_to(&path) {
                    self.status = format!("created {}", path.display());
                }
            }
        }
        self.mode = Mode::Editor;
        self.dirty = true;
    }

    // ── input ───────────────────────────────────────────────────────────────

    fn on_key(&mut self, el: &ActiveEventLoop, event: KeyEvent) {
        if event.state != ElementState::Pressed {
            return;
        }
        if self.mode == Mode::Palette {
            self.on_key_palette(event);
            return;
        }
        if self.mode == Mode::Settings {
            self.on_key_settings(event);
            return;
        }

        let ctrl = self.mods.control_key() || self.mods.super_key();

        match &event.logical_key {
            Key::Named(NamedKey::Escape) => {
                // One stray Escape should not end a writing session.
                if self.doc.editor.modified() {
                    self.status = "unsaved - Ctrl+Q to quit anyway".into();
                    self.dirty = true;
                } else {
                    el.exit();
                }
            }
            Key::Named(NamedKey::F1) => {
                let idx = Theme::ALL.iter().position(|t| *t == self.theme).unwrap_or(0);
                self.theme = Theme::ALL[(idx + 1) % Theme::ALL.len()];
                self.status = format!("theme: {}", self.theme.name);
                self.dirty = true;
            }
            Key::Named(NamedKey::F2) => self.toggle_effect(Effect::Scanline),
            Key::Named(NamedKey::F3) => self.toggle_effect(Effect::Bloom),
            Key::Named(NamedKey::F4) => self.toggle_effect(Effect::Flicker),
            Key::Named(NamedKey::F8) => {
                self.typewriter = !self.typewriter;
                self.status = format!(
                    "typewriter scrolling: {}",
                    if self.typewriter { "on" } else { "off" }
                );
                self.dirty = true;
            }
            Key::Named(NamedKey::F9) => self.open_settings(),
            Key::Named(NamedKey::F11) => self.toggle_fullscreen(),
            _ if ctrl => self.on_key_ctrl(el, &event),
            Key::Named(NamedKey::ArrowLeft) => {
                self.doc.editor.move_left();
                self.doc.editor.break_undo_group();
                self.dirty = true;
            }
            Key::Named(NamedKey::ArrowRight) => {
                self.doc.editor.move_right();
                self.doc.editor.break_undo_group();
                self.dirty = true;
            }
            Key::Named(NamedKey::ArrowUp) => {
                self.move_vertical(-1);
            }
            Key::Named(NamedKey::ArrowDown) => {
                self.move_vertical(1);
            }
            Key::Named(NamedKey::Home) => {
                self.doc.editor.move_home();
                self.dirty = true;
            }
            Key::Named(NamedKey::End) => {
                self.doc.editor.move_end();
                self.dirty = true;
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

    fn on_key_ctrl(&mut self, el: &ActiveEventLoop, event: &KeyEvent) {
        // Shortcuts are matched on the physical key: with Ctrl held, some
        // backends report `logical_key` as a control character, not "s".
        let shift = self.mods.shift_key();
        let code = match event.physical_key {
            PhysicalKey::Code(code) => code,
            PhysicalKey::Unidentified(_) => return,
        };
        match code {
            KeyCode::KeyS => self.save(),
            KeyCode::KeyP => self.open_palette(),
            KeyCode::KeyQ => el.exit(),
            KeyCode::KeyZ => {
                if shift {
                    self.doc.editor.redo();
                } else {
                    self.doc.editor.undo();
                }
                self.dirty = true;
            }
            KeyCode::KeyY => {
                self.doc.editor.redo();
                self.dirty = true;
            }
            KeyCode::ArrowLeft => {
                self.doc.editor.move_word_left();
                self.dirty = true;
            }
            KeyCode::ArrowRight => {
                self.doc.editor.move_word_right();
                self.dirty = true;
            }
            KeyCode::Home => {
                self.doc.editor.move_doc_start();
                self.dirty = true;
            }
            KeyCode::End => {
                self.doc.editor.move_doc_end();
                self.dirty = true;
            }
            _ => {}
        }
    }

    fn on_key_palette(&mut self, event: KeyEvent) {
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => {
                self.mode = Mode::Editor;
                self.dirty = true;
            }
            Key::Named(NamedKey::Enter) => self.palette_accept(),
            Key::Named(NamedKey::ArrowUp) => {
                self.palette.selected = self.palette.selected.saturating_sub(1);
                self.dirty = true;
            }
            Key::Named(NamedKey::ArrowDown) => {
                if self.palette.selected + 1 < self.palette.filtered.len() {
                    self.palette.selected += 1;
                }
                self.dirty = true;
            }
            Key::Named(NamedKey::Backspace) => {
                self.palette.input.pop();
                self.palette.refilter();
                self.dirty = true;
            }
            Key::Named(NamedKey::Space) => {
                self.palette.input.push(' ');
                self.palette.refilter();
                self.dirty = true;
            }
            Key::Character(text) => {
                for ch in text.chars().filter(|c| !c.is_control()) {
                    self.palette.input.push(ch);
                }
                self.palette.refilter();
                self.dirty = true;
            }
            _ => {}
        }
    }

    fn on_key_settings(&mut self, event: KeyEvent) {
        let ctrl = self.mods.control_key() || self.mods.super_key();
        match &event.logical_key {
            _ if ctrl && matches!(&event.logical_key, Key::Character(c) if c.eq_ignore_ascii_case("s")) => {
                self.save_settings()
            }
            Key::Named(NamedKey::Escape) => {
                self.mode = Mode::Editor;
                self.dirty = true;
            }
            Key::Named(NamedKey::Enter) => {
                if self.form.focus == 4 {
                    self.form.enabled = !self.form.enabled;
                    self.dirty = true;
                } else {
                    self.save_settings();
                }
            }
            Key::Named(NamedKey::Tab) | Key::Named(NamedKey::ArrowDown) => {
                self.form.focus = (self.form.focus + 1) % FormState::field_count();
                self.dirty = true;
            }
            Key::Named(NamedKey::ArrowUp) => {
                self.form.focus =
                    (self.form.focus + FormState::field_count() - 1) % FormState::field_count();
                self.dirty = true;
            }
            Key::Named(NamedKey::Backspace) => {
                if let Some(field) = self.form.text_field_mut() {
                    field.pop();
                }
                self.dirty = true;
            }
            Key::Named(NamedKey::Space) => {
                if let Some(field) = self.form.text_field_mut() {
                    field.push(' ');
                }
                self.dirty = true;
            }
            Key::Character(text) => {
                if let Some(field) = self.form.text_field_mut() {
                    for ch in text.chars().filter(|c| !c.is_control()) {
                        field.push(ch);
                    }
                }
                self.dirty = true;
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
        let now = Instant::now();
        self.doc.last_edit = now;
        self.last_input = now;
        // Do not wait for the frame budget after a keystroke.
        self.next_frame = now;
        self.dirty = true;
    }

    fn toggle_fullscreen(&mut self) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        let go_fullscreen = window.fullscreen().is_none();
        window.set_fullscreen(go_fullscreen.then_some(Fullscreen::Borderless(None)));
        self.status = if go_fullscreen {
            "fullscreen".into()
        } else {
            "windowed".into()
        };
        self.dirty = true;
    }

    fn toggle_effect(&mut self, effect: Effect) {
        let (slot, base, name) = match effect {
            Effect::Scanline => (&mut self.crt.scanline, self.crt_base.scanline, "scanlines"),
            Effect::Bloom => (&mut self.crt.bloom, self.crt_base.bloom, "bloom"),
            Effect::Flicker => (&mut self.crt.flicker, self.crt_base.flicker, "flicker"),
        };
        *slot = if *slot > 0.0 { 0.0 } else { base.max(0.05) };
        self.status = format!("{name}: {}", if *slot > 0.0 { "on" } else { "off" });
        self.dirty = true;
    }
}

#[derive(Clone, Copy)]
enum Effect {
    Scanline,
    Bloom,
    Flicker,
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
            self.save();
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

fn remember_last_file(path: &Path) {
    let dir = storage::config_dir();
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(dir.join("last.txt"), path.to_string_lossy().as_bytes());
}

fn last_opened_file() -> Option<PathBuf> {
    let content = std::fs::read_to_string(storage::config_dir().join("last.txt")).ok()?;
    let path = PathBuf::from(content.trim());
    path.is_file().then_some(path)
}
