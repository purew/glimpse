mod content;
mod server;
mod theme;
mod viewer;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let posts_dir = PathBuf::from("posts");
    let theme_dir = PathBuf::from("themes/default");

    let site = content::load_site(&posts_dir).context("failed to load site")?;
    println!("Loaded {} post(s)", site.posts.len());

    let theme = theme::Theme::load(&theme_dir);

    let state = server::AppState {
        site: Arc::new(site),
        theme: Arc::new(theme),
    };

    let app = server::router(state, theme_dir.join("static"));
    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    println!("Listening on http://{addr}");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
