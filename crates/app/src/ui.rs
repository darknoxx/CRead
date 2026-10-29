//! Everything that turns state into a grid of cells.
//!
//! This is deliberately plain: no retained widgets, no layout engine. A DOS
//! program redrew its screen from scratch, and so do we.

use cread_core::{Screen, Style, Theme, Viewport};

/// Live values shown on the boot screen.
pub struct Hud<'a> {
    pub adapter: &'a str,
    pub vp: &'a Viewport,
    pub fps: f32,
    pub status: &'a str,
    /// Text typed on the scratch prompt.
    pub line: &'a str,
    /// Blink phase for the cursor.
    pub cursor_on: bool,
}

pub fn draw(screen: &mut Screen, theme: &Theme, hud: &Hud<'_>) {
    let cols = screen.cols();
    let rows = screen.rows();
    let (fg, bg, dim, accent) = (theme.fg, theme.bg, theme.dim, theme.accent);

    screen.fill(cread_core::Cell::blank(fg, bg));

    // ── title bar ────────────────────────────────────────────────────────────
    screen.fill_row(0, cread_core::Cell::blank(theme.status_fg, theme.status_bg));
    screen.put_str(1, 0, "C R e a d", theme.status_fg, theme.status_bg, Style::Bold);
    let version = "v0.1.0";
    let vx = cols.saturating_sub(version.len() as u32 + 2);
    screen.put_str(vx, 0, version, theme.status_fg, theme.status_bg, Style::Normal);

    // ── body ─────────────────────────────────────────────────────────────────
    let mut y = 2;
    let label = |screen: &mut Screen, y: &mut u32, name: &str, value: &str| {
        if *y >= rows.saturating_sub(4) {
            return;
        }
        screen.put_str(1, *y, name, dim, bg, Style::Normal);
        let x = 1 + 10;
        screen.put_str_ellipsis(x, *y, value, cols.saturating_sub(x + 1), fg, bg, Style::Normal);
        *y += 1;
    };

    label(screen, &mut y, "BIOS", "CRead 0.1.0  \u{00b7}  CRT writing terminal");
    y += 1;
    label(screen, &mut y, "video", hud.adapter);
    label(
        screen,
        &mut y,
        "raster",
        &format!(
            "{} x {}  @ {}x integer scale{}",
            hud.vp.virt_w,
            hud.vp.virt_h,
            hud.vp.scale,
            if hud.vp.is_exact() { "" } else { "  (letterboxed)" }
        ),
    );
    label(
        screen,
        &mut y,
        "grid",
        &format!("{} x {} cells", hud.vp.cols(), hud.vp.rows()),
    );
    label(screen, &mut y, "font", "IBM VGA 8x16  (CP437 + extras)");
    label(screen, &mut y, "webdav", "not configured yet");
    y += 1;
    label(screen, &mut y, "keys", "F1 theme   F2 scanlines   F3 bloom   F4 flicker");
    label(screen, &mut y, " ", "F11 fullscreen   Esc quit");

    // ── scratch prompt (temporary: proves input + font) ──────────────────────
    let prompt_y = rows.saturating_sub(3);
    screen.put_str(1, prompt_y, ">", accent, bg, Style::Bold);
    let text = format!("{}", hud.line);
    screen.put_str(3, prompt_y, &text, fg, bg, Style::Normal);
    let cx = 3 + text.chars().count() as u32;
    if hud.cursor_on && cx < cols {
        screen.put(cx, prompt_y, cread_core::Cell::blank(bg, fg));
    }

    // ── status bar ───────────────────────────────────────────────────────────
    let bar = rows.saturating_sub(1);
    screen.fill_row(bar, cread_core::Cell::blank(theme.status_fg, theme.status_bg));
    let left = format!(" {} ", theme.name);
    screen.put_str(0, bar, &left, theme.status_fg, theme.status_bg, Style::Bold);
    let middle = format!("{:.0} fps  \u{00b7}  {}", hud.fps, hud.status);
    screen.put_str_ellipsis(
        left.len() as u32 + 1,
        bar,
        &middle,
        cols.saturating_sub(left.len() as u32 + 2),
        theme.status_fg,
        theme.status_bg,
        Style::Normal,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use cread_core::ScaleMode;

    fn hud<'a>(vp: &'a Viewport, adapter: &'a str, status: &'a str, line: &'a str) -> Hud<'a> {
        Hud {
            adapter,
            vp,
            fps: 60.0,
            status,
            line,
            cursor_on: true,
        }
    }

    #[test]
    fn renders_at_classic_80x25_without_overflow() {
        let vp = Viewport::compute(1280, 800, ScaleMode::DEFAULT).unwrap();
        let mut screen = Screen::new(vp.cols(), vp.rows());
        draw(&mut screen, &Theme::DOS_BLUE, &hud(&vp, "llvmpipe", "ready", "hello"));
        // Everything we drew must be inside the grid; put_str already clips, so
        // the real check is that key strings survived.
        let row0: String = (0..vp.cols()).map(|x| screen.get(x, 0).unwrap().ch).collect();
        assert!(row0.contains("C R e a d"));
        let text: String = (0..vp.cols()).map(|x| screen.get(x, 22).unwrap().ch).collect();
        assert!(text.contains("hello"));
    }

    #[test]
    fn long_values_are_ellipsised_not_wrapped() {
        let vp = Viewport::compute(640, 400, ScaleMode::DEFAULT).unwrap();
        let mut screen = Screen::new(vp.cols(), vp.rows());
        let long = "x".repeat(500);
        draw(&mut screen, &Theme::DOS_BLUE, &hud(&vp, &long, "s", "l"));
        let rows: Vec<String> = (0..vp.rows())
            .map(|y| (0..vp.cols()).map(|x| screen.get(x, y).unwrap().ch).collect())
            .collect();
        let row = rows
            .iter()
            .find(|r| r.trim_start().starts_with("video"))
            .expect("video row");
        assert!(row.trim_end().ends_with('>'), "got {row:?}");
        let x_count: usize = rows.iter().map(|r| r.matches('x').count()).sum();
        assert!(x_count <= vp.cols() as usize, "value wrapped onto another row");
    }

    #[test]
    fn cursor_blinks() {
        let vp = Viewport::compute(1280, 800, ScaleMode::DEFAULT).unwrap();
        let mut on = Screen::new(vp.cols(), vp.rows());
        let mut off = Screen::new(vp.cols(), vp.rows());
        let mut h = hud(&vp, "a", "b", "xy");
        draw(&mut on, &Theme::DOS_BLUE, &h);
        h.cursor_on = false;
        draw(&mut off, &Theme::DOS_BLUE, &h);
        assert_ne!(on.cells(), off.cells());
    }
}
