//! The embedded user manual: contents menu, jump targets and scrolling.
//!
//! The text itself is `manual.md`, compiled in; it is never a vault document.

use super::*;

/// One selectable point in the manual's contents.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ManualJump {
    /// Source line of the contents entry, for the cursor row.
    pub(super) line: usize,
    /// What the contents line says (`1. Starting`, `A — Commands …`).
    pub(super) label: String,
    /// Source line of the heading Enter jumps to.
    pub(super) target: usize,
}

/// Build the contents menu from the Markdown itself.
///
/// Only real list entries count: the bold `Part …` headings are skipped, as
/// are the Contents heading and the rule that closes the block. The matching
/// heading is the first one after Contents whose text ends with the entry's
/// label, so `Starting` finds `Step 1: Starting` and `A — Commands …` finds
/// `Appendix A — Commands …`.
pub(super) fn manual_jumps() -> Vec<ManualJump> {
    let lines: Vec<&str> = MANUAL.lines().collect();
    let Some(contents) = lines.iter().position(|line| line.trim() == "## Contents") else {
        return Vec::new();
    };
    let end = lines[contents + 1..]
        .iter()
        .position(|line| line.trim() == "---")
        .map(|offset| contents + 1 + offset)
        .unwrap_or(lines.len());

    let mut entries = Vec::new();
    for (offset, line) in lines[contents + 1..end].iter().enumerate() {
        let text = line.trim();
        let label = if let Some((number, rest)) = text.split_once(". ")
            && number.chars().all(|ch| ch.is_ascii_digit())
            && !rest.is_empty()
        {
            rest
        } else if let Some(rest) = text.strip_prefix("- ")
            && !rest.is_empty()
        {
            rest
        } else {
            continue;
        };
        entries.push((contents + 1 + offset, text.to_string(), label.to_string()));
    }

    let mut jumps = Vec::new();
    for (line, label, search) in entries {
        let wanted = normalize_manual(&search);
        let target = lines
            .iter()
            .enumerate()
            .skip(end + 1)
            .find(|(_, line)| {
                manual_heading(line).is_some_and(|heading| {
                    let heading = normalize_manual(heading);
                    heading == wanted || heading.ends_with(&format!(" {wanted}"))
                })
            })
            .map(|(line, _)| line);
        if let Some(target) = target {
            jumps.push(ManualJump {
                line,
                label,
                target,
            });
        }
    }
    jumps
}

pub(super) fn manual_heading(line: &str) -> Option<&str> {
    let line = line.trim_start();
    let hashes = line.chars().take_while(|ch| *ch == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    Some(line[hashes..].trim())
}

pub(super) fn normalize_manual(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

impl App {
    /// Open the built-in manual, rendered for the current width.
    pub(super) fn open_manual(&mut self, back: Mode) {
        let (width, _) = ui::layout(self.screen.cols());
        self.manual_jumps = manual_jumps();
        self.manual_choice = 0;
        self.manual_nav = !self.manual_jumps.is_empty();
        self.manual_top = 0;
        self.render_manual(width);
        // Land on Contents, not on the cover: the first entry is then visible
        // with the cursor already on it. On a very short screen this scrolls
        // just enough to keep the cursor on screen.
        self.manual_top = self.manual_contents_row().unwrap_or(0);
        self.keep_manual_selection_visible();
        self.manual_back = back;
        self.mode = Mode::Manual;
        self.dirty = true;
    }

    /// Render the embedded manual at `width`. The scroll position survives a
    /// resize as far as the new layout still has that row.
    pub(super) fn render_manual(&mut self, width: usize) {
        if self.manual_width != width || self.manual_rows.is_empty() {
            let lines: Vec<String> = MANUAL.lines().map(str::to_string).collect();
            self.manual_rows = cread_core::markdown::render(&lines, width);
            self.manual_width = width;
        }
        let max = self
            .manual_rows
            .len()
            .saturating_sub(ui::text_height(self.screen.rows()));
        self.manual_top = self.manual_top.min(max);
    }

    pub(super) fn leave_manual(&mut self) {
        self.mode = self.manual_back;
        self.dirty = true;
    }

    pub(super) fn manual_contents_row(&self) -> Option<usize> {
        let source = MANUAL
            .lines()
            .position(|line| line.trim() == "## Contents")?;
        self.manual_rows
            .iter()
            .position(|row| row.source_line == source)
    }

    /// The rendered rows where the cursor currently sits (a wrapped entry may
    /// occupy more than one row).
    pub(super) fn manual_selection_span(&self) -> Option<(usize, usize)> {
        let jump = self.manual_jumps.get(self.manual_choice)?;
        let first = self
            .manual_rows
            .iter()
            .position(|row| row.source_line == jump.line)?;
        let last = self.manual_rows[first..]
            .iter()
            .position(|row| row.source_line != jump.line)
            .map(|offset| first + offset)
            .unwrap_or(self.manual_rows.len());
        Some((first, last))
    }

    pub(super) fn manual_selection_row(&self) -> Option<usize> {
        self.manual_selection_span().map(|(first, _)| first)
    }

    /// Where Enter should put the top of the page.
    pub(super) fn manual_target_row(&self) -> Option<usize> {
        let jump = self.manual_jumps.get(self.manual_choice)?;
        self.manual_rows
            .iter()
            .position(|row| row.source_line == jump.target)
    }

    pub(super) fn keep_manual_selection_visible(&mut self) {
        let Some(row) = self.manual_selection_row() else {
            return;
        };
        let height = ui::text_height(self.screen.rows());
        if height < 4 {
            // The first compose can run before the viewport is known; do not
            // scroll to a menu entry in a one-row screen.
            return;
        }
        if row < self.manual_top {
            self.manual_top = row;
        } else if row >= self.manual_top + height {
            self.manual_top = row.saturating_sub(height.saturating_sub(1));
        }
    }

    pub(super) fn scroll_manual(&mut self, delta: isize) {
        let height = ui::text_height(self.screen.rows());
        let max = self.manual_rows.len().saturating_sub(height) as isize;
        self.manual_top = (self.manual_top as isize + delta).clamp(0, max.max(0)) as usize;
        // Scrolling away leaves the Contents menu; landing back on the heading
        // puts the cursor on the first entry again.
        let at_contents =
            !self.manual_jumps.is_empty() && Some(self.manual_top) == self.manual_contents_row();
        if at_contents && !self.manual_nav {
            self.manual_choice = 0;
        }
        self.manual_nav = at_contents;
        self.poke();
    }

    pub(super) fn select_manual(&mut self, delta: isize) {
        if self.manual_jumps.is_empty() {
            self.scroll_manual(delta);
            return;
        }
        let last = self.manual_jumps.len() - 1;
        self.manual_choice = (self.manual_choice as isize + delta).clamp(0, last as isize) as usize;
        self.manual_nav = true;
        self.keep_manual_selection_visible();
        self.poke();
    }

    pub(super) fn enter_manual_selection(&mut self) {
        let Some(row) = self.manual_target_row() else {
            return;
        };
        self.manual_top = row;
        self.manual_nav = false;
        self.poke();
    }

    pub(super) fn on_key_manual(&mut self, event: KeyEvent) {
        let page = ui::text_height(self.screen.rows()) as isize;
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => self.leave_manual(),
            Key::Named(NamedKey::ArrowDown) if self.manual_nav => self.select_manual(1),
            Key::Named(NamedKey::ArrowUp) if self.manual_nav => self.select_manual(-1),
            Key::Named(NamedKey::ArrowDown) => self.scroll_manual(1),
            Key::Named(NamedKey::ArrowUp) => self.scroll_manual(-1),
            Key::Named(NamedKey::Enter) if self.manual_nav => self.enter_manual_selection(),
            Key::Named(NamedKey::PageDown) | Key::Named(NamedKey::Space) => {
                self.scroll_manual(page.saturating_sub(1))
            }
            Key::Named(NamedKey::PageUp) => self.scroll_manual(-page.saturating_sub(1)),
            Key::Named(NamedKey::Home) => {
                self.manual_top = self.manual_contents_row().unwrap_or(0);
                self.manual_choice = 0;
                self.manual_nav = !self.manual_jumps.is_empty();
                self.keep_manual_selection_visible();
                self.poke();
            }
            Key::Named(NamedKey::End) => self.scroll_manual(isize::MAX / 2),
            _ => {}
        }
    }
}
