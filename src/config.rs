use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use tracing::info;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Config {
    /// Address and port the HTTP server binds to.
    pub listen: SocketAddr,
    /// Site title shown in the browser tab and page header.
    pub site_title: String,
    /// Directory containing post subdirectories.
    pub posts_dir: PathBuf,
    /// Directory where generated image/video derivatives are cached.
    pub cache_dir: PathBuf,
    /// Videos taller than this are skipped at load time.
    pub video_max_height: u32,
    /// Maximum number of image derivatives generated concurrently during a reload.
    pub preprocess_concurrency: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen: "127.0.0.1:3000".parse().expect("valid default address"),
            site_title: "Glimpse".to_owned(),
            posts_dir: PathBuf::from("posts"),
            cache_dir: PathBuf::from("cache"),
            video_max_height: 1080,
            preprocess_concurrency: 2,
        }
    }
}

impl Config {
    /// Load from `path`. Returns [`Config::default`] if the file does not exist.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
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
