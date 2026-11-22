//! Mouse, wheel and the clipboard.
//!
//! The clipboard handle lives in `App` for the whole run: on Wayland the owning
//! process serves the data, so a handle per copy would lose the text.

use super::*;

impl App {
    /// Map a physical pixel to a document position, or `None` outside the text.
    pub(super) fn position_at(&self, px: f64, py: f64) -> Option<Position> {
        let vp = self.vp?;
        let scale = vp.scale as f64;
        let vx = (px - vp.offset_x as f64) / scale;
        let vy = (py - vp.offset_y as f64) / scale;
        if vx < 0.0 || vy < 0.0 {
            return None;
        }

        let (text_width, margin) = ui::layout(self.screen.cols());
        let height = ui::text_height(self.screen.rows());

        // Two rows of chrome above the text.
        let row = (vy / self.font.cell_h() as f64) as usize;
        let row = row.checked_sub(2)?;
        if row >= height {
            return None;
        }
        // Clicking in the margin clamps to the start or end of the row instead
        // of doing nothing.
        let col = (vx / self.font.cell_w() as f64) as usize;
        let col_in_row = col.saturating_sub(margin).min(text_width);

        let rows = self.doc.editor.visible_rows(text_width, height);
        let visual = rows.get(row)?;
        let width = visual.text.chars().count();
        let doc_col = visual.start + col_in_row.min(width);
        Some(Position {
            line: visual.line,
            col: doc_col,
        })
    }

    pub(super) fn on_mouse_press(&mut self) {
        if self.mode != Mode::Write {
            return;
        }
        let (px, py) = self.pointer;
        if let Some(pos) = self.position_at(px, py) {
            self.doc.editor.begin_selection(pos);
            self.dragging = true;
            self.follow_cursor = true;
            self.poke();
        }
    }

    pub(super) fn on_mouse_release(&mut self) {
        self.dragging = false;
    }

    /// A kiosk starts without a pointer. It appears with the first real move
    /// (or click), so a machine with no mouse never shows an arrow.
    pub(super) fn hide_pointer(&mut self) {
        if !self.opts.kiosk || !self.pointer_visible {
            return;
        }
        if let Some(window) = self.window.as_ref() {
            window.set_cursor_visible(false);
        }
        self.pointer_visible = false;
    }

    pub(super) fn show_pointer(&mut self) {
        if !self.opts.kiosk || self.pointer_visible {
            return;
        }
        if let Some(window) = self.window.as_ref() {
            window.set_cursor_visible(true);
        }
        self.pointer_visible = true;
    }

    pub(super) fn on_mouse_drag(&mut self) {
        if !self.dragging || self.mode != Mode::Write {
            return;
        }
        let (px, py) = self.pointer;
        // Dragging past the edges just clamps to the last computed position.
        if let Some(pos) = self.position_at(px, py) {
            self.doc.editor.drag_to(pos);
            self.poke();
        }
    }

    pub(super) fn on_wheel(&mut self, lines: isize) {
        if self.mode == Mode::View {
            self.scroll_view(-lines);
            return;
        }
        if self.mode == Mode::Manual {
            self.scroll_manual(-lines * 3);
            return;
        }
        if self.mode != Mode::Write {
            return;
        }
        self.doc.editor.scroll_lines(lines);
        self.follow_cursor = false;
        self.poke();
    }

    pub(super) fn clipboard(&mut self) -> Result<&mut arboard::Clipboard, String> {
        if self.clipboard.is_none() {
            self.clipboard =
                Some(arboard::Clipboard::new().map_err(|e| format!("clipboard: {e}"))?);
        }
        self.clipboard
            .as_mut()
            .ok_or_else(|| "clipboard is not available".to_string())
    }

    /// The editor that has the keyboard: the quick note while it floats over
    /// the screen, the document otherwise.
    pub(super) fn active_editor(&mut self) -> &mut Editor {
        match self.note.as_mut() {
            Some(note) => &mut note.editor,
            None => &mut self.doc.editor,
        }
    }

    /// Record a clipboard edit; only the document keeps an edit clock.
    pub(super) fn touch_active(&mut self) {
        if self.note.is_some() {
            self.poke();
        } else {
            self.touch();
        }
    }

    pub(super) fn copy_selection(&mut self) -> Result<String, String> {
        let text = self.active_editor().selected_text();
        if text.is_empty() {
            return Err("nothing selected".into());
        }
        self.clipboard()?
            .set_text(text.clone())
            .map_err(|e| format!("clipboard: {e}"))?;
        Ok(format!("{} chars copied", text.chars().count()))
    }

    pub(super) fn cut_selection(&mut self) -> Result<String, String> {
        let text = self.active_editor().selected_text();
        if text.is_empty() {
            return Err("nothing selected".into());
        }
        self.clipboard()?
            .set_text(text.clone())
            .map_err(|e| format!("clipboard: {e}"))?;
        let editor = self.active_editor();
        if let Some((start, end)) = editor.selection() {
            editor.delete_range(start, end);
        }
        self.touch_active();
        Ok(format!("{} chars cut", text.chars().count()))
    }

    pub(super) fn paste(&mut self) -> Result<String, String> {
        let text = {
            let clipboard = self.clipboard()?;
            clipboard
                .get_text()
                .map_err(|e| format!("clipboard: {e}"))?
        };
        if text.is_empty() {
            return Err("clipboard is empty".into());
        }
        self.active_editor().insert_text(&text);
        self.touch_active();
        Ok(format!("{} chars pasted", text.chars().count()))
    }
}
