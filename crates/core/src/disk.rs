//! Diskettes: the single-file container a module or a document bundle arrives in.
//!
//! The format is deliberately plain text, so a diskette can be read with `type`,
//! mailed around, and diffed. The header is `key=value` lines; the payload is a
//! sequence of files introduced by `---FILE <name>`.

use std::fmt;

/// First line of every diskette.
pub const MAGIC: &str = "CRead Disk 1";

/// Highest container version this build understands.
pub const SUPPORTED_API: u32 = 1;

/// One file inside a diskette.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiskFile {
    pub name: String,
    pub text: String,
}

/// A parsed diskette.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Disk {
    pub name: String,
    pub about: String,
    pub author: String,
    /// Container version the diskette was written for.
    pub api: u32,
    /// Program this diskette mounts, if it is a program disk rather than a
    /// data disk. The code ships with CRead; the diskette is what makes the
    /// drive appear on the boot screen.
    pub program: String,
    pub files: Vec<DiskFile>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiskError {
    NotADisk,
    BadHeader(String),
    TooNew(u32),
    Empty,
}

impl fmt::Display for DiskError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DiskError::NotADisk => write!(f, "not a CRead disk"),
            DiskError::BadHeader(line) => write!(f, "bad header line: {line}"),
            DiskError::TooNew(api) => write!(
                f,
                "needs CRead OS api {api}, this build has {SUPPORTED_API}"
            ),
            DiskError::Empty => write!(f, "no files on the disk"),
        }
    }
}

impl Disk {
    /// Parse a `.dsk` file.
    pub fn parse(text: &str) -> Result<Self, DiskError> {
        let mut lines = text.lines();
        match lines.next().map(str::trim) {
            Some(MAGIC) => {}
            _ => return Err(DiskError::NotADisk),
        }

        let mut name = String::new();
        let mut about = String::new();
        let mut author = String::new();
        let mut api = 1u32;
        let mut program = String::new();
        let mut files: Vec<DiskFile> = Vec::new();
        let mut current: Option<DiskFile> = None;
        let mut in_body = false;

        for line in lines {
            if let Some(rest) = line.strip_prefix("---FILE ") {
                if let Some(file) = current.take() {
                    files.push(file);
                }
                in_body = true;
                current = Some(DiskFile {
                    name: rest.trim().to_string(),
                    text: String::new(),
                });
                continue;
            }

            if in_body {
                if let Some(file) = current.as_mut() {
                    if !file.text.is_empty() {
                        file.text.push('\n');
                    }
                    file.text.push_str(line);
                }
                continue;
            }

            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let Some((key, value)) = trimmed.split_once('=') else {
                return Err(DiskError::BadHeader(trimmed.to_string()));
            };
            let value = value.trim();
            match key.trim().to_ascii_lowercase().as_str() {
                "name" => name = value.to_string(),
                "about" => about = value.to_string(),
                "author" => author = value.to_string(),
                "program" => program = value.to_ascii_lowercase(),
                "api" => {
                    api = value
                        .parse()
                        .map_err(|_| DiskError::BadHeader(trimmed.to_string()))?;
                }
                // Unknown keys are ignored on purpose: a newer diskette may
                // carry more metadata, and that must not make it unreadable.
                _ => {}
            }
        }

        if let Some(file) = current {
            files.push(file);
        }
        if api > SUPPORTED_API {
            return Err(DiskError::TooNew(api));
        }
        // A program disk carries no documents of its own: the program lives in
        // the binary, the diskette is the mount.
        if files.is_empty() && program.is_empty() {
            return Err(DiskError::Empty);
        }
        if name.is_empty() {
            return Err(DiskError::BadHeader("name is required".into()));
        }

        Ok(Self {
            name,
            about,
            author,
            api,
            program,
            files,
        })
    }

    /// A data diskette: its files are the content.
    pub fn is_data_disk(&self) -> bool {
        self.program.is_empty()
    }
}

/// Build a diskette. Used by the tests and by `persist`-style tooling later.
pub fn format(name: &str, about: &str, author: &str, files: &[(String, String)]) -> String {
    let mut out =
        format!("{MAGIC}\nname={name}\napi={SUPPORTED_API}\nabout={about}\nauthor={author}\n");
    for (file, text) in files {
        out.push_str(&format!("\n---FILE {file}\n{text}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
CRead Disk 1
name=MAIL
api=1
about=Postfach und Nachrichten
author=andi

---FILE README.TXT
Willkommen in der Post.

Zweiter Absatz.
---FILE INBOX/1.TXT
Hallo.
";

    #[test]
    fn parses_header_and_files() {
        let disk = Disk::parse(SAMPLE).unwrap();
        assert_eq!(disk.name, "MAIL");
        assert_eq!(disk.about, "Postfach und Nachrichten");
        assert_eq!(disk.author, "andi");
        assert_eq!(disk.api, 1);
        assert_eq!(disk.files.len(), 2);
        assert_eq!(disk.files[0].name, "README.TXT");
        assert_eq!(
            disk.files[0].text,
            "Willkommen in der Post.\n\nZweiter Absatz."
        );
        assert_eq!(disk.files[1].name, "INBOX/1.TXT");
    }

    #[test]
    fn round_trips_through_format() {
        let files = vec![
            ("A.TXT".to_string(), "eins".to_string()),
            ("B.TXT".to_string(), "zwei\n drei".to_string()),
        ];
        let text = format("TEST", "about", "me", &files);
        let disk = Disk::parse(&text).unwrap();
        assert_eq!(disk.name, "TEST");
        assert_eq!(disk.files.len(), 2);
        assert_eq!(disk.files[1].text, "zwei\n drei");
    }

    #[test]
    fn rejects_foreign_files_and_bad_headers() {
        assert_eq!(Disk::parse("hello"), Err(DiskError::NotADisk));
        assert_eq!(Disk::parse(""), Err(DiskError::NotADisk));
        assert!(matches!(
            Disk::parse("CRead Disk 1\nnonsense\n---FILE A\nx"),
            Err(DiskError::BadHeader(_))
        ));
    }

    #[test]
    fn rejects_disks_for_a_newer_os() {
        let text = "CRead Disk 1\nname=X\napi=9\n\n---FILE A\nx";
        assert_eq!(Disk::parse(text), Err(DiskError::TooNew(9)));
    }

    #[test]
    fn rejects_empty_disks() {
        assert_eq!(Disk::parse("CRead Disk 1\nname=X\n"), Err(DiskError::Empty));
    }

    #[test]
    fn a_program_disk_mounts_without_carrying_files() {
        let text = "CRead Disk 1\nname=SOFTHAND\nprogram=softhand\nabout=paint";
        let disk = Disk::parse(text).unwrap();
        assert_eq!(disk.program, "softhand");
        assert!(disk.files.is_empty());
        assert!(!disk.is_data_disk());
    }

    #[test]
    fn a_program_disk_mounts_without_files() {
        let text = "CRead Disk 1\nname=MAILBOX\nprogram=mailbox\nabout=mail";
        let disk = Disk::parse(text).unwrap();
        assert_eq!(disk.program, "mailbox");
        assert!(disk.files.is_empty());
    }

    #[test]
    fn unknown_keys_are_ignored() {
        let text = "CRead Disk 1\nname=X\nfuture=whatever\n\n---FILE A\nx";
        assert_eq!(Disk::parse(text).unwrap().name, "X");
    }

    #[test]
    fn file_text_keeps_blank_lines_and_indentation() {
        let text = "CRead Disk 1\nname=X\n\n---FILE A\n  indented\n\n\nend";
        assert_eq!(
            Disk::parse(text).unwrap().files[0].text,
            "  indented\n\n\nend"
        );
    }
}
