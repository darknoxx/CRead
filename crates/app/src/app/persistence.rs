//! Documents on disk: open, save, save as, rename, delete, drafts, autosave.
//!
//! Saving is the writer's decision; autosave is opt-in and the only write that
//! fires on its own.

use super::*;

impl App {
    /// Returns the buffer and whether it counts as an open document.
    ///
    /// A file argument or `--demo` opens straight into write mode; otherwise we
    /// start on the greeting screen. Silently resuming the last file makes it
    /// too easy to type into the wrong document.
    pub(super) fn initial_document(opts: &Options) -> (Document, bool) {
        if let Some(path) = &opts.file {
            match storage::load(path) {
                Ok(text) => {
                    return (
                        Document {
                            path: Some(path.clone()),
                            title_hint: None,
                            editor: Editor::from_text(&text),
                            last_edit: Instant::now(),
                        },
                        true,
                    );
                }
                Err(e) => eprintln!("cread: cannot open {}: {e}", path.display()),
            }
        }

        if opts.demo {
            return (
                Document {
                    path: None,
                    title_hint: Some("welcome.md".into()),
                    editor: Editor::from_text(WELCOME),
                    last_edit: Instant::now(),
                },
                true,
            );
        }

        (
            Document {
                path: None,
                title_hint: None,
                editor: Editor::new(),
                last_edit: Instant::now(),
            },
            false,
        )
    }

    pub(super) fn autosave_if_due(&mut self) {
        if !self.autosave {
            return;
        }
        if self.mode != Mode::Write {
            return;
        }
        if !self.doc.editor.modified() {
            return;
        }
        if self.doc.last_edit.elapsed() < AUTOSAVE_DELAY {
            return;
        }
        let Some(path) = self.doc.path.clone() else {
            return;
        };
        if let Err(e) = self.save_to(&path) {
            self.set_status(e);
        }
    }

    pub(super) fn save_to(&mut self, path: &Path) -> Result<(), String> {
        let text = self.doc.editor.text();
        storage::save_atomic(path, &text).map_err(|e| format!("save failed: {e}"))?;
        self.unsynced = true;
        self.doc.editor.mark_saved();
        self.doc.path = Some(path.to_path_buf());
        if self.sync_cfg.enabled {
            self.request_sync();
        }
        self.dirty = true;
        Ok(())
    }

    pub(super) fn save(&mut self) -> Result<String, String> {
        if !self.has_document {
            return Err("no document - use new or open".into());
        }
        let path = match self.doc.path.clone() {
            Some(path) => path,
            None => self.free_untitled_path(),
        };
        self.save_to(&path)?;
        // `save` on the placeholder is the writer keeping that name: it is a
        // document now, not a draft the editor made up.
        if self.scratch.as_deref() == Some(path.as_path()) {
            self.scratch = None;
            self.forget_draft();
        }
        Ok(String::new())
    }

    /// Start a fresh file in the vault, named `untitled.md`, `untitled-2.md`, …
    pub(super) fn new_document(&mut self) -> Result<String, String> {
        if self.has_document && self.doc.editor.modified() {
            return Err("unsaved changes - save first".into());
        }
        self.prepare_vault();
        self.discard_scratch();
        let path = self.free_untitled_path();
        self.doc = Document {
            path: None,
            title_hint: None,
            editor: Editor::new(),
            last_edit: Instant::now(),
        };
        self.has_document = true;
        self.scratch = Some(path.clone());
        self.remember_draft();
        self.save_to(&path)?;
        Ok(format!("created {}", self.doc.title()))
    }

    /// Delete the file `new` reserved if the document is being left empty.
    ///
    /// Both the buffer and the file on disk have to be blank: a scratch file
    /// that autosave filled is somebody's text, and an in-memory document with
    /// unsaved characters must keep its place on disk.
    pub(super) fn discard_scratch(&mut self) -> Option<String> {
        let path = self.scratch.take()?;
        // Whatever happens next, this file is no longer "the draft".
        self.forget_draft();
        let same = self.doc.path.as_deref() == Some(path.as_path());
        let unsaved = same && !self.doc.editor.text().trim().is_empty();
        let blank_on_disk = std::fs::read_to_string(&path)
            .map(|text| text.trim().is_empty())
            .unwrap_or(false);
        if unsaved || !blank_on_disk {
            return None;
        }
        std::fs::remove_file(&path).ok()?;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.set_status(format!("removed empty {name}"));
        self.dirty = true;
        Some(name)
    }

    pub(super) fn save_as(&mut self, name: &str) -> Result<String, String> {
        if !self.has_document {
            return Err("no document - use new or open".into());
        }
        self.prepare_vault();
        let path = self.document_path(name)?;
        if path.exists() && self.doc.path.as_deref() != Some(path.as_path()) {
            return Err(format!("{} already exists", self.doc_title_of(&path)));
        }
        self.save_to(&path)?;
        match self.drop_scratch(&path) {
            Some(removed) => Ok(format!("saved as {} ({removed} removed)", self.doc.title())),
            None => Ok(format!("saved as {}", self.doc.title())),
        }
    }

    /// The untitled placeholder has done its job once the text has a name.
    ///
    /// Autosave may have filled it, but `saveas` just wrote the same buffer to
    /// the chosen path, so keeping the placeholder would only leave a second
    /// copy of the document in the vault - and on the server.
    pub(super) fn drop_scratch(&mut self, kept: &Path) -> Option<String> {
        self.forget_draft();
        let scratch = self.scratch.take()?;
        if scratch == kept {
            // Saved under its own placeholder name: it is a named file now.
            return None;
        }
        std::fs::remove_file(&scratch).ok()?;
        self.unsynced = true;
        self.dirty = true;
        Some(scratch.file_name()?.to_string_lossy().into_owned())
    }

    pub(super) fn vault(&self) -> PathBuf {
        self.opts.vault.clone().unwrap_or_else(storage::vault_dir)
    }

    /// Tell the sync engine which file is the editor's unnamed draft, so the
    /// server copy can follow it when the draft gets a real name.
    pub(super) fn remember_draft(&self) {
        let vault = self.vault();
        let name = self
            .scratch
            .as_ref()
            .and_then(|path| path.strip_prefix(&vault).ok())
            .map(|path| path.to_string_lossy().into_owned());
        let _ = cread_sync::set_draft(&vault, name.as_deref());
    }

    pub(super) fn forget_draft(&self) {
        let _ = cread_sync::set_draft(&self.vault(), None);
    }

    /// Check the vault layout once: a document belongs in `Documents/`, so
    /// anything still sitting in the root is moved there before we list it.
    pub(super) fn prepare_vault(&mut self) {
        if self.vault_ready {
            return;
        }
        self.vault_ready = true;
        let vault = self.vault();
        // The shelf has three rooms; an empty one still belongs to the layout,
        // so the browser and the server both show it.
        for dir in [
            storage::documents_in(&vault),
            storage::pictures_in(&vault),
            storage::roms_in(&vault),
            storage::calendar_in(&vault),
            storage::notes_in(&vault),
        ] {
            let _ = std::fs::create_dir_all(dir);
        }
        // A draft the app marked before a restart has outlived its session:
        // it belongs to the writer now, so it stops being a draft.
        let _ = cread_sync::promote_draft(&vault);
        let report = storage::migrate_documents(&vault);
        if report.moved > 0 || report.duplicates > 0 {
            self.unsynced = true;
            let mut message = format!("moved {} into Documents/", report.moved);
            if report.duplicates > 0 {
                message = format!("{message} ({} already there)", report.duplicates);
            }
            self.set_status(message);
        }
        if !report.kept.is_empty() {
            self.set_status(format!(
                "{} kept in the vault root: name already taken",
                report.kept.join(", ")
            ));
        }
    }

    /// Where documents live: `<vault>/Documents`, never the root.
    pub(super) fn documents(&self) -> PathBuf {
        storage::documents_in(&self.vault())
    }

    /// First free `untitled[-n].md` in Documents.
    pub(super) fn free_untitled_path(&self) -> PathBuf {
        storage::free_untitled(&self.documents())
    }

    /// Resolve a user-supplied name inside Documents, adding `.md` when needed.
    ///
    /// Subfolders are fine (`saveas kapitel/eins`), `..` and absolute paths are
    /// not: a document has exactly one home. A name that folds away to nothing
    /// (`.`, `..`, an empty string) is refused instead of becoming
    /// `<vault>/Documents.md`, a file next to the folder.
    pub(super) fn document_path(&self, name: &str) -> Result<PathBuf, String> {
        let mut relative = PathBuf::new();
        for part in Path::new(name).components() {
            if let std::path::Component::Normal(part) = part {
                relative.push(part);
            }
        }
        if relative.as_os_str().is_empty() {
            return Err("a document needs a name".into());
        }
        let path = self.documents().join(relative);
        Ok(match path.extension() {
            Some(ext) if !ext.is_empty() => path,
            _ => path.with_extension("md"),
        })
    }

    /// The path `old` gets when renamed to `name`: same folder, same extension
    /// when none was typed. One rule for the command and the browser.
    pub(super) fn rename_target(&self, old: &Path, name: &str) -> Result<PathBuf, String> {
        let name = Path::new(name)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if name.is_empty() {
            return Err("a name is needed".into());
        }
        let parent = old
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.vault());
        let mut target = parent.join(&name);
        if target.extension().is_none() {
            target.set_extension(old.extension().unwrap_or_else(|| "md".as_ref()));
        }
        Ok(target)
    }

    /// Find an existing document by name, with or without the extension.
    ///
    /// A bare name is looked up anywhere under Documents, so
    /// `open kapitel` finds `Documents/kapitel/eins.md`. The vault root is the
    /// last resort, for a file the migration had to leave behind.
    pub(super) fn find_document(&self, name: &str) -> Option<PathBuf> {
        if let Ok(named) = self.document_path(name)
            && named.is_file()
        {
            return Some(named);
        }
        let wanted = Path::new(name)
            .file_name()?
            .to_string_lossy()
            .to_lowercase();
        let with_ext = if wanted.ends_with(".md") {
            wanted.clone()
        } else {
            format!("{wanted}.md")
        };
        let mut matches: Vec<PathBuf> = storage::walk_files(&self.documents())
            .into_iter()
            .filter(|path| storage::is_document(path))
            .filter(|path| {
                path.file_name().is_some_and(|n| {
                    let n = n.to_string_lossy().to_lowercase();
                    n == wanted || n == with_ext
                })
            })
            .collect();
        matches.sort();
        if let Some(path) = matches.into_iter().next() {
            return Some(path);
        }
        // The vault root is the last resort, for a file the migration had to
        // leave behind - and only by its bare name, never by a path.
        let leaf = Path::new(name).file_name()?;
        let legacy = self.vault().join(leaf);
        if legacy.is_file() {
            return Some(legacy);
        }
        let legacy_md = legacy.with_extension("md");
        legacy_md.is_file().then_some(legacy_md)
    }

    pub(super) fn open_document(&mut self, path: PathBuf) -> Result<(), String> {
        if self.has_document && self.doc.editor.modified() {
            return Err("unsaved changes - save first".into());
        }
        self.discard_scratch();
        let text =
            storage::load(&path).map_err(|e| format!("cannot open {}: {e}", path.display()))?;
        self.doc = Document {
            path: Some(path.clone()),
            title_hint: None,
            editor: Editor::from_text(&text),
            last_edit: Instant::now(),
        };
        self.has_document = true;
        self.dirty = true;
        Ok(())
    }

    pub(super) fn rename_current(&mut self, name: &str) -> Result<String, String> {
        match self.doc.path.clone().filter(|p| p.is_file()) {
            Some(old) => {
                let target = self.rename_target(&old, name)?;
                if target != old && target.exists() {
                    return Err(format!("{} already exists", self.doc_title_of(&target)));
                }
                std::fs::rename(&old, &target).map_err(|e| format!("rename failed: {e}"))?;
                self.unsynced = true;
                // A name of its own means it is no longer a placeholder.
                self.scratch = None;
                self.forget_draft();
                self.doc.path = Some(target.clone());
                self.dirty = true;
                Ok(format!("renamed to {}", self.doc.title()))
            }
            // Not on disk yet: fall back to a plain save under the new name.
            None => self.save_as(name),
        }
    }

    pub(super) fn delete_current(&mut self) -> Result<String, String> {
        let Some(path) = self.doc.path.clone().filter(|p| p.is_file()) else {
            return Err("nothing to delete".into());
        };
        std::fs::remove_file(&path).map_err(|e| format!("delete failed: {e}"))?;
        self.unsynced = true;
        if self.scratch.as_deref() == Some(path.as_path()) {
            self.scratch = None;
        }
        self.doc = Document {
            path: None,
            title_hint: None,
            editor: Editor::new(),
            last_edit: Instant::now(),
        };
        self.has_document = false;
        self.dirty = true;
        Ok(format!(
            "deleted {}",
            path.file_name()
                .map(|n| n.to_string_lossy())
                .unwrap_or_default()
        ))
    }

    pub(super) fn doc_title_of(&self, path: &Path) -> String {
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    /// Persist the bits of UI state that should survive a restart.
    pub(super) fn save_prefs(&mut self) {
        self.prefs.set(self.theme);
        self.prefs.font = self.font.face().name().to_string();
        self.prefs.typewriter = self.typewriter;
        self.prefs.scanlines = self.crt.scanline > 0.0;
        self.prefs.bloom = self.crt.bloom > 0.0;
        self.prefs.roll = self.crt.roll > 0.0;
        self.prefs.screensaver = self.screensaver_on;
        self.prefs.screensaver_minutes = self.screensaver_minutes;
        self.prefs.clock_24h = self.clock_24h;
        self.prefs.date_day_first = self.date_day_first;
        if !self.persist {
            return;
        }
        if let Err(e) = self.prefs.save() {
            eprintln!("cread: cannot save preferences: {e}");
        }
    }

    /// Folder the current document lives in, home abbreviated.
    pub(super) fn location(&self) -> String {
        let dir = self
            .doc
            .path
            .as_ref()
            .and_then(|p| p.parent())
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| self.vault());
        abbreviate_home(&dir)
    }
}
