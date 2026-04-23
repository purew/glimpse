use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use arc_swap::ArcSwap;
use notify::{RecursiveMode, Watcher};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::content::{self, Site};
use crate::media::{ImageSize, MediaCache};

/// Maximum number of derivative images generated concurrently during a reload.
/// Keeping this low ensures background pre-processing does not starve request
/// serving while new content is being prepared.
const PREPROCESS_CONCURRENCY: usize = 2;

/// Spawn a background thread that watches `posts_dir` for filesystem changes.
///
/// On each change the thread rebuilds `Site`, pre-generates all image
/// derivatives at limited concurrency, then atomically swaps in the new `Site`.
/// Clients never see a post whose images are not yet ready.
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
            eprintln!("watcher: failed to initialise: {e}");
            return;
        }
    };
    if let Err(e) = watcher.watch(posts_dir, RecursiveMode::Recursive) {
        eprintln!("watcher: failed to watch {}: {e}", posts_dir.display());
        return;
    }
    println!("Watching {} for changes", posts_dir.display());

    loop {
        match rx.recv() {
            Err(_) => break,
            Ok(Err(e)) => {
                eprintln!("watcher: {e}");
                continue;
            }
            Ok(Ok(_)) => {}
        }
        // Debounce: drain any events that arrive within 300 ms.
        std::thread::sleep(Duration::from_millis(300));
        while rx.try_recv().is_ok() {}

        match content::load_site(posts_dir) {
            Ok(new_site) => {
                handle.block_on(preprocess_derivatives(&new_site, media_cache));
                println!("Reloaded site ({} post(s))", new_site.posts.len());
                site.store(Arc::new(new_site));
            }
            Err(e) => eprintln!("watcher: reload failed: {e}"),
        }
    }
}

/// Pre-generate all thumbnail and medium derivatives for every photo in `site`.
///
/// At most `PREPROCESS_CONCURRENCY` images are generated at once. Already-cached
/// derivatives are skipped cheaply by `MediaCache::ensure`. The function returns
/// only after every derivative is ready, so the caller can safely swap the site.
async fn preprocess_derivatives(site: &Site, media_cache: &Arc<MediaCache>) {
    let work: Vec<(PathBuf, ImageSize)> = site
        .posts
        .iter()
        .flat_map(|p| p.photo_groups.iter())
        .flat_map(|g| g.photos.iter())
        .flat_map(|photo| {
            [ImageSize::Thumbnail, ImageSize::Medium]
                .map(|size| (photo.clone(), size))
                .into_iter()
        })
        .collect();

    if work.is_empty() {
        return;
    }

    println!(
        "watcher: pre-processing {} derivatives (concurrency {})",
        work.len(),
        PREPROCESS_CONCURRENCY
    );

    let sem = Arc::new(Semaphore::new(PREPROCESS_CONCURRENCY));
    let mut set = JoinSet::new();

    for (photo, size) in work {
        let sem = Arc::clone(&sem);
        let cache = Arc::clone(media_cache);
        set.spawn(async move {
            let _permit = sem.acquire().await.unwrap();
            if let Err(e) = cache.ensure(&photo, size).await {
                eprintln!("watcher: pre-process failed for {}: {e}", photo.display());
            }
        });
    }

    while set.join_next().await.is_some() {}
    println!("watcher: pre-processing complete");
}
