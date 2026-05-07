mod config;
mod content;
mod media;
mod server;
mod theme;
mod users;
mod viewer;

use std::path::PathBuf;

use clap::Parser;

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
    server::run(args.config, args.users).await
}
