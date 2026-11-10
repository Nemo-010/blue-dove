//! Drawing: the menus, the track table, the album list and the player bar.
//!
//! Every function here reads [`App`] state and turns input into calls on it.

use std::path::PathBuf;
use std::time::Duration;

use eframe::egui::{self, Align2, Color32, FontId, Rect, Sense, Stroke, pos2, vec2};

use crate::abx::{Abx, Choice, Side, TRIALS};
use crate::app::{App, COMPACT_NAME_HEIGHT, COMPACT_WAVE_HEIGHT, Pick, ROW_HEIGHT, WAVE_HEIGHT};
use crate::format;
use crate::library::{Album, Track};
use crate::theme;

/// Width of the margin on either side of a track row, in points.
const ROW_PADDING: f32 = 8.0;

/// Height of a directory row in the album list, in points.
const DIRECTORY_ROW_HEIGHT: f32 = 24.0;

/// Height of a loose track row, smaller than an album row so a directory's
/// stray files read as secondary.
const LOOSE_ROW_HEIGHT: f32 = 18.0;

/// What a click on the album list asked for.
///
/// Rows are drawn from a copy of the tree, so the drawing cannot act on the
/// app while it holds that copy; it records the request and the caller
/// applies it once the drawing is done.
enum RowAction {
    /// Make this album the playlist.
    OpenAlbum(usize),
    /// Queue every album in this directory and below it.
    AddDirectory(std::path::PathBuf),
    /// Fold or unfold this directory.
    Toggle(std::path::PathBuf),
    /// Play this loose track on its own.
    PlayLoose(usize),
}

/// How far apart the two answers are drawn, on top of the usual gap.
///
/// A guess is the one press in the window that must not land on the wrong
/// button, so the two are given room between them.
const ANSWER_GAP: f32 = 7.0;

/// How much of a checksum the blind comparison shows, with the whole one on
/// hover: the two files are what the test compares, and their sums are what
/// tells two encodes of one song apart.
const SUM_CHARS: usize = 8;

/// The size of the blind comparison's window, which never changes.
///
/// It is the room the longest state of the test needs: both tracks with their
/// sums, the three play buttons, the trial, the answer row and the footer.
const ABX_WINDOW_SIZE: [f32; 2] = [420.0, 292.0];

/// Width the library sources window opens at, in points.
///
/// Wide enough for a path of a few directories, and fixed so the window does
/// not grow around the deepest one the user has.
const DIRECTORIES_WIDTH: f32 = 480.0;

/// Characters a track's name may take before it is cut short, so that the
/// sum beside it always fits.
const NAME_CHARS: usize = 44;

/// What a click in the blind comparison asked for.
///
/// It is the same split as [`RowAction`]: the test is drawn from a shared
/// borrow of the app, so the drawing records the request and the caller
/// applies it once the drawing is done.
enum AbxAction {
    /// Play the track this choice names.
    Play(Choice),
    /// Answer that X is this side.
    Answer(Side),
    /// Move to the trial after the one answered.
    Next,
    /// Run the test again with the same two tracks.
    Restart,
    /// Close the test.
    Close,
    /// Move the track under the bar to this moment.
    Seek(Duration),
    /// Play both tracks from this moment.
    Start(Duration),
}

/// Side of the cover shown beside each album in the library list.
const ALBUM_ART: f32 = 37.0;

/// Height of an album row in the album list, in points.
const ALBUM_ROW: f32 = 54.0;

/// How far an album or loose track sits in from its directory's arrow.
const ALBUM_INDENT: f32 = 8.0;

/// How far each level of the directory tree steps in.
///
/// Deep libraries nest several directories, and a wide step spends the panel's
/// width on empty space; a few points still read as a level without it.
const TREE_STEP: f32 = 3.0;

/// Margin around the album art, so it does not touch the panel edges.
const ART_MARGIN: f32 = 8.0;

/// Height of the strip the controls sit in, below the art.
const CONTROL_STRIP: f32 = 34.0;

/// How much one wheel notch over the volume slider changes the volume.
const VOLUME_PER_NOTCH: f32 = 0.05;

/// How many points egui reports for one wheel line, used to turn a
/// point-based wheel delta into notches.
const WHEEL_POINTS_PER_LINE: f32 = 50.0;

/// Height of the column titles above the track table.
const HEADER_HEIGHT: f32 = 20.0;

/// Height of an album's heading in the track table, in points.
///
/// It is taller than a track row so the cover beside the name reads as a
/// picture rather than a smudge.
const ALBUM_ROW_HEIGHT: f32 = 50.0;

/// Side of the cover shown in an album's heading.
///
/// Twice the height of a track row, so the heading reads as the start of an
/// album rather than as one more line of the list.
const ALBUM_ROW_ART: f32 = 40.0;

/// One line of the track table.
///
/// A playlist is drawn as runs of albums: each run starts with a heading
/// naming the album and showing its cover, so the table says where one album
/// ends and the next begins.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TableRow {
    /// The heading of the album at this library index.
    Album(usize),
    /// The track at this library index.
    Track(usize),
}

impl TableRow {
    /// How tall the line is drawn.
    fn height(self) -> f32 {
        match self {
            Self::Album(_) => ALBUM_ROW_HEIGHT,
            Self::Track(_) => ROW_HEIGHT,
        }
    }
}

/// Lays the playlist out as table rows, heading each run of an album.
///
/// `albums` holds the album of every track, as [`crate::library::Library::album_indices`]
/// gives it. A heading is drawn where an album starts, so a playlist that
/// goes back to an earlier album heads it again; a loose file heads nothing,
/// since it belongs to no album.
fn table_rows(tracks: &[usize], albums: &[Option<usize>]) -> Vec<TableRow> {
    let mut rows = Vec::with_capacity(tracks.len() + tracks.len() / 8);
    let mut previous = None;
    for &track in tracks {
        let album = albums.get(track).copied().flatten();
        if let Some(album) = album
            && previous != Some(album)
        {
            rows.push(TableRow::Album(album));
        }
        previous = album;
        rows.push(TableRow::Track(track));
    }
    rows
}

/// Where each row starts, in points from the top of the table.
///
/// The rows are not all the same height, so the drawing and the scrolling
/// both read their places from here rather than multiplying by one height.
fn row_tops(rows: &[TableRow]) -> Vec<f32> {
    let mut tops = Vec::with_capacity(rows.len());
    let mut top = 0.0;
    for row in rows {
        tops.push(top);
        top += row.height();
    }
    tops
}

/// The total height of `rows`, in points.
fn rows_height(rows: &[TableRow]) -> f32 {
    rows.iter().map(|row| row.height()).sum()
}

/// The rows the viewport shows, as indices into `tops`.
///
/// A row that starts above the viewport is still drawn when its lower half
/// shows, so the first index steps back one.
fn visible_rows(tops: &[f32], viewport: Rect) -> std::ops::Range<usize> {
    let first = tops
        .partition_point(|&top| top < viewport.min.y)
        .saturating_sub(1);
    let last = (tops.partition_point(|&top| top < viewport.max.y) + 1).min(tops.len());
    first..last
}

/// The rect of a row, `top` points below `origin`.
fn row_rect(origin: egui::Pos2, top: f32, width: f32, height: f32) -> Rect {
    Rect::from_min_size(pos2(origin.x, origin.y + top), vec2(width, height))
}

/// Width of the track number column, in points, before the user drags it.
pub(crate) const NUMBER_COLUMN: f32 = 38.0;

/// The narrowest a column is drawn at.
pub(crate) const MIN_COLUMN: f32 = 48.0;

/// The narrowest the number column is drawn at.
///
/// It holds a track number and no more, so it needs less room than the
/// columns that carry text.
const MIN_NUMBER_COLUMN: f32 = 22.0;

/// How far from a column border the pointer counts as being on it.
const GRAB: f32 = 4.0;

/// A column of the track table, in the order the columns are drawn.
///
/// The title column holds the tags' titles and the file name column holds the
/// names of the files themselves, so a directory whose tags are wrong or missing
/// can be read by file name instead. Either can be turned off, and both can be
/// shown at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Column {
    /// The track number.
    Number,
    /// The track's title, as its tags give it.
    Title,
    /// The name of the file itself.
    FileName,
    /// The performing artist.
    Artist,
    /// The album, with its year.
    Album,
    /// The playing time.
    Length,
}

impl Column {
    /// Every column, left to right.
    pub(crate) const ALL: [Column; 6] = [
        Column::Number,
        Column::Title,
        Column::FileName,
        Column::Artist,
        Column::Album,
        Column::Length,
    ];

    /// Where the column sits in the arrays the app keeps.
    pub(crate) fn index(self) -> usize {
        self as usize
    }

    /// The name the column's title shows.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Column::Number => "#",
            Column::Title => "Title",
            Column::FileName => "File Name",
            Column::Artist => "Artist",
            Column::Album => "Album",
            Column::Length => "Length",
        }
    }

    /// The narrowest this column is drawn at.
    pub(crate) fn min_width(self) -> f32 {
        match self {
            Column::Number => MIN_NUMBER_COLUMN,
            _ => MIN_COLUMN,
        }
    }

    /// Whether the column holds a short, fixed run of digits.
    ///
    /// A number or a length is a few characters wide whatever the panel
    /// does, so growing one only adds empty space beside its value.
    fn is_fixed(self) -> bool {
        matches!(self, Column::Number | Column::Length)
    }
}

/// The total of the stored widths of the columns shown, and the largest
/// factor that still leaves each of them at its own minimum.
fn column_totals(stored: [f32; 6], shown: [bool; 6]) -> (f32, f32) {
    let mut total = 0.0;
    let mut floor = 0.0_f32;
    for column in Column::ALL {
        let index = column.index();
        if shown[index] {
            total += stored[index];
            floor = floor.max(column.min_width() / stored[index].max(1.0));
        }
    }
    (total, floor)
}

/// Column widths of the track table, in points: number, title, file name,
/// artist, album and length, plus the factor they were scaled by.
///
/// The widths are stored as they were dragged, and scaled together to the
/// panel: wider than stored when there is room, and narrower when there is
/// not, down to each column's own minimum. Only when even that does not fit
/// does the table keep its smallest width and scroll sideways. A column the
/// user turned off is given no width, and takes no part in the scaling, so
/// the others get its room.
fn column_layout(width: f32, stored: [f32; 6], shown: [bool; 6]) -> ([f32; 6], f32) {
    let available = (width - 2.0 * ROW_PADDING).max(0.0);
    let (total, floor) = column_totals(stored, shown);
    let scale = if total > 0.0 {
        (available / total).max(floor).max(0.05)
    } else {
        1.0
    };
    let mut widths = [0.0; 6];
    for column in Column::ALL {
        let index = column.index();
        if shown[index] {
            widths[index] = stored[index] * scale;
        }
    }
    if scale > 1.0 {
        grow(&mut widths, available, stored, shown);
    }
    (widths, scale)
}

/// Gives a wide panel's spare room to the columns that carry text.
///
/// The number and length columns hold a few digits, so a wider one is only
/// empty space beside its value; they keep the width they were dragged to,
/// and the columns that hold names take the rest. Without this the length
/// column ends up with a gap on the left of every value that no drag can
/// close, since its width is already at its smallest.
fn grow(widths: &mut [f32; 6], available: f32, stored: [f32; 6], shown: [bool; 6]) {
    let sum = |fixed: bool| {
        Column::ALL
            .iter()
            .filter(|column| shown[column.index()] && column.is_fixed() == fixed)
            .map(|column| stored[column.index()])
            .sum::<f32>()
    };
    let fixed = sum(true);
    let flexible = sum(false);
    // A table with no text column left has nothing to grow into, so the
    // columns keep the even scaling they were given.
    if flexible <= 0.0 || available <= fixed {
        return;
    }
    let room = available - fixed;
    for column in Column::ALL {
        let index = column.index();
        if !shown[index] {
            continue;
        }
        widths[index] = if column.is_fixed() {
            stored[index]
        } else {
            stored[index] / flexible * room
        };
    }
}

/// The track to bring into view, if any.
///
/// The track is looked up in the whole list rather than among the rows
/// drawn, so one that is scrolled out of sight still scrolls to it.
fn row_to_reveal(reveal: bool, selected: Option<usize>, tracks: &[usize]) -> Option<usize> {
    if !reveal {
        return None;
    }
    let index = selected?;
    tracks.contains(&index).then_some(index)
}

/// One row of the album list.
///
/// The list is a flattened tree so it can draw only the rows on screen; each
/// row carries what it needs to be drawn, not a borrow of the library.
#[derive(Clone, Debug)]
enum LibraryRow {
    /// A directory, which can be folded away.
    Directory {
        /// Where it sits in the music directory.
        path: std::path::PathBuf,
        /// Its name, without its parent path.
        name: String,
        /// How far the tree steps it in.
        depth: usize,
        /// Whether it is folded away.
        collapsed: bool,
    },
    /// An album, by index into [`crate::library::Library::albums`].
    Album {
        /// Which album.
        album: usize,
        /// How far the tree steps it in.
        depth: usize,
    },
    /// A file that belongs to no album, by index into the tracks.
    Loose {
        /// Which track.
        track: usize,
        /// How far the tree steps it in.
        depth: usize,
    },
}

impl LibraryRow {
    /// How tall the row is drawn.
    fn height(&self) -> f32 {
        match self {
            Self::Directory { .. } => DIRECTORY_ROW_HEIGHT,
            Self::Album { .. } => ALBUM_ROW,
            Self::Loose { .. } => LOOSE_ROW_HEIGHT,
        }
    }

    /// The track a loose row stands for.
    ///
    /// Only meaningful for [`LibraryRow::Loose`], which is the only row a
    /// selection can land on.
    fn track_index(&self) -> usize {
        match self {
            Self::Loose { track, .. } => *track,
            _ => usize::MAX,
        }
    }
}

/// Where each row of the album list starts, in points from its top.
fn library_row_tops(rows: &[LibraryRow]) -> Vec<f32> {
    let mut tops = Vec::with_capacity(rows.len());
    let mut top = 0.0;
    for row in rows {
        tops.push(top);
        top += row.height();
    }
    tops
}

/// The total height of `rows`, in points.
fn library_rows_height(rows: &[LibraryRow]) -> f32 {
    rows.iter().map(|row| row.height()).sum()
}

/// The gap above the waveform bar, which leaves as much below it.
///
/// A strip shorter than the bar gets no gap, so the bar is never pushed out
/// of its strip.
fn wave_gap(strip: f32, height: f32) -> f32 {
    ((strip - height) / 2.0).max(0.0)
}

/// Where each column starts, in points from the row's left edge.
///
/// The header and the rows both take their positions from here, so a label
/// always sits over the values it names.
fn column_starts(widths: [f32; 6]) -> [f32; 6] {
    let mut starts = [0.0; 6];
    let mut x = ROW_PADDING;
    for (index, width) in widths.iter().enumerate() {
        starts[index] = x;
        x += width;
    }
    starts
}

/// Draws one directory in the library sources window, with its button.
///
/// Answers whether the button was clicked. The path is cut short rather than
/// wrapped, so a deep path does not make the window wider or the row taller
/// than the ones around it.
fn directory_line(ui: &mut egui::Ui, directory: &std::path::Path, button: &str) -> bool {
    let mut clicked = false;
    ui.horizontal(|ui| {
        if ui.small_button(button).clicked() {
            clicked = true;
        }
        ui.add(egui::Label::new(directory.display().to_string()).truncate());
    });
    clicked
}

/// The cell of `column` in a row, in `rect`'s coordinates.
///
/// The length is pinned to the row's right edge rather than starting where
/// its column does, so its digits line up with the edge at any width.
fn column_cell(rect: Rect, starts: [f32; 6], widths: [f32; 6], column: Column) -> Rect {
    let index = column.index();
    if column == Column::Length {
        Rect::from_min_max(
            pos2(rect.right() - ROW_PADDING - widths[index], rect.top()),
            rect.max,
        )
    } else {
        Rect::from_min_size(
            pos2(rect.left() + starts[index], rect.top()),
            vec2(widths[index], rect.height()),
        )
    }
}

/// The narrowest the table is drawn at, which is where it starts scrolling.
fn table_needed(stored: [f32; 6], shown: [bool; 6]) -> f32 {
    let (total, floor) = column_totals(stored, shown);
    2.0 * ROW_PADDING + total * floor
}

/// The transport buttons drawn in the player bar.
#[derive(Clone, Copy)]
enum Icon {
    /// Skip to the previous track.
    Previous,
    /// Start or resume playback.
    Play,
    /// Pause playback.
    Pause,
    /// Skip to the next track.
    Next,
    /// Stop playback.
    Stop,
    /// Repeat the playing track.
    Repeat,
    /// Pick the next track at random.
    Random,
}

impl App {
    /// Draws a checkbox for a copy of `value`, applying the setting when it
    /// changes.
    ///
    /// The settings are applied through the app's own setters, so a checkbox
    /// that edits a copy cannot leave the state and the widget out of step.
    /// The response comes back for the callers that do more than apply it:
    /// the transparency checkbox re-applies the theme, and some carry hover
    /// text.
    fn setting_checkbox(
        &mut self,
        ui: &mut egui::Ui,
        label: impl Into<egui::WidgetText>,
        value: bool,
        set: impl FnOnce(&mut Self, bool),
    ) -> egui::Response {
        let mut value = value;
        let response = ui.checkbox(&mut value, label);
        if response.changed() {
            set(self, value);
        }
        response
    }

    /// The File, Playback, View and Help menus.
    pub(crate) fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("menu").show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Manage Library Sources...").clicked() {
                        ui.close();
                        self.directories_open = true;
                    }
                    let scanned = !self.config.roots().is_empty();
                    if ui
                        .add_enabled(scanned, egui::Button::new("Rescan Library"))
                        .clicked()
                    {
                        ui.close();
                        self.start_scan();
                    }
                    self.setting_checkbox(
                        ui,
                        "Scan Automatically",
                        self.auto_scan,
                        Self::set_auto_scan,
                    )
                    .on_hover_text(
                        "Watches the music directories and rescans them when a file \
                         changes, so new files show up without a rescan",
                    );
                    ui.separator();
                    self.setting_checkbox(
                        ui,
                        "Keep Playing When Closed",
                        self.background,
                        Self::set_background,
                    );
                    ui.separator();
                    if ui.button("Quit").clicked() {
                        // A close is hidden rather than quit when the music
                        // is to play on, so this says it is a real one.
                        self.quitting = true;
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });

                ui.menu_button("Playback", |ui| {
                    let label = if self.paused() { "Play" } else { "Pause" };
                    if ui.button(label).clicked() {
                        ui.close();
                        self.toggle();
                    }
                    if ui.button("Next Track").clicked() {
                        ui.close();
                        self.step(true);
                    }
                    if ui.button("Previous Track").clicked() {
                        ui.close();
                        self.step(false);
                    }
                    ui.separator();

                    self.setting_checkbox(ui, "Repeat Track", self.repeat, Self::set_repeat);
                    self.setting_checkbox(ui, "Random Order", self.random, Self::set_random);
                });

                ui.menu_button("View", |ui| {
                    if self
                        .setting_checkbox(
                            ui,
                            "Translucent background",
                            self.transparency,
                            Self::set_transparency,
                        )
                        .changed()
                    {
                        theme::apply(ui.ctx(), self.theme, self.opacity());
                    }

                    self.setting_checkbox(
                        ui,
                        "Ignore hidden files",
                        self.ignore_hidden,
                        Self::set_ignore_hidden,
                    )
                    .on_hover_text("Leaves out files and directories whose name starts with a dot");

                    let ctx = ui.ctx().clone();
                    self.setting_checkbox(
                        ui,
                        "Preview a single file",
                        self.preview_enabled,
                        |app, value| app.set_preview_enabled(&ctx, value),
                    )
                    .on_hover_text(
                        "Opens a launch naming one audio file as a small player \
                         that leaves the playlists untouched",
                    );

                    ui.separator();
                    ui.menu_button("Theme", |ui| {
                        for theme in theme::Theme::ALL {
                            let chosen = self.theme == theme;
                            if ui.selectable_label(chosen, theme.name()).clicked() {
                                self.set_theme(theme);
                                theme::apply(ui.ctx(), theme, self.opacity());
                                // The whole window changes colour, and
                                // nothing else in this frame asks for the
                                // repaint that would show it.
                                ui.ctx().request_repaint();
                                ui.close();
                            }
                        }
                    });
                });

                ui.menu_button("Help", |ui| {
                    ui.label(format!("{} {}", Self::TITLE, env!("CARGO_PKG_VERSION")));
                    ui.label(format!("Renderer: {}", self.renderer));
                    ui.label("Plays the music in the directories you add.");
                    ui.hyperlink_to(
                        "Documentation",
                        "https://pkgforge-apps.github.io/blue-dove/",
                    );
                    ui.label("MIT licensed.");
                });

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // The scan is the only status the library column has
                    // room for; the track count lives in the library header.
                    if let Some(seen) = self.scan_progress() {
                        ui.label(
                            egui::RichText::new(format!("{seen} files"))
                                .color(ui.visuals().weak_text_color()),
                        );
                    }
                });
            });
        });
    }

    /// Draws the library sources window, when it is open.
    ///
    /// It lists the directories the library is read from and the ones the
    /// scan leaves out, and is where both lists are edited: a source added,
    /// removed, left out or put back is scanned again as soon as the window
    /// lets go of it.
    pub(crate) fn directories_window(&mut self, ctx: &egui::Context) {
        if !self.directories_open {
            return;
        }
        let roots: Vec<PathBuf> = self.config.roots().to_vec();
        let excluded: Vec<PathBuf> = self.config.excluded().to_vec();
        let mut open = self.directories_open;
        let mut add = false;
        let mut exclude = false;
        let mut remove = None;
        let mut include = None;

        egui::Window::new("Library Sources")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(DIRECTORIES_WIDTH)
            .show(ctx, |ui| {
                ui.label("The library is read from these directories and everything below them.");
                ui.add_space(4.0);
                if roots.is_empty() {
                    ui.weak("No directory yet.");
                }
                for (index, directory) in roots.iter().enumerate() {
                    if directory_line(ui, directory, "Remove") {
                        remove = Some(index);
                    }
                }
                ui.add_space(4.0);
                if ui.button("Add Directory...").clicked() {
                    add = true;
                }

                ui.separator();
                ui.label("Directories left out, with everything under them:");
                ui.add_space(4.0);
                if excluded.is_empty() {
                    ui.weak("Nothing is left out.");
                }
                for (index, directory) in excluded.iter().enumerate() {
                    if directory_line(ui, directory, "Scan Again") {
                        include = Some(index);
                    }
                }
                ui.add_space(4.0);
                if ui.button("Exclude Directory...").clicked() {
                    exclude = true;
                }
            });

        self.directories_open = open;
        if add {
            self.ask_for_directory(Pick::Add);
        }
        if exclude {
            self.ask_for_directory(Pick::Exclude);
        }
        if let Some(index) = remove
            && let Some(directory) = roots.get(index)
        {
            self.remove_directory(directory);
        }
        if let Some(index) = include
            && let Some(directory) = excluded.get(index)
        {
            self.include_directory(directory);
        }
    }

    /// Draws the blind comparison, when one is under way.
    ///
    /// It is a window over the playlist rather than a panel in it, so the
    /// album list and the table stay where they are and the listener can
    /// pick another pair as soon as the test ends. It opens in the middle of
    /// the window and can be dragged from there.
    pub(crate) fn abx_window(&mut self, ctx: &egui::Context) {
        // A sum is read on a thread of its own, and nothing else would wake
        // the window to take it in while the listener has yet to press a
        // button: the frame that shows it has to be asked for.
        if self.abx_reading.is_some() {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
        let Some(test) = &self.abx else {
            return;
        };
        let sums = self
            .abx_sums
            .as_ref()
            .map(|(a, b)| (a.as_str(), b.as_str()));
        let signature = test.signature(sums);
        let log = test.signed_log(sums);
        let view = AbxView {
            test,
            sums: self.abx_sums.as_ref(),
            position: self.position(),
            length: self.length,
            start: self.abx_start,
            accent: self.accent(),
            signature: signature.as_deref(),
            log: &log,
        };
        let mut action = None;
        egui::Window::new("Blind Comparison")
            .collapsible(false)
            // One size for the whole test: the answer row changes from two
            // buttons to one, and a window that followed its content would
            // jump under the pointer every time a guess is made.
            .fixed_size(ABX_WINDOW_SIZE)
            .pivot(Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .show(ctx, |ui| {
                action = abx_body(ui, &view);
            });
        match action {
            Some(AbxAction::Play(choice)) => self.abx_play(choice),
            Some(AbxAction::Answer(side)) => self.abx_answer(side),
            Some(AbxAction::Next) => self.abx_next(),
            Some(AbxAction::Restart) => self.abx_restart(),
            Some(AbxAction::Close) => self.close_abx(),
            Some(AbxAction::Seek(position)) => self.abx_seek(position),
            Some(AbxAction::Start(start)) => self.abx_set_start(start),
            None => {}
        }
    }

    /// The playing album's cover, then, in the popup, the line naming the
    /// track, then the control strip.
    ///
    /// It sits above the album list rather than beside the waveform, so the
    /// waveform can use the whole width of the window.
    fn now_playing(&mut self, ui: &mut egui::Ui, compact: bool) {
        // A track with no art gets no square at all: an empty box the size of
        // a cover is the largest thing in the window and says nothing. What
        // the player draws then is the name, the controls and the waveform.
        // The preview's window is a fixed size chosen when it opened, so it
        // is `preview_art` that decides, not whatever texture arrives now: a
        // cover that failed to load would otherwise collapse the layout inside
        // a window that cannot shrink to match. A picture that is still being
        // decoded keeps its square, so the layout does not jump when it lands.
        if !self.playing_has_art() || (self.preview.is_some() && !self.preview_art) {
            self.name_and_controls(ui, compact);
            return;
        }

        // The art leaves room for the margin around it and stops short of
        // the controls, which sit below rather than over the cover.
        let art_room = (ui.available_width() - 2.0 * ART_MARGIN).max(1.0);
        // In the popup the window can be short as well as narrow, and the
        // art is square: it takes whichever of the two limits it hits first.
        // The name line and the controls sit below it, so their height comes
        // off the room the art may take.
        let side = if compact {
            art_room.min(
                (ui.available_height() - CONTROL_STRIP - COMPACT_NAME_HEIGHT - 2.0 * ART_MARGIN)
                    .max(1.0),
            )
        } else {
            art_room
        };
        // The cover is decoded at the size this square is drawn, so the size
        // has to be known before the texture is asked for: a thumbnail
        // stretched over a square this wide reads blurry.
        let texture = self.playing_art(ui.ctx(), side);
        // A popup can be wider than it is tall, and the art is square, so it
        // is centred in the room it does not fill.
        let inset = if compact {
            ((ui.available_width() - side) / 2.0).max(ART_MARGIN)
        } else {
            ART_MARGIN
        };
        ui.horizontal(|ui| {
            ui.add_space(inset);
            let (rect, response) = ui.allocate_exact_size(vec2(side, side), Sense::click());
            match &texture {
                Some(texture) if self.cover_fit => {
                    // The whole picture, with the space it does not fill
                    // left as background: art of any shape is shown as it
                    // was drawn, rather than cropped to a square.
                    ui.painter()
                        .rect_filled(rect, 10, ui.visuals().faint_bg_color);
                    let inner = fit_rect(rect, texture.size_vec2());
                    egui::Image::new(texture)
                        .fit_to_exact_size(inner.size())
                        .corner_radius(10)
                        .paint_at(ui, inner);
                }
                _ => cover(ui, rect, texture.as_ref(), 10),
            }
            response.context_menu(|ui| self.art_menu(ui));
        });

        // The popup keeps the name line close to the art and the controls:
        // the gaps around it are the only slack in a window this small.
        ui.add_space(if compact { 2.0 } else { 6.0 });

        self.name_and_controls(ui, compact);
    }

    /// What a right click on the art offers.
    ///
    /// The preview has no menu bar, so this is also the way out of it.
    fn art_menu(&mut self, ui: &mut egui::Ui) {
        if ui
            .selectable_label(!self.cover_fit, "Fill the space")
            .on_hover_text("Crops art that is not square, so no background shows")
            .clicked()
        {
            self.set_cover_fit(false);
            ui.close();
        }
        if ui
            .selectable_label(self.cover_fit, "Fit the whole image")
            .on_hover_text("Shows all of the art, with the space beside it as background")
            .clicked()
        {
            self.set_cover_fit(true);
            ui.close();
        }
        if self.preview.is_some() {
            ui.separator();
            if ui.button("Open the Window").clicked() {
                self.leave_preview(ui.ctx());
                ui.close();
            }
        }
    }

    /// The line naming the track, then the control strip.
    ///
    /// The popup has no table to read the track from, so it names the track
    /// between the art and the controls. A track with no art is drawn without
    /// a square above it, and the menu a right click on the art gives is
    /// offered on the name line instead, since that is what is left.
    fn name_and_controls(&mut self, ui: &mut egui::Ui, compact: bool) {
        if compact {
            let response = self.compact_name(ui);
            if !self.playing_has_art() {
                response.context_menu(|ui| self.art_menu(ui));
            }
        }

        // The controls get a strip of their own, the width of the art.
        let width = ui.available_width().max(1.0);
        let (strip, _) = ui.allocate_exact_size(vec2(width, CONTROL_STRIP), Sense::hover());
        // The strip takes the theme's raised colour, so it follows a light
        // theme instead of staying dark under light buttons.
        ui.painter()
            .rect_filled(strip, 8, ui.visuals().widgets.noninteractive.bg_fill);
        let mut controls = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(strip.shrink2(vec2(8.0, 0.0)))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        self.controls(&mut controls, self.preview.is_none());
    }

    /// The playing track's name and its position, in the popup's line
    /// between the art and the controls.
    ///
    /// The name is the title, or the file name when the table is set to show
    /// file names instead, so the popup says the same thing the table does.
    /// The position follows it in the table's own form, `- 0:21/2:14`. The
    /// line keeps its height with nothing playing, so starting a track does
    /// not resize the popup.
    fn compact_name(&mut self, ui: &mut egui::Ui) -> egui::Response {
        let width = ui.available_width().max(1.0);
        let (rect, response) =
            ui.allocate_exact_size(vec2(width, COMPACT_NAME_HEIGHT), Sense::click());
        let Some(name) = self.playing_name() else {
            return response;
        };
        let position = self.position();
        let suffix = (!self.length.is_zero()).then(|| progress_text(position, self.length));

        let font = FontId::proportional(14.0);
        let accent = self.accent();
        let room = (rect.width() - 2.0 * ART_MARGIN).max(10.0);
        let suffix = suffix.map(|suffix| elided(ui, &suffix, &font, accent, room));
        let reserved = suffix.as_ref().map_or(0.0, |galley| galley.size().x);
        let name = elided(ui, &name, &font, accent, (room - reserved).max(10.0));

        // The name and the position are centred together, so the line reads
        // as one run rather than as a name and a stray number. Both take the
        // accent, as they do in the table's playing row.
        let total = name.size().x + reserved;
        let left = rect.center().x - total / 2.0;
        let top = rect.center().y - name.size().y / 2.0;
        let painter = ui.painter().with_clip_rect(rect);
        painter.galley(pos2(left, top), name.clone(), accent);
        if let Some(suffix) = suffix {
            painter.galley(pos2(left + name.size().x, top), suffix, accent);
        }
        response
    }

    /// The transport, the two playback modes and the volume, in reading
    /// order.
    ///
    /// The mode buttons take their room from the volume slider, which fills
    /// whatever is left of the strip. `whole` is off in the preview, where
    /// there is no playlist to step through and no order to walk: only
    /// repeat, stop and play are drawn, and the volume takes the room the
    /// two missing buttons leave.
    fn controls(&mut self, ui: &mut egui::Ui, whole: bool) {
        // Tighter than the style's default, so the buttons and the slider
        // both fit the width the album art sets.
        ui.spacing_mut().item_spacing.x = 4.0;

        let accent = self.accent();
        let repeat = toggle(ui, Icon::Repeat, self.repeat, accent);
        if repeat.clicked() {
            self.set_repeat(!self.repeat);
        }
        repeat.on_hover_text("Repeat Track");

        let playing = self.playing.is_some();
        if whole && transport(ui, Icon::Previous, playing).clicked() {
            self.step(false);
        }
        if transport(ui, Icon::Stop, playing).clicked() {
            self.stop();
        }
        let icon = if self.paused() {
            Icon::Play
        } else {
            Icon::Pause
        };
        if transport(ui, icon, true).clicked() {
            self.toggle();
        }
        if whole && transport(ui, Icon::Next, playing).clicked() {
            self.step(true);
        }

        if whole {
            let random = toggle(ui, Icon::Random, self.random, accent);
            if random.clicked() {
                self.set_random(!self.random);
            }
            random.on_hover_text("Random Order");
        }

        self.volume_slider(ui);
    }

    /// The volume slider, filling the space left in the strip.
    ///
    /// The wheel over it adjusts the volume by a notch, and the setting is
    /// written when the slider is let go.
    fn volume_slider(&mut self, ui: &mut egui::Ui) {
        // The slider takes what the buttons left of it did not, so the strip
        // is filled from end to end whatever the panel width is.
        ui.spacing_mut().slider_width = ui.available_width().max(24.0);
        // The slider edits a copy, so the device is only ever given the
        // volume through `set_volume`, which applies it and clamps it.
        let mut volume = self.volume;
        let response = ui.add(
            egui::Slider::new(&mut volume, 0.0..=1.0)
                .show_value(false)
                .trailing_fill(true),
        );
        let mut changed = response.changed();

        if response.hovered() {
            let steps = wheel_notches(ui.ctx());
            if steps != 0.0 {
                volume = self.volume + steps * VOLUME_PER_NOTCH;
                changed = true;
            }
        }

        if changed {
            self.set_volume(volume);
        }
        if response.drag_stopped() || response.lost_focus() {
            self.save_config();
        }
    }

    /// Switches the table to another playlist.
    ///
    /// What the old playlist left queued or shuffled no longer fits, and
    /// the selection moves to the new playlist's first track.
    fn switch_playlist(&mut self) {
        self.queue = None;
        self.forget_shuffle();
        // The tab that was showing had its own search; a new one starts whole.
        self.close_search();
        self.select_first();
        self.save_playlists();
    }

    /// The playlist tabs, above the table.
    ///
    /// A single click shows a playlist, a double click renames it, and the
    /// cross closes it. The last playlist has no cross: the window always
    /// shows one.
    fn playlist_tabs(&mut self, ui: &mut egui::Ui) {
        let mut show = None;
        let mut close = None;
        let mut rename = None;
        let mut add = false;

        ui.horizontal(|ui| {
            for (index, playlist) in self.playlists.items.iter().enumerate() {
                let active = self.playlists.is_active(index);
                let editing = self.view.renaming == Some(index);

                if editing {
                    // The box takes the keyboard on the frame it appears,
                    // and commits on Enter or when it loses focus.
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut self.view.rename_buffer)
                            .desired_width(120.0)
                            .font(egui::TextStyle::Button),
                    );
                    if !response.has_focus() && !response.lost_focus() {
                        response.request_focus();
                    }
                    // Enter arrives as a key the text box consumed, so it is
                    // read from the box's own events rather than the frame's.
                    let entered = response
                        .ctx
                        .input(|input| input.key_pressed(egui::Key::Enter));
                    if entered || response.lost_focus() {
                        rename = Some(index);
                    }
                    continue;
                }

                let label = format!("{}  ({})", playlist.name, playlist.tracks.len());
                let response = ui.selectable_label(active, label);
                if response.clicked() {
                    show = Some(index);
                }
                if response.double_clicked() {
                    self.view.rename_buffer = playlist.name.clone();
                    self.view.renaming = Some(index);
                }
                if response.secondary_clicked() {
                    self.view.rename_buffer = playlist.name.clone();
                    self.view.renaming = Some(index);
                }

                if self.playlists.items.len() > 1
                    && ui
                        .small_button(
                            egui::RichText::new("x").color(ui.visuals().weak_text_color()),
                        )
                        .clicked()
                {
                    close = Some(index);
                }
                ui.add_space(2.0);
            }

            if ui.small_button("+").on_hover_text("New playlist").clicked() {
                add = true;
            }
        });

        if let Some(index) = show {
            self.playlists.active = index;
            self.switch_playlist();
        }
        if let Some(index) = rename {
            let name = self.view.rename_buffer.clone();
            self.playlists.active = index;
            self.rename_playlist(&name);
            self.view.renaming = None;
            // The tab changes shape when the box closes, and nothing else
            // in the frame would ask for that repaint.
            ui.ctx().request_repaint();
        }
        if let Some(index) = close {
            self.playlists.close(index);
            self.switch_playlist();
        }
        if add {
            self.playlists.add();
            self.view.renaming = None;
            self.switch_playlist();
        }
    }

    /// The search box, drawn while a search is open.
    ///
    /// The box takes the keyboard when it opens, filters the table as the
    /// text is typed, and walks the matches on Enter: the next one, or the
    /// previous one with Shift. Escape closes it, which shows the whole
    /// playlist again.
    fn search_bar(&mut self, ui: &mut egui::Ui) {
        if !self.view.searching {
            return;
        }
        let mut step = None;
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Search").color(ui.visuals().weak_text_color()));
            let response = ui.add(
                egui::TextEdit::singleline(&mut self.view.filter)
                    .desired_width(ui.available_width())
                    .hint_text("title, artist or album"),
            );
            if std::mem::take(&mut self.view.search_focus) {
                response.request_focus();
            }
            if response.changed() {
                self.refilter();
            }
            // Enter is read from the box's own events: a text box gives the
            // keyboard up on it, so the frame's input no longer carries it.
            if response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                let shift = ui.input(|input| input.modifiers.shift);
                step = Some(!shift);
                // The box dropped the focus on Enter, and the next press has
                // to land in it again.
                response.request_focus();
            }
        });
        if let Some(forward) = step {
            self.step_match(forward);
        }
        ui.add_space(2.0);
    }

    /// The width the track table is drawn at, with room for the scroll bar.
    ///
    /// It takes the panel's width, and only grows past it, to the narrowest
    /// the columns can be squeezed to, once they no longer fit.
    fn table_width(&self, ui: &egui::Ui) -> f32 {
        let bar = ui.spacing().scroll.bar_width + ui.spacing().scroll.bar_inner_margin;
        (ui.available_width() - bar).max(table_needed(self.columns, self.columns_shown))
    }

    /// The track table of the selected album.
    pub(crate) fn track_table(&mut self, ui: &mut egui::Ui) {
        egui::CentralPanel::default().show(ui, |ui| {
            self.playlist_tabs(ui);
            self.search_bar(ui);
            ui.add_space(2.0);

            let tracks = self.visible_tracks();
            if tracks.is_empty() {
                ui.centered_and_justified(|ui| {
                    let hint = if self.library.is_empty() {
                        "Open a music directory from the File menu"
                    } else if !self.view.filter.trim().is_empty() {
                        "Nothing matches that search"
                    } else {
                        "Double-click an album to make it the playlist"
                    };
                    ui.label(egui::RichText::new(hint).color(ui.visuals().weak_text_color()));
                });
                // Nothing to scroll to, and a request left pending would ask
                // for a repaint on every frame.
                self.view.reveal = false;
                return;
            }

            let width = self.table_width(ui);
            let mut play = None;
            // The track to bring into view comes from the selection rather
            // than from the rows drawn, since it is usually off screen.
            let reveal = row_to_reveal(self.view.reveal, self.view.selected, &tracks);
            let albums = self.library.album_indices();
            let rows = table_rows(&tracks, &albums);

            // The titles get a strip of their own, above the rows, so they
            // stay in reach while a long playlist is scrolled: a column is
            // resized from them, and hunting for them at the top is work.
            let header = Rect::from_min_size(ui.cursor().min, vec2(width, HEADER_HEIGHT));
            ui.add_space(HEADER_HEIGHT);

            // The rows are clipped to below the strip, so a row that is
            // partly scrolled past it cannot paint over the titles.
            let clip = ui.clip_rect();
            ui.set_clip_rect(Rect::from_min_max(
                pos2(clip.min.x, header.bottom()),
                clip.max,
            ));

            // Both directions: the rows scroll up and down, and the table
            // scrolls sideways once the panel is narrower than the columns
            // need.
            let output = egui::ScrollArea::both()
                .auto_shrink([false, false])
                .show_viewport(ui, |ui, viewport| {
                    let tops = row_tops(&rows);
                    let total = rows_height(&rows);
                    ui.set_min_size(vec2(width, total));
                    let origin = ui.max_rect().min;
                    let (widths, _) = column_layout(width, self.columns, self.columns_shown);

                    // Only the rows the viewport shows are drawn, so a
                    // playlist of thousands costs no more than a short one.
                    for row in visible_rows(&tops, viewport) {
                        let rect = row_rect(origin, tops[row], width, rows[row].height());
                        let index = match rows[row] {
                            TableRow::Album(album) => {
                                let texture = self.album_cover(ui.ctx(), album);
                                if let Some(entry) = self.library.albums.get(album) {
                                    album_heading(ui, rect, entry, texture.as_ref());
                                }
                                continue;
                            }
                            TableRow::Track(index) => index,
                        };
                        play = self.track_entry(ui, row, index, rect, widths).or(play);
                    }

                    // Scrolling has to be asked for on the scroll area's own
                    // ui, which is the one whose coordinates the rows are in.
                    if let Some(track) = reveal
                        && let Some(row) = rows.iter().position(
                            |entry| matches!(entry, TableRow::Track(index) if *index == track),
                        )
                    {
                        let rect = row_rect(origin, tops[row], width, rows[row].height());
                        ui.scroll_to_rect(rect, None);
                    }
                });

            ui.set_clip_rect(clip);
            // Drawn after the rows, so it knows how far they have scrolled
            // sideways and can follow them: a label has to stay over the
            // values it names.
            self.column_header(ui, header, output.state.clamped_offset().x);
            self.view.reveal = false;

            if let Some(index) = play {
                // Playing a row by hand goes back to walking the playlist,
                // rather than the queue an earlier selection started.
                self.queue = None;
                self.forget_shuffle();
                self.play(index);
            }
        });
    }

    /// Draws one track's row and turns its clicks into selection and playback.
    ///
    /// Returns the track a double-click asked to play, for the caller to
    /// start once the whole table is drawn.
    fn track_entry(
        &mut self,
        ui: &mut egui::Ui,
        row: usize,
        index: usize,
        rect: Rect,
        widths: [f32; 6],
    ) -> Option<usize> {
        let selected = self.is_selected(index);
        let playing = self.playing == Some(index);
        // The row's own borrow ends with this block, so the clicks below can
        // change the selection.
        let response = {
            let track = self.library.tracks.get(index)?;
            track_row(
                ui,
                Row {
                    track,
                    rect,
                    widths,
                    shown: self.columns_shown,
                    extension: self.file_extension,
                    row,
                    selected,
                    playing,
                    progress: playing.then(|| self.position()),
                },
            )
        };
        if response.clicked() {
            let (shift, command) =
                ui.input(|input| (input.modifiers.shift, input.modifiers.command));
            if shift {
                self.extend_selection(index);
            } else if command {
                self.toggle_selection(index);
            } else {
                self.select_only(index);
            }
            self.save_playlists();
        }
        let play = response.double_clicked().then_some(index);

        // A right click acts on the whole selection, so one that lands outside
        // it selects the row it hit first.
        if response.secondary_clicked() && !self.is_selected(index) {
            self.select_only(index);
        }
        response.context_menu(|ui| {
            let count = self.view.selection.len();
            let play_label = match count {
                0 | 1 => "Play".to_string(),
                count => format!("Play {count} Tracks"),
            };
            let remove_label = match count {
                0 | 1 => "Remove from Playlist".to_string(),
                count => format!("Remove {count} Tracks from Playlist"),
            };
            let selected = !self.view.selection.is_empty();
            if ui
                .add_enabled(selected, egui::Button::new(play_label))
                .on_hover_text("Plays these tracks, in this order")
                .clicked()
            {
                ui.close();
                self.play_selected();
            }
            if ui
                .add_enabled(selected, egui::Button::new(remove_label))
                .on_hover_text("Leaves the files where they are")
                .clicked()
            {
                ui.close();
                self.remove_selected();
            }
            ui.separator();
            // Two rows are what a comparison needs: the first is A and the
            // second B.
            let pair = self.selected_tracks().len() == 2;
            if ui
                .add_enabled(pair, egui::Button::new("Blind Comparison"))
                .on_hover_text("Compares the two selected tracks without saying which is which")
                .clicked()
            {
                ui.close();
                self.start_abx();
            }
            ui.separator();
            if ui.button("Select All").clicked() {
                ui.close();
                self.select_all();
            }
        });
        play
    }

    /// The column titles above the track table, drawn in `strip`.
    ///
    /// The strip does not scroll with the rows, but the labels follow them
    /// sideways by `scrolled` points, so each stays over its values. The
    /// borders between two columns can be dragged, so a long album name can be
    /// given the room a short title does not need. The pointer turns into a
    /// resize arrow while it is over one. A right click opens the menu that
    /// turns columns on and off.
    fn column_header(&mut self, ui: &mut egui::Ui, strip: Rect, scrolled: f32) {
        // The strip is filled first, since a row scrolling up past it would
        // otherwise be drawn through the labels.
        ui.painter().rect_filled(strip, 0, ui.visuals().panel_fill);
        // Everything below is clipped to the strip, so a label scrolled
        // sideways does not spill over the row beside it.
        let clip = ui.clip_rect();
        ui.set_clip_rect(strip.intersect(clip));

        let rect = strip.translate(vec2(-scrolled, 0.0));
        // The menu belongs to the whole strip, and is taken before the
        // borders so a right click near one still opens it.
        let strip_response = ui.interact(rect, ui.scope_id().with("column-header"), Sense::click());

        // The borders are handled before anything is painted, so a drag shows
        // up in the labels of the same frame as in the rows below them.
        let mut hovered = None;
        let (before, _) = column_layout(rect.width(), self.columns, self.columns_shown);
        let mut edge = rect.left() + ROW_PADDING;
        // Each border is between a column and the one after it. A border with
        // a hidden column on either side is not drawn, and cannot be dragged.
        for index in 0..Column::ALL.len() - 1 {
            let column = Column::ALL[index];
            let left = before[index];
            edge += left;
            let next = Column::ALL[index + 1];
            if !(self.columns_shown[index] && self.columns_shown[next.index()]) {
                continue;
            }
            let handle = Rect::from_min_max(
                pos2(edge - GRAB, rect.top()),
                pos2(edge + GRAB, rect.bottom()),
            );
            let response = ui.interact(
                handle,
                ui.scope_id().with(("column", index)),
                Sense::click_and_drag(),
            );
            if response.hovered() || response.dragged() {
                hovered = Some(handle);
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            }
            if response.dragged()
                && let Some(pointer) = response.interact_pointer_pos()
            {
                // The border moves width between the two columns it
                // separates, so their total stays the same and the border
                // stays under the pointer however the table is scaled. The
                // ratio comes from the column on the left, since a column of
                // digits keeps its stored width while a text column is
                // scaled.
                let ratio = (left / self.columns[index].max(1.0)).max(0.05);
                let moved = (pointer.x - edge) / ratio;
                let mut columns = self.columns;
                let right = columns[next.index()];
                let pair = (left / ratio + moved)
                    .clamp(column.min_width(), left / ratio + right - next.min_width());
                columns[index] = pair;
                columns[next.index()] = left / ratio + right - pair;
                self.set_columns(columns);
            }
        }

        // A drag above may have changed the stored widths, so the labels are
        // laid out from them again.
        let (widths, _) = column_layout(rect.width(), self.columns, self.columns_shown);
        let starts = column_starts(widths);
        let font = FontId::proportional(11.5);
        let color = ui.visuals().weak_text_color();
        for column in Column::ALL {
            if !self.columns_shown[column.index()] {
                continue;
            }
            let cell = column_cell(rect, starts, widths, column);
            match column {
                Column::Length => paint_text_right(ui, cell, column.label(), &font, color),
                _ => paint_text(ui, cell, column.label(), &font, color),
            }
        }
        ui.painter().line_segment(
            [rect.left_bottom(), rect.right_bottom()],
            Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color),
        );
        if let Some(handle) = hovered {
            ui.painter().line_segment(
                [handle.center_top(), handle.center_bottom()],
                Stroke::new(1.0, ui.visuals().selection.stroke.color),
            );
        }

        strip_response.context_menu(|ui| self.column_menu(ui));
        ui.set_clip_rect(clip);
    }

    /// The menu a right click on the column titles opens.
    ///
    /// Every column can be turned off, its room going to the ones left; the
    /// file name column can drop the extension, so the names read like
    /// titles; and the widths can be put back as they started.
    fn column_menu(&mut self, ui: &mut egui::Ui) {
        for column in Column::ALL {
            let index = column.index();
            self.setting_checkbox(
                ui,
                format!("Show {}", column.label()),
                self.columns_shown[index],
                |app, shown| app.set_column_shown(index, shown),
            );
        }
        ui.separator();
        self.setting_checkbox(
            ui,
            "Show File Extension",
            self.file_extension,
            Self::set_file_extension,
        )
        .on_hover_text("Shows the .flac, .mp3 and other extensions in the file name column");
        ui.separator();
        if ui.button("Reset Column Widths").clicked() {
            self.set_columns(crate::app::default_columns());
        }
    }

    /// The compact layout: the art and its controls, under the menu bar.
    ///
    /// Nothing else is drawn, so the window works as a small popup beside
    /// whatever else is on screen. The art is square and takes the width it
    /// is given, or less when the window is short.
    pub(crate) fn compact_body(&mut self, ui: &mut egui::Ui) {
        ui.add_space(ART_MARGIN);
        self.now_playing(ui, true);
    }

    /// The preview: the art, its controls and the waveform, and nothing else.
    ///
    /// It is the popup without a menu bar, used when one audio file was named
    /// on the command line. There is no playlist behind it, so there is no
    /// Previous, no Next and no Random Order: the strip carries repeat, stop
    /// and play, and the waveform below seeks within the one track.
    pub(crate) fn preview_body(&mut self, ui: &mut egui::Ui) {
        ui.add_space(ART_MARGIN);
        self.now_playing(ui, true);
    }

    /// The album list on the right: covers and titles.
    pub(crate) fn album_list(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        self.now_playing(ui, false);
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("Library")
                    .strong()
                    .color(ui.visuals().weak_text_color()),
            );
            let add_all = ui
                .add_enabled(
                    !self.library.tracks.is_empty(),
                    egui::Button::new("Add All").small(),
                )
                .on_hover_text("Appends every track in the library to this playlist");
            if add_all.clicked() {
                self.add_all();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(self.library.albums.len().to_string())
                        .color(ui.visuals().weak_text_color()),
                );
            });
        });
        ui.add_space(4.0);

        let rows = self.library_rows();
        let mut actions = Vec::new();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_viewport(ui, |ui, viewport| {
                // Only the rows on screen are drawn: laying out and painting
                // every album made a frame cost more the larger the library
                // was.
                let tops = library_row_tops(&rows);
                let total = library_rows_height(&rows);
                let width = ui.available_width();
                ui.set_min_size(vec2(width, total));
                let origin = ui.max_rect().min;

                for index in visible_rows(&tops, viewport) {
                    let rect = row_rect(origin, tops[index], width, rows[index].height());
                    let mut row = ui.new_child(egui::UiBuilder::new().max_rect(rect));
                    self.library_row(&mut row, &rows[index], &mut actions);
                }
            });
        for action in actions {
            match action {
                RowAction::OpenAlbum(index) => self.open_album(index),
                RowAction::AddDirectory(path) => self.add_directory_at(&path),
                RowAction::Toggle(path) => self.toggle_collapsed_at(&path),
                RowAction::PlayLoose(index) => self.play_loose(index),
            }
        }
    }

    /// Every row the album list shows, in drawing order.
    ///
    /// The tree is flattened so the list can draw only the rows on screen:
    /// the rows are built once a frame from indices, and the painting, which
    /// is the part that costs, happens for the few that are visible.
    ///
    /// The music directory's own albums and loose files head the list, with
    /// no row above them: they are what sits beside the directories, not
    /// inside one, and a row naming the directory the user picked would only
    /// indent everything below it.
    fn library_rows(&self) -> Vec<LibraryRow> {
        let mut rows = Vec::new();
        let root = &self.library.root;
        for &album in &root.albums {
            rows.push(LibraryRow::Album { album, depth: 0 });
        }
        for &track in &root.loose {
            rows.push(LibraryRow::Loose { track, depth: 0 });
        }
        for directory in self.library.directories() {
            self.push_directory_rows(&mut rows, directory, 0);
        }
        rows
    }

    /// Adds `directory` and everything under it to `rows`.
    fn push_directory_rows(
        &self,
        rows: &mut Vec<LibraryRow>,
        directory: &crate::library::Directory,
        depth: usize,
    ) {
        if directory.is_empty() {
            return;
        }

        // A directory that only leads to one album is not worth a row of its
        // own: the album is shown where the directory would be, at the same
        // depth, so a release does not sit under a directory of the same name.
        // The album may sit in a subdirectory such as `Disc 1`, so the chain is
        // followed down to it.
        if directory.is_pass_through() {
            for &album in &directory.albums {
                rows.push(LibraryRow::Album { album, depth });
            }
            for child in &directory.children {
                if child.is_pass_through() {
                    self.push_directory_rows(rows, child, depth);
                }
            }
            return;
        }

        let collapsed = self.is_collapsed(directory);
        rows.push(LibraryRow::Directory {
            path: directory.path.clone(),
            name: directory.name.clone(),
            depth,
            collapsed,
        });
        if collapsed {
            return;
        }
        for &album in &directory.albums {
            rows.push(LibraryRow::Album {
                album,
                depth: depth + 1,
            });
        }
        for &track in &directory.loose {
            rows.push(LibraryRow::Loose {
                track,
                depth: depth + 1,
            });
        }
        for child in &directory.children {
            self.push_directory_rows(rows, child, depth + 1);
        }
    }

    /// Draws one row of the album list.
    fn library_row(&mut self, ui: &mut egui::Ui, row: &LibraryRow, actions: &mut Vec<RowAction>) {
        match row {
            LibraryRow::Directory {
                path,
                name,
                depth,
                collapsed,
            } => self.directory_entry(ui, path, name, *depth, *collapsed, actions),
            LibraryRow::Album { album, depth } => self.album_entry(ui, *album, *depth, actions),
            LibraryRow::Loose { track, depth } => {
                let Some(track) = self.library.tracks.get(*track) else {
                    return;
                };
                let indent = 2.0 + *depth as f32 * TREE_STEP + ALBUM_INDENT;
                let selected = self.view.selected == Some(row.track_index());
                if loose_row(ui, track, selected, indent).double_clicked() {
                    actions.push(RowAction::PlayLoose(row.track_index()));
                }
            }
        }
    }

    /// One directory row: the arrow that folds it, its icon and its name.
    fn directory_entry(
        &mut self,
        ui: &mut egui::Ui,
        path: &std::path::Path,
        name: &str,
        depth: usize,
        collapsed: bool,
        actions: &mut Vec<RowAction>,
    ) {
        let indent = 2.0 + depth as f32 * TREE_STEP;
        let (rect, response) = ui.allocate_exact_size(
            vec2(ui.available_width(), DIRECTORY_ROW_HEIGHT),
            Sense::click(),
        );
        if response.hovered() {
            ui.painter()
                .rect_filled(rect, 4, ui.visuals().widgets.hovered.weak_bg_fill);
        }

        // The arrow folds the directory; the rest of the row is for picking it.
        // They are separate targets so a double-click on the row to queue
        // the directory never also folds it.
        let arrow = Rect::from_min_size(
            pos2(rect.left() + indent, rect.top()),
            vec2(16.0, rect.height()),
        );
        let arrow_response = ui.interact(
            arrow,
            ui.scope_id().with(("directory-arrow", path)),
            Sense::click(),
        );
        paint_arrow(
            ui,
            arrow,
            collapsed,
            if arrow_response.hovered() {
                ui.visuals().text_color()
            } else {
                ui.visuals().weak_text_color()
            },
        );
        paint_directory_icon(
            ui,
            Rect::from_min_size(
                pos2(arrow.right() + 4.0, rect.top()),
                vec2(16.0, rect.height()),
            ),
            ui.visuals().weak_text_color(),
        );

        let text = Rect::from_min_max(
            pos2(arrow.right() + 24.0, rect.top()),
            pos2(rect.right() - 4.0, rect.bottom()),
        );
        paint_text(
            ui,
            text,
            name,
            &FontId::proportional(13.0),
            ui.visuals().text_color(),
        );

        if arrow_response.clicked() {
            actions.push(RowAction::Toggle(path.to_path_buf()));
        }
        if response.double_clicked() {
            actions.push(RowAction::AddDirectory(path.to_path_buf()));
        }
    }

    /// One album row, indented to `depth`.
    fn album_entry(
        &mut self,
        ui: &mut egui::Ui,
        index: usize,
        depth: usize,
        actions: &mut Vec<RowAction>,
    ) {
        let indent = 2.0 + depth as f32 * TREE_STEP + ALBUM_INDENT;
        let texture = self.album_cover(ui.ctx(), index);
        let Some(album) = self.library.albums.get(index) else {
            return;
        };
        let selected = self.view.album == Some(index);
        let row = album_row(ui, album, texture.as_ref(), selected, indent);
        if row.clicked() {
            self.view.album = Some(index);
        }
        if row.double_clicked() {
            actions.push(RowAction::OpenAlbum(index));
        }
    }

    /// The waveform strip at the bottom of the window, which seeks.
    pub(crate) fn player_bar(&mut self, ui: &mut egui::Ui) {
        // The waveform sits in the middle of the strip, so the gap above it
        // matches the gap below.
        ui.add_space(wave_gap(ui.available_height(), self.wave_height()));
        self.waveform_bar(ui);
    }

    /// Bar heights for the waveform, one per point of `width`.
    ///
    /// Each is the loudest peak its column covers, scaled to the bar. They
    /// are cached: the drawing pass runs many times a second, and rebuilding
    /// a few hundred of these every frame is work the track does not change.
    fn column_heights(&mut self, width: f32, half: f32) -> &[f32] {
        let columns = width.max(1.0) as usize;
        // The bar's height is part of the key: the popup's bar is shorter
        // than the window's, and heights worked out for one are wrong for
        // the other.
        let half_bits = half.to_bits();
        let fresh = matches!(
            &self.wave_columns,
            Some((cached_columns, cached_half, cached_len, _))
                if *cached_columns == columns
                    && *cached_half == half_bits
                    && *cached_len == self.peaks.len()
        );
        if !fresh {
            let peaks = &self.peaks;
            let mut heights = Vec::with_capacity(columns);
            for column in 0..columns {
                let from = column * peaks.len() / columns;
                let to = ((column + 1) * peaks.len() / columns).max(from + 1);
                let peak = peaks[from..to.min(peaks.len())]
                    .iter()
                    .copied()
                    .fold(0.0_f32, f32::max);
                heights.push((peak * half).max(0.5));
            }
            self.wave_columns = Some((columns, half_bits, peaks.len(), heights));
        }
        &self.wave_columns.as_ref().expect("just filled").3
    }

    /// Height of the waveform bar in the current layout.
    fn wave_height(&self) -> f32 {
        if self.compact {
            COMPACT_WAVE_HEIGHT
        } else {
            WAVE_HEIGHT
        }
    }

    /// The waveform seek bar, drawn from the peaks of the playing track.
    pub(crate) fn waveform_bar(&mut self, ui: &mut egui::Ui) {
        let height = self.wave_height();
        let width = ui.available_width().max(1.0);
        let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 6, ui.visuals().extreme_bg_color);

        let length = self.length.as_secs_f32();
        let position = self.position().as_secs_f32();
        let played = if length > 0.0 {
            (position / length).clamp(0.0, 1.0)
        } else {
            0.0
        };

        if self.peaks.is_empty() {
            // A thin bar has no room for a sentence, and the popup is small
            // enough that one would be most of it.
            if self.compact {
                return;
            }
            let hint = if self.waveform.is_some() {
                "Reading waveform..."
            } else if self.playing.is_some() {
                "No waveform for this track"
            } else {
                "Double-click a track to play it"
            };
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                hint,
                FontId::proportional(12.0),
                ui.visuals().weak_text_color(),
            );
        } else {
            let accent = self.accent();
            let idle = ui.visuals().widgets.inactive.bg_fill;
            let middle = rect.center().y;
            let played_x = rect.left() + rect.width() * played;
            // One column per point of width, each as tall as the loudest peak
            // it covers. Only the part already played is in the accent, so
            // the split moves with the cursor; the bars themselves change
            // only with the width, the bar's height or the track, which are
            // what `column_heights` keys its cache on.
            let heights = self.column_heights(rect.width(), rect.height() * 0.42);
            for (column, &height) in heights.iter().enumerate() {
                let x = rect.left() + column as f32;
                let color = if x <= played_x { accent } else { idle };
                painter.line_segment(
                    [pos2(x, middle - height), pos2(x, middle + height)],
                    Stroke::new(1.0, color),
                );
            }
            painter.line_segment(
                [pos2(played_x, rect.top()), pos2(played_x, rect.bottom())],
                Stroke::new(1.0, ui.visuals().text_color()),
            );
        }

        if response.dragged() {
            if let Some(pointer) = response.interact_pointer_pos() {
                let fraction = ((pointer.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
                self.seek = Some(self.length.mul_f32(fraction));
            }
        } else if response.drag_stopped() {
            if let Some(position) = self.seek.take() {
                self.seek_to_position(position);
            }
        } else if response.clicked()
            && let Some(pointer) = response.interact_pointer_pos()
        {
            let fraction = ((pointer.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
            self.seek_to(fraction);
        }
    }
}

/// What the comparison's window draws, gathered from the app and the test.
///
/// The window holds a borrow of the test while it is drawn, so the player's
/// own state is gathered here rather than read through the app underneath it.
struct AbxView<'a> {
    /// The test under way.
    test: &'a Abx,
    /// The sums of the two files, once they are read.
    sums: Option<&'a (String, String)>,
    /// Where the track under the bar is now.
    position: Duration,
    /// How long that track is, as the decoder reports it.
    length: Duration,
    /// The moment every choice is played from.
    start: Duration,
    /// The colour the bar and the chosen buttons are drawn in.
    accent: Color32,
    /// The signature over the log, once the test is over.
    signature: Option<&'a str>,
    /// The log the signature covers, with the signature block after it.
    log: &'a str,
}

/// Draws the body of the blind comparison and returns what a press asked for.
///
/// The two tracks are named, with the sum of each file beside them: two
/// encodes of one song carry the same tags, so the sums are what tells them
/// apart. Which of the two X is never appears until the answer is given,
/// which is what makes the test blind.
fn abx_body(ui: &mut egui::Ui, view: &AbxView) -> Option<AbxAction> {
    let test = view.test;
    let mut action = None;
    for (index, side) in [Side::A, Side::B].into_iter().enumerate() {
        let track = test.track(side);
        let sum = view
            .sums
            .map(|sums| if index == 0 { &sums.0 } else { &sums.1 });
        let name = format!("{} - {}", track.title, track.artist);
        let mut line = format!("{}: {}", side.label(), cut(&name, NAME_CHARS));
        if let Some(sum) = sum {
            line.push_str("  ");
            line.push_str(short_sum(sum));
        }
        let response = ui.add(egui::Label::new(line).truncate());
        if let Some(sum) = sum {
            response.on_hover_text(format!("sha256 {sum}"));
        }
    }
    ui.separator();

    // The trial number heads the trial, so it sits above the bar and the
    // moment the trial plays from, not down beside the answers.
    if !test.done() {
        centered_label(ui, &format!("Trial {} of {}", test.trial(), TRIALS));
    }

    // A plain bar rather than a waveform: a comparison wants a way back to
    // the same moment, not the shape of the track.
    let moved = ui
        .horizontal(|ui| {
            let clock = format!(
                "{} / {}",
                format::duration(view.position),
                format::duration(view.length)
            );
            let font = ui.style().text_styles[&egui::TextStyle::Body].clone();
            let room = (ui.available_width() - text_width(ui, &clock, font) - 16.0).max(60.0);
            let moved = abx_seek_bar(ui, view.position, view.length, room, view.accent);
            ui.label(clock);
            moved
        })
        .inner;
    if let Some(position) = moved {
        action = Some(AbxAction::Seek(position));
    }

    ui.horizontal(|ui| {
        ui.label("Start at");
        let longest = test
            .track(Side::A)
            .duration
            .max(test.track(Side::B).duration);
        let mut seconds = view.start.as_secs_f64();
        let value = egui::DragValue::new(&mut seconds)
            .speed(1.0)
            .range(0.0..=longest.as_secs_f64().max(1.0))
            .custom_formatter(|value, _| format::duration(Duration::from_secs_f64(value.max(0.0))))
            .custom_parser(parse_time);
        let response = ui.add(value).on_hover_text(
            "Both tracks start playing here, so every choice is heard from the same moment",
        );
        if response.changed() {
            action = Some(AbxAction::Start(Duration::from_secs_f64(seconds.max(0.0))));
        }
    });
    ui.separator();

    // The highlight says which one the player was last sent to, on the
    // button rather than in its text: the text stays the same so no button
    // moves under the pointer as the trial goes on.
    let plays = [Choice::A, Choice::B];
    let playing = test.playing();
    let last = playing.and_then(|choice| plays.iter().position(|play| *play == choice));
    if let Some(index) = centered_buttons(ui, &["Play A", "Play B"], last, 0.0).clicked {
        action = Some(AbxAction::Play(plays[index]));
    }
    let last = (playing == Some(Choice::X)).then_some(0);
    if centered_buttons(ui, &["Play X"], last, 0.0)
        .clicked
        .is_some()
    {
        action = Some(AbxAction::Play(Choice::X));
    }
    ui.separator();

    if test.done() {
        ui.label(format!(
            "{} of {} right, {} wrong",
            test.correct(),
            test.finished(),
            test.wrong()
        ));
        let chance = test.p_value() * 100.0;
        let verdict = if test.passed() {
            "You could tell them apart."
        } else {
            "No evidence you could tell them apart."
        };
        ui.label(format!(
            "{verdict} Guessing would have scored this well {chance:.1}% of the time."
        ));
        if let Some(signature) = view.signature {
            ui.add(egui::Label::new(
                egui::RichText::new(format!("Signature: {signature}"))
                    .monospace()
                    .size(8.0),
            ))
            .on_hover_text(
                "The SHA-256 of the log this result was signed with; Copy Log \
                 gives the text it covers",
            );
        }
    } else {
        // The question stays where it is whether the trial is answered or
        // not, so nothing moves while a guess is made.
        centered_label(ui, "Which one is X?");
        match test.answered() {
            None => {
                let sides = [Side::A, Side::B];
                // The answers are kept apart: a guess is the one press
                // that must not land on the wrong button.
                let gap = ANSWER_GAP;
                if let Some(index) = centered_buttons(ui, &["X is A", "X is B"], None, gap).clicked
                {
                    action = Some(AbxAction::Answer(sides[index]));
                }
            }
            // Whether the answer was right is said when the test ends and
            // not before: a verdict after every trial would tell the listener
            // which of the two X is on the next one.
            Some(_) => {
                if centered_buttons(ui, &["Next Trial"], None, 0.0)
                    .clicked
                    .is_some()
                {
                    action = Some(AbxAction::Next);
                }
            }
        }
    }

    let (footer, _) = abx_footer(ui, test.done(), view.log);
    if let Some(footer) = footer {
        action = Some(footer);
    }
    action
}

/// Draws the row that ends the comparison and returns what it asked for.
///
/// Close sits at the left edge rather than in the middle with the rest: the
/// middle is where the buttons pressed over and over are, and a Close under
/// the pointer there is one press away from ending a test by accident. Run
/// Again joins it once there is a test to run again.
fn abx_footer(ui: &mut egui::Ui, done: bool, log: &str) -> (Option<AbxAction>, Rect) {
    let mut action = None;
    let mut room: Option<Rect> = None;
    ui.horizontal(|ui| {
        let close = ui.button("Close");
        room = Some(close.rect);
        if close.clicked() {
            action = Some(AbxAction::Close);
        }
        if done {
            let again = ui.button("Run Again");
            room = room.map(|room| room.union(again.rect));
            if again.clicked() {
                action = Some(AbxAction::Restart);
            }
        }
        // The log is what the signature is over, so it has to be possible to
        // get it out of the window and hash it again somewhere else. There is
        // nothing to check until the test is over, so the button waits.
        if done {
            let copy = ui.button("Copy Log");
            room = room.map(|room| room.union(copy.rect));
            if copy
                .on_hover_text("The text the signature covers, ready to be checked")
                .clicked()
            {
                ui.ctx().copy_text(log.to_string());
            }
        }
    });
    (action, room.unwrap_or(Rect::NOTHING))
}

/// Draws `text` centred across the window, on a line of its own.
///
/// Returns the room the line took, which the tests check the centring on.
fn centered_label(ui: &mut egui::Ui, text: &str) -> Rect {
    let font = ui.style().text_styles[&egui::TextStyle::Body].clone();
    let room = text_width(ui, text, font);
    let shift = ((ui.available_width() - room) / 2.0).max(0.0);
    ui.horizontal(|ui| {
        ui.add_space(shift);
        ui.label(text).rect
    })
    .inner
}

/// Draws `labels` as one row and returns the index of the one clicked.
///
/// The row is centred across the window rather than left at its edge: egui
/// lays a row out from the left of whatever room it has, so the space the
/// buttons do not fill is added in front of them.
fn centered_buttons(
    ui: &mut egui::Ui,
    labels: &[&str],
    selected: Option<usize>,
    gap: f32,
) -> Centered {
    let widths: Vec<f32> = labels.iter().map(|label| button_width(ui, label)).collect();
    let spacing = ui.spacing().item_spacing.x + gap;
    let shift = centring_shift(&widths, spacing, ui.available_width());
    let mut clicked = None;
    let mut room: Option<Rect> = None;
    ui.horizontal(|ui| {
        ui.add_space(shift);
        for (index, label) in labels.iter().enumerate() {
            if index > 0 {
                ui.add_space(gap);
            }
            let button = egui::Button::new(*label).selected(selected == Some(index));
            let response = ui.add(button);
            room = Some(room.map_or(response.rect, |room| room.union(response.rect)));
            if response.clicked() {
                clicked = Some(index);
            }
        }
    });
    Centered {
        clicked,
        rect: room.unwrap_or(Rect::NOTHING),
    }
}

/// What a row of buttons did: the one pressed, and the room they took.
///
/// The room is what the row is centred on, so the tests check the centring
/// against the buttons themselves rather than against the row they sit in,
/// which egui stretches across the window.
struct Centered {
    /// The index of the button pressed, when one was.
    clicked: Option<usize>,
    /// The room the buttons took, the gaps between them included.
    ///
    /// Nothing draws from it: it is there so the tests can measure where the
    /// row landed, which is the only way to see that it is centred.
    #[cfg_attr(not(test), allow(dead_code))]
    rect: Rect,
}

/// Draws the seek bar and returns the moment it was dragged to, when it was.
///
/// It is a bar and nothing more: the comparison is about the two files, and a
/// waveform would only be something else to look at.
fn abx_seek_bar(
    ui: &mut egui::Ui,
    position: Duration,
    length: Duration,
    room: f32,
    accent: Color32,
) -> Option<Duration> {
    let (rect, response) = ui.allocate_exact_size(vec2(room, 10.0), Sense::click_and_drag());
    let fraction = if length.is_zero() {
        0.0
    } else {
        (position.as_secs_f32() / length.as_secs_f32()).clamp(0.0, 1.0)
    };
    let painter = ui.painter();
    painter.rect_filled(rect, 5, ui.visuals().widgets.inactive.bg_fill);
    let mut played = rect;
    played.set_right(rect.left() + rect.width() * fraction);
    painter.rect_filled(played, 5, accent);

    // A click and a drag both name a moment: a click jumps to it, a drag
    // follows the pointer as it goes.
    let pointer = response
        .interact_pointer_pos()
        .filter(|_| response.dragged() || response.clicked());
    pointer.map(|pointer| {
        let fraction = ((pointer.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
        length.mul_f32(fraction)
    })
}

/// Reads a moment typed as `m:ss`, `h:mm:ss` or plain seconds.
fn parse_time(text: &str) -> Option<f64> {
    let mut seconds = 0.0;
    for part in text.split(':') {
        seconds = seconds * 60.0 + part.trim().parse::<f64>().ok()?;
    }
    Some(seconds.max(0.0))
}

/// The width `text` takes in `font`, which a row sharing its room needs.
fn text_width(ui: &egui::Ui, text: &str, font: FontId) -> f32 {
    ui.ctx()
        .fonts_mut(|fonts| fonts.layout_no_wrap(text.to_owned(), font, Color32::PLACEHOLDER))
        .size()
        .x
}

/// How far in a row of `widths` starts, so that it is centred across `width`.
///
/// A row wider than the window starts at its edge rather than running off it
/// to the left, where it could not be read.
fn centring_shift(widths: &[f32], spacing: f32, width: f32) -> f32 {
    let total = widths.iter().sum::<f32>() + spacing * (widths.len() - 1) as f32;
    ((width - total) / 2.0).max(0.0)
}

/// The width a button carrying `text` takes, which centring a row needs.
///
/// It is the text plus the padding the button draws around it; egui has no
/// way to ask for a widget's size without drawing it, and the row has to be
/// placed before its buttons are.
fn button_width(ui: &egui::Ui, text: &str) -> f32 {
    let font = ui.style().text_styles[&egui::TextStyle::Button].clone();
    text_width(ui, text, font) + 2.0 * ui.spacing().button_padding.x
}

/// `text` cut to `limit` characters, with an ellipsis when it does not fit.
///
/// The window is one size for the whole test, so a name longer than the room
/// has to give way rather than wrap and push the buttons down.
fn cut(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let kept: String = text.chars().take(limit.saturating_sub(1)).collect();
    format!("{}...", kept.trim_end())
}

/// As much of a checksum as the comparison shows; the whole one is on hover.
fn short_sum(sum: &str) -> &str {
    &sum[..sum.len().min(SUM_CHARS)]
}

/// How many wheel notches the pointer moved this frame.
///
/// The raw events are read rather than `smooth_scroll_delta`, which decays
/// over several frames and would apply one notch more than once.
fn wheel_notches(ctx: &egui::Context) -> f32 {
    let mut steps = 0.0;
    ctx.input(|input| {
        for event in &input.events {
            if let egui::Event::MouseWheel { unit, delta, .. } = event {
                steps += match unit {
                    egui::MouseWheelUnit::Line => delta.y,
                    egui::MouseWheelUnit::Point => delta.y / WHEEL_POINTS_PER_LINE,
                    egui::MouseWheelUnit::Page => delta.y * WHEEL_POINTS_PER_LINE,
                };
            }
        }
    });
    steps
}

/// One row of the track table, as it is drawn.
struct Row<'a> {
    /// The track the row shows.
    track: &'a Track,
    /// Where the row sits.
    rect: Rect,
    /// Column widths: number, title, file name, artist, album and length.
    widths: [f32; 6],
    /// Which of the columns are shown.
    shown: [bool; 6],
    /// The row's place in the table, which names it for clicks.
    row: usize,
    /// Whether the row is selected.
    selected: bool,
    /// Whether the row is the playing track.
    playing: bool,
    /// Where playback is, while the row plays.
    progress: Option<Duration>,
    /// Whether the file name column keeps the extension.
    extension: bool,
}

/// Fills `rect` for a selected or hovered row.
fn row_background(ui: &egui::Ui, rect: Rect, radius: u8, selected: bool, hovered: bool) {
    if !selected && !hovered {
        return;
    }
    let fill = if selected {
        ui.visuals().selection.bg_fill
    } else {
        ui.visuals().widgets.hovered.weak_bg_fill
    };
    ui.painter().rect_filled(rect, radius, fill);
}

/// Draws `texture` in `rect`, cropped to a square, or a placeholder when
/// there is none.
///
/// Art of any shape is cropped rather than stretched, so a cover that is not
/// square fills the rect without losing its proportions.
fn cover(ui: &egui::Ui, rect: Rect, texture: Option<&egui::TextureHandle>, radius: u8) {
    match texture {
        Some(texture) => {
            egui::Image::new(texture)
                .uv(square_crop(texture.size_vec2()))
                .fit_to_exact_size(rect.size())
                .corner_radius(radius)
                .paint_at(ui, rect);
        }
        None => {
            ui.painter()
                .rect_filled(rect, radius, ui.visuals().faint_bg_color);
        }
    }
}

/// One row of the track table: number, title, file name, artist, album and
/// length, of which only the columns the user left on are drawn.
///
/// Every column of the playing row is written in the accent, and its
/// position follows the track's name, so the table says both which track is
/// playing and where in it the player is.
fn track_row(ui: &mut egui::Ui, row: Row<'_>) -> egui::Response {
    let Row {
        track,
        rect,
        widths,
        shown,
        extension,
        row: index,
        selected,
        playing,
        progress,
    } = row;
    let response = ui.interact(rect, ui.scope_id().with(("track", index)), Sense::click());

    row_background(ui, rect, 3, selected, response.hovered());

    let font = FontId::proportional(13.0);
    let (strong, weak) = if playing {
        let accent = ui.style().visuals.selection.stroke.color;
        (accent, accent)
    } else {
        (ui.visuals().text_color(), ui.visuals().weak_text_color())
    };

    // The position follows the track's name, whichever of the two name
    // columns is on, so turning the title off does not take it away.
    let name = name_column(shown);
    let suffix = match (progress, name) {
        (Some(position), Some(_)) => Some(progress_text(position, track.duration)),
        _ => None,
    };

    let starts = column_starts(widths);
    let cell = |column: Column| column_cell(rect, starts, widths, column);

    if shown[Column::Number.index()] {
        let number = match (track.disc, track.number) {
            (Some(disc), Some(number)) if disc > 1 => format!("{disc}-{number:02}"),
            (_, Some(number)) => format!("{number:02}"),
            _ => String::new(),
        };
        paint_text(ui, cell(Column::Number), &number, &font, weak);
    }

    if shown[Column::Title.index()] {
        let suffix = if name == Some(Column::Title) {
            suffix.as_deref()
        } else {
            None
        };
        paint_name(ui, cell(Column::Title), &track.title, suffix, &font, strong);
    }
    if shown[Column::FileName.index()] {
        let text = track.file_name(extension);
        let cell = cell(Column::FileName);
        if name == Some(Column::FileName) {
            paint_name(ui, cell, &text, suffix.as_deref(), &font, weak);
        } else {
            paint_text(ui, cell, &text, &font, weak);
        }
    }
    if shown[Column::Artist.index()] {
        paint_text(ui, cell(Column::Artist), &track.artist, &font, weak);
    }
    if shown[Column::Album.index()] {
        paint_text(ui, cell(Column::Album), &track.album_label(), &font, weak);
    }

    if shown[Column::Length.index()] {
        paint_text_right(
            ui,
            cell(Column::Length),
            &format::length(track.duration),
            &font,
            weak,
        );
    }
    response
}

/// An album's heading in the track table: its cover, its name and a rule
/// running to the right edge.
///
/// The heading is drawn where a run of an album's tracks starts, so the
/// table reads as albums rather than as one long column of titles. The rule
/// is what makes it read as a separator rather than as another track.
fn album_heading(ui: &egui::Ui, rect: Rect, album: &Album, texture: Option<&egui::TextureHandle>) {
    let accent = ui.visuals().selection.stroke.color;
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect_filled(
        rect,
        3,
        Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 22),
    );

    let art = Rect::from_min_size(
        pos2(
            rect.left() + ROW_PADDING,
            rect.center().y - ALBUM_ROW_ART / 2.0,
        ),
        vec2(ALBUM_ROW_ART, ALBUM_ROW_ART),
    );
    cover(ui, art, texture, 3);

    let font = FontId::proportional(15.0);
    let artist_font = FontId::proportional(13.5);
    let weak = ui.visuals().weak_text_color();
    let right = rect.right() - ROW_PADDING;
    let mut x = art.right() + 10.0;

    let title = elided(
        ui,
        &album_heading_text(album),
        &font,
        accent,
        (right - x).max(10.0),
    );
    let top = rect.center().y - title.size().y / 2.0;
    painter.galley(pos2(x, top), title.clone(), accent);
    x += title.size().x + 10.0;

    // The artist goes on the same line when the album name leaves room for
    // it, and is dropped rather than squeezed into a couple of letters.
    if x + 40.0 < right {
        let artist = elided(ui, &album.artist, &artist_font, weak, right - x - 8.0);
        if artist.size().x < right - x - 8.0 {
            painter.galley(
                pos2(x, rect.center().y - artist.size().y / 2.0),
                artist.clone(),
                weak,
            );
            x += artist.size().x + 10.0;
        }
    }

    if x < right - 8.0 {
        painter.line_segment(
            [pos2(x, rect.center().y), pos2(right, rect.center().y)],
            Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color),
        );
    }
}

/// The name an album's heading shows: its title, and its year when it has
/// one, so two releases of one name read apart.
fn album_heading_text(album: &Album) -> String {
    match album.year {
        Some(year) => format!("{} ({year})", album.title),
        None => album.title.clone(),
    }
}

/// One row of the album list: a small cover, the album and its artist.
fn album_row(
    ui: &mut egui::Ui,
    album: &Album,
    texture: Option<&egui::TextureHandle>,
    selected: bool,
    indent: f32,
) -> egui::Response {
    let height = ALBUM_ROW;
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click());

    row_background(ui, rect, 4, selected, response.hovered());

    // A cover smaller than the row leaves the text beside it more width; it
    // stays centred vertically.
    let art = Rect::from_min_size(
        pos2(
            rect.left() + 2.0 + indent,
            rect.center().y - ALBUM_ART / 2.0,
        ),
        vec2(ALBUM_ART, ALBUM_ART),
    );
    cover(ui, art, texture, 4);

    let left = art.right() + 10.0;
    let text_width = (rect.right() - left - 6.0).max(10.0);
    let painter = ui.painter().with_clip_rect(rect);
    let title = elided(
        ui,
        &album.title,
        &FontId::proportional(13.5),
        ui.visuals().text_color(),
        text_width,
    );
    painter.galley(
        pos2(left, rect.center().y - title.size().y - 1.0),
        title,
        ui.visuals().text_color(),
    );

    let artist = elided(
        ui,
        &album.artist,
        &FontId::proportional(11.5),
        ui.visuals().weak_text_color(),
        text_width,
    );
    painter.galley(
        pos2(left, rect.center().y + 3.0),
        artist,
        ui.visuals().weak_text_color(),
    );

    response
}

/// A loose track: one line, smaller than an album row, with the track name
/// and its length.
fn loose_row(ui: &mut egui::Ui, track: &Track, selected: bool, indent: f32) -> egui::Response {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, LOOSE_ROW_HEIGHT), Sense::click());
    row_background(ui, rect, 3, selected, response.hovered());

    let left = rect.left() + 6.0 + indent;
    let length = format::length(track.duration);
    let galley = elided(
        ui,
        &length,
        &FontId::proportional(11.0),
        ui.visuals().weak_text_color(),
        40.0,
    );
    let text_width = (rect.right() - left - galley.size().x - 10.0).max(10.0);
    let painter = ui.painter().with_clip_rect(rect);
    let title = elided(
        ui,
        &track.title,
        &FontId::proportional(12.0),
        ui.visuals().weak_text_color(),
        text_width,
    );
    painter.galley(
        pos2(left, rect.center().y - title.size().y / 2.0),
        title,
        ui.visuals().weak_text_color(),
    );
    painter.galley(
        pos2(
            rect.right() - galley.size().x - 4.0,
            rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        ui.visuals().weak_text_color(),
    );
    response
}

/// Draws the triangle that says whether a directory is open, pointing down when
/// it is and right when it is folded.
fn paint_arrow(ui: &egui::Ui, rect: Rect, collapsed: bool, color: Color32) {
    let center = rect.center();
    let size = 4.0;
    let points = if collapsed {
        vec![
            pos2(center.x - size * 0.6, center.y - size),
            pos2(center.x - size * 0.6, center.y + size),
            pos2(center.x + size, center.y),
        ]
    } else {
        vec![
            pos2(center.x - size, center.y - size * 0.6),
            pos2(center.x + size, center.y - size * 0.6),
            pos2(center.x, center.y + size),
        ]
    };
    ui.painter()
        .add(egui::Shape::convex_polygon(points, color, Stroke::NONE));
}

/// Draws a small directory glyph.
fn paint_directory_icon(ui: &egui::Ui, rect: Rect, color: Color32) {
    let painter = ui.painter();
    let body = Rect::from_center_size(rect.center(), vec2(15.0, 11.0));
    painter.rect_filled(body, 2, color);
    painter.rect_filled(
        Rect::from_min_size(pos2(body.left(), body.top() - 2.5), vec2(7.0, 4.0)),
        1,
        color,
    );
}

/// The part of an image that fills a square, centred, so art of any shape
/// is drawn without stretching.
fn square_crop(size: egui::Vec2) -> Rect {
    // The overhang of each axis, as a fraction of it, split between the two
    // edges. The longer axis has none, and square art has none at all.
    let side = size.x.min(size.y);
    let inset = vec2((size.x - side) / size.x, (size.y - side) / size.y) / 2.0;
    Rect::from_min_max(pos2(inset.x, inset.y), pos2(1.0 - inset.x, 1.0 - inset.y))
}

/// The part of `rect` that a picture of `size` fills, keeping its shape.
///
/// This is the opposite of [`square_crop`]: rather than cropping the picture
/// to fill the square, the picture is made as large as fits inside it, and
/// the rest of the square stays background. Art that is wider or taller than
/// it is square keeps every pixel, at the cost of a band above and below it.
fn fit_rect(rect: Rect, size: egui::Vec2) -> Rect {
    if size.x <= 0.0 || size.y <= 0.0 {
        return rect;
    }
    let scale = (rect.width() / size.x).min(rect.height() / size.y);
    Rect::from_center_size(rect.center(), size * scale)
}

/// What follows a playing track's title: its position, then its length.
fn progress_text(position: Duration, total: Duration) -> String {
    format!(
        " - {}/{}",
        format::duration(position),
        format::length(total)
    )
}

/// Draws one line of text in `cell`, ending at its right edge.
fn paint_text_right(ui: &egui::Ui, cell: Rect, text: &str, font: &FontId, color: Color32) {
    if text.is_empty() {
        return;
    }
    let galley = elided(ui, text, font, color, cell.width() - 6.0);
    let painter = ui.painter().with_clip_rect(cell);
    painter.galley(
        pos2(
            cell.right() - galley.size().x - ROW_PADDING,
            cell.center().y - galley.size().y / 2.0,
        ),
        galley,
        color,
    );
}

/// Draws a track's name, with `suffix` right after it.
///
/// The suffix keeps its place when the two do not fit: the name is elided
/// instead, so the position of the playing track is always readable.
fn paint_name(
    ui: &egui::Ui,
    cell: Rect,
    name: &str,
    suffix: Option<&str>,
    font: &FontId,
    color: Color32,
) {
    let width = (cell.width() - 6.0).max(10.0);
    let suffix = suffix.map(|suffix| elided(ui, suffix, font, color, width));
    let reserved = suffix.as_ref().map_or(0.0, |galley| galley.size().x);

    let name = elided(ui, name, font, color, (width - reserved).max(10.0));
    let painter = ui.painter().with_clip_rect(cell);
    let top = cell.center().y - name.size().y / 2.0;
    let end = cell.left() + name.size().x;
    painter.galley(pos2(cell.left(), top), name, color);
    if let Some(suffix) = suffix {
        painter.galley(pos2(end, top), suffix, color);
    }
}

/// The column holding a track's name: the title, or the file name when the
/// title is turned off.
///
/// The playing track's position follows this column, so a table told to show
/// file names instead of titles still says where in the track the player is.
pub(crate) fn name_column(shown: [bool; 6]) -> Option<Column> {
    if shown[Column::Title.index()] {
        Some(Column::Title)
    } else if shown[Column::FileName.index()] {
        Some(Column::FileName)
    } else {
        None
    }
}

/// Lays out `text` on one line, elided when it is wider than `width`.
fn elided(
    ui: &egui::Ui,
    text: &str,
    font: &FontId,
    color: Color32,
    width: f32,
) -> std::sync::Arc<egui::Galley> {
    let mut job =
        egui::text::LayoutJob::simple(text.to_string(), font.clone(), color, width.max(1.0));
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    ui.painter().layout_job(job)
}

/// Draws one line of text in `cell`, elided when it does not fit.
fn paint_text(ui: &egui::Ui, cell: Rect, text: &str, font: &FontId, color: Color32) {
    if text.is_empty() {
        return;
    }
    let galley = elided(ui, text, font, color, cell.width() - 6.0);
    let painter = ui.painter().with_clip_rect(cell);
    painter.galley(
        pos2(cell.left(), cell.center().y - galley.size().y / 2.0),
        galley,
        color,
    );
}

/// A transport button: a circle that lights up on hover, with a painted glyph.
fn transport(ui: &mut egui::Ui, icon: Icon, enabled: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::click());
    let painter = ui.painter();
    let visuals = ui.visuals();

    if !enabled {
        paint_icon(
            painter,
            rect,
            icon,
            visuals.widgets.noninteractive.fg_stroke.color,
        );
        return response;
    }

    let fill = if response.is_pointer_button_down_on() {
        visuals.widgets.active.bg_fill
    } else if response.hovered() {
        visuals.widgets.hovered.bg_fill
    } else {
        visuals.widgets.inactive.weak_bg_fill
    };
    painter.circle_filled(rect.center(), rect.width() / 2.0, fill);
    paint_icon(painter, rect, icon, visuals.text_color());
    response
}

/// A mode button: a circle that lights up on hover and stays filled while
/// its mode is on, with the glyph in the accent colour.
///
/// `accent` is passed in rather than read from the visuals because the
/// palette holds the accent, not egui's style.
fn toggle(ui: &mut egui::Ui, icon: Icon, on: bool, accent: Color32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::click());
    let painter = ui.painter();
    let visuals = ui.visuals();

    let fill = if response.is_pointer_button_down_on() {
        visuals.widgets.active.bg_fill
    } else if response.hovered() {
        visuals.widgets.hovered.bg_fill
    } else if on {
        visuals.widgets.active.bg_fill
    } else {
        Color32::TRANSPARENT
    };
    if fill != Color32::TRANSPARENT {
        painter.circle_filled(rect.center(), rect.width() / 2.0, fill);
    }
    let color = if on {
        accent
    } else if response.hovered() {
        visuals.text_color()
    } else {
        visuals.widgets.noninteractive.fg_stroke.color
    };
    paint_icon(painter, rect, icon, color);
    response
}

/// Paints a transport or mode button's glyph inside `rect`.
fn paint_icon(painter: &egui::Painter, rect: Rect, icon: Icon, color: Color32) {
    let center = rect.center();
    let size = rect.width().min(rect.height()) * 0.30;
    match icon {
        Icon::Play => triangle(painter, center, size, true, color),
        Icon::Pause => {
            let width = size * 0.34;
            for offset in [-size * 0.42, size * 0.42] {
                painter.rect_filled(
                    Rect::from_center_size(
                        pos2(center.x + offset, center.y),
                        vec2(width, size * 1.6),
                    ),
                    1,
                    color,
                );
            }
        }
        Icon::Previous | Icon::Next => {
            let forward = matches!(icon, Icon::Next);
            let sign = if forward { 1.0 } else { -1.0 };
            painter.rect_filled(
                Rect::from_center_size(
                    pos2(center.x + sign * size * 0.9, center.y),
                    vec2(size * 0.28, size * 1.6),
                ),
                1,
                color,
            );
            triangle(
                painter,
                pos2(center.x - sign * size * 0.18, center.y),
                size,
                forward,
                color,
            );
        }
        Icon::Stop => {
            painter.rect_filled(
                Rect::from_center_size(center, vec2(size * 1.5, size * 1.5)),
                2,
                color,
            );
        }
        Icon::Repeat => {
            // A loop with a gap at the top right and the bottom left, each
            // gap closed by an arrow: the shape a player uses for "again".
            let left = center.x - size * 0.88;
            let right = center.x + size * 0.88;
            let top = center.y - size * 0.72;
            let bottom = center.y + size * 0.72;
            let stroke = Stroke::new(1.4, color);
            painter.line_segment(
                [
                    pos2(left + size * 0.28, top),
                    pos2(right - size * 0.62, top),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    pos2(right, top + size * 0.28),
                    pos2(right, bottom - size * 0.28),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    pos2(right - size * 0.28, bottom),
                    pos2(left + size * 0.62, bottom),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    pos2(left, bottom - size * 0.28),
                    pos2(left, top + size * 0.28),
                ],
                stroke,
            );
            triangle(
                painter,
                pos2(right - size * 0.22, top),
                size * 0.36,
                true,
                color,
            );
            triangle(
                painter,
                pos2(left + size * 0.22, bottom),
                size * 0.36,
                false,
                color,
            );
        }
        Icon::Random => {
            // Two lines crossing, each ending in an arrow: the shuffle mark.
            let left = center.x - size * 0.88;
            let right = center.x + size * 0.6;
            let top = center.y - size * 0.68;
            let bottom = center.y + size * 0.68;
            let stroke = Stroke::new(1.4, color);
            painter.line_segment([pos2(left, top), pos2(right, bottom)], stroke);
            painter.line_segment([pos2(left, bottom), pos2(right, top)], stroke);
            triangle(
                painter,
                pos2(right + size * 0.26, bottom),
                size * 0.36,
                true,
                color,
            );
            triangle(
                painter,
                pos2(right + size * 0.3, top),
                size * 0.4,
                true,
                color,
            );
        }
    }
}

/// Paints a filled triangle pointing right or left, centred on `center`.
fn triangle(painter: &egui::Painter, center: egui::Pos2, size: f32, right: bool, color: Color32) {
    let width = size * 0.95;
    let height = size;
    let points = if right {
        vec![
            pos2(center.x - width / 2.0, center.y - height),
            pos2(center.x - width / 2.0, center.y + height),
            pos2(center.x + width / 2.0, center.y),
        ]
    } else {
        vec![
            pos2(center.x + width / 2.0, center.y - height),
            pos2(center.x + width / 2.0, center.y + height),
            pos2(center.x - width / 2.0, center.y),
        ]
    };
    painter.add(egui::Shape::convex_polygon(points, color, Stroke::NONE));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_playing_track_carries_its_position_after_its_title() {
        assert_eq!(
            progress_text(Duration::from_secs(3), Duration::from_secs(307)),
            " - 0:03/5:07"
        );
        assert_eq!(
            progress_text(Duration::from_secs(65), Duration::from_secs(3725)),
            " - 1:05/1:02:05"
        );
    }

    #[test]
    fn an_album_is_headed_where_its_run_starts() {
        // Tracks 0 and 1 are album 0, 2 and 3 are album 1.
        let albums = [Some(0), Some(0), Some(1), Some(1)];
        let rows = table_rows(&[0, 1, 2, 3], &albums);
        assert_eq!(
            rows,
            [
                TableRow::Album(0),
                TableRow::Track(0),
                TableRow::Track(1),
                TableRow::Album(1),
                TableRow::Track(2),
                TableRow::Track(3),
            ]
        );
    }

    #[test]
    fn a_loose_file_heads_nothing() {
        // A file with no album tag belongs to no album, so there is no
        // album to name above it.
        let albums = [Some(0), None, None];
        let rows = table_rows(&[0, 1, 2], &albums);
        assert_eq!(
            rows,
            [
                TableRow::Album(0),
                TableRow::Track(0),
                TableRow::Track(1),
                TableRow::Track(2),
            ]
        );
    }

    #[test]
    fn an_album_that_comes_back_is_headed_again() {
        // A playlist that goes back to an earlier album says so, rather than
        // leaving its second run under the heading of the album before it.
        let albums = [Some(0), Some(1), Some(0)];
        let rows = table_rows(&[0, 1, 2], &albums);
        assert_eq!(
            rows,
            [
                TableRow::Album(0),
                TableRow::Track(0),
                TableRow::Album(1),
                TableRow::Track(1),
                TableRow::Album(0),
                TableRow::Track(2),
            ]
        );
    }

    #[test]
    fn the_rows_sit_where_their_heights_put_them() {
        let albums = [Some(0), Some(0), Some(1)];
        let rows = table_rows(&[0, 1, 2], &albums);
        let tops = row_tops(&rows);

        assert_eq!(tops.len(), rows.len());
        assert_eq!(tops[0], 0.0);
        assert_eq!(
            tops[1], ALBUM_ROW_HEIGHT,
            "the first track follows the heading"
        );
        assert_eq!(tops[2], ALBUM_ROW_HEIGHT + ROW_HEIGHT);
        assert_eq!(tops[3], ALBUM_ROW_HEIGHT + 2.0 * ROW_HEIGHT);
        let last = rows.len() - 1;
        assert_eq!(
            rows_height(&rows),
            tops[last] + rows[last].height(),
            "every row's height is counted, the last one included"
        );
    }

    #[test]
    fn fitting_art_keeps_its_shape() {
        let square = Rect::from_min_size(pos2(10.0, 20.0), vec2(200.0, 200.0));

        // Square art fills the square exactly.
        let fitted = fit_rect(square, vec2(300.0, 300.0));
        assert!((fitted.width() - 200.0).abs() < 0.01);
        assert!((fitted.height() - 200.0).abs() < 0.01);

        // Wide art keeps its shape and is centred, with bands above and below.
        let fitted = fit_rect(square, vec2(400.0, 200.0));
        assert!((fitted.width() - 200.0).abs() < 0.01, "it spans the square");
        assert!((fitted.height() - 100.0).abs() < 0.01, "at half the height");
        assert!((fitted.center().y - square.center().y).abs() < 0.01);
        assert!(fitted.top() > square.top(), "the band above shows through");

        // Tall art does the same the other way round.
        let fitted = fit_rect(square, vec2(200.0, 400.0));
        assert!((fitted.width() - 100.0).abs() < 0.01);
        assert!((fitted.height() - 200.0).abs() < 0.01);
    }

    #[test]
    fn cropping_art_takes_the_middle_square_of_any_shape() {
        // Wide art loses a quarter of its width at each side and keeps its
        // full height.
        let uv = square_crop(vec2(400.0, 200.0));
        assert!((uv.min.x - 0.25).abs() < 1e-6);
        assert!((uv.max.x - 0.75).abs() < 1e-6);
        assert_eq!(uv.min.y, 0.0);
        assert_eq!(uv.max.y, 1.0);

        // Tall art loses a quarter of its height at each end instead.
        let uv = square_crop(vec2(200.0, 400.0));
        assert_eq!(uv.min.x, 0.0);
        assert_eq!(uv.max.x, 1.0);
        assert!((uv.min.y - 0.25).abs() < 1e-6);
        assert!((uv.max.y - 0.75).abs() < 1e-6);

        // Square art is used whole.
        let uv = square_crop(vec2(300.0, 300.0));
        assert_eq!(uv.min, pos2(0.0, 0.0));
        assert_eq!(uv.max, pos2(1.0, 1.0));
    }

    #[test]
    fn a_heading_names_the_year_it_was_released() {
        let mut album = Album {
            title: "An Album".to_string(),
            artist: "An Artist".to_string(),
            year: Some(2018),
            tracks: vec![0],
            cover: None,
            directory: std::path::PathBuf::new(),
        };
        assert_eq!(album_heading_text(&album), "An Album (2018)");
        album.year = None;
        assert_eq!(album_heading_text(&album), "An Album");
    }

    /// Which of the two name columns carries the playing position.
    #[test]
    fn the_position_follows_the_name_column_that_is_shown() {
        let all = [true; 6];
        assert_eq!(
            name_column(all),
            Some(Column::Title),
            "the title comes first"
        );

        let mut no_title = [true; 6];
        no_title[Column::Title.index()] = false;
        no_title[Column::FileName.index()] = true;
        assert_eq!(
            name_column(no_title),
            Some(Column::FileName),
            "a table showing file names still says where the player is"
        );

        let mut no_name = [true; 6];
        no_name[Column::Title.index()] = false;
        no_name[Column::FileName.index()] = false;
        assert_eq!(name_column(no_name), None, "neither name is shown");
    }
}

#[cfg(test)]
mod wheel_tests {
    use super::*;

    #[test]
    fn one_notch_moves_the_volume_by_a_twentieth() {
        assert!((VOLUME_PER_NOTCH - 0.05).abs() < f32::EPSILON);
    }

    #[test]
    fn a_point_delta_becomes_notches() {
        // egui reports a notch as about 50 points, so 150 points is three.
        assert!((150.0 / WHEEL_POINTS_PER_LINE - 3.0).abs() < f32::EPSILON);
    }
}

#[cfg(test)]
mod layout_tests {
    use super::*;

    #[test]
    fn a_playing_row_out_of_sight_is_still_found() {
        let tracks: Vec<usize> = (0..500).collect();
        // The row is not among the few that are drawn, and it is still found
        // so the table can scroll to it.
        assert_eq!(row_to_reveal(true, Some(480), &tracks), Some(480));
        assert_eq!(row_to_reveal(true, Some(0), &tracks), Some(0));
        assert_eq!(row_to_reveal(false, Some(480), &tracks), None);
        assert_eq!(row_to_reveal(true, None, &tracks), None);
        assert_eq!(
            row_to_reveal(true, Some(999), &tracks),
            None,
            "a row the table does not show cannot be scrolled to"
        );
    }

    #[test]
    fn a_track_to_reveal_is_named_by_its_index_not_its_place() {
        // A playlist is not the identity list: what comes back is the track
        // index the rows are matched on, not where it sits in the playlist.
        assert_eq!(row_to_reveal(true, Some(3), &[7, 3, 9]), Some(3));
        assert_eq!(row_to_reveal(true, Some(0), &[7, 3, 9]), None);
    }

    /// Runs `draw` once on a window 400 points wide, and returns what it
    /// reports.
    fn drawn<T>(mut draw: impl FnMut(&mut egui::Ui) -> T) -> T {
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 300.0))),
            ..Default::default()
        };
        let mut drawn = None;
        let mut output = ctx.run_ui(input, |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                ui.set_width(300.0);
                drawn = Some(draw(ui));
            });
        });
        // Nothing uploads the textures a first frame produces, and the delta
        // is loud about being dropped unapplied.
        output.textures_delta.clear();
        drawn.expect("the pass ran")
    }

    #[test]
    fn the_comparison_fits_its_window_in_every_state() {
        // The window is one fixed size, so every state has to fit inside it:
        // a taller one would be cut off, and a wider one would push the
        // window open again.
        use crate::app::tests::track;
        let sums = (
            "8923c15866d04cf7b7d9059bce7df05a".to_string(),
            "265ce68da0504e8c6049ba126856da5a".to_string(),
        );
        let mut test = Abx::new(
            track("One", 1, "A Title Long Enough To Fill The Line"),
            track("One", 2, "Short"),
            7,
        );
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 700.0))),
            ..Default::default()
        };
        let mut overflow = Vec::new();
        for state in 0..3 {
            let sums = (state > 0).then_some(&sums);
            let borrowed = sums.map(|(a, b)| (a.as_str(), b.as_str()));
            let signature = test.signature(borrowed);
            let log = test.signed_log(borrowed);
            let view = AbxView {
                test: &test,
                sums,
                position: Duration::from_secs(30),
                length: Duration::from_secs(200),
                start: Duration::from_secs(10),
                accent: Color32::WHITE,
                signature: signature.as_deref(),
                log: &log,
            };
            let mut sizes = None;
            let mut output = ctx.run_ui(input.clone(), |ui| {
                egui::Window::new("Blind Comparison")
                    .collapsible(false)
                    .fixed_size(ABX_WINDOW_SIZE)
                    .show(ui.ctx(), |ui| {
                        let room = ui.max_rect();
                        abx_body(ui, &view);
                        sizes = Some((ui.min_rect().size(), room.size()));
                    });
            });
            output.textures_delta.clear();
            let (used, room) = sizes.expect("the window drew");
            if used.x > room.x + 0.5 || used.y > room.y + 0.5 {
                overflow.push(format!("state {state}: {used:?} in {room:?}"));
            }
            // On to the next state: answered, then the result.
            test.answer(Side::A);
            test.next();
            if state == 1 {
                for _ in 1..TRIALS {
                    test.answer(Side::A);
                    test.next();
                }
            }
        }
        assert!(overflow.is_empty(), "{overflow:?}");
    }

    #[test]
    fn a_row_of_buttons_is_centred_across_the_window() {
        // The margins either side of the buttons are what is checked: egui
        // lays a row out from the left of the room it has, so the row has to
        // be shifted in by half of what the buttons leave over.
        // Both the plain rows and the answers, which are given a wider gap.
        for gap in [0.0, ANSWER_GAP] {
            let (left, right) = drawn(|ui| {
                let window = ui.max_rect();
                let row = centered_buttons(ui, &["Play A", "Play B"], None, gap).rect;
                (row.left() - window.left(), window.right() - row.right())
            });
            assert!(
                (left - right).abs() < 1.0,
                "a gap of {gap}: the buttons sit {left} from one edge and {right} from the other"
            );
            assert!(left > 10.0, "and they are not simply at the left: {left}");
        }
    }

    #[test]
    fn a_button_is_as_wide_as_the_centring_assumes() {
        // The row is placed before its buttons are drawn, so their width has
        // to be worked out from the text and the padding. This is that
        // sum against the width egui gives the same button.
        let pairs = drawn(|ui| {
            ["Play A", "Play X", "X is A", "Run Again", "Close"]
                .into_iter()
                .map(|label| {
                    let assumed = button_width(ui, label);
                    let button = egui::Button::new(label).selected(false);
                    let drawn = ui.add(button).rect.width();
                    (label, assumed, drawn)
                })
                .collect::<Vec<_>>()
        });
        for (label, assumed, drawn) in pairs {
            assert!(
                (assumed - drawn).abs() < 0.5,
                "{label}: assumed {assumed}, drawn {drawn}"
            );
        }
    }

    #[test]
    fn the_close_button_sits_at_the_left_edge() {
        // Close is the one button that must not be under the pointer while
        // the listener is pressing Play and the answers, which sit in the
        // middle: it starts at the edge and leaves the middle alone.
        let (left, right) = drawn(|ui| {
            let window = ui.max_rect();
            let (_, row) = abx_footer(ui, false, "");
            (row.left() - window.left(), window.right() - row.right())
        });
        assert_eq!(left, 0.0, "Close starts at the edge");
        assert!(right > 100.0, "and leaves the middle of the window free");
    }

    #[test]
    fn the_trial_and_its_question_are_centred_too() {
        let (left, right) = drawn(|ui| {
            let window = ui.max_rect();
            let label = centered_label(ui, "Which one is X?");
            (label.left() - window.left(), window.right() - label.right())
        });
        assert!(
            (left - right).abs() < 1.0,
            "the line sits {left} from one edge and {right} from the other"
        );
        assert!(left > 10.0, "and it is not simply at the left: {left}");
    }

    #[test]
    fn a_moment_typed_into_the_start_field_is_read_as_minutes_and_seconds() {
        assert_eq!(parse_time("90"), Some(90.0));
        assert_eq!(parse_time("1:30"), Some(90.0));
        assert_eq!(parse_time("1:02:03"), Some(3723.0));
        assert_eq!(parse_time(" 0:05 "), Some(5.0));
        assert_eq!(parse_time("nonsense"), None);
        assert_eq!(
            parse_time("-5"),
            Some(0.0),
            "a negative moment is the start"
        );
    }

    #[test]
    fn a_row_wider_than_the_window_starts_at_its_edge() {
        assert_eq!(centring_shift(&[300.0, 300.0], 8.0, 200.0), 0.0);
    }

    #[test]
    fn the_waveform_sits_in_the_middle_of_its_strip() {
        // The panel keeps two points of margin above and below its content.
        let strip = crate::app::PLAYER_HEIGHT - 4.0;
        let gap = wave_gap(strip, WAVE_HEIGHT);
        assert!(
            (gap * 2.0 + WAVE_HEIGHT - strip).abs() < 0.01,
            "the gaps above and below add up to the strip"
        );
        assert!(
            (gap - 6.9).abs() < 0.01,
            "the gap is a third narrower than centring the old bar left"
        );
        assert_eq!(
            wave_gap(10.0, WAVE_HEIGHT),
            0.0,
            "a short strip gets no gap"
        );
    }

    /// The stored widths the app starts with, in the order the columns are
    /// drawn: number, title, file name, artist, album and length.
    const STORED: [f32; 6] = [38.0, 320.0, 320.0, 230.0, 215.0, 64.0];

    /// The columns the table starts with: every one but the file name.
    const STARTING: [bool; 6] = [true, true, false, true, true, true];

    /// Every column shown, file name included.
    const ALL: [bool; 6] = [true; 6];

    fn total(widths: [f32; 6]) -> f32 {
        widths.iter().sum::<f32>() + 2.0 * ROW_PADDING
    }

    #[test]
    fn a_column_sits_where_it_is_drawn() {
        // The arrays the app keeps are indexed by `Column::index`, so a
        // column has to be in the position it is drawn at.
        for (position, column) in Column::ALL.iter().enumerate() {
            assert_eq!(column.index(), position);
        }
    }

    #[test]
    fn a_wide_panel_grows_the_columns_to_fill_it() {
        let (widths, scale) = column_layout(1400.0, STORED, ALL);
        assert!(scale > 1.0, "the columns grow to fill a wide panel");
        assert!(
            (total(widths) - 1400.0).abs() < 0.01,
            "no room is left over"
        );
        let ratio = widths[1] / widths[3];
        assert!(
            (ratio - STORED[1] / STORED[3]).abs() < 0.001,
            "they grow in step"
        );
    }

    #[test]
    fn a_wide_panel_does_not_widen_the_columns_of_digits() {
        // The number and length columns hold a few characters, so a wider
        // one is only empty space beside its value. The text columns take
        // the room instead.
        let (widths, _) = column_layout(1400.0, STORED, ALL);
        assert!(
            (widths[Column::Length.index()] - STORED[Column::Length.index()]).abs() < 0.01,
            "the length column keeps the width it was given"
        );
        assert!(
            (widths[Column::Number.index()] - STORED[Column::Number.index()]).abs() < 0.01,
            "and so does the number"
        );
        assert!(
            widths[Column::Title.index()] > STORED[Column::Title.index()],
            "the title column takes the room they did not use"
        );
        assert!(
            (total(widths) - 1400.0).abs() < 0.01,
            "the table still fills the panel"
        );
    }

    #[test]
    fn a_table_of_digits_alone_still_fills_its_panel() {
        // Nothing to give the spare room to, so the columns scale together
        // rather than leaving a strip of background at the right edge.
        let shown = [true, false, false, false, false, true];
        let (widths, _) = column_layout(900.0, STORED, shown);
        assert!(
            (total(widths) - 900.0).abs() < 0.01,
            "the two columns share the panel"
        );
    }

    #[test]
    fn a_narrow_panel_shrinks_the_columns_before_it_scrolls() {
        let (widths, scale) = column_layout(1000.0, STORED, ALL);
        assert!(scale < 1.0, "a panel this narrow shrinks the columns");
        assert!(scale > 0.75, "but not to their minimum, so nothing scrolls");
        assert!(
            (total(widths) - 1000.0).abs() < 0.01,
            "the columns still fit"
        );
        assert!(
            widths[1] > MIN_COLUMN,
            "none is squeezed to its minimum yet"
        );
    }

    #[test]
    fn the_table_scrolls_only_once_the_columns_cannot_shrink_more() {
        // The panel is narrower than the columns can be squeezed to.
        let (widths, scale) = column_layout(200.0, STORED, ALL);
        let floor = Column::ALL
            .iter()
            .map(|column| column.min_width() / STORED[column.index()])
            .fold(0.0_f32, f32::max);
        assert!((scale - floor).abs() < 0.001, "they stop at their minimum");
        assert!(total(widths) > 200.0, "and the table scrolls instead");
        assert!(widths[1] >= MIN_COLUMN - 0.01);
        assert!(
            widths[Column::Length.index()] >= MIN_COLUMN - 0.01,
            "the length column stops at its own minimum"
        );
        assert!(table_needed(STORED, ALL) > 200.0);
    }

    #[test]
    fn dragging_a_border_moves_width_between_its_two_columns() {
        // The file name column is hidden, so the title's right border is
        // the one against the artist. The drag works in the units of the
        // column on the left of the border, which is what the drawing uses:
        // a column of digits keeps its stored width while a text column is
        // scaled.
        let (widths, _) = column_layout(1200.0, STORED, STARTING);
        let ratio = widths[1] / STORED[1];
        let moved = 40.0 / ratio;
        let mut columns = STORED;
        let left = widths[1] / ratio;
        let right = columns[3];
        let pair = (left + moved).clamp(MIN_COLUMN, left + right - MIN_COLUMN);
        columns[1] = pair;
        columns[3] = left + right - pair;

        let (after, _) = column_layout(1200.0, columns, STARTING);
        assert!(
            (after[1] - (widths[1] + 40.0)).abs() < 0.01,
            "the border follows the pointer"
        );
        assert!(
            (columns[1] + columns[3] - STORED[1] - STORED[3]).abs() < 0.01,
            "the two columns keep their total, so the scale does not change"
        );
    }

    #[test]
    fn the_header_sits_over_the_values_it_names() {
        let (widths, _) = column_layout(1100.0, STORED, STARTING);
        let starts = column_starts(widths);
        // Each column starts where the one before it ends.
        for index in 1..6 {
            assert_eq!(starts[index], starts[index - 1] + widths[index - 1]);
        }
        assert_eq!(starts[0], ROW_PADDING);
        // The length column is pinned to the right edge.
        let right = starts[5] + widths[5] + ROW_PADDING;
        assert!((right - 1100.0).abs() < 0.01);
    }

    #[test]
    fn the_file_name_has_a_column_of_its_own() {
        // Turning the title off leaves the file name where it is: the two
        // are separate columns, not one column with two ways to fill it.
        let shown = [true, false, true, true, true, true];
        let (widths, _) = column_layout(1100.0, STORED, shown);
        assert_eq!(widths[Column::Title.index()], 0.0, "the title is gone");
        assert!(
            widths[Column::FileName.index()] > 0.0,
            "the file name is still there"
        );
    }

    #[test]
    fn a_hidden_column_gives_its_room_to_the_others() {
        let shown = [true, true, false, false, true, true];
        let (widths, _) = column_layout(1100.0, STORED, shown);
        assert_eq!(widths[3], 0.0, "the artist column takes no room");
        assert!(
            (total(widths) - 1100.0).abs() < 0.01,
            "the other columns fill the panel"
        );
        // The room left after the columns of digits goes to the text
        // columns, in proportion to the widths they were dragged to.
        let fixed = STORED[0] + STORED[5];
        let flexible = STORED[1] + STORED[4];
        let room = 1100.0 - 2.0 * ROW_PADDING - fixed;
        assert!(
            (widths[1] - STORED[1] / flexible * room).abs() < 0.01,
            "the title takes its share of the room the panel gives"
        );
        assert!(
            widths[1] > STORED[1],
            "which is wider than stored, since the panel has room for it"
        );
        // The header's positions follow, so no label sits over a gap.
        let starts = column_starts(widths);
        assert_eq!(
            starts[4], starts[3],
            "the album starts where the artist was"
        );
    }

    #[test]
    fn hiding_a_column_moves_the_point_where_the_table_scrolls() {
        assert!(
            table_needed(STORED, STARTING) < table_needed(STORED, ALL),
            "a table with fewer columns needs less room before it scrolls"
        );
        let (widths, _) = column_layout(300.0, STORED, STARTING);
        assert_eq!(widths[Column::FileName.index()], 0.0);
        assert!(
            widths[1] >= MIN_COLUMN - 0.01,
            "the columns that are left stop at their minimum"
        );
    }

    #[test]
    fn the_album_list_is_a_flat_list_of_rows() {
        // The list draws only the rows on screen, so the tree is flattened
        // first. A pass-through directory gives its album the row, a real
        // directory gets one of its own, and folding it hides what it holds.
        // The three albums sit in one directory, so it needs a row of its
        // own; an album in a directory of its own would not.
        let mut tracks = Vec::new();
        for (album, title) in [("One", "A"), ("Two", "B"), ("Three", "C")] {
            let mut track = crate::app::tests::track(album, 1, title);
            track.directory = std::path::PathBuf::from("Shared");
            tracks.push(track);
        }
        let mut app = crate::app::App::new(&egui::Context::default(), true, Vec::new());
        app.library = crate::library::Library::from_tracks(tracks);
        let rows = app.library_rows();
        assert!(
            rows.iter()
                .any(|row| matches!(row, LibraryRow::Directory { .. })),
            "the directory holding several albums gets a row"
        );
        assert_eq!(
            rows.iter()
                .filter(|row| matches!(row, LibraryRow::Album { .. }))
                .count(),
            3,
            "every album is listed"
        );

        let path = app.library.directories()[0].path.clone();
        app.toggle_collapsed_at(&path);
        let folded = app.library_rows();
        assert!(
            folded.iter().any(|row| matches!(
                row,
                LibraryRow::Directory {
                    collapsed: true,
                    ..
                }
            )),
            "the directory row says it is folded"
        );
        assert!(
            folded
                .iter()
                .all(|row| !matches!(row, LibraryRow::Album { .. })),
            "nothing under it is drawn"
        );
    }

    #[test]
    fn files_in_the_music_directory_get_top_level_rows() {
        // A library whose files sit directly in the music directory has no
        // directory to put them under, so they head the list themselves
        // instead of vanishing from it.
        let mut album = crate::app::tests::track("One", 1, "A");
        album.directory = std::path::PathBuf::new();
        let mut loose = crate::app::tests::track("One", 2, "B");
        loose.directory = std::path::PathBuf::new();
        loose.tagged_album = false;
        let mut app = crate::app::App::new(&egui::Context::default(), true, Vec::new());
        app.library = crate::library::Library::from_tracks(vec![album, loose]);

        let rows = app.library_rows();
        assert_eq!(rows.len(), 2, "the album and the loose file, nothing else");
        assert!(
            matches!(rows[0], LibraryRow::Album { album: 0, depth: 0 }),
            "the album is the first row, at the top"
        );
        assert!(
            matches!(rows[1], LibraryRow::Loose { track: 1, depth: 0 }),
            "the loose file follows it, at the same depth"
        );
    }

    #[test]
    fn album_list_rows_sit_where_their_heights_put_them() {
        let rows = vec![
            LibraryRow::Directory {
                path: std::path::PathBuf::from("A"),
                name: "A".to_string(),
                depth: 0,
                collapsed: false,
            },
            LibraryRow::Album { album: 0, depth: 1 },
            LibraryRow::Loose { track: 0, depth: 1 },
        ];
        let tops = library_row_tops(&rows);
        assert_eq!(
            tops,
            [0.0, DIRECTORY_ROW_HEIGHT, DIRECTORY_ROW_HEIGHT + ALBUM_ROW]
        );
        assert_eq!(
            library_rows_height(&rows),
            DIRECTORY_ROW_HEIGHT + ALBUM_ROW + LOOSE_ROW_HEIGHT
        );
    }

    #[test]
    fn every_column_can_be_turned_off() {
        // The number column is no exception: the table can be emptied.
        let (widths, _) = column_layout(800.0, STORED, [false; 6]);
        assert_eq!(widths, [0.0; 6]);
        assert_eq!(table_needed(STORED, [false; 6]), 2.0 * ROW_PADDING);
    }

    #[test]
    fn a_border_stops_at_the_minimum_width() {
        let columns = STORED;
        let left = columns[Column::Number.index()];
        let right = columns[Column::Title.index()];
        // Pull the border far left; the number column stops at its own
        // minimum, which is narrower than the others', and the title keeps
        // the rest.
        let pair = (MIN_NUMBER_COLUMN - 200.0).clamp(
            Column::Number.min_width(),
            left + right - Column::Title.min_width(),
        );
        assert_eq!(pair, MIN_NUMBER_COLUMN);
        assert_eq!(left + right - pair, 38.0 + 320.0 - MIN_NUMBER_COLUMN);
    }
}
