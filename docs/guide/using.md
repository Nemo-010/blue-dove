# Using Blue Dove

## Add the directories

Pick **File > Manage Library Sources**, or press `Ctrl+O`, and add a directory
to the list. The browser that opens walks your file system and needs nothing
installed besides Blue Dove itself.

Every directory on the list is scanned, and so is everything under it. Add as
many as you like: the album list keeps each one in a part of its own. The same
window leaves a directory out of the scan, with everything under it, without
touching the files: point it at the Christmas music inside a library you listen
to the rest of the year, and it stays where it is on disk and off the album
list.

Blue Dove reads the tags of every audio file it finds and groups them into
albums by album artist, album and year. Directories it cannot read are skipped;
a file whose tags cannot be read is still listed under its file name.

Files and directories whose name starts with a dot are left out, which is
what **View > Ignore hidden files** is for. It is on to begin with; turn
it off to list a music directory that keeps its files in a dotted
directory. Either way the directory you picked is scanned, even if its own
name starts with a dot. Changing the setting scans again.

The directories are remembered, so the next start scans them again and shows
the library at once. They are also watched while the window runs: a file added,
moved or retagged outside the window is scanned a few seconds later,
without a rescan by hand. That is what **File > Scan Automatically** covers,
and it is on to begin with; turn it off and the library is only read when you
ask, with **File > Rescan Library** or by changing the sources.

Storage that cannot report changes, such as a network share, is scanned an
hour after the last scan instead, and so is a directory with more
subdirectories than the kernel keeps watches for. Both still work, they just
notice later.

Reading is not a change: a player, an indexer or a backup reading the library
does not make it scan again. Only a file or directory that appears, goes,
is renamed, written or retagged does.

## Find an album

The menu bar and the library share the left-hand column: the menus sit
at its top, and below them your music as a tree of directories,
with the albums inside them. Click the arrow beside a directory to fold
it away.

A directory that holds a single album is not shown as a row of its own:
the album stands where the directory would, so a release is never buried
under a directory named after it. A directory holding several albums keeps
its row, with those albums listed beneath it. Files that carry no album
tag appear as small rows under their directory.

The albums and the untagged files that sit directly in one of your
directories have no row above them: they head the list, and the
directories below follow. Two sources or more each keep a row of their own,
so their subdirectories never mingle. **Add All** in the Library header
appends the whole library in that order, untagged files included.

Click an album to select it. Double-click one to make it the playlist.
Double-click a directory to append everything it holds: its albums in
playing order, the loose files beside them, and then whatever its
subdirectories hold. A directory of loose files alone queues those.

Cover art comes from the file's tags when it has any, and otherwise from
an image beside the tracks: `cover.jpg`, `front.png`, `folder.jpg` and
the other usual names first, then any image whose name starts with
`cover`, `front`, `folder` or `album`. A `covers`, `artwork` or `scans`
directory is looked in too, and a disc directory of a multi-disc rip takes the
art from the directory above it. Tracks without any of that show an empty
square.

## Starting up

The library is read once and then kept in the cache directory, under
`blue-dove`. A start shows the albums, the covers and the playlists at
once, and a scan of the directory runs behind it: files whose size or
modification time changed are read again, and everything else comes
from the cache. On a library of seven thousand tracks the first start
takes about six seconds and later ones a fraction of a second.

Deleting the cache directory is always safe; the next start scans the
directories as if they were new.

## Playlists

The tabs above the track table are playlists. There is always at least
one, named "Default" until you rename it: double-click a tab to type a
new name, and press Enter. The `+` opens another, and the `x` closes one;
the last one stays.

Double-clicking an album replaces the tracks of the active playlist with
that album. Double-clicking a directory adds every album in it and in
its subdirectories, without disturbing what the playlist already holds.
Double-clicking a loose file plays it on its own.

The tabs, their names, their tracks and the one you were last on are all
kept: the next start opens the same session. Tracks are remembered by
file, so a playlist whose files moved comes back with the ones it can
still find, and a directory that is not mounted leaves the saved tabs
alone until it is back.

A playlist that holds more than one album is drawn as a list of albums:
wherever a run of an album's tracks starts, a heading names the album,
with its cover, its artist and its year, and a rule running to the right
edge. A playlist that goes back to an earlier album heads it again, so
the heading always says which album the tracks under it belong to. Loose
files belong to no album and get no heading.

## Play

Double-click a track, or select one and press the play button. Playback
continues with the next track of the album when one ends, and wraps
around at the end.

The transport has four buttons: previous, play or pause, next, and stop.
They sit on a strip below the album art, with the repeat and random
buttons on either end, and the volume slider filling what is left of the
strip. Stop clears the now-playing bar and rewinds, leaving the track
selected, so play starts it again from the beginning.

Previous restarts the playing track once it has been playing for three
seconds, and a press while it is still at its start goes to the track
before it. So a press in the middle of a track starts it over, and a
second press moves on.

The Length column shows the length of every track. The playing row adds
its position after the track's name, as `- 0:21/2:14`, so the row you are
listening to says where in the track the player is. The name is the title
column, or the file name column when titles are turned off, so the
position is there either way. Every column of the playing row is written
in the accent colour, which is what makes it stand out from the rest.

The album art of the playing track fills the top of the library panel on
the left, with the transport, the playback modes and the volume on a
strip below it.

Art that is not square cannot both fill that space and be shown whole, so
a right click on it chooses: **Fill the space** crops the picture to a
square, and **Fit the whole image** shows all of it, with the space it
does not fill left as background. The choice is remembered.

The bar at the bottom holds the waveform of the whole track, spanning
the width of the window. The part already played is drawn in amber.
Click anywhere in it to jump there, or drag along it to scrub.

The waveform is worked out once and kept in the cache directory, so a
track played before draws its bar at once. The first time a track plays,
working it out takes a moment; until it is ready the bar says so, and the
track plays normally.

## Formats

The player decodes everything in the crate itself, with pure Rust
decoders, so nothing has to be installed. FLAC, MP3, M4A and ALAC,
AAC, Ogg Vorbis, Opus, WAV, AIFF, CAF, Matroska audio, MP1 and MP2 go
through symphonia. Monkey's Audio, WavPack, TTA, Musepack, Shorten and
AMR have decoders of their own, and so do the formats video game rips use:
Sony VAG, CRI ADX, and Nintendo DSP, BRSTM, BCSTM and BFSTM. They all
get a waveform like every other track.

Opus is decoded as it plays rather than read into memory first, so even
a long track starts at once.

A game rip carries no tags, so it is listed under its file name, and its
length shows as `--:--` until it plays.

Album art is read from the tags, or from an image beside the tracks as
described under the album list.

Titles written in Japanese, Chinese or Korean are drawn with a system
font that carries those glyphs, found among the installed fonts at
startup. Without one, those titles show as boxes: install a CJK font,
such as `noto-fonts-cjk` on Arch or `fonts-noto-cjk` on Debian and
Ubuntu, and they draw.

DSD and TAK are not supported: DSD needs a decoding path of its own,
and the only Rust TAK decoder is GPL-3.0, which would lock the project
into that licence. The chiptune formats, which play a console's sound
chips rather than samples, are left out too: SPC, NSF, GBS, VGM and the
PlayStation sound formats would each need a whole console emulated.

A file manager opens a track in the player for the formats the desktop
has a mimetype for: FLAC, MP3, M4A, AAC, Ogg Vorbis, Opus, WAV, AIFF,
Monkey's Audio, WavPack, TTA, Musepack, Shorten, AMR and Matroska audio.
shared-mime-info knows no mimetype for the game rip formats, nor for CAF
and MP1, so no file manager offers the player for those: name the file on
the command line or drop it on the window instead.

## The command line

`blue-dove --help` lists the flags. Two things are worth knowing beyond the
list:

`blue-dove /path/to/song.flac` opens the file in the **preview**: a small
player with the cover, the track's name and its position, a strip with
repeat, stop and play, and the waveform below. It is the mode a file
manager launches when you open one audio file from it, and it leaves your
playlists alone, so a look at one track never replaces what the **Default**
tab held. A previewed file plays from wherever it is, inside the music
directory or not. Right-click the cover and choose **Open the Window** to
leave the preview, which puts the track in the **Default** tab. Turn the
whole mode off with **View > Preview a single file**.

Naming two or more files, or a directory, opens the full window instead.

`blue-dove /path/to/directory` appends every track under that directory to
the playlist that is showing, the way a double click on the album list
does.

Dropping a file or a directory on the window does what naming it on the
command line does: a directory appends its tracks, and files play. A drop
arrives at a window that is already open, so it never shrinks it into the
preview.

`--play`, `--pause`, `--play-pause`, `--stop`, `--next` and `--previous`
go to the player that is already running, so they work from a script, a
keybinding or a shell prompt. They start nothing: with no player running
they say so and exit 1. Only one may be given at a time.

## Remote control

The player answers MPRIS, the interface a desktop's media keys and tools
such as `playerctl` use, over the session bus. `playerctl status`,
`playerctl play-pause`, `playerctl next`, `playerctl metadata` and the
rest work as they do with any other player, as do the media keys a
desktop maps to them.

The bus is optional: on a host without a session bus, or when another
player already holds the name, the app starts and runs exactly as it does
with one, and only the remote control is missing.

The cover of the playing track goes with the metadata, so a desktop's
media widget shows it. Asking that widget to bring the player up brings
back a window that closing it left out of sight, rather than merely
raising one that was already on screen.

## Playing in the background

File > Keep Playing When Closed leaves the music playing when the window
is closed, so the player can sit out of sight while it plays. Launching
the app again brings that window back rather than starting a second
player, which also means the desktop's launcher and a second click on the
binary both land on the window that is already running.

On Wayland the window is minimized instead, since a Wayland window cannot
be hidden by the program that owns it: the music plays on and the window
comes back from the dock.

File > Quit still ends the app, whether or not that option is on. With
nothing playing, closing the window ends it too: a window that would only
sit hidden has no reason to stay.

The running window is found through a lock file and a loopback socket
rather than the session bus, so this works on a host that has no bus. The
lock is released when the process ends, so a window that crashed does not
leave the app taken.

## The compact window

Shrink the window past a certain size, about 560 by 430 points, and it
drops the album list and the playlist: what is left is the menu bar, the
album art, the controls and a thin waveform. That is small enough to sit
in a corner as a popup while you work. Growing it again brings the rest
back.

The window snaps to its compact size the moment it crosses that line, so
the popup always opens at a size that fits it, and it can be resized
freely from there. The size it was before the snap is not restored, so
drag the window larger to get the album list back.

## Selecting tracks

Click a row to select it. Ctrl-click adds or removes a row, and
shift-click selects everything between the last clicked row and the one
you click. Ctrl+A selects every row the table shows, which a search
narrows along with the table.

Right-click a row for its menu: **Play** starts the selected tracks and
nothing else, in playlist order, without changing the playlist; Next and
Previous then walk those tracks until you play something else.
**Remove from Playlist** takes the selected rows out of the tab, and so
does the Delete key. **Select All** is there too.

Removing a track only edits the playlist. No file is ever deleted,
moved or rewritten: a removed track is still on disk, and opening its
album again puts it back in the list.

## Columns

The track table has six columns: number, title, file name, artist, album
and length. The column titles sit in a strip of their own above the rows
and stay there while the playlist scrolls, so a column can always be
resized.

Drag the border between two columns to give one more room and the other
less; the pointer turns into a resize arrow while it is over a border.
The widths are remembered between runs.

Right-click the column titles for the column menu. Every column can be
turned off, number included, and the table gives its room to the ones
left. The file name column is off to begin with: it shows each file's
own name, which is what a rip whose tags are wrong or missing needs, and
it sits beside the title column rather than replacing it, so a table can
show one, the other, or both. **Reset Column Widths** puts the widths
back as they started.

The columns shrink together when the window is made narrower, so
everything keeps fitting. Only once they reach their smallest width
does the table scroll sideways, and the rows scroll under the titles,
which stay put.

When the window is made wider, the room goes to the columns that hold
text: the number and length columns keep the width they were given,
since a wider one would only put empty space beside a few digits.

## Playback modes

**Playback > Repeat Track**, or the leftmost button on the control strip,
restarts the playing track when it ends instead of moving on.
**Playback > Random Order**, or the rightmost button, shuffles the
playlist once and plays it in that order, rather than picking a track at
every step: every track is heard once before any comes back. A mode that
is on has its button drawn in amber; a mode that is off has an outline
only. Both are remembered between runs.

The shuffle is drawn when Random Order is turned on and follows the
playlist from then on, so Previous and Next walk the same order in
opposite directions: go back a track and forward again and the same track
plays both times. Changing the playlist, or a selection becoming the
queue, draws a new shuffle. A track you start by hand carries on in the
shuffle from where it sits in it.

Selecting a track or pressing Next, Previous or Stop leaves the table
showing the playing track, so the highlighted row is always what you
hear.

## Blind comparison

To find out whether two files really sound different, select exactly two
rows, right-click one of them, and pick **Blind Comparison**. The first of
the two is labelled A and the second B, and the test plays them itself:
**Play A**, **Play B** and **Play X**, where X is one of the two, drawn at
random and never named while the test runs.

Each track is listed with the SHA-256 of its file beside it, eight
characters of it, with the whole sum on hover. Two encodes of the same song
carry the same tags, so the sums are what tells them apart.

Under the two names is a seek bar for the track that is playing, and a
**Start at** field: both tracks start from that moment every time one is
played, so a comparison can be made at the point that matters rather than at
the beginning of the file. The field takes `m:ss` or `h:mm:ss`.

A test runs ten trials. Listen to X beside A and B, say whether X was A or
B, and move on. The test says nothing about whether an answer was right
until the end, since saying so after each trial would give the next one
away. Nine right answers out of ten mean you can tell the two apart, which
is the level a test is usually held to: guessing reaches nine or more 1.1%
of the time, while eight would be reached 5.5% of the time by guessing
alone. The result gives the count, the chance and which side of that line
it falls on.

When the last trial is answered the result is signed. The signature is the
SHA-256 of the log of the test: the two file names with their sums, every
trial with the score as it stood, and the total with its p-value.

**Copy Log** puts that text on the clipboard, signature block included. To
check it, save it as `abx.log` and run:

```sh
awk '/^-- signature --$/{exit} {print}' abx.log | sha256sum
```

The hash printed has to be the line under `-- signature --`. Everything
above that line is what was signed, byte for byte, and the file sums are
the first lines of it: change a trial, a sum or the p-value and the hash no
longer matches, and a signature cannot be moved to another pair of files.

The comparison opens in the middle of the window; drag its title bar to
put it somewhere else. **Run Again** asks ten fresh trials between the same
two tracks, drawn anew, so the second run is not a repeat of the first. The
comparison keeps out of your playlists, and playing a row or opening the
preview closes it.

## Volume

The volume slider sits at the right-hand end of the control strip, below
the album art. Drag it, or roll the wheel over it: each notch moves it by
a twentieth, and the setting is written to the settings file when you let
go. `Ctrl+Up` and `Ctrl+Down` do the same from the keyboard.

## Search

`Ctrl+F` or `/` opens the search box above the playlist. It looks through
the title, the artist and the album of every track in the tab, ignoring
case, and the table narrows as you type: the first match is selected and
scrolled into view, so the result is on screen before you press anything
else.

| Key | Does |
|-----|------|
| `Enter` | The next match |
| `Shift+Enter` | The previous one |
| `Down` | Leave the box for the table, on the next match |
| `Escape` | Close the search and show the whole playlist |

`Enter` wraps around, so it walks the matches rather than stopping at the
last one. Once the box has given up the keyboard, the arrows walk the
table as usual.

Opening an album, switching tabs or adding a directory closes the search,
since what it found belonged to the playlist that was showing.

## Keyboard

| Key | Does |
|-----|------|
| `Space` | Play or pause |
| `Enter` | Play the selected row |
| `Up` / `Down` | Select the row above or below |
| `Shift+Up` / `Shift+Down` | Select the first or the last row |
| `Left` / `Right` | Seek five seconds back or forward |
| `Ctrl+Left` / `Ctrl+Right` | Previous or next track |
| `Ctrl+Up` / `Ctrl+Down` | Raise or lower the volume |
| `Ctrl+F` or `/` | Search the playlist |
| `Ctrl+O` | Manage the library sources |
| `Delete` | Remove the selected rows from the playlist |
| `Ctrl+A` | Select every row the table shows |

Shortcuts are ignored while the search box has the keyboard focus, except
`Escape`, which closes it, and `Down`, which leaves it for the table.

## Themes

**View > Theme** offers five palettes:

| Theme | Follows |
|-------|---------|
| Default | the app's own blue-black and amber |
| Adwaita Light | GNOME's light theme |
| Adwaita Dark | GNOME's dark theme |
| Breeze Light | KDE Plasma's light theme |
| Breeze Dark | KDE Plasma's dark theme |

The choice is remembered between runs. It is independent of the system
theme: the window always draws the palette picked here.

## A translucent window

**View > Translucent background** lets the desktop show through the
window, the way a terminal emulator with its opacity turned down does.
It is on by default and kept in the settings file.

The blend is the compositor's work, so the effect needs one running:
picom, fastcompmgr, or whatever your desktop uses. Without a compositor
the window is drawn opaque, and nothing is lost. Turn the option off if
you would rather have a solid window on a busy wallpaper.

## Where settings live

The music directories, the ones left out of the scan, the volume, the theme,
the two playback modes, the
column widths and which columns are shown, the open playlists and their
tabs, and the window options (translucent, keep playing when closed,
ignore hidden files, scan automatically, fit or fill the album art) are kept
in `config.toml` under the platform's config directory:

| Platform | File |
|----------|------|
| Linux | `~/.config/blue-dove/config.toml` |
| macOS | `~/Library/Application Support/blue-dove/config.toml` |
| Windows | `%APPDATA%\blue-dove\config.toml` |

Deleting the file resets all of it, and Blue Dove asks for a directory
again on the next start.

The library cache and the waveforms are separate, and live under the
platform's cache directory: `library-<hash>.toml` holds the tags, `covers/`
the album art as it was found in each file, `art/` the same art as the PNGs
a desktop's media widget shows, and `waveforms/` the peak envelopes. Art is
decoded behind the window rather than before it opens, so a library of
thousands of tracks opens at once; every cover is queued when the library
opens and decoded behind the window, so scrolling finds them ready, and the
cover on screen is decoded ahead of that queue. Each music directory has a
cache of its own, named after it with symlinks resolved, so a library reached
through a link and the directory it points at share one entry. Deleting that directory is
safe at any time, `covers/` included: the next scan finds the pictures gone
and reads the files that name them again.
