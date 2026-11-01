//! "Save and power off?": the one command that ends the machine, not just the
//! app. A kiosk needs this; `exit!` only ends the session, which systemd would
//! start again.

use super::*;

/// Which button is focused, and where to return if the user changes their mind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ShutdownPrompt {
    pub(super) yes: bool,
    pub(super) from: Mode,
}

impl App {
    pub(super) fn cmd_shutdown(&mut self) -> Result<String, String> {
        self.shutdown = Some(ShutdownPrompt {
            yes: false,
            from: self.mode,
        });
        self.mode = Mode::Shutdown;
        self.dirty = true;
        Ok(String::new())
    }

    pub(super) fn on_key_shutdown(&mut self, el: &ActiveEventLoop, event: KeyEvent) {
        let Some(prompt) = self.shutdown else {
            self.mode = Mode::Action;
            self.dirty = true;
            return;
        };
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => self.cancel_shutdown(),
            Key::Named(NamedKey::Enter) => self.confirm_shutdown(el, prompt.yes),
            Key::Named(NamedKey::ArrowLeft)
            | Key::Named(NamedKey::ArrowRight)
            | Key::Named(NamedKey::Tab) => {
                self.shutdown = Some(ShutdownPrompt {
                    yes: !prompt.yes,
                    from: prompt.from,
                });
                self.poke();
            }
            Key::Character(c) if c.eq_ignore_ascii_case("y") => self.confirm_shutdown(el, true),
            Key::Character(c) if c.eq_ignore_ascii_case("n") => self.cancel_shutdown(),
            _ => {}
        }
    }

    pub(super) fn cancel_shutdown(&mut self) {
        let Some(prompt) = self.shutdown.take() else {
            return;
        };
        self.mode = prompt.from;
        self.set_status("shutdown cancelled");
        self.poke();
    }

    fn confirm_shutdown(&mut self, el: &ActiveEventLoop, yes: bool) {
        let Some(prompt) = self.shutdown.take() else {
            return;
        };
        if !yes {
            self.mode = prompt.from;
            self.poke();
            return;
        }
        if self.has_document
            && let Err(e) = self.save()
        {
            self.mode = prompt.from;
            self.set_status(e);
            self.poke();
            return;
        }
        if self.program.is_some() && self.picture_path.is_some() {
            let _ = self.save_picture(None);
        }
        self.close_program();
        match power_off() {
            Ok(()) => el.exit(),
            Err(e) => {
                self.mode = prompt.from;
                self.set_status(format!("cannot power off: {e}"));
                self.poke();
            }
        }
    }
}

/// Ask the OS to go down. systemd is the norm; logind is the fallback for
/// systems without the systemctl wrapper.
fn power_off() -> Result<(), String> {
    let attempts: [(&str, &[&str]); 2] = [
        ("systemctl", &["--no-block", "poweroff"]),
        ("loginctl", &["poweroff"]),
    ];
    for (program, args) in attempts {
        match std::process::Command::new(program).args(args).status() {
            Ok(status) if status.success() => return Ok(()),
            Ok(status) => return Err(format!("{program} exited with {status}")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(format!("{program}: {e}")),
        }
    }
    Err("no power-off command found".into())
}
