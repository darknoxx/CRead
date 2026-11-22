//! The machine's outside world beyond the network: a printer and USB sticks.
//!
//! Everything here blocks (it runs `lp`, `lpstat`, `lsblk`, `udisksctl` and
//! copies files), so the shell calls it from a worker thread, never from the
//! UI thread.

pub mod printer;
pub mod usb;

use std::io::Write;
use std::process::{Command, Stdio};

/// Run a tool in English (its output is parsed) and return stdout, or the
/// first line it complained with.
pub(crate) fn run(program: &str, args: &[&str], stdin: Option<&str>) -> Result<String, String> {
    let mut child = Command::new(program)
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
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                format!("{program} is not installed")
            } else {
                format!("{program}: {e}")
            }
        })?;
    if let (Some(text), Some(mut pipe)) = (stdin, child.stdin.take()) {
        let _ = pipe.write_all(text.as_bytes());
    }
    let out = child
        .wait_with_output()
        .map_err(|e| format!("{program}: {e}"))?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    let line = stderr
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .trim_start_matches(&format!("{program}: "))
        .to_string();
    Err(if line.is_empty() {
        format!("{program} failed ({})", out.status)
    } else {
        line
    })
}

#[cfg(test)]
mod tests {
    /// Read-only look at this machine. `cargo test -p cread-peripherals -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn the_real_printers_and_sticks() {
        eprintln!("printers: {:?}", crate::printer::list());
        eprintln!("sticks: {:?}", crate::usb::sticks());
    }
}
