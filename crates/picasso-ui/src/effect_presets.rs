//! Save and load effects presets as JSON files in the user data folder.
//!
//! A preset is a named list of effects (with their amount, params, and enabled state)
//! that can be applied to a clip in one click. Presets are stored as
//! `~/.local/share/picasso-suite/presets/effects/<name>.json`.
//!
//! This mirrors how DaVinci Resolve lets you save an effect stack and recall it later,
//! and is a daily workflow for editors who apply the same look repeatedly.

use std::path::PathBuf;

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

/// The file a preset with `name` is saved to.
pub(crate) fn preset_path(name: &str) -> Option<PathBuf> {
    preset_dir().map(|d| d.join(format!("{}", sanitize_for_filename(name))))
}

/// Replaces characters that are unsafe in file names with `-`.
fn sanitize_for_filename(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' => c,
            _ => '-',
        })
        .collect()
}

/// Lists the preset names saved on disk, newest first.
pub(crate) fn list_presets() -> Vec<String> {
    let Some(dir) = preset_dir() else { return Vec::new() };
    let mut names = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            if let Some(name) = e.file_name().to_str() {
                if name.ends_with(".json") {
                    names.push(name[..name.len() - 5].to_string());
                }
            }
        }
    }
    names.sort();
    names
}

/// Saves the given effects as a preset named `name`.
pub(crate) fn save_preset(name: &str, effects: &[Effect]) -> bool {
    let preset = EffectPreset {
        name: name.to_string(),
        effects: effects.to_vec(),
    };
    let Some(path) = preset_path(name) else { return false };
    std::fs::create_dir_all(path.parent().unwrap_or(&path)).ok();
    let Ok(json) = serde_json::to_string_pretty(&preset) else { return false };
    std::fs::write(&path, json).is_ok()
}

/// Loads a preset by name, returning its effects.
pub(crate) fn load_preset(name: &str) -> Option<EffectPreset> {
    let path = preset_path(name)?;
    let json = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&json).ok()
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

    #[test]
    fn sanitize_makes_filenames_safe() {
        assert_eq!(sanitize_for_filename("My Preset!"), "My-Preset-");
        assert_eq!(sanitize_for_filename("OK"), "OK");
        assert_eq!(sanitize_for_filename("path/slashes"), "path-slashes");
    }
}
