//! Clip > Dip Music Under Voice: finds where there is speech on the other tracks and takes
//! the sound of the selected clips down there (`picasso_core::voice dip`), with a ramp down before
//! the speech and a ramp up after it. Clip > Remove Voice Dip takes it away.
//!
//! Adapted from the auto ducking of Drift 0.9.4 by CutWire Studios (GPL-3.0,
//! <https://github.com/CutWire-Studios/Drift>). Everything on the other tracks that makes sound
//! counts as speech: the level of the sound decides, not what it is.

use picasso_core::voice_dip::{merge_spans, VoiceDipSettings, Span};
use picasso_core::{Command, VoiceDip};

use crate::app::StudioApp;

/// How much of one clip's sound is listened to, in seconds of the file; and how much in all for
/// one music clip. The listening is done before the window draws again, so it is kept short.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
const MAX_LISTEN_SECONDS: f64 = 300.0;
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
const MAX_LISTEN_TOTAL_SECONDS: f64 = 900.0;
/// How easily quiet sound counts as speech (0 to 1); the middle of the range.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
const SENSITIVITY: f64 = 0.5;

/// The speech `found` (in seconds of the timeline) made into the voice dip of a clip that spans
/// timeline frames `music.0` to `music.1`, at `seconds_per_frame`. `None` when no speech is
/// inside the clip.
pub(crate) fn dips_for(
    found: Vec<Span>,
    music: (u64, u64),
    seconds_per_frame: f64,
    settings: &VoiceDipSettings,
) -> Option<VoiceDip> {
    if seconds_per_frame.is_nan() || seconds_per_frame <= 0.0 {
        return None;
    }
    let frame = |seconds: f64| (seconds / seconds_per_frame).round().max(0.0) as u64;
    let dips: Vec<(u64, u64)> = merge_spans(found, settings)
        .into_iter()
        .map(|s| (frame(s.start).max(music.0), frame(s.end).min(music.1)))
        .filter(|d| d.1 > d.0)
        .collect();
    if dips.is_empty() {
        return None;
    }
    Some(
        VoiceDip {
            dips,
            reduce_db: settings.reduce_db as f32,
            attack: frame(settings.attack),
            release: frame(settings.release),
        }
        .sanitized(),
    )
}

/// Where speech found at `spans` seconds into what a clip plays of its file (`from` seconds in)
/// is on the timeline, for a clip that plays at `speed` and has its first frame at timeline
/// second `start`.
pub(crate) fn on_timeline(spans: &[Span], start: f64, speed: f64) -> Vec<Span> {
    let speed = speed.max(0.01);
    spans
        .iter()
        .map(|s| Span {
            start: start + s.start / speed,
            end: start + s.end / speed,
        })
        .collect()
}

impl StudioApp {
    /// The selected clips that are heard and have a file: the music.
    #[cfg(not(target_arch = "wasm32"))]
    fn music_clips(&self) -> Vec<picasso_core::ClipId> {
        let project = self.project();
        self.selected_ids()
            .into_iter()
            .chain(self.target_clip())
            .filter(|id| {
                project.clip(*id).is_some_and(|(_, c)| c.sound)
                    && crate::waveform_align::sound_path(project, *id).is_some()
            })
            .fold(Vec::new(), |mut v, id| {
                if !v.contains(&id) {
                    v.push(id);
                }
                v
            })
    }

    /// Whether there is a clip with a file to voice_dip.
    pub(crate) fn can_dip_music(&self) -> bool {
        #[cfg(target_arch = "wasm32")]
        {
            false
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            !self.music_clips().is_empty()
        }
    }

    /// Whether the target clip is dipped.
    pub(crate) fn can_remove_voice_dip(&self) -> bool {
        let project = self.project();
        self.selected_ids()
            .into_iter()
            .chain(self.target_clip())
            .any(|id| project.clip(id).is_some_and(|(_, c)| c.voice_dip.is_some()))
    }

    /// Clip > Dip Music Under Voice.
    pub(crate) fn dip_music(&mut self) {
        #[cfg(target_arch = "wasm32")]
        {
            self.fail("voice dip needs the desktop app");
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let music = self.music_clips();
            if music.is_empty() {
                self.fail("select the music clips to voice_dip first");
                return;
            }
            let settings = VoiceDipSettings::default();
            let mut commands = Vec::new();
            let mut dips = 0;
            let mut cut_short = false;
            for id in music {
                match self.speech_over(id) {
                    Ok((found, truncated)) => {
                        cut_short |= truncated;
                        let project = self.project();
                        let Some((_, clip)) = project.clip(id) else {
                            continue;
                        };
                        let spf = f64::from(project.fps.den) / f64::from(project.fps.num.max(1));
                        if let Some(voice_dip) = dips_for(found, (clip.start, clip.end()), spf, &settings) {
                            dips += voice_dip.dips.len();
                            commands.push(Command::SetVoiceDip { clip: id, voice_dip: Some(voice_dip) });
                        }
                    }
                    Err(e) => {
                        self.fail(e);
                        return;
                    }
                }
            }
            if commands.is_empty() {
                self.fail("no speech found on the other tracks under the selected clips");
                return;
            }
            match self.session.dispatch_all(commands) {
                Ok(_) => self.info(format!(
                    "Dipped the music {dips} time{} by {} dB under speech{}",
                    if dips == 1 { "" } else { "s" },
                    -settings.reduce_db,
                    if cut_short {
                        " (only the first minutes of long clips were listened to)"
                    } else {
                        ""
                    }
                )),
                Err(e) => self.fail(e),
            }
        }
    }

    /// Clip > Remove Voice Dip.
    pub(crate) fn remove_voice_dip(&mut self) {
        let project = self.project();
        let commands: Vec<Command> = self
            .selected_ids()
            .into_iter()
            .chain(self.target_clip())
            .filter(|id| project.clip(*id).is_some_and(|(_, c)| c.voice_dip.is_some()))
            .fold(Vec::new(), |mut v: Vec<_>, id| {
                if !v.contains(&id) {
                    v.push(id);
                }
                v
            })
            .into_iter()
            .map(|clip| Command::SetVoiceDip { clip, voice_dip: None })
            .collect();
        if commands.is_empty() {
            self.fail("the selected clips are not dipped");
            return;
        }
        match self.session.dispatch_all(commands) {
            Ok(_) => self.info("Voice dip removed"),
            Err(e) => self.fail(e),
        }
    }

    /// The speech under the clip `music`, in seconds of the timeline: where the sound of the
    /// clips on the other tracks is loud.
    #[cfg(not(target_arch = "wasm32"))]
    fn speech_over(&self, music: picasso_core::ClipId) -> Result<(Vec<Span>, bool), String> {
        use picasso_core::voice_dip::level_speech_spans;
        let project = self.project();
        let Some((_, m)) = project.clip(music) else {
            return Ok((Vec::new(), false));
        };
        let spf = f64::from(project.fps.den) / f64::from(project.fps.num.max(1));
        let own_track = project.tracks.iter().position(|t| t.clips.iter().any(|c| c.id == music));
        let mut found = Vec::new();
        let (mut budget, mut truncated) = (MAX_LISTEN_TOTAL_SECONDS, false);
        for (ti, track) in project.tracks.iter().enumerate() {
            if Some(ti) == own_track || track.muted {
                continue;
            }
            for clip in &track.clips {
                // Not the picture that belongs to the music, nor what is silent or cannot be read.
                if clip.link.is_some() && clip.link == m.link
                    || !clip.enabled
                    || !clip.sound
                    || clip.freeze
                    || clip.reverse
                {
                    continue;
                }
                let (a, b) = (clip.start.max(m.start), clip.end().min(m.end()));
                if a >= b {
                    continue;
                }
                let Some(path) = crate::waveform_align::sound_path(project, clip.id) else {
                    continue;
                };
                let speed = f64::from(clip.speed).max(0.01);
                let from = (clip.source_in + clip.source_frames(a - clip.start)) as f64 * spf;
                let wanted = (b - a) as f64 * spf * speed;
                let shown = wanted.min(MAX_LISTEN_SECONDS).min(budget);
                if shown < wanted {
                    truncated = true;
                }
                if shown <= 0.0 {
                    continue;
                }
                budget -= shown;
                let sound = crate::waveform_align::clip_sound(&path, from, shown)?;
                let spans =
                    level_speech_spans(&sound, picasso_core::sync::SYNC_RATE, 0.0, SENSITIVITY);
                found.extend(on_timeline(&spans, a as f64 * spf, speed));
            }
        }
        Ok((found, truncated))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(start: f64, end: f64) -> Span {
        Span { start, end }
    }

    #[test]
    fn speech_becomes_dips_kept_inside_the_music_with_ramps_in_frames() {
        // 25 frames a second, the music from frame 50 to 500 (2 s to 20 s).
        let found = vec![span(1.0, 3.0), span(10.0, 11.0), span(30.0, 31.0)];
        let voice_dip = dips_for(found, (50, 500), 0.04, &VoiceDipSettings::default()).unwrap();
        // The first starts before the music (frame 25): cut to frame 50. The last is after it.
        assert_eq!(voice_dip.dips, vec![(50, 75), (250, 275)]);
        assert_eq!((voice_dip.attack, voice_dip.release), (5, 13));
        assert_eq!(voice_dip.reduce_db, -12.0);
    }

    #[test]
    fn no_speech_in_the_clip_gives_no_voice_dip() {
        let s = VoiceDipSettings::default();
        assert!(dips_for(vec![], (0, 100), 0.04, &s).is_none());
        assert!(dips_for(vec![span(50.0, 60.0)], (0, 100), 0.04, &s).is_none());
        assert!(dips_for(vec![span(1.0, 2.0)], (0, 100), 0.0, &s).is_none());
    }

    #[test]
    fn faster_playback_brings_the_speech_closer_and_the_start_moves_it() {
        let spans = [span(2.0, 4.0)];
        let there = on_timeline(&spans, 10.0, 2.0);
        assert_eq!(there, vec![span(11.0, 12.0)]);
        // A speed that is not usable is the slowest there is, not a division by zero.
        assert!(on_timeline(&spans, 0.0, 0.0)[0].end.is_finite());
    }
}
