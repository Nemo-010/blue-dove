//! Fallback fonts for what egui's own faces cannot draw.
//!
//! egui's bundled faces cover Latin, Greek and Cyrillic, and Latin only in
//! the precomposed letters it ships with: no combining mark, no Vietnamese
//! letter, no note glyph. A library tagged in Japanese or Persian would draw
//! boxes, and so would a title with an accent stored as a letter and a mark.
//! A system font that carries those glyphs is loaded as a fallback, one per
//! script, which costs nothing when none is installed. Letters still come out
//! in the order the tags hold them, and unjoined: egui has no bidirectional
//! layout and no shaping to apply.
//!
//! Which font is installed differs from distribution to distribution and
//! from user to user, so the font directories are searched rather than a
//! list of paths being trusted: a hardcoded list misses the fonts that are
//! actually there, and every title shows as boxes.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use eframe::egui::{self, FontData, FontDefinitions, FontFamily};

/// File name fragments of fonts that carry CJK glyphs, best first.
///
/// A full CJK font is preferred to a Japanese-only one, since it also draws
/// Chinese and Korean titles, and both are preferred to a fallback face.
const CJK: &[&str] = &[
    "notosanscjk",
    "notosanskr",
    "notoserifcjk",
    "nanum",
    "malgun",
    "gulim",
    "batang",
    "sourcehansans",
    "sourcehanserif",
    "notosansjp",
    "notoserifjp",
    "droidsansfallbackfull",
    "droidsansfallback",
    "droidsansjapanese",
    "ipaexg",
    "ipaexm",
    "ipag",
    "ipam",
    "takaogothic",
    "takaomincho",
    "takaopgothic",
    "takaopmincho",
    "vlgothic",
    "vl-gothic",
    "sarasa",
    "mplus",
    "wenquanyi",
    "wqy",
    "msyh",
    "yugoth",
    "meiryo",
    "msgothic",
    "simsun",
    "simhei",
    "pingfang",
    "hiraginosans",
    "arialuni",
];

/// File name fragments of fonts that carry Arabic letters, best first.
///
/// These draw the Arabic script proper and the letters Persian, Urdu and
/// Pashto add to it. DejaVu is last because it is the face a stock Linux
/// install almost always has.
const ARABIC: &[&str] = &[
    "notosansarabic",
    "notonaskharabic",
    "notokufiarabic",
    "notonastaliqurdu",
    "droidnaskh",
    "droidkufi",
    "amiri",
    "scheherazade",
    "kacst",
    "freefarsi",
    "dejavusans",
];

/// File name fragments of fonts that carry the Latin letters the bundled face
/// leaves out, best first.
///
/// Ubuntu Light has the precomposed letters of Latin Extended-A, which is why
/// a Czech, Polish or Croatian title reads; it has no letter of Latin Extended
/// Additional, so a Vietnamese title is boxes, and it has no combining mark at
/// all. A title stored as `z` and a caron is therefore drawn as `z` and a box
/// until a face that can place the mark is loaded. `notosansregular` is the
/// plain face rather than the CJK one, which is preferred here for its
/// accents; the CJK face is found for the script it is wanted for anyway.
const LATIN: &[&str] = &[
    "notosansregular",
    "notosans",
    "dejavusans",
    "freesans",
    "freeserif",
];

/// File name fragments of fonts that carry the symbols a title collects.
///
/// A title ends in a note, an arrow or a suit often enough to be worth a face,
/// and none of those are in the bundled fonts.
const SYMBOLS: &[&str] = &[
    "notosanssymbols2",
    "notosanssymbols",
    "symbola",
    "dejavusans",
    "freeserif",
];

/// File name fragments of fonts that carry Hebrew letters, best first.
const HEBREW: &[&str] = &[
    "notosanshebrew",
    "notoserifhebrew",
    "culmus",
    "frankruehl",
    "davidclm",
    "freeserif",
    "dejavusans",
];

/// File name fragments of fonts that carry Thai letters, best first.
const THAI: &[&str] = &[
    "notosansthai",
    "notoserifthai",
    "garuda",
    "loma",
    "norasi",
    "waree",
    "kinnari",
    "sarabun",
    "tlwg",
    "freeserif",
];

/// File name fragments of fonts that carry Devanagari letters, best first.
const DEVANAGARI: &[&str] = &[
    "notosansdevanagari",
    "notoserifdevanagari",
    "lohitdevanagari",
    "sahadeva",
    "gargi",
    "nakula",
    "freeserif",
];

/// File name fragments of fonts that carry Bengali letters, best first.
const BENGALI: &[&str] = &[
    "notosansbengali",
    "notoserifbengali",
    "lohitbengali",
    "mukti",
    "freeserif",
];

/// File name fragments of fonts that carry Gurmukhi letters, best first.
const GURMUKHI: &[&str] = &["notosansgurmukhi", "lohitgurmukhi", "gurbani", "freeserif"];

/// File name fragments of fonts that carry Gujarati letters, best first.
const GUJARATI: &[&str] = &[
    "notosansgujarati",
    "notoserifgujarati",
    "lohitgujarati",
    "freeserif",
];

/// File name fragments of fonts that carry Tamil letters, best first.
const TAMIL: &[&str] = &["notosanstamil", "notoseriftamil", "lohittamil", "freeserif"];

/// File name fragments of fonts that carry Telugu letters, best first.
const TELUGU: &[&str] = &["notosanstelugu", "lohittelugu", "gautami", "freeserif"];

/// File name fragments of fonts that carry Kannada letters, best first.
const KANNADA: &[&str] = &["notosanskannada", "lohitkannada", "tunga", "freeserif"];

/// File name fragments of fonts that carry Malayalam letters, best first.
const MALAYALAM: &[&str] = &[
    "notosansmalayalam",
    "lohitmalayalam",
    "rachana",
    "meera",
    "anjali",
    "freeserif",
];

/// File name fragments of fonts that carry Sinhala letters, best first.
const SINHALA: &[&str] = &["notosanssinhala", "lklug", "iskoola"];

/// File name fragments of fonts that carry Khmer letters, best first.
const KHMER: &[&str] = &["notosanskhmer", "khmeros"];

/// File name fragments of fonts that carry Lao letters, best first.
const LAO: &[&str] = &["notosanslao", "phetsarath", "dokchampa"];

/// File name fragments of fonts that carry Burmese letters, best first.
const MYANMAR: &[&str] = &["notosansmyanmar", "myanmar3", "padauk"];

/// File name fragments of fonts that carry Ethiopic letters, best first.
const ETHIOPIC: &[&str] = &["notosansethiopic", "abyssinal"];

/// File name fragments of fonts that carry Armenian letters, best first.
const ARMENIAN: &[&str] = &["notosansarmenian", "freesans", "dejavusans"];

/// File name fragments of fonts that carry Georgian letters, best first.
const GEORGIAN: &[&str] = &["notosansgeorgian", "freesans", "dejavusans"];

/// One entry per script the bundled faces cannot draw, each the file name
/// fragments of the fonts that carry it.
///
/// The order is the order a glyph is looked for in, so the faces that fill the
/// most gaps come first: a letter no bundled face has then comes from the
/// first of these that carries it, and a glyph several carry from the one
/// listed earliest. Each entry loads one font, and only for a script some
/// library actually uses, so a system without those fonts pays nothing.
const SCRIPTS: &[&[&str]] = &[
    LATIN, SYMBOLS, CJK, ARABIC, HEBREW, THAI, DEVANAGARI, BENGALI, GURMUKHI, GUJARATI, TAMIL,
    TELUGU, KANNADA, MALAYALAM, SINHALA, KHMER, LAO, MYANMAR, ETHIOPIC, ARMENIAN, GEORGIAN,
];

/// Font directories that XDG does not cover.
///
/// Linux keeps its fonts under `XDG_DATA_HOME` and `XDG_DATA_DIRS`, which
/// [`directories`] reads. macOS and Windows lay them out elsewhere, and the
/// X11 directory is a leftover for a system with no XDG fonts at all.
const DIRECTORIES: &[&str] = &[
    "/usr/share/X11/fonts",
    "/System/Library/Fonts",
    "/Library/Fonts",
    "C:\\Windows\\Fonts",
];

/// Where XDG puts shared data when `XDG_DATA_DIRS` says nothing.
///
/// A list, not one joined string: [`std::env::split_paths`] reads a
/// semicolon on Windows, where the variable is a Unix convention anyway.
const DATA_DIRS: &[&str] = &["/usr/local/share", "/usr/share"];

/// Extensions a font file can carry.
const EXTENSIONS: &[&str] = &["ttf", "otf", "ttc", "otc"];

/// Adds the best font found for each script as a fallback in both families.
pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let mut installed = false;
    for (index, path) in find().into_iter().enumerate() {
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let name = format!("script{index}");
        fonts
            .font_data
            .insert(name.clone(), Arc::new(FontData::from_owned(bytes)));
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            fonts.families.entry(family).or_default().push(name.clone());
        }
        installed = true;
    }
    if installed {
        ctx.set_fonts(fonts);
    }
}

/// The best font installed for each script, in the order [`SCRIPTS`] gives.
fn find() -> Vec<PathBuf> {
    let mut files = Vec::new();
    for directory in directories() {
        collect(&directory, 0, &mut files);
    }
    one_per_script(&files)
}

/// The best of `files` for each script, in the order [`SCRIPTS`] gives.
///
/// A face that serves more than one script answers for the first of them and
/// is not taken again for the next: the same file loaded twice would keep its
/// bytes in memory twice, and put the same glyphs in the chain twice, for no
/// letter more than the one copy already draws.
fn one_per_script(files: &[PathBuf]) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = Vec::new();
    for patterns in SCRIPTS {
        let Some(path) = pick(files, patterns) else {
            continue;
        };
        if !found.contains(&path) {
            found.push(path);
        }
    }
    found
}

/// The font directories of the system and of the user.
///
/// `XDG_DATA_HOME` and `XDG_DATA_DIRS` name where data lives, and each holds
/// a `fonts` directory. Reading them, rather than trusting a fixed path, is
/// what makes an Anylinux AppImage find fonts: quick-sharun rewrites the
/// `/usr/share` literal in the binary to point inside the image, whose share
/// directory carries none, while the environment still names the host's
/// directories at run time.
fn directories() -> Vec<PathBuf> {
    let mut directories = Vec::new();
    if let Some(data) = dirs::data_dir() {
        directories.push(data.join("fonts"));
    }
    for directory in data_dirs() {
        directories.push(directory.join("fonts"));
    }
    directories.extend(DIRECTORIES.iter().map(PathBuf::from));
    if let Some(windows) = std::env::var_os("WINDIR") {
        directories.push(PathBuf::from(windows).join("Fonts"));
    }
    directories
}

/// The entries of `XDG_DATA_DIRS`, or the two XDG names when it is unset.
fn data_dirs() -> Vec<PathBuf> {
    split_data_dirs(std::env::var_os("XDG_DATA_DIRS").as_deref())
}

/// The directories `XDG_DATA_DIRS` names, or the standard two when it is
/// unset or empty.
///
/// An empty variable is set but says nothing, which the XDG spec treats as
/// unset; reading it as "no directories" would hide every font.
fn split_data_dirs(value: Option<&std::ffi::OsStr>) -> Vec<PathBuf> {
    match value.filter(|dirs| !dirs.is_empty()) {
        Some(dirs) => std::env::split_paths(dirs).collect(),
        None => DATA_DIRS.iter().map(PathBuf::from).collect(),
    }
}

/// Collects the font files under `directory`, a few levels deep.
fn collect(directory: &Path, depth: usize, files: &mut Vec<PathBuf>) {
    if depth > 4 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, depth + 1, files);
        } else if is_font(&path) {
            files.push(path);
        }
    }
}

/// Whether `path` names a font file.
fn is_font(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            let extension = extension.to_ascii_lowercase();
            EXTENSIONS.contains(&extension.as_str())
        })
}

/// The best of `files`, by the order `patterns` gives.
///
/// A pattern is searched for in the whole path, so a font in a directory named
/// after its family is found too, and the first pattern that matches wins.
fn pick(files: &[PathBuf], patterns: &[&str]) -> Option<PathBuf> {
    patterns.iter().find_map(|pattern| {
        files.iter().find_map(|path| {
            let name = squashed(&path.to_string_lossy());
            name.contains(pattern).then(|| path.clone())
        })
    })
}

/// The letters and digits of `text`, in lower case.
///
/// Font files spell a family `Noto_Sans_CJK`, `Noto Sans CJK` or
/// `NotoSansCJK` depending on who packaged them, so the separators are
/// dropped before a name is compared with a pattern.
fn squashed(text: &str) -> String {
    text.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|letter| letter.to_ascii_lowercase())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(names: &[&str]) -> Vec<PathBuf> {
        names.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn a_full_cjk_font_is_preferred_to_a_japanese_one() {
        let files = paths(&[
            "/usr/share/fonts/droid/DroidSansJapanese.ttf",
            "/usr/share/fonts/noto/NotoSansCJK-Regular.ttc",
        ]);
        assert_eq!(
            pick(&files, CJK),
            Some(PathBuf::from(
                "/usr/share/fonts/noto/NotoSansCJK-Regular.ttc"
            ))
        );
    }

    #[test]
    fn a_font_the_old_list_missed_is_found() {
        // The face a stock Arch or Debian install carries, which a list of
        // fixed paths did not name.
        let files = paths(&[
            "/usr/share/fonts/liberation/LiberationSans-Regular.ttf",
            "/usr/share/fonts/droid/DroidSansFallback.ttf",
        ]);
        assert_eq!(
            pick(&files, CJK),
            Some(PathBuf::from(
                "/usr/share/fonts/droid/DroidSansFallback.ttf"
            ))
        );
    }

    #[test]
    fn the_directory_name_counts_as_well_as_the_file_name() {
        let files = paths(&["/usr/share/fonts/ipa/ipa-g.ttf"]);
        assert_eq!(
            pick(&files, CJK),
            Some(PathBuf::from("/usr/share/fonts/ipa/ipa-g.ttf"))
        );
    }

    #[test]
    fn a_name_split_by_separators_still_matches() {
        // Packagers spell a family three ways; all of them have to be found.
        for name in [
            "/usr/share/fonts/noto/Noto_Sans_CJK-Regular.ttc",
            "/usr/share/fonts/noto/Noto Sans CJK Regular.otf",
            "/usr/share/fonts/noto/NotoSansCJK-Regular.ttc",
        ] {
            let files = paths(&[name]);
            assert_eq!(pick(&files, CJK), Some(PathBuf::from(name)), "{name}");
        }
    }

    #[test]
    fn a_latin_only_system_gets_no_fallback() {
        let files = paths(&[
            "/usr/share/fonts/liberation/LiberationSans-Regular.ttf",
            "/usr/share/fonts/noto/NotoSans-Regular.ttf",
        ]);
        assert_eq!(pick(&files, CJK), None, "no font here draws Japanese");
    }

    #[test]
    fn a_font_with_arabic_letters_is_found() {
        // The face a stock Debian install carries, and the one a user who
        // writes Persian or Arabic would have installed on purpose.
        let files = paths(&[
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/truetype/noto/NotoSansArabic-Regular.ttf",
        ]);
        assert_eq!(
            pick(&files, ARABIC),
            Some(PathBuf::from(
                "/usr/share/fonts/truetype/noto/NotoSansArabic-Regular.ttf"
            )),
            "an Arabic face beats the catch-all one"
        );
    }

    #[test]
    fn every_script_gets_a_font_of_its_own() {
        let files = paths(&[
            "/usr/share/fonts/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/droid/DroidNaskh-Regular.ttf",
        ]);
        assert_eq!(
            one_per_script(&files),
            [
                PathBuf::from("/usr/share/fonts/noto/NotoSansCJK-Regular.ttc"),
                PathBuf::from("/usr/share/fonts/droid/DroidNaskh-Regular.ttf"),
            ],
            "one font per script, so a title in either draws"
        );
    }

    #[test]
    fn a_face_that_serves_two_scripts_is_loaded_once() {
        // A CJK face carries Latin too, so it answers for the Latin letters
        // as well as the CJK ones. Reading it again for the second script
        // would keep the same megabytes in memory twice.
        let files = paths(&["/usr/share/fonts/noto/NotoSansCJK-Regular.ttc"]);
        let found = one_per_script(&files);
        assert_eq!(
            found,
            [PathBuf::from(
                "/usr/share/fonts/noto/NotoSansCJK-Regular.ttc"
            )],
            "one file, however many scripts it draws"
        );
    }

    #[test]
    fn the_letters_the_bundled_face_leaves_out_get_a_font() {
        // Ubuntu Light has the precomposed Latin Extended-A letters but none
        // of Latin Extended Additional and no combining mark at all, so a
        // Vietnamese title or one written as a letter and a mark needs a face
        // of its own.
        let files = paths(&[
            "/usr/share/fonts/liberation/LiberationSans-Regular.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        ]);
        assert_eq!(
            pick(&files, LATIN),
            Some(PathBuf::from(
                "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"
            )),
            "a face with the accents, not the one without"
        );
        let bare = paths(&["/usr/share/fonts/liberation/LiberationSans-Regular.ttf"]);
        assert_eq!(
            pick(&bare, LATIN),
            None,
            "a face that draws no more than the bundled one is no fallback"
        );
    }

    #[test]
    fn a_title_ending_in_a_note_gets_a_face() {
        // No bundled face has the note, which a great many titles end in.
        let files = paths(&[
            "/usr/share/fonts/noto/NotoSansSymbols2-Regular.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        ]);
        assert_eq!(
            pick(&files, SYMBOLS),
            Some(PathBuf::from(
                "/usr/share/fonts/noto/NotoSansSymbols2-Regular.ttf"
            )),
            "the symbols face beats the catch-all one"
        );
    }

    #[test]
    fn a_font_with_hebrew_letters_is_found() {
        let files = paths(&[
            "/usr/share/fonts/liberation/LiberationSans-Regular.ttf",
            "/usr/share/fonts/truetype/noto/NotoSansHebrew-Regular.ttf",
        ]);
        assert_eq!(
            pick(&files, HEBREW),
            Some(PathBuf::from(
                "/usr/share/fonts/truetype/noto/NotoSansHebrew-Regular.ttf"
            ))
        );
        let bare = paths(&["/usr/share/fonts/liberation/LiberationSans-Regular.ttf"]);
        assert_eq!(pick(&bare, HEBREW), None, "no font here draws Hebrew");
    }

    #[test]
    fn a_latin_only_system_finds_no_arabic_font() {
        let files = paths(&["/usr/share/fonts/liberation/LiberationSans-Regular.ttf"]);
        assert_eq!(pick(&files, ARABIC), None, "no font here draws Arabic");
    }

    #[test]
    fn only_font_files_are_collected() {
        assert!(is_font(Path::new("/x/NotoSansCJK.ttc")));
        assert!(is_font(Path::new("/x/DroidSansJapanese.TTF")));
        assert!(!is_font(Path::new("/x/fonts.dir")));
        assert!(!is_font(Path::new("/x/README")));
    }

    #[test]
    fn the_xdg_directories_are_read() {
        use std::ffi::OsStr;

        assert_eq!(
            split_data_dirs(None),
            vec![
                PathBuf::from("/usr/local/share"),
                PathBuf::from("/usr/share")
            ],
            "unset, XDG_DATA_DIRS means the two standard names"
        );
        assert_eq!(
            split_data_dirs(Some(OsStr::new(""))),
            vec![
                PathBuf::from("/usr/local/share"),
                PathBuf::from("/usr/share")
            ],
            "an empty variable says nothing, so it means the same"
        );
        #[cfg(unix)]
        assert_eq!(
            split_data_dirs(Some(OsStr::new("/opt/share:/usr/share"))),
            vec![PathBuf::from("/opt/share"), PathBuf::from("/usr/share")],
            "a set variable is used as it stands"
        );
    }
}
