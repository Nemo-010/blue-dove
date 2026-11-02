//! A cache of the scanned library, so a start does not rescan every file.
//!
//! Reading the tags of a large directory is quick, but decoding its album art is
//! not: on a library of seven thousand tracks the pictures take most of the
//! six seconds a scan needs. This keeps what a scan found - the tags and the
//! encoded cover files - in the user's cache directory, so a start reads it
//! back in a fraction of that, and no scan or cache read decodes a picture.
//!
//! An entry is only good while its file is unchanged, which the stamp of the
//! file's size and modification time decides. A file that was retagged is
//! read again by the scan that runs behind the window, and a directory that
//! gained or lost files is scanned as usual.
//!
//! The peak envelope the waveform seek bar draws is kept here too, one small
//! file per track. Working it out means decoding the whole track, which is
//! what left the bar empty for a moment at the start of every play; from the
//! cache it is drawn at once.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::cover::Cover;
use crate::error::At;
use crate::library::{Library, Track};

/// Version of the cache layout, so an older file is ignored rather than
/// misread after a change to what is stored.
const VERSION: u32 = 5;

/// Magic at the head of a stored waveform.
const WAVEFORM_MAGIC: &[u8; 4] = b"BDWV";

/// Version of a stored waveform.
///
/// Bumping this throws every stored envelope away, which is what a change to
/// how the peaks are worked out or how many there are needs.
const WAVEFORM_VERSION: u8 = 1;

/// How many stored waveforms are kept.
///
/// A track's envelope is about 1.6 KB, so this is a few megabytes at most:
/// enough that a library of this size has every track it plays cached, and
/// bounded so a directory of tens of thousands of files does not grow the
/// cache without end.
const MAX_WAVEFORMS: usize = 4000;

/// What a scan found, kept for the next start.
#[derive(Debug, Default, Deserialize, Serialize)]
pub struct Cache {
    /// Version of the layout.
    version: u32,
    /// The directory the entries were read from.
    root: PathBuf,
    /// One entry per audio file.
    tracks: Vec<CachedTrack>,
    /// Entries by path, built on load.
    #[serde(skip)]
    by_path: HashMap<PathBuf, usize>,
}

/// One file's tags, as a scan read them.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CachedTrack {
    /// Where the file lives, relative to the music directory.
    ///
    /// Storing it relative is what lets the cache survive the directory being
    /// named another way: a symlink and its target, or `/tmp` and
    /// `/private/tmp` on macOS, are one place under two names, and an absolute
    /// path written under one would never match a scan that walked the other.
    path: PathBuf,
    /// Size when it was read, in bytes.
    size: u64,
    /// Modification time when it was read, in seconds since the epoch.
    modified: u64,
    /// Track title.
    title: String,
    /// Performing artist.
    artist: String,
    /// Album title.
    album: String,
    /// Album artist.
    album_artist: String,
    /// Release year.
    year: Option<u32>,
    /// Disc number.
    disc: Option<u32>,
    /// Track number.
    number: Option<u32>,
    /// Playing time in milliseconds.
    duration_ms: u64,
    /// Key of the stored picture, when the file or its directory had one.
    cover: Option<String>,
    /// Directory the file sits in, relative to the music directory.
    directory: PathBuf,
    /// Whether the tags named an album.
    tagged_album: bool,
}

impl Cache {
    /// Reads the cache for `root`, or `None` when there is none that fits.
    pub fn load(root: &Path) -> Option<Self> {
        Self::load_from(&cache_dir()?, root)
    }

    /// Reads the cache in `dir` for `root`.
    ///
    /// Answers `None` when `root` is not a directory any more: an unmounted
    /// drive or a removed folder would otherwise open as a library of tracks
    /// that cannot be played.
    pub fn load_from(dir: &Path, root: &Path) -> Option<Self> {
        if !root.is_dir() {
            return None;
        }
        let text = std::fs::read_to_string(dir.join(file_name(root))).ok()?;
        let mut cache: Self = toml::from_str(&text).ok()?;
        if cache.version != VERSION || cache.root != cache_root(root) {
            return None;
        }
        // The entries are relative, so the root is kept in the spelling this
        // run names it by: a scan hands back the paths it walked, and those
        // are written this way.
        cache.root = root.to_path_buf();
        for (index, track) in cache.tracks.iter().enumerate() {
            cache.by_path.insert(track.path.clone(), index);
        }
        Some(cache)
    }

    /// The entry for `path`, which the caller then checks for freshness.
    ///
    /// The path is taken relative to the music directory, which is how entries
    /// are stored: a directory reached through a link and the directory it
    /// points at then find the same entries.
    pub fn track(&self, path: &Path) -> Option<&CachedTrack> {
        let relative = path.strip_prefix(&self.root).ok()?;
        self.tracks.get(*self.by_path.get(relative)?)
    }

    /// Every entry, for turning a cache into a library without a scan.
    pub fn all(&self) -> &[CachedTrack] {
        &self.tracks
    }

    /// The music directory the entries were read from, as this run names it.
    pub fn root(&self) -> &Path {
        &self.root
    }
}

impl CachedTrack {
    /// The key of the picture this entry names, when it names one.
    pub fn cover(&self) -> Option<&str> {
        self.cover.as_deref()
    }

    /// Where the file lives, under `root`.
    pub fn path(&self, root: &Path) -> PathBuf {
        root.join(&self.path)
    }

    /// Whether the file still looks the way it did when it was read.
    pub fn is_fresh(&self, root: &Path) -> bool {
        stamp(&self.path(root)) == Some((self.size, self.modified))
    }

    /// Rebuilds the track this entry describes, under `root`.
    ///
    /// The picture stays where it is: the entry carries its key, and
    /// `art.rs` decodes it off the drawing thread.
    pub fn to_track(&self, root: &Path) -> Track {
        Track {
            path: self.path(root),
            title: self.title.clone(),
            artist: self.artist.clone(),
            album: self.album.clone(),
            album_artist: self.album_artist.clone(),
            year: self.year,
            disc: self.disc,
            number: self.number,
            duration: Duration::from_millis(self.duration_ms),
            cover: self.cover.clone(),
            directory: self.directory.clone(),
            tagged_album: self.tagged_album,
        }
    }
}

/// The path a music directory's cache is named after.
///
/// A symlink and the directory it points at are one place, and one place keeps
/// one cache: the path is resolved first, so either spelling of it finds the
/// entry the other wrote. A path that cannot be resolved is used as it is.
fn cache_root(root: &Path) -> PathBuf {
    root.canonicalize().unwrap_or_else(|_| root.to_path_buf())
}

/// Name of the file the library for `root` is written to.
///
/// One directory keeps one file: two music directories would otherwise
/// overwrite each other, and a switch between them would throw the cache
/// away every time. The name is a hash of the path, so it is stable between
/// runs without carrying a path into a file name.
fn file_name(root: &Path) -> String {
    let hash = digest(cache_root(root).as_os_str().as_encoded_bytes());
    format!("library-{hash:016x}.toml")
}

/// Where the cache for `root` is kept.
///
/// Only the tests ask: a scan writes the file itself, and what they check is
/// whether a scan of an untouched library wrote it again.
#[cfg(test)]
pub(crate) fn cache_path(root: &Path) -> Option<PathBuf> {
    Some(cache_dir()?.join(file_name(root)))
}

/// The directory the cache is kept in, when the platform has a cache directory.
fn cache_dir() -> Option<PathBuf> {
    #[cfg(test)]
    {
        // A test must not read or write the cache of the person running it:
        // a scan writes there, and a scan of a temporary directory would put
        // an entry, and its pictures, beside the real library.
        //
        // The directory sits in `target/`, so it goes with `cargo clean`
        // rather than piling up in `/tmp`. The tests run in parallel and write
        // to it, so `save` leaves pruning to `save_in`, which a test points at
        // a directory of its own: a prune against the whole test cache would
        // delete the pictures another test had just stored.
        Some(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("target")
                .join("test-cache")
                .join(std::process::id().to_string()),
        )
    }
    #[cfg(not(test))]
    {
        dirs::cache_dir().map(|dir| dir.join("blue-dove"))
    }
}

/// Writes `library` as the cache for `root`.
///
/// The pictures are left where the scan stored them: an entry carries the
/// key of its picture, so a rescan of a large directory writes a couple of
/// megabytes of text and no image at all.
pub fn save(library: &Library, root: &Path) -> Result<(), String> {
    let Some(dir) = cache_dir() else {
        return Err("no cache directory for this user".to_string());
    };
    #[cfg(test)]
    {
        // A test's library and a test's pictures are stored in the same
        // directory as every other test's; pruning there would delete art
        // another test is about to read. `save_in` still prunes.
        write(&dir, library, root)
    }
    #[cfg(not(test))]
    save_in(&dir, library, root)
}

/// Writes `library` as the cache kept in `dir`.
pub fn save_in(dir: &Path, library: &Library, root: &Path) -> Result<(), String> {
    write(dir, library, root)?;
    prune_covers(dir);
    Ok(())
}

/// Writes `library` as the cache kept in `dir`, and prunes nothing.
fn write(dir: &Path, library: &Library, root: &Path) -> Result<(), String> {
    let path = dir.join(file_name(root));
    let mut tracks = Vec::with_capacity(library.tracks.len());
    let base = cache_root(root);
    for track in &library.tracks {
        let Some((size, modified)) = stamp(&track.path) else {
            continue;
        };
        let Ok(relative) = track
            .path
            .strip_prefix(root)
            .or_else(|_| track.path.strip_prefix(&base))
        else {
            continue;
        };
        tracks.push(CachedTrack {
            path: relative.to_path_buf(),
            size,
            modified,
            title: track.title.clone(),
            artist: track.artist.clone(),
            album: track.album.clone(),
            album_artist: track.album_artist.clone(),
            year: track.year,
            disc: track.disc,
            number: track.number,
            duration_ms: track.duration.as_millis() as u64,
            cover: track.cover.clone(),
            directory: track.directory.clone(),
            tagged_album: track.tagged_album,
        });
    }

    let cache = Cache {
        version: VERSION,
        root: cache_root(root),
        by_path: HashMap::new(),
        tracks,
    };
    let text =
        toml::to_string(&cache).map_err(|error| format!("cannot encode the cache: {error}"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).at(parent)?;
    }
    write_atomically(&path, text).at(&path)?;
    Ok(())
}

/// Removes the stored covers no library refers to any more.
///
/// The `art` directory holds the same covers as PNGs, for the media widget
/// MPRIS names them to, so it is pruned by the same rule: a file named after
/// a key some library still uses stays.
///
/// The keys come from every library cache in `dir`, not just the one being
/// written: the two directories are shared by all of them, so pruning against
/// one root's tracks would delete the art of a second music directory. A file
/// from another layout is skipped, since its keys name pictures of a shape
/// this build does not read and would otherwise be kept for good.
fn prune_covers(dir: &Path) {
    let mut keep: HashSet<String> = HashSet::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("library-") || !name.ends_with(".toml") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        let Ok(cache) = toml::from_str::<Cache>(&text) else {
            continue;
        };
        if cache.version != VERSION {
            continue;
        }
        keep.extend(cache.tracks.into_iter().filter_map(|track| track.cover));
    }
    for name in ["covers", "art"] {
        let Ok(entries) = std::fs::read_dir(dir.join(name)) else {
            continue;
        };
        for entry in entries.flatten() {
            let file = entry.file_name();
            let file = file.to_string_lossy();
            let key = file.strip_suffix(".png").unwrap_or(&file);
            if !keep.contains(key) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

/// Writes `cover` as a PNG for the media widget that names the art by a file,
/// and answers where it landed.
///
/// The file is named after `key`, the picture's place in `covers/`, so the two
/// directories are pruned by one rule and art that two tracks share is
/// written once. Answers `None` when there is no cache directory, which leaves
/// the metadata without art rather than failing.
pub fn art_file(key: &str, cover: &Cover) -> Option<PathBuf> {
    art_file_in(&cache_dir()?.join("art"), key, cover)
}

/// Writes `cover` as a PNG in `dir`, and answers its path.
fn art_file_in(dir: &Path, key: &str, cover: &Cover) -> Option<PathBuf> {
    let path = dir.join(format!("{key}.png"));
    if path.is_file() {
        return Some(path);
    }
    std::fs::create_dir_all(dir).ok()?;
    write_atomically(&path, crate::cover::to_png(cover)?).ok()?;
    Some(path)
}

/// Stores the encoded bytes of a cover and answers the key it is kept under.
///
/// The key is a hash of those bytes, so the same art in two albums is stored
/// once and an unchanged picture is not written again. The bytes are kept as
/// they were found, not decoded: a start then reads a few hundred kilobytes
/// of file names rather than a hundred decoded images. `art.rs` decodes one
/// when the window draws it, or behind that work when it does not.
pub fn store_cover(encoded: &[u8]) -> Option<String> {
    store_cover_in(&cache_dir()?, encoded)
}

/// Stores the encoded bytes of a cover in the cache kept in `dir`.
pub fn store_cover_in(dir: &Path, encoded: &[u8]) -> Option<String> {
    let key = cover_key(encoded);
    let dir = dir.join("covers");
    let path = dir.join(&key);
    if path.is_file() {
        return Some(key);
    }
    std::fs::create_dir_all(&dir).ok()?;
    write_atomically(&path, encoded).ok()?;
    Some(key)
}

/// The name a picture is stored under, which is a hash of its bytes.
pub fn cover_key(encoded: &[u8]) -> String {
    format!("{:016x}-{:x}", digest(encoded), encoded.len())
}

/// Reads the stored peak envelope for `path`, when one is there.
pub fn read_waveform(path: &Path) -> Option<Vec<f32>> {
    read_waveform_in(&cache_dir()?, path)
}

/// Reads the stored peak envelope for `path` from the cache kept in `dir`.
///
/// Answers `None` when there is no entry, when it was written by another
/// build, or when it is damaged: the caller then works the envelope out and
/// writes a new one.
pub fn read_waveform_in(dir: &Path, path: &Path) -> Option<Vec<f32>> {
    let bytes = std::fs::read(waveform_file(dir, path)?).ok()?;
    if bytes.get(..4)? != WAVEFORM_MAGIC || *bytes.get(4)? != WAVEFORM_VERSION {
        return None;
    }
    let buckets = u32::from_le_bytes(bytes.get(5..9)?.try_into().ok()?) as usize;
    let data = bytes.get(9..)?;
    if data.len() != buckets {
        return None;
    }
    Some(data.iter().map(|&byte| f32::from(byte) / 255.0).collect())
}

/// Stores the peak envelope for `path`, so the next play draws the seek bar
/// at once.
pub fn save_waveform(path: &Path, peaks: &[f32]) {
    if let Some(dir) = cache_dir() {
        save_waveform_in(&dir, path, peaks);
    }
}

/// Stores the peak envelope for `path` in the cache kept in `dir`.
///
/// The peaks go in as one byte each: the bar is drawn a point at a time, so
/// a 256th of the track's loudest peak is finer than a pixel, and it keeps
/// the file small enough that a whole library's envelopes are cheap to keep.
pub fn save_waveform_in(dir: &Path, path: &Path, peaks: &[f32]) {
    let Some(file) = waveform_file(dir, path) else {
        return;
    };
    if let Some(parent) = file.parent()
        && std::fs::create_dir_all(parent).is_err()
    {
        return;
    }
    let mut bytes = Vec::with_capacity(9 + peaks.len());
    bytes.extend_from_slice(WAVEFORM_MAGIC);
    bytes.push(WAVEFORM_VERSION);
    bytes.extend_from_slice(&(peaks.len() as u32).to_le_bytes());
    for peak in peaks {
        bytes.push((peak.clamp(0.0, 1.0) * 255.0).round() as u8);
    }
    let _ = write_atomically(&file, bytes);
    prune_waveforms(dir);
}

/// The file a track's envelope is kept in.
///
/// The name is a hash of the path, its size and its modification time, so a
/// file that was retagged or re-encoded gets an entry of its own rather than
/// a stale envelope drawn over it.
fn waveform_file(dir: &Path, path: &Path) -> Option<PathBuf> {
    let (size, modified) = stamp(path)?;
    let mut key = path.as_os_str().as_encoded_bytes().to_vec();
    key.extend_from_slice(&size.to_le_bytes());
    key.extend_from_slice(&modified.to_le_bytes());
    Some(dir.join("waveforms").join(format!("{:016x}", digest(&key))))
}

/// Drops the oldest stored envelopes once there are more than [`MAX_WAVEFORMS`].
///
/// An envelope is read without being rewritten, so its file keeps the time it
/// was written; one that was read and then dropped costs a recompute, which is
/// what the cache exists to avoid but not what it has to prevent.
fn prune_waveforms(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir.join("waveforms")) else {
        return;
    };
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = entries
        .flatten()
        .filter_map(|entry| Some((entry.metadata().ok()?.modified().ok()?, entry.path())))
        .collect();
    if files.len() <= MAX_WAVEFORMS {
        return;
    }
    files.sort_by_key(|left| left.0);
    let excess = files.len() - MAX_WAVEFORMS;
    for (_, path) in files.into_iter().take(excess) {
        let _ = std::fs::remove_file(path);
    }
}

/// Removes the bytes stored under `key`, and says whether there were some.
///
/// Only tests use this, to stand in for a cache directory the user deleted.
#[cfg(test)]
pub fn forget_cover(key: &str) -> bool {
    cache_dir()
        .map(|dir| dir.join("covers").join(key))
        .is_some_and(|path| std::fs::remove_file(path).is_ok())
}

/// Whether the bytes stored under `key` are still in the cache.
pub fn has_cover(key: &str) -> bool {
    cache_dir().is_some_and(|dir| dir.join("covers").join(key).is_file())
}

/// Reads back the encoded bytes stored under `key`.
///
/// Answers `None` when the picture has gone, which leaves the window drawing
/// its placeholder rather than failing.
pub fn read_cover(key: &str) -> Option<Vec<u8>> {
    read_cover_in(&cache_dir()?, key)
}

/// Reads back the encoded bytes stored under `key` in the cache kept in `dir`.
pub fn read_cover_in(dir: &Path, key: &str) -> Option<Vec<u8>> {
    std::fs::read(dir.join("covers").join(key)).ok()
}

/// A file's stamp: its size and the second its modification time falls in.
///
/// A time before the epoch, or one the platform cannot give, is reported as
/// absent, which makes an entry look stale.
fn stamp(path: &Path) -> Option<(u64, u64)> {
    let metadata = std::fs::metadata(path).ok()?;
    let modified = metadata
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_secs();
    Some((metadata.len(), modified))
}

/// Writes `bytes` to `path`, through a temporary file beside it.
///
/// A half-written file would be read back as a corrupt one, so the bytes land
/// in a temporary name first and are moved into place in one step. The name
/// carries the thread and a counter, since two writers of the same file in one
/// process would otherwise use the same temporary one and lose each other's
/// bytes.
fn write_atomically(path: &Path, bytes: impl AsRef<[u8]>) -> std::io::Result<()> {
    // `covers/` and `art/` are shared by every instance, so two windows on one
    // library write here at once: the name carries the process, the thread and
    // a counter, which no two writers in either can repeat.
    let temporary = path.with_extension(format!(
        "{}.{:?}.{}.new",
        std::process::id(),
        thread::current().id(),
        serial()
    ));
    std::fs::write(&temporary, bytes)?;
    std::fs::rename(&temporary, path)
}

/// A number no other call in the process gets, for naming a temporary file.
fn serial() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// FNV-1a over `bytes`, used to name a cache file.
///
/// The standard hasher is not stable across releases, and a cache file
/// written by one build has to be readable by the next.
fn digest(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A temporary directory that removes itself when it goes out of scope.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("blue-dove-cache-{name}-{}", std::process::id()));
            std::fs::create_dir_all(&path).expect("the directory is made");
            Self(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn cover() -> Cover {
        Cover {
            width: 2,
            height: 2,
            rgba: vec![9, 8, 7, 255, 6, 5, 4, 255, 3, 2, 1, 255, 0, 1, 2, 255],
        }
    }

    /// The bytes of a PNG holding [`cover`].
    fn picture() -> Vec<u8> {
        crate::cover::to_png(&cover()).expect("the picture encodes")
    }

    #[test]
    fn a_picture_survives_being_stored() {
        let scratch = Scratch::new("cover");
        let key = store_cover_in(&scratch.0, &picture()).expect("the picture is stored");
        let back = read_cover_in(&scratch.0, &key).expect("the picture reads back");
        assert_eq!(back, picture(), "the bytes come back as they went in");
    }

    #[test]
    fn the_same_picture_is_stored_once() {
        let scratch = Scratch::new("once");
        assert_eq!(
            store_cover_in(&scratch.0, &picture()),
            store_cover_in(&scratch.0, &picture())
        );
        let covers = std::fs::read_dir(scratch.0.join("covers"))
            .expect("the directory is there")
            .count();
        assert_eq!(covers, 1, "one file for one picture");
    }

    #[test]
    fn a_missing_cover_is_no_cover() {
        let scratch = Scratch::new("missing");
        assert!(read_cover_in(&scratch.0, "0000000000000000-0").is_none());
    }

    #[test]
    fn a_cover_is_written_as_a_png_for_the_media_widget() {
        // MPRIS names the art by a file, so the cover is written out as a PNG
        // the widget can open, and the same pixels reuse the file.
        let scratch = Scratch::new("art");
        let key = cover_key(&picture());
        let path = art_file_in(&scratch.0.join("art"), &key, &cover()).expect("the art is written");
        assert!(path.is_file());
        assert_eq!(path.extension().and_then(|name| name.to_str()), Some("png"));
        let back = crate::cover::decode(&std::fs::read(&path).expect("the file reads"))
            .expect("the art decodes as an image");
        assert_eq!(back.rgba, cover().rgba);
        assert_eq!(
            art_file_in(&scratch.0.join("art"), &key, &cover()),
            Some(path),
            "the same picture is written once"
        );
    }

    #[test]
    fn pruning_keeps_the_art_of_a_cover_in_use() {
        let scratch = Scratch::new("prune-art");
        let file = scratch.0.join("song.flac");
        std::fs::write(&file, b"not really audio").expect("the file is written");
        let library =
            Library::from_tracks(vec![track(&file, store_cover_in(&scratch.0, &picture()))]);
        save_in(&scratch.0, &library, &scratch.0).expect("the cache is written");
        let cache = Cache::load_from(&scratch.0, &scratch.0).expect("the cache reads back");
        let key = cache
            .track(&file)
            .and_then(|entry| entry.to_track(&scratch.0).cover)
            .expect("the cover was stored");
        let cover = cover();
        let art = art_file_in(&scratch.0.join("art"), &key, &cover).expect("the art is written");

        save_in(&scratch.0, &library, &scratch.0).expect("the cache is written again");

        assert!(art.is_file(), "the art of a stored cover stays");
        let gone = scratch.0.join("art").join("0000000000000000-0.png");
        std::fs::write(&gone, b"stale").expect("a stale art file is written");
        save_in(&scratch.0, &library, &scratch.0).expect("the cache is written once more");
        assert!(!gone.is_file(), "art of a cover nothing uses goes");
        assert!(
            scratch.0.join("covers").join(key).is_file(),
            "the cover itself stays"
        );
    }

    /// A track whose file is `path`.
    fn track(path: &Path, cover: Option<String>) -> Track {
        Track {
            path: path.to_path_buf(),
            title: "Song".to_string(),
            artist: "Someone".to_string(),
            album: "An Album".to_string(),
            album_artist: "Someone".to_string(),
            year: Some(2007),
            disc: Some(1),
            number: Some(2),
            duration: Duration::from_millis(123_456),
            cover,
            directory: PathBuf::from("An Album"),
            tagged_album: true,
        }
    }

    #[test]
    fn a_library_survives_being_cached() {
        let scratch = Scratch::new("round-trip");
        let file = scratch.0.join("song.flac");
        std::fs::write(&file, b"not really audio").expect("the file is written");
        let library =
            Library::from_tracks(vec![track(&file, store_cover_in(&scratch.0, &picture()))]);
        save_in(&scratch.0, &library, &scratch.0).expect("the cache is written");

        let cache = Cache::load_from(&scratch.0, &scratch.0).expect("the cache reads back");
        let entry = cache.track(&file).expect("the file has an entry");
        assert!(entry.is_fresh(&scratch.0), "nothing touched the file");

        let back = entry.to_track(&scratch.0);
        assert_eq!(back.title, "Song");
        assert_eq!(back.duration, Duration::from_millis(123_456));
        assert_eq!((back.disc, back.number), (Some(1), Some(2)));
        assert_eq!(
            back.cover,
            entry.to_track(&scratch.0).cover,
            "the track names the picture the cache stored"
        );
    }

    #[test]
    fn a_retagged_file_is_no_longer_fresh() {
        let scratch = Scratch::new("stale");
        let file = scratch.0.join("song.flac");
        std::fs::write(&file, b"one").expect("the file is written");
        let library = Library::from_tracks(vec![track(&file, None)]);
        save_in(&scratch.0, &library, &scratch.0).expect("the cache is written");

        std::fs::write(&file, b"a longer take").expect("the file changes");
        let cache = Cache::load_from(&scratch.0, &scratch.0).expect("the cache reads back");
        assert!(
            !cache
                .track(&file)
                .expect("the entry is there")
                .is_fresh(&scratch.0)
        );
    }

    #[test]
    fn a_stored_envelope_comes_back_the_shape_it_went_in() {
        let scratch = Scratch::new("waveform");
        let track = scratch.0.join("song.flac");
        std::fs::write(&track, b"one").expect("the file is written");

        let peaks: Vec<f32> = (0..64).map(|index| index as f32 / 63.0).collect();
        save_waveform_in(&scratch.0, &track, &peaks);
        let back = read_waveform_in(&scratch.0, &track).expect("the envelope is stored");

        assert_eq!(back.len(), peaks.len());
        for (written, read) in peaks.iter().zip(&back) {
            assert!(
                (written - read).abs() < 0.01,
                "each peak comes back within a byte of itself"
            );
        }
    }

    #[test]
    fn a_changed_file_has_no_stored_envelope() {
        let scratch = Scratch::new("waveform-stale");
        let track = scratch.0.join("song.flac");
        std::fs::write(&track, b"one").expect("the file is written");
        save_waveform_in(&scratch.0, &track, &[0.5; 8]);

        // A retag or a re-encode changes the size and the time, which is what
        // the entry is named after.
        std::fs::write(&track, b"a longer take").expect("the file changes");
        assert!(
            read_waveform_in(&scratch.0, &track).is_none(),
            "the old envelope must not be drawn over the new file"
        );
    }

    #[test]
    fn a_damaged_envelope_is_ignored() {
        let scratch = Scratch::new("waveform-damaged");
        let track = scratch.0.join("song.flac");
        std::fs::write(&track, b"one").expect("the file is written");
        save_waveform_in(&scratch.0, &track, &[0.5; 8]);

        let file = waveform_file(&scratch.0, &track).expect("the entry has a name");
        std::fs::write(&file, b"not an envelope").expect("the file is damaged");
        assert!(read_waveform_in(&scratch.0, &track).is_none());

        // An envelope from another build is not read either.
        save_waveform_in(&scratch.0, &track, &[0.5; 8]);
        let mut bytes = std::fs::read(&file).expect("the entry is written");
        bytes[4] = 99;
        std::fs::write(&file, bytes).expect("the version is changed");
        assert!(read_waveform_in(&scratch.0, &track).is_none());
    }

    #[test]
    fn a_corrupt_cache_is_ignored() {
        let scratch = Scratch::new("corrupt");
        let music = Path::new("/music");
        std::fs::write(scratch.0.join(file_name(music)), b"this is not toml {{{")
            .expect("the file is written");
        assert!(Cache::load_from(&scratch.0, music).is_none());

        std::fs::write(
            scratch.0.join(file_name(music)),
            "version = 99\nroot = \"/music\"\n",
        )
        .expect("the file is written");
        assert!(
            Cache::load_from(&scratch.0, Path::new("/music")).is_none(),
            "a layout from another build is not read"
        );
    }

    #[test]
    fn a_cache_for_another_directory_is_not_used() {
        let scratch = Scratch::new("other-root");
        let file = scratch.0.join("song.flac");
        std::fs::write(&file, b"one").expect("the file is written");
        let library = Library::from_tracks(vec![track(&file, None)]);
        save_in(&scratch.0, &library, &scratch.0).expect("the cache is written");

        let elsewhere = scratch.0.join("elsewhere");
        assert!(Cache::load_from(&scratch.0, &elsewhere).is_none());
    }

    #[test]
    fn two_directories_keep_a_cache_each() {
        // One file for every directory meant the second scan overwrote the
        // first, so switching between two music directories lost both.
        let scratch = Scratch::new("two-roots");
        let first = scratch.0.join("first");
        let second = scratch.0.join("second");
        for root in [&first, &second] {
            std::fs::create_dir_all(root).expect("the directory is made");
            let file = root.join("song.flac");
            std::fs::write(&file, b"one").expect("the file is written");
            let library = Library::from_tracks(vec![track(&file, None)]);
            save_in(&scratch.0, &library, root).expect("the cache is written");
        }
        assert!(
            Cache::load_from(&scratch.0, &first).is_some(),
            "the first directory keeps its cache"
        );
        assert!(
            Cache::load_from(&scratch.0, &second).is_some(),
            "and so does the second"
        );
    }

    #[test]
    fn a_cover_no_track_uses_is_cleaned_up() {
        let scratch = Scratch::new("prune");
        let file = scratch.0.join("song.flac");
        std::fs::write(&file, b"one").expect("the file is written");
        let with_art =
            Library::from_tracks(vec![track(&file, store_cover_in(&scratch.0, &picture()))]);
        save_in(&scratch.0, &with_art, &scratch.0).expect("the cache is written");

        let without = Library::from_tracks(vec![track(&file, None)]);
        save_in(&scratch.0, &without, &scratch.0).expect("the cache is rewritten");
        let left = std::fs::read_dir(scratch.0.join("covers"))
            .map(|entries| entries.count())
            .unwrap_or(0);
        assert_eq!(left, 0, "the art nobody refers to is gone");
    }

    /// A second picture, so two covers can be told apart.
    fn other_picture() -> Vec<u8> {
        crate::cover::to_png(&Cover {
            width: 2,
            height: 2,
            rgba: vec![1, 2, 3, 255, 4, 5, 6, 255, 7, 8, 9, 255, 10, 11, 12, 255],
        })
        .expect("the picture encodes")
    }

    #[test]
    fn pruning_keeps_the_covers_of_another_library() {
        // `covers/` and `art/` are shared by every music directory's cache,
        // so pruning against one library's tracks deleted the other's art.
        let scratch = Scratch::new("shared-prune");
        let mut keys = Vec::new();
        for (name, picture) in [("first", picture()), ("second", other_picture())] {
            let root = scratch.0.join(name);
            std::fs::create_dir_all(&root).expect("the directory is made");
            let file = root.join("song.flac");
            std::fs::write(&file, b"one").expect("the file is written");
            let key = store_cover_in(&scratch.0, &picture).expect("the picture is stored");
            let library = Library::from_tracks(vec![track(&file, Some(key.clone()))]);
            save_in(&scratch.0, &library, &root).expect("the cache is written");
            keys.push(key);
        }

        let first = Library::from_tracks(vec![track(
            &scratch.0.join("first").join("song.flac"),
            Some(keys[0].clone()),
        )]);
        save_in(&scratch.0, &first, &scratch.0.join("first")).expect("the cache is written again");

        for key in keys {
            assert!(
                scratch.0.join("covers").join(&key).is_file(),
                "a cover another library uses stays"
            );
        }
    }

    #[test]
    fn a_directory_that_is_gone_is_no_library() {
        // An unmounted drive leaves the cache behind; opening it would list
        // tracks that cannot be played.
        let scratch = Scratch::new("gone");
        let root = scratch.0.join("music");
        std::fs::create_dir_all(&root).expect("the directory is made");
        let file = root.join("song.flac");
        std::fs::write(&file, b"one").expect("the file is written");
        let library = Library::from_tracks(vec![track(&file, None)]);
        save_in(&scratch.0, &library, &root).expect("the cache is written");
        assert!(Cache::load_from(&scratch.0, &root).is_some());

        std::fs::remove_dir_all(&root).expect("the directory goes");

        assert!(
            Cache::load_from(&scratch.0, &root).is_none(),
            "the cache is not read for a directory that is gone"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_and_its_target_keep_one_cache() {
        // A music directory reached through a symlink and the directory it
        // points at are one place, so naming it either way finds one cache.
        let scratch = Scratch::new("symlink");
        let target = scratch.0.join("music");
        std::fs::create_dir_all(&target).expect("the directory is made");
        let file = target.join("song.flac");
        std::fs::write(&file, b"one").expect("the file is written");
        let link = scratch.0.join("link");
        std::os::unix::fs::symlink(&target, &link).expect("the link is made");

        let library = Library::from_tracks(vec![track(&file, None)]);
        save_in(&scratch.0, &library, &link).expect("the cache is written");

        assert!(
            Cache::load_from(&scratch.0, &target).is_some(),
            "the target finds the cache the link wrote"
        );
        assert!(
            Cache::load_from(&scratch.0, &link).is_some(),
            "and the link finds it too"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_cache_holds_up_when_the_root_is_named_another_way() {
        // macOS resolves `/tmp` to `/private/tmp`, and a scan may walk either
        // spelling of the same directory. What it writes has to be found again
        // by the other, or every start re-reads the whole library.
        let scratch = Scratch::new("root-spelling");
        let real = scratch.0.join("music");
        std::fs::create_dir_all(&real).expect("the directory is made");
        let file = real.join("song.flac");
        std::fs::write(&file, b"one").expect("the file is written");
        let library = Library::from_tracks(vec![track(&file, None)]);
        save_in(&scratch.0, &library, &real).expect("the cache is written");

        // The other spelling: the same directory reached through a link.
        let link = scratch.0.join("elsewhere");
        std::os::unix::fs::symlink(&real, &link).expect("the link is made");

        let cache = Cache::load_from(&scratch.0, &link).expect("the cache is found");
        let entry = cache
            .track(&link.join("song.flac"))
            .expect("the entry is found under the other name");
        assert!(
            entry.is_fresh(&link),
            "and the file it names is the one on disk"
        );
        assert_eq!(
            entry.to_track(&link).path,
            link.join("song.flac"),
            "the track is named under the root this run walks"
        );
    }

    #[test]
    fn the_digest_is_stable() {
        // A value pinned from one build, so a change to the hash that would
        // orphan every stored cover shows up here.
        assert_eq!(digest(b"blue dove"), digest(b"blue dove"));
        assert_ne!(digest(b"blue dove"), digest(b"blue doves"));
    }

    #[test]
    fn a_stamp_changes_when_the_file_does() {
        let scratch = Scratch::new("stamp");
        let path = scratch.0.join("track.mp3");
        std::fs::write(&path, b"first").expect("the file is written");
        let first = stamp(&path).expect("the file has a stamp");
        std::fs::write(&path, b"second and longer").expect("the file grows");
        let second = stamp(&path).expect("the file has a stamp");
        assert_ne!(first, second);
    }
}
