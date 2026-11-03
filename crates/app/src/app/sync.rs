//! The background sync worker and the question on the way out.
//!
//! The worker is a thread with its own runtime; the UI only drops requests in
//! and picks results up on the next frame. The one exception is quitting: the
//! process is about to end, so the last round runs on this thread after the
//! worker has been shut down.

use super::*;
use cread_sync::worker::Event as SyncEvent;

/// The exit dialog: which button is focused, and where to return if the user
/// changes their mind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct QuitPrompt {
    pub(super) yes: bool,
    pub(super) from: Mode,
}

impl App {
    pub(super) fn pump_sync(&mut self) {
        let Some(worker) = self.sync.as_ref() else {
            return;
        };
        let mut events = Vec::new();
        while let Some(event) = worker.try_recv() {
            events.push(event);
        }
        for event in events {
            match event {
                SyncEvent::Started => self.set_status("sync ..."),
                SyncEvent::Done(report) => {
                    self.unsynced = false;
                    if report.conflicts > 0 {
                        self.set_status(format!(
                            "sync: {} - conflict copy kept",
                            report_word(&report)
                        ));
                    } else {
                        self.set_status(format!("sync: {}", report_word(&report)));
                    }
                    if report.downloaded > 0 || report.conflicts > 0 {
                        self.reload_after_sync();
                        self.reload_calendar();
                    }
                }
                SyncEvent::Failed(e) => self.set_status(format!("sync failed: {e}")),
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
        if self.mode == Mode::View {
            // Reading mode caches its rendered rows by width; zero forces the
            // next frame to rebuild them from the new text.
            self.view_width = 0;
        }
        self.set_status("updated from server");
        self.dirty = true;
    }

    pub(super) fn start_worker(&mut self) {
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

    pub(super) fn request_sync(&mut self) {
        if let Some(worker) = self.sync.as_ref() {
            worker.request_sync();
        }
    }

    /// Changes on disk the server has not seen: quitting should ask.
    pub(super) fn wants_sync_before_quitting(&self) -> bool {
        self.unsynced
            && self.sync_cfg.enabled
            && self.sync_cfg.is_configured()
            && self.sync.is_some()
    }

    /// Show the exit dialog if there is anything to sync. False means "quit
    /// now, nothing to ask".
    pub(super) fn ask_quit(&mut self, from: Mode) -> bool {
        if !self.wants_sync_before_quitting() {
            return false;
        }
        self.quit = Some(QuitPrompt { yes: true, from });
        self.mode = Mode::Quit;
        self.dirty = true;
        true
    }

    pub(super) fn on_key_quit(&mut self, el: &ActiveEventLoop, event: KeyEvent) {
        let Some(prompt) = self.quit else {
            self.mode = Mode::Action;
            self.dirty = true;
            self.poke();
            return;
        };
        match &event.logical_key {
            // "Esc no" on the box means no sync and quit, not cancel. Leaving
            // the prompt open with Esc would contradict the hint and the manual.
            Key::Named(NamedKey::Escape) => self.confirm_quit(el, false),
            Key::Named(NamedKey::Enter) => self.confirm_quit(el, prompt.yes),
            Key::Named(NamedKey::ArrowLeft)
            | Key::Named(NamedKey::ArrowRight)
            | Key::Named(NamedKey::Tab) => {
                self.quit = Some(QuitPrompt {
                    yes: !prompt.yes,
                    from: prompt.from,
                });
                self.poke();
            }
            Key::Character(c) if c.eq_ignore_ascii_case("y") => self.confirm_quit(el, true),
            Key::Character(c) if c.eq_ignore_ascii_case("n") => self.confirm_quit(el, false),
            _ => {}
        }
    }

    pub(super) fn confirm_quit(&mut self, el: &ActiveEventLoop, sync: bool) {
        let Some(prompt) = self.quit.take() else {
            return;
        };
        self.mode = prompt.from;
        if !sync {
            el.exit();
            return;
        }
        // A cartridge writes its battery when the program is dropped; do that
        // before the last round, or the file would appear after it.
        self.close_program();
        match self.sync_before_quit() {
            Ok(()) => el.exit(),
            Err(_) => self.dirty = true,
        }
    }

    /// One last round on the way out. The process is about to end, so the
    /// worker cannot do this one: stop it first (its Drop joins the thread and
    /// waits for a round in flight), then run a single round here.
    pub(super) fn sync_before_quit(&mut self) -> Result<(), String> {
        self.pause_sync();
        let vault = self.vault();
        self.set_status("sync ...");
        match cread_sync::run_blocking(&vault, &self.sync_cfg) {
            Ok(report) => {
                self.unsynced = false;
                self.set_status(format!("sync: {}", report.summary()));
                Ok(())
            }
            Err(e) => {
                self.set_status(format!("sync failed: {e}"));
                self.start_worker();
                Err(e)
            }
        }
    }

    fn pause_sync(&mut self) {
        // Dropping the worker shuts the thread down and waits for a round in
        // flight, so two syncs never run at once.
        self.sync = None;
    }
}
