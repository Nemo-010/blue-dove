//! Application state and the logic behind it.
//!
//! The drawing lives in [`crate::ui`], which reads this state and turns
//! input into calls on it. Everything that can be tested without a window
//! stays here.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use eframe::egui;
use winit::window::Window;

use crate::abx::{Abx, Choice, Side};
use crate::config::{Config, SavedPlaylist};
use crate::dialog;
use crate::library::{Library, ScanMessage, Track};
use crate::mpris::{Command, Playback};
use crate::player::Audio;
use crate::playlist::Playlists;
use crate::waveform;

/// Frames a smoke run draws before it closes the window.
const SMOKE_FRAMES: u32 = 3;

/// The fallback the automatic scan runs, in case nothing reports a change.
///
/// The music directory is watched, so a file added while the window runs is
/// scanned a few seconds later; this is what carries a library on
/// storage that cannot report changes, or a tree too large for the kernel's
/// watches. An hour is slow enough to cost nothing on a network share and
/// fast enough that a new file still shows up in the same session.
pub const SCAN_EVERY: Duration = Duration::from_secs(60 * 60);

/// How long a change has to stop arriving before it is scanned.
///
/// Copying an album fires a change per file and per directory, and a scan of
/// each would read the library over and over: a scan waits for the burst to
/// end, and every further change pushes that wait back. Long enough that a
/// copy over the network, whose last write and last event can arrive a few
/// seconds apart, is one scan rather than two.
pub const SETTLE: Duration = Duration::from_secs(5);

/// How long a watch that could not be started waits before it is tried again.
///
/// Storage that cannot report changes refuses every time, so a retry per frame
/// would be a failure per frame; the fallback scan covers the gap meanwhile.
const WATCH_RETRY: Duration = Duration::from_secs(60);

/// Height of one track row, in points.
pub const ROW_HEIGHT: f32 = 22.0;

/// Width of the album list, in points. The now-playing art sits at its top
/// and is as wide as the list, so the width also sets the art's size.
pub const ALBUM_LIST_WIDTH: f32 = 300.0;

/// Height of the bottom bar holding the waveform, in points.
///
/// It leaves room for the waveform plus an equal gap above and below it.
pub const PLAYER_HEIGHT: f32 = 66.0;

/// Height of the bottom bar in the compact layout, holding a thin waveform.
pub const COMPACT_PLAYER_HEIGHT: f32 = 20.0;

/// The size the compact layout snaps the window to, in points.
///
/// It is the width of the album column the art is drawn at, which is also
/// the width the control strip needs, and tall enough for the menu bar, the
/// square art at its full width, the line naming the track, the strip and
/// the thin waveform. The art is not shrunk to make the name line fit: the
/// popup is made taller instead, since the art is the reason to use it.
pub const COMPACT_SIZE: [f32; 2] = [320.0, 414.0];

/// The size below which the window drops the album list and the playlist.
///
/// A window this small cannot show the columns and the art at once, so it
/// becomes the popup: the menu bar, the art, the controls and the waveform.
pub const COMPACT_BREAK: [f32; 2] = [560.0, 430.0];

/// The size a preview opens at when its file carries no art, in points.
///
/// No square is drawn without art, so the window is only as tall as the line
/// naming the track, the controls and the waveform.
pub const PREVIEW_NO_ART_SIZE: [f32; 2] = [320.0, 92.0];

/// What the directory dialog was opened to pick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Pick {
    /// A directory to add to the library.
    Add,
    /// A directory the scan leaves out.
    Exclude,
}

/// The MPRIS track id a preview answers with.
///
/// A previewed file is in no playlist, so it has no index to be named by; a
/// value past any index keeps it apart from a track of the library that does.
const PREVIEW_TRACK: u64 = u64::MAX;

/// The size the compact layout snaps the window to when the track has no art,
/// in points.
///
/// The popup without its square: the menu bar, the line naming the track, the
/// controls and the thin waveform. Snapping to the full [`COMPACT_SIZE`]
/// would leave the height the art would have taken as empty background.
pub const COMPACT_NO_ART_SIZE: [f32; 2] = [320.0, 100.0];

/// The smallest the window may be made in the full layout, in points.
///
/// It is below [`COMPACT_BREAK`], so the window can be dragged down out of the
/// full layout and into the popup at all. Once it is in the popup its floor
/// becomes the size the popup picked for itself, so this is not the popup's
/// floor.
pub const COMPACT_MIN_SIZE: [f32; 2] = [240.0, 84.0];

/// The size the full window opens at, in points.
pub const DEFAULT_WINDOW_SIZE: [f32; 2] = [1180.0, 760.0];

/// Whether a window of `size` shows the compact layout.
pub fn compact_for(size: egui::Vec2) -> bool {
    size.x < COMPACT_BREAK[0] || size.y < COMPACT_BREAK[1]
}

/// Height of the waveform, in points.
///
/// The bar takes all but a narrow gap of the strip it sits in, so the
/// waveform is the widest thing in the player bar rather than a band with
/// dead space above and below it.
pub const WAVE_HEIGHT: f32 = 48.2;

/// Height of the waveform in the compact layout, in points: a thin line
/// that still shows the shape of the track and can be clicked to seek.
pub const COMPACT_WAVE_HEIGHT: f32 = 15.2;

/// Height of the line naming the playing track in the compact layout, in
/// points.
///
/// It sits between the art and the controls, and is reserved even with
/// nothing playing, so the popup does not change size when a track starts.
pub const COMPACT_NAME_HEIGHT: f32 = 18.0;

/// Name of the tab a file named on the command line plays in.
const DEFAULT_TAB: &str = "Default";

/// Volume used when the settings have none.
const DEFAULT_VOLUME: f32 = 0.8;

/// Widths of the track table's columns before the user drags one, in the
/// order [`crate::ui::Column::ALL`] gives: number, title, file name, artist,
/// album and length.
const DEFAULT_COLUMNS: [f32; 6] = [crate::ui::NUMBER_COLUMN, 320.0, 320.0, 230.0, 215.0, 64.0];

/// The widths the track table starts with, which the column menu restores.
pub fn default_columns() -> [f32; 6] {
    DEFAULT_COLUMNS
}

/// Which columns the track table shows, until the user turns one off: every
/// one but the file name, which is there for a directory whose tags are wrong.
const DEFAULT_SHOWN: [bool; 6] = [true, true, false, true, true, true];

/// The widest a column can be dragged to.
const MAX_COLUMN: f32 = 4000.0;

/// How long a track plays before pressing back restarts it.
///
/// Players agree on a few seconds: the first press on a track that is under
/// way goes back to its start, and a press in the first moments goes to the
/// track before it.
const RESTART_AFTER: Duration = Duration::from_secs(3);

/// What the window is showing.
#[derive(Default)]
pub struct View {
    /// The album the album list highlights.
    pub album: Option<usize>,
    /// Text of the search box.
    pub filter: String,
    /// Whether the search box is open.
    pub searching: bool,
    /// Take the keyboard into the search box on the next frame.
    ///
    /// Focus is asked for once, when the box opens: asking every frame would
    /// take it back from whatever the user clicked next.
    pub search_focus: bool,
    /// The row a range selection is measured from: the last one clicked.
    pub selected: Option<usize>,
    /// Every selected row, as indices into [`crate::library::Library::tracks`].
    pub selection: BTreeSet<usize>,
    /// Scroll the selected row into view on the next frame.
    pub reveal: bool,
    /// Directories whose subdirectories are folded away, by their path.
    pub collapsed: HashSet<PathBuf>,
    /// Playlist whose name is being edited, while its tab shows a text box.
    pub renaming: Option<usize>,
    /// The name being typed into that tab.
    pub rename_buffer: String,
}

/// A cover uploaded at the size the big square draws it.
pub(crate) struct BigArt {
    /// The key of the picture, so another track's art is not mistaken for it.
    key: String,
    /// The size it was decoded at, in pixels.
    side: u32,
    /// The texture the window draws.
    texture: egui::TextureHandle,
}

/// Everything the window draws, and the state behind it.
pub struct App {
    /// The renderer the window is drawing through.
    pub(crate) renderer: &'static str,
    /// Whether the frame-rate counter is drawn.
    pub(crate) show_fps: bool,
    /// Frames per second, as last measured.
    pub(crate) fps: f32,
    /// Frames drawn since the last measurement.
    fps_frames: u32,
    /// When the last measurement started.
    fps_last: Instant,
    /// Whether the window closes itself after [`SMOKE_FRAMES`] frames.
    pub(crate) smoke: bool,
    /// Whether settings are written to disk.
    ///
    /// A smoke run never writes: it must behave the same on every machine,
    /// and CI runs it on machines whose settings are not its own.
    pub(crate) persist: bool,
    /// Frames drawn so far, counted only during a smoke run.
    pub(crate) frames: u32,
    /// Settings, as last loaded or saved.
    pub(crate) config: Config,
    /// The scanned library.
    pub(crate) library: Library,
    /// The open playlists and which one is shown.
    pub(crate) playlists: Playlists,
    /// Scan in flight, with the number of files it has read so far.
    pub(crate) scan: Option<(Receiver<ScanMessage>, usize)>,
    /// Waveform of the playing track, until it arrives.
    pub(crate) waveform: Option<Receiver<Option<waveform::Peaks>>>,
    /// Peaks of the playing track.
    pub(crate) peaks: waveform::Peaks,
    /// Bar heights of the waveform, with the width, the bar's height and the
    /// peaks they were worked out for. Rebuilding them is a few hundred
    /// multiplications, which the drawing pass would otherwise do every
    /// frame; they only change with the track, the width or the layout.
    pub(crate) wave_columns: Option<(usize, u32, usize, Vec<f32>)>,
    /// Textures for covers, keyed by the key their picture is stored under.
    pub(crate) textures: HashMap<String, egui::TextureHandle>,
    /// The cover of the loaded track at the size the big square draws it.
    ///
    /// One at a time: the square shows one track, and a texture per track
    /// ever played would keep megabytes of pixels nothing draws.
    pub(crate) big_art: Option<BigArt>,
    /// The size the square is waiting for, so a size it grew past is dropped
    /// rather than decoded and kept.
    pub(crate) big_asked: Option<String>,
    /// Pictures decoded off the drawing thread.
    ///
    /// A start decodes none of them: the window asks for the ones it is about
    /// to draw and gets them a frame or two later, so a library of thousands
    /// of tracks opens as fast as a small one.
    pub(crate) pictures: crate::art::Art,
    /// The audio device, opened when the first track plays.
    pub(crate) audio: Option<Audio>,
    /// Index of the playing track.
    pub(crate) playing: Option<usize>,
    /// Length of the playing track, from the decoder.
    pub(crate) length: Duration,
    /// Where the seek bar was dragged to, until the drag ends.
    pub(crate) seek: Option<Duration>,
    /// Volume, from 0 to 1.
    pub(crate) volume: f32,
    /// Whether the window lets the desktop show through it.
    pub(crate) transparency: bool,
    /// Whether the playing track repeats instead of the next one starting.
    pub(crate) repeat: bool,
    /// Whether the next track is picked at random.
    pub(crate) random: bool,
    /// The colour theme.
    pub(crate) theme: crate::theme::Theme,
    /// Whether the window shows the compact popup layout.
    pub(crate) compact: bool,
    /// The tracks playback walks through, when the user started it from a
    /// selection rather than from the playlist.
    ///
    /// "Play the selected tracks only" must not throw the playlist away, so
    /// the selection becomes a queue that Next, Previous and Random follow
    /// until something else is played.
    pub(crate) queue: Option<Vec<usize>>,
    /// The order Random Order walks, once it has been drawn.
    ///
    /// It is one shuffle of the playlist rather than a fresh pick at every
    /// step: every track is heard before one comes back, and Previous then
    /// Next returns to the track that played between them.
    pub(crate) shuffle: Option<Vec<usize>>,
    /// Commands a D-Bus client sent, drained once a frame.
    ///
    /// The channel is empty for good when the host has no session bus, which
    /// is the case the app works in without noticing.
    pub(crate) remote: Option<Receiver<Command>>,
    /// Where the MPRIS service arrives from, once the bus has accepted it.
    pub(crate) service: Option<Receiver<Option<crate::mpris::Service>>>,
    /// The running MPRIS service, once it is up.
    pub(crate) mpris: Option<crate::mpris::Service>,
    /// The PNG MPRIS names for the playing track's cover.
    ///
    /// A media widget draws the art from a file, so the cover is written out
    /// once and remembered: encoding the same pixels on every frame would
    /// cost more than drawing the window does. `None` is a cover not looked
    /// at yet and `Some(None)` one there was nothing to write for, which is
    /// not tried again until the next track: a cache directory that refuses
    /// the file would otherwise be retried on every frame.
    art: Option<Option<PathBuf>>,
    /// Widths of the track table's columns, in the order
    /// [`crate::ui::Column::ALL`] gives them.
    pub(crate) columns: [f32; 6],
    /// Which of those columns are shown.
    pub(crate) columns_shown: [bool; 6],
    /// Whether the file name column keeps the extension, as in `.flac`.
    pub(crate) file_extension: bool,
    /// Counter behind the random picks, so each one draws a fresh number.
    random_state: u64,
    /// Message shown below the toolbar, such as an error.
    pub(crate) status: Option<String>,
    /// Directory dialog in flight, and what it is picking for.
    pub(crate) picker: Option<(Pick, Receiver<Option<PathBuf>>)>,
    /// Whether the window listing the music directories is open.
    pub(crate) directories_open: bool,
    /// What the window is showing.
    pub(crate) view: View,
    /// Requests from launches that found this window running, drained once a
    /// frame. `None` when nothing listens for them.
    pub(crate) requests: Option<Receiver<crate::instance::Request>>,
    /// Paths a launch named, held until the library can find them.
    pub(crate) pending_paths: Option<Vec<PathBuf>>,
    /// Whether closing the window leaves the music playing.
    pub(crate) background: bool,
    /// Whether the scan leaves out files and directories starting with a dot.
    pub(crate) ignore_hidden: bool,
    /// Whether the music directories are scanned at startup and while the
    /// window runs, so a file added outside the window shows up on its own.
    pub(crate) auto_scan: bool,
    /// When a scan last ended, for the fallback the timer runs.
    ///
    /// `None` is a run that has not scanned yet, which reads as due: a window
    /// whose library is empty has nothing to lose by looking at the directory
    /// again.
    pub(crate) scanned_at: Option<Instant>,
    /// When the change the watcher reported may be scanned.
    ///
    /// A change arrives per file, so the scan waits for the burst to end; a
    /// further change pushes the wait back rather than adding a scan.
    pub(crate) scan_after: Option<Instant>,
    /// The watches on the music directories, one per directory that can be
    /// watched.
    pub(crate) watches: Vec<crate::watch::Watch>,
    /// The directories a watch could not be started on, and when it was
    /// tried.
    ///
    /// Retrying a tree the kernel has no watcher left for would fail on every
    /// frame, so a directory that refused waits a minute before the next go.
    pub(crate) watch_failed: Vec<(PathBuf, Instant)>,
    /// Whether the scan in flight was started by the timer rather than by the
    /// user.
    ///
    /// A scan the timer started takes in only what changed: a library the
    /// window already lists would be rebuilt track by track, have its album
    /// list put back to the first album, and write the session file, every
    /// minute and for nothing.
    pub(crate) scan_quiet: bool,
    /// Whether the album art is fitted whole into its square, rather than
    /// cropped to fill it.
    pub(crate) cover_fit: bool,
    /// Whether a launch naming one audio file opens the preview.
    pub(crate) preview_enabled: bool,
    /// The file the preview is showing, when it is.
    ///
    /// The preview draws the art, the controls and the waveform only, and
    /// its track is in no playlist. The track is read from the file rather
    /// than taken from the library, so a file anywhere plays: a file manager
    /// hands over paths that need not be under the music directory.
    pub(crate) preview: Option<Track>,
    /// Whether the previewed file carried art, which sets the window's height.
    ///
    /// A file with no cover draws no square, so its window is only as tall as
    /// the name line, the controls and the waveform. The answer comes from the
    /// file read when the preview opens and is kept from then on: the window
    /// is a fixed size, so the layout and the size must not disagree.
    pub(crate) preview_art: bool,
    /// Whether the user asked to quit, which a window closing is not.
    pub(crate) quitting: bool,
    /// The blind comparison under way, when there is one.
    ///
    /// It is kept out of the playlists and the table: a highlighted row would
    /// say which of the two tracks X is, and the test would score a listener
    /// who read the answer off the window.
    pub(crate) abx: Option<Abx>,
    /// The sums of the two tracks under test, while they are being read.
    pub(crate) abx_reading: Option<Receiver<crate::digest::Sums>>,
    /// The two sums, A first, once both are read.
    pub(crate) abx_sums: Option<(String, String)>,
    /// The moment every choice is played from.
    ///
    /// Two encodes of one song are compared at the point the listener cares
    /// about, not at whatever the intro happens to be, so every play starts
    /// here rather than at the beginning. It outlives one test, since a
    /// listener sets it once and runs the same moment against several pairs.
    pub(crate) abx_start: Duration,
}

impl App {
    /// Window title and human-readable app name.
    pub const TITLE: &str = "Blue Dove";

    /// Application id. It names the desktop entry on Linux, so the
    /// compositor can match the window to its icon.
    pub const ID: &str = "blue-dove";

    /// Creates the app, loading the settings and scanning the saved directory.
    ///
    /// `smoke` makes it close itself after a few frames, which CI uses to
    /// prove a build starts and renders. A smoke run reads no settings and
    /// scans nothing, so it behaves the same on every machine.
    ///
    /// `paths` are what the command line named. They are held until the
    /// library can find them, and they decide the music directory when none
    /// is set, so `blue-dove song.flac` works on a first run.
    pub fn new(ctx: &egui::Context, smoke: bool, paths: Vec<PathBuf>) -> Self {
        let config = if smoke {
            Config::default()
        } else {
            Config::load()
        };
        let volume = config.volume.unwrap_or(DEFAULT_VOLUME).clamp(0.0, 1.0);
        let transparency = config.transparency.unwrap_or(true);
        let repeat = config.repeat.unwrap_or(false);
        let random = config.random.unwrap_or(false);
        // A theme the user picked stands. With none stored this is a first
        // run, so the session decides: KDE gets Breeze Dark, everything else
        // the app's own dark palette.
        let theme = config
            .theme
            .as_deref()
            .and_then(crate::theme::Theme::from_name)
            .unwrap_or_else(crate::theme::Theme::first_run);
        let stored_columns = config
            .column_widths
            .map(sane_columns)
            .unwrap_or(DEFAULT_COLUMNS);
        let columns_shown = config.column_shown.unwrap_or(DEFAULT_SHOWN);
        let file_extension = config.file_extension.unwrap_or(true);
        let background = config.background.unwrap_or(false);
        let ignore_hidden = config.ignore_hidden.unwrap_or(true);
        let cover_fit = config.cover_fit.unwrap_or(false);
        let auto_scan = config.auto_scan.unwrap_or(true);
        let preview_enabled = config.preview.unwrap_or(true);
        // The bus is reached on a thread of its own, so a host without one,
        // or with one that is slow to answer, never holds up the window.
        let (remote, service) = if smoke {
            (None, None)
        } else {
            // A command has to wake the window, which otherwise only draws
            // when something happens in it.
            let ctx = ctx.clone();
            let waker: crate::mpris::Waker = std::sync::Arc::new(move || ctx.request_repaint());
            let (commands, service) = crate::mpris::spawn(waker);
            (Some(commands), Some(service))
        };
        let mut app = Self {
            renderer: "OpenGL",
            show_fps: false,
            fps: 0.0,
            fps_frames: 0,
            fps_last: Instant::now(),
            smoke,
            persist: !smoke,
            frames: 0,
            config,
            library: Library::default(),
            playlists: Playlists::default(),
            scan: None,
            waveform: None,
            peaks: Vec::new(),
            wave_columns: None,
            textures: HashMap::new(),
            big_art: None,
            big_asked: None,
            pictures: crate::art::Art::default(),
            audio: None,
            playing: None,
            length: Duration::ZERO,
            seek: None,
            volume,
            transparency,
            repeat,
            random,
            theme,
            compact: false,
            queue: None,
            shuffle: None,
            remote,
            service,
            mpris: None,
            art: None,
            columns: stored_columns,
            columns_shown,
            file_extension,
            random_state: 0x9e37_79b9_7f4a_7c15,
            status: None,
            picker: None,
            directories_open: false,
            view: View::default(),
            requests: None,
            pending_paths: (!paths.is_empty()).then_some(paths),
            background,
            ignore_hidden,
            auto_scan,
            scanned_at: None,
            scan_after: None,
            watches: Vec::new(),
            watch_failed: Vec::new(),
            scan_quiet: false,
            cover_fit,
            preview_enabled,
            preview: None,
            preview_art: false,
            quitting: false,
            abx: None,
            abx_reading: None,
            abx_sums: None,
            abx_start: Duration::ZERO,
        };
        let sources = app.sources();
        if sources.roots.is_empty() {
            // A path on the command line says where the music is, which is a
            // better answer for a first run than a dialog nobody asked for.
            let named = app
                .pending_paths
                .as_ref()
                .and_then(|paths| paths.first())
                .and_then(|path| crate::cli::music_root(path));
            match named {
                Some(directory) if !smoke => app.add_directory(directory),
                // A first run has nothing to show, so ask for a directory
                // rather than leave an empty window.
                _ if !smoke => app.ask_for_directory(Pick::Add),
                _ => {}
            }
        } else {
            // The last scan is shown at once, so the window opens with its
            // albums and playlists in place; the scan that follows picks up
            // whatever changed.
            let cached = if !smoke {
                match crate::library::read_caches(&sources) {
                    Some(library) => {
                        app.adopt_library(library);
                        true
                    }
                    None => false,
                }
            } else {
                false
            };
            // The scan at startup is one of the two the automatic scan
            // covers. A directory the cache knows nothing about is scanned
            // anyway: an empty window would leave nothing to play, and the
            // next start would learn nothing about the directory.
            if app.auto_scan || !cached {
                app.start_scan();
            }
        }
        app.open_pending_paths(ctx);
        app
    }

    /// Plays what the command line named, once the library can find it.
    ///
    /// A path given at startup may arrive before the scan has finished, so it
    /// waits in `pending_paths`; this is called again when the scan lands.
    /// The first paths of a run open the preview when they name one file,
    /// which is what a file manager sends; a path sent on later, to a window
    /// that is already open, is a request to play it in the window instead.
    ///
    /// The preview does not wait for the scan. It reads its file on its own,
    /// and a file manager's file is usually outside the music directory
    /// anyway, so holding it back would leave the full window on screen while
    /// a large library was read for a track none of it holds.
    pub fn open_pending_paths(&mut self, ctx: &egui::Context) {
        if let Some(paths) = &self.pending_paths
            && let Some(path) = Self::preview_candidate(paths)
            && self.preview_enabled
        {
            let path = path.clone();
            self.pending_paths = None;
            self.open_preview(ctx, &path);
            return;
        }
        if self.scan.is_some() {
            return;
        }
        if let Some(paths) = self.pending_paths.take() {
            self.open_paths(ctx, &paths);
        }
    }

    /// Plays what a launch named: files play, directories queue.
    ///
    /// A file goes into a tab named Default, which replaces what that tab
    /// held, and plays at once. A directory appends the tracks under it to
    /// the playlist that is showing, the way a double click on the album list
    /// does. A path the library does not hold is reported rather than
    /// silently ignored.
    ///
    /// This is the full window's path, so a preview that is showing gives way
    /// first: what arrives here is a session rather than a look at one track,
    /// and the small window with its three buttons cannot show it.
    pub fn open_paths(&mut self, ctx: &egui::Context, paths: &[PathBuf]) {
        self.exit_preview(ctx);
        // What a path is comes from the library rather than the file system:
        // a file the library holds is a file, a path with tracks under it is a
        // directory, and anything else is reported.
        let mut files = Vec::new();
        let mut directories = Vec::new();
        let mut missing = None;
        for path in paths {
            if let Some(index) = self.track_of(path) {
                files.push(index);
            } else {
                let tracks = self.tracks_under(path);
                if tracks.is_empty() {
                    missing = missing.or(Some(path));
                } else {
                    directories.push(tracks);
                }
            }
        }

        if let Some(&first) = files.first() {
            self.default_tab().set(files);
            self.queue = None;
            self.forget_shuffle();
            self.play(first);
        }
        for tracks in directories {
            self.playlists.current_mut().add(tracks);
            self.queue = None;
            self.forget_shuffle();
            self.select_first();
            self.view.reveal = true;
        }
        if let Some(path) = missing {
            self.report_missing(path);
        }
        self.save_playlists();
    }

    /// The file a launch should preview, when it named exactly one audio file.
    ///
    /// A directory, or a second path, is a session rather than a look at one
    /// track. Whether the preview is then used is the setting's business, so
    /// this answers what was asked for rather than what happens.
    pub fn preview_candidate(paths: &[PathBuf]) -> Option<&PathBuf> {
        let [path] = paths else {
            return None;
        };
        (path.is_file() && crate::library::has_audio_extension(path)).then_some(path)
    }

    /// Plays one file in the preview, leaving the playlists alone.
    ///
    /// The file is read on its own rather than found in the library, so a
    /// path anywhere plays: a file manager names files that need not be in
    /// the music directory. Nothing the user has in a tab is replaced.
    ///
    /// The preview is a fixed-size window: the size is set here and the
    /// window is made unresizable, so it can be neither dragged bigger nor
    /// shrunk. The regular and compact layouts stay resizable; only the
    /// preview is pinned.
    ///
    /// The size is the one the file needs: art makes it square and tall, and
    /// a file with none is only as tall as its name, its controls and the
    /// waveform.
    pub fn open_preview(&mut self, ctx: &egui::Context, path: &std::path::Path) {
        let Some(track) = crate::library::read_one(path) else {
            self.report_missing(path);
            return;
        };
        self.queue = None;
        self.forget_shuffle();
        self.preview_art = track.cover.is_some();
        self.play_file(track);
        self.pin_window(ctx);
    }

    /// Gives the preview window its fixed size and stops it being resized.
    ///
    /// The minimum and the exact size are both set, so a window manager that
    /// honours size hints cannot stretch it either; `Resizable(false)` is what
    /// removes the handles and the drag on a border.
    fn pin_window(&self, ctx: &egui::Context) {
        let size = if self.preview_art {
            COMPACT_SIZE
        } else {
            PREVIEW_NO_ART_SIZE
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(egui::vec2(
            size[0], size[1],
        )));
        ctx.send_viewport_cmd(egui::ViewportCommand::MaxInnerSize(egui::vec2(
            size[0], size[1],
        )));
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
            size[0], size[1],
        )));
        ctx.send_viewport_cmd(egui::ViewportCommand::Resizable(false));
    }

    /// Plays `track` on its own, outside any playlist.
    fn play_file(&mut self, track: Track) {
        // The preview takes the window and the device, so a blind test ends
        // with it.
        self.abx = None;
        let path = track.path.clone();
        let tagged = track.duration;
        // The preview is set before the device is opened: which track the
        // window is showing is not something an audio device decides, and a
        // host with no sound card still has to draw its art and its name.
        self.preview = Some(track);
        self.playing = None;
        self.length = tagged;
        self.seek = None;
        self.peaks.clear();
        self.art = None;

        if self.audio.is_none() {
            match Audio::open(self.volume) {
                Ok(audio) => self.audio = Some(audio),
                Err(error) => {
                    self.status = Some(error);
                    return;
                }
            }
        }
        let Some(audio) = &self.audio else {
            return;
        };
        match audio.play_file(&path) {
            Ok(length) => {
                self.length = length.unwrap_or(tagged);
                self.waveform = Some(waveform::spawn(path));
                self.status = None;
            }
            Err(error) => self.status = Some(error),
        }
    }

    /// Drops the preview and gives the window the full layout's size back.
    ///
    /// The window goes back to the full layout's floor and its own size: the
    /// preview's floor is far below what the album list and the table need,
    /// and a window left that small would open onto a squeezed popup. The
    /// previewed track is handed back, so a caller that has somewhere to put
    /// it can; `None` means nothing was being previewed.
    fn exit_preview(&mut self, ctx: &egui::Context) -> Option<Track> {
        let track = self.preview.take()?;
        // The pin comes off in the same breath: the regular and compact
        // layouts are resizable, and a window left pinned would stay fixed
        // for the rest of the run.
        ctx.send_viewport_cmd(egui::ViewportCommand::Resizable(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::MaxInnerSize(egui::vec2(
            f32::INFINITY,
            f32::INFINITY,
        )));
        ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(egui::vec2(
            COMPACT_MIN_SIZE[0],
            COMPACT_MIN_SIZE[1],
        )));
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
            DEFAULT_WINDOW_SIZE[0],
            DEFAULT_WINDOW_SIZE[1],
        )));
        Some(track)
    }

    /// Leaves the preview, putting its track into the Default tab.
    ///
    /// The window has no way to show a track no playlist holds, so the tab is
    /// where a previewed track goes when the preview ends. Turning the
    /// setting off does this too, so the setting and the window never
    /// disagree. A file the library does not hold cannot go into a playlist,
    /// which stores paths the next scan has to find again, so it is left out
    /// and the window shows what it had.
    pub fn leave_preview(&mut self, ctx: &egui::Context) {
        let Some(track) = self.exit_preview(ctx) else {
            return;
        };
        let Some(index) = self.track_of(&track.path) else {
            self.status = Some(format!(
                "{} is outside the music directory",
                track.path.display()
            ));
            return;
        };
        self.default_tab().set([index]);
        self.playing = Some(index);
        self.save_playlists();
    }

    /// Says a path the command line named is not in the library.
    fn report_missing(&mut self, path: &std::path::Path) {
        self.status = Some(format!("{} is not in the music directory", path.display()));
    }

    /// The tab a file named on the command line plays in.
    ///
    /// That is the tab called Default, created when there is none, so the
    /// track replaces what the tab held rather than piling onto it.
    fn default_tab(&mut self) -> &mut crate::playlist::Playlist {
        let index = self
            .playlists
            .items
            .iter()
            .position(|item| item.name == DEFAULT_TAB);
        let index = match index {
            Some(index) => index,
            None => {
                self.playlists
                    .items
                    .push(crate::playlist::Playlist::new(DEFAULT_TAB));
                self.playlists.items.len() - 1
            }
        };
        self.playlists.active = index;
        self.playlists.current_mut()
    }

    /// The library index of the file at `path`, when the library holds it.
    ///
    /// The path is canonicalised first, so a relative path on the command line
    /// finds a track the scan recorded as absolute.
    fn track_of(&self, path: &std::path::Path) -> Option<usize> {
        let target = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        self.library
            .tracks
            .iter()
            .position(|track| track.path == target || track.path == path)
    }

    /// The library indices of every track under `directory`.
    fn tracks_under(&self, directory: &std::path::Path) -> Vec<usize> {
        let target = directory
            .canonicalize()
            .unwrap_or_else(|_| directory.to_path_buf());
        self.library
            .tracks
            .iter()
            .enumerate()
            .filter(|(_, track)| {
                track.path.starts_with(&target) || track.path.starts_with(directory)
            })
            .map(|(index, _)| index)
            .collect()
    }

    /// Opens the desktop's directory dialog for `pick`.
    pub fn ask_for_directory(&mut self, pick: Pick) {
        if self.picker.is_none() {
            self.picker = Some((pick, dialog::spawn(self.start_directory())));
        }
    }

    /// Where the directory dialog starts: the first music directory that is
    /// still there, else the user's music directory.
    pub fn start_directory(&self) -> PathBuf {
        self.config
            .roots()
            .iter()
            .find(|directory| directory.is_dir())
            .cloned()
            .or_else(dirs::audio_dir)
            .or_else(dirs::home_dir)
            .unwrap_or_default()
    }

    /// What a scan reads, as the settings stand now.
    fn sources(&self) -> crate::library::Sources {
        crate::library::Sources {
            roots: self.config.roots().to_vec(),
            excluded: self.config.excluded().to_vec(),
            ignore_hidden: self.ignore_hidden,
        }
    }

    /// Scans the music directories, replacing the library when it finishes.
    pub fn start_scan(&mut self) {
        self.status = None;
        self.scan_quiet = false;
        self.scan = Some((crate::library::spawn_scan(self.sources()), 0));
    }

    /// Scans the music directories on the timer's behalf, taking in only what
    /// changed.
    ///
    /// It is the same scan as a rescan by hand, but its answer is read
    /// quietly: a scan that found nothing new answers a library the window
    /// already lists, and taking that in would rebuild it, put the album list
    /// back where it started and write the session file, for nothing.
    fn start_quiet_scan(&mut self) {
        self.scan_quiet = true;
        self.scan = Some((crate::library::spawn_scan(self.sources()), 0));
    }

    /// The files the scan has read so far, for the menu bar to count.
    ///
    /// A scan the watcher or the timer started is background work: its
    /// counter would come and go on its own, saying nothing the listener
    /// asked for. Only a scan they started draws one.
    pub(crate) fn scan_progress(&self) -> Option<usize> {
        match &self.scan {
            Some((_, seen)) if !self.scan_quiet => Some(*seen),
            _ => None,
        }
    }

    /// Starts the scan that is due, when one is.
    ///
    /// A change the watcher reported is scanned once the burst that caused it
    /// has ended; the timer is what catches a library on storage that reports
    /// nothing at all. Nothing is scanned with a scan already running, with
    /// no directory to scan, or with the automatic scan turned off.
    fn scan_when_due(&mut self) {
        if !self.auto_scan || self.scan.is_some() || self.config.roots().is_empty() {
            return;
        }
        match self.scan_after {
            // The burst is still arriving, and every event pushes this back.
            Some(after) if Instant::now() < after => return,
            Some(_) => self.scan_after = None,
            // Nothing was reported: only the fallback is left to be due.
            None if matches!(self.scanned_at, Some(at) if at.elapsed() < SCAN_EVERY) => return,
            None => {}
        }
        self.start_quiet_scan();
    }

    /// How long until a scan is due.
    ///
    /// `None` when no scan is due at all: the automatic scan is off, there is
    /// no directory to scan, or a scan is already running, which says so in
    /// its own way.
    fn scan_due_in(&self) -> Option<Duration> {
        if !self.auto_scan || self.scan.is_some() || self.config.roots().is_empty() {
            return None;
        }
        Some(match (self.scan_after, self.scanned_at) {
            (Some(after), _) => after.saturating_duration_since(Instant::now()),
            (None, Some(at)) => SCAN_EVERY.saturating_sub(at.elapsed()),
            (None, None) => Duration::ZERO,
        })
    }

    /// Watches the music directories, and stops watching the ones it did.
    ///
    /// Called once a frame and cheap when there is nothing to do: it starts a
    /// watch on every directory the library is read from, and drops the ones
    /// the automatic scan was turned off for or that are no longer scanned.
    ///
    /// A watch that ended -- the directory was removed, renamed or unmounted
    /// -- is dropped and placed again rather than kept: it reports nothing
    /// from here on, and the path still matching would hide that.
    fn watch_directories(&mut self, ctx: &egui::Context) {
        let wanted: Vec<PathBuf> = if self.auto_scan {
            self.config.roots().to_vec()
        } else {
            Vec::new()
        };
        self.watches
            .retain(|watch| wanted.iter().any(|root| root == watch.root()) && !watch.ended());
        self.watch_failed
            .retain(|(root, at)| wanted.contains(root) && at.elapsed() < WATCH_RETRY);
        for directory in wanted {
            if self.watches.iter().any(|watch| watch.root() == directory)
                || self.watch_failed.iter().any(|(root, _)| *root == directory)
            {
                continue;
            }
            let ctx = ctx.clone();
            let wake: crate::watch::Waker = std::sync::Arc::new(move || ctx.request_repaint());
            match crate::watch::spawn(&directory, wake) {
                Some(watch) => self.watches.push(watch),
                None => self.watch_failed.push((directory, Instant::now())),
            }
        }
    }

    /// Adds a directory to the library, remembers it and scans.
    ///
    /// A directory already in the library is scanned again rather than listed
    /// twice.
    pub fn add_directory(&mut self, directory: PathBuf) {
        let mut roots = self.config.roots().to_vec();
        if !roots.contains(&directory) {
            roots.push(directory);
            self.config.set_roots(roots);
            self.save_settings();
        }
        self.start_scan();
    }

    /// Takes a directory out of the library, and scans again.
    pub fn remove_directory(&mut self, directory: &Path) {
        let roots: Vec<PathBuf> = self
            .config
            .roots()
            .iter()
            .filter(|root| root.as_path() != directory)
            .cloned()
            .collect();
        self.config.set_roots(roots);
        self.save_settings();
        if self.config.roots().is_empty() {
            // There is nothing left to scan, and blanking the album list under
            // the user would take away the tracks they may have just queued.
            self.scan = None;
            self.status = None;
            return;
        }
        self.start_scan();
    }

    /// Leaves a directory out of the scan, and scans again.
    pub fn exclude_directory(&mut self, directory: PathBuf) {
        let mut excluded = self.config.excluded().to_vec();
        if !excluded.contains(&directory) {
            excluded.push(directory);
            self.config.set_excluded(excluded);
            self.save_settings();
        }
        self.start_scan();
    }

    /// Puts a directory the scan left out back in it, and scans again.
    pub fn include_directory(&mut self, directory: &Path) {
        let excluded: Vec<PathBuf> = self
            .config
            .excluded()
            .iter()
            .filter(|left| left.as_path() != directory)
            .cloned()
            .collect();
        self.config.set_excluded(excluded);
        self.save_settings();
        self.start_scan();
    }

    /// Takes in what the background threads have finished.
    fn poll_workers(&mut self, ctx: &egui::Context) {
        // A change under the music directories is scanned once the burst that
        // caused it has ended, so a copy of an album is one scan rather than
        // one per file.
        self.watch_directories(ctx);
        if self.watches.iter().any(crate::watch::Watch::changed) {
            self.scan_after = Some(Instant::now() + SETTLE);
            ctx.request_repaint_after(SETTLE);
        }
        self.scan_when_due();
        let mut finished = false;
        let mut failed = false;
        let mut scanned: Option<Box<Library>> = None;
        if let Some((receiver, seen)) = &mut self.scan {
            while let Ok(message) = receiver.try_recv() {
                match message {
                    ScanMessage::Progress(files) => *seen = files,
                    ScanMessage::Done(library) => {
                        scanned = Some(library);
                        finished = true;
                    }
                    ScanMessage::Warning(error) => self.status = Some(error),
                    ScanMessage::Failed(error) => {
                        self.status = Some(error);
                        failed = true;
                        finished = true;
                    }
                }
            }
        }
        // The scan has to end before the library is swapped in, since the
        // playlists have to be carried over to it.
        if let Some(library) = scanned {
            // A scan the timer started answers with the same library as often
            // as not, and taking that in would undo what the window shows for
            // nothing.
            if !self.scan_quiet || !self.library.same_as(&library) {
                self.adopt_library(*library);
            }
        }
        if finished {
            self.scan = None;
            self.scan_quiet = false;
            // The timer counts from the end of a scan, so a scan of a large
            // directory is not followed by another the moment it lands. A
            // scan that failed is not a scan, though: it read nothing, so the
            // next one is a minute off rather than an hour, which is the case
            // an unmounted share runs into and comes back from.
            if failed {
                self.scan_after = Some(Instant::now() + WATCH_RETRY);
            } else {
                self.scanned_at = Some(Instant::now());
            }
            if self.library.is_empty() && self.status.is_none() {
                self.status = Some("No music found in the library sources".to_string());
            }
        }

        if let Some(receiver) = &self.waveform
            && let Ok(peaks) = receiver.try_recv()
        {
            self.peaks = peaks.unwrap_or_default();
            self.wave_columns = None;
            self.waveform = None;
        }

        if let Some(receiver) = &self.abx_reading
            && let Ok((a, b)) = receiver.try_recv()
        {
            self.abx_reading = None;
            // A file that cannot be read leaves the sum out rather than
            // stopping the test: the comparison still runs without it.
            if let (Ok(a), Ok(b)) = (a, b) {
                self.abx_sums = Some((a, b));
            }
            ctx.request_repaint();
        }

        if let Some((pick, receiver)) = &self.picker
            && let Ok(chosen) = receiver.try_recv()
        {
            let pick = *pick;
            self.picker = None;
            match chosen {
                Some(directory) => match pick {
                    Pick::Add => self.add_directory(directory),
                    Pick::Exclude => self.exclude_directory(directory),
                },
                // A cancel and a missing backend are one answer; the message
                // says which case this is when it can tell.
                None => self.status = dialog::explain(),
            }
        }

        if self.pictures.poll() {
            ctx.request_repaint();
        }

        // A path given at startup could not be found until the scan landed.
        self.open_pending_paths(ctx);

        if self.scan.is_some()
            || self.waveform.is_some()
            || self.picker.is_some()
            || self.pictures.pending()
        {
            ctx.request_repaint_after(Duration::from_millis(120));
        } else if let Some(wait) = self.scan_due_in() {
            // The timer is read once a frame, and an idle window draws none:
            // without this the scan due in a minute would wait for the next
            // key press or pointer move to run.
            ctx.request_repaint_after(wait.max(Duration::from_millis(120)));
        }
    }

    /// Starts playing the track at `index`.
    ///
    /// A track that cannot be opened, or an index the library does not hold,
    /// leaves the error in the status line and nothing playing.
    pub fn play(&mut self, index: usize) {
        // Playing a row by hand ends a blind test: the two would fight over
        // the device, and the row would say which track X is.
        self.abx = None;
        let Some(track) = self.library.tracks.get(index) else {
            return;
        };
        let path = track.path.clone();
        let tagged = track.duration;

        if self.audio.is_none() {
            match Audio::open(self.volume) {
                Ok(audio) => self.audio = Some(audio),
                Err(error) => {
                    self.status = Some(error);
                    return;
                }
            }
        }
        let Some(audio) = &self.audio else {
            return;
        };

        match audio.play_file(&path) {
            Ok(length) => {
                self.length = length.unwrap_or(tagged);
                self.playing = Some(index);
                self.seek = None;
                // The table highlights the playing track, so the selection
                // follows playback whether a click, Next, Repeat or Random
                // started it.
                self.select_only(index);
                self.view.reveal = true;
                self.peaks.clear();
                self.art = None;
                self.waveform = Some(waveform::spawn(path));
                self.status = None;
                self.save_playlists();
            }
            Err(error) => self.status = Some(error),
        }
    }

    /// Forgets the shuffled order, for a change of playlist or queue.
    ///
    /// The next Random step draws a new one, from the tracks the playlist
    /// holds then.
    pub fn forget_shuffle(&mut self) {
        self.shuffle = None;
    }

    /// Moves to the next or previous track of the playlist.
    ///
    /// With nothing playing it starts the playlist's first track, even when
    /// the search box hides it: the table shows what the filter leaves, and
    /// the playlist is what playback walks. The previous track wraps to the
    /// end of the playlist, and so does the next. With Random Order on, both
    /// walk the shuffled order instead.
    ///
    /// Back on a track that has played for a few seconds restarts it rather
    /// than moving on, so one press goes to its start and a second goes to
    /// the track before it.
    pub fn step(&mut self, forward: bool) {
        // The preview has no playlist behind it, so there is nothing to step
        // to; the keyboard would otherwise walk the library out from under a
        // window that is showing one file, and MPRIS would then name the
        // preview while the library played.
        if self.preview.is_some() {
            return;
        }
        let Some(current) = self.playing.or(self.view.selected) else {
            if let Some(&first) = self.playlists.current().tracks.first() {
                self.play(first);
            }
            return;
        };
        if !forward && self.playing.is_some() && back_restarts(self.position()) {
            self.seek_to(0.0);
            return;
        }
        if self.random {
            if let Some(index) = self.shuffled_neighbour(current, forward) {
                self.play(index);
            }
            return;
        }
        self.step_linear(current, forward);
    }

    /// The track after or before `current` in the shuffled order.
    ///
    /// A track that is not in the order, such as one the playlist lost,
    /// starts it again rather than stopping.
    fn shuffled_neighbour(&mut self, current: usize, forward: bool) -> Option<usize> {
        let order = self.shuffle_order();
        crate::playlist::neighbour(&order, current, forward).or_else(|| order.first().copied())
    }

    /// Moves to the next or previous track in the order being walked.
    fn step_linear(&mut self, current: usize, forward: bool) {
        let tracks = self.walked_tracks();
        // A track that was taken out of the playlist has no neighbour, so
        // playback starts the list again rather than stopping.
        let next = crate::playlist::neighbour(tracks, current, forward)
            .or_else(|| tracks.first().copied());
        if let Some(index) = next {
            self.play(index);
        }
    }

    /// The tracks Next, Previous and Random walk through.
    ///
    /// That is the queue a "play the selection" started, or the playlist.
    fn walked_tracks(&self) -> &[usize] {
        match &self.queue {
            Some(queue) => queue,
            None => &self.playlists.current().tracks,
        }
    }

    /// The order Random Order walks, drawn the first time it is needed.
    ///
    /// One shuffle is drawn and then followed both ways, which is what makes
    /// Previous and Next agree: the same track that played last plays again
    /// when the way forward is walked. The generator is a plain xorshift, so
    /// no dependency is needed for it.
    fn shuffle_order(&mut self) -> Vec<usize> {
        if let Some(order) = &self.shuffle {
            return order.clone();
        }
        let tracks = self.walked_tracks().to_vec();
        let mut state = self.random_state;
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        self.random_state = state;
        let order = crate::playlist::shuffled(&tracks, state);
        self.shuffle = Some(order.clone());
        order
    }

    /// Starts or pauses playback.
    pub fn toggle(&mut self) {
        let loaded = self.playing.is_some() || self.preview.is_some() || self.abx.is_some();
        match &self.audio {
            Some(audio) if loaded => audio.toggle(),
            _ => {
                if let Some(&first) = self.visible_tracks().first() {
                    self.play(first);
                }
            }
        }
    }

    /// Selects the first track of the playlist, if it holds one.
    pub fn select_first(&mut self) {
        let first = self.visible_tracks().first().copied();
        self.select(first);
    }

    /// Whether the row for `index` is selected.
    pub fn is_selected(&self, index: usize) -> bool {
        self.view.selection.contains(&index)
    }

    /// Selects `index` alone.
    pub fn select_only(&mut self, index: usize) {
        self.view.selection.clear();
        self.view.selection.insert(index);
        self.view.selected = Some(index);
    }

    /// Selects every track the table shows.
    pub fn select_all(&mut self) {
        let tracks = self.visible_tracks();
        self.view.selection = tracks.iter().copied().collect();
        self.view.selected = tracks.first().copied().or(self.view.selected);
    }

    /// Empties the selection.
    pub fn clear_selection(&mut self) {
        self.view.selection.clear();
        self.view.selected = None;
    }

    /// Selects `index` alone, or clears the selection when there is none.
    pub fn select(&mut self, index: Option<usize>) {
        match index {
            Some(index) => self.select_only(index),
            None => self.clear_selection(),
        }
    }

    /// Selects a track the library scan found again, and shows its album.
    ///
    /// A restored track can sit anywhere in the table, so the row is scrolled
    /// into view and the album list follows it.
    fn select_restored(&mut self, index: Option<usize>) {
        self.select(index);
        self.view.reveal = index.is_some();
        if let Some(index) = index
            && let Some(album) = self.album_of(index)
        {
            self.view.album = Some(album);
        }
    }

    /// Adds `index` to the selection, or takes it out when it is there.
    pub fn toggle_selection(&mut self, index: usize) {
        if self.view.selection.remove(&index) {
            if self.view.selected == Some(index) {
                self.view.selected = self.view.selection.iter().next().copied();
            }
        } else {
            self.view.selection.insert(index);
            self.view.selected = Some(index);
        }
    }

    /// Selects everything from the last clicked row to `index`.
    ///
    /// The range is taken from what the table shows, so a search narrows it
    /// to the rows in view.
    pub fn extend_selection(&mut self, index: usize) {
        let shown = self.visible_tracks();
        let position = |row: usize| shown.iter().position(|&shown| shown == row);
        let (Some(from), Some(to)) = (self.view.selected.and_then(position), position(index))
        else {
            self.select_only(index);
            return;
        };
        let (from, to) = if from <= to { (from, to) } else { (to, from) };
        self.view.selection = shown[from..=to].iter().copied().collect();
    }

    /// The selected tracks, in the order the playlist holds them.
    pub fn selected_tracks(&self) -> Vec<usize> {
        self.playlists
            .current()
            .tracks
            .iter()
            .copied()
            .filter(|index| self.view.selection.contains(index))
            .collect()
    }

    /// Plays the selected tracks and nothing else, in playlist order.
    ///
    /// The playlist is left as it is: the selection becomes the queue that
    /// Next, Previous and Random follow, so the rest of the tab is not lost.
    pub fn play_selected(&mut self) {
        let tracks = self.selected_tracks();
        let Some(&first) = tracks.first() else {
            return;
        };
        self.queue = Some(tracks);
        self.forget_shuffle();
        self.play(first);
    }

    /// Takes the selected tracks out of the playlist.
    ///
    /// This only edits the playlist: no file is touched, ever. The playing
    /// track carries on, since playback holds the file rather than the row.
    pub fn remove_selected(&mut self) {
        if self.view.selection.is_empty() {
            return;
        }
        let removed = std::mem::take(&mut self.view.selection);
        let playlist = self.playlists.current_mut();
        playlist.tracks.retain(|index| !removed.contains(index));
        playlist.cursor = playlist
            .cursor
            .filter(|&cursor| cursor < playlist.tracks.len());
        self.view.selected = None;
        if let Some(queue) = &mut self.queue {
            queue.retain(|index| !removed.contains(index));
            if queue.is_empty() {
                self.queue = None;
            }
        }
        self.forget_shuffle();
        self.save_playlists();
    }

    /// Makes the album at `index` the active playlist.
    ///
    /// Double-clicking an album replaces what the playlist holds, so the
    /// tab always shows the album that was picked.
    pub fn open_album(&mut self, index: usize) {
        let Some(album) = self.library.albums.get(index) else {
            return;
        };
        let tracks = album.tracks.clone();
        self.view.album = Some(index);
        self.playlists.current_mut().set(tracks);
        self.queue = None;
        self.forget_shuffle();
        // A search left over from the old playlist would hide the album that
        // was just opened.
        self.close_search();
        self.select_first();
        self.view.reveal = true;
        self.save_playlists();
    }

    /// Appends everything the directory at `path` holds to the playlist.
    ///
    /// The album list draws from a flattened list of rows rather than a
    /// borrow of the tree, so a double-click arrives as a path; this finds
    /// the directory again and queues it the way a double-click does.
    ///
    /// The directory's own albums come first, then the loose files sitting
    /// beside them, then each subdirectory in the order the album list shows
    /// them, so a double-click on a directory queues it in reading order. A
    /// directory holding no album at all queues the files it holds.
    pub fn add_directory_at(&mut self, path: &Path) {
        let tracks = match self.library.directory(path) {
            Some(directory) => directory.tracks_deep(&self.library.albums),
            None => return,
        };
        self.add_tracks(tracks);
    }

    /// Appends every track in the library to the playlist.
    ///
    /// The tracks arrive in the order the album list draws them, so the whole
    /// library reads down the table the way it reads down the list: the music
    /// directory's own albums and loose files first, then each directory
    /// below it. A directory of loose files has no album to add, so leaving
    /// them out would empty the playlist for a library that holds nothing
    /// else.
    pub fn add_all(&mut self) {
        let tracks = self.library.tracks_deep();
        if tracks.is_empty() {
            return;
        }
        self.add_tracks(tracks);
    }

    /// Appends `tracks` to the playlist and brings the first into view.
    fn add_tracks(&mut self, tracks: Vec<usize>) {
        self.playlists.current_mut().add(tracks);
        self.queue = None;
        self.forget_shuffle();
        self.close_search();
        self.select_first();
        self.view.reveal = true;
        self.save_playlists();
    }

    /// Plays a track on its own, replacing the playlist with it.
    ///
    /// A loose file is not part of an album, so playing it should not leave
    /// the rest of the tab queued behind it.
    pub fn play_loose(&mut self, index: usize) {
        self.playlists.current_mut().set([index]);
        self.queue = None;
        self.forget_shuffle();
        self.save_playlists();
        self.play(index);
    }

    /// Whether a directory is folded away in the album list.
    pub fn is_collapsed(&self, directory: &crate::library::Directory) -> bool {
        self.view.collapsed.contains(&directory.path)
    }

    /// Folds the directory at `path` away, or unfolds it.
    pub fn toggle_collapsed_at(&mut self, path: &Path) {
        if !self.view.collapsed.remove(path) {
            self.view.collapsed.insert(path.to_path_buf());
        }
    }

    /// Renames the active playlist.
    pub fn rename_playlist(&mut self, name: &str) {
        let name = name.trim();
        if !name.is_empty() {
            self.playlists.current_mut().name = name.to_string();
            self.save_playlists();
        }
    }

    /// Whether the player is paused, or has nothing loaded.
    pub fn paused(&self) -> bool {
        self.audio.as_ref().is_none_or(Audio::is_paused)
    }

    /// Stops playback and clears the now-playing bar.
    ///
    /// The track stays selected in the table, so play starts it again from
    /// the beginning.
    pub fn stop(&mut self) {
        if let Some(audio) = &self.audio {
            audio.stop();
        }
        self.playing = None;
        self.length = Duration::ZERO;
        self.seek = None;
        self.peaks.clear();
        self.wave_columns = None;
        self.waveform = None;
    }

    /// Starts a blind comparison of the two selected tracks.
    ///
    /// The first of the two rows is A and the second B. The test takes the
    /// player over from the playlist, which stops: the table highlights the
    /// playing row, and a highlight would say which of the two X is. A
    /// selection that is not exactly two rows starts nothing.
    pub fn start_abx(&mut self) {
        let tracks = self.selected_tracks();
        let [a, b] = tracks.as_slice() else {
            return;
        };
        let (Some(a), Some(b)) = (self.library.tracks.get(*a), self.library.tracks.get(*b)) else {
            return;
        };
        let (a, b) = (a.clone(), b.clone());
        self.stop();
        self.abx_reading = Some(crate::digest::spawn(a.path.clone(), b.path.clone()));
        self.abx_sums = None;
        self.abx = Some(Abx::new(a, b, self.abx_seed()));
    }

    /// A seed for the next test, drawn so that two runs differ.
    ///
    /// The counter alone would hand a restart of the same pair the sides the
    /// run before it drew, a moment later, which is the one thing a blind
    /// test must not do; the clock is mixed in to keep them apart.
    fn abx_seed(&mut self) -> u64 {
        self.random_state = self
            .random_state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let clock = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.subsec_nanos());
        self.random_state ^ u64::from(clock)
    }

    /// Plays the track a choice names, for a listener comparing the two.
    ///
    /// It starts at `abx_start` rather than at the beginning, so every choice
    /// is heard from the same moment. The device opens on the first press, as
    /// it does for a track played from the table, so a host without a sound
    /// card still opens the test and says why nothing plays.
    pub fn abx_play(&mut self, choice: Choice) {
        let Some(test) = &self.abx else {
            return;
        };
        let path = test.path(choice);
        let tagged = test.choice(choice).duration;
        if self.audio.is_none() {
            match Audio::open(self.volume) {
                Ok(audio) => self.audio = Some(audio),
                Err(error) => {
                    self.status = Some(error);
                    return;
                }
            }
        }
        let Some(audio) = &self.audio else {
            return;
        };
        let length = match audio.play_file(&path) {
            Ok(length) => length,
            Err(error) => {
                self.status = Some(error);
                return;
            }
        };
        let start = self.abx_start;
        if !start.is_zero()
            && let Err(error) = audio.seek(start)
        {
            self.status = Some(error);
        }
        self.length = length.unwrap_or(tagged);
        self.seek = None;
        self.status = None;
        if let Some(test) = &mut self.abx {
            test.set_playing(Some(choice));
        }
    }

    /// Moves the track under the comparison's seek bar to `position`.
    pub fn abx_seek(&mut self, position: Duration) {
        let Some(audio) = &self.audio else {
            return;
        };
        if let Err(error) = audio.seek(position) {
            self.status = Some(error);
        }
    }

    /// Sets the moment every choice is played from.
    pub fn abx_set_start(&mut self, start: Duration) {
        self.abx_start = start;
    }

    /// Scores the trial under way against `side`.
    ///
    /// A second answer to the same trial, or one given after the last, is
    /// ignored by the test, which counts each trial once.
    pub fn abx_answer(&mut self, side: Side) {
        if let Some(test) = &mut self.abx {
            let _ = test.answer(side);
        }
    }

    /// Moves the test on to the trial after the one answered.
    pub fn abx_next(&mut self) {
        if let Some(test) = &mut self.abx {
            test.next();
        }
    }

    /// Runs the comparison again with the same two tracks.
    pub fn abx_restart(&mut self) {
        if self.abx.is_none() {
            return;
        }
        let seed = self.abx_seed();
        if let Some(test) = &mut self.abx {
            test.restart(seed);
        }
    }

    /// Ends the comparison, stopping whatever it was playing.
    pub fn close_abx(&mut self) {
        if self.abx.take().is_some() {
            self.stop();
        }
        self.abx_reading = None;
        self.abx_sums = None;
    }

    /// The album a track belongs to.
    pub fn album_of(&self, track: usize) -> Option<usize> {
        self.library
            .albums
            .iter()
            .position(|album| album.tracks.contains(&track))
    }

    /// The playing track's name, as the popup shows it.
    ///
    /// It is the title, or the file name when the table is set to show file
    /// names instead of titles, so the popup and the table agree. `None`
    /// when nothing is playing, or the track is no longer in the library.
    pub fn playing_name(&self) -> Option<String> {
        // The preview's track is in no playlist, so it is named on its own.
        let track = match &self.preview {
            Some(track) => track,
            None => self.library.tracks.get(self.playing?)?,
        };
        Some(match crate::ui::name_column(self.columns_shown) {
            Some(crate::ui::Column::FileName) => track.file_name(self.file_extension),
            _ => track.title.clone(),
        })
    }

    /// Track indices in the playlist the table shows, filtered by the search box.
    ///
    /// The title, the artist and the album are searched, all of them in lower
    /// case, so a search reads the way the row does.
    pub fn visible_tracks(&self) -> Vec<usize> {
        let needle = self.view.filter.trim().to_lowercase();
        self.playlists
            .current()
            .tracks
            .iter()
            .copied()
            .filter(|&index| {
                needle.is_empty()
                    || self.library.tracks.get(index).is_some_and(|track| {
                        matches(&needle, &track.title)
                            || matches(&needle, &track.artist)
                            || matches(&needle, &track.album)
                    })
            })
            .collect()
    }

    /// Selects the next or previous row the table shows, and scrolls to it.
    ///
    /// The walk stops at the ends rather than wrapping, which is what a list
    /// does: holding the key does not send the selection back to the top.
    pub fn step_selection(&mut self, forward: bool) {
        let tracks = self.visible_tracks();
        if tracks.is_empty() {
            return;
        }
        let position = self
            .view
            .selected
            .and_then(|selected| tracks.iter().position(|&track| track == selected));
        let next = match (position, forward) {
            (Some(position), true) => (position + 1).min(tracks.len() - 1),
            (Some(position), false) => position.saturating_sub(1),
            (None, _) => 0,
        };
        self.select_only(tracks[next]);
        self.view.reveal = true;
    }

    /// Selects the first or the last row the table shows.
    pub fn select_edge(&mut self, last: bool) {
        let tracks = self.visible_tracks();
        let edge = if last { tracks.last() } else { tracks.first() };
        if let Some(&index) = edge {
            self.select_only(index);
            self.view.reveal = true;
        }
    }

    /// Plays the selected track, if one is selected.
    ///
    /// Playing a row by hand goes back to walking the playlist, rather than
    /// the queue an earlier selection started.
    pub fn play_selection(&mut self) {
        // A row cannot be played while the preview shows: the preview window
        // draws no table, so a press that reached here came from the keyboard
        // on a window whose rows are not there.
        if self.preview.is_some() {
            return;
        }
        let Some(index) = self.view.selected else {
            return;
        };
        self.queue = None;
        self.forget_shuffle();
        self.play(index);
    }

    /// Opens the search box and takes the keyboard into it.
    ///
    /// What is already typed stays, so pressing the key twice does not lose a
    /// search that is under way. The compact window shows no playlist, so it
    /// has nothing to search.
    pub fn open_search(&mut self) {
        if self.compact {
            return;
        }
        self.view.searching = true;
        self.view.search_focus = true;
    }

    /// Closes the search box and shows the whole playlist again.
    pub fn close_search(&mut self) {
        self.view.searching = false;
        self.view.search_focus = false;
        self.view.filter.clear();
    }

    /// Selects the first track the search leaves, and scrolls to it.
    ///
    /// This runs as the text changes, so the table follows the typing rather
    /// than waiting for Enter.
    pub fn refilter(&mut self) {
        if let Some(&first) = self.visible_tracks().first() {
            self.select_only(first);
            self.view.reveal = true;
        }
    }

    /// Selects the next or previous track the search leaves, and scrolls to it.
    ///
    /// The walk wraps around, so Enter keeps going through the matches rather
    /// than stopping at the last one.
    pub fn step_match(&mut self, forward: bool) {
        let tracks = self.visible_tracks();
        if tracks.is_empty() {
            return;
        }
        let position = self
            .view
            .selected
            .and_then(|selected| tracks.iter().position(|&track| track == selected));
        let next = match (position, forward) {
            (Some(position), true) => (position + 1) % tracks.len(),
            (Some(position), false) => (position + tracks.len() - 1) % tracks.len(),
            (None, _) => 0,
        };
        self.select_only(tracks[next]);
        self.view.reveal = true;
    }

    /// Texture for the cover stored under `key`, uploading it once decoded.
    ///
    /// A picture the thread has not decoded yet is asked for and answers
    /// `None`, which the caller draws as a placeholder; the frame after it
    /// arrives draws the art. A key with no picture behind it is remembered as
    /// such by [`crate::art::Art`], so a cache that lost one is not asked for
    /// it again.
    pub fn cover_texture(&mut self, ctx: &egui::Context, key: &str) -> Option<egui::TextureHandle> {
        if let Some(texture) = self.textures.get(key) {
            return Some(texture.clone());
        }
        let Some(cover) = self.pictures.decoded(key) else {
            self.pictures.request(key);
            return None;
        };
        let cover = cover?;
        let texture = upload(ctx, key, &cover);
        self.textures.insert(key.to_string(), texture.clone());
        Some(texture)
    }

    /// Texture for the cover of the loaded track, at the size it is drawn.
    ///
    /// The album lists draw covers the size of a row, which the thumbnail
    /// decoded for them fills exactly. The square above the controls is drawn
    /// as wide as the window instead, so the thumbnail stretched over it reads
    /// blurry: the picture is decoded again at the size that square has, in
    /// whole pixels of the screen, and kept as the one texture the window
    /// shows at that size. Dropping the pixels once they are uploaded is what
    /// keeps that from being a cover per track ever played: a decoded picture
    /// this large is a few megabytes, and the textures are what is worth
    /// keeping.
    ///
    /// A picture still being decoded leaves the thumbnail on screen rather
    /// than nothing, so the art does not blink as the square takes a new size.
    pub fn playing_art(&mut self, ctx: &egui::Context, side: f32) -> Option<egui::TextureHandle> {
        let Some(key) = self.loaded_cover_key() else {
            // Nothing loaded, so the big picture of the last track is dropped
            // rather than kept for a square that no longer shows art.
            self.big_art = None;
            self.wait_for_big(None);
            return None;
        };
        let pixels = (side * ctx.pixels_per_point()).round().max(1.0) as u32;
        let size = crate::cover::decoded_size(pixels);
        if size <= crate::cover::THUMBNAIL {
            // The square is small enough that the thumbnail fills it, so the
            // picture of its own size is not wanted any more either.
            self.big_art = None;
            self.wait_for_big(None);
            return self.cover_texture(ctx, &key);
        }
        if let Some(big) = &self.big_art {
            if big.side == size && big.key == key {
                return Some(big.texture.clone());
            }
            // The square has taken another size, or another track is loaded:
            // the texture held is not one this frame draws, and keeping it
            // would collect one per step of a resize.
            self.big_art = None;
        }
        let asked = crate::art::sized_key(&key, size);
        self.wait_for_big(Some(&asked));
        let Some(cover) = self.pictures.decoded(&asked) else {
            self.pictures.request(&asked);
            return self.cover_texture(ctx, &key);
        };
        let Some(cover) = cover else {
            // No picture behind the key, which the thumbnail path has already
            // answered for.
            return self.cover_texture(ctx, &key);
        };
        let texture = upload(ctx, &asked, &cover);
        self.pictures.forget(&[asked]);
        self.big_art = Some(BigArt {
            key,
            side: size,
            texture: texture.clone(),
        });
        Some(texture)
    }

    /// Makes `key` the picture size the square is waiting for, dropping the
    /// one it waited for before.
    ///
    /// A size the square grew past is not waited for: its decode is a few
    /// megabytes, and the frame that asked for it is not the frame that draws
    /// it any more. Dropping it here is what keeps a window widened while the
    /// picture decoded from holding both sizes, the smaller one for good.
    fn wait_for_big(&mut self, key: Option<&str>) {
        if self.big_asked.as_deref() == key {
            return;
        }
        if let Some(previous) = self.big_asked.take() {
            self.pictures.forget(&[previous]);
        }
        self.big_asked = key.map(str::to_string);
    }

    /// The key of the cover of the loaded track, when it has one.
    ///
    /// An album's cover is shared by its tracks; a loose track has no album
    /// to take one from, so its own artwork is used. The preview's file is in
    /// no library, so its cover is the file's own.
    fn loaded_cover_key(&self) -> Option<String> {
        if let Some(preview) = &self.preview {
            return preview.cover.clone();
        }
        let track = self.playing?;
        if let Some(album) = self.album_of(track) {
            return self.library.albums.get(album)?.cover.clone();
        }
        self.library.tracks.get(track)?.cover.clone()
    }

    /// Texture for an album's cover.
    pub fn album_cover(
        &mut self,
        ctx: &egui::Context,
        album: usize,
    ) -> Option<egui::TextureHandle> {
        let key = self.library.albums.get(album)?.cover.clone()?;
        self.cover_texture(ctx, &key)
    }

    /// Whether the playing track has a picture, decoded or not.
    ///
    /// The layout picks its size from this rather than from the texture: the
    /// picture arrives a frame or two after it is asked for, and a window
    /// that sized itself in the meantime would snap to the no-art size and
    /// keep it.
    pub fn playing_has_art(&self) -> bool {
        if let Some(preview) = &self.preview {
            return preview.cover.is_some();
        }
        self.playing.is_some_and(|track| {
            self.album_of(track)
                .and_then(|album| self.library.albums.get(album))
                .and_then(|album| album.cover.as_ref())
                .is_some()
                || self
                    .library
                    .tracks
                    .get(track)
                    .is_some_and(|track| track.cover.is_some())
        })
    }

    /// The playback position, following the seek bar while it is dragged.
    pub fn position(&self) -> Duration {
        match self.seek {
            Some(position) => position,
            None => self.audio.as_ref().map_or(Duration::ZERO, Audio::position),
        }
    }

    /// Seeks to a fraction of the track.
    pub fn seek_to(&mut self, fraction: f32) {
        let target = self.length.mul_f32(fraction.clamp(0.0, 1.0));
        self.seek_to_position(target);
        self.seek = None;
    }

    /// Seeks to `position`, reporting a decoder that cannot seek.
    pub fn seek_to_position(&mut self, position: Duration) {
        let Some(audio) = &self.audio else {
            return;
        };
        match audio.seek(position) {
            Ok(()) => self.status = None,
            Err(error) => self.status = Some(format!("Cannot seek: {error}")),
        }
    }

    /// Sets the volume and applies it. The settings file is written when
    /// the slider is let go, not on every frame of a drag.
    pub fn set_volume(&mut self, volume: f32) {
        self.volume = volume.clamp(0.0, 1.0);
        if let Some(audio) = &self.audio {
            audio.set_volume(self.volume);
        }
    }

    /// Sets the widths of the track table's columns, and remembers them.
    pub fn set_columns(&mut self, columns: [f32; 6]) {
        self.columns = sane_columns(columns);
        self.config.column_widths = Some(self.columns);
        self.save_settings();
    }

    /// Shows or hides one of the track table's columns, and remembers it.
    pub fn set_column_shown(&mut self, index: usize, shown: bool) {
        if let Some(column) = self.columns_shown.get_mut(index) {
            *column = shown;
        }
        self.config.column_shown = Some(self.columns_shown);
        self.save_settings();
    }

    /// Chooses whether the file name column keeps the extension, and
    /// remembers it.
    pub fn set_file_extension(&mut self, on: bool) {
        self.file_extension = on;
        self.config.file_extension = Some(on);
        self.save_settings();
    }

    /// Picks a theme, applies it, and remembers it.
    pub fn set_theme(&mut self, theme: crate::theme::Theme) {
        self.theme = theme;
        self.config.theme = Some(theme.key().to_string());
        self.save_settings();
    }

    /// Turns repeat on or off, and remembers it.
    pub fn set_repeat(&mut self, repeat: bool) {
        self.repeat = repeat;
        self.config.repeat = Some(repeat);
        self.save_settings();
    }

    /// Turns random playback on or off, and remembers it.
    ///
    /// Switching it on starts a new shuffle, so the order follows the
    /// playlist as it stands now rather than as it stood when it was last on.
    pub fn set_random(&mut self, random: bool) {
        self.random = random;
        self.forget_shuffle();
        self.config.random = Some(random);
        self.save_settings();
    }

    /// Turns the translucent background on or off, and remembers it.
    pub fn set_transparency(&mut self, transparency: bool) {
        self.transparency = transparency;
        self.config.transparency = Some(transparency);
        self.save_settings();
    }

    /// Turns the single-file preview on or off, and remembers it.
    ///
    /// Turning it off while the preview is showing leaves it for the window,
    /// so the setting and what is drawn never disagree: the preview has no
    /// menu bar to turn it back on from.
    pub fn set_preview_enabled(&mut self, ctx: &egui::Context, enabled: bool) {
        self.preview_enabled = enabled;
        self.config.preview = Some(enabled);
        self.save_settings();
        if !enabled {
            self.leave_preview(ctx);
        }
    }

    /// Keeps the music playing when the window is closed, or stops doing so.
    pub fn set_background(&mut self, background: bool) {
        self.background = background;
        self.config.background = Some(background);
        self.save_settings();
    }

    /// Fits the album art into its square whole, or crops it to fill.
    ///
    /// Art that is not square cannot do both: filling the space crops it,
    /// and showing all of it leaves a band of background beside it.
    pub fn set_cover_fit(&mut self, cover_fit: bool) {
        self.cover_fit = cover_fit;
        self.config.cover_fit = Some(cover_fit);
        self.save_settings();
    }

    /// Turns the automatic scan on or off, and remembers it.
    ///
    /// Switching it on scans at once: the setting covers the scan at startup
    /// and the one on a timer, and waiting a minute is long enough for the
    /// new setting to look as though it did nothing.
    pub fn set_auto_scan(&mut self, auto_scan: bool) {
        self.auto_scan = auto_scan;
        self.config.auto_scan = Some(auto_scan);
        self.save_settings();
        if auto_scan && self.scan.is_none() && !self.config.roots().is_empty() {
            self.start_scan();
        }
    }

    /// Leaves hidden files out of the scan, or stops doing so.
    ///
    /// The scan has to run again for the change to show: what is in the
    /// library was decided when it was read.
    pub fn set_ignore_hidden(&mut self, ignore_hidden: bool) {
        self.ignore_hidden = ignore_hidden;
        self.config.ignore_hidden = Some(ignore_hidden);
        self.save_settings();
        if !self.config.roots().is_empty() {
            self.start_scan();
        }
    }

    /// Whether closing the window should leave the music playing.
    ///
    /// Quit always ends the app, and a close with nothing loaded does too: a
    /// window that would only sit hidden has no reason to stay.
    fn keeps_playing_on_close(&self) -> bool {
        self.background && !self.quitting && self.playing.is_some()
    }

    /// The accent colour of the active theme.
    pub fn accent(&self) -> egui::Color32 {
        self.theme.palette().accent
    }

    /// How much of the desktop shows through the window, from 0 to 1.
    pub fn opacity(&self) -> f32 {
        if self.transparency {
            crate::theme::TRANSLUCENT
        } else {
            1.0
        }
    }

    /// Writes the settings to disk, unless this is a smoke run.
    ///
    /// The volume is taken from the live value each time, so a save made for
    /// another reason still records it.
    fn save_settings(&mut self) {
        if !self.persist {
            return;
        }
        self.config.volume = Some(self.volume);
        if let Err(error) = self.config.save() {
            eprintln!("blue-dove: {error}");
        }
    }

    /// Writes the settings to disk, with the volume as it now stands.
    pub fn save_config(&mut self) {
        self.save_settings();
    }

    /// Writes the open playlists to the settings file.
    ///
    /// Every change to a playlist or its tabs ends here, so what the window
    /// shows is what the next start opens, even after a crash. Tracks are
    /// stored by path; see [`SavedPlaylist`] for why.
    pub fn save_playlists(&mut self) {
        self.config.playlists = Some(
            self.playlists
                .items
                .iter()
                .map(|playlist| SavedPlaylist {
                    name: playlist.name.clone(),
                    tracks: playlist
                        .tracks
                        .iter()
                        .filter_map(|&index| track_path(&self.library, index))
                        .collect(),
                })
                .collect(),
        );
        self.config.active_playlist = Some(self.playlists.active);
        self.config.selected_track = self
            .view
            .selected
            .and_then(|index| track_path(&self.library, index));
        self.save_settings();
    }

    /// Swaps in a freshly built library, carrying the playlists over to it.
    ///
    /// A playlist holds indices into the library, and a scan produces a new
    /// list, so the indices have to be found again by path. A track that is
    /// gone drops out of its playlist; the playing track is found the same
    /// way, so a rescan while something plays does not stop it.
    pub fn adopt_library(&mut self, library: Library) {
        let paths: Vec<Vec<PathBuf>> = self
            .playlists
            .items
            .iter()
            .map(|playlist| {
                playlist
                    .tracks
                    .iter()
                    .filter_map(|&index| track_path(&self.library, index))
                    .collect()
            })
            .collect();
        let selected = self
            .view
            .selected
            .and_then(|index| track_path(&self.library, index));
        let playing = self
            .playing
            .and_then(|index| track_path(&self.library, index));

        // A scan that finds nothing, with a playlist that held something, is
        // a directory that went away rather than one that was emptied: the
        // library the window lists, the playlists and the session are all left
        // as they were. Taking the empty answer in would empty the window
        // under the user, with every row it drew pointing at nothing, which is
        // what an unmounted share looks like from here.
        if library.is_empty() && paths.iter().any(|tracks| !tracks.is_empty()) {
            return;
        }

        // The queue a "play the selection" started is indices into the track
        // list about to be replaced, so it is found again by path, the way the
        // playlists are.
        let queue: Option<Vec<PathBuf>> = self.queue.as_ref().map(|indices| {
            indices
                .iter()
                .filter_map(|&index| track_path(&self.library, index))
                .collect()
        });

        self.library = library;
        self.remap_queue(queue);
        // Textures and decoded pictures are keyed by the content of a cover,
        // so a rescan that names the same ones keeps them: clearing both would
        // blink the visible art back to placeholders and decode it again.
        let covers = self.library.covers();
        let named: HashSet<String> = covers.iter().cloned().collect();
        self.textures.retain(|key, _| named.contains(key));
        self.pictures.retain(&named);
        // A key whose bytes were gone when it was last asked for may be there
        // now, since the scan that produced this library stores what it finds.
        self.pictures.forget_missing();
        self.pictures.prefetch(covers);
        self.view.album = (!self.library.albums.is_empty()).then_some(0);

        if paths.iter().any(|tracks| !tracks.is_empty()) {
            self.remap_playlists(paths, selected, playing);
        } else {
            self.restore_playlists();
        }
    }

    /// Finds the walking queue again in the library just scanned.
    ///
    /// A queue holds indices into the track list, which a scan replaces: a
    /// file added above the playing track shifts every index below it, and the
    /// next step would walk to a track that is not there, which stops playback
    /// without a word. The shuffle order is dropped rather than remapped,
    /// since it is a drawn order and drawing it again costs nothing.
    fn remap_queue(&mut self, queue: Option<Vec<PathBuf>>) {
        if let Some(paths) = queue {
            let positions = track_positions(&self.library);
            let found: Vec<usize> = paths
                .iter()
                .filter_map(|path| positions.get(path.as_path()).copied())
                .collect();
            // A queue whose tracks all went away falls back to the playlist,
            // which is what walking it would do by hand.
            self.queue = (!found.is_empty()).then_some(found);
        }
        if self.shuffle.is_some() {
            self.forget_shuffle();
        }
    }

    /// Finds the tracks of the playlists again in the library just scanned.
    fn remap_playlists(
        &mut self,
        paths: Vec<Vec<PathBuf>>,
        selected: Option<PathBuf>,
        playing: Option<PathBuf>,
    ) {
        let positions = track_positions(&self.library);
        let find = |path: &PathBuf| positions.get(path.as_path()).copied();

        for (playlist, tracks) in self.playlists.items.iter_mut().zip(paths) {
            playlist.set(tracks.iter().filter_map(&find));
        }
        self.playlists.active = self.playlists.active.min(self.playlists.items.len() - 1);

        self.playing = playing.as_ref().and_then(find);
        let selected = selected
            .as_ref()
            .and_then(find)
            .filter(|index| self.playlists.current().tracks.contains(index));
        self.select_restored(selected);
        self.save_playlists();
    }

    /// Reopens the playlists the last run left open.
    ///
    /// The scan is what turns a saved path back into a track, so this runs
    /// when it finishes. A playlist whose files are gone keeps its tab and
    /// loses those tracks, so the shape of the session survives a rename or
    /// a move; when no track at all is found, which is what a drive that is
    /// not mounted looks like, the saved list is left alone so the next run
    /// can still open it.
    fn restore_playlists(&mut self) {
        let Some(saved) = self.config.playlists.take() else {
            return;
        };
        let positions = track_positions(&self.library);

        let mut items: Vec<crate::playlist::Playlist> = saved
            .iter()
            .map(|saved| {
                let mut playlist = crate::playlist::Playlist::new(saved.name.clone());
                playlist.add(
                    saved
                        .tracks
                        .iter()
                        .filter_map(|path| positions.get(path.as_path()).copied()),
                );
                playlist
            })
            .collect();
        if items.iter().all(|playlist| playlist.tracks.is_empty()) {
            self.config.playlists = Some(saved);
            return;
        }

        let active = self
            .config
            .active_playlist
            .unwrap_or(0)
            .min(items.len() - 1);
        let selected = self
            .config
            .selected_track
            .as_deref()
            .and_then(|path| positions.get(path).copied())
            .filter(|index| items[active].tracks.contains(index));

        self.playlists = Playlists {
            items: std::mem::take(&mut items),
            active,
        };
        self.select_restored(selected);
    }

    /// Picks the layout for the window's size, and snaps a window that has
    /// shrunk into the compact one to its fixed size.
    ///
    /// The snap happens once, when the window crosses into the compact
    /// layout, so the window manager is not fought while the user drags:
    /// after that the window can be resized freely, and growing it past the
    /// break brings the album list and the playlist back.
    fn update_layout(&mut self, ctx: &egui::Context) {
        let compact = compact_for(ctx.viewport_rect().size());
        if compact == self.compact {
            return;
        }
        self.compact = compact;
        // The preview pins itself to one size and is not the layout's to
        // resize.
        if self.preview.is_some() {
            return;
        }
        if compact {
            // The popup picks the size its content needs, so a track with no
            // art does not carry the height the square would have taken as
            // empty background. That size is also its floor: it can be made
            // larger, but never smaller than the layout it picked, or the
            // controls and the waveform would be squeezed out of a window
            // that has no room for them. The ceiling is lifted so growing
            // back past the break returns to the full layout.
            let size = if self.playing_has_art() {
                COMPACT_SIZE
            } else {
                COMPACT_NO_ART_SIZE
            };
            ctx.send_viewport_cmd(egui::ViewportCommand::MaxInnerSize(egui::vec2(
                f32::INFINITY,
                f32::INFINITY,
            )));
            ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(egui::vec2(
                size[0], size[1],
            )));
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                size[0], size[1],
            )));
        } else {
            // Back in the full layout, the floor drops to the smallest the
            // window may be at all, which is what lets it be dragged down
            // into the popup again.
            ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(egui::vec2(
                COMPACT_MIN_SIZE[0],
                COMPACT_MIN_SIZE[1],
            )));
        }
    }

    /// Takes in the commands D-Bus clients sent since the last frame.
    ///
    /// The service arrives on a channel of its own, since connecting to the
    /// bus takes a moment; until it does, this does nothing, and when the
    /// host has no bus at all it never does anything.
    fn poll_mpris(&mut self, ctx: &egui::Context, window: Option<&Window>) {
        let mut connected = false;
        if let Some(receiver) = &self.service
            && let Ok(service) = receiver.try_recv()
        {
            // A host without a session bus, or another player holding the
            // name, answers `None`: the app carries on without it.
            self.mpris = service;
            self.service = None;
            connected = self.mpris.is_some();
            if !connected {
                self.remote = None;
            }
        }

        // Collected first: draining while calling `apply` would hold a
        // borrow of `self` across the call.
        let commands: Vec<Command> = self
            .remote
            .iter()
            .flat_map(|receiver| receiver.try_iter())
            .collect();
        for command in commands {
            // A raise is the one command that needs the window itself: it has
            // to be shown again when a close left it hidden or minimized, and
            // only the frame holds the handle that does it.
            if command == Command::Raise {
                self.raise_window(ctx, window);
            } else {
                self.apply(command);
            }
        }

        if connected || self.mpris.is_some() {
            self.publish();
            // A client asking for the position expects it to move, and the
            // app only repaints on demand. The draw pass already asks for
            // this while a track plays; a quarter second is enough for a
            // client that reads the position.
            if self.playing.is_some() {
                ctx.request_repaint_after(Duration::from_millis(250));
            }
        }
    }

    /// Moves the player on when a track runs out, and keeps it waking.
    ///
    /// A window that is hidden draws nothing, so this cannot live in the
    /// drawing pass: a track that ended would never hand over to the next.
    fn advance(&mut self, ctx: &egui::Context) {
        if self.seek.is_some() {
            return;
        }
        // A blind test owns the player: a trial that runs out is not a track
        // to hand over from, and stepping would play something the listener
        // never chose.
        if self.abx.is_some() {
            if !self.audio.as_ref().is_some_and(|audio| audio.is_empty()) {
                ctx.request_repaint_after(Duration::from_millis(20));
            }
            return;
        }
        // The preview has a track of its own and no playlist behind it, so
        // it is handled before the playlist walk below, which has nothing to
        // step to.
        if self.preview.is_some() {
            if !self.audio.as_ref().is_some_and(|audio| audio.is_empty()) {
                ctx.request_repaint_after(Duration::from_millis(20));
                return;
            }
            if self.repeat
                && let Some(track) = self.preview.take()
            {
                self.open_preview(ctx, &track.path);
            }
            return;
        }
        let Some(index) = self.playing else {
            return;
        };
        if !self.audio.as_ref().is_some_and(|audio| audio.is_empty()) {
            // The seek bar's cursor moves with the position, so the window
            // has to draw while a track plays. A short track moves the
            // cursor across the bar quickly, and a slow rate shows that as
            // jumps: twenty milliseconds is fifty frames a second, which
            // stays smooth even then.
            ctx.request_repaint_after(Duration::from_millis(20));
            return;
        }
        if self.repeat {
            self.play(index);
        } else {
            self.step(true);
        }
    }

    /// Hides the window on close instead of quitting, when asked to.
    ///
    /// The audio runs on a thread of its own, so hiding leaves the music
    /// playing. Quit in the File menu still ends the app, and launching it
    /// again brings the window back.
    fn handle_close(&mut self, ctx: &egui::Context, window: Option<&Window>) {
        if !ctx.input(|input| input.viewport().close_requested()) {
            return;
        }
        if !self.keeps_playing_on_close() {
            return;
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        // The window itself is asked, rather than a viewport command left for
        // eframe to apply after the pass: this way it leaves the screen as
        // the close was clicked, not a frame later.
        match (window, on_wayland()) {
            // Wayland has no request that hides a window: the compositor owns
            // visibility, and winit ignores `set_visible` there. `xdg_toplevel`
            // does have `set_minimized`, so that is what a close asks for, and
            // the window leaves the screen with the music still playing.
            (Some(window), true) => window.set_minimized(true),
            (Some(window), false) => window.set_visible(false),
            (None, true) => ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true)),
            (None, false) => ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false)),
        }
        self.save_playlists();
    }

    /// Brings the window back when another launch asked for it.
    fn poll_requests(&mut self, ctx: &egui::Context, window: Option<&Window>) {
        let Some(receiver) = &self.requests else {
            return;
        };
        let requests: Vec<crate::instance::Request> = receiver.try_iter().collect();
        if requests.is_empty() {
            return;
        }
        let mut raise = false;
        for request in requests {
            raise |= self.take_request(ctx, request);
        }
        // The request was carried out after this frame's state was published,
        // and a stop leaves nothing that asks for another frame, so the
        // remote control would keep reporting the old state.
        ctx.request_repaint();
        if raise {
            self.raise_window(ctx, window);
        }
    }

    /// Carries out one request from another launch.
    ///
    /// Answers whether the window should be brought forward: an open asks to
    /// be seen, and a transport command does not, since a media key from a
    /// script should not pull the window up in front of the user.
    fn take_request(&mut self, ctx: &egui::Context, request: crate::instance::Request) -> bool {
        match request {
            crate::instance::Request::Show => true,
            crate::instance::Request::Action(action) => {
                self.apply(action.command());
                false
            }
            crate::instance::Request::Open(paths) => {
                // A launch naming one file is a preview, whether it started
                // the window or found this one running: a file manager sends
                // the same request either way, and the second file opened
                // while the player runs has to behave like the first.
                match Self::preview_candidate(&paths) {
                    Some(path) if self.preview_enabled => {
                        let path = path.clone();
                        self.open_preview(ctx, &path);
                    }
                    _ => self.open_paths(ctx, &paths),
                }
                true
            }
        }
    }

    /// Brings the window back to the screen.
    fn raise_window(&self, ctx: &egui::Context, window: Option<&Window>) {
        match window {
            Some(window) => {
                window.set_visible(true);
                window.set_minimized(false);
                window.focus_window();
            }
            None => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
        }
        // Wayland refuses all three of those: a client cannot show,
        // unminimize or focus its own window there. xdg-activation is the one
        // request a compositor honours, and a window asking for critical
        // attention is raised by mutter, which is how a minimized window
        // comes back after a close left it that way.
        if on_wayland() {
            ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(
                egui::UserAttentionType::Critical,
            ));
        }
        // The window was hidden while it had nothing to draw, so the first
        // pass after it comes back has to draw a whole frame.
        ctx.request_repaint();
    }

    /// Starts answering launches that ask this window to do something.
    ///
    /// The listener wakes the window through the same waker the remote
    /// control uses, since the app only draws when something asks it to.
    pub fn watch_requests(&mut self, listener: TcpListener, ctx: &egui::Context) {
        let ctx = ctx.clone();
        let waker: crate::instance::Waker = std::sync::Arc::new(move || ctx.request_repaint());
        self.requests = Some(crate::instance::serve(listener, waker));
    }

    /// Whether a command from a D-Bus client applies to what plays now.
    ///
    /// The preview plays one file with no playlist behind it, so Play and
    /// Pause are the only transport it has: a step would leave the file for a
    /// track of the playlist the preview never touched, and a stop has
    /// nowhere to put the window back to.
    fn remote_command_applies(&self, command: Command) -> bool {
        self.preview.is_none()
            || !matches!(command, Command::Next | Command::Previous | Command::Stop)
    }

    /// Carries out one command from a D-Bus client.
    fn apply(&mut self, command: Command) {
        if !self.remote_command_applies(command) {
            return;
        }
        match command {
            Command::Play if self.playing.is_none() || self.paused() => self.toggle(),
            Command::Play => {}
            Command::Pause if !self.paused() => self.toggle(),
            Command::Pause => {}
            Command::PlayPause => self.toggle(),
            Command::Stop => self.stop(),
            Command::Next => self.step(true),
            Command::Previous => self.step(false),
            Command::Seek(offset) => self.seek_to_position(Duration::from_micros(
                u64::try_from(
                    self.position()
                        .as_micros()
                        .saturating_add_signed(i128::from(offset)),
                )
                .unwrap_or(0),
            )),
            Command::SetPosition(position) => {
                self.seek_to_position(Duration::from_micros(position.max(0) as u64));
            }
            Command::SetVolume(volume) => self.set_volume(volume as f32),
            Command::SetRepeat(repeat) => self.set_repeat(repeat),
            Command::SetShuffle(shuffle) => self.set_random(shuffle),
            // Raising needs the window itself, which only the frame holds,
            // so `poll_mpris` takes the command before it reaches here.
            Command::Raise => {}
        }
    }

    /// The art file MPRIS names for the loaded track, when it has a cover.
    ///
    /// The picture is written once per track and kept: a frame that encoded
    /// it again would spend more time on the PNG than on the window. What is
    /// kept is dropped as soon as nothing plays, so a stop leaves the
    /// metadata without art rather than with the art of the last track.
    fn art_path(&mut self) -> Option<PathBuf> {
        let Some(cover_key) = self.described_track().map(|track| track.cover.clone()) else {
            // Nothing is loaded, so the art of the last track is forgotten
            // rather than left beside an empty title.
            self.art = None;
            return None;
        };
        let Some(key) = cover_key else {
            // The loaded track carries no cover, which is remembered so a
            // later frame does not look for one again.
            self.art = Some(None);
            return None;
        };
        match &self.art {
            // Nothing to write for this track, or a write that failed once
            // and would fail the same way on the next frame.
            Some(None) => return None,
            Some(Some(path)) if path.is_file() => return Some(path.clone()),
            _ => {}
        }
        // The picture is usually on screen already, so it is taken from there;
        // a track that plays without being drawn is decoded here instead.
        let cover = match self.pictures.decoded(&key) {
            Some(cover) => cover,
            None => crate::art::decode_now(&key).map(std::sync::Arc::new),
        };
        let path = cover
            .as_deref()
            .and_then(|cover| crate::cache::art_file(&key, cover));
        self.art = Some(path.clone());
        path
    }

    /// The track MPRIS describes: the preview's when one is showing, else the
    /// track that plays.
    ///
    /// A previewed file is read on its own and is in no playlist, so it has no
    /// index into the library. Asking the library for the playing track alone
    /// would leave a client with no title, no album and no art for the one file
    /// the window is looking at.
    fn described_track(&self) -> Option<&Track> {
        if let Some(preview) = &self.preview {
            return Some(preview);
        }
        self.playing
            .and_then(|index| self.library.tracks.get(index))
    }

    /// Tells the service what the player is doing.
    fn publish(&mut self) {
        if self.mpris.is_none() {
            return;
        }
        let state = self.mpris_state();
        if let Some(service) = &self.mpris {
            service.update(state);
        }
    }

    /// What the player tells a client about itself.
    fn mpris_state(&mut self) -> crate::mpris::State {
        // The art is written before the tracks are borrowed, since writing it
        // needs the app mutably.
        let art = self.art_path();
        let track = self.described_track();
        let title = track.map(|track| track.title.clone()).unwrap_or_default();
        let artists: Vec<String> = track
            .map(|track| {
                track
                    .artist
                    .split(';')
                    .map(str::trim)
                    .filter(|artist| !artist.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let album = track.map(|track| track.album.clone()).unwrap_or_default();
        let loaded = track.is_some();
        // The preview is one file, and the playlist it was opened over is not
        // what it walks: a client offered Next there would step out of the
        // file and into a track the listener never asked for.
        let stepable = self.preview.is_none() && self.walked_tracks().len() > 1;
        let playback = match loaded {
            false => Playback::Stopped,
            true if self.paused() => Playback::Paused,
            true => Playback::Playing,
        };
        crate::mpris::State {
            playback,
            title,
            artists,
            album,
            length: self.length.as_micros() as i64,
            position: self.position().as_micros() as i64,
            volume: f64::from(self.volume),
            repeat: self.repeat,
            shuffle: self.random,
            can_seek: loaded && !self.length.is_zero(),
            can_go_next: stepable,
            can_go_previous: stepable,
            track: match (self.preview.is_some(), self.playing) {
                (true, _) => PREVIEW_TRACK,
                (false, Some(index)) => index as u64,
                (false, None) => 0,
            },
            art,
        }
    }

    /// Moves the playback position by `seconds`, staying inside the track.
    fn seek_relative(&mut self, seconds: f32) {
        if self.playing.is_none() || self.length.is_zero() {
            return;
        }
        let target = self.position().as_secs_f32() + seconds;
        self.seek_to(target / self.length.as_secs_f32());
    }

    /// Handles the keyboard shortcuts, unless a text box has the focus.
    fn handle_keys(&mut self, ctx: &egui::Context) {
        // Escape belongs to the search box while it is open, and the box may
        // hold the keyboard, so it is read before the focus check below.
        if self.view.searching && ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
            self.close_search();
        }
        // Down leaves the search box for the table, so the results it found
        // can be walked with the arrows. The text stays, and the box is one
        // press of the key away again.
        if self.view.searching
            && ctx.memory(|memory| memory.focused().is_some())
            && ctx.input(|input| input.key_pressed(egui::Key::ArrowDown))
        {
            ctx.memory_mut(|memory| memory.stop_text_input());
            self.step_selection(true);
            return;
        }
        if ctx.memory(|memory| memory.focused().is_some()) {
            return;
        }
        let pressed = |key| ctx.input(|input| input.key_pressed(key));

        if pressed(egui::Key::Space) {
            self.toggle();
        }
        if pressed(egui::Key::Delete) {
            self.remove_selected();
        }
        if chord(ctx, egui::Modifiers::COMMAND, egui::Key::A) {
            self.select_all();
        }
        if chord(ctx, egui::Modifiers::COMMAND, egui::Key::O) {
            self.directories_open = true;
        }
        if chord(ctx, egui::Modifiers::COMMAND, egui::Key::F) {
            self.open_search();
        }
        // A bare slash opens the search too, the way it does in a pager. Its
        // text goes with the key, or the box opens with a slash in it, and a
        // shifted slash is a question mark that stays a character.
        if chord(ctx, egui::Modifiers::NONE, egui::Key::Slash) {
            ctx.input_mut(|input| {
                input
                    .events
                    .retain(|event| !matches!(event, egui::Event::Text(text) if text == "/"));
            });
            self.open_search();
        }
        if chord(ctx, egui::Modifiers::NONE, egui::Key::Enter) {
            self.play_selection();
        }
        if chord(ctx, egui::Modifiers::COMMAND, egui::Key::ArrowRight) {
            self.step(true);
        } else if pressed(egui::Key::ArrowRight) {
            self.seek_relative(5.0);
        }
        if chord(ctx, egui::Modifiers::COMMAND, egui::Key::ArrowLeft) {
            self.step(false);
        } else if pressed(egui::Key::ArrowLeft) {
            self.seek_relative(-5.0);
        }
        // The arrows walk the table, which is what a list does; a chord with
        // Shift jumps to its ends and one with Control turns the volume, so
        // the plain keys are free for the list. Most specific first, since
        // each match consumes the press.
        if chord(ctx, egui::Modifiers::SHIFT, egui::Key::ArrowUp) {
            self.select_edge(false);
        } else if chord(ctx, egui::Modifiers::SHIFT, egui::Key::ArrowDown) {
            self.select_edge(true);
        } else if chord(ctx, egui::Modifiers::COMMAND, egui::Key::ArrowUp) {
            let volume = self.volume;
            self.set_volume(volume + 0.05);
            self.save_config();
        } else if chord(ctx, egui::Modifiers::COMMAND, egui::Key::ArrowDown) {
            let volume = self.volume;
            self.set_volume(volume - 0.05);
            self.save_config();
        } else if pressed(egui::Key::ArrowUp) {
            self.step_selection(false);
        } else if pressed(egui::Key::ArrowDown) {
            self.step_selection(true);
        }
    }

    /// Counts frames and draws the counter when `--fps` asked for it.
    ///
    /// The counter keeps asking for repaints, so the number is the rate the
    /// backend can actually draw at rather than the rate something happened
    /// to ask for a frame.
    fn tick_fps(&mut self, ctx: &egui::Context) {
        self.fps_frames += 1;
        let now = Instant::now();
        let elapsed = now.duration_since(self.fps_last);
        if elapsed >= Duration::from_secs(1) {
            self.fps = self.fps_frames as f32 / elapsed.as_secs_f32();
            self.fps_frames = 0;
            self.fps_last = now;
            if self.show_fps {
                eprintln!("blue-dove: {:.1} fps", self.fps);
            }
        }
        if !self.show_fps {
            return;
        }
        egui::Area::new(egui::Id::unique("fps"))
            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-8.0, 8.0))
            .show(ctx, |ui| {
                egui::Frame::NONE
                    .fill(egui::Color32::from_black_alpha(160))
                    .inner_margin(egui::Margin::symmetric(6, 2))
                    .show(ui, |ui| {
                        ui.label(
                            egui::RichText::new(format!("{:.1} fps", self.fps))
                                .monospace()
                                .color(egui::Color32::WHITE),
                        );
                    });
            });
        ctx.request_repaint();
    }

    /// Closes the window after a few frames when running a smoke test.
    fn tick_smoke(&mut self, ctx: &egui::Context) {
        if !self.smoke {
            return;
        }
        self.frames += 1;
        if self.frames >= SMOKE_FRAMES {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else {
            ctx.request_repaint();
        }
    }
}

/// Whether `key` was pressed with exactly `modifiers`, consuming the press.
///
/// The modifiers are taken from the event itself rather than from
/// `input.modifiers`, which is the state when the frame runs: a quick press
/// has let go of control by then, so the chord would arrive as a bare letter
/// and be missed. `matches_exact` keeps a shifted key, such as the question
/// mark on the slash key, from matching the plain one.
fn chord(ctx: &egui::Context, modifiers: egui::Modifiers, key: egui::Key) -> bool {
    ctx.input_mut(|input| {
        let mut hit = false;
        input.events.retain(|event| {
            let matched = matches!(
                event,
                egui::Event::Key {
                    key: pressed,
                    pressed: true,
                    modifiers: event_modifiers,
                    ..
                } if *pressed == key && event_modifiers.matches_exact(modifiers)
            );
            hit |= matched;
            !matched
        });
        hit
    })
}

/// Whether `text` holds `needle`, which is already in lower case.
///
/// Case is folded on both sides, so a search for `kero` finds `Kero Kero`.
fn matches(needle: &str, text: &str) -> bool {
    text.to_lowercase().contains(needle)
}

/// Whether the window is driving a Wayland compositor.
///
/// This is the rule winit picks its backend by, and the app forces neither:
/// Wayland when `WAYLAND_DISPLAY` or `WAYLAND_SOCKET` names a display. It is
/// the only signal a window leaves, since winit reports both its visibility
/// and its minimized state as unknown there.
fn on_wayland() -> bool {
    wayland(
        std::env::var_os("WAYLAND_DISPLAY").as_deref(),
        std::env::var_os("WAYLAND_SOCKET").as_deref(),
    )
}

/// Whether one of the two variables winit reads names a display.
///
/// An empty value is treated as unset, which is what winit does with it.
fn wayland(display: Option<&std::ffi::OsStr>, socket: Option<&std::ffi::OsStr>) -> bool {
    [display, socket]
        .into_iter()
        .flatten()
        .any(|value| !value.is_empty())
}

/// Whether pressing back restarts the playing track rather than moving to
/// the one before it.
///
/// A track that has been playing for longer than [`RESTART_AFTER`] is
/// restarted: the press goes back to its start, and a press made while it is
/// still at its start goes on to the previous track.
fn back_restarts(position: Duration) -> bool {
    position > RESTART_AFTER
}

/// Keeps stored column widths usable, whatever the settings file says.
fn sane_columns(stored: [f32; 6]) -> [f32; 6] {
    let mut columns = stored;
    for column in crate::ui::Column::ALL {
        let width = &mut columns[column.index()];
        if !width.is_finite() {
            return DEFAULT_COLUMNS;
        }
        *width = width.clamp(column.min_width(), MAX_COLUMN);
    }
    columns
}

/// The file the track at `index` came from, for saving it or finding it again.
fn track_path(library: &Library, index: usize) -> Option<PathBuf> {
    library.tracks.get(index).map(|track| track.path.clone())
}

/// Uploads `cover` as a texture the window draws.
///
/// The picture arrives from the decoder thread in RGBA, which is what egui
/// takes, so nothing is converted here.
fn upload(ctx: &egui::Context, key: &str, cover: &crate::cover::Cover) -> egui::TextureHandle {
    let image = egui::ColorImage::from_rgba_unmultiplied([cover.width, cover.height], &cover.rgba);
    ctx.load_texture(format!("cover-{key}"), image, egui::TextureOptions::LINEAR)
}

/// Where each track of `library` sits, by the file it came from.
fn track_positions(library: &Library) -> HashMap<&Path, usize> {
    library
        .tracks
        .iter()
        .enumerate()
        .map(|(index, track)| (track.path.as_path(), index))
        .collect()
}

impl App {
    /// Plays what was dropped on the window from a file manager.
    ///
    /// A dropped file plays the way one named on the command line does, and a
    /// dropped directory queues its tracks, so a drop can be checked against
    /// what a launch of the same path does. The paths go through
    /// [`App::open_paths`] rather than a path of their own for that reason.
    fn take_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .map(|file| file.path().to_path_buf())
                .collect()
        });
        if dropped.is_empty() {
            return;
        }
        self.open_paths(ctx, &dropped);
        // A drop that lands nothing to play still has to put its report on
        // screen, and nothing else in this frame asks for a repaint.
        ctx.request_repaint();
    }
}

impl App {
    /// Runs the player for one pass, without drawing.
    ///
    /// eframe calls this before each drawing pass, and on its own while the
    /// window is hidden: without it a track that ran out would sit there, and
    /// a launch that asks for the window would go unheard. The software
    /// fallback calls it the same way.
    pub fn run_logic(&mut self, ctx: &egui::Context, window: Option<&Window>) {
        self.poll_workers(ctx);
        // Requests are taken before the remote control is published, so a
        // command from the command line shows up in MPRIS in the same frame
        // rather than a frame later.
        self.poll_requests(ctx, window);
        self.poll_mpris(ctx, window);
        self.take_dropped_files(ctx);
        self.handle_close(ctx, window);
        self.advance(ctx);
    }

    /// Draws the window for one pass.
    pub fn draw(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.handle_keys(&ctx);
        self.update_layout(&ctx);

        let player_height = if self.compact {
            COMPACT_PLAYER_HEIGHT
        } else {
            PLAYER_HEIGHT
        };
        egui::Panel::bottom("player")
            .exact_size(player_height)
            .show(ui, |ui| self.player_bar(ui));

        if let Some(status) = self.status.clone() {
            egui::Panel::bottom("status").show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(status).color(ui.visuals().warn_fg_color));
                    if ui.small_button("Dismiss").clicked() {
                        self.status = None;
                    }
                });
            });
        }

        // No frame margin: the default one would leave an 8 point border of
        // background between the window edge and the menu bar. The panels
        // inside carry their own padding.
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(ui.visuals().panel_fill))
            .show(ui, |ui| {
                if self.preview.is_some() {
                    // The preview: the art, its controls and the waveform,
                    // with no menu bar and no playlist behind it.
                    self.preview_body(ui);
                    return;
                }
                if self.compact {
                    // The popup: the menu bar across the top, then the art
                    // and its controls, and nothing else.
                    self.menu_bar(ui);
                    self.compact_body(ui);
                    return;
                }
                // The menu bar lives at the top of the library column, sized to
                // it, so the playlist gets the width the menu bar would have
                // taken across the whole window.
                egui::Panel::left("library")
                    .exact_size(ALBUM_LIST_WIDTH)
                    .resizable(false)
                    .show(ui, |ui| {
                        self.menu_bar(ui);
                        self.album_list(ui);
                    });
                self.track_table(ui);
            });

        // Bringing the playing row into view takes another frame, and the
        // repaint that playing asks for can be a quarter second away.
        if self.view.reveal {
            ctx.request_repaint();
        }

        self.abx_window(&ctx);
        self.directories_window(&ctx);

        self.tick_fps(&ctx);
        self.tick_smoke(&ctx);
    }
}

impl eframe::App for App {
    /// The colour the renderer clears to.
    ///
    /// A translucent window clears to nothing, so the panels' own alpha is
    /// what the compositor blends with the desktop behind the window.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        if self.transparency {
            [0.0, 0.0, 0.0, 0.0]
        } else {
            egui::Color32::from_rgb(0x12, 0x12, 0x18).to_normalized_gamma_f32()
        }
    }

    /// Writes the settings one last time as the window closes.
    ///
    /// Playlists are saved as they change, so this is for what changes
    /// without a save of its own: the selected track, and the volume if the
    /// slider was never let go.
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.save_playlists();
    }

    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.run_logic(ctx, frame.winit_window().map(|window| &**window));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.draw(ui);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::abx::TRIALS;
    use crate::library::Track;

    pub(crate) fn track(album: &str, number: u32, title: &str) -> Track {
        Track {
            path: PathBuf::from(format!("/music/{album}/{title}.flac")),
            title: title.to_string(),
            artist: "Someone".to_string(),
            album: album.to_string(),
            album_artist: "Someone".to_string(),
            year: Some(2007),
            disc: None,
            number: Some(number),
            duration: Duration::from_secs(120),
            cover: None,
            directory: PathBuf::from(album),
            tagged_album: true,
        }
    }

    pub(crate) fn app_with(albums: &[(&str, &[&str])]) -> App {
        let mut tracks = Vec::new();
        for (album, titles) in albums {
            for (index, title) in titles.iter().enumerate() {
                tracks.push(track(album, index as u32 + 1, title));
            }
        }
        let mut app = App::new(&egui::Context::default(), true, Vec::new());
        app.library = Library::from_tracks(tracks);
        app.open_album(0);
        app
    }

    /// An app holding one album, with Random Order on.
    pub(crate) fn random_app(titles: &[&str]) -> App {
        let mut app = app_with(&[("One", titles)]);
        app.set_random(true);
        app
    }

    #[test]
    fn the_table_shows_the_tracks_of_the_open_album() {
        let app = app_with(&[("One", &["A", "B"]), ("Two", &["C"])]);
        assert_eq!(app.visible_tracks().len(), 2);
    }

    #[test]
    fn the_search_box_filters_titles_and_artists() {
        let mut app = app_with(&[("One", &["Opening", "Finale"])]);
        app.view.filter = "open".to_string();
        let shown = app.visible_tracks();
        assert_eq!(shown.len(), 1);
        assert_eq!(app.library.tracks[shown[0]].title, "Opening");

        app.view.filter = "someone".to_string();
        assert_eq!(app.visible_tracks().len(), 2);

        app.view.filter = "nothing".to_string();
        assert!(app.visible_tracks().is_empty());
    }

    #[test]
    fn the_search_reads_the_album_as_well_as_the_track() {
        let mut app = app_with(&[("One", &["A"]), ("Two", &["B"])]);
        app.playlists.current_mut().add(vec![1]);
        app.view.filter = "two".to_string();
        let shown = app.visible_tracks();
        assert_eq!(shown.len(), 1, "the album the second track belongs to");
        assert_eq!(app.library.tracks[shown[0]].title, "B");
    }

    #[test]
    fn typing_selects_the_first_match_and_scrolls_to_it() {
        let mut app = app_with(&[("One", &["Opening", "Finale", "Opening Again"])]);
        app.open_search();
        assert!(app.view.searching);
        assert!(app.view.search_focus, "the box takes the keyboard");

        app.view.filter = "open".to_string();
        app.refilter();
        assert_eq!(app.view.selected, Some(0));
        assert!(app.view.reveal, "the match is brought into view");
        assert!(app.is_selected(0));
    }

    #[test]
    fn enter_walks_the_matches_and_wraps_around() {
        let mut app = app_with(&[("One", &["Opening", "Finale", "Opening Again"])]);
        app.view.filter = "open".to_string();
        app.refilter();
        assert_eq!(app.view.selected, Some(0));

        app.step_match(true);
        assert_eq!(
            app.view.selected,
            Some(2),
            "the next match, not the row after"
        );

        app.step_match(true);
        assert_eq!(app.view.selected, Some(0), "the walk wraps around");

        app.step_match(false);
        assert_eq!(app.view.selected, Some(2), "and goes back the same way");
    }

    #[test]
    fn a_search_that_matches_nothing_leaves_the_selection_alone() {
        let mut app = app_with(&[("One", &["Opening"])]);
        app.select_only(0);
        app.view.filter = "nothing".to_string();
        app.refilter();
        assert_eq!(app.view.selected, Some(0));
        app.step_match(true);
        assert_eq!(app.view.selected, Some(0), "there is no match to step to");
    }

    #[test]
    fn escape_closes_the_search_and_shows_the_whole_playlist() {
        let mut app = app_with(&[("One", &["Opening", "Finale"])]);
        app.open_search();
        app.view.filter = "open".to_string();
        app.refilter();
        assert_eq!(app.visible_tracks().len(), 1);

        app.close_search();
        assert!(!app.view.searching);
        assert!(app.view.filter.is_empty());
        assert_eq!(app.visible_tracks().len(), 2);
    }

    #[test]
    fn opening_an_album_clears_a_search_left_over_from_the_last_one() {
        let mut app = app_with(&[("One", &["Opening"]), ("Two", &["Finale"])]);
        app.open_search();
        app.view.filter = "finale".to_string();
        app.refilter();

        app.open_album(0);
        assert!(!app.view.searching);
        assert!(app.view.filter.is_empty());
        assert_eq!(app.visible_tracks().len(), 1, "the album is shown whole");
    }

    #[test]
    fn the_arrows_walk_the_rows_and_stop_at_the_ends() {
        let mut app = app_with(&[("One", &["A", "B", "C"])]);
        app.select_only(0);

        app.step_selection(true);
        assert_eq!(app.view.selected, Some(1));
        assert!(app.view.reveal, "the row is brought into view");

        app.step_selection(true);
        app.step_selection(true);
        assert_eq!(app.view.selected, Some(2), "the walk stops at the last row");

        app.step_selection(false);
        assert_eq!(app.view.selected, Some(1));

        app.step_selection(false);
        app.step_selection(false);
        assert_eq!(app.view.selected, Some(0), "and at the first one");
    }

    #[test]
    fn the_arrows_walk_only_what_the_search_leaves() {
        let mut app = app_with(&[("One", &["Opening", "Finale", "Opening Again"])]);
        app.view.filter = "open".to_string();
        app.refilter();
        assert_eq!(app.view.selected, Some(0));

        app.step_selection(true);
        assert_eq!(
            app.view.selected,
            Some(2),
            "Finale is not shown, so it is skipped"
        );
    }

    #[test]
    fn shift_jumps_to_the_first_or_the_last_row() {
        let mut app = app_with(&[("One", &["A", "B", "C"])]);
        app.select_only(1);

        app.select_edge(true);
        assert_eq!(app.view.selected, Some(2));
        assert!(app.view.reveal);

        app.select_edge(false);
        assert_eq!(app.view.selected, Some(0));
    }

    #[test]
    fn enter_plays_the_selected_track() {
        let mut app = app_with(&[("One", &["A", "B"])]);
        app.queue = Some(vec![1]);
        app.select_only(1);
        app.play_selection();
        assert_eq!(app.queue, None, "the playlist is walked again, not a queue");

        // With nothing selected there is nothing to play, and the queue a
        // selection started stays as it was.
        app.clear_selection();
        app.queue = Some(vec![0]);
        app.play_selection();
        assert_eq!(app.queue, Some(vec![0]));
    }

    #[test]
    fn a_track_knows_its_album() {
        let app = app_with(&[("One", &["A"]), ("Two", &["B", "C"])]);
        assert_eq!(app.album_of(0), Some(0));
        assert_eq!(app.album_of(1), Some(1));
        assert_eq!(app.album_of(2), Some(1));
        assert_eq!(app.album_of(9), None);
    }

    #[test]
    fn opening_an_album_replaces_the_playlist() {
        let mut app = app_with(&[("One", &["A", "B"]), ("Two", &["C"])]);
        app.open_album(0);
        assert_eq!(app.playlists.current().tracks, [0, 1]);
        app.open_album(1);
        assert_eq!(app.playlists.current().tracks, [2]);
        assert_eq!(app.view.selected, Some(2));
    }

    #[test]
    fn adding_a_directory_queues_the_loose_files_beside_its_albums() {
        // A directory can hold an album and some files of their own, or
        // nothing but files; a double-click queues both.
        let mut album = track("One", 1, "A");
        album.directory = PathBuf::from("Mixed");
        let mut second = track("One", 2, "B");
        second.directory = PathBuf::from("Mixed");
        let mut loose = track("Loose", 1, "Stray");
        loose.directory = PathBuf::from("Mixed");
        loose.tagged_album = false;
        let mut other = track("Loose", 1, "Another");
        other.directory = PathBuf::from("Mixed");
        other.tagged_album = false;

        let mut app = App::new(&egui::Context::default(), true, Vec::new());
        app.library = Library::from_tracks(vec![album, second, loose, other]);
        app.playlists.current_mut().clear();

        let mixed = app
            .library
            .directories()
            .iter()
            .find(|directory| directory.name == "Mixed")
            .cloned()
            .expect("the directory");
        assert_eq!(mixed.loose.len(), 2);
        app.add_directory_at(&mixed.path);

        let queued = app.playlists.current().tracks.clone();
        assert_eq!(
            queued,
            [0, 1, 2, 3],
            "the album in playing order, then the loose files"
        );
    }

    #[test]
    fn adding_a_directory_of_only_loose_files_queues_them() {
        let mut loose = track("Loose", 1, "Stray");
        loose.directory = PathBuf::from("Singles");
        loose.tagged_album = false;

        let mut app = App::new(&egui::Context::default(), true, Vec::new());
        app.library = Library::from_tracks(vec![loose]);
        app.playlists.current_mut().clear();

        let singles = app.library.directories()[0].clone();
        app.add_directory_at(&singles.path);
        assert_eq!(app.playlists.current().tracks, [0]);
    }

    #[test]
    fn a_file_from_the_command_line_replaces_the_default_tab() {
        let mut app = app_with(&[("One", &["A", "B"]), ("Two", &["C"])]);
        // The playlist holds album One; naming one of album Two's files
        // should replace it rather than add to it.
        let wanted = PathBuf::from("/music/Two/C.flac");
        app.open_paths(&egui::Context::default(), std::slice::from_ref(&wanted));

        let tab = app.playlists.current();
        assert_eq!(tab.name, "Default", "the file plays in the Default tab");
        assert_eq!(tab.tracks, [2], "and replaces what the tab held");
        // Whether the track then starts is the player's business: it needs an
        // output device and the file itself, neither of which a test has.
    }

    #[test]
    fn one_file_and_nothing_else_is_a_preview_candidate() {
        let directory =
            std::env::temp_dir().join(format!("blue-dove-preview-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("the directory is made");
        let file = directory.join("song.flac");
        std::fs::write(&file, b"not really audio").expect("the file is written");
        let other = directory.join("notes.txt");
        std::fs::write(&other, b"text").expect("the file is written");

        assert_eq!(
            App::preview_candidate(std::slice::from_ref(&file)),
            Some(&file)
        );
        assert_eq!(
            App::preview_candidate(&[file.clone(), other.clone()]),
            None,
            "two paths are a session, not a preview"
        );
        assert_eq!(
            App::preview_candidate(std::slice::from_ref(&directory)),
            None,
            "a directory queues its tracks instead"
        );
        assert_eq!(
            App::preview_candidate(std::slice::from_ref(&other)),
            None,
            "a file no decoder handles is not a track"
        );
        assert_eq!(App::preview_candidate(&[]), None);

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn the_preview_leaves_the_playlist_alone() {
        let mut app = app_with(&[("One", &["A", "B"])]);
        let queued = app.playlists.current().tracks.clone();
        let path = app.library.tracks[0].path.clone();
        app.open_preview(&egui::Context::default(), &path);

        assert!(app.preview.is_some(), "the window is showing the preview");
        assert_eq!(
            app.playlists.current().tracks,
            queued,
            "the playlist is untouched"
        );
    }

    #[test]
    fn mpris_describes_the_previewed_file() {
        // The preview reads its file on its own and is in no playlist, so what
        // a client shows has to come from that file: a title, an album and an
        // artist taken from the library would describe a track that is not
        // playing, and the player would read as stopped while it plays.
        let mut app = app_with(&[("One", &["A", "B"])]);
        let path = app.library.tracks[0].path.clone();
        app.open_preview(&egui::Context::default(), &path);
        assert!(app.playing.is_none(), "the preview leaves the list alone");

        let state = app.mpris_state();
        assert_eq!(state.title, "A");
        assert_eq!(state.album, "One");
        assert_eq!(
            state.artists,
            ["Unknown Artist"],
            "the artist is the file's own, not the library row's"
        );
        assert_ne!(
            state.playback,
            Playback::Stopped,
            "a file the window is showing is not stopped"
        );
        assert_eq!(state.track, PREVIEW_TRACK, "the preview is no playlist row");
    }

    #[test]
    fn mpris_names_the_previews_art() {
        // A media widget draws the art from the URI in the metadata, so the
        // picture of a previewed file has to be written out like any other.
        let mut app = app_with(&[("One", &["A"])]);
        let path = app.library.tracks[0].path.clone();
        app.open_preview(&egui::Context::default(), &path);

        let cover = crate::cover::Cover {
            width: 2,
            height: 2,
            rgba: vec![1, 2, 3, 255, 4, 5, 6, 255, 7, 8, 9, 255, 10, 11, 12, 255],
        };
        let bytes = crate::cover::to_png(&cover).expect("the picture encodes");
        let key = crate::cache::store_cover(&bytes).expect("the picture is stored");
        app.preview.as_mut().expect("the preview is showing").cover = Some(key);

        let state = app.mpris_state();
        assert!(state.art.is_some(), "the previewed file's art is named");
    }

    #[test]
    fn the_preview_offers_no_next_or_previous() {
        // The file a preview plays is in no playlist, so a client that was
        // offered Next would step into whatever the playlist happens to hold
        // and leave the file behind.
        let mut app = app_with(&[("One", &["A", "B", "C"])]);
        let path = app.library.tracks[0].path.clone();
        app.open_preview(&egui::Context::default(), &path);

        let state = app.mpris_state();
        assert!(!state.can_go_next, "there is no track after the file");
        assert!(!state.can_go_previous, "and none before it");

        // A playlist with rows either side of the playing one offers both.
        let mut playing = app_with(&[("One", &["A", "B", "C"])]);
        playing.playing = Some(0);
        let state = playing.mpris_state();
        assert!(state.can_go_next && state.can_go_previous);
    }

    #[test]
    fn the_preview_answers_play_and_pause_only() {
        let mut app = app_with(&[("One", &["A", "B"])]);
        let path = app.library.tracks[0].path.clone();
        app.open_preview(&egui::Context::default(), &path);

        assert!(app.remote_command_applies(Command::Play));
        assert!(app.remote_command_applies(Command::Pause));
        assert!(app.remote_command_applies(Command::PlayPause));
        assert!(app.remote_command_applies(Command::Seek(1_000_000)));
        assert!(!app.remote_command_applies(Command::Next));
        assert!(!app.remote_command_applies(Command::Previous));
        assert!(!app.remote_command_applies(Command::Stop));
    }

    #[test]
    fn the_preview_does_not_start_the_library_behind_it() {
        // The preview shows one file, with no playlist behind it, and the
        // keyboard transport still reaches step() and play_selection().
        // Walking the library there plays a track the window does not show,
        // and MPRIS then reports the previewed file while the library plays.
        //
        // A play of these tracks cannot succeed, since their files are not
        // there, so the status line is what says whether one was even tried.
        let mut app = app_with(&[("One", &["A", "B"]), ("Two", &["C"])]);
        app.preview = Some(app.library.tracks[0].clone());
        app.view.selected = Some(1);
        app.status = None;

        app.step(true);
        assert_eq!(app.playing, None, "the preview is not stepped away from");
        assert!(app.status.is_none(), "and no track was started");

        app.play_selection();
        assert_eq!(app.playing, None, "the selected row is not played");
        assert!(app.status.is_none(), "and no track was started");
    }

    #[test]
    fn a_playlist_play_answers_a_step_and_a_stop() {
        let app = app_with(&[("One", &["A", "B"])]);
        for command in [
            Command::Next,
            Command::Previous,
            Command::Stop,
            Command::Play,
        ] {
            assert!(
                app.remote_command_applies(command),
                "{command:?} applies to the playlist"
            );
        }
    }

    #[test]
    fn leaving_the_preview_puts_its_track_in_the_default_tab() {
        let mut app = app_with(&[("One", &["A", "B"]), ("Two", &["C"])]);
        app.playlists.current_mut().set([0]);
        let path = app.library.tracks[1].path.clone();
        app.open_preview(&egui::Context::default(), &path);

        app.leave_preview(&egui::Context::default());
        assert!(
            app.preview.is_none(),
            "the window is back to its normal layout"
        );
        assert_eq!(
            app.playlists.current().tracks,
            [1],
            "the previewed track is kept, in the tab a file goes to"
        );
    }

    #[test]
    fn turning_the_preview_off_leaves_it() {
        let mut app = app_with(&[("One", &["A", "B"])]);
        app.set_preview_enabled(&egui::Context::default(), false);

        assert!(!app.preview_enabled);
        assert!(
            app.preview.is_none(),
            "the window cannot show the preview while the setting is off"
        );
    }

    #[test]
    fn a_launch_naming_one_file_previews_it() {
        // A file manager sends one file to a launch, and that is the preview.
        // The file is read on its own, so it need not be in the library: the
        // library holds the music directory, and a drop can come from
        // anywhere.
        let directory = std::env::temp_dir().join(format!("blue-dove-drop-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("the directory is made");
        let file = directory.join("dropped.flac");
        std::fs::write(&file, b"not really audio").expect("the file is written");

        let mut app = app_with(&[("One", &["A"])]);
        let queued = app.playlists.current().tracks.clone();
        app.pending_paths = Some(vec![file.clone()]);
        app.open_pending_paths(&egui::Context::default());

        // The file cannot be decoded, so nothing plays; what the test pins is
        // that the launch took the preview path rather than the playlist one.
        assert_eq!(
            app.playlists.current().tracks,
            queued,
            "the playlist is left as it was"
        );
        assert!(
            app.status.is_none()
                || app
                    .status
                    .as_deref()
                    .is_some_and(|s| !s.contains("music directory")),
            "the file is not reported as outside the library"
        );

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_file_opened_while_the_player_runs_still_previews() {
        // A file manager hands the file to the window that is already open
        // when one is, and that arrives as a request rather than a launch.
        // It has to preview like a fresh launch does, or opening a second
        // file puts the full window on screen instead.
        let directory = std::env::temp_dir().join(format!("blue-dove-open-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("the directory is made");
        let file = directory.join("second.flac");
        std::fs::write(&file, b"not really audio").expect("the file is written");

        let mut app = app_with(&[("One", &["A"])]);
        let queued = app.playlists.current().tracks.clone();
        app.take_request(
            &egui::Context::default(),
            crate::instance::Request::Open(vec![file.clone()]),
        );

        assert_eq!(
            app.preview.as_ref().map(|track| track.path.clone()),
            Some(file.clone()),
            "the second file opens in the preview"
        );
        assert_eq!(
            app.playlists.current().tracks,
            queued,
            "and the playlist is still untouched"
        );

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_directory_opened_while_previewing_leaves_the_preview() {
        // The preview has three buttons and no playlist, so content that is a
        // session rather than one track has to give it back to the full
        // window.
        let mut app = app_with(&[("One", &["A", "B"])]);
        let path = app.library.tracks[0].path.clone();
        app.open_preview(&egui::Context::default(), &path);
        assert!(app.preview.is_some(), "the preview is showing");

        app.take_request(
            &egui::Context::default(),
            crate::instance::Request::Open(vec![PathBuf::from("/music/One")]),
        );

        assert!(
            app.preview.is_none(),
            "the directory takes the full window back"
        );
    }

    #[test]
    fn the_preview_does_not_wait_for_the_scan() {
        // A file manager names one file, and the library it belongs to may be
        // large; the preview has to open at once rather than after the scan,
        // or the full window sits on screen for as long as it takes.
        let directory = std::env::temp_dir().join(format!("blue-dove-now-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("the directory is made");
        let file = directory.join("now.flac");
        std::fs::write(&file, b"not really audio").expect("the file is written");

        let mut app = app_with(&[("One", &["A"])]);
        // A scan in flight is what the app holds while a library is read.
        let root = std::env::temp_dir();
        app.scan = Some((
            crate::library::spawn_scan(crate::library::one(&root, true)),
            0,
        ));
        assert!(app.scan.is_some(), "the scan is running");

        app.pending_paths = Some(vec![file.clone()]);
        app.open_pending_paths(&egui::Context::default());

        assert_eq!(
            app.preview.as_ref().map(|track| track.path.clone()),
            Some(file.clone()),
            "the preview opens while the scan is still running"
        );
        assert!(
            app.pending_paths.is_none(),
            "and the path is taken, not left to be applied twice"
        );

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn the_preview_is_a_fixed_size_and_the_rest_is_not() {
        // The preview is pinned: its size is the floor and the ceiling at
        // once, so neither a drag on a border nor a window manager honouring
        // size hints can change it. Leaving the preview lifts the pin, since
        // the regular and compact layouts are resizable.
        let mut app = app_with(&[("One", &["A"])]);
        let path = app.library.tracks[0].path.clone();

        app.open_preview(&egui::Context::default(), &path);
        assert!(app.preview.is_some(), "the preview is showing");
        assert!(
            !app.preview_art,
            "this track carries no art, so its window is the short one"
        );

        app.leave_preview(&egui::Context::default());
        assert!(app.preview.is_none(), "the preview is gone");
    }

    #[test]
    fn a_preview_without_art_asks_for_a_shorter_window_than_one_with_it() {
        // Without art there is no square to draw, so the window is only as
        // tall as the name line, the controls and the waveform. The two sizes
        // must differ, or a file with no cover would sit in a window of empty
        // background where the art would have been.
        const { assert!(PREVIEW_NO_ART_SIZE[1] < COMPACT_SIZE[1]) };
        const { assert!(PREVIEW_NO_ART_SIZE[0] <= COMPACT_SIZE[0]) };
        const { assert!(PREVIEW_NO_ART_SIZE[0] == COMPACT_SIZE[0]) };
    }

    #[test]
    fn a_previewed_file_outside_the_library_is_read_on_its_own() {
        // The case a file manager sends: a path the library knows nothing
        // about. It still has to reach the preview.
        let directory = std::env::temp_dir().join(format!("blue-dove-out-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("the directory is made");
        let file = directory.join("elsewhere.flac");
        std::fs::write(&file, b"not really audio").expect("the file is written");

        let mut app = app_with(&[("One", &["A"])]);
        app.open_preview(&egui::Context::default(), &file);
        assert_eq!(
            app.preview.as_ref().map(|track| track.path.clone()),
            Some(file.clone()),
            "the preview holds the file wherever it is"
        );

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_directory_from_the_command_line_queues_what_it_holds() {
        let mut app = app_with(&[("One", &["A", "B"]), ("Two", &["C"])]);
        app.playlists.current_mut().set([0]);
        app.open_paths(&egui::Context::default(), &[PathBuf::from("/music/Two")]);

        assert_eq!(
            app.playlists.current().tracks,
            [0, 2],
            "the directory's tracks join the playlist"
        );
    }

    #[test]
    fn a_path_the_library_does_not_hold_is_reported() {
        let mut app = app_with(&[("One", &["A"])]);
        app.open_paths(
            &egui::Context::default(),
            &[PathBuf::from("/elsewhere/not-here.flac")],
        );

        assert_eq!(
            app.status.as_deref(),
            Some("/elsewhere/not-here.flac is not in the music directory")
        );
    }

    #[test]
    fn a_file_plays_in_a_tab_created_for_it_when_none_is_named_default() {
        let mut app = app_with(&[("One", &["A"])]);
        app.playlists.current_mut().name = "Evening".to_string();

        app.open_paths(
            &egui::Context::default(),
            &[PathBuf::from("/music/One/A.flac")],
        );

        assert_eq!(app.playlists.items.len(), 2, "a tab is opened for it");
        assert_eq!(app.playlists.current().name, "Default");
        assert_eq!(app.playlists.current().tracks, [0]);
        assert_eq!(
            app.playlists.items[0].name, "Evening",
            "the other tab stays"
        );
    }

    #[test]
    fn adding_a_directory_queues_its_albums() {
        let mut app = app_with(&[("One", &["A", "B"]), ("Two", &["C"])]);
        let directory = app
            .library
            .directories()
            .first()
            .cloned()
            .expect("a directory");
        app.playlists.current_mut().clear();
        app.add_directory_at(&directory.path);
        let queued = app.playlists.current().tracks.clone();
        assert_eq!(
            queued,
            [0, 1],
            "the directory's own album, in playing order"
        );
    }

    #[test]
    fn adding_a_directory_queues_albums_from_its_subdirectories() {
        let mut tracks = Vec::new();
        for (album, titles) in [("Disc One", ["A"]), ("Disc Two", ["B"])] {
            for title in titles {
                let mut track = track(album, 1, title);
                track.directory = PathBuf::from("Box").join(album);
                tracks.push(track);
            }
        }
        let mut app = App::new(&egui::Context::default(), true, Vec::new());
        app.library = Library::from_tracks(tracks);
        app.playlists.current_mut().clear();

        let boxed = app
            .library
            .directories()
            .iter()
            .find(|directory| directory.name == "Box")
            .cloned()
            .expect("the shared directory");
        assert_eq!(boxed.children.len(), 2);
        app.add_directory_at(&boxed.path);
        assert_eq!(app.playlists.current().tracks, [0, 1]);
    }

    #[test]
    fn adding_the_library_queues_every_track_in_list_order() {
        // The list sorts directories by name and albums by title, so the
        // library is queued in that order, whatever order the scan found the
        // files in. A loose file belongs to no album, so it follows the
        // albums of the directory it sits in.
        let mut tracks = Vec::new();
        for (album, title) in [("Zeta", "B"), ("Alpha", "A"), ("Middle", "C")] {
            tracks.push(track(album, 1, title));
        }
        let mut loose = track("Alpha", 2, "Loose");
        loose.tagged_album = false;
        tracks.push(loose);

        let mut app = App::new(&egui::Context::default(), true, Vec::new());
        app.library = Library::from_tracks(tracks);
        app.playlists.current_mut().clear();
        app.add_all();
        assert_eq!(
            app.playlists.current().tracks,
            [1, 3, 2, 0],
            "Alpha's album, its loose file, Middle, Zeta"
        );
    }

    #[test]
    fn adding_the_library_queues_files_sitting_in_the_music_directory() {
        // Files directly in the music directory belong to no directory of
        // their own, and they used to be dropped from the tree: Add All
        // queued the subdirectories and nothing else.
        let mut at_root = track("Root", 1, "A");
        at_root.directory = PathBuf::new();
        let mut stray = track("Root", 2, "B");
        stray.directory = PathBuf::new();
        stray.tagged_album = false;
        let mut below = track("Below", 1, "C");
        below.directory = PathBuf::from("Below");

        let mut app = App::new(&egui::Context::default(), true, Vec::new());
        app.library = Library::from_tracks(vec![at_root, stray, below]);
        app.playlists.current_mut().clear();
        app.add_all();
        assert_eq!(
            app.playlists.current().tracks,
            [0, 1, 2],
            "the album and the loose file in the music directory, then Below"
        );
    }

    #[test]
    fn two_disc_directories_of_the_same_name_stay_apart() {
        // A set of releases keeps a `Disc 1` under each of them, so a
        // directory that carried only its own name would have both answer to
        // the same path and queue the wrong release.
        let mut first = track("One", 1, "A");
        first.directory = PathBuf::from("First/Disc 1");
        let mut second = track("Two", 1, "B");
        second.directory = PathBuf::from("Second/Disc 1");

        let mut app = App::new(&egui::Context::default(), true, Vec::new());
        app.library = Library::from_tracks(vec![first, second]);
        app.playlists.current_mut().clear();

        let disc = app
            .library
            .directories()
            .iter()
            .find(|directory| directory.name == "Second")
            .and_then(|directory| directory.children.first())
            .cloned()
            .expect("the second release's disc directory");
        assert_eq!(disc.path, PathBuf::from("Second/Disc 1"));
        app.add_directory_at(&disc.path);
        assert_eq!(
            app.playlists.current().tracks,
            [1],
            "only the release the directory belongs to"
        );
    }

    #[test]
    fn a_directory_can_be_folded_away() {
        let mut app = app_with(&[("One", &["A"])]);
        let directory = app
            .library
            .directories()
            .first()
            .cloned()
            .expect("a directory");
        assert!(!app.is_collapsed(&directory));
        app.toggle_collapsed_at(&directory.path);
        assert!(app.is_collapsed(&directory));
        app.toggle_collapsed_at(&directory.path);
        assert!(!app.is_collapsed(&directory));
    }

    #[test]
    fn renaming_a_playlist_keeps_the_tracks() {
        let mut app = app_with(&[("One", &["A"])]);
        app.rename_playlist("  Evening  ");
        assert_eq!(app.playlists.current().name, "Evening");
        assert_eq!(app.playlists.current().tracks.len(), 1);
        app.rename_playlist("   ");
        assert_eq!(
            app.playlists.current().name,
            "Evening",
            "a blank name is ignored"
        );
    }

    #[test]
    fn random_playback_draws_from_the_active_playlist() {
        let mut app = random_app(&["A", "B", "C"]);
        let current = app.playlists.current().tracks[0];
        let order = app.shuffle_order();
        assert_eq!(order.len(), 3);
        for track in &order {
            assert!(
                app.playlists.current().tracks.contains(track),
                "the shuffle holds only the playlist's tracks"
            );
        }
        let next = app.shuffled_neighbour(current, true).expect("a track");
        assert!(order.contains(&next));
    }

    #[test]
    fn a_playlist_is_saved_by_path() {
        let mut app = app_with(&[("One", &["A", "B"]), ("Two", &["C"])]);
        app.playlists.add();
        app.rename_playlist("Evening");
        app.playlists.current_mut().add([2]);
        app.save_playlists();

        let saved = app.config.playlists.as_ref().expect("the tabs are saved");
        assert_eq!(saved.len(), 2);
        assert_eq!(saved[0].name, "Default");
        assert_eq!(saved[1].name, "Evening");
        assert_eq!(
            saved[1].tracks,
            [PathBuf::from("/music/Two/C.flac")],
            "tracks are stored as the files they came from"
        );
        assert_eq!(app.config.active_playlist, Some(1));
    }

    #[test]
    fn saved_playlists_come_back_when_the_directory_is_scanned() {
        let mut app = app_with(&[("One", &["A", "B"]), ("Two", &["C"])]);
        app.playlists.add();
        app.rename_playlist("Evening");
        app.playlists.current_mut().add([2]);
        app.view.selected = Some(2);
        app.save_playlists();
        let saved = app.config.playlists.clone().expect("the tabs are saved");

        let mut restarted = app_with(&[("One", &["A", "B"]), ("Two", &["C"])]);
        restarted.config.playlists = Some(saved);
        restarted.config.active_playlist = Some(1);
        restarted.config.selected_track = Some(PathBuf::from("/music/Two/C.flac"));
        restarted.restore_playlists();

        assert_eq!(restarted.playlists.items.len(), 2);
        assert_eq!(restarted.playlists.current().name, "Evening");
        assert_eq!(restarted.playlists.current().tracks, [2]);
        assert_eq!(restarted.playlists.items[0].tracks, [0, 1]);
        assert_eq!(restarted.view.selected, Some(2));
    }

    #[test]
    fn a_playlist_whose_files_are_gone_keeps_its_tab() {
        let mut app = app_with(&[("One", &["A"])]);
        app.config.playlists = Some(vec![
            SavedPlaylist {
                name: "Gone".to_string(),
                tracks: vec![PathBuf::from("/elsewhere/lost.flac")],
            },
            SavedPlaylist {
                name: "Here".to_string(),
                tracks: vec![PathBuf::from("/music/One/A.flac")],
            },
        ]);
        app.config.active_playlist = Some(1);
        app.restore_playlists();

        assert_eq!(app.playlists.items.len(), 2, "both tabs are back");
        assert_eq!(app.playlists.items[0].name, "Gone");
        assert!(
            app.playlists.items[0].tracks.is_empty(),
            "its files are gone"
        );
        assert_eq!(app.playlists.current().name, "Here");
        assert_eq!(app.playlists.current().tracks, [0]);
    }

    #[test]
    fn clicking_selects_one_row_and_ctrl_click_adds_another() {
        let mut app = app_with(&[("One", &["A", "B", "C"])]);
        app.select_only(0);
        assert_eq!(app.view.selection, [0].into_iter().collect());
        app.toggle_selection(2);
        assert_eq!(app.view.selection, [0, 2].into_iter().collect());
        app.toggle_selection(0);
        assert_eq!(app.view.selection, [2].into_iter().collect(), "toggled off");
        assert_eq!(app.view.selected, Some(2), "the anchor moves with it");
    }

    #[test]
    fn shift_click_selects_the_range_from_the_anchor() {
        let mut app = app_with(&[("One", &["A", "B", "C", "D"])]);
        app.select_only(1);
        app.extend_selection(3);
        assert_eq!(app.view.selection, [1, 2, 3].into_iter().collect());
        app.extend_selection(0);
        assert_eq!(
            app.view.selection,
            [0, 1].into_iter().collect(),
            "backwards"
        );
    }

    #[test]
    fn select_all_takes_the_whole_playlist() {
        let mut app = app_with(&[("One", &["A", "B"]), ("Two", &["C"])]);
        app.select_all();
        assert_eq!(app.view.selection, [0, 1].into_iter().collect());

        // A search narrows what is shown, and so what is selected.
        app.view.filter = "b".to_string();
        app.select_all();
        assert_eq!(app.view.selection, [1].into_iter().collect());
    }

    #[test]
    fn removing_takes_the_rows_out_of_the_playlist_only() {
        // The rows leave the playlist; the files stay exactly where they are.
        let directory =
            std::env::temp_dir().join(format!("blue-dove-remove-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("the directory is made");
        let mut paths = Vec::new();
        for name in ["one.flac", "two.flac", "three.flac"] {
            let path = directory.join(name);
            std::fs::write(&path, b"pretend audio").expect("the file is written");
            paths.push(path);
        }
        let mut tracks: Vec<Track> = paths
            .iter()
            .map(|path| {
                let mut track = track("One", 1, "Song");
                track.path = path.clone();
                track
            })
            .collect();
        let mut app = App::new(&egui::Context::default(), true, Vec::new());
        app.library = Library::from_tracks(std::mem::take(&mut tracks));
        app.open_album(0);
        app.select_all();
        // Two of the three are taken out, the middle one stays.
        app.toggle_selection(1);
        app.remove_selected();

        assert_eq!(app.playlists.current().tracks, [1]);
        for path in &paths {
            assert!(path.is_file(), "{} was left on disk", path.display());
        }
        assert!(app.view.selection.is_empty(), "nothing stays selected");
        std::fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn playing_a_selection_leaves_the_playlist_alone() {
        let mut app = app_with(&[("One", &["A", "B", "C"])]);
        app.select_only(2);
        app.play_selected();

        assert_eq!(
            app.playlists.current().tracks,
            [0, 1, 2],
            "the playlist still holds everything"
        );
        assert_eq!(app.queue, Some(vec![2]), "the queue is the selection");
    }

    #[test]
    fn next_walks_the_queue_a_selection_started() {
        let mut app = app_with(&[("One", &["A", "B", "C", "D"])]);
        app.view.selection = [1, 3].into_iter().collect();
        app.queue = Some(vec![1, 3]);

        // Next from the first queued track lands on the other one, skipping
        // the playlist rows in between.
        assert_eq!(
            crate::playlist::neighbour(app.walked_tracks(), 1, true),
            Some(3)
        );
        assert_eq!(
            crate::playlist::neighbour(app.walked_tracks(), 3, true),
            Some(1)
        );
    }

    #[test]
    fn a_removed_track_leaves_the_queue_too() {
        let mut app = app_with(&[("One", &["A", "B", "C"])]);
        app.queue = Some(vec![0, 1, 2]);
        app.select_only(1);
        app.remove_selected();
        assert_eq!(app.queue, Some(vec![0, 2]));
    }

    #[test]
    fn back_restarts_a_track_that_has_been_playing_a_while() {
        assert!(
            !back_restarts(Duration::ZERO),
            "a fresh track goes back a track"
        );
        assert!(
            !back_restarts(Duration::from_secs(2)),
            "the first few seconds still go back a track"
        );
        assert!(
            !back_restarts(RESTART_AFTER),
            "the threshold itself is not past it"
        );
        assert!(
            back_restarts(RESTART_AFTER + Duration::from_millis(1)),
            "anything past it restarts the track"
        );
        assert!(back_restarts(Duration::from_secs(180)));
    }

    #[test]
    fn random_order_walks_one_shuffle_from_end_to_end() {
        let mut app = random_app(&["A", "B", "C", "D", "E"]);
        let tracks = app.playlists.current().tracks.clone();

        // Follow the order from its first track: every track comes up once
        // before any comes back, and then the order repeats.
        let order = app.shuffle_order();
        let mut heard = Vec::new();
        let mut current = order[0];
        for _ in 0..tracks.len() {
            heard.push(current);
            current = app.shuffled_neighbour(current, true).expect("a next track");
        }
        let mut sorted = heard.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, tracks, "the whole playlist is heard once");
        assert_eq!(current, order[0], "and then the shuffle starts again");
    }

    #[test]
    fn going_back_then_forward_plays_the_same_track_again() {
        let mut app = random_app(&["A", "B", "C", "D", "E"]);
        let first = app.shuffle_order()[0];

        let second = app.shuffled_neighbour(first, true).expect("a next track");
        let back = app
            .shuffled_neighbour(second, false)
            .expect("a previous track");
        assert_eq!(back, first, "previous returns to the track that played");
        let forward = app.shuffled_neighbour(back, true).expect("a next track");
        assert_eq!(
            forward, second,
            "next plays the track that played before, not a fresh draw"
        );
    }

    #[test]
    fn the_shuffle_keeps_its_order_between_steps() {
        let mut app = random_app(&["A", "B", "C", "D"]);
        let order = app.shuffle_order();
        for _ in 0..5 {
            assert_eq!(app.shuffle_order(), order, "the order does not change");
        }
    }

    #[test]
    fn a_track_played_by_hand_continues_in_the_shuffle() {
        let mut app = random_app(&["A", "B", "C", "D"]);
        let order = app.shuffle_order();
        // The second track of the shuffle is picked out of the table.
        let picked = order[1];
        let next = app.shuffled_neighbour(picked, true).expect("a next track");
        assert_eq!(next, order[2], "the shuffle carries on from that track");
    }

    #[test]
    fn a_change_of_playlist_draws_a_new_shuffle() {
        let mut app = app_with(&[("One", &["A", "B"]), ("Two", &["C", "D"])]);
        app.set_random(true);
        assert_eq!(app.shuffle_order().len(), 2);
        assert!(
            app.shuffle.is_some(),
            "the order is drawn when it is needed"
        );

        app.open_album(1);
        assert!(app.shuffle.is_none(), "the old order no longer fits");
        assert_eq!(app.shuffle_order().len(), 2);
    }

    #[test]
    fn a_removed_track_draws_a_new_shuffle() {
        let mut app = random_app(&["A", "B", "C"]);
        app.select_only(1);
        app.remove_selected();

        let order = app.shuffle_order();
        assert_eq!(order.len(), 2, "the shuffle is of what the playlist holds");
        assert!(!order.contains(&1), "the removed track is not walked");
    }

    #[test]
    fn switching_random_off_and_on_draws_a_new_shuffle() {
        let mut app = random_app(&["A", "B", "C"]);
        let first = app.shuffle_order();
        app.set_random(false);
        app.set_random(true);
        assert_eq!(app.shuffle_order().len(), 3);
        assert_ne!(app.shuffle_order(), first, "a new order is drawn");
    }

    #[test]
    fn a_small_window_is_the_popup() {
        assert!(compact_for(egui::vec2(320.0, 400.0)));
        assert!(compact_for(egui::vec2(900.0, 420.0)), "too short");
        assert!(compact_for(egui::vec2(500.0, 800.0)), "too narrow");
        assert!(!compact_for(egui::vec2(1180.0, 760.0)));
        assert!(
            !compact_for(egui::vec2(560.0, 430.0)),
            "the break is exclusive"
        );
    }

    #[test]
    fn the_popup_picks_a_size_and_will_not_go_below_it() {
        // The popup is snapped to what its content needs: with a square the
        // full height, without one the height of the name line, the controls
        // and the waveform. The empty height of a missing square was being
        // taken by the gap above the waveform.
        const { assert!(COMPACT_NO_ART_SIZE[1] < COMPACT_SIZE[1]) };
        const { assert!(COMPACT_NO_ART_SIZE[0] == COMPACT_SIZE[0]) };
        // Both snap sizes land in the popup layout, so neither flickers back
        // to the full one, and each is its own floor: the window may grow
        // from there but not shrink, since the controls and the waveform need
        // the room the layout picked.
        assert!(compact_for(egui::vec2(
            COMPACT_NO_ART_SIZE[0],
            COMPACT_NO_ART_SIZE[1]
        )));
        assert!(compact_for(egui::vec2(COMPACT_SIZE[0], COMPACT_SIZE[1])));
        // The full layout's floor is below the break, which is what lets the
        // window be dragged down into the popup at all.
        const { assert!(COMPACT_MIN_SIZE[0] < COMPACT_BREAK[0]) };
        const { assert!(COMPACT_MIN_SIZE[1] < COMPACT_BREAK[1]) };
    }

    #[test]
    fn the_compact_size_is_inside_the_compact_layout() {
        // The snap has to land where the popup still fits, or the window
        // would flicker between the two layouts.
        assert!(compact_for(egui::vec2(COMPACT_SIZE[0], COMPACT_SIZE[1])));
        // The control strip is laid out at the album column's width, so the
        // popup has to be at least that wide, and tall enough for the art
        // under the menu bar and above the player bar.
        const { assert!(COMPACT_SIZE[0] >= ALBUM_LIST_WIDTH) };
        const { assert!(COMPACT_SIZE[1] > COMPACT_PLAYER_HEIGHT + ALBUM_LIST_WIDTH) };
    }

    #[test]
    fn shrinking_the_window_switches_to_the_popup() {
        let mut app = app_with(&[("One", &["A"])]);
        assert!(!app.compact);
        // The size is what `update_layout` reads, so this drives the same
        // decision the window does.
        app.compact = compact_for(egui::vec2(300.0, 390.0));
        assert!(app.compact);
    }

    #[test]
    fn a_window_is_on_wayland_when_a_display_names_one() {
        // This is the rule winit picks its backend by, and the close that has
        // to minimize rather than hide follows it.
        let none = None;
        assert!(!wayland(none, none), "with neither variable it is X11");
        assert!(
            !wayland(
                Some(std::ffi::OsStr::new("")),
                Some(std::ffi::OsStr::new(""))
            ),
            "an empty value counts as unset, the way winit reads it"
        );
        assert!(
            wayland(Some(std::ffi::OsStr::new("wayland-0")), none),
            "a display is enough"
        );
        assert!(
            wayland(none, Some(std::ffi::OsStr::new("7"))),
            "and so is a socket"
        );
    }

    #[test]
    fn a_picture_size_the_square_grew_past_is_not_kept() {
        // A window widened while the picture decoded asks for a second size
        // before the first lands. Both would be held, a cover of that size
        // being a few megabytes, though nothing draws the smaller one again.
        fn stored(width: usize) -> String {
            let cover = crate::cover::Cover {
                width,
                height: width,
                rgba: [10, 20, 30, 255].repeat(width * width),
            };
            let bytes = crate::cover::to_png(&cover).expect("the picture encodes");
            crate::cache::store_cover(&bytes).expect("the picture is stored")
        }

        let mut app = app_with(&[("One", &["A"])]);
        let key = stored(800);
        let mut track = app.library.tracks[0].clone();
        track.cover = Some(key.clone());
        app.library = Library::from_tracks(vec![track]);
        app.open_album(0);
        app.playing = Some(0);

        let ctx = egui::Context::default();
        let small = crate::art::sized_key(&key, 640);
        let large = crate::art::sized_key(&key, 896);
        // The square is 600 points wide, and the window is widened to 700
        // before the picture of that size lands. Nothing polls in between, so
        // the smaller ask is still outstanding when the larger one is made.
        app.playing_art(&ctx, 600.0);
        app.playing_art(&ctx, 700.0);

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.pictures.decoded(&large).is_none() {
            app.pictures.poll();
            assert!(
                std::time::Instant::now() < deadline,
                "the picture of the size the square is drawn at never arrived"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }

        assert!(
            app.pictures.decoded(&small).is_none(),
            "the size the square grew past is not held"
        );
    }

    #[test]
    fn the_big_square_is_drawn_from_art_of_its_own_size() {
        // The album lists draw a cover the size of a row, which a thumbnail
        // fills. The square above the controls is drawn as wide as the
        // window, so it asks for the picture at that size instead of
        // stretching a thumbnail over it and reading blurry.
        fn stored(width: usize) -> String {
            let cover = crate::cover::Cover {
                width,
                height: width,
                rgba: [10, 20, 30, 255].repeat(width * width),
            };
            let bytes = crate::cover::to_png(&cover).expect("the picture encodes");
            crate::cache::store_cover(&bytes).expect("the picture is stored")
        }

        /// Draws the square until the picture arrives, and answers its texture.
        fn settle(app: &mut App, ctx: &egui::Context, side: f32) -> egui::TextureHandle {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            loop {
                app.pictures.poll();
                // Until the picture of its own size lands, the square answers
                // with the thumbnail, so what says it landed is the texture
                // being wider than one.
                if let Some(texture) = app.playing_art(ctx, side)
                    && texture.size()[0] > crate::cover::THUMBNAIL as usize
                {
                    return texture;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "the picture of its own size never arrived"
                );
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }

        let mut app = app_with(&[("One", &["A"])]);
        let key = stored(800);
        let mut track = app.library.tracks[0].clone();
        track.cover = Some(key);
        app.library = Library::from_tracks(vec![track]);
        app.open_album(0);
        app.playing = Some(0);

        let ctx = egui::Context::default();
        // A square 600 points wide on a screen with no scaling is 600 pixels
        // of art, rounded up to the next size a cover is decoded at.
        let texture = settle(&mut app, &ctx, 600.0);
        assert_eq!(
            texture.size(),
            [640, 640],
            "the picture is decoded at the size the square is drawn"
        );
        let key = app
            .library
            .tracks
            .first()
            .and_then(|track| track.cover.clone())
            .expect("the cover is named");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let thumbnail = loop {
            app.pictures.poll();
            if let Some(texture) = app.cover_texture(&ctx, &key) {
                break texture;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the thumbnail never arrived"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        assert_eq!(
            thumbnail.size(),
            [256, 256],
            "the list keeps its thumbnail, so both sizes are available"
        );
    }

    #[test]
    fn a_stopped_player_names_no_art() {
        // The art file is kept for the track that is playing, so a stop has
        // to drop it: the metadata would otherwise carry the cover of a track
        // that is no longer playing beside an empty title.
        let mut app = app_with(&[("One", &["A"])]);
        app.playing = Some(0);
        app.art = Some(Some(PathBuf::from("/art/one.png")));
        app.playing = None;
        assert_eq!(app.art_path(), None, "nothing plays, so nothing is named");
        assert!(app.art.is_none(), "and the last track's art is forgotten");
    }

    #[test]
    fn a_track_without_a_cover_names_no_art() {
        let mut app = app_with(&[("One", &["A"])]);
        app.playing = Some(0);
        assert_eq!(app.art_path(), None, "the track carries no cover");
        assert_eq!(
            app.art,
            Some(None),
            "and the answer is remembered, so it is not asked for again"
        );
    }

    #[test]
    fn a_rescan_keeps_the_playlists_on_the_same_tracks() {
        let mut app = app_with(&[("One", &["A", "B"]), ("Two", &["C"])]);
        app.playing = Some(0);
        let before: Vec<_> = app
            .playlists
            .current()
            .tracks
            .iter()
            .map(|&index| app.library.tracks[index].path.clone())
            .collect();

        // A rescan can list the files in another order; the playlist has to
        // follow the files, not the old positions.
        let mut tracks = app.library.tracks.clone();
        tracks.reverse();
        app.adopt_library(Library::from_tracks(tracks));

        let after: Vec<_> = app
            .playlists
            .current()
            .tracks
            .iter()
            .map(|&index| app.library.tracks[index].path.clone())
            .collect();
        assert_eq!(after, before);
        assert_eq!(
            app.playing
                .map(|index| app.library.tracks[index].path.clone()),
            before.first().cloned(),
            "the playing track follows its file too"
        );
    }

    #[test]
    fn a_scan_that_finds_nothing_does_not_empty_the_window() {
        // A directory that went away answers an empty library. Taking that in
        // would empty the album list under the user, with every row it drew
        // pointing at a track that is no longer there, which is what an
        // unmounted share looks like from here.
        let mut app = app_with(&[("One", &["A", "B"]), ("Two", &["C"])]);
        app.playing = Some(1);

        app.adopt_library(Library::default());

        assert_eq!(app.library.tracks.len(), 3, "the library is kept");
        assert_eq!(app.view.album, Some(0), "and so is the album it shows");
        assert_eq!(app.playing, Some(1), "and the track that plays");
    }

    #[test]
    fn a_rescan_keeps_the_walking_queue_on_the_same_tracks() {
        // The queue a "play the selection" started is indices into the track
        // list, and a scan replaces that list: a file that arrives above a
        // queued track shifts every index below it, and the next step walks
        // to a track that is not there, which stops playback without a word.
        let mut app = app_with(&[("One", &["A", "B"]), ("Two", &["C"])]);
        app.queue = Some(vec![1, 2]);
        app.shuffle = Some(vec![0, 1, 2]);
        let queued: Vec<PathBuf> = [1, 2]
            .iter()
            .map(|&index| app.library.tracks[index].path.clone())
            .collect();

        // A file that sorts before every track listed arrives.
        let mut tracks = vec![track("One", 1, "0")];
        tracks.extend(app.library.tracks.clone());
        app.adopt_library(Library::from_tracks(tracks));

        let queue = app.queue.clone().expect("the queue is kept");
        let after: Vec<PathBuf> = queue
            .iter()
            .map(|&index| app.library.tracks[index].path.clone())
            .collect();
        assert_eq!(after, queued, "the queue names the same files");
        assert!(
            app.shuffle.is_none(),
            "the shuffled order is drawn again rather than remapped"
        );
    }

    #[test]
    fn a_scan_that_finds_nothing_leaves_the_session_alone() {
        let mut app = app_with(&[("One", &["A"])]);
        app.adopt_library(Library::default());

        assert_eq!(
            app.playlists.current().tracks,
            [0],
            "the playlist is kept for when the directory comes back"
        );
    }

    /// A scan that has already answered, so a frame can take it in.
    fn finished(library: Library) -> Receiver<ScanMessage> {
        let (sender, receiver) = std::sync::mpsc::channel();
        let _ = sender.send(ScanMessage::Done(Box::new(library)));
        receiver
    }

    /// A scan that has already failed, so a frame can take it in.
    fn failed(error: &str) -> Receiver<ScanMessage> {
        let (sender, receiver) = std::sync::mpsc::channel();
        let _ = sender.send(ScanMessage::Failed(error.to_string()));
        receiver
    }

    #[test]
    fn a_scan_that_failed_is_not_counted_as_a_scan() {
        // A share that is unmounted mid-session fails one scan. Counting that
        // as a scan would leave the next one an hour off, on exactly the
        // storage the fallback is there for.
        let mut app = app_with(&[("One", &["A"])]);
        app.config.set_roots(vec![std::env::temp_dir()]);
        app.auto_scan = true;
        app.scanned_at = None;
        app.scan = Some((failed("that directory is not there"), 0));

        app.poll_workers(&egui::Context::default());

        assert!(app.scan.is_none(), "the scan ended");
        assert!(
            app.scanned_at.is_none(),
            "a scan that read nothing does not count as a scan"
        );
        assert!(
            app.scan_after
                .is_some_and(|after| after <= Instant::now() + WATCH_RETRY),
            "so the next one is a minute off rather than an hour"
        );

        // And the retry is not the next frame: a share that is still away
        // would fail again on every one of them.
        app.poll_workers(&egui::Context::default());
        assert!(app.scan.is_none(), "the retry waits its minute");
    }

    #[test]
    fn the_scan_the_timer_runs_leaves_an_unchanged_library_alone() {
        // A minute of music playing with nothing added answers the library
        // the window already lists. Taking that in would put the album list
        // back at its first album and queue pictures it has already drawn.
        let mut app = app_with(&[("One", &["A", "B"]), ("Two", &["C"])]);
        app.open_album(1);
        // With no row selected there is nothing for the library to be rebuilt
        // around, so a library taken in again shows the first album.
        app.select(None);
        let listed = Library::from_tracks(app.library.tracks.clone());
        app.scan = Some((finished(listed), 0));
        app.scan_quiet = true;

        app.poll_workers(&egui::Context::default());

        assert!(app.scan.is_none(), "the scan ended");
        assert!(!app.scan_quiet, "the next scan is a fresh request");
        assert_eq!(
            app.view.album,
            Some(1),
            "the album list stays on the album it showed"
        );
    }

    #[test]
    fn a_scan_the_window_did_not_ask_for_shows_no_counter() {
        // The menu bar counts the files a scan has read. A scan the watcher
        // started is not one the listener asked for, and a counter coming and
        // going on its own is noise in the menu bar.
        let mut app = app_with(&[("One", &["A"])]);
        let (_sender, receiver) = std::sync::mpsc::channel();
        app.scan = Some((receiver, 7));
        app.scan_quiet = true;

        assert_eq!(app.scan_progress(), None, "a background scan draws nothing");

        app.scan_quiet = false;
        assert_eq!(app.scan_progress(), Some(7), "a rescan counts its files");

        app.scan = None;
        assert_eq!(app.scan_progress(), None, "and nothing is drawn with none");
    }

    #[test]
    fn the_scan_the_timer_runs_takes_in_what_changed() {
        let mut app = app_with(&[("One", &["A"])]);
        let mut tracks = app.library.tracks.clone();
        tracks.push(track("Two", 1, "B"));
        app.scan = Some((finished(Library::from_tracks(tracks)), 0));
        app.scan_quiet = true;

        app.poll_workers(&egui::Context::default());

        assert_eq!(app.library.tracks.len(), 2, "the new file is listed");
        assert_eq!(app.view.album, Some(0), "and the list is rebuilt");
    }

    #[test]
    fn a_rescan_by_hand_takes_the_library_in_even_when_nothing_changed() {
        // The one the user asked for says so by being clicked, so it is taken
        // in whatever it answers: a rescan that finds the same tracks is not
        // a reason to look as though the click did nothing.
        let mut app = app_with(&[("One", &["A", "B"]), ("Two", &["C"])]);
        app.open_album(1);
        app.select(None);
        let listed = Library::from_tracks(app.library.tracks.clone());
        app.scan = Some((finished(listed), 0));

        app.poll_workers(&egui::Context::default());

        assert_eq!(app.view.album, Some(0), "the library is taken in again");
    }

    #[test]
    fn the_fallback_scan_runs_once_an_hour_has_passed() {
        let mut app = app_with(&[("One", &["A"])]);
        app.config.set_roots(vec![std::env::temp_dir()]);
        app.auto_scan = true;
        let ctx = egui::Context::default();

        // A run that has not scanned yet is due at once, which is what a
        // window whose library is empty has to lose nothing by.
        app.scanned_at = None;
        app.poll_workers(&ctx);
        assert!(app.scan.is_some(), "a scan starts when one is due");

        app.scan = None;
        app.scanned_at = Some(Instant::now());
        app.poll_workers(&ctx);
        assert!(
            app.scan.is_none(),
            "the fallback is an hour off, and the watch is what notices changes"
        );
    }

    #[test]
    fn a_change_is_scanned_once_it_stops_arriving() {
        // Copying an album fires a change per file. A scan of each would read
        // the library over and over, so the scan waits for the burst to end,
        // and a further change pushes that wait back.
        let mut app = app_with(&[("One", &["A"])]);
        // A directory of its own, since anything written to the shared one
        // while the test runs is a change that pushes the wait back.
        let directory =
            std::env::temp_dir().join(format!("blue-dove-settle-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("the directory is made");
        app.config.set_roots(vec![directory.clone()]);
        app.auto_scan = true;
        app.scanned_at = Some(Instant::now());
        let ctx = egui::Context::default();

        app.scan_after = Some(Instant::now() + SETTLE);
        app.poll_workers(&ctx);
        assert!(app.scan.is_none(), "the burst has not ended yet");

        app.scan_after = Some(Instant::now());
        app.poll_workers(&ctx);
        assert!(app.scan.is_some(), "the change is scanned once it has");
        assert!(
            app.scan_quiet,
            "and quietly, since it may well find nothing new"
        );
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_change_under_the_music_directory_is_taken_in() {
        // This is the whole point of watching instead of polling: a file that
        // arrives while the window runs sets the wait that ends in a scan,
        // rather than waiting for the hour the fallback runs on.
        let directory =
            std::env::temp_dir().join(format!("blue-dove-watched-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("the directory is made");
        let mut app = app_with(&[("One", &["A"])]);
        app.config.set_roots(vec![directory.clone()]);
        app.auto_scan = true;
        app.scanned_at = Some(Instant::now());
        let ctx = egui::Context::default();
        app.poll_workers(&ctx);
        if app.watches.is_empty() {
            // A host that cannot watch a directory has only the fallback.
            let _ = std::fs::remove_dir_all(&directory);
            return;
        }
        assert!(app.scan_after.is_none(), "nothing has changed yet");

        std::fs::write(directory.join("song.wav"), b"not really audio")
            .expect("the file is written");
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.scan_after.is_none() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
            app.poll_workers(&ctx);
        }

        assert!(
            app.scan_after.is_some(),
            "the change is noted, so a scan follows once it settles"
        );
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_watch_that_ended_is_replaced() {
        // A directory deleted, renamed or unmounted takes its watch with it,
        // and the kernel says so once. Keeping the dead watch because its path
        // still matches leaves the hourly fallback as the only thing that
        // notices, so a file written after the directory comes back stays
        // invisible for an hour.
        let directory =
            std::env::temp_dir().join(format!("blue-dove-replaced-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("the directory is made");
        let mut app = app_with(&[("One", &["A"])]);
        app.config.set_roots(vec![directory.clone()]);
        app.auto_scan = true;
        // The timer is not what this is about: what it would start would run
        // beside the watch and say nothing about it.
        app.scanned_at = Some(Instant::now());
        let ctx = egui::Context::default();
        app.poll_workers(&ctx);
        if app.watches.is_empty() {
            // A host that cannot watch a directory has only the fallback.
            let _ = std::fs::remove_dir_all(&directory);
            return;
        }

        std::fs::remove_dir_all(&directory).expect("the directory goes");
        let deadline = Instant::now() + Duration::from_secs(5);
        while !app.watches.iter().any(crate::watch::Watch::ended) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            app.watches.iter().any(crate::watch::Watch::ended),
            "the watch on the removed directory ended"
        );

        // The directory comes back, and with it a watch that reports changes.
        std::fs::create_dir_all(&directory).expect("the directory comes back");
        app.poll_workers(&ctx);
        assert!(
            app.watches.iter().any(|watch| !watch.ended()),
            "a live watch is placed on the directory that came back"
        );

        // And it is alive, not merely present: what is written under it is
        // reported.
        std::fs::write(directory.join("song.wav"), b"not really audio")
            .expect("the file is written");
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.scan_after.is_none() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
            app.poll_workers(&ctx);
        }
        assert!(
            app.scan_after.is_some(),
            "a file written after the directory came back is reported"
        );
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn adding_a_directory_keeps_the_ones_before_it() {
        let mut app = App::new(&egui::Context::default(), true, Vec::new());
        let first = std::env::temp_dir().join(format!("blue-dove-add-{}", std::process::id()));
        let second = std::env::temp_dir().join(format!("blue-dove-add-two-{}", std::process::id()));

        app.add_directory(first.clone());
        app.add_directory(second.clone());
        app.add_directory(first.clone());

        assert_eq!(
            app.config.roots(),
            [first, second],
            "each directory is added once, and the order is kept"
        );
        assert!(app.scan.is_some(), "and the library is scanned again");
    }

    #[test]
    fn removing_a_directory_leaves_the_others_scanned() {
        let mut app = App::new(&egui::Context::default(), true, Vec::new());
        let first = std::env::temp_dir().join(format!("blue-dove-keep-{}", std::process::id()));
        let second = std::env::temp_dir().join(format!("blue-dove-drop-{}", std::process::id()));
        app.config.set_roots(vec![first.clone(), second.clone()]);

        app.remove_directory(&first);

        assert_eq!(app.config.roots(), [second], "only the other is left");
        assert!(app.scan.is_some(), "and what is left is scanned");
    }

    #[test]
    fn removing_the_last_directory_leaves_the_window_as_it_is() {
        // There is nothing left to scan, and blanking the album list under the
        // user would take away the tracks they may have just queued.
        let mut app = App::new(&egui::Context::default(), true, Vec::new());
        let directory = std::env::temp_dir().join(format!("blue-dove-last-{}", std::process::id()));
        app.config.set_roots(vec![directory.clone()]);

        app.remove_directory(&directory);

        assert!(app.config.roots().is_empty());
        assert!(
            app.scan.is_none(),
            "nothing is scanned with nothing to scan"
        );
        assert!(
            app.status.is_none(),
            "and nothing is reported as having gone wrong"
        );
    }

    #[test]
    fn a_directory_left_out_of_the_scan_can_be_put_back() {
        let mut app = App::new(&egui::Context::default(), true, Vec::new());
        let directory = std::env::temp_dir().join(format!("blue-dove-in-{}", std::process::id()));
        let left_out = std::env::temp_dir().join(format!("blue-dove-out-{}", std::process::id()));
        app.config.set_roots(vec![directory]);

        app.exclude_directory(left_out.clone());

        assert_eq!(app.config.excluded(), std::slice::from_ref(&left_out));
        assert!(app.scan.is_some(), "the library is read again without it");

        app.include_directory(&left_out);

        assert!(app.config.excluded().is_empty(), "it is scanned again");
    }

    #[test]
    fn a_watch_is_started_on_the_music_directory() {
        let mut app = app_with(&[("One", &["A"])]);
        let directory =
            std::env::temp_dir().join(format!("blue-dove-autoscan-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("the directory is made");
        app.config.set_roots(vec![directory.clone()]);
        app.auto_scan = true;

        app.poll_workers(&egui::Context::default());

        assert!(
            app.watches.iter().any(|watch| watch.root() == directory),
            "the directory the library is read from is the one watched"
        );
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn turning_the_automatic_scan_off_stops_the_watch() {
        let mut app = app_with(&[("One", &["A"])]);
        app.config.set_roots(vec![std::env::temp_dir()]);
        app.auto_scan = true;
        app.poll_workers(&egui::Context::default());

        app.set_auto_scan(false);
        app.poll_workers(&egui::Context::default());

        assert!(app.watches.is_empty(), "nothing is watched with it off");
    }

    #[test]
    fn the_automatic_scan_can_be_turned_off() {
        let mut app = app_with(&[("One", &["A"])]);
        app.config.set_roots(vec![std::env::temp_dir()]);
        app.set_auto_scan(false);
        app.scanned_at = None;

        app.poll_workers(&egui::Context::default());

        assert!(app.scan.is_none(), "nothing is scanned with the timer off");
        assert_eq!(app.config.auto_scan, Some(false), "and it is remembered");
    }

    #[test]
    fn turning_the_automatic_scan_on_scans_at_once() {
        // The setting covers the scan at startup and the one on a timer, so
        // waiting a minute for either would look as though it did nothing.
        let mut app = app_with(&[("One", &["A"])]);
        app.config.set_roots(vec![std::env::temp_dir()]);

        app.set_auto_scan(true);

        assert!(app.scan.is_some(), "a scan starts with the setting");
    }

    #[test]
    fn a_session_that_finds_no_files_is_kept_for_the_next_run() {
        // An unmounted drive looks like an empty library; the saved tabs must
        // not be overwritten with nothing.
        let mut app = App::new(&egui::Context::default(), true, Vec::new());
        let saved = vec![SavedPlaylist {
            name: "Game rips".to_string(),
            tracks: vec![PathBuf::from("/music/rips/track.adx")],
        }];
        app.config.playlists = Some(saved);
        app.restore_playlists();

        assert_eq!(
            app.playlists.items.len(),
            1,
            "the window keeps its default tab"
        );
        assert_eq!(app.playlists.current().name, "Default");
        assert_eq!(
            app.config.playlists.as_ref().map(Vec::len),
            Some(1),
            "the saved tabs wait for the drive"
        );
    }

    #[test]
    fn a_run_with_nothing_saved_keeps_the_default_playlist() {
        let mut app = app_with(&[("One", &["A"])]);
        app.config.playlists = None;
        app.restore_playlists();
        assert_eq!(app.playlists.items.len(), 1);
        assert_eq!(app.playlists.current().name, "Default");
    }

    #[test]
    fn repeat_and_random_are_remembered() {
        let mut app = App::new(&egui::Context::default(), true, Vec::new());
        app.set_repeat(true);
        app.set_random(true);
        assert_eq!(app.config.repeat, Some(true));
        assert_eq!(app.config.random, Some(true));
        app.set_repeat(false);
        assert_eq!(app.config.repeat, Some(false));
    }

    #[test]
    fn repeat_restarts_the_playing_track() {
        // The auto-advance branch reads these two, so this pins the rule:
        // with repeat on, the track that ended is the one that plays again.
        let mut app = app_with(&[("One", &["A", "B"])]);
        app.playing = Some(0);
        app.set_repeat(true);
        let restart = if app.repeat { app.playing } else { None };
        assert_eq!(restart, Some(0), "repeat restarts, it does not step");

        app.set_repeat(false);
        let restart = if app.repeat { app.playing } else { None };
        assert_eq!(restart, None, "without repeat, playback steps on");
    }

    #[test]
    fn a_smoke_run_starts_with_nothing_to_do() {
        let app = App::new(&egui::Context::default(), true, Vec::new());
        assert!(app.scan.is_none());
        assert!(app.picker.is_none());
        assert!(app.library.is_empty());
    }

    #[test]
    fn closing_keeps_the_music_only_while_it_plays() {
        let mut app = App::new(&egui::Context::default(), true, Vec::new());
        app.background = true;
        app.playing = Some(0);
        assert!(
            app.keeps_playing_on_close(),
            "the option is on and a track is loaded"
        );

        app.quitting = true;
        assert!(!app.keeps_playing_on_close(), "Quit ends the app");

        app.quitting = false;
        app.playing = None;
        assert!(
            !app.keeps_playing_on_close(),
            "a window with nothing to play has no reason to hide"
        );

        app.playing = Some(0);
        app.background = false;
        assert!(!app.keeps_playing_on_close(), "the option is off");
    }

    #[test]
    fn a_blind_comparison_takes_the_two_selected_tracks() {
        let mut app = app_with(&[("One", &["A", "B", "C"])]);
        app.select_only(0);
        app.start_abx();
        assert!(app.abx.is_none(), "one row is not a pair");

        app.select_only(0);
        app.toggle_selection(1);
        app.start_abx();
        let test = app.abx.as_ref().expect("two rows start a comparison");
        assert_eq!(test.track(Side::A).title, "A");
        assert_eq!(test.track(Side::B).title, "B");
    }

    #[test]
    fn a_blind_comparison_counts_ten_answers_and_then_ends() {
        let mut app = app_with(&[("One", &["A", "B"])]);
        app.select_only(0);
        app.toggle_selection(1);
        app.start_abx();

        // The window never learns which side X is on, so neither does this:
        // the same answer ten times scores what it scores.
        for _ in 0..TRIALS {
            app.abx_answer(Side::A);
            app.abx_next();
        }

        let test = app.abx.as_ref().expect("the comparison is open");
        assert!(test.done());
        assert_eq!(test.correct() + test.wrong(), TRIALS);
    }

    #[test]
    fn playing_a_row_ends_the_comparison() {
        let mut app = app_with(&[("One", &["A", "B"])]);
        app.select_only(0);
        app.toggle_selection(1);
        app.start_abx();

        app.play(0);
        assert!(
            app.abx.is_none(),
            "a row played by hand takes the device back"
        );
    }

    #[test]
    fn closing_the_comparison_leaves_nothing_playing() {
        let mut app = app_with(&[("One", &["A", "B"])]);
        app.select_only(0);
        app.toggle_selection(1);
        app.start_abx();

        app.close_abx();
        assert!(app.abx.is_none());
        assert!(app.playing.is_none());
    }

    /// Every run of text a frame drew, so a test can say what a window shows.
    fn drawn_text(output: &egui::FullOutput) -> Vec<String> {
        fn walk(shape: &egui::epaint::Shape, into: &mut Vec<String>) {
            match shape {
                egui::epaint::Shape::Text(text) => into.push(text.galley.text().to_string()),
                egui::epaint::Shape::Vec(shapes) => {
                    for shape in shapes {
                        walk(shape, into);
                    }
                }
                _ => {}
            }
        }
        let mut text = Vec::new();
        for clipped in &output.shapes {
            walk(&clipped.shape, &mut text);
        }
        text
    }

    #[test]
    fn the_sources_window_lists_what_is_scanned_and_what_is_left_out() {
        let mut app = App::new(&egui::Context::default(), true, Vec::new());
        app.config
            .set_roots(vec![PathBuf::from("/music"), PathBuf::from("/mnt/rips")]);
        app.config
            .set_excluded(vec![PathBuf::from("/music/Christmas")]);
        app.directories_open = true;

        let ctx = egui::Context::default();
        // A window needs a screen to be laid out in, and its size a frame to
        // settle on, so the first one is drawn and thrown away.
        let mut input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1024.0, 768.0),
            )),
            ..egui::RawInput::default()
        };
        let mut output = ctx.run_pass(input.clone(), |ctx| app.directories_window(ctx));
        output.textures_delta.clear();
        input.time = Some(0.1);
        let mut output = ctx.run_pass(input, |ctx| app.directories_window(ctx));
        output.textures_delta.clear();

        let drawn = drawn_text(&output).join("\n");
        assert!(
            drawn.contains("/mnt/rips"),
            "every source is listed:\n{drawn}"
        );
        assert!(
            drawn.contains("/music/Christmas"),
            "and so is every directory left out:\n{drawn}"
        );
        assert!(
            drawn.contains("Add Directory"),
            "the window says how to add another:\n{drawn}"
        );
    }

    #[test]
    fn the_comparison_window_draws_a_trial_an_answer_and_a_result() {
        /// Draws the window once, on a context of its own.
        ///
        /// Nothing uploads the textures a first frame produces, and the
        /// delta is loud about being dropped unapplied, so it is cleared.
        fn draw(app: &mut App) {
            let ctx = egui::Context::default();
            let mut output = ctx.run_pass(egui::RawInput::default(), |ctx| app.abx_window(ctx));
            output.textures_delta.clear();
        }

        let mut app = app_with(&[("One", &["A", "B"])]);
        app.select_only(0);
        app.toggle_selection(1);
        app.start_abx();

        // A trial with nothing answered, then the same trial with the answer
        // and the way on shown, and last the result: the three shapes the
        // window takes.
        draw(&mut app);
        app.abx_answer(Side::A);
        draw(&mut app);
        app.abx_next();
        for _ in 1..TRIALS {
            app.abx_answer(Side::A);
            app.abx_next();
        }
        assert!(app.abx.as_ref().is_some_and(|test| test.done()));
        draw(&mut app);
    }
}
