//! Mark > Add Markers on Beats: finds the beat of the sound of the clip being worked on
//! (`picasso_core::beats`) and puts a marker on each beat of the timeline, the first beat of
//! every bar in another colour. Where there is no steady tempo, the strong hits get the markers.

use picasso_core::beats::Beats;
use picasso_core::Command;

use crate::app::StudioApp;

/// How much of the clip's sound is looked at, from its first shown frame.
const WINDOW_SECONDS: f64 = 180.0;
/// The most markers one run adds.
const MAX_MARKERS: usize = 600;
/// Beats in a bar, for the marker colour of the first one.
const BEATS_PER_BAR: usize = 4;

/// A marker to add: its timeline frame, name and colour.
pub(crate) type NewMarker = (u64, String, u8);

/// The markers for what was found in a clip that starts at timeline frame `start`, shows its file
/// from `source_start` seconds at `speed` and is `duration` frames long. Frames that have a marker
/// already (`taken`) are left alone.
pub(crate) fn markers_for(
    found: &Beats,
    (start, duration): (u64, u64),
    seconds_per_frame: f64,
    speed: f64,
    taken: &[u64],
) -> Vec<NewMarker> {
    let frame_of = |seconds: f64| {
        let frames = seconds / speed.max(0.01) / seconds_per_frame;
        (frames >= 0.0).then(|| start + frames.round() as u64)
    };
    let mut out: Vec<NewMarker> = Vec::new();
    let mut push = |frame: Option<u64>, name: String, color: u8| {
        let Some(frame) = frame else {
            return;
        };
        if frame >= start + duration || taken.contains(&frame) || out.iter().any(|m| m.0 == frame) {
            return;
        }
        out.push((frame, name, color));
    };
    if found.bpm.is_some() {
        for (i, beat) in found.beats.iter().enumerate() {
            if i % BEATS_PER_BAR == 0 {
                push(frame_of(*beat), format!("Bar {}", i / BEATS_PER_BAR + 1), 3);
            } else {
                push(frame_of(*beat), String::new(), 0);
            }
        }
    } else {
        for onset in found.onsets.iter().filter(|o| o.strength >= 0.5) {
            push(frame_of(onset.seconds), "Hit".into(), 5);
        }
    }
    out.truncate(MAX_MARKERS);
    out
}

impl StudioApp {
    /// Whether there is a clip with a file to take the sound from.
    pub(crate) fn can_mark_beats(&self) -> bool {
        #[cfg(target_arch = "wasm32")]
        {
            false
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.target_clip()
                .is_some_and(|id| crate::waveform_align::sound_path(self.project(), id).is_some())
        }
    }

    /// Mark > Add Markers on Beats.
    pub(crate) fn mark_beats(&mut self) {
        #[cfg(target_arch = "wasm32")]
        {
            self.fail("finding the beat needs the desktop app");
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let Some(id) = self.target_clip() else {
                self.fail("select a clip first");
                return;
            };
            let project = self.project();
            let Some((_, clip)) = project.clip(id) else {
                return;
            };
            let Some(path) = crate::waveform_align::sound_path(project, id) else {
                self.fail("this clip has no file to take the sound from");
                return;
            };
            if clip.reverse || clip.freeze {
                self.fail("a reversed or frozen clip has no beat to find");
                return;
            }
            let seconds_per_frame = f64::from(project.fps.den) / f64::from(project.fps.num.max(1));
            let from = clip.source_in as f64 * seconds_per_frame;
            let speed = f64::from(clip.speed).max(0.01);
            let shown = clip.duration as f64 * seconds_per_frame * speed;
            let (start, duration) = (clip.start, clip.duration);
            let taken: Vec<u64> = project.markers.iter().map(|m| m.frame).collect();
            let sound = match crate::waveform_align::clip_sound(&path, from, shown.min(WINDOW_SECONDS)) {
                Ok(s) => s,
                Err(e) => {
                    self.fail(e);
                    return;
                }
            };
            let found = picasso_core::beats::analyze(&sound, picasso_core::sync::SYNC_RATE);
            let markers = markers_for(&found, (start, duration), seconds_per_frame, speed, &taken);
            if markers.is_empty() {
                self.fail("no beat found in the sound of this clip");
                return;
            }
            let count = markers.len();
            let commands: Vec<Command> = markers
                .into_iter()
                .map(|(frame, name, color)| Command::SetMarker { frame, name, color })
                .collect();
            match self.session.dispatch_all(commands) {
                Ok(_) => self.info(match found.bpm {
                    Some(bpm) => format!("Marked {count} beats at {bpm:.1} BPM"),
                    None => format!("Marked {count} hits (no steady tempo)"),
                }),
                Err(e) => self.fail(e),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use picasso_core::beats::Onset;

    fn found(bpm: Option<f64>, beats: &[f64], onsets: &[(f64, f32)]) -> Beats {
        Beats {
            onsets: onsets.iter().map(|&(seconds, strength)| Onset { seconds, strength }).collect(),
            bpm,
            confidence: 0.9,
            beats: beats.to_vec(),
        }
    }

    #[test]
    fn beats_become_markers_with_the_first_of_each_bar_named() {
        // 120 BPM at 25 frames a second: a beat every 12.5 frames.
        let beats: Vec<f64> = (0..9).map(|k| 0.2 + 0.5 * k as f64).collect();
        let markers = markers_for(&found(Some(120.0), &beats, &[]), (100, 1000), 0.04, 1.0, &[]);
        assert_eq!(markers.len(), 9);
        assert_eq!(markers[0], (105, "Bar 1".to_string(), 3));
        assert_eq!(markers[1].0, 118);
        assert_eq!(markers[1].2, 0);
        assert_eq!(markers[4].1, "Bar 2");
        assert_eq!(markers[8].1, "Bar 3");
        // Faster playback puts them closer together; markers that are taken are not covered.
        let fast = markers_for(&found(Some(120.0), &beats, &[]), (100, 1000), 0.04, 2.0, &[102]);
        assert_eq!(fast[0].0, 103, "{fast:?}");
        assert!(fast.iter().all(|m| m.0 != 102));
        let busy = markers_for(&found(Some(120.0), &beats, &[]), (100, 1000), 0.04, 1.0, &[105]);
        assert_eq!(busy.len(), 8);
    }

    #[test]
    fn without_a_tempo_the_strong_hits_are_marked_and_nothing_past_the_clip() {
        let hits = [(1.0, 0.9f32), (2.0, 0.3), (3.0, 0.7), (200.0, 1.0)];
        let markers = markers_for(&found(None, &[], &hits), (0, 100), 0.04, 1.0, &[]);
        // 1.0 s and 3.0 s are frames 25 and 75; the weak hit and the one past the clip are left.
        assert_eq!(markers, vec![(25, "Hit".to_string(), 5), (75, "Hit".to_string(), 5)]);
        assert!(markers_for(&found(None, &[], &[]), (0, 100), 0.04, 1.0, &[]).is_empty());
    }
}
