//! Commands that only mean something inside a program.

use super::super::*;

impl App {
    pub(super) fn cmd_program(&mut self, command: &Command) -> Result<String, String> {
        let Some(program) = self.program.as_mut() else {
            return Err("only a program has tools".into());
        };
        program.command(command)
    }

    /// `web [address]` from any prompt: the browser comes to the front.
    pub(in crate::app) fn cmd_web(&mut self, typed: Option<String>) -> Result<String, String> {
        if !self.is_installed("wanderlust") {
            return Err("wanderlust is not installed - see SOFTWARE".into());
        }
        self.start_wanderlust(typed.as_deref());
        Ok(String::new())
    }

    /// `back`, `forward`, `reload`: the browser's own words.
    pub(in crate::app) fn cmd_browse(&mut self, command: &Command) -> Result<String, String> {
        if self.program_id != Some("wanderlust") {
            return Err("only the browser has pages - try web".into());
        }
        self.cmd_program(command)
    }

    pub(super) fn cmd_riff(&mut self, action: RiffAction) -> Result<String, String> {
        self.run_riff(action)
    }
}
