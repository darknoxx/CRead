//! The address field in the frame's top edge: one line of text and a caret.
//!
//! It opens with the current address selected, like a browser's: the first
//! character typed replaces it, an arrow key keeps it and moves instead.

use cread_core::KeyPress;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Field {
    text: Vec<char>,
    caret: usize,
    /// Everything is selected; typing replaces it.
    selected: bool,
}

/// What a key did to the field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Keep editing.
    Edit,
    /// Enter: go to what is typed.
    Go(String),
}

impl Field {
    /// Start editing `text`, all of it selected.
    pub fn open(text: &str) -> Self {
        let text: Vec<char> = text.chars().collect();
        Self {
            caret: text.len(),
            text,
            selected: true,
        }
    }

    pub fn text(&self) -> String {
        self.text.iter().collect()
    }

    pub fn is_selected(&self) -> bool {
        self.selected && !self.text.is_empty()
    }

    fn take_selection(&mut self) {
        if self.selected {
            self.text.clear();
            self.caret = 0;
            self.selected = false;
        }
    }

    /// Typed or pasted text. Line breaks become spaces: an address is one line.
    pub fn insert(&mut self, text: &str) {
        self.take_selection();
        for ch in text.chars() {
            let ch = if ch == '\n' || ch == '\r' || ch == '\t' {
                ' '
            } else {
                ch
            };
            if ch.is_control() {
                continue;
            }
            self.text.insert(self.caret, ch);
            self.caret += 1;
        }
    }

    pub fn key(&mut self, key: KeyPress) -> Action {
        match key {
            KeyPress::Enter => return Action::Go(self.text().trim().to_string()),
            KeyPress::Char(ch) => self.insert(&ch.to_string()),
            KeyPress::Ctrl('a') => {
                self.selected = true;
                self.caret = self.text.len();
            }
            KeyPress::Backspace | KeyPress::Delete if self.selected => self.take_selection(),
            KeyPress::Backspace => {
                if self.caret > 0 {
                    self.caret -= 1;
                    self.text.remove(self.caret);
                }
            }
            KeyPress::Delete => {
                if self.caret < self.text.len() {
                    self.text.remove(self.caret);
                }
            }
            KeyPress::Left => {
                self.selected = false;
                self.caret = self.caret.saturating_sub(1);
            }
            KeyPress::Right => {
                self.selected = false;
                self.caret = (self.caret + 1).min(self.text.len());
            }
            KeyPress::Home => {
                self.selected = false;
                self.caret = 0;
            }
            KeyPress::End => {
                self.selected = false;
                self.caret = self.text.len();
            }
            _ => {}
        }
        Action::Edit
    }

    /// The part that fits in `width` cells, and where the caret is in it. The
    /// view scrolls so the caret is always visible, one cell to spare.
    pub fn view(&self, width: usize) -> (String, usize) {
        if width == 0 {
            return (String::new(), 0);
        }
        let start = (self.caret + 1).saturating_sub(width);
        let shown: String = self.text.iter().skip(start).take(width).collect();
        (shown, self.caret - start)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(field: &mut Field, text: &str) {
        for ch in text.chars() {
            field.key(KeyPress::Char(ch));
        }
    }

    #[test]
    fn typing_replaces_the_old_address() {
        let mut field = Field::open("https://example.com/");
        assert!(field.is_selected());
        typed(&mut field, "wiki");
        assert_eq!(field.text(), "wiki");
        assert_eq!(field.key(KeyPress::Enter), Action::Go("wiki".into()));
    }

    #[test]
    fn an_arrow_keeps_the_address_and_edits_it() {
        let mut field = Field::open("example.com");
        field.key(KeyPress::End);
        typed(&mut field, "/a");
        assert_eq!(field.text(), "example.com/a");
        field.key(KeyPress::Home);
        field.key(KeyPress::Delete);
        assert_eq!(field.text(), "xample.com/a");
        field.key(KeyPress::Right);
        field.key(KeyPress::Backspace);
        assert_eq!(field.text(), "ample.com/a");
    }

    #[test]
    fn backspace_on_a_selection_clears_it() {
        let mut field = Field::open("example.com");
        field.key(KeyPress::Backspace);
        assert_eq!(field.text(), "");
        field.key(KeyPress::Backspace);
        assert_eq!(field.text(), "");
    }

    #[test]
    fn a_paste_is_one_line() {
        let mut field = Field::open("");
        field.insert("ibm\nps/2\u{7}");
        assert_eq!(field.text(), "ibm ps/2");
    }

    #[test]
    fn the_view_follows_the_caret() {
        let mut field = Field::open("");
        typed(&mut field, "abcdefghij");
        assert_eq!(field.view(4), ("hij".into(), 3));
        field.key(KeyPress::Home);
        assert_eq!(field.view(4), ("abcd".into(), 0));
        assert_eq!(Field::open("ab").view(10), ("ab".into(), 2));
    }

    #[test]
    fn enter_trims() {
        let mut field = Field::open("");
        typed(&mut field, "  example.com ");
        assert_eq!(field.key(KeyPress::Enter), Action::Go("example.com".into()));
    }
}
