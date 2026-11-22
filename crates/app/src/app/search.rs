//! Search in the document and the read-only view mode.

use super::*;

impl App {
    pub(super) fn on_key_search(&mut self, event: KeyEvent) {
        let shift = self.mods.shift_key();
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => self.cancel_search(),
            Key::Named(NamedKey::Enter) => self.accept_search(),
            Key::Named(NamedKey::F3) => self.step_match(!shift),
            Key::Named(NamedKey::Backspace) => {
                if let Some(search) = self.search.as_mut() {
                    search.query.pop();
                }
                self.update_search();
            }
            Key::Named(NamedKey::Space) => {
                if let Some(search) = self.search.as_mut() {
                    search.query.push(' ');
                }
                self.update_search();
            }
            Key::Character(text) => {
                if self.mods.alt_key() {
                    return;
                }
                if let Some(search) = self.search.as_mut() {
                    for ch in text.chars().filter(|c| !c.is_control()) {
                        search.query.push(ch);
                    }
                }
                self.update_search();
            }
            _ => {}
        }
    }

    pub(super) fn enter_view(&mut self) {
        if !self.has_document {
            return;
        }
        let (width, _) = ui::layout(self.screen.cols());
        self.view_rows = cread_core::markdown::render(self.doc.editor.lines(), width);
        self.view_width = width;

        // Open near the line being edited rather than at the top.
        let height = ui::text_height(self.screen.rows());
        let target = self.doc.editor.cursor().line;
        let index = self
            .view_rows
            .iter()
            .position(|row| row.source_line >= target)
            .unwrap_or(0);
        self.view_top = index.saturating_sub(height / 3);
        self.mode = Mode::View;
        self.dirty = true;
    }

    pub(super) fn scroll_view(&mut self, delta: isize) {
        let height = ui::text_height(self.screen.rows());
        let max = self.view_rows.len().saturating_sub(height) as isize;
        self.view_top = (self.view_top as isize + delta).clamp(0, max.max(0)) as usize;
        self.poke();
    }

    pub(super) fn on_key_view(&mut self, event: KeyEvent) {
        let page = ui::text_height(self.screen.rows()) as isize;
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => self.to_write(),
            Key::Named(NamedKey::ArrowDown) => self.scroll_view(1),
            Key::Named(NamedKey::ArrowUp) => self.scroll_view(-1),
            Key::Named(NamedKey::PageDown) | Key::Named(NamedKey::Space) => {
                self.scroll_view(page.saturating_sub(1))
            }
            Key::Named(NamedKey::PageUp) => self.scroll_view(-page.saturating_sub(1)),
            Key::Named(NamedKey::Home) => {
                self.view_top = 0;
                self.poke();
            }
            Key::Named(NamedKey::End) => {
                self.scroll_view(isize::MAX / 2);
            }
            _ => {}
        }
    }

    pub(super) fn start_search(&mut self, query: Option<String>) {
        if !self.has_document {
            return;
        }
        self.search = Some(Search {
            origin: self.doc.editor.cursor(),
            query: query.unwrap_or_default(),
            matches: Vec::new(),
            current: 0,
        });
        self.mode = Mode::Search;
        self.update_search();
    }

    /// Recompute the hits and jump to the first one at or after the origin.
    pub(super) fn update_search(&mut self) {
        let (matches, origin) = match &self.search {
            Some(search) => (self.doc.editor.find_all(&search.query), search.origin),
            None => return,
        };
        if let Some(search) = self.search.as_mut() {
            search.current = matches
                .iter()
                .position(|(pos, _)| *pos >= origin)
                .unwrap_or(0);
            search.matches = matches;
        }
        if let Some(pos) = self
            .search
            .as_ref()
            .and_then(|s| s.matches.get(s.current))
            .map(|(pos, _)| *pos)
        {
            self.doc.editor.set_cursor(pos);
        }
        self.dirty = true;
    }

    pub(super) fn step_match(&mut self, forward: bool) {
        let Some(search) = self.search.as_mut() else {
            return;
        };
        if search.matches.is_empty() {
            return;
        }
        let len = search.matches.len();
        search.current = if forward {
            (search.current + 1) % len
        } else {
            (search.current + len - 1) % len
        };
        let pos = search.matches[search.current].0;
        self.doc.editor.set_cursor(pos);
        self.dirty = true;
    }

    /// Enter: keep the cursor where the search landed.
    pub(super) fn accept_search(&mut self) {
        if let Some(search) = self.search.take() {
            if !search.query.is_empty() {
                self.last_search = Some(search.query.clone());
            }
            if let Some((pos, _)) = search.matches.get(search.current) {
                self.doc.editor.set_cursor(*pos);
            }
        }
        // Loading is its own destination; anything else drops back to the
        // document or the home screen.
        if self.mode != Mode::Loading {
            self.to_write();
        }
    }

    /// Esc: put the cursor back where the search started.
    pub(super) fn cancel_search(&mut self) {
        if let Some(search) = self.search.take() {
            self.doc.editor.set_cursor(search.origin);
        }
        // Loading is its own destination; anything else drops back to the
        // document or the home screen.
        if self.mode != Mode::Loading {
            self.to_write();
        }
    }

    /// F3 outside search: jump to the next hit of the previous query.
    pub(super) fn repeat_search(&mut self, forward: bool) {
        let Some(query) = self.last_search.clone() else {
            self.set_status("no previous search");
            return;
        };
        let matches = self.doc.editor.find_all(&query);
        if matches.is_empty() {
            self.set_status(format!("no matches for {query}"));
            return;
        }
        let cursor = self.doc.editor.cursor();
        let index = if forward {
            matches
                .iter()
                .position(|(pos, _)| *pos > cursor)
                .unwrap_or(0)
        } else {
            matches
                .iter()
                .rposition(|(pos, _)| *pos < cursor)
                .unwrap_or(matches.len() - 1)
        };
        self.doc.editor.set_cursor(matches[index].0);
        self.set_status(format!("{}/{} matches", index + 1, matches.len()));
    }
}
