//! `cread-core`: UI-agnostic text-editor data structures.
//!
//! Nothing in here touches a GPU, a window or a network socket, which keeps the
//! tricky parts (layout, wrapping, undo, sync bookkeeping) unit-testable.

pub mod command;
pub mod editor;
pub mod markdown;
pub mod prefs;
pub mod resolution;
pub mod screen;
pub mod storage;
pub mod theme;

pub use command::{Command, CommandError};
pub use editor::{Editor, Position, Stats, VisualRow};
pub use markdown::{Kind, Span};
pub use prefs::Prefs;
pub use resolution::{FONT_H, FONT_W, ScaleMode, Viewport};
pub use screen::{Cell, Screen};
pub use theme::{Rgb, Theme};
