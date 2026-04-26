# Share link for a single post

Generate a time-limited or permanent token that grants unauthenticated access to a single post, so a specific post can be shared with someone who has no account.

## Motivation

Sharing a private post currently requires adding the recipient as a user. A share link allows one-off sharing without account management — useful for sending a single post to someone outside the normal audience.

## Expected behaviour

- A share link encodes a post slug and a token, e.g. `/share/{token}`.
- The token maps to a specific post and optionally an expiry timestamp.
- Anyone with the link can view that post (including its photos) without logging in.
- Tokens are stored in a new `shares.toml` (or appended to `users.toml`), similar to feed tokens.
- The server validates the token, checks it covers the requested post, and constructs a `Viewer` with access only to that post.

## CLI / management

```
glimpse-share add <post-slug> [--expires 2026-12-31]   # prints the full share URL
glimpse-share list                                      # show active share links
glimpse-share revoke <token>                            # invalidate a token
```

## Affected modules

- **`users`** (or new **`shares`** module) — token storage and lookup.
- **`server`** — new `GET /share/{token}` route that renders the post detail page.
- **`viewer`** — a share-scoped `Viewer` that passes `can_view` only for the target post.
- New binary `src/bin/manage-shares.rs`.
