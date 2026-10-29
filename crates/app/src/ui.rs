//! Everything that turns state into a grid of cells.
//!
//! No retained widgets, no layout engine: a DOS program redrew its screen from
//! scratch, and so do we.

use cread_core::{
    Cell, Screen, Span, Stats, Style, Theme, VisualRow,
    markdown::{self, Kind},
};

/// Rows reserved above and below the text area.
pub const CHROME_ROWS: u32 = 2;

/// Width of the text column, and the left margin that centres it.
///
/// At 80 columns the page fills the screen like a real DOS editor; on wider
/// rasters the text sits in a ~78 character column so lines stay readable.
pub fn layout(cols: u32) -> (usize, usize) {
    let width = (cols.saturating_sub(2) as usize).clamp(1, 78);
    let margin = (cols as usize).saturating_sub(width) / 2;
    (width, margin)
}

/// Height of the text area for a given screen height.
pub fn text_height(rows: u32) -> usize {
    rows.saturating_sub(CHROME_ROWS).max(1) as usize
}

/// What occupies the bottom line.
pub enum Bottom<'a> {
    /// Write mode: statistics and a hint.
    Status { message: &'a str },
    /// Action mode: the command prompt.
    Prompt {
        input: &'a str,
        hint: &'a str,
        error: &'a str,
    },
}

pub struct DocumentView<'a> {
    pub title: &'a str,
    pub modified: bool,
    /// All logical lines of the document; rows index into this.
    pub lines: &'a [String],
    pub rows: &'a [VisualRow],
    /// Cursor as (row index into `rows`, column offset inside that row).
    pub cursor: Option<(usize, usize)>,
    pub margin: usize,
    pub stats: Stats,
    pub cursor_pos: (usize, usize),
    pub sync: &'a str,
    pub bottom: Bottom<'a>,
    pub theme_name: &'a str,
    /// Fenced-code state at the first visible line.
    pub in_fence: bool,
    pub cursor_on: bool,
}

pub fn draw_document(screen: &mut Screen, theme: &Theme, view: &DocumentView<'_>) {
    let cols = screen.cols();
    let rows = screen.rows();
    screen.fill(Cell::blank(theme.fg, theme.bg));

    draw_top_bar(screen, theme, view);
    draw_text(screen, theme, view);
    match &view.bottom {
        Bottom::Status { message } => draw_status_bar(screen, theme, view, message),
        Bottom::Prompt { input, hint, error } => draw_prompt(screen, theme, input, hint, error),
    }

    debug_assert!(cols > 0 && rows > 0);
}

fn draw_top_bar(screen: &mut Screen, theme: &Theme, view: &DocumentView<'_>) {
    let cols = screen.cols();
    let (fg, bg) = (theme.status_fg, theme.status_bg);
    screen.fill_row(0, Cell::blank(fg, bg));

    let marker = if view.modified {
        " \u{00b7} modified "
    } else {
        ""
    };
    screen.put_str_ellipsis(
        0,
        0,
        &format!(" {}{}", view.title, marker),
        cols,
        fg,
        bg,
        Style::Bold,
    );

    let right = if view.sync.is_empty() {
        format!("{} ", view.theme_name)
    } else {
        format!("{}  {} ", view.sync, view.theme_name)
    };
    let x = cols.saturating_sub(right.chars().count() as u32);
    if x > cols / 2 {
        screen.put_str(x, 0, &right, fg, bg, Style::Normal);
    }
}

fn draw_text(screen: &mut Screen, theme: &Theme, view: &DocumentView<'_>) {
    let first_row = 1u32;
    let margin = view.margin as u32;

    let mut fence = view.in_fence;
    let mut current_line = usize::MAX;
    let mut spans: Vec<Span> = Vec::new();

    for (i, row) in view.rows.iter().enumerate() {
        let y = first_row + i as u32;
        if y >= screen.rows().saturating_sub(1) {
            break;
        }

        if row.line != current_line {
            current_line = row.line;
            let line_text = view.lines.get(row.line).map(String::as_str).unwrap_or("");
            let (parsed, next) = markdown::parse_line(line_text, fence);
            fence = next;
            spans = parsed;
        }

        let mut span_idx = 0;
        for (offset, ch) in row.text.chars().enumerate() {
            let doc_col = row.start + offset;
            while span_idx + 1 < spans.len() && spans[span_idx].end <= doc_col {
                span_idx += 1;
            }
            let kind = spans
                .get(span_idx)
                .filter(|s| doc_col >= s.start && doc_col < s.end)
                .map(|s| s.kind)
                .unwrap_or(Kind::Normal);
            let (fg, bg, style) = style_for(kind, theme);
            screen.put(margin + offset as u32, y, Cell::new(ch, fg, bg, style));
        }
    }

    // Cursor: invert the cell under it, or paint a block at the line end.
    if let Some((row_idx, col)) = view.cursor
        && view.cursor_on
    {
        let y = first_row + row_idx as u32;
        if y < screen.rows().saturating_sub(1) {
            screen.put(margin + col as u32, y, Cell::blank(theme.bg, theme.fg));
        }
    }
}

fn style_for(kind: Kind, theme: &Theme) -> (u8, u8, Style) {
    match kind {
        Kind::Normal => (theme.fg, theme.bg, Style::Normal),
        Kind::Marker => (theme.dim, theme.bg, Style::Normal),
        Kind::Strong => (theme.fg, theme.bg, Style::Bold),
        Kind::Emphasis => (theme.fg, theme.bg, Style::Italic),
        Kind::StrongEmphasis => (theme.fg, theme.bg, Style::Bold),
        Kind::Code => (theme.fg, theme.code_bg, Style::Normal),
        Kind::Link => (theme.accent, theme.bg, Style::Normal),
        Kind::LinkUrl => (theme.dim, theme.bg, Style::Normal),
        Kind::Heading => (theme.accent, theme.bg, Style::Bold),
        Kind::Quote => (theme.dim, theme.bg, Style::Italic),
        Kind::Bullet => (theme.accent, theme.bg, Style::Bold),
        Kind::Rule => (theme.dim, theme.bg, Style::Normal),
    }
}

fn draw_status_bar(screen: &mut Screen, theme: &Theme, view: &DocumentView<'_>, message: &str) {
    let cols = screen.cols();
    let bar = screen.rows().saturating_sub(1);
    let (fg, bg) = (theme.status_fg, theme.status_bg);
    screen.fill_row(bar, Cell::blank(fg, bg));

    let left = format!(
        " {} words  {} chars  Ln {},Col {}",
        view.stats.words,
        view.stats.chars,
        view.cursor_pos.0 + 1,
        view.cursor_pos.1 + 1
    );
    screen.put_str(0, bar, &left, fg, bg, Style::Bold);

    let right = if message.is_empty() {
        "Esc  commands".to_string()
    } else {
        message.to_string()
    };
    let x = cols.saturating_sub(right.chars().count() as u32 + 1);
    if x > left.chars().count() as u32 + 1 {
        screen.put_str(x, bar, &right, fg, bg, Style::Normal);
    }
}

/// The action-mode command line: `> input▌` plus a dimmed hint on the right.
fn draw_prompt(screen: &mut Screen, theme: &Theme, input: &str, hint: &str, error: &str) {
    let cols = screen.cols();
    let bar = screen.rows().saturating_sub(1);
    let (fg, bg) = (theme.status_fg, theme.status_bg);
    screen.fill_row(bar, Cell::blank(fg, bg));

    screen.put_str(0, bar, ">", theme.accent, bg, Style::Bold);
    screen.put(1, bar, Cell::blank(fg, bg));

    // Long command lines scroll, keeping the tail (and the cursor) visible.
    let avail = cols.saturating_sub(3) as usize;
    let count = input.chars().count();
    let shown: String = if count > avail {
        input.chars().skip(count - avail).collect()
    } else {
        input.to_string()
    };

    let end = screen.put_str(3, bar, &shown, fg, bg, Style::Normal);
    screen.put(end.min(cols.saturating_sub(1)), bar, Cell::blank(bg, fg));

    let text = if error.is_empty() { hint } else { error };
    let color = if error.is_empty() {
        theme.status_dim
    } else {
        theme.error
    };
    let used = 3 + shown.chars().count() as u32 + 2;
    let room = cols.saturating_sub(used);
    if (text.chars().count() as u32) < room {
        let x = cols.saturating_sub(text.chars().count() as u32 + 1);
        screen.put_str(x, bar, text, color, bg, Style::Normal);
    }
}

// ── help / about ────────────────────────────────────────────────────────────

/// Full-screen reference. Without hotkeys this is the only way to discover what
/// the program can do, so it is a first-class screen rather than a popup.
pub fn draw_help(screen: &mut Screen, theme: &Theme, about: bool) {
    let cols = screen.cols();
    let rows = screen.rows();
    let (fg, bg) = (theme.fg, theme.bg);
    screen.fill(Cell::blank(fg, bg));

    // title bar
    screen.fill_row(0, Cell::blank(theme.status_fg, theme.status_bg));
    let title = if about {
        " CRead 0.1.0 ".to_string()
    } else {
        " CRead \u{00b7} commands ".to_string()
    };
    screen.put_str(0, 0, &title, theme.status_fg, theme.status_bg, Style::Bold);

    let mut y = 2;
    let last = rows.saturating_sub(2);

    if about {
        for line in [
            "A writing terminal that pretends it is 1987.",
            "",
            "Built with Rust, winit and wgpu. The text is drawn into an",
            "emulated raster and scaled by an integer factor, so scanlines",
            "stay exactly one emulated pixel tall on any display.",
            "",
            "Font: IBM VGA 8x16 from VileR's Ultimate Oldschool PC Font",
            "Pack, CC BY-SA 4.0 (https://int10h.org/oldschool-pc-fonts/).",
            "Nine glyphs CP437 never had are hand-drawn in the source.",
            "",
            "Documents are plain Markdown in ~/Documents/CRead.",
        ] {
            if y >= last {
                break;
            }
            screen.put_str_ellipsis(2, y, line, cols.saturating_sub(4), fg, bg, Style::Normal);
            y += 1;
        }
    } else {
        for spec in cread_core::command::SPECS {
            if y >= last {
                break;
            }
            let name = if spec.argument.is_empty() {
                spec.name.to_string()
            } else {
                format!("{} {}", spec.name, spec.argument)
            };
            screen.put_str(
                2,
                y,
                &format!("{name:<22}"),
                theme.accent,
                bg,
                Style::Normal,
            );
            let aliases = spec.aliases.join(", ");
            screen.put_str(
                25,
                y,
                &format!("{aliases:<15}"),
                theme.dim,
                bg,
                Style::Normal,
            );
            screen.put_str_ellipsis(
                41,
                y,
                spec.summary,
                cols.saturating_sub(42),
                fg,
                bg,
                Style::Normal,
            );
            y += 1;
        }
    }

    let bar = rows.saturating_sub(1);
    screen.fill_row(bar, Cell::blank(theme.status_fg, theme.status_bg));
    screen.put_str(
        0,
        bar,
        " Esc  back to write mode ",
        theme.status_fg,
        theme.status_bg,
        Style::Normal,
    );
    let right = "type any command name to run it ";
    let x = cols.saturating_sub(right.chars().count() as u32 + 1);
    if x > 30 {
        screen.put_str(
            x,
            bar,
            right,
            theme.status_fg,
            theme.status_bg,
            Style::Normal,
        );
    }
}

// ── settings form ───────────────────────────────────────────────────────────

pub struct FormField {
    pub label: String,
    pub value: String,
    /// Render as asterisks (passwords).
    pub secret: bool,
}

pub struct FormView<'a> {
    pub title: &'a str,
    pub fields: &'a [FormField],
    pub focus: usize,
    pub note: &'a str,
    pub hint: &'a str,
}

pub fn draw_form(screen: &mut Screen, theme: &Theme, view: &FormView<'_>) {
    let cols = screen.cols();
    let rows = screen.rows();
    let width = (cols.saturating_sub(6)).clamp(30, 68);
    let height = (view.fields.len() as u32 + 5).clamp(7, rows.saturating_sub(2));
    let x0 = (cols - width) / 2;
    let y0 = (rows - height) / 2;

    for y in y0..y0 + height {
        for x in x0..x0 + width {
            screen.put(x, y, Cell::blank(theme.fg, theme.bg));
        }
    }

    let (b, d) = (theme.accent, theme.dim);
    for x in x0 + 1..x0 + width - 1 {
        screen.put(x, y0, Cell::new('\u{2550}', b, theme.bg, Style::Normal));
        screen.put(
            x,
            y0 + height - 1,
            Cell::new('\u{2550}', b, theme.bg, Style::Normal),
        );
    }
    for y in y0 + 1..y0 + height - 1 {
        screen.put(x0, y, Cell::new('\u{2551}', b, theme.bg, Style::Normal));
        screen.put(
            x0 + width - 1,
            y,
            Cell::new('\u{2551}', b, theme.bg, Style::Normal),
        );
    }
    screen.put(x0, y0, Cell::new('\u{2554}', b, theme.bg, Style::Normal));
    screen.put(
        x0 + width - 1,
        y0,
        Cell::new('\u{2557}', b, theme.bg, Style::Normal),
    );
    screen.put(
        x0,
        y0 + height - 1,
        Cell::new('\u{255a}', b, theme.bg, Style::Normal),
    );
    screen.put(
        x0 + width - 1,
        y0 + height - 1,
        Cell::new('\u{255d}', b, theme.bg, Style::Normal),
    );
    screen.put_str(
        x0 + 2,
        y0,
        &format!(" {} ", view.title),
        b,
        theme.bg,
        Style::Bold,
    );

    for (i, field) in view.fields.iter().enumerate() {
        let y = y0 + 2 + i as u32;
        if y >= y0 + height - 2 {
            break;
        }
        let focused = i == view.focus;
        let label_fg = if focused { theme.accent } else { d };
        screen.put_str_ellipsis(
            x0 + 2,
            y,
            &format!("{:<10}", field.label),
            10,
            label_fg,
            theme.bg,
            if focused { Style::Bold } else { Style::Normal },
        );
        let shown = if field.secret {
            "*".repeat(field.value.chars().count())
        } else {
            field.value.clone()
        };
        let vx = x0 + 13;
        screen.put_str_ellipsis(
            vx,
            y,
            &shown,
            width.saturating_sub(15),
            if focused { theme.fg } else { d },
            theme.bg,
            Style::Normal,
        );
        if focused {
            let cx = vx + shown.chars().count() as u32;
            if cx < x0 + width - 2 {
                screen.put(cx, y, Cell::blank(theme.bg, theme.fg));
            }
        }
    }

    if !view.note.is_empty() {
        screen.put_str_ellipsis(
            x0 + 2,
            y0 + height - 2,
            view.note,
            width.saturating_sub(4),
            d,
            theme.bg,
            Style::Italic,
        );
    }

    let hint = format!(" {} ", view.hint);
    let hx = (x0 + width).saturating_sub(hint.chars().count() as u32 + 2);
    if hx > x0 {
        screen.put_str(hx, y0 + height - 1, &hint, d, theme.bg, Style::Normal);
    }
}

// ── command palette ─────────────────────────────────────────────────────────

pub struct PaletteView<'a> {
    pub title: &'a str,
    pub input: &'a str,
    pub items: &'a [String],
    pub selected: usize,
    pub hint: &'a str,
}

pub fn draw_palette(screen: &mut Screen, theme: &Theme, view: &PaletteView<'_>) {
    let cols = screen.cols();
    let rows = screen.rows();
    let width = (cols.saturating_sub(6)).clamp(24, 62);
    let height = (rows.saturating_sub(4)).clamp(6, 14);
    let x0 = (cols - width) / 2;
    let y0 = (rows - height) / 2;

    // Body
    for y in y0..y0 + height {
        for x in x0..x0 + width {
            screen.put(x, y, Cell::blank(theme.fg, theme.bg));
        }
    }

    // Border in box-drawing characters, like every DOS utility ever.
    let (b, d) = (theme.accent, theme.dim);
    for x in x0 + 1..x0 + width - 1 {
        screen.put(x, y0, Cell::new('\u{2550}', b, theme.bg, Style::Normal));
        screen.put(
            x,
            y0 + height - 1,
            Cell::new('\u{2550}', b, theme.bg, Style::Normal),
        );
    }
    for y in y0 + 1..y0 + height - 1 {
        screen.put(x0, y, Cell::new('\u{2551}', b, theme.bg, Style::Normal));
        screen.put(
            x0 + width - 1,
            y,
            Cell::new('\u{2551}', b, theme.bg, Style::Normal),
        );
    }
    screen.put(x0, y0, Cell::new('\u{2554}', b, theme.bg, Style::Normal));
    screen.put(
        x0 + width - 1,
        y0,
        Cell::new('\u{2557}', b, theme.bg, Style::Normal),
    );
    screen.put(
        x0,
        y0 + height - 1,
        Cell::new('\u{255a}', b, theme.bg, Style::Normal),
    );
    screen.put(
        x0 + width - 1,
        y0 + height - 1,
        Cell::new('\u{255d}', b, theme.bg, Style::Normal),
    );

    // Title, overlaid on the top border.
    let title = format!(" {} ", view.title);
    screen.put_str(x0 + 2, y0, &title, b, theme.bg, Style::Bold);

    // Query line.
    let qy = y0 + 1;
    screen.put_str(x0 + 2, qy, ">", theme.accent, theme.bg, Style::Bold);
    let shown = format!("{}_", view.input);
    screen.put_str_ellipsis(
        x0 + 4,
        qy,
        &shown,
        width.saturating_sub(5),
        theme.fg,
        theme.bg,
        Style::Normal,
    );

    // Item list.
    let list_y = y0 + 3;
    let list_h = height.saturating_sub(5) as usize;
    let offset = view.selected.saturating_sub(list_h.saturating_sub(1));
    for (i, item) in view.items.iter().enumerate().skip(offset).take(list_h) {
        let y = list_y + (i - offset) as u32;
        let selected = i == view.selected;
        let (fg, bg) = if selected {
            (theme.bg, theme.fg)
        } else {
            (theme.fg, theme.bg)
        };
        if selected {
            for x in x0 + 1..x0 + width - 1 {
                screen.put(x, y, Cell::blank(fg, bg));
            }
        }
        screen.put_str_ellipsis(
            x0 + 2,
            y,
            item,
            width.saturating_sub(4),
            fg,
            bg,
            if selected { Style::Bold } else { Style::Normal },
        );
    }

    if view.items.is_empty() {
        screen.put_str(
            x0 + 2,
            list_y,
            "no matches",
            theme.dim,
            theme.bg,
            Style::Italic,
        );
    }

    // Hint on the bottom border.
    let hint = format!(" {} ", view.hint);
    let hx = (x0 + width).saturating_sub(hint.chars().count() as u32 + 2);
    if hx > x0 {
        screen.put_str(hx, y0 + height - 1, &hint, d, theme.bg, Style::Normal);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cread_core::{Editor, ScaleMode, Viewport};

    fn rows_as_text(screen: &Screen) -> String {
        let mut out = String::new();
        for y in 0..screen.rows() {
            for x in 0..screen.cols() {
                out.push(screen.get(x, y).unwrap().ch);
            }
            out.push('\n');
        }
        out
    }

    fn vp(w: u32, h: u32) -> Viewport {
        Viewport::compute(w, h, ScaleMode::DEFAULT).unwrap()
    }

    fn view_from<'a>(
        lines: &'a [String],
        rows: &'a [VisualRow],
        cursor: Option<(usize, usize)>,
        input: (&'a str, &'a str),
    ) -> DocumentView<'a> {
        DocumentView {
            title: "note.md",
            lines,
            modified: true,
            rows,
            cursor,
            margin: 1,
            stats: Stats {
                words: 3,
                chars: 20,
                lines: 2,
            },
            cursor_pos: (0, 3),
            sync: "sync: idle",
            bottom: Bottom::Status { message: input.0 },
            theme_name: input.1,
            in_fence: false,
            cursor_on: true,
        }
    }

    #[test]
    fn renders_a_document_with_chrome_inside_the_grid() {
        let vp = vp(1280, 800);
        let mut ed = Editor::from_text("# Title\n\nSome **bold** prose.");
        ed.move_doc_end();
        let (w, _m) = layout(vp.cols());
        let rows = ed.visible_rows(w, text_height(vp.rows()));
        let cursor = rows
            .iter()
            .position(|r| r.contains(ed.cursor()))
            .map(|i| (i, ed.cursor().col - rows[i].start));

        let mut screen = Screen::new(vp.cols(), vp.rows());
        draw_document(
            &mut screen,
            &Theme::DOS_BLUE,
            &view_from(ed.lines(), &rows, cursor, ("", "dos-blue")),
        );

        let top: String = (0..vp.cols())
            .map(|x| screen.get(x, 0).unwrap().ch)
            .collect();
        assert!(top.contains("note.md"));
        assert!(top.contains("modified"));
        let first: String = (1..40).map(|x| screen.get(x, 1).unwrap().ch).collect();
        assert!(first.contains("Title"));
    }

    #[test]
    fn headings_and_markers_get_different_roles() {
        let vp = vp(1280, 800);
        let mut ed = Editor::from_text("# Title");
        ed.move_doc_end();
        let (w, m) = layout(vp.cols());
        let rows = ed.visible_rows(w, text_height(vp.rows()));
        let mut screen = Screen::new(vp.cols(), vp.rows());
        draw_document(
            &mut screen,
            &Theme::DOS_BLUE,
            &view_from(ed.lines(), &rows, Some((0, 6)), ("", "dos-blue")),
        );
        let hash = screen.get(m as u32, 1).unwrap();
        let letter = screen.get(m as u32 + 4, 1).unwrap();
        assert_eq!(hash.fg, Theme::DOS_BLUE.dim);
        assert_eq!(letter.fg, Theme::DOS_BLUE.accent);
    }

    #[test]
    fn cursor_inverts_the_cell_under_it() {
        let vp = vp(1280, 800);
        let mut ed = Editor::from_text("abc");
        ed.move_end();
        let (w, m) = layout(vp.cols());
        let rows = ed.visible_rows(w, text_height(vp.rows()));
        let mut screen = Screen::new(vp.cols(), vp.rows());
        draw_document(
            &mut screen,
            &Theme::DOS_BLUE,
            &view_from(ed.lines(), &rows, Some((0, 3)), ("", "dos-blue")),
        );
        let cell = screen.get(m as u32 + 3, 1).unwrap();
        assert_eq!(cell.fg, Theme::DOS_BLUE.bg);
        assert_eq!(cell.bg, Theme::DOS_BLUE.fg);
    }

    #[test]
    fn palette_draws_a_bordered_box_with_a_selection() {
        let vp = vp(1280, 800);
        let mut screen = Screen::new(vp.cols(), vp.rows());
        let items = vec!["alpha.md".to_string(), "beta.md".to_string()];
        draw_palette(
            &mut screen,
            &Theme::DOS_BLUE,
            &PaletteView {
                title: "OPEN",
                input: "be",
                items: &items,
                selected: 1,
                hint: "Enter open",
            },
        );
        let all = rows_as_text(&screen);
        assert!(all.contains("OPEN"));
        assert!(all.contains("alpha.md"));
        assert!(all.contains("Enter open"));
        assert!(all.contains('\u{2554}'), "corner missing");
    }

    #[test]
    fn narrow_grids_use_the_full_width_page() {
        let (w, m) = layout(80);
        assert_eq!((w, m), (78, 1));
        let (w, m) = layout(40);
        assert_eq!((w, m), (38, 1));
        let (w, m) = layout(160);
        assert_eq!((w, m), (78, 41));
    }
}
