//! Backups on a USB stick: find it, mount it, copy the vault, let it go.
//!
//! `lsblk` says what is plugged in, `udisksctl` mounts and powers off without
//! root (a polkit rule allows exactly that), and the copy is plain file IO. A
//! backup never overwrites: every run gets a folder of its own.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::run;

/// A filesystem on a removable USB drive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stick {
    /// The filesystem's device, e.g. `/dev/sda1`.
    pub device: String,
    /// The whole drive, for powering it off: `/dev/sda`.
    pub disk: String,
    pub label: String,
    pub size: String,
    pub fstype: String,
    pub mountpoint: Option<PathBuf>,
}

impl Stick {
    /// What the list shows: the label, or the device when it has none.
    pub fn name(&self) -> String {
        if self.label.is_empty() {
            self.device.trim_start_matches("/dev/").to_string()
        } else {
            self.label.clone()
        }
    }
}

/// The columns [`parse_sticks`] reads.
const LSBLK: &[&str] = &[
    "-J",
    "-o",
    "NAME,PATH,TYPE,RM,HOTPLUG,TRAN,FSTYPE,LABEL,SIZE,MOUNTPOINTS",
];

pub fn sticks() -> Result<Vec<Stick>, String> {
    let out = run("lsblk", LSBLK, None)?;
    Ok(parse_sticks(&out))
}

/// `lsblk -J`: filesystems on drives that are removable and on USB. The SD
/// card the machine runs from is neither, so it can never be picked.
pub fn parse_sticks(json: &str) -> Vec<Stick> {
    let Ok(root) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for disk in root["blockdevices"].as_array().into_iter().flatten() {
        let removable =
            disk["rm"].as_bool() == Some(true) || disk["hotplug"].as_bool() == Some(true);
        if disk["type"] != "disk" || disk["tran"] != "usb" || !removable {
            continue;
        }
        let disk_path = text(&disk["path"]);
        let children = disk["children"].as_array().cloned().unwrap_or_default();
        // A stick formatted without a partition table carries its
        // filesystem on the drive itself.
        let parts = if children.is_empty() {
            vec![disk.clone()]
        } else {
            children
        };
        for part in parts {
            let fstype = text(&part["fstype"]);
            if fstype.is_empty() || fstype == "swap" {
                continue;
            }
            let mountpoint = part["mountpoints"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .find(|m| !m.is_empty())
                .map(PathBuf::from);
            out.push(Stick {
                device: text(&part["path"]),
                disk: disk_path.clone(),
                label: text(&part["label"]),
                size: text(&part["size"]),
                fstype,
                mountpoint,
            });
        }
    }
    out
}

fn text(value: &Value) -> String {
    value.as_str().unwrap_or("").to_string()
}

/// Mount the stick (or find where it already is).
pub fn mount(stick: &Stick) -> Result<PathBuf, String> {
    if let Some(at) = &stick.mountpoint {
        return Ok(at.clone());
    }
    let out = run(
        "udisksctl",
        &["mount", "-b", &stick.device, "--no-user-interaction"],
        None,
    )?;
    parse_mounted(&out).ok_or_else(|| format!("udisksctl did not say where: {}", out.trim()))
}

/// `Mounted /dev/sda1 at /media/creaduser/STICK` (older versions end with `.`).
pub fn parse_mounted(out: &str) -> Option<PathBuf> {
    let (_, at) = out.split_once(" at ")?;
    let at = at.trim().trim_end_matches('.');
    (!at.is_empty()).then(|| PathBuf::from(at))
}

/// Unmount and power the drive off, so it can be pulled out.
pub fn release(stick: &Stick) -> Result<(), String> {
    run(
        "udisksctl",
        &["unmount", "-b", &stick.device, "--no-user-interaction"],
        None,
    )?;
    // Some card readers refuse to power off; unmounted is safe enough.
    let _ = run(
        "udisksctl",
        &["power-off", "-b", &stick.disk, "--no-user-interaction"],
        None,
    );
    Ok(())
}

/// What a backup did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    pub folder: PathBuf,
    pub copied: usize,
    /// Files that could not be written (names a FAT stick does not allow).
    pub skipped: Vec<String>,
}

/// The vault's files, hidden ones left out (the sync index stays local, as it
/// does for WebDAV).
pub fn files(vault: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut todo = vec![vault.to_path_buf()];
    while let Some(dir) = todo.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let path = entry.path();
            match entry.file_type() {
                Ok(t) if t.is_dir() => todo.push(path),
                Ok(t) if t.is_file() => out.push(path),
                _ => {}
            }
        }
    }
    out.sort();
    out
}

/// `CRead-Backup-2026-11-22`, or `-2`, `-3` … when that exists already.
pub fn free_folder(root: &Path, date: &str) -> PathBuf {
    let base = format!("CRead-Backup-{date}");
    let mut candidate = root.join(&base);
    let mut n = 2;
    while candidate.exists() {
        candidate = root.join(format!("{base}-{n}"));
        n += 1;
    }
    candidate
}

/// Copy the vault into a new folder under `root`. `progress` hears
/// `(done, total)` after every file.
pub fn backup(
    vault: &Path,
    root: &Path,
    date: &str,
    mut progress: impl FnMut(usize, usize),
) -> Result<Report, String> {
    let list = files(vault);
    let folder = free_folder(root, date);
    fs::create_dir_all(&folder).map_err(|e| format!("cannot write to the stick: {e}"))?;
    let mut report = Report {
        folder: folder.clone(),
        copied: 0,
        skipped: Vec::new(),
    };
    let total = list.len();
    for (index, file) in list.iter().enumerate() {
        let relative = file.strip_prefix(vault).unwrap_or(file);
        let target = folder.join(relative);
        let copied = target
            .parent()
            .map_or(Ok(()), fs::create_dir_all)
            .and_then(|()| fs::copy(file, &target).map(|_| ()));
        match copied {
            Ok(()) => report.copied += 1,
            Err(_) => report.skipped.push(relative.display().to_string()),
        }
        progress(index + 1, total);
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LSBLK_OUT: &str = r#"{"blockdevices": [
      {"name":"mmcblk0","path":"/dev/mmcblk0","type":"disk","rm":false,"hotplug":false,"tran":null,"fstype":null,"label":null,"size":"29.7G","mountpoints":[null],
       "children":[
         {"name":"mmcblk0p1","path":"/dev/mmcblk0p1","type":"part","rm":false,"hotplug":false,"tran":null,"fstype":"vfat","label":"bootfs","size":"512M","mountpoints":["/boot/firmware"]},
         {"name":"mmcblk0p2","path":"/dev/mmcblk0p2","type":"part","rm":false,"hotplug":false,"tran":null,"fstype":"ext4","label":"rootfs","size":"29.2G","mountpoints":["/"]}
       ]},
      {"name":"sda","path":"/dev/sda","type":"disk","rm":true,"hotplug":true,"tran":"usb","fstype":null,"label":null,"size":"14.6G","mountpoints":[null],
       "children":[
         {"name":"sda1","path":"/dev/sda1","type":"part","rm":true,"hotplug":true,"tran":null,"fstype":"vfat","label":"KINGSTON","size":"14.6G","mountpoints":[null]}
       ]},
      {"name":"sdb","path":"/dev/sdb","type":"disk","rm":false,"hotplug":true,"tran":"usb","fstype":"exfat","label":null,"size":"58G","mountpoints":["/media/u/sdb"]},
      {"name":"sdc","path":"/dev/sdc","type":"disk","rm":false,"hotplug":false,"tran":"sata","fstype":"ext4","label":"DATA","size":"1T","mountpoints":[null]}
    ]}"#;

    #[test]
    fn only_usb_sticks_are_offered_never_the_system_card() {
        let sticks = parse_sticks(LSBLK_OUT);
        assert_eq!(sticks.len(), 2, "{sticks:?}");
        assert_eq!(sticks[0].device, "/dev/sda1");
        assert_eq!(sticks[0].disk, "/dev/sda");
        assert_eq!(sticks[0].name(), "KINGSTON");
        assert_eq!(sticks[0].mountpoint, None);
        assert_eq!(sticks[1].device, "/dev/sdb");
        assert_eq!(sticks[1].name(), "sdb");
        assert_eq!(sticks[1].mountpoint, Some(PathBuf::from("/media/u/sdb")));
        assert!(parse_sticks("not json").is_empty());
    }

    #[test]
    fn the_mount_point_is_read_from_udisksctl() {
        assert_eq!(
            parse_mounted("Mounted /dev/sda1 at /media/creaduser/KINGSTON\n"),
            Some(PathBuf::from("/media/creaduser/KINGSTON"))
        );
        assert_eq!(
            parse_mounted("Mounted /dev/sda1 at /media/u/STICK.\n"),
            Some(PathBuf::from("/media/u/STICK"))
        );
        assert_eq!(parse_mounted("Error"), None);
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cread-usb-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_backup_copies_the_rooms_and_leaves_hidden_files_home() {
        let vault = scratch("vault");
        let stick = scratch("stick");
        fs::create_dir_all(vault.join("Documents/Roman")).unwrap();
        fs::create_dir_all(vault.join("Pictures")).unwrap();
        fs::write(vault.join("Documents/Roman/kapitel-1.md"), "Es war einmal").unwrap();
        fs::write(vault.join("Pictures/bild.png"), [0u8, 1, 2]).unwrap();
        fs::write(vault.join(".cread-sync"), "index").unwrap();

        let mut seen = Vec::new();
        let report = backup(&vault, &stick, "2026-11-22", |done, total| {
            seen.push((done, total))
        })
        .unwrap();
        assert_eq!(report.copied, 2);
        assert!(report.skipped.is_empty());
        assert_eq!(seen.last(), Some(&(2, 2)));
        let folder = stick.join("CRead-Backup-2026-11-22");
        assert_eq!(report.folder, folder);
        assert_eq!(
            fs::read_to_string(folder.join("Documents/Roman/kapitel-1.md")).unwrap(),
            "Es war einmal"
        );
        assert!(!folder.join(".cread-sync").exists());

        // A second run the same day never touches the first.
        let again = backup(&vault, &stick, "2026-11-22", |_, _| {}).unwrap();
        assert_eq!(again.folder, stick.join("CRead-Backup-2026-11-22-2"));

        let _ = fs::remove_dir_all(&vault);
        let _ = fs::remove_dir_all(&stick);
    }
}
