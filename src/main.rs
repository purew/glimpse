mod content;
mod media;
mod server;
mod theme;
mod users;
mod viewer;
mod watcher;

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, bail};
use arc_swap::ArcSwap;
use axum_extra::extract::cookie::Key;
use tracing::info;

use media::MediaCache;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let posts_dir = PathBuf::from("posts");
    let theme_dir = PathBuf::from("themes/default");
    let cache_dir = PathBuf::from("cache");

    let site = content::load_site(&posts_dir).context("failed to load site")?;
    info!(count = site.posts.len(), "loaded posts");

    let theme = theme::Theme::load(&theme_dir);
    let users = users::Users::load(Path::new("users.toml")).context("failed to load users")?;
    let cookie_key = key_from_env().context("failed to load session key")?;

    let site = Arc::new(ArcSwap::from_pointee(site));
    let media_cache = Arc::new(MediaCache::new(cache_dir));
    watcher::spawn(posts_dir, Arc::clone(&site), Arc::clone(&media_cache));

    let state = server::AppState {
        site,
        theme: Arc::new(theme),
        media_cache,
        users: Arc::new(users),
        cookie_key,
    };

    let app = server::router(state, theme_dir.join("static"));
    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    info!(%addr, "listening");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

/// Read `GLIMPSE_SESSION_SECRET` from the environment and decode it as a 128-char hex
/// string (64 bytes) to use as the cookie signing key.
///
/// Generate a suitable value with: `openssl rand -hex 64`
fn key_from_env() -> anyhow::Result<Key> {
    let hex = std::env::var("GLIMPSE_SESSION_SECRET")
        .context("GLIMPSE_SESSION_SECRET env var not set (generate with: openssl rand -hex 64)")?;
    let bytes = decode_hex(&hex)
        .context("GLIMPSE_SESSION_SECRET must be a 128-character hex string (64 bytes)")?;
    if bytes.len() != 64 {
        bail!(
            "GLIMPSE_SESSION_SECRET must be exactly 64 bytes (128 hex chars), got {}",
            bytes.len()
        );
    }
    Ok(Key::from(&bytes))
}

fn decode_hex(s: &str) -> anyhow::Result<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        bail!("odd-length hex string");
    }
    (0..s.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&s[i..i + 2], 16)
                .with_context(|| format!("invalid hex at position {i}"))
        })
        .collect()
}
