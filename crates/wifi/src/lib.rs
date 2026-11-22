//! Wi-Fi through NetworkManager: scan, connect, forget, and say whether the
//! machine is online.
//!
//! Everything runs `nmcli` on a worker thread; the UI drops [`Command`]s in and
//! picks [`Event`]s up when the waker rang. Passwords go to nmcli on stdin,
//! never on its command line, where any process could read them.

mod parse;

use std::io::Write;
use std::process::{Command as Process, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use cread_core::Waker;

pub use parse::{KeyMgmt, Link, Network, key_mgmt};

/// How often the link is checked while nothing else happens.
const LINK_EVERY: Duration = Duration::from_secs(30);

pub enum Command {
    /// Look for networks (and ask the radio to scan again).
    Scan,
    Connect {
        ssid: String,
        security: String,
        /// `None` for an open or an already saved network.
        password: Option<String>,
    },
    /// Delete the saved profile for this network.
    Forget(String),
    Shutdown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Networks(Vec<Network>),
    /// The link changed (or was asked for).
    Link(Link),
    Connected(String),
    Forgotten(String),
    /// One line for the screen.
    Failed(String),
    /// There is no NetworkManager here; the worker has stopped.
    Unavailable,
}

pub struct Worker {
    tx: Sender<Command>,
    rx: Receiver<Event>,
    handle: Option<JoinHandle<()>>,
}

/// Is NetworkManager's command line tool here at all?
pub fn available() -> bool {
    nmcli(&["-v"], None).is_ok()
}

impl Worker {
    /// Start the worker. It reports the link once at once, then on change.
    pub fn spawn(waker: Waker) -> Self {
        let (tx, commands) = mpsc::channel();
        let (events, rx) = mpsc::channel();
        let handle = thread::spawn(move || run(commands, events, waker));
        Self {
            tx,
            rx,
            handle: Some(handle),
        }
    }

    pub fn send(&self, command: Command) {
        let _ = self.tx.send(command);
    }

    pub fn try_recv(&self) -> Option<Event> {
        self.rx.try_recv().ok()
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.tx.send(Command::Shutdown);
        // Do not join: a connection attempt can take half a minute, and
        // quitting must not wait for it.
        self.handle.take();
    }
}

fn run(commands: Receiver<Command>, events: Sender<Event>, waker: Waker) {
    let emit = |event: Event| {
        if events.send(event).is_ok() {
            waker();
        }
    };
    if !available() {
        emit(Event::Unavailable);
        return;
    }
    let mut last = None;
    let mut report_link = |emit: &dyn Fn(Event), force: bool| {
        let now = current_link();
        if force || last.as_ref() != Some(&now) {
            emit(Event::Link(now.clone()));
            last = Some(now);
        }
    };
    report_link(&emit, true);
    loop {
        match commands.recv_timeout(LINK_EVERY) {
            Ok(Command::Scan) => match scan(true) {
                Ok(list) => emit(Event::Networks(list)),
                Err(e) => emit(Event::Failed(e)),
            },
            Ok(Command::Connect {
                ssid,
                security,
                password,
            }) => {
                match connect(&ssid, &security, password.as_deref()) {
                    Ok(()) => emit(Event::Connected(ssid)),
                    Err(e) => emit(Event::Failed(e)),
                }
                report_link(&emit, true);
                if let Ok(list) = scan(false) {
                    emit(Event::Networks(list));
                }
            }
            Ok(Command::Forget(ssid)) => {
                match forget(&ssid) {
                    Ok(()) => emit(Event::Forgotten(ssid)),
                    Err(e) => emit(Event::Failed(e)),
                }
                report_link(&emit, false);
                if let Ok(list) = scan(false) {
                    emit(Event::Networks(list));
                }
            }
            Ok(Command::Shutdown) | Err(RecvTimeoutError::Disconnected) => return,
            Err(RecvTimeoutError::Timeout) => report_link(&emit, false),
        }
    }
}

/// Run nmcli in English (its output is parsed) and return stdout, or the
/// first line of its complaint.
fn nmcli(args: &[&str], stdin: Option<&str>) -> Result<String, String> {
    let mut child = Process::new("nmcli")
        .args(args)
        .env("LC_ALL", "C")
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("nmcli: {e}"))?;
    if let (Some(text), Some(mut pipe)) = (stdin, child.stdin.take()) {
        let _ = pipe.write_all(text.as_bytes());
    }
    let out = child
        .wait_with_output()
        .map_err(|e| format!("nmcli: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr);
    // `connection edit` reports a failed save on stdout and still exits 0.
    let complaint = stderr
        .lines()
        .chain(stdout.lines())
        .find(|l| l.trim_start().starts_with("Error:"))
        .map(|l| l.trim().trim_start_matches("Error:").trim().to_string());
    match (out.status.success(), complaint) {
        (true, None) => Ok(stdout),
        (_, Some(complaint)) => Err(complaint),
        (false, None) => Err(format!("nmcli failed ({})", out.status)),
    }
}

fn scan(rescan: bool) -> Result<Vec<Network>, String> {
    let fields = [
        "-t",
        "-f",
        "IN-USE,SSID,SIGNAL,SECURITY",
        "device",
        "wifi",
        "list",
    ];
    let mut args = fields.to_vec();
    args.extend(["--rescan", if rescan { "yes" } else { "no" }]);
    // Scanning anew can be refused (permissions, a busy radio); the cached
    // list is better than none.
    let output = match nmcli(&args, None) {
        Ok(out) => out,
        Err(_) if rescan => {
            let mut cached = fields.to_vec();
            cached.extend(["--rescan", "no"]);
            nmcli(&cached, None)?
        }
        Err(e) => return Err(e),
    };
    let mut list = parse::networks(&output);
    let saved = saved_ssids();
    for network in &mut list {
        network.saved = saved.iter().any(|(_, ssid)| ssid == &network.ssid);
    }
    Ok(list)
}

/// Saved Wi-Fi profiles as (profile name, SSID).
fn saved_ssids() -> Vec<(String, String)> {
    let Ok(out) = nmcli(&["-t", "-f", "NAME,TYPE", "connection", "show"], None) else {
        return Vec::new();
    };
    parse::wifi_profiles(&out)
        .into_iter()
        .filter_map(|name| {
            let ssid = nmcli(
                &[
                    "-g",
                    "802-11-wireless.ssid",
                    "connection",
                    "show",
                    "id",
                    &name,
                ],
                None,
            )
            .ok()?;
            Some((name, ssid.trim_end_matches('\n').to_string()))
        })
        .collect()
}

fn current_link() -> Link {
    let devices = nmcli(&["-t", "-f", "TYPE,STATE", "device"], None).unwrap_or_default();
    let wifi = scan(false).unwrap_or_default();
    parse::link(&devices, &wifi)
}

fn connect(ssid: &str, security: &str, password: Option<&str>) -> Result<(), String> {
    if let Some((profile, _)) = saved_ssids().into_iter().find(|(_, s)| s == ssid)
        && password.is_none()
    {
        return nmcli(&["--wait", "30", "connection", "up", "id", &profile], None).map(|_| ());
    }
    let mgmt = key_mgmt(security)
        .ok_or_else(|| "this kind of network needs a login CRead cannot do yet".to_string())?;
    if mgmt != KeyMgmt::Open && password.is_none_or(str::is_empty) {
        return Err("this network needs a password".into());
    }
    // A fresh profile named after the network, replacing an old one with a
    // wrong password.
    let _ = forget(ssid);
    let script = parse::editor_script(ssid, mgmt, password);
    nmcli(
        &["connection", "edit", "type", "wifi", "con-name", ssid],
        Some(&script),
    )?;
    match nmcli(&["--wait", "30", "connection", "up", "id", ssid], None) {
        Ok(_) => Ok(()),
        Err(e) => {
            // Do not keep a profile that cannot connect: the next try asks
            // for the password again.
            let _ = forget(ssid);
            Err(
                if e.contains("Secrets were required") || e.contains("802-1x") {
                    "wrong password?".into()
                } else {
                    e
                },
            )
        }
    }
}

fn forget(ssid: &str) -> Result<(), String> {
    let profiles: Vec<String> = saved_ssids()
        .into_iter()
        .filter(|(_, s)| s == ssid)
        .map(|(name, _)| name)
        .collect();
    if profiles.is_empty() {
        return Err(format!("{ssid} is not saved"));
    }
    for name in profiles {
        nmcli(&["connection", "delete", "id", &name], None)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Read-only look at this machine's NetworkManager. Run with
    /// `cargo test -p cread-wifi -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn the_real_link_and_list() {
        eprintln!("available: {}", available());
        eprintln!("link: {:?}", current_link());
        eprintln!("scan: {:?}", scan(false));
        eprintln!("saved: {:?}", saved_ssids());
    }
}
