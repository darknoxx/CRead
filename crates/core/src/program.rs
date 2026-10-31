//! The program interface: what a full-screen module gets to draw on and to
//! listen to.
//!
//! Programs are compiled in (see the boot screen's drive list); this trait is
//! what keeps their code out of the shell. It lives in `core` so it stays free
//! of windows and GPUs, and so the surfaces a program touches are the same
//! plain data the tests use.

use crate::command::Command;
use crate::raster::Raster;
use crate::screen::Screen;

/// What a program wants after an event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Stay in the program.
    Continue,
    /// The program is done; the shell takes the screen back.
    Exit,
}

/// A key, already stripped of the window system's vocabulary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyPress {
    Char(char),
    Ctrl(char),
    Enter,
    Backspace,
    Delete,
    Tab,
    Escape,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    /// Function keys, `F1` = 1.
    Function(u8),
}

/// Pointer buttons, in winit's order.
pub const BUTTON_LEFT: u8 = 0;
pub const BUTTON_RIGHT: u8 = 1;
pub const BUTTON_MIDDLE: u8 = 2;

/// A pointer event in emulated raster pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PointerEvent {
    pub x: i32,
    pub y: i32,
    pub kind: PointerKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerKind {
    Move,
    Down(u8),
    Up(u8),
    /// Wheel notches: positive scrolls down.
    Wheel(i32),
}

impl PointerEvent {
    pub fn move_to(x: i32, y: i32) -> Self {
        Self {
            x,
            y,
            kind: PointerKind::Move,
        }
    }
}

/// A module that owns the screen while it runs.
///
/// Everything it draws goes through the two surfaces: `raster` is the pixel
/// layer underneath, `screen` the cell grid on top, so menus and messages are
/// ordinary text while pictures are still pictures.
pub trait Program {
    /// Shown in the system line and used as the window title suffix.
    fn title(&self) -> &str;

    fn key(&mut self, key: KeyPress) -> Outcome;

    /// The same key going up again. Programs with held buttons (a game pad)
    /// need this; a text UI can leave it alone.
    fn key_released(&mut self, key: KeyPress) {
        let _ = key;
    }

    /// True while the program needs a frame every tick: a running emulator,
    /// an animation. A still screen leaves it false.
    fn wants_frames(&self) -> bool {
        false
    }

    fn pointer(&mut self, event: PointerEvent) -> Outcome;

    fn draw(&mut self, screen: &mut Screen, raster: &mut Raster);

    /// A command the shell does not handle itself (`tool`, `size`, `colour`).
    /// Programs that have no vocabulary of their own reject everything.
    fn command(&mut self, command: &Command) -> Result<String, String> {
        let _ = command;
        Err("this program has no commands".into())
    }

    /// Right-hand text for the system line, e.g. the current tool.
    fn status(&self) -> String {
        String::new()
    }

    /// The picture this program is working on, if it has one. The shell saves
    /// it with `save`; a program that is not a paint program returns nothing.
    fn picture(&self) -> Option<&Raster> {
        None
    }

    /// Whether this program has an undo/redo to offer. The shell's `undo`
    /// command asks before it says anything.
    fn undo(&mut self) -> bool {
        false
    }

    fn redo(&mut self) -> bool {
        false
    }
}
