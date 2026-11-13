//! Saved layouts (Workspace › Layout › Save Layout and Load Layout): the sizes of the panels on
//! the pages under a name, in `layouts/<file>.txt` in the data folder. A layout file holds a
//! `name=...` line and then the same lines the preferences use for the sizes
//! (`crate::prefs::panels_text`), so it can be read and edited by hand.
//!
//! A layout is found by its file, not by its name, so a file renamed by hand still loads; saving
//! under a name that is already there replaces that layout, whatever its file is called.
//!
//! The browser has no data folder: there, no layouts are kept, as with the preferences.

use std::path::{Path, PathBuf};

use crate::panels::Panels;

/// The folder inside the data folder that the layouts are in.
const FOLDER: &str = "layouts";
/// A longer name is cut: no real layout has one.
const MAX_NAME: usize = 40;
/// At most this many layouts are listed.
const MAX_LAYOUTS: usize = 64;
/// A bigger file is no layout: a real one is a few hundred bytes.
const MAX_BYTES: u64 = 64 * 1024;

/// A saved layout: the name it shows and the file it is in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Layout {
    pub name: String,
    path: PathBuf,
}

/// The folder the layouts are in (it is made by the first save).
fn folder() -> Option<PathBuf> {
    Some(crate::project_manager::data_dir()?.join(FOLDER))
}

/// The name as it is kept: trimmed, without control characters (a new line would start a line of
/// its own in the file), and cut to `MAX_NAME`.
fn clean_name(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .trim()
        .chars()
        .take(MAX_NAME)
        .collect()
}

/// Names Windows keeps for devices, with any extension: `con.txt` is the console, not a file.
fn reserved(stem: &str) -> bool {
    let s = stem.to_ascii_lowercase();
    matches!(s.as_str(), "con" | "prn" | "aux" | "nul")
        || (s.len() == 4
            && (s.starts_with("com") || s.starts_with("lpt"))
            && s.ends_with(|c: char| ('1'..='9').contains(&c)))
}

/// The name as a file name: letters, digits, `-` and `_`; anything else becomes `-`. So a layout
/// called `../x` writes `x.txt` in the folder and not outside it, and one called `con` does not
/// write to a Windows device.
fn slug(name: &str) -> String {
    let clean: String = clean_name(name)
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect();
    let clean = clean.trim_matches('-');
    if clean.is_empty() {
        "layout".to_string()
    } else if reserved(clean) {
        format!("{clean}-layout")
    } else {
        clean.to_string()
    }
}

/// The text of a layout file, unless it is not there or too big to be one.
fn read(path: &Path) -> Option<String> {
    let size = std::fs::metadata(path).ok()?.len();
    (size <= MAX_BYTES).then(|| std::fs::read_to_string(path).ok())?
}

/// The name a layout file gives itself, or its file name when the line is missing.
fn name_of(text: &str, file: &str) -> String {
    let named = text
        .lines()
        .find_map(|l| l.trim_end_matches('\r').strip_prefix("name="))
        .map(clean_name)
        .filter(|n| !n.is_empty());
    named.unwrap_or_else(|| clean_name(file))
}

/// The layouts in `dir`, by name. The files are taken in the order of their names before any is
/// read, so which ones are listed when there are too many does not depend on the disk.
fn list_in(dir: &Path) -> Vec<Layout> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "txt") && p.is_file())
        .collect();
    paths.sort();
    let mut layouts: Vec<Layout> = paths
        .into_iter()
        .take(MAX_LAYOUTS)
        .filter_map(|path| {
            let file = path.file_stem()?.to_string_lossy().into_owned();
            let name = name_of(&read(&path)?, &file);
            Some(Layout { name, path })
        })
        .collect();
    layouts.sort_by_key(|l| l.name.to_lowercase());
    layouts
}

/// The layout called `name` in `dir`, upper or lower case alike.
fn find_in(dir: &Path, name: &str) -> Option<Layout> {
    let name = clean_name(name).to_lowercase();
    list_in(dir)
        .into_iter()
        .find(|l| l.name.to_lowercase() == name)
}

/// Saves `panels` as `name` in `dir`: over the layout of that name when there is one, otherwise in
/// a file of its own, so two names that make the same file name (`My Layout`, `my-layout`) do not
/// replace each other.
fn save_in(dir: &Path, name: &str, panels: &Panels) -> std::io::Result<()> {
    let name = clean_name(name);
    let text = format!("name={name}\n{}", crate::prefs::panels_text(panels));
    std::fs::create_dir_all(dir)?;
    let path = match find_in(dir, &name) {
        Some(layout) => layout.path,
        None => {
            let base = slug(&name);
            // Upper and lower case are the same file on Windows and macOS, so they are taken as
            // the same here too.
            let taken: Vec<String> = std::fs::read_dir(dir)?
                .flatten()
                .map(|e| e.file_name().to_string_lossy().to_lowercase())
                .collect();
            (1..)
                .map(|n| if n == 1 { format!("{base}.txt") } else { format!("{base}-{n}.txt") })
                .find(|f| !taken.contains(&f.to_lowercase()))
                .map(|f| dir.join(f))
                .unwrap_or_else(|| dir.join(format!("{base}.txt")))
        }
    };
    std::fs::write(path, text)
}

/// The saved layouts, by name.
pub(crate) fn list() -> Vec<Layout> {
    folder().map_or_else(Vec::new, |dir| list_in(&dir))
}

/// Saves `panels` as the layout `name`, replacing one of the same name. The error is the reason
/// it could not be written, to show as it is.
pub(crate) fn save(name: &str, panels: &Panels) -> Result<(), String> {
    if clean_name(name).is_empty() {
        return Err("a layout needs a name".into());
    }
    let dir = folder().ok_or("there is nowhere to save it on this computer")?;
    save_in(&dir, name, panels).map_err(|e| format!("the layout could not be saved: {e}"))
}

/// The panels of a saved layout; `None` when its file is gone or damaged.
pub(crate) fn load(layout: &Layout) -> Option<Panels> {
    crate::prefs::panels_of(&read(&layout.path)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder of its own for one test, made fresh and left behind for the next run to replace.
    /// The process id keeps two test runs at once apart.
    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "picasso_layouts_{}_{tag}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn names_in(dir: &Path) -> Vec<String> {
        list_in(dir).into_iter().map(|l| l.name).collect()
    }

    fn load_in(dir: &Path, name: &str) -> Option<Panels> {
        load(&find_in(dir, name)?)
    }

    #[test]
    fn layouts_read_back_as_they_were_saved() {
        let dir = scratch("round_trip");
        let mut panels = Panels::default();
        panels.set_edit_column(1, 600.0);
        panels.cut_upper = 400;
        panels.moved.insert("color gallery".into(), -35);
        save_in(&dir, "Editing", &panels).unwrap();
        assert_eq!(load_in(&dir, "Editing"), Some(panels.clone()));
        // The same name again replaces the first, and keeps its place in the list.
        let mut other = Panels::default();
        other.cut_pool = 800;
        save_in(&dir, "Editing", &other).unwrap();
        assert_eq!(load_in(&dir, "Editing"), Some(other.clone()));
        save_in(&dir, "Colour", &panels).unwrap();
        assert_eq!(names_in(&dir), ["Colour", "Editing"]);
        assert_eq!(load_in(&dir, "Colour"), Some(panels));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_name_stays_inside_the_folder_and_a_broken_file_is_not_a_layout() {
        let dir = scratch("names");
        save_in(&dir, "../../outside", &Panels::default()).unwrap();
        assert_eq!(names_in(&dir), ["../../outside"]);
        // Nothing was written outside the folder.
        assert!(!dir.join("../outside.txt").exists());
        // A file without a readable `panels=` line is listed but does not load as the defaults.
        std::fs::write(dir.join("damaged.txt"), "name=Damaged\npanels=1,2,3\n").unwrap();
        assert_eq!(names_in(&dir), ["../../outside", "Damaged"]);
        assert_eq!(load_in(&dir, "Damaged"), None);
        assert_eq!(load_in(&dir, "Not here"), None);
        // A name that cannot be a file name at all still lands in the folder.
        assert_eq!(slug("///"), "layout");
        assert_eq!(slug("  a b  "), "a-b");
        assert_eq!(slug(""), "layout");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn names_that_make_the_same_file_name_keep_their_own_files() {
        let dir = scratch("clash");
        let mut wide = Panels { cut_pool: 800, ..Default::default() };
        save_in(&dir, "My Layout", &Panels::default()).unwrap();
        save_in(&dir, "my-layout", &wide).unwrap();
        assert_eq!(names_in(&dir), ["My Layout", "my-layout"]);
        assert_eq!(load_in(&dir, "My Layout"), Some(Panels::default()));
        assert_eq!(load_in(&dir, "my-layout"), Some(wide.clone()));
        // The same name in other letters is the same layout, and is replaced.
        save_in(&dir, "MY LAYOUT", &wide).unwrap();
        assert_eq!(list_in(&dir).len(), 2);
        assert_eq!(load_in(&dir, "My Layout"), Some(wide));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_renamed_by_hand_still_loads() {
        let dir = scratch("renamed");
        let mut panels = Panels { cut_upper: 400, ..Default::default() };
        save_in(&dir, "Editing", &panels).unwrap();
        std::fs::rename(dir.join("Editing.txt"), dir.join("work.txt")).unwrap();
        assert_eq!(names_in(&dir), ["Editing"]);
        assert_eq!(load_in(&dir, "Editing"), Some(panels));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn windows_device_names_and_new_lines_are_not_kept() {
        assert_eq!(slug("con"), "con-layout");
        assert_eq!(slug("NUL"), "NUL-layout");
        assert_eq!(slug("com1"), "com1-layout");
        assert_eq!(slug("lpt9"), "lpt9-layout");
        assert_eq!(slug("console"), "console");
        assert_eq!(slug("com0"), "com0");
        let dir = scratch("lines");
        save_in(&dir, "Mine\npanels=1,2,3", &Panels::default()).unwrap();
        let text = std::fs::read_to_string(&list_in(&dir)[0].path).unwrap();
        assert_eq!(text.lines().filter(|l| l.starts_with("panels=")).count(), 1);
        assert_eq!(load_in(&dir, "Mine panels=1,2,3"), Some(Panels::default()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_layout_carries_the_sizes_of_every_page() {
        let dir = scratch("every_page");
        let mut panels = Panels::default();
        for (i, id) in ["media files", "photo pool", "fx nodes", "deliver queue"]
            .into_iter()
            .enumerate()
        {
            panels.moved.insert(id.into(), i as i16 - 2);
        }
        save_in(&dir, "Mine", &panels).unwrap();
        let read = load_in(&dir, "Mine").unwrap();
        assert_eq!(read, panels, "a divider moved on any page comes back moved");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
