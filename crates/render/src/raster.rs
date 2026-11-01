//! CPU rasteriser: turns a text-mode [`Screen`] into the emulated framebuffer.
//!
//! Blitting a 640x400 grid of bitmap glyphs is a few hundred microseconds, and
//! it only happens when the text actually changes — so the GPU is free to do
//! nothing but the CRT post-processing.

use cread_core::raster::CLEAR;
use cread_core::{Cell, Raster, Rgb, Screen, Theme, VGA};

use crate::font::Font;

/// RGBA8 image of the emulated machine's video output.
pub struct Framebuffer {
    width: u32,
    height: u32,
    data: Vec<u8>,
}

impl Framebuffer {
    pub fn new(width: u32, height: u32) -> Self {
        let mut fb = Self {
            width: 0,
            height: 0,
            data: Vec::new(),
        };
        fb.resize(width, height);
        fb
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if self.width == width && self.height == height {
            return;
        }
        self.width = width;
        self.height = height;
        self.data = vec![0; (width * height * 4) as usize];
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    pub fn clear(&mut self, rgb: Rgb) {
        let px = [rgb[0], rgb[1], rgb[2], 0xff];
        for chunk in self.data.as_chunks_mut::<4>().0 {
            chunk.copy_from_slice(&px);
        }
    }

    /// Fill an axis-aligned rectangle, clipped to the framebuffer.
    pub fn fill_rect(&mut self, x: i32, y: i32, w: u32, h: u32, rgb: Rgb) {
        let x0 = x.max(0) as u32;
        let y0 = y.max(0) as u32;
        let x1 = (x + w as i32).min(self.width as i32).max(0) as u32;
        let y1 = (y + h as i32).min(self.height as i32).max(0) as u32;
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let px = [rgb[0], rgb[1], rgb[2], 0xff];
        for row in y0..y1 {
            let start = ((row * self.width + x0) * 4) as usize;
            let end = ((row * self.width + x1) * 4) as usize;
            for chunk in self.data[start..end].as_chunks_mut::<4>().0 {
                chunk.copy_from_slice(&px);
            }
        }
    }

    /// Draw the whole screen, filling any leftover margin with the theme background.
    pub fn draw_screen(&mut self, screen: &Screen, theme: &Theme, font: &Font) {
        self.clear(theme.background());
        self.draw_cells(screen, theme, font, 0, 0);
    }

    /// Draw a program: the pixel layer first, then the cell grid on top.
    ///
    /// Cells marked transparent are skipped, which is what lets a program put a
    /// menu over its own picture without punching a hole in it.
    pub fn draw_program(&mut self, screen: &Screen, raster: &Raster, theme: &Theme, font: &Font) {
        self.draw_program_at(screen, raster, theme, font, 0, 0);
    }

    /// The same, with the pixel layer starting at `(x, y)`.
    ///
    /// Programs are written for the classic 640x400 screen; on a larger
    /// framebuffer the shell centres that stage instead of leaving it in the
    /// top-left corner.
    pub fn draw_program_at(
        &mut self,
        screen: &Screen,
        raster: &Raster,
        theme: &Theme,
        font: &Font,
        x: i32,
        y: i32,
    ) {
        // The page first: a program paints only what it means to paint, and
        // pixels it left clear show this colour.
        self.clear(theme.background());
        self.blit_at(raster, x, y);
        self.draw_cells(screen, theme, font, 0, 0);
    }

    /// A full-screen pixel layer with no cell grid and a fixed background.
    ///
    /// The screensaver uses it: it owns every pixel and is theme-agnostic, so
    /// the page colour is passed in rather than taken from the theme.
    pub fn draw_canvas(&mut self, raster: &Raster, background: Rgb) {
        self.clear(background);
        self.blit(raster);
    }

    /// Copy the 16-colour raster into the RGBA framebuffer.
    ///
    /// Row-wise conversion: 245k pixels, once per frame, so the per-pixel
    /// bounds check of `put` would show up in a pointer-driven program.
    fn blit(&mut self, raster: &Raster) {
        self.blit_at(raster, 0, 0);
    }

    fn blit_at(&mut self, raster: &Raster, ox: i32, oy: i32) {
        let rows = raster.height().min(self.height);
        let pixels = raster.pixels();
        // The visible part of the row: the raster may hang off either edge.
        let start = ox.max(0) as u32;
        let end = (ox + raster.width() as i32).min(self.width as i32);
        if end <= start as i32 {
            return;
        }
        let count = (end - start as i32) as usize;
        for y in 0..rows {
            let dest_y = oy + y as i32;
            if dest_y < 0 || dest_y >= self.height as i32 {
                continue;
            }
            let src_start = (start as i32 - ox) as usize;
            let src = &pixels[(y * raster.width()) as usize + src_start..][..count];
            let base = ((dest_y as u32 * self.width + start) as usize) * 4;
            let dst = &mut self.data[base..base + count * 4];
            for (index, chunk) in src.iter().zip(dst.as_chunks_mut::<4>().0) {
                if *index == CLEAR {
                    // Nobody painted this pixel: the background stays.
                    continue;
                }
                let rgb = VGA[(*index & 0x0f) as usize];
                *chunk = [rgb[0], rgb[1], rgb[2], 0xff];
            }
        }
    }

    fn draw_cells(&mut self, screen: &Screen, theme: &Theme, font: &Font, ox: i32, oy: i32) {
        for y in 0..screen.rows() {
            for x in 0..screen.cols() {
                let Some(cell) = screen.get(x, y) else {
                    continue;
                };
                if cell.is_transparent() {
                    continue;
                }
                self.draw_cell(
                    cell,
                    ox + x as i32 * font.cell_w() as i32,
                    oy + y as i32 * font.cell_h() as i32,
                    theme,
                    font,
                );
            }
        }
    }

    fn draw_cell(&mut self, cell: &Cell, x: i32, y: i32, theme: &Theme, font: &Font) {
        let (cell_w, cell_h) = (font.cell_w(), font.cell_h());
        if cell.has_background() {
            self.fill_rect(x, y, cell_w, cell_h, theme.color(cell.bg));
        }

        let slot = font.slot(cell.ch);
        if slot == font.slot(' ') {
            return;
        }
        let fg = theme.color(cell.fg);
        for row in 0..cell_h as usize {
            let bits = font.scanline(slot, row);
            if bits == 0 {
                continue;
            }
            for col in 0..cell_w as usize {
                // Bit 7 is the leftmost pixel, matching the ROM layout.
                if bits & (0x80 >> col) != 0 {
                    self.put(x + col as i32, y + row as i32, fg);
                }
            }
        }
    }

    /// Colour of one pixel, for tests.
    pub fn pixel(&self, x: i32, y: i32) -> Rgb {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return [0, 0, 0];
        }
        let idx = ((y as u32 * self.width + x as u32) * 4) as usize;
        [self.data[idx], self.data[idx + 1], self.data[idx + 2]]
    }

    #[inline]
    pub(crate) fn put(&mut self, x: i32, y: i32, rgb: Rgb) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return;
        }
        let idx = ((y as u32 * self.width + x as u32) * 4) as usize;
        self.data[idx] = rgb[0];
        self.data[idx + 1] = rgb[1];
        self.data[idx + 2] = rgb[2];
        self.data[idx + 3] = 0xff;
    }

    /// Encode the framebuffer as a PNG. Used by `--screenshot` and by the
    /// visual regression tests.
    pub fn to_png(&self, path: impl AsRef<std::path::Path>) -> std::io::Result<()> {
        write_png(path, self.width, self.height, &self.data)
    }
}

/// Write tightly packed RGBA8 pixels as a PNG.
///
/// Screenshots and pictures use the same encoder; it lives in core so the
/// picture format and the screenshot share one implementation.
pub fn write_png(
    path: impl AsRef<std::path::Path>,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> std::io::Result<()> {
    cread_core::write_png(path, width, height, rgba)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cread_core::Cell;

    fn solid_theme() -> Theme {
        let mut theme = Theme::DOS_BLUE;
        theme.palette[1] = [0, 0, 0];
        theme.palette[15] = [255, 255, 255];
        theme
    }

    #[test]
    fn background_fills_the_whole_raster() {
        let mut fb = Framebuffer::new(8 * 3 + 2, 16 + 4);
        let screen = Screen::new(3, 1);
        fb.draw_screen(&screen, &solid_theme(), &Font::new());
        // Last column is beyond the grid and must still be background.
        let idx = ((19 * fb.width() + 25) * 4) as usize;
        assert_eq!(&fb.data()[idx..idx + 3], &[0, 0, 0]);
        assert_eq!(fb.data()[idx + 3], 0xff);
    }

    #[test]
    fn a_glyph_puts_ink_on_the_right_pixels() {
        let mut fb = Framebuffer::new(8, 16);
        let mut screen = Screen::new(1, 1);
        screen.put(0, 0, Cell::new('A', 15, 1));
        fb.draw_screen(&screen, &solid_theme(), &Font::new());
        // 'A' row 2 is "...#...." -> only x=3 is lit.
        for x in 0..8 {
            let idx = ((2 * 8 + x) * 4) as usize;
            let lit = fb.data()[idx] == 255;
            assert_eq!(lit, x == 3, "x={x}");
        }
    }

    #[test]
    fn an_ink_cell_leaves_the_pixels_around_the_glyph_alone() {
        let mut fb = Framebuffer::new(16, 16);
        let mut raster = Raster::new(16, 16);
        raster.fill(4); // red
        let mut screen = Screen::new(2, 1);
        screen.fill(Cell::TRANSPARENT);
        screen.put(0, 0, Cell::ink('A', 15));

        fb.draw_program(&screen, &raster, &solid_theme(), &Font::new());

        // The glyph itself is white, its cell background is still the picture.
        assert_eq!(fb.pixel(0, 0), [170, 0, 0]);
        assert_eq!(fb.pixel(3, 2), [255, 255, 255]);
        assert_eq!(fb.pixel(7, 15), [170, 0, 0]);
    }

    #[test]
    fn a_program_paints_the_pixel_layer_under_its_cells() {
        let mut fb = Framebuffer::new(16, 32);
        let mut raster = Raster::new(16, 32);
        raster.fill(4); // red, in VGA order
        let mut screen = Screen::new(2, 2);
        screen.fill(Cell::TRANSPARENT);
        screen.put(0, 0, Cell::new('A', 15, 1));

        fb.draw_program(&screen, &raster, &solid_theme(), &Font::new());

        assert_eq!(fb.pixel(8, 0), [170, 0, 0], "transparent cell shows pixels");
        // Inside the glyph cell the theme background wins.
        assert_eq!(fb.pixel(0, 0), [0, 0, 0]);
        assert_eq!(fb.pixel(15, 31), [170, 0, 0]);
    }

    #[test]
    fn bench_program_blit() {
        let mut fb = Framebuffer::new(640, 400);
        let raster = Raster::new(640, 384);
        let screen = Screen::new(80, 25);
        let theme = solid_theme();
        let font = Font::new();
        let start = std::time::Instant::now();
        for _ in 0..50 {
            fb.draw_program(&screen, &raster, &theme, &font);
        }
        eprintln!("draw_program: {:?}/frame", start.elapsed() / 50);
    }

    #[test]
    fn a_program_can_start_below_a_top_bar() {
        let mut fb = Framebuffer::new(64, 64);
        let mut raster = Raster::new(8, 8);
        raster.fill(15);
        // A program's cell layer is transparent; the top bar would be cells.
        let mut screen = Screen::new(8, 4);
        screen.fill(Cell::TRANSPARENT);

        fb.draw_program_at(&screen, &raster, &Theme::BLACK, &Font::new(), 4, 16);
        assert_eq!(fb.pixel(0, 16), Theme::BLACK.background(), "left of it");
        assert_eq!(fb.pixel(4, 15), Theme::BLACK.background(), "the bar's row");
        assert_eq!(fb.pixel(4, 16), VGA[15], "the canvas starts one row down");
        assert_eq!(fb.pixel(10, 23), VGA[15], "and keeps its height");
        assert_eq!(fb.pixel(12, 23), Theme::BLACK.background(), "right of it");
    }

    #[test]
    fn a_canvas_owns_the_whole_screen() {
        let mut fb = Framebuffer::new(64, 32);
        let mut raster = Raster::new(8, 8);
        raster.fill(0);
        raster.set(1, 1, 15);

        fb.draw_canvas(&raster, [0, 0, 0]);
        assert_eq!(fb.pixel(1, 1), VGA[15], "the painted pixel");
        assert_eq!(fb.pixel(0, 0), [0, 0, 0], "the fixed black page");
        assert_eq!(fb.pixel(63, 31), [0, 0, 0], "edge to edge, no cell grid");
    }

    #[test]
    fn resizing_does_not_leave_stale_pixels() {
        let mut fb = Framebuffer::new(4, 4);
        fb.clear([1, 2, 3]);
        fb.resize(8, 8);
        assert_eq!(fb.data().len(), 8 * 8 * 4);
        fb.clear([9, 9, 9]);
        assert!(
            fb.data()
                .as_chunks::<4>()
                .0
                .iter()
                .all(|p| *p == [9, 9, 9, 255])
        );
    }
}
