//! The screensaver: an analog clock on a black screen.
//!
//! Deliberately theme-agnostic. Every other screen follows the theme's page and
//! ink; this one is the machine resting, so it uses the fixed VGA palette:
//! black, a grey dial, white hands and one red second hand. It is drawn into a
//! full-screen raster, which means the CRT pass still runs over it like over any
//! other picture.

use std::f32::consts::TAU;

use cread_core::Raster;

/// Fixed palette indices; no theme colour is used here.
const BLACK: u8 = 0;
const DIAL: u8 = 8;
const TICK: u8 = 7;
const HAND: u8 = 15;
const SECOND: u8 = 12;

/// Draw the clock for `hour:minute:second`, centred, as large as fits.
///
/// `hour` is the clock hour (0-23); the fractions come from the minute and the
/// second, so the hour hand creeps towards the next hour as it should.
pub fn draw(raster: &mut Raster, hour: f32, minute: f32, second: f32) {
    raster.fill(BLACK);
    let (cx, cy) = (raster.width() as i32 / 2, raster.height() as i32 / 2);
    let radius = (raster.width().min(raster.height()) as i32 / 2) - 16;
    if radius < 24 {
        // Too small to be a clock; leave the black page.
        return;
    }

    dial(raster, cx, cy, radius);
    ticks(raster, cx, cy, radius);

    // The smooth positions are what makes an analog clock look right: at 3:30
    // the hour hand sits between 3 and 4, and at 12:59:59 the minute hand is
    // already at the top.
    hand(
        raster,
        cx,
        cy,
        angle(hour % 12.0 + minute / 60.0, 12.0),
        radius * 52 / 100,
        9,
        HAND,
    );
    hand(
        raster,
        cx,
        cy,
        angle(minute + second / 60.0, 60.0),
        radius * 76 / 100,
        7,
        HAND,
    );
    hand(
        raster,
        cx,
        cy,
        angle(second, 60.0),
        radius * 86 / 100,
        2,
        SECOND,
    );

    raster.dab(cx, cy, 9, HAND);
    raster.dab(cx, cy, 3, SECOND);
}

/// The idle times SETTINGS offers, in minutes. The first one after the current
/// value is the next; after the last it wraps around.
pub const IDLE_MINUTES: &[u32] = &[1, 2, 5, 10, 15, 30, 60];

pub fn next_idle_minutes(current: u32) -> u32 {
    IDLE_MINUTES
        .iter()
        .copied()
        .find(|minutes| *minutes > current)
        .unwrap_or(IDLE_MINUTES[0])
}

/// Read the local time and draw it.
pub fn draw_now(raster: &mut Raster) {
    use chrono::Timelike;
    let now = chrono::Local::now();
    draw(
        raster,
        now.hour() as f32,
        now.minute() as f32,
        now.second() as f32,
    );
}

/// Clockwise from twelve o'clock, in radians.
fn angle(value: f32, steps: f32) -> f32 {
    value / steps * TAU
}

/// One hand, from the centre to its tip; `size` is the width in pixels.
fn hand(raster: &mut Raster, cx: i32, cy: i32, angle: f32, length: i32, size: u32, colour: u8) {
    let (dx, dy) = point(angle, length);
    raster.line((cx, cy), (cx + dx, cy + dy), size, colour);
}

/// Offset from the centre for a direction and a length. Zero is up, because
/// that is where twelve o'clock is.
fn point(angle: f32, length: i32) -> (i32, i32) {
    (
        (angle.sin() * length as f32).round() as i32,
        (-angle.cos() * length as f32).round() as i32,
    )
}

/// The ring the hands sit in, sampled densely enough that it has no gaps.
fn dial(raster: &mut Raster, cx: i32, cy: i32, radius: i32) {
    let steps = (radius as f32 * 24.0) as i32;
    for step in 0..steps {
        let (dx, dy) = point(step as f32 / steps as f32 * TAU, radius);
        raster.set(cx + dx, cy + dy, DIAL);
    }
}

/// Sixty small marks near the rim, one long and brighter every five minutes.
fn ticks(raster: &mut Raster, cx: i32, cy: i32, radius: i32) {
    for mark in 0..60 {
        let direction = angle(mark as f32, 60.0);
        let long = mark % 5 == 0;
        let outer = radius - radius / 20;
        let inner = radius - if long { radius / 4 } else { radius / 9 };
        let (ox, oy) = point(direction, outer);
        let (ix, iy) = point(direction, inner);
        raster.line(
            (cx + ox, cy + oy),
            (cx + ix, cy + iy),
            if long { 3 } else { 1 },
            if long { TICK } else { DIAL },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clock(width: u32, height: u32, hour: f32, minute: f32, second: f32) -> Raster {
        let mut raster = Raster::new(width, height);
        draw(&mut raster, hour, minute, second);
        raster
    }

    fn centre(raster: &Raster) -> (i32, i32) {
        (raster.width() as i32 / 2, raster.height() as i32 / 2)
    }

    /// Hands are thick lines, so a pixel-exact check is the wrong test; look for
    /// the colour in a small window around the point.
    fn has_near(raster: &Raster, x: i32, y: i32, wanted: u8) -> bool {
        (-4..=4).any(|dy| (-4..=4).any(|dx| raster.get(x + dx, y + dy) == Some(wanted)))
    }

    #[test]
    fn the_page_is_black_and_the_dial_is_drawn() {
        let raster = clock(240, 240, 3.0, 0.0, 0.0);
        let (cx, cy) = centre(&raster);
        let radius = 240 / 2 - 16;

        assert_eq!(raster.get(cx, 5), Some(BLACK), "the page is black");
        // The ring sits at the radius; check at 45 degrees, where no tick is.
        let (dx, dy) = point(TAU / 8.0, radius);
        assert_eq!(raster.get(cx + dx, cy + dy), Some(DIAL), "the dial");
        // A long tick at twelve is brighter than the small ones.
        let (dx, dy) = point(0.0, radius - radius / 8);
        assert_eq!(raster.get(cx + dx, cy + dy), Some(TICK), "the 12 tick");
    }

    #[test]
    fn the_hands_point_where_the_time_says() {
        let radius = 240 / 2 - 16;

        // 3:15: the minute hand points right, the hour hand a little below it
        // (a quarter of the way towards four), the second hand up.
        let raster = clock(240, 240, 3.0, 15.0, 0.0);
        let (cx, cy) = centre(&raster);
        assert!(
            has_near(&raster, cx + radius * 70 / 100, cy, HAND),
            "minute"
        );
        let (dx, dy) = point(angle(3.25, 12.0), radius * 52 / 100);
        assert!(has_near(&raster, cx + dx, cy + dy, HAND), "hour");

        // 6:30: the minute hand points down, the hour hand down and left.
        let raster = clock(240, 240, 6.0, 30.0, 0.0);
        assert!(
            has_near(&raster, cx, cy + radius * 70 / 100, HAND),
            "minute at half past"
        );
        let (dx, dy) = point(angle(6.5, 12.0), radius * 52 / 100);
        assert!(
            has_near(&raster, cx + dx, cy + dy, HAND),
            "hour at half past"
        );

        // The second hand is red and points at the seconds; at 12:00:15 it is
        // the only thing on the right, because the minute hand is at the top.
        let raster = clock(240, 240, 12.0, 0.0, 15.0);
        assert!(has_near(&raster, cx + radius * 80 / 100, cy, SECOND));
        assert!(!has_near(&raster, cx + radius * 80 / 100, cy, HAND));
        let raster = clock(240, 240, 12.0, 0.0, 0.0);
        assert!(has_near(&raster, cx, cy - radius * 80 / 100, SECOND));
    }

    #[test]
    fn a_tiny_surface_stays_black_instead_of_panicking() {
        let raster = clock(32, 32, 12.0, 0.0, 0.0);
        assert!(raster.pixels().iter().all(|index| *index == BLACK));
    }

    #[test]
    fn the_hands_turn_clockwise_from_twelve() {
        let radius = 100;
        assert_eq!(point(0.0, radius), (0, -radius), "twelve");
        assert_eq!(point(TAU / 4.0, radius), (radius, 0), "three");
        assert_eq!(point(TAU / 2.0, radius), (0, radius), "six");
        assert_eq!(point(TAU * 3.0 / 4.0, radius), (-radius, 0), "nine");
    }
}
