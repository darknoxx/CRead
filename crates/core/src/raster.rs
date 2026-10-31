//! A 16-colour indexed pixel surface, the second layer of the emulated machine.
//!
//! The text screen is an 80x25 grid of glyphs; programs that draw pictures need
//! real pixels. Both share the same 640x400 raster, so a program can put a menu
//! in cells on top of its picture and everything still goes through one CRT
//! pass.

use crate::Rgb;

/// The fixed 16 colours every picture uses, in VGA order.
///
/// Deliberately not the theme: a saved picture must look the same whichever
/// theme CRead is dressed in.
pub const VGA: [Rgb; 16] = [
    [0, 0, 0],
    [0, 0, 170],
    [0, 170, 0],
    [0, 170, 170],
    [170, 0, 0],
    [170, 0, 170],
    [170, 85, 0],
    [170, 170, 170],
    [85, 85, 85],
    [85, 85, 255],
    [85, 255, 85],
    [85, 255, 255],
    [255, 85, 85],
    [255, 85, 255],
    [255, 255, 85],
    [255, 255, 255],
];

/// Palette index of the paper a new picture starts on.
pub const PAPER: u8 = 15;

/// A pixel nobody painted: the compositor leaves the page showing there.
pub const CLEAR: u8 = 0xff;

/// A grid of palette indices.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Raster {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl Raster {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            pixels: vec![PAPER; (width * height) as usize],
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    pub fn in_bounds(&self, x: i32, y: i32) -> bool {
        x >= 0 && y >= 0 && (x as u32) < self.width && (y as u32) < self.height
    }

    pub fn get(&self, x: i32, y: i32) -> Option<u8> {
        self.in_bounds(x, y)
            .then(|| self.pixels[(y as u32 * self.width + x as u32) as usize])
    }

    pub fn set(&mut self, x: i32, y: i32, index: u8) {
        if self.in_bounds(x, y) {
            self.pixels[(y as u32 * self.width + x as u32) as usize] = index;
        }
    }

    pub fn fill(&mut self, index: u8) {
        self.pixels.fill(index);
    }

    /// Leave every pixel to the page colour underneath.
    pub fn fill_clear(&mut self) {
        self.pixels.fill(CLEAR);
    }

    /// A horizontal run, clipped to the surface.
    pub fn hline(&mut self, x: i32, y: i32, len: u32, index: u8) {
        for cx in x..x + len as i32 {
            self.set(cx, y, index);
        }
    }

    /// A filled square, centred on `(x, y)`: the brush.
    pub fn dab(&mut self, x: i32, y: i32, size: u32, index: u8) {
        let size = size.max(1) as i32;
        let start = -(size / 2);
        for dy in start..start + size {
            for dx in start..start + size {
                self.set(x + dx, y + dy, index);
            }
        }
    }

    /// Bresenham, so a fast drag leaves no gaps.
    pub fn line(&mut self, from: (i32, i32), to: (i32, i32), size: u32, index: u8) {
        let (x0, y0) = from;
        let (x1, y1) = to;
        let dx = (x1 - x0).abs();
        let dy = -(y1 - y0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let sy = if y0 < y1 { 1 } else { -1 };
        let (mut x, mut y, mut err) = (x0, y0, dx + dy);
        loop {
            self.dab(x, y, size, index);
            if x == x1 && y == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x += sx;
            }
            if e2 <= dx {
                err += dx;
                y += sy;
            }
        }
    }

    /// A rectangle between two corners, outlined or filled.
    pub fn rect(&mut self, from: (i32, i32), to: (i32, i32), size: u32, index: u8, filled: bool) {
        let (x0, x1) = (from.0.min(to.0), from.0.max(to.0));
        let (y0, y1) = (from.1.min(to.1), from.1.max(to.1));
        if filled {
            for y in y0..=y1 {
                for x in x0..=x1 {
                    self.set(x, y, index);
                }
            }
        } else {
            self.line((x0, y0), (x1, y0), size, index);
            self.line((x1, y0), (x1, y1), size, index);
            self.line((x1, y1), (x0, y1), size, index);
            self.line((x0, y1), (x0, y0), size, index);
        }
    }

    /// Flood the area of `(x, y)` with `index`. Returns the number of pixels
    /// painted, so a caller can tell a no-op click from a real edit.
    pub fn flood(&mut self, x: i32, y: i32, index: u8) -> usize {
        let Some(target) = self.get(x, y) else {
            return 0;
        };
        if target == index {
            return 0;
        }
        let mut stack = vec![(x, y)];
        let mut painted = 0;
        while let Some((cx, cy)) = stack.pop() {
            if self.get(cx, cy) != Some(target) {
                continue;
            }
            // Extend left and right, then queue the rows above and below.
            let mut left = cx;
            while self.get(left - 1, cy) == Some(target) {
                left -= 1;
            }
            let mut right = cx;
            while self.get(right + 1, cy) == Some(target) {
                right += 1;
            }
            for px in left..=right {
                self.set(px, cy, index);
                painted += 1;
            }
            for py in [cy - 1, cy + 1] {
                let mut run = false;
                for px in left..=right {
                    let same = self.get(px, py) == Some(target);
                    if same && !run {
                        stack.push((px, py));
                    }
                    run = same;
                }
            }
        }
        painted
    }

    /// Copy another surface into this one at `(x, y)`, clipped.
    ///
    /// Row-wise, because this runs on every frame of a program that copies its
    /// canvas into the composited surface.
    pub fn copy_from(&mut self, src: &Raster, x: i32, y: i32) {
        // Horizontal overlap, computed once for every row.
        let skip = (-x).max(0) as u32;
        let dx = (x + skip as i32).max(0) as u32;
        let count = src
            .width
            .saturating_sub(skip)
            .min(self.width.saturating_sub(dx));
        if count == 0 {
            return;
        }
        for sy in 0..src.height() {
            let dy = y + sy as i32;
            if dy < 0 || dy as u32 >= self.height {
                continue;
            }
            let from = (sy * src.width + skip) as usize;
            let to = (dy as u32 * self.width + dx) as usize;
            let n = count as usize;
            self.pixels[to..to + n].copy_from_slice(&src.pixels[from..from + n]);
        }
    }

    /// Index of the palette entry closest to an RGB triple.
    pub fn nearest(rgb: Rgb) -> u8 {
        let mut best = (u32::MAX, 0u8);
        for (index, entry) in VGA.iter().enumerate() {
            let dr = entry[0] as i32 - rgb[0] as i32;
            let dg = entry[1] as i32 - rgb[1] as i32;
            let db = entry[2] as i32 - rgb[2] as i32;
            let distance = (dr * dr + dg * dg + db * db) as u32;
            if distance < best.0 {
                best = (distance, index as u8);
            }
        }
        best.1
    }

    /// RGBA8 pixels, the shape a PNG wants.
    pub fn to_rgba(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.pixels.len() * 4);
        for &index in &self.pixels {
            let rgb = VGA[(index & 0x0f) as usize];
            out.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 0xff]);
        }
        out
    }

    /// Build a surface from RGBA8 pixels, mapping each colour to the palette.
    pub fn from_rgba(width: u32, height: u32, rgba: &[u8]) -> Self {
        let mut raster = Raster::new(width, height);
        for y in 0..height {
            for x in 0..width {
                let at = ((y * width + x) * 4) as usize;
                let Some(pixel) = rgba.get(at..at + 4) else {
                    continue;
                };
                let index = Raster::nearest([pixel[0], pixel[1], pixel[2]]);
                raster.set(x as i32, y as i32, index);
            }
        }
        raster
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_surface_is_paper_and_clips_writes() {
        let mut raster = Raster::new(4, 3);
        assert_eq!(raster.get(0, 0), Some(PAPER));
        raster.set(-1, 0, 3);
        raster.set(4, 0, 3);
        raster.set(0, 3, 3);
        assert!(raster.pixels().iter().all(|&p| p == PAPER));
        raster.set(1, 1, 4);
        assert_eq!(raster.get(1, 1), Some(4));
    }

    #[test]
    fn a_brush_is_a_square_around_the_point() {
        let mut raster = Raster::new(5, 5);
        raster.dab(2, 2, 3, 1);
        let painted: Vec<_> = (0..5)
            .flat_map(|y| (0..5).map(move |x| (x, y)))
            .filter(|&(x, y)| raster.get(x, y) == Some(1))
            .collect();
        assert_eq!(painted.len(), 9);
        assert_eq!(raster.get(1, 1), Some(1));
        assert_eq!(raster.get(0, 0), Some(PAPER));
    }

    #[test]
    fn a_line_has_no_gaps() {
        let mut raster = Raster::new(10, 10);
        raster.line((0, 0), (9, 9), 1, 2);
        for i in 0..10 {
            assert_eq!(raster.get(i, i), Some(2), "diagonal at {i}");
        }
    }

    #[test]
    fn an_outlined_rectangle_leaves_the_middle_alone() {
        let mut raster = Raster::new(6, 6);
        raster.rect((1, 1), (4, 4), 1, 3, false);
        assert_eq!(raster.get(1, 1), Some(3));
        assert_eq!(raster.get(4, 4), Some(3));
        assert_eq!(raster.get(2, 2), Some(PAPER));
    }

    #[test]
    fn flood_fills_one_region_and_stops_at_an_edge() {
        let mut raster = Raster::new(6, 3);
        // A wall down the middle.
        raster.hline(3, 0, 1, 0);
        raster.hline(3, 1, 1, 0);
        raster.hline(3, 2, 1, 0);
        let painted = raster.flood(0, 0, 5);
        assert_eq!(painted, 9, "three rows of three free pixels");
        assert_eq!(raster.get(2, 2), Some(5));
        assert_eq!(raster.get(4, 0), Some(PAPER), "the other side stays");
        assert_eq!(raster.get(3, 1), Some(0), "the wall stays");
    }

    #[test]
    fn flood_on_the_same_colour_does_nothing() {
        let mut raster = Raster::new(3, 3);
        assert_eq!(raster.flood(1, 1, PAPER), 0);
    }

    #[test]
    fn rgba_round_trips_through_the_palette() {
        let mut raster = Raster::new(3, 2);
        raster.set(0, 0, 4);
        raster.set(2, 1, 12);
        let rgba = raster.to_rgba();
        assert_eq!(Raster::from_rgba(3, 2, &rgba), raster);
    }

    #[test]
    fn nearest_snaps_to_the_palette() {
        assert_eq!(Raster::nearest([250, 250, 250]), 15);
        assert_eq!(Raster::nearest([3, 3, 3]), 0);
        assert_eq!(Raster::nearest([200, 20, 20]), 4);
    }

    #[test]
    fn copy_from_clips_to_the_destination() {
        let src = Raster::new(3, 3);
        let mut dst = Raster::new(2, 2);
        dst.fill(0);
        dst.copy_from(&src, 1, 1);
        assert_eq!(dst.get(1, 1), Some(PAPER));
        assert_eq!(dst.get(0, 0), Some(0));
    }
}
