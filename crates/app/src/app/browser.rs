//! The vault browser: folders, documents, pictures and cartridges in one list.
//!
//! The shell owns the layout (`Documents/`, `Pictures/`, `Roms/`) and the
//! programs own their files, so the browser only decides which program gets
//! which file and how a folder is walked.

use super::*;

#[derive(Clone, PartialEq, Eq)]
pub(super) enum BrowserMode {
    Browse,
    /// `delete x.md?` waiting for y/n.
    ConfirmDelete(PathBuf),
    /// Renaming `old` as it is typed.
    Rename {
        old: PathBuf,
        new: String,
    },
}
pub(super) struct Browser {
    pub(super) input: String,
    pub(super) entries: Vec<(String, Action)>,
    pub(super) filtered: Vec<usize>,
    pub(super) selected: usize,
    pub(super) title: String,
    pub(super) mode: BrowserMode,
    /// Pictures cannot be renamed or deleted from here, so the hint differs.
    pub(super) pictures: bool,
    /// Folder below the vault root that is shown; empty means the root.
    pub(super) path: PathBuf,
}
impl Browser {
    pub(super) fn empty() -> Self {
        Self {
            input: String::new(),
            entries: Vec::new(),
            filtered: Vec::new(),
            selected: 0,
            title: "THE VAULT".into(),
            mode: BrowserMode::Browse,
            pictures: false,
            path: PathBuf::new(),
        }
    }

    pub(super) fn refilter(&mut self) {
        let needle = self.input.to_lowercase();
        self.filtered = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, (name, _))| needle.is_empty() || name.to_lowercase().contains(&needle))
            .map(|(i, _)| i)
            .collect();
        self.selected = self.selected.min(self.filtered.len().saturating_sub(1));
    }

    pub(super) fn items(&self) -> Vec<String> {
        self.filtered
            .iter()
            .map(|i| self.entries[*i].0.clone())
            .collect()
    }

    pub(super) fn current(&self) -> Option<&Action> {
        self.filtered
            .get(self.selected)
            .map(|i| &self.entries[*i].1)
    }
}
/// Which program opens this file, if any. Folders are handled by the browser.
fn file_action(path: &Path) -> Option<Action> {
    let extension = path.extension()?.to_string_lossy().to_lowercase();
    match extension.as_str() {
        "md" | "txt" | "markdown" => Some(Action::Open(path.to_path_buf())),
        "png" => Some(Action::OpenPicture(path.to_path_buf())),
        "gb" | "gbc" => Some(Action::OpenRom(path.to_path_buf())),
        _ => None,
    }
}

impl App {
    pub(super) fn open_browser(&mut self) {
        self.prepare_vault();
        self.browser = Browser::empty();
        self.refresh_browser();
        self.mode = Mode::Browser;
        self.dirty = true;
    }

    /// Folders first, then the files a program here can open. Nothing else is
    /// listed: this is a shelf, not a file manager.
    pub(super) fn refresh_browser(&mut self) {
        let here = self.vault().join(&self.browser.path);
        let at_root = self.browser.path.as_os_str().is_empty();
        let mut entries: Vec<(String, Action)> = Vec::new();
        if at_root {
            entries.push(("+ new document".to_string(), Action::NewFile));
            entries.push(("+ sync now".to_string(), Action::SyncNow));
        } else {
            entries.push(("..".to_string(), Action::Up));
        }

        let mut folders: Vec<(String, Action)> = Vec::new();
        let mut files: Vec<(String, Action)> = Vec::new();
        match std::fs::read_dir(&here) {
            Ok(listing) => {
                for entry in listing.flatten() {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if name.starts_with('.') {
                        continue;
                    }
                    let path = entry.path();
                    if path.is_dir() {
                        let relative = self.browser.path.join(&name);
                        folders.push((format!("{name}/"), Action::EnterDir(relative)));
                    } else if let Some(action) = file_action(&path) {
                        files.push((name, action));
                    }
                }
            }
            Err(e) => self.set_status(format!("cannot list {}: {e}", here.display())),
        }
        folders.sort_by_key(|(name, _)| name.to_lowercase());
        files.sort_by_key(|(name, _)| name.to_lowercase());
        entries.extend(folders);
        entries.extend(files);

        self.browser.title = if at_root {
            "THE VAULT".into()
        } else {
            format!("THE VAULT /{}", self.browser.path.display())
        };
        self.browser.entries = entries;
        self.browser.refilter();
    }

    /// Open a folder in the browser; the filter starts fresh.
    pub(super) fn browser_enter(&mut self, path: PathBuf) {
        self.browser.path = path;
        self.browser.input.clear();
        self.browser.selected = 0;
        self.refresh_browser();
        self.poke();
    }

    /// One folder up. False at the root, where Esc leaves the vault instead.
    pub(super) fn browser_up(&mut self) -> bool {
        if self.browser.path.as_os_str().is_empty() {
            return false;
        }
        let parent = self
            .browser
            .path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        self.browser_enter(parent);
        true
    }

    pub(super) fn browser_accept(&mut self) {
        let Some(action) = self.browser.current().cloned() else {
            return;
        };
        match action {
            Action::Open(path) => self.start_loading(Pending::Document(path)),
            Action::OpenPicture(path) => self.start_loading(Pending::Picture(path)),
            Action::OpenRom(path) => self.start_loading(Pending::Rom(path)),
            Action::EnterDir(path) => {
                self.browser_enter(path);
                return;
            }
            Action::Up => {
                self.browser_up();
                return;
            }
            Action::NewFile => self.start_loading(Pending::NewDocument),
            Action::SyncNow => {
                if self.sync_cfg.enabled && self.sync.is_some() {
                    self.request_sync();
                    self.set_status("sync requested");
                } else {
                    self.set_status("webdav is off - try 'settings'");
                }
            }
        }
        // Loading is its own destination; anything else drops back to the
        // document or the home screen.
        if self.mode != Mode::Loading {
            self.to_write();
        }
    }

    pub(super) fn browser_target(&self) -> Option<PathBuf> {
        match self.browser.current()? {
            Action::Open(path) => Some(path.clone()),
            _ => None,
        }
    }

    pub(super) fn browser_delete(&mut self, path: PathBuf) {
        let name = self.doc_title_of(&path);
        if let Err(e) = std::fs::remove_file(&path) {
            self.set_status(format!("delete failed: {e}"));
            return;
        }
        if self.doc.path.as_deref() == Some(path.as_path()) {
            self.doc = Document {
                path: None,
                title_hint: None,
                editor: Editor::new(),
                last_edit: Instant::now(),
            };
            self.has_document = false;
        }
        self.unsynced = true;
        self.set_status(format!("deleted {name}"));
        self.refresh_browser();
    }

    pub(super) fn browser_rename(&mut self, old: PathBuf, new: &str) {
        let target = match self.rename_target(&old, new.trim()) {
            Ok(target) => target,
            Err(e) => {
                self.set_status(e);
                return;
            }
        };
        if target != old && target.exists() {
            self.set_status(format!("{} already exists", self.doc_title_of(&target)));
            return;
        }
        if let Err(e) = std::fs::rename(&old, &target) {
            self.set_status(format!("rename failed: {e}"));
            return;
        }
        if self.doc.path.as_deref() == Some(old.as_path()) {
            self.doc.path = Some(target.clone());
        }
        self.unsynced = true;
        self.set_status(format!("renamed to {}", self.doc_title_of(&target)));
        self.refresh_browser();
    }

    pub(super) fn on_key_browser(&mut self, event: KeyEvent) {
        // Modal sub-states first: they swallow everything.
        match self.browser.mode.clone() {
            BrowserMode::ConfirmDelete(path) => {
                if matches!(&event.logical_key, Key::Character(c) if c.eq_ignore_ascii_case("y")) {
                    self.browser.mode = BrowserMode::Browse;
                    self.browser_delete(path);
                } else {
                    self.browser.mode = BrowserMode::Browse;
                    self.set_status("cancelled");
                }
                self.poke();
                return;
            }
            BrowserMode::Rename { old, mut new } => {
                match &event.logical_key {
                    Key::Named(NamedKey::Escape) => {
                        self.browser.mode = BrowserMode::Browse;
                        self.set_status("cancelled");
                    }
                    Key::Named(NamedKey::Enter) => {
                        self.browser.mode = BrowserMode::Browse;
                        self.browser_rename(old, &new);
                    }
                    Key::Named(NamedKey::Backspace) => {
                        new.pop();
                        self.browser.mode = BrowserMode::Rename { old, new };
                    }
                    Key::Named(NamedKey::Space) => {
                        new.push(' ');
                        self.browser.mode = BrowserMode::Rename { old, new };
                    }
                    Key::Character(text) => {
                        for ch in text.chars().filter(|c| !c.is_control()) {
                            new.push(ch);
                        }
                        self.browser.mode = BrowserMode::Rename { old, new };
                    }
                    _ => {}
                }
                self.poke();
                return;
            }
            BrowserMode::Browse => {}
        }

        let ctrl = self.mods.control_key() || self.mods.super_key();
        if ctrl && let PhysicalKey::Code(code) = event.physical_key {
            {
                match code {
                    KeyCode::KeyD => {
                        if let Some(path) = self.browser_target() {
                            self.browser.mode = BrowserMode::ConfirmDelete(path);
                            self.poke();
                        }
                        return;
                    }
                    KeyCode::KeyR => {
                        if let Some(old) = self.browser_target() {
                            let new = self.doc_title_of(&old).trim_end_matches(".md").to_string();
                            self.browser.mode = BrowserMode::Rename { old, new };
                            self.poke();
                        }
                        return;
                    }
                    _ => {}
                }
            }
        }

        match &event.logical_key {
            Key::Named(NamedKey::Escape) => {
                if self.browser.pictures || !self.browser_up() {
                    self.to_write();
                }
            }
            Key::Named(NamedKey::Enter) => self.browser_accept(),
            Key::Named(NamedKey::ArrowLeft) if !self.browser.pictures => {
                self.browser_up();
            }
            Key::Named(NamedKey::ArrowUp) => {
                self.browser.selected = self.browser.selected.saturating_sub(1);
                self.poke();
            }
            Key::Named(NamedKey::ArrowDown) => {
                if self.browser.selected + 1 < self.browser.filtered.len() {
                    self.browser.selected += 1;
                }
                self.poke();
            }
            Key::Named(NamedKey::Backspace) => {
                self.browser.input.pop();
                self.browser.refilter();
                self.poke();
            }
            Key::Named(NamedKey::Space) => {
                self.browser.input.push(' ');
                self.browser.refilter();
                self.poke();
            }
            Key::Character(text) => {
                for ch in text.chars().filter(|c| !c.is_control()) {
                    self.browser.input.push(ch);
                }
                self.browser.refilter();
                self.poke();
            }
            _ => {}
        }
    }
}
