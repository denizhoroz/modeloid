//! Small JPEG previews of dataset images, cached on disk.

use std::{
    fs,
    hash::{Hash, Hasher},
    io::BufWriter,
    path::{Path, PathBuf},
};

use image::codecs::jpeg::JpegEncoder;

const QUALITY: u8 = 80;

/// A preview of `src` at most `max` pixels on its longer side, cached in
/// `cache_dir`. The cache key includes the source's size and modification
/// time, so an edited image gets a fresh preview.
pub fn thumbnail(src: &Path, cache_dir: &Path, max: u32) -> Result<PathBuf, crate::Error> {
    let meta = fs::metadata(src).map_err(|e| format!("cannot read {}: {e}", src.display()))?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (src, meta.len(), meta.modified().ok(), max).hash(&mut hasher);
    let path = cache_dir.join(format!("{:016x}.jpg", hasher.finish()));
    if path.is_file() {
        return Ok(path);
    }

    let image = image::open(src).map_err(|e| format!("cannot decode {}: {e}", src.display()))?;
    let small = image.thumbnail(max, max).to_rgb8();
    fs::create_dir_all(cache_dir)?;
    // Written under a temporary name so a half-written file is never served.
    // The name is this thread's own: two requests for one preview don't share it.
    let part = path.with_extension(format!("{:?}.part", std::thread::current().id()));
    let mut out = BufWriter::new(fs::File::create(&part)?);
    small.write_with_encoder(JpegEncoder::new_with_quality(&mut out, QUALITY))?;
    drop(out);
    if let Err(e) = crate::core::io::rename(&part, &path) {
        let _ = fs::remove_file(&part);
        // Another request finished the same preview first: use that one.
        if !path.is_file() {
            return Err(e.into());
        }
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn makes_and_reuses_thumbnails() {
        let dir = std::env::temp_dir().join(format!("modeloid-thumbs-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let src = dir.join("big.png");
        image::RgbImage::from_pixel(400, 200, image::Rgb([200, 30, 30])).save(&src).unwrap();

        let cache = dir.join("cache");
        let thumb = thumbnail(&src, &cache, 100).unwrap();
        assert_eq!(image::image_dimensions(&thumb).unwrap(), (100, 50));
        let modified = fs::metadata(&thumb).unwrap().modified().unwrap();
        assert_eq!(thumbnail(&src, &cache, 100).unwrap(), thumb, "cached");
        assert_eq!(fs::metadata(&thumb).unwrap().modified().unwrap(), modified, "not rewritten");
        assert_ne!(thumbnail(&src, &cache, 50).unwrap(), thumb, "size is part of the key");

        fs::write(dir.join("broken.jpg"), "not an image").unwrap();
        assert!(thumbnail(&dir.join("broken.jpg"), &cache, 100).is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
