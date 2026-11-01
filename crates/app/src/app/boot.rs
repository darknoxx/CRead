//! The start screen: which rows exist and what they do.
//!
//! `boot_items` is the list the keyboard walks and `boot_activate` runs; the
//! presentation is built from the same list in `boot_rows`, so a new row is
//! one struct variant, not two lists to keep in step.

use super::*;

/// One row of the boot screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BootItem {
    /// The vault: the first row, always.
    Vault,
    /// The rule between the vault and the modules. Never selectable.
    Rule,
    /// The editor, the first module.
    Inspiro,
    /// Index into [`PROGRAMS`].
    Program(usize),
    /// The install screen.
    Software,
    /// The system settings.
    Setup,
}
impl BootItem {
    pub(super) fn selectable(self) -> bool {
        !matches!(self, BootItem::Rule)
    }
}
/// A module row carries its drive letter; the letter only counts for drives.
fn drive_row(letter: &mut u8, label: impl Into<String>, detail: impl Into<String>) -> ui::BootItem {
    let row = ui::BootItem::row(
        format!("{}: {}", *letter as char, label.into()),
        detail,
        false,
    );
    *letter += 1;
    row
}

impl App {
    pub(super) fn boot_items(&self) -> Vec<BootItem> {
        // Modules carry drive letters, in the order they were installed; the
        // vault and the shelf are not drives and sit below the rule.
        let mut items = vec![BootItem::Inspiro];
        for id in &self.prefs.installed {
            if let Some(index) = PROGRAMS.iter().position(|program| program.id == id) {
                items.push(BootItem::Program(index));
            }
        }
        items.push(BootItem::Rule);
        items.push(BootItem::Software);
        items.push(BootItem::Vault);
        items.push(BootItem::Setup);
        items
    }

    /// One row per [`BootItem`], so behaviour and presentation cannot get out
    /// of step: adding a row means adding it to `boot_items` once.
    pub(super) fn boot_rows(&self) -> Vec<ui::BootItem> {
        let mut letter = b'A';
        self.boot_items()
            .into_iter()
            .map(|item| match item {
                BootItem::Rule => ui::BootItem::rule(),
                BootItem::Inspiro => drive_row(
                    &mut letter,
                    "INSPIRO",
                    "where your ideas have room to grow.",
                ),
                BootItem::Program(index) => {
                    let program = &PROGRAMS[index];
                    drive_row(&mut letter, program.entry, program.tagline)
                }
                BootItem::Software => {
                    ui::BootItem::row("SOFTWARE", "install and remove modules", true)
                }
                BootItem::Vault => {
                    ui::BootItem::row("THE VAULT", "documents, pictures, cartridges", true)
                }
                BootItem::Setup => {
                    ui::BootItem::row("SETTINGS", "webdav, theme, font, screen", true)
                }
            })
            .collect()
    }

    pub(super) fn boot_activate(&mut self) {
        // Only the boot screen has a selectable list; on Inspiro's front door
        // an empty Enter has nothing to run.
        if self.home != Home::Boot {
            return;
        }
        let Some(item) = self.boot_items().get(self.boot_selected).copied() else {
            return;
        };
        match item {
            BootItem::Vault => self.open_browser(),
            BootItem::Rule => {}
            BootItem::Inspiro => self.start_loading(Pending::Inspiro),
            BootItem::Program(index) => self.start_loading(Pending::Program(index)),
            BootItem::Software => self.open_software(),
            BootItem::Setup => self.open_setup(),
        }
    }

    /// Move the boot selection, stepping over rows that are not selectable
    /// (the rule). Stops at both ends instead of wrapping.
    pub(super) fn boot_step(&mut self, delta: isize) {
        let items = self.boot_items();
        let mut index = self.boot_selected as isize;
        loop {
            index += delta;
            if index < 0 || index >= items.len() as isize {
                return;
            }
            if items[index as usize].selectable() {
                self.boot_selected = index as usize;
                return;
            }
        }
    }
}
