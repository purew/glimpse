# glimpse-rs

A private family blog service. Posts are photo-centred with per-post access control by group.

## Running

```
cargo run
```

Serves on `http://127.0.0.1:3000` by default. Requires `posts/`, `themes/default/`, and `users.toml` to be present.

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

## Tooling

| Command | Purpose |
|---------|---------|
| `cargo run` | Start the server |
| `cargo run --bin hash-password -- <pw>` | Generate an argon2 hash for users.toml |
| `cargo test` | Run the test suite |
