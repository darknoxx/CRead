//! The SOFTWARE shelf and the SETTINGS screen.
//!
//! Installing only toggles which compiled-in module the boot screen shows; the
//! choice lives in the preferences.

use super::*;

impl App {
    pub(super) fn is_installed(&self, id: &str) -> bool {
        self.prefs.installed.iter().any(|have| have == id)
    }

    /// One row per module for the SOFTWARE screen.
    pub(super) fn software_rows(&self) -> Vec<ui::MenuItem> {
        let mut rows = vec![ui::MenuItem::row(
            "INSPIRO",
            "where your ideas have room to grow.",
            "system",
        )];
        for program in PROGRAMS {
            rows.push(ui::MenuItem::row(
                program.entry,
                program.tagline,
                if self.is_installed(program.id) {
                    "installed"
                } else {
                    "install"
                },
            ));
        }
        rows
    }

    /// The system settings, in the order they belong in: sync first, then the
    /// look, then the clock, then the machine's own business. Rules split the
    /// groups; they are never selectable.
    pub(super) fn setup_entries(&self) -> Vec<(ui::MenuItem, Option<SetupAction>)> {
        use SetupAction as Action;
        let sync = if !self.sync_cfg.enabled {
            "off"
        } else if self.sync_cfg.url.is_empty() {
            "no server"
        } else {
            "on"
        };
        let fullscreen = self
            .window
            .as_ref()
            .map(|window| window.fullscreen().is_some())
            .unwrap_or(false);
        let update = match &self.update_status {
            UpdateStatus::Unknown => ui::VERSION.to_string(),
            UpdateStatus::UpToDate => "up to date".to_string(),
            UpdateStatus::Available(version) => format!("{version} available"),
        };
        vec![
            (
                ui::MenuItem::row("WEBDAV", "sync the whole vault", sync),
                Some(Action::Webdav),
            ),
            (ui::MenuItem::rule(), None),
            (
                ui::MenuItem::row("THEME", "colour scheme", self.theme.name),
                Some(Action::Theme),
            ),
            (
                ui::MenuItem::row("FONT", "bitmap face", self.font.face().name()),
                Some(Action::Font),
            ),
            (
                ui::MenuItem::row(
                    "SCREEN",
                    "window or whole screen",
                    if fullscreen { "fullscreen" } else { "window" },
                ),
                Some(Action::Screen),
            ),
            (
                ui::MenuItem::row(
                    "SCREENSAVER",
                    "clock when idle",
                    if self.screensaver_on { "on" } else { "off" },
                ),
                Some(Action::Screensaver),
            ),
            (
                ui::MenuItem::row(
                    "IDLE",
                    "until it starts",
                    format!("{} min", self.screensaver_minutes),
                ),
                Some(Action::Idle),
            ),
            (ui::MenuItem::rule(), None),
            (
                ui::MenuItem::row(
                    "TIME",
                    "clock format",
                    if self.clock_24h {
                        "24 hours"
                    } else {
                        "12 hours"
                    },
                ),
                Some(Action::Time),
            ),
            (
                ui::MenuItem::row(
                    "DATE",
                    "date format",
                    if self.date_day_first {
                        "DD.MM.YY"
                    } else {
                        "MM/DD/YY"
                    },
                ),
                Some(Action::Date),
            ),
            (ui::MenuItem::rule(), None),
            (
                ui::MenuItem::row("INFO", "CRead OS and device info", ui::VERSION),
                Some(Action::Info),
            ),
            (
                ui::MenuItem::row("UPDATE", "check github for a new version", update),
                Some(Action::Update),
            ),
            (
                ui::MenuItem::row("SOFTWARE", "install and remove modules", ""),
                Some(Action::Software),
            ),
            (
                ui::MenuItem::row(
                    "STORAGE",
                    storage_detail(self.space),
                    storage_value(self.space),
                ),
                Some(Action::Storage),
            ),
            (
                ui::MenuItem::row(
                    "NETWORK",
                    "local address",
                    self.machine
                        .as_ref()
                        .map(Machine::ip_label)
                        .unwrap_or_else(|| "no address".into()),
                ),
                Some(Action::Network),
            ),
            (
                ui::MenuItem::row("WLAN", "scan and connect", self.wifi_label()),
                Some(Action::Wifi),
            ),
            (
                ui::MenuItem::row("USER MANUAL", "read the manual", ""),
                Some(Action::Manual),
            ),
        ]
    }

    pub(super) fn setup_rows(&self) -> Vec<ui::MenuItem> {
        self.setup_entries()
            .into_iter()
            .map(|(row, _)| row)
            .collect()
    }

    /// Move the highlight, skipping the rules.
    pub(super) fn setup_step(&mut self, delta: isize) {
        let entries = self.setup_entries();
        let mut index = self.setup_selected as isize;
        loop {
            index += delta;
            if index < 0 || index >= entries.len() as isize {
                return;
            }
            if entries[index as usize].1.is_some() {
                self.setup_selected = index as usize;
                self.poke();
                return;
            }
        }
    }

    pub(super) fn open_software(&mut self) {
        self.mode = Mode::Software;
        self.software_selected = self.software_selected.min(PROGRAMS.len());
        self.command.clear();
        self.program_prompt = false;
        self.dirty = true;
    }

    pub(super) fn open_setup(&mut self) {
        self.refresh_space();
        self.refresh_machine();
        self.mode = Mode::Setup;
        let entries = self.setup_entries();
        if self.setup_selected >= entries.len() || entries[self.setup_selected].1.is_none() {
            self.setup_selected = entries
                .iter()
                .position(|(_, action)| action.is_some())
                .unwrap_or(0);
        }
        self.command.clear();
        self.program_prompt = false;
        self.dirty = true;
    }

    pub(super) fn refresh_machine(&mut self) {
        self.machine = Some(Machine::read());
    }

    pub(super) fn refresh_space(&mut self) {
        let vault = self.vault();
        self.space = space::at(&vault).or_else(|| space::at(std::path::Path::new("/")));
    }

    /// What Enter does on the SETTINGS screen.
    pub(super) fn setup_activate(&mut self) {
        let Some(action) = self
            .setup_entries()
            .get(self.setup_selected)
            .and_then(|(_, action)| *action)
        else {
            return;
        };
        match action {
            SetupAction::Webdav => {
                // WebDAV is a form, not a value in a row.
                self.open_webdav();
            }
            SetupAction::Theme => {
                let _ = self.cycle_theme(1);
            }
            SetupAction::Font => {
                let next = match self.font.face() {
                    Face::Classic => Face::Thin,
                    Face::Thin => Face::Classic,
                };
                self.font = Font::with_face(next);
                self.prefs.font = next.name().to_string();
                self.save_prefs();
                self.dirty = true;
                self.set_status(format!("font: {}", next.name()));
            }
            SetupAction::Screen => {
                if let Some(window) = self.window.as_ref() {
                    let going_fullscreen = window.fullscreen().is_none();
                    window.set_fullscreen(going_fullscreen.then_some(Fullscreen::Borderless(None)));
                    self.set_status(if going_fullscreen {
                        "fullscreen"
                    } else {
                        "windowed"
                    });
                }
            }
            SetupAction::Screensaver => {
                self.screensaver_on = !self.screensaver_on;
                self.save_prefs();
                self.set_status(format!("screensaver: {}", on_off(self.screensaver_on)));
                self.dirty = true;
            }
            SetupAction::Idle => {
                self.screensaver_minutes = next_idle_minutes(self.screensaver_minutes);
                self.save_prefs();
                self.set_status(format!(
                    "screensaver after {} min",
                    self.screensaver_minutes
                ));
                self.dirty = true;
            }
            SetupAction::Time => {
                self.clock_24h = !self.clock_24h;
                self.save_prefs();
                self.set_status(if self.clock_24h {
                    "clock: 24 hours"
                } else {
                    "clock: 12 hours"
                });
                self.dirty = true;
            }
            SetupAction::Date => {
                self.date_day_first = !self.date_day_first;
                self.save_prefs();
                self.set_status(if self.date_day_first {
                    "date: DD.MM.YY"
                } else {
                    "date: MM/DD/YY"
                });
                self.dirty = true;
            }
            SetupAction::Info => {
                self.help_about = true;
                self.help_from_setup = true;
                self.mode = Mode::Help;
                self.dirty = true;
            }
            SetupAction::Update => self.start_update_check(),
            SetupAction::Software => self.open_software(),
            SetupAction::Storage => {
                self.refresh_space();
                self.dirty = true;
            }
            SetupAction::Network => {
                self.refresh_machine();
                self.dirty = true;
            }
            SetupAction::Wifi => self.open_wifi(),
            SetupAction::Manual => self.open_manual(Mode::Setup),
        }
    }

    pub(super) fn on_key_setup(&mut self, event: KeyEvent) {
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => self.to_write(),
            Key::Named(NamedKey::Enter) => self.setup_activate(),
            Key::Named(NamedKey::ArrowUp) => self.setup_step(-1),
            Key::Named(NamedKey::ArrowDown) => self.setup_step(1),
            _ => {}
        }
    }

    /// Enter on the SOFTWARE screen: install or remove, with the bar in between.
    pub(super) fn software_activate(&mut self) {
        // Row 0 is Inspiro, the system; it cannot be removed.
        if self.software_selected == 0 {
            self.set_status("Inspiro is the system - it stays");
            return;
        }
        let Some(program) = PROGRAMS.get(self.software_selected - 1) else {
            return;
        };
        let install = !self.is_installed(program.id);
        self.start_loading(Pending::Software {
            id: program.id,
            install,
        });
    }

    /// The change itself, once the bar has run.
    pub(super) fn apply_software(&mut self, id: &'static str, install: bool) {
        let Some(program) = PROGRAMS.iter().find(|program| program.id == id) else {
            return;
        };
        // A row is about to appear or vanish above the cursor: remember which
        // row it was on, not where, and find it again afterwards.
        let selected = self.boot_items().get(self.boot_selected).copied();
        if install {
            if !self.is_installed(id) {
                self.prefs.installed.push(id.to_string());
            }
        } else {
            if program.id == "riff" {
                // The deck is the shell's; removing the player silences it.
                if let Some(audio) = self.audio.as_mut() {
                    audio.stop();
                }
                self.riff.playing = false;
            }
            self.prefs.installed.retain(|have| have != id);
        }
        self.save_prefs();
        self.set_status(format!(
            "{} {}",
            if install { "installed" } else { "removed" },
            program.entry
        ));
        match selected.and_then(|item| self.boot_items().iter().position(|have| *have == item)) {
            Some(index) => self.boot_selected = index,
            None => {
                if self.boot_selected >= self.boot_items().len() {
                    self.boot_selected = 0;
                }
            }
        }
    }

    pub(super) fn on_key_software(&mut self, event: KeyEvent) {
        match &event.logical_key {
            Key::Named(NamedKey::Escape) | Key::Named(NamedKey::Enter) => {
                if matches!(&event.logical_key, Key::Named(NamedKey::Escape)) {
                    self.to_write();
                } else {
                    self.software_activate();
                }
            }
            Key::Named(NamedKey::ArrowUp) => {
                self.software_selected = self.software_selected.saturating_sub(1);
                self.poke();
            }
            Key::Named(NamedKey::ArrowDown) => {
                let last = PROGRAMS.len();
                self.software_selected = (self.software_selected + 1).min(last);
                self.poke();
            }
            _ => {}
        }
    }
}
