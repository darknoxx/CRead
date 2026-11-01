//! File and path handling. Small on purpose: atomic writes, no surprises.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Read a document. Invalid UTF-8 is replaced rather than rejected so a stray
/// binary byte never costs you the file.
pub fn load(path: &Path) -> io::Result<String> {
    let bytes = fs::read(path)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Write a file without ever leaving a half-written document behind.
///
/// Writes to a sibling temp file, flushes it to disk, then renames over the
/// target. On any POSIX filesystem the rename is atomic, so a crash leaves
/// either the old file or the new one.
pub fn save_atomic(path: &Path, text: &str) -> io::Result<()> {
    save_bytes_atomic(path, text.as_bytes())
}

/// The same, for anything that is not text: pictures, cartridges, what else
/// ends up in the vault.
pub fn save_bytes_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;

    let tmp = parent.join(format!(
        ".{}.tmp-{}",
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "cread".into()),
        std::process::id()
    ));

    {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }

    match fs::rename(&tmp, path) {
        Ok(()) => {}
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            return Err(e);
        }
    }

    // fsync the directory so the rename itself is durable.
    #[cfg(unix)]
    if let Ok(dir) = fs::File::open(parent) {
        let _ = dir.sync_all();
    }

    Ok(())
}

/// Where new documents live by default.
///
/// Follows the XDG user directory, so a German desktop gets `~/Dokumente/CRead`
/// instead of quietly creating an English `~/Documents` next to it.
pub fn vault_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("CREAD_VAULT") {
        return PathBuf::from(dir);
    }
    documents_dir().join("CRead")
}

/// The user's document folder, from `~/.config/user-dirs.dirs` when present.
pub fn documents_dir() -> PathBuf {
    let xdg = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".config"))
        .join("user-dirs.dirs");
    if let Ok(text) = std::fs::read_to_string(xdg)
        && let Some(dir) = documents_from_user_dirs(&text, &home())
    {
        return dir;
    }
    home().join("Documents")
}

/// Pull `XDG_DOCUMENTS_DIR` out of a `user-dirs.dirs` file.
fn documents_from_user_dirs(text: &str, home: &Path) -> Option<PathBuf> {
    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("XDG_DOCUMENTS_DIR=") else {
            continue;
        };
        let value = rest.trim().trim_matches('"');
        let path = match value {
            "$HOME" => home.to_path_buf(),
            _ if value.starts_with("$HOME/") => home.join(&value[6..]),
            _ if value.is_empty() => return None,
            other => PathBuf::from(other),
        };
        return path.is_dir().then_some(path);
    }
    None
}

/// Config: WebDAV credentials and user preferences.
pub fn config_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(dir).join("cread");
    }
    home().join(".config").join("cread")
}

pub fn home_dir() -> PathBuf {
    home()
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn free_untitled(dir: &Path) -> PathBuf {
    let mut path = dir.join("untitled.md");
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("untitled-{n}.md"));
        n += 1;
    }
    path
}

/// Documents live in `<vault>/Documents`. The vault root is only the shelf:
/// pictures, cartridges and documents each get their folder, so a name can
/// never mean two things.
pub fn documents_in(vault: &Path) -> PathBuf {
    vault.join("Documents")
}

/// Pictures live in `<vault>/Pictures`.
pub fn pictures_in(vault: &Path) -> PathBuf {
    vault.join("Pictures")
}

/// Cartridges live in `<vault>/Roms`.
pub fn roms_in(vault: &Path) -> PathBuf {
    vault.join("Roms")
}

/// Every file under `root`, recursively, sorted by path. Dotfiles and hidden
/// folders are skipped: they are the app's own, never the writer's.
pub fn walk_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.is_file() {
                out.push(path);
            }
        }
    }
    walk(root, &mut out);
    out.sort();
    out
}

/// What a round of tidying found in the vault root.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Migration {
    /// Documents moved into `Documents/`.
    pub moved: usize,
    /// Root copies removed because `Documents/` held the very same bytes.
    pub duplicates: usize,
    /// Files left alone because a different document already has that name.
    pub kept: Vec<String>,
}

/// Move loose documents from the vault root into `Documents/`.
///
/// Runs once at startup, so a vault from before the layout change still opens
/// where the writer expects. Nothing is overwritten: a name that is already
/// taken stays in the root and is reported.
pub fn migrate_documents(vault: &Path) -> Migration {
    let mut out = Migration::default();
    let target_dir = documents_in(vault);
    let Ok(files) = list_documents(vault) else {
        return out;
    };
    if files.is_empty() {
        return out;
    }
    if let Err(e) = fs::create_dir_all(&target_dir) {
        out.kept = files
            .iter()
            .map(|p| {
                p.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        let _ = e;
        return out;
    }
    for source in files {
        let Some(name) = source.file_name().map(|n| n.to_string_lossy().into_owned()) else {
            continue;
        };
        let target = target_dir.join(&name);
        if target.exists() {
            let same = match (fs::read(&source), fs::read(&target)) {
                (Ok(a), Ok(b)) => a == b,
                _ => false,
            };
            if same && fs::remove_file(&source).is_ok() {
                out.duplicates += 1;
            } else {
                out.kept.push(name);
            }
            continue;
        }
        match fs::rename(&source, &target) {
            Ok(()) => out.moved += 1,
            Err(_) => out.kept.push(name),
        }
    }
    out
}

/// Is this a file the editor opens: Markdown, or plain text?
pub fn is_document(path: &Path) -> bool {
    path.extension().is_some_and(|ext| {
        let ext = ext.to_string_lossy().to_lowercase();
        ext == "md" || ext == "txt" || ext == "markdown"
    })
}

/// Documents in a directory, sorted by name, ignoring dotfiles.
pub fn list_documents(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut out: Vec<PathBuf> = fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .filter(|p| {
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            !name.starts_with('.') && is_document(p)
        })
        .collect();
    out.sort();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cread-test-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn loose_documents_move_into_documents_once() {
        let vault = tmpdir("migrate");
        fs::create_dir_all(documents_in(&vault).join("sub")).unwrap();
        fs::write(vault.join("a.md"), "eins").unwrap();
        fs::write(vault.join("note.txt"), "zwei").unwrap();
        fs::write(vault.join("same.md"), "gleich").unwrap();
        fs::write(documents_in(&vault).join("same.md"), "gleich").unwrap();
        fs::write(documents_in(&vault).join("clash.md"), "meins").unwrap();
        fs::write(vault.join("clash.md"), "anders").unwrap();
        fs::write(vault.join("keep.gb"), "rom").unwrap();

        let report = migrate_documents(&vault);
        assert_eq!(report.moved, 2, "a.md and note.txt");
        assert_eq!(report.duplicates, 1, "same.md was already in place");
        assert_eq!(report.kept, vec!["clash.md"], "never overwritten");
        assert!(documents_in(&vault).join("a.md").is_file());
        assert!(documents_in(&vault).join("note.txt").is_file());
        assert!(!vault.join("a.md").exists());
        assert!(vault.join("clash.md").is_file(), "the clash stays put");
        assert!(
            vault.join("keep.gb").is_file(),
            "a cartridge is not a document"
        );

        let again = migrate_documents(&vault);
        assert_eq!(again.moved, 0, "migration is idempotent");

        fs::remove_dir_all(&vault).unwrap();
    }

    #[test]
    fn walking_finds_nested_files_and_skips_hidden_ones() {
        let vault = tmpdir("walk");
        fs::create_dir_all(vault.join("Documents/sub")).unwrap();
        fs::create_dir_all(vault.join(".cread-sync")).unwrap();
        fs::write(vault.join("Documents/a.md"), "").unwrap();
        fs::write(vault.join("Documents/sub/b.md"), "").unwrap();
        fs::write(vault.join(".cread-sync/index.json"), "").unwrap();
        fs::write(vault.join(".hidden.md"), "").unwrap();

        let names: Vec<String> = walk_files(&vault)
            .into_iter()
            .map(|p| p.strip_prefix(&vault).unwrap().display().to_string())
            .collect();
        assert_eq!(names, vec!["Documents/a.md", "Documents/sub/b.md"]);

        fs::remove_dir_all(&vault).unwrap();
    }

    #[test]
    fn round_trips_and_leaves_no_temp_files() {
        let dir = tmpdir("save");
        let path = dir.join("note.md");
        save_atomic(&path, "hallo\nwelt\n").unwrap();
        assert_eq!(load(&path).unwrap(), "hallo\nwelt\n");

        let entries: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(entries.len(), 1, "temp file was left behind: {entries:?}");

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn overwriting_keeps_the_new_content() {
        let dir = tmpdir("overwrite");
        let path = dir.join("a.md");
        save_atomic(&path, "first version is long").unwrap();
        save_atomic(&path, "second").unwrap();
        assert_eq!(load(&path).unwrap(), "second");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn creates_parent_directories() {
        let dir = tmpdir("nested");
        let path = dir.join("deep/deeper/x.md");
        save_atomic(&path, "x").unwrap();
        assert!(path.exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn lists_only_documents_and_sorts_them() {
        let dir = tmpdir("list");
        for name in ["b.md", "a.txt", ".hidden.md", "c.png", "d.markdown"] {
            fs::write(dir.join(name), "x").unwrap();
        }
        let names: Vec<String> = list_documents(&dir)
            .unwrap()
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["a.txt", "b.md", "d.markdown"]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn user_dirs_file_gives_the_localised_document_folder() {
        let home = Path::new("/home/noxx");
        let german =
            "XDG_DESKTOP_DIR=\"$HOME/Schreibtisch\"\nXDG_DOCUMENTS_DIR=\"$HOME/Dokumente\"\n";
        assert_eq!(
            documents_from_user_dirs(german, home),
            None,
            "a folder that does not exist is ignored"
        );

        // Point at something that certainly exists to exercise the expansion.
        let existing = std::env::temp_dir();
        let line = format!("XDG_DOCUMENTS_DIR=\"{}\"\n", existing.display());
        assert_eq!(documents_from_user_dirs(&line, home), Some(existing));

        assert_eq!(documents_from_user_dirs("", home), None);
        assert_eq!(
            documents_from_user_dirs("XDG_DOCUMENTS_DIR=\"\"", home),
            None
        );
        assert_eq!(
            documents_from_user_dirs("XDG_DOCUMENTS_DIR=\"$HOME\"", home),
            None,
            "$HOME resolves to a missing directory here"
        );
    }

    #[test]
    fn untitled_names_avoid_existing_files() {
        let dir = tmpdir("untitled");
        assert_eq!(free_untitled(&dir).file_name().unwrap(), "untitled.md");

        fs::write(dir.join("untitled.md"), "x").unwrap();
        assert_eq!(free_untitled(&dir).file_name().unwrap(), "untitled-2.md");

        fs::write(dir.join("untitled-2.md"), "x").unwrap();
        assert_eq!(free_untitled(&dir).file_name().unwrap(), "untitled-3.md");

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn invalid_utf8_does_not_fail_the_load() {
        let dir = tmpdir("utf8");
        let path = dir.join("bin.md");
        fs::write(&path, [0x68, 0x69, 0xff, 0x0a]).unwrap();
        assert_eq!(load(&path).unwrap(), "hi\u{fffd}\n");
        fs::remove_dir_all(&dir).unwrap();
    }
}
