//! The music library: tags read from files on disk, grouped into albums.
//!
//! Scanning runs on a thread of its own and reports over a channel, so a
//! large directory never blocks the UI.

use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

use lofty::prelude::*;
use unicode_normalization::UnicodeNormalization;
use walkdir::WalkDir;

/// `text` in the composed form the window's faces can draw.
///
/// A tag or a file name may spell a letter as a base and a mark on top: `z`
/// followed by a combining caron, which is how a tool that does not compose
/// its text leaves it. The faces the window bundles hold precomposed letters
/// only and cannot place a mark, so that pair draws as the letter and a box.
/// Composing the two first turns it into U+017E, which they draw. Text that is
/// already composed comes back unchanged, and the scripts with no precomposed
/// form at all are left for a fallback font to draw.
pub(crate) fn composed(text: &str) -> String {
    if text.is_ascii() {
        return text.to_owned();
    }
    text.nfc().collect()
}

/// File extensions the player reads.
///
/// Most decode through symphonia; see `decode` for the formats that get a
/// decoder of their own. DSD is not supported, and neither are TAK and WMA,
/// which have no decoder that fits the licence.
const AUDIO_EXTENSIONS: &[&str] = &[
    "aac", "adx", "aif", "aiff", "amr", "ape", "bcstm", "bfstm", "brstm", "caf", "dsp", "flac",
    "m4a", "m4b", "mka", "mp+", "mp1", "mp2", "mp3", "mp4", "mpc", "mpp", "oga", "ogg", "opus",
    "shn", "swag", "tta", "vag", "vig", "wav", "wv",
];

/// File names looked up for album art when the tracks embed none.
const COVER_NAMES: &[&str] = &[
    "cover.jpg",
    "cover.jpeg",
    "cover.png",
    "cover.svg",
    "folder.jpg",
    "folder.png",
    "folder.svg",
    "front.jpg",
    "front.png",
    "front.svg",
    "album.jpg",
    "albumart.jpg",
];

/// Starts of the file names an album's front cover usually has.
///
/// A rip that ships a booklet names its images `Cover1.jpg`, `front.png` or
/// `booklet front.png`; the first of those that is a front view is the art
/// the album list should show.
const COVER_PREFIXES: &[&str] = &["cover", "front", "folder", "album"];

/// Directories a rip puts its artwork in, rather than beside the tracks.
const ART_DIRECTORIES: &[&str] = &[
    "covers", "cover", "artwork", "art", "scans", "scan", "images", "booklet",
];

/// Extensions an image file can carry.
const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "svg"];

/// Files scanned between two progress messages.
const PROGRESS_EVERY: usize = 200;

/// One playable file and the tags read from it.
#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    /// Where the file lives.
    pub path: PathBuf,
    /// Track title, or the file name when the file carries none.
    pub title: String,
    /// Performing artist, or "Unknown Artist".
    pub artist: String,
    /// Album title, or the directory name when the file carries none.
    pub album: String,
    /// Album artist, which decides which album a track belongs to.
    pub album_artist: String,
    /// Release year, when the tags have one.
    pub year: Option<u32>,
    /// Disc number, when the tags have one.
    pub disc: Option<u32>,
    /// Track number, when the tags have one.
    pub number: Option<u32>,
    /// Playing time, as the tags report it.
    pub duration: Duration,
    /// Key of the cover art, which the cache holds as the encoded picture.
    ///
    /// Nothing is decoded while the library is listed, so a directory of
    /// thousands of tracks costs no image work at all to list.
    pub cover: Option<String>,
    /// Directory the file sits in, relative to the music directory.
    pub directory: PathBuf,
    /// Whether the tags named an album.
    ///
    /// A file without one still shows the directory name as its album, but it
    /// is a loose track: the album list shows it on a small row of its own
    /// rather than folding it into an album.
    pub tagged_album: bool,
}

impl Track {
    /// Text shown for the album column: the album, with its year when known.
    pub fn album_label(&self) -> String {
        match self.year {
            Some(year) => format!("{} ({year})", self.album),
            None => self.album.clone(),
        }
    }

    /// The name of the file itself, which the file name column shows.
    ///
    /// `extension` keeps the extension, such as `.flac`, so a directory of mixed
    /// formats says what each file is; without it the name reads the way a
    /// title does.
    pub fn file_name(&self, extension: bool) -> String {
        let Some(name) = self.path.file_name() else {
            return self.title.clone();
        };
        if extension {
            return composed(&name.to_string_lossy());
        }
        self.path
            .file_stem()
            .map(|stem| composed(&stem.to_string_lossy()))
            .unwrap_or_else(|| composed(&name.to_string_lossy()))
    }
}

/// Tracks that share an album, in playing order.
#[derive(Clone)]
pub struct Album {
    /// Album title.
    pub title: String,
    /// Album artist, or the artist of its first track.
    pub artist: String,
    /// Release year, when the tracks agree on one.
    pub year: Option<u32>,
    /// Indices into [`Library::tracks`], in playing order.
    pub tracks: Vec<usize>,
    /// Key of the cover art, taken from the first track that carries any.
    pub cover: Option<String>,
    /// Directory the album was found in, relative to the music directory.
    pub directory: PathBuf,
}

impl Album {
    /// Total playing time of the album.
    #[cfg(test)]
    pub fn duration(&self, library: &Library) -> Duration {
        self.tracks
            .iter()
            .filter_map(|&index| library.tracks.get(index))
            .map(|track| track.duration)
            .sum()
    }
}

/// Every track found under the music directory, the albums they form, and the
/// directories they were found in.
#[derive(Default)]
pub struct Library {
    /// Every track, in the order the scan found them.
    pub tracks: Vec<Track>,
    /// Albums, sorted by title.
    pub albums: Vec<Album>,
    /// The music directory itself.
    ///
    /// Its albums and loose files are the ones sitting directly in the
    /// directory the user picked, and its children are the top-level
    /// directories below it. It has no row of its own in the album list: what
    /// it holds is drawn at the top of the list, above the directories.
    pub root: Directory,
}

impl Library {
    /// Groups tracks into albums, places them in their directories, and sorts.
    ///
    /// Tracks are ordered by disc and track number, then by title, so an
    /// album plays in the order it was released. Albums are ordered by title,
    /// then year, so two releases of one name stay together.
    pub fn from_tracks(tracks: Vec<Track>) -> Self {
        let mut groups: BTreeMap<(String, String, Option<u32>), Vec<usize>> = BTreeMap::new();
        for (index, track) in tracks.iter().enumerate() {
            // A file with no album tag is loose, not an album of its own:
            // its directory name stands in for the album title only so the
            // track table has something to show.
            if !track.tagged_album {
                continue;
            }
            let key = (
                track.album_artist.to_lowercase(),
                track.album.to_lowercase(),
                track.year,
            );
            groups.entry(key).or_default().push(index);
        }

        let mut albums: Vec<Album> = groups
            .into_iter()
            .map(|((_, _, year), mut indices)| {
                indices.sort_by(|&a, &b| order(&tracks[a], &tracks[b]));
                let first = &tracks[indices[0]];
                let cover = indices
                    .iter()
                    .find_map(|&index| tracks[index].cover.clone());
                Album {
                    title: first.album.clone(),
                    artist: first.album_artist.clone(),
                    year,
                    directory: first.directory.clone(),
                    tracks: indices,
                    cover,
                }
            })
            .collect();
        albums.sort_by(|a, b| {
            a.title
                .to_lowercase()
                .cmp(&b.title.to_lowercase())
                .then(a.year.cmp(&b.year))
        });

        let root = Directory::build(&tracks, &albums);
        Self {
            tracks,
            albums,
            root,
        }
    }

    /// Whether the scan found nothing.
    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }

    /// Whether `other` lists the files this one lists, with the same tags.
    ///
    /// A scan that found nothing new answers a library of the same tracks,
    /// which the window has no reason to take in: taking one in rebuilds
    /// every track, forms the albums again and writes the session file. The
    /// albums are left out of the comparison because they are grouped and
    /// sorted from the tracks, so the same tracks form the same albums.
    pub fn same_as(&self, other: &Self) -> bool {
        self.tracks == other.tracks
    }

    /// The directory at `path`, looked up through the tree.
    ///
    /// The empty path is the music directory itself, whose tracks are the
    /// whole library, so a caller that means one directory passes a path a
    /// row gave it rather than an empty one.
    pub fn directory(&self, path: &Path) -> Option<&Directory> {
        fn find<'a>(directory: &'a Directory, path: &Path) -> Option<&'a Directory> {
            if directory.path == path {
                return Some(directory);
            }
            directory
                .children
                .iter()
                .find_map(|child| find(child, path))
        }
        find(&self.root, path)
    }

    /// The top-level directories below the music directory, sorted by name.
    pub fn directories(&self) -> &[Directory] {
        &self.root.children
    }

    /// Every track in the library, in the order the album list shows them.
    ///
    /// That is the music directory's own albums, then the loose files sitting
    /// beside them, then each directory below it, as
    /// [`Directory::tracks_deep`] walks them, so adding the whole library
    /// queues it in the order it is drawn.
    pub fn tracks_deep(&self) -> Vec<usize> {
        self.root.tracks_deep(&self.albums)
    }

    /// The key of every picture the library holds, in the order it is drawn.
    ///
    /// The album list comes first, then the loose files, so the pictures the
    /// window is about to show head the queue and the rest follow. A key two
    /// albums share is named once.
    pub fn covers(&self) -> Vec<String> {
        let mut keys = Vec::new();
        let mut seen = HashSet::new();
        for album in &self.albums {
            if let Some(key) = &album.cover
                && seen.insert(key.clone())
            {
                keys.push(key.clone());
            }
        }
        for track in &self.tracks {
            if let Some(key) = &track.cover
                && seen.insert(key.clone())
            {
                keys.push(key.clone());
            }
        }
        keys
    }

    /// The album each track belongs to, by track index.
    ///
    /// `None` is a loose file, which belongs to no album. The track table
    /// uses this to head each run of an album's tracks with the album, so a
    /// playlist of several albums reads as a list of albums rather than one
    /// undivided column of titles.
    pub fn album_indices(&self) -> Vec<Option<usize>> {
        let mut indices = vec![None; self.tracks.len()];
        for (album, entry) in self.albums.iter().enumerate() {
            for &track in &entry.tracks {
                if let Some(slot) = indices.get_mut(track) {
                    *slot = Some(album);
                }
            }
        }
        indices
    }
}

/// A directory of the scanned tree, as the album list shows it.
#[derive(Clone, Debug, Default)]
pub struct Directory {
    /// Directory name, without its parent path.
    pub name: String,
    /// Path relative to the music directory, which is empty for the root.
    pub path: PathBuf,
    /// Albums whose files sit directly in this directory, by library index.
    pub albums: Vec<usize>,
    /// Loose tracks in this directory: files that belong to no album.
    pub loose: Vec<usize>,
    /// Subdirectories, sorted by name.
    pub children: Vec<Directory>,
}

impl Directory {
    /// Builds the directory tree from where the tracks were found.
    ///
    /// Answers the music directory itself, whose children are the directories
    /// below it. A track belongs to an album when its tags name one; a file
    /// whose tags carry no album title is loose, and the album list shows it
    /// under its directory in a smaller row. A file sitting directly in the
    /// music directory belongs to that directory, whose relative path is
    /// empty, so it lands at the top of the list rather than being dropped.
    fn build(tracks: &[Track], albums: &[Album]) -> Directory {
        let mut root = Directory::default();
        for (index, album) in albums.iter().enumerate() {
            place(&mut root, &album.directory, |directory| {
                directory.albums.push(index)
            });
        }
        for (index, track) in tracks.iter().enumerate() {
            if !track.tagged_album {
                place(&mut root, &track.directory, |directory| {
                    directory.loose.push(index)
                });
            }
        }
        sort_tree(&mut root, albums);
        root
    }

    /// Every track in this directory and below it, in the order the album list
    /// shows them.
    ///
    /// That is each album's tracks in playing order, then the loose files
    /// sitting beside them, then everything in each subdirectory in turn, so a
    /// directory queued by a double-click reads down the list as it is drawn.
    pub fn tracks_deep(&self, albums: &[Album]) -> Vec<usize> {
        let mut found = Vec::new();
        for &index in &self.albums {
            if let Some(album) = albums.get(index) {
                found.extend(album.tracks.iter().copied());
            }
        }
        found.extend(self.loose.iter().copied());
        for child in &self.children {
            found.extend(child.tracks_deep(albums));
        }
        found
    }

    /// Whether this directory and everything below it holds nothing to show.
    pub fn is_empty(&self) -> bool {
        self.albums.is_empty()
            && self.loose.is_empty()
            && self.children.iter().all(Directory::is_empty)
    }

    /// Whether the directory is a pass-through: it leads to exactly one album
    /// and nothing else worth a row.
    ///
    /// A release usually lives in a directory of its own, and a row for that
    /// directory above its only album says nothing. The album is shown on its
    /// own instead, and a directory row appears only where it groups several.
    /// A directory tree such as `Release/Disc 1` is one chain of
    /// pass-through directories, so it collapses to the album.
    pub fn is_pass_through(&self) -> bool {
        self.loose.is_empty() && self.album_count() == 1 && !self.has_extra_rows()
    }

    /// How many albums this directory and everything below it hold.
    fn album_count(&self) -> usize {
        self.albums.len()
            + self
                .children
                .iter()
                .map(Directory::album_count)
                .sum::<usize>()
    }

    /// Whether anything below the directory needs a row of its own: a second
    /// album, a loose file, or a directory holding more than one album.
    fn has_extra_rows(&self) -> bool {
        !self.loose.is_empty()
            || self
                .children
                .iter()
                .any(|child| !child.loose.is_empty() || child.album_count() != 1)
    }
}

/// Finds or creates the directory for `path` below `directory`, then calls
/// `add` with it.
///
/// `path` is relative to the music directory, so each component is one level
/// of the tree and an empty one is the music directory itself. The path a
/// directory carries is the whole one, not just its own name, so two
/// directories called `Disc 1` stay apart.
fn place(directory: &mut Directory, path: &Path, add: impl FnOnce(&mut Directory)) {
    let mut components = path.components();
    let Some(first) = components.next() else {
        add(directory);
        return;
    };
    let name = composed(&first.as_os_str().to_string_lossy());
    let position = match directory
        .children
        .iter()
        .position(|child| child.name == name)
    {
        Some(position) => position,
        None => {
            directory.children.push(Directory {
                name,
                path: directory.path.join(first),
                ..Directory::default()
            });
            directory.children.len() - 1
        }
    };
    place(&mut directory.children[position], components.as_path(), add);
}

/// Sorts every directory by name, and every directory's albums by title.
fn sort_tree(directory: &mut Directory, albums: &[Album]) {
    directory
        .children
        .sort_by_key(|child| child.name.to_lowercase());
    directory.albums.sort_by(|&a, &b| {
        albums[a]
            .title
            .to_lowercase()
            .cmp(&albums[b].title.to_lowercase())
    });
    for child in &mut directory.children {
        sort_tree(child, albums);
    }
}

/// Album and disc number order, with untagged files last and titles breaking
/// ties so the order is stable whatever the file system returns.
fn order(a: &Track, b: &Track) -> std::cmp::Ordering {
    a.disc
        .unwrap_or(u32::MAX)
        .cmp(&b.disc.unwrap_or(u32::MAX))
        .then(
            a.number
                .unwrap_or(u32::MAX)
                .cmp(&b.number.unwrap_or(u32::MAX)),
        )
        .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
}

/// The directories a scan reads, and the ones it leaves out.
///
/// A scan walks each of `roots` in turn and skips anything under `excluded`,
/// so a directory the user has no use for in the album list stays where it is
/// on disk.
#[derive(Clone, Debug, Default)]
pub struct Sources {
    /// Directories to walk, in the order the album list shows them.
    pub roots: Vec<PathBuf>,
    /// Directories left out of the walk, with everything under them.
    pub excluded: Vec<PathBuf>,
    /// Whether files and directories whose name starts with a dot are left
    /// out.
    pub ignore_hidden: bool,
}

/// What a scan reports while it runs.
pub enum ScanMessage {
    /// Files read so far, for the progress line.
    Progress(usize),
    /// The finished library.
    Done(Box<Library>),
    /// A directory could not be read, while another was: what it held is
    /// missing from the library, and the rest is not.
    Warning(String),
    /// The scan could not read a directory at all.
    Failed(String),
}

/// Starts a scan of `sources` on a thread and returns the channel it reports
/// on.
///
/// `sources.ignore_hidden` leaves out files and directories whose name starts
/// with a dot, which is where desktops keep their own bookkeeping and where a
/// music directory sometimes holds a stash of files nobody wants in a
/// playlist.
pub fn spawn_scan(sources: Sources) -> Receiver<ScanMessage> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let progress = sender.clone();
        let warned = sender.clone();
        let result = read_library_with(
            &sources,
            |files| {
                let _ = progress.send(ScanMessage::Progress(files));
            },
            |error| {
                let _ = warned.send(ScanMessage::Warning(error.to_string()));
            },
        );
        let message = match result {
            Ok(library) => ScanMessage::Done(Box::new(library)),
            Err(error) => ScanMessage::Failed(error),
        };
        let _ = sender.send(message);
    });
    receiver
}

/// Reads every audio file under `root` into a library, blocking until done.
///
/// Hidden files are left out, which is what the app does by default.
#[cfg(test)]
pub fn read_library(root: &Path) -> Result<Library, String> {
    read_library_with(&one(root, true), |_| {}, |_| {})
}

/// Reads every audio file under `root`, hidden ones included.
#[cfg(test)]
pub fn read_library_with_hidden(root: &Path) -> Result<Library, String> {
    read_library_with(&one(root, false), |_| {}, |_| {})
}

/// A scan of one directory.
#[cfg(test)]
pub(crate) fn one(root: &Path, ignore_hidden: bool) -> Sources {
    Sources {
        roots: vec![root.to_path_buf()],
        ignore_hidden,
        ..Sources::default()
    }
}

/// Reads every audio file under the music directories, calling `progress` as
/// files pile up.
///
/// A directory that cannot be read does not stop the others: what it held is
/// missing, and `warning` says which one it was. Only a scan that could read
/// no directory at all fails.
fn read_library_with(
    sources: &Sources,
    mut progress: impl FnMut(usize),
    mut warning: impl FnMut(&str),
) -> Result<Library, String> {
    if sources.roots.is_empty() {
        return Err("no music directory is set".to_string());
    }

    let labels = labels(&sources.roots);
    let mut parts = Vec::new();
    let mut failure = None;
    let mut seen = 0;
    for (root, label) in sources.roots.iter().zip(labels) {
        match read_root(root, sources, |files| progress(seen + files)) {
            Ok(found) => {
                seen += found.len();
                parts.push((label, found));
            }
            Err(error) => {
                warning(&error);
                failure = Some(error);
            }
        }
    }

    if parts.is_empty() {
        return Err(failure.unwrap_or_else(|| "no music found".to_string()));
    }
    Ok(merge(parts))
}

/// Reads every audio file under `root`, and writes that directory's cache.
///
/// The directories of the tracks it answers are relative to `root`, which is
/// what the cache stores and what lets a directory be named another way
/// between two runs.
fn read_root(
    root: &Path,
    sources: &Sources,
    mut progress: impl FnMut(usize),
) -> Result<Vec<Track>, String> {
    if !root.is_dir() {
        return Err(format!("{} is not a directory", root.display()));
    }

    let cache = crate::cache::Cache::load(root);
    let mut tracks = Vec::new();
    let mut covers = Covers::default();
    let mut seen = 0;
    // Files read from disk rather than from the cache, which is what says
    // whether the cache on disk needs writing again.
    let mut read = 0;
    let excluded = excluded_under(root, &sources.excluded);
    let walk = WalkDir::new(root)
        .follow_links(true)
        .into_iter()
        // The root itself is never hidden or excluded: a music directory may
        // sit under a dotted directory and still be the one the user picked.
        // What is under it follows both settings.
        .filter_entry(|entry| {
            entry.path() == root
                || (!is_excluded(entry.path(), &excluded)
                    && (!sources.ignore_hidden || !is_hidden(entry.path())))
        });
    for entry in walk.flatten() {
        if !entry.file_type().is_file() || !has_audio_extension(entry.path()) {
            continue;
        }
        seen += 1;
        if seen % PROGRESS_EVERY == 0 {
            progress(seen);
        }
        if let Some(track) = cached_track(&cache, entry.path(), &mut covers) {
            tracks.push(track);
        } else if let Some(track) = read_track(entry.path(), root, &mut covers) {
            read += 1;
            tracks.push(track);
        }
    }
    progress(seen);

    let library = Library::from_tracks(tracks);
    // A scan of a library nothing touched reads every entry from the cache and
    // finds the files it lists, so the cache on disk already says what this
    // scan would write. Skipping the write is what makes the scan the window
    // runs every minute cost a `stat` per file rather than a rewrite of a file
    // that can be megabytes. A file added, removed, renamed or retagged, and
    // any picture stored again, shows up as a difference here.
    let unchanged = read == 0
        && !covers.failed
        && cache
            .as_ref()
            .is_some_and(|cache| cache.all().len() == library.tracks.len());
    // A failed write only costs the next start its speed, so it is reported
    // and not treated as a failed scan. A write that failed while storing a
    // picture is skipped for the same reason, and because the entry would
    // record art that is not in the cache: the next start reads it again.
    let written = if covers.failed {
        Err("cannot store album art in the cache".to_string())
    } else if unchanged {
        Ok(())
    } else {
        crate::cache::save(&library, root)
    };
    if let Err(error) = written {
        eprintln!("blue-dove: {error}");
    }
    Ok(library.tracks)
}

/// The names the music directories get in the album list, when there are
/// several of them.
///
/// One directory is the library root and needs none: its subdirectories are
/// the top-level rows. Several would all land in one tree, so each keeps its
/// own name above the rest, and a name taken twice is numbered rather than
/// merged with the other.
fn labels(roots: &[PathBuf]) -> Vec<Option<String>> {
    if roots.len() < 2 {
        return vec![None; roots.len()];
    }
    let mut taken = HashSet::new();
    roots
        .iter()
        .map(|root| {
            let base = root
                .file_name()
                .and_then(|name| name.to_str())
                .map_or_else(|| root.display().to_string(), composed);
            let mut label = base.clone();
            let mut number = 2;
            while !taken.insert(label.clone()) {
                label = format!("{base} ({number})");
                number += 1;
            }
            Some(label)
        })
        .collect()
}

/// Joins what each music directory held into one library.
///
/// A directory whose name is given keeps it as the first part of every path,
/// so the album list shows it as a row of its own; one directory has no name,
/// and its contents are the top level.
fn merge(parts: Vec<(Option<String>, Vec<Track>)>) -> Library {
    let mut tracks = Vec::new();
    for (label, mut found) in parts {
        if let Some(label) = label {
            for track in &mut found {
                track.directory = Path::new(&label).join(&track.directory);
            }
        }
        tracks.extend(found);
    }
    Library::from_tracks(tracks)
}

/// The excluded paths as a walk under `root` names them.
///
/// A directory handed over the way the file dialog names it may be spelled
/// other than the walk spells it: a link in the root's path, or `/tmp` and
/// `/private/tmp` on macOS. What both sides resolve to is tried too, so an
/// exclusion picked through a link still leaves the real directory out.
fn excluded_under(root: &Path, excluded: &[PathBuf]) -> Vec<PathBuf> {
    let resolved = root.canonicalize().ok();
    excluded
        .iter()
        .filter_map(|directory| {
            if let Ok(relative) = directory.strip_prefix(root) {
                return Some(root.join(relative));
            }
            let resolved = resolved.as_ref()?;
            let target = directory.canonicalize().ok()?;
            let relative = target.strip_prefix(resolved).ok()?;
            Some(root.join(relative))
        })
        .collect()
}

/// Whether `path` sits under a directory the scan leaves out.
fn is_excluded(path: &Path, excluded: &[PathBuf]) -> bool {
    excluded.iter().any(|directory| path.starts_with(directory))
}

/// The track the cache holds for `path`, when it still fits the file.
///
/// Returns `None` when there is no entry, the file changed, or the picture the
/// entry names is gone from the cache, which leaves the caller to read the file
/// instead and store the picture again. The picture is checked once per key,
/// not once per track, so an album costs one look at the disk.
fn cached_track(
    cache: &Option<crate::cache::Cache>,
    path: &Path,
    covers: &mut Covers,
) -> Option<Track> {
    let cache = cache.as_ref()?;
    let cached = cache.track(path)?;
    let root = cache.root();
    if !cached.is_fresh(root) || !cached.cover().is_none_or(|key| covers.present(key)) {
        return None;
    }
    Some(cached.to_track(root))
}

/// Rebuilds the library from the cache, without reading an audio file.
///
/// This is what the window opens with, so it takes the cache at its word
/// rather than checking every file against the disk: an entry whose file
/// changed is caught by the scan that runs behind the window, which reads that
/// file again and rewrites the cache. Checking here would cost one `stat` per
/// track of a library that may hold thousands, which is time the window would
/// spend blank.
///
/// Answers `None` when there is no cache for the directory at all.
pub fn read_cache(root: &Path) -> Option<Library> {
    let cache = crate::cache::Cache::load(root)?;
    let root = cache.root();
    let tracks: Vec<Track> = cache
        .all()
        .iter()
        .map(|cached| cached.to_track(root))
        .collect();
    if tracks.is_empty() {
        return None;
    }
    Some(Library::from_tracks(tracks))
}

/// Rebuilds the library of every music directory from its cache.
///
/// Answers `None` when no directory has a cache, which leaves the scan that
/// runs behind the window to open the library.
pub fn read_caches(sources: &Sources) -> Option<Library> {
    let labels = labels(&sources.roots);
    let mut parts = Vec::new();
    for (root, label) in sources.roots.iter().zip(labels) {
        if let Some(library) = read_cache(root) {
            parts.push((label, library.tracks));
        }
    }
    if parts.is_empty() {
        return None;
    }
    Some(merge(parts))
}

/// Whether the last part of `path` starts with a dot.
///
/// The scan root itself is never hidden by this: a music directory may sit
/// under a dotted directory and still be the one the user picked.
fn is_hidden(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with('.'))
}

/// Whether the path has an extension the decoder handles.
pub fn has_audio_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| AUDIO_EXTENSIONS.contains(&extension.to_lowercase().as_str()))
}

/// Reads one file's tags. A file whose tags cannot be read is still played,
/// under its file name, when it carries no tags at all.
///
/// `root` is what a track's directory is measured from. A file outside the
/// music directory, which is what a file manager hands the app, is read with
/// its own parent as the root, so it still gets a directory name.
pub fn read_one(path: &Path) -> Option<Track> {
    let root = path.parent().unwrap_or(path);
    read_track(path, root, &mut Covers::default())
}

/// Reads one file's tags against `root`, which its directory is measured
/// from. A file whose tags cannot be read at all is still played, under its
/// file name.
fn read_track(path: &Path, root: &Path, covers: &mut Covers) -> Option<Track> {
    // A game rip carries no tags lofty knows, so it is listed under its file
    // name instead of being dropped from the library.
    let tagged = lofty::read_from_path(path).ok();
    let tag = tagged
        .as_ref()
        .and_then(|tagged| tagged.primary_tag().or_else(|| tagged.first_tag()));

    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("Untitled")
        .to_string();
    let stem = composed(&stem);
    let directory = path
        .parent()
        .and_then(|parent| parent.file_name())
        .and_then(|name| name.to_str())
        .unwrap_or("Unknown Album")
        .to_string();
    let directory = composed(&directory);

    let text = |key: ItemKey| {
        tag.and_then(|tag| tag.get_string(key))
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(composed)
    };

    let artist = text(ItemKey::TrackArtist).unwrap_or_else(|| "Unknown Artist".to_string());
    let album_artist = text(ItemKey::AlbumArtist).unwrap_or_else(|| artist.clone());
    let album_tag = text(ItemKey::AlbumTitle);
    let tagged_album = album_tag.is_some();
    let album = album_tag.unwrap_or(directory);
    let title = text(ItemKey::TrackTitle).unwrap_or(stem);
    let year = text(ItemKey::RecordingDate)
        .or_else(|| text(ItemKey::Year))
        .and_then(|date| date.get(..4).and_then(|year| year.parse().ok()));

    // The picture is stored rather than decoded: `art.rs` decodes it off the
    // drawing thread, and a scan of thousands of tracks does no image work.
    let embedded = tag
        .and_then(|tag| tag.pictures().first())
        .and_then(|picture| covers.shared(picture.data()));
    let cover = embedded.or_else(|| {
        let directory = path.parent()?;
        covers.directory_cover(directory)
    });

    Some(Track {
        path: path.to_path_buf(),
        title,
        artist,
        album,
        album_artist,
        year,
        disc: tag.and_then(Accessor::disk),
        number: tag.and_then(Accessor::track),
        duration: tagged
            .as_ref()
            .map(|tagged| tagged.properties().duration())
            .unwrap_or_default(),
        cover,
        tagged_album,
        directory: path
            .parent()
            .and_then(|parent| parent.strip_prefix(root).ok())
            .map(Path::to_path_buf)
            .unwrap_or_default(),
    })
}

/// Cover art found during a scan, stored and remembered once each.
///
/// An album whose every track embeds the same artwork stores it once, and a
/// directory image is read once rather than per track. What is kept is the key
/// the picture was stored under, not its pixels: a scan writes the pictures it
/// finds and the window decodes them as it draws them.
#[derive(Default)]
struct Covers {
    /// Keys by the hash of the encoded bytes, so one picture is stored once.
    by_hash: HashMap<u64, Option<String>>,
    /// Directory images, by directory.
    by_directory: HashMap<PathBuf, Option<String>>,
    /// Whether a key's bytes are still in the cache, so it is looked at once.
    present: HashMap<String, bool>,
    /// Whether storing a picture failed, which leaves its tracks without art.
    failed: bool,
}

impl Covers {
    /// The key for the picture in `bytes`, storing it the first time.
    fn shared(&mut self, bytes: &[u8]) -> Option<String> {
        let mut hasher = DefaultHasher::new();
        bytes.hash(&mut hasher);
        let hash = hasher.finish();
        if let Some(key) = self.by_hash.get(&hash) {
            return key.clone();
        }
        let key = crate::cache::store_cover(bytes);
        // A cache directory that refuses the write leaves no key to decode, so
        // the album shows no art until one is stored. That is worth reporting
        // rather than caching away: the scan that saw it skips the cache write,
        // so the next start reads the files again and tries once more.
        if key.is_none() {
            self.failed = true;
        }
        self.by_hash.insert(hash, key.clone());
        key
    }

    /// Whether the bytes stored under `key` are still in the cache.
    ///
    /// A cache whose pictures were deleted, or pruned by another library,
    /// leaves its entries naming art that is not there. Answering `None` for
    /// such a track makes the scan read its file again and store the picture
    /// again, which is what keeps a deleted cache from costing the art for
    /// good. The answer is kept, so an album costs one look at the disk.
    fn present(&mut self, key: &str) -> bool {
        if let Some(present) = self.present.get(key) {
            return *present;
        }
        let present = crate::cache::has_cover(key);
        self.present.insert(key.to_string(), present);
        present
    }

    /// The key of the cover belonging to `directory`, when there is one.
    ///
    /// Release directories hold the artwork in one of a few ways: beside the
    /// tracks under a conventional name, in a `covers` or `scans` directory, or
    /// for a multi-disc rip in the directory above the disc directories. All
    /// three are looked for, best first.
    fn directory_cover(&mut self, directory: &Path) -> Option<String> {
        if let Some(key) = self.by_directory.get(directory) {
            return key.clone();
        }
        let bytes = art_in(directory)
            .or_else(|| art_directory_of(directory))
            // A disc directory's artwork usually sits with the other discs.
            .or_else(|| {
                let parent = directory
                    .parent()
                    .filter(|_| is_disc_directory(directory))?;
                art_in(parent).or_else(|| art_directory_of(parent))
            })
            .and_then(|path| std::fs::read(path).ok());
        let key = bytes.and_then(|bytes| self.shared(&bytes));
        self.by_directory
            .insert(directory.to_path_buf(), key.clone());
        key
    }
}

/// The front cover sitting in `directory` itself, if there is one.
///
/// A conventional name wins; failing that, an image whose name starts with
/// one of the usual words; failing that, the only image there, which is what
/// a directory holding nothing but its art looks like.
fn art_in(directory: &Path) -> Option<PathBuf> {
    for name in COVER_NAMES {
        let path = directory.join(name);
        if path.is_file() {
            return Some(path);
        }
    }
    let images = images_in(directory);
    for prefix in COVER_PREFIXES {
        if let Some(path) = images.iter().find(|path| {
            path.file_stem()
                .and_then(|stem| stem.to_str())
                .is_some_and(|stem| stem.to_lowercase().starts_with(prefix))
        }) {
            return Some(path.clone());
        }
    }
    match images.len() {
        1 => images.first().cloned(),
        _ => None,
    }
}

/// The front cover in `directory`'s artwork directory, if it has one.
fn art_directory_of(directory: &Path) -> Option<PathBuf> {
    ART_DIRECTORIES
        .iter()
        .map(|name| directory.join(name))
        .filter(|path| path.is_dir())
        .find_map(|path| art_in(&path))
}

/// The image files directly inside `directory`, in a stable order.
fn images_in(directory: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut images: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| {
                        IMAGE_EXTENSIONS.contains(&extension.to_lowercase().as_str())
                    })
        })
        .collect();
    images.sort();
    images
}

/// Whether `directory` holds one disc of a multi-disc release.
///
/// Its artwork is then usually in the directory above it, with the other discs.
fn is_disc_directory(directory: &Path) -> bool {
    let Some(name) = directory.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let name = name.trim().to_lowercase();
    ["disc", "cd", "dvd", "disk"].iter().any(|word| {
        name.strip_prefix(word).is_some_and(|rest| {
            let rest = rest.trim_start_matches([' ', '-', '_', '.']);
            rest.chars()
                .next()
                .is_some_and(|next| next.is_ascii_digit())
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// A one- or two-second mono WAV, for a directory the tests scan.
    fn wav(seconds: u32) -> Vec<u8> {
        let rate = 8000_u32;
        let samples = rate * seconds;
        let mut data = Vec::with_capacity(samples as usize * 2);
        for index in 0..samples {
            let value = if index % 2 == 0 { 1000_i16 } else { -1000 };
            data.extend_from_slice(&value.to_le_bytes());
        }
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16_u32.to_le_bytes());
        wav.extend_from_slice(&1_u16.to_le_bytes());
        wav.extend_from_slice(&1_u16.to_le_bytes());
        wav.extend_from_slice(&rate.to_le_bytes());
        wav.extend_from_slice(&(rate * 2).to_le_bytes());
        wav.extend_from_slice(&2_u16.to_le_bytes());
        wav.extend_from_slice(&16_u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(data.len() as u32).to_le_bytes());
        wav.extend_from_slice(&data);
        wav
    }

    /// A temporary directory, removed when the test ends.
    struct Scratch(PathBuf);

    impl Scratch {
        /// An empty directory, for a test that lays out its own files.
        fn empty(name: &str) -> Self {
            let root =
                std::env::temp_dir().join(format!("blue-dove-{name}-{}", std::process::id()));
            std::fs::create_dir_all(&root).expect("the directory is made");
            Self(root)
        }

        /// An empty directory holding `count` WAV files in `An Album`.
        fn new(name: &str, count: usize) -> Self {
            let scratch = Self::empty(name);
            let album = scratch.0.join("An Album");
            std::fs::create_dir_all(&album).expect("the directory is made");
            for index in 0..count {
                let path = album.join(format!("{index:02}.wav"));
                std::fs::File::create(&path)
                    .and_then(|mut file| file.write_all(&wav(1)))
                    .expect("the file is written");
            }
            scratch
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_scan_of_an_untouched_library_leaves_the_cache_as_it_is() {
        // The scan the window runs every minute is worth running only if an
        // untouched library costs a stat per file: writing a cache that says
        // what it already says would rewrite a file of megabytes a minute.
        let directory = Scratch::new("quiet-cache", 2);
        read_library(&directory.0).expect("the directory scans");
        let path = crate::cache::cache_path(&directory.0).expect("the cache is kept somewhere");
        let marker = format!(
            "{}\n# left by the first scan\n",
            std::fs::read_to_string(&path).expect("the cache reads")
        );
        std::fs::write(&path, &marker).expect("the marker is written");

        read_library(&directory.0).expect("the directory scans again");

        assert_eq!(
            std::fs::read_to_string(&path).expect("the cache reads"),
            marker,
            "nothing changed, so the cache is left alone"
        );
    }

    #[test]
    fn a_scan_that_reads_a_file_writes_the_cache_again() {
        let directory = Scratch::new("dirty-cache", 2);
        read_library(&directory.0).expect("the directory scans");
        let path = crate::cache::cache_path(&directory.0).expect("the cache is kept somewhere");
        let marker = format!(
            "{}\n# left by the first scan\n",
            std::fs::read_to_string(&path).expect("the cache reads")
        );
        std::fs::write(&path, &marker).expect("the marker is written");
        let added = directory.0.join("An Album").join("99.wav");
        std::fs::File::create(&added)
            .and_then(|mut file| file.write_all(&wav(1)))
            .expect("the file is written");

        read_library(&directory.0).expect("the directory scans again");

        let written = std::fs::read_to_string(&path).expect("the cache reads");
        assert!(
            !written.contains("# left by the first scan"),
            "a new file is in the cache, so the cache is written again"
        );
        assert!(written.contains("99"), "and the new file is in it");
    }

    #[test]
    fn a_letter_written_as_base_and_mark_is_composed() {
        // A tool written on macOS leaves the accent as a mark after the
        // letter, and the faces the window bundles cannot place a mark: the
        // pair draws as the letter and a box. Composing it first draws U+017E.
        assert_eq!(composed("z\u{30C}"), "\u{17E}");
        assert_eq!(composed("Z\u{30C}"), "\u{17D}");
        assert_eq!(composed("e\u{301}"), "\u{E9}");
        assert_eq!(
            composed("\u{17E}"),
            "\u{17E}",
            "a composed letter is unchanged"
        );
        assert_eq!(composed("Song"), "Song", "plain text is unchanged");
    }

    #[test]
    fn a_file_named_with_a_mark_is_listed_composed() {
        // The file name is the title when the tags name none, so the mark has
        // to be composed there too, not only in text read from a tag.
        let directory = Scratch::empty("composed-name");
        let album = directory.0.join("An Album");
        std::fs::create_dir_all(&album).expect("the directory is made");
        let path = album.join("z\u{30C}.wav");
        std::fs::File::create(&path)
            .and_then(|mut file| file.write_all(&wav(1)))
            .expect("the file is written");

        let track = read_one(&path).expect("the file is read");
        assert_eq!(track.title, "\u{17E}", "the name is composed, so it draws");
    }

    #[test]
    fn hidden_files_are_left_out_by_default() {
        let directory = Scratch::new("hidden", 2);
        let album = directory.0.join("An Album");
        for name in [".hidden.wav", ".hidden-dir/inner.wav"] {
            let path = directory.0.join(name);
            std::fs::create_dir_all(path.parent().expect("a file has a parent"))
                .expect("the directory is made");
            std::fs::File::create(&path)
                .and_then(|mut file| file.write_all(&wav(1)))
                .expect("the file is written");
        }
        // A file whose name merely contains a dot is not hidden.
        let dotted = album.join("song.v2.wav");
        std::fs::File::create(&dotted)
            .and_then(|mut file| file.write_all(&wav(1)))
            .expect("the file is written");

        let library = read_library(&directory.0).expect("the directory scans");
        assert_eq!(library.tracks.len(), 3, "the two hidden files are skipped");

        let all = read_library_with_hidden(&directory.0).expect("the directory scans");
        assert_eq!(all.tracks.len(), 5, "turning the option off lists them");
    }

    #[test]
    fn a_music_directory_inside_a_hidden_one_is_still_scanned() {
        // The root the user picked is not hidden by its own name, or a music
        // directory kept under a dotted one would show nothing at all.
        let directory = Scratch::new("hidden-root", 0);
        let root = directory.0.join(".music");
        std::fs::create_dir_all(root.join("An Album")).expect("the directory is made");
        std::fs::File::create(root.join("An Album").join("song.wav"))
            .and_then(|mut file| file.write_all(&wav(1)))
            .expect("the file is written");

        let library = read_library(&root).expect("the directory scans");
        assert_eq!(library.tracks.len(), 1);
    }

    #[test]
    fn a_scan_of_several_directories_keeps_each_one_apart() {
        // Two music directories both hold files sitting directly in them, and
        // the pieces of a library that name them the same way would land in
        // one tree. What says which directory a row belongs to is the source
        // it came from.
        let first = Scratch::new("sources-one", 1);
        let second = Scratch::new("sources-two", 2);
        let sources = Sources {
            roots: vec![first.0.clone(), second.0.clone()],
            ignore_hidden: true,
            ..Sources::default()
        };

        let library = read_library_with(&sources, |_| {}, |_| {}).expect("both directories scan");

        let shown: Vec<&str> = library
            .directories()
            .iter()
            .map(|directory| directory.name.as_str())
            .collect();
        let named: Vec<String> = [&first.0, &second.0]
            .iter()
            .map(|root| {
                root.file_name()
                    .expect("a scratch directory has a name")
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert_eq!(
            shown, named,
            "each source heads its own part of the album list"
        );
        assert_eq!(library.tracks.len(), 3, "and every source is read");
        let albums: Vec<usize> = library
            .directories()
            .iter()
            .map(|directory| directory.albums.len() + directory.children.len())
            .collect();
        assert_eq!(albums, [1, 1], "each source keeps the album it holds");
    }

    #[test]
    fn a_directory_left_out_of_the_scan_is_not_read() {
        let directory = Scratch::new("excluded", 2);
        let left_out = directory.0.join("Left Out");
        std::fs::create_dir_all(&left_out).expect("the directory is made");
        for name in ["one.wav", "two.wav"] {
            std::fs::File::create(left_out.join(name))
                .and_then(|mut file| file.write_all(&wav(1)))
                .expect("the file is written");
        }
        let sources = Sources {
            roots: vec![directory.0.clone()],
            excluded: vec![left_out],
            ignore_hidden: true,
        };

        let library = read_library_with(&sources, |_| {}, |_| {}).expect("the directory scans");

        assert_eq!(
            library.tracks.len(),
            2,
            "only the album beside it is listed"
        );
        assert!(
            library.directory(Path::new("Left Out")).is_none(),
            "and the directory left out has no row at all"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_directory_left_out_is_left_out_however_it_is_spelled() {
        // The file dialog names a directory the way the desktop spells it,
        // which is not always the way the walk under a linked root does:
        // what both resolve to is what decides.
        let directory = Scratch::new("excluded-spelling", 0);
        let real = directory.0.join("real");
        let left_out = real.join("Left Out");
        std::fs::create_dir_all(&left_out).expect("the directory is made");
        std::fs::File::create(left_out.join("song.wav"))
            .and_then(|mut file| file.write_all(&wav(1)))
            .expect("the file is written");
        let link = directory.0.join("link");
        std::os::unix::fs::symlink(&real, &link).expect("the link is made");
        let sources = Sources {
            roots: vec![link],
            excluded: vec![left_out],
            ignore_hidden: true,
        };

        let library = read_library_with(&sources, |_| {}, |_| {}).expect("the directory scans");

        assert!(library.is_empty(), "the linked path finds the exclusion");
    }

    #[test]
    fn a_directory_that_cannot_be_read_does_not_lose_the_others() {
        let directory = Scratch::new("one-missing", 2);
        let missing = directory.0.join("not there");
        let sources = Sources {
            roots: vec![missing, directory.0.clone()],
            ignore_hidden: true,
            ..Sources::default()
        };
        let mut warnings = Vec::new();

        let library = read_library_with(&sources, |_| {}, |error| warnings.push(error.to_string()))
            .expect("the directory that is there is read");

        assert_eq!(library.tracks.len(), 2, "what is there is listed");
        assert_eq!(warnings.len(), 1, "and the one that is not says so");
        assert!(warnings[0].contains("not a directory"));
    }

    #[test]
    fn a_scan_with_no_directory_at_all_fails() {
        let error = read_library_with(&Sources::default(), |_| {}, |_| {})
            .err()
            .expect("there is nothing to scan");
        assert!(error.contains("no music directory"), "{error}");
    }

    #[test]
    fn a_rescan_reads_only_what_changed() {
        let directory = Scratch::new("rescan", 12);
        let first = read_library(&directory.0).expect("the directory scans");
        assert_eq!(first.tracks.len(), 12);

        // Rewriting one file is what a retag looks like to the cache.
        let victim = first.tracks[0].path.clone();
        std::fs::File::create(&victim)
            .and_then(|mut file| file.write_all(&wav(2)))
            .expect("the file is rewritten");

        let second = read_library(&directory.0).expect("the directory scans again");
        assert_eq!(second.tracks.len(), 12);
        let reread = second
            .tracks
            .iter()
            .find(|track| track.path == victim)
            .expect("the rewritten file is listed");
        assert_eq!(
            reread.duration,
            Duration::from_secs(2),
            "the changed file was read again; the others came from the cache"
        );
    }

    #[test]
    fn the_cache_rebuilds_the_library_on_its_own() {
        let directory = Scratch::new("cache", 6);
        let scanned = read_library(&directory.0).expect("the directory scans");
        let cached = read_cache(&directory.0).expect("the cache fits the directory");

        assert_eq!(cached.tracks.len(), scanned.tracks.len());
        assert_eq!(cached.albums.len(), scanned.albums.len());
        for (from_scan, from_cache) in scanned.tracks.iter().zip(&cached.tracks) {
            assert_eq!(from_scan.path, from_cache.path);
            assert_eq!(from_scan.duration, from_cache.duration);
        }
    }

    #[test]
    fn a_changed_file_still_opens_from_the_cache() {
        // The window opens from the cache without a `stat` per track, so an
        // entry whose file was retagged since is shown as it was stored; the
        // scan behind the window is what reads that file again.
        let directory = Scratch::new("stale-cache", 4);
        let scanned = read_library(&directory.0).expect("the directory scans");
        let victim = scanned.tracks[0].path.clone();
        std::fs::File::create(&victim)
            .and_then(|mut file| file.write_all(&wav(2)))
            .expect("the file is rewritten");

        let cached = read_cache(&directory.0).expect("the cache still opens");
        assert_eq!(cached.tracks.len(), scanned.tracks.len());
        let shown = cached
            .tracks
            .iter()
            .find(|track| track.path == victim)
            .expect("the changed file is listed");
        assert_eq!(
            shown.duration,
            Duration::from_secs(1),
            "the window shows what the cache holds, as it was stored"
        );
    }

    #[test]
    fn a_directory_the_cache_never_saw_is_scanned() {
        let directory = Scratch::new("no-cache", 2);
        let elsewhere = directory.0.join("elsewhere");
        std::fs::create_dir_all(&elsewhere).expect("the directory is made");
        // Nothing has cached this directory, so it is read the usual way.
        read_library(&directory.0).expect("the directory scans");
        assert!(read_cache(&elsewhere).is_none());
    }

    fn track(album: &str, artist: &str, number: Option<u32>, title: &str) -> Track {
        Track {
            path: PathBuf::from(format!("/music/{album}/{title}.flac")),
            title: title.to_string(),
            artist: artist.to_string(),
            album: album.to_string(),
            album_artist: artist.to_string(),
            year: Some(2007),
            disc: None,
            number,
            duration: Duration::from_secs(180),
            cover: None,
            directory: PathBuf::from(album),
            tagged_album: true,
        }
    }

    #[test]
    fn every_track_knows_the_album_it_belongs_to() {
        let mut loose = track("Loose", "Someone", None, "A File");
        loose.tagged_album = false;
        let library = Library::from_tracks(vec![
            track("Skies", "Someone", Some(1), "First"),
            loose,
            track("Skies", "Someone", Some(2), "Second"),
        ]);

        let albums = library.album_indices();
        assert_eq!(albums.len(), library.tracks.len());
        assert_eq!(
            albums[0], albums[2],
            "two tracks of one album share its index"
        );
        assert_eq!(albums[1], None, "a file with no album tag belongs to none");
        assert_eq!(
            library.albums[albums[0].expect("the first track has an album")].title,
            "Skies"
        );
    }

    #[test]
    fn groups_tracks_by_album_and_orders_them_by_number() {
        let library = Library::from_tracks(vec![
            track("Skies", "Various Artists", Some(3), "Third"),
            track("Skies", "Various Artists", Some(1), "First"),
            track("Skies", "Various Artists", Some(2), "Second"),
            track("Other", "Someone", Some(1), "Only"),
        ]);

        assert_eq!(library.albums.len(), 2);
        let skies = &library.albums[0];
        assert_eq!(skies.title, "Other");
        let skies = &library.albums[1];
        let titles: Vec<&str> = skies
            .tracks
            .iter()
            .map(|&index| library.tracks[index].title.as_str())
            .collect();
        assert_eq!(titles, ["First", "Second", "Third"]);
    }

    #[test]
    fn untagged_files_sort_after_numbered_ones() {
        let library = Library::from_tracks(vec![
            track("Skies", "Someone", None, "Untagged"),
            track("Skies", "Someone", Some(1), "Numbered"),
        ]);
        assert_eq!(library.albums[0].tracks, [1, 0]);
    }

    #[test]
    fn reports_the_album_length() {
        let library = Library::from_tracks(vec![
            track("Skies", "Someone", Some(1), "One"),
            track("Skies", "Someone", Some(2), "Two"),
        ]);
        assert_eq!(
            library.albums[0].duration(&library),
            Duration::from_secs(360)
        );
    }

    #[test]
    fn the_file_name_keeps_its_extension_only_when_asked() {
        let mut one = track("Skies", "Someone", Some(1), "One");
        one.path = PathBuf::from("/music/Skies/01 Opening Theme.flac");

        assert_eq!(one.file_name(true), "01 Opening Theme.flac");
        assert_eq!(
            one.file_name(false),
            "01 Opening Theme",
            "without the extension the name reads like a title"
        );
    }

    #[test]
    fn a_name_with_dots_keeps_the_dots_that_are_not_the_extension() {
        let mut one = track("Skies", "Someone", Some(1), "One");
        one.path = PathBuf::from("/music/Skies/2.01 Main Theme.mp3");

        assert_eq!(one.file_name(false), "2.01 Main Theme");
    }

    #[test]
    fn a_directory_holding_one_album_is_a_pass_through() {
        let mut single = track("Skies", "Someone", Some(1), "One");
        single.directory = PathBuf::from("Skies/Disc 1");
        let mut other = track("Other", "Someone", Some(1), "Two");
        other.directory = PathBuf::from("Box/Other");
        let library = Library::from_tracks(vec![single, other]);

        let skies = library
            .directories()
            .iter()
            .find(|directory| directory.name == "Skies")
            .expect("the release directory");
        assert!(skies.is_pass_through(), "one album, nothing else");
        let disc = &skies.children[0];
        assert!(disc.is_pass_through(), "the leaf directory wraps its album");
        assert_eq!(
            disc.path,
            PathBuf::from("Skies/Disc 1"),
            "a nested directory carries its whole path, not just its own name"
        );
    }

    #[test]
    fn a_directory_holding_two_albums_is_not_a_pass_through() {
        let mut one = track("One", "Someone", Some(1), "A");
        one.directory = PathBuf::from("Compilation/Disc 1");
        let mut two = track("Two", "Someone", Some(1), "B");
        two.directory = PathBuf::from("Compilation/Disc 2");
        let library = Library::from_tracks(vec![one, two]);

        let compilation = &library.directories()[0];
        assert_eq!(compilation.name, "Compilation");
        assert!(!compilation.is_pass_through());
        assert_eq!(compilation.children.len(), 2);
        assert!(compilation.children.iter().all(Directory::is_pass_through));
    }

    #[test]
    fn loose_files_stop_a_directory_from_being_a_pass_through() {
        let mut album = track("One", "Someone", Some(1), "A");
        album.directory = PathBuf::from("Mixed");
        let mut loose = track("Mixed", "Someone", Some(1), "stray");
        loose.directory = PathBuf::from("Mixed");
        loose.tagged_album = false;
        let library = Library::from_tracks(vec![album, loose]);

        let mixed = &library.directories()[0];
        assert_eq!(mixed.loose.len(), 1);
        assert!(!mixed.is_pass_through());
    }

    #[test]
    fn files_in_the_music_directory_belong_to_it() {
        // A file sitting directly in the music directory has no directory of
        // its own below it, so its relative path is empty. It still belongs
        // somewhere: dropping it made the album list and Add All miss every
        // file the user had not put in a subdirectory.
        let mut album = track("Root", "Someone", Some(1), "A");
        album.directory = PathBuf::new();
        let mut loose = track("Root", "Someone", Some(2), "B");
        loose.directory = PathBuf::new();
        loose.tagged_album = false;
        let library = Library::from_tracks(vec![album, loose]);

        assert!(library.directories().is_empty(), "no subdirectory at all");
        assert_eq!(library.root.albums.len(), 1, "the album sits at the top");
        assert_eq!(library.root.loose.len(), 1, "and so does the loose file");
        assert_eq!(
            library.tracks_deep(),
            [0, 1],
            "the album in playing order, then the loose file"
        );
    }

    #[test]
    fn a_music_directory_of_loose_files_lists_them() {
        // The scan reads a file's directory relative to the music directory,
        // which is empty for a file sitting in the music directory itself.
        // The album list used to drop those files, so a folder holding only
        // them showed nothing at all.
        let scratch = Scratch::empty("root");
        for name in ["one.wav", "two.wav"] {
            std::fs::File::create(scratch.0.join(name))
                .and_then(|mut file| file.write_all(&wav(1)))
                .expect("the file is written");
        }

        let library = read_library(&scratch.0).expect("the directory scans");
        assert_eq!(library.tracks.len(), 2);
        assert!(library.directories().is_empty(), "there is no subdirectory");
        assert_eq!(library.root.loose.len(), 2, "both files are listed");
        assert_eq!(library.tracks_deep().len(), 2, "and both are queued");
    }

    #[test]
    fn a_missing_directory_is_an_error() {
        let missing = std::env::temp_dir().join("blue-dove-no-such-directory");
        assert!(read_library(&missing).is_err());
    }

    #[test]
    fn an_empty_directory_is_an_empty_library() {
        let directory =
            std::env::temp_dir().join(format!("blue-dove-empty-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("the temporary directory is created");
        let library = read_library(&directory).expect("an empty directory scans");
        assert!(library.is_empty());
        std::fs::remove_dir_all(&directory).expect("the temporary directory is removed");
    }

    /// A directory of artwork, removed when the test ends.
    struct Art(PathBuf);

    impl Art {
        fn new(name: &str) -> Self {
            let root =
                std::env::temp_dir().join(format!("blue-dove-art-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).expect("the directory is made");
            Self(root)
        }

        /// Puts a 1x1 PNG named `name` in `directory`, under this directory.
        fn image(&self, directory: &str, name: &str) -> PathBuf {
            let directory = self.0.join(directory);
            std::fs::create_dir_all(&directory).expect("the directory is made");
            let path = directory.join(name);
            let file = std::fs::File::create(&path).expect("the image is created");
            let mut encoder = png::Encoder::new(file, 1, 1);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .expect("the image has a header")
                .write_image_data(&[0, 0, 0, 255])
                .expect("the image has pixels");
            path
        }
    }

    impl Drop for Art {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn the_cover_a_directory_names_is_the_one_used() {
        let art = Art::new("named");
        let directory = art.0.join("Album");
        std::fs::create_dir_all(&directory).expect("the directory is made");
        art.image("Album", "back.png");
        art.image("Album", "Cover1.jpg");

        assert_eq!(
            art_in(&directory),
            Some(directory.join("Cover1.jpg")),
            "a name that says it is the cover beats one that does not"
        );
    }

    #[test]
    fn a_directory_of_images_without_a_cover_is_not_guessed_at() {
        let art = Art::new("unclear");
        art.image("Album", "back.png");
        art.image("Album", "inside.png");

        assert_eq!(
            art_in(&art.0.join("Album")),
            None,
            "two images and no cover among them leaves the album without art"
        );
    }

    #[test]
    fn art_kept_in_its_own_directory_is_found() {
        let art = Art::new("covers");
        let cover = art.image("Album/covers", "front.jpg");

        assert_eq!(art_directory_of(&art.0.join("Album")), Some(cover));
    }

    #[test]
    fn a_disc_directory_takes_the_art_from_above_it() {
        let art = Art::new("disc");
        let root = art.0.join("Release");
        std::fs::create_dir_all(root.join("Disc 1")).expect("the disc directory is made");
        art.image("Release", "cover.jpg");

        let mut covers = Covers::default();
        let key = covers
            .directory_cover(&root.join("Disc 1"))
            .expect("the disc directory finds the set's art");

        let picture = std::fs::read(root.join("cover.jpg")).expect("the picture reads");
        assert_eq!(
            key,
            crate::cache::cover_key(&picture),
            "the key names the bytes"
        );
    }

    #[test]
    fn a_cover_deleted_from_the_cache_is_stored_again() {
        // Deleting the cache directory must not cost the art of the whole
        // library: an entry naming a picture that is gone is read again.
        let art = Art::new("gone-cover");
        let album = art.0.join("Album");
        std::fs::create_dir_all(&album).expect("the album directory is made");
        art.image("Album", "cover.jpg");
        let track = album.join("song.wav");
        std::fs::File::create(&track)
            .and_then(|mut file| file.write_all(&wav(1)))
            .expect("the track is written");

        let first = read_library(&art.0).expect("the directory scans");
        let key = first
            .tracks
            .first()
            .and_then(|track| track.cover.clone())
            .expect("the scan found the art");
        assert!(crate::cache::has_cover(&key));

        assert!(
            crate::cache::forget_cover(&key),
            "the picture is removed from the cache"
        );

        let second = read_library(&art.0).expect("the directory scans again");
        assert_eq!(
            second.tracks.first().and_then(|track| track.cover.clone()),
            Some(key),
            "the picture the entry named is stored again"
        );
        assert!(
            crate::cache::has_cover(&second.tracks[0].cover.clone().unwrap_or_default()),
            "and its bytes are back in the cache"
        );
    }

    #[test]
    fn only_a_disc_directory_names_a_disc() {
        for name in ["Disc 1", "disc-2", "CD3", "cd_4", "DVD 1"] {
            assert!(is_disc_directory(Path::new(name)), "{name} is a disc");
        }
        for name in ["Discography", "CDs", "Disco", "Music"] {
            assert!(!is_disc_directory(Path::new(name)), "{name} is not a disc");
        }
    }

    /// The mimetypes shared-mime-info gives each extension the player reads.
    ///
    /// An empty list is an extension no desktop knows: the game rip formats,
    /// CAF and MP1. MP4 is left out too, since its mimetype also names the
    /// video files the player does not open.
    const MIMETYPES: &[(&str, &[&str])] = &[
        ("aac", &["audio/aac"]),
        ("adx", &[]),
        ("aif", &["audio/x-aiff"]),
        ("aiff", &["audio/x-aiff"]),
        ("amr", &["audio/AMR"]),
        ("ape", &["audio/x-ape"]),
        ("bcstm", &[]),
        ("bfstm", &[]),
        ("brstm", &[]),
        ("caf", &[]),
        ("dsp", &["audio/x-dsp"]),
        ("flac", &["audio/flac"]),
        ("m4a", &["audio/mp4"]),
        ("m4b", &["audio/x-m4b"]),
        ("mka", &["audio/matroska"]),
        ("mp+", &["audio/x-musepack"]),
        ("mp1", &[]),
        ("mp2", &["audio/mp2"]),
        ("mp3", &["audio/mpeg"]),
        ("mp4", &[]),
        ("mpc", &["audio/x-musepack"]),
        ("mpp", &["audio/x-musepack"]),
        ("oga", OGG),
        ("ogg", OGG),
        ("opus", &["audio/ogg", "audio/x-opus+ogg"]),
        ("shn", &["application/x-shorten"]),
        ("swag", &[]),
        ("tta", &["audio/x-tta"]),
        ("vag", &[]),
        ("vig", &[]),
        ("wav", &["audio/vnd.wave"]),
        ("wv", &["audio/x-wavpack"]),
    ];

    /// What an Ogg file may hold: Vorbis, FLAC or Speex, in a stream an
    /// older database calls `audio/x-vorbis+ogg` and a newer one
    /// `audio/vorbis`.
    const OGG: &[&str] = &[
        "audio/ogg",
        "audio/vorbis",
        "audio/x-vorbis+ogg",
        "audio/x-flac+ogg",
        "audio/x-speex+ogg",
    ];

    /// A file manager offers the player for every format it reads, and the
    /// desktop entry claims no mimetype the player cannot open.
    #[test]
    fn the_desktop_entry_claims_the_formats_the_player_reads() {
        let entry = include_str!("../packaging/linux/app.desktop");
        let line = entry
            .lines()
            .find_map(|line| line.strip_prefix("MimeType="))
            .expect("the desktop entry names the mimetypes it opens");
        let claimed: Vec<&str> = line.split(';').filter(|mime| !mime.is_empty()).collect();

        let mut listed: Vec<&str> = MIMETYPES.iter().map(|(extension, _)| *extension).collect();
        listed.sort_unstable();
        let mut read = AUDIO_EXTENSIONS.to_vec();
        read.sort_unstable();
        assert_eq!(listed, read, "every format the player reads belongs here");

        for (extension, mimetypes) in MIMETYPES {
            for mimetype in *mimetypes {
                assert!(
                    claimed.contains(mimetype),
                    "{extension} opens as {mimetype}, which the entry leaves out"
                );
            }
        }
        for mimetype in &claimed {
            assert!(
                MIMETYPES.iter().any(|(_, known)| known.contains(mimetype)),
                "{mimetype} is claimed but no format the player reads has it"
            );
        }
    }

    /// The entry hands the player the file that was opened, so a file manager
    /// that launches it plays that file rather than showing an empty window.
    #[test]
    fn the_desktop_entry_passes_the_opened_file() {
        let entry = include_str!("../packaging/linux/app.desktop");
        let exec = entry
            .lines()
            .find_map(|line| line.strip_prefix("Exec="))
            .expect("the desktop entry names the command that runs");
        assert!(
            exec.contains("%F"),
            "Exec={exec} names no file, so a file manager opens an empty player"
        );
    }
}
