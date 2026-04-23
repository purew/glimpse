//! File-backed user store with argon2 password verification.

use std::path::Path;

use anyhow::Context;
use argon2::{Argon2, PasswordHash, PasswordVerifier};
use serde::Deserialize;
use sha2::Digest;

/// A single user entry as stored in `users.toml`.
#[derive(Debug, Deserialize)]
pub struct User {
    pub username: String,
    password_hash: String,
    pub groups: Vec<String>,
    /// SHA-256 (hex) of the raw feed token. See `generate-feed-token` binary.
    #[serde(default)]
    pub feed_token_hash: Option<String>,
}

/// Return the SHA-256 hex digest of `token`.
///
/// Used both here (for lookup) and in the `generate-feed-token` binary (for
/// producing the hash to store in `users.toml`).
pub fn hash_feed_token(token: &str) -> String {
    let hash = sha2::Sha256::digest(token.as_bytes());
    hash.iter().map(|b| format!("{b:02x}")).collect()
}

#[derive(Debug, Default, Deserialize)]
struct UsersFile {
    #[serde(default)]
    users: Vec<User>,
}

/// In-memory view of the loaded user list.
#[derive(Debug, Default)]
pub struct Users {
    users: Vec<User>,
}

impl Users {
    /// Load users from a TOML file.
    ///
    /// If the file does not exist, returns an empty `Users` — no one can log
    /// in but the server starts normally. Any other I/O or parse error is fatal.
    ///
    /// # Errors
    ///
    /// Returns an error if the file exists but cannot be read or parsed.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let text = match std::fs::read_to_string(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
            Ok(t) => t,
        };
        let file: UsersFile =
            toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        Ok(Self { users: file.users })
    }

    /// Return the user if `username` and `password` are correct, `None` otherwise.
    ///
    /// Constant-time comparison is handled internally by the argon2 crate.
    pub fn verify(&self, username: &str, password: &str) -> Option<&User> {
        let user = self.get(username)?;
        let hash = PasswordHash::new(&user.password_hash).ok()?;
        Argon2::default()
            .verify_password(password.as_bytes(), &hash)
            .ok()?;
        Some(user)
    }

    /// Find a user by username; `None` if not found.
    pub fn get(&self, username: &str) -> Option<&User> {
        self.users.iter().find(|u| u.username == username)
    }

    /// Look up a user by their raw feed token; `None` if the token is not valid.
    ///
    /// The token is hashed before comparison so the in-memory store never holds
    /// a live token.
    pub fn lookup_by_feed_token(&self, token: &str) -> Option<&User> {
        let token_hash = hash_feed_token(token);
        self.users
            .iter()
            .find(|u| u.feed_token_hash.as_deref() == Some(token_hash.as_str()))
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use argon2::{
        Argon2, PasswordHasher,
        password_hash::{SaltString, rand_core::OsRng},
    };
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn hash(password: &str) -> String {
        let salt = SaltString::generate(&mut OsRng);
        Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .unwrap()
            .to_string()
    }

    fn toml_with_user(username: &str, password: &str, groups: &[&str]) -> NamedTempFile {
        let hash = hash(password);
        let group_list: Vec<String> = groups.iter().map(|g| format!(r#""{g}""#)).collect();
        let mut f = NamedTempFile::new().unwrap();
        write!(
            f,
            "[[users]]\nusername = \"{username}\"\npassword_hash = \"{hash}\"\ngroups = [{}]\n",
            group_list.join(", ")
        )
        .unwrap();
        f
    }

    #[test]
    fn verify_correct_password_returns_user() {
        let file = toml_with_user("alice", "hunter2", &["family"]);
        let users = Users::load(file.path()).unwrap();
        let user = users.verify("alice", "hunter2").unwrap();
        assert_eq!(user.username, "alice");
        assert_eq!(user.groups, vec!["family"]);
    }

    #[test]
    fn verify_wrong_password_returns_none() {
        let file = toml_with_user("alice", "hunter2", &["family"]);
        let users = Users::load(file.path()).unwrap();
        assert!(users.verify("alice", "wrongpassword").is_none());
    }

    #[test]
    fn verify_unknown_user_returns_none() {
        let file = toml_with_user("alice", "pw", &[]);
        let users = Users::load(file.path()).unwrap();
        assert!(users.verify("nobody", "pw").is_none());
    }

    #[test]
    fn load_missing_file_returns_empty_users() {
        let users = Users::load(Path::new("/nonexistent/path/users.toml")).unwrap();
        assert!(users.get("anyone").is_none());
    }

    #[test]
    fn get_returns_user_groups() {
        let file = toml_with_user("bob", "pw", &["family", "friends"]);
        let users = Users::load(file.path()).unwrap();
        let user = users.get("bob").unwrap();
        assert_eq!(user.groups, vec!["family", "friends"]);
    }

    #[test]
    fn get_unknown_user_returns_none() {
        let mut f = NamedTempFile::new().unwrap();
        write!(f, "").unwrap();
        let users = Users::load(f.path()).unwrap();
        assert!(users.get("ghost").is_none());
    }

    fn toml_with_feed_token(username: &str, token: &str) -> NamedTempFile {
        let token_hash = hash_feed_token(token);
        let mut f = NamedTempFile::new().unwrap();
        write!(
            f,
            "[[users]]\nusername = \"{username}\"\npassword_hash = \"\"\ngroups = []\nfeed_token_hash = \"{token_hash}\"\n",
        )
        .unwrap();
        f
    }

    #[test]
    fn lookup_by_feed_token_finds_user() {
        let file = toml_with_feed_token("alice", "abc123");
        let users = Users::load(file.path()).unwrap();
        let user = users.lookup_by_feed_token("abc123").unwrap();
        assert_eq!(user.username, "alice");
    }

    #[test]
    fn lookup_by_feed_token_wrong_token_returns_none() {
        let file = toml_with_feed_token("alice", "abc123");
        let users = Users::load(file.path()).unwrap();
        assert!(users.lookup_by_feed_token("wrongtoken").is_none());
    }

    #[test]
    fn lookup_by_feed_token_no_token_set_returns_none() {
        let file = toml_with_user("alice", "pw", &[]);
        let users = Users::load(file.path()).unwrap();
        assert!(users.lookup_by_feed_token("anything").is_none());
    }
}
