//! Softhand: the paint program.
//!
//! It owns the canvas and the tools; the shell owns the files, the prompt and
//! the system line. Everything here is plain data, so the drawing logic is
//! unit-testable without a window.

use cread_core::command::Command;
use cread_core::program::{BUTTON_LEFT, KeyPress, Outcome, PointerEvent, PointerKind, Program};
use cread_core::raster::{PAPER, VGA};
use cread_core::{Cell, Raster, Screen};

/// Canvas size: the 640x400 raster minus the 16-pixel system line.
pub const CANVAS_W: u32 = 640;
pub const CANVAS_H: u32 = 368;
/// Top of the palette strip, in canvas pixels.
pub const STRIP_TOP: u32 = CANVAS_H;
/// Full surface a program draws on.
pub const SURFACE_H: u32 = CANVAS_H + 16;
/// Width of one palette swatch.
const SWATCH: u32 = CANVAS_W / 16;

/// What a left click does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Pencil,
    Eraser,
    Line,
    Rect,
    Fill,
    Pick,
}

impl Tool {
    pub fn name(self) -> &'static str {
        match self {
            Tool::Pencil => "pencil",
            Tool::Eraser => "eraser",
            Tool::Line => "line",
            Tool::Rect => "rect",
            Tool::Fill => "fill",
            Tool::Pick => "pick",
        }
    }

    fn paints(self) -> bool {
        matches!(self, Tool::Pencil | Tool::Eraser | Tool::Line | Tool::Rect)
    }
}

pub struct Softhand {
    canvas: Raster,
    tool: Tool,
    colour: u8,
    brush: u32,
    cursor: (i32, i32),
    /// Shape anchor and last painted point while the button is down.
    anchor: Option<(i32, i32)>,
    last: (i32, i32),
    /// Canvas as it was before the stroke, for the preview and the undo entry.
    before: Option<Raster>,
    pressed: bool,
    undo: Vec<Raster>,
    redo: Vec<Raster>,
    /// Drawing marks the picture; the shell's save clears it.
    modified: bool,
}

/// How many undo steps to keep: each is a full canvas copy.
const UNDO_STEPS: usize = 8;

impl Default for Softhand {
    fn default() -> Self {
        Self::new()
    }
}

impl Softhand {
    pub fn new() -> Self {
        Self {
            canvas: Raster::new(CANVAS_W, CANVAS_H),
            tool: Tool::Pencil,
            colour: 0,
            brush: 1,
            cursor: (CANVAS_W as i32 / 2, CANVAS_H as i32 / 2),
            anchor: None,
            last: (0, 0),
            before: None,
            pressed: false,
            undo: Vec::new(),
            redo: Vec::new(),
            modified: true,
        }
    }

    /// Start from an existing picture, cropped or padded to the canvas.
    pub fn with_picture(picture: &Raster) -> Self {
        let mut app = Self::new();
        app.canvas.fill(PAPER);
        app.canvas.copy_from(picture, 0, 0);
        app.modified = false;
        app
    }

    pub fn canvas(&self) -> &Raster {
        &self.canvas
    }

    pub fn tool(&self) -> Tool {
        self.tool
    }

    pub fn colour(&self) -> u8 {
        self.colour
    }

    pub fn brush(&self) -> u32 {
        self.brush
    }

    /// What the system line shows on the right.
    #[allow(clippy::misnamed_getters)]
    pub fn status(&self) -> String {
        format!(
            "{}   colour {}   size {}",
            self.tool.name(),
            self.colour,
            self.brush
        )
    }

    fn push_undo(&mut self, before: Raster) {
        self.modified = true;
        if self.undo.len() == UNDO_STEPS {
            self.undo.remove(0);
        }
        self.undo.push(before);
        self.redo.clear();
    }

    fn set_colour(&mut self, index: u8) {
        self.colour = index & 0x0f;
    }

    fn cycle_colour(&mut self, by: i32) {
        let next = (self.colour as i32 + by).rem_euclid(16);
        self.colour = next as u8;
    }

    fn cycle_brush(&mut self, by: i32) {
        self.brush = (self.brush as i32 + by).clamp(1, 16) as u32;
    }

    /// A click on the palette strip picks that colour.
    fn pick_from_strip(&mut self, x: i32) -> bool {
        if x < 0 {
            return false;
        }
        let index = (x as u32 / SWATCH).min(15) as u8;
        self.set_colour(index);
        true
    }

    fn paint_at(&mut self, x: i32, y: i32) {
        let index = match self.tool {
            Tool::Eraser => PAPER,
            _ => self.colour,
        };
        match self.tool {
            Tool::Line | Tool::Rect => {
                let Some(anchor) = self.anchor else { return };
                if let Some(before) = &self.before {
                    self.canvas = before.clone();
                }
                if self.tool == Tool::Line {
                    self.canvas.line(anchor, (x, y), self.brush, index);
                } else {
                    self.canvas.rect(anchor, (x, y), self.brush, index, false);
                }
            }
            _ => {
                self.canvas.line(self.last, (x, y), self.brush, index);
                self.last = (x, y);
            }
        }
    }
}

impl Program for Softhand {
    fn title(&self) -> &str {
        "SOFTHAND"
    }

    fn key(&mut self, key: KeyPress) -> Outcome {
        // Letters are tool shortcuts, so shift must not change them.
        let key = match key {
            KeyPress::Char(c) if c.is_ascii_alphabetic() => KeyPress::Char(c.to_ascii_lowercase()),
            other => other,
        };
        match key {
            KeyPress::Char('b') => self.tool = Tool::Pencil,
            KeyPress::Char('e') => self.tool = Tool::Eraser,
            KeyPress::Char('l') => self.tool = Tool::Line,
            KeyPress::Char('r') => self.tool = Tool::Rect,
            KeyPress::Char('f') => self.tool = Tool::Fill,
            KeyPress::Char('i') => self.tool = Tool::Pick,
            KeyPress::Char(',') => self.cycle_colour(-1),
            KeyPress::Char('.') => self.cycle_colour(1),
            KeyPress::Char('-') => self.cycle_brush(-1),
            KeyPress::Char('+') | KeyPress::Char('=') => self.cycle_brush(1),
            KeyPress::Char('u') | KeyPress::Ctrl('z') => {
                self.undo();
            }
            KeyPress::Char('y') | KeyPress::Ctrl('y') => {
                self.redo();
            }
            _ => {}
        }
        Outcome::Continue
    }

    fn pointer(&mut self, event: PointerEvent) -> Outcome {
        self.cursor = (event.x, event.y);
        match event.kind {
            PointerKind::Move => {
                if self.pressed && self.tool.paints() {
                    self.paint_at(event.x, event.y);
                }
            }
            PointerKind::Down(button) if button == BUTTON_LEFT => {
                if event.y as u32 >= STRIP_TOP {
                    self.pick_from_strip(event.x);
                    return Outcome::Continue;
                }
                match self.tool {
                    Tool::Pick => {
                        if let Some(index) = self.canvas.get(event.x, event.y) {
                            self.set_colour(index);
                        }
                    }
                    Tool::Fill => {
                        let before = self.canvas.clone();
                        if self.canvas.flood(event.x, event.y, self.colour) > 0 {
                            self.push_undo(before);
                        }
                    }
                    _ => {
                        self.before = Some(self.canvas.clone());
                        self.anchor = Some((event.x, event.y));
                        self.last = (event.x, event.y);
                        self.pressed = true;
                        self.paint_at(event.x, event.y);
                    }
                }
            }
            PointerKind::Up(button) if button == BUTTON_LEFT => {
                if let Some(before) = self.before.take() {
                    self.push_undo(before);
                }
                self.pressed = false;
                self.anchor = None;
            }
            PointerKind::Wheel(delta) => self.cycle_brush(-delta.signum()),
            _ => {}
        }
        Outcome::Continue
    }

    fn draw(&mut self, screen: &mut Screen, raster: &mut Raster) {
        screen.fill(Cell::TRANSPARENT);
        raster.fill(PAPER);
        raster.copy_from(&self.canvas, 0, 0);

        // Palette strip: sixteen solid swatches, the current one ringed.
        for index in 0..16u8 {
            let x0 = index as u32 * SWATCH;
            for x in x0..x0 + SWATCH {
                for y in STRIP_TOP..SURFACE_H {
                    raster.set(x as i32, y as i32, index);
                }
            }
            if index == self.colour {
                let ink = if index >= 8 { 0 } else { 15 };
                for x in x0..x0 + SWATCH {
                    raster.set(x as i32, STRIP_TOP as i32, ink);
                    raster.set(x as i32, SURFACE_H as i32 - 1, ink);
                }
                for y in STRIP_TOP..SURFACE_H {
                    raster.set(x0 as i32, y as i32, ink);
                    raster.set((x0 + SWATCH - 1) as i32, y as i32, ink);
                }
            }
        }

        // Crosshair: solid where it matters, dashed elsewhere, so it reads on
        // any colour under it.
        let (cx, cy) = self.cursor;
        if cy >= 0 && (cy as u32) < STRIP_TOP {
            let ink = VGA.iter().position(|rgb| *rgb == [0, 0, 0]).unwrap_or(0) as u8;
            let ink = if raster.get(cx, cy) == Some(ink) {
                15
            } else {
                ink
            };
            for step in 1..=4 {
                for (x, y) in [
                    (cx - step, cy),
                    (cx + step, cy),
                    (cx, cy - step),
                    (cx, cy + step),
                ] {
                    raster.set(x, y, ink);
                }
            }
        }
    }

    fn status(&self) -> String {
        Softhand::status(self)
    }

    fn modified(&self) -> bool {
        self.modified
    }

    fn mark_saved(&mut self) {
        self.modified = false;
    }

    fn command(&mut self, command: &Command) -> Result<String, String> {
        match command {
            Command::Tool(None) => Ok("tools: pencil, eraser, line, rect, fill, pick".into()),
            Command::Tool(Some(name)) => {
                let tool = match name.trim().to_ascii_lowercase().as_str() {
                    "pencil" | "pen" => Tool::Pencil,
                    "eraser" | "rubber" => Tool::Eraser,
                    "line" => Tool::Line,
                    "rect" | "rectangle" | "box" => Tool::Rect,
                    "fill" | "bucket" => Tool::Fill,
                    "pick" | "eyedropper" => Tool::Pick,
                    other => return Err(format!("no tool called {other}")),
                };
                self.tool = tool;
                Ok(format!("tool: {}", tool.name()))
            }
            Command::Size(None) => Ok(format!("brush size {}", self.brush)),
            Command::Size(Some(size)) => {
                if !(1..=16).contains(size) {
                    return Err("brush size is 1 to 16".into());
                }
                self.brush = *size;
                Ok(format!("brush size {}", self.brush))
            }
            Command::Colour(None) => Ok(format!("colour {}", self.colour)),
            Command::Colour(Some(index)) => {
                if *index > 15 {
                    return Err("colour is 0 to 15".into());
                }
                self.set_colour(*index);
                Ok(format!("colour {}", self.colour))
            }
            other => Err(format!("SOFTHAND does not know {other:?}")),
        }
    }

    fn picture(&self) -> Option<&Raster> {
        Some(&self.canvas)
    }

    fn undo(&mut self) -> bool {
        let Some(previous) = self.undo.pop() else {
            return false;
        };
        let current = std::mem::replace(&mut self.canvas, previous);
        self.redo.push(current);
        true
    }

    fn redo(&mut self) -> bool {
        let Some(next) = self.redo.pop() else {
            return false;
        };
        let current = std::mem::replace(&mut self.canvas, next);
        self.undo.push(current);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn click(app: &mut Softhand, x: i32, y: i32) {
        app.pointer(PointerEvent {
            x,
            y,
            kind: PointerKind::Down(BUTTON_LEFT),
        });
        app.pointer(PointerEvent {
            x,
            y,
            kind: PointerKind::Up(BUTTON_LEFT),
        });
    }

    #[test]
    fn a_pencil_click_paints_and_undo_restores() {
        let mut app = Softhand::new();
        assert_eq!(app.canvas().get(10, 10), Some(PAPER));
        app.set_colour(4);
        click(&mut app, 10, 10);
        assert_eq!(app.canvas().get(10, 10), Some(4));
        assert!(app.undo());
        assert_eq!(app.canvas().get(10, 10), Some(PAPER));
        assert!(app.redo());
        assert_eq!(app.canvas().get(10, 10), Some(4));
    }

    #[test]
    fn dragging_draws_a_continuous_stroke() {
        let mut app = Softhand::new();
        app.set_colour(1);
        app.pointer(PointerEvent {
            x: 5,
            y: 5,
            kind: PointerKind::Down(BUTTON_LEFT),
        });
        app.pointer(PointerEvent::move_to(25, 5));
        app.pointer(PointerEvent {
            x: 25,
            y: 5,
            kind: PointerKind::Up(BUTTON_LEFT),
        });
        for x in 5..=25 {
            assert_eq!(app.canvas().get(x, 5), Some(1), "x={x}");
        }
        assert_eq!(app.canvas().get(5, 6), Some(PAPER));
    }

    #[test]
    fn the_prompt_can_set_the_brush_colour_and_tool() {
        let mut app = Softhand::new();
        assert_eq!(
            app.command(&Command::Size(Some(5))).unwrap(),
            "brush size 5"
        );
        assert_eq!(app.brush(), 5);
        assert!(app.command(&Command::Size(Some(0))).is_err());

        assert_eq!(
            app.command(&Command::Colour(Some(12))).unwrap(),
            "colour 12"
        );
        assert_eq!(app.colour(), 12);
        assert!(app.command(&Command::Colour(Some(99))).is_err());

        assert!(app.command(&Command::Tool(Some("fill".into()))).is_ok());
        assert_eq!(app.tool(), Tool::Fill);
        assert!(app.command(&Command::Tool(Some("spray".into()))).is_err());
        assert!(
            app.command(&Command::Tool(None))
                .unwrap()
                .contains("pencil")
        );
    }

    #[test]
    fn tool_letters_ignore_shift_and_undo_works_from_the_keyboard() {
        let mut app = Softhand::new();
        app.key(KeyPress::Char('F'));
        assert_eq!(app.tool(), Tool::Fill);
        app.key(KeyPress::Char('B'));
        assert_eq!(app.tool(), Tool::Pencil);
        app.set_colour(3);
        click(&mut app, 2, 2);
        assert!(app.undo());
        assert_eq!(app.canvas().get(2, 2), Some(PAPER));
    }

    #[test]
    fn the_palette_strip_picks_a_colour() {
        let mut app = Softhand::new();
        click(&mut app, (SWATCH * 4 + 3) as i32, STRIP_TOP as i32 + 4);
        assert_eq!(app.colour(), 4);
        assert_eq!(
            app.canvas().get(5, 5),
            Some(PAPER),
            "the strip is not canvas"
        );
    }

    #[test]
    fn fill_paints_the_whole_canvas_and_is_one_undo_step() {
        let mut app = Softhand::new();
        app.tool = Tool::Fill;
        app.set_colour(2);
        click(&mut app, 100, 100);
        assert_eq!(app.canvas().get(0, 0), Some(2));
        assert_eq!(app.canvas().get(CANVAS_W as i32 - 1, 0), Some(2));
        assert!(app.undo());
        assert_eq!(app.canvas().get(0, 0), Some(PAPER));
    }

    #[test]
    fn a_previewed_shape_only_lands_when_the_button_goes_up() {
        let mut app = Softhand::new();
        app.tool = Tool::Rect;
        app.set_colour(3);
        app.pointer(PointerEvent {
            x: 4,
            y: 4,
            kind: PointerKind::Down(BUTTON_LEFT),
        });
        app.pointer(PointerEvent::move_to(20, 12));
        assert_eq!(app.canvas().get(20, 12), Some(3));
        assert_eq!(app.canvas().get(10, 10), Some(PAPER), "not filled");
        app.pointer(PointerEvent {
            x: 20,
            y: 12,
            kind: PointerKind::Up(BUTTON_LEFT),
        });
        // Undo now returns to the state before the shape, not mid-drag.
        assert!(app.undo());
        assert_eq!(app.canvas().get(20, 12), Some(PAPER));
    }

    #[test]
    fn dragging_back_shrinks_the_preview_instead_of_smearing() {
        let mut app = Softhand::new();
        app.tool = Tool::Line;
        app.set_colour(5);
        app.pointer(PointerEvent {
            x: 2,
            y: 2,
            kind: PointerKind::Down(BUTTON_LEFT),
        });
        app.pointer(PointerEvent::move_to(30, 2));
        app.pointer(PointerEvent::move_to(6, 2));
        // The stretch to x=30 must be gone again.
        assert_eq!(app.canvas().get(20, 2), Some(PAPER));
        assert_eq!(app.canvas().get(6, 2), Some(5));
    }

    #[test]
    fn the_eraser_paints_paper_and_pick_reads_the_canvas() {
        let mut app = Softhand::new();
        app.set_colour(9);
        click(&mut app, 7, 7);
        app.tool = Tool::Eraser;
        click(&mut app, 7, 7);
        assert_eq!(app.canvas().get(7, 7), Some(PAPER));

        app.tool = Tool::Pencil;
        app.set_colour(9);
        click(&mut app, 8, 8);
        app.tool = Tool::Pick;
        click(&mut app, 8, 8);
        assert_eq!(app.colour(), 9);
    }

    #[test]
    fn a_picture_loads_into_the_canvas_and_is_cropped() {
        let mut picture = Raster::new(CANVAS_W + 40, CANVAS_H + 40);
        picture.fill(7);
        let app = Softhand::with_picture(&picture);
        assert_eq!(app.canvas().get(0, 0), Some(7));
        assert_eq!(
            app.canvas().get(CANVAS_W as i32 - 1, CANVAS_H as i32 - 1),
            Some(7)
        );
        assert_eq!(app.canvas().width(), CANVAS_W);
    }

    #[test]
    fn the_drawing_survives_a_frame() {
        let mut app = Softhand::new();
        app.set_colour(12);
        click(&mut app, 3, 3);
        let mut screen = Screen::new(80, 25);
        let mut raster = Raster::new(CANVAS_W, SURFACE_H);
        app.draw(&mut screen, &mut raster);
        assert_eq!(raster.get(3, 3), Some(12));
        // The strip starts at swatch 0: the first block is black.
        assert_eq!(raster.get(3, STRIP_TOP as i32 + 8), Some(0));
        assert_eq!(raster.get(SWATCH as i32 + 3, STRIP_TOP as i32 + 8), Some(1));
        assert!(screen.get(0, 0).unwrap().is_transparent());
    }
}
