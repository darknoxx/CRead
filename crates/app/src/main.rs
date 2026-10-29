//! CRead — a retro CRT writing terminal.

mod app;
mod cli;
mod ui;

use std::process::ExitCode;

fn main() -> ExitCode {
    let opts = match cli::parse(std::env::args().skip(1)) {
        Ok(cli::ParseOutcome::Run(opts)) => opts,
        Ok(cli::ParseOutcome::Help) => {
            print!("{}", cli::USAGE);
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("cread: {e}\n\n{}", cli::USAGE);
            return ExitCode::FAILURE;
        }
    };

    match app::run(opts) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("cread: {e}");
            ExitCode::FAILURE
        }
    }
}
