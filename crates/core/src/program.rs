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
    /// The bare Control keys, left and right. Emulators use them for the two
    /// small buttons a Game Boy has.
    ControlLeft,
    ControlRight,
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
    /// Wheel movement in lines, three per notch. Positive is the wheel turned
    /// up (away from you), which is winit's sign; a reader scrolls back for it.
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

/// A true-colour picture a program shows under its cell grid.
///
/// The 16-colour [`Raster`] is for pictures a program paints itself. Some
/// programs show pixels that were made elsewhere - a web page rendered by the
/// browser engine - and quantising those would make text unreadable. The frame
/// is still drawn at emulated resolution and goes through the same CRT pass.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    /// `width * height` RGBA pixels, row by row.
    pub rgba: Vec<u8>,
}

/// Wakes the shell's event loop from another thread.
///
/// A program with a worker (the browser engine) calls it when something new
/// arrived, so a still page costs nothing: there is no polling clock.
pub type Waker = std::sync::Arc<dyn Fn() + Send + Sync>;

/// What a program wants in the shell's top bar. The shell draws the bar; the
/// program only names the parts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TopBar {
    pub title: String,
    pub state: String,
    pub location: String,
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

    /// A true-colour layer under the raster, at `(x, y)` in stage pixels.
    fn frame(&self) -> Option<(i32, i32, &Frame)> {
        None
    }

    /// The shell hands over a way to wake it; see [`Waker`].
    fn set_waker(&mut self, waker: Waker) {
        let _ = waker;
    }

    /// Pick up whatever a worker delivered. True when the screen changed.
    fn poll(&mut self) -> bool {
        false
    }

    /// A program that wears the top bar names its parts here; `None` keeps
    /// the plain program line at the bottom only.
    fn top_bar(&self) -> Option<TopBar> {
        None
    }

    /// Text from the shell's clipboard (`Ctrl+V`). False when the program has
    /// nowhere to put it.
    fn paste(&mut self, text: &str) -> bool {
        let _ = text;
        false
    }

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

    /// The shell's page and ink, as indices into the fixed palette, so a
    /// program can sit on the page instead of in its own black box.
    fn palette(&mut self, background: u8, ink: u8) {
        let _ = (background, ink);
    }

    /// The shell's dim and accent slots. Programs with chrome of their own —
    /// a scrollbar, a heading, a selected row — use these so they stay in the
    /// same palette as the rest of CRead.
    fn chrome(&mut self, dim: u8, accent: u8) {
        let _ = (dim, accent);
    }

    /// Step back one level inside the program. `exit` asks this first: a
    /// cartridge returns to its shelf, and only the next `exit` leaves the
    /// program. `false` means "nothing left to close, take the screen".
    fn exit(&mut self) -> bool {
        false
    }

    /// Consume Escape before the shell opens its command prompt. A program
    /// with a modal of its own (the SCRIPTURE chapter grid) returns true when
    /// it closed that modal.
    fn escape(&mut self) -> bool {
        false
    }

    /// Whether this program has an undo/redo to offer. The shell's `undo`
    /// command asks before it says anything.
    fn undo(&mut self) -> bool {
        false
    }

    fn redo(&mut self) -> bool {
        false
    }

    /// The keys this program answers to, as one line under its work surface.
    fn key_legend(&self) -> &'static str {
        ""
    }

    /// Does the program have unsaved changes? The shell shows it in the top
    /// bar the way the editor shows `modified`.
    fn modified(&self) -> bool {
        false
    }

    /// The shell has just written the program's file.
    fn mark_saved(&mut self) {}

    /// Did this program write files of its own since it was opened?
    ///
    /// The shell asks on the way out, so a cartridge's battery save is not
    /// silently left behind: the file appears when the program is dropped,
    /// which is exactly when the shell stops being able to see it.
    fn wrote_files(&self) -> bool {
        false
    }
}
