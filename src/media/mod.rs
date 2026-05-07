//! Derivative image generation and caching.
//!
//! Source photos are never modified. Derivatives (thumbnail, medium) are
//! written to a cache directory with content-keyed filenames so they are
//! safe to delete and will be regenerated on next request.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use exif::{In, Reader as ExifReader, Tag};
use thiserror::Error;

// ── Public types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ImageSize {
    /// Max 400 px wide, aspect-ratio preserved.
    Thumbnail,
    /// Max 1200 px wide, aspect-ratio preserved.
    Medium,
}

impl ImageSize {
    fn max_width(self) -> u32 {
        match self {
            Self::Thumbnail => 400,
            Self::Medium => 1200,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Thumbnail => "thumb",
            Self::Medium => "medium",
        }
    }
}

#[derive(Debug, Error)]
pub(crate) enum MediaError {
    #[error("io error accessing {path}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("image processing failed for {path}")]
    Image {
        path: PathBuf,
        #[source]
        source: image::ImageError,
    },
}

// ── MediaCache ────────────────────────────────────────────────────────────────

pub(crate) struct MediaCache {
    cache_dir: PathBuf,
}

impl MediaCache {
    pub(crate) fn new(cache_dir: impl Into<PathBuf>) -> Self {
        Self {
            cache_dir: cache_dir.into(),
        }
    }

    /// Returns the path to a derivative of `source` at the requested `size`.
    ///
    /// If the derivative does not yet exist it is generated on a blocking
    /// thread before returning. Subsequent calls for the same source + size
    /// return immediately from the cache.
    ///
    /// # Errors
    ///
    /// Returns [`MediaError`] if the cache directory cannot be created, the
    /// source image cannot be opened, or the derivative cannot be written.
    pub(crate) async fn ensure(&self, source: &Path, size: ImageSize) -> Result<PathBuf, MediaError> {
        let dest = self.derivative_path(source, size)?;
        if dest.exists() {
            return Ok(dest);
        }

        let source = source.to_owned();
        let dest_clone = dest.clone();
        tokio::task::spawn_blocking(move || generate_derivative(&source, &dest_clone, size))
            .await
            .expect("image generation task panicked")?;

        Ok(dest)
    }

    /// Compute the cache path for a given source + size pair.
    ///
    /// The filename encodes a hash of the source path and its modification
    /// time, so it changes if the source file is replaced.
    fn derivative_path(&self, source: &Path, size: ImageSize) -> Result<PathBuf, MediaError> {
        let meta = std::fs::metadata(source).map_err(|e| MediaError::Io {
            path: source.to_owned(),
            source: e,
        })?;
        let mtime = meta.modified().unwrap_or(UNIX_EPOCH);

        let mut h = DefaultHasher::new();
        source.hash(&mut h);
        mtime
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
            .hash(&mut h);
        size.hash(&mut h);
        let key = h.finish();

        std::fs::create_dir_all(&self.cache_dir).map_err(|e| MediaError::Io {
            path: self.cache_dir.clone(),
            source: e,
        })?;

        Ok(self
            .cache_dir
            .join(format!("{:016x}-{}.jpg", key, size.label())))
    }
}

// ── Image generation (runs on blocking thread) ────────────────────────────────

fn read_exif_orientation(source: &Path) -> u32 {
    (|| -> Option<u32> {
        let file = std::fs::File::open(source).ok()?;
        let mut buf = std::io::BufReader::new(file);
        let exif = ExifReader::new().read_from_container(&mut buf).ok()?;
        exif.get_field(Tag::Orientation, In::PRIMARY)
            .and_then(|f| f.value.get_uint(0))
    })()
    .unwrap_or(1)
}

fn apply_exif_orientation(img: image::DynamicImage, orientation: u32) -> image::DynamicImage {
    match orientation {
        2 => img.fliph(),
        3 => img.rotate180(),
        4 => img.flipv(),
        5 => img.rotate90().fliph(),
        6 => img.rotate90(),
        7 => img.rotate270().fliph(),
        8 => img.rotate270(),
        _ => img,
    }
}

fn generate_derivative(source: &Path, dest: &Path, size: ImageSize) -> Result<(), MediaError> {
    let img = image::open(source).map_err(|e| MediaError::Image {
        path: source.to_owned(),
        source: e,
    })?;

    let orientation = read_exif_orientation(source);
    let img = apply_exif_orientation(img, orientation);

    let resized = if img.width() > size.max_width() {
        img.resize(
            size.max_width(),
            u32::MAX,
            image::imageops::FilterType::Lanczos3,
        )
    } else {
        img
    };

    resized
        .save_with_format(dest, image::ImageFormat::Jpeg)
        .map_err(|e| MediaError::Image {
            path: dest.to_owned(),
            source: e,
        })?;

    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_test_image(path: &Path, width: u32, height: u32) {
        let img = image::RgbImage::new(width, height);
        image::DynamicImage::ImageRgb8(img)
            .save_with_format(path, image::ImageFormat::Png)
            .unwrap();
    }

    #[tokio::test]
    async fn ensure_generates_thumbnail() {
        let tmp = TempDir::new().unwrap();
        let source = tmp.path().join("photo.png");
        write_test_image(&source, 2000, 1500);

        let cache = MediaCache::new(tmp.path().join("cache"));
        let result = cache.ensure(&source, ImageSize::Thumbnail).await.unwrap();

        assert!(result.exists());
        assert_eq!(result.extension().unwrap(), "jpg");
        let img = image::open(&result).unwrap();
        assert!(
            img.width() <= 400,
            "thumbnail width {} should be ≤ 400",
            img.width()
        );
    }

    #[tokio::test]
    async fn ensure_generates_medium() {
        let tmp = TempDir::new().unwrap();
        let source = tmp.path().join("photo.png");
        write_test_image(&source, 4000, 3000);

        let cache = MediaCache::new(tmp.path().join("cache"));
        let result = cache.ensure(&source, ImageSize::Medium).await.unwrap();

        assert!(result.exists());
        let img = image::open(&result).unwrap();
        assert!(
            img.width() <= 1200,
            "medium width {} should be ≤ 1200",
            img.width()
        );
    }

    #[tokio::test]
    async fn ensure_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        let source = tmp.path().join("photo.png");
        write_test_image(&source, 2000, 1500);

        let cache = MediaCache::new(tmp.path().join("cache"));
        let first = cache.ensure(&source, ImageSize::Thumbnail).await.unwrap();
        let second = cache.ensure(&source, ImageSize::Thumbnail).await.unwrap();

        assert_eq!(first, second);
        assert_eq!(
            std::fs::read_dir(tmp.path().join("cache")).unwrap().count(),
            1,
            "should not create duplicate cache files"
        );
    }

    #[tokio::test]
    async fn ensure_does_not_upscale_small_image() {
        let tmp = TempDir::new().unwrap();
        let source = tmp.path().join("small.png");
        write_test_image(&source, 100, 100);

        let cache = MediaCache::new(tmp.path().join("cache"));
        let result = cache.ensure(&source, ImageSize::Thumbnail).await.unwrap();

        let img = image::open(&result).unwrap();
        assert_eq!(img.width(), 100, "small image should not be upscaled");
    }

    #[test]
    fn derivative_path_differs_by_size() {
        let tmp = TempDir::new().unwrap();
        let source = tmp.path().join("photo.png");
        write_test_image(&source, 10, 10);

        let cache = MediaCache::new(tmp.path().join("cache"));
        let thumb_path = cache
            .derivative_path(&source, ImageSize::Thumbnail)
            .unwrap();
        let medium_path = cache.derivative_path(&source, ImageSize::Medium).unwrap();

        assert_ne!(thumb_path, medium_path);
        assert!(thumb_path.to_string_lossy().contains("thumb"));
        assert!(medium_path.to_string_lossy().contains("medium"));
    }
}
