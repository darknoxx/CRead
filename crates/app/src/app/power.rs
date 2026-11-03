//! "Save and power off?", "Save and restart?", "Leave for a Linux console?":
//! the three commands that end the session rather than just the app. A kiosk
//! needs them; `exit!` only leaves, and systemd starts the kiosk again.

use super::*;

/// What the prompt will do after it has saved the work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PowerAction {
    Shutdown,
    Reboot,
    Leave,
}

impl PowerAction {
    pub(super) fn title(self) -> &'static str {
        match self {
            Self::Shutdown => "POWER OFF",
            Self::Reboot => "REBOOT",
            Self::Leave => "LEAVE CREAD",
        }
    }

    pub(super) fn question(self) -> &'static str {
        match self {
            Self::Shutdown => "Save and shut down the machine?",
            Self::Reboot => "Save and restart the machine?",
            Self::Leave => "Save and open a Linux console?",
        }
    }

    fn system_verb(self) -> &'static str {
        match self {
            Self::Shutdown => "poweroff",
            Self::Reboot => "reboot",
            Self::Leave => "start",
        }
    }
}

/// Which button is focused, and where to return if the user changes their mind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PowerPrompt {
    pub(super) action: PowerAction,
    pub(super) yes: bool,
    pub(super) from: Mode,
}

impl App {
    fn open_power(&mut self, action: PowerAction) {
        self.power = Some(PowerPrompt {
            action,
            yes: false,
            from: self.mode,
        });
        self.mode = Mode::Power;
        self.dirty = true;
    }

    pub(super) fn cmd_shutdown(&mut self) -> Result<String, String> {
        self.open_power(PowerAction::Shutdown);
        Ok(String::new())
    }

    pub(super) fn cmd_reboot(&mut self) -> Result<String, String> {
        self.open_power(PowerAction::Reboot);
        Ok(String::new())
    }

    /// Stop the kiosk and open a login on tty1. The installed
    /// `cread-backtolinux.service` does the privileged part.
    pub(super) fn cmd_backtolinux(&mut self) -> Result<String, String> {
        self.open_power(PowerAction::Leave);
        Ok(String::new())
    }

    pub(super) fn on_key_power(&mut self, el: &ActiveEventLoop, event: KeyEvent) {
        let Some(prompt) = self.power else {
            self.mode = Mode::Action;
            self.dirty = true;
            return;
        };
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => self.cancel_power(),
            Key::Named(NamedKey::Enter) => self.confirm_power(el, prompt.yes),
            Key::Named(NamedKey::ArrowLeft)
            | Key::Named(NamedKey::ArrowRight)
            | Key::Named(NamedKey::Tab) => {
                self.power = Some(PowerPrompt {
                    yes: !prompt.yes,
                    ..prompt
                });
                self.poke();
            }
            Key::Character(c) if c.eq_ignore_ascii_case("y") => self.confirm_power(el, true),
            Key::Character(c) if c.eq_ignore_ascii_case("n") => self.cancel_power(),
            _ => {}
        }
    }

    pub(super) fn cancel_power(&mut self) {
        let Some(prompt) = self.power.take() else {
            return;
        };
        self.mode = prompt.from;
        self.set_status("cancelled");
        self.poke();
    }

    fn confirm_power(&mut self, el: &ActiveEventLoop, yes: bool) {
        let Some(prompt) = self.power.take() else {
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
        if self
            .program
            .as_ref()
            .is_some_and(|program| program.modified())
            && self.picture_path.is_some()
            && let Err(e) = self.save_picture(None)
        {
            self.mode = prompt.from;
            self.set_status(e);
            self.poke();
            return;
        }
        self.close_program();

        let result = match prompt.action {
            PowerAction::Leave => leave_to_linux(),
            PowerAction::Shutdown | PowerAction::Reboot => power_off(prompt.action),
        };
        match result {
            Ok(()) => el.exit(),
            Err(e) => {
                self.mode = prompt.from;
                self.set_status(format!("{} failed: {e}", prompt.action.system_verb()));
                self.poke();
            }
        }
    }
}

/// Ask the OS to go down. systemd is the norm; logind is the fallback for
/// systems without the systemctl wrapper.
fn power_off(action: PowerAction) -> Result<(), String> {
    let verb = action.system_verb();
    let attempts: [(&str, &[&str]); 2] =
        [("systemctl", &["--no-block", verb]), ("loginctl", &[verb])];
    for (program, args) in attempts {
        match std::process::Command::new(program).args(args).status() {
            Ok(status) if status.success() => return Ok(()),
            Ok(status) => return Err(format!("{program} exited with {status}")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(format!("{program}: {e}")),
        }
    }
    Err(format!("no {verb} command found"))
}

/// Queue the helper that stops the kiosk and starts a getty. `--no-block` is
/// important: the calling process is inside the service the helper stops.
fn leave_to_linux() -> Result<(), String> {
    let status = std::process::Command::new("systemctl")
        .args(["--no-block", "start", "cread-backtolinux.service"])
        .status()
        .map_err(|e| format!("systemctl: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("systemctl exited with {status}"))
    }
}
