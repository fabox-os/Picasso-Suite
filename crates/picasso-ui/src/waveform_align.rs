//! Clip > Auto Align Clips > Based on Waveform: the selected clips are moved so that the same sound
//! falls on the same timeline frame. The earliest clip stays; the others are placed from how their
//! sound lines up with its (`picasso_core::sync`). Nothing moves unless every clip finds the
//! sound of the first: a wrong alignment is worse than none.

use std::path::Path;

use picasso_core::project::MediaSource;
use picasso_core::sync::{best_lag, SYNC_RATE};
use picasso_core::{ClipId, Command, Project};

use crate::app::StudioApp;

/// How much of each clip's sound is compared, from its first shown frame: a minute is more than
/// a clap or a spoken slate needs, and what is read stays small.
const WINDOW_SECONDS: f64 = 60.0;

/// The sound a clip shows, mono at [`SYNC_RATE`], from `from_seconds` of its file for at most
/// `seconds`. An error in words when the file has no sound or cannot be read.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn clip_sound(path: &str, from_seconds: f64, seconds: f64) -> Result<Vec<f32>, String> {
    use picasso_decode::audio::{AudioDecoder, SAMPLE_RATE};
    let mut decoder = AudioDecoder::open(Path::new(path))
        .map_err(|e| format!("{path}: {e}"))?
        .ok_or_else(|| format!("{path} has no sound"))?;
    let every = (SAMPLE_RATE / SYNC_RATE) as usize;
    let first = (from_seconds.max(0.0) * f64::from(SAMPLE_RATE)).round() as u64;
    let total = (seconds.max(0.0) * f64::from(SAMPLE_RATE)).round() as usize;
    let mut out = Vec::with_capacity(total / every + 1);
    // In pieces of ten seconds, so what is held at once stays small.
    let piece = SAMPLE_RATE as usize * 10;
    let mut done = 0;
    while done < total {
        let count = piece.min(total - done);
        let stereo = decoder
            .read_at(first + done as u64, count)
            .map_err(|e| format!("{path}: {e}"))?;
        let mono: Vec<f32> = stereo.chunks_exact(2).map(|s| (s[0] + s[1]) * 0.5).collect();
        out.extend(mono.chunks(every).map(|c| c.iter().sum::<f32>() / c.len() as f32));
        done += count;
    }
    Ok(out)
}

/// The clips of `ids` that stand for their link groups: a picture and the sound linked to it are one.
fn one_per_group(project: &Project, ids: &[ClipId]) -> Vec<ClipId> {
    let mut chosen: Vec<ClipId> = Vec::new();
    for id in ids {
        if chosen.iter().any(|c| project.partners(*c).contains(id)) {
            continue;
        }
        chosen.push(*id);
    }
    chosen
}

/// The file a clip's sound is read from.
pub(crate) fn sound_path(project: &Project, id: ClipId) -> Option<String> {
    let (_, clip) = project.clip(id)?;
    match &project.media(clip.media)?.source {
        MediaSource::Video { path } | MediaSource::Audio { path } => Some(path.clone()),
        _ => None,
    }
}

impl StudioApp {
    /// Whether the selection can be aligned by waveform: two clips or more (a picture and its sound
    /// count once), each of a file.
    pub(crate) fn can_align_by_waveform(&self) -> bool {
        let project = self.project();
        let ids = one_per_group(project, &self.selected_ids());
        ids.len() >= 2 && ids.iter().all(|id| sound_path(project, *id).is_some())
    }

    /// Clip > Auto Align Clips > Based on Waveform.
    pub(crate) fn align_by_waveform(&mut self) {
        #[cfg(target_arch = "wasm32")]
        {
            self.fail("aligning by waveform needs the desktop app");
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let ids = one_per_group(self.project(), &self.selected_ids());
            if ids.len() < 2 {
                self.fail("select two clips or more to align");
                return;
            }
            match self.waveform_starts(&ids) {
                Ok(starts) => {
                    let n = starts.len();
                    if self.run(Command::PlaceClips { starts }).is_some() {
                        self.info(format!("Aligned {n} clips by their sound"));
                    }
                }
                Err(e) => self.fail(e),
            }
        }
    }

    /// Where each clip goes so that its sound lines up with the sound of the earliest one.
    #[cfg(not(target_arch = "wasm32"))]
    fn waveform_starts(&self, ids: &[ClipId]) -> Result<Vec<(ClipId, u64)>, String> {
        let project = self.project();
        let seconds_per_frame = f64::from(project.fps.den) / f64::from(project.fps.num.max(1));
        // The earliest clip anchors, so the others move to it; on a tie, the first made.
        let anchor = *ids
            .iter()
            .min_by_key(|id| project.clip(**id).map_or((u64::MAX, u64::MAX), |(_, c)| (c.start, id.0)))
            .ok_or("select two clips or more to align")?;
        let mut sounds: Vec<(ClipId, String, u64, Vec<f32>)> = Vec::new();
        for id in ids {
            let (_, clip) = project.clip(*id).ok_or("a clip is gone")?;
            let path = sound_path(project, *id)
                .ok_or_else(|| format!("{} has no file to take the sound from", clip.name))?;
            if clip.reverse || clip.freeze || (clip.speed - 1.0).abs() > 1e-3 {
                return Err(format!(
                    "{} is reversed, frozen or at another speed: align clips at normal speed",
                    clip.name
                ));
            }
            let from = clip.source_in as f64 * seconds_per_frame;
            let span = (clip.duration as f64 * seconds_per_frame).min(WINDOW_SECONDS);
            let samples = clip_sound(&path, from, span)?;
            sounds.push((*id, clip.name.clone(), clip.start, samples));
        }
        let Some(base) = sounds.iter().position(|(id, ..)| *id == anchor) else {
            return Err("select two clips or more to align".into());
        };
        let (anchor_start, anchor_name) = (sounds[base].2, sounds[base].1.clone());
        let mut placed: Vec<(ClipId, i64)> = Vec::with_capacity(sounds.len());
        for (id, name, _, samples) in &sounds {
            if *id == anchor {
                placed.push((*id, anchor_start as i64));
                continue;
            }
            let Some(lag) = best_lag(&sounds[base].3, samples) else {
                return Err(format!(
                    "could not find the sound of {anchor_name} in {name}: they share no sound that is clear enough (nothing was moved)"
                ));
            };
            // `name` sounds like the anchor `lag` samples later: it starts that long after it.
            let frames = (lag.samples as f64 / f64::from(SYNC_RATE) / seconds_per_frame).round() as i64;
            placed.push((*id, anchor_start as i64 + frames));
        }
        // None may go before the start of the timeline: all move together by what is needed.
        let lowest = placed.iter().map(|(_, s)| *s).min().unwrap_or(0);
        let shift = if lowest < 0 { -lowest } else { 0 };
        Ok(placed
            .into_iter()
            .map(|(id, s)| (id, (s + shift) as u64))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_and_its_sound_are_one_clip_to_align() {
        let mut p = Project::new("w");
        let v1 = p.tracks[0].id;
        let v2 = p.tracks[1].id;
        let clip = |p: &mut Project, track, start: u64| {
            let picasso_core::Outcome::Media(media) = (Command::AddMedia {
                name: "m".into(),
                source: MediaSource::Video {
                    path: "/media/m.mov".into(),
                },
                length: Some(1000),
                default_duration: 100,
            })
            .apply(p)
            .unwrap() else {
                panic!("media")
            };
            let picasso_core::Outcome::Clip(id) = (Command::AddClip {
                track,
                media,
                start,
                duration: Some(100),
                with_sound: true,
            })
            .apply(p)
            .unwrap() else {
                panic!("clip")
            };
            id
        };
        let a = clip(&mut p, v1, 0);
        let b = clip(&mut p, v2, 300);
        let mut all = vec![a, b];
        all.extend(p.partners(a));
        all.extend(p.partners(b));
        assert_eq!(all.len(), 4, "two pictures with their sound");
        let chosen = one_per_group(&p, &all);
        assert_eq!(chosen.len(), 2, "{chosen:?}");
        assert!(chosen.iter().all(|id| sound_path(&p, *id).is_some()));
    }

    /// A sound file from ffmpeg (noise from a seed, so it never repeats), `None` without ffmpeg.
    fn wav(dir: &std::path::Path, name: &str, input: &[&str], filter: &[&str]) -> Option<String> {
        let path = dir.join(name);
        std::process::Command::new("ffmpeg")
            .args(["-y", "-v", "error"])
            .args(input)
            .args(filter)
            .arg(&path)
            .status()
            .is_ok_and(|s| s.success())
            .then(|| path.to_string_lossy().into_owned())
    }

    /// The same sound recorded twice, the second starting 1.5 s into the first and quieter: its clip
    /// is placed 1.5 s after the first's, and a clip with no sound in common is refused.
    #[test]
    fn two_recordings_of_one_sound_are_lined_up_and_a_stranger_is_refused() {
        let dir = std::env::temp_dir().join("picasso_waveform_align");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let noise = |seed: &str| format!("anoisesrc=d=20:c=pink:r=48000:seed={seed}");
        let (Some(a), Some(b), Some(other)) = (
            wav(&dir, "a.wav", &["-f", "lavfi", "-i", &noise("5")], &[]),
            wav(&dir, "b.wav", &["-i", dir.join("a.wav").to_str().unwrap()], &["-ss", "1.5", "-t", "12", "-af", "volume=0.3"]),
            wav(&dir, "other.wav", &["-f", "lavfi", "-i", &noise("77")], &[]),
        ) else {
            return; // no ffmpeg here
        };
        let ctx = egui::Context::default();
        let mut app = StudioApp::new(&ctx);
        let fps = app.project().fps;
        let frames = |seconds: f64| (seconds * f64::from(fps.num) / f64::from(fps.den)).round() as u64;
        let add = |app: &mut StudioApp, path: &str, start: u64, seconds: f64, shown: f64| {
            let track = match app
                .session
                .dispatch(Command::AddTrack { kind: picasso_core::TrackKind::Audio })
                .unwrap()
            {
                picasso_core::Outcome::Track(t) => t,
                other => panic!("{other:?}"),
            };
            let media = match app
                .session
                .dispatch(Command::AddMedia {
                    name: "m".into(),
                    source: MediaSource::Audio { path: path.into() },
                    length: Some(frames(seconds)),
                    default_duration: frames(seconds),
                })
                .unwrap()
            {
                picasso_core::Outcome::Media(m) => m,
                other => panic!("{other:?}"),
            };
            match app
                .session
                .dispatch(Command::AddClip {
                    track,
                    media,
                    start,
                    duration: Some(frames(shown)),
                    with_sound: false,
                })
                .unwrap()
            {
                picasso_core::Outcome::Clip(c) => c,
                other => panic!("{other:?}"),
            }
        };
        let first = add(&mut app, &a, 200, 20.0, 20.0);
        let second = add(&mut app, &b, 900, 12.0, 10.0);
        let starts = app.waveform_starts(&[first, second]).expect("the sound is found");
        assert_eq!(starts.len(), 2);
        let (_, s1) = starts.iter().find(|(id, _)| *id == first).copied().unwrap();
        let (_, s2) = starts.iter().find(|(id, _)| *id == second).copied().unwrap();
        assert_eq!(s1, 200, "the earliest clip stays");
        assert!(
            s2.abs_diff(200 + frames(1.5)) <= 1,
            "the second starts 1.5 s after it: {s2} against {}",
            200 + frames(1.5)
        );
        // A clip that shows its file from a second in is lined up by what it shows: from 2.5 s of
        // the first recording on.
        app.session
            .dispatch(Command::SlipClip { clip: second, source_in: frames(1.0) })
            .unwrap();
        let starts = app.waveform_starts(&[first, second]).expect("found when slipped");
        let (_, slipped) = starts.iter().find(|(id, _)| *id == second).copied().unwrap();
        assert!(
            slipped.abs_diff(200 + frames(2.5)) <= 1,
            "{slipped} against {}",
            200 + frames(2.5)
        );
        // A recording with nothing in common is not guessed at.
        let stranger = add(&mut app, &other, 1500, 20.0, 20.0);
        let refused = app.waveform_starts(&[first, stranger]);
        assert!(refused.is_err(), "{refused:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
