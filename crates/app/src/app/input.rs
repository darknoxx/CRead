//! Keyboard input: the dispatch by mode, write mode, the action prompt, help and
//! the command history.
//!
//! Keys are editing gestures; everything else is a command in `SPECS`. Every
//! handler ends in `poke()`, or the frame waits for the next blink.

use super::*;

impl App {
    pub(super) fn on_key(&mut self, el: &ActiveEventLoop, event: KeyEvent) {
        if event.state == ElementState::Released {
            // A game pad needs the key going up again; everything else ignores it.
            // Releases reach the program even while the command prompt is
            // open. Otherwise a held pad button stays pressed forever when the
            // prompt was opened between press and release.
            if self.mode == Mode::Program
                && let Some(key) = self.program_key(&event)
                && let Some(program) = self.program.as_mut()
            {
                program.key_released(key);
            }
            return;
        }
        if event.state != ElementState::Pressed {
            return;
        }
        // The screensaver belongs to the machine: the first key brings back the
        // screen and is swallowed, so it does not also type a character.
        if self.wake_screensaver() {
            self.next_frame = Instant::now();
            return;
        }
        self.last_input = Instant::now();
        if self.note.is_some() {
            self.on_key_note(event);
            return;
        }
        if self.boot_at.take().is_some() {
            // Impatient: skip straight to the picture, and swallow the key.
            self.next_frame = Instant::now();
            self.dirty = true;
            return;
        }
        match self.mode {
            Mode::Write => self.on_key_write(el, event),
            Mode::Search => self.on_key_search(event),
            Mode::View => self.on_key_view(event),
            Mode::Loading => self.finish_loading(),
            Mode::Program => self.on_key_program(el, event),
            Mode::Riff => self.on_key_riff(el, event),
            Mode::RiffInsert => self.on_key_riff_insert(event),
            Mode::Software => self.on_key_software(event),
            Mode::Setup => self.on_key_setup(event),
            Mode::Quit => self.on_key_quit(el, event),
            Mode::Power => self.on_key_power(el, event),
            Mode::Checking => self.on_key_checking(event),
            Mode::UpdatePrompt => self.on_key_update_prompt(event),
            Mode::Manual => self.on_key_manual(event),
            Mode::Calendar => self.on_key_calendar(el, event),
            Mode::CalendarEdit => self.on_key_calendar_edit(event),
            Mode::Reminder => self.on_key_reminder(event),
            Mode::Notice => self.on_key_notice(),
            Mode::Screensaver => {
                // Waking happens above; nothing else reaches this screen.
            }
            Mode::Action => self.on_key_action(el, event),
            Mode::Browser => self.on_key_browser(event),
            Mode::Webdav => self.on_key_webdav(event),
            Mode::Help => self.on_key_help(event),
        }

        // Single choke point: whatever the key changed, draw it now. Forgetting
        // this in one handler makes that movement feel like it lags a beat
        // behind the key repeat, because the next frame would otherwise wait
        // for the cursor blink.
        if self.dirty {
            self.next_frame = Instant::now();
        }
    }

    /// Write mode: the document is live, editing keys only.
    pub(super) fn on_key_write(&mut self, _el: &ActiveEventLoop, event: KeyEvent) {
        self.follow_cursor = true;
        let ctrl = self.mods.control_key() || self.mods.super_key();
        let shift = self.mods.shift_key();
        let code = match event.physical_key {
            PhysicalKey::Code(code) => Some(code),
            PhysicalKey::Unidentified(_) => None,
        };

        // AltGr arrives as Ctrl+Alt on many X11/Wayland layouts. That is a
        // printable character, not a command chord.
        let altgr = self.mods.alt_key() && self.mods.control_key();
        if ctrl && !altgr {
            // Undo is an editing gesture, not a command, so it keeps its key.
            match code {
                Some(KeyCode::KeyF) => self.start_search(None),
                Some(KeyCode::KeyA) => {
                    self.doc.editor.select_all();
                    self.poke();
                }
                Some(KeyCode::KeyC) => {
                    let result = self.copy_selection();
                    self.report(result);
                }
                Some(KeyCode::KeyX) => {
                    let result = self.cut_selection();
                    self.report(result);
                }
                Some(KeyCode::KeyV) => {
                    let result = self.paste();
                    self.report(result);
                }
                Some(KeyCode::KeyZ) => {
                    if self.mods.shift_key() {
                        self.doc.editor.redo();
                    } else {
                        self.doc.editor.undo();
                    }
                    self.touch();
                }
                Some(KeyCode::KeyY) => {
                    self.doc.editor.redo();
                    self.touch();
                }
                Some(KeyCode::ArrowLeft) => {
                    self.doc.editor.move_word_left(shift);
                    self.poke();
                }
                Some(KeyCode::ArrowRight) => {
                    self.doc.editor.move_word_right(shift);
                    self.poke();
                }
                Some(KeyCode::Home) => {
                    self.doc.editor.move_doc_start(shift);
                    self.poke();
                }
                Some(KeyCode::End) => {
                    self.doc.editor.move_doc_end(shift);
                    self.poke();
                }
                _ => {}
            }
            return;
        }

        match &event.logical_key {
            Key::Named(NamedKey::Escape) => self.to_action(),
            Key::Named(NamedKey::ArrowLeft) => {
                self.doc.editor.move_left(shift);
                self.doc.editor.break_undo_group();
                self.poke();
            }
            Key::Named(NamedKey::ArrowRight) => {
                self.doc.editor.move_right(shift);
                self.doc.editor.break_undo_group();
                self.poke();
            }
            Key::Named(NamedKey::F3) => self.repeat_search(!shift),
            Key::Named(NamedKey::ArrowUp) => self.move_vertical(-1, shift),
            Key::Named(NamedKey::ArrowDown) => self.move_vertical(1, shift),
            Key::Named(NamedKey::Home) => {
                self.doc.editor.move_home(shift);
                self.poke();
            }
            Key::Named(NamedKey::End) => {
                self.doc.editor.move_end(shift);
                self.poke();
            }
            Key::Named(NamedKey::PageUp) => {
                let h = ui::text_height(self.screen.rows()) as isize;
                self.move_vertical(-h, shift);
            }
            Key::Named(NamedKey::PageDown) => {
                let h = ui::text_height(self.screen.rows()) as isize;
                self.move_vertical(h, shift);
            }
            Key::Named(NamedKey::Backspace) => {
                self.doc.editor.backspace();
                self.touch();
            }
            Key::Named(NamedKey::Delete) => {
                self.doc.editor.delete_forward();
                self.touch();
            }
            Key::Named(NamedKey::Enter) => {
                self.doc.editor.insert_newline();
                self.touch();
            }
            Key::Named(NamedKey::Tab) => {
                self.doc.editor.insert_tab();
                self.touch();
            }
            Key::Named(NamedKey::Space) => {
                self.doc.editor.insert_char(' ');
                self.touch();
            }
            Key::Character(text) => {
                if self.mods.alt_key() && !self.mods.control_key() {
                    return;
                }
                if text.chars().any(|c| !c.is_control()) {
                    self.doc.editor.insert_text(text);
                    self.touch();
                }
            }
            _ => {}
        }
    }

    /// Action mode: the document is frozen, keys go to the command line.
    pub(super) fn on_key_action(&mut self, el: &ActiveEventLoop, event: KeyEvent) {
        if let Some(question) = self.confirm.clone() {
            let _ = question;
            if matches!(&event.logical_key, Key::Character(c) if c.eq_ignore_ascii_case("y")) {
                self.confirm = None;
                match self.delete_current() {
                    Ok(msg) => {
                        self.set_status(msg);
                        self.to_write();
                    }
                    Err(e) => self.command_error = e,
                }
            } else {
                self.confirm = None;
                self.set_status("cancelled");
                self.to_write();
            }
            self.poke();
            return;
        }

        match &event.logical_key {
            Key::Named(NamedKey::Escape) => {
                if !self.command.is_empty() {
                    self.command.clear();
                    self.command_error.clear();
                } else if !self.has_document && self.home != Home::Boot {
                    self.home = Home::Boot;
                } else if self.has_document {
                    self.to_write();
                }
                self.poke();
            }
            Key::Named(NamedKey::Enter) => self.submit_command(el),
            Key::Named(NamedKey::Tab) => {
                if let Some(completion) = command::complete(&self.command) {
                    self.command = completion.completed;
                }
                self.poke();
            }
            Key::Named(NamedKey::ArrowUp) => {
                if self.home == Home::Boot && !self.has_document && self.command.is_empty() {
                    self.boot_step(-1);
                } else {
                    self.history_step(-1);
                }
                self.poke();
            }
            Key::Named(NamedKey::ArrowDown) => {
                if self.home == Home::Boot && !self.has_document && self.command.is_empty() {
                    self.boot_step(1);
                } else {
                    self.history_step(1);
                }
                self.poke();
            }
            Key::Named(NamedKey::Backspace) => {
                self.command.pop();
                self.command_error.clear();
                self.poke();
            }
            Key::Named(NamedKey::Space) => {
                self.command.push(' ');
                self.command_error.clear();
                self.poke();
            }
            Key::Character(text) => {
                if self.mods.alt_key() {
                    return;
                }
                for ch in text.chars().filter(|c| !c.is_control()) {
                    self.command.push(ch);
                }
                self.command_error.clear();
                self.poke();
            }
            _ => {}
        }
    }

    /// Which command list `help` shows.
    pub(super) fn help_scope(&self) -> command::Scope {
        if self.program.is_some() || self.home == Home::SoftHand {
            command::Scope::Program
        } else if self.has_document || self.home == Home::Inspiro {
            command::Scope::Editor
        } else {
            command::Scope::Boot
        }
    }

    /// Help: arrows walk the list, any other key goes back.
    pub(super) fn on_key_help(&mut self, event: KeyEvent) {
        let specs = if self.help_all {
            command::SPECS.len()
        } else {
            command::specs_for(self.help_scope()).len()
        };
        match &event.logical_key {
            Key::Named(NamedKey::ArrowUp) if !self.help_about => {
                self.help_selected = self.help_selected.saturating_sub(1);
                self.poke();
            }
            Key::Named(NamedKey::ArrowDown) if !self.help_about => {
                self.help_selected = (self.help_selected + 1).min(specs.saturating_sub(1));
                self.poke();
            }
            Key::Named(NamedKey::Escape) => self.leave_help(),
            // Any other key leaves the screen the same way Esc does; this
            // keeps a later `help` from returning to SETTINGS by mistake.
            _ => self.leave_help(),
        }
    }

    /// Leave the help/about screen: back to SETTINGS when it was opened from
    /// there, otherwise back to the work it covered.
    pub(super) fn leave_help(&mut self) {
        if self.help_from_setup {
            self.help_from_setup = false;
            self.mode = Mode::Setup;
        } else {
            self.to_write();
        }
        self.dirty = true;
    }

    pub(super) fn submit_command(&mut self, el: &ActiveEventLoop) {
        let line = self.command.trim().to_string();
        let command = match command::parse(&line) {
            Ok(command) => command,
            Err(CommandError::Empty) => {
                if self.has_document {
                    self.to_write();
                } else if self.home == Home::Poker {
                    self.start_poker();
                } else if self.home == Home::Scripture {
                    self.start_scripture();
                } else {
                    self.boot_activate();
                }
                return;
            }
            Err(e) => {
                self.command_error = e.message();
                self.poke();
                return;
            }
        };

        if self.history.last() != Some(&line) {
            self.history.push(line);
            if self.history.len() > 200 {
                self.history.remove(0);
            }
        }
        self.history_pos = None;

        match self.run_command(el, command) {
            Ok(message) => {
                if !message.is_empty() {
                    self.set_status(message);
                }
                self.command.clear();
                self.command_error.clear();
                // Commands that opened another screen keep it.
                if self.mode == Mode::Action && self.confirm.is_none() {
                    self.to_write();
                }
                // A program's prompt closes again after a command, so you are
                // back to drawing without another keypress.
                if matches!(self.mode, Mode::Program | Mode::Riff) {
                    self.program_prompt = false;
                }
            }
            Err(e) => {
                // Keep the typed line so it can be corrected.
                self.command_error = e;
            }
        }
        self.poke();
    }

    /// A small confirmation box; any key closes it and returns.
    pub(super) fn open_notice(&mut self, message: impl Into<String>, back: Mode) {
        self.notice_text = message.into();
        self.notice_back = back;
        self.mode = Mode::Notice;
        self.dirty = true;
    }

    pub(super) fn on_key_notice(&mut self) {
        self.mode = self.notice_back;
        self.dirty = true;
    }

    #[allow(clippy::wrong_self_convention)]
    pub(super) fn to_action(&mut self) {
        self.mode = Mode::Action;
        self.command.clear();
        self.command_error.clear();
        self.history_pos = None;
        self.confirm = None;
        self.dirty = true;
    }

    #[allow(clippy::wrong_self_convention)]
    pub(super) fn to_write(&mut self) {
        if matches!(self.mode, Mode::RiffInsert) {
            self.riff.explorer = None;
            self.mode = Mode::Riff;
            self.program_prompt = false;
            self.dirty = true;
            return;
        }
        if self.mode == Mode::Riff
            || (!self.has_document && self.home == Home::Riff && self.mode == Mode::Action)
        {
            self.mode = Mode::Riff;
            self.program_prompt = false;
            self.command.clear();
            self.command_error.clear();
            self.dirty = true;
            return;
        }
        if self.program.is_some() {
            self.mode = Mode::Program;
            self.program_prompt = false;
            self.command.clear();
            self.command_error.clear();
            self.history_pos = None;
            self.confirm = None;
            self.dirty = true;
            return;
        }
        self.mode = if self.has_document {
            Mode::Write
        } else {
            Mode::Action
        };
        self.command.clear();
        self.command_error.clear();
        self.history_pos = None;
        self.confirm = None;
        self.dirty = true;
    }

    pub(super) fn history_step(&mut self, delta: isize) {
        if self.history.is_empty() {
            return;
        }
        let last = self.history.len() as isize - 1;
        let next = match self.history_pos {
            None => {
                if delta < 0 {
                    last
                } else {
                    return;
                }
            }
            Some(pos) => pos as isize + delta,
        };
        if next < 0 {
            self.history_pos = Some(0);
        } else if next > last {
            self.history_pos = None;
            self.command.clear();
            return;
        } else {
            self.history_pos = Some(next as usize);
        }
        if let Some(pos) = self.history_pos {
            self.command = self.history[pos].clone();
        }
    }

    pub(super) fn move_vertical(&mut self, delta: isize, extend: bool) {
        let (text_width, _) = ui::layout(self.screen.cols());
        for _ in 0..delta.unsigned_abs() {
            if delta < 0 {
                self.doc.editor.move_up(text_width, extend);
            } else {
                self.doc.editor.move_down(text_width, extend);
            }
        }
        self.doc.editor.break_undo_group();
        // Without this the redraw waits for the next cursor blink, which makes
        // vertical movement feel like it lags a beat behind the key repeat.
        self.poke();
    }

    /// Mark the document edited and restart the autosave timer.
    pub(super) fn touch(&mut self) {
        self.doc.last_edit = Instant::now();
        self.poke();
    }
}
