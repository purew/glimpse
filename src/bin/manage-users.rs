//! CLI for managing `users.toml` — add, remove, change password, rotate feed token.
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use argon2::{
    Argon2, PasswordHasher,
    password_hash::{
        SaltString,
        rand_core::{OsRng, RngCore},
    },
};
use clap::{Parser, Subcommand};
use sha2::Digest;
use toml_edit::{ArrayOfTables, DocumentMut, Item, Table, value};

#[derive(Parser)]
#[command(name = "manage-users", about = "Manage glimpse-rs users.toml")]
struct Cli {
    /// Path to users.toml
    #[arg(long, default_value = "users.toml")]
    users: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Add a new user (prompts for password, auto-generates a feed token)
    Add {
        username: String,
        /// Comma-separated group names, e.g. family,admin
        #[arg(long, value_delimiter = ',')]
        groups: Vec<String>,
        /// Read password from file instead of prompting
        #[arg(long)]
        password_file: Option<PathBuf>,
    },
    /// Remove a user (warns and exits cleanly if the user does not exist)
    Remove { username: String },
    /// Change a user's password
    Passwd {
        username: String,
        /// Read password from file instead of prompting
        #[arg(long)]
        password_file: Option<PathBuf>,
    },
    /// Regenerate a user's feed token
    RotateToken { username: String },
    /// List all users and their groups
    List,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Add {
            username,
            groups,
            password_file,
        } => cmd_add(&cli.users, &username, &groups, password_file.as_deref()),
        Command::Remove { username } => cmd_remove(&cli.users, &username),
        Command::Passwd {
            username,
            password_file,
        } => cmd_passwd(&cli.users, &username, password_file.as_deref()),
        Command::RotateToken { username } => cmd_rotate_token(&cli.users, &username),
        Command::List => cmd_list(&cli.users),
    }
}

// ── File helpers ──────────────────────────────────────────────────────────────

fn load_doc(path: &Path) -> anyhow::Result<DocumentMut> {
    match std::fs::read_to_string(path) {
        Ok(text) => text
            .parse::<DocumentMut>()
            .with_context(|| format!("parsing {}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(DocumentMut::new()),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

fn save_doc(path: &Path, doc: &DocumentMut) -> anyhow::Result<()> {
    std::fs::write(path, doc.to_string())
        .with_context(|| format!("writing {}", path.display()))
}

// ── TOML helpers ──────────────────────────────────────────────────────────────

fn find_user_idx(doc: &DocumentMut, username: &str) -> Option<usize> {
    doc.get("users")
        .and_then(|item| item.as_array_of_tables())
        .and_then(|aot| {
            aot.iter()
                .position(|t| t.get("username").and_then(|v| v.as_str()) == Some(username))
        })
}

fn ensure_users_array(doc: &mut DocumentMut) {
    if doc.get("users").is_none() {
        doc.insert("users", Item::ArrayOfTables(ArrayOfTables::new()));
    }
}

// ── Crypto helpers ────────────────────────────────────────────────────────────

fn hash_password(password: &str) -> anyhow::Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| anyhow::anyhow!("password hashing failed: {e}"))
}

fn generate_feed_token() -> (String, String) {
    let mut bytes = [0u8; 16];
    OsRng.fill_bytes(&mut bytes);
    let token: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    let hash: String = sha2::Sha256::digest(token.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    (token, hash)
}

fn read_password(prompt: &str, password_file: Option<&Path>) -> anyhow::Result<String> {
    if let Some(path) = password_file {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading password file {}", path.display()))?;
        return Ok(text.trim_end().to_owned());
    }
    let pw = rpassword::prompt_password(prompt)?;
    let confirm = rpassword::prompt_password("Confirm password: ")?;
    anyhow::ensure!(pw == confirm, "passwords do not match");
    Ok(pw)
}

// ── Commands ──────────────────────────────────────────────────────────────────

fn cmd_add(
    path: &Path,
    username: &str,
    groups: &[String],
    password_file: Option<&Path>,
) -> anyhow::Result<()> {
    let mut doc = load_doc(path)?;

    if find_user_idx(&doc, username).is_some() {
        bail!("user '{username}' already exists");
    }

    let password = read_password("Password: ", password_file)?;
    let password_hash = hash_password(&password)?;
    let (token, token_hash) = generate_feed_token();

    ensure_users_array(&mut doc);

    let mut entry = Table::new();
    entry.insert("username", value(username));
    entry.insert("password_hash", value(password_hash));

    let mut groups_arr = toml_edit::Array::new();
    for g in groups {
        groups_arr.push(g.as_str());
    }
    entry.insert("groups", value(groups_arr));
    entry.insert("feed_token_hash", value(token_hash));

    doc["users"]
        .as_array_of_tables_mut()
        .expect("just inserted users array")
        .push(entry);

    save_doc(path, &doc)?;
    println!("Added user '{username}'.");
    println!("Feed token (share with user): {token}");
    Ok(())
}

fn cmd_remove(path: &Path, username: &str) -> anyhow::Result<()> {
    let mut doc = load_doc(path)?;

    let Some(idx) = find_user_idx(&doc, username) else {
        eprintln!("warning: user '{username}' not found, nothing to remove");
        return Ok(());
    };

    doc["users"]
        .as_array_of_tables_mut()
        .expect("user was found in users array")
        .remove(idx);

    save_doc(path, &doc)?;
    println!("Removed user '{username}'.");
    Ok(())
}

fn cmd_passwd(path: &Path, username: &str, password_file: Option<&Path>) -> anyhow::Result<()> {
    let mut doc = load_doc(path)?;

    let idx = find_user_idx(&doc, username)
        .ok_or_else(|| anyhow::anyhow!("user '{username}' not found"))?;

    let password = read_password("New password: ", password_file)?;
    let password_hash = hash_password(&password)?;

    doc["users"]
        .as_array_of_tables_mut()
        .expect("user was found in users array")
        .iter_mut()
        .nth(idx)
        .expect("idx is valid since find_user_idx found it")
        .insert("password_hash", value(password_hash));

    save_doc(path, &doc)?;
    println!("Updated password for '{username}'.");
    Ok(())
}

fn cmd_rotate_token(path: &Path, username: &str) -> anyhow::Result<()> {
    let mut doc = load_doc(path)?;

    let idx = find_user_idx(&doc, username)
        .ok_or_else(|| anyhow::anyhow!("user '{username}' not found"))?;

    let (token, token_hash) = generate_feed_token();

    doc["users"]
        .as_array_of_tables_mut()
        .expect("user was found in users array")
        .iter_mut()
        .nth(idx)
        .expect("idx is valid since find_user_idx found it")
        .insert("feed_token_hash", value(token_hash));

    save_doc(path, &doc)?;
    println!("Rotated feed token for '{username}'.");
    println!("New feed token (share with user): {token}");
    Ok(())
}

fn cmd_list(path: &Path) -> anyhow::Result<()> {
    let doc = load_doc(path)?;

    let Some(aot) = doc.get("users").and_then(|i| i.as_array_of_tables()) else {
        println!("No users.");
        return Ok(());
    };

    if aot.is_empty() {
        println!("No users.");
        return Ok(());
    }

    for table in aot.iter() {
        let username = table
            .get("username")
            .and_then(|v| v.as_str())
            .unwrap_or("<unknown>");
        let groups: Vec<&str> = table
            .get("groups")
            .and_then(|item| item.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();

        if groups.is_empty() {
            println!("{username}");
        } else {
            println!("{username}  [{}]", groups.join(", "));
        }
    }

    Ok(())
}
