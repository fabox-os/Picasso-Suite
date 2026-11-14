//! The sound of the project: plays the mix while the timeline plays, measures the level for the
//! meter and gives the waveforms the timeline draws. The browser build has no sound yet.

use std::sync::Arc;

use picasso_core::{MediaId, Project};

/// How much sound one scrub plays: a tenth of a second. While the playhead moves a new piece
/// starts at each new place, and the piece it is on is left to finish once the drag pauses.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))] // the browser build has no sound yet
const SCRUB: u64 = picasso_audio::SAMPLE_RATE as u64 / 10;

/// What a scrub does this frame.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScrubStep {
    /// Start a piece at this sample.
    Start(u64),
    /// Move the piece being played to this sample.
    Move(u64),
    /// The piece being played has reached its end: let it fall silent.
    Stop,
    /// The piece being played is still going.
    Keep,
}

/// What the scrub does at `at`, where `playing` is the piece being played (the sample it started
/// at and the sample it stops at) and `position` is where the sound card has got to.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
fn scrub_step(playing: Option<(u64, u64)>, at: u64, position: u64) -> ScrubStep {
    match playing {
        None => ScrubStep::Start(at),
        Some((was, _)) if was != at => ScrubStep::Move(at),
        Some((_, end)) if position >= end => ScrubStep::Stop,
        Some(_) => ScrubStep::Keep,
    }
}

/// Plays the project's sound, in step with the playhead.
pub(crate) struct Sound {
    #[cfg(not(target_arch = "wasm32"))]
    engine: native::Engine,
}

impl Sound {
    /// `ctx` is asked to repaint when a waveform has been measured.
    pub(crate) fn new(ctx: &egui::Context) -> Self {
        #[cfg(target_arch = "wasm32")]
        {
            let _ = ctx;
            Self {}
        }
        #[cfg(not(target_arch = "wasm32"))]
        Self {
            engine: native::Engine::new(ctx.clone()),
        }
    }

    /// Called every frame. `play_edit_key` changes when the project was edited; `playing` is true
    /// while the timeline plays forward at normal speed (the only time sound is played on its
    /// own), and `scrubbing` while the playhead is dragged, when short pieces of the mix are
    /// played as it moves (Audio Scrubbing).
    pub(crate) fn sync(
        &mut self,
        project: &Project,
        edit_key: (u64, u64),
        playhead: u64,
        playing: bool,
        scrubbing: bool,
    ) {
        #[cfg(not(target_arch = "wasm32"))]
        self.engine.sync(project, edit_key, playhead, playing, scrubbing);
        #[cfg(target_arch = "wasm32")]
        let _ = (project, edit_key, playhead, playing, scrubbing);
    }

    /// The level of the left and right channels since the last call, 0.0 to 1.0 and beyond.
    pub(crate) fn meter(&mut self) -> (f32, f32) {
        #[cfg(not(target_arch = "wasm32"))]
        return self.engine.meter();
        #[cfg(target_arch = "wasm32")]
        (0.0, 0.0)
    }

    /// The loudness blocks played since the last call, and the highest true peak among them.
    pub(crate) fn loudness(&mut self) -> (Vec<f64>, f32) {
        #[cfg(not(target_arch = "wasm32"))]
        return self.engine.loudness();
        #[cfg(target_arch = "wasm32")]
        (Vec::new(), 0.0)
    }

    /// The level of each track's sound (left, right) since the last call, in the order of the
    /// project's tracks.
    pub(crate) fn track_meters(&mut self) -> Vec<(f32, f32)> {
        #[cfg(not(target_arch = "wasm32"))]
        return self.engine.track_meters();
        #[cfg(target_arch = "wasm32")]
        Vec::new()
    }

    /// The waveform of a video's sound: one byte per fiftieth of a second. `None` while it is
    /// being measured, or when the file has no sound.
    pub(crate) fn peaks(&self, media: MediaId, path: &str) -> Option<Arc<Vec<u8>>> {
        #[cfg(not(target_arch = "wasm32"))]
        return self.engine.peaks(media, path);
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (media, path);
            None
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::sync::Arc;

    use picasso_audio::{plan, AudioOutput, AudioPlan, AudioPool, Peaks, SAMPLE_RATE};
    use picasso_core::{MediaId, Project};

    use super::{scrub_step, ScrubStep, SCRUB};

    /// The playhead and the sound may be this far apart (in samples) before the sound jumps.
    const CLOCK_SLACK: i64 = SAMPLE_RATE as i64 / 15;
    /// How far ahead of the play position sound is decoded, in seconds.
    const AHEAD: u64 = 4;

    pub(super) struct Engine {
        ctx: egui::Context,
        pool: Arc<AudioPool>,
        output: Option<AudioOutput>,
        /// Opening the sound device failed; do not try again every frame.
        unavailable: bool,
        plan: AudioPlan,
        key: Option<(u64, u64)>,
        active: bool,
        /// The sample the pool was last asked to decode from.
        warmed: Option<u64>,
        /// Where the scrub is playing, and the sample it stops at: `None` when not scrubbing.
        scrub: Option<(u64, u64)>,
        /// The sound being played is a scrub, not the timeline running: it is not measured for
        /// loudness.
        scrubbing: bool,
        meter: (f32, f32),
        /// The loudness blocks already read from the output.
        loudness_seen: u64,
        track_meters: Vec<(f32, f32)>,
    }

    impl Engine {
        pub(super) fn new(ctx: egui::Context) -> Self {
            let repaint = ctx.clone();
            Self {
                pool: Arc::new(AudioPool::new(move || repaint.request_repaint())),
                ctx,
                output: None,
                unavailable: false,
                plan: AudioPlan::default(),
                key: None,
                active: false,
                warmed: None,
                scrub: None,
                scrubbing: false,
                meter: (0.0, 0.0),
                track_meters: Vec::new(),
                loudness_seen: 0,
            }
        }

        pub(super) fn sync(
            &mut self,
            project: &Project,
            key: (u64, u64),
            playhead: u64,
            playing: bool,
            scrubbing: bool,
        ) {
            if self.key != Some(key) {
                // A different project (or an edit): new pool for a new project, new plan always.
                if self.key.is_some_and(|k| k.0 != key.0) {
                    let repaint = self.ctx.clone();
                    self.pool = Arc::new(AudioPool::new(move || repaint.request_repaint()));
                    self.output = None;
                    self.active = false;
                    self.scrub = None;
                }
                self.key = Some(key);
                self.plan = plan(project);
                if let Some(out) = &self.output {
                    out.set_plan(self.plan.clone());
                }
                self.warmed = None;
            }
            let fps = project.fps;
            let at = u64::try_from(
                u128::from(playhead) * u128::from(SAMPLE_RATE) * u128::from(fps.den)
                    / u128::from(fps.num.max(1)),
            )
            .unwrap_or(0);
            if playing {
                self.end_scrub();
                self.open();
                let Some(out) = &self.output else { return };
                if !self.active {
                    out.play(at);
                    self.active = true;
                } else if (out.position() as i64 - at as i64).abs() > CLOCK_SLACK {
                    out.seek(at);
                }
                // Keep decoding ahead of the play position, once a second.
                let position = out.position();
                if self.warmed.is_none_or(|w| {
                    position.saturating_sub(w) >= u64::from(SAMPLE_RATE) / 2 || position < w
                }) {
                    self.pool.prefetch(&self.plan, position, AHEAD);
                    self.warmed = Some(position);
                }
                self.read_levels();
                return;
            }
            if scrubbing {
                self.open();
                if self.output.is_some() {
                    self.scrub_snippet(at);
                    return;
                }
            }
            if self.active {
                if let Some(out) = &self.output {
                    out.pause();
                }
                self.active = false;
            }
            self.end_scrub();
            // Decode what the next play will need, so it starts at once.
            if self.warmed != Some(at) && !self.plan.clips.is_empty() {
                self.pool.prefetch(&self.plan, at, AHEAD);
                self.warmed = Some(at);
            }
        }

        /// Plays a tenth of a second of the mix at `at`, as a dragged playhead does. A new place
        /// starts a new piece; the piece already playing is left to finish when the drag pauses.
        fn scrub_snippet(&mut self, at: u64) {
            let Some(out) = &self.output else { return };
            match scrub_step(self.scrub, at, out.position()) {
                ScrubStep::Start(at) => {
                    out.play(at);
                    self.active = true;
                    self.scrub = Some((at, at.saturating_add(SCRUB)));
                }
                ScrubStep::Move(at) => {
                    if self.active {
                        out.seek(at);
                    } else {
                        out.play(at);
                        self.active = true;
                    }
                    self.scrub = Some((at, at.saturating_add(SCRUB)));
                }
                ScrubStep::Stop => {
                    // The drag stopped: the piece it was on falls silent, and it is not started
                    // again while the playhead stays there (the step keeps saying Stop).
                    out.pause();
                    self.active = false;
                }
                ScrubStep::Keep => {}
            }
            // Decode a second around the playhead as it moves, a quarter of a second apart: a
            // drag asks for no more than that, however many frames it takes.
            if self
                .warmed
                .is_none_or(|w| w.abs_diff(at) >= u64::from(SAMPLE_RATE) / 4)
            {
                self.pool.prefetch(&self.plan, at, 1);
                self.warmed = Some(at);
            }
            self.scrubbing = true;
            self.drop_loudness();
            // A piece must be stopped even when the pointer stops moving and egui goes idle: ask
            // for a frame just after the piece is due to end.
            if self.active {
                self.ctx.request_repaint_after(std::time::Duration::from_millis(
                    SCRUB * 1_000 / u64::from(SAMPLE_RATE) + 5,
                ));
            }
            self.read_levels();
        }

        /// Leaves a scrub, if one was going: what the sound card measured of it is dropped.
        fn end_scrub(&mut self) {
            if self.scrubbing {
                self.drop_loudness();
            }
            self.scrub = None;
            self.scrubbing = false;
        }

        /// Drops the loudness blocks and the peak the sound card measured since they were last
        /// read. A scrub is heard but not counted: what it played would otherwise be counted the
        /// next time the timeline plays, since the blocks wait until they are read.
        fn drop_loudness(&mut self) {
            if let Some(out) = &self.output {
                let _ = out.take_loudness(&mut self.loudness_seen);
                let _ = out.take_true_peak();
            }
        }

        /// Opens the sound device the first time it is needed; one failure is enough.
        fn open(&mut self) {
            if self.output.is_some() || self.unavailable {
                return;
            }
            match AudioOutput::new(self.pool.clone()) {
                Ok(out) => {
                    out.set_plan(self.plan.clone());
                    self.output = Some(out);
                }
                Err(e) => {
                    tracing::warn!("no sound: {e}");
                    self.unavailable = true;
                }
            }
        }

        /// The meter falls back smoothly as the levels are read.
        fn read_levels(&mut self) {
            let Some(out) = &self.output else { return };
            let (l, r) = out.take_levels();
            self.meter = (l.max(self.meter.0 * 0.85), r.max(self.meter.1 * 0.85));
            let tracks = out.take_track_levels();
            self.track_meters.resize(tracks.len(), (0.0, 0.0));
            for (shown, (l, r)) in self.track_meters.iter_mut().zip(tracks) {
                *shown = (l.max(shown.0 * 0.85), r.max(shown.1 * 0.85));
            }
        }

        pub(super) fn loudness(&mut self) -> (Vec<f64>, f32) {
            match &self.output {
                // A scrub is not playback: it is heard, but it does not count towards loudness
                // (what it played is dropped as it goes, see `drop_loudness`).
                Some(out) if self.active && !self.scrubbing => (
                    out.take_loudness(&mut self.loudness_seen),
                    out.take_true_peak(),
                ),
                _ => (Vec::new(), 0.0),
            }
        }

        pub(super) fn meter(&mut self) -> (f32, f32) {
            if !self.active {
                self.meter = (self.meter.0 * 0.8, self.meter.1 * 0.8);
                if self.meter.0 < 0.001 && self.meter.1 < 0.001 {
                    self.meter = (0.0, 0.0);
                }
            }
            self.meter
        }

        pub(super) fn track_meters(&mut self) -> Vec<(f32, f32)> {
            if !self.active {
                for m in &mut self.track_meters {
                    *m = (m.0 * 0.8, m.1 * 0.8);
                    if m.0 < 0.001 && m.1 < 0.001 {
                        *m = (0.0, 0.0);
                    }
                }
            }
            self.track_meters.clone()
        }

        pub(super) fn peaks(&self, media: MediaId, path: &str) -> Option<Arc<Vec<u8>>> {
            match self.pool.peaks(media, path) {
                Peaks::Ready(p) => Some(p),
                Peaks::Pending | Peaks::None => None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scrub_starts_at_the_playhead_and_stops_once_its_piece_has_played() {
        let end = 1_000 + SCRUB;
        // Nothing playing: a piece starts at the playhead.
        assert_eq!(scrub_step(None, 1_000, 0), ScrubStep::Start(1_000));
        // The playhead has not moved: the piece plays on, wherever the sound card is.
        assert_eq!(
            scrub_step(Some((1_000, end)), 1_000, 1_000),
            ScrubStep::Keep
        );
        assert_eq!(
            scrub_step(Some((1_000, end)), 1_000, end - 1),
            ScrubStep::Keep
        );
        // It has run out: the sound stops, though the playhead is still there.
        assert_eq!(
            scrub_step(Some((1_000, end)), 1_000, end),
            ScrubStep::Stop
        );
        // The playhead is still there after its piece has played: it is not played again.
        assert_eq!(
            scrub_step(Some((1_000, end)), 1_000, end + 500),
            ScrubStep::Stop
        );
        // A new place starts a new piece, wherever the old one had got to.
        assert_eq!(
            scrub_step(Some((1_000, end)), 2_000, 1_200),
            ScrubStep::Move(2_000)
        );
        // And after a piece has finished, a new place is a new piece too.
        assert_eq!(
            scrub_step(Some((1_000, end)), 3_000, end),
            ScrubStep::Move(3_000)
        );
        // A tenth of a second of sound per piece, at 48 kHz.
        assert_eq!(SCRUB, 4_800);
    }
}
