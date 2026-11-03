//! SCRIPTURE: a quiet, keyboard-first English Bible reader.
//!
//! The text is compiled in, one book at a time is parsed on demand, and the
//! reader draws only into the classic 80x25 text stage. No window, no GPU and
//! no network belong to this module.

mod books;

use std::path::{Path, PathBuf};

use cread_core::command::Command;
use cread_core::editor::wrap;
use cread_core::program::{KeyPress, Outcome, PointerEvent, PointerKind, Program};
use cread_core::scrollbar::{self, Scroll};
use cread_core::{Cell, Raster, Screen};

pub use books::BOOKS;

/// Left column width and the reading column inside the 80-column stage.
const SIDEBAR_X: u32 = 1;
const SIDEBAR_W: u32 = 19;
const DIVIDER_X: u32 = 20;
const TEXT_X: u32 = 22;
const TEXT_W: u32 = 56;
const TEXT_ROWS: usize = 20;
const SIDEBAR_ROWS: usize = 21;
const SCROLLBAR_X: u32 = 79;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Testament {
    Old,
    New,
}

pub struct Book {
    pub name: &'static str,
    pub testament: Testament,
    pub chapters: u16,
    pub text: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Verse {
    number: u16,
    text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Row {
    text: String,
    /// The verse number belongs to the first wrapped row only.
    number: Option<u16>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Focus {
    Books,
    Text,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SidebarLine {
    Group(&'static str),
    Book(usize),
}

pub struct Scripture {
    book: usize,
    chapter: usize,
    scroll: usize,
    focus: Focus,
    book_top: usize,
    /// Selected chapter in the chapter grid, zero-based.
    grid: Option<usize>,
    rows: Vec<Row>,
    /// One parsed book is enough: changing books is cheap and the cache keeps
    /// chapter changes instant.
    cache: Option<(usize, Vec<Vec<Verse>>)>,
    paper: u8,
    ink: u8,
    state_path: PathBuf,
    persist: bool,
}

impl Drop for Scripture {
    fn drop(&mut self) {
        self.save_state();
    }
}

impl Default for Scripture {
    fn default() -> Self {
        Self::new()
    }
}

impl Scripture {
    pub fn new() -> Self {
        Self::new_at(
            cread_core::storage::config_dir().join("scripture.state"),
            true,
        )
    }

    /// For tests and screenshots: choose the state file and decide whether a
    /// drop may write it.
    pub fn new_at(state_path: PathBuf, persist: bool) -> Self {
        let (book, chapter, scroll) = load_state(&state_path);
        let book = book.min(BOOKS.len().saturating_sub(1));
        let chapter = chapter.clamp(1, BOOKS[book].chapters.max(1) as usize);
        let mut scripture = Self {
            book,
            chapter,
            scroll,
            focus: Focus::Books,
            book_top: 0,
            grid: None,
            rows: Vec::new(),
            cache: None,
            paper: 15,
            ink: 0,
            state_path,
            persist,
        };
        scripture.rebuild();
        scripture.ensure_book_visible();
        scripture
    }

    pub fn book(&self) -> usize {
        self.book
    }

    pub fn chapter(&self) -> usize {
        self.chapter
    }

    pub fn scroll(&self) -> usize {
        self.scroll
    }

    fn save_state(&self) {
        if !self.persist {
            return;
        }
        let text = format!(
            "book={}\nchapter={}\nscroll={}\n",
            self.book, self.chapter, self.scroll
        );
        let _ = cread_core::storage::save_atomic(&self.state_path, &text);
    }

    fn manage_book(&mut self, next: usize) {
        if next >= BOOKS.len() || next == self.book {
            return;
        }
        self.book = next;
        self.chapter = 1;
        self.scroll = 0;
        self.cache = None;
        self.rebuild();
        self.ensure_book_visible();
    }

    fn change_chapter(&mut self, delta: isize) {
        let last = BOOKS[self.book].chapters.max(1) as isize;
        let next = (self.chapter as isize + delta).clamp(1, last) as usize;
        if next == self.chapter {
            return;
        }
        self.chapter = next;
        self.scroll = 0;
        self.rebuild();
    }

    fn set_chapter(&mut self, chapter: usize) {
        let last = BOOKS[self.book].chapters.max(1) as usize;
        self.chapter = chapter.clamp(1, last);
        self.scroll = 0;
        self.rebuild();
    }

    fn rebuild(&mut self) {
        if self.cache.as_ref().map(|(book, _)| *book) != Some(self.book) {
            let parsed = parse_book(BOOKS[self.book].text);
            self.cache = Some((self.book, parsed));
        }
        let verses = self
            .cache
            .as_ref()
            .and_then(|(_, books)| books.get(self.chapter.saturating_sub(1)));
        let mut rows = Vec::new();
        if let Some(verses) = verses {
            for verse in verses {
                let text = normalize(&verse.text);
                let segments = wrap(&text, TEXT_W as usize - 5);
                for (index, (start, end)) in segments.into_iter().enumerate() {
                    let row_text: String = text.chars().skip(start).take(end - start).collect();
                    rows.push(Row {
                        text: row_text,
                        number: (index == 0).then_some(verse.number),
                    });
                }
            }
        }
        if rows.is_empty() {
            rows.push(Row {
                text: "— empty —".into(),
                number: None,
            });
        }
        self.rows = rows;
        self.scroll = self.scroll.min(self.max_scroll());
    }

    fn max_scroll(&self) -> usize {
        self.rows.len().saturating_sub(TEXT_ROWS)
    }

    fn scroll_by(&mut self, delta: isize) {
        let max = self.max_scroll() as isize;
        self.scroll = (self.scroll as isize + delta).clamp(0, max) as usize;
    }

    fn open_grid(&mut self) {
        self.grid = Some(self.chapter.saturating_sub(1));
    }

    fn grid_key(&mut self, key: KeyPress) {
        let Some(selected) = self.grid else {
            return;
        };
        let chapters = BOOKS[self.book].chapters.max(1) as usize;
        let next = match key {
            KeyPress::Left => selected.saturating_sub(1),
            KeyPress::Right => (selected + 1).min(chapters - 1),
            KeyPress::Up => selected.saturating_sub(10),
            KeyPress::Down => (selected + 10).min(chapters - 1),
            KeyPress::PageUp => selected.saturating_sub(50),
            KeyPress::PageDown => (selected + 50).min(chapters - 1),
            KeyPress::Home => 0,
            KeyPress::End => chapters - 1,
            KeyPress::Enter => {
                self.grid = None;
                self.set_chapter(selected + 1);
                self.focus = Focus::Text;
                return;
            }
            KeyPress::Escape => {
                self.grid = None;
                return;
            }
            _ => return,
        };
        self.grid = Some(next);
    }

    fn sidebar_items() -> Vec<SidebarLine> {
        let mut items = vec![SidebarLine::Group("OLD TESTAMENT")];
        for (index, book) in BOOKS.iter().enumerate() {
            if book.testament == Testament::Old {
                items.push(SidebarLine::Book(index));
            }
        }
        items.push(SidebarLine::Group("NEW TESTAMENT"));
        for (index, book) in BOOKS.iter().enumerate() {
            if book.testament == Testament::New {
                items.push(SidebarLine::Book(index));
            }
        }
        items
    }

    fn ensure_book_visible(&mut self) {
        let items = Self::sidebar_items();
        let Some(selected) = items
            .iter()
            .position(|item| matches!(item, SidebarLine::Book(index) if *index == self.book))
        else {
            return;
        };
        let max_top = items.len().saturating_sub(SIDEBAR_ROWS);
        if selected < self.book_top {
            self.book_top = selected;
        } else if selected >= self.book_top + SIDEBAR_ROWS {
            self.book_top = selected + 1 - SIDEBAR_ROWS;
        }
        self.book_top = self.book_top.min(max_top);
    }

    fn draw_header(&self, screen: &mut Screen) {
        let chapter_count = BOOKS[self.book].chapters;
        let right = format!(
            "{} {}/{}",
            BOOKS[self.book].name.to_uppercase(),
            self.chapter,
            chapter_count
        );
        screen.put_str(1, 0, "SCRIPTURE", self.ink, self.paper);
        let x = screen
            .cols()
            .saturating_sub(right.chars().count() as u32 + 2);
        screen.put_str(x, 0, &right, self.ink, self.paper);
        screen.hline(1, 1, screen.cols().saturating_sub(2), '─', 8, self.paper);
    }

    fn draw_sidebar(&mut self, screen: &mut Screen) {
        self.ensure_book_visible();
        let items = Self::sidebar_items();
        for (row, item) in items
            .iter()
            .skip(self.book_top)
            .take(SIDEBAR_ROWS)
            .enumerate()
        {
            let y = 2 + row as u32;
            match item {
                SidebarLine::Group(name) => {
                    screen.put_str(SIDEBAR_X, y, name, 8, self.paper);
                }
                SidebarLine::Book(index) => {
                    let book = &BOOKS[*index];
                    let selected = *index == self.book;
                    let (fg, bg) = if selected {
                        (self.paper, self.ink)
                    } else {
                        (self.ink, self.paper)
                    };
                    if selected {
                        screen.hline(SIDEBAR_X, y, SIDEBAR_W, ' ', fg, bg);
                    }
                    screen.put_str_ellipsis(
                        SIDEBAR_X,
                        y,
                        book.name,
                        SIDEBAR_W.saturating_sub(1),
                        fg,
                        bg,
                    );
                }
            }
        }
        screen.hline(DIVIDER_X, 2, SIDEBAR_ROWS as u32, '│', 8, self.paper);
    }

    fn draw_text(&self, screen: &mut Screen) {
        let heading = format!("{} {}", BOOKS[self.book].name, self.chapter);
        screen.put_str(TEXT_X, 2, &heading, self.ink, self.paper);
        let last = 23u32;
        for (index, row) in self
            .rows
            .iter()
            .skip(self.scroll)
            .take(TEXT_ROWS)
            .enumerate()
        {
            let y = 3 + index as u32;
            if y >= last {
                break;
            }
            if let Some(number) = row.number {
                let prefix = format!("{number:>3}  ");
                screen.put_str(TEXT_X, y, &prefix, 8, self.paper);
            }
            screen.put_str_ellipsis(
                TEXT_X + 5,
                y,
                &row.text,
                TEXT_W.saturating_sub(5),
                self.ink,
                self.paper,
            );
        }
        scrollbar::draw_scrollbar(
            screen,
            SCROLLBAR_X,
            3,
            last - 1,
            Scroll {
                first: self.scroll,
                visible: TEXT_ROWS,
                total: self.rows.len(),
            },
            8,
            self.ink,
            self.paper,
        );
    }

    fn draw_footer(&self, screen: &mut Screen) {
        screen.fill_row(23, Cell::blank(8, self.paper));
        let hint = match self.focus {
            Focus::Books => "↑↓ book   ←→ chapter   Enter read   C chapters   Tab text",
            Focus::Text => "↑↓ scroll   ←→ chapter   Tab books   C chapters",
        };
        screen.put_str_ellipsis(1, 23, hint, screen.cols().saturating_sub(2), 8, self.paper);
    }

    fn draw_grid(&self, screen: &mut Screen, selected: usize) {
        screen.fill(Cell::blank(self.ink, self.paper));
        let title = format!("{}  ·  CHAPTER", BOOKS[self.book].name.to_uppercase());
        screen.put_str(1, 0, "SCRIPTURE", self.ink, self.paper);
        let x = screen
            .cols()
            .saturating_sub(title.chars().count() as u32 + 2);
        screen.put_str(x, 0, &title, self.ink, self.paper);
        screen.hline(1, 1, screen.cols().saturating_sub(2), '─', 8, self.paper);
        let chapters = BOOKS[self.book].chapters.max(1) as usize;
        for chapter in 1..=chapters {
            let index = chapter - 1;
            let col = (index % 10) as u32;
            let row = (index / 10) as u32;
            let x = 5 + col * 7;
            let y = 3 + row;
            let is_selected = index == selected;
            let (fg, bg) = if is_selected {
                (self.paper, self.ink)
            } else {
                (self.ink, self.paper)
            };
            if is_selected {
                screen.hline(x.saturating_sub(1), y, 5, ' ', fg, bg);
            }
            screen.put_str(x, y, &format!("{chapter:>3}"), fg, bg);
        }
        screen.fill_row(23, Cell::blank(8, self.paper));
        screen.put_str_ellipsis(
            1,
            23,
            "↑↓←→ choose   PgUp/PgDn jump   Enter open   Esc back",
            screen.cols().saturating_sub(2),
            8,
            self.paper,
        );
    }
}

impl Program for Scripture {
    fn title(&self) -> &str {
        "SCRIPTURE"
    }

    fn key(&mut self, key: KeyPress) -> Outcome {
        if self.grid.is_some() {
            self.grid_key(key);
            return Outcome::Continue;
        }
        match key {
            KeyPress::Char('c') => self.open_grid(),
            KeyPress::Char('b') => self.focus = Focus::Books,
            KeyPress::Tab => {
                self.focus = match self.focus {
                    Focus::Books => Focus::Text,
                    Focus::Text => Focus::Books,
                };
            }
            KeyPress::Up if self.focus == Focus::Books => {
                self.manage_book(self.book.saturating_sub(1))
            }
            KeyPress::Down if self.focus == Focus::Books => {
                self.manage_book((self.book + 1).min(BOOKS.len().saturating_sub(1)));
            }
            KeyPress::Up => self.scroll_by(-1),
            KeyPress::Down => self.scroll_by(1),
            KeyPress::PageUp => self.scroll_by(-(TEXT_ROWS as isize)),
            KeyPress::PageDown => self.scroll_by(TEXT_ROWS as isize),
            KeyPress::Home => {
                self.scroll = 0;
            }
            KeyPress::End => {
                self.scroll = self.max_scroll();
            }
            KeyPress::Left => self.change_chapter(-1),
            KeyPress::Right => self.change_chapter(1),
            KeyPress::Enter if self.focus == Focus::Books => self.focus = Focus::Text,
            _ => {}
        }
        Outcome::Continue
    }

    fn pointer(&mut self, event: PointerEvent) -> Outcome {
        if let PointerKind::Wheel(notches) = event.kind {
            self.scroll_by(-(notches as isize) * 3);
        }
        Outcome::Continue
    }

    fn draw(&mut self, screen: &mut Screen, _raster: &mut Raster) {
        screen.fill(Cell::blank(self.ink, self.paper));
        if let Some(selected) = self.grid {
            self.draw_grid(screen, selected);
            return;
        }
        self.draw_header(screen);
        self.draw_sidebar(screen);
        self.draw_text(screen);
        self.draw_footer(screen);
    }

    fn palette(&mut self, background: u8, ink: u8) {
        self.paper = background;
        self.ink = ink;
    }

    fn status(&self) -> String {
        format!("{} {}", BOOKS[self.book].name, self.chapter)
    }

    fn command(&mut self, _command: &Command) -> Result<String, String> {
        Err("SCRIPTURE does not know that command".into())
    }

    fn escape(&mut self) -> bool {
        if self.grid.take().is_some() {
            return true;
        }
        false
    }

    fn exit(&mut self) -> bool {
        self.save_state();
        false
    }
}

fn load_state(path: &Path) -> (usize, usize, usize) {
    let Ok(text) = std::fs::read_to_string(path) else {
        return (0, 1, 0);
    };
    let mut book = 0;
    let mut chapter = 1;
    let mut scroll = 0;
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key.trim() {
            "book" => book = value.trim().parse().unwrap_or(0),
            "chapter" => chapter = value.trim().parse().unwrap_or(1),
            "scroll" => scroll = value.trim().parse().unwrap_or(0),
            _ => {}
        }
    }
    (book, chapter, scroll)
}

fn normalize(text: &str) -> String {
    // The source uses typographic punctuation; the VGA bitmap font only has
    // the CP437 repertoire.
    text.replace('\u{2019}', "'").replace('\u{2014}', "-")
}

fn parse_book(text: &str) -> Vec<Vec<Verse>> {
    let mut chapters = Vec::new();
    let mut current: Option<usize> = None;
    for raw in text.lines() {
        let line = raw.trim_end();
        if line.starts_with("## Chapter ") {
            chapters.push(Vec::new());
            current = Some(chapters.len() - 1);
            continue;
        }
        let Some(index) = current else {
            continue;
        };
        let Some((number, body)) = line.split_once(". ") else {
            continue;
        };
        let Ok(number) = number.parse() else {
            continue;
        };
        chapters[index].push(Verse {
            number,
            text: normalize(body.trim()),
        });
    }
    chapters
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn reader() -> Scripture {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "cread-scripture-{}-{nonce}.state",
            std::process::id()
        ));
        Scripture::new_at(path, false)
    }

    #[test]
    fn the_english_canon_is_complete_and_ordered() {
        assert_eq!(BOOKS.len(), 66);
        assert_eq!(
            BOOKS
                .iter()
                .filter(|book| book.testament == Testament::Old)
                .count(),
            39
        );
        assert_eq!(
            BOOKS
                .iter()
                .filter(|book| book.testament == Testament::New)
                .count(),
            27
        );
        assert_eq!(BOOKS[0].name, "Genesis");
        assert_eq!(BOOKS[65].name, "Revelation of John");
    }

    #[test]
    fn genesis_has_the_expected_chapter_and_verse_counts() {
        let chapters = parse_book(BOOKS[0].text);
        assert_eq!(chapters.len(), 50);
        assert_eq!(chapters[0].len(), 31);
        assert!(chapters[0][0].text.contains("In the beginning"));
        assert!(chapters[0][0].text.contains("God created"));
    }

    #[test]
    fn psalm_119_has_176_verses() {
        let psalms = BOOKS.iter().position(|book| book.name == "Psalms").unwrap();
        let chapters = parse_book(BOOKS[psalms].text);
        assert_eq!(chapters[118].len(), 176);
    }

    #[test]
    fn typographic_punctuation_becomes_font_safe() {
        assert_eq!(normalize("Lord’s — day"), "Lord's - day");
    }

    #[test]
    fn chapter_navigation_clamps_and_resets_scroll() {
        let mut reader = reader();
        reader.set_chapter(2);
        reader.scroll = 10;
        reader.change_chapter(-1);
        assert_eq!(reader.chapter, 1);
        assert_eq!(reader.scroll, 0);
        reader.set_chapter(50);
        assert_eq!(reader.chapter, 50);
        reader.change_chapter(1);
        assert_eq!(reader.chapter, 50);
    }

    #[test]
    fn book_navigation_loads_the_first_chapter() {
        let mut reader = reader();
        reader.focus = Focus::Books;
        reader.manage_book(1);
        assert_eq!(reader.book, 1);
        assert_eq!(reader.chapter, 1);
        assert_eq!(reader.rows.first().unwrap().number, Some(1));
    }

    #[test]
    fn the_scrollbar_appears_for_a_long_chapter_and_not_for_a_short_one() {
        let mut reader = reader();
        reader.set_chapter(1);
        let mut screen = Screen::new(80, 25);
        let mut raster = Raster::new(640, 368);
        reader.draw(&mut screen, &mut raster);
        assert_eq!(screen.get(79, 3).unwrap().ch, '\u{2588}');

        let mut short = Scripture::new_at(reader.state_path.clone(), false);
        short.set_chapter(1);
        short.rows.truncate(TEXT_ROWS);
        let mut screen = Screen::new(80, 25);
        short.draw(&mut screen, &mut raster);
        assert_eq!(screen.get(79, 3).unwrap().ch, ' ');
    }

    #[test]
    fn the_drawer_stays_inside_the_stage() {
        let mut reader = reader();
        let mut screen = Screen::new(80, 25);
        let mut raster = Raster::new(640, 368);
        reader.draw(&mut screen, &mut raster);
        let text: String = (0..25)
            .flat_map(|y| (0..80).map(move |x| (x, y)))
            .map(|(x, y)| screen.get(x, y).unwrap().ch)
            .collect();
        assert!(text.contains("SCRIPTURE"));
        assert!(text.contains("GENESIS"));
        assert!(text.contains("OLD TESTAMENT"));
        assert!(text.contains("In the beginning"));
    }

    #[test]
    fn the_chapter_grid_opens_and_selects() {
        let mut reader = reader();
        reader.key(KeyPress::Char('c'));
        assert!(reader.grid.is_some());
        reader.key(KeyPress::Right);
        reader.key(KeyPress::Enter);
        assert_eq!(reader.chapter, 2);
        assert!(reader.grid.is_none());

        reader.key(KeyPress::Char('c'));
        assert!(reader.grid.is_some());
        assert!(reader.escape(), "Escape closes the grid");
        assert!(reader.grid.is_none());
    }
}
