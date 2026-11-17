//! Reading an AAF file's timeline: the composition an editor exported (Avid Media Composer, Pro
//! Tools, Premiere, Resolve, or this program), as the timeline an OpenTimelineIO file is read
//! into, so the two are imported the same way.
//!
//! Written from the same AMWA specifications as the writer. The objects are read from the
//! compound file's storages and `properties` streams; the timeline is the top-level composition
//! mob (or the one no other composition uses), each of its picture and sound slots a track, and
//! each source clip is followed through its master mob to the file source mob, whose locator
//! gives the file's address. A transition overlaps the clips on either side of it, as AAF lays
//! it out, and becomes a dissolve on the cut at its cut point.
//!
//! The file is untrusted: the objects read, how deep they nest and the size of a stream are
//! capped, and nothing here panics. Left out (counted in `skipped`): nested compositions,
//! effects other than the clip inside them, and markers.

use std::collections::{HashMap, HashSet};
use std::io::{Cursor, Read};

use cfb::CompoundFile;
use picasso_core::otio::{
    path_of_url, OtioClip, OtioMedia, OtioTimeline, OtioTrack, MAX_ITEMS, MAX_NESTING_DEPTH,
};
use picasso_core::{Fps, Marker, TrackKind, Transition, TransitionKind};
use uuid::Uuid;

use crate::{class, data, id, pid, SF_DATA, SF_STRONG, SF_STRONG_SET, SF_STRONG_VECTOR, SF_WEAK};

/// Objects read from one file at most, how deep they may nest, and the most bytes of one stream.
const MAX_OBJECTS: usize = 500_000;
const MAX_DEPTH: usize = 64;
const MAX_STREAM: u64 = 16 * 1024 * 1024;

/// Classes read here besides the writer's.
const SELECTOR: Uuid = id("0d010101-0101-0e00-060e-2b3402060101");
const ESSENCE_GROUP: Uuid = id("0d010101-0101-0500-060e-2b3402060101");

/// Property ids read here besides the writer's.
const USAGE_CODE: u16 = 0x4408;
const SELECTED: u16 = 0x0f01;
const CHOICES: u16 = 0x0501;
const INPUT_SEGMENTS: u16 = 0x0b02;

/// The Edit Protocol operations a time warp is written with.
const VIDEO_SPEED_CONTROL: Uuid = id("9d2ea890-0968-11d3-8a38-0050040ef7d2");
const VIDEO_REPEAT: Uuid = id("9d2ea891-0968-11d3-8a38-0050040ef7d2");

/// A composition meant as an editor's timeline, rather than one used inside another.
const USAGE_TOP_LEVEL: Uuid = id("0d010102-0101-0300-060e-2b3404010105");
/// The data definitions of pictures and sound that older Avid files use.
const LEGACY_PICTURE: Uuid = id("6f3c8ce1-6cef-11d2-807d-006008143e6f");
const LEGACY_SOUND: Uuid = id("78e1ebe1-6cef-11d2-807d-006008143e6f");
const PICTURE_WITH_MATTE: Uuid = id("05cba731-1daa-11d3-80ad-006008143e6f");

/// A property as read: its bytes, the objects it holds, or the key of what it refers to.
enum Prop {
    Data(Vec<u8>),
    Strong(Box<Object>),
    /// A vector or a set of objects.
    List(Vec<Object>),
    Weak(Vec<u8>),
}

struct Object {
    class: Uuid,
    props: Vec<(u16, Prop)>,
}

impl Object {
    fn get(&self, pid: u16) -> Option<&Prop> {
        self.props.iter().find(|(p, _)| *p == pid).map(|(_, v)| v)
    }

    fn data(&self, pid: u16) -> Option<&[u8]> {
        match self.get(pid) {
            Some(Prop::Data(b)) => Some(b),
            _ => None,
        }
    }

    fn strong(&self, pid: u16) -> Option<&Object> {
        match self.get(pid) {
            Some(Prop::Strong(o)) => Some(o),
            _ => None,
        }
    }

    fn list(&self, pid: u16) -> &[Object] {
        match self.get(pid) {
            Some(Prop::List(items)) => items,
            _ => &[],
        }
    }

    /// The id a weak reference names (a definition's AUID).
    fn weak_auid(&self, pid: u16) -> Option<Uuid> {
        match self.get(pid) {
            Some(Prop::Weak(key)) => Uuid::from_slice_le(key).ok(),
            _ => None,
        }
    }

    fn i64(&self, pid: u16) -> Option<i64> {
        Some(i64::from_le_bytes(self.data(pid)?.get(..8)?.try_into().ok()?))
    }

    fn u32(&self, pid: u16) -> Option<u32> {
        Some(u32::from_le_bytes(self.data(pid)?.get(..4)?.try_into().ok()?))
    }

    fn auid(&self, pid: u16) -> Option<Uuid> {
        Uuid::from_slice_le(self.data(pid)?.get(..16)?).ok()
    }

    /// A rational (an edit or sample rate), numerator and denominator both above 0.
    fn rate(&self, pid: u16) -> Option<(i64, i64)> {
        let b = self.data(pid)?;
        let num = i32::from_le_bytes(b.get(..4)?.try_into().ok()?);
        let den = i32::from_le_bytes(b.get(4..8)?.try_into().ok()?);
        (num > 0 && den > 0).then_some((i64::from(num), i64::from(den)))
    }

    fn text(&self, pid: u16) -> Option<String> {
        utf16(self.data(pid)?)
    }
}

/// UTF-16 (little-endian) text up to its terminating zero.
fn utf16(bytes: &[u8]) -> Option<String> {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|u| *u != 0)
        .collect();
    String::from_utf16(&units).ok()
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn join(parent: &str, name: &str) -> String {
    if parent == "/" {
        format!("/{name}")
    } else {
        format!("{parent}/{name}")
    }
}

/// The storage a strong reference names, inside `parent`: a plain name only, so that the file
/// cannot send the reader up or across the tree (`..`, `a/b`), round in a loop.
fn child(parent: &str, value: &[u8]) -> Result<(String, String), String> {
    let name = utf16(value).ok_or("the AAF file is damaged")?;
    if name.is_empty() || name.contains(['/', '\\']) || name == "." || name == ".." {
        return Err("the AAF file names a storage outside the object".into());
    }
    let path = join(parent, &name);
    Ok((name, path))
}

struct Reader<'a> {
    cf: CompoundFile<Cursor<&'a [u8]>>,
    objects: usize,
}

impl Reader<'_> {
    fn stream(&mut self, path: &str) -> Result<Vec<u8>, String> {
        let mut s = self
            .cf
            .open_stream(path)
            .map_err(|e| format!("the AAF file is damaged ({e})"))?;
        if s.len() > MAX_STREAM {
            return Err("the AAF file has a property stream too large to read".into());
        }
        let mut out = Vec::new();
        s.read_to_end(&mut out)
            .map_err(|e| format!("the AAF file could not be read ({e})"))?;
        Ok(out)
    }

    /// The property entries of the object at `path`: id, stored form and value.
    fn entries(&mut self, path: &str) -> Result<Vec<(u16, u16, Vec<u8>)>, String> {
        let b = self.stream(&join(path, "properties"))?;
        match b.first() {
            Some(0x4c) => {}
            Some(0x42) => return Err("this AAF file is big-endian, which is not read yet".into()),
            _ => return Err("this is not an AAF file (no property stream)".into()),
        }
        let count = usize::from(u16_at(&b, 2).ok_or("the AAF file is damaged")?);
        let mut value_at = 4 + count * 6;
        let mut out = Vec::with_capacity(count);
        for i in 0..count {
            let at = 4 + i * 6;
            let (Some(pid), Some(form), Some(len)) =
                (u16_at(&b, at), u16_at(&b, at + 2), u16_at(&b, at + 4))
            else {
                return Err("the AAF file is damaged".into());
            };
            let len = usize::from(len);
            let value = b
                .get(value_at..value_at + len)
                .ok_or("the AAF file is damaged")?
                .to_vec();
            value_at += len;
            out.push((pid, form, value));
        }
        Ok(out)
    }

    fn object(&mut self, path: &str, depth: usize) -> Result<Object, String> {
        self.objects += 1;
        if self.objects > MAX_OBJECTS {
            return Err("the AAF file holds too many objects".into());
        }
        if depth > MAX_DEPTH {
            return Err("the AAF file nests too deep".into());
        }
        let class = *self
            .cf
            .entry(path)
            .map_err(|e| format!("the AAF file is damaged ({e})"))?
            .clsid();
        let mut props = Vec::new();
        for (pid, form, value) in self.entries(path)? {
            if let Some(prop) = self.prop(path, form, &value, depth)? {
                props.push((pid, prop));
            }
        }
        Ok(Object { class, props })
    }

    fn prop(
        &mut self,
        path: &str,
        form: u16,
        value: &[u8],
        depth: usize,
    ) -> Result<Option<Prop>, String> {
        Ok(Some(match form {
            SF_DATA => Prop::Data(value.to_vec()),
            SF_STRONG => {
                let (_, at) = child(path, value)?;
                Prop::Strong(Box::new(self.object(&at, depth + 1)?))
            }
            SF_STRONG_VECTOR | SF_STRONG_SET => {
                let (name, _) = child(path, value)?;
                let index = self.stream(&join(path, &format!("{name} index")))?;
                let count = u32_at(&index, 0).ok_or("the AAF file is damaged")? as usize;
                // A vector's entries are local keys; a set's are a local key, a reference count
                // and the key, whose size the index gives after the key's property id.
                let (first, step) = if form == SF_STRONG_VECTOR {
                    (12, 4)
                } else {
                    let size = usize::from(*index.get(14).ok_or("the AAF file is damaged")?);
                    (15, 8 + size)
                };
                let mut items = Vec::new();
                for i in 0..count.min(MAX_OBJECTS) {
                    let Some(local) = u32_at(&index, first + i * step) else {
                        return Err("the AAF file is damaged".into());
                    };
                    let at = join(path, &format!("{name}{{{local:x}}}"));
                    items.push(self.object(&at, depth + 1)?);
                }
                Prop::List(items)
            }
            SF_WEAK => {
                let size = usize::from(*value.get(4).ok_or("the AAF file is damaged")?);
                Prop::Weak(value.get(5..5 + size).ok_or("the AAF file is damaged")?.to_vec())
            }
            // Weak reference vectors and sets, and forms this reader has no use for.
            _ => return Ok(None),
        }))
    }
}

/// Every mob a component tree refers to.
fn referenced(obj: &Object, out: &mut Vec<Vec<u8>>) {
    if obj.class == class::SOURCE_CLIP {
        if let Some(id) = obj.data(pid::SOURCE_ID) {
            out.push(id.to_vec());
        }
    }
    for (_, p) in &obj.props {
        match p {
            Prop::Strong(o) => referenced(o, out),
            Prop::List(items) => items.iter().for_each(|o| referenced(o, out)),
            _ => {}
        }
    }
}

/// Picture or sound, from a component's data definition.
fn kind_of(obj: &Object) -> Option<TrackKind> {
    match obj.weak_auid(pid::DATA_DEFINITION)? {
        k if k == data::PICTURE || k == LEGACY_PICTURE || k == PICTURE_WITH_MATTE => {
            Some(TrackKind::Video)
        }
        k if k == data::SOUND || k == LEGACY_SOUND => Some(TrackKind::Audio),
        _ => None,
    }
}

/// The source clip a segment plays: itself, or the one inside an effect, a selector or a group
/// of alternatives.
fn source_clip_of(seg: &Object) -> Option<&Object> {
    match seg.class {
        c if c == class::SOURCE_CLIP => Some(seg),
        c if c == class::OPERATION_GROUP => seg.list(INPUT_SEGMENTS).first().and_then(source_clip_of),
        c if c == SELECTOR => seg.strong(SELECTED).and_then(source_clip_of),
        c if c == ESSENCE_GROUP => seg.list(CHOICES).first().and_then(source_clip_of),
        c if c == class::SEQUENCE => seg
            .list(pid::COMPONENTS)
            .iter()
            .find_map(source_clip_of),
        _ => None,
    }
}

/// The time warp of a component: speed multiplier, direction and freeze, from an operation
/// group's operation and, for a speed change, its Speed Ratio parameter. `None` for a component
/// that plays at normal speed.
fn time_warp(component: &Object) -> Option<(f32, bool, bool)> {
    if component.class != class::OPERATION_GROUP {
        return None;
    }
    let operation = component.weak_auid(pid::OPERATION)?;
    if operation == VIDEO_REPEAT {
        return Some((1.0, false, true));
    }
    if operation != VIDEO_SPEED_CONTROL {
        return None;
    }
    // The Speed Ratio parameter: output length over input length, negative when reversed.
    let value = component.list(pid::PARAMETERS).first()?.data(pid::PARAMETER_VALUE)?;
    let num = i32::from_le_bytes(value.get(..4)?.try_into().ok()?);
    let den = i32::from_le_bytes(value.get(4..8)?.try_into().ok()?);
    if num == 0 || den == 0 {
        return None;
    }
    let ratio = f64::from(num) / f64::from(den);
    let speed = (1.0 / ratio.abs()) as f32;
    if !speed.is_finite() {
        return None;
    }
    Some((
        speed.clamp(picasso_core::MIN_SPEED, picasso_core::MAX_SPEED),
        ratio < 0.0,
        false,
    ))
}

/// Frames of the project in `value` units of `rate` (numerator, denominator).
fn frames(value: i64, rate: (i64, i64), fps: f64) -> u64 {
    let f = (value as f64 * rate.1 as f64 / rate.0 as f64 * fps).round();
    if f.is_finite() && f > 0.0 {
        (f as u64).min(picasso_core::project::MAX_FRAMES)
    } else {
        0
    }
}

/// A file of the timeline: where it is, its name and its length (frames of the project).
struct Source {
    media: OtioMedia,
    name: String,
    clip_name: String,
    available: Option<u64>,
    /// Where the file's first frame is in the source clip's time, and that time's rate.
    offset: i64,
    rate: Option<(i64, i64)>,
}

/// The address of a file source mob's media, from its descriptor's (or a sub-descriptor's)
/// network locator.
fn locator_of(descriptor: &Object) -> Option<String> {
    descriptor
        .list(pid::LOCATOR)
        .iter()
        .find_map(|l| l.text(pid::URL_STRING))
        .or_else(|| {
            descriptor
                .list(pid::FILE_DESCRIPTORS)
                .iter()
                .find_map(locator_of)
        })
}

/// The length of a descriptor's essence in frames of the project.
fn length_of(descriptor: &Object, fps: f64) -> Option<u64> {
    let own = descriptor
        .i64(pid::FILE_LENGTH)
        .zip(descriptor.rate(pid::SAMPLE_RATE))
        .map(|(l, r)| frames(l, r, fps))
        .filter(|l| *l > 0);
    own.or_else(|| {
        descriptor
            .list(pid::FILE_DESCRIPTORS)
            .iter()
            .filter_map(|d| length_of(d, fps))
            .max()
    })
}

struct Timeline<'a> {
    /// The mobs by id: a large project holds tens of thousands, and every clip looks some up.
    mobs: HashMap<&'a [u8], &'a Object>,
    fps: f64,
}

impl<'a> Timeline<'a> {
    fn mob(&self, id: &[u8]) -> Option<&'a Object> {
        self.mobs.get(id).copied()
    }

    /// What a composition's source clip plays, following master mobs to the file source mob.
    /// `None` for a nested composition (a compound clip), which `track` reads as nested tracks
    /// instead.
    fn source(&self, clip: &Object) -> Option<Source> {
        let mut mob = self.mob(clip.data(pid::SOURCE_ID)?)?;
        let mut slot_id = clip.u32(pid::SOURCE_MOB_SLOT_ID);
        let mut clip_name = String::new();
        let (mut offset, mut rate) = (0_i64, None);
        for _ in 0..8 {
            if mob.class == class::COMPOSITION_MOB {
                return None;
            }
            if clip_name.is_empty() {
                clip_name = mob.text(pid::MOB_NAME).unwrap_or_default();
            }
            if mob.class == class::SOURCE_MOB {
                let descriptor = mob.strong(pid::ESSENCE_DESCRIPTION);
                let path = descriptor.and_then(locator_of).and_then(|u| path_of_url(&u));
                let name = path
                    .as_deref()
                    .map(|p| p.rsplit(['/', '\\']).next().unwrap_or(p).to_string())
                    .unwrap_or_else(|| clip_name.clone());
                // A file mob without a locator may still lead to its tape or another file.
                if path.is_some() || mob.list(pid::SLOTS).is_empty() {
                    return Some(Source {
                        media: path.map_or(OtioMedia::Missing, |path| OtioMedia::File { path }),
                        name,
                        clip_name,
                        available: descriptor.and_then(|d| length_of(d, self.fps)),
                        offset,
                        rate,
                    });
                }            }
            // A master (or a source without a locator): through the slot the clip names.
            let slots = mob.list(pid::SLOTS);
            let slot = slots
                .iter()
                .find(|s| slot_id.is_some_and(|id| s.u32(pid::SLOT_ID) == Some(id)))
                .or_else(|| slots.first())?;
            rate = rate.or_else(|| slot.rate(pid::EDIT_RATE));
            let next = slot.strong(pid::SEGMENT).and_then(source_clip_of)?;
            offset = offset.saturating_add(next.i64(pid::START_TIME).unwrap_or(0));
            let next_id = next.data(pid::SOURCE_ID)?;
            if next_id.iter().all(|b| *b == 0) {
                // The end of the chain: the mob itself holds the media.
                return Some(Source {
                    media: OtioMedia::Missing,
                    name: clip_name.clone(),
                    clip_name,
                    available: None,
                    offset,
                    rate,
                });
            }
            slot_id = next.u32(pid::SOURCE_MOB_SLOT_ID);
            mob = self.mob(next_id)?;
        }
        None
    }

    fn track(
        &self,
        slot: &Object,
        visited: &mut HashSet<Vec<u8>>,
        depth: usize,
        skipped: &mut usize,
    ) -> Option<(TrackKind, u32, OtioTrack)> {
        let rate = slot.rate(pid::EDIT_RATE)?;
        let segment = slot.strong(pid::SEGMENT)?;
        let kind = kind_of(segment)?;
        let components: Vec<&Object> = if segment.class == class::SEQUENCE {
            segment.list(pid::COMPONENTS).iter().collect()
        } else {
            vec![segment]
        };
        // The slot's pieces in its own units: clips (start, length), and transitions (start,
        // length, cut point), which overlap the clips on either side.
        enum Piece<'o> {
            Clip(i64, i64, &'o Object),
            Transition(i64, i64, i64),
            Gap,
        }
        let mut pieces = Vec::new();
        let mut cursor = 0_i64;
        for c in components.iter().take(MAX_ITEMS) {
            let length = c.i64(pid::LENGTH).unwrap_or(0).max(0);
            if c.class == class::TRANSITION {
                let cut = c.i64(pid::CUT_POINT).unwrap_or(length / 2).clamp(0, length);
                cursor = cursor.saturating_sub(length);
                pieces.push(Piece::Transition(cursor, length, cut));
                continue;
            }
            match source_clip_of(c) {
                Some(_) if c.class != class::SEQUENCE => {
                    pieces.push(Piece::Clip(cursor, length, c));
                }
                _ => {
                    if c.class != class::FILLER {
                        *skipped += 1;
                    }
                    pieces.push(Piece::Gap);
                }
            }
            cursor = cursor.saturating_add(length);
        }
        let mut clips = Vec::new();
        for (i, piece) in pieces.iter().enumerate() {
            let Piece::Clip(start, length, component) = piece else {
                continue;
            };
            let Some(clip) = source_clip_of(component) else {
                continue;
            };
            let (speed, reverse, freeze) = time_warp(component).unwrap_or((1.0, false, false));
            let (mut start, mut length) = (*start, *length);
            let mut source_in = clip.i64(pid::START_TIME).unwrap_or(0);
            // A transition before: the clip starts at its cut, the frames before are handles.
            let before = i.checked_sub(1).and_then(|j| pieces.get(j));
            let transition = match before {
                Some(Piece::Transition(t, len, cut)) if *len > 0 => {
                    let into = (t + cut).saturating_sub(start).max(0);
                    start += into;
                    length -= into;
                    source_in += into;
                    Some(Transition {
                        kind: TransitionKind::CrossDissolve,
                        frames: frames(*len, rate, self.fps),
                    })
                }
                _ => None,
            };
            // A transition after: the clip ends at that one's cut.
            if let Some(Piece::Transition(t, len, cut)) = pieces.get(i + 1) {
                if *len > 0 {
                    length = length.min((t + cut).saturating_sub(start));
                }
            }
            if length <= 0 {
                continue;
            }
            // A source clip whose source is another composition is a nested sequence (a
            // compound clip): its tracks are read into the clip, never deeper than
            // MAX_NESTING_DEPTH and never a composition already being read.
            if let Some(mob) = clip.data(pid::SOURCE_ID).and_then(|id| self.mob(id)) {
                if mob.class == class::COMPOSITION_MOB {
                    let nested = self.nested_tracks(mob, visited, depth + 1, skipped);
                    if nested.is_empty() {
                        *skipped += 1;
                        continue;
                    }
                    let name = mob.text(pid::MOB_NAME).unwrap_or_default();
                    clips.push(OtioClip {
                        name: name.clone(),
                        media_name: name,
                        media: OtioMedia::Missing,
                        available: None,
                        record: frames(start, rate, self.fps),
                        source_in: 0,
                        duration: frames(length, rate, self.fps).max(1),
                        speed: 1.0,
                        reverse: false,
                        freeze: false,
                        enabled: true,
                        transition: transition.filter(|t| t.frames > 0),
                        nested,
                    });
                    continue;
                }
            }
            let Some(source) = self.source(clip) else {
                *skipped += 1;
                continue;
            };
            // The source clip's start is in the master's rate when it has one.
            let source_rate = source.rate.unwrap_or(rate);
            let in_source = source_in.saturating_add(source.offset);
            let source_in = frames(in_source, source_rate, self.fps);
            clips.push(OtioClip {
                name: source.clip_name.clone(),
                media_name: source.name,
                media: source.media,
                available: source.available,
                record: frames(start, rate, self.fps),
                source_in,
                duration: frames(length, rate, self.fps).max(1),
                speed,
                reverse,
                freeze,
                enabled: true,
                transition: transition.filter(|t| t.frames > 0),
                nested: vec![],
            });
        }
        let number = slot.u32(pid::PHYSICAL_TRACK_NUMBER).unwrap_or(u32::MAX);
        Some((
            kind,
            number,
            OtioTrack {
                kind,
                enabled: true,
                clips,
            },
        ))
    }

    /// The tracks of a composition used inside another (a nested sequence): its slots read as
    /// tracks, pictures first then sound. Reading stops at `MAX_NESTING_DEPTH` and at a
    /// composition that is already being read, so a file that nests deep or refers back to
    /// itself cannot loop.
    fn nested_tracks(
        &self,
        mob: &'a Object,
        visited: &mut HashSet<Vec<u8>>,
        depth: usize,
        skipped: &mut usize,
    ) -> Vec<OtioTrack> {
        if depth > MAX_NESTING_DEPTH {
            *skipped += 1;
            return Vec::new();
        }
        let Some(id) = mob.data(pid::MOB_ID) else {
            return Vec::new();
        };
        let key = id.to_vec();
        if !visited.insert(key.clone()) {
            *skipped += 1;
            return Vec::new();
        }
        let mut tracks: Vec<(TrackKind, u32, usize, OtioTrack)> = mob
            .list(pid::SLOTS)
            .iter()
            .filter(|s| s.class == class::TIMELINE_MOB_SLOT)
            .enumerate()
            .filter_map(|(i, s)| {
                let (kind, number, track) = self.track(s, visited, depth, skipped)?;
                Some((kind, number, i, track))
            })
            .collect();
        // Pictures first, then sound, each in the order of their track numbers.
        tracks.sort_by_key(|(kind, number, i, _)| (*kind == TrackKind::Audio, *number, *i));
        visited.remove(&key);
        tracks.into_iter().map(|(_, _, _, t)| t).collect()
    }
}

/// Reads the timeline of an AAF file (its top-level composition), with every time in frames of
/// `fps`. Errors when the bytes are not an AAF file or hold no composition.
pub fn read_aaf(bytes: &[u8], fps: Fps) -> Result<OtioTimeline, String> {
    let cf = CompoundFile::open(Cursor::new(bytes))
        .map_err(|e| format!("this is not an AAF file ({e})"))?;
    let mut reader = Reader { cf, objects: 0 };
    // Only the header is read from the root: the meta-dictionary describes classes, which the
    // baseline ones read here do not need.
    let header_entry = reader
        .entries("/")?
        .into_iter()
        .find(|(p, _, _)| *p == pid::HEADER)
        .ok_or("this AAF file has no header")?;
    let header = match reader.prop("/", header_entry.1, &header_entry.2, 0)? {
        Some(Prop::Strong(h)) => h,
        _ => return Err("this AAF file has no header".into()),
    };
    let content = header
        .strong(pid::CONTENT)
        .ok_or("this AAF file has no content")?;
    let all = content.list(pid::MOBS);
    let mut mobs = HashMap::with_capacity(all.len());
    for m in all {
        if let Some(id) = m.data(pid::MOB_ID) {
            mobs.entry(id).or_insert(m);
        }
    }
    let compositions: Vec<&Object> = all
        .iter()
        .filter(|m| m.class == class::COMPOSITION_MOB)
        .collect();
    // The timeline: the top-level composition, or the first one no other composition uses.
    let mut refs = Vec::new();
    for c in &compositions {
        referenced(c, &mut refs);
    }
    let used: HashSet<Vec<u8>> = refs.into_iter().collect();
    let top = compositions
        .iter()
        .find(|c| c.auid(USAGE_CODE) == Some(USAGE_TOP_LEVEL))
        .or_else(|| {
            compositions
                .iter()
                .find(|c| c.data(pid::MOB_ID).is_some_and(|id| !used.contains(id)))
        })
        .or_else(|| compositions.first())
        .ok_or("this AAF file holds no timeline (composition)")?;
    let timeline = Timeline {
        mobs,
        fps: fps.as_f64(),
    };
    let mut skipped = 0;
    let mut visited = HashSet::new();
    let mut tracks: Vec<(TrackKind, u32, usize, OtioTrack)> = top
        .list(pid::SLOTS)
        .iter()
        .filter(|s| s.class == class::TIMELINE_MOB_SLOT)
        .enumerate()
        .filter_map(|(i, s)| {
            let (kind, number, track) = timeline.track(s, &mut visited, 0, &mut skipped)?;
            Some((kind, number, i, track))
        })
        .collect();
    // Pictures first, then sound, each in the order of their track numbers.
    tracks.sort_by_key(|(kind, number, i, _)| (*kind == TrackKind::Audio, *number, *i));
    // Markers: locator objects on the composition mob.
    let markers: Vec<Marker> = top
        .list(pid::LOCATOR)
        .iter()
        .filter_map(|loc| {
            let frame = loc.i64(pid::START_TIME)? as u64;
            let name = loc.text(pid::MOB_NAME).unwrap_or_default();
            Some(Marker {
                frame,
                name,
                color: 0,
            })
        })
        .collect();
    Ok(OtioTimeline {
        name: top.text(pid::MOB_NAME).unwrap_or_default().trim().to_string(),
        tracks: tracks.into_iter().map(|(_, _, _, t)| t).collect(),
        markers,
        skipped,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use picasso_core::{Command, MediaSource, Outcome, Project};

    /// V1: two clips with a dissolve between them; A1: music, a gap, and the first video's sound.
    fn project() -> Project {
        let mut p = Project::new("aaf read");
        let v1 = p.tracks[0].id;
        let Ok(Outcome::Track(a1)) = (Command::AddTrack {
            kind: TrackKind::Audio,
        })
        .apply(&mut p) else {
            panic!("track");
        };
        let mut add = |source: MediaSource, track, start, length, source_in| {
            let Ok(Outcome::Media(m)) = (Command::AddMedia {
                name: "m".into(),
                source,
                length: Some(240),
                default_duration: 48,
            })
            .apply(&mut p) else {
                panic!("media");
            };
            let Ok(Outcome::Clip(c)) = (Command::AddClip {
                track,
                media: m,
                start,
                duration: Some(length),
                with_sound: false,
            })
            .apply(&mut p) else {
                panic!("clip");
            };
            Command::SlipClip { clip: c, source_in }.apply(&mut p).ok();
            m
        };
        let video = |path: &str| MediaSource::Video { path: path.into() };
        let a = add(video(r"C:\Media\a.mov"), v1, 0, 48, 24);
        add(video("/media/b & c.mov"), v1, 48, 48, 30);
        add(MediaSource::Audio { path: "/media/m.wav".into() }, a1, 12, 36, 0);
        Command::AddClip {
            track: a1,
            media: a,
            start: 60,
            duration: Some(24),
            with_sound: false,
        }
        .apply(&mut p)
        .unwrap_or_else(|e| panic!("{e}"));
        p.tracks[0].clips[1].transition = Some(Transition {
            kind: TransitionKind::CrossDissolve,
            frames: 12,
        });
        p
    }

    fn places(track: &OtioTrack) -> Vec<(u64, u64, u64)> {
        track
            .clips
            .iter()
            .map(|c| (c.record, c.duration, c.source_in))
            .collect()
    }

    #[test]
    fn a_written_timeline_reads_back_as_it_was() {
        let p = project();
        let written = crate::to_aaf_at(&p, crate::Stamp::from_unix(1_798_000_000))
            .unwrap_or_else(|e| panic!("{e}"));
        let t = read_aaf(&written.bytes, p.fps).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(t.name, p.timeline_name);
        assert_eq!(t.skipped, 0);
        // A track for each of the timeline's, empty ones too, pictures first.
        let kinds: Vec<TrackKind> = t.tracks.iter().map(|t| t.kind).collect();
        let want: Vec<TrackKind> = p.tracks.iter().map(|t| t.kind).collect();
        assert_eq!(kinds, want);
        let first = |kind| t.tracks.iter().find(|t| t.kind == kind);
        let (Some(v1), Some(a1)) = (first(TrackKind::Video), first(TrackKind::Audio)) else {
            panic!("tracks");
        };
        // The dissolve's overlap is given back to the cut it is centred on.
        assert_eq!(places(v1), [(0, 48, 24), (48, 48, 30)]);
        assert_eq!(
            v1.clips[1].transition,
            Some(Transition {
                kind: TransitionKind::CrossDissolve,
                frames: 12
            })
        );
        assert_eq!(v1.clips[0].transition, None);
        assert_eq!(
            v1.clips[0].media,
            OtioMedia::File {
                path: "C:/Media/a.mov".into()
            }
        );
        assert_eq!(
            v1.clips[1].media,
            OtioMedia::File {
                path: "/media/b & c.mov".into()
            }
        );
        assert_eq!(v1.clips[0].media_name, "a.mov");
        assert_eq!(v1.clips[0].available, Some(240));
        // The gap on A1 between the music and the sound is kept.
        assert_eq!(places(a1), [(12, 36, 0), (60, 24, 0)]);
        assert_eq!(
            a1.clips[1].media,
            OtioMedia::File {
                path: "C:/Media/a.mov".into()
            },
            "a video's sound points at the same file"
        );
    }

    #[test]
    fn speeds_and_markers_read_back_as_they_were() {
        let mut p = project();
        let v1 = p.tracks[0].id;
        let media = p.tracks[0].clips[0].media;
        (Command::AddClip {
            track: v1,
            media,
            start: 96,
            duration: Some(24),
            with_sound: false,
        })
        .apply(&mut p)
        .unwrap_or_else(|e| panic!("{e}"));
        p.tracks[0].clips[0].speed = 2.0;
        p.tracks[0].clips[1].speed = 0.5;
        p.tracks[0].clips[1].reverse = true;
        p.tracks[0].clips[2].freeze = true;
        p.markers.push(Marker {
            frame: 12,
            name: "Cut".into(),
            color: 3,
        });
        p.markers.push(Marker {
            frame: 40,
            name: "Beat".into(),
            color: 1,
        });
        let written = crate::to_aaf_at(&p, crate::Stamp::from_unix(1_798_000_000))
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(written.retimed, 3, "the three retimed clips are written");
        let t = read_aaf(&written.bytes, p.fps).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(t.skipped, 0);
        let v1 = &t.tracks[0];
        assert_eq!(
            (v1.clips[0].speed, v1.clips[0].reverse, v1.clips[0].freeze),
            (2.0, false, false),
            "a speed change"
        );
        assert_eq!(
            (v1.clips[1].speed, v1.clips[1].reverse, v1.clips[1].freeze),
            (0.5, true, false),
            "a reversed half-speed clip"
        );
        assert_eq!(
            (v1.clips[2].speed, v1.clips[2].reverse, v1.clips[2].freeze),
            (1.0, false, true),
            "a freeze frame"
        );
        // Markers are locators on the composition: their frame and name come back.
        assert_eq!(
            t.markers
                .iter()
                .map(|m| (m.frame, m.name.as_str()))
                .collect::<Vec<_>>(),
            [(12, "Cut"), (40, "Beat")]
        );
    }

    #[test]
    fn other_rates_are_converted_and_other_files_refused() {
        let p = project();
        let written = crate::to_aaf_at(&p, crate::Stamp::from_unix(0)).unwrap_or_else(|e| panic!("{e}"));
        // A 24 fps timeline read into a 48 fps project: twice the frames.
        let t = read_aaf(&written.bytes, Fps { num: 48, den: 1 }).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(places(&t.tracks[0])[0], (0, 96, 48));
        let fps = Fps { num: 24, den: 1 };
        assert!(read_aaf(b"not a compound file", fps).is_err());
        assert!(read_aaf(&[], fps).is_err());
        // A compound file that is not an AAF file.
        let mut cf = CompoundFile::create(Cursor::new(Vec::new())).unwrap_or_else(|e| panic!("{e}"));
        cf.create_stream("/other").unwrap_or_else(|e| panic!("{e}"));
        let bytes = cf.into_inner().into_inner();
        assert!(read_aaf(&bytes, fps).is_err());
    }

    #[test]
    fn a_composition_that_refers_to_itself_stops() -> Result<(), Box<dyn std::error::Error>> {
        use crate::{auid, i64_of, rational};
        // A composition whose one picture track plays a source clip that points back at the
        // composition: reading must stop at the loop, not recurse for ever.
        let mob_id = vec![7_u8; 16];
        let clip = Object {
            class: class::SOURCE_CLIP,
            props: vec![
                (pid::DATA_DEFINITION, Prop::Weak(auid(data::PICTURE))),
                (pid::LENGTH, Prop::Data(i64_of(20))),
                (pid::START_TIME, Prop::Data(i64_of(0))),
                (pid::SOURCE_ID, Prop::Data(mob_id.clone())),
            ],
        };
        let slot = Object {
            class: class::TIMELINE_MOB_SLOT,
            props: vec![
                (pid::SLOT_ID, Prop::Data(1_u32.to_le_bytes().to_vec())),
                (pid::EDIT_RATE, Prop::Data(rational(25, 1))),
                (pid::SEGMENT, Prop::Strong(Box::new(clip))),
            ],
        };
        let composition = Object {
            class: class::COMPOSITION_MOB,
            props: vec![
                (pid::MOB_ID, Prop::Data(mob_id.clone())),
                (pid::SLOTS, Prop::List(vec![slot])),
            ],
        };
        let mut mobs = HashMap::new();
        mobs.insert(mob_id.as_slice(), &composition);
        let timeline = Timeline { mobs, fps: 25.0 };
        let mut visited = HashSet::new();
        let mut skipped = 0;
        let slot = composition
            .list(pid::SLOTS)
            .first()
            .ok_or("the composition has a slot")?;
        let (_, _, track) = timeline
            .track(slot, &mut visited, 0, &mut skipped)
            .ok_or("the track is read")?;
        assert_eq!(skipped, 2, "the loop and the empty composition are counted");
        assert!(visited.is_empty(), "the visited set is cleared");
        // The nested composition is kept as a compound clip, but its own track is empty.
        assert_eq!(track.clips.len(), 1);
        assert!(track.clips[0].nested.iter().all(|t| t.clips.is_empty()));
        Ok(())
    }
}
