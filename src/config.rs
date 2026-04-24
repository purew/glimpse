use std::path::Path;

use serde::Deserialize;
use tracing::info;

fn default_video_max_height() -> u32 {
    1080
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Videos taller than this are skipped at load time.
    #[serde(default = "default_video_max_height")]
    pub video_max_height: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            video_max_height: default_video_max_height(),
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
