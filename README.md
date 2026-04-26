# glimpse-rs

Glimpse serves select photo folders and narration from your personal photo files library as blog posts. Posts can be fully public or restricted under certain access groups like `family` and `friends`. I built this as a way of sharing life updates with remote family and friends.

It assumes you have your photo library arranged like so:

```
├── 2025-03-18 Hawaii
│   ├── index.md
│   └── photos
│       ├── 2025-03-18 Travel day
│       │   ├── 2025-03-16_nikon z6_3_dsc_031.jpg
│       │   ├── 2025-03-18_sm-s901u1_20250318_211908.jpg
│       │   └── 2025-03-18_sm-s901u1_20250318_211932.jpg
│       ├── 2025-03-19 Manoa Falls hiking
│       │   ├── 2025-03-19_nikon z6_3_dsc_0742.jpg
│       │   └── 2025-03-19_sm-s901u1_20250319_073233.jpg
│       └── 2025-03-20 Diamondhead crater hiking
├── 2018-05-28 Visiting Washington DC
│   ├── index.md
│   └── photos
│       ├── 2018-05-28_080456_d7500_dsc_0960.jpg
│       ├── 2018-05-28_081605_d7500_dsc_0967.jpg
│       ├── 2018-05-28_081615_d7500_dsc_0968.jpg
...
```

Each root level folder becomes its own blog post with beautiful rendering of photos and videos if `index.md` is properly defined.

## Defining `index.md`

`index.md` defines a new blog post when placed in the root of a post folder.

It's expected to have the following frontmatter format, defining `title`, `date`, `access`, optional `cover` photo. After the frontmatter, you add freeform markdown text for the blog post. Typically in my own usage, this describes the situation and location, what people attended and any interesting tidbits I'd like to save for the future and share with my close ones.

```
---
title: "2025-03 Hawaii"
date: 2025-03-18
access: [family, friends]
cover: "2025-03-19_nikon z6_3_dsc_0808.jpg"
---


# Hawaii travel

Lorem ipsum ...

```

## Design Principles

- **Modern browsers only** — target current evergreen browsers; use plain HTML, CSS, and browser-native APIs without polyfills or transpilation.
- **Server-side first** — render and process on the server; keep client-side logic to a minimum.
- **Simple auth is fine** — the audience is family and friends, not the public internet; favour simplicity over elaborate auth schemes.

## Running

```bash
export GLIMPSE_SESSION_SECRET=$(openssl rand -hex 64)
cargo run
```

Serves on `http://127.0.0.1:3000` by default. Requires `themes/default/` to be present. `users.toml` and `glimpse.toml` are optional — if absent the server starts with defaults and no registered users.

Pass `--config` or `--users` to override the default file paths:

```bash
cargo run -- --config /etc/glimpse/glimpse.toml --users /etc/glimpse/users.toml
```

## glimpse.toml

All fields are optional. The file itself is optional — omitting it uses the defaults shown below.

```toml
listen        = "127.0.0.1:3000"  # address and port to bind
site_title    = "Glimpse"         # shown in browser tab and page header
posts_dir     = "posts"           # directory containing post subdirectories
cache_dir     = "cache"           # directory for generated image/video derivatives
preprocess_concurrency = 2        # max concurrent derivative generation during reload
```

## users.toml

`users.toml` is not tracked in git — manage it with the `manage-users` binary or create it by hand next to the server binary.

```toml
[[users]]
username = "alice"
password_hash = "$argon2id$v=19$m=19456,t=2,p=1$..."
groups = ["family"]
feed_token_hash = "d4e2f9b7..."   # optional; enables the Atom feed for this user

[[users]]
username = "admin"
password_hash = "$argon2id$v=19$m=19456,t=2,p=1$..."
groups = ["admin"]
```

**Fields:**

| Field             | Required | Description |
|-------------------|----------|-------------|
| `username`        | yes      | Login name; case-sensitive |
| `password_hash`   | yes      | Argon2id hash — see below |
| `groups`          | yes      | List of group names this user belongs to |
| `feed_token_hash` | no       | SHA-256 hex of the user's Atom feed token |

**Groups** are plain strings. The special group `admin` bypasses all access checks and sees draft posts. All other group names must match the `access` list in a post's frontmatter for that post to be visible.

### Managing users with manage-users

The `manage-users` binary handles hashing, token generation, and file editing in one step:

```bash
# Add a user — prompts for password, generates a feed token automatically
cargo run --bin manage-users -- add alice --groups family,friends

# List all users and their groups
cargo run --bin manage-users -- list

# Change a password
cargo run --bin manage-users -- passwd alice

# Rotate the Atom feed token (prints the new token to share)
cargo run --bin manage-users -- rotate-token alice

# Remove a user
cargo run --bin manage-users -- remove alice
```

By default `manage-users` reads and writes `users.toml` in the current directory. Pass `--users <path>` to override. For scripted use, `--password-file <path>` reads the password from a file instead of prompting.

### Low-level helpers

These binaries are still available when you need them directly:

```bash
# Generate an Argon2id hash for a password
cargo run --bin hash-password -- <password>

# Mint a feed token and print both the raw token and the hash to store
cargo run --bin generate-feed-token
```

## Post format

Posts live under `posts/` (or the configured `posts_dir`) as dated folders:

```
posts/
  2025-03-18 Hawaii/
    index.md
    photos/
      2025-03-18 Travel day/
        *.jpg
      2025-03-19 Manoa Falls hiking/
        *.jpg
```

`index.md` frontmatter:

```yaml
---
title: "2025-03 Hawaii"
date: 2025-03-18
access: [family]          # groups that can see this post
cover: "photos/.../x.jpg" # optional hero image
---
```

**Access values:**

| `access` value | Visibility |
|----------------|------------|
| `[public]` | Everyone, including unauthenticated visitors |
| `[family]`, `[friends]`, etc. | Logged-in users whose groups overlap |
| omitted or `[]` | Draft — visible only to `admin` |

Adding at least one group publishes a post. `admin` users always see everything including drafts.

## Atom feeds

Each user can have a private Atom feed personalised to their access groups. Feeds use a per-user token in the URL instead of a session cookie, so any feed reader that fetches plain HTTPS works without extra configuration.

### Setting up a feed for a user

A feed token is generated automatically when you add a user with `manage-users add`. The token is printed once — copy it before the prompt closes:

```
$ cargo run --bin manage-users -- add alice --groups family
Password: ········
Confirm password: ········
Added user 'alice'.
Feed token (share with user): 3f8a1c0e...
```

The feed is live at:

```
https://yoursite.com/feed/3f8a1c0e....xml
```

Share this URL with the user. The feed contains only posts their groups can see.

### Revoking or rotating a feed token

```bash
cargo run --bin manage-users -- rotate-token alice
```

This replaces `feed_token_hash` in `users.toml` and prints the new token. The old token stops working immediately on the next hot reload. To disable the feed entirely, remove `feed_token_hash` from the user's entry by hand.

### Security notes

- Only the SHA-256 hash of the token is stored — a leaked `users.toml` does not expose live feed URLs.
- Image URLs inside the feed are signed with the token (`?t=<token>`), so feed readers can load photos without a session cookie.
- The feed endpoint returns 404 for unknown tokens — it does not confirm that the endpoint exists.
- Feed responses carry `Cache-Control: private, no-store`.

## Tooling

| Command | Purpose |
|---------|---------|
| `cargo run` | Start the server |
| `cargo clippy` | Lint (must be clean before committing) |
| `cargo test` | Run the test suite |
| `cargo run --bin manage-users -- <subcommand>` | Add/remove/passwd/rotate-token/list users |
| `cargo run --bin hash-password -- <pw>` | Low-level: generate an Argon2id hash |
| `cargo run --bin generate-feed-token` | Low-level: mint a new Atom feed token |
