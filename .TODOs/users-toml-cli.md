# users.toml management CLI

Build a dedicated CLI tool for managing `users.toml` — adding, removing, and updating users without manually running `hash-password` and `generate-feed-token` and hand-editing the file.

## Motivation

Currently managing users requires three separate steps: run `hash-password`, run `generate-feed-token`, then manually edit `users.toml`. This is error-prone and inconvenient, especially when deploying or rotating credentials.

## Expected behaviour

```
glimpse-users add <username> --groups family,admin   # prompts for password, generates feed token
glimpse-users remove <username>
glimpse-users passwd <username>                      # re-hash a new password in place
glimpse-users rotate-token <username>                # regenerate feed token
glimpse-users list                                   # print usernames and groups
```

- Reads and writes `users.toml` in place, preserving unrelated entries.
- `--users <path>` flag (or reads `glimpse.toml` for the configured path) to locate the file.
- Password input via interactive prompt (hidden) or `--password-file` for scripted use.
- Idempotent: adding an existing user is an error; removing a missing user is a no-op with a warning.

## Affected modules

- New binary `src/bin/manage-users.rs` (or `glimpse-users`).
- May reuse `users` module for the `User` type and Argon2id hashing logic.
- `generate-feed-token` and `hash-password` binaries can remain for low-level use but become optional.
