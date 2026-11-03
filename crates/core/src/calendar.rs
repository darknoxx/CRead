//! Calendar events: a small Markdown front matter and a month grid.
//!
//! The files live in `<vault>/Calendar` and are ordinary Markdown so Inspiro
//! can open them. One file per event keeps WebDAV conflicts local.

use std::path::{Path, PathBuf};

use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime, Weekday};

/// A calendar entry, independent of its file name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub title: String,
    pub date: NaiveDate,
    pub time: Option<NaiveTime>,
    /// Minutes before `time`; `None` means no alarm.
    pub remind: Option<u32>,
    /// A tombstone: kept in the vault so a deleted event does not come back
    /// from the WebDAV server on the next sync.
    pub deleted: bool,
    pub notes: String,
}

impl Event {
    pub fn parse(text: &str) -> Result<Self, String> {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let mut lines = text.lines();
        let first = lines.next().unwrap_or_default();
        if first.trim() != "---" {
            return Err("missing calendar front matter".into());
        }
        let mut fields = std::collections::HashMap::new();
        let mut closed = false;
        for line in lines.by_ref() {
            if line.trim() == "---" {
                closed = true;
                break;
            }
            let Some((key, value)) = line.split_once(':') else {
                continue;
            };
            fields.insert(
                key.trim().to_ascii_lowercase(),
                value.trim().trim_matches('"').to_string(),
            );
        }
        if !closed {
            return Err("calendar front matter has no closing ---".into());
        }
        let title = fields
            .get("title")
            .cloned()
            .unwrap_or_else(|| "Untitled".into());
        let date = fields
            .get("date")
            .ok_or_else(|| "calendar event has no date".to_string())
            .and_then(|text| {
                NaiveDate::parse_from_str(text, "%Y-%m-%d").map_err(|_| format!("bad date: {text}"))
            })?;
        let time = match fields.get("time").map(String::as_str) {
            None | Some("") => None,
            Some(text) => Some(
                NaiveTime::parse_from_str(text, "%H:%M")
                    .map_err(|_| format!("bad time: {text}"))?,
            ),
        };
        let remind = match fields.get("remind").map(String::as_str) {
            None | Some("") => None,
            Some(text) => Some(
                text.parse::<u32>()
                    .map_err(|_| format!("bad reminder: {text}"))?,
            ),
        };
        let deleted = fields.get("deleted").is_some_and(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "true" | "1" | "yes" | "on"
            )
        });
        let notes = lines.collect::<Vec<_>>().join("\n");
        Ok(Self {
            title,
            date,
            time,
            remind,
            deleted,
            notes,
        })
    }

    pub fn to_markdown(&self) -> String {
        let mut out = String::from("---\n");
        out.push_str(&format!("title: {}\n", self.title.replace('\n', " ")));
        out.push_str(&format!("date: {}\n", self.date.format("%Y-%m-%d")));
        if let Some(time) = self.time {
            out.push_str(&format!("time: {}\n", time.format("%H:%M")));
        } else {
            out.push_str("time: \n");
        }
        match self.remind {
            Some(minutes) => out.push_str(&format!("remind: {minutes}\n")),
            None => out.push_str("remind: \n"),
        }
        if self.deleted {
            out.push_str("deleted: true\n");
        } else {
            out.push_str("deleted: false\n");
        }
        out.push_str("---\n");
        out.push_str(&self.notes);
        out.push('\n');
        out
    }

    pub fn reminder_at(&self) -> Option<NaiveDateTime> {
        let time = self.time?;
        let minutes = self.remind?;
        let start = self.date.and_time(time);
        start.checked_sub_signed(Duration::minutes(i64::from(minutes)))
    }

    pub fn time_label(&self, clock_24h: bool) -> String {
        let Some(time) = self.time else {
            return "all day".into();
        };
        if clock_24h {
            time.format("%H:%M").to_string()
        } else {
            time.format("%I:%M %p").to_string()
        }
    }

    pub fn sort_key(&self) -> (NaiveDate, Option<NaiveTime>, String) {
        (self.date, self.time, self.title.clone())
    }
}

/// An event and the file it came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub path: PathBuf,
    pub event: Event,
}

/// Scan `Calendar/` for active and deleted events.
///
/// Dotfiles (the sync index), conflict copies and malformed files are ignored.
pub fn load_events(dir: &Path) -> Vec<Entry> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.')
            || name.contains(".conflict-")
            || !name.to_ascii_lowercase().ends_with(".md")
        {
            continue;
        }
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(event) = Event::parse(&text) else {
            continue;
        };
        out.push(Entry { path, event });
    }
    out.sort_by_key(|entry| entry.event.sort_key());
    out
}

/// A stable, readable file name for a new event.
pub fn event_filename(date: NaiveDate, time: Option<NaiveTime>, title: &str) -> String {
    let mut slug = String::new();
    let mut dash = false;
    for ch in title.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            dash = false;
        } else if !dash && !slug.is_empty() {
            slug.push('-');
            dash = true;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        slug.push_str("event");
    }
    slug.truncate(32);
    let clock = time
        .map(|time| time.format("%H%M").to_string())
        .unwrap_or_else(|| "allday".into());
    format!("{}-{}-{}.md", date.format("%Y-%m-%d"), clock, slug)
}

/// A free path for a fresh event; an existing file gets a `-2`, `-3`, …
pub fn free_event_path(
    dir: &Path,
    date: NaiveDate,
    time: Option<NaiveTime>,
    title: &str,
) -> PathBuf {
    let name = event_filename(date, time, title);
    let stem = name.trim_end_matches(".md");
    let mut path = dir.join(&name);
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{stem}-{n}.md"));
        n += 1;
    }
    path
}

/// One month, always six weeks so the screen has a stable height.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Month {
    pub year: i32,
    pub month: u32,
    pub weeks: [[Option<NaiveDate>; 7]; 6],
}

pub fn month(year: i32, month: u32) -> Option<Month> {
    let first = NaiveDate::from_ymd_opt(year, month, 1)?;
    let next = if month == 12 {
        NaiveDate::from_ymd_opt(year + 1, 1, 1)?
    } else {
        NaiveDate::from_ymd_opt(year, month + 1, 1)?
    };
    let days = (next - first).num_days();
    let offset = first.weekday().num_days_from_monday() as usize;
    let mut weeks = [[None; 7]; 6];
    for day in 0..days {
        let index = offset + day as usize;
        let week = index / 7;
        let col = index % 7;
        if week < 6 {
            weeks[week][col] = first.checked_add_signed(Duration::days(day));
        }
    }
    Some(Month { year, month, weeks })
}

pub fn weekday_labels() -> [&'static str; 7] {
    ["MO", "TU", "WE", "TH", "FR", "SA", "SU"]
}

pub fn month_label(year: i32, month: u32) -> String {
    const NAMES: [&str; 12] = [
        "JANUARY",
        "FEBRUARY",
        "MARCH",
        "APRIL",
        "MAY",
        "JUNE",
        "JULY",
        "AUGUST",
        "SEPTEMBER",
        "OCTOBER",
        "NOVEMBER",
        "DECEMBER",
    ];
    NAMES
        .get(month.saturating_sub(1) as usize)
        .map(|name| format!("{name} {year}"))
        .unwrap_or_else(|| format!("{year}-{month:02}"))
}

pub fn is_same_day(date: NaiveDate, other: NaiveDate) -> bool {
    date == other
}

pub fn weekday(date: NaiveDate) -> Weekday {
    date.weekday()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(text: &str) -> NaiveDate {
        NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap()
    }

    fn time(text: &str) -> NaiveTime {
        NaiveTime::parse_from_str(text, "%H:%M").unwrap()
    }

    #[test]
    fn a_timed_event_round_trips() {
        let event = Event {
            title: "Zahnarzt".into(),
            date: date("2026-11-05"),
            time: Some(time("10:30")),
            remind: Some(15),
            deleted: false,
            notes: "Karte mitnehmen.".into(),
        };
        let parsed = Event::parse(&event.to_markdown()).unwrap();
        assert_eq!(parsed, event);
        assert_eq!(
            event.reminder_at(),
            Some(NaiveDateTime::parse_from_str("2026-11-05 10:15", "%Y-%m-%d %H:%M").unwrap())
        );
    }

    #[test]
    fn all_day_events_have_no_alarm() {
        let event =
            Event::parse("---\ntitle: Feiertag\ndate: 2026-12-25\ntime: \nremind: 10\n---\n")
                .unwrap();
        assert!(event.time.is_none());
        assert!(event.reminder_at().is_none());
        assert_eq!(event.time_label(true), "all day");
    }

    #[test]
    fn tombstones_are_marked_and_kept() {
        let event = Event {
            title: "Old".into(),
            date: date("2026-01-02"),
            time: None,
            remind: None,
            deleted: true,
            notes: String::new(),
        };
        assert!(Event::parse(&event.to_markdown()).unwrap().deleted);
    }

    #[test]
    fn malformed_events_are_rejected_not_panicked() {
        assert!(Event::parse("no header").is_err());
        assert!(Event::parse("---\ntitle: x\ndate: nope\n---\n").is_err());
        assert!(Event::parse("---\ntitle: x\ndate: 2026-01-01\ntime: 99:99\n---\n").is_err());
    }

    #[test]
    fn the_month_starts_on_monday_and_pads_six_weeks() {
        let november = month(2026, 11).unwrap();
        assert_eq!(
            november.weeks[0][6],
            Some(date("2026-11-01")),
            "1 Nov is Sunday"
        );
        assert_eq!(november.weeks[1][0], Some(date("2026-11-02")));
        assert_eq!(november.weeks[4][6], Some(date("2026-11-29")));
        assert_eq!(november.weeks[5][0], Some(date("2026-11-30")));
        assert_eq!(november.weeks[5][1], None);
    }

    #[test]
    fn leap_years_and_month_ends() {
        let feb = month(2028, 2).unwrap();
        assert_eq!(feb.weeks[4][1], Some(date("2028-02-29")));
        assert!(month(2026, 13).is_none());
        assert!(month(2026, 0).is_none());
    }

    #[test]
    fn filenames_are_safe_and_unique() {
        let name = event_filename(
            date("2026-11-05"),
            Some(time("10:30")),
            "Zahnarzt / Kontrolle",
        );
        assert_eq!(name, "2026-11-05-1030-zahnarzt-kontrolle.md");
        let all_day = event_filename(date("2026-11-05"), None, "Feiertag");
        assert_eq!(all_day, "2026-11-05-allday-feiertag.md");
    }

    #[test]
    fn loading_skips_conflicts_hidden_files_and_bad_events() {
        let dir = std::env::temp_dir().join(format!("cread-calendar-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let good = Event {
            title: "Good".into(),
            date: date("2026-11-05"),
            time: None,
            remind: None,
            deleted: false,
            notes: String::new(),
        };
        std::fs::write(dir.join("good.md"), good.to_markdown()).unwrap();
        std::fs::write(dir.join("good.conflict-1.md"), good.to_markdown()).unwrap();
        std::fs::write(dir.join(".hidden.md"), good.to_markdown()).unwrap();
        std::fs::write(dir.join("bad.md"), "broken").unwrap();
        let entries = load_events(&dir);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].event.title, "Good");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
