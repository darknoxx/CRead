//! Commands that only mean something inside a program.

use super::super::*;

impl App {
    pub(super) fn cmd_program(&mut self, command: &Command) -> Result<String, String> {
        let Some(program) = self.program.as_mut() else {
            return Err("only a program has tools".into());
        };
        program.command(command)
    }

    pub(super) fn cmd_riff(&mut self, action: RiffAction) -> Result<String, String> {
        self.run_riff(action)
    }
}
