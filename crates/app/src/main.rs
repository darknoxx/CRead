//! CRead — a retro CRT writing terminal.

mod app;
mod audio;
mod beep;
mod cli;
mod ui;

use std::process::ExitCode;

use cread_core::storage;

fn run_sync_once(opts: &cli::Options) -> ExitCode {
    let vault = opts.vault.clone().unwrap_or_else(storage::vault_dir);
    let cfg = cread_sync::Config::load();
    if !cfg.is_configured() {
        eprintln!(
            "cread: no WebDAV configured. Write {} (see --help).",
            cread_sync::Config::path().display()
        );
        return ExitCode::FAILURE;
    }
    match cread_sync::run_blocking(&vault, &cfg) {
        Ok(report) => {
            println!("cread: synced {} — {}", vault.display(), report.summary());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("cread: sync failed: {e}");
            ExitCode::FAILURE
        }
    }
}

fn main() -> ExitCode {
    let opts = match cli::parse(std::env::args().skip(1)) {
        Ok(cli::ParseOutcome::Run(opts)) => opts,
        Ok(cli::ParseOutcome::Help) => {
            print!("{}", cli::USAGE);
            return ExitCode::SUCCESS;
        }
        Ok(cli::ParseOutcome::Version) => {
            println!("cread {}", ui::VERSION);
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("cread: {e}\n\n{}", cli::USAGE);
            return ExitCode::FAILURE;
        }
    };

    if opts.sync_once {
        return run_sync_once(&opts);
    }

    match app::run(opts) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("cread: {e}");
            ExitCode::FAILURE
        }
    }
}
