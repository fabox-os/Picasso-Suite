//! Reads DaVinci Resolve project archives (`.drp`) into Picasso Suite projects, and effect
//! bundles (`.picef`) into the templates and look-up tables they hold.
//!
//! A `.drp` file, as Resolve exports it, is a ZIP archive of XML documents. Resolve does not
//! publish what those documents hold, so this reader does not depend on their exact layout. It
//! looks through every XML document in the archive for what can be recognised on its own:
//!
//! * the project's name (an element called `ProjectName` or `Name` in the archive's
//!   `project.xml`);
//! * the frame rate (a number between 1 and 240 in an element or attribute whose name contains
//!   `FrameRate`);
//! * the picture size (numbers in elements or attributes whose names contain `Width` or `Height`
//!   together with `Resolution` or `Timeline`);
//! * every media file the project uses: any text or attribute value that is an absolute path
//!   (or a `file://` address) ending in a picture, video or sound extension.
//!
//! [`to_project`] turns what was found into a [`Project`] whose media pool holds those files.
//! Where clips sit on Resolve's timelines is not reconstructed: that needs the layout of
//! Resolve's timeline documents, which is to be written down from sample files first (see the
//! clean-room rules in the roadmap).
//!
//! The archive is untrusted input: entries, sizes and the total read are capped, and nothing
//! here panics.

use std::collections::BTreeSet;
use std::io::{Cursor, Read};

use picasso_core::{Command, Fps, MediaSource, Project};
use quick_xml::events::Event;
use quick_xml::Reader;

pub mod bundle;

/// Entries read from one archive at most.
const MAX_ENTRIES: usize = 20_000;
/// Bytes read from one entry at most.
const MAX_ENTRY_BYTES: u64 = 64 * 1024 * 1024;
/// Bytes read from the whole archive at most.
const MAX_TOTAL_BYTES: u64 = 512 * 1024 * 1024;
/// Media files kept at most.
const MAX_MEDIA: usize = 10_000;

/// The extensions of Resolve's files this crate reads: projects, timelines and bins (ZIP
/// archives of XML), and XML exports.
pub const RESOLVE_EXTENSIONS: [&str; 4] = ["drp", "drt", "drb", "xml"];

/// Whether a file name is one of Resolve's files (see [`RESOLVE_EXTENSIONS`]).
pub fn is_resolve_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    RESOLVE_EXTENSIONS
        .iter()
        .any(|e| lower.ends_with(&format!(".{e}")))
}

/// Picture, video and sound extensions recognised in paths.
pub const IMAGE_EXTENSIONS: [&str; 9] = [
    "png", "jpg", "jpeg", "tif", "tiff", "exr", "dpx", "bmp", "webp",
];
pub const VIDEO_EXTENSIONS: [&str; 16] = [
    "mp4", "mov", "mkv", "webm", "avi", "mxf", "m4v", "ts", "mts", "m2ts", "wmv", "flv", "mpg",
    "mpeg", "ogv", "braw",
];
pub const AUDIO_EXTENSIONS: [&str; 10] = [
    "wav", "mp3", "flac", "aac", "m4a", "ogg", "opus", "aif", "aiff", "wma",
];

/// Why a `.drp` file could not be read.
#[derive(Debug, thiserror::Error)]
pub enum DrpError {
    #[error("this is not a DaVinci Resolve project archive ({0})")]
    NotAnArchive(String),
    #[error("the archive holds no project documents")]
    Empty,
    #[error("the archive is too big to read")]
    TooBig,
}

/// What kind of media a path is, from its extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaKind {
    Image,
    Video,
    Audio,
}

/// A media file the project uses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DrpMedia {
    /// The path as the project stores it (a `file://` address is turned into a path).
    pub path: String,
    /// The file's name, for the media pool.
    pub name: String,
    pub kind: MediaKind,
}

/// What was found in a `.drp` archive.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DrpProject {
    /// The project's name, when the archive gives one.
    pub name: Option<String>,
    /// Frames per second, when the archive gives them.
    pub fps: Option<f64>,
    /// The picture size, when the archive gives it.
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub media: Vec<DrpMedia>,
    /// The archive's entries, for reports.
    pub entries: Vec<String>,
    /// How many of the entries were XML documents that were read.
    pub documents: usize,
}

/// The kind of media a file name has, if any.
pub fn media_kind(name: &str) -> Option<MediaKind> {
    let lower = name.to_ascii_lowercase();
    let ext = lower.rsplit_once('.')?.1;
    if IMAGE_EXTENSIONS.contains(&ext) {
        Some(MediaKind::Image)
    } else if VIDEO_EXTENSIONS.contains(&ext) {
        Some(MediaKind::Video)
    } else if AUDIO_EXTENSIONS.contains(&ext) {
        Some(MediaKind::Audio)
    } else {
        None
    }
}

/// Undoes `%xx` escapes, as in a `file://` address; anything malformed is kept as it is.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3])
                .ok()
                .and_then(|h| u8::from_str_radix(h, 16).ok());
            if let Some(b) = hex {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A media file if `value` is one: an absolute path (Unix, Windows drive or network share) or a
/// `file://` address, ending in a media extension.
pub fn as_media_path(value: &str) -> Option<DrpMedia> {
    let value = value.trim();
    if value.len() < 4 || value.len() > 4096 || value.contains('\n') {
        return None;
    }
    let path = if let Some(rest) = value.strip_prefix("file://") {
        let decoded = percent_decode(rest);
        // file:///C:/x is a Windows path; file:///x a Unix one.
        match decoded.strip_prefix('/') {
            Some(win) if win.as_bytes().get(1) == Some(&b':') => win.to_string(),
            _ => decoded,
        }
    } else {
        value.to_string()
    };
    let b = path.as_bytes();
    let absolute = path.starts_with('/')
        || path.starts_with("\\\\")
        || (b.len() > 3
            && b[0].is_ascii_alphabetic()
            && b[1] == b':'
            && (b[2] == b'\\' || b[2] == b'/'));
    if !absolute {
        return None;
    }
    let kind = media_kind(&path)?;
    let name = path
        .rsplit(['/', '\\'])
        .next()
        .filter(|n| !n.is_empty())
        .unwrap_or(&path)
        .to_string();
    Some(DrpMedia { path, name, kind })
}

/// The part of an XML name after any `prefix:`.
fn local(name: &[u8]) -> String {
    let name = String::from_utf8_lossy(name);
    name.rsplit(':').next().unwrap_or(&name).to_string()
}

/// What one document adds to the project.
#[derive(Default)]
struct Found {
    name: Option<String>,
    fps: Option<f64>,
    width: Option<u32>,
    height: Option<u32>,
    media: Vec<DrpMedia>,
}

impl Found {
    /// Looks at one named value (an element's text or an attribute).
    fn see(&mut self, field: &str, value: &str, in_project_doc: bool) {
        let value = value.trim();
        if value.is_empty() {
            return;
        }
        if let Some(m) = as_media_path(value) {
            if self.media.len() < MAX_MEDIA {
                self.media.push(m);
            }
            return;
        }
        let lower = field.to_ascii_lowercase();
        if in_project_doc
            && self.name.is_none()
            && (lower == "projectname" || lower == "name")
            && value.len() <= 256
        {
            self.name = Some(value.to_string());
        }
        if self.fps.is_none() && lower.contains("framerate") {
            if let Ok(f) = value.parse::<f64>() {
                if (1.0..=240.0).contains(&f) {
                    self.fps = Some(f);
                }
            }
        }
        let sized = lower.contains("resolution") || lower.contains("timeline");
        if sized {
            if let Ok(n) = value.parse::<u32>() {
                if (16..=16_384).contains(&n) {
                    if lower.contains("width") && self.width.is_none() {
                        self.width = Some(n);
                    } else if lower.contains("height") && self.height.is_none() {
                        self.height = Some(n);
                    }
                }
            }
        }
    }
}

/// Reads one XML document; a broken one gives what was found before the break.
fn read_document(text: &str, in_project_doc: bool) -> Found {
    let mut found = Found::default();
    let mut reader = Reader::from_str(text);
    let mut stack: Vec<String> = Vec::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let name = local(e.local_name().as_ref());
                for attr in e.attributes().flatten() {
                    let key = local(attr.key.local_name().as_ref());
                    if let Ok(v) = attr.normalized_value(quick_xml::XmlVersion::Implicit1_0) {
                        found.see(&key, &v, in_project_doc);
                    }
                }
                stack.push(name);
                if stack.len() > 512 {
                    break;
                }
            }
            Ok(Event::Empty(e)) => {
                for attr in e.attributes().flatten() {
                    let key = local(attr.key.local_name().as_ref());
                    if let Ok(v) = attr.normalized_value(quick_xml::XmlVersion::Implicit1_0) {
                        found.see(&key, &v, in_project_doc);
                    }
                }
            }
            Ok(Event::Text(t)) => {
                if let (Some(field), Ok(v)) = (stack.last(), t.decode()) {
                    let field = field.clone();
                    found.see(&field, &v, in_project_doc);
                }
            }
            Ok(Event::CData(t)) => {
                if let Some(field) = stack.last() {
                    let field = field.clone();
                    found.see(&field, &String::from_utf8_lossy(&t), in_project_doc);
                }
            }
            Ok(Event::End(_)) => {
                stack.pop();
            }
            Ok(Event::Eof) | Err(_) => break,
            Ok(_) => {}
        }
    }
    found
}

/// Reads a Resolve file: a `.drp`, `.drt` or `.drb` archive of XML documents, or one plain
/// XML document (an XML export).
pub fn read(bytes: &[u8]) -> Result<DrpProject, DrpError> {
    if bytes.len() as u64 > MAX_TOTAL_BYTES {
        return Err(DrpError::TooBig);
    }
    let head = String::from_utf8_lossy(bytes.get(..bytes.len().min(64)).unwrap_or(&[]));
    if head
        .trim_start_matches('\u{feff}')
        .trim_start()
        .starts_with('<')
    {
        return Ok(read_plain_xml(bytes));
    }
    read_archive(bytes)
}

/// One XML document on its own, as Resolve's XML exports are.
fn read_plain_xml(bytes: &[u8]) -> DrpProject {
    let text = String::from_utf8_lossy(bytes);
    let body = text.trim_start_matches('\u{feff}').trim_start();
    let found = read_document(body, true);
    let mut seen = BTreeSet::new();
    DrpProject {
        name: found.name,
        fps: found.fps,
        width: found.width,
        height: found.height,
        media: found
            .media
            .into_iter()
            .filter(|m| seen.insert(m.path.clone()))
            .collect(),
        entries: Vec::new(),
        documents: 1,
    }
}

fn read_archive(bytes: &[u8]) -> Result<DrpProject, DrpError> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| DrpError::NotAnArchive(e.to_string()))?;
    let mut project = DrpProject::default();
    let mut seen = BTreeSet::new();
    let mut total = 0u64;
    let mut media = Vec::new();
    let count = archive.len().min(MAX_ENTRIES);
    for i in 0..count {
        let Ok(mut entry) = archive.by_index(i) else {
            continue;
        };
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_string();
        project.entries.push(name.clone());
        let mut data = Vec::new();
        if (&mut entry)
            .take(MAX_ENTRY_BYTES)
            .read_to_end(&mut data)
            .is_err()
        {
            continue;
        }
        total += data.len() as u64;
        if total > MAX_TOTAL_BYTES {
            return Err(DrpError::TooBig);
        }
        let text = String::from_utf8_lossy(&data);
        let body = text.trim_start_matches('\u{feff}').trim_start();
        if !body.starts_with('<') {
            continue;
        }
        project.documents += 1;
        let file = name
            .rsplit('/')
            .next()
            .unwrap_or(&name)
            .to_ascii_lowercase();
        let found = read_document(body, file == "project.xml");
        project.name = project.name.or(found.name);
        project.fps = project.fps.or(found.fps);
        project.width = project.width.or(found.width);
        project.height = project.height.or(found.height);
        for m in found.media {
            if media.len() < MAX_MEDIA && seen.insert(m.path.clone()) {
                media.push(m);
            }
        }
    }
    if project.documents == 0 {
        return Err(DrpError::Empty);
    }
    project.media = media;
    Ok(project)
}

/// The frame rate closest to `fps`, keeping the NTSC rates exact.
pub fn fps_of(fps: f64) -> Fps {
    for (rate, num) in [(23.976, 24_000), (29.97, 30_000), (59.94, 60_000)] {
        if (fps - rate).abs() < 0.01 {
            return Fps { num, den: 1001 };
        }
    }
    let n = (fps.round() as u32).clamp(1, 240);
    Fps { num: n, den: 1 }
}

/// A Picasso Suite project from what was found: its name (or `fallback_name`), frame rate,
/// picture size, and every media file in the media pool, in the order the archive lists them.
pub fn to_project(drp: &DrpProject, fallback_name: &str) -> Project {
    let name = drp
        .name
        .clone()
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| fallback_name.to_string());
    let mut project = Project::new(name);
    if let Some(fps) = drp.fps {
        let rate = fps_of(fps);
        if rate.is_valid() {
            project.fps = rate;
        }
    }
    if let (Some(w), Some(h)) = (drp.width, drp.height) {
        project.width = w;
        project.height = h;
    }
    let default_duration = project.fps.nominal() * 5;
    for m in &drp.media {
        let source = match m.kind {
            MediaKind::Image => MediaSource::Image {
                path: m.path.clone(),
            },
            MediaKind::Video => MediaSource::Video {
                path: m.path.clone(),
            },
            MediaKind::Audio => MediaSource::Audio {
                path: m.path.clone(),
            },
        };
        let stem = m
            .name
            .rsplit_once('.')
            .map_or(m.name.as_str(), |(s, _)| s)
            .to_string();
        let _ = Command::AddMedia {
            name: stem,
            source,
            length: None,
            default_duration,
        }
        .apply(&mut project);
    }
    project
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// A small archive with the given (name, contents) entries.
    fn archive(entries: &[(&str, &str)]) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut out);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            for (name, text) in entries {
                zip.start_file(*name, options).unwrap();
                zip.write_all(text.as_bytes()).unwrap();
            }
            zip.finish().unwrap();
        }
        out.into_inner()
    }

    #[test]
    fn finds_the_name_rate_size_and_media() {
        let bytes = archive(&[
            (
                "project.xml",
                r#"<?xml version="1.0"?><Project><ProjectName>Trailer cut</ProjectName>
                   <Settings TimelineFrameRate="29.97" TimelineResolutionWidth="3840"
                             TimelineResolutionHeight="2160"/></Project>"#,
            ),
            (
                "MediaPool/Master/clip.xml",
                r#"<Clip><Name>A</Name><Path>D:\Footage\a.mov</Path>
                   <Sound path="/home/me/music/theme.wav"/>
                   <Url>file:///C:/Stills/logo%20big.png</Url>
                   <Other>D:\Footage\a.mov</Other><Note>not a path.mov</Note></Clip>"#,
            ),
            ("thumb.bin", "\u{0}\u{1}binary"),
        ]);
        let p = read(&bytes).unwrap();
        assert_eq!(p.name.as_deref(), Some("Trailer cut"));
        assert_eq!(p.fps, Some(29.97));
        assert_eq!((p.width, p.height), (Some(3840), Some(2160)));
        assert_eq!(p.documents, 2);
        let paths: Vec<_> = p.media.iter().map(|m| (m.path.as_str(), m.kind)).collect();
        assert_eq!(
            paths,
            vec![
                ("D:\\Footage\\a.mov", MediaKind::Video),
                ("/home/me/music/theme.wav", MediaKind::Audio),
                ("C:/Stills/logo big.png", MediaKind::Image),
            ]
        );
        assert_eq!(p.media[2].name, "logo big.png");

        let project = to_project(&p, "fallback");
        assert_eq!(project.name, "Trailer cut");
        assert_eq!(
            project.fps,
            Fps {
                num: 30_000,
                den: 1001
            }
        );
        assert_eq!((project.width, project.height), (3840, 2160));
        assert_eq!(project.media.len(), 3);
        assert_eq!(project.media[0].name, "a");
    }

    #[test]
    fn falls_back_to_the_file_name_and_default_settings() {
        let bytes = archive(&[("project.xml", "<Project><Other>x</Other></Project>")]);
        let project = to_project(&read(&bytes).unwrap(), "From disk");
        assert_eq!(project.name, "From disk");
        assert_eq!(project.fps, Fps::FPS_24);
        assert!(project.media.is_empty());
    }

    #[test]
    fn refuses_what_is_not_a_project_archive() {
        assert!(matches!(
            read(b"plain text"),
            Err(DrpError::NotAnArchive(_))
        ));
        assert!(matches!(read(&[]), Err(DrpError::NotAnArchive(_))));
        let only_binary = archive(&[("a.bin", "\u{0}\u{0}")]);
        assert!(matches!(read(&only_binary), Err(DrpError::Empty)));
        // Broken XML gives what came before the break and does not panic.
        let broken = archive(&[("project.xml", "<Project><Name>Half</Name><Path>/x/a.mp4")]);
        let p = read(&broken).unwrap();
        assert_eq!(p.name.as_deref(), Some("Half"));
    }

    #[test]
    fn recognises_paths_and_rates() {
        assert!(as_media_path("relative/a.mp4").is_none());
        assert!(as_media_path("/a/b.txt").is_none());
        assert!(as_media_path("\\\\server\\share\\a.MXF").is_some());
        assert_eq!(
            fps_of(23.976),
            Fps {
                num: 24_000,
                den: 1001
            }
        );
        assert_eq!(fps_of(25.0), Fps::FPS_25);
        assert_eq!(fps_of(1000.0).num, 240);
        assert_eq!(percent_decode("a%20b%zz"), "a b%zz");
    }

    #[test]
    fn reads_plain_xml_exports_too() {
        let xml = r#"<?xml version="1.0"?><xmeml><sequence><name>Cut 1</name>
            <rate><timebase>25</timebase></rate>
            <file><pathurl>file:///Volumes/Media/shot%201.mov</pathurl></file>
            <file><pathurl>file:///Volumes/Media/shot%201.mov</pathurl></file></sequence></xmeml>"#;
        let p = read(xml.as_bytes()).unwrap();
        assert_eq!(p.name.as_deref(), Some("Cut 1"));
        assert_eq!(p.media.len(), 1);
        assert_eq!(p.media[0].path, "/Volumes/Media/shot 1.mov");
        assert!(is_resolve_name("a.DRP") && is_resolve_name("b.drt") && !is_resolve_name("c.mp4"));
    }

    /// Hostile files: none may panic, hang or use unbounded memory.
    #[test]
    fn hostile_files_are_refused_or_read_quickly() {
        use std::time::Instant;
        let t = Instant::now();
        // Extremely deep nesting.
        let deep = format!(
            "<?xml version=\"1.0\"?>{}{}",
            "<a>".repeat(300_000),
            "</a>".repeat(300_000)
        );
        let _ = read(deep.as_bytes());
        assert!(
            t.elapsed().as_secs() < 10,
            "deep nesting took {:?}",
            t.elapsed()
        );

        // A zip bomb: eight 64 MB entries of one repeated tag, a few hundred KB compressed.
        let t = Instant::now();
        let filler = "<File path=\"/a\"/>".repeat(64 * 1024 * 1024 / 17);
        let entries: Vec<(String, &str)> = (0..8)
            .map(|i| (format!("d{i}.xml"), filler.as_str()))
            .collect();
        let refs: Vec<(&str, &str)> = entries.iter().map(|(n, t)| (n.as_str(), *t)).collect();
        let bomb = archive(&refs);
        assert!(bomb.len() < 4 * 1024 * 1024, "bomb is {} bytes", bomb.len());
        let _ = read(&bomb);
        assert!(t.elapsed().as_secs() < 60, "bomb took {:?}", t.elapsed());

        // Many entries, and junk that is neither zip nor XML.
        let many: Vec<(String, &str)> =
            (0..30_000).map(|i| (format!("e{i}.xml"), "<a/>")).collect();
        let refs: Vec<(&str, &str)> = many.iter().map(|(n, t)| (n.as_str(), *t)).collect();
        let _ = read(&archive(&refs));
        assert!(read(&[0u8; 1024]).is_err());
        assert!(read(b"PK truncated").is_err());
    }
}
