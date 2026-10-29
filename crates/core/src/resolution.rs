//! Integer-scaled virtual raster: the heart of the CRT look.
//!
//! The emulated machine never scales by a fractional factor. We instead pick an
//! integer `scale`, render at `physical / scale`, and blow the result up by
//! exactly `scale`, centring it in the physical surface. Every emulated pixel
//! therefore covers an exact `scale x scale` block of real pixels, which is what
//! keeps scanlines crisp on any display.

/// Width of one character cell in emulated pixels (IBM VGA 8x16).
pub const FONT_W: u32 = 8;
/// Height of one character cell in emulated pixels (IBM VGA 8x16).
pub const FONT_H: u32 = 16;

/// Smallest raster we are willing to run in, in character cells.
const MIN_COLS: u32 = 40;
const MIN_ROWS: u32 = 12;

/// How the integer upscale factor is chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScaleMode {
    /// Pick the factor that lands closest to a target physical line height.
    ///
    /// `target_line_px` is the wanted height of one text line in physical
    /// pixels; the caller usually derives it from the monitor scale factor so a
    /// 4K/200% display gets the same apparent text size as a 1080p/100% one.
    Auto { target_line_px: u32 },
    /// Explicit factor; clamped down until the raster still fits the surface.
    Fixed(u32),
}

impl ScaleMode {
    /// Default heuristic: ~32 physical pixels per text line.
    pub const DEFAULT: ScaleMode = ScaleMode::Auto { target_line_px: 32 };
}

/// A fully resolved emulated display: scale factor, raster size and the
/// physical-pixel rectangle it occupies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Viewport {
    /// Physical surface size the viewport was computed for.
    pub phys_w: u32,
    pub phys_h: u32,
    /// Integer upscale factor; each emulated pixel is `scale x scale` real ones.
    pub scale: u32,
    /// Emulated raster size in pixels.
    pub virt_w: u32,
    pub virt_h: u32,
    /// Physical offset of the top-left emulated pixel (letterbox border).
    pub offset_x: u32,
    pub offset_y: u32,
}

impl Viewport {
    /// Resolve a viewport for a physical surface size.
    ///
    /// Degenerate surfaces (zero in either axis) yield `None`; the caller should
    /// simply skip rendering in that frame.
    pub fn compute(phys_w: u32, phys_h: u32, mode: ScaleMode) -> Option<Self> {
        if phys_w < FONT_W * MIN_COLS || phys_h < FONT_H * MIN_ROWS {
            return None;
        }

        let max_by_w = phys_w / (FONT_W * MIN_COLS);
        let max_by_h = phys_h / (FONT_H * MIN_ROWS);
        let max_scale = max_by_w.min(max_by_h).max(1);

        let wanted = match mode {
            ScaleMode::Fixed(s) => s.max(1),
            ScaleMode::Auto { target_line_px } => {
                // Round to the nearest whole factor; `+ FONT_H / 2` keeps .5 up.
                ((target_line_px + FONT_H / 2) / FONT_H).max(1)
            }
        };

        let scale = wanted.clamp(1, max_scale);
        let virt_w = phys_w / scale;
        let virt_h = phys_h / scale;

        Some(Self {
            phys_w,
            phys_h,
            scale,
            virt_w,
            virt_h,
            // `virt * scale` can be a few pixels short of `phys`; centre it.
            offset_x: (phys_w - virt_w * scale) / 2,
            offset_y: (phys_h - virt_h * scale) / 2,
        })
    }

    /// Text grid width in character cells.
    pub fn cols(&self) -> u32 {
        self.virt_w / FONT_W
    }

    /// Text grid height in character cells.
    pub fn rows(&self) -> u32 {
        self.virt_h / FONT_H
    }

    /// Physical size of the upscaled raster, in real pixels.
    pub fn scaled_size(&self) -> (u32, u32) {
        (self.virt_w * self.scale, self.virt_h * self.scale)
    }

    /// True when the raster fills the surface exactly (no letterbox border).
    pub fn is_exact(&self) -> bool {
        self.virt_w * self.scale == self.phys_w && self.virt_h * self.scale == self.phys_h
    }
}

/// Make `Auto` independent of the monitor scale factor for a given DPI.
pub fn target_line_px_for_scale_factor(scale_factor: f64) -> u32 {
    // 32px at 100%, 64px at 200%, but never silly.
    ((32.0 * scale_factor.max(1.0)).round() as u32).clamp(16, 128)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn auto(phys_w: u32, phys_h: u32) -> Viewport {
        Viewport::compute(phys_w, phys_h, ScaleMode::DEFAULT).unwrap()
    }

    #[test]
    fn classic_vga_on_the_1280x800_vm() {
        let vp = auto(1280, 800);
        assert_eq!((vp.scale, vp.virt_w, vp.virt_h), (2, 640, 400));
        assert_eq!((vp.cols(), vp.rows()), (80, 25));
        assert!(vp.is_exact());
        assert_eq!((vp.offset_x, vp.offset_y), (0, 0));
    }

    #[test]
    fn common_desktop_sizes() {
        for (w, h, scale) in [
            (1920, 1080, 2),
            (2560, 1440, 2),
            (2560, 1080, 2),
            (1920, 1200, 2),
            (1366, 768, 2),
            (1024, 768, 2),
        ] {
            let vp = auto(w, h);
            assert_eq!(vp.scale, scale, "{w}x{h}");
            assert!(vp.is_exact(), "{w}x{h} should divide cleanly");
        }
    }

    #[test]
    fn high_dpi_keeps_apparent_text_size() {
        // 4K at 200% should pick a bigger factor than 4K at 100%.
        let normal = Viewport::compute(3840, 2160, ScaleMode::Fixed(2)).unwrap();
        let hidpi = Viewport::compute(
            3840,
            2160,
            ScaleMode::Auto {
                target_line_px: target_line_px_for_scale_factor(2.0),
            },
        )
        .unwrap();
        assert_eq!(normal.scale, 2);
        assert_eq!(hidpi.scale, 4);
        assert_eq!((hidpi.cols(), hidpi.rows()), (120, 33));
    }

    #[test]
    fn odd_sizes_still_align_integrally() {
        for w in (640..=4000).step_by(7) {
            for h in (480..=2400).step_by(11) {
                for mode in [ScaleMode::Fixed(1), ScaleMode::Fixed(3), ScaleMode::DEFAULT] {
                    let vp = Viewport::compute(w, h, mode).unwrap();
                    let (sw, sh) = vp.scaled_size();
                    assert!(sw <= w && sh <= h, "{w}x{h} {mode:?}");
                    assert!(w - sw < vp.scale, "border must be thinner than one pixel");
                    assert!(h - sh < vp.scale);
                    assert_eq!(vp.offset_x, (w - sw) / 2);
                    assert_eq!(vp.offset_y, (h - sh) / 2);
                }
            }
        }
    }

    #[test]
    fn tiny_surfaces_are_rejected() {
        assert!(Viewport::compute(0, 0, ScaleMode::DEFAULT).is_none());
        assert!(Viewport::compute(100, 100, ScaleMode::DEFAULT).is_none());
    }

    #[test]
    fn fixed_scale_is_clamped_to_fit() {
        // Asking for 8x on a small screen must not overflow.
        let vp = Viewport::compute(800, 600, ScaleMode::Fixed(8)).unwrap();
        assert!(vp.scale < 8);
        assert!(vp.is_exact() || vp.virt_w * vp.scale <= 800);
    }
}
