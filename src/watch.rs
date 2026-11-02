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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

use notify::event::{AccessKind, AccessMode, EventKind, ModifyKind, RemoveKind};
use notify::{ErrorKind, RecommendedWatcher, RecursiveMode, Watcher as _};

/// How often the directory is looked for when nothing else says it went.
///
/// A watch that ended is not something every backend reports, or reports with
/// the path the window named: Windows answers a deleted directory by dropping
/// the watch without a word to the handler, and FSEvents names the path the
/// kernel resolved, which a symbolic link in the path does not match. Asking
/// the filesystem covers both, on a thread of its own so that a mount which
/// stopped answering cannot hold a frame.
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
    /// Set when the watch is dropped, so the thread looking for the directory
    /// ends with it rather than one a second for the life of the window.
    stop: Arc<AtomicBool>,
}

impl Watch {
    /// Whether the watch on the directory ended, and a new one is needed.
    ///
    /// What the backend reported is taken first, and the directory itself is
    /// looked for once a second for the backends that report nothing.
    pub fn ended(&self) -> bool {
        self.ended.load(Ordering::Relaxed)
    }
    /// Whether anything under the directory changed since the last call.
    ///
    /// Only the events that mean a change reach here: a read of the library
    /// wakes nothing, so a scan of it does not ask for another.
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

impl Drop for Watch {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Whether the directory is gone, taking the watch with it.
///
/// A stat rather than an event, because the event is not to be had
/// everywhere; an error is not the directory going away either, since a
/// permission that comes and goes still leaves it there to report changes
/// under.
fn looked_for(root: &Path, ended: &AtomicBool) -> bool {
    if ended.load(Ordering::Relaxed) {
        return true;
    }
    if !matches!(root.try_exists(), Ok(false)) {
        return false;
    }
    ended.store(true, Ordering::Relaxed);
    true
}

/// Watches `directory` and everything under it, waking the window on a change.
///
/// Answers `None` when the directory cannot be watched at all: no backend for
/// it, or no watches left for it. A tree with more directories than the kernel
/// keeps watches for is not that: it is watched as far as the watches go, the
/// rest is left to the fallback scan, and the window is never asked to place
/// one watch per frame for it.
pub fn spawn(directory: &Path, wake: Waker) -> Option<Watch> {
    let (sender, events) = mpsc::channel();
    let ended = Arc::new(AtomicBool::new(false));
    let ended_by_event = Arc::clone(&ended);
    let stop = Arc::new(AtomicBool::new(false));
    let root = directory.to_path_buf();
    let resolved = root.canonicalize().unwrap_or_else(|_| root.clone());
    let watched = root.clone();
    let watched_resolved = resolved.clone();
    let wake_on_event = Arc::clone(&wake);
    // The tree is walked again from the top by a watch placed here, so a
    // reason to place one every frame is a reason not to watch at all: what
    // reports is kept, and the fallback scan carries what does not.
    let mut watcher = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
        match result {
            // The directory itself going away ends the watch, and the kernel
            // says so once: a watch that is not replaced watches nothing.
            Ok(event) if ends_the_watch(&event, &watched, &watched_resolved) => {
                ended_by_event.store(true, Ordering::Relaxed);
                if sender.send(()).is_ok() {
                    wake_on_event();
                }
            }
            // Reading the library is not a change to it, and a scan is nothing
            // but reads.
            Ok(event) if !changes_the_library(&event) => {}
            Ok(_) => {
                if sender.send(()).is_ok() {
                    wake_on_event();
                }
            }
            // The kernel has no watches left for a tree this large. What is
            // watched is still reported and the rest arrives with the
            // fallback scan, which is what the hourly one is for.
            Err(error) if matches!(error.kind, ErrorKind::MaxFilesWatch) => {}
            // Anything else is a read that failed: the files it could not see
            // arrive with a full scan, so that is what it asks for.
            Err(_) => {
                if sender.send(()).is_ok() {
                    wake_on_event();
                }
            }
        }
    })
    .ok()?;
    watcher.watch(directory, RecursiveMode::Recursive).ok()?;
    let look_root = root.clone();
    let look_ended = Arc::clone(&ended);
    let look_stop = Arc::clone(&stop);
    let look_wake = Arc::clone(&wake);
    let _ = thread::Builder::new()
        .name("blue-dove watch look".to_string())
        .spawn(move || {
            while !look_stop.load(Ordering::Relaxed) {
                thread::sleep(CHECK_EVERY);
                if looked_for(&look_root, &look_ended) {
                    look_wake();
                    return;
                }
            }
        });
    Some(Watch {
        root,
        _watcher: watcher,
        events,
        ended,
        stop,
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

/// Whether `event` says the library changed, rather than that it was read.
///
/// A scan opens every directory it walks and every file whose tags it reads,
/// and the backends report some of that as events of their own: inotify hands
/// over an open, and Windows calls a directory that was only read modified.
/// Counting those would have every scan ask for the next one five seconds
/// later, for ever, with nothing changing.
///
/// A file closed after a write is a change, and so is a file whose metadata
/// moved, since a retag is a write. A directory that is only touched is not:
/// what changes inside it arrives named after the file or directory that
/// appeared, went or was renamed.
fn changes_the_library(event: &notify::Event) -> bool {
    match event.kind {
        EventKind::Access(AccessKind::Close(AccessMode::Write)) => true,
        EventKind::Access(_) => false,
        EventKind::Modify(ModifyKind::Any | ModifyKind::Metadata(_)) => {
            event.paths.iter().any(|path| !path.is_dir())
        }
        _ => true,
    }
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

    /// Waits until nothing has been reported for a while.
    ///
    /// One write is more than one event, and the platform may hand them over in
    /// batches a moment apart: a test that counts what arrives after a change
    /// has to let the change finish.
    fn quiet(watch: &Watch) {
        let mut last = Instant::now();
        while last.elapsed() < Duration::from_millis(300) {
            if watch.changed() {
                last = Instant::now();
            }
            std::thread::sleep(Duration::from_millis(20));
        }
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
        // looked for instead. What the filesystem answers is what is tested
        // here, a second apart on a thread of its own in `spawn`.
        let scratch = Scratch::new("gone");
        let ended = AtomicBool::new(false);
        assert!(!looked_for(&scratch.0, &ended), "the directory is there");

        std::fs::remove_dir_all(&scratch.0).expect("the directory goes");

        assert!(looked_for(&scratch.0, &ended), "and is found gone");
        assert!(
            ended.load(Ordering::Relaxed),
            "which the watch remembers without asking again"
        );
    }

    #[test]
    fn reading_the_library_is_not_a_change() {
        // A scan opens every directory it walks and every file whose tags it
        // reads, and inotify hands those opens over as events: counting them
        // has every scan ask for the next one, five seconds later, for ever.
        let scratch = Scratch::new("changing");
        let path = scratch.0.join("song.wav");
        std::fs::write(&path, b"not really audio").expect("the file is written");
        let album = scratch.0.join("An Album");
        std::fs::create_dir_all(&album).expect("the album directory is made");
        let opened = notify::Event::new(EventKind::Access(AccessKind::Open(AccessMode::Any)))
            .add_path(path.clone());
        let written = notify::Event::new(EventKind::Access(AccessKind::Close(AccessMode::Write)))
            .add_path(path.clone());
        let retagged = notify::Event::new(EventKind::Modify(ModifyKind::Metadata(
            notify::event::MetadataKind::Any,
        )))
        .add_path(path.clone());
        let created =
            notify::Event::new(EventKind::Create(notify::event::CreateKind::File)).add_path(path);
        let touched_directory =
            notify::Event::new(EventKind::Modify(ModifyKind::Any)).add_path(album.clone());

        assert!(!changes_the_library(&opened), "an open is a read");
        assert!(
            changes_the_library(&written),
            "a file closed after a write changed"
        );
        assert!(changes_the_library(&retagged), "a retag changed the tags");
        assert!(changes_the_library(&created), "a new file is a change");
        assert!(
            !changes_the_library(&touched_directory),
            "a directory that was read is not a change: what is in it arrives named"
        );
    }

    #[test]
    fn a_scan_of_the_library_leaves_the_watch_quiet() {
        // The loop this exists to keep out, for three different reasons: the
        // inotify walk reports every open it makes, Windows calls a directory
        // that was read modified, and FSEvents says nothing about a read at
        // all. Walking the tree and reading a file is not a change, so it
        // does not ask for the scan that would read it again.
        let scratch = Scratch::new("scanning");
        let album = scratch.0.join("An Album");
        std::fs::create_dir_all(&album).expect("the album directory is made");
        std::fs::write(album.join("song.wav"), b"not really audio").expect("the file is written");
        let Some(watch) = spawn(&scratch.0, Arc::new(|| {})) else {
            return;
        };

        // The watch has to be alive for the rest of this to say anything, and
        // a file that appears is what says so.
        std::fs::write(scratch.0.join("new.wav"), b"not really audio")
            .expect("the file is written");
        assert!(settle(&watch), "a new file is reported");
        quiet(&watch);

        let mut directories = vec![scratch.0.clone()];
        while let Some(directory) = directories.pop() {
            for entry in std::fs::read_dir(&directory).expect("the directory reads") {
                let path = entry.expect("the entry reads").path();
                if path.is_dir() {
                    directories.push(path);
                } else {
                    std::fs::read(&path).expect("the file reads");
                }
            }
        }

        // The reads are taken in and dropped, or never arrive; either way the
        // window is not asked to scan.
        std::thread::sleep(Duration::from_millis(500));
        assert!(!watch.changed(), "a scan of the library is not a change");
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
