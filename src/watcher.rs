use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use arc_swap::ArcSwap;
use notify::{RecursiveMode, Watcher};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tracing::{error, info};

use crate::content::{self, Post, Site};
use crate::media::{ImageSize, MediaCache};

/// Maximum number of derivative images generated concurrently during a reload.
/// Keeping this low ensures background pre-processing does not starve request
/// serving while new content is being prepared.
const PREPROCESS_CONCURRENCY: usize = 2;

/// Spawn a background thread that watches `posts_dir` for filesystem changes.
///
/// On each change the thread re-parses only the affected post directories,
/// pre-generates their image derivatives at limited concurrency, then
/// atomically swaps in the updated `Site`. Clients never see a post whose
/// images are not yet ready.
///
/// Reload errors are logged but leave the previous `Site` live.
pub fn spawn(posts_dir: PathBuf, site: Arc<ArcSwap<Site>>, media_cache: Arc<MediaCache>) {
    let handle = tokio::runtime::Handle::current();
    std::thread::spawn(move || run(&posts_dir, &site, &media_cache, &handle));
}

fn run(
    posts_dir: &Path,
    site: &Arc<ArcSwap<Site>>,
    media_cache: &Arc<MediaCache>,
    handle: &tokio::runtime::Handle,
) {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut watcher = match notify::recommended_watcher(tx) {
        Ok(w) => w,
        Err(e) => {
            error!(error = %e, "failed to initialise watcher");
            return;
        }
    };
    if let Err(e) = watcher.watch(posts_dir, RecursiveMode::Recursive) {
        error!(path = %posts_dir.display(), error = %e, "failed to watch path");
        return;
    }
    info!(path = %posts_dir.display(), "watching for changes");

    loop {
        // Wait for the first event of a batch.
        let first = match rx.recv() {
            Err(_) => break,
            Ok(result) => result,
        };

        let mut paths: Vec<PathBuf> = match first {
            Err(e) => {
                error!(error = %e, "watcher error");
                continue;
            }
            Ok(event) => event.paths,
        };

        // Debounce: drain any events that arrive within 300 ms.
        std::thread::sleep(Duration::from_millis(300));
        while let Ok(result) = rx.try_recv() {
            if let Ok(event) = result {
                paths.extend(event.paths);
            }
        }

        // Resolve each event path to its immediate post directory under posts_dir.
        let affected: HashSet<PathBuf> = paths
            .iter()
            .filter_map(|p| post_dir_for_path(posts_dir, p))
            .collect();

        if affected.is_empty() {
            continue;
        }

        // Clone all posts that were not affected.
        let current = site.load();
        let mut posts: Vec<Post> = current
            .posts
            .iter()
            .filter(|p| !affected.contains(&p.source_dir))
            .cloned()
            .collect();

        // Re-parse each affected directory that still exists.
        let mut changed: Vec<Post> = Vec::new();
        for post_dir in &affected {
            if post_dir.is_dir() {
                match content::parse_post(post_dir) {
                    Ok(post) => {
                        info!(slug = %post.slug, "reloaded post");
                        changed.push(post);
                    }
                    Err(e) => {
                        error!(path = %post_dir.display(), error = %e, "failed to parse post");
                    }
                }
            } else {
                info!(path = %post_dir.display(), "post removed");
            }
        }

        handle.block_on(preprocess_derivatives(&changed, media_cache));

        posts.extend(changed);
        posts.sort_by(|a, b| a.date.cmp(&b.date));

        info!(count = posts.len(), "updated site");
        site.store(Arc::new(Site { posts }));
    }
}

/// Return the immediate child of `posts_dir` that contains `event_path`,
/// or `None` if `event_path` is not inside `posts_dir` or is `posts_dir` itself.
fn post_dir_for_path(posts_dir: &Path, event_path: &Path) -> Option<PathBuf> {
    let rel = event_path.strip_prefix(posts_dir).ok()?;
    let first = rel.components().next()?;
    Some(posts_dir.join(first))
}

/// Pre-generate thumbnail and medium derivatives for the given posts.
///
/// At most `PREPROCESS_CONCURRENCY` images are generated at once. Already-cached
/// derivatives are skipped cheaply by `MediaCache::ensure`. The function returns
/// only after every derivative is ready, so the caller can safely swap the site.
async fn preprocess_derivatives(posts: &[Post], media_cache: &Arc<MediaCache>) {
    let work: Vec<(PathBuf, ImageSize)> = posts
        .iter()
        .flat_map(|p| p.photo_groups.iter())
        .flat_map(|g| g.media.iter())
        .filter(|item| !item.is_video)
        .flat_map(|item| {
            [ImageSize::Thumbnail, ImageSize::Medium]
                .map(|size| (item.path.clone(), size))
                .into_iter()
        })
        .collect();

    if work.is_empty() {
        return;
    }

    info!(
        count = work.len(),
        concurrency = PREPROCESS_CONCURRENCY,
        "pre-processing derivatives"
    );

    let sem = Arc::new(Semaphore::new(PREPROCESS_CONCURRENCY));
    let mut set = JoinSet::new();

    for (photo, size) in work {
        let sem = Arc::clone(&sem);
        let cache = Arc::clone(media_cache);
        set.spawn(async move {
            let _permit = sem.acquire().await.unwrap();
            if let Err(e) = cache.ensure(&photo, size).await {
                error!(path = %photo.display(), error = %e, "pre-process failed");
            }
        });
    }

    while set.join_next().await.is_some() {}
    info!("pre-processing complete");
}
