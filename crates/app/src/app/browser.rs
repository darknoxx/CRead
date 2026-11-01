//! One file browser, two roots.
//!
//! The vault (documents, pictures, cartridges) and the music folder are the
//! same problem: a folder, a list, a selection, a way up. The kind decides
//! which files count and what Enter does; the walking, filtering and scrolling
//! are shared, so the two can never disagree about what "one folder up" means.

use super::*;

/// Which shelf a browser is walking.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum BrowserKind {
    /// The whole vault: whatever a program here can open.
    Vault,
    /// Inspiro's room: folders and documents.
    Documents,
    /// Softhand's room: folders and pictures.
    Pictures,
    /// The music folder: albums and playable files.
    Music,
}

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
    pub(super) kind: BrowserKind,
    /// Absolute base. Navigation never leaves it.
    pub(super) root: PathBuf,
    /// Folder below the root that is shown; empty means the root itself.
    pub(super) path: PathBuf,
    pub(super) input: String,
    pub(super) entries: Vec<(String, Action)>,
    pub(super) filtered: Vec<usize>,
    pub(super) selected: usize,
    pub(super) title: String,
    pub(super) mode: BrowserMode,
}

impl Browser {
    /// The vault browser, with the three rooms and everything in them.
    pub(super) fn vault(root: PathBuf) -> Self {
        Self::new(BrowserKind::Vault, root)
    }

    /// Inspiro's browser: starts in `Documents/`, where documents live.
    pub(super) fn documents(vault: &Path) -> Self {
        Self::new(BrowserKind::Documents, storage::documents_in(vault))
    }

    /// Softhand's browser: starts in `Pictures/`, where paintings live.
    pub(super) fn pictures(vault: &Path) -> Self {
        Self::new(BrowserKind::Pictures, storage::pictures_in(vault))
    }

    /// The music browser, opened on the system's music folder.
    pub(super) fn music(root: PathBuf) -> Self {
        Self::new(BrowserKind::Music, root)
    }

    fn new(kind: BrowserKind, root: PathBuf) -> Self {
        Self {
            kind,
            root,
            path: PathBuf::new(),
            input: String::new(),
            entries: Vec::new(),
            filtered: Vec::new(),
            selected: 0,
            title: String::new(),
            mode: BrowserMode::Browse,
        }
    }

    /// The absolute folder being shown.
    pub(super) fn dir(&self) -> PathBuf {
        self.root.join(&self.path)
    }

    /// Rebuild the rows for the current folder.
    pub(super) fn refresh(&mut self) -> Result<(), String> {
        let here = self.dir();
        let at_root = self.path.as_os_str().is_empty();
        let mut entries: Vec<(String, Action)> = Vec::new();
        if at_root && matches!(self.kind, BrowserKind::Vault | BrowserKind::Documents) {
            entries.push(("+ new document".to_string(), Action::NewFile));
        }
        if at_root && self.kind == BrowserKind::Vault {
            entries.push(("+ sync now".to_string(), Action::SyncNow));
        } else if !at_root {
            entries.push(("..".to_string(), Action::Up));
        }

        match self.kind {
            BrowserKind::Music => self.list_music(&here, &mut entries),
            kind => self.list_room(kind, &here, &mut entries)?,
        }

        let label = match self.kind {
            BrowserKind::Vault => "THE VAULT",
            BrowserKind::Documents => "DOCUMENTS",
            BrowserKind::Pictures => "PICTURES",
            BrowserKind::Music => "RIFF \u{00b7} insert music",
        };
        self.title = if at_root {
            label.into()
        } else {
            format!("{label} /{}", self.path.display())
        };
        self.entries = entries;
        self.refilter();
        Ok(())
    }

    /// Folders first, then the files this room cares about. Nothing else is
    /// listed: this is a shelf, not a file manager.
    fn list_room(
        &self,
        kind: BrowserKind,
        here: &Path,
        entries: &mut Vec<(String, Action)>,
    ) -> Result<(), String> {
        let listing =
            std::fs::read_dir(here).map_err(|e| format!("cannot list {}: {e}", here.display()))?;
        let mut folders: Vec<(String, Action)> = Vec::new();
        let mut files: Vec<(String, Action)> = Vec::new();
        for entry in listing.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let path = entry.path();
            if path.is_dir() {
                let relative = self.path.join(&name);
                folders.push((format!("{name}/"), Action::EnterDir(relative)));
            } else if let Some(action) = file_action(kind, &path) {
                files.push((name, action));
            }
        }
        folders.sort_by_key(|(name, _)| name.to_lowercase());
        files.sort_by_key(|(name, _)| name.to_lowercase());
        entries.extend(folders);
        entries.extend(files);
        Ok(())
    }

    /// `music::entries` already puts folders first and skips what cannot play.
    fn list_music(&self, here: &Path, entries: &mut Vec<(String, Action)>) {
        for entry in music::entries(here) {
            if entry.folder {
                let relative = entry
                    .path
                    .strip_prefix(&self.root)
                    .unwrap_or(&entry.path)
                    .to_path_buf();
                entries.push((format!("{}/", entry.name), Action::EnterDir(relative)));
            } else {
                entries.push((entry.name, Action::PlayTrack(entry.path)));
            }
        }
    }

    /// Move the highlight inside the filtered list, clamped at both ends.
    pub(super) fn step(&mut self, delta: isize) {
        let len = self.filtered.len();
        if len == 0 {
            return;
        }
        let next = self.selected as isize + delta;
        self.selected = next.clamp(0, len as isize - 1) as usize;
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

/// Whether a row is a folder (including the ".." row).
pub(super) fn is_folder(action: &Action) -> bool {
    matches!(action, Action::EnterDir(_) | Action::Up)
}

/// Which program opens this file in this room, if any. Folders are handled by
/// the browser; a room only lists what its program can do something with.
fn file_action(kind: BrowserKind, path: &Path) -> Option<Action> {
    let extension = path.extension()?.to_string_lossy().to_lowercase();
    let document = matches!(extension.as_str(), "md" | "txt" | "markdown");
    match (kind, extension.as_str()) {
        (BrowserKind::Documents, _) if document => Some(Action::Open(path.to_path_buf())),
        (BrowserKind::Pictures, "png") => Some(Action::OpenPicture(path.to_path_buf())),
        (BrowserKind::Vault, _) if document => Some(Action::Open(path.to_path_buf())),
        (BrowserKind::Vault, "png") => Some(Action::OpenPicture(path.to_path_buf())),
        (BrowserKind::Vault, "gb" | "gbc") => Some(Action::OpenRom(path.to_path_buf())),
        _ => None,
    }
}

impl App {
    fn show_browser(&mut self, browser: Browser) {
        self.prepare_vault();
        self.browser = browser;
        self.refresh_browser();
        self.mode = Mode::Browser;
        self.dirty = true;
    }

    /// THE VAULT: the whole machine, one shelf.
    pub(super) fn open_browser(&mut self) {
        self.show_browser(Browser::vault(self.vault()));
    }

    /// `open` in Inspiro starts in `Documents/`, the editor's own room.
    pub(super) fn open_documents(&mut self) {
        self.show_browser(Browser::documents(&self.vault()));
    }

    /// `open` in Softhand starts in `Pictures/`.
    pub(super) fn open_pictures(&mut self) {
        self.show_browser(Browser::pictures(&self.vault()));
    }

    /// Relist the folder the browser is looking at.
    pub(super) fn refresh_browser(&mut self) {
        if let Err(e) = self.browser.refresh() {
            self.set_status(e);
        }
    }

    /// Open a folder in the browser; the filter starts fresh.
    pub(super) fn browser_enter(&mut self, path: PathBuf) {
        self.browser.path = path;
        self.browser.input.clear();
        self.browser.selected = 0;
        self.refresh_browser();
        self.poke();
    }

    /// One folder up. False at the root, where Esc leaves instead.
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

    /// Move the highlight inside the list, clamped at both ends.
    pub(super) fn browser_move(&mut self, delta: isize) {
        self.browser.step(delta);
        self.poke();
    }

    pub(super) fn browser_accept(&mut self) {
        let Some(action) = self.browser.current().cloned() else {
            return;
        };
        match action {
            Action::Open(path) => self.start_loading(Pending::Document(path)),
            Action::OpenPicture(path) => self.start_loading(Pending::Picture(path)),
            Action::OpenRom(path) => self.start_loading(Pending::Rom(path)),
            Action::PlayTrack(path) => {
                let dir = path.parent().unwrap_or(&path).to_path_buf();
                let index = music::tracks_in(&dir)
                    .iter()
                    .position(|track| track.path == path)
                    .unwrap_or(0);
                match self.riff_load(&dir, index) {
                    Ok(message) => {
                        self.close_explorer_into_player();
                        self.set_status(message);
                    }
                    Err(e) => self.set_status(e),
                }
                return;
            }
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

        match &event.logical_key {
            Key::Named(NamedKey::Escape) => {
                if !self.browser_up() {
                    self.to_write();
                }
            }
            Key::Named(NamedKey::Enter) => self.browser_accept(),
            Key::Named(NamedKey::ArrowLeft) => {
                self.browser_up();
            }
            Key::Named(NamedKey::ArrowUp) => self.browser_move(-1),
            Key::Named(NamedKey::ArrowDown) => self.browser_move(1),
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
