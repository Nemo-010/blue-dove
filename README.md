<p align="center">
  <img src="assets/icon.png" width="128" alt="">
</p>

<h1 align="center">Blue Dove</h1>

<p align="center">A fast music player with an album browser and waveform seekbar</p>

<p align="center">
  <a href="https://pkgforge-apps.github.io/blue-dove/">Documentation</a> |
  <a href="https://github.com/pkgforge-apps/blue-dove/releases/latest">Latest release</a>
</p>

Blue Dove plays the music in the directories you point it at and nothing
else. It reads the tags already in your files, groups them into albums, and
draws the waveform
of the playing track so you can see where you are and jump anywhere in
it. There is no library database to build, no service to sign in to,
and no network access.

It is a native Rust binary drawn with egui, so it starts at once and
stays small in memory. It is an alternative to heavyweight players such
as Clementine, Strawberry and Elisa.

## Features

- Draws Japanese, Chinese and Korean titles, using a system CJK font
  when one is installed.
- Reads FLAC, MP3, M4A and ALAC, AAC, Ogg Vorbis, Opus, WAV, AIFF,
  CAF, Matroska audio, Monkey's Audio, WavPack, TTA, Musepack and
  Shorten, with their tags. Every decoder is pure Rust and compiled
  into the binary, so nothing has to be installed to play them.
- Plays the formats video game rips use: Sony VAG, CRI ADX, and
  Nintendo DSP, BRSTM, BCSTM and BFSTM.
- Repeats the playing track, or shuffles the playlist and plays it in
  that order.
- Browses your directory as a collapsible tree of directories and albums.
- Groups tracks into albums by album artist, album and year, and plays
  them in disc and track order.
- Keeps several playlists open as tabs, each with its own name, and heads
  each album in a playlist with its cover and year.
- Shows album art, embedded or from an image beside the tracks, a
  `covers` or `scans` directory, or the directory above a disc directory.
  A right click on the art fills its square or fits the whole picture,
  for art that is not square.
- Draws the waveform of the playing track across the width of the
  window, and seeks anywhere in it by clicking or dragging, forwards or
  backwards.
- Selects several tracks at once, plays just those, and removes them
  from the playlist with the Delete key. Removing never touches the
  files.
- Searches the tracks of an album as you type, and leaves hidden files
  out of the library unless asked to include them.
- Resizes the track columns by dragging their borders, and shrinks them
  to fit before the table ever scrolls sideways. A right click on the
  column titles turns any column off, number included, and can show each
  file's name beside its tagged title.
- Adjusts the volume by dragging the slider or rolling the wheel over it.
- Reads several directories as one library, each kept in its own part of the
  album list, and leaves any directory out of the scan without moving it.
- Remembers the music directories, the volume, the window opacity and every
  open playlist between runs, so the next start shows the same tabs.
- Caches the tags and the album art as it found it, so a start on a large
  library opens at once and only re-reads the files that changed.
- Shrinks into a compact popup of just the art, the controls and a thin
  waveform, for keeping in a corner while you work.
- Answers MPRIS over D-Bus, so `playerctl` and the desktop's media keys
  control it. Without a session bus it still runs, just without them.
- Keeps playing when the window is closed, if asked to, and comes back
  when launched again rather than opening a second player. Neither needs
  D-Bus.
- Draws a translucent window, so the desktop shows through it, when a
  compositor is running.
- Ships five themes: its own, plus light and dark versions of Adwaita and
  Breeze, so it sits next to GTK and Qt programs.
- Opens a directory with its own browser, so it needs no portal or file
  dialog backend.
- Plays a file or queues a directory from the command line, and takes
  `--play`, `--next` and the rest against a player that is already running,
  so it can drive from a script or a keybinding.

## Install

Download the file for your platform from the
[latest release](https://github.com/pkgforge-apps/blue-dove/releases/latest).

| Platform | File |
|----------|------|
| Linux, any distribution | `blue-dove-anylinux-<arch>.AppImage` |
| Linux, portable | `blue-dove-<arch>-linux.onelf` |
| Linux | `blue-dove-<arch>-linux.tar.xz` |
| macOS | `blue-dove-<arch>-macos.dmg` |
| Windows | `blue-dove-x86_64-windows.exe` |

The `.onelf` file is a single executable that carries its own libraries.
Make it executable and run it:

```sh
chmod +x blue-dove-x86_64-linux.onelf
./blue-dove-x86_64-linux.onelf
```

Run it once with `--onelf-integrate` to add it to your application menu.

The macOS disk image opens to the app and an Applications link; drag it
across. The Windows `.exe` runs as it is, with the C runtime linked in, so
no Visual C++ Redistributable is needed.

The AppImage is a single file too, and needs no FUSE. Make it
executable and run it:

```sh
chmod +x blue-dove-anylinux-x86_64.AppImage
./blue-dove-anylinux-x86_64.AppImage
```

It is an [Anylinux AppImage](https://github.com/pkgforge-dev/Anylinux-AppImages),
which is the one to take if you want the widest reach. It carries its own
glibc, dynamic loader and every library it needs, so it does not care what
the distribution underneath it is:

- **Any libc.** It runs on a glibc distribution and on a musl one such as
  Alpine, since it brings its own C library rather than borrowing the
  host's.
- **Any layout.** It runs on a non-FHS distribution such as NixOS, where
  the usual `/lib` and `/usr` are not there to find.
- **Old kernels.** Its compatibility layer covers kernels older than the
  one the other builds are linked against.
- **No FUSE.** If the kernel cannot mount it, it falls back to namespaces
  and then to extracting itself and running from there.

It is built in an Arch container by `packaging/appimage`, and its own
self-updater is off while an AppImage manager such as `am` or `soar` owns
the file.

## Build from source

```sh
cargo build --release --locked
./target/release/blue-dove
```

Building needs the ALSA headers, `alsa-lib` on Arch and
`libasound2-dev` on Debian and Ubuntu.

## License

[MIT](LICENSE)
