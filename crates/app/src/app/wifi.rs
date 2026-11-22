//! WLAN: the Wi-Fi screen behind SETTINGS, and the network mark in the top bar.
//!
//! NetworkManager does the work on `cread-wifi`'s worker thread; this is the
//! screen and the bookkeeping. The list is a menu like SOFTWARE, the password
//! a one-field form like WEBDAV.

use super::*;
use cread_wifi::{Command as WifiCommand, Event as WifiEvent, Link, Network};

/// The screen's state; the link itself lives in `App::wifi_link`.
#[derive(Default)]
pub(super) struct WifiState {
    pub(super) networks: Vec<Network>,
    pub(super) selected: usize,
    /// What the worker is doing: `scanning`, `connecting to …`.
    pub(super) busy: Option<String>,
    /// The last result, shown under the list.
    pub(super) note: String,
    /// A password being typed for this network.
    pub(super) password: Option<(Network, String)>,
    /// No NetworkManager on this machine.
    pub(super) unavailable: bool,
}

/// Signal strength as four cells.
fn bars(signal: u8) -> String {
    let lit = match signal {
        0..=24 => 1,
        25..=49 => 2,
        50..=74 => 3,
        _ => 4,
    };
    format!("{}{}", "\u{25a0}".repeat(lit), "\u{b7}".repeat(4 - lit))
}

impl App {
    /// Start the worker once there is a loop to wake. Tests never do.
    pub(super) fn start_wifi(&mut self) {
        if self.wifi.is_some() || !self.persist {
            return;
        }
        if let Some(waker) = self.waker.clone() {
            self.wifi = Some(cread_wifi::Worker::spawn(waker));
        }
    }

    /// Collect the worker's news. Draws only when something changed.
    pub(super) fn poll_wifi(&mut self) {
        let mut events = Vec::new();
        if let Some(worker) = self.wifi.as_ref() {
            while let Some(event) = worker.try_recv() {
                events.push(event);
            }
        }
        if events.is_empty() {
            return;
        }
        for event in events {
            self.wifi_event(event);
        }
        self.poke();
    }

    pub(super) fn wifi_event(&mut self, event: WifiEvent) {
        match event {
            WifiEvent::Networks(list) => {
                self.wifi_ui.networks = list;
                let last = self.wifi_ui.networks.len().saturating_sub(1);
                self.wifi_ui.selected = self.wifi_ui.selected.min(last);
                self.wifi_ui.busy = None;
            }
            WifiEvent::Link(link) => self.wifi_link = link,
            WifiEvent::Connected(ssid) => {
                self.wifi_ui.busy = None;
                self.wifi_ui.note = format!("connected to {ssid}");
            }
            WifiEvent::Forgotten(ssid) => {
                self.wifi_ui.busy = None;
                self.wifi_ui.note = format!("forgot {ssid}");
            }
            WifiEvent::Failed(why) => {
                self.wifi_ui.busy = None;
                self.wifi_ui.note = why;
            }
            WifiEvent::Unavailable => {
                self.wifi_ui.unavailable = true;
                self.wifi_ui.busy = None;
                self.wifi = None;
            }
        }
    }

    /// The top bar's mark: Wi-Fi in three strengths, a cable, or nothing.
    pub(super) fn net_mark(&self) -> Option<char> {
        use cread_render::font::{NET_WIFI, NET_WIRED};
        match &self.wifi_link {
            Link::Offline => None,
            Link::Wired => Some(NET_WIRED),
            Link::Wifi { signal, .. } => Some(match signal {
                0..=33 => NET_WIFI[0],
                34..=66 => NET_WIFI[1],
                _ => NET_WIFI[2],
            }),
        }
    }

    /// The SETTINGS row's value.
    pub(super) fn wifi_label(&self) -> String {
        if self.wifi_ui.unavailable {
            return "no network manager".into();
        }
        match &self.wifi_link {
            Link::Wifi { ssid, .. } if !ssid.is_empty() => ssid.clone(),
            Link::Wifi { .. } => "connected".into(),
            Link::Wired => "cable".into(),
            Link::Offline => "not connected".into(),
        }
    }

    pub(super) fn open_wifi(&mut self) {
        self.start_wifi();
        self.wifi_ui.password = None;
        self.wifi_ui.note.clear();
        if self.wifi_ui.unavailable {
            self.wifi_ui.note = "NetworkManager (nmcli) is not installed".into();
        }
        self.wifi_scan();
        self.mode = Mode::Wifi;
        self.poke();
    }

    fn wifi_scan(&mut self) {
        if let Some(worker) = self.wifi.as_ref() {
            worker.send(WifiCommand::Scan);
            self.wifi_ui.busy = Some("scanning".into());
        }
    }

    fn wifi_selected(&self) -> Option<Network> {
        self.wifi_ui.networks.get(self.wifi_ui.selected).cloned()
    }

    /// Enter on a network: connect, asking for a password only when needed.
    fn wifi_activate(&mut self) {
        let Some(network) = self.wifi_selected() else {
            return;
        };
        if network.in_use {
            self.wifi_ui.note = format!("already on {}", network.ssid);
            return;
        }
        if cread_wifi::key_mgmt(&network.security).is_none() {
            self.wifi_ui.note = "this kind of network needs a login CRead cannot do yet".into();
            return;
        }
        if network.saved || network.is_open() {
            self.wifi_connect(network, None);
        } else {
            self.wifi_ui.password = Some((network, String::new()));
        }
    }

    fn wifi_connect(&mut self, network: Network, password: Option<String>) {
        let Some(worker) = self.wifi.as_ref() else {
            return;
        };
        self.wifi_ui.busy = Some(format!("connecting to {}", network.ssid));
        self.wifi_ui.note.clear();
        worker.send(WifiCommand::Connect {
            ssid: network.ssid,
            security: network.security,
            password,
        });
    }

    pub(super) fn on_key_wifi(&mut self, event: KeyEvent) {
        if self.wifi_ui.password.is_some() {
            self.on_key_wifi_password(event);
            return;
        }
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => {
                self.mode = Mode::Setup;
            }
            Key::Named(NamedKey::Enter) => self.wifi_activate(),
            Key::Named(NamedKey::ArrowUp) => {
                self.wifi_ui.selected = self.wifi_ui.selected.saturating_sub(1);
            }
            Key::Named(NamedKey::ArrowDown) => {
                let last = self.wifi_ui.networks.len().saturating_sub(1);
                self.wifi_ui.selected = (self.wifi_ui.selected + 1).min(last);
            }
            Key::Named(NamedKey::Delete) => {
                if let Some(network) = self.wifi_selected() {
                    if !network.saved {
                        self.wifi_ui.note = format!("{} is not saved", network.ssid);
                    } else if let Some(worker) = self.wifi.as_ref() {
                        self.wifi_ui.busy = Some(format!("forgetting {}", network.ssid));
                        worker.send(WifiCommand::Forget(network.ssid));
                    }
                }
            }
            Key::Character(text) if text.eq_ignore_ascii_case("r") => self.wifi_scan(),
            _ => return,
        }
        self.poke();
    }

    fn on_key_wifi_password(&mut self, event: KeyEvent) {
        let paste = matches!(&event.logical_key, Key::Character(t) if t == "v")
            && (self.mods.control_key() || self.mods.super_key());
        if paste {
            let text = self
                .clipboard()
                .and_then(|c| c.get_text().map_err(|e| e.to_string()))
                .unwrap_or_default();
            if let Some((_, typed)) = self.wifi_ui.password.as_mut() {
                typed.push_str(text.trim_end_matches(['\n', '\r']));
            }
            self.poke();
            return;
        }
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => self.wifi_ui.password = None,
            Key::Named(NamedKey::Enter) => {
                if let Some((network, typed)) = self.wifi_ui.password.take() {
                    if typed.is_empty() {
                        self.wifi_ui.password = Some((network, typed));
                    } else {
                        self.wifi_connect(network, Some(typed));
                    }
                }
            }
            Key::Named(NamedKey::Backspace) => {
                if let Some((_, typed)) = self.wifi_ui.password.as_mut() {
                    typed.pop();
                }
            }
            Key::Named(NamedKey::Space) => {
                if let Some((_, typed)) = self.wifi_ui.password.as_mut() {
                    typed.push(' ');
                }
            }
            Key::Character(text) => {
                if let Some((_, typed)) = self.wifi_ui.password.as_mut() {
                    typed.push_str(text);
                }
            }
            _ => return,
        }
        self.poke();
    }

    pub(super) fn wifi_rows(&self) -> Vec<ui::MenuItem> {
        if self.wifi_ui.networks.is_empty() {
            let text = if self.wifi_ui.busy.is_some() {
                "looking for networks ..."
            } else {
                "no networks in range"
            };
            return vec![ui::MenuItem::row("", text, "")];
        }
        self.wifi_ui
            .networks
            .iter()
            .map(|network| {
                let value = if network.in_use {
                    "connected"
                } else if network.saved {
                    "saved"
                } else if network.is_open() {
                    "open"
                } else {
                    "locked"
                };
                ui::MenuItem::row(bars(network.signal), network.ssid.clone(), value)
            })
            .collect()
    }

    pub(super) fn draw_wifi(&self, screen: &mut Screen) {
        let rows = self.wifi_rows();
        // A long list is cut to what fits; the strongest networks come first.
        let room = screen.rows().saturating_sub(9) as usize;
        let first = self.wifi_ui.selected.saturating_sub(room.saturating_sub(1));
        let shown: Vec<ui::MenuItem> = rows.into_iter().skip(first).take(room.max(1)).collect();
        ui::draw_menu(
            screen,
            &self.theme,
            "WLAN",
            "Enter connect   R scan   Del forget",
            &shown,
            self.wifi_ui.selected - first,
        );
        let line = match (&self.wifi_ui.busy, self.wifi_ui.note.is_empty()) {
            (Some(busy), _) => format!("{busy} ..."),
            (None, false) => self.wifi_ui.note.clone(),
            (None, true) => String::new(),
        };
        if !line.is_empty() {
            let y = 3 + shown.len() as u32 + 2;
            ui::put_centered(screen, y, &line, self.theme.dim, self.theme.bg);
        }
        if let Some((network, typed)) = &self.wifi_ui.password {
            let fields = [ui::FormField {
                label: "password".into(),
                value: typed.clone(),
                secret: true,
            }];
            let note = format!("for {}", network.ssid);
            ui::draw_form(
                screen,
                &self.theme,
                &ui::FormView {
                    title: "WLAN PASSWORD",
                    fields: &fields,
                    focus: 0,
                    note: &note,
                    hint: "Enter connect   Esc cancel",
                },
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::bars;

    #[test]
    fn signal_becomes_four_cells() {
        assert_eq!(bars(10), "\u{25a0}\u{b7}\u{b7}\u{b7}");
        assert_eq!(bars(60), "\u{25a0}\u{25a0}\u{25a0}\u{b7}");
        assert_eq!(bars(100), "\u{25a0}\u{25a0}\u{25a0}\u{25a0}");
    }
}
