# AGENTS.md

<!-- BEGIN SHARED GUIDELINES: copied from pkgforge-apps/.github by scripts/sync-guidelines.sh. Edit them there. -->

Guidelines for anyone, human or agent, working on a pkgforge-apps
repository. Every app in the organization starts from the
[app template](https://github.com/pkgforge-apps/app-template) and follows
these rules, so the repositories stay consistent with each other.

These guidelines, the shared skills, the shared scripts and the CI and
release workflows live in
[pkgforge-apps/.github](https://github.com/pkgforge-apps/.github). Each
app carries a copy that `scripts/sync-guidelines.sh` refreshes. When
they change there, pkgforge-apps-bot opens a pull request in every app
with the update. Edit them there,
never in an app; an edit made in an app is overwritten by the next sync.
`.agents/shared-guidelines.lock` lists every shared file.

## Purpose

pkgforge-apps builds lightweight native desktop apps: original ones,
and alternatives to heavyweight apps, Electron apps in particular. Each
app is a single Rust binary. Small binary, fast start, low memory: if a
change works against those, it needs a strong reason.

### Choosing the GUI library

- [egui](https://github.com/emilk/egui), through eframe, is the default
  and what the app template is built on. It is immediate mode and
  mature: a good fit for tools, utilities, viewers and dashboards.
- [smudge](https://github.com/QaidVoid/smudge) is designed for
  general-purpose apps: a retained widget tree, fine-grained reactive
  signals, and repainting only what changed, which keeps an idle app
  idle. Prefer it for conventional desktop apps with many views, forms
  or documents. It is still in development: its API changes without
  notice and it is not on crates.io yet, so check its state and agree on
  it with the maintainer before starting an app on it.

Pick one per app at the start; do not mix them.

## Platforms

- Every app runs on Linux, macOS and Windows unless its function is tied
  to one OS. Say so in the README when it is.
- Put OS-specific code behind `#[cfg(target_os = "...")]` in its own
  module, with a fallback or a clear "unsupported" path on the others.
- Never hard-code path separators, line endings or config locations.
  Use `std::path`, and the platform conventions for config, data and
  cache directories.
- CI tests on all three platforms. A change that only compiles on yours
  is not done.
- An app that links a system library names its Debian packages, one per
  line, in `.github/apt-packages.txt`, and the jobs that build on Ubuntu
  install them first. The jobs that build in an Arch container read the
  same list in `.github/pacman-packages.txt`, with Arch's package names.
  Both files take comments. Keep them to what the build needs: the
  release bundles carry their own libraries, so this is for compiling,
  not for running.

## Workflow

- Commit one logical change at a time. Never commit a broken state.
- Run `scripts/check.sh` before every commit. It runs what CI runs.
- Never bump the version or edit `CHANGELOG.md` by hand. release-plz
  owns both.

### Commit messages

Semantic, imperative, one line, at most 68 characters:

```
type: imperative message
type(scope): imperative message
```

Types: `feat`, `fix`, `perf`, `refactor`, `docs`, `test`, `style`,
`chore`, `ci`, `revert`. Use `feat!:` or a `BREAKING CHANGE:` footer for
breaking changes. Keep any body short. The commit types decide the next
version and the changelog groups, so pick them carefully:

- `feat: add a recent files menu`
- `fix(editor): keep the cursor on undo`
- `perf: cache decoded thumbnails`

## Code

- Write idiomatic Rust, edition 2024, on the latest stable toolchain.
- Document every item: modules (`//!`), functions, structs, enums,
  variants, fields and constants (`///`). Doc comments say what a thing
  is for and any contract a caller must keep. CI enforces this through
  `missing_docs` and `clippy::missing_docs_in_private_items`.
- No narrative comments. Do not restate what the code does. An inline
  comment is for a non-obvious *why*: a platform quirk, a workaround, a
  constraint that is not visible from the code.
- ASCII only, in every file: source, docs, configs, scripts. Write
  glyphs in Rust strings as `\u{..}` escapes. `scripts/check-ascii.sh`
  enforces it.
- Do not over-engineer. No abstraction, trait, generic or config knob
  for a case that does not exist yet. Three similar lines beat a
  premature helper.
- Prefer `Result` with context over panics. `expect` is for invariants
  that cannot fail, with a message that states the invariant.
- Scripts are POSIX `sh` (`#!/bin/sh`), never bash. They must pass
  `shellcheck -s sh`.

### egui apps

- Keep state and logic apart from drawing. Logic lives in plain types
  with plain methods that tests can call; `ui` functions read that
  state and turn input into calls on it.
- Never block the UI thread. Run I/O and heavy work on a
  `std::thread`, send results back over a channel, and call
  `ctx.request_repaint()` when they arrive. Reach for an async runtime
  only when the work is genuinely async-heavy.
- Repaint on demand. Do not request continuous repaints unless
  something is animating.
- The renderer is glow (OpenGL). Keep eframe's default features off and
  enable only what the app uses.
- Keep the `--smoke` flag working. CI and the release pipeline launch
  every build with it to prove the app opens a window and draws.

### smudge apps

- The template is built on egui. A smudge app starts from it and
  replaces `src/` and the eframe dependency; everything else stays,
  including the `--smoke` flag, which must draw a frame and exit 0.
- Until smudge is on crates.io, depend on it by git, pinned to a commit
  with `rev`. Move the pin deliberately, one change at a time, and read
  smudge's commits since the last pin first, since its API breaks
  without notice. Switch to crates.io once it is published.
- Follow smudge's own patterns: state in signals, views derived from
  them, and no work on the UI thread that could block a frame.
- Test views with smudge's headless harness, which runs the same frame
  pipeline a window does.

## Dependencies

- Add and change dependencies with `cargo add` and `cargo remove`,
  never by editing `Cargo.toml` by hand. Use `--no-default-features`
  and list only the features the app uses.
- Vet a crate before adding it. The `add-dependency` skill has the
  procedure. In short:
  - The latest release or commit must be under a year old.
  - An older crate is acceptable only when it is clearly complete,
    with no unanswered bug reports or stale PRs piling up.
  - Check what it pulls in with `cargo tree`. A large tree for a small
    feature is a reason to say no.
- Prefer inlining over a dependency when the functionality is a few
  dozen lines. A small, tested function in the app beats a crate.
- Prefer pure-Rust crates. A crate that links a C library adds a system
  dependency on every platform.
- No git dependencies and no `[patch]` sections. Releases build from
  crates.io. The one exception is smudge, until it is published.

## Tests

- Test behavior: what a user or caller can observe. A test should fail
  when the behavior breaks and pass through any refactor that keeps it.
- Test logic in unit tests next to the code. Test UI interaction when
  the behavior lives in the UI (clicking, typing, focus, layout that
  matters): with `egui_kittest` in an egui app, with smudge's headless
  harness in a smudge app.
- Every bug fix comes with a test that fails without the fix.
- Do not write tests that only restate the code: asserting a constant,
  that a constructor sets fields, or that a mock was called.
- Tests run on all three platforms. Use temporary directories, never
  fixed paths, and never depend on test order or wall-clock timing.

## Releases

Releases are fully automated by
[release-plz](https://release-plz.dev) on `GITHUB_TOKEN` only. No
personal tokens and no registry secrets; the `release` skill explains
the pipeline. In short:

1. Every push to `main` refreshes a release PR owned by
   github-actions[bot], with the next version and changelog.
2. Merging it tags the version and creates the GitHub release as a
   draft.
3. That starts `release.yml`, which builds and attaches the assets, then
   publishes the release. The organization makes published releases
   immutable, so assets go on while it is still a draft:

| Platform | Asset |
|----------|-------|
| Linux, portable | `<app>-<arch>-linux.onelf` |
| Linux, AppImage, opt-in | `<app>-anylinux-<arch>.AppImage` and its `.zsync` |
| Linux | `<app>-<arch>-linux.tar.xz` |
| macOS | `<app>-<arch>-macos.dmg` with an `.app` bundle |
| Windows | `<app>-x86_64-windows.exe` |

Only the plain Linux build is an archive, since it carries a desktop
entry and icons. The macOS disk image opens to the app and an
Applications link, the Windows executable runs as is with the C runtime
linked in, and the `.onelf` and `.AppImage` are single files. No
checksum files are published; every asset carries a build provenance
attestation instead.

The onelf bundle uses the stipplefs payload. An egui app's bundle
carries Mesa, so it draws on hosts without a usable GL stack, musl
distributions included; NVIDIA's driver still comes from the host. It
is the Mesa [onelf-pkgs/mesa](https://github.com/onelf-pkgs/mesa) builds
without LLVM, which adds about 8 MB; do not drop it to save space. A smudge
app drawing with its CPU renderer needs no GL, so it turns `gl` and
`dri` off in `onelf.toml`; confirm with the `test-linux-bundle` skill.

The onelf bundle updates itself with `--onelf-update`, downloading only
what changed since the copy it replaces. Once the app has its update key
it accepts only releases signed with it; the `release` skill covers how
the key is made and used.

Run the `test-linux-bundle` skill before merging any change that can
alter what the bundle loads: `onelf.toml`, a dependency that links or
opens a system library, an onelf upgrade, or a feature that reaches a
new part of the system such as audio, file dialogs or notifications.
The `onelf` skill covers inspecting and debugging a bundle.

An app may also ship an Anylinux AppImage, for users and managers that
expect that format, by setting `appimage = true` under
`[package.metadata.pkgforge]` in `Cargo.toml`. It is a second format,
not a replacement for the onelf bundle. The release pipeline builds it
with `scripts/package-appimage.sh`; the `anylinux-appimage` skill
covers it. An app whose stack quick-sharun cannot detect, such as the
audio servers reached through ALSA plugins, names the `DEPLOY_*`
variables it needs in `packaging/appimage/deploy.env`.

Apps are not published to crates.io by default. When one should be,
publish it through crates.io trusted publishing, never with a token
stored in repository secrets.

## Docs

- The docs site lives in `docs/`, built with VitePress and bun, and
  deploys to GitHub Pages from `main`.
- The site is personalized for the app: its own palette, type and
  home page. Do not ship the stock VitePress look. The `app-docs` skill
  covers it.
- Update the docs in the same change as the behavior they describe.

## Icons

`assets/icon.svg` is the single source for every icon, or
`assets/icon.png` when the icon is pixel art. Regenerate the rasters, the
Windows `.ico` and the docs favicons with `scripts/icons.sh` after
changing it. The `app-icon` skill covers the design rules.

## Layout

```
.
|-- src/
|   |-- main.rs          entry point, window options
|   `-- app.rs           application state and root UI
|-- assets/
|   |-- icon.svg         icon source, or icon.png for pixel art
|   |-- icon.ico         Windows icon, generated
|   `-- icons/           PNG sizes, generated
|-- packaging/
|   |-- linux/app.desktop
|   |-- appimage/deploy.env  optional DEPLOY_* variables
|   `-- macos/Info.plist
|-- docs/                VitePress site
|-- scripts/             shared, except bootstrap.sh in the template
|   |-- check.sh         every CI gate
|   |-- check-ascii.sh   the ASCII rule
|   |-- icons.sh         icon generation
|   |-- package.sh       Linux archive, macOS disk image, Windows exe
|   |-- package-onelf.sh portable Linux bundle
|   `-- sync-guidelines.sh  refresh every shared file
|-- onelf.toml           onelf recipe
|-- .release-plz.toml
|-- .agents/skills/      shared and app-specific skills
|-- .claude/skills/      a symlink per skill, for Claude Code
`-- .github/workflows/   shared callers: ci, release-plz, release,
                         docs
```

## Skills

Procedures live in `.agents/skills/`. Claude Code only looks in
`.claude/skills/`, so each skill also has a symlink there, which
`scripts/sync-guidelines.sh` creates. These are shared by every app:

- `add-dependency` vets and adds a crate, or decides to inline it.
- `anylinux-appimage` packages the app as an Anylinux AppImage.
- `app-icon` designs the icon and regenerates every raster.
- `app-docs` personalizes the docs site.
- `release` explains and troubleshoots the release pipeline, including
  crates.io trusted publishing.
- `onelf` inspects and debugs the portable Linux bundle.
- `test-linux-bundle` smoke-runs a bundle on the hosts users have.
- `sync-guidelines` refreshes the shared guidelines and skills.

The template also carries `bootstrap-app`, which turns a fresh copy into
a new app and is removed once that is done. An app may add skills of its
own; give them names the shared ones do not use, since a sync replaces a
shared skill of the same name.

<!-- END SHARED GUIDELINES -->

## This app

Blue Dove is a music player for a directory of audio files. It scans the
directory, reads tags with `lofty`, decodes with `rodio`, and draws with
egui. It has no database: the directory is the library, rescanned at each
start.

Modules, in `src/`:

| Module | Holds |
|--------|-------|
| `app.rs` | application state and the logic behind it |
| `ui.rs` | every drawing function and the input it turns into calls |
| `library.rs` | scanning, tag reading, album grouping |
| `cover.rs` | album art decoding and scaling |
| `cache.rs` | the on-disk cache of the scanned library |
| `mpris.rs` | the MPRIS service, which lets playerctl and media keys drive it |
| `instance.rs` | the one-window lock and the socket a second launch asks through |
| `decode.rs` | opening an audio file with the decoder its format needs |
| `game/` | the decoders for video game rip formats |
| `opus.rs` | the Ogg Opus reader, which symphonia lacks |
| `player.rs` | the rodio device, player and seek calls |
| `waveform.rs` | the peak envelope of a track, computed off the UI thread |
| `dialog.rs` | the directory chooser, through rfd with zenity as the fallback |
| `config.rs` | the settings file, playlists included |
| `theme.rs` | the palette and type scale |
| `fonts.rs` | the CJK fallback font, loaded from the system when present |
| `format.rs` | duration and count formatting |
| `error.rs` | the `At` trait, which puts a path on a decoder's error |
| `cli.rs` | the command line: flags, paths and the music directory they imply |
| `abx.rs` | the blind comparison: ten trials, scored against guessing |

Platform notes:

- Audio goes through cpal's ALSA backend, so the build links
  `libasound`: `alsa-lib` on Arch, `libasound2-dev` on Debian and
  Ubuntu. This is the only system dependency.
- The ALSA plugins that reach PulseAudio and PipeWire are loaded by
  name from a directory compiled into libasound. The AppImage gets them
  from quick-sharun's audio deployment; `onelf.toml` lists them in
  `include` with `/usr/lib/alsa-lib` on `search-paths`, since the
  library scan cannot see them.
- Every decoder is pure Rust: symphonia through rodio for the common
  formats, and `decode.rs` for the ones symphonia lacks (Monkey's Audio,
  WavPack, TTA, Musepack, Shorten, AMR). Those decode a file whole into
  memory. DSD is not supported, and neither are TAK and WMA: the only
  Rust TAK decoder is GPL-3.0, and this app is MIT.
- AMR comes from `rvoip-codec-core`, which is bit-exact against the 3GPP
  reference. Its lib is named `codec_core`, not after the package, so the
  import is `codec_core::..`. Only the `amr` feature is on, so the G.729,
  Opus and G.711 codecs are not compiled in. The storage format's magic
  number picks narrowband (8 kHz) from wideband (16 kHz); a comfort-noise,
  no-data or lost frame carries no mode of its own, so it is decoded in the
  mode the last speech frame used.
- `opus.rs` streams instead of decoding up front: it hands rodio one
  packet at a time, so `open` costs a read of the file's tail for the
  length, not a decode of the file. Decoding whole took 789 ms on a
  3-minute track and 1.4 s on a 6-minute one, all of it before the first
  sample played, which is why Opus tracks lagged the others. Its seek
  uses the Ogg reader's page bisection and then drops the samples it
  decoded ahead of the target, which lands exactly; a seek costs about
  5 ms. `discard` counts interleaved samples, not frames, so it has to be
  scaled by the channel count.
- Game rips have their own decoders in `game/`: Sony VAG, CRI ADX and
  Nintendo DSP, BRSTM, BCSTM and BFSTM. They are small headers over an
  ADPCM stream, so they are written by hand rather than pulled from a
  crate. A rip carries no tags, so `library.rs` lists it under its file
  name, and its length stays unknown until it plays.
- Album art comes from `cover.rs`, which decodes JPEG with `zune-jpeg`,
  PNG with `png` and SVG with `resvg`, then scales the result. eframe
  pulls `image` in on its own, so the crate is still in the tree; the
  app does not use it.
- A file's own art wins. Failing that, `library.rs` looks beside the
  tracks: the conventional names first, then an image named after
  `cover`, `front`, `folder` or `album`, then the only image there. A
  `covers`, `artwork` or `scans` directory is searched too, and a disc
  directory takes the art from the directory above it, where a multi-disc rip
  keeps one cover for the set.
- The directory chooser goes through `rfd`, which asks the XDG file-chooser
  portal and falls back to `zenity`. Neither is required to build; both
  are optional at runtime, and the window says what to install when
  neither answers. zenity is deployed into the AppImage.
- The scan leaves out files and directories whose name starts with a dot
  unless **View > Ignore hidden files** is turned off; it is on by default.
  `read_library_with` filters the walk with `filter_entry`, and the root is
  never filtered by its own name, since a music directory may sit under a
  dotted one. The setting lives in `Config::ignore_hidden` and changing it
  starts a scan again.
- The scan result is cached under the user's cache directory:
  `library-<hash>.toml` holds the tags, one file per music directory so
  two of them keep a cache each, `covers/` holds the decoded art, named
  by a hash of its pixels, and `waveforms/` holds the peak envelopes the
  seek bar draws. Decoding art is what made a scan slow
  (about 4.6 s of 6.2 s on seven thousand tracks), so the cache is what
  makes a start quick; a start shows the cached library and scans behind
  it. An entry is used only while the file's size and modification time
  match, so a retagged file is read again. Bump `cache::VERSION` when the
  stored shape changes, or an older file will be misread.
- Settings live in `config.toml` under the user's config directory.
  Playlists are stored there by file path, not by library index, because
  the library is rescanned at every start. They are written as they
  change and again on exit, so a killed window loses nothing; a smoke
  run writes nothing at all.
- Ten themes: the app's own palette, Adwaita and Breeze in both variants,
  Nord, Catppuccin Mocha and Latte, and Solarized Dark and Light. A stored
  theme is what the window shows; only a first run with none stored asks
  the desktop, through `Theme::first_run`. KDE gets Breeze Dark, since its
  programs look wrong in an amber palette beside a Breeze window, and every
  other desktop keeps the app's own dark theme. `Theme::first_run` reads
  `XDG_CURRENT_DESKTOP`, and `Theme::for_desktop` is the pure part the test
  drives.
- `mpris.rs` serves MPRIS on the session bus, so playerctl and the media
  keys work. The bus is optional: the service is connected on its own
  thread, and a host without a bus, or with another player holding the
  name, leaves the app running with only the remote control missing. A
  command wakes the window through a waker, since the app repaints on
  demand and would otherwise not notice until the mouse moved.
- Random Order is one shuffle of the playlist, held in `App::shuffle`, not
  a fresh draw at every step: every track is heard once before any comes
  back, and Previous then Next plays the same track again. The order is
  drawn the first time it is needed and dropped by `forget_shuffle` when
  the playlist or queue changes, so it always fits what is being walked.
- With "Keep Playing When Closed" on, closing the window hides it and the
  music plays on; the audio is on its own thread, so nothing stops. The
  rule lives in `keeps_playing_on_close`, which also says Quit and a
  close with nothing loaded still end the app. A hidden window gets no
  drawing pass, so everything the player does between frames runs in
  eframe's `logic` hook: `App::logic` polls the workers, the remote
  control and launches, and calls `advance`, which hands over to the next
  track when one ends. A track that ran out would otherwise sit there.
- `cli.rs` parses the arguments before a window exists, so a bad one is
  reported on the terminal. `--help` and `--version` answer and exit;
  `--play`, `--pause`, `--play-pause`, `--stop`, `--next` and `--previous`
  become an `Action` and travel to the running window as a
  `cli::Action::command()`, which is the same `mpris::Command` the bus
  uses, so the two cannot drift. Paths travel too: `App::open_paths` puts
  a file in the tab named Default (creating it when there is none) and
  replaces what it held, and appends a directory's tracks to the showing
  playlist. A path that arrives before the scan has finished waits in
  `App::pending_paths` and is applied when the scan lands.
- A launch naming exactly one audio file opens the **preview**: the art, the
  line naming the track, and a strip with only repeat, stop and play, with
  the waveform below and no menu bar. `App::preview_candidate` decides what
  qualifies (one path, a file, an extension the decoder takes); the setting
  in **View > Preview a single file**, on by default, decides whether it
  applies. The file is read by `library::read_one`, not looked up in the
  library, because a file manager names files that need not be under the
  music directory; the track lives in `App::preview` and in no playlist, so
  a preview never replaces what the Default tab holds. Right clicking the
  art leaves the preview for the window, which puts the track in the Default
  tab; a file the library does not hold cannot go there, since a save stores
  paths the next scan has to find again. `advance` handles the preview before
  the playlist walk, so a track that runs out repeats or stops rather than
  stepping to a track that is not there. `open_pending_paths` takes the
  preview path before it looks at the scan: the file is read on its own, so
  waiting for a library that need not contain it would leave the full window
  on screen while eight thousand files were read.
- The preview is a **fixed-size** window: `App::pin_window` sets its minimum
  and its maximum to the same size and sends `Resizable(false)`, so a drag on
  a border and a window manager honouring size hints both refuse. The size is
  `COMPACT_SIZE` when the file carries art and the shorter
  `PREVIEW_NO_ART_SIZE` when it does not; the answer is kept in
  `App::preview_art`, read from the file when the preview opens, so the
  layout inside a window that cannot shrink cannot change either.
  `exit_preview` lifts the pin before the full layout lands: the regular and
  compact layouts stay resizable, and a window left pinned would stay fixed
  for the rest of the run. A track with no cover draws no square at all: an
  empty box the size of a cover would be the largest thing in the window and
  say nothing. The menu a right click on the art gives, which is also the way
  out of the preview, is offered on the name line when there is no art.
- `main.rs`'s `window_viewport` sets a maximum size for the preview alone, and
  the full window asks for none rather than for an infinite one: Wayland
  carries the max size as a `u32` on the client side and as an `i32` in the
  protocol, so infinity arrives as -1, a size xdg-shell refuses, and a
  compositor enforcing that, mutter among them, closes the window instead of
  opening it.
- A file opened through `instance::Request::Open`, which is what a file
  manager sends when the player is already running, previews exactly like a
  fresh launch: the two paths meet in `App::take_request`. Anything else
  arriving that way is a session, so `open_paths` calls `exit_preview` first
  and the window gets its floor and its size back before the content lands.
- Files dropped on the window from a file manager go through the same
  `App::open_paths` a command line path does, so a drop and a launch of the
  same path behave alike. The drop is read from `raw.dropped_files` in
  `logic`, which runs even while the window is hidden.
- `instance.rs` makes a launch find the window that is already running
  instead of opening a second one. It takes an exclusive `try_lock` on a
  file in the runtime directory (the cache directory stands in where that
  is missing) and listens on a loopback socket whose port it writes
  beside the lock; a second launch reads the port, sends its
  `instance::Request` (show, an action, or paths to open), and exits 0.
  A request is text: a verb on the first line, then one path per line, so
  a path holding a line break is refused when the arguments are parsed. The kernel drops the lock when the process ends, so
  a crashed window does not leave the app taken. No D-Bus is involved,
  which is what makes it work on a host without a session bus.
- Previous restarts the playing track while it has played for more than
  `RESTART_AFTER` (three seconds, the usual few), and moves to the track
  before it only when the press comes earlier or after such a restart.
  `back_restarts` holds that rule, so it is testable without a device. The
  MPRIS Previous command goes through the same `step`.
- The track table has a multi-row selection in `View::selection`, with
  `View::selected` as the anchor a shift-click measures from. Playing a
  selection does not rewrite the playlist: it becomes `App::queue`, which
  Next, Previous and Random walk until a row is played by hand. Removing
  rows edits `playlist.tracks` and nothing else: the app never deletes,
  moves or rewrites an audio file, and a test pins that.
- A blind comparison, **Playback > Blind Comparison**, takes exactly two
  selected rows, the first as A and the second as B, and lives in
  `App::abx` and `abx.rs`. It is kept out of the playlists and the table
  on purpose: the table highlights the playing row, and a highlight would
  say which of the two X is. For the same reason `advance` leaves the
  player alone while a test is open, and playing a row or opening the
  preview ends the test. `App::abx_seed` mixes the clock into
  `random_state` so a restart does not repeat the sides the run before it
  drew. Nine right answers of ten pass, the smallest count under the
  usual 5% level: guessing reaches nine or more 1.1% of the time, eight
  5.5%.
- The search box is `View::searching` and `View::filter`, drawn above the
  table by `ui::search_bar`. `Ctrl+F` or `/` opens it and `View::search_focus`
  asks for the keyboard once, so the box does not take it back every frame.
  `App::refilter` selects the first match as the text changes, and
  `App::step_match` walks them on Enter, wrapping at the end. `Down` in the
  box surrenders the keyboard and steps into the table, which is how the
  results are walked with the arrows. Opening an album, switching tabs or
  adding a directory calls `close_search`, since a filter left over from
  the old playlist would hide the new one. `visible_tracks` is the one
  place the filter is applied, so the table, the selection walk, select-all
  and the range selection all agree on what is shown.
- A shortcut is matched by `chord`, which reads the modifiers off the key
  event and consumes the press. `input.modifiers` is the state when the
  frame runs, and a quick press has released control by then, so a chord
  read from it arrives as a bare letter and is missed; the plain arrows
  never worked as volume keys for the same reason. The arrows walk the
  table, `Shift` jumps to its ends and `Ctrl` changes the volume.
- Double-clicking a directory appends what it holds to the playlist:
  `Directory::tracks_deep` walks it in the order the album list draws it,
  so each album's tracks in playing order, then the loose files beside
  them, then each subdirectory. A directory of loose files alone queues
  those, since it holds no album.
- The **Add All** button in the Library header appends every album in the
  library to the playlist, in the order the album list draws them:
  `Library::album_tracks` walks the tree the way `tracks_deep` does but
  stops at the albums, so the loose files, which belong to none, stay out.
- Both long lists draw only the rows on screen. The track table does it
  from `ui::table_rows`, and the album list from `ui::LibraryRow`, a
  flattened tree built once a frame (indices and paths, no borrows) with
  `library_row_tops` giving each row its place. The album list used to lay
  out and paint every album on every frame, which made a frame cost more
  the larger the library was: 520 albums came to 0.85 ms of CPU per frame
  and now cost 0.32 ms, nearly flat as the library grows. Rows carry
  paths rather than a borrow of the tree, so `App::add_directory_at` and
  `App::toggle_collapsed_at` take the path a click gives.
- The peak envelope itself is kept in the cache directory under
  `waveforms/`, one file per track named by a hash of its path, size and
  modification time, so a retag or a re-encode gets a fresh entry. Peaks
  are stored a byte each: the bar is drawn a point at a time, so a 256th
  of the loudest peak is finer than a pixel, and an envelope is about
  1.6 KB. `waveform::spawn` reads a stored envelope and sends it before
  it returns, so a track played before draws its bar in the frame it
  starts; decoding it took 51 ms on a short track, which is what left the
  bar empty at the start of every play. `MAX_WAVEFORMS` bounds the
  directory, oldest dropped first. No database: the files sit beside the
  covers the same way.
- The waveform's bar heights are cached in `App::wave_columns`, keyed on
  the width, the bar's height and the number of peaks. Rebuilding a few
  hundred of them is cheap but not free, and the drawing pass runs at
  50 fps while a track plays.
- While a track plays the window repaints every 20 ms, so the seek bar's
  cursor moves smoothly; a short track moves it across the bar quickly,
  and a slower rate showed as jumps. The MPRIS position poll stays at
  250 ms, which is all a client reading the position needs.
- Below `COMPACT_BREAK` the window drops the album list and the playlist
  and shows only the menu bar, the art, the controls and a thin waveform.
  It snaps once, when it crosses into that layout, so the window manager
  is not fought while the user drags; growing it back past the break
  restores the full layout. The snap is `COMPACT_SIZE` when the track has
  art and the shorter `COMPACT_NO_ART_SIZE` when it does not, so a missing
  square leaves no empty height behind. The popup stays resizable but the
  size it picked is also its floor, so it can be made larger and not
  smaller: below that the controls and the waveform are squeezed out of a
  window with no room for them. The ceiling is lifted with it, so growing
  back past the break still returns the full layout, and the floor drops to
  `COMPACT_MIN_SIZE` again on the way out, which is what lets the window be
  dragged back down into the popup. The art keeps
  its full width in the popup, and
  the window is made tall enough for the line naming the track
  (`compact_name`: the name, or the file name, then its position, both in
  the accent) between the art and the controls; shrinking the art to fit
  the line would defeat the point of the popup.
- The track table's columns are `ui::Column`, in drawing order: number,
  title, file name, artist, album and length. `column_layout` in `ui.rs`
  takes the six stored widths and `App::columns_shown`, and scales the
  shown ones together to the panel; each stops at its own minimum
  (`Column::min_width`, narrower for the number), and only past that does
  the table keep its width and scroll sideways. A hidden column gets no
  width and takes no part in the scaling, so the others take its room.
  `column_starts` is shared by the header and the rows, so a label cannot
  drift away from the values under it. A drag moves width between the two
  columns a border separates, which keeps their total fixed and the border
  under the pointer at any scale.
- The table is laid out as `ui::TableRow`s, not one row per track:
  `table_rows` heads each run of an album's tracks with `TableRow::Album`,
  which draws the cover, the album name with its year, its artist and a
  rule to the right edge. A run that starts again after another album is
  headed again, and a loose file heads nothing. The headings are taller
  than the rows (`ALBUM_ROW_HEIGHT`), so `row_tops` gives each row its
  place and the drawing and the reveal scroll both read from it rather
  than multiplying a row index by one height. `Library::album_indices`
  is what says which album a track belongs to, built once per frame.
- The column titles sit in a strip above the scroll area rather than in
  it, so they stay in reach while a long playlist scrolls; the strip
  follows the rows sideways by the scroll offset. A right click on it
  opens the column menu, where every column can be turned off and the
  widths can be put back. The title column holds the tags' titles and the
  file name column holds the files' own names; they are separate columns
  so a directory with bad tags can be read by name without losing the title.
- The playing row is drawn entirely in the accent, every column of it, and
  carries its position after the track's name. `name_column` decides which
  column that is: the title, or the file name when the title is off. The
  position used to hang off the title alone, so a table showing file names
  instead lost it.
- The album art sits at the top of the left-hand column, and the control
  strip below it carries the repeat button, the transport, the random
  button and the volume slider, in that order. The volume takes whatever
  width the buttons leave. The waveform below spans the whole width.
- The art's square is filled one of two ways, chosen by a right click on
  it and kept in `Config::cover_fit`: cropped (`square_crop`, the default,
  which fills the square and loses the edges of art that is not square) or
  fitted whole (`fit_rect`, which keeps every pixel and leaves the space
  beside it as background).
- CJK titles need a system font with those glyphs; `fonts.rs` searches
  the system and user font directories for a font whose name says it
  carries them, preferring a full CJK face to a Japanese one, and carries
  none itself, to keep the binary small. A list of fixed paths was not
  enough: stock Arch and Debian install `DroidSansFallback`, which those
  paths did not name, so every Japanese title drew as boxes.
- The directories are `XDG_DATA_HOME/fonts` and `XDG_DATA_DIRS/fonts`,
  with the spec's `/usr/local/share:/usr/share` standing in when the
  latter is unset or empty, plus the macOS and Windows locations. Reading
  the environment rather than a fixed path is what makes the AppImage
  work: quick-sharun rewrites the `/usr/share` literal in the binary to
  point inside the image, and the image's share directory carries no
  fonts, so a compiled-in path leads nowhere. The environment still names
  the host's directories at run time, so the fonts the user has are the
  ones that get used. Confirmed with strace on the shipped image: the
  compiled-in path opened `/tmp/<random>/fonts` and found nothing, and
  this one opens `/usr/share/fonts/droid/DroidSansFallbackFull.ttf`.
- Fontconfig is not consulted. It would mean parsing its configuration or
  linking a C library, and the XDG variables say the same thing in two
  lines; egui does its own font handling, so a font database would be a
  whole dependency for one path lookup.
- eframe's `accesskit` feature is off. Its atspi and zbus bindings cost
  around 1.5 MB, which the size budget for this app does not allow, so
  the window has no screen-reader support.
- The release binary is kept under 14 MB. Weigh it after a change that
  adds a dependency or a decoder; `strip`, fat LTO, `panic = "abort"`
  and one codegen unit are already on, and `opt-level` stays at the
  default rather than `"s"`.

The AppImage workflow is app-specific (`appimage.yml`). It runs
`packaging/appimage/get-dependencies.sh` first, which adds the ALSA and
PulseAudio packages the shared setup action leaves out, then the
debloated packages, then `scripts/package-appimage.sh` with
`DEPLOY_PULSE` set, so quick-sharun deploys the plugin cpal reaches
through libasound. It smoke-runs the image, and produces
`blue-dove-anylinux-<arch>.AppImage` and its `.zsync`, named for what the
file is: an image that runs on any distribution, not one built for a
particular one.

The image carries its own glibc, `ld-linux` and libraries, so it depends
on neither the host's libc nor an FHS layout. Verified by running the
shipped image's `--version` in an `alpine` container, which has only the
musl loader, and in a `nixos/nix` one, which has no `/etc/os-release` and
no glibc loader; both printed the version and exited 0. Keep the wording
in `README.md` and `docs/guide/installation.md` to what the image does,
not to what the build host is.

It starts on `workflow_run` of the `release` workflow, and that covers a
release only when a person dispatched it. release-plz dispatches the
release workflow with `GITHUB_TOKEN`, and GitHub starts no workflow for an
event a `GITHUB_TOKEN` produced, the completion included, so a release cut
by release-plz publishes with no image until this workflow is dispatched
by hand with its tag. `release.yml` is a synced copy of the shared one and
cannot carry a second job, so a real fix belongs in the shared
`app-release.yml`.

PipeWire is not deployed. cpal reaches the sound server through an ALSA
plugin, and the image's default PCM is the pulse one, because
`99-pulseaudio-default.conf` sorts after `99-pipewire-default.conf`. That
plugin serves a PulseAudio host and a PipeWire host running
pipewire-pulse, which is the usual desktop setup. Deploying pipewire
too would add its client library and SPA plugins, about 2.9 MB, that no
config selects.

The app does not set `appimage = true` in `Cargo.toml`: the shared
release job installs neither of those packages and cannot pass
`DEPLOY_PULSE`, so its image would carry no audio plugin. That is
why the images are attached by the app's own workflow after the shared
one has published the release, which needs a published release to still
take assets: the organization's immutable releases are off. Turning them
back on would leave every release without an AppImage, so this workflow
would have to move into the shared release job instead.

`.github/apt-packages.txt` names the Debian packages the build needs and
`.github/pacman-packages.txt` the Arch ones, which the jobs building in an
Arch container read. Both are what keep `alsa-sys` from failing: on the
Ubuntu runners, and in the onelf and AppImage containers.

