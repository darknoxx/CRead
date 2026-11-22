//! `--show`: start in a given view, for screenshots and visual checks.
//!
//! Many of these states are otherwise only reachable by typing; `cli::VIEWS`
//! lists every name this accepts.

use super::*;

impl App {
    /// Put the machine into the view named by `--show`, once the window and
    /// the vault are ready.
    pub(super) fn apply_show(&mut self) {
        match self.opts.show.as_str() {
            "palette" => self.open_browser(),
            "settings" => self.open_setup(),
            "webdav" => self.open_webdav(),
            "vault" => self.open_browser(),
            "screensaver" => self.start_screensaver(),
            "about" => {
                self.refresh_machine();
                self.help_about = true;
                self.mode = Mode::Help;
            }
            "updatecheck" => {
                self.update_started = Instant::now();
                self.update_installing = false;
                self.mode = Mode::Checking;
            }
            "updateprompt" => {
                self.update_offer = Some(cread_update::Offer {
                    version: "0.23.0".into(),
                    tarball: "https://example.org/cread.tar.gz".into(),
                    checksum: "https://example.org/cread.sha256".into(),
                    signature: "https://example.org/cread.minisig".into(),
                });
                self.update_status = UpdateStatus::Available("0.23.0".into());
                self.mode = Mode::UpdatePrompt;
            }
            "documents" => self.open_documents(),
            "shutdown" => {
                let _ = self.cmd_shutdown();
            }
            "reboot" => {
                let _ = self.cmd_reboot();
            }
            "backtolinux" => {
                let _ = self.cmd_backtolinux();
            }
            "quit" => {
                self.unsynced = true;
                self.quit = Some(QuitPrompt {
                    yes: true,
                    from: Mode::Action,
                });
                self.mode = Mode::Quit;
            }
            "help" => {
                self.mode = Mode::Help;
            }
            "manual" => self.open_manual(self.mode),
            "calendar" => self.open_calendar(),
            "note" => {
                let _ = self.open_note();
            }
            "calendaredit" => {
                self.open_calendar();
                self.new_event_form();
            }
            "reminder" => {
                self.open_calendar();
                self.reminder_title = "Demo appointment".into();
                self.reminder_when = "Today · 10:30".into();
                self.reminder_date = chrono::Local::now().date_naive();
                self.reminder_back = self.mode;
                self.mode = Mode::Reminder;
            }
            "printer" => self.open_printer(),
            "backup" => self.open_backup(),
            "backuprun" | "backupdone" => {
                // No real lookup: its answer would replace the demo.
                self.backup = peripherals::BackupState {
                    from: Some(Mode::Action),
                    ..Default::default()
                };
                self.mode = Mode::Backup;
                let stick = cread_peripherals::usb::Stick {
                    device: "/dev/sda1".into(),
                    disk: "/dev/sda".into(),
                    label: "KINGSTON".into(),
                    size: "14.6G".into(),
                    fstype: "vfat".into(),
                    mountpoint: None,
                };
                self.backup.sticks = vec![stick];
                self.backup.target = "KINGSTON".into();
                if self.opts.show == "backuprun" {
                    self.backup.phase = peripherals::BackupPhase::Running;
                    self.backup.progress = (87, 240);
                } else {
                    self.backup.phase = peripherals::BackupPhase::Done;
                    self.backup.note =
                        "240 files in CRead-Backup-2026-11-22 - the stick can come out".into();
                }
            }
            "wlan" => self.open_wifi(),
            "wlandemo" => {
                self.open_wifi();
                let net = |ssid: &str, signal, security: &str, in_use, saved| cread_wifi::Network {
                    ssid: ssid.into(),
                    signal,
                    security: security.into(),
                    in_use,
                    saved,
                };
                self.wifi = None;
                self.wifi_ui.busy = None;
                self.wifi_ui.networks = vec![
                    net("FRITZ!Box 7590", 82, "WPA2", true, true),
                    net("Nachbar-WLAN", 46, "WPA1 WPA2", false, false),
                    net("Cafe Gast", 31, "", false, false),
                    net("Buero", 12, "WPA2 802.1X", false, false),
                ];
                self.wifi_ui.selected = 1;
                self.wifi_ui.note = "connected to FRITZ!Box 7590".into();
            }
            "wlanpassword" => {
                self.open_wifi();
                let demo = cread_wifi::Network {
                    ssid: "FRITZ!Box 7590".into(),
                    signal: 82,
                    security: "WPA2".into(),
                    in_use: false,
                    saved: false,
                };
                self.wifi_ui.password = Some((demo, "geheim".into()));
            }
            "notice" => {
                self.open_webdav();
                self.open_notice("SYNC ON", Mode::Webdav);
            }
            "view" => self.enter_view(),
            "inspiro" => {
                self.home = Home::Inspiro;
                self.mode = Mode::Action;
            }
            "softhand" => {
                self.home = Home::SoftHand;
                self.mode = Mode::Action;
            }
            "riff" => {
                self.home = Home::Riff;
                self.mode = Mode::Action;
            }
            "poker" => {
                self.home = Home::Poker;
                self.mode = Mode::Action;
            }
            "scripture" => {
                self.home = Home::Scripture;
                self.mode = Mode::Action;
            }
            "scriptureread" => self.start_scripture(),
            "scripturegrid" => {
                self.start_scripture();
                if let Some(program) = self.program.as_mut() {
                    program.key(KeyPress::Char('c'));
                }
            }
            "pokerplay" => self.start_poker(),
            "pokerhand" => self.show_poker(cread_poker::Poker::demo_hand()),
            "pokerrisk" => self.show_poker(cread_poker::Poker::demo_risk()),
            "pokerover" => self.show_poker(cread_poker::Poker::demo_bust()),
            "riffplay" => {
                if let Err(e) = self
                    .riff_ensure_loaded()
                    .and_then(|()| self.riff_start_current())
                {
                    eprintln!("cread: {e}");
                }
                self.mode = Mode::Riff;
                if !self.opts.command.is_empty() {
                    self.command = self.opts.command.clone();
                    self.program_prompt = true;
                }
            }
            "riffinsert" => self.open_explorer(),
            "pocketboi" => {
                self.program = Some(Box::new(cread_pocketboi::Pocketboi::new(&self.vault())));
                self.program_id = Some("pocketboi");
                self.mode = Mode::Program;
            }
            other if other.starts_with("pocketboi:") => {
                let rom = PathBuf::from(&other["pocketboi:".len()..]);
                let mut boi = cread_pocketboi::Pocketboi::new(&self.vault());
                match boi.play(&rom) {
                    Ok(()) => {
                        self.program = Some(Box::new(boi));
                        self.program_id = Some("pocketboi");
                        self.mode = Mode::Program;
                    }
                    Err(e) => eprintln!("cread: {e}"),
                }
            }
            "picture" => {
                self.new_picture();
                if !self.opts.command.is_empty() {
                    self.command = self.opts.command.clone();
                    self.program_prompt = true;
                }
            }
            "pictures" => self.open_pictures(),
            other if other.starts_with("picture:") => {
                let path = PathBuf::from(&other["picture:".len()..]);
                if let Err(e) = self.open_picture(path) {
                    eprintln!("cread: {e}");
                }
            }
            "loading" => self.start_loading(Pending::Inspiro),
            "software" => self.open_software(),
            "search" => {
                let query = self.opts.command.clone();
                self.start_search((!query.is_empty()).then_some(query));
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
}
