//! Reading `nmcli --terse` output. Pure text in, plain data out.

/// One network in range.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Network {
    pub ssid: String,
    /// 0 to 100.
    pub signal: u8,
    /// nmcli's words: `WPA2`, `WPA1 WPA2`, `WPA3`, `802.1X`, or empty (open).
    pub security: String,
    /// The machine is on this network now.
    pub in_use: bool,
    /// A profile for it exists: no password needed to connect.
    pub saved: bool,
}

impl Network {
    pub fn is_open(&self) -> bool {
        self.security.trim().is_empty() || self.security.trim() == "--"
    }
}

/// How the machine is online, for the top bar.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Link {
    #[default]
    Offline,
    Wired,
    Wifi {
        ssid: String,
        signal: u8,
    },
}

/// What a connection attempt needs to say to NetworkManager.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyMgmt {
    Open,
    /// WPA/WPA2 personal (and WPA3 in transition mode).
    WpaPsk,
    /// WPA3 personal only.
    Sae,
}

/// Which key management a network's security words call for. `None` for
/// what the first version does not handle: enterprise logins and WEP.
pub fn key_mgmt(security: &str) -> Option<KeyMgmt> {
    let words = security.to_ascii_uppercase();
    if words.trim().is_empty() || words.trim() == "--" {
        return Some(KeyMgmt::Open);
    }
    if words.contains("802.1X") || words.contains("WEP") {
        return None;
    }
    if words.contains("WPA3") && !words.contains("WPA2") && !words.contains("WPA1") {
        return Some(KeyMgmt::Sae);
    }
    Some(KeyMgmt::WpaPsk)
}

/// Split one terse line into its fields. nmcli escapes `:` and `\` inside a
/// value with a backslash; an SSID may contain either.
pub fn fields(line: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut chars = line.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' => {
                if let Some(next) = chars.next() {
                    out.last_mut().unwrap().push(next);
                }
            }
            ':' => out.push(String::new()),
            other => out.last_mut().unwrap().push(other),
        }
    }
    out
}

/// `nmcli -t -f IN-USE,SSID,SIGNAL,SECURITY device wifi list`
///
/// Hidden networks (no SSID) are left out, and an SSID seen from several
/// access points appears once, with its strongest signal.
pub fn networks(output: &str) -> Vec<Network> {
    let mut out: Vec<Network> = Vec::new();
    for line in output.lines() {
        let f = fields(line);
        if f.len() < 4 {
            continue;
        }
        let ssid = f[1].clone();
        if ssid.is_empty() {
            continue;
        }
        let network = Network {
            ssid,
            signal: f[2].trim().parse::<u8>().unwrap_or(0).min(100),
            security: f[3].trim().to_string(),
            in_use: f[0].trim() == "*",
            saved: false,
        };
        match out.iter_mut().find(|n| n.ssid == network.ssid) {
            Some(existing) => {
                existing.in_use |= network.in_use;
                if network.signal > existing.signal {
                    existing.signal = network.signal;
                    existing.security = network.security;
                }
            }
            None => out.push(network),
        }
    }
    // The one in use first, then by signal.
    out.sort_by(|a, b| b.in_use.cmp(&a.in_use).then(b.signal.cmp(&a.signal)));
    out
}

/// `nmcli -t -f TYPE,STATE device` plus the in-use row of the wifi list.
pub fn link(devices: &str, wifi: &[Network]) -> Link {
    let mut wired = false;
    let mut wireless = false;
    for line in devices.lines() {
        let f = fields(line);
        if f.len() < 2 || f[1] != "connected" {
            continue;
        }
        match f[0].as_str() {
            "ethernet" => wired = true,
            "wifi" => wireless = true,
            _ => {}
        }
    }
    if wireless && let Some(net) = wifi.iter().find(|n| n.in_use) {
        return Link::Wifi {
            ssid: net.ssid.clone(),
            signal: net.signal,
        };
    }
    if wired {
        return Link::Wired;
    }
    if wireless {
        // Connected, but the scan list did not say to what.
        return Link::Wifi {
            ssid: String::new(),
            signal: 0,
        };
    }
    Link::Offline
}

/// `nmcli -t -f NAME,TYPE connection show`: the names of saved Wi-Fi profiles.
pub fn wifi_profiles(output: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|line| {
            let f = fields(line);
            (f.len() >= 2 && f[1] == "802-11-wireless").then(|| f[0].clone())
        })
        .collect()
}

/// The commands that create a profile through `nmcli connection edit`, read
/// from stdin so the password never appears in a process list. The answers
/// to its two questions (save with autoconnect? quit unsaved?) are included.
pub fn editor_script(ssid: &str, mgmt: KeyMgmt, password: Option<&str>) -> String {
    let mut script = format!("set 802-11-wireless.ssid {}\n", one_line(ssid));
    match mgmt {
        KeyMgmt::Open => {}
        KeyMgmt::WpaPsk | KeyMgmt::Sae => {
            let name = if mgmt == KeyMgmt::Sae {
                "sae"
            } else {
                "wpa-psk"
            };
            script.push_str(&format!("set 802-11-wireless-security.key-mgmt {name}\n"));
            if let Some(password) = password {
                script.push_str(&format!(
                    "set 802-11-wireless-security.psk {}\n",
                    one_line(password)
                ));
            }
        }
    }
    script.push_str("set connection.autoconnect yes\nsave persistent\nyes\nquit\nyes\n");
    script
}

fn one_line(text: &str) -> String {
    text.replace(['\n', '\r'], " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: &str = "\
*:FRITZ!Box 7590:82:WPA2
 :Nachbar\\:WLAN:40:WPA1 WPA2
 :Café Gast:31:
 ::55:WPA2
 :FRITZ!Box 7590:60:WPA2
 :Back\\\\slash:20:WPA3
";

    #[test]
    fn terse_fields_unescape() {
        assert_eq!(fields("a\\:b:c\\\\d:"), vec!["a:b", "c\\d", ""]);
    }

    #[test]
    fn the_list_is_deduplicated_sorted_and_without_hidden_networks() {
        let list = networks(LIST);
        let names: Vec<&str> = list.iter().map(|n| n.ssid.as_str()).collect();
        assert_eq!(
            names,
            ["FRITZ!Box 7590", "Nachbar:WLAN", "Café Gast", "Back\\slash"]
        );
        assert!(list[0].in_use);
        assert_eq!(list[0].signal, 82);
        assert!(list[2].is_open());
        assert!(!list[1].is_open());
    }

    #[test]
    fn the_link_names_the_network_in_use() {
        let list = networks(LIST);
        let devices = "wifi:connected\nethernet:unavailable\nloopback:connected (externally)\n";
        assert_eq!(
            link(devices, &list),
            Link::Wifi {
                ssid: "FRITZ!Box 7590".into(),
                signal: 82
            }
        );
        assert_eq!(
            link("ethernet:connected\nwifi:disconnected\n", &list),
            Link::Wired
        );
        assert_eq!(link("wifi:disconnected\n", &[]), Link::Offline);
    }

    #[test]
    fn security_words_pick_the_key_management() {
        assert_eq!(key_mgmt(""), Some(KeyMgmt::Open));
        assert_eq!(key_mgmt("WPA1 WPA2"), Some(KeyMgmt::WpaPsk));
        assert_eq!(key_mgmt("WPA2 WPA3"), Some(KeyMgmt::WpaPsk));
        assert_eq!(key_mgmt("WPA3"), Some(KeyMgmt::Sae));
        assert_eq!(key_mgmt("WPA2 802.1X"), None);
        assert_eq!(key_mgmt("WEP"), None);
    }

    #[test]
    fn only_wifi_profiles_count() {
        let out = "Home:802-11-wireless\nWired connection 1:802-3-ethernet\nlo:loopback\n";
        assert_eq!(wifi_profiles(out), ["Home"]);
    }

    #[test]
    fn the_editor_script_carries_the_password_on_stdin() {
        let script = editor_script("Home\nNet", KeyMgmt::WpaPsk, Some("geheim 123"));
        assert!(script.starts_with("set 802-11-wireless.ssid Home Net\n"));
        assert!(script.contains("key-mgmt wpa-psk\n"));
        assert!(script.contains("psk geheim 123\n"));
        assert!(script.contains("save persistent\nyes\n"));
        let open = editor_script("Café", KeyMgmt::Open, None);
        assert!(!open.contains("security"));
        assert!(editor_script("x", KeyMgmt::Sae, Some("p")).contains("key-mgmt sae"));
    }
}
