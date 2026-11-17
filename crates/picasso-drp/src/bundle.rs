//! Reads effect bundles: `.picef` files — a ZIP archive holding effect templates, titles,
//! transitions and look-up tables.
//!
//! A bundle is untrusted input, so it is read the way the `.drp` reader reads its archives:
//!
//! * the file itself, every entry and the total read are capped, and each entry is read through
//!   [`Read::take`], so a zip bomb cannot fill memory;
//! * an entry's path is refused when it is absolute, holds `..`, a drive letter (`:`), a
//!   backslash or a control character — nothing in a bundle may name a place to write to;
//! * what is kept is only what this program can use: `.cube` look-up tables (parsed and checked
//!   by [`picasso_color::Lut`]) and `.json` descriptors of an effect template, a title or a
//!   transition. Icons, manifests and everything else are ignored.
//!
//! A descriptor is one of:
//!
//! ```json
//! {"name": "Warm Film", "effects": [{"kind": "ColorTemperature", "amount": 60.0}]}
//! {"name": "Lower Third", "title": {"text": "Your Name"}}
//! {"name": "Glitch Cut", "transition": {"kind": "Glitch", "frames": 12}}
//! ```
//!
//! The names are the bundle's; where they are installed is up to the caller (the interface keeps
//! them in the user's data folder, under file names it makes safe itself).

use std::io::{Cursor, Read};
use std::path::{Component, Path};

use picasso_color::{Effect, Lut};
use picasso_core::{TitleText, TransitionKind, MAX_EFFECTS};
use serde::{Deserialize, Serialize};

/// The extensions of the effect bundles this crate reads.
pub const BUNDLE_EXTENSIONS: [&str; 1] = ["picef"];

/// Bytes read from one bundle at most.
pub const MAX_BUNDLE_BYTES: u64 = 64 * 1024 * 1024;
/// Entries a bundle holds at most.
pub const MAX_ENTRIES: usize = 256;
/// Bytes read from one entry at most.
pub const MAX_ENTRY_BYTES: u64 = 16 * 1024 * 1024;
/// Bytes read from the whole bundle at most.
pub const MAX_TOTAL_BYTES: u64 = 64 * 1024 * 1024;
/// Items kept from one bundle at most.
pub const MAX_ITEMS: usize = 512;
/// The longest name an item keeps, in characters.
pub const MAX_NAME_CHARS: usize = 80;
/// The longest a transition may be, in frames.
pub const MAX_TRANSITION_FRAMES: u64 = 10_000;

/// Whether a file name is an effect bundle (see [`BUNDLE_EXTENSIONS`]).
pub fn is_bundle_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    BUNDLE_EXTENSIONS
        .iter()
        .any(|e| lower.ends_with(&format!(".{e}")))
}

/// Why a bundle could not be read.
#[derive(Debug, thiserror::Error)]
pub enum BundleError {
    #[error("this is not an effect bundle ({0})")]
    NotAnArchive(String),
    #[error("the bundle is too big to read")]
    TooBig,
    #[error("the bundle holds too many files")]
    TooManyEntries,
    #[error("the bundle holds no effect template, title, transition or LUT")]
    Empty,
}

/// A transition a bundle offers: which one, and how long.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransitionTemplate {
    pub kind: TransitionKind,
    /// Frames, when the bundle says; otherwise the project's one-second default is used.
    #[serde(default)]
    pub frames: Option<u64>,
}

/// One thing a bundle offers.
#[derive(Clone, Debug, PartialEq)]
pub enum BundleItem {
    /// An effect template: the effects it puts on a clip.
    Effects { name: String, effects: Vec<Effect> },
    /// A title: its words and how they look.
    Title { name: String, title: TitleText },
    /// A transition and its length.
    Transition {
        name: String,
        template: TransitionTemplate,
    },
    /// A look-up table, kept as the `.cube` text it was read from.
    Lut { name: String, cube: String },
}

impl BundleItem {
    /// The name the bundle gives this item.
    pub fn name(&self) -> &str {
        match self {
            Self::Effects { name, .. }
            | Self::Title { name, .. }
            | Self::Transition { name, .. }
            | Self::Lut { name, .. } => name,
        }
    }

    /// What kind of item this is, for a report: `"effect"`, `"title"`, `"transition"` or `"LUT"`.
    pub fn kind_label(&self) -> &'static str {
        match self {
            Self::Effects { .. } => "effect",
            Self::Title { .. } => "title",
            Self::Transition { .. } => "transition",
            Self::Lut { .. } => "LUT",
        }
    }
}

/// What a bundle holds.
#[derive(Clone, Debug, Default)]
pub struct Bundle {
    /// The bundle's name, from its file name.
    pub name: String,
    /// What it offers, in the order the archive lists it.
    pub items: Vec<BundleItem>,
    /// The entries that looked like an item but could not be read, and why.
    pub skipped: Vec<String>,
}

impl Bundle {
    /// How many items of each kind the bundle holds: `(effects, titles, transitions, LUTs)`.
    pub fn counts(&self) -> (usize, usize, usize, usize) {
        let mut counts = (0, 0, 0, 0);
        for item in &self.items {
            match item {
                BundleItem::Effects { .. } => counts.0 += 1,
                BundleItem::Title { .. } => counts.1 += 1,
                BundleItem::Transition { .. } => counts.2 += 1,
                BundleItem::Lut { .. } => counts.3 += 1,
            }
        }
        counts
    }
}

/// What an archive entry is, from its extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Lut,
    Template,
    Other,
}

fn kind_of(path: &str) -> Kind {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".cube") {
        Kind::Lut
    } else if lower.ends_with(".json") {
        Kind::Template
    } else {
        Kind::Other
    }
}

/// Whether an archive entry's name is a plain relative file path: no absolute path, no `..`, no
/// drive letter, no backslash and no control character.
fn safe_entry(name: &str) -> bool {
    if name.is_empty() || name.len() > 1024 {
        return false;
    }
    if name
        .chars()
        .any(|c| c.is_control() || c == '\\' || c == ':')
    {
        return false;
    }
    if name.starts_with('/') {
        return false;
    }
    Path::new(name)
        .components()
        .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

/// A name an item may be shown and saved under: trimmed, at most [`MAX_NAME_CHARS`] characters,
/// with control characters turned into spaces. Empty names become `Untitled`.
fn clean_name(name: &str) -> String {
    let name: String = name
        .trim()
        .chars()
        .take(MAX_NAME_CHARS)
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let name = name.trim();
    if name.is_empty() {
        "Untitled".to_string()
    } else {
        name.to_string()
    }
}

/// The name an archive entry gives an item that does not name itself: its file name without the
/// folder or the extension.
fn stem_of(path: &str) -> String {
    let file = path.rsplit('/').next().unwrap_or(path);
    let stem = file.rsplit_once('.').map_or(file, |(stem, _)| stem);
    clean_name(stem)
}

/// What a `.json` descriptor says. One of the three fields is present.
#[derive(Deserialize)]
struct Descriptor {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    effects: Option<Vec<Effect>>,
    #[serde(default)]
    title: Option<TitleText>,
    #[serde(default)]
    transition: Option<TransitionTemplate>,
}

/// A look-up table from a `.cube` entry.
fn lut_item(path: &str, data: &[u8]) -> Result<BundleItem, String> {
    let text = std::str::from_utf8(data).map_err(|_| "the file is not text".to_string())?;
    let stem = stem_of(path);
    let lut = Lut::parse_cube(text, &stem).map_err(|e| e.to_string())?;
    Ok(BundleItem::Lut {
        name: clean_name(&lut.title),
        cube: text.to_string(),
    })
}

/// An effect template, a title or a transition from a `.json` entry.
fn template_item(path: &str, data: &[u8]) -> Result<BundleItem, String> {
    let text = std::str::from_utf8(data).map_err(|_| "the file is not text".to_string())?;
    let stem = stem_of(path);
    let descriptor: Descriptor = serde_json::from_str(text)
        .map_err(|e| format!("the file is not an effect, title or transition ({e})"))?;
    let name = clean_name(descriptor.name.as_deref().unwrap_or(stem.as_str()));
    if let Some(effects) = descriptor.effects {
        if effects.is_empty() {
            return Err("the effect template has no effects".to_string());
        }
        let mut effects: Vec<Effect> = effects.into_iter().map(Effect::sanitized).collect();
        effects.truncate(MAX_EFFECTS);
        return Ok(BundleItem::Effects { name, effects });
    }
    if let Some(title) = descriptor.title {
        return Ok(BundleItem::Title {
            name,
            title: title.sanitized(),
        });
    }
    if let Some(template) = descriptor.transition {
        let frames = template
            .frames
            .map(|f| f.clamp(1, MAX_TRANSITION_FRAMES));
        return Ok(BundleItem::Transition {
            name,
            template: TransitionTemplate {
                kind: template.kind,
                frames,
            },
        });
    }
    Err("the file is not an effect, title or transition".to_string())
}

/// Reads a bundle. `file_name` is the name it was opened under, which names it.
pub fn read(bytes: &[u8], file_name: &str) -> Result<Bundle, BundleError> {
    if bytes.len() as u64 > MAX_BUNDLE_BYTES {
        return Err(BundleError::TooBig);
    }
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| BundleError::NotAnArchive(e.to_string()))?;
    if archive.len() > MAX_ENTRIES {
        return Err(BundleError::TooManyEntries);
    }
    let mut bundle = Bundle {
        name: stem_of(file_name),
        ..Bundle::default()
    };
    let mut total = 0u64;
    for i in 0..archive.len() {
        let Ok(mut entry) = archive.by_index(i) else {
            continue;
        };
        if entry.is_dir() {
            continue;
        }
        let path = entry.name().to_string();
        let kind = kind_of(&path);
        if kind == Kind::Other {
            continue;
        }
        if !safe_entry(&path) {
            bundle.skipped.push(format!("{path}: not a safe file name"));
            continue;
        }
        if entry.size() > MAX_ENTRY_BYTES {
            bundle.skipped.push(format!("{path}: too big"));
            continue;
        }
        // Read at most one byte past the cap, so an entry that lies about its size stops there.
        let mut data = Vec::new();
        if (&mut entry)
            .take(MAX_ENTRY_BYTES + 1)
            .read_to_end(&mut data)
            .is_err()
        {
            bundle.skipped.push(format!("{path}: could not be read"));
            continue;
        }
        if data.len() as u64 > MAX_ENTRY_BYTES {
            bundle.skipped.push(format!("{path}: too big"));
            continue;
        }
        total += data.len() as u64;
        if total > MAX_TOTAL_BYTES {
            return Err(BundleError::TooBig);
        }
        let made = match kind {
            Kind::Lut => lut_item(&path, &data),
            Kind::Template => template_item(&path, &data),
            Kind::Other => continue,
        };
        match made {
            Ok(item) if bundle.items.len() < MAX_ITEMS => bundle.items.push(item),
            Ok(_) => bundle
                .skipped
                .push(format!("{path}: the bundle holds too many items")),
            Err(why) => bundle.skipped.push(format!("{path}: {why}")),
        }
    }
    if bundle.items.is_empty() {
        return Err(BundleError::Empty);
    }
    Ok(bundle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use picasso_color::EffectKind;
    use std::io::Write;

    /// A small archive with the given (name, contents) entries, deflated as a real bundle is.
    fn archive(entries: &[(&str, &[u8])]) -> Result<Vec<u8>, String> {
        let mut out = Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut out);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            for (name, data) in entries {
                zip.start_file(*name, options).map_err(|e| e.to_string())?;
                zip.write_all(data).map_err(|e| e.to_string())?;
            }
            zip.finish().map_err(|e| e.to_string())?;
        }
        Ok(out.into_inner())
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

    const EFFECTS: &str = r#"{"name":"Warm Film","effects":[
        {"kind":"ColorTemperature","amount":60.0},
        {"kind":"RgbMixer","amount":900.0,"params":[9,-3,1,1,1,1,1,1,1]}]}"#;
    const TITLE: &str = r#"{"name":"Lower Third","title":{"text":"Your Name","size":900}}"#;
    const TRANSITION: &str = r#"{"name":"Glitch Cut","transition":{"kind":"Glitch","frames":12}}"#;

    #[test]
    fn reads_the_four_kinds_of_item() -> Result<(), String> {
        let cube = identity_cube();
        let bytes = archive(&[
            ("manifest.xml", b"<manifest/>"),
            ("EffectTemplates/Warm Film.json", EFFECTS.as_bytes()),
            ("Titles/Lower Third.json", TITLE.as_bytes()),
            ("Transitions/Glitch Cut.json", TRANSITION.as_bytes()),
            ("LUTs/Identity.cube", cube.as_bytes()),
            ("Icons/warm.png", b"\x89PNG"),
        ])?;
        let bundle = read(&bytes, "Warm Looks.picef").map_err(|e| e.to_string())?;
        assert_eq!(bundle.name, "Warm Looks");
        assert_eq!(bundle.counts(), (1, 1, 1, 1));
        assert!(bundle.skipped.is_empty(), "{:?}", bundle.skipped);

        let names: Vec<_> = bundle
            .items
            .iter()
            .map(|i| (i.name(), i.kind_label()))
            .collect();
        assert_eq!(
            names,
            [
                ("Warm Film", "effect"),
                ("Lower Third", "title"),
                ("Glitch Cut", "transition"),
                ("Identity", "LUT"),
            ]
        );

        // The effect template's numbers are kept in range, as a hand-written file's are.
        let Some(BundleItem::Effects { effects, .. }) = bundle.items.first() else {
            return Err("the effect template is first".to_string());
        };
        assert_eq!(effects.len(), 2);
        assert_eq!(effects[0].kind, EffectKind::ColorTemperature);
        assert_eq!(effects[1].amount, 100.0);
        assert_eq!(effects[1].params[..2], [2.0, 0.0]);

        // The title's size is in range, and the transition keeps its length.
        let Some(BundleItem::Title { title, .. }) = bundle.items.get(1) else {
            return Err("the title is second".to_string());
        };
        assert_eq!(title.text, "Your Name");
        assert_eq!(title.size, 500);
        let Some(BundleItem::Transition { template, .. }) = bundle.items.get(2) else {
            return Err("the transition is third".to_string());
        };
        assert_eq!(template.kind, TransitionKind::Glitch);
        assert_eq!(template.frames, Some(12));

        // The LUT is kept as its `.cube` text, which reads back.
        let Some(BundleItem::Lut { cube: text, .. }) = bundle.items.get(3) else {
            return Err("the LUT is fourth".to_string());
        };
        assert!(Lut::parse_cube(text, "x").is_ok());
        Ok(())
    }

    #[test]
    fn an_item_without_a_name_is_named_by_its_file() -> Result<(), String> {
        let bytes = archive(&[(
            "Effects/Cool.json",
            br#"{"effects":[{"kind":"Diffusion","amount":40.0}]}"#,
        )])?;
        let bundle = read(&bytes, "b.picef").map_err(|e| e.to_string())?;
        assert_eq!(bundle.items[0].name(), "Cool");
        Ok(())
    }

    #[test]
    fn entries_that_are_not_items_are_left_out_with_a_reason() -> Result<(), String> {
        let bytes = archive(&[
            ("Effects/good.json", EFFECTS.as_bytes()),
            ("Effects/broken.json", b"{not json"),
            ("Effects/empty.json", br#"{"effects":[]}"#),
            ("LUTs/bad.cube", b"LUT_3D_SIZE 9999\n"),
            ("LUTs/binary.cube", b"\xff\xfe\x00"),
        ])?;
        let bundle = read(&bytes, "b.picef").map_err(|e| e.to_string())?;
        assert_eq!(bundle.items.len(), 1);
        assert_eq!(bundle.skipped.len(), 4, "{:?}", bundle.skipped);
        assert!(bundle.skipped.iter().all(|s| s.contains(':')));
        Ok(())
    }

    /// Hostile files: none may panic, and nothing may name a place outside the bundle.
    #[test]
    fn hostile_bundles_are_refused() -> Result<(), String> {
        // Path traversal, an absolute path, a drive letter and a control character.
        let bytes = archive(&[
            ("Effects/../../../evil.json", EFFECTS.as_bytes()),
            ("/etc/evil.json", EFFECTS.as_bytes()),
            ("C:/Windows/evil.json", EFFECTS.as_bytes()),
            ("Effects/ev\til.json", EFFECTS.as_bytes()),
            ("Effects/ok.json", br#"{"effects":[{"kind":"Diffusion","amount":10.0}]}"#),
        ])?;
        let bundle = read(&bytes, "b.picef").map_err(|e| e.to_string())?;
        let names: Vec<_> = bundle.items.iter().map(|i| i.name()).collect();
        assert_eq!(bundle.items.len(), 1, "items: {names:?}");
        assert_eq!(bundle.items[0].name(), "ok", "items: {names:?}");
        assert_eq!(bundle.skipped.len(), 4, "{:?}", bundle.skipped);

        // Not a ZIP at all, and nothing this program can use.
        assert!(matches!(
            read(b"plain text", "b.picef"),
            Err(BundleError::NotAnArchive(_))
        ));
        assert!(matches!(read(&[], "b.picef"), Err(BundleError::NotAnArchive(_))));
        let icons = archive(&[("Icons/a.png", b"\x89PNG")])?;
        assert!(matches!(read(&icons, "b.picef"), Err(BundleError::Empty)));

        // More entries than a bundle may hold.
        let many: Vec<(String, &[u8])> = (0..MAX_ENTRIES + 1)
            .map(|i| (format!("Icons/{i}.png"), &b"x"[..]))
            .collect();
        let refs: Vec<(&str, &[u8])> = many.iter().map(|(n, d)| (n.as_str(), *d)).collect();
        let too_many = archive(&refs)?;
        assert!(matches!(
            read(&too_many, "b.picef"),
            Err(BundleError::TooManyEntries)
        ));

        // A file larger than a bundle may be.
        let big = vec![0u8; MAX_BUNDLE_BYTES as usize + 1];
        assert!(matches!(read(&big, "b.picef"), Err(BundleError::TooBig)));
        Ok(())
    }

    /// A zip bomb: entries that expand far past the cap are refused, not held in memory.
    #[test]
    fn a_zip_bomb_is_refused() -> Result<(), String> {
        // One entry of 32 MB of spaces, which compresses to a few KB.
        let filler = vec![b' '; 32 * 1024 * 1024];
        let bomb = archive(&[("Effects/bomb.json", &filler)])?;
        assert!(bomb.len() < 1024 * 1024, "bomb is {} bytes", bomb.len());
        let bundle = read(&bomb, "b.picef");
        // It is either refused whole (no usable item) or left out entry by entry.
        match bundle {
            Ok(b) => {
                assert!(b.items.is_empty());
                assert_eq!(b.skipped.len(), 1);
            }
            Err(BundleError::Empty) => {}
            Err(other) => return Err(format!("unexpected {other}")),
        }
        Ok(())
    }

    #[test]
    fn names_and_extensions_are_recognised() {
        assert!(is_bundle_name("Warm.picef") && is_bundle_name("a.PICEF"));
        assert!(!is_bundle_name("a.zip") && !is_bundle_name("picef"));
        assert!(!is_bundle_name("a.drfx") && !is_bundle_name("a.DRFX"));
        assert_eq!(clean_name("  a\nb  "), "a b");
        assert_eq!(clean_name("   "), "Untitled");
        assert_eq!(stem_of("a/b/Cool.cube"), "Cool");
        assert_eq!(stem_of("Cool.cube"), "Cool");
        assert!(safe_entry("Effects/a.json"));
        assert!(!safe_entry("../a.json"));
        assert!(!safe_entry("a/../../b.json"));
        assert!(!safe_entry("/a.json"));
        assert!(!safe_entry("C:\\a.json"));
    }
}
