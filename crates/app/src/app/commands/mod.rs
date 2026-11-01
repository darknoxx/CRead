//! Every command, one handler each.
//!
//! `run_command` is only a switchboard: it maps a [`Command`] to the function
//! that does the work, grouped by what the command is about. Adding a command
//! means adding a parse arm in core, a family here, and one line in the match.

use super::*;

mod document;
mod program;
mod screen;
mod system;

impl App {
    pub(super) fn run_command(
        &mut self,
        el: &ActiveEventLoop,
        command: Command,
    ) -> Result<String, String> {
        match command {
            Command::Save => self.cmd_save(),
            Command::SaveAs(name) => self.cmd_save_as(&name),
            Command::Open(name) => self.cmd_open(name),
            Command::New => self.cmd_new(),
            Command::Rename(name) => self.rename_current(&name),
            Command::Delete => self.cmd_delete(),
            Command::Undo => self.cmd_undo(false),
            Command::Redo => self.cmd_undo(true),
            Command::View => self.cmd_view(),
            Command::Find(query) => self.cmd_find(query),

            Command::Theme(argument) => self.cmd_theme(argument),
            Command::Scanlines { on, roll } => self.cmd_scanlines(on, roll),
            Command::Bloom(want) => self.cmd_bloom(want),
            Command::Roll(want) => self.cmd_roll(want),
            Command::Autosave(want) => self.cmd_autosave(want),
            Command::Beep(want) => self.cmd_beep(want),
            Command::Typewriter(want) => self.cmd_typewriter(want),
            Command::Font(face) => self.cmd_font(face),
            Command::Fullscreen(on) => self.cmd_fullscreen(on),

            Command::Sync => self.cmd_sync(),
            Command::Software => self.cmd_software(),
            Command::Settings => self.cmd_settings(),
            Command::Update => self.cmd_update(),
            Command::Help(argument) => self.cmd_help(argument),
            Command::About => self.cmd_about(),
            Command::Exit => self.cmd_exit(el),
            Command::ExitForce => self.cmd_exit_force(el),

            Command::Riff(action) => self.cmd_riff(action),
            Command::Tool(_) | Command::Size(_) | Command::Colour(_) | Command::Mute => {
                self.cmd_program(&command)
            }
        }
    }
}
