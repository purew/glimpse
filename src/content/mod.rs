//! Content model: scans `posts/`, parses frontmatter + markdown, discovers photos.
//!
//! Pure module — no HTTP, no HTML templating.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use exif::{In, Reader as ExifReader, Tag};
use pulldown_cmark::{Options, Parser, html};
use serde::Deserialize;
use thiserror::Error;
use tracing::{info, warn};

// ── Errors ────────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub(crate) enum ContentError {
    #[error("io error reading {path}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("missing or malformed frontmatter in {path}")]
    MissingFrontmatter { path: PathBuf },

    #[error("invalid frontmatter YAML in {path}")]
    InvalidFrontmatter {
        path: PathBuf,
        #[source]
        source: serde_yaml::Error,
    },
}

// ── Public model ──────────────────────────────────────────────────────────────

/// EXIF metadata extracted from a photo at load time.
#[derive(Debug, Clone)]
pub(crate) struct ExifData {
    /// Formatted capture time, e.g. `"2025-03-18 14:32"`.
    pub(crate) datetime: Option<String>,
    /// Camera make + model, e.g. `"Nikon Z6_3"`.
    pub(crate) camera: Option<String>,
    /// Lens model, e.g. `"Nikon Nikkor Z 50mm f/1.8 S"`.
    pub(crate) lens: Option<String>,
    /// Aperture, e.g. `"f/2.8"`.
    pub(crate) aperture: Option<String>,
    /// Shutter speed, e.g. `"1/250s"`.
    pub(crate) shutter: Option<String>,
    /// ISO sensitivity, e.g. `"ISO 400"`.
    pub(crate) iso: Option<String>,
    /// Focal length, e.g. `"50mm"`.
    pub(crate) focal_length: Option<String>,
}

/// A single media item (photo or video) within a post.
#[derive(Debug, Clone)]
pub(crate) struct MediaItem {
    pub(crate) path: PathBuf,
    pub(crate) is_video: bool,
    /// `None` for videos or photos without readable EXIF.
    pub(crate) exif: Option<ExifData>,
    /// Pixel dimensions. `None` for videos or unreadable images.
    pub(crate) dimensions: Option<(u32, u32)>,
}

/// A group of media from one subfolder (subfolder name becomes a section heading).
#[derive(Debug, Clone)]
pub(crate) struct PhotoGroup {
    /// Display name: frontmatter title if present, else subfolder name; empty string for flat media.
    pub(crate) name: String,
    /// Pre-rendered HTML from a section `index.md`, if one exists in the subfolder.
    pub(crate) body_html: Option<String>,
    pub(crate) media: Vec<MediaItem>,
}

/// A single post parsed from a `posts/` subfolder.
#[derive(Debug, Clone)]
pub(crate) struct Post {
    /// URL-safe identifier derived from the folder name.
    pub(crate) slug: String,
    pub(crate) title: String,
    /// ISO 8601 date string (YYYY-MM-DD).
    pub(crate) date: String,
    /// Groups allowed to view this post. Empty = draft (admin-only).
    pub(crate) access: Vec<String>,
    pub(crate) cover: Option<PathBuf>,
    /// Markdown body pre-rendered to HTML at load time.
    pub(crate) body_html: String,
    pub(crate) photo_groups: Vec<PhotoGroup>,
    pub(crate) source_dir: PathBuf,
}

impl Post {
    pub(crate) fn is_draft(&self) -> bool {
        self.access.is_empty()
    }
}

/// The full in-memory site model.
pub(crate) struct Site {
    /// Posts sorted ascending by date.
    pub(crate) posts: Vec<Post>,
}

// ── Private helpers ───────────────────────────────────────────────────────────

/// Raw YAML frontmatter fields.
#[derive(Debug, Deserialize)]
struct Frontmatter {
    title: String,
    date: serde_yaml::Value, // accept both bare dates and quoted strings
    #[serde(default)]
    access: Vec<String>,
    cover: Option<String>,
}

/// Optional frontmatter for a section `index.md` inside a post subfolder.
#[derive(Debug, Deserialize, Default)]
struct SectionFrontmatter {
    title: Option<String>,
}

/// Split `---\n<yaml>\n---\n<body>` into (yaml_str, body_str).
fn split_frontmatter<'a>(
    content: &'a str,
    path: &Path,
) -> Result<(&'a str, &'a str), ContentError> {
    let rest = content
        .strip_prefix("---\n")
        .ok_or_else(|| ContentError::MissingFrontmatter {
            path: path.to_owned(),
        })?;
    let end = rest
        .find("\n---\n")
        .ok_or_else(|| ContentError::MissingFrontmatter {
            path: path.to_owned(),
        })?;
    Ok((&rest[..end], &rest[end + 5..]))
}

fn render_markdown(markdown: &str) -> String {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TABLES);
    let parser = Parser::new_ext(markdown, opts);
    let mut html_output = String::new();
    html::push_html(&mut html_output, parser);
    html_output
}

/// Parse a section `index.md` in `dir`, returning `(title_override, body_html)`.
///
/// Returns `None` when no `index.md` exists. Frontmatter is optional; if absent
/// the entire file is treated as the Markdown body.
fn parse_section(dir: &Path) -> Option<(Option<String>, Option<String>)> {
    let index_path = dir.join("index.md");
    let content = std::fs::read_to_string(&index_path).ok()?;

    let (title, body_html) = match split_frontmatter(&content, &index_path) {
        Ok((yaml, body)) => {
            let title = serde_yaml::from_str::<SectionFrontmatter>(yaml)
                .ok()
                .and_then(|fm| fm.title);
            let body_html = if body.trim().is_empty() { None } else { Some(render_markdown(body)) };
            (title, body_html)
        }
        Err(_) => {
            let body_html = if content.trim().is_empty() { None } else { Some(render_markdown(&content)) };
            (None, body_html)
        }
    };

    Some((title, body_html))
}

/// Convert a folder name like `"2025-03-18 Hawaii"` to `"2025-03-18-hawaii"`.
fn slug_from_dir_name(name: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    for word in name.split_whitespace() {
        let normalized: String = word
            .to_lowercase()
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' {
                    c
                } else {
                    '-'
                }
            })
            .collect();
        for segment in normalized.split('-').filter(|s| !s.is_empty()) {
            parts.push(segment.to_owned());
        }
    }
    parts.join("-")
}

fn is_photo(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(str::to_lowercase)
            .as_deref(),
        Some("jpg" | "jpeg" | "png" | "webp" | "gif")
    )
}

fn is_video(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(str::to_lowercase)
            .as_deref(),
        Some("mp4" | "mov" | "webm")
    )
}

fn is_nsfw(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.to_lowercase().contains("nsfw"))
}

fn is_web_optimized(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.contains("web-optimized"))
}

fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    a
}

fn ascii_exif(exif: &exif::Exif, tag: Tag) -> Option<String> {
    exif.get_field(tag, In::PRIMARY).and_then(|f| {
        if let exif::Value::Ascii(ref v) = f.value {
            v.first()
                .and_then(|s| std::str::from_utf8(s).ok())
                .map(|s| s.trim_matches('\0').trim().to_owned())
                .filter(|s| !s.is_empty())
        } else {
            None
        }
    })
}

fn rational_exif(exif: &exif::Exif, tag: Tag) -> Option<exif::Rational> {
    exif.get_field(tag, In::PRIMARY).and_then(|f| {
        if let exif::Value::Rational(ref v) = f.value {
            v.first().copied()
        } else {
            None
        }
    })
}

fn format_aperture(r: exif::Rational) -> String {
    if r.denom == 0 {
        return String::new();
    }
    let value = f64::from(r.num) / f64::from(r.denom);
    if value.fract() == 0.0 { format!("f/{value:.0}") } else { format!("f/{value:.1}") }
}

fn format_shutter(r: exif::Rational) -> String {
    if r.denom == 0 || r.num == 0 {
        return String::new();
    }
    if r.denom == 1 {
        return format!("{}s", r.num);
    }
    let g = gcd(r.num, r.denom);
    let n = r.num / g;
    let d = r.denom / g;
    if n == 1 { format!("1/{d}s") } else { format!("{n}/{d}s") }
}

fn title_case(s: &str) -> String {
    s.split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => {
                    first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase()
                }
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn format_focal_length(r: exif::Rational) -> String {
    if r.denom == 0 {
        return String::new();
    }
    let mm = f64::from(r.num) / f64::from(r.denom);
    if mm.fract() == 0.0 {
        format!("{mm:.0}mm")
    } else {
        format!("{mm:.1}mm")
    }
}

fn format_exif_datetime(raw: &str) -> String {
    // EXIF stores "2025:03:18 14:32:00"; convert to "2025-03-18 14:32"
    if raw.len() >= 16 {
        let date = raw[..10].replace(':', "-");
        format!("{date} {}", &raw[11..16])
    } else {
        raw.to_owned()
    }
}

fn exiftool_lens_cache_path(path: &Path, cache_dir: &Path) -> Option<PathBuf> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta.modified().unwrap_or(UNIX_EPOCH);
    let mut h = DefaultHasher::new();
    path.hash(&mut h);
    mtime.duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos().hash(&mut h);
    let key = h.finish();
    let exif_cache = cache_dir.join("exif");
    std::fs::create_dir_all(&exif_cache).ok()?;
    Some(exif_cache.join(format!("{key:016x}-lens.txt")))
}

fn exiftool_lens(path: &Path, cache_dir: &Path) -> Option<String> {
    let cache_path = exiftool_lens_cache_path(path, cache_dir)?;

    if cache_path.exists() {
        let cached = std::fs::read_to_string(&cache_path).ok()?;
        let s = cached.trim().to_owned();
        return if s.is_empty() { None } else { Some(s) };
    }

    let output = std::process::Command::new("exiftool")
        .args(["-Lens", "-s3"])
        .arg(path)
        .output()
        .ok()?;

    let lens = if output.status.success() {
        let s = std::str::from_utf8(&output.stdout).ok()?.trim().to_owned();
        if s.is_empty() { None } else { Some(s) }
    } else {
        None
    };

    let _ = std::fs::write(&cache_path, lens.as_deref().unwrap_or(""));
    lens
}

fn read_exif(path: &Path, cache_dir: &Path) -> Option<ExifData> {
    let file = std::fs::File::open(path).ok()?;
    let exif = ExifReader::new()
        .read_from_container(&mut BufReader::new(file))
        .ok()?;

    let datetime = ascii_exif(&exif, Tag::DateTimeOriginal)
        .or_else(|| ascii_exif(&exif, Tag::DateTime))
        .map(|s| format_exif_datetime(&s))
        .filter(|s| !s.is_empty());

    let make = ascii_exif(&exif, Tag::Make).map(|s| title_case(&s));
    let model = ascii_exif(&exif, Tag::Model);
    let camera = match (make, model) {
        (Some(mk), Some(mo)) if mo.to_uppercase().starts_with(&mk.to_uppercase()) => Some(mo),
        (Some(mk), Some(mo)) => Some(format!("{mk} {mo}")),
        (Some(mk), None) => Some(mk),
        (None, Some(mo)) => Some(mo),
        (None, None) => None,
    };

    let lens = ascii_exif(&exif, Tag::LensModel)
        .or_else(|| exiftool_lens(path, cache_dir));

    let aperture = rational_exif(&exif, Tag::FNumber)
        .map(format_aperture)
        .filter(|s| !s.is_empty());

    let shutter = rational_exif(&exif, Tag::ExposureTime)
        .map(format_shutter)
        .filter(|s| !s.is_empty());

    let iso = [Tag::PhotographicSensitivity, Tag::ISOSpeed]
        .iter()
        .find_map(|&tag| {
            exif.get_field(tag, In::PRIMARY).and_then(|f| {
                // Some cameras (e.g. Nikon Z6 III) store ISO as int32s (SLong).
                f.value.get_uint(0).or_else(|| {
                    if let exif::Value::SLong(ref v) = f.value {
                        v.first().copied().and_then(|n| u32::try_from(n).ok())
                    } else {
                        None
                    }
                })
            })
        })
        .map(|v| format!("ISO {v}"));

    let focal_length = rational_exif(&exif, Tag::FocalLength)
        .map(format_focal_length)
        .filter(|s| !s.is_empty());

    if datetime.is_none()
        && camera.is_none()
        && lens.is_none()
        && aperture.is_none()
        && shutter.is_none()
        && iso.is_none()
        && focal_length.is_none()
    {
        return None;
    }

    Some(ExifData { datetime, camera, lens, aperture, shutter, iso, focal_length })
}

fn read_dimensions(path: &Path) -> Option<(u32, u32)> {
    let reader = image::ImageReader::open(path).ok()?;
    let (w, h) = reader.into_dimensions().ok()?;
    Some((w, h))
}

fn collect_media(dir: &Path, cache_dir: &Path) -> Result<Vec<MediaItem>, ContentError> {
    let entries = std::fs::read_dir(dir).map_err(|e| ContentError::Io {
        path: dir.to_owned(),
        source: e,
    })?;
    let mut items: Vec<MediaItem> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            if is_nsfw(p) {
                info!(path = %p.display(), "skipping nsfw media file");
                return false;
            }
            if is_video(p) && is_web_optimized(p) {
                info!(path = %p.display(), "ingesting web-optimized video");
                return true;
            }
            is_photo(p)
        })
        .map(|p| {
            let is_video = is_video(&p);
            let exif = if is_video { None } else { read_exif(&p, cache_dir) };
            let dimensions = if is_video { None } else { read_dimensions(&p) };
            MediaItem { path: p, is_video, exif, dimensions }
        })
        .collect();
    items.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(items)
}

fn discover_photo_groups(post_dir: &Path, cache_dir: &Path) -> Result<Vec<PhotoGroup>, ContentError> {
    let mut entries: Vec<_> = std::fs::read_dir(post_dir)
        .map_err(|e| ContentError::Io {
            path: post_dir.to_owned(),
            source: e,
        })?
        .filter_map(|e| e.ok())
        .collect();
    entries.sort_by_key(|e| e.file_name());

    let mut groups: Vec<PhotoGroup> = Vec::new();
    let mut flat_media: Vec<MediaItem> = Vec::new();

    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            let folder_name = entry.file_name().to_string_lossy().into_owned();
            let media = collect_media(&path, cache_dir)?;
            let (title_override, body_html) = parse_section(&path).unwrap_or((None, None));
            let name = title_override.unwrap_or(folder_name);
            if !media.is_empty() || body_html.is_some() {
                groups.push(PhotoGroup { name, body_html, media });
            }
        } else if is_nsfw(&path) {
            info!(path = %path.display(), "skipping nsfw media file");
        } else if is_video(&path) && is_web_optimized(&path) {
            info!(path = %path.display(), "ingesting web-optimized video");
            flat_media.push(MediaItem { path, is_video: true, exif: None, dimensions: None });
        } else if is_photo(&path) {
            let exif = read_exif(&path, cache_dir);
            let dimensions = read_dimensions(&path);
            flat_media.push(MediaItem { path, is_video: false, exif, dimensions });
        }
    }

    if !flat_media.is_empty() {
        flat_media.sort_by(|a, b| a.path.cmp(&b.path));
        groups.insert(0, PhotoGroup {
            name: String::new(),
            body_html: None,
            media: flat_media,
        });
    }

    Ok(groups)
}

pub(crate) fn parse_post(post_dir: &Path, cache_dir: &Path) -> Result<Post, ContentError> {
    let index_path = post_dir.join("index.md");
    let content = std::fs::read_to_string(&index_path).map_err(|e| ContentError::Io {
        path: index_path.clone(),
        source: e,
    })?;

    let (yaml, body) = split_frontmatter(&content, &index_path)?;
    let fm: Frontmatter =
        serde_yaml::from_str(yaml).map_err(|e| ContentError::InvalidFrontmatter {
            path: index_path.clone(),
            source: e,
        })?;

    let date = match &fm.date {
        serde_yaml::Value::String(s) => s.clone(),
        other => format!("{other:?}"),
    };

    let dir_name = post_dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let slug = slug_from_dir_name(&dir_name);
    let body_html = render_markdown(body);
    let photo_groups = discover_photo_groups(post_dir, cache_dir)?;
    let cover = fm.cover.map(|c| {
        // Search discovered media for a file whose name matches `c`, so the
        // frontmatter value only needs to be the filename regardless of which
        // subfolder it lives in.
        match photo_groups
            .iter()
            .flat_map(|g| g.media.iter())
            .find(|item| item.path.file_name().is_some_and(|n| n == c.as_str()))
        {
            Some(item) => item.path.clone(),
            None => {
                warn!(post = %slug, cover = %c, "cover photo not found among discovered media");
                post_dir.join(&c)
            }
        }
    });

    Ok(Post {
        slug,
        title: fm.title,
        date,
        access: fm.access,
        cover,
        body_html,
        photo_groups,
        source_dir: post_dir.to_owned(),
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn load_site(posts_dir: &Path, cache_dir: &Path) -> Result<Site, ContentError> {
        let mut entries: Vec<_> = std::fs::read_dir(posts_dir)
            .map_err(|e| ContentError::Io { path: posts_dir.to_owned(), source: e })?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .collect();
        entries.sort_by_key(|e| e.file_name());
        let mut posts = Vec::with_capacity(entries.len());
        for entry in entries {
            posts.push(parse_post(&entry.path(), cache_dir)?);
        }
        posts.sort_by(|a, b| a.date.cmp(&b.date));
        Ok(Site { posts })
    }

    fn make_post(tmp: &TempDir, dir_name: &str, frontmatter: &str, body: &str) -> PathBuf {
        let post_dir = tmp.path().join(dir_name);
        fs::create_dir_all(&post_dir).unwrap();
        fs::write(
            post_dir.join("index.md"),
            format!("---\n{frontmatter}\n---\n{body}"),
        )
        .unwrap();
        post_dir
    }

    fn make_photo(dir: &Path, name: &str) {
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join(name), b"JFIF").unwrap();
    }

    // ── Unit tests ────────────────────────────────────────────────────────────

    #[test]
    fn slug_strips_spaces_and_lowercases() {
        assert_eq!(slug_from_dir_name("2025-03-18 Hawaii"), "2025-03-18-hawaii");
        assert_eq!(slug_from_dir_name("My Trip!"), "my-trip");
        assert_eq!(slug_from_dir_name("  leading   spaces  "), "leading-spaces");
        assert_eq!(slug_from_dir_name("already-kebab"), "already-kebab");
    }

    #[test]
    fn is_photo_recognises_image_extensions() {
        assert!(is_photo(Path::new("foo.jpg")));
        assert!(is_photo(Path::new("foo.JPG")));
        assert!(is_photo(Path::new("foo.jpeg")));
        assert!(is_photo(Path::new("foo.png")));
        assert!(is_photo(Path::new("foo.webp")));
        assert!(!is_photo(Path::new("foo.txt")));
        assert!(!is_photo(Path::new("foo.md")));
        assert!(!is_photo(Path::new("foo")));
    }

    #[test]
    fn split_frontmatter_parses_valid_document() {
        let content = "---\ntitle: Hello\n---\nBody text\n";
        let (yaml, body) = split_frontmatter(content, Path::new("test.md")).unwrap();
        assert_eq!(yaml, "title: Hello");
        assert_eq!(body, "Body text\n");
    }

    #[test]
    fn split_frontmatter_errors_on_missing_delimiter() {
        let content = "No frontmatter here";
        assert!(matches!(
            split_frontmatter(content, Path::new("test.md")),
            Err(ContentError::MissingFrontmatter { .. })
        ));
    }

    #[test]
    fn split_frontmatter_errors_on_unclosed_block() {
        let content = "---\ntitle: Hello\nno closing delimiter";
        assert!(matches!(
            split_frontmatter(content, Path::new("test.md")),
            Err(ContentError::MissingFrontmatter { .. })
        ));
    }

    #[test]
    fn render_markdown_produces_html() {
        let html = render_markdown("# Hello\n\nParagraph with **bold**.");
        assert!(html.contains("<h1>"));
        assert!(html.contains("<strong>bold</strong>"));
    }

    // ── Integration tests ─────────────────────────────────────────────────────

    #[test]
    fn parse_post_published() {
        let tmp = TempDir::new().unwrap();
        make_post(
            &tmp,
            "2025-03-18 Hawaii",
            "title: Hawaii Trip\ndate: \"2025-03-18\"\naccess: [family, friends]",
            "## Day 1\n\nWe arrived.",
        );

        let post = parse_post(&tmp.path().join("2025-03-18 Hawaii"), tmp.path()).unwrap();

        assert_eq!(post.slug, "2025-03-18-hawaii");
        assert_eq!(post.title, "Hawaii Trip");
        assert_eq!(post.date, "2025-03-18");
        assert_eq!(post.access, ["family", "friends"]);
        assert!(!post.is_draft());
        assert!(post.body_html.contains("<h2>Day 1</h2>"));
    }

    #[test]
    fn parse_post_draft_when_access_absent() {
        let tmp = TempDir::new().unwrap();
        make_post(
            &tmp,
            "2025-05-01 Draft",
            "title: WIP\ndate: \"2025-05-01\"",
            "Work in progress.",
        );

        let post = parse_post(&tmp.path().join("2025-05-01 Draft"), tmp.path()).unwrap();

        assert!(post.is_draft());
        assert!(post.access.is_empty());
    }

    #[test]
    fn parse_post_discovers_subfolder_photo_groups() {
        let tmp = TempDir::new().unwrap();
        let post_dir = tmp.path().join("2025-03-18 Hawaii");
        make_post(
            &tmp,
            "2025-03-18 Hawaii",
            "title: Hawaii\ndate: \"2025-03-18\"",
            "",
        );
        make_photo(&post_dir.join("2025-03-18 Travel day"), "a.jpg");
        make_photo(&post_dir.join("2025-03-18 Travel day"), "b.jpg");
        make_photo(&post_dir.join("2025-03-19 Hiking"), "c.jpg");

        let post = parse_post(&post_dir, tmp.path()).unwrap();

        assert_eq!(post.photo_groups.len(), 2);
        assert_eq!(post.photo_groups[0].name, "2025-03-18 Travel day");
        assert_eq!(post.photo_groups[0].media.len(), 2);
        assert_eq!(post.photo_groups[1].name, "2025-03-19 Hiking");
        assert_eq!(post.photo_groups[1].media.len(), 1);
    }

    #[test]
    fn parse_post_discovers_flat_photos() {
        let tmp = TempDir::new().unwrap();
        let post_dir = tmp.path().join("2025-03-18 Hawaii");
        make_post(
            &tmp,
            "2025-03-18 Hawaii",
            "title: Hawaii\ndate: \"2025-03-18\"",
            "",
        );
        make_photo(&post_dir, "photo1.jpg");
        make_photo(&post_dir, "photo2.jpg");

        let post = parse_post(&post_dir, tmp.path()).unwrap();

        assert_eq!(post.photo_groups.len(), 1);
        assert_eq!(post.photo_groups[0].name, "");
        assert_eq!(post.photo_groups[0].media.len(), 2);
    }

    #[test]
    fn parse_post_section_index_md_provides_body_and_title() {
        let tmp = TempDir::new().unwrap();
        let post_dir = tmp.path().join("2025-03-18 Hawaii");
        make_post(
            &tmp,
            "2025-03-18 Hawaii",
            "title: Hawaii\ndate: \"2025-03-18\"",
            "",
        );
        let section_dir = post_dir.join("2025-03-18 Travel day");
        make_photo(&section_dir, "a.jpg");
        fs::write(
            section_dir.join("index.md"),
            "---\ntitle: Travel Day\n---\nWe flew in.\n",
        )
        .unwrap();

        let post = parse_post(&post_dir, tmp.path()).unwrap();

        assert_eq!(post.photo_groups.len(), 1);
        assert_eq!(post.photo_groups[0].name, "Travel Day");
        assert!(post.photo_groups[0].body_html.as_deref().unwrap_or("").contains("We flew in."));
    }

    #[test]
    fn parse_post_section_index_md_title_falls_back_to_folder_name() {
        let tmp = TempDir::new().unwrap();
        let post_dir = tmp.path().join("2025-03-18 Hawaii");
        make_post(
            &tmp,
            "2025-03-18 Hawaii",
            "title: Hawaii\ndate: \"2025-03-18\"",
            "",
        );
        let section_dir = post_dir.join("2025-03-18 Travel day");
        make_photo(&section_dir, "a.jpg");
        fs::write(
            section_dir.join("index.md"),
            "---\n---\nJust a body, no title.\n",
        )
        .unwrap();

        let post = parse_post(&post_dir, tmp.path()).unwrap();

        assert_eq!(post.photo_groups[0].name, "2025-03-18 Travel day");
        assert!(post.photo_groups[0].body_html.as_deref().unwrap_or("").contains("Just a body"));
    }

    #[test]
    fn parse_post_no_photos_dir() {
        let tmp = TempDir::new().unwrap();
        make_post(
            &tmp,
            "2025-03-18 Hawaii",
            "title: Hawaii\ndate: \"2025-03-18\"",
            "",
        );

        let post = parse_post(&tmp.path().join("2025-03-18 Hawaii"), tmp.path()).unwrap();

        assert!(post.photo_groups.is_empty());
    }

    #[test]
    fn load_site_sorts_posts_by_date() {
        let tmp = TempDir::new().unwrap();
        make_post(
            &tmp,
            "2025-06-01 Later",
            "title: Later\ndate: \"2025-06-01\"",
            "",
        );
        make_post(
            &tmp,
            "2025-01-01 Earlier",
            "title: Earlier\ndate: \"2025-01-01\"",
            "",
        );

        let site = load_site(tmp.path(), tmp.path()).unwrap();

        assert_eq!(site.posts.len(), 2);
        assert_eq!(site.posts[0].date, "2025-01-01");
        assert_eq!(site.posts[1].date, "2025-06-01");
    }

    #[test]
    fn nsfw_files_are_excluded_from_flat_and_subfolder_media() {
        let tmp = TempDir::new().unwrap();
        let post_dir = tmp.path().join("2025-03-18 Hawaii");
        make_post(
            &tmp,
            "2025-03-18 Hawaii",
            "title: Hawaii\ndate: \"2025-03-18\"",
            "",
        );
        // Flat layout: one safe photo, one nsfw photo
        make_photo(&post_dir, "safe.jpg");
        make_photo(&post_dir, "nsfw_private.jpg");
        // Subfolder layout: one safe photo, one nsfw photo
        make_photo(&post_dir.join("Day 1"), "safe2.jpg");
        make_photo(&post_dir.join("Day 1"), "day1-NSFW.jpg");

        let post = parse_post(&post_dir, tmp.path()).unwrap();

        let flat = post.photo_groups.iter().find(|g| g.name.is_empty()).unwrap();
        assert_eq!(flat.media.len(), 1);
        assert!(flat.media[0].path.file_name().unwrap() == "safe.jpg");

        let day1 = post.photo_groups.iter().find(|g| g.name == "Day 1").unwrap();
        assert_eq!(day1.media.len(), 1);
        assert!(day1.media[0].path.file_name().unwrap() == "safe2.jpg");
    }

    #[test]
    fn load_site_empty_directory() {
        let tmp = TempDir::new().unwrap();
        let site = load_site(tmp.path(), tmp.path()).unwrap();
        assert!(site.posts.is_empty());
    }
}
