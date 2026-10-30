//! The IBM VGA 8x16 bitmap font, plus the handful of glyphs CP437 never had.
//!
//! The face is stored as raw CP437 glyph data (256 x 16 bytes, one bit per
//! pixel, MSB leftmost) extracted from the original video ROM dump. Extended
//! glyphs we need for modern text are hand-drawn below.

use std::collections::HashMap;

use cread_core::Style;

pub const GLYPH_W: usize = 8;
pub const GLYPH_H: usize = 16;

/// The classic VGA face: sturdy strokes, the one everyone remembers.
const VGA_GLYPHS: &[u8; 256 * GLYPH_H] = include_bytes!("../../../assets/fonts/ibm_vga_8x16.bin");

/// The ISO 9241 variant from the same pack. Same cell, noticeably thinner
/// strokes — 22.7 inked pixels per glyph instead of 33.
const ISO_GLYPHS: &[u8; 256 * GLYPH_H] = include_bytes!("../../../assets/fonts/ibm_dos_iso8.bin");

/// The original 8x8 BIOS face: half the cell, so twice as many device pixels
/// per scanline at the same apparent text size. That is what makes a beam
/// profile possible on a modern display.
const CGA_GLYPHS: &[u8; 256 * 8] = include_bytes!("../../../assets/fonts/ibm_vga_8x8.bin");

/// Which bitmap face to draw with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Face {
    /// IBM VGA 8x16.
    #[default]
    Classic,
    /// IBM DOS ISO8: the same size, lighter strokes.
    Thin,
    /// IBM BIOS 8x8: half the cell height, so the raster holds twice as many
    /// rows — or, at the same apparent size, twice the scanline resolution.
    Cga,
}

impl Face {
    pub fn name(self) -> &'static str {
        match self {
            Face::Classic => "classic",
            Face::Thin => "thin",
            Face::Cga => "cga",
        }
    }

    /// Cell size in emulated pixels.
    pub fn cell(self) -> (u32, u32) {
        match self {
            Face::Cga => (8, 8),
            Face::Classic | Face::Thin => (8, 16),
        }
    }

    pub fn by_name(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "classic" | "vga" | "normal" => Some(Face::Classic),
            "thin" | "iso" | "light" => Some(Face::Thin),
            "cga" | "bios" | "8x8" => Some(Face::Cga),
            _ => None,
        }
    }

    fn glyphs(self) -> &'static [u8] {
        match self {
            Face::Classic => VGA_GLYPHS,
            Face::Thin => ISO_GLYPHS,
            Face::Cga => CGA_GLYPHS,
        }
    }

    fn rows(self) -> usize {
        self.cell().1 as usize
    }
}

/// CP437 slot for each Unicode scalar. Index == CP437 code.
///
/// The low control range holds the classic box/symbol glyphs; `0x20..=0x7e` is
/// plain ASCII and `0x80..` is the IBM extended set.
#[rustfmt::skip]
const CP437_UNICODE: [char; 256] = [
    '\u{0}',    '☺', '☻', '♥', '♦', '♣', '♠', '•', '◘', '○', '◙', '♂', '♀', '♪', '♫', '☼',
    '►', '◄', '↕', '‼', '¶', '§', '▬', '↨', '↑', '↓', '→', '←', '∟', '↔', '▲', '▼',
    ' ', '!', '"', '#', '$', '%', '&', '\'', '(', ')', '*', '+', ',', '-', '.', '/',
    '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', ':', ';', '<', '=', '>', '?',
    '@', 'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I', 'J', 'K', 'L', 'M', 'N', 'O',
    'P', 'Q', 'R', 'S', 'T', 'U', 'V', 'W', 'X', 'Y', 'Z', '[', '\\', ']', '^', '_',
    '`', 'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l', 'm', 'n', 'o',
    'p', 'q', 'r', 's', 't', 'u', 'v', 'w', 'x', 'y', 'z', '{', '|', '}', '~', '⌂',
    'Ç', 'ü', 'é', 'â', 'ä', 'à', 'å', 'ç', 'ê', 'ë', 'è', 'ï', 'î', 'ì', 'Ä', 'Å',
    'É', 'æ', 'Æ', 'ô', 'ö', 'ò', 'û', 'ù', 'ÿ', 'Ö', 'Ü', '¢', '£', '¥', '₧', 'ƒ',
    'á', 'í', 'ó', 'ú', 'ñ', 'Ñ', 'ª', 'º', '¿', '⌐', '¬', '½', '¼', '¡', '«', '»',
    '░', '▒', '▓', '│', '┤', '╡', '╢', '╖', '╕', '╣', '║', '╗', '╝', '╜', '╛', '┐',
    '└', '┴', '┬', '├', '─', '┼', '╞', '╟', '╚', '╔', '╩', '╦', '╠', '═', '╬', '╧',
    '╨', '╤', '╥', '╙', '╘', '╒', '╓', '╫', '╪', '┘', '┌', '█', '▄', '▌', '▐', '▀',
    'α', 'ß', 'Γ', 'π', 'Σ', 'σ', 'µ', 'τ', 'Φ', 'Θ', 'Ω', 'δ', '∞', 'φ', 'ε', '∩',
    '≡', '±', '≥', '≤', '⌠', '⌡', '÷', '≈', '°', '∙', '·', '√', 'ⁿ', '²', '■', '\u{a0}',
];

/// Glyphs missing from CP437 that normal prose needs.
///
/// Drawn on the same 8x16 grid; `#` is ink. Written out as text so they stay
/// editable without a hex editor.
#[rustfmt::skip]
const EXTRA_GLYPHS: &[(char, [&str; 16])] = &[
    ('€', [
        "........", "........", ".....##.", "....#...",
        "...#....", "..#####.", "...#....", "..#####.",
        "...#....", "....#...", ".....##.", "........",
        "........", "........", "........", "........",
    ]),
    ('„', [
        "........", "........", "........", "........",
        "........", "........", "........", "........",
        "........", "........", "..##.##.", "..##.##.",
        ".#...#..", "........", "........", "........",
    ]),
    ('“', [
        "........", "..##..##", ".##...##", "........",
        "........", "........", "........", "........",
        "........", "........", "........", "........",
        "........", "........", "........", "........",
    ]),
    ('”', [
        "........", ".##..##.", "..##..##", "........",
        "........", "........", "........", "........",
        "........", "........", "........", "........",
        "........", "........", "........", "........",
    ]),
    ('‘', [
        "........", "...#....", "..#.....", "........",
        "........", "........", "........", "........",
        "........", "........", "........", "........",
        "........", "........", "........", "........",
    ]),
    ('’', [
        "........", "...#....", "....#...", "........",
        "........", "........", "........", "........",
        "........", "........", "........", "........",
        "........", "........", "........", "........",
    ]),
    ('–', [
        "........", "........", "........", "........",
        "........", "........", "........", "..###...",
        "........", "........", "........", "........",
        "........", "........", "........", "........",
    ]),
    ('—', [
        "........", "........", "........", "........",
        "........", "........", "........", ".######.",
        "........", "........", "........", "........",
        "........", "........", "........", "........",
    ]),
    ('…', [
        "........", "........", "........", "........",
        "........", "........", "........", "........",
        "........", "........", "........", ".#..#..#",
        "........", "........", "........", "........",
    ]),
];

/// The same extras drawn for the 8x8 face.
#[rustfmt::skip]
const EXTRA_GLYPHS_8: &[(char, [&str; 8])] = &[
    ('€', [
        "........", "...##...", "..#.....", ".####...",
        "..#.....", ".####...", "...#....", "........",
    ]),
    ('„', [
        "........", "........", "........", "........",
        "..##.##.", "..##.##.", ".#...#..", "........",
    ]),
    ('“', [
        "..##..##", ".##...##", "........", "........",
        "........", "........", "........", "........",
    ]),
    ('”', [
        ".##..##.", "..##..##", "........", "........",
        "........", "........", "........", "........",
    ]),
    ('‘', [
        "...#....", "..#.....", "........", "........",
        "........", "........", "........", "........",
    ]),
    ('’', [
        "...#....", "....#...", "........", "........",
        "........", "........", "........", "........",
    ]),
    ('–', [
        "........", "........", "..###...", "........",
        "........", "........", "........", "........",
    ]),
    ('—', [
        "........", "........", ".#####..", "........",
        "........", "........", "........", "........",
    ]),
    ('…', [
        "........", "........", "........", "........",
        "........", "........", ".#..#..#", "........",
    ]),
];

const EXTRA_BASE: u16 = 256;

/// Bitmap face with Unicode lookup and style synthesis.
pub struct Font {
    face: Face,
    cell_w: u32,
    cell_h: u32,
    /// 256 CP437 glyphs followed by the hand-drawn extras, 16 scanlines each.
    glyphs: Vec<u8>,
    slots: HashMap<char, u16>,
    /// Slot used for any character we have no bitmap for.
    fallback: u16,
}

impl Font {
    pub fn new() -> Self {
        Self::with_face(Face::Classic)
    }

    pub fn with_face(face: Face) -> Self {
        let mut glyphs = face.glyphs().to_vec();
        let mut slots = HashMap::with_capacity(300);
        for (cp, &ch) in CP437_UNICODE.iter().enumerate() {
            // Skip NUL so a stray control char renders as the fallback instead.
            if cp == 0 {
                continue;
            }
            slots.entry(ch).or_insert(cp as u16);
        }

        // The hand-drawn extras exist at both cell heights; they are separate
        // tables because their arrays differ in length.
        if face.rows() == 8 {
            for (idx, (ch, rows)) in EXTRA_GLYPHS_8.iter().enumerate() {
                glyphs.extend_from_slice(&parse_glyph(rows));
                slots.insert(*ch, EXTRA_BASE + idx as u16);
            }
        } else {
            for (idx, (ch, rows)) in EXTRA_GLYPHS.iter().enumerate() {
                glyphs.extend_from_slice(&parse_glyph(rows));
                slots.insert(*ch, EXTRA_BASE + idx as u16);
            }
        }

        let (cell_w, cell_h) = face.cell();
        Self {
            face,
            cell_w,
            cell_h,
            glyphs,
            slots,
            fallback: b'?' as u16,
        }
    }

    pub fn cell_w(&self) -> u32 {
        self.cell_w
    }

    pub fn cell_h(&self) -> u32 {
        self.cell_h
    }

    pub fn face(&self) -> Face {
        self.face
    }

    /// Glyph slot for a character, falling back to `?`.
    pub fn slot(&self, ch: char) -> u16 {
        *self.slots.get(&ch).unwrap_or(&self.fallback)
    }

    /// True when the character has a real bitmap (used by tests/diagnostics).
    pub fn has_glyph(&self, ch: char) -> bool {
        self.slots.contains_key(&ch)
    }

    /// Scanline `row` (0 = top) of a glyph, already synthesised into `style`.
    #[inline]
    pub fn scanline(&self, slot: u16, style: Style, row: usize) -> u8 {
        let base = slot as usize * self.cell_h as usize + row;
        let bits = self.glyphs[base];
        match style {
            Style::Normal => bits,
            // Double-strike to the right: the classic way to fake bold in
            // single-bit fonts, and what DOS-era editors actually did.
            Style::Bold => bits | (bits >> 1),
            // Shear: shift further right as we move down the cell. Two steps of
            // one pixel keep the slant without the staircase gaps that three or
            // four steps cut into an 8px glyph.
            Style::Italic => {
                let shift = (row / 6) as u32;
                ((bits as u32) >> shift) as u8
            }
        }
    }
}

impl Default for Font {
    fn default() -> Self {
        Self::new()
    }
}

fn parse_glyph<const N: usize>(rows: &[&str; N]) -> Vec<u8> {
    let mut out = Vec::with_capacity(N);
    for row in rows.iter() {
        debug_assert_eq!(row.chars().count(), GLYPH_W, "bad glyph row: {row}");
        let mut bits = 0u8;
        for ch in row.chars() {
            bits <<= 1;
            if ch != '.' && ch != ' ' {
                bits |= 1;
            }
        }
        out.push(bits);
    }
    out
}

/// CP437 slot that reads as a given character, for tests and debugging.
pub fn cp437_slot(ch: char) -> Option<u16> {
    CP437_UNICODE
        .iter()
        .position(|&c| c == ch)
        .map(|p| p as u16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cp437_table_is_bijective() {
        let mut seen = std::collections::HashSet::new();
        for &ch in CP437_UNICODE.iter().skip(1) {
            assert!(seen.insert(ch), "duplicate CP437 entry {ch:?}");
        }
    }

    #[test]
    fn german_letters_exist() {
        let f = Font::new();
        for ch in "äöüÄÖÜß€„“”–—…".chars() {
            assert!(f.has_glyph(ch), "missing glyph {ch:?}");
        }
    }

    #[test]
    fn unknown_chars_fall_back() {
        let f = Font::new();
        assert!(!f.has_glyph('\u{1F600}'));
        assert_eq!(f.slot('\u{1F600}'), f.slot('?'));
    }

    #[test]
    fn the_thin_face_uses_less_ink_but_keeps_the_same_grid() {
        let classic = Font::with_face(Face::Classic);
        let thin = Font::with_face(Face::Thin);
        let ink = |f: &Font| -> u32 {
            (0..256)
                .flat_map(|slot| (0..GLYPH_H).map(move |row| (slot as u16, row)))
                .map(|(slot, row)| f.scanline(slot, Style::Normal, row).count_ones())
                .sum()
        };
        assert!(
            ink(&thin) < ink(&classic) * 9 / 10,
            "thin should be lighter"
        );
        assert_eq!(thin.face(), Face::Thin);
    }

    #[test]
    fn faces_resolve_by_name() {
        assert_eq!(Face::by_name("thin"), Some(Face::Thin));
        assert_eq!(Face::by_name("ISO"), Some(Face::Thin));
        assert_eq!(Face::by_name("classic"), Some(Face::Classic));
        assert_eq!(Face::by_name("nope"), None);
    }

    #[test]
    fn a_looks_like_an_a() {
        let f = Font::new();
        let rows: Vec<u8> = (0..GLYPH_H)
            .map(|r| f.scanline(f.slot('A'), Style::Normal, r))
            .collect();
        assert_eq!(rows[2], 0b0001_0000, "apex");
        assert_eq!(rows[7], 0b1111_1110, "crossbar");
    }

    #[test]
    fn bold_is_wider_than_normal_and_italic_is_sheared() {
        let f = Font::new();
        let slot = f.slot('H');
        let n: Vec<u8> = (0..GLYPH_H)
            .map(|r| f.scanline(slot, Style::Normal, r))
            .collect();
        let b: Vec<u8> = (0..GLYPH_H)
            .map(|r| f.scanline(slot, Style::Bold, r))
            .collect();
        let i: Vec<u8> = (0..GLYPH_H)
            .map(|r| f.scanline(slot, Style::Italic, r))
            .collect();
        assert!(
            b.iter()
                .zip(&n)
                .all(|(bb, nn)| bb & nn == *nn && bb.count_ones() >= nn.count_ones())
        );
        assert_ne!(i, n);
        // The top row of an upright 'H' stays put; lower rows move right.
        assert_eq!(i[0], n[0]);
        assert_ne!(i[8], n[8]);
    }
}
