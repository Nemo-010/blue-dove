//! The command line: what a launch was asked to do.
//!
//! Arguments come in three kinds: a flag that asks a question and exits
//! (`--help`, `--version`), a transport command for a player that is already
//! running (`--next`), and paths to play or queue. Parsing happens before a
//! window exists, so a bad argument is reported on the terminal rather than
//! in a status line nobody is looking at yet.

use std::path::PathBuf;

/// The renderer a launch asked for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Renderer {
    /// The GPU path through glow/OpenGL.
    #[default]
    OpenGl,
    /// The CPU renderer.
    Software,
}

impl Renderer {
    /// The command line flag that names this renderer.
    pub fn flag(self) -> &'static str {
        match self {
            Self::OpenGl => "--opengl",
            Self::Software => "--software",
        }
    }
}

/// What a launch was asked to do.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Cli {
    /// Paths to play or queue, in the order they were given.
    pub paths: Vec<PathBuf>,
    /// The transport command, when one was given.
    pub action: Option<Action>,
    /// Whether to show the frame-rate counter.
    pub fps: bool,
    /// Whether to disable vsync on the GPU renderer.
    pub no_vsync: bool,
    /// The renderer the launch asked for, when it named one.
    pub renderer: Option<Renderer>,
}

/// A transport command given on the command line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Start or resume playback.
    Play,
    /// Pause playback.
    Pause,
    /// Play when paused, pause when playing.
    Toggle,
    /// Stop playback.
    Stop,
    /// Play the next track.
    Next,
    /// Play the previous track.
    Previous,
}

impl Action {
    /// The command line flag that names this action.
    pub fn flag(self) -> &'static str {
        match self {
            Self::Play => "--play",
            Self::Pause => "--pause",
            Self::Toggle => "--play-pause",
            Self::Stop => "--stop",
            Self::Next => "--next",
            Self::Previous => "--previous",
        }
    }

    /// The word this action travels as, to the window that is running.
    pub fn verb(self) -> &'static str {
        match self {
            Self::Play => "play",
            Self::Pause => "pause",
            Self::Toggle => "toggle",
            Self::Stop => "stop",
            Self::Next => "next",
            Self::Previous => "previous",
        }
    }

    /// The action a flag names, when it names one.
    pub fn from_flag(flag: &str) -> Option<Self> {
        match flag {
            "--play" => Some(Self::Play),
            "--pause" => Some(Self::Pause),
            "--play-pause" => Some(Self::Toggle),
            "--stop" => Some(Self::Stop),
            "--next" => Some(Self::Next),
            "--previous" => Some(Self::Previous),
            _ => None,
        }
    }

    /// The action `verb` names, when it names one.
    pub fn from_verb(verb: &str) -> Option<Self> {
        match verb {
            "play" => Some(Self::Play),
            "pause" => Some(Self::Pause),
            "toggle" => Some(Self::Toggle),
            "stop" => Some(Self::Stop),
            "next" => Some(Self::Next),
            "previous" => Some(Self::Previous),
            _ => None,
        }
    }

    /// The player command this stands for.
    ///
    /// The player already answers these over D-Bus, so the command line sends
    /// the same ones rather than a second set that could drift from them.
    pub fn command(self) -> crate::mpris::Command {
        match self {
            Self::Play => crate::mpris::Command::Play,
            Self::Pause => crate::mpris::Command::Pause,
            Self::Toggle => crate::mpris::Command::PlayPause,
            Self::Stop => crate::mpris::Command::Stop,
            Self::Next => crate::mpris::Command::Next,
            Self::Previous => crate::mpris::Command::Previous,
        }
    }
}

/// What parsing the arguments came to.
#[derive(Debug, PartialEq, Eq)]
pub enum Parsed {
    /// The window should run, with this request.
    Run(Cli),
    /// Print the usage and exit.
    Help,
    /// Print the version and exit.
    Version,
    /// The arguments make no sense; the text says why.
    Error(String),
}

/// The usage, shown by `--help` and after a bad argument.
pub const USAGE: &str = "\
Usage: blue-dove [OPTIONS] [PATH]...

  A PATH that is a file plays it in a tab named Default, replacing what that
  tab held. A PATH that is a directory queues every track under it.

Options:
  -h, --help       Show this message and exit
  -V, --version    Show the version and exit
      --play       Start or resume playback
      --pause      Pause playback
      --play-pause Toggle between playing and paused
      --stop       Stop playback
      --next       Play the next track
      --previous   Play the previous track
      --fps        Show a frame-rate counter
      --no-vsync   Disable vsync on the GPU renderer
      --opengl     Draw through OpenGL (the default)
      --software   Draw with the CPU renderer

A command goes to the player that is already running, and starts nothing.
With no player running, a PATH opens the window and a bare command says
there is nobody to send it to.";

/// Reads the arguments, without the program's own name.
///
/// `--smoke` is accepted and ignored here: it is the release pipeline's flag,
/// and [`crate::main`] acts on it.
pub fn parse(args: impl IntoIterator<Item = String>) -> Parsed {
    let mut cli = Cli::default();
    let mut rest_are_paths = false;
    for arg in args {
        if !rest_are_paths {
            match arg.as_str() {
                "--smoke" => continue,
                "--fps" => {
                    cli.fps = true;
                    continue;
                }
                "--no-vsync" => {
                    cli.no_vsync = true;
                    continue;
                }
                "--opengl" | "--software" => {
                    let renderer = if arg == "--opengl" {
                        Renderer::OpenGl
                    } else {
                        Renderer::Software
                    };
                    if let Some(previous) = cli.renderer
                        && previous != renderer
                    {
                        return Parsed::Error(format!(
                            "{} and {} cannot both be given",
                            previous.flag(),
                            renderer.flag()
                        ));
                    }
                    cli.renderer = Some(renderer);
                    continue;
                }
                "-h" | "--help" => return Parsed::Help,
                "-V" | "--version" => return Parsed::Version,
                "--" => {
                    rest_are_paths = true;
                    continue;
                }
                _ => {}
            }
            if let Some(action) = Action::from_flag(&arg) {
                if let Some(previous) = cli.action
                    && previous != action
                {
                    return Parsed::Error(format!(
                        "{} and {} cannot both be given",
                        previous.flag(),
                        action.flag()
                    ));
                }
                cli.action = Some(action);
                continue;
            }
            if arg.starts_with('-') && arg != "-" {
                return Parsed::Error(format!("unknown option {arg}"));
            }
        }
        // A path travels one per line to a running window, so one that holds
        // a line break cannot be sent and is better refused than mangled.
        if arg.contains('\n') {
            return Parsed::Error("a path cannot hold a line break".to_string());
        }
        cli.paths.push(PathBuf::from(arg));
    }
    Parsed::Run(cli)
}

/// The directory the music of `path` lives in: the path itself when it is a
/// directory, its parent when it is a file.
///
/// A first run has no music directory, and a path on the command line is a
/// better answer than a dialog the user did not ask for.
pub fn music_root(path: &std::path::Path) -> Option<PathBuf> {
    if path.is_dir() {
        Some(path.to_path_buf())
    } else {
        path.parent().map(std::path::Path::to_path_buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parses arguments written the way a shell would give them.
    fn parse_args(args: &[&str]) -> Parsed {
        parse(args.iter().map(|arg| arg.to_string()))
    }

    /// The request a set of arguments came to, which is what most tests want.
    fn run(args: &[&str]) -> Cli {
        match parse_args(args) {
            Parsed::Run(cli) => cli,
            other => panic!("expected a run, got {other:?}"),
        }
    }

    #[test]
    fn no_arguments_ask_for_a_window() {
        assert_eq!(run(&[]), Cli::default());
    }

    #[test]
    fn the_fps_flag_asks_for_the_counter() {
        assert!(run(&["--fps"]).fps);
        assert!(!run(&[]).fps);
    }

    #[test]
    fn the_no_vsync_flag_asks_for_no_vsync() {
        assert!(run(&["--no-vsync"]).no_vsync);
        assert!(!run(&[]).no_vsync);
    }

    #[test]
    fn each_renderer_flag_names_its_renderer() {
        assert_eq!(run(&["--opengl"]).renderer, Some(Renderer::OpenGl));
        assert_eq!(run(&["--software"]).renderer, Some(Renderer::Software));
        assert_eq!(run(&[]).renderer, None);
        assert_eq!(
            parse_args(&["--opengl", "--software"]),
            Parsed::Error("--opengl and --software cannot both be given".to_string())
        );
    }

    #[test]
    fn a_path_is_a_path() {
        let cli = run(&["/music/one.flac", "two.flac"]);
        assert_eq!(
            cli.paths,
            [PathBuf::from("/music/one.flac"), PathBuf::from("two.flac")]
        );
        assert_eq!(cli.action, None);
    }

    #[test]
    fn help_and_version_ask_their_question() {
        assert_eq!(parse_args(&["--help"]), Parsed::Help);
        assert_eq!(parse_args(&["-h"]), Parsed::Help);
        assert_eq!(parse_args(&["--version"]), Parsed::Version);
        assert_eq!(parse_args(&["-V"]), Parsed::Version);
        // A question wins wherever it appears.
        assert_eq!(parse_args(&["/music", "--help"]), Parsed::Help);
    }

    #[test]
    fn every_transport_flag_names_its_action() {
        for action in [
            Action::Play,
            Action::Pause,
            Action::Toggle,
            Action::Stop,
            Action::Next,
            Action::Previous,
        ] {
            assert_eq!(run(&[action.flag()]).action, Some(action));
            assert_eq!(Action::from_verb(action.verb()), Some(action));
        }
    }

    #[test]
    fn two_different_actions_are_refused() {
        assert_eq!(
            parse_args(&["--next", "--previous"]),
            Parsed::Error("--next and --previous cannot both be given".to_string())
        );
        // The same one twice is harmless.
        assert_eq!(run(&["--next", "--next"]).action, Some(Action::Next));
    }

    #[test]
    fn an_unknown_option_is_refused() {
        assert_eq!(
            parse_args(&["--frobnicate"]),
            Parsed::Error("unknown option --frobnicate".to_string())
        );
        assert!(matches!(parse_args(&["-x"]), Parsed::Error(_)));
    }

    #[test]
    fn a_double_dash_ends_the_options() {
        // A file really called `--next` is still a file after `--`.
        let cli = run(&["--next", "--", "--next"]);
        assert_eq!(cli.action, Some(Action::Next));
        assert_eq!(cli.paths, [PathBuf::from("--next")]);
    }

    #[test]
    fn a_smoke_run_is_accepted_and_ignored() {
        assert_eq!(run(&["--smoke"]), Cli::default());
    }

    #[test]
    fn a_path_that_holds_a_line_break_is_refused() {
        assert_eq!(
            parse_args(&["bad\nname.flac"]),
            Parsed::Error("a path cannot hold a line break".to_string())
        );
    }

    #[test]
    fn the_music_directory_is_the_parent_of_a_file() {
        let root = std::env::temp_dir().join("blue-dove-cli-root");
        std::fs::create_dir_all(&root).expect("the directory is made");
        let file = root.join("song.flac");
        std::fs::write(&file, b"not really audio").expect("the file is written");

        assert_eq!(music_root(&file), Some(root.clone()));
        assert_eq!(music_root(&root), Some(root.clone()));
        std::fs::remove_dir_all(&root).expect("the directory is removed");
    }
}
