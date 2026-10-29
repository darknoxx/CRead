//! `cread-core`: UI-agnostic text-editor data structures.
//!
//! Nothing in here touches a GPU, a window or a network socket, which keeps the
//! tricky parts (layout, wrapping, undo, sync bookkeeping) unit-testable.

pub mod resolution;
pub mod screen;
pub mod theme;

pub use resolution::{FONT_H, FONT_W, ScaleMode, Viewport};
pub use screen::{Cell, Screen, Style};
pub use theme::{Rgb, Theme};
