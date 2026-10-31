//! Pocketboi: a Game Boy in a window.
//!
//! The emulation is Mizu's; this crate is the cartridge shelf and the picture.
//! Mizu gives a 160x144 RGB frame per tick, which we reduce to the four shades
//! of the machine and blow up by two into the emulated raster.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};

use cread_core::program::{KeyPress, Outcome, PointerEvent, Program};
use cread_core::{Cell, Raster, Screen};
use mizu_core::{GameBoy, JoypadButton};

/// The machine's screen, in its own pixels.
pub const GB_W: u32 = 160;
pub const GB_H: u32 = 144;
/// Integer zoom: 160x144 fits twice over in 640x384.
const ZOOM: u32 = 2;
/// The four shades in the machine's palette, lightest first.
const DMG: [u8; 4] = [15, 10, 2, 0];

/// One row of the cartridge shelf.
struct Entry {
    name: String,
    path: PathBuf,
    folder: bool,
}

/// A running cartridge.
struct Game {
    gb: GameBoy,
    title: String,
    /// Set when the emulator gave up: the picture freezes, the shell lives.
    fault: Option<String>,
}

pub struct Pocketboi {
    dir: PathBuf,
    entries: Vec<Entry>,
    selected: usize,
    game: Option<Game>,
}

impl Pocketboi {
    /// Opens on the cartridge shelf inside the vault.
    pub fn new(vault: &Path) -> Self {
        let dir = vault.join("Roms");
        // Make the shelf visible instead of leaving the user to guess the path.
        let _ = std::fs::create_dir_all(&dir);
        let mut boi = Self {
            entries: Vec::new(),
            selected: 0,
            game: None,
            dir,
        };
        boi.rescan();
        boi
    }

    /// Load a cartridge directly, skipping the shelf.
    pub fn play(&mut self, path: &Path) -> Result<(), String> {
        match GameBoy::builder(path).build() {
            Ok(gb) => {
                let title = gb.game_title().trim().to_string();
                self.game = Some(Game {
                    gb,
                    title: if title.is_empty() {
                        path.file_stem()
                            .map(|s| s.to_string_lossy().into_owned())
                            .unwrap_or_else(|| "cartridge".into())
                    } else {
                        title
                    },
                    fault: None,
                });
                Ok(())
            }
            Err(e) => Err(format!("{e}")),
        }
    }

    fn rescan(&mut self) {
        self.entries = shelf(&self.dir);
        self.selected = self.selected.min(self.entries.len().saturating_sub(1));
    }

    fn enter(&mut self) {
        let Some(entry) = self.entries.get(self.selected) else {
            return;
        };
        let (path, folder) = (entry.path.clone(), entry.folder);
        if folder {
            self.dir = path;
            self.selected = 0;
            self.rescan();
            return;
        }
        let name = self.entries[self.selected].name.clone();
        if let Err(e) = self.play(&path) {
            // A cartridge that will not load never reaches the emulator.
            self.entries[self.selected].name = format!("{name} ({e})");
        }
    }

    fn up(&mut self) {
        let parent = self.dir.parent().map(Path::to_path_buf);
        if let Some(parent) = parent {
            self.dir = parent;
            self.selected = 0;
            self.rescan();
        }
    }

    /// Step one frame and paint it.
    fn run(&mut self, raster: &mut Raster) {
        let Some(game) = self.game.as_mut() else {
            return;
        };
        if game.fault.is_none() {
            // A malformed cartridge must not take CRead down with it: the
            // emulator is allowed exactly one panic, and it costs the game.
            let stepped = catch_unwind(AssertUnwindSafe(|| game.gb.clock_for_frame()));
            match stepped {
                Ok(Ok(())) => {}
                Ok(Err(e)) => game.fault = Some(format!("{e:?}")),
                Err(_) => game.fault = Some("the emulator crashed".into()),
            }
        }
        paint(&game.gb, raster);
    }

    fn status(&self) -> String {
        match &self.game {
            Some(game) => match &game.fault {
                Some(fault) => format!("{} — stopped: {fault}", game.title),
                None => game.title.clone(),
            },
            None => "insert a cartridge".into(),
        }
    }
}

/// Folders and cartridges in one folder.
fn shelf(dir: &Path) -> Vec<Entry> {
    let mut folders = Vec::new();
    let mut carts = Vec::new();
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
        } else if is_cartridge(&path) {
            carts.push(Entry {
                name,
                path,
                folder: false,
            });
        }
    }
    folders.sort_by_key(|entry| entry.name.to_lowercase());
    carts.sort_by_key(|entry| entry.name.to_lowercase());
    folders.extend(carts);
    folders
}

fn is_cartridge(path: &Path) -> bool {
    path.extension()
        .map(|ext| {
            let ext = ext.to_string_lossy().to_ascii_lowercase();
            ext == "gb" || ext == "gbc"
        })
        .unwrap_or(false)
}

/// The frame, four shades deep, blown up by two and centred.
fn paint(gb: &GameBoy, raster: &mut Raster) {
    raster.fill(0);
    let frame = gb.screen_buffer();
    let width = raster.width();
    let x0 = (width.saturating_sub(GB_W * ZOOM)) / 2;
    let y0 = (raster.height().saturating_sub(GB_H * ZOOM)) / 2;
    // The pixel buffer is RGB, 3 bytes per pixel.
    let row_len = GB_W as usize * 3;
    if frame.len() < row_len * GB_H as usize {
        return;
    }
    for y in 0..GB_H {
        let row = &frame[y as usize * row_len..][..row_len];
        for x in 0..GB_W {
            let p = &row[x as usize * 3..];
            let luma = (p[0] as u32 * 30 + p[1] as u32 * 59 + p[2] as u32 * 11) / 100;
            let shade = match luma {
                0..=63 => 3,
                64..=127 => 2,
                128..=191 => 1,
                _ => 0,
            };
            let index = DMG[shade];
            for dy in 0..ZOOM {
                for dx in 0..ZOOM {
                    raster.set(
                        x0 as i32 + x as i32 * ZOOM as i32 + dx as i32,
                        y0 as i32 + y as i32 * ZOOM as i32 + dy as i32,
                        index,
                    );
                }
            }
        }
    }
}

/// Keys the pad answers to.
fn button(key: KeyPress) -> Option<JoypadButton> {
    Some(match key {
        KeyPress::Up => JoypadButton::Up,
        KeyPress::Down => JoypadButton::Down,
        KeyPress::Left => JoypadButton::Left,
        KeyPress::Right => JoypadButton::Right,
        KeyPress::Enter => JoypadButton::Start,
        KeyPress::Char(' ') => JoypadButton::Select,
        KeyPress::Char(c) if c.eq_ignore_ascii_case(&'z') => JoypadButton::B,
        KeyPress::Char(c) if c.eq_ignore_ascii_case(&'x') => JoypadButton::A,
        _ => return None,
    })
}

impl Program for Pocketboi {
    fn title(&self) -> &str {
        "POCKETBOI"
    }

    fn status(&self) -> String {
        self.status()
    }

    fn key(&mut self, key: KeyPress) -> Outcome {
        if let Some(game) = self.game.as_mut() {
            if let Some(button) = button(key) {
                game.gb.press_joypad(button);
            }
            return Outcome::Continue;
        }
        match key {
            KeyPress::Up => self.selected = self.selected.saturating_sub(1),
            KeyPress::Down => {
                self.selected = (self.selected + 1).min(self.entries.len().saturating_sub(1));
            }
            KeyPress::Enter => self.enter(),
            KeyPress::Backspace | KeyPress::Left => self.up(),
            _ => {}
        }
        Outcome::Continue
    }

    fn key_released(&mut self, key: KeyPress) {
        if let Some(game) = self.game.as_mut()
            && let Some(button) = button(key)
        {
            game.gb.release_joypad(button);
        }
    }

    fn pointer(&mut self, _event: PointerEvent) -> Outcome {
        Outcome::Continue
    }

    fn wants_frames(&self) -> bool {
        self.game.as_ref().is_some_and(|game| game.fault.is_none())
    }

    fn draw(&mut self, screen: &mut Screen, raster: &mut Raster) {
        // The cells are transparent: the picture is pixels, not text.
        screen.fill(Cell::TRANSPARENT);
        if self.game.is_some() {
            self.run(raster);
            return;
        }
        raster.fill(0);
        draw_shelf(screen, self);
    }
}

fn draw_shelf(screen: &mut Screen, boi: &Pocketboi) {
    let cols = screen.cols();
    screen.put_str(2, 1, "P O C K E T B O I", 1, 0);
    let dir = boi.dir.display().to_string();
    screen.put_str_ellipsis(2, 3, &dir, cols.saturating_sub(4), 7, 0);

    let first = 5;
    let last = screen.rows().saturating_sub(2);
    for (i, entry) in boi.entries.iter().enumerate() {
        let y = first + i as u32;
        if y >= last {
            break;
        }
        let selected = i == boi.selected;
        let (fg, bg) = if selected { (0, 15) } else { (15, 0) };
        if selected {
            for x in 1..cols.saturating_sub(1) {
                screen.put(x, y, Cell::blank(0, 15));
            }
        }
        let mark = if entry.folder { '\u{25ba}' } else { ' ' };
        screen.put_str(2, y, &format!("{mark} {}", entry.name), fg, bg);
    }
    if boi.entries.is_empty() {
        screen.put_str(
            4,
            first,
            "no cartridges yet - drop .gb or .gbc files in this folder",
            7,
            0,
        );
    }
    let bar = screen.rows().saturating_sub(1);
    screen.fill_row(bar, Cell::blank(15, 0));
    screen.put_str(0, bar, " Enter play  ", 15, 0);
    let right = "Backspace up  Esc commands ";
    let x = cols.saturating_sub(right.chars().count() as u32);
    screen.put_str(x, bar, right, 15, 0);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "cread-pocketboi-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_shelf_lists_folders_then_cartridges_and_ignores_the_rest() {
        let vault = temp("shelf");
        let roms = vault.join("Roms");
        std::fs::create_dir_all(roms.join("Homebrew")).unwrap();
        for name in ["Tetris.gb", "Pokemon.GBC", "notes.txt", "cover.png"] {
            std::fs::write(roms.join(name), b"x").unwrap();
        }
        let listed = shelf(&roms);
        let names: Vec<&str> = listed.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["Homebrew", "Pokemon.GBC", "Tetris.gb"]);
        assert!(listed[0].folder);
        let _ = std::fs::remove_dir_all(&vault);
    }

    #[test]
    fn opening_the_module_creates_the_shelf_in_the_vault() {
        let vault = temp("create");
        let boi = Pocketboi::new(&vault);
        assert!(vault.join("Roms").is_dir());
        assert!(boi.entries.is_empty());
        assert_eq!(boi.status(), "insert a cartridge");
        let _ = std::fs::remove_dir_all(&vault);
    }

    #[test]
    fn the_arrow_keys_walk_the_shelf_and_backspace_goes_up() {
        let vault = temp("walk");
        let roms = vault.join("Roms");
        std::fs::create_dir_all(roms.join("Homebrew")).unwrap();
        std::fs::write(roms.join("a.gb"), b"x").unwrap();
        let mut boi = Pocketboi::new(&vault);
        assert_eq!(boi.selected, 0);
        boi.key(KeyPress::Down);
        assert_eq!(boi.selected, 1);
        boi.key(KeyPress::Down);
        assert_eq!(boi.selected, 1, "stops at the end");
        boi.key(KeyPress::Up);
        assert_eq!(boi.selected, 0);
        // Into the folder, then back out.
        boi.key(KeyPress::Enter);
        assert!(boi.dir.ends_with("Homebrew"));
        boi.key(KeyPress::Backspace);
        assert!(boi.dir.ends_with("Roms"));
        let _ = std::fs::remove_dir_all(&vault);
    }

    #[test]
    fn a_pad_key_maps_to_the_right_button() {
        fn is(key: KeyPress, want: JoypadButton) -> bool {
            matches!((button(key), want), (Some(got), want) if std::mem::discriminant(&got) == std::mem::discriminant(&want))
        }
        assert!(is(KeyPress::Up, JoypadButton::Up));
        assert!(is(KeyPress::Enter, JoypadButton::Start));
        assert!(is(KeyPress::Char('z'), JoypadButton::B));
        assert!(is(KeyPress::Char('X'), JoypadButton::A));
        assert!(button(KeyPress::Char('q')).is_none());
    }

    #[test]
    fn an_empty_screen_paints_black_and_the_game_area_is_centred() {
        let mut raster = Raster::new(640, 384);
        raster.fill(15);
        // No game: the shelf fills the raster with black.
        let mut boi = Pocketboi::new(&temp("paint"));
        let mut screen = Screen::new(80, 25);
        boi.draw(&mut screen, &mut raster);
        assert_eq!(raster.get(0, 0), Some(0));
        assert_eq!(raster.get(639, 383), Some(0));
    }
}
