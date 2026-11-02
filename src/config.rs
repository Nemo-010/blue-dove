//! Settings kept between runs: the music directory, the volume and the
//! playlists that were open.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::At;

/// Settings written to the user's config directory.
#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Config {
    /// Directory scanned at startup.
    ///
    /// The name it was written under before the app called it a directory is
    /// still read, so an older settings file keeps its music.
    #[serde(alias = "music_folder")]
    pub music_directory: Option<PathBuf>,
    /// Output volume, from 0 to 1.
    pub volume: Option<f32>,
    /// Whether the window lets the desktop show through it.
    pub transparency: Option<bool>,
    /// Whether the playing track repeats instead of the next one starting.
    pub repeat: Option<bool>,
    /// Whether the next track is picked at random.
    pub random: Option<bool>,
    /// Name of the colour theme, as [`crate::theme::Theme::name`] gives it.
    pub theme: Option<String>,
    /// The playlists that were open, in tab order.
    pub playlists: Option<Vec<SavedPlaylist>>,
    /// Which of them was shown, counting from zero.
    pub active_playlist: Option<usize>,
    /// The track that was selected in that playlist.
    pub selected_track: Option<PathBuf>,
    /// Widths of the track table's columns, in the order
    /// [`crate::ui::Column::ALL`] gives them.
    pub column_widths: Option<[f32; 6]>,
    /// Which of those columns the track table shows, in the same order.
    pub column_shown: Option<[bool; 6]>,
    /// Whether the file name column keeps the extension, as in `.flac`.
    pub file_extension: Option<bool>,
    /// Whether closing the window leaves the music playing, so the app has
    /// to be launched again to bring the window back.
    pub background: Option<bool>,
    /// Whether files and directories whose name starts with a dot are left
    /// out of the scan.
    pub ignore_hidden: Option<bool>,
    /// Whether the music directory is scanned at startup and again while the
    /// window runs, so a file added outside the window shows up without a
    /// rescan by hand.
    pub auto_scan: Option<bool>,
    /// Whether the album art is fitted into its square whole, rather than
    /// cropped to fill it.
    pub cover_fit: Option<bool>,
    /// Whether a launch naming one audio file opens the preview instead of
    /// the full window.
    pub preview: Option<bool>,
}

/// A playlist as the settings file stores it.
///
/// Tracks are kept by path rather than by index: the library is scanned
/// afresh at every start, so an index means nothing until the scan finishes,
/// and a file that moved should not turn into a different track.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SavedPlaylist {
    /// Name shown on the tab.
    pub name: String,
    /// Files the playlist holds, in playing order.
    pub tracks: Vec<PathBuf>,
}

impl Config {
    /// Reads the settings, falling back to defaults when there are none or
    /// they cannot be parsed.
    pub fn load() -> Self {
        default_path()
            .as_deref()
            .map(Self::load_from)
            .unwrap_or_default()
    }

    /// Reads settings from `path`.
    pub fn load_from(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| toml::from_str(&text).ok())
            .unwrap_or_default()
    }

    /// Writes the settings to the user's config directory.
    pub fn save(&self) -> Result<(), String> {
        match default_path() {
            Some(path) => self.save_to(&path),
            None => Err("no config directory for this user".to_string()),
        }
    }

    /// Writes the settings to `path`, creating its directory.
    pub fn save_to(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).at(parent)?;
        }
        let text = toml::to_string_pretty(self)
            .map_err(|error| format!("cannot encode the settings: {error}"))?;
        std::fs::write(path, text).at(path)
    }
}

/// `$XDG_CONFIG_HOME/blue-dove/config.toml`, or the platform's equivalent.
fn default_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("blue-dove").join("config.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_survive_a_round_trip() {
        let directory =
            std::env::temp_dir().join(format!("blue-dove-config-{}", std::process::id()));
        let path = directory.join("nested").join("config.toml");
        let config = Config {
            music_directory: Some(PathBuf::from("/music")),
            volume: Some(0.4),
            transparency: Some(true),
            repeat: Some(true),
            random: Some(true),
            theme: Some("Breeze Dark".to_string()),
            playlists: Some(vec![
                SavedPlaylist {
                    name: "Default".to_string(),
                    tracks: vec![PathBuf::from("/music/one.flac")],
                },
                SavedPlaylist {
                    name: "Evening".to_string(),
                    tracks: vec![
                        PathBuf::from("/music/two.flac"),
                        PathBuf::from("/music/three.flac"),
                    ],
                },
            ]),
            active_playlist: Some(1),
            selected_track: Some(PathBuf::from("/music/two.flac")),
            column_widths: Some([38.0, 300.0, 280.0, 210.0, 190.0, 64.0]),
            column_shown: Some([true, true, false, false, true, true]),
            file_extension: Some(false),
            background: Some(true),
            ignore_hidden: Some(false),
            auto_scan: Some(false),
            cover_fit: Some(true),
            preview: Some(false),
        };

        config.save_to(&path).expect("the settings are written");
        let loaded = Config::load_from(&path);
        std::fs::remove_dir_all(&directory).expect("the temporary directory is removed");

        assert_eq!(loaded.music_directory, Some(PathBuf::from("/music")));
        assert_eq!(loaded.volume, Some(0.4));
        assert_eq!(loaded.transparency, Some(true));
        assert_eq!(loaded.repeat, Some(true));
        assert_eq!(loaded.random, Some(true));
        assert_eq!(loaded.theme.as_deref(), Some("Breeze Dark"));
        assert_eq!(loaded.active_playlist, Some(1));
        assert_eq!(
            loaded.column_widths,
            Some([38.0, 300.0, 280.0, 210.0, 190.0, 64.0])
        );
        assert_eq!(
            loaded.column_shown,
            Some([true, true, false, false, true, true])
        );
        assert_eq!(loaded.file_extension, Some(false));
        assert_eq!(loaded.auto_scan, Some(false));
        assert_eq!(loaded.background, Some(true));
        assert_eq!(loaded.ignore_hidden, Some(false));
        assert_eq!(loaded.cover_fit, Some(true));
        assert_eq!(loaded.preview, Some(false));
        assert_eq!(
            loaded.selected_track,
            Some(PathBuf::from("/music/two.flac"))
        );

        let playlists = loaded.playlists.expect("the playlists are written");
        assert_eq!(playlists.len(), 2);
        assert_eq!(playlists[1].name, "Evening");
        assert_eq!(
            playlists[1].tracks,
            [
                PathBuf::from("/music/two.flac"),
                PathBuf::from("/music/three.flac")
            ]
        );
    }

    #[test]
    fn a_settings_file_without_playlists_still_loads() {
        let directory = std::env::temp_dir().join(format!("blue-dove-old-{}", std::process::id()));
        let path = directory.join("config.toml");
        std::fs::create_dir_all(&directory).expect("the directory is made");
        std::fs::write(&path, "music_directory = \"/music\"\nvolume = 0.5\n")
            .expect("the file is written");

        let loaded = Config::load_from(&path);
        std::fs::remove_dir_all(&directory).expect("the temporary directory is removed");

        assert_eq!(loaded.volume, Some(0.5));
        assert!(loaded.playlists.is_none(), "an older file has no playlists");
    }

    #[test]
    fn a_settings_file_that_with_the_old_key_still_finds_its_music() {
        // The key was called `music_folder` before the app said directory.
        let directory =
            std::env::temp_dir().join(format!("blue-dove-old-key-{}", std::process::id()));
        let path = directory.join("config.toml");
        std::fs::create_dir_all(&directory).expect("the directory is made");
        std::fs::write(&path, "music_folder = \"/music\"\n").expect("the file is written");

        let loaded = Config::load_from(&path);
        std::fs::remove_dir_all(&directory).expect("the temporary directory is removed");

        assert_eq!(
            loaded.music_directory,
            Some(PathBuf::from("/music")),
            "the older key is read as the directory to scan"
        );
    }

    #[test]
    fn a_missing_file_gives_defaults() {
        let missing = std::env::temp_dir().join("blue-dove-no-such-config.toml");
        let config = Config::load_from(&missing);
        assert!(config.music_directory.is_none());
        assert!(config.volume.is_none());
    }
}
