//! OpenTimelineIO export: the timeline as an `.otio` file (JSON), which DaVinci Resolve, Premiere
//! Pro, Avid, Nuke, Blender and others import. Written from the OTIO schema (Timeline.1, Stack.1,
//! Track.1, Clip.1 ...: the oldest versions, which every reader takes), with OTIO's own rules: the tracks of a stack go bottom to top, a gap is
//! an item of its own, and a clip's range is in the frames of its media.
//!
//! What OTIO has no field for (opacity, the transform, the volume) is kept under
//! `metadata.picasso`, which other programs leave alone.

use serde_json::{json, Value};

use crate::{Clip, MediaSource, Project, TrackKind, TransitionKind};

/// The timeline starts at this time of day (01:00:00:00), as on the timeline.
const START_SECONDS: u64 = 3600;

fn time(value: u64, rate: f64) -> Value {
    json!({ "OTIO_SCHEMA": "RationalTime.1", "rate": rate, "value": value as f64 })
}

fn range(start: u64, duration: u64, rate: f64) -> Value {
    json!({
        "OTIO_SCHEMA": "TimeRange.1",
        "start_time": time(start, rate),
        "duration": time(duration, rate),
    })
}

/// A `file://` address for a path on any system: `C:\a b\c.mov` is `file:///C:/a%20b/c.mov`.
pub fn file_url(path: &str) -> String {
    let path = path.replace('\\', "/");
    let mut out = String::from("file://");
    if !path.starts_with('/') {
        out.push('/');
    }
    for b in path.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' | b':' => {
                out.push(char::from(b));
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// OTIO's names for the eight marker colours (the last, fawn, is nearest to orange).
const MARKER_COLORS: [&str; 8] = [
    "BLUE", "CYAN", "GREEN", "YELLOW", "RED", "PINK", "PURPLE", "ORANGE",
];

/// What a clip shows: a file, a generated picture, or nothing OTIO can point to.
fn media_reference(project: &Project, clip: &Clip, rate: f64) -> Value {
    let Some(media) = project.media(clip.media) else {
        return json!({ "OTIO_SCHEMA": "MissingReference.1", "name": clip.name, "metadata": {} });
    };
    let available = media.length.map_or(Value::Null, |n| range(0, n, rate));
    let generator = |kind: &str, parameters: Value| {
        json!({
            "OTIO_SCHEMA": "GeneratorReference.1",
            "name": media.name,
            "generator_kind": kind,
            "parameters": parameters,
            "available_range": available,
            "metadata": {},
        })
    };
    match &media.source {
        MediaSource::Video { path } | MediaSource::Audio { path } | MediaSource::Image { path } => {
            json!({
                "OTIO_SCHEMA": "ExternalReference.1",
                "name": media.name,
                "target_url": file_url(path),
                "available_range": available,
                "metadata": {},
            })
        }
        MediaSource::Solid { rgb } => generator("SolidColor", json!({ "color": rgb })),
        MediaSource::Gradient { from, to } => {
            generator("Gradient", json!({ "from": from, "to": to }))
        }
        MediaSource::ColorBars => generator("SMPTEBars", json!({})),
        MediaSource::Generator { pattern, colors } => generator(
            "PicassoGenerator",
            json!({ "name": pattern.label(), "colors": colors }),
        ),
        MediaSource::Title(title) => generator("Text", json!({ "text": title.text })),
        MediaSource::Compound { timeline } => json!({
            "OTIO_SCHEMA": "MissingReference.1",
            "name": media.name,
            "metadata": { "picasso": { "compound_timeline": timeline } },
        }),
        MediaSource::EffectsClip => generator("EffectsClip", json!({})),
    }
}

fn clip_json(project: &Project, clip: &Clip, rate: f64) -> Value {
    let mut effects = Vec::new();
    if clip.freeze {
        effects.push(json!({
            "OTIO_SCHEMA": "FreezeFrame.1",
            "name": "Freeze Frame",
            "effect_name": "FreezeFrame",
            "time_scalar": 0.0,
            "metadata": {},
        }));
    } else if (clip.speed - 1.0).abs() > 1e-4 || clip.reverse {
        let scalar = f64::from(clip.speed) * if clip.reverse { -1.0 } else { 1.0 };
        effects.push(json!({
            "OTIO_SCHEMA": "LinearTimeWarp.1",
            "name": "Speed",
            "effect_name": "LinearTimeWarp",
            "time_scalar": scalar,
            "metadata": {},
        }));
    }
    let t = &clip.transform;
    json!({
        "OTIO_SCHEMA": "Clip.1",
        "name": clip.name,
        "source_range": range(clip.source_in, clip.duration, rate),
        "media_reference": media_reference(project, clip, rate),
        "effects": effects,
        "markers": [],
        "enabled": clip.enabled,
        "metadata": { "picasso": {
            "opacity": clip.opacity,
            "gain_db": clip.audio.gain_db,
            "transform": {
                "x": t.x, "y": t.y, "scale_x": t.scale_x, "scale_y": t.scale_y,
                "rotation": t.rotation,
            },
        }},
    })
}

/// The timeline of `project` as an OTIO file.
pub fn to_otio(project: &Project) -> String {
    let rate = project.fps.as_f64();
    let mut tracks = Vec::new();
    // Picture tracks bottom (V1) first, then the sound tracks, as OTIO stacks them.
    for kind in [TrackKind::Video, TrackKind::Audio] {
        for track in project.tracks.iter().filter(|t| t.kind == kind) {
            let mut clips: Vec<&Clip> = track.clips.iter().collect();
            clips.sort_by_key(|c| c.start);
            let mut children = Vec::new();
            let mut cursor = 0;
            for (i, clip) in clips.iter().enumerate() {
                // A transition into this clip sits between it and the one before, half over each.
                let left = i.checked_sub(1).and_then(|j| clips.get(j));
                let frames = left.and_then(|l| crate::project::transition_frames(l, clip));
                if clip.start > cursor {
                    children.push(json!({
                        "OTIO_SCHEMA": "Gap.1",
                        "name": "",
                        "source_range": range(0, clip.start - cursor, rate),
                        "effects": [], "markers": [], "enabled": true, "metadata": {},
                    }));
                }
                if let (Some(frames), Some(t)) = (frames, clip.transition) {
                    children.push(json!({
                        "OTIO_SCHEMA": "Transition.1",
                        "name": match t.kind {
                            TransitionKind::DipToBlack => "Dip to Black",
                            other => other.label(),
                        },
                        "transition_type": "SMPTE_Dissolve",
                        "in_offset": time(frames / 2, rate),
                        "out_offset": time(frames - frames / 2, rate),
                        "metadata": { "picasso": { "kind": format!("{:?}", t.kind) } },
                    }));
                }
                children.push(clip_json(project, clip, rate));
                cursor = cursor.max(clip.end());
            }
            tracks.push(json!({
                "OTIO_SCHEMA": "Track.1",
                "name": track.name,
                "kind": if kind == TrackKind::Video { "Video" } else { "Audio" },
                "children": children,
                "source_range": null,
                "effects": [],
                "markers": [],
                "enabled": !track.muted,
                "metadata": { "picasso": { "locked": track.locked, "gain_db": track.gain_db } },
            }));
        }
    }
    let markers: Vec<Value> = project
        .markers
        .iter()
        .map(|m| {
            json!({
                "OTIO_SCHEMA": "Marker.2",
                "name": m.name,
                "color": MARKER_COLORS[usize::from(m.color) % MARKER_COLORS.len()],
                "marked_range": range(m.frame, 0, rate),
                "comment": "",
                "metadata": {},
            })
        })
        .collect();
    let timeline = json!({
        "OTIO_SCHEMA": "Timeline.1",
        "name": project.timeline_name,
        "global_start_time": time(START_SECONDS * project.fps.nominal(), rate),
        "metadata": { "picasso": {
            "project": project.name,
            "width": project.width,
            "height": project.height,
            "fps": [project.fps.num, project.fps.den],
        }},
        "tracks": {
            "OTIO_SCHEMA": "Stack.1",
            "name": "tracks",
            "children": tracks,
            "source_range": null,
            "effects": [],
            "markers": markers,
            "enabled": true,
            "metadata": {},
        },
    });
    serde_json::to_string_pretty(&timeline).unwrap_or_default()
}

/// The most clips, gaps and transitions read from one file.
pub const MAX_ITEMS: usize = 100_000;

/// How deep a nested sequence (a compound clip, a nested composition) may nest when a file is
/// read. A file that nests deeper, or refers back to a sequence already being read, stops there
/// instead of overflowing the stack.
pub const MAX_NESTING_DEPTH: usize = 8;

/// What a clip of a file being read shows.
#[derive(Clone, Debug, PartialEq)]
pub enum OtioMedia {
    /// A file on this computer (or one that was on the computer that wrote the timeline).
    File { path: String },
    /// A picture made by the program: a solid, colour bars, a gradient or a title.
    Generated(MediaSource),
    /// Nothing that can be found: kept by name, so the clip can be matched to the media pool.
    Missing,
}

/// A clip of a timeline read from an OTIO file, in the frames of the project.
#[derive(Clone, Debug, PartialEq)]
pub struct OtioClip {
    pub name: String,
    /// The media's own name (the file name without its folder), for matching to the pool.
    pub media_name: String,
    pub media: OtioMedia,
    /// The length of the media, when the file gives it.
    pub available: Option<u64>,
    pub record: u64,
    pub source_in: u64,
    pub duration: u64,
    pub speed: f32,
    pub reverse: bool,
    pub freeze: bool,
    pub enabled: bool,
    /// A transition from the clip before: its length and kind.
    pub transition: Option<crate::Transition>,
    /// A nested timeline (compound clip): the tracks inside the stack, when the clip is a
    /// nested sequence rather than a file or generated clip.
    pub nested: Vec<OtioTrack>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OtioTrack {
    pub kind: TrackKind,
    pub enabled: bool,
    pub clips: Vec<OtioClip>,
}

/// A timeline read from an OTIO file.
#[derive(Clone, Debug, PartialEq)]
pub struct OtioTimeline {
    pub name: String,
    pub tracks: Vec<OtioTrack>,
    pub markers: Vec<crate::Marker>,
    /// Items that could not be read as clips (nested stacks, unknown schemas).
    pub skipped: usize,
}

/// The path of a `file://` address (`file:///C:/a%20b.mov` is `C:/a b.mov`), or of a plain path.
/// Other addresses (`http://`, `s3://`) have no path here.
pub fn path_of_url(url: &str) -> Option<String> {
    let url = url.trim();
    let rest = match url.split_once("://") {
        Some((scheme, rest)) if scheme.eq_ignore_ascii_case("file") => rest,
        Some(_) => return None,
        None => return (!url.is_empty()).then(|| url.to_string()),
    };
    // `file://localhost/x` and `file:///x` name the same file.
    let rest = match rest.strip_prefix("localhost") {
        Some(r) if r.is_empty() || r.starts_with('/') => r,
        _ => rest,
    };
    // `file://server/share/x` names another computer: it is the network path `//server/share/x`
    // (which is not opened without asking), never `server/share/x` beside the program. A drive
    // written without the slash (`file://C:/x`) is the drive.
    let host = !rest.is_empty() && !rest.starts_with('/') && rest.as_bytes().get(1) != Some(&b':');
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = (bytes[i] == b'%')
            .then(|| bytes.get(i + 1..i + 3))
            .flatten()
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match hex {
            Some(b) => {
                out.push(b);
                i += 3;
            }
            None => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    let path = String::from_utf8_lossy(&out).into_owned();
    if host {
        return Some(format!("//{path}"));
    }
    // `/C:/x` is a Windows path.
    Some(match path.strip_prefix('/') {
        Some(win) if win.as_bytes().get(1) == Some(&b':') => win.to_string(),
        _ => path,
    })
}

/// Frames of the project in an OTIO `RationalTime` (which has a rate of its own).
fn frames_of(time: &Value, fps: f64) -> Option<u64> {
    let value = time["value"].as_f64()?;
    let rate = time["rate"].as_f64().filter(|r| *r > 0.0)?;
    let frames = (value / rate * fps).round();
    (frames.is_finite() && frames >= 0.0 && frames <= crate::project::MAX_FRAMES as f64)
        .then_some(frames as u64)
}

/// The start and length of an OTIO `TimeRange`.
fn range_of(range: &Value, fps: f64) -> Option<(u64, u64)> {
    Some((
        frames_of(&range["start_time"], fps)?,
        frames_of(&range["duration"], fps)?,
    ))
}

/// Whether an object has `schema` (any version): `Clip.1` and `Clip.2` are both clips.
fn is(v: &Value, schema: &str) -> bool {
    v["OTIO_SCHEMA"]
        .as_str()
        .and_then(|s| s.split('.').next())
        .is_some_and(|s| s == schema)
}

/// An sRGB colour from OTIO generator parameters: `[r, g, b]` as bytes or as 0..1 numbers.
fn rgb_of(v: &Value) -> Option<[u8; 3]> {
    let parts: Vec<f64> = v.as_array()?.iter().filter_map(Value::as_f64).collect();
    let scale = if parts.iter().all(|c| *c <= 1.0) {
        255.0
    } else {
        1.0
    };
    match parts[..] {
        [r, g, b, ..] => Some([r, g, b].map(|c| (c * scale).round().clamp(0.0, 255.0) as u8)),
        _ => None,
    }
}

fn media_of(reference: &Value) -> OtioMedia {
    if is(reference, "ExternalReference") {
        if let Some(path) = reference["target_url"].as_str().and_then(path_of_url) {
            return OtioMedia::File { path };
        }
    }
    if is(reference, "ImageSequenceReference") {
        // A sequence of numbered images has no single file to import.
        return OtioMedia::Missing;
    }
    if is(reference, "GeneratorReference") {
        let p = &reference["parameters"];
        let kind = reference["generator_kind"].as_str().unwrap_or("");
        let source = match kind.to_ascii_lowercase().as_str() {
            "solidcolor" | "solid" | "color" => Some(MediaSource::Solid {
                rgb: rgb_of(&p["color"]).unwrap_or([0, 0, 0]),
            }),
            "smptebars" | "bars" | "colorbars" => Some(MediaSource::ColorBars),
            "gradient" => Some(MediaSource::Gradient {
                from: rgb_of(&p["from"]).unwrap_or([0, 0, 0]),
                to: rgb_of(&p["to"]).unwrap_or([255, 255, 255]),
            }),
            "text" => Some(MediaSource::Title(crate::TitleText {
                text: p["text"]
                    .as_str()
                    .unwrap_or("Title")
                    .chars()
                    .take(crate::MAX_TITLE_CHARS)
                    .collect(),
                ..crate::TitleText::default()
            })),
            "effectsclip" => Some(MediaSource::EffectsClip),
            "picassogenerator" => p["name"]
                .as_str()
                .and_then(crate::GeneratorKind::from_name)
                .map(|pattern| {
                    let mut colors = pattern.default_colors();
                    if let Some(list) = p["colors"].as_array() {
                        for (slot, value) in colors.iter_mut().zip(list) {
                            let parts: Vec<u8> = value
                                .as_array()
                                .map(|a| {
                                    a.iter()
                                        .filter_map(Value::as_u64)
                                        .map(|c| c.min(255) as u8)
                                        .collect()
                                })
                                .unwrap_or_default();
                            if let [r, g, b] = parts[..] {
                                *slot = [r, g, b];
                            }
                        }
                    }
                    MediaSource::Generator { pattern, colors }
                }),
            _ => None,
        };
        if let Some(source) = source {
            return OtioMedia::Generated(source);
        }
    }
    OtioMedia::Missing
}

/// The speed, direction and freeze of a clip's time effects.
fn time_effects(effects: &Value) -> (f32, bool, bool) {
    let mut speed = 1.0_f64;
    let (mut reverse, mut freeze) = (false, false);
    for e in effects.as_array().into_iter().flatten() {
        if is(e, "FreezeFrame") {
            freeze = true;
        } else if is(e, "LinearTimeWarp") {
            let s = e["time_scalar"].as_f64().unwrap_or(1.0);
            if s == 0.0 {
                freeze = true;
            } else if s.is_finite() {
                speed *= s.abs();
                reverse ^= s < 0.0;
            }
        }
    }
    (speed as f32, reverse, freeze)
}

/// The colour number of an OTIO marker colour.
fn marker_color(name: &str) -> u8 {
    MARKER_COLORS
        .iter()
        .position(|c| c.eq_ignore_ascii_case(name))
        .and_then(|i| u8::try_from(i).ok())
        .unwrap_or(0)
}

fn read_track(
    track: &Value,
    fps: f64,
    items: &mut usize,
    skipped: &mut usize,
    depth: usize,
) -> OtioTrack {
    let kind = if track["kind"].as_str() == Some("Audio") {
        TrackKind::Audio
    } else {
        TrackKind::Video
    };
    let mut clips: Vec<OtioClip> = Vec::new();
    let mut cursor = 0_u64;
    // A transition read and waiting for the clip after it.
    let mut pending: Option<crate::Transition> = None;
    for item in track["children"].as_array().into_iter().flatten() {
        *items += 1;
        if *items > MAX_ITEMS {
            *skipped += 1;
            continue;
        }
        if is(item, "Transition") {
            let half = |key: &str| frames_of(&item[key], fps).unwrap_or(0);
            let kind = item["metadata"]["picasso"]["kind"]
                .as_str()
                .and_then(TransitionKind::from_name)
                .unwrap_or(TransitionKind::CrossDissolve);
            pending = Some(crate::Transition {
                kind,
                frames: half("in_offset") + half("out_offset"),
            });
            continue;
        }
        let reference = &item["media_reference"];
        let range = range_of(&item["source_range"], fps)
            .or_else(|| range_of(&reference["available_range"], fps));
        let Some((source_in, duration)) = range else {
            *skipped += 1;
            continue;
        };
        if is(item, "Clip") && duration > 0 {
            let media = media_of(reference);
            let media_name = match &media {
                OtioMedia::File { path } => {
                    path.rsplit(['/', '\\']).next().unwrap_or(path).to_string()
                }
                _ => reference["name"].as_str().unwrap_or("").to_string(),
            };
            let (speed, reverse, freeze) = time_effects(&item["effects"]);
            // A media reference's range starts at its own first frame (a timecode, often).
            let media_start = range_of(&reference["available_range"], fps).map_or(0, |r| r.0);
            clips.push(OtioClip {
                name: item["name"].as_str().unwrap_or("").to_string(),
                media_name,
                media,
                available: range_of(&reference["available_range"], fps).map(|r| r.1),
                record: cursor,
                source_in: source_in.saturating_sub(media_start),
                duration,
                speed,
                reverse,
                freeze,
                enabled: item["enabled"].as_bool().unwrap_or(true),
                transition: pending.take(),
                nested: vec![],
            });
        } else if is(item, "Stack") {
            // A nested stack (compound clip): read its tracks as nested timeline data. A stack
            // with nothing to read, or one nested deeper than MAX_NESTING_DEPTH, is left out.
            let nested = if depth < MAX_NESTING_DEPTH {
                stack_tracks(item, fps, items, skipped, depth + 1)
            } else {
                Vec::new()
            };
            if nested.is_empty() {
                *skipped += 1;
            } else {
                clips.push(OtioClip {
                    name: item["name"].as_str().unwrap_or("").to_string(),
                    media_name: item["name"].as_str().unwrap_or("Nested").to_string(),
                    media: OtioMedia::Missing,
                    available: None,
                    record: cursor,
                    source_in: 0,
                    duration,
                    speed: 1.0,
                    reverse: false,
                    freeze: false,
                    enabled: item["enabled"].as_bool().unwrap_or(true),
                    transition: pending.take(),
                    nested,
                });
            }
        } else if !is(item, "Gap") {
            // Something unknown: its time is kept.
            *skipped += 1;
        }
        pending = None;
        cursor = cursor.saturating_add(duration);
    }
    OtioTrack {
        kind,
        enabled: track["enabled"].as_bool().unwrap_or(true),
        clips,
    }
}

/// Reads a nested Stack.1 item's tracks into `Vec<OtioTrack>`, recursing into further nesting.
fn stack_tracks(
    stack: &Value,
    fps: f64,
    items: &mut usize,
    skipped: &mut usize,
    depth: usize,
) -> Vec<OtioTrack> {
    let mut out = Vec::new();
    let children = stack["children"].as_array().into_iter().flatten();
    for child in children {
        *items += 1;
        if *items > MAX_ITEMS {
            *skipped += 1;
            continue;
        }
        if is(child, "Track") {
            out.push(read_track(child, fps, items, skipped, depth));
        }
        // Unknown child schemas inside a stack are left out.
    }
    out
}

/// Reads the first timeline of an OTIO file (a timeline, or a collection holding timelines),
/// with every time in frames of `fps`. Errors when the text is not an OTIO timeline.
pub fn read_otio(text: &str, fps: crate::Fps) -> Result<OtioTimeline, String> {
    let root: Value =
        serde_json::from_str(text).map_err(|e| format!("this is not an OTIO file ({e})"))?;
    let timeline = if is(&root, "Timeline") {
        &root
    } else if is(&root, "SerializableCollection") {
        root["children"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|c| is(c, "Timeline"))
            .ok_or("the collection holds no timeline")?
    } else {
        return Err("this is not an OTIO timeline".into());
    };
    let rate = fps.as_f64();
    let stack = &timeline["tracks"];
    let (mut items, mut skipped) = (0, 0);
    let tracks = stack["children"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|t| is(t, "Track"))
        .map(|t| read_track(t, rate, &mut items, &mut skipped, 0))
        .collect();
    let markers = stack["markers"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| {
            Some(crate::Marker {
                frame: frames_of(&m["marked_range"]["start_time"], rate)?,
                name: m["name"].as_str().unwrap_or("").to_string(),
                color: marker_color(m["color"].as_str().unwrap_or("")),
            })
        })
        .take(crate::project::MAX_MARKERS)
        .collect();
    Ok(OtioTimeline {
        name: timeline["name"].as_str().unwrap_or("").trim().to_string(),
        tracks,
        markers,
        skipped,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Command, Outcome, Transition};

    fn project() -> Project {
        let mut p = Project::new("otio test");
        let v1 = p.tracks[0].id;
        let add = |p: &mut Project, name: &str, path: &str, start: u64| {
            let Ok(Outcome::Media(m)) = (Command::AddMedia {
                name: name.into(),
                source: MediaSource::Video { path: path.into() },
                length: Some(240),
                default_duration: 48,
            })
            .apply(p) else {
                panic!("media");
            };
            Command::AddClip {
                track: v1,
                media: m,
                start,
                duration: Some(48),
                with_sound: false,
            }
            .apply(p)
            .unwrap_or_else(|e| panic!("{e}"));
        };
        add(&mut p, "a.mov", r"C:\Media\a b.mov", 0);
        add(&mut p, "b.mov", "/home/me/b.mov", 72);
        p
    }

    #[test]
    fn the_timeline_is_tracks_of_clips_and_gaps() {
        let mut p = project();
        p.markers.push(crate::Marker {
            frame: 10,
            name: "Here".into(),
            color: 4,
        });
        let text = to_otio(&p);
        let v: Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(v["OTIO_SCHEMA"], "Timeline.1");
        let stack = &v["tracks"];
        assert_eq!(stack["markers"][0]["color"], "RED");
        let v1 = &stack["children"][0];
        assert_eq!(v1["kind"], "Video");
        let kinds: Vec<&str> = v1["children"]
            .as_array()
            .map(|a| a.iter().filter_map(|c| c["OTIO_SCHEMA"].as_str()).collect())
            .unwrap_or_default();
        assert_eq!(kinds, ["Clip.1", "Gap.1", "Clip.1"]);
        let gap = &v1["children"][1]["source_range"]["duration"]["value"];
        assert_eq!(gap.as_f64(), Some(24.0), "48 to 72");
        let clip = &v1["children"][0];
        assert_eq!(
            clip["media_reference"]["target_url"],
            "file:///C:/Media/a%20b.mov"
        );
        assert_eq!(
            clip["media_reference"]["available_range"]["duration"]["value"].as_f64(),
            Some(240.0)
        );
        // The timeline starts at 01:00:00:00, as Resolve's do.
        assert_eq!(
            v["global_start_time"]["value"].as_f64(),
            Some(3600.0 * p.fps.as_f64().round())
        );
    }

    #[test]
    fn transitions_and_speed_changes_are_written() {
        let mut p = project();
        let track = &mut p.tracks[0];
        let second = &mut track.clips[1];
        second.start = 48;
        second.transition = Some(Transition {
            kind: TransitionKind::CrossDissolve,
            frames: 12,
        });
        second.speed = 2.0;
        let v: Value = serde_json::from_str(&to_otio(&p)).unwrap_or_else(|e| panic!("{e}"));
        let children = &v["tracks"]["children"][0]["children"];
        assert_eq!(children[1]["OTIO_SCHEMA"], "Transition.1");
        assert_eq!(children[1]["in_offset"]["value"].as_f64(), Some(6.0));
        assert_eq!(children[2]["effects"][0]["time_scalar"].as_f64(), Some(2.0));
    }

    #[test]
    fn an_exported_timeline_reads_back() {
        let mut p = project();
        let second = &mut p.tracks[0].clips[1];
        second.start = 48;
        second.transition = Some(Transition {
            kind: TransitionKind::DipToBlack,
            frames: 12,
        });
        second.speed = 2.0;
        second.duration = 24;
        p.tracks[0].clips[0].enabled = false;
        p.markers.push(crate::Marker {
            frame: 30,
            name: "Here".into(),
            color: 4,
        });
        let read = read_otio(&to_otio(&p), p.fps).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(read.name, p.timeline_name);
        assert_eq!(read.skipped, 0);
        assert_eq!(read.markers, p.markers);
        let v1 = &read.tracks[0];
        assert_eq!(v1.kind, TrackKind::Video);
        let got: Vec<(u64, u64, u64, bool)> = v1
            .clips
            .iter()
            .map(|c| (c.record, c.source_in, c.duration, c.enabled))
            .collect();
        assert_eq!(got, [(0, 0, 48, false), (48, 0, 24, true)]);
        let b = &v1.clips[1];
        assert_eq!(b.speed, 2.0);
        assert_eq!(
            b.transition,
            Some(Transition {
                kind: TransitionKind::DipToBlack,
                frames: 12
            })
        );
        assert_eq!(
            b.media,
            OtioMedia::File {
                path: "/home/me/b.mov".into()
            }
        );
        assert_eq!(b.media_name, "b.mov");
        assert_eq!(b.available, Some(240));
        assert_eq!(
            v1.clips[0].media,
            OtioMedia::File {
                path: "C:/Media/a b.mov".into()
            }
        );
    }

    #[test]
    fn an_effects_clip_reads_back_as_one() {
        let mut p = project();
        let Outcome::Clip(_) = (Command::CreateEffectsClip {
            start: 0,
            name: "Effects Clip 1".into(),
        })
        .apply(&mut p)
        .unwrap_or_else(|e| panic!("{e}"))
        else {
            panic!("expected a clip")
        };
        let read = read_otio(&to_otio(&p), p.fps).unwrap_or_else(|e| panic!("{e}"));
        assert!(
            read.tracks
                .iter()
                .flat_map(|t| &t.clips)
                .any(|c| c.media == OtioMedia::Generated(MediaSource::EffectsClip)),
            "an effects clip is written and read as one, not as missing media"
        );
    }

    #[test]
    fn files_from_other_programs_are_read() {
        // As other programs write it: a collection, a 25 fps media range that starts at a
        // timecode, a reversed half-speed clip, a nested stack and a generator.
        let text = r#"{
          "OTIO_SCHEMA": "SerializableCollection.1", "name": "bin",
          "children": [{
            "OTIO_SCHEMA": "Timeline.1", "name": " Cut 3 ",
            "tracks": { "OTIO_SCHEMA": "Stack.1", "markers": [
                { "OTIO_SCHEMA": "Marker.2", "name": "m", "color": "green",
                  "marked_range": { "start_time": { "value": 50, "rate": 25 },
                                    "duration": { "value": 0, "rate": 25 } } } ],
              "children": [
              { "OTIO_SCHEMA": "Track.1", "kind": "Video", "children": [
                { "OTIO_SCHEMA": "Gap.1", "source_range": {
                    "start_time": { "value": 0, "rate": 25 }, "duration": { "value": 25, "rate": 25 } } },
                { "OTIO_SCHEMA": "Clip.2", "name": "shot",
                  "source_range": { "start_time": { "value": 90050, "rate": 25 },
                                    "duration": { "value": 50, "rate": 25 } },
                  "effects": [ { "OTIO_SCHEMA": "LinearTimeWarp.1", "time_scalar": -0.5 } ],
                  "media_references": {},
                  "media_reference": { "OTIO_SCHEMA": "ExternalReference.1",
                    "target_url": "file://localhost/D:/Footage/shot%201.mxf",
                    "available_range": { "start_time": { "value": 90000, "rate": 25 },
                                         "duration": { "value": 500, "rate": 25 } } } },
                { "OTIO_SCHEMA": "Stack.1", "name": "nested", "source_range": {
                    "start_time": { "value": 0, "rate": 25 }, "duration": { "value": 25, "rate": 25 } },
                  "children": [
                    { "OTIO_SCHEMA": "Track.1", "kind": "Video", "children": [
                      { "OTIO_SCHEMA": "Clip.1", "name": "inner",
                        "source_range": { "start_time": { "value": 0, "rate": 25 },
                                          "duration": { "value": 25, "rate": 25 } },
                        "media_reference": { "OTIO_SCHEMA": "ExternalReference.1",
                          "target_url": "file://localhost/D:/Footage/inner.mxf" } }
                    ] }
                  ] },
                { "OTIO_SCHEMA": "Clip.1", "name": "black",
                  "source_range": { "start_time": { "value": 0, "rate": 25 },
                                    "duration": { "value": 25, "rate": 25 } },
                  "media_reference": { "OTIO_SCHEMA": "GeneratorReference.1",
                    "generator_kind": "SolidColor", "parameters": { "color": [1.0, 0.5, 0] } } },
                { "OTIO_SCHEMA": "Clip.1", "name": "web",
                  "source_range": { "start_time": { "value": 0, "rate": 25 },
                                    "duration": { "value": 25, "rate": 25 } },
                  "media_reference": { "OTIO_SCHEMA": "ExternalReference.1",
                    "name": "web.mov", "target_url": "https://example.com/web.mov" } }
              ] }
            ] }
          }]
        }"#;
        let fps = crate::Fps::FPS_24;
        let read = read_otio(text, fps).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(read.name, "Cut 3");
        // The nested stack is now read as a clip with nested tracks, not skipped.
        assert_eq!(read.skipped, 0);
        assert_eq!(read.markers[0].frame, 48);
        assert_eq!(read.markers[0].color, 2);
        let clips = &read.tracks[0].clips;
        assert_eq!(clips.len(), 4);
        let shot = &clips[0];
        // 25 fps times in a 24 fps project: one second of gap is 24 frames.
        assert_eq!((shot.record, shot.source_in, shot.duration), (24, 48, 48));
        assert_eq!((shot.speed, shot.reverse, shot.freeze), (0.5, true, false));
        assert_eq!(
            shot.media,
            OtioMedia::File {
                path: "D:/Footage/shot 1.mxf".into()
            }
        );
        assert_eq!(shot.available, Some(480));
        // The nested stack follows the shot: 24 + 48 = 72, and keeps its own track.
        assert_eq!(clips[1].record, 72, "after the shot");
        assert_eq!(clips[1].name, "nested");
        assert_eq!(clips[1].nested.len(), 1, "the nested stack's track");
        let inner = &clips[1].nested[0].clips[0];
        assert_eq!((inner.record, inner.duration), (0, 24));
        assert_eq!(
            inner.media,
            OtioMedia::File {
                path: "D:/Footage/inner.mxf".into()
            }
        );
        assert_eq!(
            clips[2].media,
            OtioMedia::Generated(MediaSource::Solid { rgb: [255, 128, 0] })
        );
        assert_eq!(clips[3].media, OtioMedia::Missing);
        assert_eq!(clips[3].media_name, "web.mov");
        assert!(read_otio("{}", fps).is_err());
        assert!(read_otio("not json", fps).is_err());
    }

    #[test]
    fn a_stack_that_nests_too_deep_stops() {
        let fps = crate::Fps::FPS_24;
        let range = serde_json::json!({
            "start_time": { "value": 0, "rate": 24 }, "duration": { "value": 24, "rate": 24 }
        });
        let mut item = serde_json::json!({
            "OTIO_SCHEMA": "Clip.1", "name": "bottom", "source_range": range,
            "media_reference": { "OTIO_SCHEMA": "ExternalReference.1",
                                 "target_url": "file://localhost/bottom.mov" }
        });
        // Each level is a stack inside a track, deeper than MAX_NESTING_DEPTH.
        for _ in 0..=MAX_NESTING_DEPTH {
            item = serde_json::json!({
                "OTIO_SCHEMA": "Stack.1", "name": "level", "source_range": range,
                "children": [{
                    "OTIO_SCHEMA": "Track.1", "kind": "Video", "children": [item]
                }]
            });
        }
        let text = serde_json::json!({
            "OTIO_SCHEMA": "Timeline.1", "name": "deep",
            "tracks": { "OTIO_SCHEMA": "Stack.1", "children": [{
                "OTIO_SCHEMA": "Track.1", "kind": "Video", "children": [item]
            }] }
        })
        .to_string();
        let read = read_otio(&text, fps).unwrap_or_else(|e| panic!("{e}"));
        // The clips that were read stop at the cap rather than recursing for ever.
        let mut tracks = &read.tracks;
        let mut count = 0;
        while let Some(clip) = tracks.iter().flat_map(|t| &t.clips).next() {
            count += 1;
            assert!(count <= MAX_NESTING_DEPTH, "nesting stops at the cap");
            tracks = &clip.nested;
        }
        assert!(
            read.skipped > 0,
            "the nesting past the cap is left out ({} skipped, {count} read)",
            read.skipped
        );
        assert!(count > 1, "the nesting was read to the cap");
    }

    #[test]
    fn importing_places_what_fits_on_a_new_timeline() {
        let mut p = project();
        let media = p.tracks[0].clips[0].media;
        let timelines = p.timelines.len();
        let clip = |start: u64, source_in: u64, duration: u64| crate::ImportedClip {
            media: Some(media),
            name: "c".into(),
            start,
            source_in,
            duration,
            speed: 1.0,
            reverse: false,
            freeze: false,
            enabled: true,
            transition: None,
            nested: Vec::new(),
        };
        let tracks = vec![
            crate::ImportedTrack {
                kind: TrackKind::Video,
                muted: false,
                clips: vec![
                    clip(0, 0, 48),
                    clip(24, 0, 48),   // overlaps the first: left out
                    clip(48, 200, 48), // runs past the end of the media: left out
                    crate::ImportedClip {
                        speed: 4.0,
                        ..clip(96, 0, 10)
                    },
                ],
            },
            crate::ImportedTrack {
                kind: TrackKind::Video,
                muted: false,
                clips: vec![],
            },
            crate::ImportedTrack {
                kind: TrackKind::Video,
                muted: false,
                clips: vec![clip(0, 0, 5)],
            },
            crate::ImportedTrack {
                kind: TrackKind::Audio,
                muted: true,
                clips: vec![clip(0, 0, 5)],
            },
        ];
        let markers = vec![
            crate::Marker {
                frame: 9,
                name: "b".into(),
                color: 12,
            },
            crate::Marker {
                frame: 3,
                name: "a".into(),
                color: 1,
            },
        ];
        Command::ImportTimeline {
            name: "Imported".into(),
            tracks,
            markers,
        }
        .apply(&mut p)
        .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(p.timelines.len(), timelines + 1, "the old timeline is kept");
        assert_eq!(p.timeline_name, "Imported");
        let video: Vec<_> = p
            .tracks
            .iter()
            .filter(|t| t.kind == TrackKind::Video)
            .collect();
        assert_eq!(video.len(), 3, "a third video track was added");
        let v1: Vec<(u64, u64, f32)> = video[0]
            .clips
            .iter()
            .map(|c| (c.start, c.duration, c.speed))
            .collect();
        assert_eq!(v1, [(0, 48, 1.0), (96, 10, 4.0)]);
        assert_eq!(video[2].clips.len(), 1);
        let a1 = p
            .tracks
            .iter()
            .find(|t| t.kind == TrackKind::Audio)
            .unwrap();
        assert!(a1.muted);
        assert_eq!(a1.clips.len(), 1, "a video's sound fits an audio track");
        // The picture on V3 and the sound on A1 are the same file at the same place: they are
        // linked, so the picture's own sound is not heard as well.
        let (picture, sound) = (&video[2].clips[0], &a1.clips[0]);
        assert!(picture.link.is_some());
        assert_eq!(picture.link, sound.link);
        assert!(video[0].clips.iter().all(|c| c.link.is_none()), "nothing else is");
        let frames: Vec<u64> = p.markers.iter().map(|m| m.frame).collect();
        assert_eq!(frames, [3, 9]);
        assert!(p.markers.iter().all(|m| m.color < crate::MARKER_COLORS));
        let ids: std::collections::HashSet<_> = p
            .tracks
            .iter()
            .flat_map(|t| t.clips.iter().map(|c| c.id))
            .collect();
        assert_eq!(ids.len(), 4, "every clip has an id of its own");
    }

    #[test]
    fn file_addresses_read_back_as_paths() {
        assert_eq!(
            path_of_url("file:///C:/a%20b.mov").as_deref(),
            Some("C:/a b.mov")
        );
        assert_eq!(
            path_of_url("file://localhost/x/y.mov").as_deref(),
            Some("/x/y.mov")
        );
        assert_eq!(
            path_of_url("file:///D:/x/%C3%A9.mp4").as_deref(),
            Some("D:/x/é.mp4")
        );
        assert_eq!(
            path_of_url("/plain/path.mov").as_deref(),
            Some("/plain/path.mov")
        );
        assert_eq!(path_of_url("https://a/b.mov"), None);
        assert_eq!(
            path_of_url("file:///a/100%.mov").as_deref(),
            Some("/a/100%.mov")
        );
        // A host is another computer: a network path, which the decoder refuses to open
        // unasked, never a path relative to wherever the program runs.
        assert_eq!(
            path_of_url("file://server/share/x.mov").as_deref(),
            Some("//server/share/x.mov")
        );
        assert_eq!(
            path_of_url("file://localhostile/x.mov").as_deref(),
            Some("//localhostile/x.mov")
        );
        assert_eq!(path_of_url("file://C:/x.mov").as_deref(), Some("C:/x.mov"));
    }

    #[test]
    fn file_addresses_escape_what_a_url_cannot_hold() {
        assert_eq!(file_url("/a/b c#1.mov"), "file:///a/b%20c%231.mov");
        assert_eq!(file_url(r"D:\x\é.mp4"), "file:///D:/x/%C3%A9.mp4");
    }
}
