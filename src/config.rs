use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use tracing::info;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(crate) struct Config {
    /// Address and port the HTTP server binds to.
    pub(crate) listen: SocketAddr,
    /// Site title shown in the browser tab and page header.
    pub(crate) site_title: String,
    /// Directory containing post subdirectories.
    pub(crate) posts_dir: PathBuf,
    /// Directory where generated image/video derivatives are cached.
    pub(crate) cache_dir: PathBuf,
    /// Number of posts to preprocess concurrently at startup and on reload.
    pub(crate) preprocess_concurrency: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen: "127.0.0.1:3000".parse().expect("valid default address"),
            site_title: "Glimpse".to_owned(),
            posts_dir: PathBuf::from("posts"),
            cache_dir: PathBuf::from("cache"),
            preprocess_concurrency: 2,
        }
    }
}

impl Config {
    /// Load from `path`. Returns [`Config::default`] if the file does not exist.
    pub(crate) fn load(path: &Path) -> anyhow::Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let cfg = toml::from_str(&text)?;
                info!(path = %path.display(), "loaded config");
                Ok(cfg)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                info!(path = %path.display(), "config file not found, using defaults");
                Ok(Self::default())
            }
            Err(e) => Err(e.into()),
        }
    }
}
