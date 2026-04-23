# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
# Build
cargo build

# Run (serves on http://127.0.0.1:3000)
cargo run

# Run all tests
cargo test

# Run a single test by name
cargo test <test_name>

# Run tests for a specific module
cargo test --test <module>  # or: cargo test content::tests
```

## Architecture

`glimpse-rs` is a personal photo-blog server. At startup it reads all posts from disk into memory, then serves them via an Axum HTTP server. There is no database.

### Layered module design

```
content  →  viewer  →  theme  →  server
```

- **`content`** — Pure I/O. Scans `posts/`, parses YAML frontmatter + Markdown, discovers photos. Produces `Site { posts: Vec<Post> }`. No HTTP, no templating.
- **`viewer`** — Access control. A `Viewer` holds a list of group memberships. `viewer::visible(site, viewer)` filters `Site.posts` by access rules. Posts with no `access` groups are drafts (admin-only).
- **`theme`** — Pure rendering. Loads MiniJinja templates from `themes/default/templates/`. Translates domain model into template context structs (`PostSummaryCtx`, `PostDetailCtx`) before rendering HTML. URL construction lives here (`/media/{slug}/...`).
- **`media`** — Derivative image generation. `MediaCache::ensure(source, size)` checks for a cached JPEG derivative (keyed by path + mtime hash) and generates one on a blocking thread if absent. Cache lives in `cache/` at runtime.
- **`server`** — Axum router. Holds `AppState { site, theme, media_cache }` in `Arc`. Three routes: `GET /`, `GET /posts/{slug}`, `GET /media/{post}/{*path}`. Static theme assets served from `themes/default/static/` under `/static/`.

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

Folder name is the slug source: `"2025-03-18 Hawaii"` → `"2025-03-18-hawaii"`.

### Access control model

- `access: [public]` — visible to everyone
- `access: [family, friends]` — visible only to viewers in those groups
- `access:` absent or `[]` — draft; only `Viewer::admin()` can see it
- Currently all HTTP requests use `Viewer::public()` — authentication is not yet implemented

### Media URL scheme

Photos are served at `/media/{post-slug}/{relative-path-under-photos/}`. Query param `?size=thumb` or `?size=medium` returns a cached JPEG derivative (max 400 px or 1200 px wide respectively). Originals served with `Cache-Control: max-age=3600`; derivatives with `immutable`.

### Templates

MiniJinja templates in `themes/default/templates/`: `base.html`, `index.html`, `post.html`, `login.html`. Theme CSS is at `themes/default/static/style.css`.
