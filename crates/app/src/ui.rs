//! Everything that turns state into a grid of cells.
//!
//! No retained widgets, no layout engine: a DOS program redrew its screen from
//! scratch, and so do we.

use cread_core::{
    Cell, Screen, Span, Stats, Theme, VisualRow,
    markdown::{self, Kind},
};

/// Rows reserved above and below the text area: a title bar, a blank line, the
/// text, another blank line, then the status or command line.
pub const CHROME_ROWS: u32 = 4;

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
    /// Action or search prompt.
    Prompt {
        /// `>` for commands, `/` for search.
        sigil: &'a str,
        input: &'a str,
        hint: &'a str,
        error: &'a str,
    },
}

pub struct DocumentView<'a> {
    pub title: &'a str,
    /// "saved", "modified", or empty when there is no file yet.
    pub state: &'a str,
    /// Where the document lives, home abbreviated. Shown top right.
    pub location: &'a str,
    /// All logical lines of the document; rows index into this.
    pub lines: &'a [String],
    pub rows: &'a [VisualRow],
    /// Highlighted range, in document positions.
    pub selection: Option<(cread_core::Position, cread_core::Position)>,
    /// Search hits: start position and length in chars.
    pub matches: &'a [(cread_core::Position, usize)],
    /// Index of the hit the cursor is on.
    pub current_match: Option<usize>,
    /// Cursor as (row index into `rows`, column offset inside that row).
    pub cursor: Option<(usize, usize)>,
    pub margin: usize,
    pub stats: Stats,
    pub cursor_pos: (usize, usize),
    pub bottom: Bottom<'a>,
    /// Fenced-code state at the first visible line.
    pub in_fence: bool,
    pub cursor_on: bool,
    /// `♪ track · 0:42` while music plays, drawn in the gap of the status bar.
    pub now: Option<&'a str>,
}

pub fn draw_document(screen: &mut Screen, theme: &Theme, view: &DocumentView<'_>) {
    let cols = screen.cols();
    let rows = screen.rows();
    screen.fill(Cell::blank(theme.fg, theme.bg));

    draw_top_bar(screen, theme, view.title, view.state, view.location);
    draw_text(screen, theme, view);
    match &view.bottom {
        Bottom::Status { message } => draw_status_bar(screen, theme, view, message),
        Bottom::Prompt {
            sigil,
            input,
            hint,
            error,
        } => draw_prompt(screen, theme, sigil, input, hint, error),
    }

    debug_assert!(cols > 0 && rows > 0);
}

fn draw_top_bar(screen: &mut Screen, theme: &Theme, title: &str, state: &str, location: &str) {
    let cols = screen.cols();
    let (fg, bg) = (theme.status_fg, theme.status_bg);
    screen.fill_row(0, Cell::blank(fg, bg));

    // The location is right-aligned, so reserve its space first; otherwise a
    // long path would eat into the file name.
    let location = format!("{location} ");
    let location_width = location.chars().count() as u32;
    let name_width = cols.saturating_sub(location_width + 1);

    let left = if state.is_empty() {
        format!(" {title}")
    } else {
        format!(" {title}  {state}")
    };
    screen.put_str_ellipsis(0, 0, &left, name_width, fg, bg);

    if location_width < cols / 2 {
        let x = cols.saturating_sub(location_width);
        screen.put_str(x, 0, &location, theme.status_dim, bg);
    }
}

fn draw_text(screen: &mut Screen, theme: &Theme, view: &DocumentView<'_>) {
    let first_row = 2u32;
    let margin = view.margin as u32;

    let mut fence = view.in_fence;
    let mut current_line = usize::MAX;
    let mut spans: Vec<Span> = Vec::new();

    for (i, row) in view.rows.iter().enumerate() {
        let y = first_row + i as u32;
        if y >= screen.rows().saturating_sub(2) {
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
            let (fg, bg) = style_for(kind, theme);
            let selected = view.selection.is_some_and(|(start, end)| {
                let pos = cread_core::Position {
                    line: row.line,
                    col: doc_col,
                };
                pos >= start && pos < end
            });
            let pos = cread_core::Position {
                line: row.line,
                col: doc_col,
            };
            let hit = view.matches.iter().position(|(start, len)| {
                *start <= pos && pos.col < start.col + len && start.line == pos.line
            });
            let cell = if selected {
                Cell::new(ch, bg, fg)
            } else if let Some(idx) = hit {
                if view.current_match == Some(idx) {
                    Cell::new(ch, theme.bg, theme.accent)
                } else {
                    Cell::new(ch, theme.accent, theme.code_bg)
                }
            } else {
                Cell::new(ch, fg, bg)
            };
            screen.put(margin + offset as u32, y, cell);
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

/// Colour for a markdown role. Weight and slant are gone: the 1-bit face only
/// gets muddier with synthesised bold and italic, so colour and the dimmed
/// markers carry the structure.
fn style_for(kind: Kind, theme: &Theme) -> (u8, u8) {
    match kind {
        Kind::Normal => (theme.fg, theme.bg),
        Kind::Marker => (theme.dim, theme.bg),
        Kind::Strong => (theme.accent, theme.bg),
        Kind::Emphasis => (theme.accent, theme.bg),
        Kind::StrongEmphasis => (theme.accent, theme.bg),
        Kind::Code => (theme.fg, theme.code_bg),
        Kind::Link => (theme.accent, theme.bg),
        Kind::LinkUrl => (theme.dim, theme.bg),
        Kind::Heading => (theme.accent, theme.bg),
        Kind::Quote => (theme.dim, theme.bg),
        Kind::Bullet => (theme.accent, theme.bg),
        Kind::Rule => (theme.dim, theme.bg),
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
    screen.put_str(0, bar, &left, fg, bg);

    let (mut right, colour) = if message.is_empty() {
        ("Esc switch to action mode".to_string(), theme.status_dim)
    } else {
        (message.to_string(), fg)
    };
    // Narrow windows fall back to a shorter wording rather than dropping it.
    let left_width = left.chars().count() as u32;
    if cols.saturating_sub(right.chars().count() as u32 + 1) <= left_width + 1 {
        right = if message.is_empty() {
            "Esc: action mode".to_string()
        } else {
            right
        };
    }
    let right_width = right.chars().count() as u32;
    let x = cols.saturating_sub(right_width + 1);
    let music = view.now;
    let music_width = music.map(|m| m.chars().count() as u32).unwrap_or(0);
    let music_x = left_width + 2;
    let music_room = x.saturating_sub(music_x + 2);

    if music_width > 0 && music_room < music_width {
        // Not enough room for both: the music takes the right slot and the
        // hint gives way, because a playing track is the rarer information.
        let mx = cols.saturating_sub(music_width + 1);
        if mx > left_width + 1 {
            screen.put_str(mx, bar, music.unwrap_or_default(), theme.status_dim, bg);
        } else if x > left_width + 1 {
            screen.put_str(x, bar, &right, colour, bg);
        }
    } else {
        if x > left_width + 1 {
            screen.put_str(x, bar, &right, colour, bg);
        }
        if let Some(now) = music {
            screen.put_str_ellipsis(music_x, bar, now, music_room, theme.status_dim, bg);
        }
    }
}

/// The action-mode command line: `> input▌` plus a dimmed hint on the right.
fn draw_prompt(
    screen: &mut Screen,
    theme: &Theme,
    sigil: &str,
    input: &str,
    hint: &str,
    error: &str,
) {
    let cols = screen.cols();
    let bar = screen.rows().saturating_sub(1);
    let (fg, bg) = (theme.status_fg, theme.status_bg);
    screen.fill_row(bar, Cell::blank(fg, bg));

    screen.put_str(0, bar, sigil, theme.accent_on_status(), bg);
    screen.put(1, bar, Cell::blank(fg, bg));

    // Long command lines scroll, keeping the tail (and the cursor) visible.
    let avail = cols.saturating_sub(3) as usize;
    let count = input.chars().count();
    let shown: String = if count > avail {
        input.chars().skip(count - avail).collect()
    } else {
        input.to_string()
    };

    let end = screen.put_str(2, bar, &shown, fg, bg);
    screen.put(end.min(cols.saturating_sub(1)), bar, Cell::blank(bg, fg));

    let text = if !error.is_empty() {
        error
    } else if !hint.is_empty() {
        hint
    } else {
        "Esc switch to write mode"
    };
    let color = if error.is_empty() {
        theme.status_dim
    } else {
        theme.error
    };
    let used = 2 + shown.chars().count() as u32 + 2;
    let room = cols.saturating_sub(used);
    if (text.chars().count() as u32) < room {
        let x = cols.saturating_sub(text.chars().count() as u32 + 1);
        screen.put_str(x, bar, text, color, bg);
    }
}

// ── Inspiro front door ──────────────────────────────────────────────────────

/// Inspiro's own home screen: the tagline and the prompt, nothing else.
pub fn draw_inspiro(
    screen: &mut Screen,
    theme: &Theme,
    title_row: u32,
    input: &str,
    hint: &str,
    error: &str,
) {
    draw_front_door(
        screen,
        theme,
        title_row,
        "where your ideas have room to grow",
        "type new or open to start",
        input,
        hint,
        error,
    );
}

/// A program's front door: logo (blitted by the caller), tagline, prompt.
#[allow(clippy::too_many_arguments)]
pub fn draw_front_door(
    screen: &mut Screen,
    theme: &Theme,
    title_row: u32,
    tagline: &str,
    call: &str,
    input: &str,
    hint: &str,
    error: &str,
) {
    screen.fill(Cell::blank(theme.fg, theme.bg));
    put_centered(screen, title_row, tagline, theme.status_dim, theme.bg);
    put_centered(screen, title_row + 2, call, theme.accent, theme.bg);
    draw_prompt(screen, theme, ">", input, hint, error);
}

// ── program system line ─────────────────────────────────────────────────────

/// The bottom row while a program runs: its name on the left, its state on the
/// right, or the command prompt when it is open.
pub fn draw_program_line(
    screen: &mut Screen,
    theme: &Theme,
    title: &str,
    status: &str,
    now: Option<&str>,
    prompt: Option<(&str, &str, &str)>,
) {
    if let Some((input, hint, error)) = prompt {
        draw_prompt(screen, theme, ">", input, hint, error);
        return;
    }
    let cols = screen.cols();
    let bar = screen.rows().saturating_sub(1);
    let (fg, bg) = (theme.status_fg, theme.status_bg);
    screen.fill_row(bar, Cell::blank(fg, bg));
    screen.put_str(1, bar, title, theme.accent_on_status(), bg);
    let width = status.chars().count() as u32;
    let x = cols.saturating_sub(width + 1);
    screen.put_str(x, bar, status, theme.status_dim, bg);

    if let Some(now) = now {
        let start = title.chars().count() as u32 + 3;
        let room = x.saturating_sub(start + 2);
        if room >= 10 {
            screen.put_str_ellipsis(start, bar, now, room, theme.status_dim, bg);
        }
    }
}

// ── software ────────────────────────────────────────────────────────────────

/// One row of a menu screen: a name, what it is, and where it stands.
pub struct MenuItem {
    pub name: String,
    pub detail: String,
    pub value: String,
}

/// A boxed list with a value on the right: SOFTWARE and SETTINGS are the same
/// screen with different rows, and that is the point.
/// A double-line box: the shape every dialog in CRead uses.
///
/// Border only. A menu lets the page show through; a dialog that covers a
/// picture asks for [`frame_filled`] instead.
pub fn frame(screen: &mut Screen, theme: &Theme, x0: u32, y0: u32, width: u32, height: u32) {
    let (b, bg) = (theme.accent, theme.bg);
    for x in x0 + 1..x0 + width.saturating_sub(1) {
        screen.put(x, y0, Cell::new('\u{2550}', b, bg));
        screen.put(
            x,
            y0 + height.saturating_sub(1),
            Cell::new('\u{2550}', b, bg),
        );
    }
    for y in y0 + 1..y0 + height.saturating_sub(1) {
        screen.put(x0, y, Cell::new('\u{2551}', b, bg));
        screen.put(
            x0 + width.saturating_sub(1),
            y,
            Cell::new('\u{2551}', b, bg),
        );
    }
    screen.put(x0, y0, Cell::new('\u{2554}', b, bg));
    screen.put(
        x0 + width.saturating_sub(1),
        y0,
        Cell::new('\u{2557}', b, bg),
    );
    screen.put(
        x0,
        y0 + height.saturating_sub(1),
        Cell::new('\u{255a}', b, bg),
    );
    screen.put(
        x0 + width.saturating_sub(1),
        y0 + height.saturating_sub(1),
        Cell::new('\u{255d}', b, bg),
    );
}

/// The same box with the page filled first: it hides whatever is underneath.
pub fn frame_filled(screen: &mut Screen, theme: &Theme, x0: u32, y0: u32, width: u32, height: u32) {
    for y in y0..y0 + height {
        for x in x0..x0 + width {
            screen.put(x, y, Cell::blank(theme.fg, theme.bg));
        }
    }
    frame(screen, theme, x0, y0, width, height);
}

pub fn draw_menu(
    screen: &mut Screen,
    theme: &Theme,
    title: &str,
    hint: &str,
    items: &[MenuItem],
    selected: usize,
) {
    let cols = screen.cols();
    let rows = screen.rows();
    screen.fill(Cell::blank(theme.fg, theme.bg));

    screen.fill_row(0, Cell::blank(theme.status_fg, theme.status_bg));
    screen.put_str(
        0,
        0,
        &format!(" {title} "),
        theme.status_fg,
        theme.status_bg,
    );
    let right = format!("{hint} ");
    let x = cols.saturating_sub(right.chars().count() as u32);
    if x > 20 {
        screen.put_str(x, 0, &right, theme.status_fg, theme.status_bg);
    }

    let inner = cols.saturating_sub(6).min(66);
    let x0 = (cols - inner) / 2;
    let bg = theme.bg;
    let first = 3;
    let height = items.len() as u32 + 2;
    frame(screen, theme, x0, first - 1, inner, height + 1);

    for (index, item) in items.iter().enumerate() {
        let y = first + index as u32;
        let selected = index == selected;
        let (fg, row_bg) = if selected {
            (theme.bg, theme.fg)
        } else {
            (theme.fg, bg)
        };
        if selected {
            for px in x0 + 1..x0 + inner - 1 {
                screen.put(px, y, Cell::blank(theme.bg, theme.fg));
            }
        }
        let mark = if selected { "\u{25ba}" } else { " " };
        screen.put_str(x0 + 2, y, mark, theme.accent, row_bg);
        screen.put_str_ellipsis(x0 + 4, y, &item.name, 12, fg, row_bg);
        screen.put_str_ellipsis(
            x0 + 17,
            y,
            &item.detail,
            inner.saturating_sub(34),
            theme.dim,
            row_bg,
        );
        let value_x = x0 + inner - 2 - item.value.chars().count() as u32;
        screen.put_str(value_x, y, &item.value, fg, row_bg);
    }

    let bar = rows.saturating_sub(1);
    screen.fill_row(bar, Cell::blank(theme.status_fg, theme.status_bg));
    screen.put_str(0, bar, " Esc  back ", theme.status_fg, theme.status_bg);
    let right = "Enter open ";
    let hx = cols.saturating_sub(right.chars().count() as u32 + 1);
    if hx > 16 {
        screen.put_str(hx, bar, right, theme.status_fg, theme.status_bg);
    }
}

// ── riff ────────────────────────────────────────────────────────────────────

pub struct RiffView<'a> {
    pub title: &'a str,
    pub album: &'a str,
    /// 0 to 1 through the track.
    pub progress: f32,
    pub elapsed: &'a str,
    pub total: &'a str,
    /// "3 of 12", or empty.
    pub counter: &'a str,
    pub playing: bool,
    pub paused: bool,
    pub volume: u8,
    /// Playlist window: (title, is current).
    pub lines: &'a [(String, bool)],
    /// Toast line: nothing loaded, or why the last action failed.
    pub note: &'a str,
}

/// The player: title block, tape counter, transport, playlist.
pub fn draw_riff(screen: &mut Screen, theme: &Theme, view: &RiffView<'_>) {
    let cols = screen.cols();
    let rows = screen.rows();
    screen.fill(Cell::blank(theme.fg, theme.bg));

    // Banner: spaced-out letters, because this is 1987.
    screen.put_str(2, 1, "R I F F", theme.accent, theme.bg);
    let right = "a Soundtrack for your life.";
    let x = cols.saturating_sub(right.chars().count() as u32 + 2);
    if x > 10 {
        screen.put_str(x, 1, right, theme.status_dim, theme.bg);
    }

    let inner = cols.saturating_sub(6).min(62);
    let x0 = (cols - inner) / 2;
    let bg = theme.bg;
    // Double-line frame around the track.
    frame(screen, theme, x0, 3, inner, 6);

    if view.title.is_empty() {
        put_centered_at(screen, 5, view.note, theme.status_dim, theme.bg, x0, inner);
    } else {
        screen.put_str_ellipsis(x0 + 2, 4, "\u{266b}", 1, theme.accent, bg);
        screen.put_str_ellipsis(x0 + 4, 4, view.title, inner - 6, theme.fg, bg);
        let sub = if view.album.is_empty() {
            view.counter.to_string()
        } else {
            format!("{}  \u{00b7}  {}", view.album, view.counter)
        };
        screen.put_str_ellipsis(x0 + 4, 5, &sub, inner - 6, theme.status_dim, bg);

        // Tape counter: elapsed, bar, total.
        let bar_w = inner.saturating_sub(16);
        let filled = ((view.progress.clamp(0.0, 1.0) * bar_w as f32).round() as u32).min(bar_w);
        screen.put_str(x0 + 2, 7, view.elapsed, theme.status_dim, bg);
        for i in 0..bar_w {
            let (ch, fg) = if i < filled {
                ('\u{2588}', theme.accent)
            } else {
                ('\u{2591}', theme.dim)
            };
            screen.put(x0 + 8 + i, 7, Cell::new(ch, fg, bg));
        }
        screen.put_str(x0 + 9 + bar_w, 7, view.total, theme.status_dim, bg);
    }

    // Transport.
    let state = if view.playing && !view.paused {
        "\u{25ba}"
    } else {
        "\u{2551}"
    };
    let transport = format!("\u{25c4}\u{25c4}    {state}    \u{25a0}    \u{25ba}\u{25ba}");
    put_centered_at(screen, 10, &transport, theme.fg, bg, 0, cols);

    // Volume, right aligned on the same line.
    let vol_w = 10u32;
    let lvl = (view.volume as u32 * vol_w).div_ceil(100).min(vol_w);
    let vx = cols.saturating_sub(vol_w + 8);
    if vx > 30 {
        screen.put_str(vx, 10, "vol", theme.status_dim, bg);
        for i in 0..vol_w {
            let (ch, fg) = if i < lvl {
                ('\u{2588}', theme.accent)
            } else {
                ('\u{2591}', theme.dim)
            };
            screen.put(vx + 5 + i, 10, Cell::new(ch, fg, bg));
        }
    }

    // Playlist. The bottom row belongs to the shell's action line.
    let start = 13;
    for (i, (title, current)) in view.lines.iter().enumerate() {
        let y = start + i as u32;
        if y + 1 >= rows {
            break;
        }
        let (fg, bgc) = if *current {
            (theme.bg, theme.fg)
        } else {
            (theme.fg, theme.bg)
        };
        if *current {
            for x in 1..cols.saturating_sub(1) {
                screen.put(x, y, Cell::blank(theme.bg, theme.fg));
            }
        }
        let mark = if *current { "\u{25ba}" } else { " " };
        screen.put_str(3, y, mark, theme.accent, bgc);
        screen.put_str_ellipsis(5, y, title, cols.saturating_sub(8), fg, bgc);
    }
}

/// A string centred inside a horizontal slice.
fn put_centered_at(screen: &mut Screen, y: u32, text: &str, fg: u8, bg: u8, x0: u32, width: u32) {
    let len = text.chars().count() as u32;
    if len > width {
        screen.put_str_ellipsis(x0, y, text, width, fg, bg);
        return;
    }
    screen.put_str(x0 + (width - len) / 2, y, text, fg, bg);
}

pub struct ExplorerView<'a> {
    pub dir: &'a str,
    pub entries: &'a [(String, bool)],
    pub selected: usize,
}

/// The music folder, one level at a time.
pub fn draw_explorer(screen: &mut Screen, theme: &Theme, view: &ExplorerView<'_>) {
    let cols = screen.cols();
    let rows = screen.rows();
    screen.fill(Cell::blank(theme.fg, theme.bg));

    screen.fill_row(0, Cell::blank(theme.status_fg, theme.status_bg));
    screen.put_str(
        0,
        0,
        " RIFF \u{00b7} insert music ",
        theme.status_fg,
        theme.status_bg,
    );
    screen.put_str_ellipsis(
        2,
        2,
        view.dir,
        cols.saturating_sub(4),
        theme.accent,
        theme.bg,
    );

    let first = 4;
    let last = rows.saturating_sub(2);
    // Scroll in the same step the other browser uses: the highlight is always
    // visible, and the list only moves when it has to.
    let visible = last.saturating_sub(first) as usize;
    let top = view.selected.saturating_sub(visible.saturating_sub(1));
    for (i, (name, folder)) in view.entries.iter().enumerate().skip(top) {
        let y = first + (i - top) as u32;
        if y >= last {
            break;
        }
        let selected = i == view.selected;
        let (fg, bg) = if selected {
            (theme.bg, theme.fg)
        } else if *folder {
            (theme.accent, theme.bg)
        } else {
            (theme.fg, theme.bg)
        };
        if selected {
            for x in 1..cols.saturating_sub(1) {
                screen.put(x, y, Cell::blank(theme.bg, theme.fg));
            }
        }
        let mark = if *folder { "\u{25ba}" } else { " " };
        screen.put_str(2, y, mark, fg, bg);
        screen.put_str_ellipsis(4, y, name, cols.saturating_sub(6), fg, bg);
    }

    if view.entries.is_empty() {
        screen.put_str(
            4,
            first,
            "no folders or playable files here",
            theme.status_dim,
            theme.bg,
        );
    }

    let bar = rows.saturating_sub(1);
    screen.fill_row(bar, Cell::blank(theme.status_fg, theme.status_bg));
    screen.put_str(0, bar, " Esc back ", theme.status_fg, theme.status_bg);
    let right = "Enter open or play   i insert folder ";
    let x = cols.saturating_sub(right.chars().count() as u32);
    screen.put_str(x, bar, right, theme.status_fg, theme.status_bg);
}

// ── loading ─────────────────────────────────────────────────────────────────

/// The fake loading bar: a boxed meter, because an OS loads things.
pub fn draw_loading(
    screen: &mut Screen,
    theme: &Theme,
    label: &str,
    progress: f32,
    blink: bool,
    hint: &str,
) {
    let cols = screen.cols();
    let rows = screen.rows();
    screen.fill(Cell::blank(theme.fg, theme.bg));

    let width = cols.saturating_sub(12).clamp(24, 52);
    let height = 7u32.min(rows.saturating_sub(4)).max(5);
    let x0 = (cols - width) / 2;
    let y0 = (rows - height) / 2;

    let d = theme.dim;
    frame_filled(screen, theme, x0, y0, width, height);

    screen.put_str_ellipsis(
        x0 + 2,
        y0 + 2,
        label,
        width.saturating_sub(4),
        theme.fg,
        theme.bg,
    );

    // The meter itself.
    let bar_width = width.saturating_sub(10);
    let filled = ((progress.clamp(0.0, 1.0) * bar_width as f32).round() as u32).min(bar_width);
    for i in 0..bar_width {
        let (ch, fg) = if i < filled {
            ('\u{2588}', theme.accent)
        } else {
            ('\u{2591}', d)
        };
        screen.put(x0 + 2 + i, y0 + 3, Cell::new(ch, fg, theme.bg));
    }
    let percent = format!("{:>3}%", (progress.clamp(0.0, 1.0) * 100.0).round() as u32);
    screen.put_str(
        x0 + width.saturating_sub(6),
        y0 + 3,
        &percent,
        theme.fg,
        theme.bg,
    );

    // The drive light, in text: a block that comes and goes while it thinks.
    screen.put(
        x0 + width.saturating_sub(2),
        y0 + 3,
        Cell::new(if blink { '\u{2588}' } else { ' ' }, theme.accent, theme.bg),
    );

    put_centered(screen, y0 + height + 1, hint, theme.status_dim, theme.bg);
}

// ── reader (view mode) ──────────────────────────────────────────────────────

pub struct ReaderView<'a> {
    pub title: &'a str,
    pub state: &'a str,
    pub location: &'a str,
    pub rows: &'a [markdown::Rendered],
    /// Index of the first visible row.
    pub top: usize,
    pub words: usize,
    /// What leaving this view does.
    pub hint: &'a str,
}

/// The document with the markup taken out: what the thing actually says.
pub fn draw_reader(screen: &mut Screen, theme: &Theme, view: &ReaderView<'_>) {
    let cols = screen.cols();
    let rows = screen.rows();
    screen.fill(Cell::blank(theme.fg, theme.bg));

    draw_top_bar(screen, theme, view.title, view.state, view.location);

    let (_, margin) = layout(cols);
    let height = text_height(rows);
    let first_row = 2u32;
    let margin = margin as u32;
    let top = view.top.min(view.rows.len().saturating_sub(1));

    for (i, row) in view.rows.iter().skip(top).take(height).enumerate() {
        let y = first_row + i as u32;
        if y >= rows.saturating_sub(2) {
            break;
        }
        let mut span_idx = 0;
        for (offset, ch) in row.text.chars().enumerate() {
            while span_idx + 1 < row.spans.len() && row.spans[span_idx].end <= offset {
                span_idx += 1;
            }
            let kind = row
                .spans
                .get(span_idx)
                .filter(|s| offset >= s.start && offset < s.end)
                .map(|s| s.kind)
                .unwrap_or(Kind::Normal);
            let (fg, bg) = style_for(kind, theme);
            screen.put(margin + offset as u32, y, Cell::new(ch, fg, bg));
        }
    }

    // Status line: where you are, and how to get out.
    let bar = rows.saturating_sub(1);
    let (fg, bg) = (theme.status_fg, theme.status_bg);
    screen.fill_row(bar, Cell::blank(fg, bg));
    let left = format!(
        " view  \u{00b7}  {} words  \u{00b7}  {}/{} ",
        view.words,
        top + 1,
        view.rows.len().max(1)
    );
    screen.put_str(0, bar, &left, fg, bg);

    let right = view.hint;
    let x = cols.saturating_sub(right.chars().count() as u32 + 1);
    if x > left.chars().count() as u32 + 1 {
        screen.put_str(x, bar, right, theme.status_dim, bg);
    }
}

// ── welcome ─────────────────────────────────────────────────────────────────

/// The start screen: a centred greeting with a command prompt at the bottom.
pub struct BootItem {
    pub label: String,
    pub detail: String,
    /// A horizontal rule instead of a row: the shelf which separates the vault
    /// from the modules. Never selectable, never counted for the block width.
    pub rule: bool,
    /// Drawn without a drive letter: SOFTWARE, THE VAULT and SETTINGS.
    pub bare: bool,
}

impl BootItem {
    pub fn row(label: impl Into<String>, detail: impl Into<String>, bare: bool) -> Self {
        Self {
            label: label.into(),
            detail: detail.into(),
            rule: false,
            bare,
        }
    }

    pub fn rule() -> Self {
        Self {
            label: String::new(),
            detail: String::new(),
            rule: true,
            bare: false,
        }
    }
}

pub struct WelcomeView<'a> {
    pub show_prompt: bool,
    /// Row of the greeting; the drive list follows two rows below.
    pub title_row: u32,
    pub items: &'a [BootItem],
    pub selected: usize,
    /// Shown dimmed under the list.
    pub note: &'a str,
    pub input: &'a str,
    pub hint: &'a str,
    pub error: &'a str,
}

/// The boot screen. Same shape as the original welcome screen — centred logo,
/// centred greeting — with the drives in place of the old hint lines.
pub fn draw_welcome(screen: &mut Screen, theme: &Theme, view: &WelcomeView<'_>) {
    let cols = screen.cols();
    let rows = screen.rows();
    screen.fill(Cell::blank(theme.fg, theme.bg));

    put_centered(
        screen,
        view.title_row,
        "Welcome to CRead",
        theme.accent,
        theme.bg,
    );

    // The list is a centred block so it sits under the logo like the old
    // greeting did, instead of hanging off the left edge.
    const LABEL_W: u32 = 14;
    let widest = view
        .items
        .iter()
        .filter(|item| !item.rule)
        .map(|item| LABEL_W + item.detail.chars().count() as u32)
        .max()
        .unwrap_or(0);
    let block_w = widest.min(cols.saturating_sub(6));
    let x0 = (cols.saturating_sub(block_w)) / 2;

    for (i, item) in view.items.iter().enumerate() {
        let y = view.title_row + 2 + i as u32;
        if y >= rows.saturating_sub(3) {
            break;
        }
        if item.rule {
            // A rule across the drive block, matching the highlight's width so
            // the shelf lines up with the rows above and below it.
            let start = x0.saturating_sub(1);
            let len = (block_w + 2).min(cols.saturating_sub(start));
            screen.put_str(
                start,
                y,
                &"\u{2500}".repeat(len as usize),
                theme.dim,
                theme.bg,
            );
            continue;
        }
        let selected = i == view.selected;
        if selected {
            for x in x0.saturating_sub(1)..(x0 + block_w + 1).min(cols) {
                screen.put(x, y, Cell::blank(theme.bg, theme.fg));
            }
        }
        let (x, label) = if item.bare {
            // `A: NAME` is nine cells of drive before the text; a row without a
            // letter lines up with the names instead.
            (x0, item.label.as_str())
        } else {
            (x0, item.label.as_str())
        };
        let bar_bg = if selected { theme.fg } else { theme.bg };
        let (label_fg, detail_fg) = if selected {
            (theme.bg, theme.bg)
        } else {
            (theme.accent, theme.dim)
        };
        screen.put_str(
            x,
            y,
            &format!("{:<width$}", label, width = (x0 + LABEL_W - x) as usize),
            label_fg,
            bar_bg,
        );
        screen.put_str_ellipsis(
            x0 + LABEL_W,
            y,
            &item.detail,
            block_w.saturating_sub(LABEL_W),
            detail_fg,
            bar_bg,
        );
    }

    if !view.note.is_empty() {
        put_centered(
            screen,
            (view.title_row + 2 + view.items.len() as u32 + 1).min(rows.saturating_sub(3)),
            view.note,
            theme.status_dim,
            theme.bg,
        );
    }

    if view.show_prompt {
        draw_prompt(screen, theme, ">", view.input, view.hint, view.error);
    }
}

fn put_centered(screen: &mut Screen, y: u32, text: &str, fg: u8, bg: u8) {
    let width = text.chars().count() as u32;
    let x = screen.cols().saturating_sub(width) / 2;
    screen.put_str(x, y, text, fg, bg);
}

// ── help / about ────────────────────────────────────────────────────────────

/// Version shown on the about screen and in the help title.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The about screen, one paragraph per entry. Wrapped at draw time, so a narrow
/// grid gets more lines instead of chopped sentences.
const ABOUT: &[&str] = &[
    "CReadOS is a minimal fake operating system, that helps you to stay focused on what really counts.",
    "Built in Rust, winit and wgpu.",
    "Font: IBM VGA 8x16 from VileR's Ultimate Oldschool PC Font Pack, CC BY-SA 4.0 (https://int10h.org/oldschool-pc-fonts/).",
    "Pocketboi runs on mizu-core 1.3.0 (MIT), a Game Boy and Game Boy Color emulator in Rust, vendored with five small patches: https://github.com/Amjad50/mizu",
];

/// The signature at the bottom of the about screen.
const ABOUT_SIGNATURE: &str = "- darknoxx 2026";

/// Greedy word wrap. A word longer than the line gets a line of its own and is
/// clipped by the renderer, which is better than a mid-URL break.
fn wrap_line(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let extra = if current.is_empty() {
            word.len()
        } else {
            word.len() + 1
        };
        if !current.is_empty() && current.len() + extra > width {
            lines.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

/// Full-screen reference. Without hotkeys this is the only way to discover what
/// the program can do, so it is a first-class screen rather than a popup.
pub fn draw_help(
    screen: &mut Screen,
    theme: &Theme,
    about: bool,
    specs: &[&cread_core::command::Spec],
    selected: usize,
) {
    let cols = screen.cols();
    let rows = screen.rows();
    let (fg, bg) = (theme.fg, theme.bg);
    screen.fill(Cell::blank(fg, bg));

    // title bar
    screen.fill_row(0, Cell::blank(theme.status_fg, theme.status_bg));
    let title = if about {
        format!(" CRead {} ", VERSION)
    } else {
        " CRead \u{00b7} commands ".to_string()
    };
    screen.put_str(0, 0, &title, theme.status_fg, theme.status_bg);

    let mut y = 2;
    let last = rows.saturating_sub(2);

    if about {
        let width = cols.saturating_sub(4) as usize;
        for (paragraph, text) in ABOUT.iter().enumerate() {
            if paragraph > 0 && y < last {
                y += 1;
            }
            for line in wrap_line(text, width) {
                if y >= last {
                    break;
                }
                screen.put_str_ellipsis(2, y, &line, cols.saturating_sub(4), fg, bg);
                y += 1;
            }
        }

        // The signature sits on the last content row, right-aligned.
        let signature = ABOUT_SIGNATURE;
        let x = cols.saturating_sub(2 + signature.chars().count() as u32);
        screen.put_str(x.max(2), last, signature, theme.accent, bg);
    } else {
        // Two columns, balanced: a help screen that silently drops its last
        // rows is worse than none.
        let rows_free = (last - y) as usize;
        let per_col = specs.len().div_ceil(2).min(rows_free).max(1);
        for (i, spec) in specs.iter().enumerate() {
            let (col, row) = (i / per_col, i % per_col);
            if col > 1 {
                break;
            }
            let x = if col == 0 { 2 } else { cols / 2 };
            let line_y = y + row as u32;
            let name = if spec.argument.is_empty() {
                spec.name.to_string()
            } else {
                format!("{} {}", spec.name, spec.argument)
            };
            let (name_fg, alias_fg) = if i == selected {
                (theme.bg, theme.bg)
            } else {
                (theme.accent, theme.dim)
            };
            if i == selected {
                for px in x.saturating_sub(1)..cols / 2 * (col as u32 + 1) {
                    screen.put(px, line_y, Cell::blank(theme.bg, theme.fg));
                }
            }
            let row_bg = if i == selected { theme.fg } else { theme.bg };
            // 22 cells hold even `scanlines [on|off] -r`.
            screen.put_str_ellipsis(x, line_y, &name, 22, name_fg, row_bg);
            let aliases = spec.aliases.join(", ");
            screen.put_str_ellipsis(x + 22, line_y, &aliases, 14, alias_fg, row_bg);
        }
    }

    let bar = rows.saturating_sub(1);
    screen.fill_row(bar, Cell::blank(theme.status_fg, theme.status_bg));
    screen.put_str(0, bar, " Esc  back ", theme.status_fg, theme.status_bg);
    let right = if about {
        "type any command name to run it ".to_string()
    } else {
        match specs.get(selected) {
            Some(spec) => format!("{} ", spec.summary),
            None => String::new(),
        }
    };
    let x = cols.saturating_sub(right.chars().count() as u32 + 1);
    if x > 30 {
        screen.put_str(x, bar, &right, theme.status_fg, theme.status_bg);
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

    let (b, d) = (theme.accent, theme.dim);
    frame_filled(screen, theme, x0, y0, width, height);
    screen.put_str(x0 + 2, y0, &format!(" {} ", view.title), b, theme.bg);

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
        );
    }

    let hint = format!(" {} ", view.hint);
    let hx = (x0 + width).saturating_sub(hint.chars().count() as u32 + 2);
    if hx > x0 {
        screen.put_str(hx, y0 + height - 1, &hint, d, theme.bg);
    }
}

// ── vault browser ──────────────────────────────────────────────────────────

/// A yes/no question in a box: the last word before the machine goes off.
pub fn draw_confirm(screen: &mut Screen, theme: &Theme, title: &str, message: &str, yes: bool) {
    let cols = screen.cols();
    let rows = screen.rows();
    let width = cols.saturating_sub(12).clamp(30, 54);
    let height = 7;
    let x0 = (cols - width) / 2;
    let y0 = (rows - height) / 2;

    for y in y0..y0 + height {
        for x in x0..x0 + width {
            screen.put(x, y, Cell::blank(theme.fg, theme.bg));
        }
    }
    let (b, d) = (theme.accent, theme.dim);
    for x in x0 + 1..x0 + width - 1 {
        screen.put(x, y0, Cell::new('\u{2550}', b, theme.bg));
        screen.put(x, y0 + height - 1, Cell::new('\u{2550}', b, theme.bg));
    }
    for y in y0 + 1..y0 + height - 1 {
        screen.put(x0, y, Cell::new('\u{2551}', b, theme.bg));
        screen.put(x0 + width - 1, y, Cell::new('\u{2551}', b, theme.bg));
    }
    screen.put(x0, y0, Cell::new('\u{2554}', b, theme.bg));
    screen.put(x0 + width - 1, y0, Cell::new('\u{2557}', b, theme.bg));
    screen.put(x0, y0 + height - 1, Cell::new('\u{255a}', b, theme.bg));
    screen.put(
        x0 + width - 1,
        y0 + height - 1,
        Cell::new('\u{255d}', b, theme.bg),
    );

    let label = format!(" {title} ");
    screen.put_str(x0 + 2, y0, &label, b, theme.bg);
    screen.put_str_ellipsis(
        x0 + 2,
        y0 + 2,
        message,
        width.saturating_sub(4),
        theme.fg,
        theme.bg,
    );

    // Two buttons, the focused one inverted, like a selected row.
    let buttons = ["[ Yes ]", "[ No ]"];
    let inner = width.saturating_sub(2) as usize;
    let total: usize = buttons.iter().map(|b| b.chars().count()).sum::<usize>() + 2;
    let mut x = x0 + 1 + ((inner.saturating_sub(total)) / 2) as u32;
    for (index, button) in buttons.iter().enumerate() {
        let focused = (index == 0) == yes;
        if focused {
            for (offset, ch) in button.chars().enumerate() {
                screen.put(x + offset as u32, y0 + 4, Cell::new(ch, theme.bg, theme.fg));
            }
        } else {
            screen.put_str(x, y0 + 4, button, d, theme.bg);
        }
        x += button.chars().count() as u32 + 2;
    }

    let hint = " Enter confirm   Esc no ";
    let hx = (x0 + width).saturating_sub(hint.chars().count() as u32 + 1);
    screen.put_str(hx, y0 + height - 1, hint, d, theme.bg);
}

pub struct BrowserView<'a> {
    pub title: &'a str,
    /// Prompt sigil or label in front of the input.
    pub prompt: &'a str,
    pub input: &'a str,
    pub items: &'a [String],
    pub selected: usize,
    pub hint: &'a str,
}

pub fn draw_browser(screen: &mut Screen, theme: &Theme, view: &BrowserView<'_>) {
    let cols = screen.cols();
    let rows = screen.rows();
    let width = (cols.saturating_sub(6)).clamp(24, 62);
    let height = (rows.saturating_sub(4)).clamp(6, 14);
    let x0 = (cols - width) / 2;
    let y0 = (rows - height) / 2;

    // Border in box-drawing characters, like every DOS utility ever.
    let (b, d) = (theme.accent, theme.dim);
    frame_filled(screen, theme, x0, y0, width, height);

    // Title, overlaid on the top border.
    let title = format!(" {} ", view.title);
    screen.put_str(x0 + 2, y0, &title, b, theme.bg);

    // Query line.
    let qy = y0 + 1;
    screen.put_str(x0 + 2, qy, view.prompt, theme.accent, theme.bg);
    let prompt_len = view.prompt.chars().count() as u32;
    let shown = format!("{}_", view.input);
    screen.put_str_ellipsis(
        x0 + 3 + prompt_len,
        qy,
        &shown,
        width.saturating_sub(4 + prompt_len),
        theme.fg,
        theme.bg,
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
        screen.put_str_ellipsis(x0 + 2, y, item, width.saturating_sub(4), fg, bg);
    }

    if view.items.is_empty() {
        screen.put_str(x0 + 2, list_y, "no matches", theme.dim, theme.bg);
    }

    // Hint on the bottom border.
    let hint = format!(" {} ", view.hint);
    let hx = (x0 + width).saturating_sub(hint.chars().count() as u32 + 2);
    if hx > x0 {
        screen.put_str(hx, y0 + height - 1, &hint, d, theme.bg);
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
            state: "modified",
            location: "~/Dokumente/CRead",
            lines,
            rows,
            selection: None,
            matches: &[],
            current_match: None,
            cursor,
            margin: 1,
            stats: Stats {
                words: 3,
                chars: 20,
                lines: 2,
            },
            cursor_pos: (0, 3),
            bottom: Bottom::Status { message: input.0 },
            in_fence: false,
            cursor_on: true,
            now: None,
        }
    }

    #[test]
    fn renders_a_document_with_chrome_inside_the_grid() {
        let vp = vp(1280, 800);
        let mut ed = Editor::from_text("# Title\n\nSome **bold** prose.");
        ed.move_doc_end(false);
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
        let first: String = (1..40).map(|x| screen.get(x, 2).unwrap().ch).collect();
        assert!(first.contains("Title"));
    }

    #[test]
    fn headings_and_markers_get_different_roles() {
        let vp = vp(1280, 800);
        let mut ed = Editor::from_text("# Title");
        ed.move_doc_end(false);
        let (w, m) = layout(vp.cols());
        let rows = ed.visible_rows(w, text_height(vp.rows()));
        let mut screen = Screen::new(vp.cols(), vp.rows());
        draw_document(
            &mut screen,
            &Theme::DOS_BLUE,
            &view_from(ed.lines(), &rows, Some((0, 6)), ("", "dos-blue")),
        );
        let hash = screen.get(m as u32, 2).unwrap();
        let letter = screen.get(m as u32 + 4, 2).unwrap();
        assert_eq!(hash.fg, Theme::DOS_BLUE.dim);
        assert_eq!(letter.fg, Theme::DOS_BLUE.accent);
    }

    #[test]
    fn cursor_inverts_the_cell_under_it() {
        let vp = vp(1280, 800);
        let mut ed = Editor::from_text("abc");
        ed.move_end(false);
        let (w, m) = layout(vp.cols());
        let rows = ed.visible_rows(w, text_height(vp.rows()));
        let mut screen = Screen::new(vp.cols(), vp.rows());
        draw_document(
            &mut screen,
            &Theme::DOS_BLUE,
            &view_from(ed.lines(), &rows, Some((0, 3)), ("", "dos-blue")),
        );
        let cell = screen.get(m as u32 + 3, 2).unwrap();
        assert_eq!(cell.fg, Theme::DOS_BLUE.bg);
        assert_eq!(cell.bg, Theme::DOS_BLUE.fg);
    }

    #[test]
    fn selection_inverts_the_cells_it_covers() {
        let vp = vp(1280, 800);
        let mut ed = Editor::from_text("abcdef");
        let (w, m) = layout(vp.cols());
        let rows = ed.visible_rows(w, text_height(vp.rows()));
        ed.set_cursor(cread_core::Position { line: 0, col: 1 });
        ed.move_right(true);
        ed.move_right(true);

        let mut screen = Screen::new(vp.cols(), vp.rows());
        let mut view = view_from(ed.lines(), &rows, None, ("", "black"));
        view.selection = ed.selection();
        draw_document(&mut screen, &Theme::BLACK, &view);

        let sel = screen.get(m as u32 + 1, 2).unwrap();
        let plain = screen.get(m as u32 + 4, 2).unwrap();
        assert_eq!(sel.fg, Theme::BLACK.bg, "selected cell is inverted");
        assert_eq!(sel.bg, Theme::BLACK.fg);
        assert_eq!(plain.fg, Theme::BLACK.fg, "unselected cell is not");
    }

    #[test]
    fn search_hits_get_a_highlight_and_the_current_one_is_inverted() {
        let vp = vp(1280, 800);
        let ed = Editor::from_text("one two one");
        let (w, m) = layout(vp.cols());
        let rows = ed.visible_rows(w, text_height(vp.rows()));
        let hits = ed.find_all("one");

        let mut screen = Screen::new(vp.cols(), vp.rows());
        let mut view = view_from(ed.lines(), &rows, None, ("", "black"));
        view.matches = &hits;
        view.current_match = Some(1);
        draw_document(&mut screen, &Theme::BLACK, &view);

        let first = screen.get(m as u32, 2).unwrap();
        let second = screen.get(m as u32 + 8, 2).unwrap();
        let plain = screen.get(m as u32 + 4, 2).unwrap();

        assert_eq!(first.bg, Theme::BLACK.code_bg, "other hits are marked");
        assert_eq!(
            second.bg,
            Theme::BLACK.accent,
            "the current hit is inverted"
        );
        assert_eq!(plain.bg, Theme::BLACK.bg, "plain text is untouched");
    }

    #[test]
    fn the_browser_draws_a_bordered_box_with_a_selection() {
        let vp = vp(1280, 800);
        let mut screen = Screen::new(vp.cols(), vp.rows());
        let items = vec!["alpha.md".to_string(), "beta.md".to_string()];
        draw_browser(
            &mut screen,
            &Theme::DOS_BLUE,
            &BrowserView {
                title: "OPEN",
                prompt: ">",
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
    fn words_wrap_instead_of_being_cut() {
        assert_eq!(wrap_line("one two three", 7), vec!["one two", "three"]);
        assert_eq!(
            wrap_line("a supercalifragilistic b", 5),
            vec!["a", "supercalifragilistic", "b"],
            "a word longer than the line keeps a line of its own"
        );
        assert!(wrap_line("", 10).is_empty());
    }

    #[test]
    fn the_about_screen_credits_what_is_not_ours() {
        let vp = vp(1280, 800);
        let mut screen = Screen::new(vp.cols(), vp.rows());
        draw_help(&mut screen, &Theme::BLACK, true, &[], 0);
        let text = rows_as_text(&screen);

        for needle in [
            "CReadOS is a minimal fake operating system",
            "Built in Rust, winit and wgpu",
            "VileR",
            "CC BY-SA",
            "4.0",
            "https://int10h.org/oldschool-pc-fonts/",
            "mizu-core 1.3.0",
            "https://github.com/Amjad50/mizu",
        ] {
            assert!(text.contains(needle), "{needle:?} missing from:\n{text}");
        }

        // The signature sits on the last content row, right-aligned.
        let last = vp.rows().saturating_sub(2) as usize;
        let line = text.lines().nth(last).unwrap_or("");
        assert!(
            line.trim_end().ends_with("- darknoxx 2026"),
            "signature missing: {line:?}"
        );
    }

    #[test]
    fn welcome_shows_the_drives_and_the_prompt() {
        let vp = vp(1280, 800);
        let mut screen = Screen::new(vp.cols(), vp.rows());
        let items = vec![
            BootItem::row("A: INSPIRO", "the writing terminal", false),
            BootItem::rule(),
            BootItem::row("SOFTWARE", "install and remove modules", true),
            BootItem::row("THE VAULT", "documents, pictures, cartridges", true),
            BootItem::row("SETTINGS", "webdav, theme, font, screen", true),
        ];
        draw_welcome(
            &mut screen,
            &Theme::BLACK,
            &WelcomeView {
                show_prompt: true,
                title_row: 4,
                items: &items,
                selected: 2,
                note: "",
                input: "ne",
                hint: "arrows select",
                error: "",
            },
        );
        let text = rows_as_text(&screen);
        assert!(text.contains("Welcome to CRead"));
        assert!(text.contains("A: INSPIRO"));
        assert!(text.contains("SOFTWARE"));
        assert!(text.contains("THE VAULT"));
        assert!(text.contains("> ne"));

        // The rule sits between the modules and the shelf, and is not a row.
        let rule_row = 4 + 2 + 1;
        let grid = rows_as_text(&screen);
        let rule = grid.lines().nth(rule_row as usize).unwrap_or("");
        assert!(rule.contains('\u{2500}'), "the rule is drawn: {rule:?}");
        let inverted_rule = (0..vp.cols()).any(|x| {
            screen
                .get(x, rule_row)
                .is_some_and(|c| c.bg == Theme::BLACK.fg)
        });
        assert!(!inverted_rule, "the rule is never selectable");

        // The selected row is inverted somewhere along its length.
        let row = 4 + 2 + 2;
        let inverted = (0..vp.cols()).any(|x| {
            screen
                .get(x, row)
                .is_some_and(|c| c.bg == Theme::BLACK.fg && c.fg == Theme::BLACK.bg)
        });
        assert!(inverted, "selected row is not highlighted");
    }

    #[test]
    fn the_status_bar_shows_the_music_between_the_counts_and_the_hint() {
        let vp = vp(1280, 800);
        let mut screen = Screen::new(vp.cols(), vp.rows());
        let lines = vec!["hello world".to_string()];
        let rows = vec![VisualRow {
            line: 0,
            start: 0,
            text: "hello world".into(),
        }];
        let mut view = view_from(&lines, &rows, Some((0, 3)), ("", ""));
        view.now = Some("\u{266a} Morgenlicht \u{00b7} 00:42");
        draw_document(&mut screen, &Theme::BLACK, &view);
        let text = rows_as_text(&screen);
        let bar = text.lines().last().unwrap_or("");
        assert!(bar.contains("Morgenlicht"), "{bar}");
        // Between the counts and the hint, not at either end.
        let at = bar.find("Morgenlicht").unwrap();
        assert!(at > 10, "after the counts");
        assert!(at < bar.len() - 10, "before the hint");
    }

    #[test]
    fn the_music_is_dropped_when_the_bar_has_no_room_for_it() {
        let mut screen = Screen::new(40, 10);
        let lines = vec!["x".to_string()];
        let rows = vec![VisualRow {
            line: 0,
            start: 0,
            text: "x".into(),
        }];
        let mut view = view_from(&lines, &rows, Some((0, 0)), ("", ""));
        view.now = Some("\u{266a} Morgenlicht \u{00b7} 00:42");
        draw_document(&mut screen, &Theme::BLACK, &view);
        let text = rows_as_text(&screen);
        assert!(!text.contains("Morgenlicht"), "no room, no music: {text}");
    }

    #[test]
    fn the_program_line_shows_the_music_after_the_title() {
        let mut screen = Screen::new(80, 25);
        draw_program_line(
            &mut screen,
            &Theme::BLACK,
            "SOFTHAND",
            "pencil   colour 0   size 1",
            Some("\u{266a} Morgenlicht \u{00b7} 00:42"),
            None,
        );
        let text = rows_as_text(&screen);
        let bar = text.lines().last().unwrap_or("");
        assert!(bar.contains("SOFTHAND"), "{bar}");
        assert!(bar.contains("Morgenlicht"), "{bar}");
        assert!(bar.contains("pencil"), "{bar}");
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
