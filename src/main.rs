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
mod theme;
mod ui;
mod watch;
mod waveform;

use app::App;
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

fn main() -> eframe::Result {
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

    // A smoke run must draw its frames on every machine CI gives it, so it
    // neither takes the lock nor answers another window's.
    let (lock, listener) = if smoke {
        (None, None)
    } else {
        match instance::claim() {
            // A window appeared while the request above was being sent.
            instance::Launch::AlreadyRunning => {
                let request = request.unwrap_or(instance::Request::Show);
                if !instance::send(&request) {
                    eprintln!("blue-dove: another window is running and did not answer");
                }
                return Ok(());
            }
            instance::Launch::Owner { lock, listener } => (Some(lock), listener),
        }
    };

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
    let options = eframe::NativeOptions {
        viewport: window_viewport(preview, start_size, icon),
        ..Default::default()
    };
    let paths = cli.paths;
    let result = eframe::run_native(
        App::TITLE,
        options,
        Box::new(move |cc| {
            fonts::install(&cc.egui_ctx);
            let mut app = App::new(&cc.egui_ctx, smoke, paths);
            if let Some(listener) = listener {
                app.watch_requests(listener, &cc.egui_ctx);
            }
            theme::apply(&cc.egui_ctx, app.theme, app.opacity());
            Ok(Box::new(app))
        }),
    );
    // The lock is held for as long as the window lives; the event loop has
    // returned, so this is where it goes.
    drop(lock);
    result
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
