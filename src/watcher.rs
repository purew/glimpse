use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use arc_swap::ArcSwap;
use notify::{RecursiveMode, Watcher};

use crate::content::{self, Site};

/// Spawn a background thread that watches `posts_dir` for filesystem changes
/// and atomically replaces `site` on each reload.
///
/// Errors from a single reload are logged but do not stop the watcher; the
/// previous `Site` stays live until a successful reload replaces it.
pub fn spawn(posts_dir: PathBuf, site: Arc<ArcSwap<Site>>) {
    std::thread::spawn(move || run(&posts_dir, &site));
}

fn run(posts_dir: &Path, site: &Arc<ArcSwap<Site>>) {
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
                println!("Reloaded site ({} post(s))", new_site.posts.len());
                site.store(Arc::new(new_site));
            }
            Err(e) => eprintln!("watcher: reload failed: {e}"),
        }
    }
}
