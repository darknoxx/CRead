//! The music folder: reading it, and the playlist built from it.
//!
//! Nothing here knows about audio devices. A "track" is a file with a name; the
//! title comes from the file stem and the album from the folder, which is what a
//! folder of `01 Morgenlicht.flac` actually offers.

use std::path::{Path, PathBuf};

/// What Riff can play. Everything the decoder handles and nothing else, so the
/// explorer never offers a file that would fail on Enter.
pub const EXTENSIONS: [&str; 4] = ["mp3", "flac", "ogg", "wav"];

/// The XDG music folder, falling back to `~/Music`.
pub fn music_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("XDG_MUSIC_DIR").map(PathBuf::from)
        && dir.is_dir()
    {
        return dir;
    }
    if let Some(dir) = crate::storage::xdg_user_dir("XDG_MUSIC_DIR") {
        return dir;
    }
    let home = crate::storage::home_dir();
    let music = home.join("Music");
    if music.is_dir() {
        return music;
    }
    // German desktops name it differently; worth a glance before giving up.
    let german = home.join("Musik");
    if german.is_dir() {
        return german;
    }
    music
}

pub fn is_audio(path: &Path) -> bool {
    path.extension()
        .map(|ext| {
            let ext = ext.to_string_lossy().to_ascii_lowercase();
            EXTENSIONS.contains(&ext.as_str())
        })
        .unwrap_or(false)
}

/// One playable file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Track {
    pub path: PathBuf,
    /// File stem: "01 Morgenlicht".
    pub title: String,
    /// Name of the folder it came from.
    pub album: String,
}

impl Track {
    /// A track from a file name: the stem is the title, the folder the album.
    pub fn from_path(path: PathBuf) -> Self {
        let title = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "untitled".into());
        let album = path
            .parent()
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        Self { path, title, album }
    }
}

/// One row of the explorer: a folder to open or a track to play.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub folder: bool,
}

/// Everything the explorer shows, folders first, then the tracks, alphabetically.
pub fn entries(dir: &Path) -> Vec<Entry> {
    let mut folders = Vec::new();
    let mut tracks = Vec::new();
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    for entry in read.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            folders.push(Entry {
                name,
                path,
                folder: true,
            });
        } else if is_audio(&path) {
            tracks.push(Entry {
                name,
                path,
                folder: false,
            });
        }
    }
    folders.sort_by_key(|entry| entry.name.to_lowercase());
    tracks.sort_by_key(|entry| entry.name.to_lowercase());
    folders.extend(tracks);
    folders
}

/// The playable files of one folder, in name order.
pub fn tracks_in(dir: &Path) -> Vec<Track> {
    entries(dir)
        .into_iter()
        .filter(|entry| !entry.folder)
        .map(|entry| Track::from_path(entry.path))
        .collect()
}

/// A tiny deterministic generator, so shuffle is testable and needs no crate.
#[derive(Clone, Copy, Debug)]
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        let mut x = self.0.max(1);
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, limit: usize) -> usize {
        if limit == 0 {
            0
        } else {
            (self.next() % limit as u64) as usize
        }
    }
}

/// An ordered list to play through, with an optional shuffle.
#[derive(Clone, Debug)]
pub struct Playlist {
    tracks: Vec<Track>,
    /// Indices into `tracks`, in playing order. Identity unless shuffled.
    order: Vec<usize>,
    /// Position in `order`.
    pos: usize,
    shuffle: bool,
    rng: Rng,
}

impl Default for Playlist {
    fn default() -> Self {
        Self {
            tracks: Vec::new(),
            order: Vec::new(),
            pos: 0,
            shuffle: false,
            rng: Rng(seed()),
        }
    }
}

fn seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9e37_79b9_7f4a_7c15)
}

impl Playlist {
    /// A new list, starting at `start` (clamped).
    pub fn load(tracks: Vec<Track>, start: usize) -> Self {
        let mut playlist = Self {
            tracks,
            order: Vec::new(),
            pos: 0,
            shuffle: false,
            rng: Rng(seed()),
        };
        playlist.order = (0..playlist.tracks.len()).collect();
        playlist.pos = start.min(playlist.tracks.len().saturating_sub(1));
        playlist
    }

    /// The same, with a fixed shuffle seed, for tests.
    pub fn with_seed(tracks: Vec<Track>, start: usize, seed: u64) -> Self {
        let mut playlist = Self::load(tracks, start);
        playlist.rng = Rng(seed);
        playlist
    }

    pub fn set_shuffle(&mut self, on: bool) {
        if on == self.shuffle {
            return;
        }
        self.shuffle = on;
        self.rewrite_order();
    }

    pub fn shuffle(&self) -> bool {
        self.shuffle
    }

    fn rewrite_order(&mut self) {
        let current = self.current_index();
        self.order = (0..self.tracks.len()).collect();
        if self.shuffle {
            // Fisher-Yates, then put the running track back in front.
            for i in (1..self.order.len()).rev() {
                let j = self.rng.below(i + 1);
                self.order.swap(i, j);
            }
        }
        if let Some(current) = current {
            let at = self
                .order
                .iter()
                .position(|&index| index == current)
                .unwrap_or(0);
            self.order.swap(0, at);
            self.pos = 0;
        }
    }

    pub fn len(&self) -> usize {
        self.tracks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }

    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    /// Index into `tracks` of the current track.
    pub fn current_index(&self) -> Option<usize> {
        self.order.get(self.pos).copied()
    }

    pub fn current(&self) -> Option<&Track> {
        self.current_index().and_then(|i| self.tracks.get(i))
    }

    /// How far into the list we are, for a "3 of 12" readout.
    pub fn position(&self) -> (usize, usize) {
        (self.pos + 1, self.order.len())
    }

    /// Step forward. Returns false when the list is over.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> bool {
        if self.pos + 1 >= self.order.len() {
            return false;
        }
        self.pos += 1;
        true
    }

    /// Step back. Returns false at the start.
    pub fn prev(&mut self) -> bool {
        if self.pos == 0 {
            return false;
        }
        self.pos -= 1;
        true
    }

    /// Jump to a track by index into `tracks`.
    pub fn select(&mut self, index: usize) -> bool {
        let Some(at) = self.order.iter().position(|&i| i == index) else {
            return false;
        };
        self.pos = at;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "cread-music-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn tracks(names: &[&str]) -> Vec<Track> {
        names
            .iter()
            .map(|name| Track::from_path(PathBuf::from(format!("/music/album/{name}.flac"))))
            .collect()
    }

    #[test]
    fn the_explorer_lists_folders_before_tracks_and_skips_the_rest() {
        let dir = temp("explorer");
        std::fs::create_dir_all(dir.join("Zebra")).unwrap();
        std::fs::create_dir_all(dir.join("adagio")).unwrap();
        for name in ["b.mp3", "a.FLAC", "notes.txt", "cover.png"] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        let listed = entries(&dir);
        let names: Vec<&str> = listed.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["adagio", "Zebra", "a.FLAC", "b.mp3"]);
        assert!(listed[0].folder);
        assert!(!listed[2].folder);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_track_takes_its_title_from_the_file_and_its_album_from_the_folder() {
        let dir = temp("album");
        let album = dir.join("Morgenlicht");
        std::fs::create_dir_all(&album).unwrap();
        std::fs::write(album.join("01 Aufwachen.flac"), b"x").unwrap();
        std::fs::write(album.join("02 Kaffee.ogg"), b"x").unwrap();
        let list = tracks_in(&album);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].title, "01 Aufwachen");
        assert_eq!(list[0].album, "Morgenlicht");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_playlist_walks_forwards_and_stops_at_the_end() {
        let mut list = Playlist::load(tracks(&["a", "b", "c"]), 0);
        assert_eq!(list.current().unwrap().title, "a");
        assert!(list.next());
        assert_eq!(list.current().unwrap().title, "b");
        assert!(list.next());
        assert!(!list.next(), "the end is the end");
        assert_eq!(list.position(), (3, 3));
        assert!(list.prev());
        assert_eq!(list.current().unwrap().title, "b");
    }

    #[test]
    fn starting_mid_list_is_clamped() {
        let list = Playlist::load(tracks(&["a", "b"]), 9);
        assert_eq!(list.current().unwrap().title, "b");
    }

    #[test]
    fn shuffle_changes_the_order_but_keeps_every_track() {
        let list = tracks(&["a", "b", "c", "d", "e"]);
        let mut shuffled = Playlist::with_seed(list.clone(), 0, 42);
        shuffled.set_shuffle(true);
        let mut seen: Vec<String> = Vec::new();
        loop {
            seen.push(shuffled.current().unwrap().title.clone());
            if !shuffled.next() {
                break;
            }
        }
        seen.sort();
        let mut expected: Vec<String> = list.iter().map(|t| t.title.clone()).collect();
        expected.sort();
        assert_eq!(seen, expected, "no track lost, none doubled");
    }

    #[test]
    fn turning_shuffle_off_keeps_the_running_track() {
        let mut list = Playlist::with_seed(tracks(&["a", "b", "c", "d"]), 0, 7);
        list.set_shuffle(true);
        list.next();
        let running = list.current().unwrap().title.clone();
        list.set_shuffle(false);
        assert_eq!(list.current().unwrap().title, running);
        assert_eq!(list.position(), (1, 4), "back in file order");
    }

    #[test]
    fn select_jumps_to_a_track() {
        let mut list = Playlist::load(tracks(&["a", "b", "c"]), 0);
        assert!(list.select(2));
        assert_eq!(list.current().unwrap().title, "c");
        assert!(!list.select(9));
    }

    #[test]
    fn the_music_folder_is_somewhere_sensible() {
        let dir = music_dir();
        assert!(dir.is_absolute(), "{dir:?}");
        assert!(
            dir.ends_with("Music") || dir.ends_with("Musik"),
            "falls back to a music folder: {dir:?}"
        );
    }
}
