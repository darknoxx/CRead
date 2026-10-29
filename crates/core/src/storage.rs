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
        file.write_all(text.as_bytes())?;
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
pub fn vault_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("CREAD_VAULT") {
        return PathBuf::from(dir);
    }
    home().join("Documents").join("CRead")
}

/// Config: WebDAV credentials and user preferences.
pub fn config_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(dir).join("cread");
    }
    home().join(".config").join("cread")
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Markdown documents in a directory, sorted by name, ignoring dotfiles.
pub fn list_documents(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut out: Vec<PathBuf> = fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .filter(|p| {
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            !name.starts_with('.')
                && (name.ends_with(".md") || name.ends_with(".txt") || name.ends_with(".markdown"))
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
    fn round_trips_and_leaves_no_temp_files() {
        let dir = tmpdir("save");
        let path = dir.join("note.md");
        save_atomic(&path, "hallo\nwelt\n").unwrap();
        assert_eq!(load(&path).unwrap(), "hallo\nwelt\n");

        let entries: Vec<_> = fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
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
    fn invalid_utf8_does_not_fail_the_load() {
        let dir = tmpdir("utf8");
        let path = dir.join("bin.md");
        fs::write(&path, [0x68, 0x69, 0xff, 0x0a]).unwrap();
        assert_eq!(load(&path).unwrap(), "hi\u{fffd}\n");
        fs::remove_dir_all(&dir).unwrap();
    }
}
