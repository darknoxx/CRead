//! Modules on the 80x25 stage: launching, keys and pointer, leaving.
//!
//! A program owns its surface and gestures; the shell owns the prompt, the files
//! and the loading bar in front of it.

use super::*;

impl App {
    /// A program's own keys: Esc opens the command line, everything else is
    /// the program's gesture.
    pub(super) fn on_key_program(&mut self, el: &ActiveEventLoop, event: KeyEvent) {
        if self.program_prompt {
            // An empty Esc closes the prompt; otherwise this is the ordinary
            // command line, history and completion included.
            if matches!(&event.logical_key, Key::Named(NamedKey::Escape)) && self.command.is_empty()
            {
                self.program_prompt = false;
                self.poke();
                return;
            }
            self.on_key_action(el, event);
            return;
        }
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => {
                let consumed = self
                    .program
                    .as_mut()
                    .is_some_and(|program| program.escape());
                if consumed {
                    self.poke();
                    return;
                }
                self.program_prompt = true;
                self.command.clear();
                self.command_error.clear();
                self.poke();
            }
            _ => {
                let Some(key) = self.program_key(&event) else {
                    return;
                };
                // The clipboard is the shell's: a program that takes text gets
                // it handed over, instead of a bare Ctrl+V it cannot use.
                if key == KeyPress::Ctrl('v') && self.program_takes_paste() {
                    self.poke();
                    return;
                }
                let mut done = false;
                if let Some(program) = self.program.as_mut() {
                    done = program.key(key) == Outcome::Exit;
                }
                if done {
                    if self.program_id == Some("poker") {
                        self.leave_poker();
                    } else if self.program_id == Some("scripture") {
                        self.leave_scripture();
                    } else {
                        self.leave_to_boot();
                    }
                }
                self.poke();
            }
        }
    }

    /// Translate a winit key into the program's vocabulary.
    pub(super) fn program_key(&self, event: &KeyEvent) -> Option<KeyPress> {
        let ctrl = self.mods.control_key() || self.mods.super_key();
        Some(match &event.logical_key {
            Key::Character(text) => {
                let ch = text.chars().next()?;
                if ctrl {
                    KeyPress::Ctrl(ch.to_ascii_lowercase())
                } else {
                    KeyPress::Char(ch)
                }
            }
            Key::Named(NamedKey::Enter) => KeyPress::Enter,
            Key::Named(NamedKey::Backspace) => KeyPress::Backspace,
            Key::Named(NamedKey::Delete) => KeyPress::Delete,
            Key::Named(NamedKey::Tab) => KeyPress::Tab,
            Key::Named(NamedKey::ArrowLeft) => KeyPress::Left,
            Key::Named(NamedKey::ArrowRight) => KeyPress::Right,
            Key::Named(NamedKey::ArrowUp) => KeyPress::Up,
            Key::Named(NamedKey::ArrowDown) => KeyPress::Down,
            Key::Named(NamedKey::Home) => KeyPress::Home,
            Key::Named(NamedKey::End) => KeyPress::End,
            Key::Named(NamedKey::PageUp) => KeyPress::PageUp,
            Key::Named(NamedKey::PageDown) => KeyPress::PageDown,
            // The space bar arrives as a named key, not as the character " ".
            Key::Named(NamedKey::Space) => KeyPress::Char(' '),
            Key::Named(NamedKey::Control) => match event.physical_key {
                PhysicalKey::Code(KeyCode::ControlLeft) => KeyPress::ControlLeft,
                PhysicalKey::Code(KeyCode::ControlRight) => KeyPress::ControlRight,
                _ => return None,
            },
            _ => return None,
        })
    }

    /// Give the clipboard's text to the program. True when it took it.
    fn program_takes_paste(&mut self) -> bool {
        let Ok(text) = self
            .clipboard()
            .and_then(|c| c.get_text().map_err(|e| e.to_string()))
        else {
            return false;
        };
        if text.is_empty() {
            return false;
        }
        self.program
            .as_mut()
            .is_some_and(|program| program.paste(&text))
    }

    /// Collect what the program's worker delivered; draw when it changed.
    pub(super) fn poll_program(&mut self) {
        if self.program.as_mut().is_some_and(|program| program.poll()) {
            self.poke();
        }
    }

    /// Open the browser, at an address or on its home page.
    pub(super) fn start_wanderlust(&mut self, typed: Option<&str>) {
        if self.program_id != Some("wanderlust") {
            self.close_program();
            // Tests never start a real browser.
            let mut web = if self.persist {
                cread_wanderlust::Wanderlust::new()
            } else {
                let dirs = cread_wanderlust::Dirs::for_binary(None);
                cread_wanderlust::Wanderlust::with(None, dirs)
            };
            if let Some(waker) = self.waker.clone() {
                web.set_waker(waker);
            }
            self.program = Some(Box::new(web));
            self.program_id = Some("wanderlust");
        }
        self.mode = Mode::Program;
        self.program_prompt = false;
        self.program_point = (-1, -1);
        self.command.clear();
        self.command_error.clear();
        if let Some(program) = self.program.as_mut() {
            let _ = program.command(&Command::Web(typed.map(str::to_string)));
        }
        self.dirty = true;
    }

    /// Pointer position in emulated raster pixels.
    pub(super) fn raster_point(&self) -> (i32, i32) {
        let (ox, oy) = self.program_origin();
        match self.vp.as_ref() {
            Some(vp) => {
                let scale = vp.scale.max(1) as f64;
                (
                    ((self.pointer.0 - vp.offset_x as f64) / scale) as i32 - ox,
                    ((self.pointer.1 - vp.offset_y as f64) / scale) as i32 - oy,
                )
            }
            None => (self.pointer.0 as i32 - ox, self.pointer.1 as i32 - oy),
        }
    }

    /// The stage's pixel origin: the cell origin from `program_stage`, in the
    /// framebuffer's pixels.
    pub(super) fn program_origin(&self) -> (i32, i32) {
        let (cols, rows) = match self.vp.as_ref() {
            Some(vp) => (
                vp.virt_w / cread_core::FONT_W,
                vp.virt_h / cread_core::FONT_H,
            ),
            None => (self.screen.cols(), self.screen.rows()),
        };
        let (x, y) = self.program_stage(cols, rows);
        (
            (x * cread_core::FONT_W) as i32,
            (y * cread_core::FONT_H) as i32,
        )
    }

    /// Hand a pointer event to the running program, if it should get one.
    pub(super) fn program_pointer(&mut self, kind: PointerKind) -> bool {
        if self.mode != Mode::Program {
            return false;
        }
        // The prompt takes presses, but a release still has to reach the
        // program or a brush stays glued to the canvas.
        if self.program_prompt && !matches!(kind, PointerKind::Up(_)) {
            return false;
        }
        let (x, y) = self.raster_point();
        if matches!(kind, PointerKind::Move) && (x, y) == self.program_point {
            // Same emulated pixel: nothing on screen would change.
            return true;
        }
        self.program_point = (x, y);
        if let Some(program) = self.program.as_mut() {
            if program.pointer(PointerEvent { x, y, kind }) == Outcome::Exit {
                self.leave_to_boot();
            }
            self.poke();
        }
        true
    }

    /// Where a program's classic 640x400 stage sits in the current grid.
    ///
    /// Programs are written for 80x25 cells; a bigger framebuffer gets the whole
    /// stage centred instead of pinned to the top-left. Softhand is 24 rows tall
    /// and wears the top bar, so its stage is centred below that bar.
    pub(super) fn program_stage(&self, cols: u32, rows: u32) -> (u32, u32) {
        let stage_rows = self.program_stage_height();
        let top = if self.program_wears_top_bar() { 1 } else { 0 };
        (
            cols.saturating_sub(STAGE_COLS) / 2,
            top + rows.saturating_sub(top + stage_rows) / 2,
        )
    }

    /// Rows the current program's stage occupies, strip and legend included.
    pub(super) fn program_stage_height(&self) -> u32 {
        match self.program_id {
            Some("softhand") => SOFTHAND_ROWS,
            Some("wanderlust") => WANDERLUST_ROWS,
            _ => STAGE_ROWS,
        }
    }

    /// Softhand and the browser sit under the top bar instead of on the whole
    /// stage.
    pub(super) fn program_wears_top_bar(&self) -> bool {
        matches!(self.program_id, Some("softhand" | "wanderlust"))
    }

    /// Show the fake progress bar, then do `next`.
    pub(super) fn start_loading(&mut self, next: Pending) {
        self.pending = Some(next);
        self.loading_started = Instant::now();
        self.mode = Mode::Loading;
        self.dirty = true;
    }

    pub(super) fn finish_loading(&mut self) {
        let Some(next) = self.pending.take() else {
            self.mode = Mode::Action;
            return;
        };
        match next {
            Pending::Inspiro => {
                self.home = Home::Inspiro;
                self.mode = Mode::Action;
                self.command.clear();
            }
            Pending::Program(index) => {
                let id = PROGRAMS.get(index).map(|p| p.id).unwrap_or_default();
                if id == "wanderlust" {
                    self.start_wanderlust(None);
                    return;
                }
                if id == "pocketboi" {
                    self.program = Some(Box::new(cread_pocketboi::Pocketboi::new(&self.vault())));
                    self.program_id = Some("pocketboi");
                    self.mode = Mode::Program;
                    self.program_prompt = false;
                    self.command.clear();
                    self.dirty = true;
                    return;
                }
                self.home = program_home(index);
                // A disc is still in the tray: Riff opens on the player, not on
                // the front door it was left from.
                if self.home == Home::Riff && !self.riff.playlist.is_empty() {
                    self.mode = Mode::Riff;
                } else {
                    self.mode = Mode::Action;
                }
                self.command.clear();
            }
            Pending::NewPicture => {
                self.new_picture();
            }
            Pending::Rom(path) => {
                if !self.is_installed("pocketboi") {
                    self.set_status("pocketboi is not installed - see SOFTWARE");
                    self.mode = Mode::Browser;
                    return;
                }
                let mut boi = cread_pocketboi::Pocketboi::new(&self.vault());
                match boi.play(&path) {
                    Ok(()) => {
                        self.program = Some(Box::new(boi));
                        self.program_id = Some("pocketboi");
                        self.mode = Mode::Program;
                        self.program_prompt = false;
                        self.command.clear();
                        self.dirty = true;
                    }
                    Err(e) => {
                        self.set_status(e);
                        self.mode = Mode::Browser;
                    }
                }
            }
            Pending::Picture(path) => {
                // Whatever program was in front lets go first, so a cartridge
                // writes its battery before the painting takes the screen.
                self.close_program();
                match self.open_picture(path) {
                    Ok(()) => {}
                    Err(e) => {
                        self.set_status(e);
                        self.home = Home::SoftHand;
                        self.mode = Mode::Action;
                    }
                }
            }
            Pending::Document(path) => match self.open_document(path) {
                Ok(()) => {
                    // A document belongs to the editor, not to whatever program
                    // happened to be in front when `open` was typed.
                    self.close_program();
                    self.mode = Mode::Write;
                }
                Err(e) => {
                    self.set_status(e);
                    self.mode = Mode::Action;
                }
            },
            Pending::Software { id, install } => {
                self.apply_software(id, install);
                self.mode = Mode::Software;
            }
            Pending::NewDocument => match self.new_document() {
                Ok(msg) => {
                    self.close_program();
                    if !msg.is_empty() {
                        self.set_status(msg);
                    }
                    self.mode = Mode::Write;
                }
                Err(e) => {
                    self.set_status(e);
                    self.mode = Mode::Action;
                }
            },
        }
        self.dirty = true;
    }

    /// Open the machine from its front door.
    pub(super) fn start_poker(&mut self) {
        let poker = if self.persist {
            cread_poker::Poker::new()
        } else {
            // The window and exit paths cash out in Drop; tests must never
            // write the real wallet from a temporary app.
            let path =
                std::env::temp_dir().join(format!("cread-poker-app-{}.save", std::process::id()));
            cread_poker::Poker::new_at(path)
        };
        self.show_poker(poker);
    }

    pub(super) fn show_poker(&mut self, program: cread_poker::Poker) {
        self.program = Some(Box::new(program));
        self.program_id = Some("poker");
        self.home = Home::Poker;
        self.mode = Mode::Program;
        self.program_prompt = false;
        self.command.clear();
        self.command_error.clear();
        self.dirty = true;
    }

    /// Leaving the machine returns to its own front door, not the boot list:
    /// the wallet and highscore are the next thing the player wants to see.
    pub(super) fn leave_poker(&mut self) {
        self.close_program();
        self.picture_path = None;
        self.program_prompt = false;
        self.program_point = (-1, -1);
        self.mode = Mode::Action;
        self.command.clear();
        self.command_error.clear();
        self.dirty = true;
    }

    /// Open the Bible reader from its cover.
    pub(super) fn start_scripture(&mut self) {
        let scripture = if self.persist && self.opts.screenshot.is_none() {
            cread_scripture::Scripture::new()
        } else {
            let path = std::env::temp_dir()
                .join(format!("cread-scripture-app-{}.state", std::process::id()));
            cread_scripture::Scripture::new_at(path, false)
        };
        self.program = Some(Box::new(scripture));
        self.program_id = Some("scripture");
        self.home = Home::Scripture;
        self.mode = Mode::Program;
        self.program_prompt = false;
        self.command.clear();
        self.command_error.clear();
        self.dirty = true;
    }

    /// Leaving the reader returns to the CRead welcome screen, like every other
    /// module. The cover is only the way in, never the way out.
    pub(super) fn leave_scripture(&mut self) {
        self.close_program();
        self.picture_path = None;
        self.program_prompt = false;
        self.program_point = (-1, -1);
        self.home = Home::Boot;
        self.mode = Mode::Action;
        self.command.clear();
        self.command_error.clear();
        self.dirty = true;
    }

    /// Drop everything and stand on the boot screen again.
    pub(super) fn leave_to_boot(&mut self) {
        self.close_program();
        self.picture_path = None;
        self.program_prompt = false;
        self.program_point = (-1, -1);
        self.discard_scratch();
        self.doc = Document {
            path: None,
            title_hint: None,
            editor: Editor::new(),
            last_edit: Instant::now(),
        };
        self.has_document = false;
        self.home = Home::Boot;
        self.mode = Mode::Action;
        self.command.clear();
        self.command_error.clear();
        self.dirty = true;
    }

    /// Drop the open program. Dropping is also when a cartridge writes its
    /// battery save, so the shell marks the vault as changed first.
    pub(super) fn close_program(&mut self) {
        if self
            .program
            .as_ref()
            .is_some_and(|program| program.wrote_files())
        {
            self.unsynced = true;
        }
        self.program = None;
        self.program_id = None;
    }

    /// A fresh canvas on an empty sheet.
    pub(super) fn new_picture(&mut self) {
        self.program = Some(Box::new(Softhand::new()));
        self.program_id = Some("softhand");
        self.picture_path = None;
        self.program_prompt = false;
        self.mode = Mode::Program;
        self.dirty = true;
    }

    pub(super) fn open_picture(&mut self, path: PathBuf) -> Result<(), String> {
        let image = picture::load(&path)?;
        self.program = Some(Box::new(Softhand::with_picture(&image)));
        self.program_id = Some("softhand");
        let resized = self
            .program
            .as_ref()
            .is_some_and(|program| program.modified());
        // A cropped or padded source is a new canvas, not the file on disk.
        // Keeping the old path would overwrite the original on quit.
        self.picture_path = (!resized).then_some(path.clone());
        self.program_prompt = false;
        self.mode = Mode::Program;
        self.dirty = true;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.set_status(if resized {
            format!("opened {name} (resized copy - save as new)")
        } else {
            format!("opened {name}")
        });
        Ok(())
    }

    /// Write the running program's picture. `None` keeps the current name, or
    /// picks the next free one.
    pub(super) fn save_picture(&mut self, name: Option<&str>) -> Result<String, String> {
        let vault = self.vault();
        let path = match name {
            Some(name) => {
                let path = picture::resolve(&vault, name)?;
                if path.exists() && self.picture_path.as_deref() != Some(path.as_path()) {
                    return Err(format!(
                        "{} already exists",
                        path.file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default()
                    ));
                }
                path
            }
            None => match self.picture_path.clone() {
                Some(path) => path,
                None => picture::free_name(&vault),
            },
        };
        let Some(program) = self.program.as_ref() else {
            return Err("no picture open".into());
        };
        let Some(canvas) = program.picture() else {
            return Err("this program has no picture".into());
        };
        picture::save(&path, canvas)?;
        if let Some(program) = self.program.as_mut() {
            program.mark_saved();
        }
        self.unsynced = true;
        self.picture_path = Some(path.clone());
        Ok(format!(
            "saved {}",
            path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        ))
    }

    /// True while Softhand is the thing in front: its front door or its canvas.
    pub(super) fn in_softhand(&self) -> bool {
        self.program_id == Some("softhand") || (!self.has_document && self.home == Home::SoftHand)
    }
}
