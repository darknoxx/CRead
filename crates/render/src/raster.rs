//! CPU rasteriser: turns a text-mode [`Screen`] into the emulated framebuffer.
//!
//! Blitting a 640x400 grid of bitmap glyphs is a few hundred microseconds, and
//! it only happens when the text actually changes — so the GPU is free to do
//! nothing but the CRT post-processing.

use cread_core::{Cell, Rgb, Screen, Theme};

use crate::font::{Font, GLYPH_H, GLYPH_W};

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

        for y in 0..screen.rows() {
            for x in 0..screen.cols() {
                let Some(cell) = screen.get(x, y) else {
                    continue;
                };
                self.draw_cell(cell, x as i32 * GLYPH_W as i32, y as i32 * GLYPH_H as i32, theme, font);
            }
        }
    }

    fn draw_cell(&mut self, cell: &Cell, x: i32, y: i32, theme: &Theme, font: &Font) {
        self.fill_rect(x, y, GLYPH_W as u32, GLYPH_H as u32, theme.color(cell.bg));

        let slot = font.slot(cell.ch);
        if slot == font.slot(' ') {
            return;
        }
        let fg = theme.color(cell.fg);
        for row in 0..GLYPH_H {
            let bits = font.scanline(slot, cell.style, row);
            if bits == 0 {
                continue;
            }
            for col in 0..GLYPH_W {
                // Bit 7 is the leftmost pixel, matching the ROM layout.
                if bits & (0x80 >> col) != 0 {
                    self.put(x + col as i32, y + row as i32, fg);
                }
            }
        }
    }

    #[inline]
    fn put(&mut self, x: i32, y: i32, rgb: Rgb) {
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
pub fn write_png(
    path: impl AsRef<std::path::Path>,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> std::io::Result<()> {
    let file = std::fs::File::create(path)?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    writer
        .write_image_data(rgba)
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cread_core::{Cell, Style};

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
        screen.put(0, 0, Cell::new('A', 15, 1, Style::Normal));
        fb.draw_screen(&screen, &solid_theme(), &Font::new());
        // 'A' row 2 is "...#...." -> only x=3 is lit.
        for x in 0..8 {
            let idx = ((2 * 8 + x) * 4) as usize;
            let lit = fb.data()[idx] == 255;
            assert_eq!(lit, x == 3, "x={x}");
        }
    }

    #[test]
    fn resizing_does_not_leave_stale_pixels() {
        let mut fb = Framebuffer::new(4, 4);
        fb.clear([1, 2, 3]);
        fb.resize(8, 8);
        assert_eq!(fb.data().len(), 8 * 8 * 4);
        fb.clear([9, 9, 9]);
        assert!(fb.data().as_chunks::<4>().0.iter().all(|p| *p == [9, 9, 9, 255]));
    }
}
