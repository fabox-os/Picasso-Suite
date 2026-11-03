//! Reads video files through FFmpeg (Agents.md: "Decode: ffmpeg-next"). Native only; the browser
//! build has no FFmpeg. Times are whole microseconds (`i64`), never floating-point seconds.

pub mod audio;
pub mod hw;

use std::path::{Path, PathBuf};
use std::sync::Once;

use ff::format::{context::Input, Pixel};
use ff::media::Type;
use ff::software::scaling::{context::Context as Scaler, flag::Flags};
use ffmpeg_next as ff;
use thiserror::Error;

/// Microseconds in a second, FFmpeg's own `AV_TIME_BASE`.
pub(crate) const MICROS: i64 = 1_000_000;

#[derive(Debug, Error)]
pub enum DecodeError {
    #[error("could not read the file: {0}")]
    Open(String),
    #[error("{0}")]
    Refused(String),
    #[error("the file has no video")]
    NoVideo,
    #[error("the file has no sound")]
    NoAudio,
    #[error("could not decode: {0}")]
    Decode(String),
    #[error("there is no frame at that time")]
    NoFrame,
    /// A newer picture was asked for while this one was being decoded ([`Scrub::cancel`]).
    #[error("the picture was not wanted any more")]
    Cancelled,
}

impl From<ff::Error> for DecodeError {
    fn from(e: ff::Error) -> Self {
        Self::Decode(e.to_string())
    }
}

/// What a file contains.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaInfo {
    pub width: u32,
    pub height: u32,
    /// Frame rate as a fraction (frames, seconds).
    pub fps: (u32, u32),
    /// Length in microseconds; `None` when the container does not say.
    pub duration_us: Option<i64>,
    pub video_codec: String,
    pub audio_codec: Option<String>,
    /// The video codec's profile ("High", "Main 10"), when the codec has profiles.
    pub profile: Option<String>,
    /// Its level ("4.0", "5.1"), read from the codec's configuration in the file.
    pub level: Option<String>,
    /// The start timecode the file carries ("01:00:00:00"), when it has one.
    pub timecode: Option<String>,
    /// Whether the pictures are not a fixed time apart (a phone or a screen recording): the rate
    /// the file is made to run at differs from the average one by more than a few percent. A copy
    /// of such a file at one rate would not have its pictures at the original's times.
    pub variable_frame_rate: bool,
}

impl MediaInfo {
    /// The codec as editors show it: "H.264 High L4.0", "H.265 Main L5.1", "VP9".
    pub fn codec_label(&self) -> String {
        let name = match self.video_codec.as_str() {
            "h264" => "H.264".to_string(),
            "hevc" => "H.265".to_string(),
            "prores" => "Apple ProRes".to_string(),
            "av1" => "AV1".to_string(),
            "vp9" => "VP9".to_string(),
            "vp8" => "VP8".to_string(),
            "mpeg4" => "MPEG-4".to_string(),
            "mjpeg" => "Motion JPEG".to_string(),
            other => other.to_uppercase(),
        };
        let mut label = name;
        if let Some(p) = &self.profile {
            label.push(' ');
            label.push_str(p);
        }
        if let Some(l) = &self.level {
            label.push_str(" L");
            label.push_str(l);
        }
        label
    }
}

/// A readable name for a codec profile (`H264(High10)` is "High 10").
fn profile_name(profile: ff::codec::Profile) -> Option<String> {
    use ff::codec::Profile;
    let text = match profile {
        Profile::Unknown | Profile::Reserved => return None,
        other => format!("{other:?}"),
    };
    // "H264(ConstrainedBaseline)" -> "Constrained Baseline".
    let inner = text
        .split_once('(')
        .map_or(text.as_str(), |(_, rest)| rest.trim_end_matches(')'));
    let mut out = String::new();
    for (i, c) in inner.chars().enumerate() {
        let after_letter = out.chars().last().is_some_and(|p: char| p.is_alphabetic());
        if i > 0 && ((c.is_uppercase() && after_letter) || (c.is_ascii_digit() && after_letter)) {
            out.push(' ');
        }
        out.push(c);
    }
    (!out.is_empty()).then_some(out)
}

/// The level of an H.264 or H.265 stream, from its configuration record in the file (`avcC` or
/// `hvcC`), which sits near the start or the end of an MP4 or QuickTime file.
fn level_of(path: &Path, codec: &str) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    const PART: u64 = 4 << 20;
    let (tag, at): (&[u8], usize) = match codec {
        "h264" => (b"avcC", 3),
        "hevc" => (b"hvcC", 12),
        _ => return None,
    };
    let mut file = std::fs::File::open(path).ok()?;
    let size = file.metadata().ok()?.len();
    let mut found = None;
    for start in [0, size.saturating_sub(PART)] {
        if file.seek(SeekFrom::Start(start)).is_err() {
            continue;
        }
        let mut buf = Vec::new();
        if (&mut file).take(PART).read_to_end(&mut buf).is_err() {
            continue;
        }
        if let Some(i) = buf.windows(4).position(|w| w == tag) {
            // The record starts after the four letters.
            found = buf.get(i + 4 + at).copied();
            if found.is_some() {
                break;
            }
        }
    }
    let idc = found.filter(|v| *v > 0)?;
    Some(if codec == "hevc" {
        // H.265 levels are 30 times the level: 153 is 5.1.
        let tenths = u32::from(idc) / 3;
        format!("{}.{}", tenths / 10, tenths % 10)
    } else {
        format!("{}.{}", idc / 10, idc % 10)
    })
}

/// One decoded picture, 8-bit RGBA.
#[derive(Clone, Debug)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    /// Presentation time in microseconds.
    pub pts_us: i64,
    pub rgba: Vec<u8>,
}

/// Anything that can give the picture at a time (Agents.md: `FrameProvider`).
pub trait FrameProvider {
    fn frame_at(&mut self, pts_us: i64) -> Result<Frame, DecodeError>;
}

pub(crate) fn init() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        if let Err(e) = ff::init() {
            tracing::error!("ffmpeg did not start: {e}");
        }
        ff::util::log::set_level(ff::util::log::Level::Error);
    });
}

/// Why a media path must not be opened on its own, or `None` when it is an ordinary file path.
///
/// Project files and Resolve files carry media paths, and a file from someone else must not be
/// able to make the program reach out:
/// - FFmpeg reads `http://...`, `ftp:`, `concat:`, `pipe:`, `subfile,...` and the like as protocols
///   and not as file names, so those are refused;
/// - on Windows, touching `\server\share` sends the user's login hash to that server, so network
///   paths are refused unless `PICASSO_ALLOW_NETWORK_MEDIA` is set. Mapped drive letters are fine.
pub fn refuse_path(path: &str) -> Option<String> {
    let p = path.trim_start();
    let scheme_len = p
        .bytes()
        .take_while(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
        .count();
    // One letter before the colon is a drive (`C:`); more is a protocol.
    if p.as_bytes().get(scheme_len) == Some(&b':') && scheme_len > 1 {
        return Some(format!(
            "{}: is a network or special address, not a file on this computer",
            &p[..scheme_len]
        ));
    }
    if p.starts_with("subfile,") {
        return Some("special FFmpeg addresses are not opened".to_string());
    }
    if (p.starts_with("\\\\") || p.starts_with("//"))
        && std::env::var_os("PICASSO_ALLOW_NETWORK_MEDIA").is_none()
    {
        return Some(
            "network paths are not opened automatically (set PICASSO_ALLOW_NETWORK_MEDIA=1 to allow them)".to_string(),
        );
    }
    None
}

fn guard(path: &Path) -> Result<(), DecodeError> {
    match path.to_str().and_then(refuse_path) {
        Some(why) => Err(DecodeError::Refused(why)),
        None => Ok(()),
    }
}

fn open(path: &Path) -> Result<Input, DecodeError> {
    guard(path)?;
    init();
    ff::format::input(path).map_err(|e| DecodeError::Open(e.to_string()))
}

fn rational(r: ff::Rational) -> (u32, u32) {
    (
        u32::try_from(r.numerator()).unwrap_or(0),
        u32::try_from(r.denominator()).unwrap_or(1).max(1),
    )
}

/// Reads the metadata of a file without decoding it.
pub fn probe(path: &Path) -> Result<MediaInfo, DecodeError> {
    let input = open(path)?;
    let video = input
        .streams()
        .best(Type::Video)
        .ok_or(DecodeError::NoVideo)?;
    let ctx = ff::codec::context::Context::from_parameters(video.parameters())?;
    let codec_name = ctx.id().name().to_string();
    let decoder = ctx.decoder().video()?;
    let profile = profile_name(decoder.profile());
    let fps = {
        let r = rational(video.avg_frame_rate());
        if r.0 == 0 {
            rational(video.rate())
        } else {
            r
        }
    };
    let variable_frame_rate = {
        let (avg, base) = (rational(video.avg_frame_rate()), rational(video.rate()));
        avg.0 > 0 && base.0 > 0 && {
            let (a, b) = (f64::from(avg.0) / f64::from(avg.1), f64::from(base.0) / f64::from(base.1));
            (a - b).abs() / b > 0.03
        }
    };
    let duration = input.duration();
    // A camera writes its start timecode in the video stream's metadata (the `tmcd` track of a
    // QuickTime file); some containers keep it on the file instead.
    let stream_timecode = video.metadata().get("timecode").map(str::to_string);
    let timecode = stream_timecode.or_else(|| input.metadata().get("timecode").map(str::to_string));
    Ok(MediaInfo {
        width: decoder.width(),
        height: decoder.height(),
        fps,
        duration_us: (duration > 0).then_some(duration),
        level: level_of(path, &codec_name),
        profile,
        video_codec: codec_name,
        timecode,
        variable_frame_rate,
        audio_codec: input.streams().best(Type::Audio).and_then(|a| {
            ff::codec::context::Context::from_parameters(a.parameters())
                .ok()
                .map(|c| c.id().name().to_string())
        }),
    })
}

/// A key frame this much earlier than the picture asked for is worth showing on its own: the
/// pictures between them take longer than that to decode.
const PREVIEW_GAP_US: i64 = 250_000;

/// What dragging the playhead needs of a read ([`VideoDecoder::frame_at_scrub`]): the decoder
/// seeks to the key frame before the picture asked for and decodes forward, which on video with
/// few key frames takes long enough that a newer request is waiting by the end.
pub struct Scrub<'a> {
    /// Asked between packets: when it says yes, the read stops with [`DecodeError::Cancelled`].
    /// The decoder stays where it got to, so the next read goes on from there when it can.
    pub cancel: &'a dyn Fn() -> bool,
    /// Gets the key frame the read started from, as soon as it is decoded, when the picture
    /// asked for is far after it: something to show while the rest is decoded.
    pub preview: &'a mut dyn FnMut(Frame),
}

/// Pictures read at the start of a trial before it counts: the first ones after opening carry
/// the decoder's start-up.
const TRIAL_SKIP: u32 = 5;
/// Pictures each way is timed over.
const TRIAL_PICTURES: u32 = 30;
/// A card this fast (ms a picture, read and converted) is kept without trying the processor.
const CARD_FAST_MS: f64 = 6.0;
/// The processor is kept only when it is this much faster than the card (it uses more power).
const PROCESSOR_MARGIN: f64 = 0.85;

/// Which way each file decodes faster, learnt while it played (true: the graphics card), so it
/// is learnt once a run.
static FASTER: std::sync::Mutex<Option<std::collections::HashMap<PathBuf, bool>>> =
    std::sync::Mutex::new(None);

fn faster_way(path: &Path) -> Option<bool> {
    let known = FASTER.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    known.as_ref()?.get(path).copied()
}

fn learn_faster_way(path: &Path, card: bool) {
    let mut known = FASTER.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    known
        .get_or_insert_with(Default::default)
        .insert(path.to_path_buf(), card);
}

/// What a [`Trial`] says after a picture.
#[derive(Clone, Copy, Debug, PartialEq)]
enum TrialStep {
    /// Still timing.
    Timing,
    /// The card was timed and is slow: time the processor now.
    TryProcessor,
    /// Decided: the card (true) or the processor.
    Use(bool),
}

/// Times how long the pictures of a file take while it plays, first on the graphics card and,
/// when that is slow, on the processor, to keep the faster way. Copying a picture off the card
/// costs more on some systems (Direct3D 11 on Windows, 10-bit pictures above all) than decoding
/// it on a processor with many cores; on others the card is far ahead.
#[derive(Clone, Debug, Default)]
struct Trial {
    /// The card's time, ms a picture, once measured.
    card_ms: Option<f64>,
    sum_ms: f64,
    count: u32,
}

impl Trial {
    /// Counts one picture read while playing forward, `ms` long, read the way `on_card` says.
    fn record(&mut self, ms: f64, on_card: bool) -> TrialStep {
        self.count += 1;
        if self.count <= TRIAL_SKIP {
            return TrialStep::Timing;
        }
        self.sum_ms += ms;
        if self.count < TRIAL_SKIP + TRIAL_PICTURES {
            return TrialStep::Timing;
        }
        let average = self.sum_ms / f64::from(TRIAL_PICTURES);
        (self.count, self.sum_ms) = (0, 0.0);
        if on_card {
            if average <= CARD_FAST_MS {
                return TrialStep::Use(true);
            }
            self.card_ms = Some(average);
            return TrialStep::TryProcessor;
        }
        let card = self.card_ms.unwrap_or(f64::INFINITY);
        TrialStep::Use(average >= card * PROCESSOR_MARGIN)
    }
}

/// Decodes the pictures of one file, on request.
pub struct VideoDecoder {
    path: PathBuf,
    /// The pictures come from the graphics card ([`hw`]).
    on_card: bool,
    /// Whether hardware decoding was asked for when the file was opened.
    asked: bool,
    input: Input,
    stream: usize,
    decoder: ff::decoder::Video,
    time_base: ff::Rational,
    /// The last picture at or before the time asked for, kept so playing forward needs no seek.
    best: Option<(ff::frame::Video, i64)>,
    /// The first picture after it.
    ahead: Option<(ff::frame::Video, i64)>,
    /// The end of the file was read and the decoder drained: nothing more comes until a seek.
    at_end: bool,
    /// The converter to RGBA, kept while the pictures keep their format and size.
    scaler: Option<Scaler>,
    /// Timing the card against the processor while the file plays ([`Trial`]); `None` once
    /// decided, or when there is nothing to choose.
    trial: Option<Trial>,
    /// Whether the last read had to seek (it is not counted by the trial).
    sought: bool,
    /// The converter's output, kept so each picture is not written into a fresh allocation of
    /// several megabytes (which costs page faults, more of them on Windows).
    rgba: ff::frame::Video,
}

impl VideoDecoder {
    /// Opens the video of a file, decoding it on the graphics card when System Preferences says
    /// so and the card can.
    pub fn open(path: &Path) -> Result<Self, DecodeError> {
        Self::open_with(path, hw::hardware_decoding())
    }

    fn open_with(path: &Path, hardware: bool) -> Result<Self, DecodeError> {
        let input = open(path)?;
        let (stream, params, time_base) = {
            let s = input
                .streams()
                .best(Type::Video)
                .ok_or(DecodeError::NoVideo)?;
            (s.index(), s.parameters(), s.time_base())
        };
        let mut context = ff::codec::context::Context::from_parameters(params)?;
        // A file found to decode faster on the processor stays there for the rest of the run.
        let learnt = if hardware { faster_way(path) } else { None };
        let on_card = hardware && learnt != Some(false) && hw::attach(&mut context);
        // Decode on every core: pictures of a 1080p or 4K file do not arrive in time on one. The
        // card needs no threads, and each would hold more of its memory.
        context.set_threading(ff::threading::Config {
            kind: ff::threading::Type::Frame,
            count: if on_card { 1 } else { 0 },
        });
        let decoder = context.decoder().video()?;
        Ok(Self {
            path: path.to_path_buf(),
            on_card,
            asked: hardware,
            input,
            stream,
            decoder,
            time_base,
            best: None,
            ahead: None,
            at_end: false,
            scaler: None,
            trial: (on_card && learnt.is_none()).then(Trial::default),
            sought: false,
            rgba: ff::frame::Video::empty(),
        })
    }

    fn to_us(&self, pts: i64) -> i64 {
        let tb = self.time_base;
        pts.saturating_mul(i64::from(tb.numerator()))
            .saturating_mul(MICROS)
            / i64::from(tb.denominator()).max(1)
    }

    fn convert(
        &mut self,
        decoded: &ff::frame::Video,
        pts_us: i64,
        reduce: u32,
    ) -> Result<Frame, DecodeError> {
        // A proxy: the picture is made smaller while it is converted, which costs less than
        // converting it whole and scaling it afterwards.
        let reduce = reduce.max(1);
        let out_w = (decoded.width() / reduce).max(2);
        let out_h = (decoded.height() / reduce).max(2);
        let same = self.scaler.as_ref().is_some_and(|s| {
            let (input, output) = (s.input(), s.output());
            input.format == decoded.format()
                && input.width == decoded.width()
                && input.height == decoded.height()
                && output.width == out_w
                && output.height == out_h
        });
        if !same {
            self.scaler = Some(Scaler::get(
                decoded.format(),
                decoded.width(),
                decoded.height(),
                Pixel::RGBA,
                out_w,
                out_h,
                Flags::FAST_BILINEAR,
            )?);
            self.rgba = ff::frame::Video::empty();
        }
        let scaler = self.scaler.as_mut().ok_or(DecodeError::NoFrame)?;
        let rgba = &mut self.rgba;
        scaler.run(decoded, rgba)?;
        let (w, h) = (rgba.width() as usize, rgba.height() as usize);
        let stride = rgba.stride(0);
        let data = rgba.data(0);
        let row = w * 4;
        let out = if stride == row {
            data.get(..row * h).ok_or(DecodeError::NoFrame)?.to_vec()
        } else {
            let mut out = Vec::with_capacity(row * h);
            for y in 0..h {
                let start = y * stride;
                out.extend_from_slice(data.get(start..start + row).ok_or(DecodeError::NoFrame)?);
            }
            out
        };
        Ok(Frame {
            width: rgba.width(),
            height: rgba.height(),
            pts_us,
            rgba: out,
        })
    }
}

impl FrameProvider for VideoDecoder {
    fn frame_at(&mut self, pts_us: i64) -> Result<Frame, DecodeError> {
        self.frame_at_reduced(pts_us, 1)
    }
}

impl VideoDecoder {
    /// The picture shown at `pts_us`: the last one that starts at or before it, made `reduce`
    /// times smaller on each side (1 is the whole picture). Asking for times that move forward a
    /// little each time decodes on from where it was; anything else seeks to the key frame before
    /// the time.
    ///
    /// If the graphics card fails, the file is opened again and decoded on the processor.
    pub fn frame_at_reduced(&mut self, pts_us: i64, reduce: u32) -> Result<Frame, DecodeError> {
        self.frame_at_with(pts_us, reduce, None)
    }

    /// `frame_at_reduced` for a playhead that is being dragged: it gives up as soon as `scrub`
    /// says a newer picture is wanted, and hands over the key frame it starts from on the way.
    pub fn frame_at_scrub(
        &mut self,
        pts_us: i64,
        reduce: u32,
        scrub: &mut Scrub<'_>,
    ) -> Result<Frame, DecodeError> {
        self.frame_at_with(pts_us, reduce, Some(scrub))
    }

    fn frame_at_with(
        &mut self,
        pts_us: i64,
        reduce: u32,
        scrub: Option<&mut Scrub<'_>>,
    ) -> Result<Frame, DecodeError> {
        let started = std::time::Instant::now();
        let read = self.read_or_fall_back(pts_us, reduce, scrub);
        if read.is_ok() && !self.sought {
            self.time_picture(started.elapsed().as_secs_f64() * 1000.0);
        }
        read
    }

    /// Counts a picture read while playing forward, and acts on what the trial says: the
    /// processor is tried, or the faster way is kept for this file. A change opens the file
    /// again; the next read seeks to its picture.
    fn time_picture(&mut self, ms: f64) {
        let on_card = self.on_card;
        let Some(step) = self.trial.as_mut().map(|t| t.record(ms, on_card)) else {
            return;
        };
        let reopen = |this: &Self, hardware: bool| {
            let asked = this.asked;
            Self::open_with(&this.path, hardware).map(|mut d| {
                d.asked = asked;
                d
            })
        };
        match step {
            TrialStep::Timing => {}
            TrialStep::TryProcessor => {
                let trial = self.trial.take();
                if let Ok(d) = reopen(self, false) {
                    *self = d;
                }
                self.trial = trial;
            }
            TrialStep::Use(card) => {
                let card_ms = self.trial.take().and_then(|t| t.card_ms);
                tracing::info!(
                    "{}: decoding on the {} (the card took {} ms a picture)",
                    self.path.display(),
                    if card { "graphics card" } else { "processor" },
                    card_ms.map_or_else(|| format!("at most {CARD_FAST_MS}"), |m| format!("{m:.1}")),
                );
                learn_faster_way(&self.path, card);
                if card != self.on_card {
                    if let Ok(d) = reopen(self, true) {
                        *self = d;
                    }
                }
            }
        }
    }

    fn read_or_fall_back(
        &mut self,
        pts_us: i64,
        reduce: u32,
        mut scrub: Option<&mut Scrub<'_>>,
    ) -> Result<Frame, DecodeError> {
        match self.read(pts_us, reduce, scrub.as_deref_mut()) {
            Err(e) if self.on_card && !matches!(e, DecodeError::Cancelled) => {
                tracing::warn!(
                    "hardware decoding of {} failed ({e}); decoding on the processor",
                    self.path.display()
                );
                let asked = self.asked;
                *self = Self::open_with(&self.path.clone(), false)?;
                // Still counts as opened with the setting on, so it is not opened again for it.
                self.asked = asked;
                self.read(pts_us, reduce, scrub)
            }
            other => other,
        }
    }

    /// The time of the picture that `frame_at_reduced(pts_us, ..)` would answer with, when the
    /// decoder already holds it and needs to decode nothing more for it. A project that plays
    /// faster than its video (24 fps footage in a 60 fps project) asks for the same picture
    /// several times in a row; the caller can keep the answer it made and skip the conversion.
    pub fn held_picture_at(&self, pts_us: i64) -> Option<i64> {
        let target = pts_us.max(0);
        let (_, held) = self.best.as_ref()?;
        let (_, next) = self.ahead.as_ref()?;
        (*held <= target && target < *next).then_some(*held)
    }

    /// Whether System Preferences turned hardware decoding on or off since the file was opened,
    /// so it should be opened again.
    pub fn is_stale(&self) -> bool {
        self.asked != hw::hardware_decoding()
    }

    /// The file the decoder is reading. A caller uses it to tell when the file has switched
    /// (the optimised copy standing in for the original it was opened on) and the decoder must
    /// be reopened to read the new one.
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn read(
        &mut self,
        pts_us: i64,
        reduce: u32,
        mut scrub: Option<&mut Scrub<'_>>,
    ) -> Result<Frame, DecodeError> {
        let target = pts_us.max(0);
        let continuing = self
            .best
            .as_ref()
            .is_some_and(|(_, t)| target >= *t && target - *t <= 2 * MICROS);
        self.sought = !continuing;
        // Only a read that starts from a key frame has one to hand over.
        let mut preview_due = !continuing && scrub.is_some();
        if !continuing {
            // Jump to the key frame at or before the target, then decode forward.
            self.input.seek(target, ..target.saturating_add(1))?;
            self.decoder.flush();
            self.best = None;
            self.ahead = None;
            self.at_end = false;
        }
        // The picture that was already waiting.
        if let Some((frame, t)) = self.ahead.take() {
            if t <= target {
                self.best = Some((frame, t));
            } else {
                self.ahead = Some((frame, t));
            }
        }
        let mut decoded = ff::frame::Video::empty();
        let mut done = self.ahead.is_some();
        while !done {
            if scrub.as_deref().is_some_and(|s| (s.cancel)()) {
                return Err(DecodeError::Cancelled);
            }
            if !self.at_end {
                let mut packet = ff::Packet::empty();
                match packet.read(&mut self.input) {
                    Ok(()) => {
                        if packet.stream() != self.stream {
                            continue;
                        }
                        self.decoder.send_packet(&packet)?;
                    }
                    Err(ff::Error::Eof) => {
                        // Once: the decoder then gives what it holds, and past the last
                        // picture the last one stays.
                        self.decoder.send_eof()?;
                        self.at_end = true;
                    }
                    Err(e) => return Err(e.into()),
                }
            }
            let mut got = false;
            while self.decoder.receive_frame(&mut decoded).is_ok() {
                got = true;
                // The best-effort time also covers streams whose pictures carry no time of their own.
                let t = self.to_us(decoded.timestamp().unwrap_or(0));
                // Pictures on the card are copied out at once: the card has few of them.
                let picture = if hw::on_card(&decoded) {
                    hw::download(&decoded)?
                } else {
                    // Moved, not copied: the decoder fills a fresh frame for the next picture.
                    std::mem::replace(&mut decoded, ff::frame::Video::empty())
                };
                if t <= target {
                    // The first picture after a seek is the key frame.
                    if preview_due && target - t > PREVIEW_GAP_US {
                        preview_due = false;
                        if let Some(s) = scrub.as_deref_mut() {
                            let key = self.convert(&picture, t, reduce)?;
                            (s.preview)(key);
                        }
                    }
                    self.best = Some((picture, t));
                } else {
                    self.ahead = Some((picture, t));
                    done = true;
                    break;
                }
            }
            if self.at_end && !got {
                done = true;
            }
        }
        // Before the first picture of the file, show the first one.
        // Taken out and put back, not copied: the held picture is a whole decoded frame.
        let from_best = self.best.is_some();
        let (frame, t) = self
            .best
            .take()
            .or_else(|| self.ahead.take())
            .ok_or(DecodeError::NoFrame)?;
        let converted = self.convert(&frame, t, reduce);
        if from_best {
            self.best = Some((frame, t));
        } else {
            self.ahead = Some((frame, t));
        }
        converted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_file_is_an_error_not_a_crash() {
        assert!(matches!(
            probe(Path::new("this-file-does-not-exist.mp4")),
            Err(DecodeError::Open(_))
        ));
    }

    /// Makes a two-second test clip with the `ffmpeg` program; `None` when it is not installed.
    fn test_clip(name: &str) -> Option<std::path::PathBuf> {
        let path = std::env::temp_dir().join(name);
        let ok = std::process::Command::new("ffmpeg")
            .args(["-y", "-v", "error", "-f", "lavfi", "-i"])
            .arg("testsrc=duration=2:size=320x180:rate=24")
            .args(["-pix_fmt", "yuv420p", "-g", "12"])
            .arg(&path)
            .status()
            .is_ok_and(|s| s.success());
        ok.then_some(path)
    }

    /// A test clip that carries a start timecode, as a camera writes it.
    fn test_clip_with_timecode(name: &str) -> Option<std::path::PathBuf> {
        let path = std::env::temp_dir().join(name);
        let ok = std::process::Command::new("ffmpeg")
            .args(["-y", "-v", "error", "-f", "lavfi", "-i"])
            .arg("testsrc=duration=2:size=320x180:rate=24")
            .args(["-pix_fmt", "yuv420p", "-timecode", "01:00:00:00"])
            .arg(&path)
            .status()
            .is_ok_and(|s| s.success());
        ok.then_some(path)
    }

    #[test]
    fn the_start_timecode_is_read_from_the_file() -> Result<(), String> {
        let Some(path) = test_clip_with_timecode("picasso_decode_timecode.mp4") else {
            return Ok(());
        };
        let info = probe(&path).map_err(|e| e.to_string())?;
        assert_eq!(info.timecode.as_deref(), Some("01:00:00:00"));
        let _ = std::fs::remove_file(path);
        Ok(())
    }

    /// Eight seconds with a key frame only every 250 pictures, as cameras write long GOPs.
    fn long_gop_clip(name: &str) -> Option<std::path::PathBuf> {
        let path = std::env::temp_dir().join(name);
        let ok = std::process::Command::new("ffmpeg")
            .args(["-y", "-v", "error", "-f", "lavfi", "-i"])
            .arg("testsrc=duration=8:size=320x180:rate=25")
            .args(["-pix_fmt", "yuv420p", "-g", "250", "-keyint_min", "250"])
            .args(["-sc_threshold", "0"])
            .arg(&path)
            .status()
            .is_ok_and(|s| s.success());
        ok.then_some(path)
    }

    #[test]
    fn scrubbing_hands_over_the_key_frame_and_stops_when_told() -> Result<(), String> {
        let Some(path) = long_gop_clip("picasso_decode_long_gop.mp4") else {
            return Ok(());
        };
        let target = 6_000_000;
        // What a plain read gives, to compare with.
        let plain = VideoDecoder::open_with(&path, false)
            .and_then(|mut d| d.frame_at_reduced(target, 1))
            .map_err(|e| e.to_string())?;

        let mut d = VideoDecoder::open_with(&path, false).map_err(|e| e.to_string())?;
        // Nobody asks for anything newer: the key frame comes first, then the exact picture.
        let mut previews = Vec::new();
        let never = || false;
        let mut take = |f: Frame| previews.push(f.pts_us);
        let exact = d
            .frame_at_scrub(
                target,
                1,
                &mut Scrub {
                    cancel: &never,
                    preview: &mut take,
                },
            )
            .map_err(|e| e.to_string())?;
        assert_eq!(previews, [0], "the key frame at the start of the file");
        assert_eq!((exact.pts_us, &exact.rgba), (plain.pts_us, &plain.rgba));

        // A newer request arrives while the pictures are being decoded: the read stops, at once.
        let mut d = VideoDecoder::open_with(&path, false).map_err(|e| e.to_string())?;
        let asked = std::cell::Cell::new(0);
        let soon = || {
            asked.set(asked.get() + 1);
            asked.get() > 5
        };
        let mut nothing = |_: Frame| {};
        let stopped = d.frame_at_scrub(
            target,
            1,
            &mut Scrub {
                cancel: &soon,
                preview: &mut nothing,
            },
        );
        assert!(
            matches!(stopped, Err(DecodeError::Cancelled)),
            "{stopped:?}"
        );
        assert!(asked.get() <= 7, "it went on after being told to stop");
        // The decoder is not left broken: the same picture, asked for plainly, is the same.
        let again = d.frame_at_reduced(target, 1).map_err(|e| e.to_string())?;
        assert_eq!((again.pts_us, &again.rgba), (plain.pts_us, &plain.rgba));

        // A picture close after its key frame needs no preview.
        let mut d = VideoDecoder::open_with(&path, false).map_err(|e| e.to_string())?;
        let mut previews = 0;
        let mut count = |_: Frame| previews += 1;
        d.frame_at_scrub(
            100_000,
            1,
            &mut Scrub {
                cancel: &never,
                preview: &mut count,
            },
        )
        .map_err(|e| e.to_string())?;
        assert_eq!(previews, 0);
        let _ = std::fs::remove_file(path);
        Ok(())
    }

    /// Feeds a trial `n` pictures of `ms` each, read the way `on_card` says; the last step.
    fn feed(trial: &mut Trial, n: u32, ms: f64, on_card: bool) -> TrialStep {
        let mut step = TrialStep::Timing;
        for _ in 0..n {
            step = trial.record(ms, on_card);
            if step != TrialStep::Timing {
                break;
            }
        }
        step
    }

    #[test]
    fn a_fast_card_is_kept_without_trying_the_processor() {
        let mut t = Trial::default();
        assert_eq!(feed(&mut t, 100, 3.0, true), TrialStep::Use(true));
    }

    #[test]
    fn a_slow_card_is_timed_against_the_processor_and_the_faster_kept() {
        // The start-up pictures do not count, however slow.
        let mut t = Trial::default();
        for _ in 0..TRIAL_SKIP {
            assert_eq!(t.record(500.0, true), TrialStep::Timing);
        }
        assert_eq!(feed(&mut t, 100, 20.0, true), TrialStep::TryProcessor);
        // The processor is clearly faster: kept.
        let mut faster = t.clone();
        assert_eq!(feed(&mut faster, 100, 9.0, false), TrialStep::Use(false));
        // About as fast: the card is kept, it costs less power.
        let mut same = t.clone();
        assert_eq!(feed(&mut same, 100, 18.0, false), TrialStep::Use(true));
        // Slower: the card.
        assert_eq!(feed(&mut t, 100, 40.0, false), TrialStep::Use(true));
    }

    #[test]
    fn the_way_learnt_for_a_file_is_used_when_it_is_opened_again() -> Result<(), String> {
        let Some(path) = test_clip("picasso_decode_learnt.mp4") else {
            return Ok(());
        };
        learn_faster_way(&path, false);
        let d = VideoDecoder::open_with(&path, true).map_err(|e| e.to_string())?;
        // Asked for the card, but the processor was found faster for this file: no trial again.
        assert!(!d.on_card);
        assert!(d.trial.is_none());
        assert!(!d.is_stale());
        let _ = std::fs::remove_file(path);
        Ok(())
    }

    #[test]
    fn the_picture_held_answers_the_next_requests_without_decoding() -> Result<(), String> {
        let Some(path) = test_clip("picasso_decode_held.mp4") else {
            return Ok(());
        };
        let mut d = VideoDecoder::open_with(&path, false).map_err(|e| e.to_string())?;
        // Nothing is held before the first read.
        assert_eq!(d.held_picture_at(0), None);
        let first = d.frame_at_reduced(0, 1).map_err(|e| e.to_string())?;
        // 24 fps footage: a 60 fps project asks at 0, 16.7 and 33.3 ms; the picture at 0 lasts
        // until 41.7 ms.
        assert_eq!(d.held_picture_at(16_667), Some(first.pts_us));
        assert_eq!(d.held_picture_at(33_333), Some(first.pts_us));
        // Not the next picture, and not a time before the held one.
        assert_eq!(d.held_picture_at(50_000), None);
        let _ = std::fs::remove_file(path);
        Ok(())
    }

    #[test]
    fn profile_names_read_like_words() {
        use ff::codec::profile::{Profile, H264, HEVC};
        assert_eq!(
            profile_name(Profile::H264(H264::ConstrainedBaseline)).as_deref(),
            Some("Constrained Baseline")
        );
        assert_eq!(
            profile_name(Profile::H264(H264::High10)).as_deref(),
            Some("High 10")
        );
        assert_eq!(
            profile_name(Profile::HEVC(HEVC::Main10)).as_deref(),
            Some("Main 10")
        );
        assert_eq!(profile_name(Profile::Unknown), None);
    }

    #[test]
    fn probes_and_decodes_a_clip() -> Result<(), String> {
        let Some(path) = test_clip("picasso_decode_test.mp4") else {
            return Ok(());
        };
        let info = probe(&path).map_err(|e| e.to_string())?;
        assert_eq!((info.width, info.height, info.fps), (320, 180, (24, 1)));
        let label = info.codec_label();
        assert!(label.starts_with("H.264 High L"), "{label}");
        let mut dec = VideoDecoder::open(&path).map_err(|e| e.to_string())?;
        let frame = dec.frame_at(1_000_000).map_err(|e| e.to_string())?;
        assert_eq!((frame.width, frame.rgba.len()), (320, 320 * 180 * 4));
        // 24 fps: one second is where frame 24 starts.
        assert_eq!(frame.pts_us / 1000, 1000);
        // A proxy at a quarter of the size, for the same moment.
        let small = dec
            .frame_at_reduced(1_000_000, 4)
            .map_err(|e| e.to_string())?;
        assert_eq!((small.width, small.height), (80, 45));
        assert_eq!(small.rgba.len(), 80 * 45 * 4);
        assert_eq!(small.pts_us, frame.pts_us);
        let _ = std::fs::remove_file(path);
        Ok(())
    }

    #[test]
    fn the_card_and_the_processor_give_the_same_pictures() -> Result<(), String> {
        let Some(path) = test_clip("picasso_decode_hw.mp4") else {
            return Ok(());
        };
        let mut card = VideoDecoder::open_with(&path, true).map_err(|e| e.to_string())?;
        let mut cpu = VideoDecoder::open_with(&path, false).map_err(|e| e.to_string())?;
        assert!(!cpu.on_card);
        for t in [0, 500_000, 1_000_000, 1_958_333] {
            let a = card.frame_at(t).map_err(|e| e.to_string())?;
            let b = cpu.frame_at(t).map_err(|e| e.to_string())?;
            assert_eq!((a.width, a.height, a.pts_us), (b.width, b.height, b.pts_us));
            // The card hands over NV12 rather than planar YUV, so colour edges round a little
            // differently (about one level in 255 on average); a wrong picture is far off.
            let total: u64 = a
                .rgba
                .iter()
                .zip(&b.rgba)
                .map(|(x, y)| u64::from(x.abs_diff(*y)))
                .sum();
            let mean = total as f64 / a.rgba.len() as f64;
            assert!(
                mean < 3.0,
                "{mean} levels apart at {t} (card: {})",
                card.on_card
            );
        }
        let _ = std::fs::remove_file(path);
        Ok(())
    }

    #[test]
    fn playing_past_the_end_and_looping_back_keeps_decoding() -> Result<(), String> {
        let Some(path) = test_clip("picasso_decode_loop.mp4") else {
            return Ok(());
        };
        let mut dec = VideoDecoder::open(&path).map_err(|e| e.to_string())?;
        // Play the last second forward, then on past the end, as reading ahead does.
        let mut last = 0;
        for k in 0..40 {
            let t = 1_000_000 + k * 41_667;
            let frame = dec.frame_at(t).map_err(|e| format!("at {t}: {e}"))?;
            last = frame.pts_us;
        }
        assert!(last > 1_900_000, "the last picture stays: {last}");
        // A loop goes back to the start.
        let first = dec.frame_at(0).map_err(|e| e.to_string())?;
        assert_eq!(first.pts_us, 0);
        let next = dec.frame_at(41_667).map_err(|e| e.to_string())?;
        assert!(next.pts_us > 0);
        let _ = std::fs::remove_file(path);
        Ok(())
    }
}

#[cfg(test)]
mod refuse_tests {
    use super::*;

    #[test]
    fn addresses_and_network_paths_are_refused_but_files_are_not() {
        for bad in [
            "http://example.com/a.mp4",
            "https://example.com/a.mp4",
            "ftp://h/a.mp4",
            "concat:/etc/passwd|/etc/hosts",
            "pipe:0",
            "data:video/mp4;base64,AAAA",
            "file:///etc/passwd",
            "subfile,,start,0,end,10,,:/etc/passwd",
            r"\\server\share\clip.mov",
            "//server/share/clip.mov",
            "  http://padded.example/a.mp4",
        ] {
            assert!(refuse_path(bad).is_some(), "{bad} should be refused");
        }
        for fine in [
            r"C:\Videos\clip.mp4",
            "C:/Videos/clip.mp4",
            "/home/me/clip.mp4",
            "clip.mp4",
            "relative/dir/clip.mov",
            "/Volumes/Media/shot 1.mov",
        ] {
            assert!(refuse_path(fine).is_none(), "{fine} should be allowed");
        }
    }

    #[test]
    fn opening_a_refused_path_is_an_error_not_a_network_request() {
        let err = probe(Path::new("http://127.0.0.1:9/a.mp4")).unwrap_err();
        assert!(matches!(err, DecodeError::Refused(_)), "{err}");
    }
}
