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
    crate::storage::pictures_in(vault)
}

/// Every `.png` under the folder, subfolders included, sorted by path.
pub fn list(vault: &Path) -> std::io::Result<Vec<PathBuf>> {
    let dir = dir(vault);
    if !dir.exists() {
        return Ok(Vec::new());
    }
    Ok(crate::storage::walk_files(&dir)
        .into_iter()
        .filter(|path| {
            path.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("png"))
        })
        .collect())
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

/// Resolve a user-supplied picture name, adding `.png`.
///
/// Pictures always land in `Pictures/`, subfolders allowed, `..` not: a
/// painting is never saved somewhere else in the vault. A name that folds away
/// to nothing (`""`, `.`, `..`) is an error, not a file next to the folder.
pub fn resolve(vault: &Path, name: &str) -> Result<PathBuf, String> {
    let mut relative = PathBuf::new();
    for part in Path::new(name).components() {
        if let std::path::Component::Normal(part) = part {
            relative.push(part);
        }
    }
    if relative.as_os_str().is_empty() {
        return Err("a picture needs a name".into());
    }
    let path = dir(vault).join(relative);
    Ok(match path.extension() {
        Some(ext) if !ext.is_empty() => path,
        _ => path.with_extension("png"),
    })
}

/// Write a surface as a 16-colour PNG.
pub fn save(path: &Path, raster: &Raster) -> Result<(), String> {
    let png = encode_png(raster.width(), raster.height(), &raster.to_rgba())
        .map_err(|e| format!("cannot encode {}: {e}", path.display()))?;
    crate::storage::save_bytes_atomic(path, &png)
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

/// Encode tightly packed RGBA8 pixels as a PNG in memory.
pub fn encode_png(width: u32, height: u32, rgba: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        writer
            .write_image_data(rgba)
            .map_err(|e| std::io::Error::other(e.to_string()))?;
    }
    Ok(out)
}

/// Write tightly packed RGBA8 pixels as a PNG.
pub fn write_png(
    path: impl AsRef<Path>,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> std::io::Result<()> {
    let bytes = encode_png(width, height, rgba)?;
    crate::storage::save_bytes_atomic(path.as_ref(), &bytes)
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
    fn a_trailing_dot_still_gets_the_png_extension() {
        let vault = temp("extension");
        let path = resolve(&vault, "doodle.").unwrap();
        assert_eq!(path.file_name().unwrap(), "doodle.png");
        std::fs::remove_dir_all(&vault).unwrap();
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
            resolve(vault, "doodle").unwrap(),
            Path::new("/tmp/vault/Pictures/doodle.png")
        );
        assert_eq!(
            resolve(vault, "doodle.png").unwrap(),
            Path::new("/tmp/vault/Pictures/doodle.png")
        );
    }

    #[test]
    fn a_picture_cannot_escape_its_folder() {
        let vault = Path::new("/tmp/vault");
        assert_eq!(
            resolve(vault, "../../etc/passwd").unwrap(),
            Path::new("/tmp/vault/Pictures/etc/passwd.png"),
            "dots are dropped, not followed"
        );
        assert_eq!(
            resolve(vault, "/tmp/elsewhere/x.png").unwrap(),
            Path::new("/tmp/vault/Pictures/tmp/elsewhere/x.png"),
            "an absolute name is re-rooted inside Pictures"
        );
        assert_eq!(
            resolve(vault, "sketch/blue").unwrap(),
            Path::new("/tmp/vault/Pictures/sketch/blue.png"),
            "subfolders are fine"
        );
        for empty in ["", ".", "..", "../.."] {
            assert!(
                resolve(vault, empty).is_err(),
                "{empty:?} must not become a file next to the folder"
            );
        }
    }

    #[test]
    fn a_missing_folder_lists_as_empty() {
        let vault = temp("missing");
        assert!(list(&vault).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&vault);
    }
}
