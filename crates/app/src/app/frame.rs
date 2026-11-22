//! The frame loop, the viewport and the idle screensaver.
//!
//! There is no general frame clock: a frame is drawn when something is dirty and
//! `next_frame` is due, and the fastest running clock decides when that is.

use super::*;

impl App {
    pub(super) fn backends(&self) -> wgpu::Backends {
        match self.opts.backend.as_deref() {
            Some("vulkan") => wgpu::Backends::VULKAN,
            Some("gl") => wgpu::Backends::GL,
            _ => wgpu::Backends::all(),
        }
    }

    pub(super) fn recompute_viewport(&mut self) {
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

    pub(super) fn target_phys_size(&self, window: &Window) -> (u32, u32) {
        if self.opts.screenshot.is_some()
            && let Some(size) = self.opts.size
        {
            return size;
        }
        let size = window.inner_size();
        (size.width.max(1), size.height.max(1))
    }

    pub(super) fn frame(&mut self, el: &ActiveEventLoop) {
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

    #[allow(clippy::wrong_self_convention)]
    /// Has the machine been quiet long enough to rest?
    pub(super) fn screensaver_due(&self) -> bool {
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

    /// A key woke the clock. True when it did, so the key is swallowed instead
    /// of also typing or opening something.
    pub(super) fn wake_screensaver(&mut self) -> bool {
        if self.mode != Mode::Screensaver {
            return false;
        }
        self.mode = self.screensaver_back;
        self.last_input = Instant::now();
        self.dirty = true;
        true
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
}
