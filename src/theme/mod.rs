//! Theme module: loads MiniJinja templates and renders HTML.
//!
//! The theme module is pure — it takes the `Site` model and a `Viewer` and
//! returns rendered HTML strings. It has no knowledge of HTTP or sessions.

use std::path::Path;

use minijinja::{Environment, context, path_loader};
use serde::Serialize;
use thiserror::Error;

use crate::content::{Post, Site};
use crate::viewer::{Viewer, visible};

// ── Errors ────────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum ThemeError {
    #[error("could not load template '{name}'")]
    Load { name: &'static str, #[source] source: minijinja::Error },
    #[error("could not render template '{name}'")]
    Render { name: &'static str, #[source] source: minijinja::Error },
}

// ── Theme ─────────────────────────────────────────────────────────────────────

pub struct Theme {
    env: Environment<'static>,
}

impl Theme {
    /// Load a theme from `theme_dir`.
    ///
    /// Templates are read from `{theme_dir}/templates/` on demand. The function
    /// itself does not fail even if the directory is absent; template errors will
    /// surface at render time.
    pub fn load(theme_dir: &Path) -> Self {
        let templates_dir = theme_dir.join("templates");
        let mut env = Environment::new();
        env.set_loader(path_loader(templates_dir));
        Self { env }
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
            .map_err(|e| ThemeError::Load { name: "index.html", source: e })?;

        let posts: Vec<PostSummaryCtx> =
            visible(site, viewer).map(PostSummaryCtx::from_post).collect();

        tmpl.render(context! { posts, is_admin => viewer.is_admin() })
            .map_err(|e| ThemeError::Render { name: "index.html", source: e })
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
            .map_err(|e| ThemeError::Load { name: "post.html", source: e })?;

        let ctx = PostDetailCtx::from_post(post);
        tmpl.render(context! { post => ctx, is_admin => viewer.is_admin() })
            .map_err(|e| ThemeError::Render { name: "post.html", source: e })
    }

    /// Render the login page.
    ///
    /// `error` is an optional message shown when a previous attempt failed
    /// (e.g. "Invalid username or password").
    ///
    /// # Errors
    ///
    /// Returns [`ThemeError`] if the template cannot be loaded or rendered.
    pub fn render_login(&self, error: Option<&str>) -> Result<String, ThemeError> {
        let tmpl = self
            .env
            .get_template("login.html")
            .map_err(|e| ThemeError::Load { name: "login.html", source: e })?;

        tmpl.render(context! { error })
            .map_err(|e| ThemeError::Render { name: "login.html", source: e })
    }
}

// ── View models ───────────────────────────────────────────────────────────────
//
// These structs translate the domain model into template-friendly values. Paths
// become URL strings, counts are computed here so templates stay logic-free.

/// Compute the media URL for a photo given its absolute path and the post slug.
///
/// The path is made relative to `{source_dir}/photos/`, then prefixed with
/// `/media/{slug}/`. Subdirectory structure is preserved.
fn photo_url(slug: &str, source_dir: &Path, photo: &Path) -> String {
    let photos_dir = source_dir.join("photos");
    let rel = photo.strip_prefix(&photos_dir).unwrap_or(photo);
    format!("/media/{}/{}", slug, rel.display())
}

#[derive(Debug, Serialize)]
struct PostSummaryCtx {
    slug: String,
    title: String,
    date: String,
    is_draft: bool,
    cover: Option<String>,
    photo_count: usize,
}

impl PostSummaryCtx {
    fn from_post(post: &Post) -> Self {
        let photo_count = post.photo_groups.iter().map(|g| g.photos.len()).sum();
        let cover = post.cover.as_deref().map(|p| photo_url(&post.slug, &post.source_dir, p));
        Self {
            slug: post.slug.clone(),
            title: post.title.clone(),
            date: post.date.clone(),
            is_draft: post.is_draft(),
            cover,
            photo_count,
        }
    }
}

#[derive(Debug, Serialize)]
struct PhotoGroupCtx {
    name: String,
    photos: Vec<String>,
}

#[derive(Debug, Serialize)]
struct PostDetailCtx {
    slug: String,
    title: String,
    date: String,
    is_draft: bool,
    body_html: String,
    cover: Option<String>,
    photo_groups: Vec<PhotoGroupCtx>,
}

impl PostDetailCtx {
    fn from_post(post: &Post) -> Self {
        let cover = post.cover.as_deref().map(|p| photo_url(&post.slug, &post.source_dir, p));
        let photo_groups = post
            .photo_groups
            .iter()
            .map(|group| {
                let photos = group
                    .photos
                    .iter()
                    .map(|p| photo_url(&post.slug, &post.source_dir, p))
                    .collect();
                PhotoGroupCtx { name: group.name.clone(), photos }
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
    use crate::content::PhotoGroup;
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
        let photos_dir = source_dir.join("photos").join("Day 1");
        Post {
            slug: slug.into(),
            title: "With Photos".into(),
            date: "2025-02-01".into(),
            access: vec!["family".into()],
            cover: None,
            body_html: String::new(),
            photo_groups: vec![PhotoGroup {
                name: "Day 1".into(),
                photos: vec![photos_dir.join("a.jpg"), photos_dir.join("b.jpg")],
            }],
            source_dir,
        }
    }

    fn load_theme() -> Theme {
        Theme::load(Path::new(THEME_DIR))
    }

    #[test]
    fn render_index_contains_post_title() {
        let theme = load_theme();
        let site = Site {
            posts: vec![make_post("hawaii", "Hawaii Trip", vec!["family"], "Great time!")],
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
        assert!(!html.contains("Secret Draft"), "draft should not appear for non-admin");
    }

    #[test]
    fn render_index_shows_draft_badge_for_admin() {
        let theme = load_theme();
        let site = Site {
            posts: vec![make_post("draft-post", "Secret Draft", vec![], "")],
        };

        let html = theme.render_index(&site, &Viewer::admin()).unwrap();

        assert!(html.contains("Secret Draft"), "admin should see draft");
        assert!(html.to_uppercase().contains("DRAFT"), "admin should see DRAFT badge");
    }

    #[test]
    fn render_post_contains_body_and_title() {
        let theme = load_theme();
        let post = make_post("hawaii", "Hawaii Trip", vec!["family"], "We arrived safely.");

        let html = theme.render_post(&post, &Viewer::with_groups(["family"])).unwrap();

        assert!(html.contains("Hawaii Trip"));
        assert!(html.contains("We arrived safely."));
    }

    #[test]
    fn render_post_shows_draft_banner_for_admin() {
        let theme = load_theme();
        let post = make_post("draft-post", "WIP Post", vec![], "Not ready.");

        let html = theme.render_post(&post, &Viewer::admin()).unwrap();

        assert!(html.to_uppercase().contains("DRAFT"), "draft banner should be visible to admin");
    }

    #[test]
    fn render_post_photo_urls_use_media_prefix() {
        let theme = load_theme();
        let post = make_post_with_photos("trip");

        let html = theme.render_post(&post, &Viewer::with_groups(["family"])).unwrap();

        // MiniJinja HTML-encodes '/' as '&#x2f;' (OWASP recommendation).
        // Decode before asserting so the test checks logical URL content.
        let decoded = html.replace("&#x2f;", "/");
        assert!(decoded.contains("/media/trip/"), "photo URLs should start with /media/slug/");
        assert!(decoded.contains("a.jpg"));
    }

    #[test]
    fn render_login_produces_form() {
        let theme = load_theme();
        let html = theme.render_login(None).unwrap();
        assert!(html.contains("<form"), "login page should have a form element");
    }

    #[test]
    fn render_login_with_error_shows_message() {
        let theme = load_theme();
        let html = theme.render_login(Some("Invalid username or password")).unwrap();
        assert!(html.contains("Invalid username or password"));
    }

    #[test]
    fn photo_url_flat_photo() {
        let source = PathBuf::from("/posts/hawaii");
        let photo = source.join("photos").join("img.jpg");
        assert_eq!(photo_url("hawaii", &source, &photo), "/media/hawaii/img.jpg");
    }

    #[test]
    fn photo_url_subfolder_photo() {
        let source = PathBuf::from("/posts/hawaii");
        let photo = source.join("photos").join("Day 1").join("img.jpg");
        assert_eq!(photo_url("hawaii", &source, &photo), "/media/hawaii/Day 1/img.jpg");
    }
}
