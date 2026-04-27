//! Theme module: loads MiniJinja templates and renders HTML.
//!
//! The theme module is pure — it takes the `Site` model and a `Viewer` and
//! returns rendered HTML strings. It has no knowledge of HTTP or sessions.

use std::path::Path;

use atom_syndication::{Content, Entry, Feed, Link, Text};
use chrono::{DateTime, FixedOffset, NaiveDate, NaiveTime};
use minijinja::{Environment, context, path_loader};
use serde::Serialize;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::content::{MediaItem, Post, Site};
use crate::viewer::{Viewer, visible};

// ── Errors ────────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum ThemeError {
    #[error("could not load template '{name}'")]
    Load {
        name: &'static str,
        #[source]
        source: minijinja::Error,
    },
    #[error("could not render template '{name}'")]
    Render {
        name: &'static str,
        #[source]
        source: minijinja::Error,
    },
}

// ── Theme ─────────────────────────────────────────────────────────────────────

pub struct Theme {
    env: Environment<'static>,
    pub site_title: String,
    style_version: String,
}

impl Theme {
    /// Load a theme from `theme_dir`.
    ///
    /// Templates are read from `{theme_dir}/templates/` on demand. The function
    /// itself does not fail even if the directory is absent; template errors will
    /// surface at render time.
    pub fn load(theme_dir: &Path, site_title: String) -> Self {
        let templates_dir = theme_dir.join("templates");
        let mut env = Environment::new();
        env.set_loader(path_loader(templates_dir));
        let style_version = hash_file(&theme_dir.join("static").join("style.css"));
        Self { env, site_title, style_version }
    }

    /// Render the post-listing index page.
    ///
    /// # Errors
    ///
    /// Returns [`ThemeError`] if the template cannot be loaded or rendered.
    pub fn render_index(&self, site: &Site, viewer: &Viewer) -> Result<String, ThemeError> {
        let tmpl = self
            .env
            .get_template("index.html")
            .map_err(|e| ThemeError::Load {
                name: "index.html",
                source: e,
            })?;

        let mut posts: Vec<PostSummaryCtx> = visible(site, viewer)
            .map(PostSummaryCtx::from_post)
            .collect();
        posts.reverse();

        tmpl.render(context! { posts, is_admin => viewer.is_admin(), logged_in => viewer.logged_in, username => &viewer.username, site_title => &self.site_title, style_version => &self.style_version })
            .map_err(|e| ThemeError::Render {
                name: "index.html",
                source: e,
            })
    }

    /// Render a single post page.
    ///
    /// The caller is responsible for verifying `viewer` has access to `post`
    /// before calling this function.
    ///
    /// # Errors
    ///
    /// Returns [`ThemeError`] if the template cannot be loaded or rendered.
    pub fn render_post(&self, post: &Post, viewer: &Viewer) -> Result<String, ThemeError> {
        let tmpl = self
            .env
            .get_template("post.html")
            .map_err(|e| ThemeError::Load {
                name: "post.html",
                source: e,
            })?;

        let ctx = PostDetailCtx::from_post(post);
        tmpl.render(context! { post => ctx, is_admin => viewer.is_admin(), logged_in => viewer.logged_in, username => &viewer.username, site_title => &self.site_title, style_version => &self.style_version })
            .map_err(|e| ThemeError::Render {
                name: "post.html",
                source: e,
            })
    }

    /// Render the 404 not-found page.
    ///
    /// # Errors
    ///
    /// Returns [`ThemeError`] if the template cannot be loaded or rendered.
    pub fn render_not_found(&self, viewer: &Viewer) -> Result<String, ThemeError> {
        let tmpl = self
            .env
            .get_template("404.html")
            .map_err(|e| ThemeError::Load { name: "404.html", source: e })?;

        tmpl.render(context! { is_admin => viewer.is_admin(), logged_in => viewer.logged_in, username => &viewer.username, site_title => &self.site_title, style_version => &self.style_version })
            .map_err(|e| ThemeError::Render { name: "404.html", source: e })
    }

    /// Render the login page.
    ///
    /// `error` is an optional message shown when a previous attempt failed
    /// (e.g. "Invalid username or password").
    ///
    /// # Errors
    ///
    /// Returns [`ThemeError`] if the template cannot be loaded or rendered.
    pub fn render_login(&self, error: Option<&str>, next: Option<&str>) -> Result<String, ThemeError> {
        let tmpl = self
            .env
            .get_template("login.html")
            .map_err(|e| ThemeError::Load {
                name: "login.html",
                source: e,
            })?;

        tmpl.render(context! { error, next, site_title => &self.site_title, style_version => &self.style_version })
            .map_err(|e| ThemeError::Render {
                name: "login.html",
                source: e,
            })
    }
}

// ── Feed rendering ────────────────────────────────────────────────────────────

/// Render a personalised Atom feed for `viewer`.
///
/// `base_url` must be the scheme+host with no trailing slash
/// (e.g. `"https://example.com"`). It is embedded in every absolute URL inside
/// the feed so that feed readers can load images without a session cookie.
///
/// `token` is the raw (unhashed) feed token for this viewer; it is appended
/// as `?t=<token>` to every image URL so the media route can authenticate the
/// request without a cookie.
pub fn render_feed(site: &Site, viewer: &Viewer, base_url: &str, token: &str, site_title: &str) -> String {
    let entries: Vec<Entry> = visible(site, viewer)
        .map(|post| feed_entry(post, base_url, token))
        .collect();

    let updated = entries
        .first()
        .map(|e| e.updated)
        .unwrap_or_else(fallback_date);

    let feed = Feed {
        title: Text::plain(site_title),
        id: format!("{base_url}/"),
        updated,
        links: vec![
            Link {
                href: format!("{base_url}/"),
                rel: "alternate".into(),
                ..Default::default()
            },
            Link {
                href: format!("{base_url}/feed/{token}.xml"),
                rel: "self".into(),
                ..Default::default()
            },
        ],
        entries,
        ..Default::default()
    };

    feed.to_string()
}

fn feed_entry(post: &Post, base_url: &str, token: &str) -> Entry {
    let post_url = format!("{base_url}/posts/{}", post.slug);
    let updated = parse_post_date(&post.date);
    let content_html = entry_content_html(post, base_url, token);

    Entry {
        title: Text::plain(post.title.as_str()),
        id: post_url.clone(),
        updated,
        links: vec![Link {
            href: post_url,
            rel: "alternate".into(),
            ..Default::default()
        }],
        content: Some(Content {
            content_type: Some("html".into()),
            value: Some(content_html),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn entry_content_html(post: &Post, base_url: &str, token: &str) -> String {
    let mut html = String::new();

    if !post.body_html.is_empty() {
        html.push_str(&post.body_html);
    }

    for group in &post.photo_groups {
        if !group.name.is_empty() {
            html.push_str(&format!("<h2>{}</h2>\n", group.name));
        }
        if let Some(body) = &group.body_html {
            html.push_str(body);
        }
        for item in &group.media {
            if item.is_video {
                continue;
            }
            let rel = item.path.strip_prefix(&post.source_dir).unwrap_or(&item.path);
            let url = format!("{base_url}/media/{}/{}", post.slug, rel.display());
            html.push_str(&format!(
                "<img src=\"{url}?size=medium&amp;t={token}\" style=\"max-width:100%;display:block\">\n"
            ));
        }
    }

    html
}

fn parse_post_date(date_str: &str) -> DateTime<FixedOffset> {
    let naive = NaiveDate::parse_from_str(date_str, "%Y-%m-%d")
        .unwrap_or_else(|_| NaiveDate::from_ymd_opt(2000, 1, 1).expect("valid fallback date"))
        .and_time(NaiveTime::from_hms_opt(0, 0, 0).expect("valid time"));
    naive.and_utc().fixed_offset()
}

fn fallback_date() -> DateTime<FixedOffset> {
    NaiveDate::from_ymd_opt(2000, 1, 1)
        .expect("valid fallback date")
        .and_time(NaiveTime::from_hms_opt(0, 0, 0).expect("valid time"))
        .and_utc()
        .fixed_offset()
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn hash_file(path: &Path) -> String {
    let bytes = std::fs::read(path).unwrap_or_default();
    let digest = Sha256::digest(&bytes);
    format!("{digest:x}")[..8].to_owned()
}

// ── View models ───────────────────────────────────────────────────────────────
//
// These structs translate the domain model into template-friendly values. Paths
// become URL strings, counts are computed here so templates stay logic-free.

/// Strips leading words from `lens` that already appear (case-insensitively) in `camera`,
/// then returns `"camera · remaining_lens"` (or just `camera` if nothing remains).
fn combine_camera_lens(camera: &str, lens: &str) -> String {
    let camera_words: std::collections::HashSet<String> =
        camera.split_whitespace().map(|w| w.to_lowercase()).collect();
    let lens_words: Vec<&str> = lens.split_whitespace().collect();
    let skip = lens_words
        .iter()
        .take_while(|w| camera_words.contains(&w.to_lowercase()))
        .count();
    let remainder = lens_words[skip..].join(" ");
    if remainder.is_empty() {
        camera.to_owned()
    } else {
        format!("{camera} · {remainder}")
    }
}

fn media_url(slug: &str, source_dir: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(source_dir).unwrap_or(path);
    format!("/media/{}/{}", slug, rel.display())
}

#[derive(Debug, Clone, Serialize)]
struct MediaCtx {
    url: String,
    /// `?size=thumb` derivative URL; empty for videos.
    thumb: String,
    /// `?size=medium` derivative URL; empty for videos.
    medium: String,
    is_video: bool,
    /// Aspect ratio (width/height) used for CSS `flex-grow` and `aspect-ratio`.
    /// Defaults to 4/3 when dimensions are unknown; 1.0 for videos (unused).
    flex_grow: f64,
    /// Focal length · aperture · shutter · ISO, e.g. `"50mm · f/2.8 · 1/250s · ISO 400"`.
    exif_tech: Option<String>,
    /// Camera + deduplicated lens on one line, e.g. `"Nikon Z6_3 · Nikkor Z 50mm f/1.8 S"`.
    exif_camera_lens: Option<String>,
    /// Formatted capture datetime, e.g. `"2025-03-18 14:32"`.
    exif_datetime: Option<String>,
}

impl MediaCtx {
    fn from_item(slug: &str, source_dir: &Path, item: &MediaItem) -> Self {
        let url = media_url(slug, source_dir, &item.path);
        if item.is_video {
            Self {
                url,
                thumb: String::new(),
                medium: String::new(),
                is_video: true,
                flex_grow: 1.0,
                exif_tech: None,
                exif_camera_lens: None,
                exif_datetime: None,
            }
        } else {
            let thumb = format!("{url}?size=thumb");
            let medium = format!("{url}?size=medium");
            let flex_grow = item.dimensions
                .map(|(w, h)| w as f64 / h as f64)
                .unwrap_or(4.0 / 3.0);
            let exif = item.exif.as_ref();
            let exif_tech = exif.and_then(|e| {
                let parts: Vec<&str> = [
                    e.focal_length.as_deref(),
                    e.aperture.as_deref(),
                    e.shutter.as_deref(),
                    e.iso.as_deref(),
                ]
                .into_iter()
                .flatten()
                .collect();
                if parts.is_empty() { None } else { Some(parts.join(" · ")) }
            });
            let exif_camera_lens = match (
                exif.and_then(|e| e.camera.as_deref()),
                exif.and_then(|e| e.lens.as_deref()),
            ) {
                (Some(cam), Some(lens)) => Some(combine_camera_lens(cam, lens)),
                (Some(cam), None) => Some(cam.to_owned()),
                (None, Some(lens)) => Some(lens.to_owned()),
                (None, None) => None,
            };
            let exif_datetime = exif.and_then(|e| e.datetime.clone());
            Self { url, thumb, medium, is_video: false, flex_grow, exif_tech, exif_camera_lens, exif_datetime }
        }
    }

    fn from_photo_path(slug: &str, source_dir: &Path, path: &Path) -> Self {
        let url = media_url(slug, source_dir, path);
        let thumb = format!("{url}?size=thumb");
        let medium = format!("{url}?size=medium");
        Self {
            url,
            thumb,
            medium,
            is_video: false,
            flex_grow: 4.0 / 3.0,
            exif_tech: None,
            exif_camera_lens: None,
            exif_datetime: None,
        }
    }
}

#[derive(Debug, Serialize)]
struct PostSummaryCtx {
    slug: String,
    title: String,
    date: String,
    is_draft: bool,
    cover: Option<MediaCtx>,
    photo_count: usize,
    /// Up to 3 non-video photos used for the collage preview when no cover is set.
    preview_photos: Vec<MediaCtx>,
}

impl PostSummaryCtx {
    fn from_post(post: &Post) -> Self {
        let photo_count = post.photo_groups.iter().map(|g| g.media.len()).sum();
        let cover = post
            .cover
            .as_deref()
            .map(|p| MediaCtx::from_photo_path(&post.slug, &post.source_dir, p));
        let preview_photos: Vec<MediaCtx> = post
            .photo_groups
            .iter()
            .flat_map(|g| g.media.iter())
            .filter(|item| !item.is_video)
            .take(3)
            .map(|item| MediaCtx::from_item(&post.slug, &post.source_dir, item))
            .collect();
        Self {
            slug: post.slug.clone(),
            title: post.title.clone(),
            date: post.date.clone(),
            is_draft: post.is_draft(),
            cover,
            photo_count,
            preview_photos,
        }
    }
}

/// Container width and target row height used only for deciding which images share a row.
/// CSS flex handles the actual scaling, so these are approximate.
const GALLERY_CONTAINER_W: f64 = 1160.0;
const GALLERY_TARGET_H: f64 = 280.0;
const GALLERY_AR_THRESHOLD: f64 = GALLERY_CONTAINER_W / GALLERY_TARGET_H;

#[derive(Debug, Serialize)]
struct GalleryRowCtx {
    media: Vec<MediaCtx>,
    /// True for the final partial row; a spacer absorbs leftover space so items stay at natural size.
    is_last_unjustified: bool,
    /// True when this row holds a single video (height: auto, not the fixed gallery row height).
    is_video_row: bool,
}

/// Packs a flat list of media items into justified gallery rows.
///
/// Each row's items share a CSS `flex-grow` proportional to their aspect ratio, producing
/// justified rows that reflow at any viewport width without JavaScript.
fn pack_into_rows(media: Vec<MediaCtx>) -> Vec<GalleryRowCtx> {
    let mut rows: Vec<GalleryRowCtx> = Vec::new();
    let mut current: Vec<MediaCtx> = Vec::new();
    let mut ar_sum: f64 = 0.0;

    for item in media {
        if item.is_video {
            if !current.is_empty() {
                rows.push(GalleryRowCtx { media: current, is_last_unjustified: false, is_video_row: false });
                current = Vec::new();
                ar_sum = 0.0;
            }
            rows.push(GalleryRowCtx { media: vec![item], is_last_unjustified: false, is_video_row: true });
        } else {
            let ar = item.flex_grow;
            if !current.is_empty() && ar_sum + ar > GALLERY_AR_THRESHOLD {
                rows.push(GalleryRowCtx { media: current, is_last_unjustified: false, is_video_row: false });
                current = Vec::new();
                ar_sum = 0.0;
            }
            ar_sum += ar;
            current.push(item);
        }
    }

    if !current.is_empty() {
        let is_sparse = current.len() < 3 && ar_sum < GALLERY_AR_THRESHOLD * 0.5;
        if is_sparse
            && let Some(prev) = rows.last_mut().filter(|r| !r.is_video_row)
        {
            prev.media.extend(current);
            prev.is_last_unjustified = true;
            return rows;
        }
        rows.push(GalleryRowCtx { media: current, is_last_unjustified: true, is_video_row: false });
    }

    rows
}

#[derive(Debug, Serialize)]
struct PhotoGroupCtx {
    name: String,
    body_html: Option<String>,
    rows: Vec<GalleryRowCtx>,
}

#[derive(Debug, Serialize)]
struct PostDetailCtx {
    slug: String,
    title: String,
    date: String,
    is_draft: bool,
    body_html: String,
    cover: Option<MediaCtx>,
    photo_groups: Vec<PhotoGroupCtx>,
}

impl PostDetailCtx {
    fn from_post(post: &Post) -> Self {
        let cover = post
            .cover
            .as_deref()
            .map(|p| MediaCtx::from_photo_path(&post.slug, &post.source_dir, p));
        let photo_groups = post
            .photo_groups
            .iter()
            .map(|group| {
                let media = group
                    .media
                    .iter()
                    .map(|item| MediaCtx::from_item(&post.slug, &post.source_dir, item))
                    .collect();
                PhotoGroupCtx {
                    name: group.name.clone(),
                    body_html: group.body_html.clone(),
                    rows: pack_into_rows(media),
                }
            })
            .collect();

        Self {
            slug: post.slug.clone(),
            title: post.title.clone(),
            date: post.date.clone(),
            is_draft: post.is_draft(),
            body_html: post.body_html.clone(),
            cover,
            photo_groups,
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::{MediaItem, PhotoGroup};
    use std::path::PathBuf;

    const THEME_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/themes/default");

    fn make_post(slug: &str, title: &str, access: Vec<&str>, body_md: &str) -> Post {
        Post {
            slug: slug.into(),
            title: title.into(),
            date: "2025-01-01".into(),
            access: access.into_iter().map(str::to_owned).collect(),
            cover: None,
            body_html: format!("<p>{body_md}</p>"),
            photo_groups: vec![],
            source_dir: PathBuf::from("/posts").join(slug),
        }
    }

    fn make_post_with_photos(slug: &str) -> Post {
        let source_dir = PathBuf::from("/posts").join(slug);
        let day1_dir = source_dir.join("Day 1");
        Post {
            slug: slug.into(),
            title: "With Photos".into(),
            date: "2025-02-01".into(),
            access: vec!["family".into()],
            cover: None,
            body_html: String::new(),
            photo_groups: vec![PhotoGroup {
                name: "Day 1".into(),
                body_html: None,
                media: vec![
                    MediaItem { path: day1_dir.join("a.jpg"), is_video: false, exif: None, dimensions: None },
                    MediaItem { path: day1_dir.join("b.jpg"), is_video: false, exif: None, dimensions: None },
                ],
            }],
            source_dir,
        }
    }

    fn load_theme() -> Theme {
        Theme::load(Path::new(THEME_DIR), "Glimpse".to_owned())
    }

    #[test]
    fn render_index_contains_post_title() {
        let theme = load_theme();
        let site = Site {
            posts: vec![make_post(
                "hawaii",
                "Hawaii Trip",
                vec!["family"],
                "Great time!",
            )],
        };
        let viewer = Viewer::with_groups(["family"]);

        let html = theme.render_index(&site, &viewer).unwrap();

        assert!(html.contains("Hawaii Trip"), "index should list post title");
        assert!(html.contains("/posts/hawaii"), "index should link to post");
    }

    #[test]
    fn render_index_hides_draft_from_regular_viewer() {
        let theme = load_theme();
        let site = Site {
            posts: vec![
                make_post("published", "Published", vec!["family"], ""),
                make_post("draft-post", "Secret Draft", vec![], ""),
            ],
        };
        let viewer = Viewer::with_groups(["family"]);

        let html = theme.render_index(&site, &viewer).unwrap();

        assert!(html.contains("Published"));
        assert!(
            !html.contains("Secret Draft"),
            "draft should not appear for non-admin"
        );
    }

    #[test]
    fn render_index_shows_draft_badge_for_admin() {
        let theme = load_theme();
        let site = Site {
            posts: vec![make_post("draft-post", "Secret Draft", vec![], "")],
        };

        let html = theme.render_index(&site, &Viewer::admin()).unwrap();

        assert!(html.contains("Secret Draft"), "admin should see draft");
        assert!(
            html.to_uppercase().contains("DRAFT"),
            "admin should see DRAFT badge"
        );
    }

    #[test]
    fn render_post_contains_body_and_title() {
        let theme = load_theme();
        let post = make_post(
            "hawaii",
            "Hawaii Trip",
            vec!["family"],
            "We arrived safely.",
        );

        let html = theme
            .render_post(&post, &Viewer::with_groups(["family"]))
            .unwrap();

        assert!(html.contains("Hawaii Trip"));
        assert!(html.contains("We arrived safely."));
    }

    #[test]
    fn render_post_shows_draft_banner_for_admin() {
        let theme = load_theme();
        let post = make_post("draft-post", "WIP Post", vec![], "Not ready.");

        let html = theme.render_post(&post, &Viewer::admin()).unwrap();

        assert!(
            html.to_uppercase().contains("DRAFT"),
            "draft banner should be visible to admin"
        );
    }

    #[test]
    fn render_post_photo_urls_use_media_prefix() {
        let theme = load_theme();
        let post = make_post_with_photos("trip");

        let html = theme
            .render_post(&post, &Viewer::with_groups(["family"]))
            .unwrap();

        // MiniJinja HTML-encodes '/' as '&#x2f;' (OWASP recommendation).
        // Decode before asserting so the test checks logical URL content.
        let decoded = html.replace("&#x2f;", "/");
        assert!(
            decoded.contains("/media/trip/"),
            "photo URLs should start with /media/slug/"
        );
        assert!(decoded.contains("a.jpg"));
    }

    #[test]
    fn render_login_produces_form() {
        let theme = load_theme();
        let html = theme.render_login(None, None).unwrap();
        assert!(
            html.contains("<form"),
            "login page should have a form element"
        );
    }

    #[test]
    fn render_login_with_error_shows_message() {
        let theme = load_theme();
        let html = theme
            .render_login(Some("Invalid username or password"), None)
            .unwrap();
        assert!(html.contains("Invalid username or password"));
    }

    #[test]
    fn media_url_flat_photo() {
        let source = PathBuf::from("/posts/hawaii");
        let photo = source.join("img.jpg");
        assert_eq!(
            media_url("hawaii", &source, &photo),
            "/media/hawaii/img.jpg"
        );
    }

    #[test]
    fn media_url_subfolder_photo() {
        let source = PathBuf::from("/posts/hawaii");
        let photo = source.join("Day 1").join("img.jpg");
        assert_eq!(
            media_url("hawaii", &source, &photo),
            "/media/hawaii/Day 1/img.jpg"
        );
    }

    #[test]
    fn render_feed_contains_post_title_and_link() {
        let site = Site {
            posts: vec![make_post(
                "hawaii",
                "Hawaii Trip",
                vec!["family"],
                "Great time!",
            )],
        };
        let viewer = Viewer::with_groups(["family"]);

        let xml = render_feed(&site, &viewer, "https://example.com", "mytoken", "Glimpse");

        assert!(
            xml.contains("Hawaii Trip"),
            "feed should contain post title"
        );
        assert!(
            xml.contains("https://example.com/posts/hawaii"),
            "feed should link to post"
        );
    }

    #[test]
    fn render_feed_excludes_inaccessible_posts() {
        let site = Site {
            posts: vec![
                make_post("visible", "Visible", vec!["family"], ""),
                make_post("draft", "Draft Post", vec![], ""),
            ],
        };
        let viewer = Viewer::with_groups(["family"]);

        let xml = render_feed(&site, &viewer, "https://example.com", "tok", "Glimpse");

        assert!(xml.contains("Visible"));
        assert!(
            !xml.contains("Draft Post"),
            "draft should not appear in feed"
        );
    }

    #[test]
    fn render_feed_image_urls_include_token() {
        let post = make_post_with_photos("trip");
        let site = Site { posts: vec![post] };
        let viewer = Viewer::with_groups(["family"]);

        let xml = render_feed(&site, &viewer, "https://example.com", "tok123", "Glimpse");

        assert!(
            xml.contains("t=tok123"),
            "image URLs should carry the feed token"
        );
        assert!(
            xml.contains("https://example.com/media/trip/"),
            "image URLs should be absolute"
        );
    }

    #[test]
    fn render_feed_self_link_contains_token() {
        let site = Site { posts: vec![] };
        let xml = render_feed(&site, &Viewer::public(), "https://example.com", "mytoken", "Glimpse");
        assert!(xml.contains("mytoken.xml"), "self link should embed token");
    }
}
