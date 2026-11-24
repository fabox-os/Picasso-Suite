//! The settings of this computer rather than of a project (System Preferences and Keyboard
//! Customization), in `preferences.txt` in the data folder: `hardware_decoding=on` and
//! `audio_scrubbing=on` lines, a `ui_size=0` line (how large the interface may grow, 0: fit the window), a `language=es` line (Help > Language, `crate::i18n`), a `key<TAB>Undo<TAB>Ctrl+U` line for each command whose keys were
//! changed, with the keys it has instead of the menus' (none: no key), and a
//! `panels=147,457,1186,474,960,519` line with the sizes of the panels whose dividers were
//! dragged (`crate::panels`). Lines that cannot be read are skipped, so a file from another
//! version still opens.

use std::collections::HashMap;

use crate::i18n::Language;
use crate::menu::{self, Act};
use crate::panels::Panels;

#[cfg_attr(test, allow(dead_code))] // the tests never touch the real file
const FILE: &str = "preferences.txt";
/// More lines than any real file has: the rest of a damaged or hostile file is not read.
const MAX_LINES: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Preferences {
    /// Decode H.264, H.265, AV1, VP9, MPEG-2 and VC-1 on the graphics card where it can.
    pub hardware_decoding: bool,
    /// Play short pieces of the sound while the playhead is dragged (the Clip menu's
    /// "Enable / Disable Audio Scrubbing").
    pub audio_scrubbing: bool,
    /// The language of the menus (Help > Language); English until another is chosen.
    pub language: Language,
    /// Playback > Use Optimised Media was switched on: read the small copies when there are any.
    pub optimised_media: bool,
    /// The commands whose keys were changed, and their keys.
    pub keys: HashMap<Act, Vec<String>>,
    /// The sizes of the panels on the pages.
    pub panels: Panels,
    /// How large the interface may grow, as a percentage of the operating system's scale: 0
    /// fits the window (the interface is always as wide as the window), and a number keeps the
    /// text from growing past that size in a big window, the layout then staying in the middle.
    pub ui_size: u16,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            hardware_decoding: true,
            audio_scrubbing: true,
            language: Language::English,
            optimised_media: true,
            keys: HashMap::new(),
            panels: Panels::default(),
            ui_size: 0,
        }
    }
}

/// The sizes System Preferences offers for [`Preferences::ui_size`] (0 is "fit the window").
pub(crate) const UI_SIZES: [u16; 5] = [0, 100, 125, 150, 200];

impl Preferences {
    pub(crate) fn parse(text: &str) -> Self {
        let mut prefs = Self::default();
        for line in text.lines().take(MAX_LINES) {
            let line = line.trim_end_matches('\r');
            if let Some(rest) = line.strip_prefix("key\t") {
                let mut parts = rest.split('\t');
                let Some(act) = parts.next().and_then(menu::act_named) else {
                    continue;
                };
                let keys = parts
                    .filter(|k| menu::parse_keys(k).is_some())
                    .take(8)
                    .map(str::to_string)
                    .collect();
                prefs.keys.insert(act, keys);
            } else if let Some(rest) = line.strip_prefix("panel\t") {
                let Some((name, by)) = rest.split_once('\t') else {
                    continue;
                };
                let moved = &mut prefs.panels.moved;
                if let Ok(by) = by.trim().parse::<i16>() {
                    if name.len() <= 40 && moved.len() < 64 {
                        moved.insert(name.to_string(), by);
                    }
                }
            } else if let Some((name, value)) = line.split_once('=') {
                match name.trim() {
                    "hardware_decoding" => prefs.hardware_decoding = value.trim() != "off",
                    "audio_scrubbing" => prefs.audio_scrubbing = value.trim() != "off",
                    // A language this version does not have stays English.
                    "language" => prefs.language = Language::from_code(value).unwrap_or_default(),
                    "optimised_media" => prefs.optimised_media = value.trim() != "off",
                    // Anything out of range is "fit the window".
                    "ui_size" => {
                        prefs.ui_size = value
                            .trim()
                            .parse::<u16>()
                            .ok()
                            .filter(|v| (50..=400).contains(v))
                            .unwrap_or(0);
                    }
                    "panels" => {
                        if let Some(panels) = parse_panels(value) {
                            let moved = std::mem::take(&mut prefs.panels.moved);
                            prefs.panels = Panels { moved, ..panels };
                        }
                    }
                    _ => {}
                }
            }
        }
        prefs
    }

    pub(crate) fn text(&self) -> String {
        let mut out = format!(
            "hardware_decoding={}
audio_scrubbing={}
language={}
optimised_media={}
ui_size={}
",
            if self.hardware_decoding { "on" } else { "off" },
            if self.audio_scrubbing { "on" } else { "off" },
            self.language.code(),
            if self.optimised_media { "on" } else { "off" },
            self.ui_size
        );
        out.push_str(&panels_text(&self.panels));
        let mut keys: Vec<(String, &Vec<String>)> = self
            .keys
            .iter()
            .map(|(act, keys)| (menu::act_name(*act), keys))
            .collect();
        keys.sort();
        for (name, keys) in keys {
            out.push_str("key\t");
            out.push_str(&name);
            for k in keys {
                out.push('\t');
                out.push_str(&k.replace(['\t', '\n'], ""));
            }
            out.push('\n');
        }
        out
    }

    /// Puts the preferences to work: the keys, the language, and hardware decoding for the files opened next.
    pub(crate) fn apply(&self) {
        menu::set_custom_keys(self.keys.clone());
        crate::i18n::set(self.language);
        #[cfg(not(target_arch = "wasm32"))]
        picasso_decode::hw::set_hardware_decoding(self.hardware_decoding);
    }

    /// Reads the file in the data folder; the defaults when there is none.
    #[cfg_attr(test, allow(dead_code))]
    pub(crate) fn load() -> Self {
        crate::project_manager::data_dir()
            .and_then(|dir| std::fs::read_to_string(dir.join(FILE)).ok())
            .map_or_else(Self::default, |text| Self::parse(&text))
    }

    #[cfg_attr(test, allow(dead_code))]
    pub(crate) fn save(&self) {
        let Some(dir) = crate::project_manager::data_dir() else {
            return;
        };
        if let Err(e) =
            std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(dir.join(FILE), self.text()))
        {
            tracing::warn!("could not save the preferences: {e}");
        }
    }
}

/// The six numbers of a `panels=` line; nothing unless all six read.
fn parse_panels(value: &str) -> Option<Panels> {
    let n: Vec<u16> = value
        .split(',')
        .map(|v| v.trim().parse().ok())
        .collect::<Option<_>>()?;
    let [a, b, c, viewers, pool, upper] = n[..] else {
        return None;
    };
    Some(
        Panels {
            edit_columns: [a, b, c],
            edit_viewers: viewers,
            cut_pool: pool,
            cut_upper: upper,
            moved: Default::default(),
        }
        .checked(),
    )
}

/// The lines that hold the panels' sizes: a `panels=` line, then a `panel` line for each divider
/// that was moved. A saved layout (`crate::layouts`) is these lines and nothing else.
pub(crate) fn panels_text(p: &Panels) -> String {
    let [a, b, c] = p.edit_columns;
    let mut out = format!(
        "panels={a},{b},{c},{},{},{}\n",
        p.edit_viewers, p.cut_pool, p.cut_upper
    );
    for (name, by) in &p.moved {
        out.push_str(&format!("panel\t{}\t{by}\n", name.replace(['\t', '\n'], "")));
    }
    out
}

/// The panels' sizes in lines that `panels_text` wrote; `None` when the `panels=` line is missing
/// or cannot be read (a damaged or hand-edited file), so a broken layout is not taken for the
/// default sizes.
pub(crate) fn panels_of(text: &str) -> Option<Panels> {
    let value = text
        .lines()
        .take(MAX_LINES)
        .find_map(|l| l.trim_end_matches('\r').strip_prefix("panels="))?;
    parse_panels(value)?;
    Some(Preferences::parse(text).panels)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferences_read_back_as_they_are_written() {
        let mut prefs = Preferences {
            hardware_decoding: false,
            audio_scrubbing: false,
            language: Language::Portuguese,
            ..Preferences::default()
        };
        prefs.keys.insert(Act::Undo, vec!["Ctrl+U".into()]);
        prefs.panels.set_edit_column(1, 600.0);
        prefs.panels.cut_upper = 400;
        prefs.panels.moved.insert("color gallery".into(), -35);
        prefs.keys.insert(Act::Redo, Vec::new());
        prefs
            .keys
            .insert(Act::TogglePlay, vec!["K".into(), "Ctrl+[".into()]);
        let text = prefs.text();
        assert!(
            text.starts_with("hardware_decoding=off\naudio_scrubbing=off\nlanguage=pt\n"),
            "{text}"
        );
        assert_eq!(Preferences::parse(&text), prefs);
        // Windows line ends, too.
        assert_eq!(Preferences::parse(&text.replace('\n', "\r\n")), prefs);
    }

    #[test]
    fn the_interface_size_is_kept_and_out_of_range_means_fit() {
        assert_eq!(Preferences::default().ui_size, 0);
        let prefs = Preferences {
            ui_size: 150,
            ..Preferences::default()
        };
        assert!(prefs.text().contains("ui_size=150\n"));
        assert_eq!(Preferences::parse(&prefs.text()).ui_size, 150);
        for bad in ["ui_size=7", "ui_size=9000", "ui_size=big", "ui_size=-1", "ui_size="] {
            assert_eq!(Preferences::parse(bad).ui_size, 0, "{bad}");
        }
        // A file from before this setting still opens, as "fit the window".
        assert_eq!(Preferences::parse("language=de\n").ui_size, 0);
    }

    #[test]
    fn what_cannot_be_read_is_left_out() {
        let prefs = Preferences::parse(
            "hardware_decoding=on\nkey\tNoSuchCommand\tCtrl+Q\nkey\tUndo\tCtrl+Nonsense\tCtrl+U\nrubbish\n=\n",
        );
        assert!(prefs.hardware_decoding);
        assert_eq!(prefs.keys.len(), 1);
        assert_eq!(
            prefs.keys.get(&Act::Undo),
            Some(&vec!["Ctrl+U".to_string()])
        );
        assert_eq!(Preferences::parse(""), Preferences::default());
        // A language this version does not have, or none: English.
        assert_eq!(Preferences::parse("language=klingon").language, Language::English);
        assert_eq!(Preferences::parse("language=").language, Language::English);
        assert_eq!(Preferences::parse("language=de").language, Language::German);
        // Panel sizes: all six numbers or none, and in range.
        assert_eq!(
            Preferences::parse("panels=1,2,3").panels,
            Preferences::default().panels
        );
        assert_eq!(
            Preferences::parse("panels=147,457,1186,9999,960,519")
                .panels
                .edit_viewers,
            750
        );
    }
}
