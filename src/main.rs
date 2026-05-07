mod config;
mod content;
mod media;
mod server;
mod theme;
mod users;
mod viewer;
mod watcher;

use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use anyhow::{Context, bail};
use arc_swap::ArcSwap;
use axum_extra::extract::cookie::Key;
use tracing::{error, info};

use media::MediaCache;

#[derive(Parser)]
struct Args {
    /// Path to the TOML configuration file.
    #[arg(long, default_value = "glimpse.toml")]
    config: PathBuf,
    /// Path to the users TOML file.
    #[arg(long, default_value = "users.toml")]
    users: PathBuf,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let args = Args::parse();
    let cfg = Arc::new(config::Config::load(&args.config).context("failed to load config")?);

    let posts_dir = cfg.posts_dir.clone();
    let cache_dir = cfg.cache_dir.clone();
    let theme_dir = std::env::var("GLIMPSE_THEME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("themes/default"));

    let theme = theme::Theme::load(&theme_dir, cfg.site_title.clone());
    let users = users::Users::load(&args.users).context("failed to load users")?;
    let cookie_key = key_from_env().context("failed to load session key")?;

    // Start with an empty site so the listener can bind immediately; the real
    // site is loaded in the background and swapped in when ready.
    let site = Arc::new(ArcSwap::from_pointee(content::Site { posts: vec![] }));
    let media_cache = Arc::new(MediaCache::new(cache_dir.clone()));
    watcher::spawn(posts_dir.clone(), Arc::clone(&site), Arc::clone(&media_cache), Arc::clone(&cfg));

    let addr = cfg.listen;
    let load_posts_dir = cfg.posts_dir.clone();
    let load_cache_dir = cfg.cache_dir.clone();
    let state = server::AppState {
        site: Arc::clone(&site),
        theme: Arc::new(theme),
        media_cache,
        users: Arc::new(users),
        cookie_key,
        posts_dir,
        cfg,
    };

    let app = server::router(state, theme_dir.join("static"));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!(%addr, "listening");
    tokio::spawn(async move {
        // Parse one post at a time and swap the site after each, so already-cached
        // posts become visible immediately while a slow new folder is still loading.
        let result = tokio::task::spawn_blocking(move || {
            let mut entries: Vec<_> = std::fs::read_dir(&load_posts_dir)
                .map_err(|e| content::ContentError::Io { path: load_posts_dir.clone(), source: e })?
                .filter_map(|e| e.ok())
                .filter(|e| e.path().is_dir())
                .collect();
            entries.sort_by_key(|e| e.file_name());

            let mut posts: Vec<content::Post> = Vec::with_capacity(entries.len());
            for entry in entries {
                let t = std::time::Instant::now();
                let post = content::parse_post(&entry.path(), &load_cache_dir)?;
                let slug = post.slug.clone();
                posts.push(post);
                posts.sort_by(|a, b| a.date.cmp(&b.date));
                site.store(Arc::new(content::Site { posts: posts.clone() }));
                info!(%slug, elapsed_ms = t.elapsed().as_millis(), "post ready");
            }
            Ok::<_, content::ContentError>(posts.len())
        }).await;

        match result {
            Ok(Ok(count)) => info!(count, "finished loading posts"),
            Ok(Err(e)) => error!("failed to load site: {e:#}"),
            Err(e) => error!("site loader panicked: {e}"),
        }
    });

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
