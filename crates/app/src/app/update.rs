//! Checking for updates, installing them, and restarting into the new binary.
//!
//! The network work happens on the update worker; this module only starts it,
//! draws the progress window while it runs, and acts on the result.

use std::ffi::OsString;

use super::*;

impl App {
    /// SETTINGS -> UPDATE, and the `update` command: ask GitHub for the newest
    /// release. The window stays up until the worker answers.
    pub(super) fn start_update_check(&mut self) {
        let Some(target) = self.self_exe.clone() else {
            self.set_status("cannot tell which binary is running");
            return;
        };
        if self.updater.is_none() {
            self.updater = Some(cread_update::Worker::spawn(
                ui::VERSION.to_string(),
                target,
                self.update_api.clone(),
            ));
        }
        self.update_installing = false;
        self.update_started = Instant::now();
        self.mode = Mode::Checking;
        self.dirty = true;
        if let Some(worker) = self.updater.as_ref() {
            worker.check();
        }
    }

    /// "Install and restart?" -> Yes. The worker downloads, verifies and
    /// replaces the binary; `pump_update` takes over from there.
    fn start_update_install(&mut self) {
        let Some(offer) = self.update_offer.clone() else {
            return;
        };
        self.update_installing = true;
        self.update_started = Instant::now();
        self.mode = Mode::Checking;
        self.dirty = true;
        if let Some(worker) = self.updater.as_ref() {
            worker.install(offer);
        }
    }

    /// Collect what the worker said, once per frame.
    pub(super) fn pump_update(&mut self, el: &ActiveEventLoop) {
        let Some(worker) = self.updater.as_ref() else {
            return;
        };
        let mut events = Vec::new();
        while let Some(event) = worker.try_recv() {
            events.push(event);
        }
        // A cancelled check keeps its late answer to itself.
        if self.mode != Mode::Checking {
            return;
        }
        for event in events {
            match event {
                UpdateEvent::UpToDate(version) => {
                    self.update_status = UpdateStatus::UpToDate;
                    self.set_status(format!("{version} is the newest"));
                    self.mode = Mode::Setup;
                }
                UpdateEvent::Available(offer) => {
                    self.update_status = UpdateStatus::Available(offer.version.clone());
                    self.update_offer = Some(offer);
                    self.update_yes = true;
                    self.mode = Mode::UpdatePrompt;
                }
                UpdateEvent::Installed(version) => self.restart_after_update(el, &version),
                UpdateEvent::Failed(error) => {
                    self.update_installing = false;
                    self.set_status(format!("update failed: {error}"));
                    self.mode = Mode::Setup;
                }
            }
            self.dirty = true;
        }
    }

    /// While the window is up, Esc cancels a check. An install is not
    /// cancellable: the binary may be half replaced.
    pub(super) fn on_key_checking(&mut self, event: KeyEvent) {
        if self.update_installing {
            return;
        }
        if matches!(&event.logical_key, Key::Named(NamedKey::Escape)) {
            self.mode = Mode::Setup;
            self.set_status("update check cancelled");
            self.poke();
        }
    }

    pub(super) fn on_key_update_prompt(&mut self, event: KeyEvent) {
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => {
                self.mode = Mode::Setup;
                self.set_status("update postponed");
                self.poke();
            }
            Key::Named(NamedKey::Enter) => {
                if self.update_yes {
                    self.start_update_install();
                } else {
                    self.mode = Mode::Setup;
                    self.set_status("update postponed");
                    self.poke();
                }
            }
            Key::Named(NamedKey::ArrowLeft)
            | Key::Named(NamedKey::ArrowRight)
            | Key::Named(NamedKey::Tab) => {
                self.update_yes = !self.update_yes;
                self.poke();
            }
            Key::Character(c) if c.eq_ignore_ascii_case("y") => self.start_update_install(),
            Key::Character(c) if c.eq_ignore_ascii_case("n") => {
                self.mode = Mode::Setup;
                self.set_status("update postponed");
                self.poke();
            }
            _ => {}
        }
    }

    /// The new binary is in place: save like `exit!` would, then start it with
    /// the arguments this session was given.
    fn restart_after_update(&mut self, el: &ActiveEventLoop, version: &str) {
        if self.has_document
            && self.doc.editor.modified()
            && let Err(e) = self.save()
        {
            self.set_status(format!("updated to {version}, but saving failed: {e}"));
            self.mode = Mode::Setup;
            return;
        }
        // A painting that was never saved keeps no file behind; one that has a
        // path is the writer's and must survive the restart.
        if self.program.is_some() && self.picture_path.is_some() {
            let _ = self.save_picture(None);
        }
        self.close_program();

        // A kiosk is a whole session, not just this process: cage ends with
        // us, so spawning a new CRead would leave it without a compositor.
        // Exiting lets systemd start cage and the new binary together.
        if self.opts.kiosk {
            self.set_status(format!("updated to {version} - restarting"));
            el.exit();
            return;
        }

        let Some(exe) = self.restart_target() else {
            self.set_status(format!("updated to {version}, but cannot find the binary"));
            self.mode = Mode::Setup;
            return;
        };
        let args: Vec<OsString> = self.argv.iter().skip(1).cloned().collect();
        match cread_update::restart_command(&exe, &args).spawn() {
            Ok(_) => {
                self.set_status(format!("updated to {version} - restarting"));
                el.exit();
            }
            Err(e) => {
                self.set_status(format!("updated to {version}, restart failed: {e}"));
                self.mode = Mode::Setup;
            }
        }
    }
}
