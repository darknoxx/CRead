//! `cread-render`: emulated framebuffer, bitmap font and the CRT effect chain.

pub mod font;
pub mod gpu;
pub mod raster;
pub mod splash;

pub use font::{Font, GLYPH_H, GLYPH_W};
pub use gpu::{CrtSettings, FrameStatus, Renderer};
pub use raster::Framebuffer;
pub use splash::{Splash, SplashStyle};
