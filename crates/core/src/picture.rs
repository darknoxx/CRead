//! Pictures: 16-colour PNGs in `<vault>/Pictures`.
//!
//! The vault holds documents as text and pictures as images, side by side. A
//! picture is always written in the fixed [`crate::raster::VGA`] palette, so the
//! file looks the same whichever theme CRead is running in.

use std::io::BufReader;
use std::path::{Path, PathBuf};

use crate::raster::Raster;

/// Folder pictures live in, inside the vault.
pub fn dir(vault: &Path) -> PathBuf {
    vault.join("Pictures")
}

/// Every `.png` in the folder, by name.
pub fn list(vault: &Path) -> std::io::Result<Vec<PathBuf>> {
    let dir = dir(vault);
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    for entry in std::fs::read_dir(&dir)? {
        let path = entry?.path();
        let is_png = path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("png"));
        if is_png && path.is_file() {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

/// First free `picture[-n].png` in the vault.
pub fn free_name(vault: &Path) -> PathBuf {
    let dir = dir(vault);
    let mut path = dir.join("picture.png");
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("picture-{n}.png"));
        n += 1;
    }
    path
}

/// Resolve a user-supplied picture name inside the vault, adding `.png`.
pub fn resolve(vault: &Path, name: &str) -> PathBuf {
    let raw = PathBuf::from(name);
    let path = if raw.is_absolute() {
        raw
    } else {
        dir(vault).join(raw)
    };
    if path.extension().is_some() {
        path
    } else {
        path.with_extension("png")
    }
}

/// Write a surface as a 16-colour PNG.
pub fn save(path: &Path, raster: &Raster) -> Result<(), String> {
    if let Some(parent) = path.parent()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        return Err(format!("cannot create {}: {e}", parent.display()));
    }
    write_png(path, raster.width(), raster.height(), &raster.to_rgba())
        .map_err(|e| format!("cannot write {}: {e}", path.display()))
}

/// Read a PNG back into palette indices.
pub fn load(path: &Path) -> Result<Raster, String> {
    let file =
        std::fs::File::open(path).map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    let mut decoder = png::Decoder::new(BufReader::new(file));
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder
        .read_info()
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let size = reader
        .output_buffer_size()
        .ok_or_else(|| format!("{}: image too large", path.display()))?;
    let mut buffer = vec![0; size];
    let info = reader
        .next_frame(&mut buffer)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let data = &buffer[..info.buffer_size()];
    let (width, height) = (info.width, info.height);
    let pixels = width as usize * height as usize;

    let mut rgba = Vec::with_capacity(pixels * 4);
    match info.color_type {
        png::ColorType::Rgba => rgba.extend_from_slice(data),
        png::ColorType::Rgb => {
            for pixel in data.as_chunks::<3>().0 {
                rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 0xff]);
            }
        }
        png::ColorType::Grayscale => {
            for &value in data {
                rgba.extend_from_slice(&[value, value, value, 0xff]);
            }
        }
        png::ColorType::GrayscaleAlpha => {
            for pixel in data.as_chunks::<2>().0 {
                rgba.extend_from_slice(&[pixel[0], pixel[0], pixel[0], pixel[1]]);
            }
        }
        png::ColorType::Indexed => return Err("palette PNG did not expand".into()),
    }
    Ok(Raster::from_rgba(width, height, &rgba))
}

/// Write tightly packed RGBA8 pixels as a PNG.
pub fn write_png(
    path: impl AsRef<Path>,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> std::io::Result<()> {
    let file = std::fs::File::create(path)?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    writer
        .write_image_data(rgba)
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "cread-picture-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_picture_round_trips_through_png() {
        let vault = temp("roundtrip");
        let mut raster = Raster::new(16, 8);
        raster.set(3, 4, 12);
        raster.dab(10, 4, 3, 2);
        let path = dir(&vault).join("test.png");
        save(&path, &raster).unwrap();
        let back = load(&path).unwrap();
        assert_eq!(back, raster);
        let _ = std::fs::remove_dir_all(&vault);
    }

    #[test]
    fn names_count_up_and_listing_is_sorted() {
        let vault = temp("names");
        assert_eq!(free_name(&vault), dir(&vault).join("picture.png"));
        std::fs::create_dir_all(dir(&vault)).unwrap();
        std::fs::write(dir(&vault).join("picture.png"), b"x").unwrap();
        assert_eq!(free_name(&vault), dir(&vault).join("picture-2.png"));
        std::fs::write(dir(&vault).join("a.png"), b"x").unwrap();
        std::fs::write(dir(&vault).join("notes.txt"), b"x").unwrap();
        let names: Vec<_> = list(&vault)
            .unwrap()
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["a.png", "picture.png"]);
        let _ = std::fs::remove_dir_all(&vault);
    }

    #[test]
    fn a_bare_name_gets_the_pictures_folder_and_an_extension() {
        let vault = Path::new("/tmp/vault");
        assert_eq!(
            resolve(vault, "doodle"),
            Path::new("/tmp/vault/Pictures/doodle.png")
        );
        assert_eq!(
            resolve(vault, "doodle.png"),
            Path::new("/tmp/vault/Pictures/doodle.png")
        );
    }

    #[test]
    fn a_missing_folder_lists_as_empty() {
        let vault = temp("missing");
        assert!(list(&vault).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&vault);
    }
}
