//! Content model: scans `posts/`, parses frontmatter + markdown, discovers photos.
//!
//! Pure module — no HTTP, no HTML templating.

use std::path::{Path, PathBuf};

use pulldown_cmark::{Options, Parser, html};
use serde::Deserialize;
use thiserror::Error;

// ── Errors ────────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum ContentError {
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

/// A single media item (photo or video) within a post.
#[derive(Debug, Clone)]
pub struct MediaItem {
    pub path: PathBuf,
    pub is_video: bool,
}

/// A group of media from one subfolder (subfolder name becomes a section heading).
#[derive(Debug, Clone)]
pub struct PhotoGroup {
    /// Subfolder name; empty string when media is flat under `photos/`.
    pub name: String,
    pub media: Vec<MediaItem>,
}

/// A single post parsed from a `posts/` subfolder.
#[derive(Debug, Clone)]
pub struct Post {
    /// URL-safe identifier derived from the folder name.
    pub slug: String,
    pub title: String,
    /// ISO 8601 date string (YYYY-MM-DD).
    pub date: String,
    /// Groups allowed to view this post. Empty = draft (admin-only).
    pub access: Vec<String>,
    pub cover: Option<PathBuf>,
    /// Markdown body pre-rendered to HTML at load time.
    pub body_html: String,
    pub photo_groups: Vec<PhotoGroup>,
    pub source_dir: PathBuf,
}

impl Post {
    pub fn is_draft(&self) -> bool {
        self.access.is_empty()
    }
}

/// The full in-memory site model.
pub struct Site {
    /// Posts sorted ascending by date.
    pub posts: Vec<Post>,
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

fn collect_media(dir: &Path) -> Result<Vec<MediaItem>, ContentError> {
    let entries = std::fs::read_dir(dir).map_err(|e| ContentError::Io {
        path: dir.to_owned(),
        source: e,
    })?;
    let mut items: Vec<MediaItem> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| is_photo(p) || is_video(p))
        .map(|p| {
            let is_video = is_video(&p);
            MediaItem { path: p, is_video }
        })
        .collect();
    items.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(items)
}

fn discover_photo_groups(post_dir: &Path) -> Result<Vec<PhotoGroup>, ContentError> {
    let photos_dir = post_dir.join("photos");
    if !photos_dir.exists() {
        return Ok(Vec::new());
    }

    let mut entries: Vec<_> = std::fs::read_dir(&photos_dir)
        .map_err(|e| ContentError::Io {
            path: photos_dir.clone(),
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
            let name = entry.file_name().to_string_lossy().into_owned();
            let media = collect_media(&path)?;
            if !media.is_empty() {
                groups.push(PhotoGroup { name, media });
            }
        } else if is_photo(&path) || is_video(&path) {
            let is_video = is_video(&path);
            flat_media.push(MediaItem { path, is_video });
        }
    }

    // Flat layout: media directly under photos/ with no subfolders.
    if groups.is_empty() && !flat_media.is_empty() {
        flat_media.sort_by(|a, b| a.path.cmp(&b.path));
        groups.push(PhotoGroup {
            name: String::new(),
            media: flat_media,
        });
    }

    Ok(groups)
}

pub fn parse_post(post_dir: &Path) -> Result<Post, ContentError> {
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
    let cover = fm.cover.map(|c| post_dir.join(c));
    let photo_groups = discover_photo_groups(post_dir)?;

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

// ── Public API ────────────────────────────────────────────────────────────────

/// Scan `posts_dir` and return a fully-parsed [`Site`].
///
/// Posts are sorted ascending by date string (ISO 8601 sorts lexicographically).
///
/// # Errors
///
/// Returns [`ContentError`] if any post directory cannot be read or parsed.
pub fn load_site(posts_dir: &Path) -> Result<Site, ContentError> {
    let mut entries: Vec<_> = std::fs::read_dir(posts_dir)
        .map_err(|e| ContentError::Io {
            path: posts_dir.to_owned(),
            source: e,
        })?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .collect();
    entries.sort_by_key(|e| e.file_name());

    let mut posts = Vec::with_capacity(entries.len());
    for entry in entries {
        posts.push(parse_post(&entry.path())?);
    }
    posts.sort_by(|a, b| a.date.cmp(&b.date));

    Ok(Site { posts })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    // ── Helpers ───────────────────────────────────────────────────────────────

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

        let post = parse_post(&tmp.path().join("2025-03-18 Hawaii")).unwrap();

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

        let post = parse_post(&tmp.path().join("2025-05-01 Draft")).unwrap();

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
        make_photo(
            &post_dir.join("photos").join("2025-03-18 Travel day"),
            "a.jpg",
        );
        make_photo(
            &post_dir.join("photos").join("2025-03-18 Travel day"),
            "b.jpg",
        );
        make_photo(&post_dir.join("photos").join("2025-03-19 Hiking"), "c.jpg");

        let post = parse_post(&post_dir).unwrap();

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
        make_photo(&post_dir.join("photos"), "photo1.jpg");
        make_photo(&post_dir.join("photos"), "photo2.jpg");

        let post = parse_post(&post_dir).unwrap();

        assert_eq!(post.photo_groups.len(), 1);
        assert_eq!(post.photo_groups[0].name, "");
        assert_eq!(post.photo_groups[0].media.len(), 2);
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

        let post = parse_post(&tmp.path().join("2025-03-18 Hawaii")).unwrap();

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

        let site = load_site(tmp.path()).unwrap();

        assert_eq!(site.posts.len(), 2);
        assert_eq!(site.posts[0].date, "2025-01-01");
        assert_eq!(site.posts[1].date, "2025-06-01");
    }

    #[test]
    fn load_site_empty_directory() {
        let tmp = TempDir::new().unwrap();
        let site = load_site(tmp.path()).unwrap();
        assert!(site.posts.is_empty());
    }
}
