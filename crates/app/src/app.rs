//! Window lifetime, frame loop and input.

use std::sync::Arc;
use std::time::Instant;

use cread_core::{resolution::target_line_px_for_scale_factor, ScaleMode, Screen, Theme, Viewport};
use cread_render::{raster, CrtSettings, Font, FrameStatus, Framebuffer, Renderer};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Fullscreen, Window, WindowId};

use crate::cli::Options;
use crate::ui;

pub fn run(opts: Options) -> Result<(), String> {
    let event_loop = EventLoop::new().map_err(|e| format!("event loop: {e}"))?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = App::new(opts);
    event_loop.run_app(&mut app).map_err(|e| format!("event loop: {e}"))
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
    /// Effect settings to restore when a toggle flips something back on.
    crt_base: CrtSettings,

    start: Instant,
    last_frame: Instant,
    fps: f32,
    dirty: bool,
    blink: bool,

    line: String,
    status: String,
    screenshot_done: bool,
}

impl App {
    fn new(opts: Options) -> Self {
        let crt = opts.crt;
        let theme = opts.theme;
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
            fps: 60.0,
            dirty: true,
            blink: true,
            line: String::new(),
            status: String::from("ready"),
            screenshot_done: false,
        }
    }

    fn backends(&self) -> wgpu::Backends {
        match self.opts.backend.as_deref() {
            Some("vulkan") => wgpu::Backends::VULKAN,
            Some("gl") => wgpu::Backends::GL,
            _ => wgpu::Backends::all(),
        }
    }

    /// Recompute the emulated raster for the current surface size.
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

    /// Physical size we render into: the window, unless a screenshot run was
    /// told to emulate a specific display.
    fn target_phys_size(&self, window: &Window) -> (u32, u32) {
        if self.opts.screenshot.is_some() {
            if let Some(size) = self.opts.size {
                return size;
            }
        }
        let size = window.inner_size();
        (size.width.max(1), size.height.max(1))
    }

    fn frame(&mut self, el: &ActiveEventLoop) {
        let Some(vp) = self.vp else {
            return;
        };
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };

        let time = self.start.elapsed().as_secs_f32();

        // Paper-like schemes are not emissive; printing a glow onto them just
        // makes the text mushy.
        let mut crt = self.crt;
        if self.theme.is_light() {
            crt.bloom = 0.0;
            crt.scanline *= 0.4;
            crt.flicker *= 0.4;
        }

        let blink = (time * 1.6) as u32 % 2 == 0;
        if blink != self.blink {
            self.blink = blink;
            self.dirty = true;
        }

        if self.dirty {
            if self.screen.cols() != vp.cols() || self.screen.rows() != vp.rows() {
                self.screen = Screen::new(vp.cols(), vp.rows());
            }
            if self.fb.width() != vp.virt_w || self.fb.height() != vp.virt_h {
                self.fb.resize(vp.virt_w, vp.virt_h);
            }

            let hud = ui::Hud {
                adapter: &self.adapter,
                vp: &vp,
                fps: self.fps,
                status: &self.status,
                line: &self.line,
                cursor_on: self.blink,
            };
            ui::draw(&mut self.screen, &self.theme, &hud);
            self.fb.draw_screen(&self.screen, &self.theme, &self.font);
            renderer.upload(&self.fb);
            self.dirty = false;
        }

        // Headless verification path: no window frame needed.
        if let Some(path) = self.opts.screenshot.clone() {
            if !self.screenshot_done {
                // Render at the full physical size so the letterbox border is
                // visible in the capture, exactly as on screen.
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
    }

    fn on_key(&mut self, el: &ActiveEventLoop, event: KeyEvent) {
        if event.state != ElementState::Pressed {
            return;
        }

        match &event.logical_key {
            Key::Named(NamedKey::Escape) => el.exit(),
            Key::Named(NamedKey::F11) => {
                if let Some(window) = self.window.as_ref() {
                    let go_fullscreen = window.fullscreen().is_none();
                    window.set_fullscreen(
                        go_fullscreen.then_some(Fullscreen::Borderless(None)),
                    );
                    self.status = if go_fullscreen {
                        "fullscreen".into()
                    } else {
                        "windowed".into()
                    };
                    self.dirty = true;
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
            Key::Named(NamedKey::Backspace) => {
                self.line.pop();
                self.dirty = true;
            }
            Key::Named(NamedKey::Space) => {
                self.line.push(' ');
                self.dirty = true;
            }
            Key::Named(NamedKey::Enter) => {
                if !self.line.is_empty() {
                    self.status = format!("echo: {}", self.line);
                    self.line.clear();
                    self.dirty = true;
                }
            }
            Key::Character(text) => {
                for ch in text.chars().filter(|c| !c.is_control()) {
                    self.line.push(ch);
                }
                self.dirty = true;
            }
            _ => {}
        }
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
        // In screenshot mode the window is only scaffolding for the wgpu
        // adapter; the emulated display size comes from `--size`.
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

        // The first surface configure happened with the pre-fullscreen size;
        // re-apply the real one before the first frame.
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
    }

    fn window_event(
        &mut self,
        el: &ActiveEventLoop,
        _id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => el.exit(),
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

    fn about_to_wait(&mut self, _el: &ActiveEventLoop) {
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }
}
