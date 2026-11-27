//! Final Cut Pro 7 XML (`xmeml`, version 5): the timeline as a sequence that Final Cut Pro 7,
//! Premiere Pro, DaVinci Resolve, Avid Media Composer (through its importers) and others read.
//! Premiere's "Final Cut Pro XML" import and export use the same format, so one writer serves
//! both presets. Written from Apple's published description of the interchange format (the
//! `xmeml` document type: `sequence`, `media`, `track`, `clipitem`, `file`, `transitionitem`).
//!
//! The sequence keeps the timeline's tracks, clips (by the files they come from), gaps, speed
//! changes, reverse, freeze frames, dissolves and markers. A clip of generated media (a title,
//! colour bars, a solid or a gradient) or a compound clip has no file to point to, so it is left
//! out; [`Written::skipped`] counts them. Grades, effects and keyframes stay in Picasso Suite.
//!
//! [`read_xmeml`] reads such a sequence back, from this program or from Final Cut Pro 7,
//! Premiere or Resolve, into the timeline an OpenTimelineIO file is read into, so both are
//! imported the same way. The document is untrusted: its size and depth are capped and nothing
//! here panics.

use std::collections::{HashMap, HashSet};

use crate::otio::{OtioClip, OtioMedia, OtioTimeline, OtioTrack, MAX_NESTING_DEPTH};
use crate::{Clip, MediaSource, Project, TrackKind, TransitionKind};

/// The timeline starts at this time of day (01:00:00:00), as on the timeline.
const START_SECONDS: u64 = 3600;

/// The document, and how many clips it could not carry.
pub struct Written {
    pub text: String,
    pub skipped: usize,
}

/// Escapes the five characters XML gives a meaning to.
pub(crate) fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // Control characters other than tab and new line are not allowed in XML 1.0.
            c if (c as u32) < 0x20 && c != '\t' && c != '\n' => {}
            c => out.push(c),
        }
    }
    out
}

/// The `<rate>` of a frame rate: the whole frames of a second, and whether it is an NTSC rate
/// (29.97 is 30 with `ntsc` TRUE).
fn rate(project: &Project) -> String {
    let ntsc = project.fps.den == 1001;
    format!(
        "<rate><timebase>{}</timebase><ntsc>{}</ntsc></rate>",
        project.fps.nominal(),
        if ntsc { "TRUE" } else { "FALSE" }
    )
}

/// The `file://localhost/` address Final Cut Pro and Premiere both read.
fn path_url(path: &str) -> String {
    let url = crate::otio::file_url(path);
    match url.strip_prefix("file://") {
        Some(rest) => format!("file://localhost{rest}"),
        None => url,
    }
}

/// The path of a clip's media, when it has a file.
fn file_path<'a>(project: &'a Project, clip: &Clip) -> Option<&'a str> {
    match &project.media(clip.media)?.source {
        MediaSource::Video { path } | MediaSource::Audio { path } | MediaSource::Image { path } => {
            Some(path)
        }
        _ => None,
    }
}

/// Writes the files a sequence uses: the first clip of a file describes it in full, later ones
/// only name it, as the format asks.
struct Files {
    seen: Vec<u64>,
}

impl Files {
    fn element(&mut self, project: &Project, clip: &Clip, path: &str) -> String {
        let id = clip.media.0;
        if self.seen.contains(&id) {
            return format!("<file id=\"file-{id}\"/>");
        }
        self.seen.push(id);
        let media = project.media(clip.media);
        let name = media.map_or(clip.name.as_str(), |m| m.name.as_str());
        let file_name = path.rsplit(['/', '\\']).next().unwrap_or(name);
        let length = media
            .and_then(|m| m.length)
            .unwrap_or(clip.source_in + clip.source_span());
        let (video, audio) = match media.map(|m| &m.source) {
            Some(MediaSource::Audio { .. }) => (false, true),
            Some(MediaSource::Image { .. }) => (true, false),
            _ => (true, true),
        };
        let mut kinds = String::new();
        if video {
            kinds.push_str(&format!(
                "<video><samplecharacteristics>{}<width>{}</width><height>{}</height></samplecharacteristics></video>",
                rate(project),
                project.width,
                project.height
            ));
        }
        if audio {
            kinds.push_str(
                "<audio><samplecharacteristics><depth>16</depth><samplerate>48000</samplerate></samplecharacteristics><channelcount>2</channelcount></audio>",
            );
        }
        format!(
            "<file id=\"file-{id}\"><name>{}</name><pathurl>{}</pathurl>{}<duration>{length}</duration><timecode>{}<string>00:00:00:00</string><frame>0</frame><displayformat>NDF</displayformat></timecode><media>{kinds}</media></file>",
            escape(file_name),
            escape(&path_url(path)),
            rate(project),
            rate(project),
        )
    }
}

/// The Time Remap filter that carries a clip's speed, direction or freeze.
fn speed_filter(clip: &Clip) -> Option<String> {
    let speed = if clip.freeze {
        0.0
    } else {
        f64::from(clip.speed) * 100.0
    };
    if !clip.freeze && !clip.reverse && (speed - 100.0).abs() < 0.01 {
        return None;
    }
    Some(format!(
        "<filter><effect><name>Time Remap</name><effectid>timeremap</effectid><effectcategory>motion</effectcategory><effecttype>motion</effecttype><mediatype>video</mediatype>\
<parameter><parameterid>variablespeed</parameterid><name>variablespeed</name><value>0</value></parameter>\
<parameter><parameterid>speed</parameterid><name>speed</name><value>{speed:.4}</value></parameter>\
<parameter><parameterid>reverse</parameterid><name>reverse</name><value>{}</value></parameter>\
<parameter><parameterid>frameblending</parameterid><name>frameblending</name><value>FALSE</value></parameter>\
</effect></filter>",
        if clip.reverse { "TRUE" } else { "FALSE" }
    ))
}

/// A dissolve centred on the cut at `cut`, `frames` long.
fn transition_item(
    project: &Project,
    kind: TransitionKind,
    cut: u64,
    frames: u64,
    video: bool,
) -> String {
    let start = cut - frames / 2;
    let end = cut + (frames - frames / 2);
    let (name, id, category, media) = match (video, kind) {
        (false, _) => ("Cross Fade (+3dB)", "KGAudioTransCrossFade3dB", "", "audio"),
        (true, TransitionKind::DipToBlack) => (
            "Dip to Color Dissolve",
            "Dip to Color Dissolve",
            "Dissolve",
            "video",
        ),
        // Other editors do not know the more transitions: they get a dissolve of the same length.
        (true, _) => ("Cross Dissolve", "Cross Dissolve", "Dissolve", "video"),
    };
    format!(
        "<transitionitem>{}<start>{start}</start><end>{end}</end><alignment>center</alignment><effect><name>{name}</name><effectid>{id}</effectid><effectcategory>{category}</effectcategory><effecttype>transition</effecttype><mediatype>{media}</mediatype></effect></transitionitem>",
        rate(project)
    )
}

/// One track of the sequence.
fn track(project: &Project, t: &crate::Track, files: &mut Files, skipped: &mut usize) -> String {
    let video = t.kind == TrackKind::Video;
    let mut clips: Vec<&Clip> = t.clips.iter().collect();
    clips.sort_by_key(|c| c.start);
    let mut out = String::from("<track>");
    for (i, clip) in clips.iter().enumerate() {
        let Some(path) = file_path(project, clip) else {
            *skipped += 1;
            continue;
        };
        let left = i.checked_sub(1).and_then(|j| clips.get(j));
        let frames = left.and_then(|l| crate::project::transition_frames(l, clip));
        if let (Some(frames), Some(t)) = (frames, clip.transition) {
            out.push_str(&transition_item(project, t.kind, clip.start, frames, video));
        }
        let length = project
            .media(clip.media)
            .and_then(|m| m.length)
            .unwrap_or(clip.source_in + clip.source_span());
        let source_out = clip.source_in + clip.source_span();
        out.push_str(&format!(
            "<clipitem id=\"clipitem-{}\"><name>{}</name><enabled>{}</enabled><duration>{length}</duration>{}<start>{}</start><end>{}</end><in>{}</in><out>{source_out}</out>",
            clip.id.0,
            escape(&clip.name),
            if clip.enabled { "TRUE" } else { "FALSE" },
            rate(project),
            clip.start,
            clip.end(),
            clip.source_in,
        ));
        out.push_str(&files.element(project, clip, path));
        if !video {
            out.push_str(
                "<sourcetrack><mediatype>audio</mediatype><trackindex>1</trackindex></sourcetrack>",
            );
        }
        if video {
            if let Some(filter) = speed_filter(clip) {
                out.push_str(&filter);
            }
        }
        out.push_str("</clipitem>");
    }
    out.push_str(&format!(
        "<enabled>{}</enabled><locked>{}</locked></track>",
        if t.muted || (video && !t.picture) {
            "FALSE"
        } else {
            "TRUE"
        },
        if t.locked { "TRUE" } else { "FALSE" }
    ));
    out
}

/// The timeline of `project` as a Final Cut Pro 7 / Premiere XML document.
pub fn to_xmeml(project: &Project) -> Written {
    let mut files = Files { seen: Vec::new() };
    let mut skipped = 0;
    let start = START_SECONDS * project.fps.nominal();
    let mut video = String::new();
    let mut audio = String::new();
    for t in &project.tracks {
        match t.kind {
            TrackKind::Video => video.push_str(&track(project, t, &mut files, &mut skipped)),
            TrackKind::Audio => audio.push_str(&track(project, t, &mut files, &mut skipped)),
        }
    }
    let markers: String = project
        .markers
        .iter()
        .map(|m| {
            format!(
                "<marker><name>{}</name><comment></comment><in>{}</in><out>-1</out></marker>",
                escape(&m.name),
                m.frame
            )
        })
        .collect();
    let text = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE xmeml>\n<xmeml version=\"5\"><sequence id=\"sequence-1\"><name>{}</name><duration>{}</duration>{}<timecode>{}<string>{}</string><frame>{start}</frame><displayformat>NDF</displayformat></timecode><media><video><format><samplecharacteristics>{}<width>{}</width><height>{}</height><pixelaspectratio>square</pixelaspectratio></samplecharacteristics></format>{video}</video><audio><numOutputChannels>2</numOutputChannels><format><samplecharacteristics><depth>16</depth><samplerate>48000</samplerate></samplecharacteristics></format>{audio}</audio></media>{markers}</sequence></xmeml>\n",
        escape(&project.timeline_name),
        project.duration(),
        rate(project),
        rate(project),
        project.fps.timecode(start),
        rate(project),
        project.width,
        project.height,
    );
    Written { text, skipped }
}

// Reading: a Final Cut Pro 7 / Premiere / Resolve XML sequence as a timeline to import (the same
// form an OpenTimelineIO file is read into, so both go through one import).

/// The largest document read, the elements read from it at most, and how deep they may nest;
/// the document is untrusted, and past any of them it is refused. A feature-length edit is a
/// few megabytes and some hundred thousand elements.
const MAX_BYTES: usize = 256 * 1024 * 1024;
const MAX_NODES: usize = 1_000_000;
const MAX_DEPTH: usize = 256;

/// An element of the document: its name, attributes, text and the elements inside it.
#[derive(Default)]
struct Node {
    name: String,
    attrs: Vec<(String, String)>,
    text: String,
    children: Vec<Node>,
}

impl Node {
    fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.name == name)
    }

    fn all<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.children.iter().filter(move |c| c.name == name)
    }

    /// The trimmed text of the child `name`.
    fn text_of(&self, name: &str) -> Option<&str> {
        self.child(name).map(|c| c.text.trim())
    }

    /// The child `name` as a whole number (`-1` is one too: a clip edge in a transition).
    fn number(&self, name: &str) -> Option<i64> {
        let text = self.text_of(name)?;
        text.parse::<i64>()
            .ok()
            .or_else(|| text.parse::<f64>().ok().filter(|v| v.is_finite()).map(|v| v.round() as i64))
    }

    fn flag(&self, name: &str) -> Option<bool> {
        self.text_of(name).map(|t| t.eq_ignore_ascii_case("true"))
    }

    fn attr(&self, key: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    /// The first element called `name` here or below, in document order.
    fn find(&self, name: &str) -> Option<&Node> {
        if self.name == name {
            return Some(self);
        }
        self.children.iter().find_map(|c| c.find(name))
    }

    /// Every element called `name` here or below, outside the ones it is in.
    fn find_all<'a>(&'a self, name: &str, out: &mut Vec<&'a Node>) {
        for c in &self.children {
            if c.name == name {
                out.push(c);
            }
            c.find_all(name, out);
        }
    }
}

/// The name without its namespace prefix.
fn local_name(name: &[u8]) -> String {
    let name = String::from_utf8_lossy(name);
    name.rsplit(':').next().unwrap_or(&name).to_string()
}

/// The document as a tree of elements.
fn parse(text: &str) -> Result<Node, String> {
    use quick_xml::events::Event;
    let mut reader = quick_xml::Reader::from_str(text);
    let mut stack = vec![Node::default()];
    let mut count = 0usize;
    let element = |e: &quick_xml::events::BytesStart| Node {
        name: local_name(e.local_name().as_ref()),
        attrs: e
            .attributes()
            .flatten()
            .filter_map(|a| {
                let value = a.normalized_value(quick_xml::XmlVersion::Implicit1_0).ok()?;
                Some((local_name(a.key.local_name().as_ref()), value.into_owned()))
            })
            .collect(),
        ..Node::default()
    };
    loop {
        let event = reader
            .read_event()
            .map_err(|e| format!("this is not a readable XML file ({e})"))?;
        if matches!(event, Event::Start(_) | Event::Empty(_)) {
            count += 1;
            if count > MAX_NODES {
                return Err("the XML file is too large".into());
            }
        }
        match event {
            Event::Start(e) => {
                stack.push(element(&e));
                if stack.len() > MAX_DEPTH {
                    return Err("the XML file nests too deep".into());
                }
            }
            Event::Empty(e) => {
                let node = element(&e);
                if let Some(top) = stack.last_mut() {
                    top.children.push(node);
                }
            }
            Event::End(_) => {
                if stack.len() > 1 {
                    if let Some(done) = stack.pop() {
                        if let Some(top) = stack.last_mut() {
                            top.children.push(done);
                        }
                    }
                }
            }
            // The space that indents the document is not kept: it is most of a large file.
            Event::Text(t) => {
                if let (Some(top), Ok(v)) = (stack.last_mut(), t.decode()) {
                    if !(top.text.is_empty() && v.trim().is_empty()) {
                        top.text.push_str(&v);
                    }
                }
            }
            Event::CData(t) => {
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&String::from_utf8_lossy(&t));
                }
            }
            // `&amp;`, `&#233;`: quick-xml hands them over on their own.
            Event::GeneralRef(r) => {
                let resolved = match r.resolve_char_ref() {
                    Ok(Some(c)) => Some(c.to_string()),
                    _ => r.decode().ok().and_then(|name| {
                        quick_xml::escape::resolve_predefined_entity(&name).map(str::to_string)
                    }),
                };
                if let (Some(top), Some(text)) = (stack.last_mut(), resolved) {
                    top.text.push_str(&text);
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    // An element left open at the end is kept with what it had.
    while stack.len() > 1 {
        if let Some(done) = stack.pop() {
            if let Some(top) = stack.last_mut() {
                top.children.push(done);
            }
        }
    }
    stack.pop().ok_or_else(|| "empty XML file".to_string())
}

/// The frames a second of a `<rate>`: `timebase`, times 1000/1001 when `ntsc` is TRUE.
fn rate_of(node: Option<&Node>) -> Option<f64> {
    let rate = node?.child("rate")?;
    let base = rate.number("timebase").filter(|b| (1..=1000).contains(b))? as f64;
    Some(if rate.flag("ntsc") == Some(true) {
        base * 1000.0 / 1001.0
    } else {
        base
    })
}

/// Frames at `from` frames a second as frames of the project; a negative number is none.
fn frames_at(value: i64, from: f64, to: f64) -> Option<u64> {
    if value < 0 {
        return None;
    }
    let frames = (value as f64 / from * to).round();
    (frames.is_finite() && frames <= crate::project::MAX_FRAMES as f64).then_some(frames as u64)
}

/// A file a sequence uses, from its first, full `<file>` element.
#[derive(Clone)]
struct XmlFile {
    path: Option<String>,
    name: String,
    /// Its length, in frames of the project.
    length: Option<u64>,
}

/// The file a `<file>` element describes, when it describes one (rather than only naming it).
fn file_of(f: &Node, seq_rate: f64, fps: f64) -> Option<XmlFile> {
    if f.child("pathurl").is_none() && f.child("name").is_none() {
        return None;
    }
    let path = f.text_of("pathurl").and_then(crate::otio::path_of_url);
    let name = f
        .text_of("name")
        .filter(|n| !n.is_empty())
        .map(str::to_string)
        .or_else(|| {
            path.as_deref()
                .map(|p| p.rsplit(['/', '\\']).next().unwrap_or(p).to_string())
        })
        .unwrap_or_default();
    let rate = rate_of(Some(f)).unwrap_or(seq_rate);
    let length = f.number("duration").and_then(|d| frames_at(d, rate, fps));
    Some(XmlFile { path, name, length })
}

/// Every file described in the document, by its id (later `<file id="..."/>` only name it); the
/// first description of an id is the one kept.
fn files_of(root: &Node, seq_rate: f64, fps: f64) -> HashMap<String, XmlFile> {
    let mut nodes = Vec::new();
    root.find_all("file", &mut nodes);
    let mut out = HashMap::new();
    for f in nodes {
        let Some(id) = f.attr("id").filter(|i| !i.is_empty()) else {
            continue;
        };
        if !out.contains_key(id) {
            if let Some(file) = file_of(f, seq_rate, fps) {
                out.insert(id.to_string(), file);
            }
        }
    }
    out
}

/// The speed, direction and freeze of a clip's Time Remap filter.
fn time_remap(clip: &Node) -> (f32, bool, bool) {
    for effect in clip.all("filter").filter_map(|f| f.child("effect")) {
        let id = effect.text_of("effectid").unwrap_or("");
        let name = effect.text_of("name").unwrap_or("");
        if !id.eq_ignore_ascii_case("timeremap") && !name.eq_ignore_ascii_case("time remap") {
            continue;
        }
        let param = |key: &str| {
            effect.all("parameter").find(|p| {
                p.text_of("parameterid")
                    .or_else(|| p.text_of("name"))
                    .is_some_and(|k| k.eq_ignore_ascii_case(key))
            })
        };
        let speed = param("speed")
            .and_then(|p| p.text_of("value"))
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|v| v.is_finite())
            .unwrap_or(100.0);
        let reverse = param("reverse")
            .and_then(|p| p.text_of("value"))
            .is_some_and(|v| v.eq_ignore_ascii_case("true"))
            ^ (speed < 0.0);
        if speed == 0.0 {
            return (1.0, reverse, true);
        }
        return ((speed.abs() / 100.0) as f32, reverse, false);
    }
    (1.0, false, false)
}

/// What a track's item is, in the order the track lists them.
enum Item<'a> {
    Clip(&'a Node),
    /// A transition: where it starts and ends, where its cut is, and its kind.
    Transition {
        start: i64,
        end: i64,
        kind: TransitionKind,
    },
    Other,
}

fn items_of(track: &Node) -> Vec<Item<'_>> {
    track
        .children
        .iter()
        .filter_map(|n| match n.name.as_str() {
            "clipitem" => Some(Item::Clip(n)),
            "transitionitem" => {
                let name = n
                    .child("effect")
                    .and_then(|e| e.text_of("name"))
                    .unwrap_or("");
                Some(Item::Transition {
                    start: n.number("start").unwrap_or(-1),
                    end: n.number("end").unwrap_or(-1),
                    kind: if name.to_ascii_lowercase().contains("dip") {
                        TransitionKind::DipToBlack
                    } else {
                        TransitionKind::CrossDissolve
                    },
                })
            }
            "generatoritem" => Some(Item::Other),
            _ => None,
        })
        .collect()
}

/// The cut of a transition: its middle (the format's `center` alignment, the usual one).
fn cut_of(item: Option<&Item>) -> Option<i64> {
    match item {
        Some(Item::Transition { start, end, .. }) if *start >= 0 && *end >= *start => {
            Some(start + (end - start) / 2)
        }
        _ => None,
    }
}

/// What reading a sequence's tracks needs: the document's files and named sequences, and the
/// rates its times are converted to.
struct Ctx<'a> {
    files: &'a HashMap<String, XmlFile>,
    sequences: &'a HashMap<String, &'a Node>,
    seq_rate: f64,
    fps: f64,
}

fn read_xml_track(
    track: &Node,
    kind: TrackKind,
    ctx: &Ctx<'_>,
    taken: &mut HashSet<(String, u64, u64, u64)>,
    visited: &mut HashSet<String>,
    depth: usize,
    skipped: &mut usize,
) -> OtioTrack {
    let (seq_rate, fps) = (ctx.seq_rate, ctx.fps);
    let items = items_of(track);
    let mut clips = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let node = match item {
            Item::Clip(node) => *node,
            Item::Transition { .. } => continue,
            Item::Other => {
                *skipped += 1;
                continue;
            }
        };
        // A clipitem may hold a nested sequence instead of a file: its own <name>, <in>, <out>,
        // <start> and <end> are read as the outer clip's, and the sequence's tracks as the
        // compound clip's.
        let nested = node
            .child("sequence")
            .map(|seq| nested_tracks(seq, ctx, visited, depth + 1, skipped))
            .unwrap_or_default();
        let file = node.child("file").and_then(|file_node| {
            file_node
                .attr("id")
                .and_then(|id| ctx.files.get(id))
                .cloned()
                .or_else(|| file_of(file_node, seq_rate, fps))
        });
        // A nested sequence or a multicam clip has no one file.
        if file.is_none() && nested.is_empty() {
            *skipped += 1;
            continue;
        }
        let clip_rate = rate_of(Some(node)).unwrap_or(seq_rate);
        let (speed, reverse, freeze) = time_remap(node);
        let source_in = node.number("in").and_then(|v| frames_at(v, clip_rate, fps));
        let source_out = node.number("out").and_then(|v| frames_at(v, clip_rate, fps));
        let span = match (source_in, source_out) {
            (Some(a), Some(b)) if b > a => Some(b - a),
            _ => None,
        };
        // A clip edge inside a transition is -1: the transition's cut is the edge.
        let before = i.checked_sub(1).and_then(|j| items.get(j));
        let start = node
            .number("start")
            .and_then(|v| frames_at(v, seq_rate, fps))
            .or_else(|| cut_of(before).and_then(|c| frames_at(c, seq_rate, fps)));
        let end = node
            .number("end")
            .and_then(|v| frames_at(v, seq_rate, fps))
            .or_else(|| cut_of(items.get(i + 1)).and_then(|c| frames_at(c, seq_rate, fps)));
        let on_timeline = |span: u64| {
            if freeze {
                span
            } else {
                (span as f64 / f64::from(speed.max(f32::MIN_POSITIVE))).round() as u64
            }
        };
        let (start, duration) = match (start, end, span) {
            (Some(s), Some(e), _) if e > s => (s, e - s),
            (Some(s), _, Some(span)) => (s, on_timeline(span)),
            (None, Some(e), Some(span)) => (e.saturating_sub(on_timeline(span)), on_timeline(span)),
            _ => {
                *skipped += 1;
                continue;
            }
        };
        if duration == 0 {
            continue;
        }
        let source_in = source_in.unwrap_or(0);
        // A stereo pair is the same file on two sound tracks: it is heard once.
        if let Some(file) = &file {
            let key = (
                file.path.clone().unwrap_or_else(|| file.name.clone()),
                start,
                duration,
                source_in,
            );
            if kind == TrackKind::Audio && !taken.insert(key) {
                continue;
            }
        }
        let transition = match before {
            Some(Item::Transition { start: s, end: e, kind }) if *e > *s => {
                frames_at(e - s, seq_rate, fps).map(|frames| crate::Transition {
                    kind: *kind,
                    frames,
                })
            }
            _ => None,
        };
        let name = node.text_of("name").unwrap_or("").to_string();
        let media_name = match &file {
            Some(file) => file.name.clone(),
            None => name.clone(),
        };
        clips.push(OtioClip {
            name,
            media_name,
            media: match &file {
                Some(file) => match &file.path {
                    Some(path) => OtioMedia::File { path: path.clone() },
                    None => OtioMedia::Missing,
                },
                None => OtioMedia::Missing,
            },
            available: file.as_ref().and_then(|f| f.length),
            record: start,
            source_in,
            duration,
            speed,
            reverse,
            freeze,
            enabled: node.flag("enabled") != Some(false),
            transition,
            nested,
        });
        if clips.len() >= crate::otio::MAX_ITEMS {
            break;
        }
    }
    OtioTrack {
        kind,
        enabled: track.flag("enabled") != Some(false),
        clips,
    }
}

/// The tracks of a nested sequence in a clipitem: the sequence itself when it is inline, or the
/// `<sequence id="...">` it refers to. Reading stops at `MAX_NESTING_DEPTH` and at a sequence
/// that is already being read, so a file that nests deep or refers back to itself cannot loop.
fn nested_tracks(
    seq: &Node,
    ctx: &Ctx<'_>,
    visited: &mut HashSet<String>,
    depth: usize,
    skipped: &mut usize,
) -> Vec<OtioTrack> {
    let seq = if seq.child("media").is_some() {
        seq
    } else {
        match seq.attr("id").and_then(|id| ctx.sequences.get(id).copied()) {
            Some(found) => found,
            None => return Vec::new(),
        }
    };
    if depth > MAX_NESTING_DEPTH {
        *skipped += 1;
        return Vec::new();
    }
    let id = seq.attr("id").map(str::to_string);
    if let Some(id) = &id {
        if !visited.insert(id.clone()) {
            *skipped += 1;
            return Vec::new();
        }
    }
    let mut out = Vec::new();
    if let Some(media) = seq.child("media") {
        // A stereo pair inside the nested sequence is heard once, as on the timeline.
        let mut taken = HashSet::new();
        for (kind, part) in [(TrackKind::Video, "video"), (TrackKind::Audio, "audio")] {
            for track in media.child(part).into_iter().flat_map(|m| m.all("track")) {
                out.push(read_xml_track(
                    track, kind, ctx, &mut taken, visited, depth, skipped,
                ));
            }
        }
    }
    if let Some(id) = id {
        visited.remove(&id);
    }
    out
}

/// Whether `text` looks like a Final Cut Pro 7 / Premiere XML sequence with clips (rather than
/// another XML file), without reading all of it.
pub fn is_xmeml_timeline(text: &str) -> bool {
    let head: String = text.chars().take(4096).collect();
    head.contains("<xmeml") && text.contains("<clipitem")
}

/// Reads the first sequence of a Final Cut Pro 7 / Premiere XML file (the format Resolve and
/// Premiere also export), with every time in frames of `fps`: its tracks of clips (by the files
/// they come from), dissolves, speed changes, reverse, freeze frames, disabled clips, turned
/// off tracks and markers. A clipitem that holds a nested sequence (a compound clip) keeps its
/// tracks in [`OtioClip::nested`], at most `MAX_NESTING_DEPTH` deep. Multicam clips and
/// generators are left out (counted in `skipped`).
pub fn read_xmeml(text: &str, fps: crate::Fps) -> Result<OtioTimeline, String> {
    if text.len() > MAX_BYTES {
        return Err("the XML file is too large to import".into());
    }
    let root = parse(text)?;
    let xmeml = root.find("xmeml").ok_or("this is not a Final Cut Pro XML file")?;
    let sequence = xmeml
        .find("sequence")
        .ok_or("the XML file holds no sequence")?;
    let media = sequence
        .child("media")
        .ok_or("the sequence has no tracks")?;
    let fps = fps.as_f64();
    let seq_rate = rate_of(Some(sequence)).unwrap_or(fps);
    let files = files_of(xmeml, seq_rate, fps);
    // The sequences that a clipitem may refer to by id, for nested sequences.
    let mut seq_nodes = Vec::new();
    xmeml.find_all("sequence", &mut seq_nodes);
    let mut sequences: HashMap<String, &Node> = HashMap::new();
    for s in seq_nodes {
        if let Some(id) = s.attr("id") {
            sequences.entry(id.to_string()).or_insert(s);
        }
    }
    let ctx = Ctx {
        files: &files,
        sequences: &sequences,
        seq_rate,
        fps,
    };
    let mut skipped = 0;
    let mut taken = HashSet::new();
    let mut visited = HashSet::new();
    let mut tracks = Vec::new();
    for (kind, part) in [(TrackKind::Video, "video"), (TrackKind::Audio, "audio")] {
        for track in media.child(part).into_iter().flat_map(|m| m.all("track")) {
            tracks.push(read_xml_track(
                track,
                kind,
                &ctx,
                &mut taken,
                &mut visited,
                0,
                &mut skipped,
            ));
        }
    }
    let markers = sequence
        .all("marker")
        .filter_map(|m| {
            Some(crate::Marker {
                frame: m.number("in").and_then(|v| frames_at(v, seq_rate, fps))?,
                name: m.text_of("name").unwrap_or("").to_string(),
                color: 0,
            })
        })
        .take(crate::project::MAX_MARKERS)
        .collect();
    Ok(OtioTimeline {
        name: sequence.text_of("name").unwrap_or("").to_string(),
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
        let mut p = Project::new("xml test");
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
        add(&mut p, "a & b", r"C:\Media\a b.mov", 0);
        add(&mut p, "c", "/home/me/c.mov", 48);
        p
    }

    #[test]
    fn clips_files_and_dissolves_are_written() {
        let mut p = project();
        p.tracks[0].clips[1].transition = Some(Transition {
            kind: TransitionKind::CrossDissolve,
            frames: 12,
        });
        p.tracks[0].clips[1].speed = 2.0;
        p.markers.push(crate::Marker {
            frame: 5,
            name: "Here".into(),
            color: 0,
        });
        let w = to_xmeml(&p);
        assert_eq!(w.skipped, 0);
        let t = &w.text;
        assert!(t.starts_with("<?xml version=\"1.0\""), "{t}");
        assert!(t.contains("<xmeml version=\"5\">"));
        assert!(t.contains("<name>a &amp; b</name>"));
        assert!(
            t.contains("<pathurl>file://localhost/C:/Media/a%20b.mov</pathurl>"),
            "{t}"
        );
        assert!(t.contains("<pathurl>file://localhost/home/me/c.mov</pathurl>"));
        assert!(
            t.contains("<start>42</start><end>54</end><alignment>center</alignment>"),
            "{t}"
        );
        assert!(t.contains("<value>200.0000</value>"));
        assert!(t.contains("<marker><name>Here</name><comment></comment><in>5</in>"));
        assert!(t.contains("<timebase>24</timebase><ntsc>FALSE</ntsc>"));
        assert!(t.contains("<string>01:00:00:00</string><frame>86400</frame>"));
        assert_well_formed(t);
    }

    /// Every element that opens is closed, in order.
    fn assert_well_formed(text: &str) {
        let mut open: Vec<&str> = Vec::new();
        let body = text.split_once("<xmeml").map_or(text, |(_, b)| b);
        let doc = format!("<xmeml{body}");
        for piece in doc.split('<').skip(1) {
            let tag = piece.split_once('>').map_or(piece, |(t, _)| t);
            if let Some(name) = tag.strip_prefix('/') {
                assert_eq!(open.pop(), Some(name), "closing {name}");
            } else if !tag.ends_with('/') {
                open.push(tag.split(' ').next().unwrap_or(tag));
            }
        }
        assert!(open.is_empty(), "left open: {open:?}");
    }

    #[test]
    fn a_file_is_described_once_and_generated_media_is_skipped() {
        let mut p = project();
        let media = p.tracks[0].clips[0].media;
        let v1 = p.tracks[0].id;
        Command::AddClip {
            track: v1,
            media,
            start: 200,
            duration: Some(10),
            with_sound: false,
        }
        .apply(&mut p)
        .unwrap();
        let w = to_xmeml(&p);
        assert_well_formed(&w.text);
        assert_eq!(
            w.text
                .matches(&format!("<file id=\"file-{}\">", media.0))
                .count(),
            1
        );
        assert_eq!(
            w.text
                .matches(&format!("<file id=\"file-{}\"/>", media.0))
                .count(),
            1
        );
        let generated = to_xmeml(&Project::demo());
        assert_eq!(generated.skipped, 3);
        assert!(!generated.text.contains("<clipitem"));
    }

    /// Writes the test sequence to `PICASSO_XML_SAMPLE`, for checking with other readers
    /// (`scripts/check_interchange.py`, in CI).
    #[test]
    fn sample_for_other_readers() {
        if let Ok(path) = std::env::var("PICASSO_XML_SAMPLE") {
            let mut p = project();
            p.tracks[0].clips[1].transition = Some(Transition {
                kind: TransitionKind::CrossDissolve,
                frames: 12,
            });
            std::fs::write(path, to_xmeml(&p).text).unwrap_or_else(|e| panic!("{e}"));
        }
    }

    #[test]
    fn a_written_sequence_reads_back_as_it_was() {
        let mut p = project();
        {
            let second = &mut p.tracks[0].clips[1];
            second.transition = Some(Transition {
                kind: TransitionKind::DipToBlack,
                frames: 12,
            });
            second.speed = 2.0;
            second.reverse = true;
            second.duration = 24;
        }
        p.tracks[0].clips[0].enabled = false;
        p.markers.push(crate::Marker {
            frame: 30,
            name: "Look".into(),
            color: 2,
        });
        let text = to_xmeml(&p).text;
        assert!(is_xmeml_timeline(&text));
        let t = read_xmeml(&text, p.fps).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(t.name, p.timeline_name);
        assert_eq!(t.skipped, 0);
        let v1 = &t.tracks[0];
        assert_eq!(v1.kind, TrackKind::Video);
        let (a, c) = (&v1.clips[0], &v1.clips[1]);
        assert_eq!(a.name, "a & b");
        assert_eq!(
            a.media,
            OtioMedia::File {
                path: "C:/Media/a b.mov".into()
            }
        );
        assert_eq!(a.media_name, "a b.mov");
        assert_eq!((a.record, a.duration, a.source_in), (0, 48, 0));
        assert!(!a.enabled);
        assert_eq!(a.available, Some(240));
        assert_eq!((c.record, c.duration), (48, 24));
        assert_eq!((c.speed, c.reverse, c.freeze), (2.0, true, false));
        assert_eq!(
            c.transition,
            Some(Transition {
                kind: TransitionKind::DipToBlack,
                frames: 12
            })
        );
        assert_eq!(t.markers.len(), 1);
        assert_eq!((t.markers[0].frame, t.markers[0].name.as_str()), (30, "Look"));
    }

    /// A sequence as Final Cut Pro 7 and Premiere write it: 29.97 fps, clip edges of -1 inside
    /// a transition, files named by id after the first, a stereo pair on two sound tracks, a
    /// nested sequence and a generator.
    const FOREIGN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE xmeml>
<xmeml version="4"><project><name>P</name><children>
<sequence id="sequence-1"><name>Edit &amp; Mix</name><rate><timebase>30</timebase><ntsc>TRUE</ntsc></rate>
<media><video><track>
  <clipitem id="clipitem-1"><name>Interview</name><start>0</start><end>-1</end><in>100</in><out>190</out>
    <file id="file-1"><name>A&amp;B.mov</name><pathurl>file://localhost/Volumes/Media/A%26B.mov</pathurl>
      <rate><timebase>30</timebase><ntsc>TRUE</ntsc></rate><duration>900</duration></file></clipitem>
  <transitionitem><start>80</start><end>100</end><alignment>center</alignment>
    <effect><name>Cross Dissolve</name></effect></transitionitem>
  <clipitem id="clipitem-2"><name>Wide</name><start>-1</start><end>150</end><in>20</in><out>80</out>
    <file id="file-2"><name>wide.mov</name><pathurl>file:///C:/Media/wide.mov</pathurl><duration>600</duration></file>
    <filter><effect><name>Time Remap</name><effectid>timeremap</effectid>
      <parameter><parameterid>speed</parameterid><value>100</value></parameter></effect></filter></clipitem>
  <clipitem id="clipitem-3"><name>Nest</name><start>150</start><end>200</end><in>0</in><out>50</out>
    <sequence id="sequence-2"><name>Nested</name><media/></sequence></clipitem>
  <generatoritem id="gen-1"><name>Slug</name><start>200</start><end>230</end></generatoritem>
  <enabled>TRUE</enabled></track></video>
<audio>
  <track><clipitem id="clipitem-4"><name>Interview</name><start>0</start><end>90</end><in>100</in><out>190</out>
    <file id="file-1"/><sourcetrack><mediatype>audio</mediatype><trackindex>1</trackindex></sourcetrack></clipitem>
    <enabled>FALSE</enabled></track>
  <track><clipitem id="clipitem-5"><name>Interview</name><start>0</start><end>90</end><in>100</in><out>190</out>
    <file id="file-1"/><sourcetrack><mediatype>audio</mediatype><trackindex>2</trackindex></sourcetrack></clipitem></track>
</audio></media>
<marker><name>Chapter</name><in>45</in><out>-1</out></marker>
</sequence></children></project></xmeml>"#;

    #[test]
    fn a_sequence_from_another_editor_is_read() {
        let fps = crate::Fps { num: 30000, den: 1001 };
        let t = read_xmeml(FOREIGN, fps).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(t.name, "Edit & Mix");
        // The nested sequence and the generator are left out.
        assert_eq!(t.skipped, 2);
        let v1 = &t.tracks[0];
        assert_eq!(v1.clips.len(), 2);
        let (a, b) = (&v1.clips[0], &v1.clips[1]);
        // `&amp;` and `%26` are both an ampersand; a file named by id is the one described.
        assert_eq!(
            a.media,
            OtioMedia::File {
                path: "/Volumes/Media/A&B.mov".into()
            }
        );
        assert_eq!(a.media_name, "A&B.mov");
        // The clip's end inside the transition is the transition's middle.
        assert_eq!((a.record, a.duration, a.source_in), (0, 90, 100));
        assert_eq!((b.record, b.duration, b.source_in), (90, 60, 20));
        assert_eq!(
            b.transition,
            Some(Transition {
                kind: TransitionKind::CrossDissolve,
                frames: 20
            })
        );
        assert_eq!(a.available, Some(900));
        // The stereo pair is one sound clip, and a turned off track is read as such.
        let sounds: Vec<&OtioTrack> = t
            .tracks
            .iter()
            .filter(|t| t.kind == TrackKind::Audio)
            .collect();
        assert_eq!(sounds.len(), 2);
        assert!(!sounds[0].enabled);
        assert_eq!(sounds[0].clips.len(), 1);
        assert!(sounds[1].clips.is_empty(), "the second channel is not a second clip");
        assert_eq!(t.markers[0].frame, 45);
    }

    const NESTED: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<xmeml version="5">
 <sequence id="sequence-main"><name>Main</name>
  <rate><timebase>24</timebase><ntsc>FALSE</ntsc></rate>
  <media><video><track>
   <clipitem id="clipitem-outer"><name>Nested</name><start>0</start><end>48</end><in>0</in><out>48</out>
    <sequence id="sequence-inner"><name>Inner</name>
     <rate><timebase>24</timebase><ntsc>FALSE</ntsc></rate>
     <media><video><track>
      <clipitem id="clipitem-inner"><name>Shot</name><start>0</start><end>48</end><in>0</in><out>48</out>
       <file id="file-1"><name>shot.mov</name><pathurl>file://localhost/media/shot.mov</pathurl>
        <rate><timebase>24</timebase><ntsc>FALSE</ntsc></rate><duration>240</duration></file></clipitem>
     </track></video></media>
    </sequence></clipitem>
  </track></video></media>
 </sequence>
</xmeml>"#;

    #[test]
    fn a_nested_sequence_keeps_its_tracks() -> Result<(), Box<dyn std::error::Error>> {
        let fps = crate::Fps { num: 24, den: 1 };
        let t = read_xmeml(NESTED, fps)?;
        assert_eq!(t.skipped, 0);
        assert_eq!(t.tracks.len(), 1);
        let outer = t.tracks[0].clips.first().ok_or("the outer clip is read")?;
        // The outer clipitem's own name, in, out, start and end are kept, not the inner one's.
        assert_eq!(outer.name, "Nested");
        assert_eq!((outer.record, outer.source_in, outer.duration), (0, 0, 48));
        assert_eq!(outer.nested.len(), 1, "the nested sequence's track");
        let inner = outer.nested[0]
            .clips
            .first()
            .ok_or("the inner clip is read")?;
        assert_eq!(inner.name, "Shot");
        assert_eq!(
            inner.media,
            OtioMedia::File {
                path: "/media/shot.mov".into()
            }
        );
        assert_eq!((inner.record, inner.duration), (0, 48));
        Ok(())
    }

    #[test]
    fn a_sequence_that_refers_to_itself_stops() -> Result<(), Box<dyn std::error::Error>> {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<xmeml version="5">
 <sequence id="sequence-main"><name>Loop</name>
  <rate><timebase>24</timebase><ntsc>FALSE</ntsc></rate>
  <media><video><track>
   <clipitem id="clipitem-1"><name>Again</name><start>0</start><end>48</end><in>0</in><out>48</out>
    <sequence id="sequence-main"/></clipitem>
  </track></video></media>
 </sequence>
</xmeml>"#;
        let fps = crate::Fps { num: 24, den: 1 };
        let t = read_xmeml(xml, fps)?;
        assert!(t.skipped >= 1, "the loop is cut");
        // It terminated, and nothing recursed into the same sequence twice.
        let outer = t.tracks[0].clips.first().ok_or("the clip is read")?;
        assert!(outer.nested.iter().all(|track| track.clips.is_empty()));
        Ok(())
    }

    #[test]
    fn other_times_are_converted_and_broken_files_refused() {
        // A 30 fps sequence read into a 60 fps project: twice the frames.
        let t = read_xmeml(FOREIGN, crate::Fps { num: 60000, den: 1001 })
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            (t.tracks[0].clips[0].record, t.tracks[0].clips[0].duration),
            (0, 180)
        );
        let fps = crate::Fps { num: 24, den: 1 };
        assert!(read_xmeml("<html><body/></html>", fps).is_err());
        assert!(read_xmeml("not xml at all <<<", fps).is_err());
        assert!(read_xmeml("<xmeml><project/></xmeml>", fps).is_err());
        let deep = "<a>".repeat(MAX_DEPTH + 10);
        assert!(read_xmeml(&deep, fps).is_err());
        assert!(!is_xmeml_timeline("<?xml version=\"1.0\"?><resolve><project/></resolve>"));
    }
}
