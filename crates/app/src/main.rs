//! CRead — a retro CRT writing terminal.

mod app;
mod audio;
mod beep;
mod cli;
mod screensaver;
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

fn run_update_once() -> ExitCode {
    let api =
        std::env::var("CREAD_UPDATE_API").unwrap_or_else(|_| cread_update::LATEST.to_string());
    let offer = match cread_update::check(&api, ui::VERSION) {
        Ok(None) => {
            println!("cread: {} is the newest", ui::VERSION);
            return ExitCode::SUCCESS;
        }
        Ok(Some(offer)) => offer,
        Err(e) => {
            eprintln!("cread: update check failed: {e}");
            return ExitCode::FAILURE;
        }
    };
    let Ok(target) = std::env::current_exe() else {
        eprintln!("cread: cannot tell which binary is running");
        return ExitCode::FAILURE;
    };
    match cread_update::install(&offer, &target, cread_update::PUBLIC_KEYS) {
        Ok(backup) => {
            println!(
                "cread: updated to {} ({} kept)",
                offer.version,
                backup.display()
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("cread: update failed: {e}");
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
    if opts.update_once {
        return run_update_once();
    }

    match app::run(opts) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("cread: {e}");
            ExitCode::FAILURE
        }
    }
}
