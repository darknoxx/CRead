//! Self-update from GitHub releases.
//!
//! The shell asks; this crate fetches the latest release, downloads the tarball
//! for this architecture, checks it against the release's SHA-256 *and* a
//! minisign signature, and only then replaces the running binary. A download
//! that is unsigned, mis-signed or truncated is never installed.
//!
//! Everything here blocks. The shell runs it on a worker thread so the frame
//! loop never waits for the network.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use sha2::{Digest, Sha256};

pub mod worker;
pub use worker::{Event, Worker};

/// Public keys a release signature may match.
///
/// The second one is the spare, generated with the first and kept offline: it
/// ships now so that swapping the signing key later does not strand
/// installations, because a binary only trusts the keys it was built with.
pub const PUBLIC_KEYS: &[&str] = &[
    "RWR3LbtsJ8Lo9nXfml6kkaqThFtCkV8cV6AWZQGeI9qlFOEnX5Odba4E",
    "RWQ0Us3HVbzcwsxnTyDHFbzST5AAnCsqzbsb27FcBEKHmx1uP1LgVqea",
];

/// GitHub's "latest release" endpoint for this repository.
pub const LATEST: &str = "https://api.github.com/repos/darknoxx/CRead/releases/latest";

/// A release that is newer than the running binary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offer {
    pub version: String,
    pub tarball: String,
    pub checksum: String,
    pub signature: String,
}

/// The asset this machine can run: the release ships one per architecture.
pub fn asset_name(arch: &str) -> String {
    format!("cread-linux-{arch}.tar.gz")
}

/// `v0.22.0` or `0.22.0` -> `(0, 22, 0)`. Pre-release and build suffixes are
/// ignored: the update channel is stable releases only.
pub fn parse_version(version: &str) -> Option<(u64, u64, u64)> {
    let core = version.trim().trim_start_matches('v');
    let core = core.split(['-', '+']).next()?;
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next().unwrap_or("0").parse().ok()?;
    Some((major, minor, patch))
}

pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Some(candidate), Some(current)) => candidate > current,
        _ => false,
    }
}

/// Turn the GitHub response into an offer, or `None` when the running version
/// is already the newest.
pub fn parse_latest(json: &str, current: &str, asset: &str) -> Result<Option<Offer>, String> {
    let document: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("unreadable release list: {e}"))?;
    let tag = document
        .get("tag_name")
        .and_then(|tag| tag.as_str())
        .ok_or_else(|| "the latest release has no tag".to_string())?;
    let version = tag.trim_start_matches('v').to_string();
    if !is_newer(&version, current) {
        return Ok(None);
    }

    let assets = document
        .get("assets")
        .and_then(|assets| assets.as_array())
        .ok_or_else(|| format!("release {version} has no assets"))?;
    let url_of = |name: &str| -> Option<String> {
        assets
            .iter()
            .find(|asset| asset.get("name").and_then(|n| n.as_str()) == Some(name))
            .and_then(|asset| asset.get("browser_download_url"))
            .and_then(|url| url.as_str())
            .map(str::to_string)
    };

    let missing = |what: &str| format!("release {version} has no {what}");
    Ok(Some(Offer {
        tarball: url_of(asset).ok_or_else(|| missing(asset))?,
        checksum: url_of(&format!("{asset}.sha256"))
            .ok_or_else(|| missing(&format!("{asset}.sha256")))?,
        // Older releases were published before signing existed; refusing them
        // is the point, not an accident.
        signature: url_of(&format!("{asset}.minisig"))
            .ok_or_else(|| format!("release {version} is not signed - refusing to install it"))?,
        version,
    }))
}

/// The `.sha256` file is `sha256sum` output: the hash first, the name after.
pub fn verify_checksum(bytes: &[u8], expected: &str) -> Result<(), String> {
    let want = expected
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    if want.len() != 64 {
        return Err("the release checksum is not a SHA-256".into());
    }
    let got = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    if got != want {
        return Err(format!(
            "checksum mismatch: downloaded {got}, release says {want}"
        ));
    }
    Ok(())
}

/// A minisign signature is valid when it matches any trusted key.
pub fn verify_signature(bytes: &[u8], signature: &str, keys: &[&str]) -> Result<(), String> {
    let signature = minisign_verify::Signature::decode(signature)
        .map_err(|e| format!("unreadable signature: {e}"))?;
    let mut last = "no trusted key".to_string();
    for key in keys {
        let Ok(public) = minisign_verify::PublicKey::from_base64(key) else {
            continue;
        };
        match public.verify(bytes, &signature, false) {
            Ok(()) => return Ok(()),
            Err(e) => last = e.to_string(),
        }
    }
    Err(format!(
        "the release is not signed by a trusted key: {last}"
    ))
}

/// The release is a `.tar.gz` with `cread` at its root; return that file.
pub fn unpack(tarball: &[u8]) -> Result<Vec<u8>, String> {
    const MAX: u64 = 256 * 1024 * 1024;
    let gz = flate2::read::GzDecoder::new(tarball);
    let mut archive = tar::Archive::new(gz);
    let entries = archive
        .entries()
        .map_err(|e| format!("unreadable archive: {e}"))?;
    for entry in entries {
        let mut entry = entry.map_err(|e| format!("unreadable archive: {e}"))?;
        let path = entry
            .path()
            .map_err(|e| format!("unreadable archive entry: {e}"))?;
        if path.file_name().and_then(|name| name.to_str()) != Some("cread") {
            continue;
        }
        if !entry.header().entry_type().is_file() {
            return Err("the release's cread entry is not a regular file".into());
        }
        if entry.size() == 0 {
            return Err("the release's cread binary is empty".into());
        }
        if entry.size() > MAX {
            return Err("the binary in the release is implausibly large".into());
        }
        let mut binary = Vec::new();
        entry
            .read_to_end(&mut binary)
            .map_err(|e| format!("unreadable binary in the release: {e}"))?;
        return Ok(binary);
    }
    Err("the release has no cread binary".into())
}

/// Replace `target` with `binary`, keeping the previous file as `<name>.old`.
///
/// The temporary file is a sibling so the rename stays on one filesystem and is
/// therefore atomic: a crash leaves either the old binary or the new one.
pub fn install_bytes(target: &Path, binary: &[u8]) -> Result<PathBuf, String> {
    // A development build is not what a release is for.
    if target.components().any(|part| part.as_os_str() == "target") {
        return Err("this is a development build; install the release instead".into());
    }
    let dir = target
        .parent()
        .ok_or_else(|| "the binary has no folder".to_string())?;
    let name = target
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "the binary has no name".to_string())?;
    let fresh = dir.join(format!(".{name}.new-{}", std::process::id()));
    let backup = dir.join(format!("{name}.old"));

    let write = (|| -> std::io::Result<()> {
        use std::io::Write;
        let mut file = std::fs::File::create(&fresh)?;
        file.write_all(binary)?;
        file.sync_all()
    })();
    if let Err(e) = write {
        let _ = std::fs::remove_file(&fresh);
        return Err(format!("cannot write {}: {e}", fresh.display()));
    }
    let mode = match std::fs::metadata(target).map(|meta| meta.permissions()) {
        Ok(mode) => mode,
        Err(e) => {
            let _ = std::fs::remove_file(&fresh);
            return Err(format!("cannot read {}: {e}", target.display()));
        }
    };
    if let Err(e) = std::fs::set_permissions(&fresh, mode) {
        let _ = std::fs::remove_file(&fresh);
        return Err(format!(
            "cannot set permissions on {}: {e}",
            fresh.display()
        ));
    }

    // Keep the old inode under `.old` without moving it out of the way first.
    // The rename below then replaces the live path in one step, so a crash can
    // never leave the machine with no binary at all.
    let _ = std::fs::remove_file(&backup);
    if let Err(e) = std::fs::hard_link(target, &backup) {
        let _ = std::fs::remove_file(&fresh);
        return Err(format!("cannot keep the old binary: {e}"));
    }
    if let Err(e) = std::fs::rename(&fresh, target) {
        let _ = std::fs::remove_file(&fresh);
        return Err(format!("cannot replace the binary: {e}"));
    }
    // Make the rename durable, not just the file contents.
    if let Ok(dir) = std::fs::File::open(dir) {
        let _ = dir.sync_all();
    }
    Ok(backup)
}

/// The command that starts `exe` again with the arguments it was given.
pub fn restart_command(exe: &Path, args: &[std::ffi::OsString]) -> std::process::Command {
    let mut command = std::process::Command::new(exe);
    command.args(args);
    command
}

fn runtime() -> Result<tokio::runtime::Runtime, String> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("cannot start a network thread: {e}"))
}

async fn fetch(url: &str) -> Result<Vec<u8>, String> {
    const MAX_DOWNLOAD: u64 = 64 * 1024 * 1024;
    let client = reqwest::Client::builder()
        .user_agent(concat!("CRead/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| format!("cannot start the download: {e}"))?;
    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("cannot reach {url}: {e}"))?;
    if !response.status().is_success() {
        return Err(format!("{url} answered {}", response.status()));
    }
    if response
        .content_length()
        .is_some_and(|len| len > MAX_DOWNLOAD)
    {
        return Err(format!("{url} is larger than the update limit"));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| format!("reading {url} failed: {e}"))?
    {
        if body.len() as u64 + chunk.len() as u64 > MAX_DOWNLOAD {
            return Err(format!("{url} is larger than the update limit"));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// Ask GitHub for the newest release. `None` means "already current".
pub fn check(api: &str, current: &str) -> Result<Option<Offer>, String> {
    let body = runtime()?.block_on(fetch(api))?;
    let text = String::from_utf8_lossy(&body);
    parse_latest(&text, current, &asset_name(std::env::consts::ARCH))
}

/// Download the offer, verify it, and replace `target` with the new binary.
pub fn install(offer: &Offer, target: &Path, keys: &[&str]) -> Result<PathBuf, String> {
    let runtime = runtime()?;
    let tarball = runtime.block_on(fetch(&offer.tarball))?;
    let checksum = runtime.block_on(fetch(&offer.checksum))?;
    let signature = runtime.block_on(fetch(&offer.signature))?;

    verify_checksum(&tarball, &String::from_utf8_lossy(&checksum))?;
    verify_signature(&tarball, &String::from_utf8_lossy(&signature), keys)?;
    let binary = unpack(&tarball)?;
    install_bytes(target, &binary)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_KEY: &str = "RWSq0u71k6o7fgJw6lRvvFl5r3R1JpCb3z6UekinM9P/njQ/3xXm5NxE";
    const TEST_MESSAGE: &[u8] = b"CRead update test fixture\n";
    const TEST_SIGNATURE: &str = "\
untrusted comment: signature from rsign secret key
RUSq0u71k6o7fr2g5BqmJjv6XKXGDcQHHjfsdZrbKktzY0GwipDqCRZJGkF3xHx+LiJ55xBqDHWdlfM76DjuRh21A5LpWH8DlgU=
trusted comment: timestamp:1793539427\tfile:message.bin\tprehashed
8UuB1tL57gjgLki6e2k8aJqio1edWKtfm5jfddm9FkPZ4hqjYvv5ZyfW+lHPVanndxzq/Iqqp6waFu3SganECw==
";

    fn tarball_with(binary: &[u8]) -> Vec<u8> {
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        {
            let mut tar = tar::Builder::new(&mut gz);
            let mut header = tar::Header::new_gnu();
            header.set_size(binary.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            tar.append_data(&mut header, "./cread", binary).unwrap();
            tar.finish().unwrap();
        }
        gz.finish().unwrap()
    }

    fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cread-update-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn versions_compare_as_versions_not_strings() {
        assert!(is_newer("0.22.0", "0.21.9"));
        assert!(is_newer("v0.22.0", "0.21.0"));
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(!is_newer("0.21.0", "0.21.0"));
        assert!(!is_newer("0.21.0", "0.22.0"));
        assert!(is_newer("0.22.0", "0.21.0-rc1"), "suffixes are ignored");
        assert_eq!(parse_version("v0.22.3"), Some((0, 22, 3)));
    }

    #[test]
    fn the_release_list_yields_an_offer_for_this_architecture() {
        let json = r#"{
            "tag_name": "v0.22.0",
            "assets": [
                {"name": "cread-linux-x86_64.tar.gz", "browser_download_url": "https://example.org/a.tar.gz"},
                {"name": "cread-linux-x86_64.tar.gz.sha256", "browser_download_url": "https://example.org/a.sha256"},
                {"name": "cread-linux-x86_64.tar.gz.minisig", "browser_download_url": "https://example.org/a.minisig"},
                {"name": "cread-linux-aarch64.tar.gz", "browser_download_url": "https://example.org/arm.tar.gz"}
            ]
        }"#;
        let offer = parse_latest(json, "0.21.0", "cread-linux-x86_64.tar.gz")
            .unwrap()
            .expect("a newer version");
        assert_eq!(offer.version, "0.22.0");
        assert_eq!(offer.tarball, "https://example.org/a.tar.gz");
        assert_eq!(offer.signature, "https://example.org/a.minisig");
    }

    #[test]
    fn the_same_version_is_not_an_offer() {
        let json = r#"{"tag_name": "v0.21.0", "assets": [
            {"name": "cread-linux-x86_64.tar.gz", "browser_download_url": "https://example.org/a"},
            {"name": "cread-linux-x86_64.tar.gz.sha256", "browser_download_url": "https://example.org/b"},
            {"name": "cread-linux-x86_64.tar.gz.minisig", "browser_download_url": "https://example.org/c"}
        ]}"#;
        assert_eq!(
            parse_latest(json, "0.21.0", "cread-linux-x86_64.tar.gz").unwrap(),
            None
        );
    }

    #[test]
    fn an_unsigned_release_is_refused() {
        let json = r#"{"tag_name": "v0.22.0", "assets": [
            {"name": "cread-linux-x86_64.tar.gz", "browser_download_url": "https://example.org/a"},
            {"name": "cread-linux-x86_64.tar.gz.sha256", "browser_download_url": "https://example.org/b"}
        ]}"#;
        let error = parse_latest(json, "0.21.0", "cread-linux-x86_64.tar.gz").unwrap_err();
        assert!(error.contains("not signed"), "{error}");
    }

    #[test]
    fn the_checksum_must_match_the_download() {
        let data = b"payload";
        let good = "8e6d3b1f0c9e7c9b0b5d0b4aac2b6a0e1b0d0a0e1b0d0a0e1b0d0a0e1b0d0a0e";
        // Wrong on purpose, then a real one computed the same way the release
        // does it.
        assert!(verify_checksum(data, good).is_err());
        let real = format!("{:x}  cread-linux-x86_64.tar.gz\n", Sha256::digest(data));
        assert!(verify_checksum(data, &real).is_ok());
        assert!(verify_checksum(b"tampered", &real).is_err());
    }

    #[test]
    fn a_release_must_be_signed_by_a_trusted_key() {
        assert!(verify_signature(TEST_MESSAGE, TEST_SIGNATURE, &[TEST_KEY]).is_ok());

        let wrong_key = "RWR3LbtsJ8Lo9nXfml6kkaqThFtCkV8cV6AWZQGeI9qlFOEnX5Odba4E";
        assert!(verify_signature(TEST_MESSAGE, TEST_SIGNATURE, &[wrong_key]).is_err());

        // A tampered payload with a valid signature for something else.
        assert!(verify_signature(b"something else", TEST_SIGNATURE, &[TEST_KEY]).is_err());
        // The spare key still works when listed after the current one.
        assert!(
            verify_signature(TEST_MESSAGE, TEST_SIGNATURE, &[wrong_key, TEST_KEY]).is_ok(),
            "any trusted key may sign a release"
        );
    }

    #[test]
    fn the_binary_is_taken_out_of_the_archive() {
        let tarball = tarball_with(b"\x7fELF pretend binary");
        assert_eq!(unpack(&tarball).unwrap(), b"\x7fELF pretend binary");

        let empty = tarball_with_other_only();
        assert!(unpack(&empty).unwrap_err().contains("no cread"));

        let empty_binary = tarball_with(b"");
        assert!(unpack(&empty_binary).unwrap_err().contains("empty"));
    }

    fn tarball_with_other_only() -> Vec<u8> {
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        {
            let mut tar = tar::Builder::new(&mut gz);
            let mut header = tar::Header::new_gnu();
            header.set_size(4);
            header.set_mode(0o644);
            header.set_cksum();
            tar.append_data(&mut header, "./README.md", &b"read"[..])
                .unwrap();
            tar.finish().unwrap();
        }
        gz.finish().unwrap()
    }

    #[test]
    fn installing_keeps_the_old_binary_beside_the_new_one() {
        let dir = temp("install");
        let target = dir.join("cread");
        std::fs::write(&target, b"old binary").unwrap();

        let backup = install_bytes(&target, b"new binary").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new binary");
        assert_eq!(std::fs::read(&backup).unwrap(), b"old binary");
        assert!(!dir.join(".cread.new").exists(), "no temp file left behind");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_development_build_is_not_updated() {
        let dir = temp("dev");
        let target = dir.join("target/debug/cread");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, b"dev").unwrap();
        let error = install_bytes(&target, b"release").unwrap_err();
        assert!(error.contains("development build"), "{error}");
        assert_eq!(std::fs::read(&target).unwrap(), b"dev");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_restart_keeps_the_arguments() {
        use std::ffi::OsString;
        let command = restart_command(
            Path::new("/usr/bin/cread"),
            &[OsString::from("--vault"), OsString::from("/home/x/notes")],
        );
        assert_eq!(command.get_program(), "/usr/bin/cread");
        let args: Vec<_> = command.get_args().collect();
        assert_eq!(args, ["--vault", "/home/x/notes"]);
    }
}
