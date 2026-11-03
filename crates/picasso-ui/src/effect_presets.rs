//! Save and load effects presets as JSON files in the user data folder.
//!
//! A preset is a named list of effects (with their amount, params, and enabled state)
//! that can be applied to a clip in one click. Presets are stored as
//! `~/.local/share/picasso-suite/presets/effects/<name>.json`.
//!
//! This mirrors how DaVinci Resolve lets you save an effect stack and recall it later,
//! and is a daily workflow for editors who apply the same look repeatedly.

use std::path::{Path, PathBuf};

use picasso_color::Effect;
use serde::{Deserialize, Serialize};

use crate::project_manager::data_dir;

/// A saved effects preset: a name and the list of effects it restores.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EffectPreset {
    pub name: String,
    pub effects: Vec<Effect>,
}

/// The folder presets live in: `<data_dir>/presets/effects/<name>.json`.
pub(crate) fn preset_dir() -> Option<PathBuf> {
    data_dir().map(|d| d.join("presets").join("effects"))
}

/// The file name a preset with `name` is saved under (`<name>.json`, the form `list_in` looks
/// for). `None` when the name leaves nothing to name a file by.
fn file_name(name: &str) -> Option<String> {
    let stem = sanitize_for_filename(name.trim());
    if stem.chars().all(|c| matches!(c, '-' | '.')) || stem.len() > MAX_NAME_BYTES {
        return None;
    }
    // Windows treats these as devices whatever the extension.
    let base = stem.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((base.starts_with("COM") || base.starts_with("LPT"))
            && base.len() == 4
            && base.as_bytes()[3].is_ascii_digit());
    if reserved {
        return None;
    }
    Some(format!("{stem}.json"))
}

/// The longest preset name that is saved (as a file name stem, in bytes).
const MAX_NAME_BYTES: usize = 100;
/// The most presets that are listed, and the largest preset file that is read: a folder full of
/// other things must not make the Inspector slow or read gigabytes.
const MAX_PRESETS: usize = 500;
const MAX_FILE_BYTES: u64 = 1 << 20;

/// Replaces characters that are unsafe in file names with `-`.
fn sanitize_for_filename(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' => c,
            _ => '-',
        })
        .collect()
}

/// Lists the preset names saved on disk, in alphabetical order (the same order the Inspector
/// shows them in).
pub(crate) fn list_presets() -> Vec<String> {
    preset_dir().map(|d| list_in(&d)).unwrap_or_default()
}

/// Saves the given effects as a preset named `name`.
pub(crate) fn save_preset(name: &str, effects: &[Effect]) -> bool {
    preset_dir().is_some_and(|d| save_in(&d, name, effects))
}

/// Loads a preset by name, returning its effects.
pub(crate) fn load_preset(name: &str) -> Option<EffectPreset> {
    load_in(&preset_dir()?, name)
}

fn list_in(dir: &Path) -> Vec<String> {
    let mut names = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            if let Some(stem) = e.file_name().to_str().and_then(|n| n.strip_suffix(".json")) {
                // Only names that load back under the name they are shown with.
                if file_name(stem).is_some_and(|f| f == format!("{stem}.json")) {
                    names.push(stem.to_string());
                }
            }
        }
    }
    names.sort();
    names.truncate(MAX_PRESETS);
    names
}

fn save_in(dir: &Path, name: &str, effects: &[Effect]) -> bool {
    let Some(file) = file_name(name) else {
        return false;
    };
    let preset = EffectPreset {
        name: name.trim().to_string(),
        effects: effects.to_vec(),
    };
    let Ok(json) = serde_json::to_string_pretty(&preset) else {
        return false;
    };
    std::fs::create_dir_all(dir).is_ok() && std::fs::write(dir.join(file), json).is_ok()
}

fn load_in(dir: &Path, name: &str) -> Option<EffectPreset> {
    let path = dir.join(file_name(name)?);
    // A regular file of a sane size: not a device, a link to one, or a huge file.
    let meta = std::fs::metadata(&path).ok()?;
    if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
        return None;
    }
    let json = std::fs::read_to_string(&path).ok()?;
    let mut preset: EffectPreset = serde_json::from_str(&json).ok()?;
    // A file can be edited by hand: keep every value in the range its effect allows.
    preset.effects = preset.effects.into_iter().map(Effect::sanitized).collect();
    preset.effects.truncate(picasso_core::MAX_EFFECTS);
    Some(preset)
}

#[cfg(test)]
mod tests {
    use super::*;
    use picasso_color::EffectKind;

    #[test]
    fn presets_round_trip_through_json() {
        let effects = vec![
            Effect::new(EffectKind::RgbMixer),
            Effect { amount: 50.0, kind: EffectKind::Diffusion, ..Effect::new(EffectKind::Diffusion) },
        ];
        let json = serde_json::to_string(&effects).unwrap();
        let loaded: Vec<Effect> = serde_json::from_str(&json).unwrap();
        assert_eq!(effects, loaded);
    }

    #[test]
    fn preset_struct_round_trips() {
        let preset = EffectPreset {
            name: "My Look".to_string(),
            effects: vec![Effect::new(EffectKind::ColorTemperature)],
        };
        let json = serde_json::to_string(&preset).unwrap();
        let loaded: EffectPreset = serde_json::from_str(&json).unwrap();
        assert_eq!(preset.name, loaded.name);
        assert_eq!(preset.effects.len(), loaded.effects.len());
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("picasso_presets_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_saved_preset_is_listed_and_loads_back() {
        let dir = temp_dir("roundtrip");
        let effects = vec![Effect::new(EffectKind::RgbMixer), Effect::new(EffectKind::Diffusion)];
        assert!(save_in(&dir, "My Look", &effects));
        assert!(save_in(&dir, "Another", &effects[..1]));
        assert_eq!(list_in(&dir), ["Another", "My-Look"]);
        let loaded = load_in(&dir, "My Look").expect("the preset loads by the name it was saved under");
        assert_eq!(loaded.effects, effects);
        assert_eq!(loaded.name, "My Look");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn names_with_nothing_to_name_a_file_by_save_nothing() {
        let dir = temp_dir("names");
        for name in ["", "   ", ".", "..", "!!!", "CON", "nul.x", "com1", "LPT9", &"a".repeat(200)] {
            assert!(!save_in(&dir, name, &[]), "{name:?}");
        }
        assert!(list_in(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_preset_edited_by_hand_is_kept_in_range() {
        let dir = temp_dir("range");
        std::fs::create_dir_all(&dir).unwrap();
        // JSON has no NaN; the out-of-range numbers it can hold are what a hand edit makes.
        let one = r#"{"kind":"RgbMixer","amount":900.0,"enabled":true,"params":[9.0,-3.0,1,1,1,1,1,1,1]}"#;
        let json = format!(r#"{{"name":"bad","effects":[{}]}}"#, vec![one; 40].join(","));
        std::fs::write(dir.join("bad.json"), json).unwrap();
        let loaded = load_in(&dir, "bad").unwrap();
        assert_eq!(loaded.effects.len(), picasso_core::MAX_EFFECTS);
        assert_eq!(loaded.effects[0].params[..2], [2.0, 0.0]);
        assert_eq!(loaded.effects[0].amount, 100.0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_loadable_regular_files_are_listed_and_read() {
        let dir = temp_dir("junk");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("good.json"), r#"{"name":"good","effects":[]}"#).unwrap();
        std::fs::write(dir.join("has space.json"), "{}").unwrap(); // would not load back
        std::fs::write(dir.join("notes.txt"), "x").unwrap();
        std::fs::create_dir_all(dir.join("folder.json")).unwrap();
        let big = format!(r#"{{"name":"big","effects":[],"pad":"{}"}}"#, "x".repeat(2 << 20));
        std::fs::write(dir.join("big.json"), big).unwrap();
        assert!(load_in(&dir, "good").is_some());
        assert!(load_in(&dir, "big").is_none(), "a file over the size cap is not read");
        assert!(load_in(&dir, "folder").is_none(), "a folder is not a preset");
        let listed = list_in(&dir);
        assert!(listed.contains(&"good".to_string()));
        assert!(!listed.contains(&"has space".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sanitize_makes_filenames_safe() {
        assert_eq!(sanitize_for_filename("My Preset!"), "My-Preset-");
        assert_eq!(sanitize_for_filename("OK"), "OK");
        assert_eq!(sanitize_for_filename("path/slashes"), "path-slashes");
    }
}
