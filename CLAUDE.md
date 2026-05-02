# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Design Principles

- **Photo albums first** — the primary purpose is displaying beautiful photo albums; text and navigation are lightweight context around the imagery, not the focus.
- **Simplicity over complexity** — favour simple, direct solutions; avoid large UI frameworks or unnecessary abstractions.
- **Modern browsers only** — target current evergreen browsers; use plain HTML, CSS, and browser-native APIs without polyfills or transpilation.
- **Server-side first** — render and process on the server; keep client-side logic to a minimum.
- **Simple auth is fine** — the audience is family and friends, not the public internet; favour simplicity over elaborate auth schemes.

## Commands

```bash
# Build
cargo build

# Run (requires GLIMPSE_SESSION_SECRET env var — see Runtime requirements below)
cargo run

# Lint (must be clean before committing)
cargo clippy

# Run all tests
cargo test

# Run a single test by name
cargo test <test_name>

# Run tests for a specific module
cargo test content::tests

# Manage users.toml (add/remove/passwd/rotate-token/list)
cargo run --bin manage-users -- --help
cargo run --bin manage-users -- add <username> --groups family,admin
cargo run --bin manage-users -- remove <username>
cargo run --bin manage-users -- passwd <username>
cargo run --bin manage-users -- rotate-token <username>
cargo run --bin manage-users -- list

# Low-level helpers (still available)
# Generate a password hash for users.toml
cargo run --bin hash-password -- <password>

# Generate a feed token for users.toml
cargo run --bin generate-feed-token
```

## Runtime requirements

The server requires `GLIMPSE_SESSION_SECRET` to be set — a 64-byte hex string used to sign session cookies:

```bash
export GLIMPSE_SESSION_SECRET=$(openssl rand -hex 64)
cargo run
```

It also reads `glimpse.toml` (optional config), `users.toml` (missing file is non-fatal), and `posts/` at startup.

`glimpse.toml` fields (all optional; these are the defaults):

```toml
listen = "127.0.0.1:3000"
site_title = "Glimpse"
posts_dir = "posts"
cache_dir = "cache"
preprocess_concurrency = 2
```

Override the theme directory (default `themes/default`) with the `GLIMPSE_THEME_DIR` env var.

## Linting

Workspace lints in `Cargo.toml` set `warnings = "deny"` and `clippy::all = "deny"`. All warnings are hard errors — `cargo clippy` must be clean before committing.

## Architecture

`glimpse-rs` is a personal photo-blog server. At startup it reads all posts from disk into memory, then serves them via an Axum HTTP server. There is no database.

### Layered module design

```
content  →  viewer  →  theme  →  server
```

- **`content`** — Pure I/O. Scans `posts/`, parses YAML frontmatter + Markdown, discovers photos. Produces `Site { posts: Vec<Post> }`. No HTTP, no templating.
- **`viewer`** — Access control. A `Viewer` holds a list of group memberships. `viewer::visible(site, viewer)` returns an iterator over posts the viewer may see. Posts with no `access` groups are drafts (admin-only). The `admin` group bypasses all access checks.
- **`theme`** — Pure rendering. Loads MiniJinja templates from `themes/default/templates/`. Translates domain model into template context structs before rendering HTML. URL construction lives here (`/media/{slug}/...`).
- **`media`** — Derivative image generation. `MediaCache::ensure(source, size)` checks for a cached JPEG derivative keyed by `hash(path, mtime, size)` and generates one on a blocking thread if absent. Cache lives in `cache/`. EXIF orientation is corrected before resizing; no upscaling.
- **`watcher`** — Hot reload. Background thread watching `posts/` via `notify`. On change: debounce 300 ms → rebuild `Site` → pre-generate all derivatives (concurrency capped at 2) → atomically swap via `ArcSwap`. Errors leave the previous `Site` live.
- **`users`** — User registry loaded from `users.toml`. Passwords verified with Argon2id. Feed tokens stored as SHA-256 hashes.
- **`config`** — Loads `glimpse.toml` (missing is non-fatal). Produces `Config` passed as `Arc<Config>` through `AppState` and `watcher`.
- **`server`** — Axum router. `AppState { site, theme, media_cache, users, cookie_key, cfg }` held in `Arc`.

### Post directory layout

```
posts/
  2025-03-18 Hawaii/
    index.md          # YAML frontmatter + Markdown body
    photos/
      img.jpg         # flat layout → one unnamed PhotoGroup
      Day 1/          # subfolder layout → one named PhotoGroup per subfolder
        a.jpg
```

Frontmatter fields: `title` (string), `date` (YYYY-MM-DD), `access` (list of group names; omit for draft), `cover` (optional relative path to cover image).

Media items include photos (jpg, png, webp, gif) and videos (mp4, mov, webm). Videos are only included if their filename contains `web-optimized`. Each `MediaItem` carries an `is_video` flag used by templates to render `<video>` vs `<img>` elements.

Folder name is the slug source: `"2025-03-18 Hawaii"` → `"2025-03-18-hawaii"`.

### Access control model

- `access: [public]` — visible to everyone
- `access: [family, friends]` — visible only to viewers in those groups
- `access:` absent or `[]` — draft; only `Viewer::admin()` can see it
- Users in the `admin` group see everything including drafts

Access is enforced by `viewer::can_view(post)`, called by all routes that return content. The `visible()` iterator in `viewer.rs` is the standard way to filter a `Site`.

### Authentication and sessions

Sessions use encrypted private cookies (`axum-extra` `PrivateCookieJar`) signed with the `GLIMPSE_SESSION_SECRET` key. The cookie stores only a username; on each request `viewer_from_jar()` looks up the user in `Users` and constructs their `Viewer`.

`users.toml` format:

```toml
[[users]]
username = "alice"
password_hash = "<argon2id hash from hash-password binary>"
groups = ["family", "admin"]
feed_token_hash = "<sha256 hex from generate-feed-token binary>"  # optional
```

### Routes

| Method | Path | Description |
|--------|------|-------------|
| GET | `/` | Post index (visible to current viewer) |
| GET | `/posts/{slug}` | Post detail page |
| GET | `/media/{post}/{*path}` | Photo serving; access-gated; `?size=thumb` or `?size=medium` for derivatives |
| GET | `/feed/{token}` | Personalised Atom feed; token identifies the user |
| GET | `/login` | Login form |
| POST | `/login` | Authenticate; sets session cookie |
| POST | `/logout` | Clears session cookie |
| POST | `/admin/reload` | Force-reload site from disk; requires `admin` session |
| GET | `/static/*` | Theme assets (CSS, fonts) |

The media route also accepts `?t={feed_token}` to authenticate feed readers that cannot send cookies (for image embeds inside Atom entries).

The feed route accepts an optional `.xml` suffix (`/feed/{token}.xml`) for compatibility with feed readers that expect a file extension; the suffix is stripped before token lookup.

### Media URL scheme

Photos are served at `/media/{post-slug}/{relative-path-under-photos/}`. `?size=thumb` returns a max-400 px JPEG; `?size=medium` returns max-1200 px. Originals: `Cache-Control: max-age=3600`; derivatives: `Cache-Control: immutable`.

### Templates

MiniJinja templates in `themes/default/templates/`: `base.html`, `index.html`, `post.html`, `login.html`. Theme CSS at `themes/default/static/style.css`.

Template context structs live in `src/theme/mod.rs`: `PostSummaryCtx` (index), `PostDetailCtx` (post page), `PhotoCtx` (photo URL variants), `PhotoGroupCtx`.
