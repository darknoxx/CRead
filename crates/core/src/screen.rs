//! The text-mode screen: a `cols x rows` grid of cells, like VGA text memory.

/// One character cell: code point plus a VGA-style foreground/background pair.
///
/// There is deliberately no weight or slant: this is a 1-bit face at 8x16 and
/// synthesised bold and italic only ever made the glyphs harder to read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    pub ch: char,
    pub fg: u8,
    pub bg: u8,
}

impl Cell {
    pub const fn new(ch: char, fg: u8, bg: u8) -> Self {
        Self { ch, fg, bg }
    }

    /// A blank cell with the given colours.
    pub const fn blank(fg: u8, bg: u8) -> Self {
        Self::new(' ', fg, bg)
    }

    /// A cell that draws nothing, so the pixel layer underneath shows through.
    /// Programs fill the grid with this and then put their menus on top.
    pub const TRANSPARENT: Cell = Self::new('\0', 0, 0);

    pub const fn is_transparent(&self) -> bool {
        self.ch == '\0'
    }
}

impl Default for Cell {
    fn default() -> Self {
        Self::new(' ', 7, 0)
    }
}

/// A fixed-size grid of cells.
#[derive(Clone, Debug)]
pub struct Screen {
    cols: u32,
    rows: u32,
    cells: Vec<Cell>,
}

impl Screen {
    pub fn new(cols: u32, rows: u32) -> Self {
        let cells = vec![Cell::default(); (cols * rows) as usize];
        Self { cols, rows, cells }
    }

    pub fn cols(&self) -> u32 {
        self.cols
    }

    pub fn rows(&self) -> u32 {
        self.rows
    }

    pub fn cells(&self) -> &[Cell] {
        &self.cells
    }

    /// Resize, preserving as much content as still fits.
    pub fn resize(&mut self, cols: u32, rows: u32) {
        if cols == self.cols && rows == self.rows {
            return;
        }
        let mut next = Screen::new(cols, rows);
        for y in 0..rows.min(self.rows) {
            for x in 0..cols.min(self.cols) {
                next.cells[(y * cols + x) as usize] = self.cells[(y * self.cols + x) as usize];
            }
        }
        *self = next;
    }

    pub fn in_bounds(&self, x: u32, y: u32) -> bool {
        x < self.cols && y < self.rows
    }

    pub fn get(&self, x: u32, y: u32) -> Option<&Cell> {
        self.in_bounds(x, y)
            .then(|| &self.cells[(y * self.cols + x) as usize])
    }

    pub fn put(&mut self, x: u32, y: u32, cell: Cell) {
        if self.in_bounds(x, y) {
            self.cells[(y * self.cols + x) as usize] = cell;
        }
    }

    /// Fill the whole screen with one cell.
    pub fn fill(&mut self, cell: Cell) {
        self.cells.fill(cell);
    }

    /// Fill a single row.
    pub fn fill_row(&mut self, y: u32, cell: Cell) {
        if y < self.rows {
            let start = (y * self.cols) as usize;
            self.cells[start..start + self.cols as usize].fill(cell);
        }
    }

    /// Write a string, clipping at the right edge. Returns the next free column.
    pub fn put_str(&mut self, x: u32, y: u32, text: &str, fg: u8, bg: u8) -> u32 {
        let mut cx = x;
        for ch in text.chars() {
            if cx >= self.cols {
                break;
            }
            self.put(cx, y, Cell::new(ch, fg, bg));
            cx += 1;
        }
        cx
    }

    /// Like [`Screen::put_str`] but clips to `width` cells with a `>` marker.
    #[allow(clippy::too_many_arguments)]
    pub fn put_str_ellipsis(&mut self, x: u32, y: u32, text: &str, width: u32, fg: u8, bg: u8) {
        let count = text.chars().count() as u32;
        if count <= width {
            self.put_str(x, y, text, fg, bg);
            return;
        }
        let keep = width.saturating_sub(1);
        let clipped: String = text.chars().take(keep as usize).collect();
        let end = self.put_str(x, y, &clipped, fg, bg);
        self.put(end, y, Cell::new('>', fg, bg));
    }

    /// Horizontal rule made of a repeated character.
    pub fn hline(&mut self, x: u32, y: u32, len: u32, ch: char, fg: u8, bg: u8) {
        for cx in x..x + len {
            self.put(cx, y, Cell::new(ch, fg, bg));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_str_clips_at_right_edge() {
        let mut s = Screen::new(5, 2);
        assert_eq!(s.put_str(3, 0, "abcdef", 7, 1), 5);
        assert_eq!(s.get(4, 0).unwrap().ch, 'b');
        assert_eq!(s.get(0, 0).unwrap().ch, ' ');
    }

    #[test]
    fn resize_keeps_overlap() {
        let mut s = Screen::new(4, 2);
        s.put_str(0, 0, "abcd", 7, 1);
        s.resize(2, 1);
        assert_eq!(s.get(1, 0).unwrap().ch, 'b');
        assert!(s.get(2, 0).is_none());
    }

    #[test]
    fn ellipsis_helper_marks_overflow() {
        let mut s = Screen::new(4, 1);
        s.put_str_ellipsis(0, 0, "abcdefgh", 4, 7, 1);
        let text: String = (0..4).map(|x| s.get(x, 0).unwrap().ch).collect();
        assert_eq!(text, "abc>");
    }
}
