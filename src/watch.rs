//! Watching the music directory, so a file added outside the window shows up.
//!
//! A scan costs a `stat` per file of the whole library whether or not
//! anything changed, which is why the timer in [`crate::app::SCAN_EVERY`] runs
//! an hour apart: the watcher is what notices a change at all, and the timer
//! is only the fallback for storage that reports none, or for a tree with more
//! directories than the kernel keeps watches for.
//!
//! What a change means here is only that something under the directory moved,
//! appeared or was written; the directory is then scanned the way a rescan by
//! hand scans it. The kernel knows which files changed, but the cache and the
//! library are keyed by what a full read finds, so one scan after the burst is
//! simpler and no slower than patching the library from events.
//!
//! Events arrive on a thread of notify's own, one per file of a copy, so the
//! window is only woken: it coalesces them into one scan once they stop.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};

use notify::{RecommendedWatcher, RecursiveMode, Watcher as _};

/// Called after an event, so the window draws and takes it in.
pub type Waker = Arc<dyn Fn() + Send + Sync>;

/// A directory watched for changes, for as long as it is held.
pub struct Watch {
    /// The directory watched, so a new root replaces the old watch.
    root: PathBuf,
    /// The watcher, kept alive by being held: dropping it ends the watches.
    _watcher: RecommendedWatcher,
    /// Events seen and not taken yet.
    events: Receiver<()>,
}

impl Watch {
    /// Whether anything under the directory changed since the last call.
    ///
    /// Every event the platform reported counts as one, since they are all
    /// the same thing here: a reason to scan once they stop.
    pub fn changed(&self) -> bool {
        let mut changed = false;
        while self.events.try_recv().is_ok() {
            changed = true;
        }
        changed
    }

    /// The directory watched.
    pub fn root(&self) -> &Path {
        &self.root
    }
}

/// Watches `directory` and everything under it, waking the window on a change.
///
/// Answers `None` when the directory cannot be watched: a tree with more
/// directories than the kernel keeps watches for, or a storage driver that
/// cannot report changes. The fallback scan then carries the library on its
/// own, which is what makes this worth failing quietly rather than refusing
/// to open the window.
pub fn spawn(directory: &Path, wake: Waker) -> Option<Watch> {
    let (sender, events) = mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
        // A backend that could not watch something answers with an error
        // rather than an event, which leaves nothing for the window to take
        // in: the file it could not see arrives with the next full scan.
        if result.is_ok() && sender.send(()).is_ok() {
            wake();
        }
    })
    .ok()?;
    watcher.watch(directory, RecursiveMode::Recursive).ok()?;
    Some(Watch {
        root: directory.to_path_buf(),
        _watcher: watcher,
        events,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// A directory of its own, removed when the test ends.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("blue-dove-watch-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("the directory is made");
            Self(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Waits until `watch` reports a change, or the deadline passes.
    fn settle(watch: &Watch) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if watch.changed() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    #[test]
    fn a_file_added_under_the_directory_is_reported() {
        // The fallback scan would find it an hour later; the watcher is what
        // makes a file that arrived while the window ran show up at once.
        let scratch = Scratch::new("added");
        let Some(watch) = spawn(&scratch.0, Arc::new(|| {})) else {
            // A host that cannot watch a directory has only the fallback, and
            // that is a case the app is built for rather than a failure here.
            return;
        };

        std::fs::write(scratch.0.join("song.wav"), b"not really audio")
            .expect("the file is written");

        assert!(settle(&watch), "the new file is reported");
        assert!(
            !watch.changed(),
            "the events of that one file are taken in one go"
        );
    }

    #[test]
    fn a_file_added_below_the_directory_is_reported() {
        // A scan walks the tree, so a watch that only sees the top directory
        // would miss every album: it has to be recursive.
        let scratch = Scratch::new("nested");
        let album = scratch.0.join("An Album");
        std::fs::create_dir_all(&album).expect("the album directory is made");
        let Some(watch) = spawn(&scratch.0, Arc::new(|| {})) else {
            return;
        };

        std::fs::write(album.join("song.wav"), b"not really audio").expect("the file is written");

        assert!(settle(&watch), "a file two levels down is reported");
    }

    #[test]
    fn the_watched_directory_is_named() {
        let scratch = Scratch::new("root");
        let Some(watch) = spawn(&scratch.0, Arc::new(|| {})) else {
            return;
        };
        assert_eq!(watch.root(), scratch.0, "the root is what was asked for");
    }
}
