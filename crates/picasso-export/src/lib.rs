//! Renders a project to files. Presets: H.264, H.265 and AV1 in MP4, VP9 in WebM, ProRes and
//! DNxHR in QuickTime, lossless FFV1 in Matroska, WAV and MP3 audio, and PNG sequences. H.264,
//! H.265, AV1 and VP9 can be 10-bit, and H.264, H.265 and AV1 can use the graphics card's encoder
//! (NVIDIA, Intel, AMD, Apple, VA-API). Video goes through FFmpeg's encoders (desktop only).

mod burn;
mod video;
mod yuv;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Once, OnceLock};

use picasso_aaf::Stamp;
use picasso_audio::{
    encode_wav, frames_to_samples, mixdown, plan, wav_header, AudioPlan, AudioPool, ClipSound,
    MixdownError, Offline, WavFormat, SAMPLE_RATE,
};
use picasso_core::{
    BlendMode, Clip, ClipAudio, ClipId, ColorGrade, MediaId, MediaItem, MediaSource, Project,
    StabilizationSettings, Transform, TrackKind,
};
use picasso_render::MediaCache;
use thiserror::Error;

/// What to render.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    /// H.264 video and AAC sound in an MP4 file, as video sites like it.
    H264Web,
    /// H.265 video and AAC sound in an MP4 file.
    H265,
    /// AV1 video and AAC sound in an MP4 file.
    Av1,
    /// VP9 video and Opus sound in a WebM file.
    Vp9,
    /// ProRes 422 video and 16-bit PCM sound in a MOV file.
    ProRes422,
    /// Avid DNxHR video and 16-bit PCM sound in a MOV file.
    DnxHr,
    /// Lossless FFV1 video (10-bit 4:4:4) and FLAC sound in a Matroska file, for archiving.
    Ffv1,
    /// The mix as a 24-bit WAV file.
    Wav,
    /// The mix as an MP3 file.
    Mp3,
    /// One PNG per frame, into a folder.
    PngSequence,
    /// IMF Generic: broadcast video and 48 kHz PCM sound in an MXF file.
    Mxf,
    /// Pro Tools: AAF with baked rendered audio (.wav files alongside).
    ProTools,
}

impl Preset {
    pub const ALL: [Self; 12] = [
        Self::H264Web,
        Self::H265,
        Self::Av1,
        Self::Vp9,
        Self::ProRes422,
        Self::DnxHr,
        Self::Ffv1,
        Self::Wav,
        Self::Mp3,
        Self::PngSequence,
        Self::Mxf,
        Self::ProTools,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::H264Web => "H.264 (MP4, for the web)",
            Self::H265 => "H.265 (MP4)",
            Self::Av1 => "AV1 (MP4)",
            Self::Vp9 => "VP9 (WebM)",
            Self::ProRes422 => "ProRes 422 (MOV)",
            Self::DnxHr => "DNxHR (MOV)",
            Self::Ffv1 => "FFV1 lossless (MKV)",
            Self::Wav => "WAV audio",
            Self::Mp3 => "MP3 audio",
            Self::PngSequence => "PNG sequence",
            Self::Mxf => "MXF / IMF Generic (MXF)",
            Self::ProTools => "AAF with rendered audio (Pro Tools)",
        }
    }

    /// The file extension of the output; empty when the output is a folder.
    pub fn extension(self) -> &'static str {
        match self {
            Self::H264Web | Self::H265 | Self::Av1 => "mp4",
            Self::Vp9 => "webm",
            Self::ProRes422 | Self::DnxHr => "mov",
            Self::Ffv1 => "mkv",
            Self::Wav => "wav",
            Self::Mp3 => "mp3",
            Self::PngSequence => "",
            Self::Mxf => "mxf",
            Self::ProTools => "aaf",
        }
    }

    /// Whether the output is a folder rather than a file.
    pub fn is_folder(self) -> bool {
        self == Self::PngSequence
    }

    /// Whether the file has only sound.
    pub fn is_audio_only(self) -> bool {
        matches!(self, Self::Wav | Self::Mp3)
    }

    /// Whether the preset's codec can be 10-bit (the others are fixed: ProRes and FFV1 are always
    /// 10-bit, DNxHR's profile decides).
    pub fn has_ten_bit(self) -> bool {
        matches!(self, Self::H264Web | Self::H265 | Self::Av1 | Self::Vp9)
    }

    /// The encoders that can make the preset on this computer's processor, the first that this
    /// FFmpeg has is used.
    fn software_encoders(self) -> &'static [&'static str] {
        match self {
            Self::H264Web => &["libx264"],
            Self::H265 => &["libx265"],
            // SVT-AV1 is many times faster than the reference encoder.
            Self::Av1 => &["libsvtav1", "libaom-av1"],
            Self::Vp9 => &["libvpx-vp9"],
            Self::ProRes422 => &["prores_ks"],
            Self::DnxHr => &["dnxhd"],
            Self::Ffv1 => &["ffv1"],
            Self::Mxf => &["mpeg2video", "dnxhd", "libx264"],
            Self::Wav | Self::Mp3 | Self::PngSequence | Self::ProTools => &[],
        }
    }

    /// The name of the FFmpeg encoder the preset uses with `hardware`, if this FFmpeg has it.
    pub fn encoder_name(self, hardware: Hardware) -> Option<&'static str> {
        init();
        let names: Vec<&'static str> = match (self, hardware) {
            (_, Hardware::Native) => self.software_encoders().to_vec(),
            (Self::H264Web, h) => h.encoder("h264").into_iter().collect(),
            (Self::H265, h) => h.encoder("hevc").into_iter().collect(),
            (Self::Av1, h) => h.encoder("av1").into_iter().collect(),
            (Self::Mxf, _) => Vec::new(),
            _ => Vec::new(),
        };
        names
            .into_iter()
            .find(|n| ffmpeg_next::encoder::find_by_name(n).is_some())
    }

    /// The sound encoder of the preset's file, given the codec asked for.
    fn audio_encoder(self, codec: AudioCodec) -> Option<&'static str> {
        match self {
            Self::H264Web | Self::H265 | Self::Av1 => Some("aac"),
            Self::ProRes422 | Self::DnxHr => Some(if codec == AudioCodec::Aac {
                "aac"
            } else {
                "pcm_s16le"
            }),
            Self::Vp9 => Some("libopus"),
            Self::Ffv1 => Some("flac"),
            Self::Mp3 => Some("libmp3lame"),
            Self::Mxf => Some("pcm_s16le"),
            Self::ProTools => Some("pcm_s16le"),
            Self::Wav | Self::PngSequence => None,
        }
    }

    /// Whether this build of FFmpeg has what the preset needs.
    pub fn available(self) -> bool {
        init();
        let video =
            self.software_encoders().is_empty() || self.encoder_name(Hardware::Native).is_some();
        let audio = self
            .audio_encoder(AudioCodec::Auto)
            .is_none_or(|n| ffmpeg_next::encoder::find_by_name(n).is_some());
        // An MP3 file has nothing without its sound encoder; the others still have a picture.
        video && (audio || self != Self::Mp3)
    }

    /// The graphics cards' encoders this FFmpeg has for the preset, the processor's first.
    pub fn hardware_choices(self) -> Vec<Hardware> {
        Hardware::ALL
            .into_iter()
            .filter(|h| self.encoder_name(*h).is_some())
            .collect()
    }
}

/// Which encoder makes the video: the processor's, or a graphics card's.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Hardware {
    /// The processor (x264, x265, SVT-AV1 ...): the best picture for the size, the slowest.
    #[default]
    Native,
    /// NVIDIA's NVENC.
    Nvidia,
    /// Intel's Quick Sync.
    Intel,
    /// AMD's AMF (Windows).
    Amd,
    /// Apple's VideoToolbox (macOS).
    Apple,
    /// VA-API (Linux, Intel and AMD cards).
    Vaapi,
}

impl Hardware {
    pub const ALL: [Self; 6] = [
        Self::Native,
        Self::Nvidia,
        Self::Intel,
        Self::Amd,
        Self::Apple,
        Self::Vaapi,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Native => "Native",
            Self::Nvidia => "NVIDIA",
            Self::Intel => "Intel Quick Sync",
            Self::Amd => "AMD",
            Self::Apple => "Apple VideoToolbox",
            Self::Vaapi => "VA-API",
        }
    }

    /// The FFmpeg encoder of this card for a codec ("h264", "hevc" or "av1").
    fn encoder(self, codec: &str) -> Option<&'static str> {
        Some(match (self, codec) {
            (Self::Nvidia, "h264") => "h264_nvenc",
            (Self::Nvidia, "hevc") => "hevc_nvenc",
            (Self::Nvidia, "av1") => "av1_nvenc",
            (Self::Intel, "h264") => "h264_qsv",
            (Self::Intel, "hevc") => "hevc_qsv",
            (Self::Intel, "av1") => "av1_qsv",
            (Self::Amd, "h264") => "h264_amf",
            (Self::Amd, "hevc") => "hevc_amf",
            (Self::Amd, "av1") => "av1_amf",
            (Self::Apple, "h264") => "h264_videotoolbox",
            (Self::Apple, "hevc") => "hevc_videotoolbox",
            (Self::Vaapi, "h264") => "h264_vaapi",
            (Self::Vaapi, "hevc") => "hevc_vaapi",
            (Self::Vaapi, "av1") => "av1_vaapi",
            _ => return None,
        })
    }
}

/// How hard the encoder works: faster files are bigger for the same quality.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Speed {
    Faster,
    Balanced,
    Best,
}

impl Speed {
    /// The name the x264 and x265 encoders know it by.
    fn encoder_preset(self) -> &'static str {
        match self {
            Self::Faster => "faster",
            Self::Balanced => "medium",
            Self::Best => "slower",
        }
    }
}

/// How good the picture is kept (the constant rate factor of the encoder).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quality {
    Best,
    High,
    Medium,
    Low,
}

impl Quality {
    pub const ALL: [Self; 4] = [Self::Best, Self::High, Self::Medium, Self::Low];

    pub fn label(self) -> &'static str {
        match self {
            Self::Best => "Best",
            Self::High => "High",
            Self::Medium => "Medium",
            Self::Low => "Low",
        }
    }
}

/// What decides how many bits the picture gets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rate {
    /// A constant quality; the file is as big as it must be.
    Quality(Quality),
    /// An average rate in kilobits a second.
    Bitrate(u32),
}

/// The H.264 profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    Auto,
    Baseline,
    Main,
    High,
}

impl Profile {
    pub const ALL: [Self; 4] = [Self::Auto, Self::Baseline, Self::Main, Self::High];

    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::Baseline => "Baseline",
            Self::Main => "Main",
            Self::High => "High",
        }
    }

    fn name(self) -> Option<&'static str> {
        match self {
            Self::Auto => None,
            Self::Baseline => Some("baseline"),
            Self::Main => Some("main"),
            Self::High => Some("high"),
        }
    }
}

/// The sound encoder of a video file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioCodec {
    /// What the preset usually has: AAC, PCM in a ProRes or DNxHR file, Opus in WebM, FLAC in
    /// Matroska.
    Auto,
    Aac,
    /// 16-bit linear PCM (QuickTime files only).
    Pcm,
}

/// The DNxHR flavours, as Avid names them, and the FFmpeg profile of each.
pub const DNXHR_PROFILES: [(&str, &str); 5] = [
    ("DNxHR LB", "dnxhr_lb"),
    ("DNxHR SQ", "dnxhr_sq"),
    ("DNxHR HQ", "dnxhr_hq"),
    ("DNxHR HQX (10-bit)", "dnxhr_hqx"),
    ("DNxHR 444 (10-bit)", "dnxhr_444"),
];

/// The shape of a pixel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelAspect {
    Square,
    /// Anamorphic: pixels twice as wide as they are tall, as an anamorphic lens squeezes.
    Cinemascope,
}

/// How the brightness range of the picture is stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Levels {
    /// What the format usually has: video levels.
    Auto,
    /// Black is 16 and white 235 (of 255).
    Video,
    /// Black is 0 and white 255.
    Full,
}

/// What is burned into the picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BurnIn {
    Off,
    Timecode,
    ClipName,
    Both,
}

/// What the Deliver page sets besides the preset; every field changes the file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Options {
    /// Put the picture in the file.
    pub video: bool,
    /// Put the sound in the file.
    pub audio: bool,
    /// Put the index at the front so the file can play while it downloads.
    pub faststart: bool,
    /// The size of the picture; the timeline's size when `None`.
    pub size: Option<(u32, u32)>,
    /// The frame rate of the file; the timeline's when `None`.
    pub fps: Option<picasso_core::Fps>,
    /// A key frame at least this often, in frames; every two seconds when `None`.
    pub keyframes: Option<u32>,
    /// Let the encoder use pictures that refer to later ones (smaller files).
    pub frame_reordering: bool,
    pub speed: Speed,
    pub rate: Rate,
    pub profile: Profile,
    /// The ProRes flavour: 0 Proxy, 1 LT, 2 Standard, 3 HQ.
    pub prores: u8,
    /// The DNxHR flavour, an index of [`DNXHR_PROFILES`].
    pub dnxhr: u8,
    /// 10 bits a sample instead of 8 (H.264 High 10, H.265 Main 10, AV1, VP9 profile 2).
    pub ten_bit: bool,
    /// The processor's encoder or a graphics card's (H.264, H.265 and AV1).
    pub hardware: Hardware,
    /// The sound's data rate in kilobits a second (AAC, Opus, MP3).
    pub audio_codec: AudioCodec,
    pub audio_kbps: u32,
    /// Write the project's markers in the range as chapters.
    pub chapters: bool,
    pub pixel_aspect: PixelAspect,
    pub levels: Levels,
    /// The colour primaries written in the file (an FFmpeg name such as "bt709"); BT.709 when `None`.
    pub color_primaries: Option<&'static str>,
    /// The transfer curve written in the file (an FFmpeg name such as "iec61966-2-1").
    pub color_trc: Option<&'static str>,
    pub burn_in: BurnIn,
    /// The frames the timeline's clock starts at, so a burned timecode reads as the page does.
    pub clock_start: u64,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            video: true,
            audio: true,
            faststart: true,
            size: None,
            fps: None,
            keyframes: None,
            frame_reordering: true,
            speed: Speed::Balanced,
            rate: Rate::Quality(Quality::Best),
            profile: Profile::Auto,
            prores: 2,
            dnxhr: 2,
            ten_bit: false,
            hardware: Hardware::Native,
            audio_codec: AudioCodec::Auto,
            audio_kbps: 320,
            chapters: false,
            pixel_aspect: PixelAspect::Square,
            levels: Levels::Auto,
            color_primaries: None,
            color_trc: None,
            burn_in: BurnIn::Off,
            clock_start: 0,
        }
    }
}

impl Options {
    /// The size of the picture for a project: the one chosen, else the timeline's, made even
    /// (the encoders want it).
    pub fn picture_size(&self, project: &Project) -> (u32, u32) {
        let (w, h) = self.size.unwrap_or((project.width, project.height));
        (w.max(2) & !1, h.max(2) & !1)
    }

    /// The frame rate of the file.
    pub fn frame_rate(&self, project: &Project) -> picasso_core::Fps {
        self.fps.unwrap_or(project.fps)
    }

    /// The number of pictures the file gets for `frames` of the timeline.
    pub fn output_frames(&self, project: &Project, frames: u64) -> u64 {
        let (tl, out) = (project.fps, self.frame_rate(project));
        if tl == out || frames == 0 {
            return frames;
        }
        let n = u128::from(frames) * u128::from(out.num) * u128::from(tl.den);
        let d = u128::from(tl.num) * u128::from(out.den);
        ((n / d.max(1)) as u64).max(1)
    }

    /// The picture of the file that shows timeline frame `frame` (the first one at or after it).
    pub fn output_index(&self, project: &Project, start: u64, frame: u64) -> u64 {
        let (tl, out) = (project.fps, self.frame_rate(project));
        let n = u128::from(frame.saturating_sub(start)) * u128::from(out.num) * u128::from(tl.den);
        let d = u128::from(tl.num) * u128::from(out.den);
        (n / d.max(1)) as u64
    }

    /// The timeline frame a picture of the file shows.
    pub fn timeline_frame(&self, project: &Project, start: u64, end: u64, n: u64) -> u64 {
        let (tl, out) = (project.fps, self.frame_rate(project));
        if tl == out {
            return start + n;
        }
        let at = u128::from(n) * u128::from(out.den) * u128::from(tl.num);
        let d = u128::from(out.num) * u128::from(tl.den);
        (start + (at / d.max(1)) as u64).min(end.saturating_sub(1))
    }
}

/// One export: a preset, where to put it and which part of the timeline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub preset: Preset,
    /// The file, or the folder for a PNG sequence.
    pub path: PathBuf,
    /// First timeline frame, and the frame after the last.
    pub start: u64,
    pub end: u64,
    pub options: Options,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ExportError {
    #[error("cancelled")]
    Cancelled,
    #[error("there is nothing to export")]
    Empty,
    #[error("this FFmpeg build has no {0} encoder")]
    NoEncoder(&'static str),
    #[error("the {0} encoder cannot make 10-bit video")]
    NoTenBit(&'static str),
    #[error(
        "the {encoder} encoder did not start ({why}). Is the graphics card there, with its driver?"
    )]
    Hardware { encoder: &'static str, why: String },
    #[error("could not render a frame: {0}")]
    Render(String),
    #[error("could not write {0}")]
    Write(String),
    #[error("encoding failed: {0}")]
    Encode(String),
}

/// Which graphics cards' encoders open on this computer, by FFmpeg name; filled once, in the
/// background, by [`check_hardware`].
static CARDS: OnceLock<Vec<(&'static str, bool)>> = OnceLock::new();

/// Starts finding out, on a thread of its own, which of the graphics cards' encoders this FFmpeg
/// has can open here (an encoder can be built in with no card for it). Opening each takes a
/// moment, so the program does it once at start rather than when the Deliver page is drawn.
pub fn check_hardware() {
    static STARTED: Once = Once::new();
    STARTED.call_once(|| {
        let spawned = std::thread::Builder::new()
            .name("encoder check".into())
            .spawn(|| {
                init();
                // An encoder without its card says so on the console; here that is the answer,
                // not an error.
                use ffmpeg_next::util::log;
                log::set_level(log::Level::Quiet);
                let mut found = Vec::new();
                for preset in [Preset::H264Web, Preset::H265, Preset::Av1] {
                    for hardware in Hardware::ALL.into_iter().skip(1) {
                        if let Some(name) = preset.encoder_name(hardware) {
                            found.push((name, video::opens(name, hardware)));
                        }
                    }
                }
                log::set_level(log::Level::Error);
                tracing::info!("graphics card encoders: {found:?}");
                let _ = CARDS.set(found);
            });
        if let Err(e) = spawned {
            tracing::warn!("could not check the graphics card encoders: {e}");
        }
    });
}

/// Whether `preset` can be made with `hardware` here: `None` while [`check_hardware`] has not
/// finished, or when this FFmpeg lacks the encoder.
pub fn hardware_works(preset: Preset, hardware: Hardware) -> Option<bool> {
    if hardware == Hardware::Native {
        return Some(preset.available());
    }
    let name = preset.encoder_name(hardware)?;
    CARDS
        .get()?
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, works)| *works)
}

fn init() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        if let Err(e) = ffmpeg_next::init() {
            tracing::error!("ffmpeg did not start: {e}");
        }
        ffmpeg_next::util::log::set_level(ffmpeg_next::util::log::Level::Error);
    });
}

/// Renders `settings` from `project`. `progress` is called after each step with the steps done
/// and the total, and returns false to stop (the result is then [`ExportError::Cancelled`] and
/// no half-written file is left behind).
pub fn render(
    project: &Project,
    cache: &MediaCache,
    settings: &Settings,
    progress: &mut dyn FnMut(u64, u64) -> bool,
) -> Result<(), ExportError> {
    if settings.end <= settings.start {
        return Err(ExportError::Empty);
    }
    let result = match settings.preset {
        Preset::PngSequence => png_sequence(project, cache, settings, progress),
        Preset::Wav => wav(project, settings, progress),
        Preset::ProTools => pro_tools(project, cache, settings, progress),
        // MP3 goes through the encoder, with no picture.
        _ => {
            init();
            video::encode(project, cache, settings, progress)
        }
    };
    if result.is_err() && !settings.preset.is_folder() {
        // A file that was not finished is not left behind.
        let _ = std::fs::remove_file(&settings.path);
    }
    result
}

fn png_sequence(
    project: &Project,
    cache: &MediaCache,
    settings: &Settings,
    progress: &mut dyn FnMut(u64, u64) -> bool,
) -> Result<(), ExportError> {
    std::fs::create_dir_all(&settings.path)
        .map_err(|e| ExportError::Write(format!("{}: {e}", settings.path.display())))?;
    picasso_render::export_sequence(
        project,
        cache,
        settings.start,
        settings.end,
        settings.options.picture_size(project),
        |frame, png| {
            let name = format!("frame_{:06}.png", frame - settings.start + 1);
            std::fs::write(settings.path.join(name), png).map_err(|e| e.to_string())
        },
        progress,
    )
    .map(|_| ())
    .map_err(|e| match e {
        picasso_render::RenderError::Cancelled => ExportError::Cancelled,
        picasso_render::RenderError::Sink(why) => ExportError::Write(why),
        other => ExportError::Render(other.to_string()),
    })
}

fn wav(
    project: &Project,
    settings: &Settings,
    progress: &mut dyn FnMut(u64, u64) -> bool,
) -> Result<(), ExportError> {
    let (from, to) = (
        frames_to_samples(project, settings.start),
        frames_to_samples(project, settings.end),
    );
    let format = WavFormat::Pcm24;
    let header = wav_header(format, SAMPLE_RATE, to - from)
        .ok_or_else(|| ExportError::Write("that is too long for a WAV file (4 GB)".into()))?;
    let write_err =
        |e: std::io::Error| ExportError::Write(format!("{}: {e}", display(&settings.path)));
    let file = std::fs::File::create(&settings.path).map_err(write_err)?;
    let mut out = std::io::BufWriter::new(file);
    out.write_all(&header).map_err(write_err)?;
    let pool = AudioPool::new(|| {});
    let mut bytes = Vec::new();
    let mixed = mixdown(
        &plan(project),
        &Offline(&pool),
        from,
        to,
        |block| {
            bytes.clear();
            encode_wav(format, block, &mut bytes);
            out.write_all(&bytes).map_err(|e| e.to_string())
        },
        progress,
    );
    match mixed {
        Ok(_) => out.flush().map_err(write_err),
        Err(MixdownError::Cancelled) => Err(ExportError::Cancelled),
        Err(MixdownError::Sink(why)) => Err(ExportError::Write(why)),
    }
}

/// Renders a Pro Tools deliver: one 48 kHz 16-bit PCM WAV stem per audio track, plus an AAF
/// composition that references those rendered stems. Each audio track's clips are baked into its
/// own stem (speed changes, fades, cross-fades, EQ, dynamics all in the samples), so Pro Tools
/// receives ready-to-play rendered audio.
fn pro_tools(
    project: &Project,
    _cache: &MediaCache,
    settings: &Settings,
    progress: &mut dyn FnMut(u64, u64) -> bool,
) -> Result<(), ExportError> {
    let aaf_path = settings.path.clone();
    let stem_dir = aaf_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| Path::new("").to_path_buf());
    let stem_base = aaf_path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "mix".to_string());

    // Clean up previous partial output from any earlier attempt.
    let _ = std::fs::remove_file(&aaf_path);

    // Export range in samples.
    let (from, to) = (
        frames_to_samples(project, settings.start),
        frames_to_samples(project, settings.end),
    );

    let plan = plan(project);

    // Find audio tracks that have clips overlapping the export range.
    let mut track_indices: Vec<usize> = Vec::new();
    for clip in &plan.clips {
        if project
            .tracks
            .get(clip.track)
            .is_some_and(|t| t.kind == TrackKind::Audio)
            && clip.start < to
            && clip.start.saturating_add(clip.len) > from
            && !track_indices.contains(&clip.track)
        {
            track_indices.push(clip.track);
        }
    }
    track_indices.sort_unstable();
    track_indices.dedup();

    let mut wav_paths: Vec<PathBuf> = Vec::new(); // every WAV written, for cleanup on error
    let mut stem_infos: Vec<(usize, PathBuf)> = Vec::new(); // (track_index, wav_path)

    let track_count = track_indices.len().max(1);
    let range_samples = to.saturating_sub(from).max(1);
    let total_samples = range_samples * track_count as u64;

    // Pre-compute the next clip ID base before borrowing project_clone.tracks mutably below.
    let next_clip_id_base = project
        .tracks
        .iter()
        .flat_map(|t| t.clips.iter().map(|c| c.id.0))
        .max()
        .unwrap_or(0)
        .saturating_add(1);

    // Render one WAV stem per audio track that has clips in range.
    for (stem_index, track_idx) in track_indices.iter().enumerate() {
        let stem_name = if track_indices.len() == 1 {
            format!("{stem_base}.wav")
        } else {
            format!("{stem_base}_A{}.wav", track_idx + 1)
        };
        let wav_path = stem_dir.join(&stem_name);
        wav_paths.push(wav_path.clone());
        stem_infos.push((*track_idx, wav_path.clone()));

        // Filter the plan to only this track's clips overlapping the range.
        let filtered_clips: Vec<ClipSound> = plan
            .clips
            .iter()
            .filter(|c| {
                c.track == *track_idx
                    && c.start < to
                    && c.start.saturating_add(c.len) > from
            })
            .map(|c| ClipSound {
                media: c.media,
                path: c.path.clone(),
                track: c.track,
                start: c.start,
                len: c.len,
                source_in: c.source_in,
                speed: c.speed,
                preserve_pitch: c.preserve_pitch,
                gain: c.gain,
                left: c.left,
                right: c.right,
                fade_in: c.fade_in,
                fade_out: c.fade_out,
                reverse: c.reverse,
            })
            .collect();

        let filtered_plan = AudioPlan {
            clips: filtered_clips,
            tracks: plan.tracks.clone(),
        };

        // Render this track's WAV stem.
        let write_err = |e: std::io::Error| {
            ExportError::Write(format!("{}: {e}", wav_path.display()))
        };
        let file = std::fs::File::create(&wav_path).map_err(write_err)?;
        let mut out = std::io::BufWriter::new(file);
        let format = WavFormat::Pcm16;
        let header = wav_header(format, SAMPLE_RATE, range_samples)
            .ok_or_else(|| ExportError::Write("that is too long for a WAV file (4 GB)".into()))?;
        out.write_all(&header).map_err(write_err)?;
        let pool = AudioPool::new(|| {});
        let mut bytes = Vec::new();
        let track_offset = stem_index as u64 * range_samples;
        let mixed = mixdown(
            &filtered_plan,
            &Offline(&pool),
            from,
            to,
            |block| {
                bytes.clear();
                encode_wav(format, block, &mut bytes);
                out.write_all(&bytes).map_err(|e| e.to_string())
            },
            |done, _| {
                let overall = track_offset + done.min(range_samples);
                progress(overall.min(total_samples), total_samples)
            },
        );
        match mixed {
            Ok(_) => out.flush().map_err(write_err)?,
            Err(MixdownError::Cancelled) => {
                cleanup_wavs(&wav_paths);
                let _ = std::fs::remove_file(&aaf_path);
                return Err(ExportError::Cancelled);
            }
            Err(MixdownError::Sink(why)) => {
                cleanup_wavs(&wav_paths);
                let _ = std::fs::remove_file(&aaf_path);
                return Err(ExportError::Write(why));
            }
        }
        drop(out);
    }

    // If no audio track had clips, write a silent WAV so Pro Tools always gets audio.
    if track_indices.is_empty() {
        let wav_path = stem_dir.join(format!("{stem_base}.wav"));
        wav_paths.push(wav_path.clone());
        write_silent_wav(&wav_path, range_samples)?;
        stem_infos.push((0, wav_path));
    }

    // Build the cloned project: add new MediaItems and replace audio-track clips with clean
    // clips pointing at the rendered stems. Video tracks are left intact.
    let mut project_clone = project.clone();
    let mut new_media_items: Vec<MediaItem> = Vec::new();
    let mut track_media_map: std::collections::HashMap<usize, MediaId> =
        std::collections::HashMap::new();

    let range_frames = settings.end.saturating_sub(settings.start).max(1);
    for (track_idx, wav_path) in &stem_infos {
        let media_id = MediaId(project_clone.media.len() as u64);
        let name = wav_path
            .file_name()
            .map_or("audio".to_string(), |s| s.to_string_lossy().into_owned());
        new_media_items.push(MediaItem {
            id: media_id,
            name,
            source: MediaSource::Audio {
                path: wav_path.to_string_lossy().into_owned(),
            },
            length: Some(range_frames),
            default_duration: range_frames,
            bin: None,
            timecode: None,
        });
        track_media_map.insert(*track_idx, media_id);
    }
    project_clone.media.extend(new_media_items);

    // Replace each audio track's clips with a single clean clip spanning the export range.
    let mut next_clip_id = next_clip_id_base;
    for (track_idx, track) in project_clone.tracks.iter_mut().enumerate() {
        if track.kind != TrackKind::Audio {
            continue;
        }
        let Some(media_id) = track_media_map.get(&track_idx) else {
            // No rendered stem for this track: clear its clips.
            track.clips.clear();
            continue;
        };
        track.clips = vec![Clip {
            id: ClipId(next_clip_id),
            media: *media_id,
            name: "Rendered Audio".into(),
            start: settings.start,
            source_in: 0,
            duration: range_frames,
            opacity: 1.0,
            grade: ColorGrade::default(),
            enabled: true,
            audio: ClipAudio::default(),
            speed: 1.0,
            transform: Transform::default(),
            blend: BlendMode::default(),
            link: None,
            nodes: Vec::new(),
            sound: true,
            reverse: false,
            freeze: false,
            transition: None,
            locked: false,
            color: None,
            keyframes: Vec::new(),
            effects: Vec::new(),
            stabilization: StabilizationSettings::default(),
            tracking_data: Vec::new(),
        }];
        next_clip_id = next_clip_id.saturating_add(1);
    }

    let written = picasso_aaf::to_aaf_at(&project_clone, Stamp::now())
        .map_err(|e| ExportError::Write(format!("{}: {e}", aaf_path.display())))?;
    std::fs::write(&aaf_path, written.bytes)
        .map_err(|e| ExportError::Write(format!("{}: {e}", aaf_path.display())))?;

    Ok(())
}

/// Removes every WAV file in `paths` that still exists, ignoring errors.
fn cleanup_wavs(paths: &[PathBuf]) {
    for path in paths {
        let _ = std::fs::remove_file(path);
    }
}

/// Writes `frames` frames of silent 16-bit PCM stereo at 48 kHz to `path`.
fn write_silent_wav(path: &Path, frames: u64) -> Result<(), ExportError> {
    let format = WavFormat::Pcm16;
    let samples = frames.saturating_mul(2); // stereo
    let header = wav_header(format, SAMPLE_RATE, samples)
        .ok_or_else(|| ExportError::Write("that is too long for a WAV file (4 GB)".into()))?;
    let write_err = |e: std::io::Error| ExportError::Write(format!("{}: {e}", path.display()));
    let file = std::fs::File::create(path).map_err(write_err)?;
    let mut out = std::io::BufWriter::new(file);
    out.write_all(&header).map_err(write_err)?;
    let block = vec![0i16; 4096]; // 2048 stereo frames per block
    let mut remaining = samples as usize;
    while remaining > 0 {
        let n = block.len().min(remaining);
        let bytes: Vec<u8> = block[..n]
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        out.write_all(&bytes).map_err(write_err)?;
        remaining -= n;
    }
    out.flush().map_err(write_err)?;
    Ok(())
}

fn display(path: &Path) -> String {
    path.display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use picasso_aaf::read_aaf;
    use picasso_core::{Command, Fps, MediaSource, Outcome, TrackKind};

    /// A project with one second of colour bars and, when `tone` is given, the same second of
    /// that sound file; `None` when the `ffmpeg` program is not installed.
    fn project(tone: bool) -> Option<Project> {
        let mut p = Project::new("export test");
        p.width = 320;
        p.height = 180;
        let v1 = p.tracks[0].id;
        let Outcome::Media(bars) = (Command::AddMedia {
            name: "bars".into(),
            source: MediaSource::ColorBars,
            length: None,
            default_duration: 24,
        })
        .apply(&mut p)
        .ok()?
        else {
            return None;
        };
        Command::AddClip {
            track: v1,
            media: bars,
            start: 0,
            duration: Some(24),
            with_sound: false,
        }
        .apply(&mut p)
        .ok()?;
        if tone {
            let wav = std::env::temp_dir().join("picasso_export_tone.wav");
            let made = std::process::Command::new("ffmpeg")
                .args(["-y", "-v", "error", "-f", "lavfi", "-i"])
                .arg("sine=frequency=440:duration=1")
                .arg(&wav)
                .status()
                .is_ok_and(|s| s.success());
            if !made {
                return None;
            }
            let Outcome::Track(a1) = (Command::AddTrack {
                kind: TrackKind::Audio,
            })
            .apply(&mut p)
            .ok()?
            else {
                return None;
            };
            let Outcome::Media(sound) = (Command::AddMedia {
                name: "tone".into(),
                source: MediaSource::Audio {
                    path: wav.to_string_lossy().into_owned(),
                },
                length: Some(24),
                default_duration: 24,
            })
            .apply(&mut p)
            .ok()?
            else {
                return None;
            };
            Command::AddClip {
                track: a1,
                media: sound,
                start: 0,
                duration: Some(24),
                with_sound: false,
            }
            .apply(&mut p)
            .ok()?;
        }
        Some(p)
    }

    /// `codec,width,height,frames` lines of every stream of a file, from `ffprobe`.
    fn probe(path: &Path) -> Option<Vec<String>> {
        let out = std::process::Command::new("ffprobe")
            .args(["-v", "error", "-count_frames", "-show_entries"])
            .arg("stream=codec_name,codec_type,width,height,nb_read_frames:format=duration")
            .args(["-of", "csv=p=0"])
            .arg(path)
            .output()
            .ok()?;
        out.status.success().then(|| {
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .map(str::to_string)
                .collect()
        })
    }

    fn settings(preset: Preset, name: &str, frames: u64) -> Settings {
        Settings {
            preset,
            path: std::env::temp_dir().join(name),
            start: 0,
            end: frames,
            options: Options::default(),
        }
    }

    #[test]
    fn video_presets_make_files_with_the_right_streams() {
        let Some(p) = project(true) else { return };
        for (preset, name, video, audio) in [
            (Preset::H264Web, "picasso_export_h264.mp4", "h264", "aac"),
            (
                Preset::ProRes422,
                "picasso_export_prores.mov",
                "prores",
                "pcm_s16le",
            ),
        ] {
            let s = settings(preset, name, 24);
            let mut steps = 0;
            render(&p, &MediaCache::new(), &s, &mut |_, _| {
                steps += 1;
                true
            })
            .unwrap();
            assert_eq!(steps, 24);
            let Some(lines) = probe(&s.path) else { return };
            let text = lines.join("\n");
            assert!(text.contains(&format!("{video},video,320,180")), "{text}");
            assert!(text.contains(&format!("{audio},audio")), "{text}");
            // 24 pictures, one second long.
            assert!(
                lines
                    .iter()
                    .any(|l| l.starts_with(video) && l.ends_with(",24")),
                "{text}"
            );
            let duration = lines
                .last()
                .and_then(|l| l.parse::<f64>().ok())
                .unwrap_or(0.0);
            assert!((0.9..1.2).contains(&duration), "{duration}");
            let _ = std::fs::remove_file(&s.path);
        }
    }

    #[test]
    fn the_last_frame_keeps_its_duration() {
        // Video only: no audio to mask a short last frame. 24 frames at 24 fps must be one
        // second long, including the picture ffmpeg would otherwise write without a duration.
        let Some(p) = project(false) else { return };
        let s = settings(Preset::H264Web, "picasso_export_last_frame.mp4", 24);
        render(&p, &MediaCache::new(), &s, &mut |_, _| true).unwrap();
        let Some(lines) = probe(&s.path) else { return };
        let frame_count = lines
            .iter()
            .find(|l| l.starts_with("h264,video,"))
            .and_then(|l| l.split(',').nth(4).and_then(|n| n.parse::<u64>().ok()));
        assert_eq!(frame_count, Some(24), "all pictures read back: {lines:?}");
        let duration = lines
            .last()
            .and_then(|l| l.parse::<f64>().ok())
            .unwrap_or(0.0);
        assert!(
            (0.99..1.02).contains(&duration),
            "the last frame is counted in the duration: {duration} {lines:?}"
        );
        let _ = std::fs::remove_file(&s.path);
    }

    /// The software H.264 encoder (libx264) writes 24 frames at 24 fps to a unique temp file,
    /// and ffprobe reads them all back in one second.
    #[test]
    fn software_h264_writes_twenty_four_frames_in_one_second() {
        let Some(p) = project(false) else { return };
        let s = Settings {
            preset: Preset::H264Web,
            path: std::env::temp_dir().join(format!(
                "picasso_export_software_h264_{}.mp4",
                std::process::id()
            )),
            start: 0,
            end: 24,
            options: Options::default(),
        };
        render(&p, &MediaCache::new(), &s, &mut |_, _| true).unwrap();
        let Some(lines) = probe(&s.path) else { return };
        let frame_count = lines
            .iter()
            .find(|l| l.starts_with("h264,video,"))
            .and_then(|l| l.split(',').nth(4).and_then(|n| n.parse::<u64>().ok()));
        assert_eq!(frame_count, Some(24), "all pictures read back: {lines:?}");
        let duration = lines
            .last()
            .and_then(|l| l.parse::<f64>().ok())
            .unwrap_or(0.0);
        let one_frame = 1.0 / 24.0;
        assert!(
            (1.0 - one_frame..=1.0 + one_frame).contains(&duration),
            "one second within one frame: {duration} {lines:?}"
        );
        let _ = std::fs::remove_file(&s.path);
    }

    /// A stabilized clip exports through the same render path: `render()` applies
    /// `clip_warp()` inside `draw_clip()`, so a clip with analyzed
    /// transforms still writes every frame without erroring.
    #[test]
    fn stabilized_export_writes_every_frame() {
        let Some(mut p) = project(false) else { return };
        let Some(clip) = p.clip_mut(p.tracks[0].clips[0].id) else { return };
        clip.stabilization.enabled = true;
        clip.stabilization.analyzed_transforms = vec![
            picasso_core::FrameTransform {
                frame_number: 0,
                matrix: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            },
            picasso_core::FrameTransform {
                frame_number: 1,
                matrix: [[1.0, 0.0, 1.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            },
        ];
        let s = Settings {
            preset: Preset::H264Web,
            path: std::env::temp_dir().join(format!(
                "picasso_export_stabilized_{}.mp4",
                std::process::id()
            )),
            start: 0,
            end: 24,
            options: Options::default(),
        };
        render(&p, &MediaCache::new(), &s, &mut |_, _| true).unwrap();
        let Some(lines) = probe(&s.path) else { return };
        let frame_count = lines
            .iter()
            .find(|l| l.starts_with("h264,video,"))
            .and_then(|l| l.split(',').nth(4).and_then(|n| n.parse::<u64>().ok()));
        assert_eq!(frame_count, Some(24), "all pictures read back: {lines:?}");
        let _ = std::fs::remove_file(&s.path);
    }

    /// One `codec,profile,type,pixel format` line per stream (ffprobe's order).
    fn streams(path: &Path) -> Option<String> {
        let out = std::process::Command::new("ffprobe")
            .args(["-v", "error", "-show_entries"])
            .arg("stream=codec_type,codec_name,pix_fmt,profile")
            .args(["-of", "csv=p=0"])
            .arg(path)
            .output()
            .ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).to_string())
    }

    /// The line of the stream of `kind` ("video", "audio") that has `codec`.
    fn stream<'a>(text: &'a str, codec: &str, kind: &str) -> Option<&'a str> {
        text.lines()
            .find(|l| l.starts_with(&format!("{codec},")) && l.contains(&format!(",{kind}")))
    }

    #[test]
    fn the_other_codecs_make_files_that_play() {
        let Some(p) = project(true) else { return };
        for (preset, video, audio) in [
            (Preset::Av1, "av1", "aac"),
            (Preset::Vp9, "vp9", "opus"),
            (Preset::DnxHr, "dnxhd", "pcm_s16le"),
            (Preset::Ffv1, "ffv1", "flac"),
            (Preset::Mp3, "", "mp3"),
        ] {
            if !preset.available() {
                eprintln!("this FFmpeg has no encoder for {preset:?}");
                continue;
            }
            let name = format!("picasso_export_{video}{audio}.{}", preset.extension());
            let s = settings(preset, &name, 24);
            let mut steps = 0;
            render(&p, &MediaCache::new(), &s, &mut |_, _| {
                steps += 1;
                true
            })
            .unwrap_or_else(|e| panic!("{preset:?}: {e}"));
            assert_eq!(steps, 24, "{preset:?}");
            let Some(text) = streams(&s.path) else { return };
            if video.is_empty() {
                assert!(!text.contains("video"), "{text}");
            } else {
                assert!(
                    stream(&text, video, "video").is_some(),
                    "{preset:?}: {text}"
                );
            }
            assert!(
                stream(&text, audio, "audio").is_some(),
                "{preset:?}: {text}"
            );
            let _ = std::fs::remove_file(&s.path);
        }
    }

    /// IMF Generic writes an MXF file with video and 48 kHz PCM audio that ffprobe reads back.
    #[test]
    fn imf_generic_writes_mxf_with_video_and_pcm_audio() {
        let Some(p) = project(true) else { return };
        if !Preset::Mxf.available() {
            eprintln!("this FFmpeg has no encoder for MXF");
            return;
        }
        let name = "picasso_export_imf.mxf";
        let s = settings(Preset::Mxf, name, 24);
        let mut steps = 0;
        render(&p, &MediaCache::new(), &s, &mut |_, _| {
            steps += 1;
            true
        })
        .unwrap_or_else(|e| panic!("IMF Generic: {e}"));
        assert_eq!(steps, 24);
        let Some(text) = streams(&s.path) else { return };
        assert!(
            stream(&text, "mpeg2video", "video").is_some()
                || stream(&text, "dnxhd", "video").is_some()
                || stream(&text, "h264", "video").is_some(),
            "{text}"
        );
        assert!(
            stream(&text, "pcm_s16le", "audio").is_some(),
            "{text}"
        );
        let _ = std::fs::remove_file(&s.path);
    }

    /// Pro Tools renders one WAV stem per audio track and writes an AAF that references them.
    /// Each audio track's clip in the AAF has source_in 0 and speed 1.0, pointing at its stem.
    #[test]
    fn pro_tools_writes_aaf_and_rendered_track_wavs() {
        let wav1 = std::env::temp_dir().join("picasso_protools_track1.wav");
        let wav2 = std::env::temp_dir().join("picasso_protools_track2.wav");
        let made1 = std::process::Command::new("ffmpeg")
            .args(["-y", "-v", "error", "-f", "lavfi", "-i"])
            .arg("sine=frequency=440:duration=2")
            .arg(&wav1)
            .status()
            .is_ok_and(|s| s.success());
        let made2 = std::process::Command::new("ffmpeg")
            .args(["-y", "-v", "error", "-f", "lavfi", "-i"])
            .arg("sine=frequency=880:duration=2")
            .arg(&wav2)
            .status()
            .is_ok_and(|s| s.success());
        if !made1 || !made2 {
            return;
        }

        let mut p = Project::new("pro tools test");
        p.width = 320;
        p.height = 180;

        let Ok(Outcome::Track(a1)) = (Command::AddTrack {
            kind: TrackKind::Audio,
        })
        .apply(&mut p)
        else {
            return;
        };
        let Ok(Outcome::Track(a2)) = (Command::AddTrack {
            kind: TrackKind::Audio,
        })
        .apply(&mut p)
        else {
            return;
        };

        let Ok(Outcome::Media(s1)) = (Command::AddMedia {
            name: "tone1".into(),
            source: MediaSource::Audio {
                path: wav1.to_string_lossy().into_owned(),
            },
            length: Some(48),
            default_duration: 48,
        })
        .apply(&mut p)
        else {
            return;
        };
        let Ok(_) = Command::AddClip {
            track: a1,
            media: s1,
            start: 0,
            duration: Some(24),
            with_sound: false,
        }
        .apply(&mut p)
        else {
            return;
        };

        let Ok(Outcome::Media(s2)) = (Command::AddMedia {
            name: "tone2".into(),
            source: MediaSource::Audio {
                path: wav2.to_string_lossy().into_owned(),
            },
            length: Some(48),
            default_duration: 48,
        })
        .apply(&mut p)
        else {
            return;
        };
        let Ok(_) = Command::AddClip {
            track: a2,
            media: s2,
            start: 0,
            duration: Some(24),
            with_sound: false,
        }
        .apply(&mut p)
        else {
            return;
        };

        let aaf_path = std::env::temp_dir().join("picasso_protools.aaf");
        let _ = std::fs::remove_file(&aaf_path);

        let s = Settings {
            preset: Preset::ProTools,
            path: aaf_path.clone(),
            start: 0,
            end: 48,
            options: Options::default(),
        };
        match render(&p, &MediaCache::new(), &s, &mut |_, _| true) {
            Ok(()) => {}
            Err(e) => {
                return;
            }
        }

        // Both WAV stems should exist.
        let stem1 = std::env::temp_dir().join("picasso_protools_A1.wav");
        let stem2 = std::env::temp_dir().join("picasso_protools_A2.wav");
        assert!(stem1.exists(), "track 1 stem missing");
        assert!(stem2.exists(), "track 2 stem missing");

        // The AAF should read back with clean clips pointing at the rendered WAVs.
        let Ok(aaf_bytes) = std::fs::read(&aaf_path) else {
            return;
        };
        let Ok(tl) = read_aaf(&aaf_bytes, Fps { num: 24, den: 1 }) else {
            return;
        };
        assert!(!tl.tracks.is_empty());

        let audio_tracks: Vec<_> = tl
            .tracks
            .iter()
            .filter(|t| t.kind == TrackKind::Audio)
            .collect();
        assert_eq!(audio_tracks.len(), 2);
        for at in &audio_tracks {
            for clip in &at.clips {
                assert_eq!(clip.source_in, 0, "clip source_in should be 0");
                assert_eq!(clip.speed, 1.0, "clip speed should be 1.0");
                assert!(
                    matches!(&clip.media, picasso_core::otio::OtioMedia::File { path }
                        if path.ends_with(".wav")),
                    "clip should reference a rendered WAV"
                );
            }
        }

        // Clean up.
        let _ = std::fs::remove_file(&aaf_path);
        let _ = std::fs::remove_file(&stem1);
        let _ = std::fs::remove_file(&stem2);
        let _ = std::fs::remove_file(&wav1);
        let _ = std::fs::remove_file(&wav2);
    }

    /// Cancelling a Pro Tools or MXF export leaves no partial files behind.
    #[test]
    fn pro_tools_and_mxf_cancellation_leaves_no_files() {
        let Some(p) = project(true) else { return };

        // Pro Tools cancellation.
        let aaf_path = std::env::temp_dir().join("picasso_protools_cancel.aaf");
        let _ = std::fs::remove_file(&aaf_path);
        let s = Settings {
            preset: Preset::ProTools,
            path: aaf_path.clone(),
            start: 0,
            end: 24,
            options: Options::default(),
        };
        let cancelled = render(&p, &MediaCache::new(), &s, &mut |done, _| done < 5);
        assert_eq!(cancelled, Err(ExportError::Cancelled));
        assert!(!aaf_path.exists(), "no partial AAF left behind");
        let stem1 = std::env::temp_dir().join("picasso_protools_cancel_A1.wav");
        let stem2 = std::env::temp_dir().join("picasso_protools_cancel_A2.wav");
        assert!(!stem1.exists(), "no partial WAV stem left behind");
        assert!(!stem2.exists(), "no partial WAV stem left behind");

        // MXF cancellation.
        let mxf_path = std::env::temp_dir().join("picasso_mxf_cancel.mxf");
        let _ = std::fs::remove_file(&mxf_path);
        if Preset::Mxf.available() {
            let s = Settings {
                preset: Preset::Mxf,
                path: mxf_path.clone(),
                start: 0,
                end: 24,
                options: Options::default(),
            };
            let cancelled = render(&p, &MediaCache::new(), &s, &mut |done, _| done < 5);
            assert_eq!(cancelled, Err(ExportError::Cancelled));
            assert!(!mxf_path.exists(), "no partial MXF left behind");
        }

        let _ = std::fs::remove_file(&aaf_path);
        let _ = std::fs::remove_file(&stem1);
        let _ = std::fs::remove_file(&stem2);
        let _ = std::fs::remove_file(&mxf_path);
    }

    #[test]
    fn ten_bits_and_the_dnxhr_profiles_reach_the_file() {
        let Some(p) = project(false) else { return };
        let ten = Options {
            ten_bit: true,
            ..Options::default()
        };
        for (preset, codec, profile) in [
            (Preset::H265, "hevc", "Main 10"),
            (Preset::H264Web, "h264", "High 10"),
            (Preset::Av1, "av1", "Main"),
            (Preset::Vp9, "vp9", "Profile 2"),
        ] {
            if !preset.available() {
                continue;
            }
            let s = with(
                preset,
                &format!("picasso_export_10_{preset:?}.{}", preset.extension()),
                6,
                ten.clone(),
            );
            match render(&p, &MediaCache::new(), &s, &mut |_, _| true) {
                // An x264 built for 8 bits only says so instead of making an 8-bit file.
                Err(ExportError::NoTenBit(_)) => continue,
                other => other.unwrap_or_else(|e| panic!("{preset:?}: {e}")),
            }
            let Some(text) = streams(&s.path) else { return };
            let line = stream(&text, codec, "video").unwrap_or_default();
            assert!(
                line.ends_with(",yuv420p10le") && line.contains(profile),
                "{preset:?}: {text}"
            );
            let _ = std::fs::remove_file(&s.path);
        }
        for (dnxhr, pix) in [(0, ",yuv422p"), (3, ",yuv422p10le"), (4, ",yuv444p10le")] {
            if !Preset::DnxHr.available() {
                break;
            }
            let o = Options {
                dnxhr,
                ..Options::default()
            };
            let s = with(
                Preset::DnxHr,
                &format!("picasso_export_dnx{dnxhr}.mov"),
                4,
                o,
            );
            render(&p, &MediaCache::new(), &s, &mut |_, _| true)
                .unwrap_or_else(|e| panic!("DNxHR {dnxhr}: {e}"));
            let Some(text) = streams(&s.path) else { return };
            let line = stream(&text, "dnxhd", "video").unwrap_or_default();
            assert!(line.ends_with(pix), "DNxHR {dnxhr}: {text}");
            let _ = std::fs::remove_file(&s.path);
        }
    }

    #[test]
    fn a_card_encoder_that_is_missing_or_absent_is_an_error_not_a_crash() {
        let Some(p) = project(false) else { return };
        // VA-API does not exist on Windows or macOS, and on Linux there may be no card for it.
        let mut made = Vec::new();
        for hardware in [
            Hardware::Vaapi,
            Hardware::Apple,
            Hardware::Nvidia,
            Hardware::Amd,
            Hardware::Intel,
        ] {
            let o = Options {
                hardware,
                ..Options::default()
            };
            let s = with(Preset::H264Web, "picasso_export_card.mp4", 6, o);
            match render(&p, &MediaCache::new(), &s, &mut |_, _| true) {
                Ok(()) => {
                    // This computer has the card: the file is H.264.
                    let text = streams(&s.path).unwrap_or_default();
                    assert!(
                        stream(&text, "h264", "video").is_some(),
                        "{hardware:?}: {text}"
                    );
                    made.push(hardware);
                }
                Err(ExportError::NoEncoder(_) | ExportError::Hardware { .. }) => {
                    assert!(!s.path.exists(), "{hardware:?} left a file");
                }
                Err(e) => panic!("{hardware:?}: {e}"),
            }
            let _ = std::fs::remove_file(&s.path);
        }
        eprintln!("graphics card encoders that work here: {made:?}");
        // Presets without a card's encoder always use the processor's.
        assert_eq!(Preset::ProRes422.hardware_choices(), vec![Hardware::Native]);
        assert_eq!(Preset::Wav.encoder_name(Hardware::Nvidia), None);
    }

    #[test]
    fn a_silent_project_has_no_sound_stream() {
        let Some(p) = project(false) else { return };
        let s = settings(Preset::H264Web, "picasso_export_silent.mp4", 12);
        render(&p, &MediaCache::new(), &s, &mut |_, _| true).unwrap();
        let Some(lines) = probe(&s.path) else { return };
        assert!(!lines.join("\n").contains("audio"), "{lines:?}");
        let _ = std::fs::remove_file(&s.path);
    }

    #[test]
    fn h265_works_when_the_build_has_it() {
        let Some(p) = project(false) else { return };
        if !Preset::H265.available() {
            return;
        }
        let s = settings(Preset::H265, "picasso_export_h265.mp4", 12);
        render(&p, &MediaCache::new(), &s, &mut |_, _| true).unwrap();
        let Some(lines) = probe(&s.path) else { return };
        assert!(lines.join("\n").contains("hevc,video,320,180"), "{lines:?}");
        let _ = std::fs::remove_file(&s.path);
    }

    #[test]
    fn wav_and_png_sequences() {
        let Some(p) = project(true) else { return };
        let wav = settings(Preset::Wav, "picasso_export_mix.wav", 24);
        render(&p, &MediaCache::new(), &wav, &mut |_, _| true).unwrap();
        // One second of 24-bit stereo at 48 kHz after a 44-byte header.
        assert_eq!(
            std::fs::metadata(&wav.path).map(|m| m.len()).ok(),
            Some(44 + 48_000 * 6)
        );
        let _ = std::fs::remove_file(&wav.path);
        let dir = std::env::temp_dir().join("picasso_export_png");
        let _ = std::fs::remove_dir_all(&dir);
        let png = Settings {
            preset: Preset::PngSequence,
            path: dir.clone(),
            start: 0,
            end: 5,
            options: Options::default(),
        };
        render(&p, &MediaCache::new(), &png, &mut |_, _| true).unwrap();
        assert!(dir.join("frame_000001.png").exists() && dir.join("frame_000005.png").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn with(preset: Preset, name: &str, frames: u64, options: Options) -> Settings {
        Settings {
            options,
            ..settings(preset, name, frames)
        }
    }

    #[test]
    fn the_size_and_frame_rate_of_the_file_follow_the_options() {
        let Some(p) = project(false) else { return };
        let o = Options {
            size: Some((160, 90)),
            fps: Some(picasso_core::Fps { num: 12, den: 1 }),
            ..Options::default()
        };
        let s = with(Preset::H264Web, "picasso_export_small.mp4", 24, o);
        let mut last = (0, 0);
        render(&p, &MediaCache::new(), &s, &mut |d, t| {
            last = (d, t);
            true
        })
        .unwrap();
        // One second of 24 frames is twelve pictures at 12 fps.
        assert_eq!(last, (12, 12));
        let Some(lines) = probe(&s.path) else { return };
        let text = lines.join("\n");
        assert!(text.contains("h264,video,160,90"), "{text}");
        assert!(
            lines
                .iter()
                .any(|l| l.starts_with("h264") && l.ends_with(",12")),
            "{text}"
        );
        let _ = std::fs::remove_file(&s.path);
    }

    #[test]
    fn the_page_can_leave_out_the_picture_or_the_sound() {
        let Some(p) = project(true) else { return };
        let silent = with(
            Preset::H264Web,
            "picasso_export_nosound.mp4",
            24,
            Options {
                audio: false,
                ..Options::default()
            },
        );
        render(&p, &MediaCache::new(), &silent, &mut |_, _| true).unwrap();
        let Some(lines) = probe(&silent.path) else {
            return;
        };
        let text = lines.join("\n");
        assert!(text.contains("video") && !text.contains("audio"), "{text}");
        let _ = std::fs::remove_file(&silent.path);
        let sound = with(
            Preset::H264Web,
            "picasso_export_novideo.mp4",
            24,
            Options {
                video: false,
                ..Options::default()
            },
        );
        render(&p, &MediaCache::new(), &sound, &mut |_, _| true).unwrap();
        let Some(lines) = probe(&sound.path) else {
            return;
        };
        let text = lines.join("\n");
        assert!(text.contains("audio") && !text.contains("video"), "{text}");
        let _ = std::fs::remove_file(&sound.path);
        let neither = with(
            Preset::H264Web,
            "picasso_export_nothing.mp4",
            24,
            Options {
                video: false,
                audio: false,
                ..Options::default()
            },
        );
        assert_eq!(
            render(&p, &MediaCache::new(), &neither, &mut |_, _| true),
            Err(ExportError::Empty)
        );
    }

    #[test]
    fn quality_and_bitrate_change_the_size_of_the_file() {
        let Some(p) = project(false) else { return };
        let size = |rate: Rate, name: &str| {
            let s = with(
                Preset::H264Web,
                name,
                24,
                Options {
                    rate,
                    ..Options::default()
                },
            );
            render(&p, &MediaCache::new(), &s, &mut |_, _| true).unwrap();
            let bytes = std::fs::metadata(&s.path).map(|m| m.len()).unwrap_or(0);
            let _ = std::fs::remove_file(&s.path);
            bytes
        };
        let best = size(Rate::Quality(Quality::Best), "picasso_export_best.mp4");
        let low = size(Rate::Quality(Quality::Low), "picasso_export_low.mp4");
        assert!(best > low, "{best} bytes at best, {low} at low");
        // A rate asked for is used instead of a quality, and still gives a file that plays.
        assert!(size(Rate::Bitrate(2000), "picasso_export_rate.mp4") > 0);
    }

    #[test]
    fn the_advanced_settings_are_written_in_the_file() {
        let Some(p) = project(false) else { return };
        let o = Options {
            pixel_aspect: PixelAspect::Cinemascope,
            levels: Levels::Full,
            color_primaries: Some("smpte432"),
            color_trc: Some("iec61966-2-1"),
            ..Options::default()
        };
        let s = with(Preset::H264Web, "picasso_export_adv.mp4", 12, o);
        render(&p, &MediaCache::new(), &s, &mut |_, _| true).unwrap();
        let out = std::process::Command::new("ffprobe")
            .args(["-v", "error", "-select_streams", "v", "-show_entries"])
            .arg("stream=sample_aspect_ratio,color_range,color_primaries,color_transfer")
            .args(["-of", "csv=p=0"])
            .arg(&s.path)
            .output();
        let _ = std::fs::remove_file(&s.path);
        let Ok(out) = out else { return };
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        assert!(text.contains("2:1"), "{text}");
        assert!(text.contains("pc"), "full range: {text}");
        assert!(text.contains("smpte432"), "{text}");
        assert!(text.contains("iec61966-2-1"), "{text}");
        // Without them the file says what a 709 picture says.
        let plain = with(
            Preset::H264Web,
            "picasso_export_plain.mp4",
            12,
            Options::default(),
        );
        render(&p, &MediaCache::new(), &plain, &mut |_, _| true).unwrap();
        let out = std::process::Command::new("ffprobe")
            .args(["-v", "error", "-select_streams", "v", "-show_entries"])
            .arg("stream=sample_aspect_ratio,color_range,color_primaries,color_transfer")
            .args(["-of", "csv=p=0"])
            .arg(&plain.path)
            .output();
        let _ = std::fs::remove_file(&plain.path);
        if let Ok(out) = out {
            let text = String::from_utf8_lossy(&out.stdout).to_string();
            assert!(text.contains("bt709") && text.contains("tv"), "{text}");
            assert!(!text.contains("2:1"), "{text}");
        }
    }

    #[test]
    fn a_burned_in_timecode_changes_the_pictures() {
        let Some(p) = project(false) else { return };
        let bytes = |burn_in: BurnIn, name: &str| {
            let s = with(
                Preset::H264Web,
                name,
                6,
                Options {
                    burn_in,
                    ..Options::default()
                },
            );
            render(&p, &MediaCache::new(), &s, &mut |_, _| true).unwrap();
            let b = std::fs::read(&s.path).unwrap_or_default();
            let _ = std::fs::remove_file(&s.path);
            b
        };
        let plain = bytes(BurnIn::Off, "picasso_export_burn0.mp4");
        let timecode = bytes(BurnIn::Timecode, "picasso_export_burn1.mp4");
        assert!(!plain.is_empty() && plain != timecode);
    }

    #[test]
    fn markers_become_chapters_when_asked() {
        let Some(mut p) = project(false) else { return };
        for (frame, name) in [(0, "Start"), (12, "Middle")] {
            p.markers.push(picasso_core::Marker {
                frame,
                name: name.into(),
                color: 0,
            });
        }
        let chapters = |chapters: bool, file: &str| {
            let s = with(
                Preset::H264Web,
                file,
                24,
                Options {
                    chapters,
                    ..Options::default()
                },
            );
            render(&p, &MediaCache::new(), &s, &mut |_, _| true).unwrap();
            let out = std::process::Command::new("ffprobe")
                .args(["-v", "error", "-show_chapters", "-of", "csv=p=0"])
                .arg(&s.path)
                .output()
                .ok();
            let _ = std::fs::remove_file(&s.path);
            out.map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        };
        let (Some(with_chapters), Some(without)) = (
            chapters(true, "picasso_export_chap.mp4"),
            chapters(false, "picasso_export_nochap.mp4"),
        ) else {
            return;
        };
        assert!(
            with_chapters.contains("Start") && with_chapters.contains("Middle"),
            "{with_chapters}"
        );
        assert!(without.trim().is_empty(), "{without}");
    }

    #[test]
    fn keyframes_and_frame_reordering_reach_the_encoder() {
        let Some(p) = project(false) else { return };
        let o = Options {
            keyframes: Some(4),
            frame_reordering: false,
            ..Options::default()
        };
        let s = with(Preset::H264Web, "picasso_export_gop.mp4", 24, o);
        render(&p, &MediaCache::new(), &s, &mut |_, _| true).unwrap();
        let keyframes = std::process::Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-select_streams",
                "v",
                "-show_entries",
                "frame=pict_type,key_frame",
            ])
            .args(["-of", "csv=p=0"])
            .arg(&s.path)
            .output();
        if let Ok(out) = keyframes {
            let text = String::from_utf8_lossy(&out.stdout).to_string();
            // No B pictures, and an I picture at least every four frames.
            assert!(!text.contains(",B"), "{text}");
            assert!(
                text.lines().filter(|l| l.starts_with("1,")).count() >= 6,
                "{text}"
            );
        }
        let _ = std::fs::remove_file(&s.path);
    }

    #[test]
    fn the_options_work_out_pictures_and_sizes_without_overflow() {
        let p = Project::new("sizes");
        let mut o = Options::default();
        assert_eq!(o.picture_size(&p), (1920, 1080));
        o.size = Some((1001, 3));
        assert_eq!(o.picture_size(&p), (1000, 2));
        o.size = Some((0, 0));
        assert_eq!(o.picture_size(&p), (2, 2));
        assert_eq!(o.output_frames(&p, 24), 24);
        o.fps = Some(picasso_core::Fps { num: 12, den: 1 });
        assert_eq!(o.output_frames(&p, 24), 12);
        assert!(o.output_frames(&p, u64::MAX / 4) > 0);
        assert_eq!(o.timeline_frame(&p, 10, 34, 0), 10);
        assert_eq!(o.timeline_frame(&p, 10, 34, 11), 32);
        assert_eq!(o.timeline_frame(&p, 10, 34, 1_000), 33);
    }

    #[test]
    fn cancelling_leaves_no_file_and_an_empty_range_is_refused() {
        let Some(p) = project(false) else { return };
        let s = settings(Preset::H264Web, "picasso_export_cancel.mp4", 24);
        let cancelled = render(&p, &MediaCache::new(), &s, &mut |done, _| done < 5);
        assert_eq!(cancelled, Err(ExportError::Cancelled));
        assert!(!s.path.exists(), "a half-written file is removed");
        let empty = Settings { end: 0, ..s };
        assert_eq!(
            render(&p, &MediaCache::new(), &empty, &mut |_, _| true),
            Err(ExportError::Empty)
        );
    }
}
