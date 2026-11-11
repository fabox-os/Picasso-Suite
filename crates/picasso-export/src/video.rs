//! Video (and its sound) through FFmpeg's encoders.

use ff::codec::{self, encoder};
use ff::format::sample::{Sample, Type};
use ff::format::{self, Pixel};
use ff::{color, frame, ChannelLayout, Dictionary, Packet, Rational};
use ffmpeg_next as ff;
use picasso_audio::{frames_to_samples, plan, AudioPlan, AudioPool, Mixer, Offline, SAMPLE_RATE};
use picasso_core::Project;
use picasso_render::{render_frame, MediaCache};

use crate::burn::burn_in;
use crate::yuv::{rgba_to_yuv, Layout};
use crate::{BurnIn, Levels, PixelAspect, DNXHR_PROFILES};
use crate::{ExportError, Hardware, Options, Preset, Quality, Rate, Settings, Speed};

fn enc_err(e: ff::Error) -> ExportError {
    ExportError::Encode(e.to_string())
}

/// What a preset asks of the video encoder.
struct VideoSpec {
    encoder: &'static str,
    /// The layout the pictures are converted to.
    pixel: Pixel,
    /// A graphics card's encoder.
    hardware: bool,
    /// The encoder takes its pictures from the card's memory (VA-API).
    vaapi: bool,
    /// Options for the encoder, from the preset and what the Deliver page set.
    options: Vec<(&'static str, String)>,
    /// The data rate in bits a second; 0 when a quality decides it (FFmpeg's own default of
    /// 200 kb/s would hold the encoders back).
    bit_rate: usize,
    /// The encoder's quality number, for encoders that take it as `global_quality`.
    global_quality: Option<i32>,
    /// `global_quality` is a quality scale (VideoToolbox).
    qscale: bool,
    /// Every picture is a key frame (FFV1).
    intra: bool,
}

/// The four qualities as numbers of an encoder's own scale, best first.
fn scale(encoder: &str) -> [i32; 4] {
    if encoder.ends_with("videotoolbox") {
        // 1 to 100, higher is better.
        return [80, 65, 50, 35];
    }
    if encoder.contains("av1") {
        return [20, 28, 35, 42];
    }
    if encoder == "libvpx-vp9" {
        return [15, 24, 31, 38];
    }
    if encoder.contains("hevc") || encoder == "libx265" {
        return [18, 22, 27, 32];
    }
    [14, 18, 23, 28]
}

fn quality_index(q: Quality) -> usize {
    match q {
        Quality::Best => 0,
        Quality::High => 1,
        Quality::Medium => 2,
        Quality::Low => 3,
    }
}

/// The name of the encoder a preset wants with `hardware`, for messages when it is missing.
fn wanted(preset: Preset, hardware: Hardware) -> &'static str {
    let codec = match preset {
        Preset::H265 => "hevc",
        Preset::Av1 => "av1",
        _ => "h264",
    };
    match hardware {
        Hardware::Native => preset.software_encoders().first().copied(),
        h => h.encoder(codec),
    }
    .unwrap_or("video")
}

fn video_spec(preset: Preset, o: &Options) -> Result<Option<VideoSpec>, ExportError> {
    if preset.software_encoders().is_empty() {
        return Ok(None);
    }
    let hardware = match preset {
        Preset::H264Web | Preset::H265 | Preset::Av1 => o.hardware,
        _ => Hardware::Native,
    };
    let encoder = preset
        .encoder_name(hardware)
        .ok_or(ExportError::NoEncoder(wanted(preset, hardware)))?;
    let ten = o.ten_bit && preset.has_ten_bit();
    // The cards make 10-bit H.265 and AV1, but (but for the newest NVIDIA ones) not H.264.
    if ten && preset == Preset::H264Web && hardware != Hardware::Native {
        return Err(ExportError::NoTenBit(encoder));
    }
    let quality = match o.rate {
        Rate::Quality(q) => Some(scale(encoder)[quality_index(q)]),
        Rate::Bitrate(_) => None,
    };
    let bit_rate = match o.rate {
        Rate::Quality(_) => 0,
        Rate::Bitrate(kbps) => kbps.max(1) as usize * 1000,
    };
    let fast = |faster: &str, balanced: &str, best: &str| {
        match o.speed {
            Speed::Faster => faster,
            Speed::Balanced => balanced,
            Speed::Best => best,
        }
        .to_string()
    };
    let h264 = preset == Preset::H264Web;
    let hevc = preset == Preset::H265;
    // The H.264 profile asked for, or High 10 / Main 10 for 10 bits.
    let profile = if ten && h264 {
        Some("high10")
    } else if ten && hevc {
        Some("main10")
    } else if h264 {
        o.profile.name()
    } else {
        None
    };
    let (pixel_8, pixel_10) = if hardware == Hardware::Native {
        (Pixel::YUV420P, Pixel::YUV420P10LE)
    } else {
        // The cards take the colour samples side by side.
        (Pixel::NV12, Pixel::P010LE)
    };
    let mut spec = VideoSpec {
        encoder,
        pixel: if ten { pixel_10 } else { pixel_8 },
        hardware: hardware != Hardware::Native,
        vaapi: hardware == Hardware::Vaapi,
        options: Vec::new(),
        bit_rate,
        global_quality: None,
        qscale: false,
        intra: false,
    };
    let opts = &mut spec.options;
    let quality_as = |name: &'static str, opts: &mut Vec<(&'static str, String)>| {
        if let Some(q) = quality {
            opts.push((name, q.to_string()));
        }
    };
    match encoder {
        "libx264" | "libx265" => {
            quality_as("crf", opts);
            opts.push(("preset", o.speed.encoder_preset().to_string()));
            if encoder == "libx265" {
                opts.push(("x265-params", "log-level=error".to_string()));
            }
            if let Some(p) = profile {
                opts.push(("profile", p.to_string()));
            }
        }
        "libsvtav1" => {
            quality_as("crf", opts);
            opts.push(("preset", fast("10", "8", "5")));
        }
        "libaom-av1" => {
            quality_as("crf", opts);
            opts.push(("cpu-used", fast("6", "4", "2")));
            opts.push(("row-mt", "1".to_string()));
        }
        "libvpx-vp9" => {
            quality_as("crf", opts);
            opts.push(("deadline", "good".to_string()));
            opts.push(("cpu-used", fast("4", "2", "1")));
            opts.push(("row-mt", "1".to_string()));
            if ten {
                opts.push(("profile", "2".to_string()));
            }
        }
        "prores_ks" => {
            spec.pixel = Pixel::YUV422P10LE;
            // 0 is Proxy, 1 LT, 2 ProRes 422 (standard), 3 HQ.
            opts.push(("profile", o.prores.min(3).to_string()));
            spec.bit_rate = 0;
        }
        "dnxhd" => {
            let i = usize::from(o.dnxhr).min(DNXHR_PROFILES.len() - 1);
            let name = DNXHR_PROFILES.get(i).map_or("dnxhr_hq", |p| p.1);
            spec.pixel = match name {
                "dnxhr_hqx" => Pixel::YUV422P10LE,
                "dnxhr_444" => Pixel::YUV444P10LE,
                _ => Pixel::YUV422P,
            };
            opts.push(("profile", name.to_string()));
            spec.bit_rate = 0;
        }
        "ffv1" => {
            spec.pixel = Pixel::YUV444P10LE;
            opts.push(("level", "3".to_string()));
            opts.push(("slicecrc", "1".to_string()));
            spec.intra = true;
            spec.bit_rate = 0;
        }
        e if e.ends_with("_nvenc") => {
            opts.push(("preset", fast("p3", "p5", "p7")));
            opts.push(("tune", "hq".to_string()));
            opts.push(("rc", "vbr".to_string()));
            quality_as("cq", opts);
            if let Some(p) = profile {
                opts.push(("profile", p.to_string()));
            }
        }
        e if e.ends_with("_qsv") => {
            opts.push(("preset", fast("veryfast", "medium", "veryslow")));
            spec.global_quality = quality;
            if let Some(p) = profile {
                opts.push(("profile", p.to_string()));
            }
        }
        e if e.ends_with("_amf") => {
            opts.push(("quality", fast("speed", "balanced", "quality")));
            if let Some(q) = quality {
                opts.push(("rc", "cqp".to_string()));
                for name in ["qp_i", "qp_p", "qp_b"] {
                    opts.push((name, q.to_string()));
                }
            } else {
                opts.push(("rc", "vbr_peak".to_string()));
            }
            // AMF has no High 10; 10-bit H.264 on it is refused by the pixel check instead.
            match profile {
                Some("baseline") => opts.push(("profile", "constrained_baseline".to_string())),
                Some("high10") | None => {}
                Some(p) => opts.push(("profile", p.to_string())),
            }
        }
        e if e.ends_with("_videotoolbox") => {
            spec.global_quality = quality;
            spec.qscale = quality.is_some();
            if let Some(p) = profile {
                opts.push(("profile", p.to_string()));
            }
        }
        e if e.ends_with("_vaapi") => {
            spec.global_quality = quality;
            opts.push((
                "rc_mode",
                if quality.is_some() { "CQP" } else { "VBR" }.to_string(),
            ));
            if let Some(p) = profile {
                let p = if p == "baseline" {
                    "constrained_baseline"
                } else {
                    p
                };
                opts.push(("profile", p.to_string()));
            }
        }
        _ => {}
    }
    Ok(Some(spec))
}

/// Whether a graphics card's encoder opens here, tried with a small picture. VA-API is tried on
/// the default card and then on every render node, each without and with the low-power mode; the
/// first way that works is remembered for the export.
pub(crate) fn opens(name: &str, hardware: Hardware) -> bool {
    if hardware != Hardware::Vaapi {
        return opens_on(name, hardware, &picasso_decode::hw::VaapiChoice::default());
    }
    let nodes = picasso_decode::hw::vaapi_nodes();
    let candidates = std::iter::once(None).chain(nodes.into_iter().map(Some));
    for node in candidates {
        for low_power in [false, true] {
            let choice = picasso_decode::hw::VaapiChoice {
                node: node.clone(),
                low_power,
            };
            if opens_on(name, hardware, &choice) {
                picasso_decode::hw::remember_vaapi_choice(name, choice);
                return true;
            }
        }
    }
    false
}

fn opens_on(name: &str, hardware: Hardware, choice: &picasso_decode::hw::VaapiChoice) -> bool {
    let Some(codec) = encoder::find_by_name(name) else {
        return false;
    };
    let Ok(mut enc) = codec::context::Context::new_with_codec(codec)
        .encoder()
        .video()
    else {
        return false;
    };
    let (width, height) = (640, 360);
    enc.set_width(width);
    enc.set_height(height);
    enc.set_time_base(Rational(1, 30));
    enc.set_frame_rate(Some(Rational(30, 1)));
    let _frames = if hardware == Hardware::Vaapi {
        let Ok(frames) = picasso_decode::hw::VaapiFrames::new(
            width,
            height,
            Pixel::NV12,
            choice.node.as_deref(),
        ) else {
            return false;
        };
        enc.set_format(Pixel::VAAPI);
        frames.attach(&mut enc);
        Some(frames)
    } else {
        enc.set_format(Pixel::NV12);
        None
    };
    let mut options = Dictionary::new();
    if choice.low_power {
        options.set("low_power", "1");
    }
    enc.open_with(options).is_ok()
}

/// The sound encoder of a file and how it takes its samples.
struct AudioSpec {
    encoder: &'static str,
    /// 32-bit float, a plane a channel (AAC, MP3); otherwise 16-bit, interleaved.
    planar: bool,
    /// The data rate in bits a second, for the encoders that have one.
    bit_rate: Option<usize>,
}

fn audio_spec(preset: Preset, o: &Options) -> Option<AudioSpec> {
    let encoder = preset.audio_encoder(o.audio_codec)?;
    let kbps = |lo: u32, hi: u32| Some(o.audio_kbps.clamp(lo, hi) as usize * 1000);
    Some(match encoder {
        "aac" => AudioSpec {
            encoder,
            planar: true,
            bit_rate: kbps(32, 640),
        },
        "libmp3lame" => AudioSpec {
            encoder,
            planar: true,
            bit_rate: kbps(32, 320),
        },
        "libopus" => AudioSpec {
            encoder,
            planar: false,
            bit_rate: kbps(6, 510),
        },
        _ => AudioSpec {
            encoder,
            planar: false,
            bit_rate: None,
        },
    })
}

/// An open sound encoder and the samples waiting for it, as it wants them in blocks.
struct AudioOut {
    enc: encoder::Audio,
    stream: usize,
    stream_time_base: Rational,
    planar: bool,
    block: usize,
    /// Interleaved stereo samples not yet encoded.
    pending: Vec<f32>,
    /// Samples (per channel) encoded so far, the time of the next frame.
    encoded: i64,
}

impl AudioOut {
    fn send(
        &mut self,
        octx: &mut format::context::Output,
        count: usize,
    ) -> Result<(), ExportError> {
        let layout = ChannelLayout::STEREO;
        let sample = if self.planar {
            Sample::F32(Type::Planar)
        } else {
            Sample::I16(Type::Packed)
        };
        let mut frame = frame::Audio::new(sample, count, layout);
        frame.set_rate(SAMPLE_RATE);
        frame.set_pts(Some(self.encoded));
        let samples: Vec<f32> = self.pending.drain(..count * 2).collect();
        if self.planar {
            for (ch, plane) in [0usize, 1].into_iter().enumerate() {
                let dst = frame.plane_mut::<f32>(plane);
                for (d, s) in dst.iter_mut().zip(samples.iter().skip(ch).step_by(2)) {
                    *d = *s;
                }
            }
        } else {
            let dst = frame.plane_mut::<(i16, i16)>(0);
            for (d, s) in dst.iter_mut().zip(samples.chunks_exact(2)) {
                let q = |v: f32| (v.clamp(-1.0, 1.0) * 32767.0).round() as i16;
                *d = (q(s[0]), q(s[1]));
            }
        }
        self.encoded += count as i64;
        self.enc.send_frame(&frame).map_err(enc_err)?;
        self.drain(octx)
    }

    fn drain(&mut self, octx: &mut format::context::Output) -> Result<(), ExportError> {
        let mut packet = Packet::empty();
        while self.enc.receive_packet(&mut packet).is_ok() {
            // FLAC ends with an empty packet that only updates its header (the checksum), which
            // Matroska refuses; the header written at the start stands.
            if packet.size() == 0 {
                continue;
            }
            packet.set_stream(self.stream);
            packet.rescale_ts(Rational(1, SAMPLE_RATE as i32), self.stream_time_base);
            packet.write_interleaved(octx).map_err(enc_err)?;
        }
        Ok(())
    }

    /// Encodes whole blocks of what is pending; with `all`, the rest as a last short block.
    fn flush_pending(
        &mut self,
        octx: &mut format::context::Output,
        all: bool,
    ) -> Result<(), ExportError> {
        while self.pending.len() / 2 >= self.block {
            self.send(octx, self.block)?;
        }
        let rest = self.pending.len() / 2;
        if all && rest > 0 {
            self.send(octx, rest)?;
        }
        Ok(())
    }

    fn finish(&mut self, octx: &mut format::context::Output) -> Result<(), ExportError> {
        self.flush_pending(octx, true)?;
        self.enc.send_eof().map_err(enc_err)?;
        self.drain(octx)
    }
}

pub(crate) fn encode(
    project: &Project,
    cache: &MediaCache,
    settings: &Settings,
    progress: &mut dyn FnMut(u64, u64) -> bool,
) -> Result<(), ExportError> {
    let preset = settings.preset;
    let o = &settings.options;
    let spec = video_spec(preset, o)?;
    let sound_plan: AudioPlan = plan(project);
    let want_video = o.video && spec.is_some();
    let want_audio = o.audio && !sound_plan.clips.is_empty();
    if !want_video && !want_audio {
        return Err(ExportError::Empty);
    }
    let (width, height) = o.picture_size(project);
    let fps = o.frame_rate(project);
    // ProRes and DNxHR are always at video levels; the others follow the page.
    let full_range =
        o.levels == Levels::Full && !matches!(preset, Preset::ProRes422 | Preset::DnxHr);
    let time_base = Rational(fps.den as i32, fps.num as i32);

    let mut octx = format::output(&settings.path)
        .map_err(|e| ExportError::Write(format!("{}: {e}", settings.path.display())))?;
    let global_header = octx.format().flags().contains(format::Flags::GLOBAL_HEADER);

    // ---- video
    let mut video_index = 0;
    let mut venc: Option<encoder::Video> = None;
    let mut vaapi: Option<picasso_decode::hw::VaapiFrames> = None;
    let mut layout = None;
    if let (true, Some(spec)) = (want_video, spec.as_ref()) {
        let codec =
            encoder::find_by_name(spec.encoder).ok_or(ExportError::NoEncoder(spec.encoder))?;
        // A 10-bit picture needs an encoder built for it (x264 is built for one depth).
        let takes = |p: Pixel| {
            codec
                .video()
                .ok()
                .and_then(|v| v.formats().map(|mut f| f.any(|x| x == p)))
                .unwrap_or(true)
        };
        let fed = if spec.vaapi { Pixel::VAAPI } else { spec.pixel };
        if !takes(fed) {
            return Err(if o.ten_bit {
                ExportError::NoTenBit(spec.encoder)
            } else {
                ExportError::NoEncoder(spec.encoder)
            });
        }
        layout = Some(
            Layout::of(spec.pixel)
                .ok_or_else(|| ExportError::Encode(format!("{:?} pictures", spec.pixel)))?,
        );
        let mut stream = octx.add_stream(codec).map_err(enc_err)?;
        video_index = stream.index();
        let mut enc = codec::context::Context::new_with_codec(codec)
            .encoder()
            .video()
            .map_err(enc_err)?;
        enc.set_width(width);
        enc.set_height(height);
        enc.set_format(fed);
        enc.set_time_base(time_base);
        enc.set_frame_rate(Some(Rational(fps.num as i32, fps.den as i32)));
        let gop = if spec.intra {
            1
        } else {
            o.keyframes.unwrap_or(fps.nominal() as u32 * 2)
        };
        enc.set_gop(gop.max(1));
        if !o.frame_reordering || o.profile == crate::Profile::Baseline {
            enc.set_max_b_frames(0);
        }
        enc.set_bit_rate(spec.bit_rate);
        if let Some(q) = spec.global_quality {
            // A quality scale counts in FFmpeg's lambda units.
            enc.set_global_quality(if spec.qscale { q * 118 } else { q });
        }
        enc.set_colorspace(color::Space::BT709);
        enc.set_color_range(if full_range {
            color::Range::JPEG
        } else {
            color::Range::MPEG
        });
        if o.pixel_aspect == PixelAspect::Cinemascope {
            enc.set_aspect_ratio(Rational(2, 1));
        }
        let mut flags = codec::Flags::empty();
        if global_header {
            flags |= codec::Flags::GLOBAL_HEADER;
        }
        if spec.qscale {
            flags |= codec::Flags::QSCALE;
        }
        enc.set_flags(flags);
        // Where the card encoder was found to work (its render node, and its low-power mode).
        let choice = if spec.vaapi {
            picasso_decode::hw::vaapi_choice(spec.encoder)
        } else {
            picasso_decode::hw::VaapiChoice::default()
        };
        if spec.vaapi {
            let frames = picasso_decode::hw::VaapiFrames::new(
                width,
                height,
                spec.pixel,
                choice.node.as_deref(),
            )
            .map_err(|why| ExportError::Hardware {
                encoder: spec.encoder,
                why,
            })?;
            frames.attach(&mut enc);
            vaapi = Some(frames);
        }
        let mut options = Dictionary::new();
        for (k, v) in &spec.options {
            options.set(k, v);
        }
        if choice.low_power {
            options.set("low_power", "1");
        }
        // What the file says about its colours; the pictures themselves are not changed.
        options.set("color_primaries", o.color_primaries.unwrap_or("bt709"));
        options.set("color_trc", o.color_trc.unwrap_or("bt709"));
        let opened = enc.open_with(options).map_err(|e| {
            if spec.hardware {
                ExportError::Hardware {
                    encoder: spec.encoder,
                    why: e.to_string(),
                }
            } else {
                enc_err(e)
            }
        })?;
        stream.set_parameters(&opened);
        venc = Some(opened);
    }

    // ---- sound, when anything makes a sound
    let mut audio: Option<AudioOut> = None;
    if let (true, Some(sound)) = (want_audio, audio_spec(preset, o)) {
        let name = sound.encoder;
        let acodec = encoder::find_by_name(name).ok_or(ExportError::NoEncoder(name))?;
        let mut astream = octx.add_stream(acodec).map_err(enc_err)?;
        let stream_index = astream.index();
        let mut a = codec::context::Context::new_with_codec(acodec)
            .encoder()
            .audio()
            .map_err(enc_err)?;
        let planar = sound.planar;
        a.set_rate(SAMPLE_RATE as i32);
        a.set_channel_layout(ChannelLayout::STEREO);
        a.set_format(if planar {
            Sample::F32(Type::Planar)
        } else {
            Sample::I16(Type::Packed)
        });
        if let Some(rate) = sound.bit_rate {
            a.set_bit_rate(rate);
        }
        a.set_time_base(Rational(1, SAMPLE_RATE as i32));
        if global_header {
            a.set_flags(codec::Flags::GLOBAL_HEADER);
        }
        let aenc = a.open_as(acodec).map_err(enc_err)?;
        astream.set_parameters(&aenc);
        let block = match aenc.frame_size() {
            0 => 1024,
            n => n as usize,
        };
        audio = Some(AudioOut {
            enc: aenc,
            stream: stream_index,
            stream_time_base: Rational(1, SAMPLE_RATE as i32),
            planar,
            block,
            pending: Vec::new(),
            encoded: 0,
        });
    }

    let timeline_frames = settings.end - settings.start;
    let total = o.output_frames(project, timeline_frames);
    if o.chapters {
        let marks: Vec<_> = project
            .markers
            .iter()
            .filter(|m| m.frame >= settings.start && m.frame < settings.end)
            .collect();
        for (i, mark) in marks.iter().enumerate() {
            let from = o
                .output_index(project, settings.start, mark.frame)
                .min(total);
            let to = marks
                .get(i + 1)
                .map_or(total, |next| {
                    o.output_index(project, settings.start, next.frame)
                })
                .clamp(from + 1, total.max(from + 1));
            octx.add_chapter(
                i as i64,
                time_base,
                from as i64,
                to as i64,
                mark.name.as_str(),
            )
            .map_err(enc_err)?;
        }
    }
    if venc.is_none() && audio.is_none() {
        return Err(ExportError::Empty);
    }
    let mut header_options = Dictionary::new();
    if o.faststart && matches!(preset.extension(), "mp4" | "mov") {
        // Put the index at the front so the file can start playing while it downloads.
        header_options.set("movflags", "+faststart");
    }
    octx.write_header_with(header_options).map_err(enc_err)?;
    // The muxer may have changed the time bases when it wrote the header.
    let video_stream_tb = octx
        .stream(video_index)
        .map_or(time_base, |s| s.time_base());
    if let Some(a) = audio.as_mut() {
        if let Some(s) = octx.stream(a.stream) {
            a.stream_time_base = s.time_base();
        }
    }

    // ---- frames
    let cache = cache.blocking();
    let pool = AudioPool::new(|| {});
    let source = Offline(&pool);
    let mut mixer = Mixer::new();
    let audio_origin = frames_to_samples(project, settings.start);
    let audio_length = frames_to_samples(project, settings.end) - audio_origin;
    let mut audio_done = 0u64; // samples mixed so far
    let pixel = spec.as_ref().map_or(Pixel::YUV420P, |s| s.pixel);
    let mut vframe = frame::Video::new(pixel, width, height);
    vframe.set_color_space(color::Space::BT709);
    vframe.set_color_range(if full_range {
        color::Range::JPEG
    } else {
        color::Range::MPEG
    });
    // The planes are separate buffers of the frame; they are filled here, then copied in.
    let strides: Vec<usize> = (0..vframe.planes()).map(|i| vframe.stride(i)).collect();
    let mut planes: Vec<Vec<u8>> = (0..vframe.planes())
        .map(|i| vec![0u8; vframe.data(i).len()])
        .collect();
    let mut packet = Packet::empty();
    for n in 0..total {
        let timeline_frame = o.timeline_frame(project, settings.start, settings.end, n);
        if let Some(venc) = venc.as_mut() {
            let mut picture = render_frame(project, &cache, timeline_frame, width, height)
                .map_err(|e| ExportError::Render(e.to_string()))?;
            if o.burn_in != BurnIn::Off {
                let clock = project.fps.timecode(o.clock_start + timeline_frame);
                let name = project
                    .tracks
                    .iter()
                    .rev()
                    .flat_map(|t| t.clips.iter())
                    .find(|c| c.enabled && c.contains(timeline_frame))
                    .map(|c| c.name.as_str());
                let (code, name) = match o.burn_in {
                    BurnIn::Timecode => (Some(clock.as_str()), None),
                    BurnIn::ClipName => (None, name),
                    _ => (Some(clock.as_str()), name),
                };
                burn_in(
                    &mut picture.rgba,
                    (width as usize, height as usize),
                    code,
                    name,
                );
            }
            if let Some(layout) = layout {
                let mut slices: Vec<&mut [u8]> = planes.iter_mut().map(Vec::as_mut_slice).collect();
                rgba_to_yuv(
                    &picture.rgba,
                    (width as usize, height as usize),
                    layout,
                    &mut slices,
                    &strides,
                    full_range,
                );
            }
            for (i, plane) in planes.iter().enumerate() {
                vframe.data_mut(i).copy_from_slice(plane);
            }
            vframe.set_pts(Some(n as i64));
            match &vaapi {
                Some(frames) => {
                    let on_card = frames.upload(&vframe).map_err(enc_err)?;
                    venc.send_frame(&on_card)
                }
                None => venc.send_frame(&vframe),
            }
            .map_err(enc_err)?;
            while venc.receive_packet(&mut packet).is_ok() {
                packet.set_stream(video_index);
                // A constant-frame-rate picture is one time-base unit long. ffmpeg does not
                // always give the last packet a duration from the last frame, which makes the
                // file read back one frame short; state it on the packet in codec time_base and
                // let rescale_ts convert it to the stream's time_base.
                packet.set_duration(1);
                packet.rescale_ts(time_base, video_stream_tb);
                packet.write_interleaved(&mut octx).map_err(enc_err)?;
            }
        }
        // The sound up to the end of this picture.
        if let Some(a) = audio.as_mut() {
            let target = if n + 1 == total {
                audio_length
            } else {
                (u128::from(n + 1) * u128::from(fps.den) * u128::from(SAMPLE_RATE)
                    / u128::from(fps.num.max(1)))
                .min(u128::from(audio_length)) as u64
            };
            let count = target.saturating_sub(audio_done) as usize;
            if count > 0 {
                let mut block = vec![0.0f32; count * 2];
                mixer.mix(&sound_plan, &source, audio_origin + audio_done, &mut block);
                a.pending.extend_from_slice(&block);
                audio_done = target;
                a.flush_pending(&mut octx, false)?;
            }
        }
        if !progress(n + 1, total) {
            return Err(ExportError::Cancelled);
        }
    }

    // ---- finish
    if let Some(venc) = venc.as_mut() {
        venc.send_eof().map_err(enc_err)?;
        while venc.receive_packet(&mut packet).is_ok() {
            packet.set_stream(video_index);
            packet.set_duration(1);
            packet.rescale_ts(time_base, video_stream_tb);
            packet.write_interleaved(&mut octx).map_err(enc_err)?;
        }
    }
    if let Some(a) = audio.as_mut() {
        a.finish(&mut octx)?;
    }
    octx.write_trailer().map_err(enc_err)?;
    Ok(())
}
