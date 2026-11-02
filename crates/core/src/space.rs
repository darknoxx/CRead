//! How full the disk is, for the SETTINGS row.
//!
//! `df -P -k` is POSIX and exists on the Pi and macOS; asking the shell is
//! simpler than a statvfs binding, and the value is only read when SETTINGS
//! opens or the row is activated.

use std::path::Path;

/// Space on one filesystem, in bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Space {
    pub total: u64,
    pub used: u64,
    pub free: u64,
}

impl Space {
    /// Used space as a percentage, rounded to the nearest point.
    pub fn percent(&self) -> u32 {
        if self.total == 0 {
            return 0;
        }
        ((self.used.saturating_mul(100) + self.total / 2) / self.total) as u32
    }

    /// A ten-cell bar: filled for used space, empty for the rest.
    pub fn bar(&self) -> String {
        let cells = 10usize;
        let mut filled = (self.used.saturating_mul(cells as u64) + self.total / 2)
            .checked_div(self.total)
            .unwrap_or(0) as usize;
        filled = filled.min(cells);
        // A disk that is not empty gets at least one cell, even below 5%.
        if self.total > 0 && self.used > 0 && filled == 0 {
            filled = 1;
        }
        let mut out = String::with_capacity(cells);
        for index in 0..cells {
            out.push(if index < filled {
                '\u{2588}'
            } else {
                '\u{2591}'
            });
        }
        out
    }

    /// `12.4/29.7 GB` for the value column, compact enough for 80 columns.
    pub fn used_total(&self) -> String {
        const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
        const MIB: f64 = 1024.0 * 1024.0;
        if self.total >= GIB as u64 {
            let used = self.used as f64 / GIB;
            let total = self.total as f64 / GIB;
            if total >= 100.0 {
                format!("{used:.0}/{total:.0} GB")
            } else {
                format!("{used:.1}/{total:.1} GB")
            }
        } else if self.total >= MIB as u64 {
            let used = self.used as f64 / MIB;
            let total = self.total as f64 / MIB;
            format!("{used:.0}/{total:.0} MB")
        } else {
            format!("{}/{} KB", self.used / 1024, self.total / 1024)
        }
    }
}

/// Read the filesystem that holds `path`. `None` when the path or `df` fails.
pub fn at(path: &Path) -> Option<Space> {
    let output = std::process::Command::new("df")
        .args(["-P", "-k"])
        .arg(path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse(&String::from_utf8_lossy(&output.stdout))
}

/// Parse the last line of POSIX `df -P -k` output. Exposed for the test.
pub fn parse(text: &str) -> Option<Space> {
    let line = text.lines().rev().find(|line| !line.trim().is_empty())?;
    let fields: Vec<&str> = line.split_whitespace().collect();
    if fields.len() < 4 {
        return None;
    }
    let blocks = |index: usize| fields[index].parse::<u64>().ok()?.checked_mul(1024);
    Some(Space {
        total: blocks(1)?,
        used: blocks(2)?,
        free: blocks(3)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const DF: &str = "\
Filesystem     1024-blocks     Used Available Capacity Mounted on
/dev/root         30475200 12345678  16612345      43% /
";

    #[test]
    fn reads_the_last_line_of_df() {
        let space = parse(DF).expect("a filesystem line");
        assert_eq!(space.total, 30_475_200 * 1024);
        assert_eq!(space.used, 12_345_678 * 1024);
        assert_eq!(space.free, 16_612_345 * 1024);
        assert_eq!(space.percent(), 41);
    }

    #[test]
    fn refuses_text_that_is_not_df() {
        assert_eq!(parse(""), None);
        assert_eq!(parse("no filesystem here"), None);
        assert_eq!(
            parse("Filesystem 1K-blocks Used Available Capacity Mounted\n"),
            None
        );
    }

    #[test]
    fn the_bar_fills_from_used_space() {
        let space = |used, total| Space {
            total,
            used,
            free: total.saturating_sub(used),
        };
        assert_eq!(space(0, 100).bar(), "\u{2591}".repeat(10));
        assert_eq!(
            space(50, 100).bar(),
            format!("{}{}", "\u{2588}".repeat(5), "\u{2591}".repeat(5))
        );
        assert_eq!(space(100, 100).bar(), "\u{2588}".repeat(10));
        assert_eq!(space(50, 0).bar(), "\u{2591}".repeat(10));
        assert_eq!(
            space(1, 100).bar(),
            format!("{}{}", "\u{2588}", "\u{2591}".repeat(9))
        );
    }

    #[test]
    fn sizes_stay_compact() {
        let gib = 1024u64 * 1024 * 1024;
        let space = Space {
            total: 32 * gib,
            used: 12 * gib,
            free: 20 * gib,
        };
        assert_eq!(space.used_total(), "12.0/32.0 GB");

        let large = Space {
            total: 256 * gib,
            used: 128 * gib,
            free: 128 * gib,
        };
        assert_eq!(large.used_total(), "128/256 GB");
    }
}
