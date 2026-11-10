//! A music player that reads the tags in a directory and plays it.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod abx;
mod app;
mod art;
mod cache;
mod cli;
mod config;
mod cover;
mod decode;
mod dialog;
mod digest;
mod downmix;
mod error;
mod fonts;
mod format;
mod game;
mod instance;
mod library;
mod mpris;
mod opus;
mod player;
mod playlist;
mod software;
mod theme;
mod ui;
mod watch;
mod waveform;

use std::fs::File;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;

use app::App;
use cli::Renderer;
use eframe::egui;

/// Window and taskbar icon, generated from `assets/icon.svg`.
const ICON_PNG: &[u8] = include_bytes!("../assets/icons/256.png");

/// The viewport the window opens with, at the size it was given.
///
/// The preview is pinned: its floor and its ceiling are the one size it was
/// given, so a window manager honouring size hints cannot stretch it. The
/// full window has only a floor, and so no ceiling is set at all rather than
/// an infinite one: winit carries the max size to Wayland as a `u32` and back
/// as an `i32`, so an infinite one arrives as -1, a size xdg-shell refuses,
/// and a compositor that checks it closes the window instead of opening it.
fn window_viewport(
    preview: bool,
    start_size: [f32; 2],
    icon: egui::IconData,
) -> egui::ViewportBuilder {
    let viewport = egui::ViewportBuilder::default()
        .with_title(App::TITLE)
        .with_app_id(App::ID)
        .with_inner_size(start_size)
        .with_resizable(!preview)
        // The full window's floor is the compact layout's minimum, so the
        // window can be shrunk into the popup and on down to the size the
        // popup itself allows.
        .with_min_inner_size(if preview {
            start_size
        } else {
            app::COMPACT_MIN_SIZE
        })
        // The window is drawn translucent so the desktop shows through it; a
        // compositor blends it, and without one it looks solid.
        .with_transparent(true)
        .with_icon(icon);
    if preview {
        viewport.with_max_inner_size(start_size)
    } else {
        viewport
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let smoke = args.iter().any(|arg| arg == "--smoke");
    let cli = match cli::parse(args) {
        cli::Parsed::Run(cli) => cli,
        cli::Parsed::Help => {
            println!("{}", cli::USAGE);
            return Ok(());
        }
        cli::Parsed::Version => {
            println!("{} {}", App::TITLE, env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        cli::Parsed::Error(message) => {
            eprintln!("blue-dove: {message}\n\n{}", cli::USAGE);
            std::process::exit(2);
        }
    };

    // Paths win over a transport flag: "blue-dove --play song.flac" means
    // play that song, and a path that opens the window plays anyway.
    let request = if !cli.paths.is_empty() {
        Some(instance::Request::Open(cli.paths.clone()))
    } else {
        cli.action.map(instance::Request::Action)
    };

    // A command belongs to the window that is already running. Only a path
    // with no window to take it opens one; a bare command has nothing to do.
    if !smoke && let Some(request) = &request {
        if instance::send(request) {
            return Ok(());
        }
        if cli.paths.is_empty() {
            eprintln!("blue-dove: no player is running");
            std::process::exit(1);
        }
    }

    let icon =
        eframe::icon_data::from_png_bytes(ICON_PNG).expect("the bundled icon is a valid PNG");
    // A launch naming one audio file opens the preview, so the window starts
    // at the size the preview needs rather than at the full one and then
    // shrinking once the first frame has drawn. The file is read here only
    // for whether it carries art: a track without one gets no square, and
    // the window is only as tall as the name, the controls and the waveform.
    //
    // The preview is a fixed-size window, so it starts unresizable with its
    // size given as both the floor and the ceiling; `App::open_preview` pins
    // it the same way when a launch lands on a window that is already open.
    let preview = App::preview_candidate(&cli.paths).is_some() && !smoke;
    let start_size = if preview {
        let has_art = App::preview_candidate(&cli.paths)
            .and_then(|path| crate::library::read_one(path))
            .is_some_and(|track| track.cover.is_some());
        if has_art {
            app::COMPACT_SIZE
        } else {
            app::PREVIEW_NO_ART_SIZE
        }
    } else {
        app::DEFAULT_WINDOW_SIZE
    };
    let viewport = window_viewport(preview, start_size, icon);
    let show_fps = cli.fps;
    let no_vsync = cli.no_vsync;
    // A smoke run exercises the software fallback unless the launch asked for
    // the GPU path, which is what `--smoke --opengl` is for.
    let renderer = cli.renderer.unwrap_or(if smoke {
        Renderer::Software
    } else {
        Renderer::OpenGl
    });
    let paths = cli.paths;

    // BLUE_DOVE_SOFTWARE is the switch a wrapper can use when it cannot pass
    // --software. A smoke run takes the CPU path unless it asked otherwise.
    let software_only =
        renderer == Renderer::Software || std::env::var_os("BLUE_DOVE_SOFTWARE").is_some();
    if software_only {
        return run_software(viewport, smoke, paths, show_fps, request.as_ref());
    }

    // The child runs the GPU path; the parent waits and falls back if it
    // dies, which catches a segfault or an abort that unwinding cannot.
    if std::env::var_os("BLUE_DOVE_GPU").is_some() {
        return run_gpu(viewport, smoke, paths, show_fps, no_vsync, request.as_ref());
    }

    // The child reports on its stdout once it has a window, so a death
    // before that is a GPU failure. A death after it is the app's own (a
    // kill, an out-of-memory, a crash after hours) and must not restart the
    // app on the CPU with its state gone.
    let (mut ready, ready_writer) = std::io::pipe()?;
    let status = std::process::Command::new(std::env::current_exe()?)
        .args(std::env::args().skip(1))
        .env("BLUE_DOVE_GPU", "1")
        .stdout(ready_writer)
        .status()?;
    let mut signal = [0u8; 1];
    let got_a_window = ready.read(&mut signal)? == 1;
    if status.success() || got_a_window {
        std::process::exit(status.code().unwrap_or(1));
    }
    eprintln!("blue-dove: the GPU backend failed; using the software renderer");
    run_software(viewport, false, paths, show_fps, request.as_ref())
}

/// Claims the app, or asks the window that already owns it to show itself.
///
/// A smoke run takes no lock and answers no other window, so it can draw on
/// every machine CI gives it.
fn claim(
    smoke: bool,
    request: Option<&instance::Request>,
) -> Option<(Option<File>, Option<TcpListener>)> {
    if smoke {
        return Some((None, None));
    }
    match instance::claim() {
        instance::Launch::AlreadyRunning => {
            let show = instance::Request::Show;
            if !instance::send(request.unwrap_or(&show)) {
                eprintln!("blue-dove: another window is running and did not answer");
            }
            None
        }
        instance::Launch::Owner { lock, listener } => Some((Some(lock), listener)),
    }
}

/// Runs the GPU path in the child process.
fn run_gpu(
    viewport: egui::ViewportBuilder,
    smoke: bool,
    paths: Vec<PathBuf>,
    show_fps: bool,
    no_vsync: bool,
    request: Option<&instance::Request>,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some((lock, listener)) = claim(smoke, request) else {
        return Ok(());
    };
    let options = eframe::NativeOptions {
        viewport,
        glow_options: eframe::egui_glow::GlowConfiguration {
            vsync: !no_vsync,
            ..Default::default()
        },
        ..Default::default()
    };
    let result = eframe::run_native(
        App::TITLE,
        options,
        Box::new(move |cc| {
            // One byte on stdout tells the parent the window exists, so a
            // later death is the app's own and must not be restarted.
            std::io::stdout().write_all(b"1").ok();
            std::io::stdout().flush().ok();
            Ok(Box::new(build_app(
                &cc.egui_ctx,
                smoke,
                paths,
                listener,
                "OpenGL",
                show_fps,
            )))
        }),
    );
    drop(lock);
    result.map_err(Into::into)
}

/// Runs the CPU fallback, reporting its error without the `Debug` wrapper
/// `main` would print for a boxed error.
fn run_software(
    viewport: egui::ViewportBuilder,
    smoke: bool,
    paths: Vec<PathBuf>,
    show_fps: bool,
    request: Option<&instance::Request>,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some((lock, listener)) = claim(smoke, request) else {
        return Ok(());
    };
    let result = software::run(viewport, move |ctx| {
        build_app(ctx, smoke, paths, listener, "software", show_fps)
    });
    drop(lock);
    match result {
        Ok(()) => Ok(()),
        Err(error) => {
            eprintln!("blue-dove: {error}");
            std::process::exit(1);
        }
    }
}

/// Builds the app against the context the chosen backend created.
fn build_app(
    ctx: &egui::Context,
    smoke: bool,
    paths: Vec<PathBuf>,
    listener: Option<TcpListener>,
    renderer: &'static str,
    show_fps: bool,
) -> App {
    eprintln!("blue-dove: using the {renderer} renderer");
    fonts::install(ctx);
    let mut app = App::new(ctx, smoke, paths);
    app.renderer = renderer;
    app.show_fps = show_fps;
    if let Some(listener) = listener {
        app.watch_requests(listener, ctx);
    }
    theme::apply(ctx, app.theme, app.opacity());
    app
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Any icon will do: the builder only carries it to the window.
    fn icon() -> egui::IconData {
        egui::IconData {
            rgba: Vec::new(),
            width: 1,
            height: 1,
        }
    }

    #[test]
    fn the_full_window_asks_for_no_max_size() {
        let viewport = window_viewport(false, app::DEFAULT_WINDOW_SIZE, icon());
        assert_eq!(viewport.max_inner_size, None);
    }

    #[test]
    fn the_preview_is_pinned_to_its_size() {
        let size = egui::vec2(app::COMPACT_SIZE[0], app::COMPACT_SIZE[1]);
        let viewport = window_viewport(true, app::COMPACT_SIZE, icon());
        assert_eq!(viewport.max_inner_size, Some(size));
        assert_eq!(viewport.min_inner_size, Some(size));
    }
}
