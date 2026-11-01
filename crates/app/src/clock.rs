//! The bar clock: 24 or 12 hours, German or American date order.
//!
//! Pure formatting, so the two settings can be tested without a window or a
//! clock. Seconds are not shown: the bars redraw with the cursor blink, so the
//! minute is as fine as the screen needs.

use chrono::{DateTime, Datelike, Local, Timelike};

/// `14:07` or `02:07 PM`.
pub fn time(hour: u32, minute: u32, clock_24h: bool) -> String {
    if clock_24h {
        format!("{hour:02}:{minute:02}")
    } else {
        let meridiem = if hour < 12 { "AM" } else { "PM" };
        let hour = match hour % 12 {
            0 => 12,
            hour => hour,
        };
        format!("{hour:02}:{minute:02} {meridiem}")
    }
}

/// `07.11.26` or `11/07/26`.
pub fn date(year: i32, month: u32, day: u32, day_first: bool) -> String {
    let year = year.rem_euclid(100);
    if day_first {
        format!("{day:02}.{month:02}.{year:02}")
    } else {
        format!("{month:02}/{day:02}/{year:02}")
    }
}

/// What the boot bar shows: `07.11.26 - 14:07` (or the chosen formats).
pub fn stamp(now: &DateTime<Local>, clock_24h: bool, day_first: bool, with_date: bool) -> String {
    let time = time(now.hour(), now.minute(), clock_24h);
    if with_date {
        format!(
            "{} - {}",
            date(now.year(), now.month(), now.day(), day_first),
            time
        )
    } else {
        time
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn the_clock_reads_in_both_conventions() {
        assert_eq!(time(14, 7, true), "14:07");
        assert_eq!(time(14, 7, false), "02:07 PM");
        assert_eq!(time(0, 5, false), "12:05 AM");
        assert_eq!(time(12, 0, false), "12:00 PM");
    }

    #[test]
    fn the_date_has_a_german_and_an_american_order() {
        assert_eq!(date(2026, 11, 7, true), "07.11.26");
        assert_eq!(date(2026, 11, 7, false), "11/07/26");
    }

    #[test]
    fn the_boot_stamp_joins_date_and_time() {
        let now = Local.with_ymd_and_hms(2026, 11, 7, 14, 7, 0).unwrap();
        assert_eq!(stamp(&now, true, true, true), "07.11.26 - 14:07");
        assert_eq!(stamp(&now, false, false, true), "11/07/26 - 02:07 PM");
        assert_eq!(stamp(&now, true, true, false), "14:07");
    }
}
