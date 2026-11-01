//! Commands that belong to the machine: sync, the shelf, help, leaving.

use super::super::*;

impl App {
    pub(super) fn cmd_sync(&mut self) -> Result<String, String> {
        if !self.sync_cfg.enabled || self.sync.is_none() {
            return Err("webdav is off - try 'settings'".into());
        }
        self.request_sync();
        Ok("sync requested".into())
    }

    pub(super) fn cmd_software(&mut self) -> Result<String, String> {
        self.open_software();
        Ok(String::new())
    }

    pub(super) fn cmd_settings(&mut self) -> Result<String, String> {
        self.open_setup();
        Ok(String::new())
    }

    pub(super) fn cmd_help(&mut self, argument: Option<String>) -> Result<String, String> {
        self.help_about = false;
        self.help_all = argument
            .as_deref()
            .is_some_and(|a| a.eq_ignore_ascii_case("all"));
        self.help_selected = 0;
        self.mode = Mode::Help;
        self.dirty = true;
        Ok(String::new())
    }

    pub(super) fn cmd_about(&mut self) -> Result<String, String> {
        self.help_about = true;
        self.mode = Mode::Help;
        self.dirty = true;
        Ok(String::new())
    }

    pub(super) fn cmd_exit(&mut self, el: &ActiveEventLoop) -> Result<String, String> {
        if self.has_document && self.doc.editor.modified() {
            self.save()?;
        }
        // A program with its own levels closes one of them first.
        let mut closed = false;
        let mut wrote = false;
        if let Some(program) = self.program.as_mut() {
            closed = program.exit();
            if closed {
                wrote = program.wrote_files();
            }
        }
        if closed {
            if wrote {
                self.unsynced = true;
            }
            self.dirty = true;
            return Ok(String::new());
        }
        // On the boot screen `exit` means what it means everywhere else: stop
        // the program. That is CRead itself.
        if !self.has_document && self.program.is_none() && self.home == Home::Boot {
            if !self.ask_quit(Mode::Action) {
                el.exit();
            }
            return Ok(String::new());
        }
        self.leave_to_boot();
        Ok(String::new())
    }

    pub(super) fn cmd_exit_force(&mut self, el: &ActiveEventLoop) -> Result<String, String> {
        if self.has_document {
            self.save()?;
        }
        // A picture that was never saved keeps no file behind: quitting must
        // not silently create one.
        if self.program.is_some() && self.picture_path.is_some() {
            let _ = self.save_picture(None);
        }
        let from = self.mode;
        if !self.ask_quit(from) {
            el.exit();
        }
        Ok(String::new())
    }
}
