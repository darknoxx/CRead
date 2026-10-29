//! Sync engine: mirror a vault folder onto a WebDAV server, local-first.
//!
//! The rule is simple and deliberately boring: the local file is the source of
//! truth, and a genuine both-sides edit never overwrites anything silently. It
//! keeps the conflicting remote copy next to the local one instead.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use reqwest_dav::list_cmd::{ListEntity, ListFile};
use reqwest_dav::{Auth, Client, ClientBuilder, Depth};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use cread_core::storage;

pub mod worker;
pub use worker::{Event, Worker};

/// Connection settings, persisted as JSON.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Config {
    /// Base URL of the WebDAV endpoint, e.g. `https://cloud.example.org/remote.php/dav`.
    pub url: String,
    pub username: String,
    pub password: String,
    /// Folder inside the endpoint that mirrors the vault. Empty means the root.
    pub remote_dir: String,
    pub enabled: bool,
}

impl Config {
    pub fn path() -> PathBuf {
        storage::config_dir().join("sync.json")
    }

    pub fn load() -> Self {
        let Ok(text) = std::fs::read_to_string(Self::path()) else {
            return Self::default();
        };
        serde_json::from_str(&text).unwrap_or_default()
    }

    /// Persist with owner-only permissions; it holds a password.
    pub fn save(&self) -> std::io::Result<()> {
        let path = Self::path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text =
            serde_json::to_string_pretty(self).map_err(|e| std::io::Error::other(e.to_string()))?;

        #[cfg(unix)]
        {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&path)?;
            file.write_all(text.as_bytes())?;
        }
        #[cfg(not(unix))]
        {
            std::fs::write(&path, &text)?;
        }
        Ok(())
    }

    /// Password from the environment wins, for people who would rather not have
    /// it on disk at all.
    pub fn password(&self) -> String {
        std::env::var("CREAD_WEBDAV_PASSWORD").unwrap_or_else(|_| self.password.clone())
    }

    pub fn is_configured(&self) -> bool {
        !self.url.trim().is_empty()
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Entry {
    /// SHA-256 of the local content at the time of the last successful sync.
    hash: String,
    /// Server-side version token (ETag, or mtime when the server has none).
    version: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Index {
    files: BTreeMap<String, Entry>,
}

impl Index {
    fn load(vault: &Path) -> Self {
        std::fs::read_to_string(vault.join(".cread-sync/index.json"))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    fn save(&self, vault: &Path) -> std::io::Result<()> {
        let dir = vault.join(".cread-sync");
        std::fs::create_dir_all(&dir)?;
        let text =
            serde_json::to_string_pretty(self).map_err(|e| std::io::Error::other(e.to_string()))?;
        std::fs::write(dir.join("index.json"), text)
    }
}

/// What happened in one sync round.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub uploaded: usize,
    pub downloaded: usize,
    pub conflicts: usize,
    pub skipped: usize,
}

impl Report {
    pub fn summary(&self) -> String {
        if self.uploaded == 0 && self.downloaded == 0 && self.conflicts == 0 {
            return "up to date".into();
        }
        let mut parts = Vec::new();
        if self.uploaded > 0 {
            parts.push(format!("{} up", self.uploaded));
        }
        if self.downloaded > 0 {
            parts.push(format!("{} down", self.downloaded));
        }
        if self.conflicts > 0 {
            parts.push(format!("{} conflict", self.conflicts));
        }
        parts.join(", ")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plan {
    Nothing,
    Upload,
    Download,
    Conflict,
}

/// The whole conflict policy, in four lines.
pub fn plan(local_changed: bool, remote_changed: bool) -> Plan {
    match (local_changed, remote_changed) {
        (false, false) => Plan::Nothing,
        (true, false) => Plan::Upload,
        (false, true) => Plan::Download,
        // Both sides moved: keep the local text, park the remote one beside it.
        (true, true) => Plan::Conflict,
    }
}

fn client(cfg: &Config) -> Result<Client, String> {
    let auth = if cfg.username.is_empty() {
        Auth::Anonymous
    } else {
        Auth::Basic(cfg.username.clone(), cfg.password())
    };
    ClientBuilder::new()
        .set_host(cfg.url.clone())
        .set_auth(auth)
        .build()
        .map_err(|e| format!("cannot create WebDAV client: {e}"))
}

fn hash(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Percent-encode a path segment so umlauts and spaces survive the URL round trip.
pub fn encode_segment(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub fn decode_segment(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(byte) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            out.push(byte);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn encode_path(path: &str) -> String {
    path.split('/')
        .filter(|s| !s.is_empty())
        .map(encode_segment)
        .collect::<Vec<_>>()
        .join("/")
}

/// Name of the remote entry a listing href points at.
fn name_of_href(href: &str) -> String {
    let trimmed = href.trim_end_matches('/');
    let last = trimmed.rsplit('/').next().unwrap_or(trimmed);
    decode_segment(last)
}

/// Run one full sync round. Blocking; call from the worker thread.
pub async fn sync_once(vault: &Path, cfg: &Config) -> Result<Report, String> {
    if !cfg.is_configured() {
        return Err("WebDAV is not configured".into());
    }
    let client = client(cfg)?;
    let remote_dir = encode_path(&cfg.remote_dir);

    // Make sure the target collection exists; servers answer 404 otherwise.
    ensure_remote_dir(&client, &remote_dir).await;
    let remote = list_remote(&client, &remote_dir).await?;
    let local_files = storage::list_documents(vault).unwrap_or_default();
    let mut index = Index::load(vault);
    let mut report = Report::default();
    let mut touched_remote = false;

    for path in &local_files {
        let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else {
            continue;
        };
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        let local_hash = hash(&bytes);
        let entry = index.files.get(&name).cloned();
        let remote_version = remote.get(&name).cloned();

        let local_changed = entry.as_ref().map(|e| e.hash != local_hash).unwrap_or(true);
        let remote_changed = match (&entry, &remote_version) {
            (Some(e), Some(v)) => &e.version != v,
            (None, Some(_)) => true,
            // Gone from the server. Only count it as a change if we had synced it.
            (Some(_), None) => true,
            (None, None) => false,
        };

        let remote_path = join_remote(&remote_dir, &name);
        match plan(local_changed, remote_changed) {
            Plan::Nothing => report.skipped += 1,

            Plan::Upload => {
                client
                    .put(&remote_path, bytes.clone())
                    .await
                    .map_err(|e| format!("upload {name} failed: {e}"))?;
                touched_remote = true;
                report.uploaded += 1;
                set_hash(&mut index, &name, local_hash);
            }

            Plan::Download => {
                let body = download(&client, &remote_path, &name).await?;
                storage::save_atomic(path, &String::from_utf8_lossy(&body))
                    .map_err(|e| format!("writing {name} failed: {e}"))?;
                index.files.insert(
                    name.clone(),
                    Entry {
                        hash: hash(&body),
                        version: remote_version.unwrap_or_default(),
                    },
                );
                report.downloaded += 1;
            }

            Plan::Conflict => {
                let body = download(&client, &remote_path, &name).await?;
                if body == bytes {
                    // Same content on both sides: a first sync, not a conflict.
                    index.files.insert(
                        name.clone(),
                        Entry {
                            hash: local_hash,
                            version: remote_version.unwrap_or_default(),
                        },
                    );
                    report.skipped += 1;
                    continue;
                }

                let conflict = conflict_path(vault, &name);
                storage::save_atomic(&conflict, &String::from_utf8_lossy(&body))
                    .map_err(|e| format!("writing conflict copy failed: {e}"))?;

                // Local text keeps the original name; the other version lives
                // beside it for a human to merge.
                client
                    .put(&remote_path, bytes.clone())
                    .await
                    .map_err(|e| format!("upload {name} failed: {e}"))?;
                touched_remote = true;
                report.conflicts += 1;
                set_hash(&mut index, &name, local_hash);
            }
        }
    }

    // Files that only exist on the server, from another device or an earlier
    // session. Unknown names are pulled down; names we knew about are left alone
    // because a missing local file means the user deleted it here.
    for name in remote.keys() {
        if index.files.contains_key(name) || local_exists(&local_files, name) {
            continue;
        }
        let remote_path = join_remote(&remote_dir, name);
        let body = download(&client, &remote_path, name).await?;
        let target = vault.join(name);
        storage::save_atomic(&target, &String::from_utf8_lossy(&body))
            .map_err(|e| format!("writing {name} failed: {e}"))?;
        index.files.insert(
            name.clone(),
            Entry {
                hash: hash(&body),
                version: remote.get(name).cloned().unwrap_or_default(),
            },
        );
        report.downloaded += 1;
    }

    // One extra PROPFIND so the stored versions match the server, whether or not
    // we pushed anything. Cheap, and it keeps the next round quiet.
    if (touched_remote || report.downloaded > 0)
        && let Ok(fresh) = list_remote(&client, &remote_dir).await
    {
        for (name, version) in fresh {
            if let Some(entry) = index.files.get_mut(&name) {
                entry.version = version;
            }
        }
    }

    index
        .files
        .retain(|name, _| local_exists(&local_files, name));
    index
        .save(vault)
        .map_err(|e| format!("cannot write sync index: {e}"))?;

    Ok(report)
}

/// Convenience wrapper for one-off runs (CLI, tests) that owns its runtime.
pub fn run_blocking(vault: &Path, cfg: &Config) -> Result<Report, String> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("cannot start async runtime: {e}"))?;
    rt.block_on(sync_once(vault, cfg))
}

/// Create each path segment of the remote folder. Existing collections make
/// `MKCOL` fail with 405, which is exactly what we want to ignore.
async fn ensure_remote_dir(client: &Client, dir: &str) {
    let mut prefix = String::new();
    for segment in dir.split('/').filter(|s| !s.is_empty()) {
        if !prefix.is_empty() {
            prefix.push('/');
        }
        prefix.push_str(segment);
        let _ = client.mkcol(&prefix).await;
    }
}

async fn list_remote(
    client: &Client,
    remote_dir: &str,
) -> Result<BTreeMap<String, String>, String> {
    let listing = client
        .list(remote_dir, Depth::Number(1))
        .await
        .map_err(|e| format!("listing {remote_dir:?} failed: {e}"))?;
    Ok(listing
        .into_iter()
        .filter_map(|entity| match entity {
            ListEntity::File(file) => Some((name_of_href(&file.href), version_of(&file))),
            ListEntity::Folder(_) => None,
        })
        .collect())
}

async fn download(client: &Client, path: &str, name: &str) -> Result<Vec<u8>, String> {
    let response = client
        .get(path)
        .await
        .map_err(|e| format!("download {name} failed: {e}"))?;
    let body = response
        .bytes()
        .await
        .map_err(|e| format!("reading {name} failed: {e}"))?;
    Ok(body.to_vec())
}

fn set_hash(index: &mut Index, name: &str, hash: String) {
    index.files.entry(name.to_string()).or_default().hash = hash;
}

fn local_exists(files: &[PathBuf], name: &str) -> bool {
    files.iter().any(|p| {
        p.file_name()
            .map(|n| n.to_string_lossy() == name)
            .unwrap_or(false)
    })
}

fn version_of(file: &ListFile) -> String {
    version_from(file.tag.as_deref(), &file.last_modified.to_rfc3339())
}

fn version_from(tag: Option<&str>, mtime: &str) -> String {
    match tag {
        Some(tag) if !tag.is_empty() => tag.to_string(),
        _ => mtime.to_string(),
    }
}

fn join_remote(remote_dir: &str, name: &str) -> String {
    if remote_dir.is_empty() {
        encode_segment(name)
    } else {
        format!("{remote_dir}/{}", encode_segment(name))
    }
}

fn conflict_path(vault: &Path, name: &str) -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let stem = name.rsplit_once('.').map(|(s, _)| s).unwrap_or(name);
    let ext = name.rsplit_once('.').map(|(_, e)| e).unwrap_or("md");
    vault.join(format!("{stem}.conflict-{stamp}.{ext}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_conflict_matrix_is_total_and_boring() {
        assert_eq!(plan(false, false), Plan::Nothing);
        assert_eq!(plan(true, false), Plan::Upload);
        assert_eq!(plan(false, true), Plan::Download);
        assert_eq!(plan(true, true), Plan::Conflict);
    }

    #[test]
    fn umlauts_and_spaces_survive_the_url_round_trip() {
        for name in ["über.md", "mein text.md", "a+b&c#d.md", "plain.md"] {
            let encoded = encode_segment(name);
            assert!(!encoded.contains(' '), "{encoded}");
            assert!(!encoded.contains('+'));
            assert!(!encoded.contains('#'));
            assert_eq!(decode_segment(&encoded), name);
        }
    }

    #[test]
    fn hrefs_reduce_to_plain_names() {
        assert_eq!(name_of_href("/dav/cread/%C3%BCber.md"), "über.md");
        assert_eq!(name_of_href("/dav/cread/"), "cread");
        assert_eq!(name_of_href("file.md"), "file.md");
    }

    #[test]
    fn remote_paths_are_encoded_per_segment() {
        assert_eq!(join_remote("dav/cread", "a b.md"), "dav/cread/a%20b.md");
        assert_eq!(join_remote("", "x.md"), "x.md");
    }

    #[test]
    fn versions_prefer_etag_and_fall_back_to_mtime() {
        assert_eq!(
            version_from(Some("\"abc\""), "2026-10-29T10:00:00Z"),
            "\"abc\""
        );
        assert_eq!(
            version_from(Some(""), "2026-10-29T10:00:00Z"),
            "2026-10-29T10:00:00Z"
        );
        assert_eq!(
            version_from(None, "2026-10-29T10:00:00Z"),
            "2026-10-29T10:00:00Z"
        );
    }

    #[test]
    fn report_summaries_read_like_english() {
        assert_eq!(Report::default().summary(), "up to date");
        assert_eq!(
            Report {
                uploaded: 2,
                downloaded: 1,
                conflicts: 1,
                skipped: 0
            }
            .summary(),
            "2 up, 1 down, 1 conflict"
        );
    }

    #[test]
    fn config_needs_a_url_to_be_useful() {
        assert!(!Config::default().is_configured());
        assert!(
            Config {
                url: "https://x".into(),
                ..Config::default()
            }
            .is_configured()
        );
    }
}
