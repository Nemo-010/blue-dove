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

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use notify::event::{EventKind, ModifyKind, RemoveKind};
use notify::{RecommendedWatcher, RecursiveMode, Watcher as _};

/// How often the directory is looked for when nothing else says it went.
///
/// A watch that ended is not something every backend reports, or reports with
/// the path the window named: Windows answers a deleted directory by dropping
/// the watch without a word to the handler, and FSEvents names the path the
/// kernel resolved, which a symbolic link in the path does not match. Asking
/// the filesystem covers both, and one `stat` a second on the directory the
/// library lives in costs nothing beside the scan that follows it.
const CHECK_EVERY: Duration = Duration::from_secs(1);

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
    /// Whether the watch ended, which the kernel may never say.
    ///
    /// A directory that is deleted, renamed or unmounted takes its watch with
    /// it, and nothing under that path is reported again: the window has to
    /// place a new one, and this is how it learns to.
    ended: Arc<AtomicBool>,
    /// When the directory was last looked for, so that it is not on every
    /// frame.
    looked: Cell<Instant>,
}

impl Watch {
    /// Whether the watch on the directory ended, and a new one is needed.
    ///
    /// What the backend reported is taken first, and the directory itself is
    /// looked for once a second for the backends that report nothing.
    pub fn ended(&self) -> bool {
        if self.ended.load(Ordering::Relaxed) {
            return true;
        }
        let now = Instant::now();
        if now.saturating_duration_since(self.looked.get()) < CHECK_EVERY {
            return false;
        }
        self.looked.set(now);
        // An error is not the directory going away: a permission that comes
        // and goes still leaves it there to report changes under.
        if !matches!(self.root.try_exists(), Ok(false)) {
            return false;
        }
        self.ended.store(true, Ordering::Relaxed);
        true
    }
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
    let ended = Arc::new(AtomicBool::new(false));
    let ended_by_event = Arc::clone(&ended);
    let root = directory.to_path_buf();
    let resolved = root.canonicalize().unwrap_or_else(|_| root.clone());
    let watched = root.clone();
    let watched_resolved = resolved.clone();
    let mut watcher = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
        match result {
            // The directory itself going away ends the watch, and the kernel
            // says so once: a watch that is not replaced watches nothing.
            Ok(event) if ends_the_watch(&event, &watched, &watched_resolved) => {
                ended_by_event.store(true, Ordering::Relaxed);
                if sender.send(()).is_ok() {
                    wake();
                }
            }
            Ok(_) => {
                if sender.send(()).is_ok() {
                    wake();
                }
            }
            // A backend that could not watch something answers with an error
            // rather than an event, which leaves nothing for the window to
            // take in: the file it could not see arrives with the next full
            // scan, and the watch is placed again.
            Err(_) => {
                ended_by_event.store(true, Ordering::Relaxed);
                if sender.send(()).is_ok() {
                    wake();
                }
            }
        }
    })
    .ok()?;
    watcher.watch(directory, RecursiveMode::Recursive).ok()?;
    Some(Watch {
        root,
        _watcher: watcher,
        events,
        ended,
        looked: Cell::new(Instant::now()),
    })
}

/// Whether `event` says the watched directory itself is gone.
///
/// A watch is placed on the directory and on everything below it; the one on
/// the directory ends when the directory is removed or renamed away, and the
/// kernel reports that once. Anything else is a change to take in.
fn ends_the_watch(event: &notify::Event, root: &Path, resolved: &Path) -> bool {
    let gone = matches!(
        event.kind,
        EventKind::Remove(RemoveKind::Folder | RemoveKind::Any)
            | EventKind::Modify(ModifyKind::Name(_))
    );
    gone && event
        .paths
        .iter()
        .any(|path| path == root || path == resolved)
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
    fn the_watched_directory_going_away_ends_the_watch() {
        // A watch on a directory that is removed or moved away watches
        // nothing, and the kernel says so once. The window reads this to
        // place a new one, since nothing under the old path is reported.
        //
        // The other names are the same directory: FSEvents reports the path
        // the kernel resolved, which on macOS is what a watch placed through
        // the `/var` symbolic link gets.
        let root = Path::new("/var/music");
        let resolved = Path::new("/private/var/music");
        let removed = notify::Event::new(EventKind::Remove(RemoveKind::Folder))
            .add_path(PathBuf::from("/var/music"));
        let removed_resolved = notify::Event::new(EventKind::Remove(RemoveKind::Folder))
            .add_path(PathBuf::from("/private/var/music"));
        let moved = notify::Event::new(EventKind::Modify(ModifyKind::Name(
            notify::event::RenameMode::From,
        )))
        .add_path(PathBuf::from("/var/music"));
        let file = notify::Event::new(EventKind::Remove(RemoveKind::File))
            .add_path(PathBuf::from("/var/music/song.wav"));
        let written = notify::Event::new(EventKind::Modify(ModifyKind::Data(
            notify::event::DataChange::Any,
        )))
        .add_path(PathBuf::from("/var/music/song.wav"));

        assert!(
            ends_the_watch(&removed, root, resolved),
            "the directory was removed"
        );
        assert!(
            ends_the_watch(&removed_resolved, root, resolved),
            "or removed, named the way the kernel resolves it"
        );
        assert!(ends_the_watch(&moved, root, resolved), "or renamed away");
        assert!(
            !ends_the_watch(&file, root, resolved),
            "a file under it is a change, not the end of the watch"
        );
        assert!(
            !ends_the_watch(&written, root, resolved),
            "and so is a write"
        );
    }

    #[test]
    fn a_directory_that_is_gone_ends_the_watch_unasked() {
        // Windows ends the watch on a directory that is deleted without a
        // word to the handler, so nothing arrives to say so: the directory is
        // looked for instead, which is what this places no watch around.
        let scratch = Scratch::new("gone");
        let (_, events) = mpsc::channel();
        let watch = Watch {
            root: scratch.0.clone(),
            _watcher: notify::recommended_watcher(|_| {}).expect("a watcher is made"),
            events,
            ended: Arc::new(AtomicBool::new(false)),
            looked: Cell::new(Instant::now()),
        };
        assert!(!watch.ended(), "the directory is there to begin with");

        std::fs::remove_dir_all(&scratch.0).expect("the directory goes");
        let deadline = Instant::now() + Duration::from_secs(5);
        while !watch.ended() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }

        assert!(watch.ended(), "the directory is looked for, and found gone");
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
