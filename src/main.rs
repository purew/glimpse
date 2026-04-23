mod content;

use anyhow::Context;
use std::path::Path;

fn main() -> anyhow::Result<()> {
    let posts_dir = Path::new("posts");
    let site = content::load_site(posts_dir).context("failed to load site")?;

    println!("Site loaded: {} post(s)\n", site.posts.len());

    for post in &site.posts {
        println!("slug:   {}", post.slug);
        println!("title:  {}", post.title);
        println!("date:   {}", post.date);
        println!("draft:  {}", post.is_draft());
        println!("access: {:?}", post.access);
        println!("groups: {}", post.photo_groups.len());
        for group in &post.photo_groups {
            let label = if group.name.is_empty() { "(flat)" } else { &group.name };
            println!("  [{label}] {} photo(s)", group.photos.len());
        }
        println!("html:   {} chars", post.body_html.len());
        println!();
    }

    Ok(())
}
