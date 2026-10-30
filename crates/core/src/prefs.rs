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
    /// "classic" (VGA) or "thin" (ISO), see `cread_render::Face`.
    pub font: String,
}

impl Default for Prefs {
    fn default() -> Self {
        let crt = cread_crt_defaults();
        Self {
            theme: Theme::BLACK.name.to_string(),
            typewriter: false,
            scanlines: false,
            bloom: crt.1,
            font: "classic".to_string(),
        }
    }
}

/// The renderer's defaults, so preferences and code agree on what "on" means.
/// Horizontal scanlines are off out of the box: at two device pixels per line
/// they can only be a hard comb through the glyphs, which reads as a filter.
/// `scanlines on` opts into them.
fn cread_crt_defaults() -> (bool, bool) {
    (false, true)
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
        Theme::by_name(&self.theme).unwrap_or(Theme::BLACK)
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
    fn defaults_are_the_black_theme_with_both_effects_on() {
        let p = Prefs::default();
        assert_eq!(p.theme(), Theme::BLACK);
        assert!(!p.scanlines, "scanlines are opt-in");
        assert!(p.bloom);
        assert!(!p.typewriter);
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
            font: "thin".into(),
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
        assert_eq!(Prefs::load_from(&path).theme(), Theme::BLACK);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
