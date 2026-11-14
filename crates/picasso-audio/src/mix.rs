//! Which clips sound at which sample, and adding them up.

use std::collections::HashMap;

use picasso_core::{MediaId, MediaSource, Project, MIN_GAIN_DB};

use crate::stretch::TimeStretch;

/// The sample rate of the mix.
pub const SAMPLE_RATE: u32 = 48_000;

/// Tracks whose level is measured; later tracks are mixed but not metered.
pub const METERED_TRACKS: usize = 64;

/// One clip that makes sound: where it sits on the timeline, what part of its file it plays and
/// how loud it is.
#[derive(Clone, Debug, PartialEq)]
pub struct ClipSound {
    pub media: MediaId,
    pub path: String,
    /// Index of the clip's track in the project.
    pub track: usize,
    /// First sample on the timeline.
    pub start: u64,
    /// Number of samples.
    pub len: u64,
    /// First sample of the file that is played.
    pub source_in: u64,
    /// Samples of the file played per sample of the timeline (2.0 is twice as fast).
    pub speed: f32,
    /// Keep the pitch when the speed is not 1.0 (the sound is time-stretched). Off: the sound is
    /// resampled like tape, so a faster clip sounds higher. A reversed clip is stretched on the
    /// sound read backwards.
    pub preserve_pitch: bool,
    /// The clip's gain times its track's gain, as a factor.
    pub gain: f32,
    /// What the track's pan leaves of each channel (1.0 is all of it).
    pub left: f32,
    pub right: f32,
    /// Samples over which the sound rises at the start and falls at the end.
    pub fade_in: u64,
    pub fade_out: u64,
    /// Played backwards, from the end of what the clip uses.
    pub reverse: bool,
}

impl ClipSound {
    /// A clip at full level, centred, without fades.
    pub fn plain(media: MediaId, path: &str, start: u64, len: u64, source_in: u64) -> Self {
        Self {
            media,
            path: path.to_string(),
            track: 0,
            start,
            len,
            source_in,
            speed: 1.0,
            preserve_pitch: false,
            gain: 1.0,
            left: 1.0,
            right: 1.0,
            fade_in: 0,
            fade_out: 0,
            reverse: false,
        }
    }

    /// How much of the sound is left `at` samples into the clip: the fades, as a factor.
    fn envelope(&self, at: u64) -> f32 {
        let mut e = 1.0f32;
        if at < self.fade_in {
            e = e.min(at as f32 / self.fade_in as f32);
        }
        let remaining = self.len.saturating_sub(at);
        if remaining < self.fade_out {
            e = e.min(remaining as f32 / self.fade_out as f32);
        }
        e
    }
}

/// Every sounding clip of a project, a snapshot for the audio thread.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AudioPlan {
    pub clips: Vec<ClipSound>,
    /// The EQ and dynamics of each track, by its index in the project.
    pub tracks: Vec<picasso_core::TrackFx>,
}

/// Gives the samples of a file, never waiting: what is not decoded yet is silence.
pub trait SampleSource {
    /// Adds `gain` times the stereo samples of `media`, starting at `source_sample` of the file,
    /// to `out` (interleaved, so `out.len() / 2` samples). Returns false when some of them were
    /// not available yet (they were left silent).
    fn add_samples(
        &self,
        media: MediaId,
        path: &str,
        source_sample: u64,
        gain: f32,
        out: &mut [f32],
    ) -> bool;
}

/// Where timeline frame `frames` is, in samples of the mix.
pub fn frames_to_samples(project: &Project, frames: u64) -> u64 {
    let fps = project.fps;
    let n = u128::from(frames) * u128::from(SAMPLE_RATE) * u128::from(fps.den)
        / u128::from(fps.num.max(1));
    u64::try_from(n).unwrap_or(u64::MAX)
}

/// Decibels as a factor; the quietest setting is silence.
fn db_to_gain(db: f32) -> f32 {
    if db <= MIN_GAIN_DB {
        0.0
    } else {
        10f32.powf(db / 20.0)
    }
}

/// The clips of `project` that make sound: enabled clips on unmuted tracks (only the soloed
/// ones when any track is soloed) whose media is a video or an audio file and whose sound is
/// on (a picture whose sound has a linked clip of its own is silent), with their gain, fades
/// and the pan of their track. (Whether such a file has sound is only known when it is
/// decoded.)
pub fn plan(project: &Project) -> AudioPlan {
    let mut clips = Vec::new();
    // Solo: when any track is soloed, the others fall silent.
    let soloed = project.any_soloed();
    for (index, track) in project
        .tracks
        .iter()
        .enumerate()
        .filter(|(_, t)| !t.muted && (!soloed || t.solo))
    {
        // A balance control: the pan lowers the channel it points away from.
        let left = if track.pan > 0.0 {
            1.0 - track.pan
        } else {
            1.0
        };
        let right = if track.pan < 0.0 {
            1.0 + track.pan
        } else {
            1.0
        };
        // A frozen clip holds a picture; it has no sound.
        for (ci, clip) in track
            .clips
            .iter()
            .enumerate()
            .filter(|(_, c)| c.enabled && c.sound && !c.freeze)
        {
            let Some(item) = project.media(clip.media) else {
                continue;
            };
            let path = match &item.source {
                MediaSource::Video { path } | MediaSource::Audio { path } => path,
                MediaSource::Compound { timeline } => {
                    let audio = clip.audio.sanitized(clip.duration);
                    let gain = db_to_gain(audio.gain_db) * db_to_gain(track.gain_db);
                    let outer = Outer {
                        index,
                        gain,
                        left,
                        right,
                    };
                    nested_sounds(project, *timeline, clip, &outer, 1, &mut clips);
                    continue;
                }
                _ => continue,
            };
            let audio = clip.audio.sanitized(clip.duration);
            // A transition at either end: the sound plays on past the cut and cross-fades.
            let before = ci
                .checked_sub(1)
                .and_then(|i| track.clips.get(i))
                .and_then(|l| picasso_core::transition_frames(l, clip));
            let after = track
                .clips
                .get(ci + 1)
                .and_then(|r| picasso_core::transition_frames(clip, r));
            let (early, late) = if clip.reverse {
                (0, 0)
            } else {
                (
                    before.map_or(0, |f| (f / 2).min(clip.source_in).min(clip.start)),
                    after.map_or(0, |f| f - f / 2),
                )
            };
            clips.push(ClipSound {
                media: clip.media,
                path: path.clone(),
                track: index,
                start: frames_to_samples(project, clip.start - early),
                len: frames_to_samples(project, clip.duration + early + late),
                source_in: frames_to_samples(
                    project,
                    clip.source_in - clip.source_frames(early).min(clip.source_in),
                ),
                speed: clip.speed,
                preserve_pitch: true,
                gain: db_to_gain(audio.gain_db) * db_to_gain(track.gain_db),
                left,
                right,
                fade_in: frames_to_samples(
                    project,
                    before.map_or(audio.fade_in, |f| f.max(audio.fade_in)),
                ),
                fade_out: frames_to_samples(
                    project,
                    after.map_or(audio.fade_out, |f| f.max(audio.fade_out)),
                ),
                reverse: clip.reverse,
            });
        }
    }
    AudioPlan {
        clips,
        tracks: project.tracks.iter().map(|t| t.fx.clone()).collect(),
    }
}

/// What a compound clip passes on to the sounds inside it.
struct Outer {
    index: usize,
    gain: f32,
    left: f32,
    right: f32,
}

/// The sounds of compound clip `outer` (whose media is `timeline`), placed where they are heard
/// on the timeline. A compound clip played faster, slower or backwards is silent.
fn nested_sounds(
    project: &Project,
    timeline: u64,
    outer: &picasso_core::Clip,
    to: &Outer,
    depth: u32,
    clips: &mut Vec<ClipSound>,
) {
    if depth > 8 || outer.speed != 1.0 || outer.reverse {
        return;
    }
    let Some(tracks) = project.timeline_tracks(timeline) else {
        return;
    };
    let (first, last) = (outer.source_in, outer.source_in + outer.duration);
    let soloed = tracks.iter().any(|t| t.solo);
    for track in tracks.iter().filter(|t| !t.muted && (!soloed || t.solo)) {
        let left = to.left
            * if track.pan > 0.0 {
                1.0 - track.pan
            } else {
                1.0
            };
        let right = to.right
            * if track.pan < 0.0 {
                1.0 + track.pan
            } else {
                1.0
            };
        for clip in track
            .clips
            .iter()
            .filter(|c| c.enabled && c.sound && !c.freeze)
        {
            // The part of the clip the compound clip shows.
            let (a, b) = (clip.start.max(first), clip.end().min(last));
            if a >= b {
                continue;
            }
            let Some(item) = project.media(clip.media) else {
                continue;
            };
            let audio = clip.audio.sanitized(clip.duration);
            let gain = to.gain * db_to_gain(audio.gain_db) * db_to_gain(track.gain_db);
            let start = outer.start + (a - first);
            // Backwards, the end of the clip is where its sound starts in the file.
            let cut = if clip.reverse {
                clip.end() - b
            } else {
                a - clip.start
            };
            match &item.source {
                MediaSource::Video { path } | MediaSource::Audio { path } => {
                    clips.push(ClipSound {
                        media: clip.media,
                        path: path.clone(),
                        track: to.index,
                        start: frames_to_samples(project, start),
                        len: frames_to_samples(project, b - a),
                        source_in: frames_to_samples(
                            project,
                            clip.source_in + clip.source_frames(cut),
                        ),
                        speed: clip.speed,
                        preserve_pitch: true,
                        gain,
                        left,
                        right,
                        fade_in: if a == clip.start {
                            frames_to_samples(project, audio.fade_in)
                        } else {
                            0
                        },
                        fade_out: if b == clip.end() {
                            frames_to_samples(project, audio.fade_out)
                        } else {
                            0
                        },
                        reverse: clip.reverse,
                    });
                }
                MediaSource::Compound { timeline } => {
                    let mut inner = clip.clone();
                    (inner.start, inner.source_in, inner.duration) =
                        (start, clip.source_in + (a - clip.start), b - a);
                    let to = Outer {
                        index: to.index,
                        gain,
                        left,
                        right,
                    };
                    nested_sounds(project, *timeline, &inner, &to, depth + 1, clips);
                }
                _ => {}
            }
        }
    }
}

/// Mixes blocks of the timeline. It keeps its buffer between blocks, so mixing does not
/// allocate once the buffer is big enough.
pub struct Mixer {
    scratch: Vec<f32>,
    /// The file's samples before they are resampled to another speed.
    source_buf: Vec<f32>,
    /// The loudest left and right sample of each track in the last block mixed.
    pub track_peaks: [[f32; 2]; METERED_TRACKS],
    /// The tracks with effects are mixed here first, then shaped, then added to the mix.
    track_bufs: Vec<Vec<f32>>,
    /// The effects' memory, by track index.
    pub track_dsp: Vec<crate::dsp::TrackDsp>,
    /// Time stretchers of the clips that keep their pitch, with the block they were last used in.
    stretchers: HashMap<StretchKey, (TimeStretch, u64)>,
    block: u64,
}

/// Which clip a stretcher belongs to: its file, track, place, start in the file and speed.
type StretchKey = (MediaId, usize, u64, u64, u32);

/// Speeds the pitch is kept for; outside them (a freeze, a blur of sound) the clip plays like tape.
const STRETCH_SPEEDS: std::ops::RangeInclusive<f32> = 0.1..=16.0;

impl Default for Mixer {
    fn default() -> Self {
        Self {
            scratch: Vec::new(),
            source_buf: Vec::new(),
            track_peaks: [[0.0; 2]; METERED_TRACKS],
            track_bufs: Vec::new(),
            track_dsp: Vec::new(),
            stretchers: HashMap::new(),
            block: 0,
        }
    }
}

impl Mixer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mixes the timeline from sample `from` into `out` (interleaved stereo; it is overwritten).
    /// Returns false when some sound was not decoded yet.
    pub fn mix(
        &mut self,
        plan: &AudioPlan,
        source: &impl SampleSource,
        from: u64,
        out: &mut [f32],
    ) -> bool {
        out.fill(0.0);
        self.block = self.block.wrapping_add(1);
        self.track_peaks = [[0.0; 2]; METERED_TRACKS];
        // Tracks with effects get a buffer of their own (kept between blocks).
        let shaped = |t: usize| plan.tracks.get(t).is_some_and(|f| !f.is_plain());
        if self.track_bufs.len() < plan.tracks.len() {
            self.track_bufs.resize_with(plan.tracks.len(), Vec::new);
            self.track_dsp
                .resize_with(plan.tracks.len(), crate::dsp::TrackDsp::default);
        }
        for (t, buf) in self.track_bufs.iter_mut().enumerate() {
            if shaped(t) {
                buf.clear();
                buf.resize(out.len(), 0.0);
            }
        }
        let frames = (out.len() / 2) as u64;
        let to = from.saturating_add(frames);
        let mut complete = true;
        for clip in &plan.clips {
            let end = clip.start.saturating_add(clip.len);
            if clip.start >= to || end <= from {
                continue;
            }
            // The part of the block this clip covers.
            let first = clip.start.max(from);
            let last = end.min(to);
            let dst = ((first - from) * 2) as usize;
            let n = ((last - first) * 2) as usize;
            let target: &mut [f32] = match self.track_bufs.get_mut(clip.track) {
                Some(buf) if shaped(clip.track) => buf,
                _ => &mut *out,
            };
            let Some(window) = target.get_mut(dst..dst + n) else {
                continue;
            };
            self.scratch.clear();
            self.scratch.resize(n, 0.0);
            // Where in the file the first sample of this part comes from.
            let at = clip.source_in as f64 + (first - clip.start) as f64 * f64::from(clip.speed);
            let (base, frac) = (at.floor() as u64, (at - at.floor()) as f32);
            let stretch =
                clip.preserve_pitch && clip.speed != 1.0 && STRETCH_SPEEDS.contains(&clip.speed);
            if clip.reverse && !stretch {
                // Read the stretch of the file the part spans, then walk it from its end.
                let samples = n / 2;
                let speed = f64::from(clip.speed);
                let total = clip.len as f64 * speed;
                let hi = clip.source_in as f64 + total - (first - clip.start) as f64 * speed;
                let lo = (hi - samples as f64 * speed).max(clip.source_in as f64);
                let base = lo.floor() as u64;
                let wanted = (hi - base as f64).ceil() as usize + 2;
                self.source_buf.clear();
                self.source_buf.resize(wanted * 2, 0.0);
                complete &=
                    source.add_samples(clip.media, &clip.path, base, 1.0, &mut self.source_buf);
                let top = hi - base as f64 - 1.0;
                for (i, frame) in self.scratch.chunks_exact_mut(2).enumerate() {
                    let x = (top - i as f64 * speed).max(0.0);
                    let (i0, w) = (x as usize, x.fract() as f32);
                    for (c, v) in frame.iter_mut().enumerate() {
                        let a = self.source_buf.get(i0 * 2 + c).copied().unwrap_or(0.0);
                        let b = self.source_buf.get(i0 * 2 + 2 + c).copied().unwrap_or(a);
                        *v = a * (1.0 - w) + b * w;
                    }
                }
            } else if stretch {
                // Time-stretched: the pitch stays while the clip runs faster or slower. A
                // reversed clip is stretched on the sound read backwards: sample `j` of that
                // sound is the file's sample `top - j`, so going forward in it is going back in
                // the file.
                let key = (
                    clip.media,
                    clip.track,
                    clip.start,
                    clip.source_in,
                    clip.speed.to_bits(),
                );
                let entry = self
                    .stretchers
                    .entry(key)
                    .or_insert_with(|| (TimeStretch::new(), 0));
                entry.1 = self.block;
                let speed = f64::from(clip.speed);
                let reversed = clip.reverse;
                let origin = if reversed { 0.0 } else { clip.source_in as f64 };
                let top = (clip.source_in as f64 + clip.len as f64 * speed).round() as i64 - 1;
                let mut got_all = true;
                let mut fetch = |first: i64, buf: &mut [f32]| {
                    // Before the start of the file there is silence.
                    let skip = if first < 0 {
                        (first.unsigned_abs() as usize).min(buf.len() / 2)
                    } else {
                        0
                    };
                    got_all &= source.add_samples(
                        clip.media,
                        &clip.path,
                        first.max(0) as u64,
                        1.0,
                        &mut buf[skip * 2..],
                    );
                };
                let mut backwards = Vec::new();
                let mut read = |first: i64, buf: &mut [f32]| {
                    if !reversed {
                        fetch(first, buf);
                        return;
                    }
                    let frames = buf.len() / 2;
                    backwards.clear();
                    backwards.resize(buf.len(), 0.0);
                    fetch(top - first - (frames as i64 - 1), &mut backwards);
                    for (i, frame) in buf.chunks_exact_mut(2).enumerate() {
                        let from = (frames - 1 - i) * 2;
                        frame.copy_from_slice(&backwards[from..from + 2]);
                    }
                };
                entry.0.render(
                    (first - clip.start) as i64,
                    &mut self.scratch,
                    &|o| origin + o as f64 * speed,
                    &mut read,
                );
                complete &= got_all;
            } else if clip.speed == 1.0 && frac == 0.0 {
                complete &=
                    source.add_samples(clip.media, &clip.path, base, 1.0, &mut self.scratch);
            } else {
                // Read what the part spans, then take samples from it `speed` apart.
                let samples = n / 2;
                let wanted = (frac + samples as f32 * clip.speed).ceil() as usize + 2;
                self.source_buf.clear();
                self.source_buf.resize(wanted * 2, 0.0);
                complete &=
                    source.add_samples(clip.media, &clip.path, base, 1.0, &mut self.source_buf);
                for (i, frame) in self.scratch.chunks_exact_mut(2).enumerate() {
                    let x = frac + i as f32 * clip.speed;
                    let (i0, w) = (x as usize, x.fract());
                    for (c, v) in frame.iter_mut().enumerate() {
                        let a = self.source_buf.get(i0 * 2 + c).copied().unwrap_or(0.0);
                        let b = self.source_buf.get(i0 * 2 + 2 + c).copied().unwrap_or(a);
                        *v = a * (1.0 - w) + b * w;
                    }
                }
            }
            let mut peak = [0.0f32; 2];
            for (i, (frame, dry)) in window
                .chunks_exact_mut(2)
                .zip(self.scratch.chunks_exact(2))
                .enumerate()
            {
                let level = clip.gain * clip.envelope(first - clip.start + i as u64);
                let (l, r) = (dry[0] * level * clip.left, dry[1] * level * clip.right);
                frame[0] += l;
                frame[1] += r;
                peak[0] = peak[0].max(l.abs());
                peak[1] = peak[1].max(r.abs());
            }
            if let Some(p) = self.track_peaks.get_mut(clip.track) {
                p[0] = p[0].max(peak[0]);
                p[1] = p[1].max(peak[1]);
            }
        }
        let block = self.block;
        self.stretchers.retain(|_, v| v.1 == block);
        for (t, fx) in plan.tracks.iter().enumerate() {
            if fx.is_plain() {
                if let Some(d) = self.track_dsp.get_mut(t) {
                    d.reduction_db = 0.0;
                }
                continue;
            }
            let (Some(buf), Some(dsp)) = (self.track_bufs.get_mut(t), self.track_dsp.get_mut(t))
            else {
                continue;
            };
            dsp.process(fx, buf);
            for (o, v) in out.iter_mut().zip(buf.iter()) {
                *o += v;
            }
        }
        complete
    }
}

/// Mixes one block with a fresh [`Mixer`] (for tests and one-off use).
pub fn mix_block(plan: &AudioPlan, source: &impl SampleSource, from: u64, out: &mut [f32]) -> bool {
    Mixer::new().mix(plan, source, from, out)
}

/// Why a mixdown stopped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MixdownError {
    /// `progress` asked to stop.
    Cancelled,
    /// `sink` failed (the file could not be written).
    Sink(String),
}

/// Mixes the timeline from sample `from` up to `to` in blocks and hands each block (interleaved
/// stereo) to `sink`. `progress` gets the samples done and the total, and returns false to stop.
/// Returns the number of samples mixed.
pub fn mixdown(
    plan: &AudioPlan,
    source: &impl SampleSource,
    from: u64,
    to: u64,
    mut sink: impl FnMut(&[f32]) -> Result<(), String>,
    mut progress: impl FnMut(u64, u64) -> bool,
) -> Result<u64, MixdownError> {
    const BLOCK: u64 = 4800;
    let total = to.saturating_sub(from);
    let mut mixer = Mixer::new();
    let mut block = Vec::new();
    let mut done = 0;
    while done < total {
        let n = BLOCK.min(total - done);
        block.clear();
        block.resize((n * 2) as usize, 0.0);
        mixer.mix(plan, source, from + done, &mut block);
        sink(&block).map_err(MixdownError::Sink)?;
        done += n;
        if !progress(done, total) {
            return Err(MixdownError::Cancelled);
        }
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use picasso_core::{ClipAudio, Command, Outcome};

    /// A source where every file is a constant: its media id as the sample value.
    struct Constant;

    impl SampleSource for Constant {
        fn add_samples(&self, media: MediaId, _: &str, _: u64, gain: f32, out: &mut [f32]) -> bool {
            for s in out.iter_mut() {
                *s += media.0 as f32 * gain;
            }
            true
        }
    }

    /// A source whose sample at position n is the number n, in both channels.
    struct Ramp;

    impl SampleSource for Ramp {
        fn add_samples(&self, _: MediaId, _: &str, from: u64, gain: f32, out: &mut [f32]) -> bool {
            for (i, frame) in out.chunks_exact_mut(2).enumerate() {
                frame[0] += (from + i as u64) as f32 * gain;
                frame[1] += (from + i as u64) as f32 * gain;
            }
            true
        }
    }

    /// A source that has nothing decoded.
    struct Nothing;

    impl SampleSource for Nothing {
        fn add_samples(&self, _: MediaId, _: &str, _: u64, _: f32, _: &mut [f32]) -> bool {
            false
        }
    }

    fn video(p: &mut Project, name: &str) -> MediaId {
        let Outcome::Media(m) = (Command::AddMedia {
            name: name.into(),
            source: MediaSource::Video {
                path: format!("{name}.mp4"),
            },
            length: Some(1000),
            default_duration: 48,
        })
        .apply(p)
        .unwrap() else {
            panic!()
        };
        m
    }

    fn left(out: &[f32]) -> Vec<f32> {
        out.chunks_exact(2).map(|f| f[0]).collect()
    }

    #[test]
    fn only_enabled_clips_on_unmuted_tracks_that_are_video_make_sound() {
        let mut p = Project::new("t");
        let v = video(&mut p, "a");
        let solid = {
            let Outcome::Media(m) = (Command::AddMedia {
                name: "s".into(),
                source: MediaSource::ColorBars,
                length: None,
                default_duration: 48,
            })
            .apply(&mut p)
            .unwrap() else {
                panic!()
            };
            m
        };
        let (v1, v2) = (p.tracks[0].id, p.tracks[1].id);
        let add = |p: &mut Project, track, media, start| {
            let Outcome::Clip(c) = (Command::AddClip {
                track,
                media,
                start,
                duration: None,
                with_sound: false,
            })
            .apply(p)
            .unwrap() else {
                panic!()
            };
            c
        };
        let c1 = add(&mut p, v1, v, 0);
        add(&mut p, v1, solid, 100);
        add(&mut p, v2, v, 0);
        assert_eq!(plan(&p).clips.len(), 2, "the bars have no sound");
        Command::SetClipEnabled {
            clip: c1,
            enabled: false,
        }
        .apply(&mut p)
        .unwrap();
        assert_eq!(plan(&p).clips.len(), 1);
        p.tracks[1].muted = true;
        assert!(plan(&p).clips.is_empty());
    }

    #[test]
    fn solo_silences_the_tracks_that_are_not_soloed() {
        let mut p = Project::new("t");
        let m = video(&mut p, "a");
        let (v1, v2) = (p.tracks[0].id, p.tracks[1].id);
        for track in [v1, v2] {
            Command::AddClip {
                track,
                media: m,
                start: 0,
                duration: None,
                with_sound: false,
            }
            .apply(&mut p)
            .unwrap();
        }
        assert_eq!(plan(&p).clips.len(), 2);
        Command::SetTrackSolo {
            track: v2,
            solo: true,
        }
        .apply(&mut p)
        .unwrap();
        let mix = plan(&p);
        assert_eq!(mix.clips.len(), 1, "only the soloed track is heard");
        assert_eq!(mix.clips[0].track, 1);
        Command::SetTrackSolo {
            track: v2,
            solo: false,
        }
        .apply(&mut p)
        .unwrap();
        assert_eq!(plan(&p).clips.len(), 2, "unsoloing brings the rest back");
        // Mute still wins over solo.
        Command::SetTrackSolo {
            track: v2,
            solo: true,
        }
        .apply(&mut p)
        .unwrap();
        Command::SetTrackMuted {
            track: v2,
            muted: true,
        }
        .apply(&mut p)
        .unwrap();
        assert!(plan(&p).clips.is_empty());
    }

    #[test]
    fn a_clip_is_placed_in_samples_at_the_project_rate() {
        let mut p = Project::new("t");
        let m = video(&mut p, "a");
        let v1 = p.tracks[0].id;
        Command::AddClip {
            track: v1,
            media: m,
            start: 24,
            duration: Some(48),
            with_sound: false,
        }
        .apply(&mut p)
        .unwrap();
        let c = &plan(&p).clips[0];
        // 24 fps: 24 frames are one second.
        assert_eq!((c.start, c.len, c.source_in), (48_000, 96_000, 0));
    }

    #[test]
    fn the_plan_carries_gain_fades_and_pan() {
        let mut p = Project::new("t");
        let m = video(&mut p, "a");
        let v1 = p.tracks[0].id;
        let Outcome::Clip(clip) = (Command::AddClip {
            track: v1,
            media: m,
            start: 0,
            duration: Some(48),
            with_sound: false,
        })
        .apply(&mut p)
        .unwrap() else {
            panic!()
        };
        Command::SetClipAudio {
            clip,
            audio: ClipAudio {
                gain_db: -6.0,
                fade_in: 12,
                fade_out: 24,
            },
        }
        .apply(&mut p)
        .unwrap();
        Command::SetTrackMix {
            track: v1,
            gain_db: -6.0,
            pan: 0.5,
        }
        .apply(&mut p)
        .unwrap();
        let c = &plan(&p).clips[0];
        // -6 dB twice is about a quarter; the pan lowers the left channel by half.
        assert!((c.gain - 0.2512).abs() < 0.001, "{}", c.gain);
        assert_eq!((c.left, c.right), (0.5, 1.0));
        assert_eq!((c.fade_in, c.fade_out), (24_000, 48_000));
        // The quietest setting is silence, not a very quiet sound.
        Command::SetTrackMix {
            track: v1,
            gain_db: -60.0,
            pan: 0.0,
        }
        .apply(&mut p)
        .unwrap();
        assert_eq!(plan(&p).clips[0].gain, 0.0);
    }

    #[test]
    fn a_block_gets_only_the_part_each_clip_covers_and_overlaps_add_up() {
        let mut half = ClipSound::plain(MediaId(10), "", 6, 10, 0);
        half.gain = 0.5;
        let plan = AudioPlan {
            clips: vec![ClipSound::plain(MediaId(1), "", 4, 4, 0), half],
            ..Default::default()
        };
        // Samples 2..12 (10 samples, 20 numbers).
        let mut out = vec![9.9; 20];
        assert!(mix_block(&plan, &Constant, 2, &mut out));
        // 2,3: nothing. 4,5: clip 1. 6,7: clip 1 + clip 2. 8..11: clip 2.
        assert_eq!(
            left(&out),
            [0.0, 0.0, 1.0, 1.0, 6.0, 6.0, 5.0, 5.0, 5.0, 5.0]
        );
    }

    #[test]
    fn fades_follow_the_clip_not_the_block() {
        let mut c = ClipSound::plain(MediaId(1), "", 0, 10, 0);
        c.fade_in = 4;
        c.fade_out = 5;
        let plan = AudioPlan {
            clips: vec![c],
            ..Default::default()
        };
        // The same samples mixed as one block and as two blocks are the same.
        let mut whole = vec![0.0; 20];
        mix_block(&plan, &Constant, 0, &mut whole);
        let mut parts = vec![0.0; 20];
        let mut mixer = Mixer::new();
        mixer.mix(&plan, &Constant, 0, &mut parts[..8]);
        mixer.mix(&plan, &Constant, 4, &mut parts[8..]);
        assert_eq!(whole, parts);
        let l = left(&whole);
        // Rising over 4 samples, flat, then falling over the last 5.
        assert_eq!(l[0], 0.0);
        assert!((l[2] - 0.5).abs() < 1e-6);
        assert_eq!(l[4], 1.0);
        assert_eq!(l[5], 1.0);
        assert!((l[7] - 0.6).abs() < 1e-6 && (l[9] - 0.2).abs() < 1e-6);
    }

    #[test]
    fn pan_and_the_meters_apply_per_track() {
        let mut a = ClipSound::plain(MediaId(1), "", 0, 4, 0);
        a.left = 0.5;
        a.track = 0;
        let mut b = ClipSound::plain(MediaId(2), "", 0, 4, 0);
        b.track = 2;
        b.gain = 0.5;
        let plan = AudioPlan {
            clips: vec![a, b],
            ..Default::default()
        };
        let mut out = vec![0.0; 8];
        let mut mixer = Mixer::new();
        mixer.mix(&plan, &Constant, 0, &mut out);
        // Left: 1 * 0.5 + 2 * 0.5; right: 1 + 1.
        assert_eq!(&out[..2], &[1.5, 2.0]);
        assert_eq!(mixer.track_peaks[0], [0.5, 1.0]);
        assert_eq!(mixer.track_peaks[1], [0.0, 0.0]);
        assert_eq!(mixer.track_peaks[2], [1.0, 1.0]);
    }

    #[test]
    fn missing_sound_is_silence_and_is_reported() {
        let plan = AudioPlan {
            clips: vec![ClipSound::plain(MediaId(1), "", 0, 100, 0)],
            ..Default::default()
        };
        let mut out = vec![1.0; 8];
        assert!(!mix_block(&plan, &Nothing, 0, &mut out));
        assert!(out.iter().all(|s| *s == 0.0));
        // Beyond the clip nothing is missing.
        assert!(mix_block(&plan, &Nothing, 1000, &mut out));
    }

    #[test]
    fn a_mixdown_is_the_same_as_mixing_in_one_go() {
        let mut c = ClipSound::plain(MediaId(3), "", 100, 12_000, 0);
        c.fade_in = 500;
        c.fade_out = 700;
        let plan = AudioPlan {
            clips: vec![c],
            ..Default::default()
        };
        let mut mixed = Vec::new();
        let mut reports = Vec::new();
        let n = mixdown(
            &plan,
            &Constant,
            0,
            15_000,
            |block| {
                mixed.extend_from_slice(block);
                Ok(())
            },
            |done, total| {
                reports.push((done, total));
                true
            },
        )
        .unwrap();
        assert_eq!((n, mixed.len()), (15_000, 30_000));
        let mut whole = vec![0.0; 30_000];
        mix_block(&plan, &Constant, 0, &mut whole);
        assert_eq!(mixed, whole);
        assert_eq!(reports.last(), Some(&(15_000, 15_000)));
        // A stop request and a failing sink both end it.
        let stop = mixdown(&plan, &Constant, 0, 15_000, |_| Ok(()), |d, _| d < 5000);
        assert_eq!(stop, Err(MixdownError::Cancelled));
        let fail = mixdown(
            &plan,
            &Constant,
            0,
            15_000,
            |_| Err("disk full".into()),
            |_, _| true,
        );
        assert_eq!(fail, Err(MixdownError::Sink("disk full".into())));
    }

    /// A 440 Hz tone.
    struct Tone;

    impl SampleSource for Tone {
        fn add_samples(&self, _: MediaId, _: &str, from: u64, gain: f32, out: &mut [f32]) -> bool {
            for (i, frame) in out.chunks_exact_mut(2).enumerate() {
                let n = (from + i as u64) as f64;
                let v =
                    (2.0 * std::f64::consts::PI * 440.0 * n / f64::from(SAMPLE_RATE)).sin() as f32;
                frame[0] += v * gain;
                frame[1] += v * gain;
            }
            true
        }
    }

    /// Zero crossings per second of the left channel.
    fn hertz(out: &[f32]) -> f64 {
        let left: Vec<f32> = out.chunks_exact(2).map(|f| f[0]).collect();
        let up = left
            .windows(2)
            .filter(|w| w[0] < 0.0 && w[1] >= 0.0)
            .count();
        up as f64 * f64::from(SAMPLE_RATE) / left.len() as f64
    }

    #[test]
    fn a_faster_clip_keeps_its_pitch_unless_asked_to_play_like_tape() {
        let mut clip = ClipSound::plain(MediaId(1), "", 0, 96_000, 10_000);
        clip.speed = 1.5;
        let play = |preserve: bool| {
            let mut c = clip.clone();
            c.preserve_pitch = preserve;
            let plan = AudioPlan {
                clips: vec![c],
                ..Default::default()
            };
            let mut mixer = Mixer::new();
            let mut all = Vec::new();
            // the way playback asks for it: block after block
            for b in 0..10 {
                let mut out = vec![0.0; 4800 * 2];
                mixer.mix(&plan, &Tone, 20_000 + b * 4800, &mut out);
                all.extend(out);
            }
            all
        };
        assert!((hertz(&play(true)) - 440.0).abs() < 15.0);
        assert!((hertz(&play(false)) - 660.0).abs() < 15.0);
    }

    #[test]
    fn a_reversed_faster_clip_keeps_its_pitch_unless_asked_to_play_like_tape() {
        let mut clip = ClipSound::plain(MediaId(1), "", 0, 96_000, 10_000);
        clip.speed = 1.5;
        clip.reverse = true;
        let play = |preserve: bool| {
            let mut c = clip.clone();
            c.preserve_pitch = preserve;
            let plan = AudioPlan {
                clips: vec![c],
                ..Default::default()
            };
            let mut mixer = Mixer::new();
            let mut all = Vec::new();
            for b in 0..10 {
                let mut out = vec![0.0; 4800 * 2];
                mixer.mix(&plan, &Tone, 20_000 + b * 4800, &mut out);
                all.extend(out);
            }
            all
        };
        assert!(
            (hertz(&play(true)) - 440.0).abs() < 15.0,
            "{}",
            hertz(&play(true))
        );
        assert!((hertz(&play(false)) - 660.0).abs() < 15.0);
        // Slower too, and never silent or with the wrong level.
        let mut slow = clip.clone();
        slow.speed = 0.75;
        slow.preserve_pitch = true;
        let plan = AudioPlan {
            clips: vec![slow],
            ..Default::default()
        };
        let mut mixer = Mixer::new();
        let mut all = Vec::new();
        for b in 0..10 {
            let mut out = vec![0.0; 4800 * 2];
            mixer.mix(&plan, &Tone, 20_000 + b * 4800, &mut out);
            all.extend(out);
        }
        assert!((hertz(&all) - 440.0).abs() < 15.0, "{}", hertz(&all));
        let peak = all.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!((0.8..=1.2).contains(&peak), "peak {peak}");
    }

    #[test]
    fn a_reversed_stretched_clip_plays_the_file_backwards() {
        /// A ramp that rises with the file: sample n is n / 200000.
        struct Ramp;
        impl SampleSource for Ramp {
            fn add_samples(
                &self,
                _: MediaId,
                _: &str,
                from: u64,
                gain: f32,
                out: &mut [f32],
            ) -> bool {
                for (i, frame) in out.chunks_exact_mut(2).enumerate() {
                    let v = (from + i as u64) as f32 / 200_000.0;
                    frame[0] += v * gain;
                    frame[1] += v * gain;
                }
                true
            }
        }
        let mut clip = ClipSound::plain(MediaId(1), "", 0, 48_000, 10_000);
        clip.speed = 2.0;
        clip.reverse = true;
        clip.preserve_pitch = true;
        let plan = AudioPlan {
            clips: vec![clip],
            ..Default::default()
        };
        let mut mixer = Mixer::new();
        let mut all = Vec::new();
        for b in 0..10 {
            let mut out = vec![0.0; 4800 * 2];
            mixer.mix(&plan, &Ramp, b * 4800, &mut out);
            all.extend(out);
        }
        // Played backwards the ramp falls: the start is higher than the end.
        let left: Vec<f32> = all.chunks_exact(2).map(|f| f[0]).collect();
        let mean = |r: std::ops::Range<usize>| left[r.clone()].iter().sum::<f32>() / r.len() as f32;
        assert!(
            mean(4000..8000) > mean(40000..44000) + 0.1,
            "{} vs {}",
            mean(4000..8000),
            mean(40000..44000)
        );
    }

    #[test]
    fn a_stretcher_lives_only_while_its_clip_is_heard() {
        let mut clip = ClipSound::plain(MediaId(1), "", 0, 96_000, 10_000);
        clip.speed = 1.5;
        clip.preserve_pitch = true;
        let plan = AudioPlan {
            clips: vec![clip],
            ..Default::default()
        };
        let mut mixer = Mixer::new();
        let mut out = vec![0.0; 4800 * 2];
        mixer.mix(&plan, &Tone, 0, &mut out);
        assert_eq!(mixer.stretchers.len(), 1);
        // a block where the clip is not heard drops its stretcher
        mixer.mix(&plan, &Tone, 200_000, &mut out);
        assert!(mixer.stretchers.is_empty());
    }

    #[test]
    fn speed_resamples_like_tape() {
        let mut fast = ClipSound::plain(MediaId(1), "", 10, 100, 100);
        fast.speed = 2.0;
        let mut slow = ClipSound::plain(MediaId(1), "", 10, 100, 100);
        slow.speed = 0.5;
        for (clip, expected) in [
            // Timeline sample 10 + i is file sample 100 + i * speed.
            (fast, [100.0, 102.0, 104.0, 106.0]),
            (slow, [100.0, 100.5, 101.0, 101.5]),
        ] {
            let plan = AudioPlan {
                clips: vec![clip],
                ..Default::default()
            };
            let mut out = vec![0.0; 8];
            mix_block(&plan, &Ramp, 10, &mut out);
            assert_eq!(left(&out), expected);
        }
    }

    #[test]
    fn a_transition_cross_fades_the_sounds() {
        let mut p = Project::new("t");
        let m = video(&mut p, "v");
        let v1 = p.tracks[0].id;
        for start in [0, 48] {
            Command::AddClip {
                track: v1,
                media: m,
                start,
                duration: Some(48),
                with_sound: false,
            }
            .apply(&mut p)
            .unwrap();
        }
        let (first, second) = (p.tracks[0].clips[0].id, p.tracks[0].clips[1].id);
        Command::SlipClip {
            clip: second,
            source_in: 100,
        }
        .apply(&mut p)
        .unwrap();
        Command::SetTransition {
            clip: second,
            transition: Some(picasso_core::Transition {
                kind: picasso_core::TransitionKind::CrossDissolve,
                frames: 24,
            }),
        }
        .apply(&mut p)
        .unwrap();
        let sounds = plan(&p).clips;
        let f = |n| frames_to_samples(&p, n);
        let a = sounds.iter().find(|s| s.start == 0).unwrap();
        assert_eq!(
            (a.len, a.fade_out),
            (f(60), f(24)),
            "the first plays 12 frames on"
        );
        let b = sounds.iter().find(|s| s.start != 0).unwrap();
        assert_eq!(
            (b.start, b.len, b.source_in, b.fade_in),
            (f(36), f(60), f(88), f(24)),
            "the second starts 12 frames early, from 12 frames earlier in its file"
        );
        let _ = first;
    }

    #[test]
    fn a_compound_clip_is_heard_where_its_clips_are() {
        let mut p = Project::new("t");
        let m = video(&mut p, "v");
        let v1 = p.tracks[0].id;
        let Outcome::Clip(inner) = (Command::AddClip {
            track: v1,
            media: m,
            start: 24,
            duration: Some(48),
            with_sound: false,
        })
        .apply(&mut p)
        .unwrap() else {
            panic!()
        };
        let Outcome::Clip(compound) = (Command::CreateCompound {
            clips: vec![inner],
            name: "C".into(),
        })
        .apply(&mut p)
        .unwrap() else {
            panic!()
        };
        // Trimmed by 12 frames at the start, the compound clip plays the clip from its 12th
        // frame, at the trimmed start.
        Command::TrimStart {
            clip: compound,
            start: 36,
        }
        .apply(&mut p)
        .unwrap();
        let sounds = plan(&p).clips;
        assert_eq!(sounds.len(), 1);
        let s = &sounds[0];
        assert_eq!(
            (s.start, s.len, s.source_in),
            (
                frames_to_samples(&p, 36),
                frames_to_samples(&p, 36),
                frames_to_samples(&p, 12)
            )
        );
    }

    #[test]
    fn a_reversed_clip_plays_from_its_end() {
        let mut c = ClipSound::plain(MediaId(1), "", 10, 100, 100);
        c.reverse = true;
        let plan = AudioPlan {
            clips: vec![c.clone()],
            ..Default::default()
        };
        let mut out = vec![0.0; 8];
        mix_block(&plan, &Ramp, 10, &mut out);
        assert_eq!(left(&out), [199.0, 198.0, 197.0, 196.0]);
        // In pieces, at another speed, it is the same as in one go.
        c.speed = 1.5;
        let plan = AudioPlan {
            clips: vec![c],
            ..Default::default()
        };
        let mut whole = vec![0.0; 240];
        mix_block(&plan, &Ramp, 0, &mut whole);
        let mut parts = vec![0.0; 240];
        let mut mixer = Mixer::new();
        for (k, chunk) in parts.chunks_mut(14).enumerate() {
            mixer.mix(&plan, &Ramp, (k * 7) as u64, chunk);
        }
        for (a, b) in whole.iter().zip(&parts) {
            assert!((a - b).abs() < 1e-3, "{a} {b}");
        }
        assert!((whole[20] - 249.0).abs() < 1e-3, "{}", whole[20]);
    }

    #[test]
    fn a_fast_clip_mixed_in_pieces_is_the_same_as_in_one_go() {
        let mut c = ClipSound::plain(MediaId(1), "", 3, 50, 7);
        c.speed = 1.37;
        let plan = AudioPlan {
            clips: vec![c],
            ..Default::default()
        };
        let mut whole = vec![0.0; 120];
        mix_block(&plan, &Ramp, 0, &mut whole);
        let mut parts = vec![0.0; 120];
        let mut mixer = Mixer::new();
        for (k, chunk) in parts.chunks_mut(14).enumerate() {
            mixer.mix(&plan, &Ramp, (k * 7) as u64, chunk);
        }
        for (a, b) in whole.iter().zip(&parts) {
            assert!((a - b).abs() < 1e-3, "{a} {b}");
        }
    }

    #[test]
    fn a_picture_whose_sound_has_its_own_clip_is_not_heard_twice() {
        let mut p = Project::new("t");
        let Ok(picasso_core::Outcome::Media(m)) = (Command::AddMedia {
            name: "v".into(),
            source: MediaSource::Video {
                path: "v.mp4".into(),
            },
            length: Some(1000),
            default_duration: 48,
        })
        .apply(&mut p) else {
            panic!()
        };
        let v1 = p.tracks[0].id;
        Command::AddClip {
            track: v1,
            media: m,
            start: 0,
            duration: None,
            with_sound: true,
        }
        .apply(&mut p)
        .unwrap();
        let plan = plan(&p);
        assert_eq!(plan.clips.len(), 1, "only the sound clip plays");
        assert_eq!(
            p.tracks[plan.clips[0].track].kind,
            picasso_core::TrackKind::Audio
        );
    }

    #[test]
    fn a_track_with_an_eq_is_shaped_and_the_others_are_not() {
        // Two steady (DC) sounds on two tracks; the low shelf of the first lifts it by 4x.
        let mut a = ClipSound::plain(MediaId(1), "a", 0, 48_000, 0);
        a.track = 0;
        let mut b = ClipSound::plain(MediaId(2), "b", 0, 48_000, 0);
        b.track = 1;
        let lifted = picasso_core::TrackFx {
            eq: picasso_core::Equalizer {
                enabled: true,
                low_gain: 12.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut plan = AudioPlan {
            clips: vec![a.clone()],
            tracks: vec![lifted, Default::default()],
        };
        let mut mixer = Mixer::new();
        let mut out = vec![0.0f32; 9600];
        for block in 0..4 {
            mixer.mix(&plan, &Constant, block * 4800, &mut out);
        }
        let v = out[out.len() - 2];
        assert!((v - 1.0 * 3.98).abs() < 0.1, "{v}");
        // The other track is left as it is (a fresh mixer: the first track's filter still
        // rings down after its sound ends).
        plan.clips = vec![b];
        Mixer::new().mix(&plan, &Constant, 0, &mut out);
        assert_eq!(out[0], 2.0);
    }
}
