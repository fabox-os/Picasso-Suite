//! What the menu entries and keys do: every [`Act`], the selection and clipboard they work on,
//! markers and the In/Out range, and the playback keys.
//!
//! Every change to the project goes through a command of `picasso-core`; several commands that
//! belong together run as one undo step (`Session::dispatch_all`).

use std::collections::BTreeSet;

use egui::{Button, Context, Ui, ViewportCommand};
use picasso_core::{
    ClipId, ColorGrade, Command, MediaId, MediaSource, Outcome, TrackId, TrackKind, Transition,
    TransitionKind, MARKER_COLORS,
};

use crate::app::{Page, StudioApp};
use crate::menu::{self, Act, Entry};
use crate::menu_extra::RampDialog;
use crate::platform;

/// A clip on the clipboard: where it was relative to the first copied clip, and what it is.
#[derive(Clone)]
pub(crate) struct ClipCopy {
    track: TrackId,
    offset: u64,
    media: MediaId,
    name: String,
    source_in: u64,
    reverse: bool,
    freeze: bool,
    duration: u64,
    opacity: f32,
    grade: ColorGrade,
    enabled: bool,
    audio: picasso_core::ClipAudio,
    speed: f32,
    transform: picasso_core::Transform,
    blend: picasso_core::BlendMode,
    sound: bool,
    nodes: Vec<ColorGrade>,
    /// The link the copied clip had, so pasted copies of linked clips are linked again.
    link: Option<u64>,
}

/// Playback that stops at a frame, and where to go back to afterwards.
#[derive(Clone, Copy)]
pub(crate) struct PlayRange {
    pub stop_at: u64,
    pub return_to: Option<u64>,
}

/// The guides drawn over the viewer.
#[derive(Clone, Copy, Default)]
pub(crate) struct Overlays {
    pub safe_area: bool,
    pub crosshair: bool,
    pub grid: bool,
    /// A checkerboard behind the picture where it is transparent.
    pub checkerboard: bool,
}

/// The marker dialog: a name and a colour for the marker at `frame`.
pub(crate) struct MarkerDialog {
    pub frame: u64,
    pub name: String,
    pub color: u8,
    pub exists: bool,
}

/// The most the playback keys speed up to.
const MAX_SPEED: f64 = 8.0;

impl StudioApp {
    // ----- selection -----

    /// Every selected clip that still exists, in timeline order. With Linked Selection on, the
    /// clips linked to a selected one are selected too.
    pub(crate) fn selected_ids(&self) -> Vec<ClipId> {
        let project = self.project();
        let mut ids: Vec<(u64, ClipId)> = self
            .selection
            .iter()
            .chain(self.selected_more.iter())
            .flat_map(|id| std::iter::once(*id).chain(project.partners(*id)))
            .filter_map(|id| project.clip(id).map(|(_, c)| (c.start, c.id)))
            .collect();
        ids.sort();
        ids.dedup();
        ids.into_iter().map(|(_, id)| id).collect()
    }

    pub(crate) fn is_selected(&self, id: ClipId) -> bool {
        let chosen = |c: &ClipId| self.selection == Some(*c) || self.selected_more.contains(c);
        chosen(&id) || self.project().partners(id).iter().any(chosen)
    }

    /// One clip of each linked group in `ids`: an edit of one clip of a group (delete, say)
    /// already does the same to the others while Linked Selection is on.
    fn one_per_link(&self, ids: Vec<ClipId>) -> Vec<ClipId> {
        let project = self.project();
        if !project.linked_selection {
            return ids;
        }
        let mut seen = Vec::new();
        ids.into_iter()
            .filter(|id| match project.clip(*id).and_then(|(_, c)| c.link) {
                Some(link) if seen.contains(&link) => false,
                Some(link) => {
                    seen.push(link);
                    true
                }
                None => true,
            })
            .collect()
    }

    /// Whether a video file has sound, so placing it also gives its sound a linked clip.
    pub(crate) fn has_sound(&self, media: MediaId) -> bool {
        let Some(item) = self.project().media(media) else {
            return false;
        };
        let picasso_core::MediaSource::Video { path } = &item.source else {
            return false;
        };
        #[cfg(not(target_arch = "wasm32"))]
        {
            match self.media_facts.get(&media) {
                Some(facts) => facts.audio_codec.is_some(),
                None => picasso_decode::probe(std::path::Path::new(path))
                    .is_ok_and(|info| info.audio_codec.is_some()),
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = path;
            false
        }
    }

    pub(crate) fn select_only(&mut self, id: ClipId) {
        self.selection = Some(id);
        self.selected_more.clear();
    }

    /// Starts a new, empty timeline named "Timeline N" (the first N not taken) and edits it.
    pub(crate) fn new_timeline(&mut self) {
        let taken: Vec<String> = self
            .project()
            .timeline_list()
            .into_iter()
            .map(|(_, name)| name)
            .collect();
        let name = (1..)
            .map(|n| format!("Timeline {n}"))
            .find(|n| !taken.contains(n))
            .unwrap_or_default();
        if self
            .run(Command::NewTimeline { name: name.clone() })
            .is_some()
        {
            self.clear_selection();
            self.go_to(0);
            self.info(format!("Created {name}"));
        }
    }

    /// Puts the selected clips (and, with Linked Selection, the clips linked to them) into a
    /// compound clip named "Compound Clip N".
    fn create_compound(&mut self) {
        let mut ids = self.selected_ids();
        if self.project().linked_selection {
            let linked: Vec<ClipId> = ids
                .iter()
                .flat_map(|id| self.project().linked_to(*id))
                .collect();
            for id in linked {
                if !ids.contains(&id) {
                    ids.push(id);
                }
            }
        }
        let taken: Vec<String> = self
            .project()
            .media
            .iter()
            .map(|m| m.name.clone())
            .collect();
        let name = (1..)
            .map(|n| format!("Compound Clip {n}"))
            .find(|n| !taken.contains(n))
            .unwrap_or_default();
        if let Some(Outcome::Clip(id)) = self.run(Command::CreateCompound {
            clips: ids,
            name: name.clone(),
        }) {
            self.select_only(id);
            self.info(format!("Created {name}"));
        }
    }

    /// Adds an effects clip at the playhead on a free video track above the clips there: a
    /// transparent clip whose grade and effects apply to the composite below.
    pub(crate) fn create_effects_clip(&mut self) {
        let start = self.playhead;
        let taken: Vec<String> = self
            .project()
            .media
            .iter()
            .map(|m| m.name.clone())
            .collect();
        let name = (1..)
            .map(|n| format!("Effects Clip {n}"))
            .find(|n| !taken.contains(n))
            .unwrap_or_default();
        if let Some(Outcome::Clip(id)) = self.run(Command::CreateEffectsClip {
            start,
            name: name.clone(),
        }) {
            self.select_only(id);
            self.info(format!("Created {name}"));
        }
    }

    /// The clip after the cut nearest the playhead, on the selected clip's track when it is of
    /// `kind`, else on any unlocked track of `kind`: a cut between two clips that touch.
    /// With `with_transition`, only cuts that have a transition count.
    pub(crate) fn nearest_cut(&self, kind: TrackKind, with_transition: bool) -> Option<ClipId> {
        let project = self.project();
        let selected = self
            .selection
            .and_then(|id| project.clip(id))
            .map(|(t, _)| t.id)
            .filter(|id| project.track(*id).is_some_and(|t| t.kind == kind));
        let at = self.playhead;
        project
            .tracks
            .iter()
            .filter(|t| t.kind == kind && !t.locked)
            .filter(|t| selected.is_none_or(|id| t.id == id))
            .flat_map(|t| t.clips.windows(2))
            .filter(|w| w[0].end() == w[1].start)
            .filter(|w| !with_transition || w[1].transition.is_some())
            .min_by_key(|w| w[1].start.abs_diff(at))
            .map(|w| w[1].id)
    }

    /// Puts a one-second transition of `kind` on the cut nearest the playhead.
    fn add_transition(&mut self, track: TrackKind, kind: TransitionKind) {
        let Some(clip) = self.nearest_cut(track, false) else {
            self.fail("there is no cut between two clips that touch");
            return;
        };
        let frames = self.project().fps.nominal().max(2);
        self.run(Command::SetTransition {
            clip,
            transition: Some(Transition { kind, frames }),
        });
    }

    /// Adds a title ("Basic Title", white, in the middle) at the playhead, on a free track above
    /// the clips there, five seconds long, as one undo step, and selects it.
    pub(crate) fn add_title(&mut self) {
        self.add_title_of("Text", picasso_core::TitleText::default());
    }

    /// Effects > Titles > Text+: a title with an outline and a soft drop shadow.
    pub(crate) fn add_text_plus(&mut self) {
        let title = picasso_core::TitleText {
            text: "Custom Title".into(),
            bold: true,
            style: picasso_core::TextStyle {
                outline: 4,
                shadow: 8,
                shadow_blur: 6,
                shadow_opacity: 70,
                ..Default::default()
            },
            ..Default::default()
        };
        self.add_title_of("Text+", title);
    }

    fn add_title_of(&mut self, name: &str, title: picasso_core::TitleText) {
        let start = self.playhead;
        let length = self.project().fps.nominal().max(1) * 5;
        let made = self.session.dispatch_all_then(
            vec![Command::AddMedia {
                name: name.into(),
                source: MediaSource::Title(title),
                length: None,
                default_duration: length,
            }],
            |outcomes| match outcomes.first() {
                Some(Outcome::Media(media)) => vec![Command::PlaceOnTop {
                    media: *media,
                    start,
                    with_sound: false,
                }],
                _ => Vec::new(),
            },
        );
        match made {
            Ok(outcomes) => {
                // The clip made from the new media.
                let clip = match outcomes.first() {
                    Some(Outcome::Media(m)) => self
                        .project()
                        .tracks
                        .iter()
                        .flat_map(|t| t.clips.iter())
                        .find(|c| c.media == *m)
                        .map(|c| c.id),
                    _ => None,
                };
                if let Some(id) = clip {
                    self.select_only(id);
                }
                self.info("Added a title: edit its words in the Inspector");
            }
            Err(e) => self.fail(e),
        }
    }

    /// Edits timeline `id` instead of the current one.
    pub(crate) fn switch_timeline(&mut self, id: u64) {
        if id != self.project().timeline_id && self.run(Command::SwitchTimeline { id }).is_some() {
            self.clear_selection();
            let end = self.project().duration();
            if self.playhead > end {
                self.go_to(end);
            }
        }
    }

    pub(crate) fn clear_selection(&mut self) {
        self.selection = None;
        self.selected_more.clear();
    }

    fn select_many(&mut self, ids: Vec<ClipId>) {
        let mut ids = ids.into_iter();
        self.selection = ids.next();
        self.selected_more = ids.collect();
    }

    /// The clip a trim or enable acts on: the selected one, else the one under the playhead.
    pub(crate) fn target_clip(&self) -> Option<ClipId> {
        self.selection
            .filter(|id| self.project().clip(*id).is_some())
            .or_else(|| self.current_clip())
    }

    // ----- clipboard -----

    pub(crate) fn copy_selection(&mut self) -> bool {
        let ids = self.selected_ids();
        let project = self.project();
        let clips: Vec<(TrackId, picasso_core::Clip)> = ids
            .iter()
            .filter_map(|id| project.clip(*id).map(|(t, c)| (t.id, c.clone())))
            .collect();
        let Some(base) = clips.iter().map(|(_, c)| c.start).min() else {
            self.fail("select a clip first");
            return false;
        };
        self.clipboard = clips
            .into_iter()
            .map(|(track, c)| ClipCopy {
                track,
                offset: c.start - base,
                media: c.media,
                name: c.name,
                source_in: c.source_in,
                duration: c.duration,
                opacity: c.opacity,
                grade: c.grade,
                enabled: c.enabled,
                audio: c.audio,
                speed: c.speed,
                transform: c.transform,
                blend: c.blend,
                sound: c.sound,
                nodes: c.nodes,
                link: c.link,
                reverse: c.reverse,
                freeze: c.freeze,
            })
            .collect();
        self.info(format!("Copied {} clip(s)", self.clipboard.len()));
        true
    }

    /// Puts the clipboard on the timeline with its first clip at `at`, as one undo step. With
    /// `insert`, the tracks it goes on make room first: what is at `at` and after moves later.
    fn paste_clips(&mut self, at: u64, insert: bool) {
        if self.clipboard.is_empty() {
            self.fail("there is nothing to paste");
            return;
        }
        let project = self.project();
        // Where a clip goes when its own track is gone: the first unlocked track of its kind.
        let fallback_for = |media: MediaId| {
            let kind = project.media(media)?.source.track_kind();
            project
                .tracks
                .iter()
                .find(|t| t.kind == kind && !t.locked)
                .map(|t| t.id)
        };
        // The copies' links, in the order of the commands, to link the pasted clips again.
        let mut links: Vec<Option<u64>> = Vec::new();
        let commands: Vec<Command> = self
            .clipboard
            .iter()
            .filter_map(|c| {
                let track = if project.track(c.track).is_some() {
                    c.track
                } else {
                    fallback_for(c.media)?
                };
                links.push(c.link);
                Some(Command::PasteClip {
                    track,
                    media: c.media,
                    name: c.name.clone(),
                    start: at.checked_add(c.offset)?,
                    source_in: c.source_in,
                    duration: c.duration,
                    opacity: c.opacity,
                    grade: c.grade.clone(),
                    enabled: c.enabled,
                    audio: c.audio,
                    speed: c.speed,
                    transform: c.transform,
                    blend: c.blend,
                    sound: c.sound,
                    nodes: c.nodes.clone(),
                    reverse: c.reverse,
                    freeze: c.freeze,
                })
            })
            .collect();
        let mut gaps: Vec<Command> = Vec::new();
        if insert {
            let frames = self
                .clipboard
                .iter()
                .map(|c| c.offset + c.duration)
                .max()
                .unwrap_or(0);
            for command in &commands {
                if let Command::PasteClip { track, .. } = command {
                    let gap = Command::InsertGap {
                        track: *track,
                        at,
                        frames,
                    };
                    if !gaps.contains(&gap) {
                        gaps.push(gap);
                    }
                }
            }
        }
        let skip = gaps.len();
        let commands: Vec<Command> = gaps.into_iter().chain(commands).collect();
        let relink = move |outcomes: &[Outcome]| {
            let mut groups: Vec<(u64, Vec<ClipId>)> = Vec::new();
            for (link, outcome) in links.iter().zip(outcomes.iter().skip(skip)) {
                if let (Some(link), Outcome::Clip(id)) = (link, outcome) {
                    match groups.iter_mut().find(|(l, _)| l == link) {
                        Some((_, ids)) => ids.push(*id),
                        None => groups.push((*link, vec![*id])),
                    }
                }
            }
            groups
                .into_iter()
                .filter(|(_, ids)| ids.len() > 1)
                .map(|(_, clips)| Command::LinkClips { clips })
                .collect()
        };
        match self.session.dispatch_all_then(commands, relink) {
            Ok(outcomes) => {
                let ids = outcomes
                    .into_iter()
                    .filter_map(|o| {
                        if let Outcome::Clip(id) = o {
                            Some(id)
                        } else {
                            None
                        }
                    })
                    .collect();
                self.select_many(ids);
            }
            Err(e) => self.fail(e),
        }
    }

    /// Paste Mix to Current Track: the sound of the copied clips, on the current sound track (the
    /// one the selected clip is on, or the first) with the first clip at `at`. No picture is
    /// pasted: a picture on the clipboard brings only its sound.
    fn paste_mix(&mut self, at: u64) {
        if self.clipboard.is_empty() {
            self.fail("there is nothing to paste");
            return;
        }
        let planned = {
            let project = self.project();
            let track = self
                .selection
                .and_then(|id| project.clip(id))
                .map(|(track, _)| track)
                .filter(|t| t.kind == TrackKind::Audio && !t.locked)
                .or_else(|| {
                    project
                        .tracks
                        .iter()
                        .find(|t| t.kind == TrackKind::Audio && !t.locked)
                });
            track.map(|track| {
                let commands: Vec<Command> = self
                    .clipboard
                    .iter()
                    .filter(|c| {
                        // A clip whose sound is off has none to paste, and a picture whose sound
                        // has a clip of its own on an audio track is silent: that clip is on the
                        // clipboard too, and is the one to paste.
                        c.sound
                            && matches!(
                                project.media(c.media).map(|m| &m.source),
                                Some(MediaSource::Video { .. } | MediaSource::Audio { .. })
                            )
                    })
                    .filter_map(|c| {
                        Some(Command::PasteClip {
                            track: track.id,
                            media: c.media,
                            name: c.name.clone(),
                            start: at.checked_add(c.offset)?,
                            source_in: c.source_in,
                            duration: c.duration,
                            // A sound clip shows no picture: it keeps only how it sounds.
                            opacity: 1.0,
                            grade: ColorGrade::default(),
                            enabled: c.enabled,
                            audio: c.audio,
                            speed: c.speed,
                            transform: picasso_core::Transform::default(),
                            blend: picasso_core::BlendMode::Normal,
                            sound: true,
                            nodes: Vec::new(),
                            reverse: c.reverse,
                            freeze: c.freeze,
                        })
                    })
                    .collect();
                (commands, track.name.clone())
            })
        };
        let Some((commands, name)) = planned else {
            // No sound track at all, or the ones there are are locked: the two are worth telling
            // apart, since only the second can be undone by unlocking them.
            let any = self
                .project()
                .tracks
                .iter()
                .any(|t| t.kind == TrackKind::Audio);
            self.fail(if any {
                "the sound tracks are locked"
            } else {
                "there is no sound track to paste onto"
            });
            return;
        };
        if commands.is_empty() {
            self.fail("the copied clips have no sound");
            return;
        }
        let count = commands.len();
        match self.session.dispatch_all(commands) {
            Ok(outcomes) => {
                let ids = outcomes
                    .into_iter()
                    .filter_map(|outcome| match outcome {
                        Outcome::Clip(id) => Some(id),
                        _ => None,
                    })
                    .collect();
                self.select_many(ids);
                self.info(format!("Pasted the sound of {count} clip(s) on {name}"));
            }
            Err(e) => self.fail(e),
        }
    }

    pub(crate) fn delete_selected(&mut self, ripple: bool) {
        let mut ids = self.one_per_link(self.selected_ids());
        if ids.is_empty() {
            self.fail("select a clip first");
            return;
        }
        // Where the first deleted clip starts: the playhead follows the cut, as the clips behind
        // it close up.
        let cut = ids
            .first()
            .and_then(|id| self.project().clip(*id))
            .map(|(_, c)| c.start);
        // Latest first, so closing a gap never moves a clip that is still to be deleted.
        ids.reverse();
        let commands = ids
            .into_iter()
            .map(|clip| {
                if ripple {
                    Command::RippleDelete { clip }
                } else {
                    Command::Delete { clip }
                }
            })
            .collect();
        match self.session.dispatch_all(commands) {
            Ok(_) => {
                self.clear_selection();
                if let Some(at) = cut.filter(|_| ripple) {
                    self.go_to(at);
                }
            }
            Err(e) => self.fail(e),
        }
    }

    /// Audio > Auto Align Clips > Based on Timecode: lines the selected clips up by the start
    /// timecode their files carry.
    fn align_by_timecode(&mut self) {
        let clips = self.selected_ids();
        if self.run(Command::AlignClips { clips }).is_some() {
            self.info("Aligned the clips by their timecode");
        }
    }

    // ----- trimming -----

    fn trim_to_playhead(&mut self, start: bool) {
        let Some(clip) = self.target_clip() else {
            self.fail("select a clip first");
            return;
        };
        let at = self.playhead;
        self.run(if start {
            Command::TrimStart { clip, start: at }
        } else {
            Command::TrimEnd { clip, end: at }
        });
    }

    /// Trims the clip to the playhead and closes the gap on its track.
    fn ripple_trim(&mut self, at_start: bool) {
        let Some(id) = self.target_clip() else {
            self.fail("select a clip first");
            return;
        };
        let at = self.playhead;
        let Some((track, clip)) = self.project().clip(id) else {
            return;
        };
        if !(clip.start < at && at < clip.end()) {
            self.fail("put the playhead inside the clip");
            return;
        }
        let (start, end, track_id) = (clip.start, clip.end(), track.id);
        let removed = if at_start { at - start } else { end - at };
        let later: Vec<(ClipId, u64)> = track
            .clips
            .iter()
            .filter(|c| c.start >= end)
            .map(|c| (c.id, c.start))
            .collect();
        let mut commands = if at_start {
            vec![
                Command::TrimStart {
                    clip: id,
                    start: at,
                },
                Command::MoveClip {
                    clip: id,
                    track: track_id,
                    start,
                },
            ]
        } else {
            vec![Command::TrimEnd { clip: id, end: at }]
        };
        commands.extend(later.into_iter().map(|(clip, s)| Command::MoveClip {
            clip,
            track: track_id,
            start: s - removed,
        }));
        if let Err(e) = self.session.dispatch_all(commands) {
            self.fail(e);
        }
    }

    /// Moves the nearest end of the clip to the playhead.
    fn extend_edit(&mut self) {
        let Some(clip) = self.target_clip() else {
            self.fail("select a clip first");
            return;
        };
        let at = self.playhead;
        let Some((_, c)) = self.project().clip(clip) else {
            return;
        };
        if at > c.end() {
            self.run(Command::TrimEnd { clip, end: at });
        } else if at < c.start {
            self.run(Command::TrimStart { clip, start: at });
        } else {
            self.fail("put the playhead before or after the clip");
        }
    }

    /// Shows the footage one frame earlier or later, leaving the clip where it is.
    fn slip(&mut self, later: bool) {
        let Some(clip) = self.target_clip() else {
            self.fail("select a clip first");
            return;
        };
        let Some((_, c)) = self.project().clip(clip) else {
            return;
        };
        let step = c.source_frames(1).max(1);
        let source_in = if later {
            c.source_in.checked_add(step)
        } else {
            c.source_in.checked_sub(step)
        };
        match source_in {
            Some(source_in) => {
                self.run(Command::SlipClip { clip, source_in });
            }
            None => self.fail("the clip already shows the start of its media"),
        }
    }

    /// Moves the clip one frame between its neighbours, which make room or fill the space.
    fn slide(&mut self, right: bool) {
        let Some(clip) = self.target_clip() else {
            self.fail("select a clip first");
            return;
        };
        let Some((_, c)) = self.project().clip(clip) else {
            return;
        };
        let start = if right {
            c.start.checked_add(1)
        } else {
            c.start.checked_sub(1)
        };
        match start {
            Some(start) => {
                self.run(Command::SlideClip { clip, start });
            }
            None => self.fail("the clip is already at the start of the timeline"),
        }
    }

    /// Holds the clip on the frame under the playhead (its first frame when the playhead is
    /// elsewhere), or plays it again when it is held.
    fn freeze_frame(&mut self) {
        let Some(clip) = self.target_clip() else {
            self.fail("select a clip first");
            return;
        };
        let Some((_, c)) = self.project().clip(clip) else {
            return;
        };
        let frame = if c.freeze {
            None
        } else if (c.start..c.end()).contains(&self.playhead) {
            Some(c.source_frame_at(self.playhead))
        } else {
            Some(c.source_frame_at(c.start))
        };
        self.run(Command::SetFreeze { clip, frame });
    }

    // ----- In and Out, markers -----

    fn set_marks(&mut self, mark_in: Option<u64>, mark_out: Option<u64>) {
        self.run(Command::SetMarks { mark_in, mark_out });
    }

    fn marker_at(&self, frame: u64) -> Option<&picasso_core::Marker> {
        self.project().markers.iter().find(|m| m.frame == frame)
    }

    fn open_marker_dialog(&mut self, require_existing: bool) {
        let frame = self.playhead;
        let existing = self.marker_at(frame).cloned();
        if require_existing && existing.is_none() {
            self.fail("there is no marker at the playhead");
            return;
        }
        self.marker_dialog = Some(match existing {
            Some(m) => MarkerDialog {
                frame,
                name: m.name,
                color: m.color,
                exists: true,
            },
            None => MarkerDialog {
                frame,
                name: String::new(),
                color: 0,
                exists: false,
            },
        });
    }

    fn go_to_marker(&mut self, forward: bool) {
        let at = self.playhead;
        let markers = &self.project().markers;
        let target = if forward {
            markers.iter().find(|m| m.frame > at)
        } else {
            markers.iter().rev().find(|m| m.frame < at)
        }
        .map(|m| m.frame);
        match target {
            Some(frame) => self.go_to(frame),
            None => self.fail(if forward {
                "there is no marker after the playhead"
            } else {
                "there is no marker before the playhead"
            }),
        }
    }

    // ----- playback -----

    fn play(&mut self, speed: f64) {
        let end = self.project().duration();
        if end == 0 {
            return;
        }
        if speed > 0.0 && self.playhead >= end {
            self.playhead = 0;
        }
        if speed < 0.0 && self.playhead == 0 {
            self.playhead = end;
        }
        self.play_range = None;
        self.play_accum = 0.0;
        self.play_speed = speed;
        self.playing = true;
    }

    /// J and L: start at normal speed, then double the speed on each press in the same direction.
    fn shuttle(&mut self, forward: bool) {
        let sign = if forward { 1.0 } else { -1.0 };
        if self.playing && self.play_speed * sign > 0.0 {
            self.play_speed = (self.play_speed.abs() * 2.0).min(MAX_SPEED) * sign;
        } else {
            self.play(sign);
        }
    }

    pub(crate) fn play_around(&mut self) {
        let end = self.project().duration();
        if end == 0 {
            return;
        }
        let around = self.project().fps.nominal() * 2;
        let at = self.playhead;
        self.play(1.0);
        self.playhead = at.saturating_sub(around);
        self.play_range = Some(PlayRange {
            stop_at: at.saturating_add(around),
            return_to: Some(at),
        });
    }

    fn play_in_to_out(&mut self) {
        let (Some(a), Some(b)) = (self.project().mark_in, self.project().mark_out) else {
            self.fail("set an In and an Out first");
            return;
        };
        self.play(1.0);
        self.playhead = a;
        self.play_range = Some(PlayRange {
            stop_at: b,
            return_to: None,
        });
    }

    pub(crate) fn go_to_edit(&mut self, forward: bool) {
        let at = self.playhead;
        let edges = self
            .project()
            .tracks
            .iter()
            .flat_map(|t| t.clips.iter())
            .flat_map(|c| [c.start, c.end()]);
        let target = if forward {
            edges.filter(|e| *e > at).min()
        } else {
            edges.filter(|e| *e < at).max()
        };
        match target {
            Some(frame) => self.go_to(frame),
            None => self.go_to(if forward { u64::MAX } else { 0 }),
        }
    }

    // ----- state of entries -----

    fn has_selection(&self) -> bool {
        !self.selected_ids().is_empty()
    }

    pub(crate) fn act_enabled(&self, act: Act) -> bool {
        let project = self.project();
        match act {
            Act::Undo => self.session.undo_label().is_some(),
            Act::Redo => self.session.redo_label().is_some(),
            Act::Cut
            | Act::Copy
            | Act::Duplicate
            | Act::DeleteSelected
            | Act::RippleDeleteSelected => self.has_selection(),
            Act::Paste | Act::PasteInsert | Act::PasteMix => !self.clipboard.is_empty(),
            Act::SelectAll | Act::SelectForward => {
                project.tracks.iter().any(|t| !t.clips.is_empty())
            }
            Act::DeselectAll => self.has_selection(),
            Act::NewTimeline => true,
            Act::AddTitle | Act::AddTextPlus => true,
            Act::GrabStill => self.current_clip().is_some(),
            Act::PlayStill => !project.stills.is_empty(),
            Act::AddCrossDissolve | Act::AddDipToBlack | Act::AddTransition(_) => {
                self.nearest_cut(TrackKind::Video, false).is_some()
            }
            Act::AddCrossFade => self.nearest_cut(TrackKind::Audio, false).is_some(),
            Act::RemoveTransition => {
                self.nearest_cut(TrackKind::Video, true).is_some()
                    || self.nearest_cut(TrackKind::Audio, true).is_some()
            }
            Act::AddTransitionToAll => project
                .tracks
                .iter()
                .filter(|t| !t.locked)
                .any(|t| t.clips.windows(2).any(|w| w[0].end() == w[1].start)),
            Act::CreateCompound | Act::BounceSelectedTracks => self.has_selection(),
            Act::CreateEffectsClip => true,
            Act::DecomposeCompound => self.target_clip().is_some_and(|id| {
                self.project()
                    .clip(id)
                    .and_then(|(_, c)| self.project().media(c.media))
                    .is_some_and(|m| matches!(m.source, MediaSource::Compound { .. }))
            }),
            Act::SwitchTimeline | Act::DeleteTimeline => !project.timelines.is_empty(),
            Act::FitToFill | Act::PlaceOnTop => self.selected_media.is_some(),
            Act::LinkClips => self.selected_ids().len() >= 2,
            Act::UnlinkClips => self.selected_ids().iter().any(|id| {
                self.project()
                    .clip(*id)
                    .is_some_and(|(_, c)| c.link.is_some())
            }),
            Act::RippleStart
            | Act::RippleEnd
            | Act::TrimStartToPlayhead
            | Act::TrimEndToPlayhead
            | Act::ExtendEdit
            | Act::SlipLeft
            | Act::SlipRight
            | Act::SlideLeft
            | Act::SlideRight
            | Act::ToggleEnableClip
            | Act::FreezeFrame
            | Act::ReverseClip
            | Act::MarkClip
            | Act::ResetCurrentGrade
            | Act::ApplyLut => self.target_clip().is_some(),
            Act::Revert => self.saved_text.is_some(),
            Act::Quit | Act::FolderAsBin => cfg!(not(target_arch = "wasm32")),
            Act::ClearIn => project.mark_in.is_some(),
            Act::ClearOut => project.mark_out.is_some(),
            Act::ClearInOut => project.mark_in.is_some() || project.mark_out.is_some(),
            Act::ModifyMarker | Act::DeleteMarkerHere => self.marker_at(self.playhead).is_some(),
            Act::DeleteAllMarkers => !project.markers.is_empty(),
            Act::PreviousMarker => project.markers.iter().any(|m| m.frame < self.playhead),
            Act::NextMarker => project.markers.iter().any(|m| m.frame > self.playhead),
            Act::PlayInToOut => project.mark_in.is_some() && project.mark_out.is_some(),
            Act::ResetAllGrades => project
                .tracks
                .iter()
                .flat_map(|t| t.clips.iter())
                .any(|c| c.is_graded() || !c.nodes.is_empty()),
            Act::AddSerialNode
            | Act::ToggleNode
            | Act::AddNodeBefore
            | Act::AddParallelNode
            | Act::AddLayerNode
            | Act::AddOutsideNode
            | Act::LabelNode
            | Act::ToggleHighlight => self.current_clip().is_some(),
            Act::ChangeClipSpeed
            | Act::AddEffect(_)
            | Act::SpeedRamp
            | Act::ChangeClipDuration
            | Act::LockClip
            | Act::RenderInPlace
            | Act::ClipAttributes
            | Act::NewMulticam
            | Act::MulticamViewer
            | Act::SetClipColor(_)
            | Act::NormalizeAudio => self.target_clip().is_some(),
            Act::AutoAlignWaveform => self.can_align_by_waveform(),
            Act::AutoAlignTimecode => {
                let ids = self.selected_ids();
                ids.len() >= 2
                    && ids.iter().all(|id| {
                        project
                            .clip(*id)
                            .and_then(|(_, c)| project.media(c.media))
                            .is_some_and(|m| m.timecode.is_some())
                    })
            }
            Act::MarkTimeline
            | Act::SelectNearestEdit
            | Act::ExportEdl
            | Act::ExportOtio
            | Act::ExportXml(_)
            | Act::ExportAaf
            | Act::StillToPool => project.duration() > 0,
            Act::PlayAroundMark => project.mark_in.is_some() || project.mark_out.is_some(),
            Act::TimelineFromSelection => !self.selected_ids().is_empty(),
            _ => true,
        }
    }

    pub(crate) fn act_checked(&self, act: Act) -> Option<bool> {
        match act {
            Act::ToggleSnap => Some(self.timeline.snap()),
            Act::ToggleLoop => Some(self.looping),
            Act::ToggleHighlight => Some(self.color.highlight),
            Act::LockClip => self
                .target_clip()
                .and_then(|id| self.project().clip(id))
                .map(|(_, c)| c.locked),
            Act::SetClipColor(color) => self
                .target_clip()
                .and_then(|id| self.project().clip(id))
                .map(|(_, c)| c.color == color),
            Act::PositionLock => Some(self.extra.position_lock),
            Act::TrimMode => Some(self.extra.trim_mode),
            Act::DynamicTrim => Some(self.extra.dynamic_trim),
            Act::LoopTrim => Some(self.extra.loop_trim),
            Act::AsymmetricTrim => Some(self.extra.asymmetric),
            Act::SelectionMode => Some(!self.extra.trim_mode),
            Act::SelectionFollowsPlayhead => Some(self.extra.selection_follows),
            Act::TimecodeWindow => Some(self.extra.timecode_window),
            Act::ShowInspector => Some(self.inspector_open),
            Act::ShowEditIndex => Some(self.extra.edit_index),
            Act::ToggleLinkedSelection => Some(self.project().linked_selection),
            Act::ProxyOff => Some(self.proxy <= 1),
            Act::ProxyHalf => Some(self.proxy == 2),
            Act::ProxyQuarter => Some(self.proxy == 4),
            Act::ProxyEighth => Some(self.proxy == 8),
            Act::ToggleOptimisedMedia => Some(self.use_optimised),
            Act::ToggleHardwareDecoding => Some(self.extra.prefs.hardware_decoding),
            Act::ToggleAudioScrubbing => Some(self.extra.prefs.audio_scrubbing),
            Act::Language(language) => Some(self.extra.prefs.language == language),
            Act::OverlaySafeArea => Some(self.overlays.safe_area),
            Act::OverlayCrosshair => Some(self.overlays.crosshair),
            Act::OverlayGrid => Some(self.overlays.grid),
            Act::FreezeFrame | Act::ReverseClip => self
                .target_clip()
                .and_then(|id| self.project().clip(id))
                .map(|(_, c)| {
                    if act == Act::FreezeFrame {
                        c.freeze
                    } else {
                        c.reverse
                    }
                }),
            Act::OverlayCheckerboard => Some(self.overlays.checkerboard),
            Act::FullScreenViewer => Some(self.viewer_full),
            Act::ToggleScopes => Some(self.scopes_visible),
            Act::ToggleEnableClip => Some(
                self.target_clip()
                    .and_then(|id| self.project().clip(id))
                    .is_some_and(|(_, c)| c.enabled),
            ),
            _ => None,
        }
    }

    // ----- doing -----

    pub(crate) fn do_act(&mut self, ctx: &Context, act: Act) {
        let at = self.playhead;
        match act {
            Act::ProjectManager => self.show_project_manager(),
            Act::NewProject => self.new_project(),
            Act::OpenProject => self.open_dialog(ctx),
            Act::SaveProject => self.save(ctx, false),
            Act::SaveProjectAs => self.save(ctx, true),
            Act::SaveCopy => self.save_copy(ctx),
            Act::Revert => self.revert(),
            Act::ImportMedia => self.import_dialog(ctx),
            Act::ProjectSettings => self.show_settings = true,
            Act::MediaManagement => self.show_media_management(),
            Act::QuickExport => {
                self.page = Page::Deliver;
                self.export_frame(ctx);
            }
            Act::Quit => ctx.send_viewport_cmd(ViewportCommand::Close),

            Act::Undo => self.undo(),
            Act::Redo => self.redo(),
            Act::Copy => {
                self.copy_selection();
            }
            Act::Cut => {
                if self.copy_selection() {
                    self.delete_selected(false);
                }
            }
            Act::Paste => self.paste_clips(at, false),
            Act::PasteInsert => self.paste_clips(at, true),
            Act::PasteMix => self.paste_mix(at),
            Act::FitToFill => self.fit_to_fill_edit(),
            Act::PlaceOnTop => self.place_on_top_edit(),
            Act::Duplicate => {
                let end = self
                    .selected_ids()
                    .into_iter()
                    .filter_map(|id| self.project().clip(id).map(|(_, c)| c.end()))
                    .max();
                if self.copy_selection() {
                    if let Some(end) = end {
                        self.paste_clips(end, false);
                    }
                }
            }
            // The Cut page always closes the gap, as the original does.
            Act::DeleteSelected => self.delete_selected(self.page == Page::Cut),
            Act::RippleDeleteSelected => self.delete_selected(true),
            Act::SelectAll => {
                let ids: Vec<ClipId> = self
                    .project()
                    .tracks
                    .iter()
                    .flat_map(|t| t.clips.iter())
                    .map(|c| c.id)
                    .collect();
                self.select_many(ids);
            }
            Act::DeselectAll => self.clear_selection(),
            Act::NewTimeline => self.new_timeline(),
            Act::AddTitle => self.add_title(),
            Act::AddTextPlus => self.add_text_plus(),
            Act::TrackEq => self.open_track_fx(crate::track_fx::FxTab::Eq),
            Act::TrackDynamics => self.open_track_fx(crate::track_fx::FxTab::Dynamics),
            Act::AddEffect(kind) => match self.target_clip() {
                Some(clip) => {
                    if self.run(Command::AddEffect { clip, kind }).is_some() {
                        self.inspector_open = true;
                        self.info(format!(
                            "Added {}: change it in the Inspector",
                            kind.label()
                        ));
                    }
                }
                None => self.fail("select a clip first"),
            },
            Act::GrabStill => self.grab_still(),
            Act::PlayStill => {
                self.page = crate::app::Page::Color;
                self.toggle_play_still();
            }
            Act::AddCrossDissolve => {
                self.add_transition(TrackKind::Video, TransitionKind::CrossDissolve)
            }
            Act::AddDipToBlack => self.add_transition(TrackKind::Video, TransitionKind::DipToBlack),
            Act::AddTransition(kind) => self.add_transition(TrackKind::Video, kind),
            Act::AddCrossFade => {
                self.add_transition(TrackKind::Audio, TransitionKind::CrossDissolve)
            }
            Act::RemoveTransition => {
                let clip = self
                    .nearest_cut(TrackKind::Video, true)
                    .or_else(|| self.nearest_cut(TrackKind::Audio, true));
                if let Some(clip) = clip {
                    self.run(Command::SetTransition {
                        clip,
                        transition: None,
                    });
                }
            }
            Act::AddTransitionToAll => {
                let frames = self.project().fps.nominal().max(2);
                let commands: Vec<Command> = self
                    .project()
                    .tracks
                    .iter()
                    .filter(|t| !t.locked)
                    .flat_map(|t| t.clips.windows(2))
                    .filter(|w| w[0].end() == w[1].start && w[1].transition.is_none())
                    .map(|w| Command::SetTransition {
                        clip: w[1].id,
                        transition: Some(Transition {
                            kind: TransitionKind::CrossDissolve,
                            frames,
                        }),
                    })
                    .collect();
                let n = commands.len();
                match self.session.dispatch_all(commands) {
                    Ok(_) => self.info(format!("Added {n} cross dissolve(s)")),
                    Err(e) => self.fail(e),
                }
            }
            Act::CreateCompound => self.create_compound(),
            Act::CreateEffectsClip => self.create_effects_clip(),
            Act::DecomposeCompound => {
                if let Some(clip) = self.target_clip() {
                    if self.run(Command::DecomposeCompound { clip }).is_some() {
                        self.clear_selection();
                    }
                }
            }
            Act::SwitchTimeline => {
                // The next timeline in the order they were made, round to the first.
                let list = self.project().timeline_list();
                let current = self.project().timeline_id;
                let next = list
                    .iter()
                    .position(|(id, _)| *id == current)
                    .and_then(|i| list.get((i + 1) % list.len()))
                    .map(|(id, _)| *id);
                if let Some(id) = next {
                    self.switch_timeline(id);
                }
            }
            Act::DeleteTimeline => {
                let (id, name) = (
                    self.project().timeline_id,
                    self.project().timeline_name.clone(),
                );
                if self.run(Command::DeleteTimeline { id }).is_some() {
                    self.clear_selection();
                    self.info(format!("Deleted {name}"));
                }
            }
            Act::SelectForward => {
                let ids: Vec<ClipId> = self
                    .project()
                    .tracks
                    .iter()
                    .flat_map(|t| t.clips.iter())
                    .filter(|c| c.end() > at)
                    .map(|c| c.id)
                    .collect();
                self.select_many(ids);
            }
            Act::ShowShortcuts => self.show_shortcuts = true,

            Act::RippleStart => self.ripple_trim(true),
            Act::RippleEnd => self.ripple_trim(false),
            Act::TrimStartToPlayhead => self.trim_to_playhead(true),
            Act::TrimEndToPlayhead => self.trim_to_playhead(false),
            Act::ExtendEdit => self.extend_edit(),
            Act::FreezeFrame => self.freeze_frame(),
            Act::ReverseClip => {
                if let Some(clip) = self.target_clip() {
                    let reverse = self.project().clip(clip).is_some_and(|(_, c)| !c.reverse);
                    self.run(Command::SetReverse { clip, reverse });
                }
            }
            Act::SlipLeft => self.slip(false),
            Act::SlipRight => self.slip(true),
            Act::SlideLeft => self.slide(false),
            Act::SlideRight => self.slide(true),

            Act::SplitClip => self.split_at_playhead(),
            Act::ToggleSnap => {
                let on = !self.timeline.snap();
                for view in [
                    &mut self.timeline,
                    &mut self.cut_detail,
                    &mut self.cut_overview,
                ] {
                    view.set_snap(on);
                }
            }
            Act::ToggleEnableClip => {
                if let Some(clip) = self.target_clip() {
                    if let Some((_, c)) = self.project().clip(clip) {
                        let enabled = !c.enabled;
                        self.run(Command::SetClipEnabled { clip, enabled });
                    }
                }
            }

            Act::MarkIn => {
                let out = self.project().mark_out.filter(|o| *o > at);
                self.set_marks(Some(at), out);
            }
            Act::MarkOut => {
                let out = at + 1;
                let inn = self.project().mark_in.filter(|i| *i < out);
                self.set_marks(inn, Some(out));
            }
            Act::MarkClip => {
                if let Some(clip) = self.target_clip() {
                    if let Some((_, c)) = self.project().clip(clip) {
                        let (a, b) = (c.start, c.end());
                        self.set_marks(Some(a), Some(b));
                    }
                }
            }
            Act::ClearIn => self.set_marks(None, self.project().mark_out),
            Act::ClearOut => self.set_marks(self.project().mark_in, None),
            Act::ClearInOut => self.set_marks(None, None),
            Act::AddMarker => {
                if self.marker_at(at).is_some() {
                    self.fail("there is already a marker here (Mark > Modify Marker changes it)");
                } else {
                    self.run(Command::SetMarker {
                        frame: at,
                        name: String::new(),
                        color: 0,
                    });
                }
            }
            Act::AddMarkerDialog => self.open_marker_dialog(false),
            Act::ModifyMarker => self.open_marker_dialog(true),
            Act::DeleteMarkerHere => {
                self.run(Command::DeleteMarker { frame: at });
            }
            Act::DeleteAllMarkers => {
                self.run(Command::ClearMarkers);
            }
            Act::PreviousMarker => self.go_to_marker(false),
            Act::NextMarker => self.go_to_marker(true),

            Act::FullScreenViewer => {
                self.viewer_full = !self.viewer_full;
                self.cinema = false;
            }
            Act::CinemaViewer => {
                self.viewer_full = !self.viewer_full;
                self.cinema = self.viewer_full;
                ctx.send_viewport_cmd(ViewportCommand::Fullscreen(self.cinema));
            }
            Act::LeaveViewer => {
                if self.viewer_full {
                    self.viewer_full = false;
                    if self.cinema {
                        self.cinema = false;
                        ctx.send_viewport_cmd(ViewportCommand::Fullscreen(false));
                    }
                }
            }
            Act::OverlaySafeArea => self.overlays.safe_area = !self.overlays.safe_area,
            Act::OverlayCrosshair => self.overlays.crosshair = !self.overlays.crosshair,
            Act::OverlayGrid => self.overlays.grid = !self.overlays.grid,
            Act::OverlayCheckerboard => self.overlays.checkerboard = !self.overlays.checkerboard,
            Act::OverlayAll => {
                let all = self.overlays.safe_area && self.overlays.crosshair && self.overlays.grid;
                self.overlays = Overlays {
                    safe_area: !all,
                    crosshair: !all,
                    grid: !all,
                    ..self.overlays
                };
            }
            Act::ShowScopes => {
                self.page = Page::Color;
                self.scopes_visible = true;
            }

            Act::TogglePlay => self.toggle_play(),
            Act::PlayForward => self.shuttle(true),
            Act::PlayReverse => self.shuttle(false),
            Act::Stop => self.playing = false,
            Act::PlayAround => self.play_around(),
            Act::PlayInToOut => self.play_in_to_out(),
            Act::ToggleLoop => self.looping = !self.looping,
            Act::ProxyOff => self.set_proxy(1),
            Act::ProxyHalf => self.set_proxy(2),
            Act::ProxyQuarter => self.set_proxy(4),
            Act::ProxyEighth => self.set_proxy(8),
            Act::ToggleOptimisedMedia => self.set_optimised(!self.use_optimised),
            Act::ToggleHardwareDecoding => {
                let on = !self.extra.prefs.hardware_decoding;
                self.set_hardware_decoding(on);
            }
            Act::ToggleAudioScrubbing => {
                let on = !self.extra.prefs.audio_scrubbing;
                self.set_audio_scrubbing(on);
            }
            Act::LinkClips => {
                let clips = self.selected_ids();
                if self.run(Command::LinkClips { clips }).is_some() {
                    self.info("The clips are linked");
                }
            }
            Act::UnlinkClips => {
                let clips = self.selected_ids();
                if self.run(Command::UnlinkClips { clips }).is_some() {
                    self.info("The clips are unlinked: each can be moved and trimmed alone");
                }
            }
            Act::ToggleLinkedSelection => {
                let on = !self.project().linked_selection;
                if self.run(Command::SetLinkedSelection { on }).is_some() {
                    self.info(if on {
                        "Linked Selection is on: linked clips are edited together"
                    } else {
                        "Linked Selection is off: each clip is edited alone (for J- and L-cuts)"
                    });
                }
            }
            Act::DeleteRenderCache => match self.render_cache.clear() {
                Ok(0) => self.info("The render cache was emptied"),
                Ok(n) => self.info(format!("The render cache was emptied ({n} files deleted)")),
                Err(e) => self.fail(format!("could not delete the render cache: {e}")),
            },
            Act::GoFirst => self.go_to(0),
            Act::GoLast => self.go_to(u64::MAX),
            Act::GoIn => self.go_to(self.project().mark_in.unwrap_or(0)),
            Act::GoOut => self.go_to(
                self.project()
                    .mark_out
                    .map_or(u64::MAX, |o| o.saturating_sub(1)),
            ),
            Act::PreviousFrame => self.go_to(at.saturating_sub(1)),
            Act::NextFrame => self.go_to(at + 1),
            Act::PreviousEdit => self.go_to_edit(false),
            Act::NextEdit => self.go_to_edit(true),
            Act::SecondBack => self.go_to(at.saturating_sub(self.project().fps.nominal())),
            Act::SecondForward => self.go_to(at + self.project().fps.nominal()),

            Act::GenerateSolid => self.add_generated(
                "Solid",
                MediaSource::Solid {
                    rgb: self.pool.solid_color(),
                },
            ),
            Act::GenerateBars => self.add_generated("Color Bars", MediaSource::ColorBars),
            Act::GenerateGradient => {
                let (from, to) = self.pool.gradient_colors();
                self.add_generated("Gradient", MediaSource::Gradient { from, to });
            }

            Act::ResetAllGrades => {
                let commands: Vec<Command> = self
                    .project()
                    .tracks
                    .iter()
                    .flat_map(|t| t.clips.iter())
                    .filter(|c| c.is_graded() || !c.nodes.is_empty())
                    .map(|c| Command::ResetNodes { clip: c.id })
                    .collect();
                if commands.is_empty() {
                    self.info("No clip has a grade");
                } else if let Err(e) = self.session.dispatch_all(commands) {
                    self.fail(e);
                }
            }
            Act::ResetCurrentGrade => {
                if let Some(clip) = self.target_clip() {
                    let node = self.node_for(clip);
                    self.run(Command::SetGrade {
                        clip,
                        grade: ColorGrade::default(),
                        node,
                    });
                }
            }
            Act::AddSerialNode => {
                if let Some(clip) = self.current_clip() {
                    let after = self.node_for(clip);
                    if self.run(Command::AddSerialNode { clip, after }).is_some() {
                        self.color_node = after + 1;
                        self.info(format!("Added node {:02}", after + 2));
                    }
                }
            }
            Act::ToggleNode => {
                if let Some(clip) = self.current_clip() {
                    let node = self.node_for(clip);
                    let on = self
                        .project()
                        .clip(clip)
                        .and_then(|(_, c)| c.node(node))
                        .is_some_and(|g| g.enabled);
                    self.run(Command::SetNodeEnabled {
                        clip,
                        node,
                        enabled: !on,
                    });
                }
            }
            Act::ApplyLut => self.lut_dialog(ctx),
            Act::ToggleScopes => self.scopes_visible = !self.scopes_visible,

            Act::ResetLayout => {
                ctx.memory_mut(|m| m.reset_areas());
                ctx.data_mut(|d| d.clear());
                // The panels' dividers too: they are part of the layout (0.3.0).
                self.extra.prefs.panels = crate::panels::Panels::default();
                #[cfg(not(test))]
                self.extra.prefs.save();
                self.info("The layout was reset");
            }
            Act::SaveLayout => self.extra.save_layout = Some(String::new()),
            Act::LoadLayout => self.extra.load_layout = true,
            Act::PageMedia => self.page = Page::Media,
            Act::PageCut => self.page = Page::Cut,
            Act::PageEdit => self.page = Page::Edit,
            Act::PageColor => self.page = Page::Color,
            Act::PageDeliver => self.page = Page::Deliver,
            Act::PageAudio => self.page = Page::Audio,

            Act::About => self.show_about = true,
            #[cfg(not(target_arch = "wasm32"))]
            Act::ExportLog => self.export_log(ctx),
            // The browser build's messages are in the browser's console.
            #[cfg(target_arch = "wasm32")]
            Act::ExportLog => {
                self.info("the browser's console (F12) holds this page's log")
            }
            #[cfg(not(target_arch = "wasm32"))]
            Act::CheckForUpdates => self.updater.check(ctx),
            // The browser build is the page itself: reloading it gets the latest version.
            #[cfg(target_arch = "wasm32")]
            Act::CheckForUpdates => self.info("reload the page to get the latest version"),
            Act::ToggleHighlight => {
                self.page = Page::Color;
                self.color.highlight = !self.color.highlight;
            }
            Act::MarkTimeline => self.mark_timeline(),
            Act::SelectNearestEdit => self.select_nearest_edit(),
            Act::PositionLock => {
                self.extra.position_lock = !self.extra.position_lock;
                self.info(if self.extra.position_lock {
                    "Position Lock is on: clips cannot be moved or trimmed"
                } else {
                    "Position Lock is off"
                });
            }
            Act::SelectionFollowsPlayhead => {
                self.extra.selection_follows = !self.extra.selection_follows;
                self.follow_playhead();
            }
            Act::PageEffects => self.go_to_page(Page::Effects),
            Act::ShowInspector => {
                if self.page != Page::Edit {
                    self.page = Page::Edit;
                    self.inspector_open = true;
                } else {
                    self.inspector_open = !self.inspector_open;
                }
            }
            Act::ShowMixer => self.go_to_page(Page::Audio),
            Act::ShowMediaPool => self.go_to_page(Page::Media),
            Act::ShowEditIndex => self.extra.edit_index = !self.extra.edit_index,
            Act::TimecodeWindow => self.extra.timecode_window = !self.extra.timecode_window,
            Act::PlayAroundMark => self.play_around_mark(),
            Act::ChangeClipSpeed => self.open_speed_dialog(),
            Act::ChangeClipDuration => self.open_duration_dialog(),
            Act::PreviousNodeGraph => self.go_to_clip(false),
            Act::NextNodeGraph => self.go_to_clip(true),
            Act::AddNodeBefore => self.add_node_before(),
            Act::AddOutsideNode => self.add_outside_node(),
            Act::TimelineFromBin => self.timeline_from_bin(),
            Act::BounceMix => self.bounce_mix(ctx),
            Act::BounceSelectedTracks => self.bounce_tracks(ctx),
            Act::AddParallelNode => self.add_mode_node(picasso_color::NodeMode::Parallel),
            Act::AddLayerNode => self.add_mode_node(picasso_color::NodeMode::Layer),
            Act::LabelNode => self.open_label_dialog(),
            Act::TimelineFromSelection => self.timeline_from_selection(),
            Act::Manual => self.extra.manual = true,
            Act::LockClip => self.toggle_clip_lock(),
            Act::RenderInPlace => self.render_in_place(ctx),
            Act::ClipAttributes => self.open_attributes(),
            Act::NewMulticam => self.open_new_multicam(),
            Act::MulticamViewer => self.toggle_multicam_viewer(),
            Act::SetClipColor(color) => self.set_clip_color(color),
            Act::NormalizeAudio => self.extra.normalize = Some(-9.0),
            Act::AutoAlignTimecode => self.align_by_timecode(),
            Act::AutoAlignWaveform => self.align_by_waveform(),
            Act::ExportEdl => self.export_edl(ctx),
            Act::ExportOtio => self.export_otio(ctx),
            Act::ExportXml(premiere) => self.export_xml(ctx, premiere),
            Act::ExportAaf => self.export_aaf(ctx),
            Act::ImportTimeline => self.open_dialog(ctx),
            Act::ExportSubtitles => self.export_subtitles(ctx),
            Act::CreateSubtitleTrack => {
                self.run(Command::AddTrack {
                    kind: TrackKind::Video,
                });
                self.info(
                    "Added a track for subtitles: Timeline > Create Subtitle Region adds one",
                );
            }
            Act::CreateSubtitleRegion => self.add_subtitle(),
            Act::StillToPool => self.still_to_pool(),
            Act::MatchFrame => self.match_frame_to_source(),
            Act::SpeedRamp => match self.target_clip() {
                Some(clip) => self.extra.ramp = Some(RampDialog::new(clip)),
                None => self.fail("select a clip first"),
            },
            Act::TrimMode => {
                self.extra.trim_mode = true;
                self.info("Trim mode: drag a clip to slip it, an edge to ripple the track");
            }
            Act::SelectionMode => self.extra.trim_mode = false,
            Act::DynamicTrim => self.toggle_dynamic_trim(),
            Act::LoopTrim => {
                self.extra.loop_trim = !self.extra.loop_trim;
                self.info(if self.extra.loop_trim {
                    "Loop Trim is on: after a trim the edit plays in a loop"
                } else {
                    "Loop Trim is off"
                });
            }
            Act::AsymmetricTrim => {
                self.extra.asymmetric = !self.extra.asymmetric;
                self.info(if self.extra.asymmetric {
                    "Asymmetric Trimming is on: dragging an edit moves one side and ripples"
                } else {
                    "Asymmetric Trimming is off"
                });
            }
            Act::FolderAsBin => {
                #[cfg(not(target_arch = "wasm32"))]
                crate::platform::pick_bin_folder(self.inbox.clone(), ctx.clone());
                #[cfg(target_arch = "wasm32")]
                self.fail("importing a folder needs the desktop app");
            }
            Act::Preferences => self.extra.preferences = true,
            Act::Language(language) => {
                self.extra.prefs.language = language;
                crate::i18n::set(language);
                #[cfg(not(test))]
                self.extra.prefs.save();
                self.info(format!("{}: {}", crate::i18n::tr("Language"), language.name()));
            }
        }
    }

    /// Runs the acts of the keys pressed this frame.
    pub(crate) fn run_shortcuts(&mut self, ctx: &Context) {
        // A key being chosen in Keyboard Customization does nothing else.
        if ctx.egui_wants_keyboard_input() || self.extra.keys.capture.is_some() {
            return;
        }
        for binding in menu::bindings().iter() {
            if ctx.input_mut(|i| i.consume_shortcut(&binding.shortcut))
                && self.act_enabled(binding.act)
            {
                self.do_act(ctx, binding.act);
            }
        }
        // The three edit tools of the transport row have keys of their own.
        use egui::{Key, Modifiers};
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::F9)) {
            self.insert_edit();
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::F10)) {
            self.overwrite_edit();
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::F11)) {
            self.replace_edit();
        }
    }

    // ----- drawing the menus -----

    pub(crate) fn draw_entries(&mut self, ui: &mut Ui, ctx: &Context, entries: &'static [Entry]) {
        for entry in entries {
            match entry {
                Entry::Sep => {
                    ui.separator();
                }
                Entry::Sub { label, items } if *label == "Recent Projects" => {
                    let recent = self.recent(10);
                    ui.menu_button(crate::i18n::tr(label), |ui| {
                        ui.set_min_width(240.0);
                        if recent.is_empty() {
                            ui.add_enabled(false, Button::new("No recent projects"));
                        }
                        for entry in &recent {
                            if ui.button(&entry.name).on_hover_text(&entry.path).clicked() {
                                ui.close();
                                self.open_recent(ctx, entry);
                            }
                        }
                        ui.separator();
                        self.draw_entries(ui, ctx, items);
                    });
                }
                Entry::Sub { label, items } => {
                    ui.add_enabled_ui(menu::is_live(items), |ui| {
                        ui.menu_button(crate::i18n::tr(label), |ui| {
                            ui.set_min_width(240.0);
                            // A long list (the languages) is a short window that scrolls, not a
                            // menu taller than the screen.
                            if items.len() > 14 {
                                egui::ScrollArea::vertical()
                                    .max_height(340.0)
                                    .auto_shrink([true, true])
                                    .show(ui, |ui| self.draw_entries(ui, ctx, items));
                            } else {
                                self.draw_entries(ui, ctx, items);
                            }
                        });
                    });
                }
                Entry::Item {
                    label,
                    shown,
                    keys,
                    act,
                } => {
                    let enabled = act.is_some_and(|a| self.act_enabled(a));
                    let mut button = Button::new(crate::i18n::tr(label));
                    let text = menu::display_keys(*act, shown, keys);
                    if !text.is_empty() {
                        button = button.shortcut_text(text);
                    }
                    if let Some(checked) = act.and_then(|a| self.act_checked(a)) {
                        button = button.selected(checked);
                    }
                    let response = ui.add_enabled(enabled, button);
                    let response = if act.is_none() {
                        response.on_disabled_hover_text("Planned: not built yet")
                    } else {
                        response
                    };
                    if response.clicked() {
                        if let Some(act) = act {
                            ui.close();
                            self.do_act(ctx, *act);
                        }
                    }
                }
            }
        }
    }

    // ----- windows -----

    pub(crate) fn action_windows(&mut self, ctx: &Context) {
        #[cfg(not(target_arch = "wasm32"))]
        self.updater.window(ctx);
        if self.show_about {
            let mut open = true;
            egui::Window::new("About Picasso Suite")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.heading("Picasso Suite");
                    // Shift-click the version for the credits line (`crate::credits`); a plain
                    // click does nothing, so it stays a discovery.
                    let version = ui.add(
                        egui::Label::new(format!("Version {}", env!("CARGO_PKG_VERSION")))
                            .sense(egui::Sense::click()),
                    );
                    if version.clicked() && ui.input(|i| i.modifiers.shift) {
                        self.show_credits = true;
                    }
                    if let Some((card, api)) = &self.graphics {
                        ui.label(format!("Graphics: {card} ({api})"));
                    }
                    ui.label(
                        "A video editor written in Rust for Windows, Linux, macOS and the web.",
                    );
                    ui.label("Free software under the GPL, version 3 or later.");
                    if self.show_credits {
                        ui.separator();
                        crate::credits::row(ui);
                    }
                });
            self.show_about = open;
            if !self.show_about {
                self.show_credits = false;
            }
        }
        self.keyboard_window(ctx);
        let mut dialog = self.marker_dialog.take();
        if let Some(d) = dialog.as_mut() {
            let mut keep = true;
            let mut apply = false;
            let mut delete = false;
            let fps = self.project().fps;
            let title = if d.exists {
                "Modify marker"
            } else {
                "Add marker"
            };
            egui::Window::new(title)
                .open(&mut keep)
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label(format!("At {}", self.tc(d.frame as i64)));
                    ui.horizontal(|ui| {
                        ui.label("Name");
                        ui.text_edit_singleline(&mut d.name);
                    });
                    ui.horizontal(|ui| {
                        ui.label("Color");
                        for color in 0..MARKER_COLORS {
                            let (rect, response) = ui
                                .allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::click());
                            ui.painter()
                                .rect_filled(rect, 4.0, crate::theme::marker_color(color));
                            if d.color == color {
                                ui.painter().rect_stroke(
                                    rect.expand(2.0),
                                    5.0,
                                    egui::Stroke::new(2.0, egui::Color32::WHITE),
                                    egui::StrokeKind::Outside,
                                );
                            }
                            if response.clicked() {
                                d.color = color;
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        apply = ui.button(if d.exists { "Update" } else { "Add" }).clicked();
                        if d.exists {
                            delete = ui.button("Delete").clicked();
                        }
                    });
                });
            let _ = fps;
            if apply {
                self.run(Command::SetMarker {
                    frame: d.frame,
                    name: d.name.trim().to_string(),
                    color: d.color,
                });
                keep = false;
            }
            if delete {
                self.run(Command::DeleteMarker { frame: d.frame });
                keep = false;
            }
            if !keep {
                dialog = None;
            }
        }
        self.marker_dialog = dialog;
    }

    // ----- saving and reverting -----

    pub(crate) fn save_copy(&mut self, ctx: &Context) {
        let text = match self.project().to_json() {
            Ok(text) => text,
            Err(e) => {
                self.fail(e);
                return;
            }
        };
        let bytes = match self.project_file_bytes(&text) {
            Ok(bytes) => bytes,
            Err(e) => {
                self.fail(e);
                return;
            }
        };
        platform::save_as(
            self.inbox.clone(),
            ctx.clone(),
            format!(
                "{}.picasso",
                self.project().name.replace(['/', '\\', ':'], "_")
            ),
            (
                "Picasso Suite project",
                platform::PROJECT_EXTENSIONS.to_vec(),
            ),
            bytes,
            false,
        );
    }

    pub(crate) fn revert(&mut self) {
        let Some(text) = self.saved_text.clone() else {
            self.fail("this project has not been saved yet");
            return;
        };
        // The changes are given up on purpose: a copy kept for a crash would only offer them back.
        let path = self.project_path.clone().map(std::path::PathBuf::from);
        let title = self.project().name.clone();
        crate::autosave::clear(path.as_deref(), &title);
        let name = self.project().name.clone();
        let path = self.project_path.clone();
        self.open_text(&name, path, &text);
    }

    /// The clips a set of `BTreeSet` ids names, for the code that keeps `selected_more` tidy.
    pub(crate) fn prune_selection(&mut self) {
        let project = self.project();
        let alive: BTreeSet<ClipId> = self
            .selected_more
            .iter()
            .copied()
            .filter(|id| project.clip(*id).is_some())
            .collect();
        self.selected_more = alive;
    }
}

#[cfg(test)]
#[path = "actions_tests.rs"]
mod tests;
