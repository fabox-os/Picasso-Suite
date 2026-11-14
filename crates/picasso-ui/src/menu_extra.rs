//! The menu entries that open a small window or change a view setting: Change Clip Speed and
//! Change Clip Duration, Label Node, the timecode window, the edit index, the manual, and the
//! timeline settings Position Lock and Selection Follows Playhead.

use egui::{Context, RichText};
use picasso_core::{ClipId, Command, Project, TrackKind};

use crate::app::{Page, StudioApp};

/// The state of those windows and settings.
#[derive(Default)]
pub(crate) struct Extra {
    /// Change Clip Speed: the clip and the speed being typed, in percent.
    pub speed: Option<(ClipId, f32)>,
    /// Change Clip Duration: the clip and the duration being typed, in frames.
    pub duration: Option<(ClipId, u64)>,
    /// Label Node: the clip, node and the words being typed.
    pub label: Option<(ClipId, usize, String)>,
    pub timecode_window: bool,
    pub edit_index: bool,
    pub manual: bool,
    /// Clips cannot be moved or trimmed in the timeline.
    pub position_lock: bool,
    /// The clip under the playhead is selected as it moves.
    pub selection_follows: bool,
    /// Normalize Audio Levels: the target sample peak in dBFS.
    pub normalize: Option<f32>,
    /// The System Preferences window is open.
    pub preferences: bool,
    /// The Media Management window is open.
    pub media_management: bool,
    /// Trim mode: dragging a clip slips it, dragging an edge ripples the track.
    pub trim_mode: bool,
    /// Dynamic trim: the chosen edit follows the playhead when playback stops or a scrub ends.
    pub dynamic_trim: bool,
    /// The edit (left clip, right clip) dynamic trim moves.
    pub dynamic_edit: Option<(ClipId, ClipId)>,
    /// Whether playback was running at the last frame (to see it stop).
    pub was_playing: bool,
    /// After a trim, play around the edit in a loop to judge it.
    pub loop_trim: bool,
    /// Dragging an edit moves only the clip's own edge and ripples the track, not both sides.
    pub asymmetric: bool,
    /// The Speed Ramp window.
    pub ramp: Option<RampDialog>,
    /// The Create Subtitles from Audio window, while it is open.
    pub auto_subtitles: Option<crate::auto_subtitles::AutoSubtitles>,
    /// The Choose Video Layout window, while it is open.
    pub video_layout: Option<crate::video_layout::VideoLayoutChooser>,
    /// The project's video layout has been asked about (or the project is past the question).
    pub video_layout_decided: bool,
    /// The Audio FX window: the track and whether it shows the EQ or the dynamics.
    pub track_fx: Option<(picasso_core::TrackId, crate::track_fx::FxTab)>,
    /// Clip Attributes: the clip and the name being typed.
    pub attributes: Option<(ClipId, String)>,
    /// Rename Bin: the bin and the name being typed.
    pub rename_bin: Option<(u64, String)>,
    /// Workspace › Layout › Save Layout: the name being typed.
    pub save_layout: Option<String>,
    /// Workspace › Layout › Load Layout: the list of the saved layouts is open.
    pub load_layout: bool,
    /// The saved layouts the two layout windows list, read from the disk when one opens and again
    /// after a save, not on every frame.
    pub layouts: Option<Vec<crate::layouts::Layout>>,
    /// New Smart Bin: its name, the words to look for and the type (0 is any).
    pub smart_bin: Option<(String, String, usize)>,
    /// What System Preferences and Keyboard Customization set for this computer.
    pub prefs: crate::prefs::Preferences,
    /// Keyboard Customization.
    pub keys: crate::keyboard::KeyEditor,
}

/// The Speed Ramp window: from one speed to another over the clip, in steps, along a curve.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RampDialog {
    pub clip: ClipId,
    /// In percent.
    pub from: f32,
    pub to: f32,
    pub steps: usize,
    /// 0 linear, 1 ease in (slow then fast), 2 ease out.
    pub curve: usize,
}

impl RampDialog {
    pub(crate) fn new(clip: ClipId) -> Self {
        Self {
            clip,
            from: 100.0,
            to: 300.0,
            steps: 8,
            curve: 0,
        }
    }

    /// The speed of each piece, as factors (1.0 is normal).
    pub(crate) fn speeds(&self) -> Vec<f32> {
        let n = self.steps.clamp(2, picasso_core::MAX_RAMP_STEPS);
        (0..n)
            .map(|i| {
                let t = (i as f32 + 0.5) / n as f32;
                let t = match self.curve {
                    1 => t * t,
                    2 => 1.0 - (1.0 - t) * (1.0 - t),
                    _ => t,
                };
                (self.from + (self.to - self.from) * t) / 100.0
            })
            .collect()
    }
}

/// The types a smart bin can keep to (0 is any).
const SMART_KINDS: [&str; 7] = [
    "Any",
    "Video",
    "Audio",
    "Still",
    "Title",
    "Generator",
    "Compound Clip",
];

impl StudioApp {
    pub(crate) fn open_speed_dialog(&mut self) {
        match self
            .target_clip()
            .and_then(|id| self.project().clip(id).map(|(_, c)| (id, c.speed)))
        {
            Some((id, speed)) => self.extra.speed = Some((id, speed * 100.0)),
            None => self.fail("select a clip first"),
        }
    }

    pub(crate) fn open_duration_dialog(&mut self) {
        match self
            .target_clip()
            .and_then(|id| self.project().clip(id).map(|(_, c)| (id, c.duration)))
        {
            Some((id, frames)) => self.extra.duration = Some((id, frames)),
            None => self.fail("select a clip first"),
        }
    }

    pub(crate) fn open_label_dialog(&mut self) {
        let Some(clip) = self.current_clip() else {
            self.fail("there is no clip at the playhead");
            return;
        };
        let node = self.node_for(clip);
        let label = self
            .project()
            .clip(clip)
            .and_then(|(_, c)| c.node(node))
            .map(|g| g.label.clone())
            .unwrap_or_default();
        self.extra.label = Some((clip, node, label));
    }

    /// Mark Timeline In/Out: In at the first frame, Out at the last.
    pub(crate) fn mark_timeline(&mut self) {
        let end = self.project().duration();
        if end == 0 {
            self.fail("the timeline is empty");
            return;
        }
        let ok = self.run(Command::SetMarks {
            mark_in: Some(0),
            mark_out: Some(end - 1),
        });
        if ok.is_some() {
            self.info("Marked the whole timeline");
        }
    }

    /// Select Nearest Edit Point: the playhead goes to the nearest cut and the clip that starts
    /// there is selected.
    pub(crate) fn select_nearest_edit(&mut self) {
        let at = self.playhead;
        let nearest = self
            .project()
            .tracks
            .iter()
            .flat_map(|t| t.clips.iter())
            .flat_map(|c| [(c.start, Some(c.id)), (c.end(), None)])
            .min_by_key(|(edge, starts)| (edge.abs_diff(at), starts.is_none()));
        match nearest {
            Some((edge, clip)) => {
                self.go_to(edge);
                if let Some(id) = clip {
                    self.select_only(id);
                }
            }
            None => self.fail("there is no edit point"),
        }
    }

    /// Plays two seconds either side of the In (or the Out when there is no In).
    pub(crate) fn play_around_mark(&mut self) {
        let Some(mark) = self.project().mark_in.or(self.project().mark_out) else {
            self.fail("set an In or an Out first");
            return;
        };
        self.go_to(mark);
        self.do_act(&Context::default(), crate::menu::Act::PlayAround);
    }

    /// Previous / Next Node Graph: the Color page goes to the clip before or after.
    pub(crate) fn go_to_clip(&mut self, forward: bool) {
        let at = self.playhead;
        let starts = self
            .project()
            .tracks
            .iter()
            .filter(|t| t.kind == TrackKind::Video)
            .flat_map(|t| t.clips.iter())
            .map(|c| c.start);
        let current = self
            .current_clip()
            .and_then(|id| self.project().clip(id))
            .map_or(at, |(_, c)| c.start);
        let target = if forward {
            starts.filter(|s| *s > current).min()
        } else {
            starts.filter(|s| *s < current).max()
        };
        match target {
            Some(frame) => {
                self.go_to(frame);
                self.color_node = 0;
            }
            None => self.info(if forward {
                "This is the last clip"
            } else {
                "This is the first clip"
            }),
        }
    }

    /// Add Parallel Node / Add Layer Mixer: a new node after the current one that joins it as
    /// `mode` says; it becomes the edited one.
    pub(crate) fn add_mode_node(&mut self, mode: picasso_color::NodeMode) {
        let Some(clip) = self.current_clip() else {
            self.fail("there is no clip at the playhead");
            return;
        };
        let n = self.node_for(clip);
        let grade = picasso_color::ColorGrade {
            mode,
            ..Default::default()
        };
        let result = self.session.dispatch_all(vec![
            Command::AddSerialNode { clip, after: n },
            Command::SetGrade {
                clip,
                grade,
                node: n + 1,
            },
        ]);
        match result {
            Ok(_) => {
                self.color_node = n + 1;
                self.info(format!("Added {} node {:02}", mode.name(), n + 2));
            }
            Err(e) => self.fail(e),
        }
    }

    /// Add Serial Before: a new node in front of the current one, which becomes the edited one.
    pub(crate) fn add_node_before(&mut self) {
        let Some(clip) = self.current_clip() else {
            self.fail("there is no clip at the playhead");
            return;
        };
        let n = self.node_for(clip);
        let Some(grade) = self
            .project()
            .clip(clip)
            .and_then(|(_, c)| c.node(n))
            .cloned()
        else {
            return;
        };
        // A node after it takes its grade; it becomes the new, empty one.
        let result = self.session.dispatch_all(vec![
            Command::AddSerialNode { clip, after: n },
            Command::SetGrade {
                clip,
                grade,
                node: n + 1,
            },
            Command::SetGrade {
                clip,
                grade: picasso_color::ColorGrade::default(),
                node: n,
            },
        ]);
        match result {
            Ok(_) => {
                self.color_node = n;
                self.info(format!("Added node {:02}", n + 1));
            }
            Err(e) => self.fail(e),
        }
    }

    /// New Timeline Using Selected Clips: a new timeline with copies of the selected clips,
    /// in their order, from its first frame.
    pub(crate) fn timeline_from_selection(&mut self) {
        if self.selected_ids().is_empty() {
            self.fail("select clips first");
            return;
        }
        if !self.copy_selection() {
            return;
        }
        self.new_timeline();
        self.go_to(0);
        self.do_act(&Context::default(), crate::menu::Act::Paste);
    }

    /// Cuts every clip that a marker falls inside, at the marker, as one undo step.
    pub(crate) fn cut_at_markers(&mut self) {
        let markers: Vec<u64> = self.project().markers.iter().map(|m| m.frame).collect();
        if markers.is_empty() {
            self.fail("there are no markers to cut at");
            return;
        }
        // Each cut is planned on the clips as they are after the earlier cuts: clips that a
        // marker falls inside, cut from the last marker back so earlier cuts keep their ids.
        let mut commands = Vec::new();
        let mut sorted = markers;
        sorted.sort_unstable();
        sorted.dedup();
        for at in sorted.into_iter().rev() {
            for c in self
                .project()
                .tracks
                .iter()
                .filter(|t| !t.locked)
                .flat_map(|t| t.clips.iter())
                .filter(|c| c.start < at && at < c.end())
            {
                // A linked partner is cut with its clip.
                let partner_listed = c.link.is_some()
                    && commands.iter().any(|cmd| match cmd {
                        Command::Split { clip, at: a } => {
                            *a == at
                                && self
                                    .project()
                                    .clip(*clip)
                                    .is_some_and(|(_, o)| o.link == c.link)
                        }
                        _ => false,
                    });
                if !partner_listed {
                    commands.push(Command::Split { clip: c.id, at });
                }
            }
        }
        if commands.is_empty() {
            self.fail("no marker falls inside a clip");
            return;
        }
        let n = commands.len();
        match self.session.dispatch_all(commands) {
            Ok(_) => self.info(format!("Made {n} cuts at the markers")),
            Err(e) => self.fail(e),
        }
    }

    /// Selects the topmost video clip under the playhead (Selection Follows Playhead).
    pub(crate) fn follow_playhead(&mut self) {
        if !self.extra.selection_follows {
            return;
        }
        let at = self.playhead;
        let top = self
            .project()
            .tracks
            .iter()
            .rev()
            .filter(|t| t.kind == TrackKind::Video)
            .find_map(|t| Project::clip_at(t, at))
            .map(|c| c.id);
        if let Some(id) = top {
            if self.selection != Some(id) {
                self.select_only(id);
            }
        }
    }

    /// The clips a clip command works on: the selected ones, or the clip at the playhead.
    fn chosen_clips(&self) -> Vec<ClipId> {
        let selected = self.selected_ids();
        if selected.is_empty() {
            self.target_clip().into_iter().collect()
        } else {
            selected.into_iter().collect()
        }
    }

    /// Lock Clip: locks the chosen clips, or unlocks them when the first one is locked.
    pub(crate) fn toggle_clip_lock(&mut self) {
        let clips = self.chosen_clips();
        let Some(first) = clips.first() else {
            self.fail("select a clip first");
            return;
        };
        let locked = !self.project().clip(*first).is_some_and(|(_, c)| c.locked);
        let commands = clips
            .iter()
            .map(|&clip| Command::SetClipLocked { clip, locked })
            .collect();
        match self.session.dispatch_all(commands) {
            Ok(_) => self.info(if locked {
                "Locked: the clip cannot be moved, trimmed or deleted"
            } else {
                "Unlocked"
            }),
            Err(e) => self.fail(e),
        }
    }

    /// Set Clip Color for the chosen clips.
    pub(crate) fn set_clip_color(&mut self, color: Option<u8>) {
        let commands: Vec<Command> = self
            .chosen_clips()
            .into_iter()
            .map(|clip| Command::SetClipColor { clip, color })
            .collect();
        if commands.is_empty() {
            self.fail("select a clip first");
            return;
        }
        if let Err(e) = self.session.dispatch_all(commands) {
            self.fail(e);
        }
    }

    /// The loudest sample of a clip's sound (0..1), from its waveform; None while the waveform
    /// is being measured or when the clip has no sound.
    pub(crate) fn clip_peak(&self, clip: ClipId) -> Option<f32> {
        let (_, c) = self.project().clip(clip)?;
        let path = match &self.project().media(c.media)?.source {
            picasso_core::MediaSource::Video { path }
            | picasso_core::MediaSource::Audio { path } => path.clone(),
            _ => return None,
        };
        let peaks = self.sound.peaks(c.media, &path)?;
        let fps = self.project().fps;
        let per_frame = f64::from(fps.den) / f64::from(fps.num.max(1));
        let first = (c.source_in as f64 * per_frame * 50.0) as usize;
        let last = ((c.source_in + c.source_span()) as f64 * per_frame * 50.0).ceil() as usize;
        let range = first.min(peaks.len())..last.min(peaks.len());
        let peak = peaks.get(range)?.iter().max().copied()?;
        (peak > 0).then(|| f32::from(peak) / 255.0)
    }

    /// Sets the gain of the chosen clips so their loudest sample reaches `target_db` dBFS.
    pub(crate) fn normalize_audio(&mut self, target_db: f32) {
        let mut commands = Vec::new();
        let mut silent = 0;
        for clip in self.chosen_clips() {
            let Some((_, c)) = self.project().clip(clip) else {
                continue;
            };
            let mut audio = c.audio;
            match self.clip_peak(clip) {
                Some(peak) => {
                    audio.gain_db = target_db - 20.0 * peak.log10();
                    commands.push(Command::SetClipAudio { clip, audio });
                }
                None => silent += 1,
            }
        }
        if commands.is_empty() {
            self.fail("the clips have no sound to measure (or it is still being measured)");
            return;
        }
        let n = commands.len();
        match self.session.dispatch_all(commands) {
            Ok(_) if silent > 0 => self.info(format!(
                "Normalized {n} clips; {silent} had no sound to measure"
            )),
            Ok(_) => self.info(format!("Normalized {n} clips to {target_db:.1} dBFS")),
            Err(e) => self.fail(e),
        }
    }

    fn normalize_window(&mut self, ctx: &Context) {
        let Some(mut target) = self.extra.normalize else {
            return;
        };
        let mut open = true;
        let mut done = None;
        egui::Window::new("Normalize Audio Levels")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Normalization Mode");
                    ui.strong("Sample Peak Program");
                });
                ui.horizontal(|ui| {
                    ui.label("Target Level");
                    ui.add(
                        egui::DragValue::new(&mut target)
                            .range(-60.0..=0.0)
                            .speed(0.1)
                            .suffix(" dBFS"),
                    );
                });
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Normalize").clicked() {
                        done = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        done = Some(false);
                    }
                });
            });
        self.extra.normalize = Some(target);
        if done == Some(true) {
            self.normalize_audio(target);
        }
        if !open || done.is_some() {
            self.extra.normalize = None;
        }
    }

    /// File > Export > Timeline: EDL: asks where, then writes the timeline as a CMX 3600 list.
    pub(crate) fn export_edl(&mut self, ctx: &Context) {
        let title = self.project().timeline_name.clone();
        let text = picasso_core::edl::to_edl(self.project(), &title);
        let stem: String = title
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '_' })
            .collect();
        crate::platform::save_as(
            self.inbox.clone(),
            ctx.clone(),
            format!("{stem}.edl"),
            ("Edit decision list", vec!["edl"]),
            text.into_bytes(),
            false,
        );
    }

    /// Writes the timeline as an OpenTimelineIO file, for Resolve, Premiere, Avid and the like.
    pub(crate) fn export_otio(&mut self, ctx: &Context) {
        let title = self.project().timeline_name.clone();
        let text = picasso_core::otio::to_otio(self.project());
        let stem: String = title
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '_' })
            .collect();
        crate::platform::save_as(
            self.inbox.clone(),
            ctx.clone(),
            format!("{stem}.otio"),
            ("OpenTimelineIO", vec!["otio"]),
            text.into_bytes(),
            false,
        );
    }

    /// Reads an EDL into a new timeline, matching its clips to the media pool by name (the
    /// clip name, else the reel). Events whose media is not in the pool are left out.
    pub(crate) fn import_edl(&mut self, name: &str, text: &str) {
        let events = match picasso_core::edl::parse_edl(text, self.project().fps) {
            Ok(events) => events,
            Err(e) => {
                self.fail(format!("{name}: {e}"));
                return;
            }
        };
        let find = |project: &Project, wanted: &str| {
            let wanted = wanted.to_lowercase();
            let stem = |s: &str| {
                let s = s.to_lowercase();
                s.rsplit_once('.').map_or(s.clone(), |(a, _)| a.to_string())
            };
            project
                .media
                .iter()
                .find(|m| m.name.to_lowercase() == wanted)
                .or_else(|| {
                    project
                        .media
                        .iter()
                        .find(|m| stem(&m.name) == stem(&wanted))
                })
                .map(|m| m.id)
        };
        self.new_timeline();
        let title = name
            .rsplit_once('.')
            .map_or(name, |(stem, _)| stem)
            .to_string();
        let id = self.project().timeline_id;
        self.run(Command::RenameTimeline { id, name: title });
        let track_of = |project: &Project, kind: TrackKind| {
            project.tracks.iter().find(|t| t.kind == kind).map(|t| t.id)
        };
        let mut places = Vec::new();
        let mut slips = Vec::new();
        let mut missing = 0;
        for e in &events {
            let kind = if e.audio {
                TrackKind::Audio
            } else {
                TrackKind::Video
            };
            match (
                find(self.project(), &e.name),
                track_of(self.project(), kind),
            ) {
                (Some(media), Some(track)) => {
                    places.push(Command::AddClip {
                        track,
                        media,
                        start: e.record,
                        duration: Some(e.duration),
                        with_sound: false,
                    });
                    slips.push(e.source_in);
                }
                _ => missing += 1,
            }
        }
        if places.is_empty() {
            self.fail(format!(
                "{name}: none of its {} clips is in the media pool (import the media first)",
                events.len()
            ));
            return;
        }
        let placed = places.len();
        let result = self.session.dispatch_all_then(places, move |outcomes| {
            outcomes
                .iter()
                .zip(&slips)
                .filter_map(|(o, source_in)| match o {
                    picasso_core::Outcome::Clip(clip) if *source_in > 0 => {
                        Some(Command::SlipClip {
                            clip: *clip,
                            source_in: *source_in,
                        })
                    }
                    _ => None,
                })
                .collect()
        });
        match result {
            Ok(_) if missing > 0 => self.info(format!(
                "Imported {placed} clips from {name}; {missing} were not in the media pool"
            )),
            Ok(_) => self.info(format!("Imported {placed} clips from {name}")),
            Err(e) => self.fail(format!("{name}: {e}")),
        }
    }

    /// How a subtitle looks: smaller than a title, low in the picture.
    fn subtitle_style(text: &str) -> picasso_core::TitleText {
        picasso_core::TitleText {
            text: text.chars().take(picasso_core::MAX_TITLE_CHARS).collect(),
            size: 55,
            y: 880,
            ..picasso_core::TitleText::default()
        }
    }

    /// Timeline > Create Subtitle Region: a subtitle at the playhead, three seconds long.
    pub(crate) fn add_subtitle(&mut self) {
        let start = self.playhead;
        let length = self.project().fps.nominal().max(1) * 3;
        let made = self.session.dispatch_all_then(
            vec![Command::AddMedia {
                name: "Subtitle".into(),
                source: picasso_core::MediaSource::Title(Self::subtitle_style("Subtitle")),
                length: None,
                default_duration: length,
            }],
            move |outcomes| match outcomes.first() {
                Some(picasso_core::Outcome::Media(media)) => vec![Command::PlaceOnTop {
                    media: *media,
                    start,
                    with_sound: false,
                }],
                _ => Vec::new(),
            },
        );
        match made {
            Ok(outcomes) => {
                if let Some(picasso_core::Outcome::Clip(id)) = outcomes.last() {
                    self.select_only(*id);
                }
                self.info("Added a subtitle: type its words in the Inspector");
            }
            Err(e) => self.fail(e),
        }
    }

    /// File > Import > Subtitle: each cue of an SRT or WebVTT file becomes a subtitle on a new
    /// track, as one undo step.
    pub(crate) fn import_subtitles(&mut self, name: &str, text: &str) {
        let cues = crate::subtitles::parse(text, self.project().fps);
        if cues.is_empty() {
            self.fail(format!("{name}: no subtitles found"));
            return;
        }
        self.place_subtitles(name, cues);
    }

    /// Puts `cues` (in frames of the timeline) on a new track as subtitles, one undo step.
    pub(crate) fn place_subtitles(&mut self, name: &str, cues: Vec<crate::subtitles::Cue>) {
        let mut first: Vec<Command> = cues
            .iter()
            .enumerate()
            .map(|(n, cue)| Command::AddMedia {
                name: format!("Subtitle {}", n + 1),
                source: picasso_core::MediaSource::Title(Self::subtitle_style(&cue.text)),
                length: None,
                default_duration: cue.end - cue.start,
            })
            .collect();
        first.push(Command::AddTrack {
            kind: TrackKind::Video,
        });
        let spans: Vec<(u64, u64)> = cues.iter().map(|c| (c.start, c.end - c.start)).collect();
        let result = self.session.dispatch_all_then(first, move |outcomes| {
            let Some(picasso_core::Outcome::Track(track)) = outcomes.last() else {
                return Vec::new();
            };
            outcomes
                .iter()
                .zip(&spans)
                .filter_map(|(o, (start, length))| match o {
                    picasso_core::Outcome::Media(media) => Some(Command::AddClip {
                        track: *track,
                        media: *media,
                        start: *start,
                        duration: Some(*length),
                        with_sound: false,
                    }),
                    _ => None,
                })
                .collect()
        });
        match result {
            Ok(_) => self.info(format!("Added {} subtitles from {name}", cues.len())),
            Err(e) => self.fail(format!("{name}: {e}")),
        }
    }

    /// File > Export > Subtitle: the timeline's titles as an SRT file.
    pub(crate) fn export_subtitles(&mut self, ctx: &Context) {
        let end = self.project().duration();
        let cues = crate::subtitles::cues(self.project(), 0, end);
        if cues.is_empty() {
            self.fail("there are no titles or subtitles on the timeline");
            return;
        }
        let text = crate::subtitles::write(
            crate::subtitles::SubtitleKind::Srt,
            &cues,
            self.project().fps,
        );
        let title = self.project().timeline_name.clone();
        let stem: String = title
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '_' })
            .collect();
        crate::platform::save_as(
            self.inbox.clone(),
            ctx.clone(),
            format!("{stem}.srt"),
            ("SubRip subtitles", vec!["srt"]),
            text.into_bytes(),
            false,
        );
    }

    /// Clip > Add to Media Pool as Stills: the picture at the playhead, at the project's size,
    /// becomes a still in the media pool (kept as a PNG in the app's Stills folder).
    pub(crate) fn still_to_pool(&mut self) {
        let (w, h) = (self.project().width, self.project().height);
        let png = match picasso_render::render_frame(
            self.session.project(),
            &self.cache.blocking(),
            self.playhead,
            w,
            h,
        )
        .and_then(|f| picasso_render::encode_png(&f))
        {
            Ok(png) => png,
            Err(e) => {
                self.fail(e);
                return;
            }
        };
        let tc = self.tc(self.playhead as i64).replace(':', "_");
        let name = format!("Still {tc}.png");
        // On the desktop the picture is kept as a file, so the project can find it again.
        // (Tests leave the user's folder alone.)
        let path = crate::project_manager::data_dir()
            .filter(|_| !cfg!(test))
            .map(|d| d.join("Stills"))
            .filter(|d| std::fs::create_dir_all(d).is_ok())
            .map(|d| d.join(&name))
            .filter(|p| std::fs::write(p, &png).is_ok())
            .map(|p| p.to_string_lossy().into_owned());
        self.import_file(crate::platform::ImportedFile {
            name,
            path,
            bytes: png,
        });
    }

    fn preferences_window(&mut self, ctx: &Context) {
        let mut open = self.extra.preferences;
        if !open {
            return;
        }
        let mut snap = self.timeline.snap();
        let mut linked = self.project().linked_selection;
        let mut proxy = self.proxy;
        let mut looping = self.looping;
        let mut lock = self.extra.position_lock;
        let mut follow = self.extra.selection_follows;
        let mut volume = self.monitor.volume;
        let mut scrubbing = self.extra.prefs.audio_scrubbing;
        let mut hardware = self.extra.prefs.hardware_decoding;
        egui::Window::new("System Preferences")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.heading("Editing");
                ui.checkbox(&mut snap, "Snap to clips, markers and the playhead");
                ui.checkbox(&mut linked, "Linked selection (picture and sound together)");
                ui.checkbox(&mut lock, "Position lock");
                ui.checkbox(&mut follow, "Selection follows the playhead");
                ui.separator();
                ui.heading("Playback");
                ui.checkbox(&mut looping, "Loop playback");
                ui.horizontal(|ui| {
                    ui.label("Timeline proxy");
                    for (label, v) in [("Off", 1), ("Half", 2), ("Quarter", 4), ("Eighth", 8)] {
                        ui.radio_value(&mut proxy, v, label);
                    }
                });
                ui.add(egui::Slider::new(&mut volume, 0.0..=1.0).text("Monitor volume"));
                ui.checkbox(&mut scrubbing, "Audio scrubbing (sound while the playhead is dragged)");
                ui.separator();
                ui.heading("Decode Options");
                ui.checkbox(
                    &mut hardware,
                    "Decode supported video using hardware acceleration",
                );
                ui.label(
                    RichText::new(
                        "H.264, H.265, AV1, VP9, MPEG-2 and VC-1 are decoded on the graphics card \
                         when its driver can. Turn this off if clips show green or broken \
                         pictures. PICASSO_NO_HWACCEL=1 turns it off for one run.",
                    )
                    .small()
                    .weak(),
                );
                ui.separator();
                ui.label(format!("Picasso Suite {}", env!("CARGO_PKG_VERSION")));
            });
        if snap != self.timeline.snap() {
            self.do_act(ctx, crate::menu::Act::ToggleSnap);
        }
        if linked != self.project().linked_selection {
            self.do_act(ctx, crate::menu::Act::ToggleLinkedSelection);
        }
        if proxy != self.proxy {
            self.set_proxy(proxy);
        }
        self.looping = looping;
        self.extra.position_lock = lock;
        self.extra.selection_follows = follow;
        if volume != self.monitor.volume {
            self.set_monitor(|m| m.volume = volume);
        }
        self.set_audio_scrubbing(scrubbing);
        self.set_hardware_decoding(hardware);
        self.extra.preferences = open;
    }

    /// Turns audio scrubbing on or off (the Clip menu's entry, and System Preferences >
    /// Playback), for this computer: with it on, dragging the playhead plays short pieces of the
    /// sound as it moves.
    pub(crate) fn set_audio_scrubbing(&mut self, on: bool) {
        if on == self.extra.prefs.audio_scrubbing {
            return;
        }
        self.extra.prefs.audio_scrubbing = on;
        #[cfg(not(test))]
        self.extra.prefs.save();
        self.info(if on {
            "Audio scrubbing is on: the sound follows the playhead as it is dragged"
        } else {
            "Audio scrubbing is off: dragging the playhead is silent"
        });
    }

    /// Turns decoding on the graphics card on or off (System Preferences > Decode Options, and
    /// Playback > Hardware Decoding), for this computer. The clips open again the new way as they
    /// are next drawn, and the viewer pictures made the old way are forgotten, so a picture the
    /// card got wrong does not keep showing.
    pub(crate) fn set_hardware_decoding(&mut self, on: bool) {
        if on == self.extra.prefs.hardware_decoding {
            return;
        }
        self.extra.prefs.hardware_decoding = on;
        self.extra.prefs.apply();
        #[cfg(not(test))]
        self.extra.prefs.save();
        let _ = self.render_cache.clear();
        self.media_generation += 1;
        self.info(if on {
            "Hardware decoding is on: video is decoded on the graphics card where it can be"
        } else {
            "Hardware decoding is off: video is decoded on the processor"
        });
    }

    fn rename_bin_window(&mut self, ctx: &Context) {
        let Some((id, mut name)) = self.extra.rename_bin.take() else {
            return;
        };
        let mut open = true;
        let mut done = None;
        egui::Window::new("Rename Bin")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                let field = ui.text_edit_singleline(&mut name);
                if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    done = Some(true);
                }
                ui.horizontal(|ui| {
                    if ui.button("OK").clicked() {
                        done = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        done = Some(false);
                    }
                });
            });
        if done == Some(true) {
            self.run(Command::RenameBin { id, name });
        } else if open && done.is_none() {
            self.extra.rename_bin = Some((id, name));
        }
    }

    /// Workspace › Layout › Save Layout: the sizes of the panels on every page under a name.
    fn save_layout_window(&mut self, ctx: &Context) {
        let Some(mut name) = self.extra.save_layout.take() else {
            return;
        };
        let saved = self
            .extra
            .layouts
            .get_or_insert_with(crate::layouts::list)
            .iter()
            .map(|l| l.name.clone())
            .collect::<Vec<_>>();
        let mut open = true;
        let mut done = None;
        egui::Window::new("Save Layout")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label("Name");
                let named = !name.trim().is_empty();
                let field = ui.text_edit_singleline(&mut name);
                // Enter saves only when there is a name, as the button does.
                if named && field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    done = Some(true);
                }
                ui.horizontal(|ui| {
                    if ui.add_enabled(named, egui::Button::new("Save")).clicked() {
                        done = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        done = Some(false);
                    }
                });
                if !saved.is_empty() {
                    ui.separator();
                    ui.label(RichText::new(format!("Saved: {}", saved.join(", "))).weak());
                }
            });
        if done == Some(true) {
            let panels = self.extra.prefs.panels.clone();
            match crate::layouts::save(&name, &panels) {
                Ok(()) => self.info(format!("Saved the layout {}", name.trim())),
                Err(why) => self.fail(why),
            }
            self.extra.layouts = None;
        } else if open && done.is_none() {
            self.extra.save_layout = Some(name);
        } else if !self.extra.load_layout {
            self.extra.layouts = None;
        }
    }

    /// Workspace › Layout › Load Layout: one of the saved sizes back.
    fn load_layout_window(&mut self, ctx: &Context) {
        if !self.extra.load_layout {
            return;
        }
        let layouts = self.extra.layouts.get_or_insert_with(crate::layouts::list).clone();
        let mut open = true;
        let mut chosen: Option<crate::layouts::Layout> = None;
        egui::Window::new("Load Layout")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                if layouts.is_empty() {
                    ui.label("No layouts are saved yet.");
                    ui.label(
                        RichText::new(
                            "Workspace › Layout › Save Layout keeps the sizes you have now.",
                        )
                        .weak(),
                    );
                }
                for layout in &layouts {
                    if ui.button(&layout.name).clicked() {
                        chosen = Some(layout.clone());
                    }
                }
            });
        if let Some(layout) = chosen {
            let name = &layout.name;
            match crate::layouts::load(&layout) {
                Some(panels) => {
                    self.extra.prefs.panels = panels;
                    #[cfg(not(test))]
                    self.extra.prefs.save();
                    self.info(format!("Loaded the layout {name}"));
                }
                None => self.fail(format!("the layout {name} could not be read")),
            }
            open = false;
        }
        self.extra.load_layout = open;
        if !open && self.extra.save_layout.is_none() {
            self.extra.layouts = None;
        }
    }

    fn smart_bin_window(&mut self, ctx: &Context) {
        let Some((mut name, mut contains, mut kind)) = self.extra.smart_bin.take() else {
            return;
        };
        let mut open = true;
        let mut done = None;
        egui::Window::new("New Smart Bin")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                egui::Grid::new("smart_bin").num_columns(2).show(ui, |ui| {
                    ui.label("Name");
                    ui.text_edit_singleline(&mut name);
                    ui.end_row();
                    ui.label("Clip name contains");
                    ui.text_edit_singleline(&mut contains);
                    ui.end_row();
                    ui.label("Type");
                    egui::ComboBox::from_id_salt("smart_kind")
                        .selected_text(SMART_KINDS.get(kind).copied().unwrap_or("Any"))
                        .show_ui(ui, |ui| {
                            for (n, k) in SMART_KINDS.iter().enumerate() {
                                ui.selectable_value(&mut kind, n, *k);
                            }
                        });
                    ui.end_row();
                });
                ui.horizontal(|ui| {
                    if ui.button("Create Smart Bin").clicked() {
                        done = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        done = Some(false);
                    }
                });
            });
        if done == Some(true) {
            let name = if name.trim().is_empty() {
                format!("Smart Bin {}", self.project().smart_bins.len() + 1)
            } else {
                name
            };
            let made = self.run(Command::AddSmartBin {
                name,
                contains,
                kind: (kind > 0)
                    .then(|| SMART_KINDS.get(kind).map(|k| k.to_string()))
                    .flatten(),
            });
            if made.is_some() {
                if let Some(b) = self.project().smart_bins.last() {
                    self.pool.bin = crate::pool::BinView::Smart(b.id);
                }
            }
        } else if open && done.is_none() {
            self.extra.smart_bin = Some((name, contains, kind));
        }
    }

    /// File > Import > Folder as Bin: the media files of a folder (not its sub-folders), in a new
    /// bin named after the folder.
    pub(crate) fn import_folder_as_bin(&mut self, folder: &str) {
        #[cfg(target_arch = "wasm32")]
        {
            let _ = folder;
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            use crate::platform;
            let dir = std::path::Path::new(folder);
            let Ok(entries) = std::fs::read_dir(dir) else {
                self.fail(format!("cannot read {folder}"));
                return;
            };
            let mut files: Vec<String> = entries
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.is_file())
                .filter_map(|p| p.to_str().map(str::to_string))
                .filter(|p| {
                    let name = p.to_lowercase();
                    platform::is_video_name(&name)
                        || platform::is_audio_name(&name)
                        || [
                            ".png", ".jpg", ".jpeg", ".bmp", ".gif", ".tif", ".tiff", ".webp",
                        ]
                        .iter()
                        .any(|e| name.ends_with(e))
                })
                .collect();
            files.sort();
            files.truncate(1000);
            if files.is_empty() {
                self.fail(format!("there is no media in {folder}"));
                return;
            }
            let name = dir.file_name().map_or_else(
                || "Folder".to_string(),
                |n| n.to_string_lossy().into_owned(),
            );
            if self.run(Command::AddBin { name: name.clone() }).is_none() {
                return;
            }
            let Some(bin) = self.project().bins.last().map(|b| b.id) else {
                return;
            };
            let before: Vec<picasso_core::MediaId> =
                self.project().media.iter().map(|m| m.id).collect();
            for f in &files {
                self.open_path_now(f);
            }
            let moves: Vec<Command> = self
                .project()
                .media
                .iter()
                .filter(|m| !before.contains(&m.id))
                .map(|m| Command::SetMediaBin {
                    media: m.id,
                    bin: Some(bin),
                })
                .collect();
            let n = moves.len();
            if let Err(e) = self.session.dispatch_all(moves) {
                self.fail(e);
                return;
            }
            self.pool.bin = crate::pool::BinView::Bin(bin);
            self.info(format!("Imported {n} files into the bin {name}"));
        }
    }

    /// A ripple trim of clip `id`: its start (`at_start`) or end goes to timeline frame `to`,
    /// the clip keeps its place and the clips after it on the track move by the difference,
    /// as one undo step.
    pub(crate) fn ripple_edit(&mut self, id: ClipId, at_start: bool, to: u64) {
        let Some((track, clip)) = self.project().clip(id) else {
            return;
        };
        let (start, end, track_id) = (clip.start, clip.end(), track.id);
        let edge = if at_start { start } else { end };
        if to == edge {
            return;
        }
        // How much longer the clip gets (negative: shorter).
        let grow = if at_start {
            start as i64 - to as i64
        } else {
            to as i64 - end as i64
        };
        let mut later: Vec<(ClipId, u64)> = track
            .clips
            .iter()
            .filter(|c| c.start >= end)
            .map(|c| (c.id, c.start))
            .collect();
        if grow > 0 {
            // Make room first, from the last clip back.
            later.reverse();
        }
        let shift = |commands: &mut Vec<Command>| {
            for (clip, s) in &later {
                commands.push(Command::MoveClip {
                    clip: *clip,
                    track: track_id,
                    start: (*s as i64 + grow).max(0) as u64,
                });
            }
        };
        let mut commands = Vec::new();
        if grow > 0 {
            shift(&mut commands);
        }
        if at_start {
            commands.push(Command::TrimStart {
                clip: id,
                start: to,
            });
            commands.push(Command::MoveClip {
                clip: id,
                track: track_id,
                start,
            });
        } else {
            commands.push(Command::TrimEnd { clip: id, end: to });
        }
        if grow < 0 {
            shift(&mut commands);
        }
        if let Err(e) = self.session.dispatch_all(commands) {
            self.fail(e);
        }
    }

    /// Slips clip `id` by `frames` frames of the timeline: the picture inside it moves with the
    /// pointer (to the right shows earlier footage).
    pub(crate) fn slip_by(&mut self, id: ClipId, frames: i64) {
        let Some((_, clip)) = self.project().clip(id) else {
            return;
        };
        let moved = clip.source_frames(frames.unsigned_abs()) as i64 * frames.signum();
        let source_in = (clip.source_in as i64 - moved).max(0) as u64;
        if source_in != clip.source_in {
            self.run(Command::SlipClip {
                clip: id,
                source_in,
            });
        }
    }

    fn ramp_window(&mut self, ctx: &Context) {
        let Some(mut d) = self.extra.ramp.take() else {
            return;
        };
        let mut open = true;
        let mut done = None;
        egui::Window::new("Speed Ramp")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                egui::Grid::new("ramp").num_columns(2).show(ui, |ui| {
                    ui.label("Start speed");
                    ui.add(
                        egui::DragValue::new(&mut d.from)
                            .range(1.0..=1000.0)
                            .suffix(" %"),
                    );
                    ui.end_row();
                    ui.label("End speed");
                    ui.add(
                        egui::DragValue::new(&mut d.to)
                            .range(1.0..=1000.0)
                            .suffix(" %"),
                    );
                    ui.end_row();
                    ui.label("Steps");
                    ui.add(egui::Slider::new(
                        &mut d.steps,
                        2..=picasso_core::MAX_RAMP_STEPS,
                    ));
                    ui.end_row();
                    ui.label("Curve");
                    ui.horizontal(|ui| {
                        for (n, name) in ["Linear", "Ease In", "Ease Out"].iter().enumerate() {
                            ui.radio_value(&mut d.curve, n, *name);
                        }
                    });
                    ui.end_row();
                });
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Ramp").clicked() {
                        done = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        done = Some(false);
                    }
                });
            });
        if done == Some(true) {
            let speeds = d.speeds();
            if self
                .run(Command::SpeedRamp {
                    clip: d.clip,
                    speeds,
                })
                .is_some()
            {
                self.info(format!(
                    "Ramped the clip from {:.0}% to {:.0}% in {} steps",
                    d.from, d.to, d.steps
                ));
            }
        } else if open && done.is_none() {
            self.extra.ramp = Some(d);
        }
    }

    /// The windows of this module.
    pub(crate) fn extra_windows(&mut self, ctx: &Context) {
        self.track_fx_window(ctx);
        self.video_layout_window(ctx);
        self.auto_subtitles_window(ctx);
        self.multicam_windows(ctx);
        self.attributes_window(ctx);
        self.ramp_window(ctx);
        self.rename_bin_window(ctx);
        self.smart_bin_window(ctx);
        self.save_layout_window(ctx);
        self.load_layout_window(ctx);
        self.normalize_window(ctx);
        self.preferences_window(ctx);
        self.media_management_window(ctx);
        self.speed_window(ctx);
        self.duration_window(ctx);
        self.label_window(ctx);
        self.timecode_window(ctx);
        self.edit_index_window(ctx);
        self.manual_window(ctx);
        self.fps_prompt_window(ctx);
        self.exit_prompt_window(ctx);
    }

    fn speed_window(&mut self, ctx: &Context) {
        let Some((clip, mut percent)) = self.extra.speed else {
            return;
        };
        let mut open = true;
        let mut done = None;
        egui::Window::new("Change Clip Speed")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Speed");
                    ui.add(
                        egui::DragValue::new(&mut percent)
                            .range(1.0..=1000.0)
                            .speed(1.0)
                            .suffix(" %"),
                    );
                });
                ui.horizontal(|ui| {
                    for p in [25.0, 50.0, 100.0, 200.0, 400.0] {
                        if ui.small_button(format!("{p:.0}%")).clicked() {
                            percent = p;
                        }
                    }
                });
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Change").clicked() {
                        done = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        done = Some(false);
                    }
                });
            });
        self.extra.speed = Some((clip, percent));
        if done == Some(true) {
            self.run(Command::SetClipSpeed {
                clip,
                speed: percent / 100.0,
            });
        }
        if !open || done.is_some() {
            self.extra.speed = None;
        }
    }

    fn duration_window(&mut self, ctx: &Context) {
        let Some((clip, mut frames)) = self.extra.duration else {
            return;
        };
        let fps = self.project().fps;
        let mut open = true;
        let mut done = None;
        egui::Window::new("Change Clip Duration")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Duration");
                    ui.add(
                        egui::DragValue::new(&mut frames)
                            .range(1..=u64::from(u32::MAX))
                            .speed(1.0)
                            .suffix(" frames"),
                    );
                });
                ui.label(RichText::new(fps.timecode(frames)).monospace());
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Change").clicked() {
                        done = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        done = Some(false);
                    }
                });
            });
        self.extra.duration = Some((clip, frames));
        if done == Some(true) {
            if let Some(start) = self.project().clip(clip).map(|(_, c)| c.start) {
                self.run(Command::TrimEnd {
                    clip,
                    end: start + frames.max(1),
                });
            }
        }
        if !open || done.is_some() {
            self.extra.duration = None;
        }
    }

    fn label_window(&mut self, ctx: &Context) {
        let Some((clip, node, mut text)) = self.extra.label.take() else {
            return;
        };
        let mut open = true;
        let mut done = None;
        egui::Window::new("Label Node")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                let field = ui.text_edit_singleline(&mut text);
                field.request_focus();
                if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    done = Some(true);
                }
                ui.horizontal(|ui| {
                    if ui.button("OK").clicked() {
                        done = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        done = Some(false);
                    }
                });
            });
        if done == Some(true) {
            if let Some(mut grade) = self
                .project()
                .clip(clip)
                .and_then(|(_, c)| c.node(node))
                .cloned()
            {
                grade.label = text.chars().take(64).collect();
                self.run(Command::SetGrade { clip, grade, node });
            }
        }
        if open && done.is_none() {
            self.extra.label = Some((clip, node, text));
        }
    }

    fn timecode_window(&mut self, ctx: &Context) {
        let mut open = self.extra.timecode_window;
        if !open {
            return;
        }
        let tc = self.tc(self.playhead as i64);
        egui::Window::new("Timecode")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(RichText::new(tc).monospace().size(40.0).strong());
            });
        self.extra.timecode_window = open;
    }

    fn edit_index_window(&mut self, ctx: &Context) {
        let mut open = self.extra.edit_index;
        if !open {
            return;
        }
        // Every clip, in timeline order: number, track, name, in and out.
        let project = self.project();
        let mut rows: Vec<(u64, String, String, u64, u64, ClipId)> = project
            .tracks
            .iter()
            .flat_map(|t| {
                t.clips.iter().map(move |c| {
                    let name = project
                        .media(c.media)
                        .map_or_else(String::new, |m| m.name.clone());
                    (c.start, t.name.clone(), name, c.start, c.end(), c.id)
                })
            })
            .collect();
        rows.sort_by_key(|r| (r.0, r.1.clone()));
        let mut go = None;
        egui::Window::new("Edit Index")
            .open(&mut open)
            .default_width(560.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(360.0)
                    .show(ui, |ui| {
                        egui::Grid::new("edit_index")
                            .striped(true)
                            .num_columns(5)
                            .show(ui, |ui| {
                                for h in ["#", "Track", "Clip", "Record In", "Record Out"] {
                                    ui.strong(h);
                                }
                                ui.end_row();
                                for (n, (_, track, name, start, end, id)) in rows.iter().enumerate()
                                {
                                    ui.label(format!("{:03}", n + 1));
                                    ui.label(track);
                                    if ui.link(name).clicked() {
                                        go = Some((*start, *id));
                                    }
                                    ui.monospace(self.tc(*start as i64));
                                    ui.monospace(self.tc(*end as i64));
                                    ui.end_row();
                                }
                            });
                    });
            });
        if let Some((frame, id)) = go {
            self.go_to(frame);
            self.select_only(id);
        }
        self.extra.edit_index = open;
    }

    fn manual_window(&mut self, ctx: &Context) {
        let mut open = self.extra.manual;
        if !open {
            return;
        }
        egui::Window::new("Picasso Suite Manual")
            .open(&mut open)
            .default_width(520.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(420.0)
                    .show(ui, |ui| {
                        for (title, text) in MANUAL {
                            ui.heading(*title);
                            ui.label(*text);
                            ui.add_space(6.0);
                        }
                    });
            });
        self.extra.manual = open;
    }

    /// Switches to a page, for the Show Panel entries.
    pub(crate) fn go_to_page(&mut self, page: Page) {
        self.page = page;
    }
}

/// The manual: one section per page and a few ways of working.
const MANUAL: &[(&str, &str)] = &[
    (
        "Media",
        "Import pictures, video and sound with File > Import > Media (Ctrl+I) or by dropping \
         files on the window. Double-click an item to see it in the viewer.",
    ),
    (
        "Cut and Edit",
        "Drag media to the timeline, or use Insert (F9), Overwrite (F10), Place on Top (F12) and \
         Fit to Fill (Shift+F11). S or Ctrl+B cuts at the playhead; Backspace deletes, Delete \
         deletes and closes the gap. Mark In (I) and Out (O) set the range to play or export.",
    ),
    (
        "Trim",
        "Ctrl+[ and Ctrl+] ripple the start and end of a clip to the playhead; E extends the \
         nearest edit. Comma and full stop slip a clip; Alt with them slides it.",
    ),
    (
        "Color",
        "The wheels grade the clip under the playhead, node by node (Alt+S adds a node). The \
         Qualifier palette keys a colour range (use its picker on the viewer), the Window \
         palette adds shapes that can be dragged on the viewer; Highlight shows what a node \
         grades. Grab Still (Ctrl+Alt+G) keeps a grade in the gallery to apply to other clips.",
    ),
    (
        "Audio",
        "The mixer sets each track's level and pan; the loudness meter measures the mix as it \
         plays (EBU R 128). Cross Fade softens the cut between two sound clips.",
    ),
    (
        "Deliver",
        "Choose a format and press Add to Render Queue, then Render All. Subtitles and chapter \
         markers are written with the file.",
    ),
    (
        "Keys",
        "Help > Keyboard Customization lists every key. Space plays and stops, J, K and L \
         shuttle, the arrows step frames and edits.",
    ),
];
