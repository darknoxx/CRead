//! The text buffer: cursor, soft wrap, undo and the view window.
//!
//! Deliberately line-oriented (`Vec<String>`) rather than a rope. Documents this
//! app is for are prose, not logs; a few hundred KB and a few thousand lines is
//! nothing, and line vectors make wrap/cursor maths obvious and testable.

use unicode_segmentation::UnicodeSegmentation;

/// A position in the document, in char indices (not bytes).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Position {
    pub line: usize,
    pub col: usize,
}

/// One visual row after soft wrapping.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisualRow {
    /// Logical line this row belongs to.
    pub line: usize,
    /// Char offset into that line where the row starts.
    pub start: usize,
    /// The row's text, without the trailing newline.
    pub text: String,
}

impl VisualRow {
    pub fn end(&self) -> Position {
        Position {
            line: self.line,
            col: self.start + self.text.chars().count(),
        }
    }

    pub fn contains(&self, pos: Position) -> bool {
        self.line == pos.line
            && pos.col >= self.start
            && pos.col <= self.start + self.text.chars().count()
    }
}

/// Document statistics for the status bar.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub words: usize,
    pub chars: usize,
    pub lines: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EditKind {
    Typing,
    Deleting,
    Structural,
    Paste,
}

#[derive(Clone)]
struct Snapshot {
    lines: Vec<String>,
    cursor: Position,
}

/// Coalescing undo: one entry per run of similar edits, which is what people
/// actually expect from Ctrl+Z (not one entry per keystroke).
struct History {
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    last_kind: Option<EditKind>,
    /// Rough memory cap so huge documents cannot balloon.
    limit_bytes: usize,
}

impl History {
    fn new() -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            last_kind: None,
            limit_bytes: 48 * 1024 * 1024,
        }
    }

    fn record(&mut self, kind: EditKind, lines: &[String], cursor: Position) {
        let coalesce = self.last_kind == Some(kind)
            && !matches!(kind, EditKind::Structural | EditKind::Paste)
            && !self.undo.is_empty();
        self.redo.clear();
        if !coalesce {
            self.undo.push(Snapshot {
                lines: lines.to_vec(),
                cursor,
            });
            self.trim();
        }
        self.last_kind = Some(kind);
    }

    fn break_group(&mut self) {
        self.last_kind = None;
    }

    fn trim(&mut self) {
        let mut total: usize = self
            .undo
            .iter()
            .map(|s| s.lines.iter().map(|l| l.len()).sum::<usize>())
            .sum();
        while total > self.limit_bytes && self.undo.len() > 1 {
            if let Some(removed) = self.undo.first() {
                total -= removed.lines.iter().map(|l| l.len()).sum::<usize>();
            }
            self.undo.remove(0);
        }
    }
}

pub struct Editor {
    lines: Vec<String>,
    cursor: Position,
    /// Top-left position of the viewport (first visible char).
    top: Position,
    /// Column the user aimed for with Up/Down, in visual cells.
    preferred_x: Option<usize>,
    /// Other end of the selection. `None` means no selection.
    anchor: Option<Position>,
    history: History,
    modified: bool,
}

impl Default for Editor {
    fn default() -> Self {
        Self::new()
    }
}

impl Editor {
    pub fn new() -> Self {
        Self {
            lines: vec![String::new()],
            cursor: Position::default(),
            top: Position::default(),
            preferred_x: None,
            anchor: None,
            history: History::new(),
            modified: false,
        }
    }

    pub fn from_text(text: &str) -> Self {
        let mut editor = Self::new();
        // Keep a trailing newline meaningful: "a\n" splits into ["a", ""], which
        // round-trips through `text()` exactly.
        editor.lines = text
            .split('\n')
            .map(|l| l.trim_end_matches('\r').to_string())
            .collect();
        if editor.lines.is_empty() {
            editor.lines.push(String::new());
        }
        editor.modified = false;
        editor
    }

    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    pub fn cursor(&self) -> Position {
        self.cursor
    }

    pub fn top(&self) -> Position {
        self.top
    }

    pub fn modified(&self) -> bool {
        self.modified
    }

    pub fn mark_saved(&mut self) {
        self.modified = false;
    }

    pub fn set_cursor(&mut self, pos: Position) {
        self.cursor = self.clamp(pos);
        self.preferred_x = None;
        self.anchor = None;
        self.history.break_group();
    }

    // ── selection ───────────────────────────────────────────────────────────

    /// Selected range, ordered, or `None` when nothing is selected.
    pub fn selection(&self) -> Option<(Position, Position)> {
        let anchor = self.anchor?;
        if anchor == self.cursor {
            return None;
        }
        Some(if anchor < self.cursor {
            (anchor, self.cursor)
        } else {
            (self.cursor, anchor)
        })
    }

    pub fn has_selection(&self) -> bool {
        self.selection().is_some()
    }

    pub fn select_all(&mut self) {
        self.anchor = Some(Position::default());
        self.cursor = Position {
            line: self.lines.len() - 1,
            col: self.lines[self.lines.len() - 1].chars().count(),
        };
        self.preferred_x = None;
    }

    pub fn clear_selection(&mut self) {
        self.anchor = None;
    }

    /// The selected text, with real newlines between lines.
    pub fn selected_text(&self) -> String {
        let Some((start, end)) = self.selection() else {
            return String::new();
        };
        if start.line == end.line {
            return self.lines[start.line]
                .chars()
                .skip(start.col)
                .take(end.col - start.col)
                .collect();
        }
        let mut out: String = self.lines[start.line].chars().skip(start.col).collect();
        for line in start.line + 1..end.line {
            out.push('\n');
            out.push_str(&self.lines[line]);
        }
        out.push('\n');
        out.extend(self.lines[end.line].chars().take(end.col));
        out
    }

    /// Start (or continue) a selection for a movement, or drop it.
    fn begin_move(&mut self, extend: bool) {
        if extend {
            self.anchor.get_or_insert(self.cursor);
        } else {
            self.anchor = None;
        }
    }

    /// Remove the selected range without recording undo, returning whether
    /// anything was deleted.
    fn delete_selection_raw(&mut self) -> bool {
        let Some((start, end)) = self.selection() else {
            return false;
        };
        let tail: String = self.lines[end.line].chars().skip(end.col).collect();
        let head: String = self.lines[start.line].chars().take(start.col).collect();
        self.lines[start.line] = head + &tail;
        self.lines.drain(start.line + 1..=end.line);
        self.cursor = start;
        self.anchor = None;
        self.preferred_x = None;
        true
    }

    /// Tell the undo stack that the next edit starts a new step.
    pub fn break_undo_group(&mut self) {
        self.history.break_group();
    }

    pub fn stats(&self) -> Stats {
        let mut words = 0;
        let mut chars = 0;
        for line in &self.lines {
            words += line.split_whitespace().count();
            chars += line.chars().count();
        }
        Stats {
            words,
            chars,
            lines: self.lines.len(),
        }
    }

    fn clamp(&self, pos: Position) -> Position {
        let line = pos.line.min(self.lines.len() - 1);
        Position {
            line,
            col: pos.col.min(self.lines[line].chars().count()),
        }
    }

    fn before_edit(&mut self, kind: EditKind) {
        self.history.record(kind, &self.lines, self.cursor);
        self.modified = true;
    }

    // ── editing ─────────────────────────────────────────────────────────────

    pub fn insert_char(&mut self, ch: char) {
        if ch == '\n' {
            self.insert_newline();
            return;
        }
        self.before_edit(EditKind::Typing);
        self.delete_selection_raw();
        let byte = byte_of_char(&self.lines[self.cursor.line], self.cursor.col);
        self.lines[self.cursor.line].insert(byte, ch);
        self.cursor.col += 1;
        self.preferred_x = None;
    }

    pub fn insert_text(&mut self, text: &str) {
        let kind = if text.contains('\n') {
            EditKind::Paste
        } else {
            EditKind::Typing
        };
        self.before_edit(kind);
        self.delete_selection_raw();
        for ch in text.chars() {
            if ch == '\r' {
                continue;
            }
            if ch == '\n' {
                self.split_line();
            } else {
                let byte = byte_of_char(&self.lines[self.cursor.line], self.cursor.col);
                self.lines[self.cursor.line].insert(byte, ch);
                self.cursor.col += 1;
            }
        }
        self.preferred_x = None;
    }

    pub fn insert_newline(&mut self) {
        self.before_edit(EditKind::Structural);
        self.delete_selection_raw();
        self.split_line();
        self.preferred_x = None;
    }

    fn split_line(&mut self) {
        let line = self.cursor.line;
        let byte = byte_of_char(&self.lines[line], self.cursor.col);
        let rest = self.lines[line].split_off(byte);
        self.lines.insert(line + 1, rest);
        self.cursor = Position {
            line: line + 1,
            col: 0,
        };
    }

    pub fn insert_tab(&mut self) {
        self.insert_text("    ");
    }

    pub fn backspace(&mut self) {
        if self.has_selection() {
            self.before_edit(EditKind::Deleting);
            self.delete_selection_raw();
            return;
        }
        if self.cursor.col == 0 {
            if self.cursor.line == 0 {
                return;
            }
            let line = self.cursor.line;
            let prev_len = self.lines[line - 1].chars().count();
            self.before_edit(EditKind::Structural);
            let current = self.lines.remove(line);
            self.lines[line - 1].push_str(&current);
            self.cursor = Position {
                line: line - 1,
                col: prev_len,
            };
        } else {
            self.before_edit(EditKind::Deleting);
            let line = &mut self.lines[self.cursor.line];
            let col = self.cursor.col;
            // Remove the whole grapheme cluster before the cursor, so emoji and
            // combining marks don't get shredded.
            let (start, end) = grapheme_at_char(line, col - 1);
            let start_byte = byte_of_char(line, start);
            let end_byte = byte_of_char(line, end);
            line.replace_range(start_byte..end_byte, "");
            self.cursor.col = start;
        }
        self.preferred_x = None;
    }

    pub fn delete_forward(&mut self) {
        if self.has_selection() {
            self.before_edit(EditKind::Deleting);
            self.delete_selection_raw();
            return;
        }
        let line_len = self.lines[self.cursor.line].chars().count();
        if self.cursor.col >= line_len {
            if self.cursor.line + 1 >= self.lines.len() {
                return;
            }
            self.before_edit(EditKind::Structural);
            let next = self.lines.remove(self.cursor.line + 1);
            self.lines[self.cursor.line].push_str(&next);
        } else {
            self.before_edit(EditKind::Deleting);
            let line = &mut self.lines[self.cursor.line];
            let col = self.cursor.col;
            let (start, end) = grapheme_at_char(line, col);
            let start_byte = byte_of_char(line, start);
            let end_byte = byte_of_char(line, end);
            line.replace_range(start_byte..end_byte, "");
        }
        self.preferred_x = None;
    }

    /// Delete from the cursor to `to` (used for selections later).
    pub fn delete_range(&mut self, from: Position, to: Position) {
        let (from, to) = if from <= to { (from, to) } else { (to, from) };
        if from == to {
            return;
        }
        let from = self.clamp(from);
        let to = self.clamp(to);
        self.before_edit(EditKind::Deleting);
        let tail = self.lines[to.line].chars().skip(to.col).collect::<String>();
        let head = self.lines[from.line]
            .chars()
            .take(from.col)
            .collect::<String>();
        self.lines[from.line] = head + &tail;
        self.lines.drain(from.line + 1..=to.line);
        self.cursor = from;
        self.preferred_x = None;
    }

    // ── navigation ──────────────────────────────────────────────────────────

    pub fn move_left(&mut self, extend: bool) {
        self.begin_move(extend);
        self.preferred_x = None;
        if self.cursor.col > 0 {
            self.cursor.col =
                grapheme_at_char(&self.lines[self.cursor.line], self.cursor.col - 1).0;
        } else if self.cursor.line > 0 {
            self.cursor.line -= 1;
            self.cursor.col = self.lines[self.cursor.line].chars().count();
        }
    }

    pub fn move_right(&mut self, extend: bool) {
        self.begin_move(extend);
        self.preferred_x = None;
        let line = &self.lines[self.cursor.line];
        if self.cursor.col < line.chars().count() {
            self.cursor.col = grapheme_at_char(line, self.cursor.col).1;
        } else if self.cursor.line + 1 < self.lines.len() {
            self.cursor.line += 1;
            self.cursor.col = 0;
        }
    }

    pub fn move_home(&mut self, extend: bool) {
        self.begin_move(extend);
        self.preferred_x = None;
        self.cursor.col = 0;
    }

    pub fn move_end(&mut self, extend: bool) {
        self.begin_move(extend);
        self.preferred_x = None;
        self.cursor.col = self.lines[self.cursor.line].chars().count();
    }

    pub fn move_doc_start(&mut self, extend: bool) {
        self.begin_move(extend);
        self.preferred_x = None;
        self.cursor = Position::default();
    }

    pub fn move_doc_end(&mut self, extend: bool) {
        self.begin_move(extend);
        self.preferred_x = None;
        self.cursor = Position {
            line: self.lines.len() - 1,
            col: self.lines[self.lines.len() - 1].chars().count(),
        };
    }

    pub fn move_word_left(&mut self, extend: bool) {
        self.begin_move(extend);
        self.preferred_x = None;
        let line = &self.lines[self.cursor.line];
        let before: String = line.chars().take(self.cursor.col).collect();
        let trimmed = before.trim_end();
        match trimmed.rfind(char::is_whitespace) {
            Some(byte) => self.cursor.col = trimmed[..=byte].chars().count(),
            None => self.cursor.col = 0,
        }
    }

    pub fn move_word_right(&mut self, extend: bool) {
        self.begin_move(extend);
        self.preferred_x = None;
        let line = &self.lines[self.cursor.line];
        let after: String = line.chars().skip(self.cursor.col).collect();
        let trimmed = after.trim_start();
        let skipped = after.chars().count() - trimmed.chars().count();
        match trimmed.find(char::is_whitespace) {
            Some(byte) => self.cursor.col += skipped + trimmed[..byte].chars().count(),
            None => self.cursor.col = line.chars().count(),
        }
    }

    pub fn move_up(&mut self, width: usize, extend: bool) {
        self.begin_move(extend);
        self.move_vertical(-1, width, extend);
    }

    pub fn move_down(&mut self, width: usize, extend: bool) {
        self.begin_move(extend);
        self.move_vertical(1, width, extend);
    }

    fn move_vertical(&mut self, delta: isize, width: usize, _extend: bool) {
        let width = width.max(1);
        let segs = wrap(&self.lines[self.cursor.line], width);
        let idx = segment_of(&segs, self.cursor.col);
        let x = self
            .preferred_x
            .unwrap_or_else(|| self.cursor.col - segs[idx].0);

        let target = idx as isize + delta;
        if target >= 0 && (target as usize) < segs.len() {
            let (start, end) = segs[target as usize];
            self.cursor.col = start + x.min(end - start);
        } else if delta > 0 {
            if self.cursor.line + 1 < self.lines.len() {
                self.cursor.line += 1;
                let segs = wrap(&self.lines[self.cursor.line], width);
                let (start, end) = segs[0];
                self.cursor.col = start + x.min(end - start);
            } else {
                self.cursor.col = self.lines[self.cursor.line].chars().count();
            }
        } else {
            if self.cursor.line > 0 {
                self.cursor.line -= 1;
                let segs = wrap(&self.lines[self.cursor.line], width);
                let (start, end) = *segs.last().unwrap();
                self.cursor.col = start + x.min(end - start);
            } else {
                self.cursor.col = 0;
            }
        }
        self.preferred_x = Some(x);
    }

    // ── undo ────────────────────────────────────────────────────────────────

    pub fn undo(&mut self) -> bool {
        // A pending coalesced run still has its "before" snapshot on the stack.
        let Some(snap) = self.history.undo.pop() else {
            return false;
        };
        self.history.redo.push(Snapshot {
            lines: self.lines.clone(),
            cursor: self.cursor,
        });
        self.lines = snap.lines;
        self.cursor = self.clamp(snap.cursor);
        self.anchor = None;
        self.history.break_group();
        self.modified = true;
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(snap) = self.history.redo.pop() else {
            return false;
        };
        self.history.undo.push(Snapshot {
            lines: self.lines.clone(),
            cursor: self.cursor,
        });
        self.lines = snap.lines;
        self.cursor = self.clamp(snap.cursor);
        self.anchor = None;
        self.history.break_group();
        self.modified = true;
        true
    }

    pub fn can_undo(&self) -> bool {
        !self.history.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.history.redo.is_empty()
    }

    // ── viewport ────────────────────────────────────────────────────────────

    /// Visual rows starting at `top`, at most `limit` of them.
    pub fn visible_rows(&self, width: usize, limit: usize) -> Vec<VisualRow> {
        let width = width.max(1);
        let mut rows = Vec::new();
        let mut line = self.top.line.min(self.lines.len().saturating_sub(1));
        let mut col = self.top.col;

        while line < self.lines.len() && (rows.len() < limit || limit == 0) {
            let text = &self.lines[line];
            let segs = wrap(text, width);
            let mut skipping = col > 0;
            for (start, end) in segs {
                if skipping {
                    if end <= col && end > start {
                        continue;
                    }
                    skipping = false;
                }
                let chars: String = text.chars().skip(start).take(end - start).collect();
                rows.push(VisualRow {
                    line,
                    start,
                    text: chars,
                });
                if limit > 0 && rows.len() >= limit {
                    return rows;
                }
            }
            line += 1;
            col = 0;
        }
        rows
    }

    /// Keep the cursor on screen. With `anchor` set the cursor is pinned to that
    /// row (typewriter scrolling).
    pub fn scroll_to_cursor(&mut self, width: usize, height: usize, anchor: Option<usize>) {
        if width == 0 || height == 0 {
            return;
        }
        let cursor = self.cursor;
        if cursor < self.top {
            // Jumping backwards: align the cursor to the target row from above.
            self.anchor_from_cursor(width, height, anchor);
            return;
        }

        let rows = self.visible_rows(width, height + 1);
        let offset = rows.iter().position(|r| r.contains(cursor));
        match (offset, anchor) {
            (Some(i), None) if i < height => {}
            (_, target) => self.anchor_from_cursor(width, height, target),
        }
    }

    /// Walk backwards from the cursor so that it lands on `anchor` (or the last
    /// row when `anchor` is None).
    fn anchor_from_cursor(&mut self, width: usize, height: usize, anchor: Option<usize>) {
        let width = width.max(1);
        let target = anchor
            .unwrap_or(height.saturating_sub(1))
            .min(height.saturating_sub(1));

        let mut line = self.cursor.line;
        let mut segs = wrap(&self.lines[line], width);
        let mut seg = segment_of(&segs, self.cursor.col);
        let mut remaining = target;

        while remaining > 0 {
            if seg > 0 {
                seg -= 1;
                remaining -= 1;
            } else if line > 0 {
                line -= 1;
                segs = wrap(&self.lines[line], width);
                seg = segs.len() - 1;
                remaining -= 1;
            } else {
                break;
            }
        }

        self.top = Position {
            line,
            col: segs[seg].0,
        };
    }
}

// ── helpers ─────────────────────────────────────────────────────────────────

/// Wrap a line into char ranges, breaking on spaces where possible.
pub fn wrap(line: &str, width: usize) -> Vec<(usize, usize)> {
    let width = width.max(1);
    let chars: Vec<char> = line.chars().collect();
    if chars.is_empty() {
        return vec![(0, 0)];
    }
    if chars.len() <= width {
        return vec![(0, chars.len())];
    }

    let mut out = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        let hard_end = (start + width).min(chars.len());
        let mut end = hard_end;
        if hard_end < chars.len() {
            // Prefer the last space (keep it on this row) so words stay whole.
            if let Some(rel) = chars[start..hard_end].iter().rposition(|c| *c == ' ')
                && rel > 0
            {
                end = start + rel + 1;
            }
        }
        out.push((start, end));
        start = end;
    }
    out
}

/// Index of the segment containing `col`.
fn segment_of(segs: &[(usize, usize)], col: usize) -> usize {
    for (i, (start, end)) in segs.iter().enumerate() {
        if col >= *start && col < *end {
            return i;
        }
    }
    segs.len().saturating_sub(1)
}

fn byte_of_char(s: &str, col: usize) -> usize {
    s.char_indices().nth(col).map(|(b, _)| b).unwrap_or(s.len())
}

/// Char range of the grapheme cluster containing `char_idx`.
fn grapheme_at_char(s: &str, char_idx: usize) -> (usize, usize) {
    let mut start = 0;
    for g in s.graphemes(true) {
        let end = start + g.chars().count();
        if char_idx < end {
            return (start, end);
        }
        start = end;
    }
    (start, start)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ed(text: &str) -> Editor {
        Editor::from_text(text)
    }

    #[test]
    fn round_trips_exact_bytes() {
        for text in ["", "a", "a\n", "a\nb", "a\nb\n", "\n\n", "äöü\nß"] {
            assert_eq!(ed(text).text(), text, "{text:?}");
        }
    }

    #[test]
    fn typing_moves_the_cursor_and_marks_dirty() {
        let mut e = ed("");
        e.insert_char('h');
        e.insert_char('i');
        assert_eq!(e.text(), "hi");
        assert_eq!(e.cursor(), Position { line: 0, col: 2 });
        assert!(e.modified());
    }

    #[test]
    fn enter_splits_and_backspace_joins() {
        let mut e = ed("ab");
        e.move_end(false);
        e.insert_newline();
        e.insert_char('c');
        assert_eq!(e.text(), "ab\nc");
        assert_eq!(e.cursor(), Position { line: 1, col: 1 });
        e.backspace();
        e.backspace();
        assert_eq!(e.text(), "ab");
        assert_eq!(e.cursor(), Position { line: 0, col: 2 });
    }

    #[test]
    fn backspace_at_start_joins_lines() {
        let mut e = ed("ab\ncd");
        e.set_cursor(Position { line: 1, col: 0 });
        e.backspace();
        assert_eq!(e.text(), "abcd");
        assert_eq!(e.cursor(), Position { line: 0, col: 2 });
    }

    #[test]
    fn typing_coalesces_into_one_undo_step() {
        let mut e = ed("");
        for c in "hello".chars() {
            e.insert_char(c);
        }
        assert_eq!(e.text(), "hello");
        assert!(e.undo());
        assert_eq!(e.text(), "");
        assert!(!e.can_undo());
        assert!(e.redo());
        assert_eq!(e.text(), "hello");
    }

    #[test]
    fn structural_edits_break_the_group() {
        let mut e = ed("");
        e.insert_char('a');
        e.insert_newline();
        e.insert_char('b');
        e.undo();
        assert_eq!(e.text(), "a\n");
        e.undo();
        assert_eq!(e.text(), "a");
        e.undo();
        assert_eq!(e.text(), "");
    }

    #[test]
    fn wrap_breaks_on_spaces_and_never_exceeds_width() {
        let segs = wrap("hello world again", 8);
        assert_eq!(segs, vec![(0, 6), (6, 12), (12, 17)]);
        for (s, e) in wrap(&"x".repeat(50), 7) {
            assert!(e - s <= 7);
        }
    }

    #[test]
    fn wrap_of_unbreakable_text_hard_cuts() {
        assert_eq!(wrap("abcdefghij", 4), vec![(0, 4), (4, 8), (8, 10)]);
    }

    #[test]
    fn up_down_follow_visual_rows_when_wrapped() {
        let mut e = ed(&"x".repeat(30));
        // Three visual rows of 10.
        e.set_cursor(Position { line: 0, col: 25 });
        e.move_up(10, false);
        assert_eq!(e.cursor().col, 15);
        e.move_up(10, false);
        assert_eq!(e.cursor().col, 5);
        e.move_down(10, false);
        assert_eq!(e.cursor().col, 15);
    }

    #[test]
    fn up_down_crosses_lines() {
        let mut e = ed("abc\ndefgh");
        e.set_cursor(Position { line: 1, col: 3 });
        e.move_up(80, false);
        assert_eq!(e.cursor(), Position { line: 0, col: 3 });
        e.move_down(80, false);
        assert_eq!(e.cursor(), Position { line: 1, col: 3 });
        // Down from the last line stays put instead of running off the end.
        e.set_cursor(Position { line: 1, col: 3 });
        e.move_down(80, false);
        assert_eq!(e.cursor(), Position { line: 1, col: 5 });
    }

    #[test]
    fn scroll_keeps_cursor_in_view() {
        let text = (0..100)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut e = ed(&text);
        e.move_doc_end(false);
        e.scroll_to_cursor(80, 10, None);
        let rows = e.visible_rows(80, 10);
        assert!(rows.iter().any(|r| r.contains(e.cursor())));
        assert_eq!(rows.len(), 10);
    }

    #[test]
    fn typewriter_anchor_pins_the_cursor_row() {
        let text = (0..100)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut e = ed(&text);
        for _ in 0..60 {
            e.move_down(80, false);
            e.scroll_to_cursor(80, 20, Some(9));
        }
        let rows = e.visible_rows(80, 20);
        let idx = rows.iter().position(|r| r.contains(e.cursor())).unwrap();
        assert_eq!(idx, 9, "cursor should sit on the anchor row");
    }

    #[test]
    fn scrolling_back_up_works() {
        let text = (0..50)
            .map(|i| format!("l{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut e = ed(&text);
        e.move_doc_end(false);
        e.scroll_to_cursor(80, 10, None);
        e.move_doc_start(false);
        e.scroll_to_cursor(80, 10, None);
        let rows = e.visible_rows(80, 10);
        assert_eq!(rows[0].line, 0);
        assert!(rows.iter().any(|r| r.contains(e.cursor())));
    }

    #[test]
    fn deleting_a_grapheme_keeps_the_cluster_intact() {
        let mut e = ed("a👨‍👩‍👧b");
        e.set_cursor(Position { line: 0, col: 6 });
        e.backspace();
        assert_eq!(e.text(), "ab");
        assert_eq!(e.cursor().col, 1);
    }

    #[test]
    fn shift_movement_builds_a_selection() {
        let mut e = ed("hello world");
        e.move_right(false);
        e.move_right(true);
        e.move_right(true);
        assert_eq!(
            e.selection(),
            Some((Position { line: 0, col: 1 }, Position { line: 0, col: 3 }))
        );
        assert_eq!(e.selected_text(), "el");
    }

    #[test]
    fn plain_movement_drops_the_selection() {
        let mut e = ed("hello");
        e.move_right(true);
        e.move_right(true);
        assert!(e.has_selection());
        e.move_left(false);
        assert!(!e.has_selection());
    }

    #[test]
    fn select_all_and_selected_text_across_lines() {
        let mut e = ed("one
two
three");
        e.select_all();
        assert_eq!(e.selected_text(), "one\ntwo\nthree");

        let mut e = ed("one\ntwo");
        e.set_cursor(Position { line: 0, col: 1 });
        e.move_doc_end(true);
        assert_eq!(e.selected_text(), "ne\ntwo");
    }

    #[test]
    fn typing_replaces_the_selection() {
        let mut e = ed("hello world");
        e.select_all();
        e.insert_char('x');
        assert_eq!(e.text(), "x");
        assert!(!e.has_selection());
    }

    #[test]
    fn backspace_deletes_the_selection_instead_of_one_char() {
        let mut e = ed("hello world");
        e.set_cursor(Position { line: 0, col: 0 });
        for _ in 0..5 {
            e.move_right(true);
        }
        e.backspace();
        assert_eq!(e.text(), " world");
        assert!(!e.has_selection());
    }

    #[test]
    fn undo_restores_replaced_selection() {
        let mut e = ed("abc");
        e.select_all();
        e.insert_text("xy");
        assert_eq!(e.text(), "xy");
        assert!(e.undo());
        assert_eq!(e.text(), "abc");
    }

    #[test]
    fn stats_count_words_and_chars() {
        let e = ed("hello world\nzweite zeile\n");
        let s = e.stats();
        assert_eq!(s.words, 4);
        assert_eq!(s.lines, 3);
        assert_eq!(s.chars, "hello worldzweite zeile".len());
    }

    #[test]
    fn moving_resets_the_undo_group() {
        let mut e = ed("");
        e.insert_char('a');
        e.break_undo_group();
        e.insert_char('b');
        e.undo();
        assert_eq!(e.text(), "a");
    }
}
