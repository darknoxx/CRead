//! The boot logo, drawn straight into the emulated framebuffer.
//!
//! It is pixel art on a transparent background, so it is blitted at its native
//! size in *emulated* pixels: the CRT upscale then keeps the hard pixel edges.

use cread_core::Rgb;

use crate::raster::Framebuffer;

/// How to colour the image.
pub enum SplashStyle {
    /// Keep the artwork's own colours, for a colour logo.
    Native,
    /// Paint the artwork's *dark* pixels in one ink colour and let the light
    /// ones show the background through. The logo is black line art with white
    /// counters, so darkness is the coverage and luminance is the anti-aliasing.
    Ink(Rgb),
}

pub struct Splash {
    width: u32,
    height: u32,
    /// RGBA8, row major.
    pixels: Vec<u8>,
}

impl Splash {
    pub fn from_png(bytes: &[u8]) -> Result<Self, String> {
        let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
        decoder.set_transformations(png::Transformations::ALPHA | png::Transformations::EXPAND);
        let mut reader = decoder.read_info().map_err(|e| format!("splash: {e}"))?;

        let size = reader
            .output_buffer_size()
            .ok_or("splash: image too large")?;
        let mut buffer = vec![0; size];
        let info = reader
            .next_frame(&mut buffer)
            .map_err(|e| format!("splash: {e}"))?;
        buffer.truncate(info.buffer_size());

        if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
            return Err(format!(
                "splash: expected RGBA8, got {:?}/{:?}",
                info.color_type, info.bit_depth
            ));
        }

        Ok(Self {
            width: info.width,
            height: info.height,
            pixels: buffer,
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }
}

impl Framebuffer {
    /// Blit the logo with its top-left corner at (`x`, `y`) in emulated pixels.
    ///
    /// `divisor` shrinks it by a whole factor when the raster is too small;
    /// fractional scaling would blur the pixel art, so the caller picks an
    /// integer and we sample every n-th pixel instead.
    pub fn draw_splash(
        &mut self,
        splash: &Splash,
        x: i32,
        y: i32,
        divisor: u32,
        style: &SplashStyle,
    ) {
        let step = divisor.max(1);
        for py in (0..splash.height).step_by(step as usize) {
            for px in (0..splash.width).step_by(step as usize) {
                let idx = ((py * splash.width + px) * 4) as usize;
                let alpha = splash.pixels[idx + 3];
                if alpha < 128 {
                    continue;
                }
                let source = [
                    splash.pixels[idx],
                    splash.pixels[idx + 1],
                    splash.pixels[idx + 2],
                ];
                let dx = x + (px / step) as i32;
                let dy = y + (py / step) as i32;
                match style {
                    SplashStyle::Native => self.put(dx, dy, source),
                    SplashStyle::Ink(ink) => {
                        // Darkness is the coverage: the artwork is black line
                        // art with white counters, so white must show the
                        // background through rather than paint over it.
                        let luma = (0.2126 * source[0] as f32
                            + 0.7152 * source[1] as f32
                            + 0.0722 * source[2] as f32)
                            / 255.0;
                        let coverage = (1.0 - luma) * (alpha as f32 / 255.0);
                        if coverage <= 0.02 {
                            continue;
                        }
                        let under = self.pixel(dx, dy);
                        let blend =
                            |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * coverage) as u8;
                        self.put(
                            dx,
                            dy,
                            [
                                blend(under[0], ink[0]),
                                blend(under[1], ink[1]),
                                blend(under[2], ink[2]),
                            ],
                        );
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 2x1 image: one opaque white pixel, one fully transparent.
    fn tiny_png() -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut out, 2, 1);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer
                .write_image_data(&[255, 255, 255, 255, 255, 255, 255, 0])
                .unwrap();
        }
        out
    }

    #[test]
    fn decodes_size_and_skips_transparent_pixels() {
        let splash = Splash::from_png(&tiny_png()).unwrap();
        assert_eq!((splash.width(), splash.height()), (2, 1));

        let mut fb = Framebuffer::new(4, 4);
        fb.clear([0, 0, 0]);
        fb.draw_splash(&splash, 1, 1, 1, &SplashStyle::Native);

        // Opaque pixel painted, transparent one left alone.
        assert_eq!(fb.pixel(1, 1), [255, 255, 255]);
        assert_eq!(fb.pixel(2, 1), [0, 0, 0]);
        assert_eq!(fb.pixel(0, 0), [0, 0, 0]);
    }

    #[test]
    fn ink_uses_darkness_as_coverage() {
        let splash = Splash::from_png(&tiny_png()).unwrap();
        let mut fb = Framebuffer::new(2, 1);
        fb.clear([241, 237, 227]);
        // The test image is one white pixel (ink 0) and one transparent one.
        fb.draw_splash(&splash, 0, 0, 1, &SplashStyle::Ink([20, 20, 20]));
        assert_eq!(fb.pixel(0, 0), [241, 237, 227], "white stays background");
        assert_eq!(fb.pixel(1, 0), [241, 237, 227], "transparent untouched");
    }

    #[test]
    fn rejects_images_that_are_not_rgba8() {
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut out, 1, 1);
            encoder.set_color(png::ColorType::Grayscale);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&[7]).unwrap();
        }
        // Grayscale + ALPHA transformation produces GrayAlpha, which we reject.
        assert!(Splash::from_png(&out).is_err());
    }
}
