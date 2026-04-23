mod content;
mod theme;
mod viewer;

use std::fs;
use std::path::Path;

use anyhow::Context;

use theme::Theme;
use viewer::{Viewer, visible};

fn main() -> anyhow::Result<()> {
    let posts_dir = Path::new("posts");
    let theme_dir = Path::new("themes/default");
    let output_dir = Path::new("output");

    let site = content::load_site(posts_dir).context("failed to load site")?;
    println!("Loaded {} post(s)", site.posts.len());

    let theme = Theme::load(theme_dir);

    fs::create_dir_all(output_dir).context("failed to create output dir")?;
    fs::create_dir_all(output_dir.join("posts")).context("failed to create output/posts dir")?;

    // Render index as admin so all posts (including drafts) appear.
    let admin = Viewer::admin();
    let index_html = theme.render_index(&site, &admin).context("failed to render index")?;
    let index_path = output_dir.join("index.html");
    fs::write(&index_path, &index_html).context("failed to write index.html")?;
    println!("Wrote {}", index_path.display());

    for post in visible(&site, &admin) {
        let post_html =
            theme.render_post(post, &admin).with_context(|| format!("render post '{}'", post.slug))?;
        let post_path = output_dir.join("posts").join(format!("{}.html", post.slug));
        fs::write(&post_path, &post_html)
            .with_context(|| format!("write {}", post_path.display()))?;
        println!("Wrote {}", post_path.display());
    }

    let login_html = theme.render_login().context("failed to render login")?;
    let login_path = output_dir.join("login.html");
    fs::write(&login_path, &login_html).context("failed to write login.html")?;
    println!("Wrote {}", login_path.display());

    println!("\nDone. Open output/index.html to review.");
    Ok(())
}
