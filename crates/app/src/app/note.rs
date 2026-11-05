//! The quick note: a small editable sheet over whatever is already on screen.
//!
//! It is not a mode. The screen underneath stays composed exactly as it was;
//! this module only paints a centred box and takes the keyboard until Esc. The
//! text is written to `Notes/` immediately on closing.

use super::*;
use chrono::Local;
use cread_core::{Cell, Editor};

/// The box is compact on purpose: a phone number or an address, not a letter.
const NOTE_W: u32 = 42;
const NOTE_H: u32 = 12;

pub(super) struct QuickNote {
    pub(super) editor: Editor,
}

impl QuickNote {
    fn new() -> Self {
        Self {
            editor: Editor::new(),
        }
    }
}

impl App {
    /// Open the note over the current screen. It is deliberately not a mode;
    /// the shell keeps drawing whatever was in front.
    pub(super) fn open_note(&mut self) -> Result<String, String> {
        if self.note.is_some() {
            return Err("a note is already open".into());
        }
        self.note = Some(QuickNote::new());
        self.dirty = true;
        Ok("note open - Esc saves".into())
    }

    fn note_width(&self) -> usize {
        self.screen
            .cols()
            .saturating_sub(2)
            .min(NOTE_W)
            .saturating_sub(2)
            .max(1) as usize
    }

    /// Write the note to `Notes/` and close it. An empty note leaves no file.
    pub(super) fn save_note(&mut self) -> Result<(), String> {
        let Some(note) = self.note.as_ref() else {
            return Ok(());
        };
        let text = note.editor.text();
        if text.trim().is_empty() {
            self.note = None;
            self.set_status("empty note discarded");
            self.dirty = true;
            return Ok(());
        }

        self.prepare_vault();
        let dir = storage::notes_in(&self.vault());
        let path = free_note_path(&dir);
        storage::save_atomic(&path, &text).map_err(|e| format!("note save failed: {e}"))?;
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "note.md".into());
        self.note = None;
        self.unsynced = true;
        self.set_status(format!("note saved: {name}"));
        self.dirty = true;
        Ok(())
    }

    /// The note has already captured the keyboard; nothing else sees these
    /// keys until it is closed.
    pub(super) fn on_key_note(&mut self, event: KeyEvent) {
        if matches!(&event.logical_key, Key::Named(NamedKey::Escape)) {
            if let Err(e) = self.save_note() {
                self.set_status(e);
            }
            self.poke();
            return;
        }

        let width = self.note_width();
        let shift = self.mods.shift_key();
        let ctrl = self.mods.control_key() || self.mods.super_key();
        let Some(note) = self.note.as_mut() else {
            return;
        };

        if ctrl {
            match event.physical_key {
                PhysicalKey::Code(KeyCode::KeyZ) => {
                    if shift {
                        note.editor.redo();
                    } else {
                        note.editor.undo();
                    }
                }
                PhysicalKey::Code(KeyCode::KeyY) => {
                    note.editor.redo();
                }
                PhysicalKey::Code(KeyCode::KeyA) => note.editor.select_all(),
                _ => {}
            }
            self.poke();
            return;
        }

        match &event.logical_key {
            Key::Named(NamedKey::Enter) => note.editor.insert_newline(),
            Key::Named(NamedKey::Backspace) => note.editor.backspace(),
            Key::Named(NamedKey::Delete) => note.editor.delete_forward(),
            Key::Named(NamedKey::Tab) => note.editor.insert_tab(),
            Key::Named(NamedKey::Space) => note.editor.insert_char(' '),
            Key::Named(NamedKey::ArrowLeft) => note.editor.move_left(shift),
            Key::Named(NamedKey::ArrowRight) => note.editor.move_right(shift),
            Key::Named(NamedKey::ArrowUp) => note.editor.move_up(width, shift),
            Key::Named(NamedKey::ArrowDown) => note.editor.move_down(width, shift),
            Key::Named(NamedKey::Home) => note.editor.move_home(shift),
            Key::Named(NamedKey::End) => note.editor.move_end(shift),
            Key::Character(text)
                if !self.mods.alt_key() && text.chars().any(|ch| !ch.is_control()) =>
            {
                note.editor.insert_text(text);
            }
            _ => {}
        }
        self.poke();
    }

    /// Draw the sheet after the rest of the frame, so it floats over the work.
    pub(super) fn draw_note(&mut self, screen: &mut Screen) {
        let Some(note) = self.note.as_mut() else {
            return;
        };
        if screen.cols() < 10 || screen.rows() < 5 {
            return;
        }
        let width = screen.cols().saturating_sub(2).clamp(8, NOTE_W);
        let height = screen.rows().saturating_sub(2).clamp(5, NOTE_H);
        let x = (screen.cols().saturating_sub(width)) / 2;
        let y = (screen.rows().saturating_sub(height)) / 2;
        let (paper, ink, accent, dim) = (
            self.theme.bg,
            self.theme.fg,
            self.theme.accent,
            self.theme.dim,
        );

        // A solid sheet, but only inside its own frame: the workflow around it
        // stays visible.
        for row in y..y + height {
            for col in x..x + width {
                screen.put(col, row, Cell::blank(ink, paper));
            }
        }
        screen.put(x, y, Cell::new('\u{250c}', accent, paper));
        screen.put(x + width - 1, y, Cell::new('\u{2510}', accent, paper));
        screen.put(x, y + height - 1, Cell::new('\u{2514}', accent, paper));
        screen.put(
            x + width - 1,
            y + height - 1,
            Cell::new('\u{2518}', accent, paper),
        );
        screen.hline(x + 1, y, width - 2, '\u{2500}', accent, paper);
        screen.hline(x + 1, y + height - 1, width - 2, '\u{2500}', accent, paper);
        for row in y + 1..y + height - 1 {
            screen.put(x, row, Cell::new('\u{2502}', accent, paper));
            screen.put(x + width - 1, row, Cell::new('\u{2502}', accent, paper));
        }
        screen.put_str(x + 2, y, " NOTE ", accent, paper);
        screen.put_str(x + 2, y + height - 1, " Esc saves ", dim, paper);

        let text_x = x + 1;
        let text_y = y + 1;
        let text_width = (width - 2).max(1) as usize;
        let text_height = (height - 2).max(1) as usize;
        note.editor.scroll_to_cursor(text_width, text_height, None);
        let rows = note.editor.visible_rows(text_width, text_height);
        let cursor = note.editor.cursor();
        let mut cursor_cell = None;
        for (index, row) in rows.iter().enumerate() {
            screen.put_str(text_x, text_y + index as u32, &row.text, ink, paper);
            if row.contains(cursor) {
                cursor_cell = Some((index, cursor.col - row.start));
            }
        }
        if let Some((row, col)) = cursor_cell {
            let ch = rows
                .get(row)
                .and_then(|row| row.text.chars().nth(col))
                .unwrap_or(' ');
            screen.put(
                text_x + col as u32,
                text_y + row as u32,
                Cell::new(ch, paper, ink),
            );
        }
    }
}

fn free_note_path(dir: &std::path::Path) -> PathBuf {
    let stamp = Local::now().format("%Y-%m-%d_%H-%M-%S");
    let mut path = dir.join(format!("note-{stamp}.md"));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("note-{stamp}-{n}.md"));
        n += 1;
    }
    path
}
