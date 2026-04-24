# glimpse-rs

A private family blog service. Posts are photo-centred with per-post access control by group.

## Running

```bash
export GLIMPSE_SESSION_SECRET=$(openssl rand -hex 64)
cargo run
```

Serves on `http://127.0.0.1:3000`. Requires `posts/` and `themes/default/` to be present. `users.toml` is optional — if absent the server starts with no registered users.

## users.toml

`users.toml` is not tracked in git — create it manually next to the binary or in the working directory.

```toml
[[users]]
username = "alice"
password_hash = "$argon2id$v=19$m=19456,t=2,p=1$..."
groups = ["family"]

[[users]]
username = "admin"
password_hash = "$argon2id$v=19$m=19456,t=2,p=1$..."
groups = ["admin"]
```

**Fields:**

| Field           | Required | Description |
|----------------|----------|-------------|
| `username`      | yes      | Login name; case-sensitive |
| `password_hash` | yes      | Argon2id hash produced by `cargo run --bin hash-password` |
| `groups`        | yes      | List of group names this user belongs to |

**Groups** are plain strings. The special group `admin` bypasses all access checks and sees draft posts. All other group names must match the `access` list in a post's frontmatter for that post to be visible.

### Generating a password hash

```
cargo run --bin hash-password -- <password>
```

Paste the output into the `password_hash` field. Each call produces a different salt, which is correct — all hashes for the same password will still verify.

### Resetting a password

Edit `users.toml`, replace the `password_hash` value with a new hash, and restart the server. There is no self-service reset.

## Post format

Posts live under `posts/` as dated folders:

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

Omitting `access` (or leaving it empty) makes a post a **draft** — visible only to `admin`. Adding at least one group publishes it.

## Atom feeds

Each user can have a private Atom feed personalised to their access groups. Feeds use a per-user token in the URL instead of a session cookie, so any feed reader that fetches plain HTTPS works without extra configuration.

### Setting up a feed for a user

**1. Generate a token:**

```
cargo run --bin generate-feed-token
```

Output:
```
Token (use in feed URL):   3f8a1c0e...
Hash  (add to users.toml): d4e2f9b7...
```

**2. Add the hash to `users.toml`:**

```toml
[[users]]
username = "alice"
password_hash = "..."
groups = ["family"]
feed_token_hash = "d4e2f9b7..."
```

Restart the server (or wait for hot reload). The feed is now live at:

```
https://yoursite.com/feed/3f8a1c0e....xml
```

Share this URL with the user. The feed contains only posts their groups can see.

### Revoking a feed token

Replace `feed_token_hash` with a new value from `generate-feed-token` (or delete the field to disable the feed entirely) and restart.

### Security notes

- Only the SHA-256 hash of the token is stored — a leaked `users.toml` does not expose live feed URLs.
- Image URLs inside the feed are signed with the token (`?t=<token>`), so feed readers can load photos without a session cookie.
- The feed endpoint returns 404 for unknown tokens — it does not confirm that the endpoint exists.
- Feed responses carry `Cache-Control: private, no-store`.

## Tooling

| Command | Purpose |
|---------|---------|
| `cargo run` | Start the server |
| `cargo run --bin hash-password -- <pw>` | Generate an argon2 hash for users.toml |
| `cargo run --bin generate-feed-token` | Mint a new Atom feed token for a user |
| `cargo test` | Run the test suite |
