//! The printer and USB backups: two SETTINGS screens and their commands.
//!
//! The work (`lp`, `lpstat`, `lsblk`, `udisksctl`, copying) happens on a
//! short-lived thread per job; it reports through `App::jobs` and rings the
//! waker, so the UI never waits on a printer or a stick.

use std::sync::mpsc::Sender;

use super::*;
use cread_peripherals::printer::{self, Printers};
use cread_peripherals::usb::{self, Report, Stick};

/// What a background job reports.
pub(super) enum Job {
    Printers(Result<Printers, String>),
    /// The printer it went to (if one was named) and the job id.
    Printed(Option<String>, Result<String, String>),
    Sticks(Result<Vec<Stick>, String>),
    Progress(usize, usize),
    BackedUp(Result<Report, String>),
}

#[derive(Default)]
pub(super) struct PrinterState {
    pub(super) printers: Printers,
    pub(super) loaded: bool,
    pub(super) selected: usize,
    pub(super) note: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum BackupPhase {
    /// Asking `lsblk`.
    #[default]
    Looking,
    /// The sticks are listed (maybe none).
    Choose,
    /// Copying; `progress` moves.
    Running,
    /// Finished, the note says how it went.
    Done,
}

#[derive(Default)]
pub(super) struct BackupState {
    pub(super) phase: BackupPhase,
    pub(super) sticks: Vec<Stick>,
    pub(super) selected: usize,
    pub(super) progress: (usize, usize),
    pub(super) target: String,
    pub(super) note: String,
    /// Where Esc goes back to.
    pub(super) from: Option<Mode>,
}

impl App {
    /// Run `work` on its own thread; whatever it sends wakes the loop.
    fn spawn_job(&self, work: impl FnOnce(&dyn Fn(Job)) + Send + 'static) {
        let tx: Sender<Job> = self.jobs_tx.clone();
        let waker = self.waker.clone();
        std::thread::spawn(move || {
            let emit = |job: Job| {
                if tx.send(job).is_ok()
                    && let Some(waker) = waker.as_ref()
                {
                    waker();
                }
            };
            work(&emit);
        });
    }

    pub(super) fn poll_jobs(&mut self) {
        let mut any = false;
        while let Ok(job) = self.jobs_rx.try_recv() {
            self.job_done(job);
            any = true;
        }
        if any {
            self.poke();
        }
    }

    pub(super) fn job_done(&mut self, job: Job) {
        match job {
            Job::Printers(Ok(printers)) => {
                self.printer.printers = printers;
                self.printer.loaded = true;
                let last = self.printer.printers.names.len().saturating_sub(1);
                self.printer.selected = self.printer.selected.min(last);
            }
            Job::Printers(Err(e)) => {
                self.printer.loaded = true;
                self.printer.note = e;
            }
            Job::Printed(printer, Ok(id)) => {
                let on = printer.map(|p| format!(" on {p}")).unwrap_or_default();
                self.set_status(if id.is_empty() {
                    format!("printed{on}")
                } else {
                    format!("printed{on} ({id})")
                });
            }
            Job::Printed(_, Err(e)) => self.set_status(format!("cannot print: {e}")),
            Job::Sticks(result) => {
                self.backup.phase = BackupPhase::Choose;
                match result {
                    Ok(sticks) => {
                        self.backup.sticks = sticks;
                        let last = self.backup.sticks.len().saturating_sub(1);
                        self.backup.selected = self.backup.selected.min(last);
                    }
                    Err(e) => {
                        self.backup.sticks.clear();
                        self.backup.note = e;
                    }
                }
            }
            Job::Progress(done, total) => self.backup.progress = (done, total),
            Job::BackedUp(result) => {
                self.backup.phase = BackupPhase::Done;
                self.backup.note = match result {
                    Ok(report) => {
                        let folder = report
                            .folder
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        let mut note = format!(
                            "{} files in {folder} - the stick can come out",
                            report.copied
                        );
                        if !report.skipped.is_empty() {
                            note = format!(
                                "{note}; {} not copied (names the stick does not allow): {}",
                                report.skipped.len(),
                                report.skipped.join(", ")
                            );
                        }
                        note
                    }
                    Err(e) => format!("backup failed: {e}"),
                };
            }
        }
    }

    // ── printing ────────────────────────────────────────────────────────────

    /// `print`: the document as text, or Softhand's picture as an image.
    pub(in crate::app) fn cmd_print(&mut self) -> Result<String, String> {
        if self.in_softhand() {
            let picture = self
                .program
                .as_ref()
                .and_then(|program| program.picture())
                .ok_or("no picture to print")?;
            let path = std::env::temp_dir().join(format!("cread-print-{}.png", std::process::id()));
            picture::save(&path, picture)?;
            let title = self
                .picture_path
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "picture".into());
            let chosen = self.prefs.printer.clone();
            self.spawn_job(move |emit| {
                let target = destination(chosen);
                let result = printer::print_file(target.as_deref(), &title, &path);
                let _ = std::fs::remove_file(&path);
                emit(Job::Printed(target, result));
            });
            return Ok("printing ...".into());
        }
        if !self.has_document {
            return Err("open a document to print".into());
        }
        let text = printer::page_text(self.doc.editor.lines());
        let title = self.doc.title();
        let chosen = self.prefs.printer.clone();
        self.spawn_job(move |emit| {
            let target = destination(chosen);
            let result = printer::print_text(target.as_deref(), &title, &text);
            emit(Job::Printed(target, result));
        });
        Ok("printing ...".into())
    }

    pub(super) fn load_printers(&self) {
        self.spawn_job(|emit| emit(Job::Printers(printer::list())));
    }

    pub(super) fn open_printer(&mut self) {
        self.printer.note.clear();
        self.load_printers();
        self.mode = Mode::Printer;
        self.poke();
    }

    /// The printer jobs go to, as far as the screen knows: the chosen one,
    /// or the only one there is.
    fn printer_in_use(&self) -> Option<String> {
        printer::resolve(self.prefs.printer.as_deref(), &self.printer.printers.names)
    }

    /// The SETTINGS row's value.
    pub(super) fn printer_label(&self) -> String {
        match (self.printer_in_use(), self.printer.loaded) {
            (Some(name), _) => name,
            (None, false) => String::new(),
            (None, true) if self.printer.printers.names.is_empty() => "none".into(),
            (None, true) => "not chosen".into(),
        }
    }

    pub(super) fn on_key_printer(&mut self, event: KeyEvent) {
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => self.mode = Mode::Setup,
            Key::Named(NamedKey::ArrowUp) => {
                self.printer.selected = self.printer.selected.saturating_sub(1);
            }
            Key::Named(NamedKey::ArrowDown) => {
                let last = self.printer.printers.names.len().saturating_sub(1);
                self.printer.selected = (self.printer.selected + 1).min(last);
            }
            Key::Named(NamedKey::Enter) => {
                if let Some(name) = self
                    .printer
                    .printers
                    .names
                    .get(self.printer.selected)
                    .cloned()
                {
                    self.prefs.printer = Some(name.clone());
                    self.save_prefs();
                    self.printer.note = format!("print now goes to {name}");
                }
            }
            Key::Character(text) if text.eq_ignore_ascii_case("r") => {
                self.printer.note.clear();
                self.load_printers();
            }
            _ => return,
        }
        self.poke();
    }

    pub(super) fn draw_printer(&self, screen: &mut Screen) {
        let rows: Vec<ui::MenuItem> = if self.printer.printers.names.is_empty() {
            let text = if self.printer.loaded {
                "no printer found - see the manual"
            } else {
                "asking the print system ..."
            };
            vec![ui::MenuItem::row("", text, "")]
        } else {
            self.printer
                .printers
                .names
                .iter()
                .map(|name| {
                    let value = if self.printer_in_use().as_deref() == Some(name.as_str()) {
                        "in use"
                    } else {
                        ""
                    };
                    ui::MenuItem::row("", name.clone(), value)
                })
                .collect()
        };
        ui::draw_menu(
            screen,
            &self.theme,
            "PRINTER",
            "Enter choose   R refresh",
            &rows,
            self.printer.selected,
        );
        if !self.printer.note.is_empty() {
            let y = 3 + rows.len() as u32 + 2;
            ui::put_centered(screen, y, &self.printer.note, self.theme.dim, self.theme.bg);
        }
    }

    // ── backup ──────────────────────────────────────────────────────────────

    pub(in crate::app) fn cmd_backup(&mut self) -> Result<String, String> {
        self.open_backup();
        Ok(String::new())
    }

    pub(super) fn open_backup(&mut self) {
        if self.backup.phase == BackupPhase::Running {
            self.mode = Mode::Backup;
            return;
        }
        let from = if self.mode == Mode::Setup {
            Mode::Setup
        } else {
            Mode::Action
        };
        self.backup = BackupState {
            from: Some(from),
            ..BackupState::default()
        };
        self.find_sticks();
        self.mode = Mode::Backup;
        self.poke();
    }

    fn find_sticks(&mut self) {
        self.backup.phase = BackupPhase::Looking;
        self.backup.note.clear();
        self.spawn_job(|emit| emit(Job::Sticks(usb::sticks())));
    }

    fn start_backup(&mut self) {
        let Some(stick) = self.backup.sticks.get(self.backup.selected).cloned() else {
            return;
        };
        let vault = self.vault();
        let date = chrono::Local::now().format("%Y-%m-%d").to_string();
        self.backup.phase = BackupPhase::Running;
        self.backup.progress = (0, 0);
        self.backup.target = stick.name();
        self.backup.note.clear();
        self.spawn_job(move |emit| {
            let result = usb::mount(&stick).and_then(|root| {
                let report = usb::backup(&vault, &root, &date, |done, total| {
                    emit(Job::Progress(done, total))
                });
                // Let go of the stick even when the copy failed half way.
                let released = usb::release(&stick);
                let report = report?;
                released.map_err(|e| format!("copied, but the stick is still busy: {e}"))?;
                Ok(report)
            });
            emit(Job::BackedUp(result));
        });
    }

    pub(super) fn on_key_backup(&mut self, event: KeyEvent) {
        let phase = self.backup.phase;
        match &event.logical_key {
            Key::Named(NamedKey::Escape) if phase != BackupPhase::Running => {
                self.mode = self.backup.from.unwrap_or(Mode::Action);
            }
            Key::Named(NamedKey::Enter) if phase == BackupPhase::Choose => self.start_backup(),
            Key::Named(NamedKey::Enter) if phase == BackupPhase::Done => {
                self.mode = self.backup.from.unwrap_or(Mode::Action);
            }
            Key::Named(NamedKey::ArrowUp) if phase == BackupPhase::Choose => {
                self.backup.selected = self.backup.selected.saturating_sub(1);
            }
            Key::Named(NamedKey::ArrowDown) if phase == BackupPhase::Choose => {
                let last = self.backup.sticks.len().saturating_sub(1);
                self.backup.selected = (self.backup.selected + 1).min(last);
            }
            Key::Character(text)
                if text.eq_ignore_ascii_case("r")
                    && matches!(phase, BackupPhase::Choose | BackupPhase::Done) =>
            {
                self.find_sticks();
            }
            _ => return,
        }
        self.poke();
    }

    pub(super) fn draw_backup(&self, screen: &mut Screen) {
        if self.backup.phase == BackupPhase::Running {
            let (done, total) = self.backup.progress;
            let fraction = if total == 0 {
                0.0
            } else {
                done as f32 / total as f32
            };
            let label = format!("BACKUP TO {}", self.backup.target);
            let hint = format!("{done} of {total} files");
            ui::draw_loading(screen, &self.theme, &label, fraction, false, &hint);
            return;
        }
        let rows: Vec<ui::MenuItem> = match self.backup.phase {
            BackupPhase::Looking => vec![ui::MenuItem::row("", "looking for USB sticks ...", "")],
            _ if self.backup.sticks.is_empty() => {
                vec![ui::MenuItem::row(
                    "",
                    "insert a USB stick, then press R",
                    "",
                )]
            }
            _ => self
                .backup
                .sticks
                .iter()
                .map(|stick| {
                    ui::MenuItem::row(
                        stick.size.clone(),
                        format!("{}  {}", stick.name(), stick.fstype),
                        "",
                    )
                })
                .collect(),
        };
        let hint = if self.backup.phase == BackupPhase::Done {
            "Enter done   R again"
        } else {
            "Enter back up   R look again"
        };
        ui::draw_menu(
            screen,
            &self.theme,
            "USB BACKUP",
            hint,
            &rows,
            self.backup.selected,
        );
        let mut note = self.backup.note.clone();
        if note.is_empty() && self.doc.editor.modified() {
            note = "the open document has unsaved changes - save it first".into();
        }
        if !note.is_empty() {
            let y = 3 + rows.len() as u32 + 2;
            let width = screen.cols().saturating_sub(8);
            screen.put_str_ellipsis(4, y, &note, width, self.theme.dim, self.theme.bg);
        }
    }
}

/// The printer for a job, decided on the job's thread: asking CUPS for the
/// list takes a moment, and is only needed when nothing was chosen.
fn destination(chosen: Option<String>) -> Option<String> {
    if chosen.is_some() {
        return chosen;
    }
    let known = printer::list().map(|p| p.names).unwrap_or_default();
    printer::resolve(None, &known)
}
