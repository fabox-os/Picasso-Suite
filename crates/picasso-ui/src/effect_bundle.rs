//! Effect bundles (`.picef`): what a bundle holds is installed into the user's data folder, and
//! offered where the program already offers effects, titles, transitions and LUTs:
//!
//! * effect templates go where the Inspector's presets live (`presets/effects/<name>.json`), so
//!   they are in the Edit page's Presets list at once;
//! * titles (`presets/titles/`), transitions (`presets/transitions/`) and look-up tables
//!   (`presets/luts/`) are listed by [`library`] and offered from the Effects > Titles and
//!   Effects > Video Transitions menus and the Color page's LUTs dialog.
//!
//! The bundle itself is read by [`picasso_drp::bundle`], which refuses anything hostile. This
//! module only writes what it found, and every file it writes is named through
//! [`crate::effect_presets::file_name_with`], so no name a bundle gives can reach outside
//! `<data folder>/presets/`:
//!
//! Everything is read back through the same checks: a file edited by hand is kept in range, and a
//! table that no longer parses is not offered.

use std::path::{Path, PathBuf};

use picasso_color::Lut;
use picasso_core::TitleText;
use picasso_drp::bundle::{BundleItem, TransitionTemplate};
use serde::{Deserialize, Serialize};

use crate::project_manager::data_dir;

/// The most items of one kind a library lists.
const MAX_LIBRARY: usize = 500;
/// The largest installed file that is read.
const MAX_FILE_BYTES: u64 = 1 << 20;

/// The folder one kind of installed item lives in: `<data folder>/presets/<kind>`.
fn dir_of(root: &Path, kind: &str) -> PathBuf {
    root.join("presets").join(kind)
}

/// What installing one bundle did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Installed {
    /// The bundle's name, from its file name.
    pub bundle: String,
    pub effects: usize,
    pub titles: usize,
    pub transitions: usize,
    pub luts: usize,
    /// Entries the bundle held that could not be read, and why.
    pub skipped: Vec<String>,
    /// Items whose name left nothing to name a file by.
    pub refused: Vec<String>,
}

impl Installed {
    /// How many items were installed.
    pub fn total(&self) -> usize {
        self.effects + self.titles + self.transitions + self.luts
    }

    /// One line for the status bar.
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        for (count, what) in [
            (self.effects, "effect template"),
            (self.titles, "title"),
            (self.transitions, "transition"),
            (self.luts, "LUT"),
        ] {
            if count > 0 {
                parts.push(format!(
                    "{count} {what}{}",
                    if count == 1 { "" } else { "s" }
                ));
            }
        }
        let mut line = format!("Installed {}: {}", self.bundle, parts.join(", "));
        if !self.skipped.is_empty() {
            line.push_str(&format!("; {} file(s) left out", self.skipped.len()));
        }
        if !self.refused.is_empty() {
            line.push_str(&format!("; {} item(s) could not be named", self.refused.len()));
        }
        line
    }
}

/// What is installed: the names of the items of each kind.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Library {
    pub effects: Vec<String>,
    pub titles: Vec<String>,
    pub transitions: Vec<String>,
    pub luts: Vec<String>,
}

impl Library {}

/// Installs a bundle from the user's data folder.
pub(crate) fn install(name: &str, bytes: &[u8]) -> Result<Installed, String> {
    let root =
        data_dir().ok_or_else(|| format!("{name}: there is no data folder to install into"))?;
    install_in(&root, name, bytes)
}

/// Installs a bundle into `root` (the data folder, or a test's own folder).
pub(crate) fn install_in(root: &Path, name: &str, bytes: &[u8]) -> Result<Installed, String> {
    let bundle = picasso_drp::bundle::read(bytes, name).map_err(|e| format!("{name}: {e}"))?;
    let mut out = Installed {
        bundle: bundle.name.clone(),
        ..Installed::default()
    };
    for item in &bundle.items {
        let wrote = match item {
            BundleItem::Effects { name, effects } => {
                let wrote = crate::effect_presets::save_in(&dir_of(root, "effects"), name, effects);
                if wrote {
                    out.effects += 1;
                }
                wrote
            }
            BundleItem::Title { name, title } => {
                let wrote = write_json(
                    &dir_of(root, "titles"),
                    name,
                    &StoredTitle {
                        name: name.clone(),
                        title: title.clone(),
                    },
                );
                if wrote {
                    out.titles += 1;
                }
                wrote
            }
            BundleItem::Transition { name, template } => {
                let wrote = write_json(
                    &dir_of(root, "transitions"),
                    name,
                    &StoredTransition {
                        name: name.clone(),
                        transition: *template,
                    },
                );
                if wrote {
                    out.transitions += 1;
                }
                wrote
            }
            BundleItem::Lut { name, cube } => {
                let wrote = write_text(&dir_of(root, "luts"), name, "cube", cube);
                if wrote {
                    out.luts += 1;
                }
                wrote
            }
        };
        if !wrote {
            out.refused.push(format!("{} {}", item.kind_label(), item.name()));
        }
    }
    out.skipped = bundle.skipped;
    Ok(out)
}

/// What is installed in the user's data folder.
pub(crate) fn library() -> Library {
    data_dir().map(|d| library_in(&d)).unwrap_or_default()
}

/// What is installed in `root`.
pub(crate) fn library_in(root: &Path) -> Library {
    Library {
        effects: crate::effect_presets::list_in(&dir_of(root, "effects")),
        titles: list_in(&dir_of(root, "titles"), "json"),
        transitions: list_in(&dir_of(root, "transitions"), "json"),
        luts: list_in(&dir_of(root, "luts"), "cube"),
    }
}

/// An installed title.
pub(crate) fn load_title(name: &str) -> Option<TitleText> {
    let root = data_dir()?;
    load_title_in(&root, name)
}

fn load_title_in(root: &Path, name: &str) -> Option<TitleText> {
    let text = read_text(&dir_of(root, "titles"), name, "json")?;
    let stored: StoredTitle = serde_json::from_str(&text).ok()?;
    Some(stored.title.sanitized())
}

/// An installed transition and its length.
pub(crate) fn load_transition(name: &str) -> Option<TransitionTemplate> {
    let root = data_dir()?;
    load_transition_in(&root, name)
}

fn load_transition_in(root: &Path, name: &str) -> Option<TransitionTemplate> {
    let text = read_text(&dir_of(root, "transitions"), name, "json")?;
    let stored: StoredTransition = serde_json::from_str(&text).ok()?;
    Some(stored.transition)
}

/// An installed look-up table, read and checked again: a file edited by hand is not trusted.
pub(crate) fn load_lut(name: &str) -> Option<Lut> {
    let root = data_dir()?;
    load_lut_in(&root, name)
}

fn load_lut_in(root: &Path, name: &str) -> Option<Lut> {
    let text = read_text(&dir_of(root, "luts"), name, "cube")?;
    let lut = Lut::parse_cube(&text, name).ok()?;
    lut.is_valid().then_some(lut)
}

/// A title as it is written to disk.
#[derive(Serialize, Deserialize)]
struct StoredTitle {
    name: String,
    title: TitleText,
}

/// A transition as it is written to disk.
#[derive(Serialize, Deserialize)]
struct StoredTransition {
    name: String,
    transition: TransitionTemplate,
}

fn write_json<T: Serialize>(dir: &Path, name: &str, value: &T) -> bool {
    let Some(file) = crate::effect_presets::file_name_with(name, "json") else {
        return false;
    };
    let Ok(text) = serde_json::to_string_pretty(value) else {
        return false;
    };
    std::fs::create_dir_all(dir).is_ok() && std::fs::write(dir.join(file), text).is_ok()
}

fn write_text(dir: &Path, name: &str, ext: &str, text: &str) -> bool {
    let Some(file) = crate::effect_presets::file_name_with(name, ext) else {
        return false;
    };
    std::fs::create_dir_all(dir).is_ok() && std::fs::write(dir.join(file), text).is_ok()
}

/// The names of the files in `dir` with extension `ext`, in alphabetical order: only names that
/// are found again under the name they are shown with.
fn list_in(dir: &Path, ext: &str) -> Vec<String> {
    let suffix = format!(".{ext}");
    let mut names = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let file_name = e.file_name();
            let Some(stem) = file_name.to_str().and_then(|n| n.strip_suffix(&suffix)) else {
                continue;
            };
            if crate::effect_presets::file_name_with(stem, ext)
                .is_some_and(|f| f == format!("{stem}{suffix}"))
            {
                names.push(stem.to_string());
            }
        }
    }
    names.sort();
    names.truncate(MAX_LIBRARY);
    names
}

fn read_text(dir: &Path, name: &str, ext: &str) -> Option<String> {
    let path = dir.join(crate::effect_presets::file_name_with(name, ext)?);
    let meta = std::fs::metadata(&path).ok()?;
    if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
        return None;
    }
    std::fs::read_to_string(&path).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "picasso_bundles_{tag}_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// A 2-cube that maps every colour to itself.
    fn identity_cube() -> String {
        let mut s = String::from("TITLE \"Identity\"\nLUT_3D_SIZE 2\n");
        for b in 0..2 {
            for g in 0..2 {
                for r in 0..2 {
                    s += &format!("{r} {g} {b}\n");
                }
            }
        }
        s
    }

    /// A bundle with an effect template, a title, a transition and a LUT.
    fn bundle() -> Result<Vec<u8>, String> {
        let mut out = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut out);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            let entries: [(&str, String); 4] = [
                (
                    "EffectTemplates/Warm Film.json",
                    r#"{"name":"Warm Film","effects":[{"kind":"Diffusion","amount":40.0}]}"#
                        .to_string(),
                ),
                (
                    "Titles/Lower Third.json",
                    r#"{"name":"Lower Third","title":{"text":"Your Name","size":900}}"#.to_string(),
                ),
                (
                    "Transitions/Glitch Cut.json",
                    r#"{"name":"Glitch Cut","transition":{"kind":"Glitch","frames":12}}"#
                        .to_string(),
                ),
                ("LUTs/Identity.cube", identity_cube()),
            ];
            for (name, data) in entries {
                zip.start_file(name, options).map_err(|e| e.to_string())?;
                zip.write_all(data.as_bytes()).map_err(|e| e.to_string())?;
            }
            zip.finish().map_err(|e| e.to_string())?;
        }
        Ok(out.into_inner())
    }

    #[test]
    fn a_bundle_installs_into_the_four_folders_and_is_offered() -> Result<(), String> {
        let root = temp_dir("install");
        let installed = install_in(&root, "Warm Looks.picef", &bundle()?)?;
        assert_eq!(installed.bundle, "Warm Looks");
        assert_eq!(installed.total(), 4, "{installed:?}");
        assert!(installed.skipped.is_empty() && installed.refused.is_empty());

        // The effect template is in the Inspector's list, and loads back in range.
        let library = library_in(&root);
        assert_eq!(library.effects, ["Warm-Film"]);
        assert_eq!(library.titles, ["Lower-Third"]);
        assert_eq!(library.transitions, ["Glitch-Cut"]);
        assert_eq!(library.luts, ["Identity"]);

        let effects = crate::effect_presets::load_in(&dir_of(&root, "effects"), "Warm Film")
            .ok_or("the effect template loads back")?;
        assert_eq!(effects.effects.len(), 1);

        let title = load_title_in(&root, "Lower Third").ok_or("the title loads back")?;
        assert_eq!(title.text, "Your Name");
        assert_eq!(title.size, 500, "the size is kept in range");

        let transition =
            load_transition_in(&root, "Glitch Cut").ok_or("the transition loads back")?;
        assert_eq!(transition.kind, picasso_core::TransitionKind::Glitch);
        assert_eq!(transition.frames, Some(12));

        let lut = load_lut_in(&root, "Identity").ok_or("the LUT loads back")?;
        assert!(lut.is_valid());

        // Everything is in the data folder, under the folder of its kind.
        for (kind, file) in [
            ("effects", "Warm-Film.json"),
            ("titles", "Lower-Third.json"),
            ("transitions", "Glitch-Cut.json"),
            ("luts", "Identity.cube"),
        ] {
            assert!(
                dir_of(&root, kind).join(file).is_file(),
                "{kind}/{file} is written"
            );
        }
        let _ = std::fs::remove_dir_all(&root);
        Ok(())
    }

    #[test]
    fn a_bundle_is_named_either_way_and_its_items_are_never_written_outside() -> Result<(), String> {
        let root = temp_dir("hostile");
        let mut out = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut out);
            let options = zip::write::SimpleFileOptions::default();
            let entries = [
                // An entry that tries to leave the bundle: refused before it is read.
                ("Effects/../../evil.json", r#"{"name":"evil","effects":[{"kind":"Diffusion"}]}"#),
                // A name that tries to leave the folder: it is made safe, not followed.
                ("Titles/nasty.json", r#"{"name":"../evil","title":{"text":"x"}}"#),
                // A name Windows would treat as a device: refused.
                ("Titles/CON.json", r#"{"name":"CON","title":{"text":"x"}}"#),
                ("Titles/ok.json", r#"{"name":"ok","title":{"text":"fine"}}"#),
            ];
            for (name, data) in entries {
                zip.start_file(name, options).map_err(|e| e.to_string())?;
                zip.write_all(data.as_bytes()).map_err(|e| e.to_string())?;
            }
            zip.finish().map_err(|e| e.to_string())?;
        }
        let installed = install_in(&root, "b.picef", &out.into_inner())?;
        assert_eq!(installed.titles, 2, "{installed:?}");
        assert_eq!(installed.refused.len(), 1, "{installed:?}");
        assert_eq!(installed.skipped.len(), 1, "{installed:?}");
        let summary = installed.summary();
        assert!(summary.contains("1 item(s) could not be named"), "{summary}");
        assert!(summary.contains("1 file(s) left out"), "{summary}");
        // The name with `..` in it is written inside the titles folder, not above it.
        assert!(root.join("presets/titles/..-evil.json").is_file());
        assert!(!root.join("evil.json").exists());
        assert!(!root.join("presets/evil.json").exists());
        let _ = std::fs::remove_dir_all(&root);
        Ok(())
    }

    #[test]
    fn a_bundle_that_is_not_one_writes_nothing() {
        let root = temp_dir("garbage");
        let installed = install_in(&root, "bad.picef", b"not a zip");
        assert!(installed.is_err());
        assert!(!root.join("presets").exists(), "nothing was written");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_installed_file_edited_by_hand_is_not_trusted() -> Result<(), String> {
        let root = temp_dir("hand");
        let dir = dir_of(&root, "luts");
        let titles = dir_of(&root, "titles");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        std::fs::create_dir_all(&titles).map_err(|e| e.to_string())?;
        std::fs::write(dir.join("Broken.cube"), "LUT_3D_SIZE 9999\n")
            .map_err(|e| e.to_string())?;
        std::fs::write(titles.join("Bad.json"), "{not json").map_err(|e| e.to_string())?;
        assert!(load_lut_in(&root, "Broken").is_none());
        assert!(load_title_in(&root, "Bad").is_none());
        // A file over the size cap is not read.
        std::fs::write(dir.join("Big.cube"), "x".repeat(2 << 20)).map_err(|e| e.to_string())?;
        assert!(load_lut_in(&root, "Big").is_none());
        let _ = std::fs::remove_dir_all(&root);
        Ok(())
    }
}
