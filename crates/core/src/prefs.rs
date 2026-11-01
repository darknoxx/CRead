//! Remembered preferences: the bits of UI state that should survive a restart.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{storage, theme::Theme};

/// Stored as JSON in the config directory. Every field has a default, so an old
/// or hand-edited file with missing keys still loads.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub theme: String,
    pub typewriter: bool,
    pub scanlines: bool,
    pub bloom: bool,
    /// Let the scanline pattern drift downwards.
    pub roll: bool,
    /// "classic" (VGA) or "thin" (ISO), see `cread_render::Face`.
    pub font: String,
    /// The PC speaker beep once the screen has settled.
    pub beep: bool,
    /// Write the document while typing. Off by default: saving is the writer's
    /// decision, not the program's.
    pub autosave: bool,
    /// Loudness of the music player, 0 to 100.
    pub volume: u8,
    /// Folder or file last inserted into Riff.
    pub riff_last: String,
    /// Shuffle the playlist.
    pub riff_random: bool,
    /// Ids of the modules installed through SOFTWARE. Inspiro is the system and
    /// is never in here.
    pub installed: Vec<String>,
    /// Blank the screen with the clock after a while without input.
    pub screensaver: bool,
    /// Minutes of silence before the screensaver starts.
    pub screensaver_minutes: u32,
    /// 24-hour clock in the bars; off means 12 hours with AM/PM.
    pub clock_24h: bool,
    /// German date order (`07.11.26`); off means `11/07/26`.
    pub date_day_first: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        let crt = cread_crt_defaults();
        Self {
            theme: Theme::DEFAULT.name.to_string(),
            typewriter: false,
            scanlines: crt.0,
            bloom: crt.1,
            roll: true,
            font: "classic".to_string(),
            beep: true,
            autosave: false,
            volume: 70,
            riff_last: String::new(),
            riff_random: false,
            installed: Vec::new(),
            screensaver: true,
            screensaver_minutes: 5,
            clock_24h: true,
            date_day_first: true,
        }
    }
}

/// The renderer's defaults, so preferences and code agree on what "on" means.
/// Scanlines and their drift are on out of the box: the moving raster is what
/// stops the pattern reading as a printed overlay.
fn cread_crt_defaults() -> (bool, bool) {
    (true, true)
}

impl Prefs {
    pub fn path() -> PathBuf {
        storage::config_dir().join("prefs.json")
    }

    pub fn load() -> Self {
        Self::load_from(&Self::path())
    }

    pub fn load_from(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> io::Result<()> {
        self.save_to(&Self::path())
    }

    pub fn save_to(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text =
            serde_json::to_string_pretty(self).map_err(|e| io::Error::other(e.to_string()))?;
        std::fs::write(path, text)
    }

    /// Resolved theme, falling back to the default if the name is unknown.
    pub fn theme(&self) -> Theme {
        Theme::by_name(&self.theme).unwrap_or(Theme::DEFAULT)
    }

    pub fn set(&mut self, theme: Theme) {
        self.theme = theme.name.to_string();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cread-prefs-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn defaults_are_paper_with_scanlines_and_roll() {
        let p = Prefs::default();
        assert_eq!(p.theme(), Theme::PAPER);
        assert!(p.scanlines);
        assert!(p.roll);
        assert!(p.bloom);
        assert!(!p.typewriter);
        assert!(p.beep);
        assert!(!p.autosave, "saving is manual until asked for");
        assert_eq!(p.volume, 70);
        assert!(p.riff_last.is_empty());
        assert!(p.screensaver, "the clock is on out of the box");
        assert_eq!(p.screensaver_minutes, 5);
    }

    #[test]
    fn round_trips_through_a_file() {
        let dir = tmp("roundtrip");
        let path = dir.join("prefs.json");
        let saved = Prefs {
            theme: "amber".into(),
            typewriter: true,
            scanlines: false,
            bloom: true,
            roll: false,
            font: "thin".into(),
            beep: false,
            autosave: true,
            volume: 40,
            riff_last: "/music/Morgenlicht".into(),
            riff_random: true,
            installed: vec!["riff".into()],
            screensaver: false,
            screensaver_minutes: 15,
            clock_24h: false,
            date_day_first: false,
        };
        saved.save_to(&path).unwrap();
        assert_eq!(Prefs::load_from(&path), saved);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_keys_fall_back_to_defaults() {
        let dir = tmp("partial");
        let path = dir.join("prefs.json");
        std::fs::write(&path, r#"{"theme":"green"}"#).unwrap();
        let loaded = Prefs::load_from(&path);
        assert_eq!(loaded.theme(), Theme::GREEN);
        assert_eq!(loaded.scanlines, Prefs::default().scanlines);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn garbage_and_unknown_themes_do_not_break_startup() {
        let dir = tmp("garbage");
        let path = dir.join("prefs.json");
        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(Prefs::load_from(&path), Prefs::default());

        std::fs::write(&path, r#"{"theme":"neon"}"#).unwrap();
        assert_eq!(Prefs::load_from(&path).theme(), Theme::DEFAULT);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
