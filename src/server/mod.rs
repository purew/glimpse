//! HTTP server: Axum router, request handlers, sessions, and static file serving.

use std::path::PathBuf;
use std::sync::Arc;

use arc_swap::ArcSwap;

use axum::{
    Form, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
};
use axum_extra::extract::cookie::{Cookie, Key, PrivateCookieJar, SameSite};
use serde::Deserialize;
use tower_http::services::ServeDir;

use crate::content::Site;
use crate::media::{ImageSize, MediaCache};
use crate::theme::Theme;
use crate::users::Users;
use crate::viewer::{Viewer, visible};

// ── Constants ─────────────────────────────────────────────────────────────────

const SESSION_USER_KEY: &str = "username";

// ── App state ─────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct AppState {
    pub site: Arc<ArcSwap<Site>>,
    pub theme: Arc<Theme>,
    pub media_cache: Arc<MediaCache>,
    pub users: Arc<Users>,
    pub cookie_key: Key,
}

impl axum::extract::FromRef<AppState> for Key {
    fn from_ref(state: &AppState) -> Self {
        state.cookie_key.clone()
    }
}

// ── Router ────────────────────────────────────────────────────────────────────

/// Build the application router.
///
/// `static_dir` is the directory served under `/static/` (theme CSS, fonts, etc.).
pub fn router(state: AppState, static_dir: PathBuf) -> Router {
    Router::new()
        .route("/", get(index_handler))
        .route("/posts/{slug}", get(post_handler))
        .route("/media/{post}/{*path}", get(media_handler))
        .route("/feed/{token}", get(feed_handler))
        .route("/login", get(login_get_handler).post(login_post_handler))
        .route("/logout", post(logout_handler))
        .nest_service("/static", ServeDir::new(static_dir))
        .with_state(state)
}

// ── Session helpers ───────────────────────────────────────────────────────────

fn viewer_from_jar(jar: &PrivateCookieJar, users: &Users) -> Viewer {
    let username = jar.get(SESSION_USER_KEY).map(|c| c.value().to_owned());
    match username.as_deref().and_then(|u| users.get(u)) {
        Some(user) => Viewer::with_groups(user.groups.iter().cloned()),
        None => Viewer::public(),
    }
}

fn session_cookie(name: &'static str, value: String) -> Cookie<'static> {
    Cookie::build((name, value))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Lax)
        .build()
}

// ── ETag helpers ─────────────────────────────────────────────────────────────

fn html_etag(html: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    html.hash(&mut h);
    format!("\"{}\"", h.finish())
}

fn html_response(html: String, request_headers: &HeaderMap) -> Response {
    let etag = html_etag(&html);
    // Safe: etag is always a valid ASCII string.
    let etag_val = HeaderValue::from_str(&etag).expect("etag is valid header value");

    if request_headers.get(header::IF_NONE_MATCH) == Some(&etag_val) {
        return (StatusCode::NOT_MODIFIED, [(header::ETAG, etag_val)]).into_response();
    }

    (
        [
            (header::ETAG, etag_val),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-cache")),
        ],
        Html(html),
    )
        .into_response()
}

// ── Handlers ──────────────────────────────────────────────────────────────────

async fn index_handler(
    State(state): State<AppState>,
    jar: PrivateCookieJar,
    request_headers: HeaderMap,
) -> Response {
    let viewer = viewer_from_jar(&jar, &state.users);
    let site = state.site.load_full();
    match state.theme.render_index(&site, &viewer) {
        Ok(html) => html_response(html, &request_headers),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn post_handler(
    State(state): State<AppState>,
    jar: PrivateCookieJar,
    request_headers: HeaderMap,
    Path(slug): Path<String>,
) -> Response {
    let viewer = viewer_from_jar(&jar, &state.users);
    let site = state.site.load_full();
    let Some(post) = visible(&site, &viewer).find(|p| p.slug == slug) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match state.theme.render_post(post, &viewer) {
        Ok(html) => html_response(html, &request_headers),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn feed_handler(
    State(state): State<AppState>,
    Path(token_filename): Path<String>,
    request_headers: HeaderMap,
) -> Response {
    // Strip the ".xml" extension from the path segment.
    let token = token_filename
        .strip_suffix(".xml")
        .unwrap_or(&token_filename);

    let Some(user) = state.users.lookup_by_feed_token(token) else {
        // Return 404, not 401, to avoid confirming the endpoint exists.
        return StatusCode::NOT_FOUND.into_response();
    };

    let viewer = crate::viewer::Viewer::with_groups(user.groups.iter().cloned());
    let site = state.site.load_full();
    let base_url = derive_base_url(&request_headers);
    let xml = crate::theme::render_feed(&site, &viewer, &base_url, token);

    (
        [
            (header::CONTENT_TYPE, "application/atom+xml; charset=utf-8"),
            (header::CACHE_CONTROL, "private, no-store"),
        ],
        xml,
    )
        .into_response()
}

/// Build a base URL from the incoming `Host` header.
///
/// Falls back to `http://localhost` if the header is absent or malformed.
fn derive_base_url(headers: &HeaderMap) -> String {
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("localhost");
    let scheme = if host.starts_with("localhost") || host.starts_with("127.") {
        "http"
    } else {
        "https"
    };
    format!("{scheme}://{host}")
}

#[derive(Deserialize)]
struct MediaParams {
    size: Option<String>,
    /// Feed token for unauthenticated media access from feed readers.
    t: Option<String>,
}

async fn media_handler(
    State(state): State<AppState>,
    jar: PrivateCookieJar,
    Path((post_slug, file_path)): Path<(String, String)>,
    Query(params): Query<MediaParams>,
) -> Response {
    if !is_safe_subpath(&file_path) {
        return StatusCode::NOT_FOUND.into_response();
    }

    // Accept either a valid session cookie or a valid feed token.
    let viewer = if let Some(token) = &params.t {
        match state.users.lookup_by_feed_token(token) {
            Some(user) => crate::viewer::Viewer::with_groups(user.groups.iter().cloned()),
            None => return StatusCode::NOT_FOUND.into_response(),
        }
    } else {
        viewer_from_jar(&jar, &state.users)
    };

    let site = state.site.load_full();
    let Some(post) = visible(&site, &viewer).find(|p| p.slug == post_slug) else {
        return StatusCode::NOT_FOUND.into_response();
    };

    let source = post.source_dir.join(&file_path);

    let size = match params.size.as_deref() {
        Some("thumb") => Some(ImageSize::Thumbnail),
        Some("medium") => Some(ImageSize::Medium),
        _ => None,
    };

    if let Some(size) = size {
        return match state.media_cache.ensure(&source, size).await {
            Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
            Ok(path) => match tokio::fs::read(&path).await {
                Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
                Ok(bytes) => (
                    [
                        (header::CONTENT_TYPE, "image/jpeg"),
                        (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
                    ],
                    bytes,
                )
                    .into_response(),
            },
        };
    }

    let content_type = media_content_type(&source);
    match tokio::fs::read(&source).await {
        Ok(bytes) => (
            [
                (header::CONTENT_TYPE, content_type),
                (header::CACHE_CONTROL, "public, max-age=3600"),
            ],
            bytes,
        )
            .into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn login_get_handler(State(state): State<AppState>) -> Response {
    match state.theme.render_login(None) {
        Ok(html) => Html(html).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[derive(Deserialize)]
struct LoginForm {
    username: String,
    password: String,
}

async fn login_post_handler(
    State(state): State<AppState>,
    jar: PrivateCookieJar,
    Form(form): Form<LoginForm>,
) -> Response {
    if state.users.verify(&form.username, &form.password).is_some() {
        let updated_jar = jar.add(session_cookie(SESSION_USER_KEY, form.username));
        (updated_jar, Redirect::to("/")).into_response()
    } else {
        match state
            .theme
            .render_login(Some("Invalid username or password"))
        {
            Ok(html) => (StatusCode::UNAUTHORIZED, Html(html)).into_response(),
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
        }
    }
}

async fn logout_handler(jar: PrivateCookieJar) -> Response {
    let updated_jar = jar.remove(Cookie::from(SESSION_USER_KEY));
    (updated_jar, Redirect::to("/login")).into_response()
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Returns `true` only when every component of `path` is a plain file/dir name —
/// no `..`, no absolute root, no prefix components.
fn is_safe_subpath(path: &str) -> bool {
    std::path::Path::new(path)
        .components()
        .all(|c| matches!(c, std::path::Component::Normal(_)))
}

fn media_content_type(path: &std::path::Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_lowercase)
        .as_deref()
    {
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("png") => "image/png",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        Some("mp4") => "video/mp4",
        Some("mov") => "video/quicktime",
        Some("webm") => "video/webm",
        _ => "application/octet-stream",
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::{MediaItem, PhotoGroup, Post, Site};
    use axum::http::Request;
    use http_body_util::BodyExt;
    use std::path::Path;
    use tempfile::TempDir;
    use tower::ServiceExt;

    const THEME_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/themes/default");

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn make_post(slug: &str, access: Vec<&str>) -> Post {
        Post {
            slug: slug.into(),
            title: format!("Post {slug}"),
            date: "2025-01-01".into(),
            access: access.into_iter().map(str::to_owned).collect(),
            cover: None,
            body_html: "<p>Body text.</p>".into(),
            photo_groups: vec![],
            source_dir: PathBuf::from("/posts").join(slug),
        }
    }

    fn make_post_with_photo(slug: &str, source_dir: &Path) -> Post {
        Post {
            slug: slug.into(),
            title: "Photo Post".into(),
            date: "2025-01-01".into(),
            access: vec!["public".into()],
            cover: None,
            body_html: String::new(),
            photo_groups: vec![PhotoGroup {
                name: String::new(),
                media: vec![MediaItem { path: source_dir.join("img.jpg"), is_video: false }],
            }],
            source_dir: source_dir.to_owned(),
        }
    }

    fn write_test_image(path: &std::path::Path, width: u32, height: u32) {
        let img = image::RgbImage::new(width, height);
        image::DynamicImage::ImageRgb8(img)
            .save_with_format(path, image::ImageFormat::Png)
            .unwrap();
    }

    fn test_state(posts: Vec<Post>, cache_dir: PathBuf) -> AppState {
        AppState {
            site: Arc::new(ArcSwap::from_pointee(Site { posts })),
            theme: Arc::new(crate::theme::Theme::load(Path::new(THEME_DIR))),
            media_cache: Arc::new(MediaCache::new(cache_dir)),
            users: Arc::new(crate::users::Users::default()),
            cookie_key: Key::generate(),
        }
    }

    fn test_state_with_users(
        posts: Vec<Post>,
        cache_dir: PathBuf,
        users: crate::users::Users,
    ) -> AppState {
        AppState {
            site: Arc::new(ArcSwap::from_pointee(Site { posts })),
            theme: Arc::new(crate::theme::Theme::load(Path::new(THEME_DIR))),
            media_cache: Arc::new(MediaCache::new(cache_dir)),
            users: Arc::new(users),
            cookie_key: Key::generate(),
        }
    }

    fn users_with_feed_token(token: &str) -> crate::users::Users {
        use std::io::Write;
        use tempfile::NamedTempFile;
        let token_hash = crate::users::hash_feed_token(token);
        let mut f = NamedTempFile::new().unwrap();
        write!(
            f,
            "[[users]]\nusername = \"alice\"\npassword_hash = \"\"\ngroups = [\"family\"]\nfeed_token_hash = \"{token_hash}\"\n"
        )
        .unwrap();
        crate::users::Users::load(f.path()).unwrap()
    }

    async fn body_string(response: axum::response::Response) -> String {
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        String::from_utf8_lossy(&bytes).into_owned()
    }

    fn get(uri: &str) -> Request<axum::body::Body> {
        Request::builder()
            .uri(uri)
            .body(axum::body::Body::empty())
            .unwrap()
    }

    fn build_router(state: AppState) -> Router {
        router(state, PathBuf::from(THEME_DIR).join("static"))
    }

    // ── Index ─────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn index_returns_200() {
        let tmp = TempDir::new().unwrap();
        let app = build_router(test_state(
            vec![make_post("p1", vec!["public"])],
            tmp.path().join("cache"),
        ));
        let resp = app.oneshot(get("/")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn index_lists_public_post() {
        let tmp = TempDir::new().unwrap();
        let app = build_router(test_state(
            vec![make_post("hawaii", vec!["public"])],
            tmp.path().join("cache"),
        ));
        let resp = app.oneshot(get("/")).await.unwrap();
        let html = body_string(resp).await;
        assert!(html.contains("Post hawaii"));
        assert!(html.contains("/posts/hawaii"));
    }

    #[tokio::test]
    async fn index_hides_draft_from_public_viewer() {
        let tmp = TempDir::new().unwrap();
        let app = build_router(test_state(
            vec![
                make_post("published", vec!["public"]),
                make_post("secret-draft", vec![]),
            ],
            tmp.path().join("cache"),
        ));
        let resp = app.oneshot(get("/")).await.unwrap();
        let html = body_string(resp).await;
        assert!(html.contains("Post published"));
        assert!(!html.contains("Post secret-draft"));
    }

    // ── Post ──────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn post_returns_200_for_public_post() {
        let tmp = TempDir::new().unwrap();
        let app = build_router(test_state(
            vec![make_post("hawaii", vec!["public"])],
            tmp.path().join("cache"),
        ));
        let resp = app.oneshot(get("/posts/hawaii")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let html = body_string(resp).await;
        assert!(html.contains("Post hawaii"));
        assert!(html.contains("Body text."));
    }

    #[tokio::test]
    async fn post_returns_404_for_unknown_slug() {
        let tmp = TempDir::new().unwrap();
        let app = build_router(test_state(vec![], tmp.path().join("cache")));
        let resp = app.oneshot(get("/posts/does-not-exist")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn post_returns_404_for_draft() {
        let tmp = TempDir::new().unwrap();
        let app = build_router(test_state(
            vec![make_post("wip", vec![])],
            tmp.path().join("cache"),
        ));
        let resp = app.oneshot(get("/posts/wip")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn post_returns_404_for_group_restricted_post_without_session() {
        let tmp = TempDir::new().unwrap();
        let app = build_router(test_state(
            vec![make_post("family-only", vec!["family"])],
            tmp.path().join("cache"),
        ));
        let resp = app.oneshot(get("/posts/family-only")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    // ── Login ─────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn login_get_returns_form() {
        let tmp = TempDir::new().unwrap();
        let app = build_router(test_state(vec![], tmp.path().join("cache")));
        let resp = app.oneshot(get("/login")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let html = body_string(resp).await;
        assert!(html.contains("<form"));
    }

    #[tokio::test]
    async fn login_post_bad_credentials_returns_error_page() {
        let tmp = TempDir::new().unwrap();
        let app = build_router(test_state(vec![], tmp.path().join("cache")));
        let req = Request::builder()
            .method("POST")
            .uri("/login")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(axum::body::Body::from("username=alice&password=wrong"))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        let html = body_string(resp).await;
        assert!(html.contains("Invalid username or password"));
    }

    #[tokio::test]
    async fn logout_redirects_to_login() {
        let tmp = TempDir::new().unwrap();
        let app = build_router(test_state(vec![], tmp.path().join("cache")));
        let req = Request::builder()
            .method("POST")
            .uri("/logout")
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::SEE_OTHER);
        assert_eq!(resp.headers().get(header::LOCATION).unwrap(), "/login");
    }

    // ── Media — original ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn media_serves_original_with_short_cache() {
        let tmp = TempDir::new().unwrap();
        write_test_image(&tmp.path().join("img.png"), 10, 10);

        let post = make_post_with_photo("trip", tmp.path());
        let app = build_router(test_state(vec![post], tmp.path().join("cache")));

        let resp = app.oneshot(get("/media/trip/img.png")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            "image/png"
        );
        assert_eq!(
            resp.headers().get(header::CACHE_CONTROL).unwrap(),
            "public, max-age=3600"
        );
    }

    #[tokio::test]
    async fn media_returns_404_for_unknown_post() {
        let tmp = TempDir::new().unwrap();
        let app = build_router(test_state(vec![], tmp.path().join("cache")));
        let resp = app
            .oneshot(get("/media/no-such-post/img.jpg"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn media_returns_404_for_draft_post() {
        let tmp = TempDir::new().unwrap();
        let mut post = make_post_with_photo("draft", tmp.path());
        post.access.clear();
        let app = build_router(test_state(vec![post], tmp.path().join("cache")));
        let resp = app.oneshot(get("/media/draft/img.jpg")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    // ── Media — derivatives ───────────────────────────────────────────────────

    #[tokio::test]
    async fn media_serves_thumbnail_with_immutable_cache() {
        let tmp = TempDir::new().unwrap();
        write_test_image(&tmp.path().join("img.png"), 800, 600);

        let post = make_post_with_photo("trip", tmp.path());
        let app = build_router(test_state(vec![post], tmp.path().join("cache")));

        let resp = app
            .oneshot(get("/media/trip/img.png?size=thumb"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            "image/jpeg"
        );
        assert_eq!(
            resp.headers().get(header::CACHE_CONTROL).unwrap(),
            "public, max-age=31536000, immutable"
        );
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let img = image::load_from_memory(&bytes).unwrap();
        assert!(img.width() <= 400);
    }

    #[tokio::test]
    async fn media_serves_medium_derivative() {
        let tmp = TempDir::new().unwrap();
        write_test_image(&tmp.path().join("img.png"), 2000, 1500);

        let post = make_post_with_photo("trip", tmp.path());
        let app = build_router(test_state(vec![post], tmp.path().join("cache")));

        let resp = app
            .oneshot(get("/media/trip/img.png?size=medium"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let img = image::load_from_memory(&bytes).unwrap();
        assert!(img.width() <= 1200);
    }

    // ── Path safety ───────────────────────────────────────────────────────────

    #[test]
    fn safe_subpath_allows_plain_name() {
        assert!(is_safe_subpath("img.jpg"));
        assert!(is_safe_subpath("Day 1/img.jpg"));
        assert!(is_safe_subpath("a/b/c.png"));
    }

    #[test]
    fn safe_subpath_rejects_traversal() {
        assert!(!is_safe_subpath("../etc/passwd"));
        assert!(!is_safe_subpath("Day 1/../../secret"));
        assert!(!is_safe_subpath("/etc/passwd"));
    }

    #[tokio::test]
    async fn media_returns_404_for_path_traversal() {
        let tmp = TempDir::new().unwrap();
        let post = make_post_with_photo("trip", tmp.path());
        let app = build_router(test_state(vec![post], tmp.path().join("cache")));
        let resp = app
            .oneshot(get("/media/trip/..%2F..%2Fetc%2Fpasswd"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    // ── Feed ──────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn feed_returns_404_for_unknown_token() {
        let tmp = TempDir::new().unwrap();
        let app = build_router(test_state(vec![], tmp.path().join("cache")));
        let resp = app.oneshot(get("/feed/nosuchtoken.xml")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn feed_returns_atom_for_valid_token() {
        let tmp = TempDir::new().unwrap();
        let users = users_with_feed_token("mytoken");
        let post = make_post("hawaii", vec!["family"]);
        let app = build_router(test_state_with_users(
            vec![post],
            tmp.path().join("cache"),
            users,
        ));
        let resp = app.oneshot(get("/feed/mytoken.xml")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers()
                .get(header::CONTENT_TYPE)
                .unwrap()
                .to_str()
                .unwrap(),
            "application/atom+xml; charset=utf-8"
        );
        assert_eq!(
            resp.headers()
                .get(header::CACHE_CONTROL)
                .unwrap()
                .to_str()
                .unwrap(),
            "private, no-store"
        );
        let body = body_string(resp).await;
        assert!(
            body.contains("Post hawaii"),
            "feed should contain post title"
        );
    }

    #[tokio::test]
    async fn feed_excludes_posts_inaccessible_to_token_user() {
        let tmp = TempDir::new().unwrap();
        let users = users_with_feed_token("tok");
        let posts = vec![
            make_post("visible", vec!["family"]),
            make_post("restricted", vec!["friends"]),
        ];
        let app = build_router(test_state_with_users(
            posts,
            tmp.path().join("cache"),
            users,
        ));
        let resp = app.oneshot(get("/feed/tok.xml")).await.unwrap();
        let body = body_string(resp).await;
        assert!(body.contains("Post visible"));
        assert!(!body.contains("Post restricted"));
    }

    // ── Media — feed token auth ───────────────────────────────────────────────

    #[tokio::test]
    async fn media_serves_image_with_valid_feed_token() {
        let tmp = TempDir::new().unwrap();
        write_test_image(&tmp.path().join("img.png"), 10, 10);

        let users = users_with_feed_token("feedtok");
        let post = Post {
            slug: "trip".into(),
            title: "Trip".into(),
            date: "2025-01-01".into(),
            access: vec!["family".into()],
            cover: None,
            body_html: String::new(),
            photo_groups: vec![],
            source_dir: tmp.path().to_owned(),
        };
        let app = build_router(test_state_with_users(
            vec![post],
            tmp.path().join("cache"),
            users,
        ));

        let resp = app
            .oneshot(get("/media/trip/img.png?t=feedtok"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn media_returns_404_for_invalid_feed_token() {
        let tmp = TempDir::new().unwrap();
        write_test_image(&tmp.path().join("img.png"), 10, 10);

        let post = make_post_with_photo("trip", tmp.path());
        let app = build_router(test_state(vec![post], tmp.path().join("cache")));

        let resp = app
            .oneshot(get("/media/trip/img.png?t=badtoken"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
}
