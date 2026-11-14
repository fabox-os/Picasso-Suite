//! Writes a Picasso Suite timeline as an AAF file (Advanced Authoring Format), which Avid Media
//! Composer, Pro Tools, Premiere Pro, DaVinci Resolve and others import.
//!
//! Written from the AMWA's published AAF Object Specification (v1.1) and AAF Low-Level Container
//! Specification: an AAF file is a Microsoft compound file whose storages are objects (each
//! storage's class id is the object's class) and whose `properties` streams hold their
//! properties. A strong reference is a storage named after its property; a vector or set of them
//! is an index stream and one storage per element; a weak reference names a definition by its
//! key through the root's `referenced properties` table.
//!
//! What is written is the editorial part of the AAF Edit Protocol:
//!
//! * a composition mob for the timeline, with a timecode slot that starts at 01:00:00:00 and one
//!   slot per track (V1..., A1...), each a sequence of source clips, fillers and dissolves;
//! * for each file the timeline uses, a master mob and a file source mob whose descriptor
//!   (CDCI for pictures, PCM for sound, both under a multiple descriptor for a video with sound)
//!   carries a network locator with the file's address, so the receiving editor links to the
//!   media where it is.
//!
//! Mob ids are made from the file path, so the same file has the same master clip in every
//! export. The meta-dictionary is left empty: the classes used are all in the AAF baseline,
//! which readers know. Clips of generated media (titles, bars, solids) and compound clips have
//! no file and are left out; speed changes and reverse are written as the Edit Protocol's Video
//! Speed Control operation, a freeze as Video Repeat, and timeline markers as locators, while
//! grades and effects stay in Picasso Suite.

use std::collections::BTreeMap;
use std::io::{Cursor, Write};

use cfb::CompoundFile;
use picasso_core::{Clip, MediaId, MediaSource, Project, TrackKind};
use uuid::Uuid;

mod read;
pub use read::read_aaf;

/// The file, and what could not be carried.
pub struct Written {
    pub bytes: Vec<u8>,
    /// Clips left out: generated media and compound clips, which have no file.
    pub skipped: usize,
    /// Clips with a speed change, reverse or freeze, written as time warps.
    pub retimed: usize,
}

// --- Identifiers (AUIDs as GUID text; written in the GUID byte order) ---

const fn id(text: &str) -> Uuid {
    match Uuid::try_parse(text) {
        Ok(u) => u,
        Err(_) => Uuid::nil(),
    }
}

/// The class id of an AAF file's root storage.
const ROOT_CLSID: Uuid = id("b3b398a5-1c90-11d4-8053-080036210804");

/// Classes of the AAF baseline (SMPTE labels 06.0e.2b.34.02.06.01.01.0d.01.01.01.01.01.xx.00).
mod class {
    use super::{id, Uuid};
    pub const META_DICTIONARY: Uuid = id("0d010101-0225-0000-060e-2b3402060101");
    pub const HEADER: Uuid = id("0d010101-0101-2f00-060e-2b3402060101");
    pub const IDENTIFICATION: Uuid = id("0d010101-0101-3000-060e-2b3402060101");
    pub const CONTENT_STORAGE: Uuid = id("0d010101-0101-1800-060e-2b3402060101");
    pub const DICTIONARY: Uuid = id("0d010101-0101-2200-060e-2b3402060101");
    pub const DATA_DEFINITION: Uuid = id("0d010101-0101-1b00-060e-2b3402060101");
    pub const OPERATION_DEFINITION: Uuid = id("0d010101-0101-1c00-060e-2b3402060101");
    pub const COMPOSITION_MOB: Uuid = id("0d010101-0101-3500-060e-2b3402060101");
    pub const MASTER_MOB: Uuid = id("0d010101-0101-3600-060e-2b3402060101");
    pub const SOURCE_MOB: Uuid = id("0d010101-0101-3700-060e-2b3402060101");
    pub const TIMELINE_MOB_SLOT: Uuid = id("0d010101-0101-3b00-060e-2b3402060101");
    pub const SEQUENCE: Uuid = id("0d010101-0101-0f00-060e-2b3402060101");
    pub const FILLER: Uuid = id("0d010101-0101-0900-060e-2b3402060101");
    pub const SOURCE_CLIP: Uuid = id("0d010101-0101-1100-060e-2b3402060101");
    pub const TRANSITION: Uuid = id("0d010101-0101-1700-060e-2b3402060101");
    pub const OPERATION_GROUP: Uuid = id("0d010101-0101-0a00-060e-2b3402060101");
    pub const CONSTANT_VALUE: Uuid = id("0d010101-0101-3d00-060e-2b3402060101");
    pub const PARAMETER_DEFINITION: Uuid = id("0d010101-0101-1d00-060e-2b3402060101");
    pub const TIMECODE: Uuid = id("0d010101-0101-1400-060e-2b3402060101");
    pub const CDCI_DESCRIPTOR: Uuid = id("0d010101-0101-2800-060e-2b3402060101");
    pub const PCM_DESCRIPTOR: Uuid = id("0d010101-0101-4800-060e-2b3402060101");
    pub const MULTIPLE_DESCRIPTOR: Uuid = id("0d010101-0101-4400-060e-2b3402060101");
    pub const NETWORK_LOCATOR: Uuid = id("0d010101-0101-3200-060e-2b3402060101");
}

/// Data definitions: what kind of essence a component carries.
mod data {
    use super::{id, Uuid};
    pub const PICTURE: Uuid = id("01030202-0100-0000-060e-2b3404010101");
    pub const SOUND: Uuid = id("01030202-0200-0000-060e-2b3404010101");
    pub const TIMECODE: Uuid = id("01030201-0100-0000-060e-2b3404010101");
}

const VIDEO_DISSOLVE: Uuid = id("0c3bea40-fc05-11d2-8a29-0050040ef7d2");
const AUDIO_DISSOLVE: Uuid = id("0c3bea41-fc05-11d2-8a29-0050040ef7d2");
/// The AAF Edit Protocol operational pattern.
const EDIT_PROTOCOL: Uuid = id("0d011201-0100-0000-060e-2b3404010105");
/// This program, in the identification list.
const PRODUCT_ID: Uuid = id("5e0d6c3a-8f2b-4f4e-9a51-7c1d2e3f4a5b");
/// A timeline marker (locator): position and name on the composition mob.
const LOCATOR_CLASS: Uuid = id("0d010101-0101-2b00-060e-2b3402060101");

/// Property ids of the AAF baseline.
mod pid {
    pub const META_DICTIONARY: u16 = 0x0001;
    pub const HEADER: u16 = 0x0002;
    pub const BYTE_ORDER: u16 = 0x3b01;
    pub const LAST_MODIFIED: u16 = 0x3b02;
    pub const CONTENT: u16 = 0x3b03;
    pub const DICTIONARY: u16 = 0x3b04;
    pub const VERSION: u16 = 0x3b05;
    pub const IDENTIFICATION_LIST: u16 = 0x3b06;
    pub const OBJECT_MODEL_VERSION: u16 = 0x3b07;
    pub const OPERATIONAL_PATTERN: u16 = 0x3b09;
    pub const COMPANY_NAME: u16 = 0x3c01;
    pub const PRODUCT_NAME: u16 = 0x3c02;
    pub const PRODUCT_VERSION_STRING: u16 = 0x3c04;
    pub const PRODUCT_ID: u16 = 0x3c05;
    pub const DATE: u16 = 0x3c06;
    pub const PLATFORM: u16 = 0x3c08;
    pub const GENERATION_AUID: u16 = 0x3c09;
    pub const MOBS: u16 = 0x1901;
    pub const OPERATION_DEFINITIONS: u16 = 0x2603;
    pub const PARAMETER_DEFINITIONS: u16 = 0x2604;
    pub const DATA_DEFINITIONS: u16 = 0x2605;
    pub const TYPE_DEFINITIONS: u16 = 0x0004;
    pub const IDENTIFICATION: u16 = 0x1b01;
    pub const NAME: u16 = 0x1b02;
    pub const DESCRIPTION: u16 = 0x1b03;
    pub const OP_DATA_DEFINITION: u16 = 0x1e01;
    pub const IS_TIME_WARP: u16 = 0x1e02;
    pub const NUMBER_INPUTS: u16 = 0x1e07;
    pub const BYPASS: u16 = 0x1e08;
    pub const PARAMETER_TYPE: u16 = 0x1f01;
    pub const MOB_ID: u16 = 0x4401;
    pub const MOB_NAME: u16 = 0x4402;
    pub const SLOTS: u16 = 0x4403;
    pub const MOB_LAST_MODIFIED: u16 = 0x4404;
    pub const CREATION_TIME: u16 = 0x4405;
    pub const ESSENCE_DESCRIPTION: u16 = 0x4701;
    pub const SLOT_ID: u16 = 0x4801;
    pub const SLOT_NAME: u16 = 0x4802;
    pub const SEGMENT: u16 = 0x4803;
    pub const PHYSICAL_TRACK_NUMBER: u16 = 0x4804;
    pub const EDIT_RATE: u16 = 0x4b01;
    pub const ORIGIN: u16 = 0x4b02;
    pub const DATA_DEFINITION: u16 = 0x0201;
    pub const LENGTH: u16 = 0x0202;
    pub const COMPONENTS: u16 = 0x1001;
    pub const SOURCE_ID: u16 = 0x1101;
    pub const SOURCE_MOB_SLOT_ID: u16 = 0x1102;
    pub const START_TIME: u16 = 0x1201;
    pub const OPERATION_GROUP: u16 = 0x1801;
    pub const CUT_POINT: u16 = 0x1802;
    pub const OPERATION: u16 = 0x0b01;
    pub const INPUT_SEGMENTS: u16 = 0x0b02;
    pub const PARAMETERS: u16 = 0x0b03;
    pub const TC_START: u16 = 0x1501;
    pub const TC_FPS: u16 = 0x1502;
    pub const TC_DROP: u16 = 0x1503;
    pub const LOCATOR: u16 = 0x2f01;
    pub const SAMPLE_RATE: u16 = 0x3001;
    pub const FILE_LENGTH: u16 = 0x3002;
    pub const STORED_HEIGHT: u16 = 0x3202;
    pub const STORED_WIDTH: u16 = 0x3203;
    pub const FRAME_LAYOUT: u16 = 0x320c;
    pub const VIDEO_LINE_MAP: u16 = 0x320d;
    pub const IMAGE_ASPECT_RATIO: u16 = 0x320e;
    pub const COMPONENT_WIDTH: u16 = 0x3301;
    pub const HORIZONTAL_SUBSAMPLING: u16 = 0x3302;
    pub const VERTICAL_SUBSAMPLING: u16 = 0x3308;
    pub const QUANTIZATION_BITS: u16 = 0x3d01;
    pub const AUDIO_SAMPLING_RATE: u16 = 0x3d03;
    pub const CHANNELS: u16 = 0x3d07;
    pub const AVERAGE_BPS: u16 = 0x3d09;
    pub const BLOCK_ALIGN: u16 = 0x3d0a;
    pub const FILE_DESCRIPTORS: u16 = 0x3f01;
    pub const URL_STRING: u16 = 0x4001;
    pub const PARAMETER_DEFINITION: u16 = 0x4c01;
    pub const PARAMETER_VALUE: u16 = 0x4d01;
}

/// Tags of the `referenced properties` table: where weak references point.
const TAG_DATA_DEFINITIONS: u16 = 0;
const TAG_OPERATION_DEFINITIONS: u16 = 1;
const TAG_PARAMETER_DEFINITIONS: u16 = 2;
const TAG_TYPE_DEFINITIONS: u16 = 3;

// --- The object model being written ---

enum Value {
    Data(Vec<u8>),
    Strong(Box<Obj>),
    Vector(Vec<Obj>),
    /// A set of objects, each found by the value of its `key_pid` property.
    Set {
        key_pid: u16,
        items: Vec<(Vec<u8>, Obj)>,
    },
    Weak {
        tag: u16,
        key_pid: u16,
        key: Vec<u8>,
    },
}

struct Prop {
    pid: u16,
    name: &'static str,
    value: Value,
}

struct Obj {
    class: Uuid,
    props: Vec<Prop>,
}

impl Obj {
    fn new(class: Uuid) -> Self {
        Self {
            class,
            props: Vec::new(),
        }
    }

    fn with(mut self, pid: u16, name: &'static str, value: Value) -> Self {
        self.props.push(Prop { pid, name, value });
        self
    }

    fn data(self, pid: u16, name: &'static str, bytes: Vec<u8>) -> Self {
        self.with(pid, name, Value::Data(bytes))
    }
}

// --- Value encodings (little-endian, as the byte order says) ---

fn text(s: &str) -> Vec<u8> {
    s.encode_utf16()
        .chain(std::iter::once(0))
        .flat_map(u16::to_le_bytes)
        .collect()
}

fn auid(u: Uuid) -> Vec<u8> {
    u.to_bytes_le().to_vec()
}

fn rational(num: u32, den: u32) -> Vec<u8> {
    let (n, d) = (
        i32::try_from(num).unwrap_or(i32::MAX),
        i32::try_from(den).unwrap_or(1),
    );
    [n.to_le_bytes(), d.to_le_bytes()].concat()
}

fn i64_of(v: u64) -> Vec<u8> {
    i64::try_from(v).unwrap_or(i64::MAX).to_le_bytes().to_vec()
}

/// A SMPTE UMID (the mob id): the AAF label, then the material number.
fn mob_id(material: Uuid) -> Vec<u8> {
    let mut out = vec![
        0x06, 0x0a, 0x2b, 0x34, 0x01, 0x01, 0x01, 0x05, 0x01, 0x01, 0x0f, 0x20, 0x13, 0, 0, 0,
    ];
    out.extend(auid(material));
    out
}

/// The id no mob has: the end of a chain of source references.
fn null_mob_id() -> Vec<u8> {
    vec![0; 32]
}

/// A time of day: year, month, day, hour, minute, second, quarter-hundredths.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
    pub year: i16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

impl Stamp {
    fn bytes(self) -> Vec<u8> {
        let mut out = self.year.to_le_bytes().to_vec();
        out.extend([self.month, self.day, self.hour, self.minute, self.second, 0]);
        out
    }

    /// The time from seconds since 1970 (UTC), by the civil calendar.
    pub fn from_unix(secs: u64) -> Self {
        let days = i64::try_from(secs / 86_400).unwrap_or(0);
        let rem = secs % 86_400;
        // Days to a date, as the proleptic Gregorian calendar counts them.
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = doy - (153 * mp + 2) / 5 + 1;
        let month = if mp < 10 { mp + 3 } else { mp - 9 };
        let year = yoe + era * 400 + i64::from(month <= 2);
        let small = |v: u64| u8::try_from(v).unwrap_or(0);
        Self {
            year: i16::try_from(year).unwrap_or(1970),
            month: u8::try_from(month).unwrap_or(1),
            day: u8::try_from(day).unwrap_or(1),
            hour: small(rem / 3600),
            minute: small(rem / 60 % 60),
            second: small(rem % 60),
        }
    }

    /// Now, where the clock can be read (not in the browser).
    pub fn now() -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        if let Ok(d) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
            return Self::from_unix(d.as_secs());
        }
        Self::from_unix(1_767_225_600) // 2026-01-01
    }
}

/// A version-4-shaped id made from `parts` (FNV-1a, two ways), so the same input gives the
/// same id: a file keeps its master clip from one export to the next.
fn derived_id(parts: &[&[u8]]) -> Uuid {
    let mut halves = [0xcbf2_9ce4_8422_2325_u64, 0x8422_2325_cbf2_9ce4_u64];
    for (i, h) in halves.iter_mut().enumerate() {
        for part in parts {
            for b in part.iter().chain([0xff_u8, i as u8].iter()) {
                *h ^= u64::from(*b);
                *h = h.wrapping_mul(0x0100_0000_01b3);
            }
        }
    }
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&halves[0].to_be_bytes());
    bytes[8..].copy_from_slice(&halves[1].to_be_bytes());
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}

// --- Writing objects into the compound file ---

/// Stored forms of the property index.
const SF_DATA: u16 = 0x82;
const SF_STRONG: u16 = 0x22;
const SF_STRONG_VECTOR: u16 = 0x32;
const SF_STRONG_SET: u16 = 0x3a;
const SF_WEAK: u16 = 0x02;

/// The storage name of property `name` (`Mobs-1901`). A collection's elements add `{n}`, and
/// compound file names hold 31 characters, so a long collection name keeps its first 8 and last
/// 12 characters.
fn storage_name(name: &str, pid: u16, collection: bool) -> String {
    let full = format!("{name}-{pid:x}");
    let limit = if collection { 21 } else { 31 };
    let chars: Vec<char> = full.chars().collect();
    if chars.len() <= limit {
        return full;
    }
    let head: String = chars[..8].iter().collect();
    let tail: String = chars[chars.len() - 12..].iter().collect();
    format!("{head}-{tail}")
}

type File = CompoundFile<Cursor<Vec<u8>>>;

fn join(parent: &str, name: &str) -> String {
    if parent == "/" {
        format!("/{name}")
    } else {
        format!("{parent}/{name}")
    }
}

fn write_stream(cf: &mut File, path: &str, bytes: &[u8]) -> std::io::Result<()> {
    let mut s = cf.create_stream(path)?;
    s.write_all(bytes)
}

fn too_long() -> std::io::Error {
    std::io::Error::other("a property value is too long for AAF")
}

/// Writes `obj` into the storage at `path` (which exists): its class, its properties, and the
/// storages of the objects it holds.
fn write_obj(cf: &mut File, path: &str, obj: &Obj) -> std::io::Result<()> {
    cf.set_storage_clsid(path, obj.class)?;
    let mut entries: Vec<u8> = Vec::new();
    let mut values: Vec<u8> = Vec::new();
    for p in &obj.props {
        let (form, bytes) = match &p.value {
            Value::Data(b) => (SF_DATA, b.clone()),
            Value::Strong(child) => {
                let name = storage_name(p.name, p.pid, false);
                let at = join(path, &name);
                cf.create_storage(&at)?;
                write_obj(cf, &at, child)?;
                (SF_STRONG, text(&name))
            }
            Value::Vector(items) => {
                let name = storage_name(p.name, p.pid, true);
                let n = u32::try_from(items.len()).map_err(|_| too_long())?;
                let mut index = Vec::new();
                for v in [n, n, u32::MAX] {
                    index.extend(v.to_le_bytes());
                }
                for (i, item) in items.iter().enumerate() {
                    let at = join(path, &format!("{name}{{{i:x}}}"));
                    cf.create_storage(&at)?;
                    write_obj(cf, &at, item)?;
                    index.extend(u32::try_from(i).map_err(|_| too_long())?.to_le_bytes());
                }
                write_stream(cf, &join(path, &format!("{name} index")), &index)?;
                (SF_STRONG_VECTOR, text(&name))
            }
            Value::Set { key_pid, items } => {
                let name = storage_name(p.name, p.pid, true);
                let n = u32::try_from(items.len()).map_err(|_| too_long())?;
                let key_size = items.first().map_or(16, |(k, _)| k.len());
                let mut index = Vec::new();
                for v in [n, n, u32::MAX] {
                    index.extend(v.to_le_bytes());
                }
                index.extend(key_pid.to_le_bytes());
                index.push(u8::try_from(key_size).map_err(|_| too_long())?);
                for (i, (key, item)) in items.iter().enumerate() {
                    let at = join(path, &format!("{name}{{{i:x}}}"));
                    cf.create_storage(&at)?;
                    write_obj(cf, &at, item)?;
                    index.extend(u32::try_from(i).map_err(|_| too_long())?.to_le_bytes());
                    index.extend(1u32.to_le_bytes());
                    index.extend(key);
                }
                write_stream(cf, &join(path, &format!("{name} index")), &index)?;
                (SF_STRONG_SET, text(&name))
            }
            Value::Weak { tag, key_pid, key } => {
                let mut b = tag.to_le_bytes().to_vec();
                b.extend(key_pid.to_le_bytes());
                b.push(u8::try_from(key.len()).map_err(|_| too_long())?);
                b.extend(key);
                (SF_WEAK, b)
            }
        };
        entries.extend(p.pid.to_le_bytes());
        entries.extend(form.to_le_bytes());
        entries.extend(
            u16::try_from(bytes.len())
                .map_err(|_| too_long())?
                .to_le_bytes(),
        );
        values.extend(bytes);
    }
    let count = u16::try_from(obj.props.len()).map_err(|_| too_long())?;
    let mut stream = vec![0x4c, 0x20];
    stream.extend(count.to_le_bytes());
    stream.extend(entries);
    stream.extend(values);
    write_stream(cf, &join(path, "properties"), &stream)
}

/// The root's table of where weak references point: Header > Dictionary > DataDefinitions,
/// OperationDefinitions and ParameterDefinitions, and Header > MetaDictionary > TypeDefinitions.
fn referenced_properties() -> Vec<u8> {
    let paths: [&[u16]; 4] = [
        &[pid::HEADER, pid::DICTIONARY, pid::DATA_DEFINITIONS],
        &[pid::HEADER, pid::DICTIONARY, pid::OPERATION_DEFINITIONS],
        &[pid::HEADER, pid::DICTIONARY, pid::PARAMETER_DEFINITIONS],
        &[pid::HEADER, pid::META_DICTIONARY, pid::TYPE_DEFINITIONS],
    ];
    let total: usize = paths.iter().map(|p| p.len() + 1).sum();
    let mut out = vec![0x4c];
    out.extend(u16::try_from(paths.len()).unwrap_or(0).to_le_bytes());
    out.extend(u32::try_from(total).unwrap_or(0).to_le_bytes());
    for path in paths {
        for p in path {
            out.extend(p.to_le_bytes());
        }
        out.extend(0u16.to_le_bytes());
    }
    out
}

// --- Building the objects of a timeline ---

fn data_def(kind: Uuid) -> Value {
    Value::Weak {
        tag: TAG_DATA_DEFINITIONS,
        key_pid: pid::IDENTIFICATION,
        key: auid(kind),
    }
}

fn definition(class: Uuid, ident: Uuid, name: &str, description: &str) -> Obj {
    Obj::new(class)
        .data(pid::IDENTIFICATION, "Identification", auid(ident))
        .data(pid::NAME, "Name", text(name))
        .data(pid::DESCRIPTION, "Description", text(description))
}

fn dictionary() -> Obj {
    let defs = [
        (data::PICTURE, "Picture", "Picture data"),
        (data::SOUND, "Sound", "Sound data"),
        (data::TIMECODE, "Timecode", "Timecode data"),
    ];
    let data_defs = defs
        .iter()
        .map(|(u, n, d)| (auid(*u), definition(class::DATA_DEFINITION, *u, n, d)))
        .collect();
    // (identification, name, data definition, is a time warp, inputs)
    let ops = [
        (VIDEO_DISSOLVE, "Video Dissolve", data::PICTURE, false, 2u32),
        (AUDIO_DISSOLVE, "Mono Audio Dissolve", data::SOUND, false, 2u32),
        (
            VIDEO_SPEED_CONTROL,
            "Video Speed Control",
            data::PICTURE,
            true,
            1u32,
        ),
        (VIDEO_REPEAT, "Video Repeat", data::PICTURE, true, 1u32),
    ];
    let op_defs = ops
        .iter()
        .map(|(u, n, kind, warp, inputs)| {
            let obj = definition(class::OPERATION_DEFINITION, *u, n, n)
                .with(pid::OP_DATA_DEFINITION, "DataDefinition", data_def(*kind))
                .data(
                    pid::IS_TIME_WARP,
                    "IsTimeWarp",
                    vec![u8::from(*warp)],
                )
                .data(
                    pid::NUMBER_INPUTS,
                    "NumberInputs",
                    (*inputs).to_le_bytes().to_vec(),
                )
                .data(pid::BYPASS, "Bypass", 1u32.to_le_bytes().to_vec());
            (auid(*u), obj)
        })
        .collect();
    // The Speed Ratio parameter, a rational: the output length over the input length.
    let speed_ratio = definition(
        class::PARAMETER_DEFINITION,
        SPEED_RATIO,
        "Speed Ratio",
        "Speed Ratio",
    )
    .with(
        pid::PARAMETER_TYPE,
        "Type",
        Value::Weak {
            tag: TAG_TYPE_DEFINITIONS,
            key_pid: pid::IDENTIFICATION,
            key: auid(TYPE_RATIONAL),
        },
    );
    Obj::new(class::DICTIONARY)
        .with(
            pid::OPERATION_DEFINITIONS,
            "OperationDefinitions",
            Value::Set {
                key_pid: pid::IDENTIFICATION,
                items: op_defs,
            },
        )
        .with(
            pid::PARAMETER_DEFINITIONS,
            "ParameterDefinitions",
            Value::Set {
                key_pid: pid::IDENTIFICATION,
                items: vec![(auid(SPEED_RATIO), speed_ratio)],
            },
        )
        .with(
            pid::DATA_DEFINITIONS,
            "DataDefinitions",
            Value::Set {
                key_pid: pid::IDENTIFICATION,
                items: data_defs,
            },
        )
}

/// A mob with its slots.
fn mob(class: Uuid, id: &[u8], name: &str, slots: Vec<Obj>, now: Stamp) -> Obj {
    Obj::new(class)
        .data(pid::MOB_ID, "MobID", id.to_vec())
        .data(pid::MOB_NAME, "Name", text(name))
        .data(pid::CREATION_TIME, "CreationTime", now.bytes())
        .data(pid::MOB_LAST_MODIFIED, "LastModified", now.bytes())
        .with(pid::SLOTS, "Slots", Value::Vector(slots))
}

fn slot(id: u32, name: &str, rate: (u32, u32), track: Option<u32>, segment: Obj) -> Obj {
    let mut obj = Obj::new(class::TIMELINE_MOB_SLOT)
        .data(pid::SLOT_ID, "SlotID", id.to_le_bytes().to_vec())
        .data(pid::SLOT_NAME, "SlotName", text(name))
        .data(pid::EDIT_RATE, "EditRate", rational(rate.0, rate.1))
        .data(pid::ORIGIN, "Origin", i64_of(0));
    if let Some(n) = track {
        obj = obj.data(
            pid::PHYSICAL_TRACK_NUMBER,
            "PhysicalTrackNumber",
            n.to_le_bytes().to_vec(),
        );
    }
    obj.with(pid::SEGMENT, "Segment", Value::Strong(Box::new(segment)))
}

fn source_clip(kind: Uuid, length: u64, start: u64, source: &[u8], source_slot: u32) -> Obj {
    Obj::new(class::SOURCE_CLIP)
        .with(pid::DATA_DEFINITION, "DataDefinition", data_def(kind))
        .data(pid::LENGTH, "Length", i64_of(length))
        .data(pid::START_TIME, "StartTime", i64_of(start))
        .data(pid::SOURCE_ID, "SourceID", source.to_vec())
        .data(
            pid::SOURCE_MOB_SLOT_ID,
            "SourceMobSlotID",
            source_slot.to_le_bytes().to_vec(),
        )
}

/// A clip that plays faster or slower, backwards, or holds one frame: the source clip inside an
/// operation group, as the AAF Edit Protocol lays a time warp out. The group's length is the
/// clip's length on the timeline and the source clip's the frames it takes from its source; a
/// speed change carries the Speed Ratio parameter (output length over input length, negative
/// when reversed) and a freeze the Video Repeat operation.
#[allow(clippy::too_many_arguments)]
fn time_warped(
    kind: Uuid,
    length: u64,
    start: u64,
    source: &[u8],
    source_slot: u32,
    speed: f32,
    reverse: bool,
    freeze: bool,
) -> Obj {
    let speed = if speed.is_finite() {
        speed.clamp(picasso_core::MIN_SPEED, picasso_core::MAX_SPEED)
    } else {
        1.0
    };
    // The frames of the source the clip uses: a freeze holds one.
    let input = if freeze {
        1
    } else {
        (length as f64 * f64::from(speed)).ceil().max(1.0) as u64
    };
    let mut group = Obj::new(class::OPERATION_GROUP)
        .with(pid::DATA_DEFINITION, "DataDefinition", data_def(kind))
        .data(pid::LENGTH, "Length", i64_of(length))
        .with(
            pid::OPERATION,
            "Operation",
            Value::Weak {
                tag: TAG_OPERATION_DEFINITIONS,
                key_pid: pid::IDENTIFICATION,
                key: auid(if freeze { VIDEO_REPEAT } else { VIDEO_SPEED_CONTROL }),
            },
        )
        .with(
            pid::INPUT_SEGMENTS,
            "InputSegments",
            Value::Vector(vec![source_clip(kind, input, start, source, source_slot)]),
        );
    if !freeze {
        let ratio = f64::from(speed).recip() * if reverse { -1.0 } else { 1.0 };
        let parameter = Obj::new(class::CONSTANT_VALUE)
            .with(
                pid::PARAMETER_DEFINITION,
                "Definition",
                Value::Weak {
                    tag: TAG_PARAMETER_DEFINITIONS,
                    key_pid: pid::IDENTIFICATION,
                    key: auid(SPEED_RATIO),
                },
            )
            .data(pid::PARAMETER_VALUE, "Value", speed_ratio(ratio));
        group = group.with(
            pid::PARAMETERS,
            "Parameters",
            Value::Vector(vec![parameter]),
        );
    }
    group
}

/// A speed ratio as the rational the parameter takes: the numerator and denominator reduced.
fn speed_ratio(ratio: f64) -> Vec<u8> {
    // Enough places that any speed a clip may have comes back as it was.
    const DEN: i64 = 1_000_000;
    let scaled = (ratio * DEN as f64).round();
    let num = if scaled.is_finite() {
        scaled.clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i64
    } else {
        DEN
    };
    let g = gcd(num.unsigned_abs(), DEN as u64);
    let (n, d) = ((num / g as i64) as i32, (DEN / g as i64) as i32);
    [n.to_le_bytes(), d.to_le_bytes()].concat()
}

/// The greatest divisor of two numbers, never zero.
fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        let t = a % b;
        a = b;
        b = t;
    }
    a.max(1)
}

/// The Video Speed Control operation: a clip played faster, slower or backwards.
const VIDEO_SPEED_CONTROL: Uuid = id("9d2ea890-0968-11d3-8a38-0050040ef7d2");
/// The Video Repeat operation: a clip that holds one frame.
const VIDEO_REPEAT: Uuid = id("9d2ea891-0968-11d3-8a38-0050040ef7d2");
/// The Speed Ratio parameter definition: a rational, output length over input length.
const SPEED_RATIO: Uuid = id("72559a80-24d7-11d3-8a50-0050040ef7d2");
/// TypeID_Rational: two 32-bit integers, numerator then denominator.
const TYPE_RATIONAL: Uuid = id("03010100-0000-0000-060e-2b3401040101");

fn filler(kind: Uuid, length: u64) -> Obj {
    Obj::new(class::FILLER)
        .with(pid::DATA_DEFINITION, "DataDefinition", data_def(kind))
        .data(pid::LENGTH, "Length", i64_of(length))
}

fn transition(kind: Uuid, length: u64) -> Obj {
    let operation = if kind == data::SOUND {
        AUDIO_DISSOLVE
    } else {
        VIDEO_DISSOLVE
    };
    let group = Obj::new(class::OPERATION_GROUP)
        .with(pid::DATA_DEFINITION, "DataDefinition", data_def(kind))
        .data(pid::LENGTH, "Length", i64_of(length))
        .with(
            pid::OPERATION,
            "Operation",
            Value::Weak {
                tag: TAG_OPERATION_DEFINITIONS,
                key_pid: pid::IDENTIFICATION,
                key: auid(operation),
            },
        );
    Obj::new(class::TRANSITION)
        .with(pid::DATA_DEFINITION, "DataDefinition", data_def(kind))
        .data(pid::LENGTH, "Length", i64_of(length))
        .data(pid::CUT_POINT, "CutPoint", i64_of(length / 2))
        .with(
            pid::OPERATION_GROUP,
            "OperationGroup",
            Value::Strong(Box::new(group)),
        )
}

fn locator(url: &str) -> Value {
    Value::Vector(vec![Obj::new(class::NETWORK_LOCATOR).data(
        pid::URL_STRING,
        "URLString",
        text(url),
    )])
}

/// How a file of the timeline is used: its picture, its sound, or both.
struct FileUse {
    path: String,
    name: String,
    picture: bool,
    sound: bool,
    /// Frames of the file (its length, or as far as the clips reach).
    length: u64,
}

impl FileUse {
    /// The master mob's slots: the picture is slot 1, the sound slot 1 or 2.
    fn picture_slot(&self) -> u32 {
        1
    }

    fn sound_slot(&self) -> u32 {
        if self.picture {
            2
        } else {
            1
        }
    }

    fn master_id(&self) -> Vec<u8> {
        mob_id(derived_id(&[b"master", self.path.as_bytes()]))
    }

    fn file_id(&self) -> Vec<u8> {
        mob_id(derived_id(&[b"file", self.path.as_bytes()]))
    }
}

fn descriptor(project: &Project, f: &FileUse, url: &str) -> Obj {
    let rate = (project.fps.num, project.fps.den);
    let fps = project.fps.as_f64().max(1.0);
    let samples = (f.length as f64 * 48_000.0 / fps).round() as u64;
    let picture = || {
        Obj::new(class::CDCI_DESCRIPTOR)
            .data(pid::SAMPLE_RATE, "SampleRate", rational(rate.0, rate.1))
            .data(pid::FILE_LENGTH, "Length", i64_of(f.length))
            .data(
                pid::STORED_HEIGHT,
                "StoredHeight",
                project.height.to_le_bytes().to_vec(),
            )
            .data(
                pid::STORED_WIDTH,
                "StoredWidth",
                project.width.to_le_bytes().to_vec(),
            )
            .data(pid::FRAME_LAYOUT, "FrameLayout", vec![0]) // full frame
            .data(
                pid::VIDEO_LINE_MAP,
                "VideoLineMap",
                [0i32.to_le_bytes(), 0i32.to_le_bytes()].concat(),
            )
            .data(
                pid::IMAGE_ASPECT_RATIO,
                "ImageAspectRatio",
                rational(project.width, project.height.max(1)),
            )
            .data(
                pid::COMPONENT_WIDTH,
                "ComponentWidth",
                8u32.to_le_bytes().to_vec(),
            )
            .data(
                pid::HORIZONTAL_SUBSAMPLING,
                "HorizontalSubsampling",
                2u32.to_le_bytes().to_vec(),
            )
            .data(
                pid::VERTICAL_SUBSAMPLING,
                "VerticalSubsampling",
                2u32.to_le_bytes().to_vec(),
            )
            .with(pid::LOCATOR, "Locator", locator(url))
    };
    let sound = || {
        Obj::new(class::PCM_DESCRIPTOR)
            .data(pid::SAMPLE_RATE, "SampleRate", rational(48_000, 1))
            .data(pid::FILE_LENGTH, "Length", i64_of(samples))
            .data(
                pid::AUDIO_SAMPLING_RATE,
                "AudioSamplingRate",
                rational(48_000, 1),
            )
            .data(pid::CHANNELS, "Channels", 2u32.to_le_bytes().to_vec())
            .data(
                pid::QUANTIZATION_BITS,
                "QuantizationBits",
                16u32.to_le_bytes().to_vec(),
            )
            .data(pid::BLOCK_ALIGN, "BlockAlign", 4u16.to_le_bytes().to_vec())
            .data(
                pid::AVERAGE_BPS,
                "AverageBPS",
                192_000u32.to_le_bytes().to_vec(),
            )
            .with(pid::LOCATOR, "Locator", locator(url))
    };
    match (f.picture, f.sound) {
        (true, true) => Obj::new(class::MULTIPLE_DESCRIPTOR)
            .data(pid::SAMPLE_RATE, "SampleRate", rational(rate.0, rate.1))
            .data(pid::FILE_LENGTH, "Length", i64_of(f.length))
            .with(pid::LOCATOR, "Locator", locator(url))
            .with(
                pid::FILE_DESCRIPTORS,
                "FileDescriptors",
                Value::Vector(vec![picture(), sound()]),
            ),
        (false, true) => sound(),
        _ => picture(),
    }
}

/// The master mob and file source mob of a file.
fn media_mobs(project: &Project, f: &FileUse, now: Stamp) -> [Obj; 2] {
    let rate = (project.fps.num, project.fps.den);
    let url = picasso_core::otio::file_url(&f.path);
    let mut kinds = Vec::new();
    if f.picture {
        kinds.push((data::PICTURE, f.picture_slot()));
    }
    if f.sound {
        kinds.push((data::SOUND, f.sound_slot()));
    }
    let file_slots = kinds
        .iter()
        .map(|(kind, n)| {
            slot(
                *n,
                "",
                rate,
                Some(*n),
                source_clip(*kind, f.length, 0, &null_mob_id(), 0),
            )
        })
        .collect();
    let master_slots = kinds
        .iter()
        .map(|(kind, n)| {
            slot(
                *n,
                "",
                rate,
                Some(*n),
                source_clip(*kind, f.length, 0, &f.file_id(), *n),
            )
        })
        .collect();
    let file = mob(class::SOURCE_MOB, &f.file_id(), &f.name, file_slots, now).with(
        pid::ESSENCE_DESCRIPTION,
        "EssenceDescription",
        Value::Strong(Box::new(descriptor(project, f, &url))),
    );
    let master = mob(
        class::MASTER_MOB,
        &f.master_id(),
        &f.name,
        master_slots,
        now,
    );
    [master, file]
}

/// A piece of a composition track before it is written.
enum Piece {
    Clip {
        media: MediaId,
        start: u64,
        length: u64,
        speed: f32,
        reverse: bool,
        freeze: bool,
    },
    Gap(u64),
    Dissolve(u64),
}

/// The pieces of a track: clips of files, gaps (and clips without a file), and dissolves
/// centred on cuts where both clips have the media frames the dissolve needs.
fn pieces(
    track: &picasso_core::Track,
    files: &BTreeMap<MediaId, FileUse>,
    skipped: &mut usize,
    retimed: &mut usize,
) -> Vec<Piece> {
    let mut clips: Vec<&Clip> = track.clips.iter().collect();
    clips.sort_by_key(|c| c.start);
    let mut out: Vec<Piece> = Vec::new();
    let mut cursor = 0;
    let mut last: Option<&Clip> = None;
    for clip in clips {
        if !files.contains_key(&clip.media) {
            *skipped += 1;
            continue;
        }
        if clip.start > cursor {
            out.push(Piece::Gap(clip.start - cursor));
        }
        if (clip.speed - 1.0).abs() > 1e-4 || clip.reverse || clip.freeze {
            *retimed += 1;
        }
        let mut start = clip.source_in;
        let mut length = clip.duration;
        // A dissolve takes half its frames from before the cut and half from after it: the
        // clip before runs on past the cut and this one starts earlier, by those halves.
        let dissolve = last
            .filter(|l| l.end() == clip.start)
            .and_then(|l| picasso_core::transition_frames(l, clip));
        if let (
            Some(frames),
            Some(Piece::Clip {
                length: prev_len,
                start: prev_start,
                media,
                speed: _,
                reverse: _,
                freeze: _,
            }),
        ) = (dissolve, out.last_mut())
        {
            let (before, after) = (frames / 2, frames - frames / 2);
            let prev_room = files
                .get(media)
                .is_some_and(|f| *prev_start + *prev_len + after <= f.length);
            if before <= start && prev_room && after <= length {
                *prev_len += after;
                start -= before;
                length += before;
                out.push(Piece::Dissolve(frames));
            }
        }
        out.push(Piece::Clip {
            media: clip.media,
            start,
            length,
            speed: clip.speed,
            reverse: clip.reverse,
            freeze: clip.freeze,
        });
        cursor = cursor.max(clip.end());
        last = Some(clip);
    }
    out
}

/// The files the timeline's clips use, with what of them is used.
fn files_used(project: &Project) -> BTreeMap<MediaId, FileUse> {
    let mut files: BTreeMap<MediaId, FileUse> = BTreeMap::new();
    for track in &project.tracks {
        for clip in &track.clips {
            let Some(media) = project.media(clip.media) else {
                continue;
            };
            let path = match &media.source {
                MediaSource::Video { path }
                | MediaSource::Audio { path }
                | MediaSource::Image { path } => path,
                _ => continue,
            };
            let audio_file = matches!(media.source, MediaSource::Audio { .. });
            let reach = clip.source_in + clip.source_span() + clip.duration;
            let f = files.entry(clip.media).or_insert_with(|| FileUse {
                path: path.clone(),
                name: media.name.clone(),
                picture: false,
                sound: false,
                length: media.length.unwrap_or(0),
            });
            if track.kind == TrackKind::Audio || audio_file {
                f.sound = true;
            } else {
                f.picture = true;
            }
            if media.length.is_none() {
                f.length = f.length.max(reach);
            }
        }
    }
    files
}

/// The meta-dictionary: empty sets of class and type definitions. Every class written is in
/// the AAF baseline, which readers know without it.
fn meta_dictionary() -> Obj {
    // Meta-definitions are found by their identification (pid 0x0005).
    let empty = || Value::Set {
        key_pid: 0x0005,
        items: Vec::new(),
    };
    Obj::new(class::META_DICTIONARY)
        .with(0x0003, "ClassDefinitions", empty())
        .with(0x0004, "TypeDefinitions", empty())
}

/// The timeline of `project` as an AAF file, stamped with the time `now`.
pub fn to_aaf_at(project: &Project, now: Stamp) -> std::io::Result<Written> {
    let rate = (project.fps.num, project.fps.den);
    let files = files_used(project);
    let (mut skipped, mut retimed) = (0, 0);
    let mut slots = Vec::new();
    // The timecode slot: the timeline starts at 01:00:00:00.
    let nominal = project.fps.nominal();
    let total = project.duration().max(1);
    let timecode = Obj::new(class::TIMECODE)
        .with(
            pid::DATA_DEFINITION,
            "DataDefinition",
            data_def(data::TIMECODE),
        )
        .data(pid::LENGTH, "Length", i64_of(total))
        .data(pid::TC_START, "Start", i64_of(3600 * nominal))
        .data(
            pid::TC_FPS,
            "FPS",
            u16::try_from(nominal).unwrap_or(30).to_le_bytes().to_vec(),
        )
        .data(pid::TC_DROP, "Drop", vec![0]);
    slots.push(slot(1, "TC1", rate, Some(1), timecode));
    let mut next_slot = 2;
    for kind in [TrackKind::Video, TrackKind::Audio] {
        let mut number = 0;
        for track in project.tracks.iter().filter(|t| t.kind == kind) {
            number += 1;
            let data_kind = if kind == TrackKind::Video {
                data::PICTURE
            } else {
                data::SOUND
            };
            let mut length: u64 = 0;
            let mut components = Vec::new();
            for piece in pieces(track, &files, &mut skipped, &mut retimed) {
                match piece {
                    Piece::Gap(n) => {
                        length += n;
                        components.push(filler(data_kind, n));
                    }
                    Piece::Dissolve(n) => {
                        length = length.saturating_sub(n);
                        components.push(transition(data_kind, n));
                    }
                    Piece::Clip {
                        media,
                        start,
                        length: n,
                        speed,
                        reverse,
                        freeze,
                    } => {
                        let Some(f) = files.get(&media) else { continue };
                        let source_slot = if kind == TrackKind::Video {
                            f.picture_slot()
                        } else {
                            f.sound_slot()
                        };
                        length += n;
                        components.push(if (speed - 1.0).abs() > 1e-4 || reverse || freeze {
                            time_warped(
                                data_kind,
                                n,
                                start,
                                &f.master_id(),
                                source_slot,
                                speed,
                                reverse,
                                freeze,
                            )
                        } else {
                            source_clip(data_kind, n, start, &f.master_id(), source_slot)
                        });
                    }
                }
            }
            let sequence = Obj::new(class::SEQUENCE)
                .with(pid::DATA_DEFINITION, "DataDefinition", data_def(data_kind))
                .data(pid::LENGTH, "Length", i64_of(length))
                .with(pid::COMPONENTS, "Components", Value::Vector(components));
            let name = format!(
                "{}{number}",
                if kind == TrackKind::Video { 'V' } else { 'A' }
            );
            slots.push(slot(next_slot, &name, rate, Some(number), sequence));
            next_slot += 1;
        }
    }
    let stamp = format!("{now:?}");
    let comp_id = mob_id(derived_id(&[
        b"composition",
        project.name.as_bytes(),
        project.timeline_name.as_bytes(),
        &project.timeline_id.to_le_bytes(),
        stamp.as_bytes(),
    ]));
    let mut mobs: Vec<(Vec<u8>, Obj)> = Vec::new();
    // Markers become locator objects on the composition mob.
    let locators: Vec<Obj> = project
        .markers
        .iter()
        .map(|m| {
            Obj::new(LOCATOR_CLASS)
                .data(pid::START_TIME, "Position", i64_of(m.frame))
                .data(pid::MOB_NAME, "Name", text(&m.name))
        })
        .collect();
    let composition = mob(
        class::COMPOSITION_MOB,
        &comp_id,
        &project.timeline_name,
        slots,
        now,
    );
    let composition = if locators.is_empty() {
        composition
    } else {
        composition.with(
            pid::LOCATOR,
            "Locators",
            Value::Vector(locators),
        )
    };
    mobs.push((comp_id, composition));
    for f in files.values() {
        let [master, file] = media_mobs(project, f, now);
        mobs.push((f.master_id(), master));
        mobs.push((f.file_id(), file));
    }
    let content = Obj::new(class::CONTENT_STORAGE).with(
        pid::MOBS,
        "Mobs",
        Value::Set {
            key_pid: pid::MOB_ID,
            items: mobs,
        },
    );
    let identification = Obj::new(class::IDENTIFICATION)
        .data(
            pid::COMPANY_NAME,
            "CompanyName",
            text("Picasso Suite contributors"),
        )
        .data(pid::PRODUCT_NAME, "ProductName", text("Picasso Suite"))
        .data(
            pid::PRODUCT_VERSION_STRING,
            "ProductVersionString",
            text(env!("CARGO_PKG_VERSION")),
        )
        .data(pid::PRODUCT_ID, "ProductID", auid(PRODUCT_ID))
        .data(pid::DATE, "Date", now.bytes())
        .data(pid::PLATFORM, "Platform", text("Picasso Suite"))
        .data(
            pid::GENERATION_AUID,
            "GenerationAUID",
            auid(derived_id(&[b"generation", stamp.as_bytes()])),
        );
    let header = Obj::new(class::HEADER)
        .data(
            pid::BYTE_ORDER,
            "ByteOrder",
            0x4949i16.to_le_bytes().to_vec(),
        )
        .data(pid::LAST_MODIFIED, "LastModified", now.bytes())
        .data(pid::VERSION, "Version", vec![1, 1])
        .data(
            pid::OBJECT_MODEL_VERSION,
            "ObjectModelVersion",
            1u32.to_le_bytes().to_vec(),
        )
        .data(
            pid::OPERATIONAL_PATTERN,
            "OperationalPattern",
            auid(EDIT_PROTOCOL),
        )
        .with(pid::CONTENT, "Content", Value::Strong(Box::new(content)))
        .with(
            pid::DICTIONARY,
            "Dictionary",
            Value::Strong(Box::new(dictionary())),
        )
        .with(
            pid::IDENTIFICATION_LIST,
            "IdentificationList",
            Value::Vector(vec![identification]),
        );
    let root = Obj::new(ROOT_CLSID)
        .with(
            pid::META_DICTIONARY,
            "MetaDictionary",
            Value::Strong(Box::new(meta_dictionary())),
        )
        .with(pid::HEADER, "Header", Value::Strong(Box::new(header)));

    let mut cf = CompoundFile::create_with_version(cfb::Version::V4, Cursor::new(Vec::new()))?;
    write_obj(&mut cf, "/", &root)?;
    write_stream(&mut cf, "/referenced properties", &referenced_properties())?;
    cf.flush()?;
    Ok(Written {
        bytes: cf.into_inner().into_inner(),
        skipped,
        retimed,
    })
}

/// The timeline of `project` as an AAF file.
pub fn to_aaf(project: &Project) -> std::io::Result<Written> {
    to_aaf_at(project, Stamp::now())
}

#[cfg(test)]
mod tests {
    use super::*;
    use picasso_core::{Command, Outcome, Transition, TransitionKind};
    use std::io::Read;

    fn project() -> Project {
        let mut p = Project::new("aaf test");
        let v1 = p.tracks[0].id;
        let Ok(Outcome::Track(a1)) = (Command::AddTrack {
            kind: TrackKind::Audio,
        })
        .apply(&mut p) else {
            panic!("track");
        };
        let mut add = |name: &str, source: MediaSource, track, start, source_in| {
            let Ok(Outcome::Media(m)) = (Command::AddMedia {
                name: name.into(),
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
                duration: Some(48),
                with_sound: false,
            })
            .apply(&mut p) else {
                panic!("clip");
            };
            Command::SlipClip { clip: c, source_in }.apply(&mut p).ok();
        };
        add(
            "a",
            MediaSource::Video {
                path: r"C:\Media\a.mov".into(),
            },
            v1,
            0,
            24,
        );
        add(
            "b",
            MediaSource::Video {
                path: "/media/b.mov".into(),
            },
            v1,
            48,
            24,
        );
        add(
            "music",
            MediaSource::Audio {
                path: "/media/m.wav".into(),
            },
            a1,
            12,
            0,
        );
        // The sound of video "a" on A1 too: its file gives picture and sound.
        let a = p.tracks[0].clips[0].media;
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

    fn stamp() -> Stamp {
        Stamp::from_unix(1_798_000_000)
    }

    fn open(bytes: Vec<u8>) -> CompoundFile<Cursor<Vec<u8>>> {
        CompoundFile::open(Cursor::new(bytes)).unwrap_or_else(|e| panic!("{e}"))
    }

    fn stream(cf: &mut CompoundFile<Cursor<Vec<u8>>>, path: &str) -> Vec<u8> {
        let mut out = Vec::new();
        cf.open_stream(path)
            .unwrap_or_else(|e| panic!("{path}: {e}"))
            .read_to_end(&mut out)
            .unwrap_or_else(|e| panic!("{e}"));
        out
    }

    /// The bytes of property `pid` of the object at `path`.
    fn prop(cf: &mut CompoundFile<Cursor<Vec<u8>>>, path: &str, pid: u16) -> Vec<u8> {
        let b = stream(cf, &format!("{path}/properties"));
        let count = usize::from(u16::from_le_bytes([b[2], b[3]]));
        let mut at = 4 + count * 6;
        for i in 0..count {
            let e = 4 + i * 6;
            let id = u16::from_le_bytes([b[e], b[e + 1]]);
            let len = usize::from(u16::from_le_bytes([b[e + 4], b[e + 5]]));
            if id == pid {
                return b[at..at + len].to_vec();
            }
            at += len;
        }
        panic!("no property {pid:x} at {path}");
    }

    /// The definition a weak property of the object at `path` names.
    fn weak(cf: &mut CompoundFile<Cursor<Vec<u8>>>, path: &str, pid: u16) -> Uuid {
        let b = prop(cf, path, pid);
        let key = b.get(5..).unwrap_or_else(|| panic!("{pid:x} is no weak reference"));
        Uuid::from_slice_le(key).unwrap_or_else(|e| panic!("{e}"))
    }

    #[test]
    fn the_file_is_a_compound_file_of_aaf_objects() {
        let w = to_aaf_at(&project(), stamp()).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!((w.skipped, w.retimed), (0, 0));
        let mut cf = open(w.bytes);
        assert_eq!(cf.root_entry().clsid(), &ROOT_CLSID);
        let header = cf.entry("/Header-2").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(header.clsid(), &class::HEADER);
        // The root's properties name the two storages it holds.
        let props = stream(&mut cf, "/properties");
        assert_eq!(&props[..4], &[0x4c, 0x20, 2, 0]);
        // A composition, and a master and a file mob for each of the three files.
        let index = stream(&mut cf, "/Header-2/Content-3b03/Mobs-1901 index");
        assert_eq!(
            u32::from_le_bytes([index[0], index[1], index[2], index[3]]),
            7
        );
        assert_eq!(&index[12..15], &[0x01, 0x44, 32]);
        let refs = stream(&mut cf, "/referenced properties");
        assert_eq!(refs, referenced_properties());
        // Long collection names are shortened as compound files need.
        assert!(cf.is_storage("/Header-2/Identifi-ionList-3b06{0}"));
        assert!(cf.is_storage("/Header-2/Dictionary-3b04/Operatio-nitions-2603{0}"));
    }

    #[test]
    fn a_dissolve_borrows_frames_from_both_clips() {
        let p = project();
        let files = files_used(&p);
        let (mut skipped, mut retimed) = (0, 0);
        let got: Vec<String> = pieces(&p.tracks[0], &files, &mut skipped, &mut retimed)
            .iter()
            .map(|piece| match piece {
                Piece::Clip { start, length, .. } => format!("clip {start}+{length}"),
                Piece::Gap(n) => format!("gap {n}"),
                Piece::Dissolve(n) => format!("dissolve {n}"),
            })
            .collect();
        // 48 + 6 frames of the first clip, a 12-frame dissolve, then the second from 6 frames
        // earlier: 54 + 54 - 12 is still 96 frames of timeline.
        assert_eq!(got, ["clip 24+54", "dissolve 12", "clip 18+54"]);
    }

    #[test]
    fn a_speed_change_is_written_as_a_video_speed_control() {
        let mut p = project();
        p.tracks[0].clips[0].speed = 2.0;
        let w = to_aaf_at(&p, stamp()).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(w.retimed, 1);
        let mut cf = open(w.bytes);
        // The clip is an operation group over the source clip, as the Edit Protocol lays a time
        // warp out, and the group holds a parameter.
        let at =
            "/Header-2/Content-3b03/Mobs-1901{0}/Slots-4403{1}/Segment-4803/Components-1001{0}";
        assert_eq!(
            cf.entry(at).unwrap_or_else(|e| panic!("{e}")).clsid(),
            &class::OPERATION_GROUP
        );
        assert_eq!(weak(&mut cf, at, pid::OPERATION), VIDEO_SPEED_CONTROL);
        let input = format!("{at}/InputSegments-b02{{0}}");
        assert_eq!(
            cf.entry(&input).unwrap_or_else(|e| panic!("{e}")).clsid(),
            &class::SOURCE_CLIP
        );
        let parameter = format!("{at}/Parameters-b03{{0}}");
        assert_eq!(
            cf.entry(&parameter).unwrap_or_else(|e| panic!("{e}")).clsid(),
            &class::CONSTANT_VALUE
        );
        assert_eq!(
            weak(&mut cf, &parameter, pid::PARAMETER_DEFINITION),
            SPEED_RATIO
        );
        // Speed 2 is a ratio of 1/2: the output is half the input, and the source clip holds the
        // frames the clip takes from its source.
        assert_eq!(
            prop(&mut cf, &parameter, pid::PARAMETER_VALUE),
            [1, 0, 0, 0, 2, 0, 0, 0]
        );
        // The dissolve runs the clip 6 frames on: 54 frames on the timeline, 108 of the source.
        assert_eq!(prop(&mut cf, at, pid::LENGTH), 54i64.to_le_bytes());
        assert_eq!(prop(&mut cf, &input, pid::LENGTH), 108i64.to_le_bytes());
        // The dictionary names the operation and the parameter it takes.
        assert!(cf.is_storage("/Header-2/Dictionary-3b04/Paramete-nitions-2604{0}"));
    }

    #[test]
    fn a_freeze_is_written_as_a_video_repeat() {
        let mut p = project();
        p.tracks[0].clips[0].freeze = true;
        let w = to_aaf_at(&p, stamp()).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(w.retimed, 1);
        let mut cf = open(w.bytes);
        let at =
            "/Header-2/Content-3b03/Mobs-1901{0}/Slots-4403{1}/Segment-4803/Components-1001{0}";
        assert_eq!(weak(&mut cf, at, pid::OPERATION), VIDEO_REPEAT);
        // The source clip is the one frame the freeze holds.
        let input = format!("{at}/InputSegments-b02{{0}}");
        assert_eq!(prop(&mut cf, &input, pid::LENGTH), 1i64.to_le_bytes());
        // A frame repeat takes no parameter.
        assert!(!cf.is_storage(&format!("{at}/Parameters-b03{{0}}")));
    }

    #[test]
    fn names_and_ids() {
        assert_eq!(storage_name("Mobs", 0x1901, true), "Mobs-1901");
        assert_eq!(
            storage_name("ContainerDefinitions", 0x2608, true),
            "Containe-nitions-2608"
        );
        assert_eq!(
            storage_name("EssenceDescription", 0x4701, false),
            "EssenceDescription-4701"
        );
        assert_eq!(derived_id(&[b"a"]), derived_id(&[b"a"]));
        assert_ne!(derived_id(&[b"a"]), derived_id(&[b"b"]));
        assert_eq!(
            Stamp::from_unix(0),
            Stamp {
                year: 1970,
                month: 1,
                day: 1,
                hour: 0,
                minute: 0,
                second: 0
            }
        );
        assert_eq!(
            Stamp::from_unix(1_798_000_000),
            Stamp {
                year: 2026,
                month: 12,
                day: 23,
                hour: 4,
                minute: 26,
                second: 40
            }
        );
        assert_eq!(text("ab"), [b'a', 0, b'b', 0, 0, 0]);
    }

    /// Writes the test timeline to `PICASSO_AAF_SAMPLE`, for checking with other readers.
    #[test]
    fn sample_for_other_readers() {
        if let Ok(path) = std::env::var("PICASSO_AAF_SAMPLE") {
            let w = to_aaf_at(&project(), stamp()).unwrap_or_else(|e| panic!("{e}"));
            std::fs::write(path, w.bytes).unwrap_or_else(|e| panic!("{e}"));
        }
    }

    #[test]
    fn generated_media_is_left_out() {
        let w = to_aaf_at(&Project::demo(), stamp()).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(w.skipped, 3);
    }
}
