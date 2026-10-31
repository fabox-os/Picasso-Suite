//! Every change to a project is a command.
//!
//! A command is validated completely before it changes anything, so a refused command leaves the
//! project exactly as it was.

use serde::{Deserialize, Serialize};

use crate::error::EditError;
use crate::project::{
    BlendMode, Clip, ClipAudio, ClipId, Marker, MediaId, MediaItem, MediaSource, Project, Track,
    TrackId, TrackKind, Transform, MARKER_COLORS, MAX_FRAMES, MAX_MARKERS,
};
use crate::time::Fps;
use picasso_color::ColorGrade;

/// What a command made, so the caller can select it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Nothing,
    Media(MediaId),
    Clip(ClipId),
    /// The two halves of a split: the one that kept the id, and the new one.
    Split(ClipId, ClipId),
    Track(TrackId),
}

/// A change to a project.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum Command {
    RenameProject {
        name: String,
    },
    SetFormat {
        width: u32,
        height: u32,
        fps: Fps,
    },
    AddMedia {
        name: String,
        source: MediaSource,
        length: Option<u64>,
        default_duration: u64,
    },
    RemoveMedia {
        media: MediaId,
    },
    AddTrack {
        kind: TrackKind,
    },
    /// Deletes a track and the clips on it. The last video or audio track cannot be deleted.
    RemoveTrack {
        track: TrackId,
    },
    SetTrackLocked {
        track: TrackId,
        locked: bool,
    },
    SetTrackMuted {
        track: TrackId,
        muted: bool,
    },
    /// Solo: with any track soloed, only the soloed tracks are heard.
    SetTrackSolo {
        track: TrackId,
        solo: bool,
    },
    /// Whether the track's picture is drawn; off keeps it out of the program.
    SetTrackPicture {
        track: TrackId,
        picture: bool,
    },
    /// Places media on a track; `duration` defaults to the media's. With `with_sound`, a video's
    /// sound also gets a clip on an audio track, linked to the picture (see [`Command::LinkClips`]).
    AddClip {
        track: TrackId,
        media: MediaId,
        start: u64,
        duration: Option<u64>,
        #[serde(default)]
        with_sound: bool,
    },
    /// Puts a clip at `start` and pushes everything after it on the track later (a clip that
    /// is cut through by `start` is split there). With `with_sound`, the video's sound is
    /// inserted the same way on the first audio track.
    InsertClip {
        track: TrackId,
        media: MediaId,
        start: u64,
        duration: Option<u64>,
        #[serde(default)]
        with_sound: bool,
    },
    /// Puts a clip at `start` over whatever is there: clips it covers are removed, clips it
    /// partly covers are shortened, and a clip it lies inside of is split around it. With
    /// `with_sound`, the video's sound overwrites the first audio track the same way.
    OverwriteClip {
        track: TrackId,
        media: MediaId,
        start: u64,
        duration: Option<u64>,
        #[serde(default)]
        with_sound: bool,
    },
    /// Puts all of the media between `start` and `start + duration`, over what is there,
    /// changing the clip's speed so it fits exactly.
    FitToFill {
        track: TrackId,
        media: MediaId,
        start: u64,
        duration: u64,
        #[serde(default)]
        with_sound: bool,
    },
    /// Puts the media at `start` on the first free track above every clip there, adding a
    /// track when none is free.
    PlaceOnTop {
        media: MediaId,
        start: u64,
        #[serde(default)]
        with_sound: bool,
    },
    /// Links clips (and every clip already linked to them), so they are edited together.
    LinkClips {
        clips: Vec<ClipId>,
    },
    /// Unlinks the clips and everything linked to them.
    UnlinkClips {
        clips: Vec<ClipId>,
    },
    /// Turns Linked Selection on or off: whether editing a clip edits its linked clips too.
    SetLinkedSelection {
        on: bool,
    },
    /// Swaps the media of a clip, keeping its place, length, look and sound settings.
    ReplaceClip {
        clip: ClipId,
        media: MediaId,
    },
    /// Moves the cut between two neighbouring clips of a track to `to`: one gets shorter and
    /// the other longer, and nothing else moves.
    RollTrim {
        left: ClipId,
        right: ClipId,
        to: u64,
    },
    /// Shows another part of the media in the same place: the clip keeps its start and
    /// length, and its first frame of media becomes `source_in`.
    SlipClip {
        clip: ClipId,
        source_in: u64,
    },
    /// Moves the clip to `start` keeping what it shows; the clips touching it on either side
    /// get longer or shorter, so the track keeps its length and no gap opens.
    SlideClip {
        clip: ClipId,
        start: u64,
    },
    MoveClip {
        clip: ClipId,
        track: TrackId,
        start: u64,
    },
    /// Moves the first frame of the clip to `start`, trimming it (or extending it).
    TrimStart {
        clip: ClipId,
        start: u64,
    },
    /// Moves the end of the clip to `end` (the first frame after it).
    TrimEnd {
        clip: ClipId,
        end: u64,
    },
    Split {
        clip: ClipId,
        at: u64,
    },
    Delete {
        clip: ClipId,
    },
    /// Deletes the clip and closes the gap on its track.
    RippleDelete {
        clip: ClipId,
    },
    SetOpacity {
        clip: ClipId,
        opacity: f32,
    },
    /// Sets the grade of colour node `node` of the clip (0 is the first node).
    SetGrade {
        clip: ClipId,
        grade: ColorGrade,
        #[serde(default)]
        node: usize,
    },
    /// Adds a neutral serial node after node `after` (Color › Nodes › Add Serial Node).
    AddSerialNode {
        clip: ClipId,
        after: usize,
    },
    /// Deletes colour node `node`; the last node left cannot go (reset it instead).
    DeleteNode {
        clip: ClipId,
        node: usize,
    },
    /// Switches colour node `node` on or off, keeping its settings.
    SetNodeEnabled {
        clip: ClipId,
        node: usize,
        enabled: bool,
    },
    /// Takes every node of the clip back to one neutral node.
    ResetNodes {
        clip: ClipId,
    },
    /// Shows or hides a clip without removing it.
    SetClipEnabled {
        clip: ClipId,
        enabled: bool,
    },
    /// Moves, scales, turns and crops a clip's picture.
    SetClipTransform {
        clip: ClipId,
        transform: Transform,
    },
    /// Chooses how a clip's picture is mixed with what is below it.
    SetBlend {
        clip: ClipId,
        blend: BlendMode,
    },
    /// Puts a keyframe of `param` at clip frame `at` (frames from the clip's start), replacing
    /// one there; `value: None` takes the keyframe there away.
    SetKeyframe {
        clip: ClipId,
        param: crate::KeyParam,
        at: u64,
        value: Option<f32>,
        #[serde(default)]
        ease: bool,
    },
    /// Sets a track's EQ and dynamics.
    SetTrackFx {
        track: TrackId,
        fx: crate::TrackFx,
    },
    /// Adds an effect after the clip's others.
    AddEffect {
        clip: ClipId,
        kind: crate::EffectKind,
    },
    /// Changes effect `index` of a clip: its strength and whether it is on.
    SetEffect {
        clip: ClipId,
        index: usize,
        effect: crate::Effect,
    },
    RemoveEffect {
        clip: ClipId,
        index: usize,
    },
    /// Replaces the entire effects list of a clip (used by presets).
    SetEffects {
        clip: ClipId,
        effects: Vec<crate::Effect>,
    },
    /// Takes away every keyframe of `param`: the property holds its value again.
    ClearKeyframes {
        clip: ClipId,
        param: crate::KeyParam,
    },
    /// A speed ramp: the clip (and the clips linked to it) is cut into `speeds.len()` pieces
    /// that each show an equal part of its media at their own speed, one after the other from
    /// the clip's start; the clips after it on its track move by the change in length.
    SpeedRamp {
        clip: ClipId,
        speeds: Vec<f32>,
    },
    /// Changes how fast a clip plays. The clip stays at its start and gets shorter or longer so
    /// that it still shows the same part of the media.
    SetClipSpeed {
        clip: ClipId,
        speed: f32,
    },
    /// Sets the gain and fades of a clip's sound.
    SetClipAudio {
        clip: ClipId,
        audio: ClipAudio,
    },
    /// Sets what a track adds to the mix: a gain in decibels and a pan from -1 to 1.
    SetTrackMix {
        track: TrackId,
        gain_db: f32,
        pan: f32,
    },
    /// Places a copy of a clip's contents and look on a track.
    PasteClip {
        track: TrackId,
        media: MediaId,
        name: String,
        start: u64,
        source_in: u64,
        duration: u64,
        opacity: f32,
        grade: ColorGrade,
        enabled: bool,
        #[serde(default)]
        audio: ClipAudio,
        #[serde(default = "one")]
        speed: f32,
        #[serde(default)]
        transform: Transform,
        #[serde(default)]
        blend: BlendMode,
        #[serde(default = "yes")]
        sound: bool,
        #[serde(default)]
        nodes: Vec<ColorGrade>,
        #[serde(default)]
        reverse: bool,
        #[serde(default)]
        freeze: bool,
    },
    /// Puts clips into one compound clip named `name`, which takes their place on the lowest
    /// video track among them. Its clips are kept in a timeline of their own.
    CreateCompound {
        clips: Vec<ClipId>,
        name: String,
    },
    /// Creates an effects clip named `name` at `start` on a free video track above the clips
    /// there. Like a compound clip it is made by a command so the whole edit is one undo step,
    /// but it is transparent: its grade and effects apply to the composite below.
    CreateEffectsClip {
        start: u64,
        name: String,
    },
    /// Puts the clips of a compound clip back on the timeline in its place.
    DecomposeCompound {
        clip: ClipId,
    },
    /// Keeps the timeline being edited and starts a new, empty one named `name`.
    NewTimeline {
        name: String,
    },
    /// Edits timeline `id` instead, keeping the one being edited as it is.
    SwitchTimeline {
        id: u64,
    },
    RenameTimeline {
        id: u64,
        name: String,
    },
    /// Deletes timeline `id`; deleting the one being edited switches to another. The last
    /// timeline cannot be deleted.
    DeleteTimeline {
        id: u64,
    },
    /// Opens a gap of `frames` at `at` on a track: the clip there is cut, and everything from
    /// `at` on moves later.
    InsertGap {
        track: TrackId,
        at: u64,
        frames: u64,
    },
    /// Sets (or with None removes) the transition at the start of a clip, from the clip that
    /// ends where it starts.
    SetTransition {
        clip: ClipId,
        transition: Option<crate::Transition>,
    },
    /// Makes a multicam clip of video media, each with the frame that shows the sync moment.
    CreateMulticam {
        name: String,
        angles: Vec<(MediaId, i64)>,
    },
    DeleteMulticam {
        id: u64,
    },
    /// Moves an angle's sync frame (for the switches made afterwards).
    SetAngleOffset {
        multicam: u64,
        angle: usize,
        offset: i64,
    },
    /// From frame `at` on, the video clip of the multicam shows another angle (the clip is cut
    /// there).
    SwitchAngle {
        multicam: u64,
        angle: usize,
        at: u64,
    },
    /// Adds a bin to the media pool.
    AddBin {
        name: String,
    },
    RenameBin {
        id: u64,
        name: String,
    },
    /// Deletes a bin; its media go back to Master.
    DeleteBin {
        id: u64,
    },
    /// Keeps media in a bin (None: Master).
    SetMediaBin {
        media: MediaId,
        bin: Option<u64>,
    },
    AddSmartBin {
        name: String,
        contains: String,
        kind: Option<String>,
    },
    DeleteSmartBin {
        id: u64,
    },
    /// Adds a still to the gallery.
    GrabStill {
        name: String,
        frame: u64,
        grade: ColorGrade,
        nodes: Vec<ColorGrade>,
        png: String,
    },
    DeleteStill {
        id: u64,
    },
    /// Gives a clip the nodes of a still.
    ApplyStill {
        still: u64,
        clip: ClipId,
    },
    /// Changes the words or the look of a title.
    SetTitle {
        media: MediaId,
        title: crate::TitleText,
    },
    /// Locks a clip in place, or unlocks it.
    SetClipLocked {
        clip: ClipId,
        locked: bool,
    },
    /// Gives a clip one of the clip colours (None: the track's colour).
    SetClipColor {
        clip: ClipId,
        color: Option<u8>,
    },
    /// Names a clip.
    RenameClip {
        clip: ClipId,
        name: String,
    },
    /// Swaps a clip's footage for a render of it (media made from the clip with its grade,
    /// effects, transform and speed baked in), which then plays untouched.
    RenderInPlace {
        clip: ClipId,
        media: MediaId,
    },
    /// Plays the clip's footage backwards, or forwards again.
    SetReverse {
        clip: ClipId,
        reverse: bool,
    },
    /// Holds the clip on frame `frame` of its media for its whole length (Some), or plays it
    /// again from there (None).
    SetFreeze {
        clip: ClipId,
        frame: Option<u64>,
    },
    /// Adds a marker, or changes the one at the same frame.
    SetMarker {
        frame: u64,
        name: String,
        color: u8,
    },
    DeleteMarker {
        frame: u64,
    },
    ClearMarkers,
    /// Sets the range playback and export use; `None` clears an end.
    SetMarks {
        mark_in: Option<u64>,
        mark_out: Option<u64>,
    },
    /// Starts a new timeline named `name` and puts the tracks, clips and markers of a timeline
    /// read from another program on it (File > Import > Timeline), as one edit. A clip that
    /// cannot be placed (its media is missing or does not fit the track, it overlaps the clip
    /// before it, or it runs past the end of its media) is left out.
    ImportTimeline {
        name: String,
        tracks: Vec<crate::ImportedTrack>,
        markers: Vec<Marker>,
    },
    /// Records the start timecode a media file carries, so clips of it can be aligned by
    /// timecode (Audio > Auto Align Clips).
    SetMediaTimecode {
        media: MediaId,
        timecode: Option<String>,
    },
    /// Points media with a file at `path` instead, for when the file moved or the project was
    /// opened on another computer. Every clip of it follows: they share the media, not the path.
    /// Generated media (a title, a solid, a compound clip) has no file to point at.
    RelinkMedia {
        media: MediaId,
        path: String,
    },
    /// Moves the clips so that the same source timecode falls on the same timeline frame. The
    /// clip with the earliest timecode stays where it is and the others line up with it.
    AlignClips {
        clips: Vec<ClipId>,
    },
    /// Moves each clip to the frame given, and its linked sound by the same distance: what
    /// Auto Align by waveform decides from the sound, which the project does not hear.
    PlaceClips {
        starts: Vec<(ClipId, u64)>,
    },
}

impl Command {
    /// The stable id of the command, for history, logs and a future CLI.
    pub fn id(&self) -> &'static str {
        match self {
            Self::RenameProject { .. } => "project.rename",
            Self::SetFormat { .. } => "project.set_format",
            Self::AddMedia { .. } => "media.add",
            Self::RemoveMedia { .. } => "media.remove",
            Self::AddTrack { .. } => "track.add",
            Self::RemoveTrack { .. } => "track.remove",
            Self::SetTrackLocked { .. } => "track.set_locked",
            Self::SetTrackMuted { .. } => "track.set_muted",
            Self::SetTrackSolo { .. } => "track.set_solo",
            Self::SetTrackPicture { .. } => "track.set_picture",
            Self::AddClip { .. } => "clip.add",
            Self::FitToFill { .. } => "clip.fit_to_fill",
            Self::PlaceOnTop { .. } => "clip.place_on_top",
            Self::MoveClip { .. } => "clip.move",
            Self::TrimStart { .. } => "clip.trim_start",
            Self::TrimEnd { .. } => "clip.trim_end",
            Self::Split { .. } => "clip.split",
            Self::Delete { .. } => "clip.delete",
            Self::RippleDelete { .. } => "clip.ripple_delete",
            Self::SetOpacity { .. } => "clip.set_opacity",
            Self::SetGrade { .. } => "clip.set_grade",
            Self::SetClipEnabled { .. } => "clip.set_enabled",
            Self::SetClipTransform { .. } => "clip.set_transform",
            Self::SetBlend { .. } => "clip.set_blend",
            Self::SetClipSpeed { .. } => "clip.set_speed",
            Self::SpeedRamp { .. } => "clip.speed_ramp",
            Self::SetKeyframe { .. } => "clip.set_keyframe",
            Self::AddEffect { .. } => "clip.add_effect",
            Self::SetTrackFx { .. } => "track.set_fx",
            Self::SetEffect { .. } => "clip.set_effect",
            Self::RemoveEffect { .. } => "clip.remove_effect",
            Self::SetEffects { .. } => "clip.set_effects",
            Self::ClearKeyframes { .. } => "clip.clear_keyframes",
            Self::InsertClip { .. } => "clip.insert",
            Self::OverwriteClip { .. } => "clip.overwrite",
            Self::ReplaceClip { .. } => "clip.replace",
            Self::RollTrim { .. } => "clip.roll_trim",
            Self::SlipClip { .. } => "clip.slip",
            Self::SlideClip { .. } => "clip.slide",
            Self::SetClipAudio { .. } => "clip.set_audio",
            Self::SetTrackMix { .. } => "track.set_mix",
            Self::PasteClip { .. } => "clip.paste",
            Self::SetReverse { .. } => "clip.set_reverse",
            Self::SetClipLocked { .. } => "clip.set_locked",
            Self::SetClipColor { .. } => "clip.set_color",
            Self::RenameClip { .. } => "clip.rename",
            Self::RenderInPlace { .. } => "clip.render_in_place",
            Self::SetTitle { .. } => "media.set_title",
            Self::GrabStill { .. } => "gallery.grab_still",
            Self::CreateMulticam { .. } => "multicam.create",
            Self::DeleteMulticam { .. } => "multicam.delete",
            Self::SetAngleOffset { .. } => "multicam.set_offset",
            Self::SwitchAngle { .. } => "multicam.switch_angle",
            Self::AddBin { .. } => "bin.add",
            Self::RenameBin { .. } => "bin.rename",
            Self::DeleteBin { .. } => "bin.delete",
            Self::SetMediaBin { .. } => "media.set_bin",
            Self::AddSmartBin { .. } => "bin.add_smart",
            Self::DeleteSmartBin { .. } => "bin.delete_smart",
            Self::DeleteStill { .. } => "gallery.delete_still",
            Self::ApplyStill { .. } => "gallery.apply_still",
            Self::SetTransition { .. } => "clip.set_transition",
            Self::InsertGap { .. } => "track.insert_gap",
            Self::NewTimeline { .. } => "timeline.new",
            Self::ImportTimeline { .. } => "timeline.import",
            Self::SetMediaTimecode { .. } => "media.set_timecode",
            Self::RelinkMedia { .. } => "media.relink",
            Self::AlignClips { .. } => "clip.align",
            Self::PlaceClips { .. } => "clip.place",
            Self::CreateCompound { .. } => "clip.create_compound",
            Self::CreateEffectsClip { .. } => "clip.create_effects_clip",
            Self::DecomposeCompound { .. } => "clip.decompose_compound",
            Self::SwitchTimeline { .. } => "timeline.switch",
            Self::RenameTimeline { .. } => "timeline.rename",
            Self::DeleteTimeline { .. } => "timeline.delete",
            Self::SetFreeze { .. } => "clip.set_freeze",
            Self::SetMarker { .. } => "marker.set",
            Self::DeleteMarker { .. } => "marker.delete",
            Self::ClearMarkers => "marker.clear",
            Self::SetMarks { .. } => "marks.set",
            Self::LinkClips { .. } => "clip.link",
            Self::AddSerialNode { .. } => "color.add_serial_node",
            Self::DeleteNode { .. } => "color.delete_node",
            Self::SetNodeEnabled { .. } => "color.set_node_enabled",
            Self::ResetNodes { .. } => "color.reset_nodes",
            Self::UnlinkClips { .. } => "clip.unlink",
            Self::SetLinkedSelection { .. } => "timeline.set_linked_selection",
        }
    }

    /// A short name for the undo menu.
    pub fn label(&self) -> &'static str {
        match self {
            Self::RenameProject { .. } => "Rename project",
            Self::SetFormat { .. } => "Change format",
            Self::AddMedia { .. } => "Add media",
            Self::RemoveMedia { .. } => "Remove media",
            Self::AddTrack { .. } => "Add track",
            Self::RemoveTrack { .. } => "Delete track",
            Self::SetTrackLocked { .. } => "Lock track",
            Self::SetTrackMuted { .. } => "Mute track",
            Self::SetTrackSolo { .. } => "Solo track",
            Self::SetTrackPicture { .. } => "Picture track",
            Self::AddClip { .. } => "Add clip",
            Self::FitToFill { .. } => "Fit to fill",
            Self::PlaceOnTop { .. } => "Place on top",
            Self::MoveClip { .. } => "Move clip",
            Self::TrimStart { .. } | Self::TrimEnd { .. } => "Trim clip",
            Self::Split { .. } => "Split clip",
            Self::Delete { .. } => "Delete clip",
            Self::RippleDelete { .. } => "Ripple delete",
            Self::SetOpacity { .. } => "Change opacity",
            Self::SetGrade { .. } => "Change grade",
            Self::SetClipEnabled { .. } => "Enable or disable clip",
            Self::SetClipTransform { .. } => "Clip transform",
            Self::SetBlend { .. } => "Blend mode",
            Self::SetClipSpeed { .. } => "Clip speed",
            Self::SpeedRamp { .. } => "Speed ramp",
            Self::SetKeyframe { .. } => "Keyframe",
            Self::AddEffect { .. } => "Add effect",
            Self::SetTrackFx { .. } => "Track EQ and dynamics",
            Self::SetEffect { .. } => "Change effect",
            Self::RemoveEffect { .. } => "Remove effect",
            Self::SetEffects { .. } => "Replace effects",
            Self::ClearKeyframes { .. } => "Clear keyframes",
            Self::InsertClip { .. } => "Insert clip",
            Self::OverwriteClip { .. } => "Overwrite clip",
            Self::ReplaceClip { .. } => "Replace clip",
            Self::RollTrim { .. } => "Roll trim",
            Self::SlipClip { .. } => "Slip clip",
            Self::SlideClip { .. } => "Slide clip",
            Self::SetClipAudio { .. } => "Clip audio",
            Self::SetTrackMix { .. } => "Track mix",
            Self::PasteClip { .. } => "Paste clip",
            Self::SetReverse { .. } => "Reverse clip",
            Self::SetClipLocked { .. } => "Lock clip",
            Self::SetClipColor { .. } => "Clip color",
            Self::RenameClip { .. } => "Rename clip",
            Self::RenderInPlace { .. } => "Render in place",
            Self::SetTitle { .. } => "Edit title",
            Self::GrabStill { .. } => "Grab still",
            Self::CreateMulticam { .. } => "New multicam clip",
            Self::DeleteMulticam { .. } => "Delete multicam clip",
            Self::SetAngleOffset { .. } => "Sync angle",
            Self::SwitchAngle { .. } => "Switch angle",
            Self::AddBin { .. } => "New bin",
            Self::RenameBin { .. } => "Rename bin",
            Self::DeleteBin { .. } => "Delete bin",
            Self::SetMediaBin { .. } => "Move to bin",
            Self::AddSmartBin { .. } => "New smart bin",
            Self::DeleteSmartBin { .. } => "Delete smart bin",
            Self::DeleteStill { .. } => "Delete still",
            Self::ApplyStill { .. } => "Apply grade",
            Self::SetTransition { .. } => "Transition",
            Self::InsertGap { .. } => "Insert gap",
            Self::NewTimeline { .. } => "New timeline",
            Self::ImportTimeline { .. } => "Import timeline",
            Self::SetMediaTimecode { .. } => "Media timecode",
            Self::RelinkMedia { .. } => "Relink media",
            Self::AlignClips { .. } => "Align by timecode",
            Self::PlaceClips { .. } => "Align by waveform",
            Self::CreateCompound { .. } => "Compound clip",
            Self::CreateEffectsClip { .. } => "Effects clip",
            Self::DecomposeCompound { .. } => "Decompose in place",
            Self::SwitchTimeline { .. } => "Switch timeline",
            Self::RenameTimeline { .. } => "Rename timeline",
            Self::DeleteTimeline { .. } => "Delete timeline",
            Self::SetFreeze { .. } => "Freeze frame",
            Self::SetMarker { .. } => "Set marker",
            Self::DeleteMarker { .. } => "Delete marker",
            Self::ClearMarkers => "Delete all markers",
            Self::SetMarks { .. } => "Set in and out",
            Self::LinkClips { .. } => "Link clips",
            Self::AddSerialNode { .. } => "Add serial node",
            Self::DeleteNode { .. } => "Delete node",
            Self::SetNodeEnabled { .. } => "Enable or disable node",
            Self::ResetNodes { .. } => "Reset grades and nodes",
            Self::UnlinkClips { .. } => "Unlink clips",
            Self::SetLinkedSelection { .. } => "Linked selection",
        }
    }

    /// Runs the command. On `Err` the project is unchanged.
    pub fn apply(self, p: &mut Project) -> Result<Outcome, EditError> {
        match self {
            Self::RenameProject { name } => {
                p.name = name;
                Ok(Outcome::Nothing)
            }
            Self::SetFormat { width, height, fps } => {
                let pixels = u64::from(width) * u64::from(height);
                if width == 0
                    || height == 0
                    || width > 16_384
                    || height > 16_384
                    || pixels > 100_000_000
                {
                    return Err(EditError::Invalid("the picture size"));
                }
                if !fps.is_valid() {
                    return Err(EditError::Invalid("the frame rate"));
                }
                (p.width, p.height, p.fps) = (width, height, fps);
                Ok(Outcome::Nothing)
            }
            Self::AddMedia {
                name,
                source,
                length,
                default_duration,
            } => {
                if default_duration == 0
                    || default_duration > MAX_FRAMES
                    || length.is_some_and(|l| l == 0 || l > MAX_FRAMES)
                {
                    return Err(EditError::Invalid("the media length"));
                }
                let source = match source {
                    // Only a compound clip makes the timeline its media needs.
                    MediaSource::Compound { .. } => {
                        return Err(EditError::Invalid("a compound clip is made from clips"))
                    }
                    MediaSource::Title(t) => MediaSource::Title(t.sanitized()),
                    other => other,
                };
                let id = MediaId(p.fresh_id());
                p.media.push(MediaItem {
                    id,
                    name,
                    source,
                    length,
                    default_duration,
                    bin: None,
                    timecode: None,
                });
                Ok(Outcome::Media(id))
            }
            Self::RemoveMedia { media } => {
                if p.media(media).is_none() {
                    return Err(EditError::NoSuchMedia);
                }
                if p.uses_media(media) || p.multicams.iter().any(|m| m.angle_of(media).is_some()) {
                    return Err(EditError::MediaInUse);
                }
                if let Some(MediaSource::Compound { timeline }) =
                    p.media(media).map(|m| m.source.clone())
                {
                    p.timelines.retain(|t| t.id != timeline);
                }
                p.media.retain(|m| m.id != media);
                Ok(Outcome::Nothing)
            }
            Self::AddTrack { kind } => {
                if p.tracks.len() >= 64 {
                    return Err(EditError::Invalid("too many tracks"));
                }
                let n = p.tracks.iter().filter(|t| t.kind == kind).count() + 1;
                let prefix = match kind {
                    TrackKind::Video => 'V',
                    TrackKind::Audio => 'A',
                };
                let id = TrackId(p.fresh_id());
                p.tracks.push(Track {
                    id,
                    name: format!("{prefix}{n}"),
                    kind,
                    clips: Vec::new(),
                    muted: false,
                    locked: false,
                    solo: false,
                    picture: true,
                    gain_db: 0.0,
                    pan: 0.0,
                    fx: Default::default(),
                });
                Ok(Outcome::Track(id))
            }
            Self::RemoveTrack { track } => {
                let i = p
                    .tracks
                    .iter()
                    .position(|t| t.id == track)
                    .ok_or(EditError::NoSuchTrack)?;
                if p.tracks[i].locked {
                    return Err(EditError::Locked);
                }
                let kind = p.tracks[i].kind;
                if p.tracks.iter().filter(|t| t.kind == kind).count() <= 1 {
                    return Err(EditError::Invalid(
                        "the last track of its kind cannot be deleted",
                    ));
                }
                p.tracks.remove(i);
                Ok(Outcome::Nothing)
            }
            Self::SetTrackLocked { track, locked } => {
                p.track_mut(track).ok_or(EditError::NoSuchTrack)?.locked = locked;
                Ok(Outcome::Nothing)
            }
            Self::SetTrackMuted { track, muted } => {
                p.track_mut(track).ok_or(EditError::NoSuchTrack)?.muted = muted;
                Ok(Outcome::Nothing)
            }
            Self::SetTrackSolo { track, solo } => {
                p.track_mut(track).ok_or(EditError::NoSuchTrack)?.solo = solo;
                Ok(Outcome::Nothing)
            }
            Self::SetTrackPicture { track, picture } => {
                p.track_mut(track).ok_or(EditError::NoSuchTrack)?.picture = picture;
                Ok(Outcome::Nothing)
            }
            Self::AddClip {
                track,
                media,
                start,
                duration,
                with_sound,
            } => with_linked_sound(p, with_sound, SoundEdit::Place, |q| {
                add_clip(q, track, media, start, duration)
            }),
            Self::FitToFill {
                track,
                media,
                start,
                duration,
                with_sound,
            } => with_linked_sound(p, with_sound, SoundEdit::Overwrite, |q| {
                fit_to_fill(q, track, media, start, duration)
            }),
            Self::PlaceOnTop {
                media,
                start,
                with_sound,
            } => {
                let mut q = p.clone();
                let track = top_free_track(&mut q, media, start, None)?;
                let out = with_linked_sound(&mut q, with_sound, SoundEdit::Place, |r| {
                    add_clip(r, track, media, start, None)
                })?;
                *p = q;
                Ok(out)
            }
            Self::MoveClip { clip, track, start } => linked_move(p, clip, track, start),
            Self::TrimStart { clip, start } => linked_trim(p, clip, Some(start), None),
            Self::TrimEnd { clip, end } => linked_trim(p, clip, None, Some(end)),
            Self::Split { clip, at } => linked_split(p, clip, at),
            Self::Delete { clip } => with_partners(p, clip, |q, id| {
                let (ti, ci) = editable(q, id)?;
                q.tracks[ti].clips.remove(ci);
                Ok(())
            }),
            Self::RippleDelete { clip } => with_partners(p, clip, |q, id| {
                let (ti, ci) = editable(q, id)?;
                let removed = q.tracks[ti].clips.remove(ci);
                for later in &mut q.tracks[ti].clips[ci..] {
                    later.start -= removed.duration;
                }
                Ok(())
            }),
            Self::LinkClips { clips } => link_clips(p, &clips),
            Self::UnlinkClips { clips } => {
                let links: Vec<u64> = clips
                    .iter()
                    .filter_map(|id| p.clip(*id).and_then(|(_, c)| c.link))
                    .collect();
                if links.is_empty() {
                    return Err(EditError::Invalid("the clips are not linked"));
                }
                for c in p.tracks.iter_mut().flat_map(|t| t.clips.iter_mut()) {
                    if c.link.is_some_and(|l| links.contains(&l)) {
                        c.link = None;
                    }
                }
                Ok(Outcome::Nothing)
            }
            Self::SetLinkedSelection { on } => {
                p.linked_selection = on;
                Ok(Outcome::Nothing)
            }
            Self::SetOpacity { clip, opacity } => {
                if !opacity.is_finite() {
                    return Err(EditError::Invalid("the opacity"));
                }
                let (ti, ci) = editable(p, clip)?;
                p.tracks[ti].clips[ci].opacity = opacity.clamp(0.0, 1.0);
                Ok(Outcome::Nothing)
            }
            Self::SetGrade { clip, grade, node } => {
                // A grade is not an edit of the timeline, so a locked track can still be graded.
                let (ti, ci) = p.locate(clip).ok_or(EditError::NoSuchClip)?;
                *p.tracks[ti].clips[ci]
                    .node_mut(node)
                    .ok_or(EditError::Invalid("there is no such node"))? = grade.sanitized();
                Ok(Outcome::Nothing)
            }
            Self::AddSerialNode { clip, after } => {
                let (ti, ci) = p.locate(clip).ok_or(EditError::NoSuchClip)?;
                let c = &mut p.tracks[ti].clips[ci];
                if after >= c.node_count() {
                    return Err(EditError::Invalid("there is no such node"));
                }
                if c.node_count() >= MAX_NODES {
                    return Err(EditError::Invalid("too many nodes"));
                }
                // The new node is number `after + 1`; `nodes` starts at node 1.
                c.nodes.insert(after, ColorGrade::default());
                Ok(Outcome::Nothing)
            }
            Self::DeleteNode { clip, node } => {
                let (ti, ci) = p.locate(clip).ok_or(EditError::NoSuchClip)?;
                let c = &mut p.tracks[ti].clips[ci];
                if node >= c.node_count() {
                    return Err(EditError::Invalid("there is no such node"));
                }
                if c.node_count() == 1 {
                    return Err(EditError::Invalid(
                        "a clip keeps one node; reset it instead",
                    ));
                }
                if node == 0 {
                    c.grade = c.nodes.remove(0);
                    c.grade.mode = picasso_color::NodeMode::Serial;
                } else {
                    c.nodes.remove(node - 1);
                }
                Ok(Outcome::Nothing)
            }
            Self::SetNodeEnabled {
                clip,
                node,
                enabled,
            } => {
                let (ti, ci) = p.locate(clip).ok_or(EditError::NoSuchClip)?;
                p.tracks[ti].clips[ci]
                    .node_mut(node)
                    .ok_or(EditError::Invalid("there is no such node"))?
                    .enabled = enabled;
                Ok(Outcome::Nothing)
            }
            Self::ResetNodes { clip } => {
                let (ti, ci) = p.locate(clip).ok_or(EditError::NoSuchClip)?;
                let c = &mut p.tracks[ti].clips[ci];
                c.grade = ColorGrade::default();
                c.nodes.clear();
                Ok(Outcome::Nothing)
            }
            Self::SetClipEnabled { clip, enabled } => with_partners(p, clip, |q, id| {
                let (ti, ci) = editable(q, id)?;
                q.tracks[ti].clips[ci].enabled = enabled;
                Ok(())
            }),
            Self::InsertClip {
                track,
                media,
                start,
                duration,
                with_sound,
            } => with_linked_sound(p, with_sound, SoundEdit::Insert, |q| {
                insert_clip(q, track, media, start, duration)
            }),
            Self::OverwriteClip {
                track,
                media,
                start,
                duration,
                with_sound,
            } => with_linked_sound(p, with_sound, SoundEdit::Overwrite, |q| {
                overwrite_clip(q, track, media, start, duration)
            }),
            Self::ReplaceClip { clip, media } => replace_clip(p, clip, media),
            Self::RollTrim { left, right, to } => roll_trim(p, left, right, to),
            Self::SetTransition { clip, transition } => {
                let (ti, ci) = editable(p, clip)?;
                if let Some(t) = transition {
                    let start = p.tracks[ti].clips[ci].start;
                    if !p.tracks[ti]
                        .clips
                        .iter()
                        .any(|c| c.end() == start && c.id != clip)
                    {
                        return Err(EditError::Invalid(
                            "a transition needs a clip right before this one",
                        ));
                    }
                    if t.frames < 2 || t.frames > crate::MAX_TRANSITION {
                        return Err(EditError::Invalid("the transition length"));
                    }
                }
                p.tracks[ti].clips[ci].transition = transition;
                Ok(Outcome::Clip(clip))
            }
            Self::CreateMulticam { name, angles } => {
                if p.multicams.len() >= crate::multicam::MAX_MULTICAMS {
                    return Err(EditError::Invalid("too many multicam clips"));
                }
                if angles.len() < 2 || angles.len() > crate::multicam::MAX_ANGLES {
                    return Err(EditError::Invalid("a multicam clip has 2 to 16 angles"));
                }
                let mut made = Vec::new();
                for (media, offset) in angles {
                    let item = p.media(media).ok_or(EditError::NoSuchMedia)?;
                    if !matches!(item.source, MediaSource::Video { .. }) {
                        return Err(EditError::Invalid("angles must be video media"));
                    }
                    if made.iter().any(|a: &crate::Angle| a.media == media) {
                        return Err(EditError::Invalid("an angle is used twice"));
                    }
                    made.push(crate::Angle {
                        media,
                        offset: offset.clamp(
                            -crate::multicam::MAX_ANGLE_OFFSET,
                            crate::multicam::MAX_ANGLE_OFFSET,
                        ),
                    });
                }
                let id = p.fresh_id();
                p.multicams.push(crate::Multicam {
                    id,
                    name: crate::Multicam::clean_name(&name),
                    angles: made,
                });
                Ok(Outcome::Nothing)
            }
            Self::DeleteMulticam { id } => {
                let before = p.multicams.len();
                p.multicams.retain(|m| m.id != id);
                if p.multicams.len() == before {
                    return Err(EditError::Invalid("there is no such multicam clip"));
                }
                Ok(Outcome::Nothing)
            }
            Self::SetAngleOffset {
                multicam,
                angle,
                offset,
            } => {
                let a = p
                    .multicams
                    .iter_mut()
                    .find(|m| m.id == multicam)
                    .ok_or(EditError::Invalid("there is no such multicam clip"))?
                    .angles
                    .get_mut(angle)
                    .ok_or(EditError::Invalid("there is no such angle"))?;
                a.offset = offset.clamp(
                    -crate::multicam::MAX_ANGLE_OFFSET,
                    crate::multicam::MAX_ANGLE_OFFSET,
                );
                Ok(Outcome::Nothing)
            }
            Self::SwitchAngle {
                multicam,
                angle,
                at,
            } => switch_angle(p, multicam, angle, at),
            Self::AddBin { name } => {
                if p.bins.len() >= crate::project::MAX_BINS {
                    return Err(EditError::Invalid("too many bins"));
                }
                let id = p.fresh_id();
                p.bins.push(crate::Bin {
                    id,
                    name: crate::project::bin_name(&name),
                });
                Ok(Outcome::Nothing)
            }
            Self::RenameBin { id, name } => {
                let bin = p
                    .bins
                    .iter_mut()
                    .find(|b| b.id == id)
                    .ok_or(EditError::Invalid("there is no such bin"))?;
                bin.name = crate::project::bin_name(&name);
                Ok(Outcome::Nothing)
            }
            Self::DeleteBin { id } => {
                let before = p.bins.len();
                p.bins.retain(|b| b.id != id);
                if p.bins.len() == before {
                    return Err(EditError::Invalid("there is no such bin"));
                }
                for m in &mut p.media {
                    if m.bin == Some(id) {
                        m.bin = None;
                    }
                }
                Ok(Outcome::Nothing)
            }
            Self::SetMediaBin { media, bin } => {
                if bin.is_some_and(|b| !p.bins.iter().any(|x| x.id == b)) {
                    return Err(EditError::Invalid("there is no such bin"));
                }
                let item = p
                    .media
                    .iter_mut()
                    .find(|m| m.id == media)
                    .ok_or(EditError::NoSuchMedia)?;
                item.bin = bin;
                Ok(Outcome::Media(media))
            }
            Self::AddSmartBin {
                name,
                contains,
                kind,
            } => {
                if p.smart_bins.len() >= crate::project::MAX_BINS {
                    return Err(EditError::Invalid("too many bins"));
                }
                let id = p.fresh_id();
                p.smart_bins.push(crate::SmartBin {
                    id,
                    name: crate::project::bin_name(&name),
                    contains: contains
                        .chars()
                        .take(crate::project::MAX_BIN_NAME)
                        .collect(),
                    kind,
                });
                Ok(Outcome::Nothing)
            }
            Self::DeleteSmartBin { id } => {
                let before = p.smart_bins.len();
                p.smart_bins.retain(|b| b.id != id);
                if p.smart_bins.len() == before {
                    return Err(EditError::Invalid("there is no such bin"));
                }
                Ok(Outcome::Nothing)
            }
            Self::GrabStill {
                name,
                frame,
                grade,
                nodes,
                png,
            } => {
                if p.stills.len() >= crate::MAX_STILLS {
                    return Err(EditError::Invalid("the gallery is full"));
                }
                if png.len() > crate::project::MAX_STILL_PNG
                    || crate::base64::decode(&png).is_none()
                {
                    return Err(EditError::Invalid("the still's picture"));
                }
                if nodes.len() >= MAX_NODES {
                    return Err(EditError::Invalid("too many nodes"));
                }
                let id = p.fresh_id();
                p.stills.push(crate::Still {
                    id,
                    name,
                    frame,
                    grade: grade.sanitized(),
                    nodes: nodes.into_iter().map(ColorGrade::sanitized).collect(),
                    png,
                });
                Ok(Outcome::Nothing)
            }
            Self::DeleteStill { id } => {
                let before = p.stills.len();
                p.stills.retain(|s| s.id != id);
                if p.stills.len() == before {
                    return Err(EditError::Invalid("there is no such still"));
                }
                Ok(Outcome::Nothing)
            }
            Self::ApplyStill { still, clip } => {
                let s = p
                    .stills
                    .iter()
                    .find(|s| s.id == still)
                    .ok_or(EditError::Invalid("there is no such still"))?
                    .clone();
                // Like a grade, a still can be applied on a locked track.
                let (ti, ci) = p.locate(clip).ok_or(EditError::NoSuchClip)?;
                let c = &mut p.tracks[ti].clips[ci];
                (c.grade, c.nodes) = (s.grade, s.nodes);
                Ok(Outcome::Clip(clip))
            }
            Self::SetTitle { media, title } => {
                let item = p
                    .media
                    .iter_mut()
                    .find(|m| m.id == media)
                    .ok_or(EditError::NoSuchMedia)?;
                if !matches!(item.source, MediaSource::Title(_)) {
                    return Err(EditError::Invalid("that media is not a title"));
                }
                item.source = MediaSource::Title(title.sanitized());
                Ok(Outcome::Nothing)
            }
            Self::SetClipLocked { clip, locked } => {
                let (ti, ci) = p.locate(clip).ok_or(EditError::NoSuchClip)?;
                p.tracks[ti].clips[ci].locked = locked;
                Ok(Outcome::Clip(clip))
            }
            Self::SetClipColor { clip, color } => {
                if color.is_some_and(|c| usize::from(c) >= crate::project::CLIP_COLORS.len()) {
                    return Err(EditError::Invalid("there is no such clip colour"));
                }
                let (ti, ci) = p.locate(clip).ok_or(EditError::NoSuchClip)?;
                p.tracks[ti].clips[ci].color = color;
                Ok(Outcome::Clip(clip))
            }
            Self::RenameClip { clip, name } => {
                let name: String = name
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(crate::project::MAX_BIN_NAME)
                    .collect();
                if name.trim().is_empty() {
                    return Err(EditError::Invalid("a clip needs a name"));
                }
                let (ti, ci) = p.locate(clip).ok_or(EditError::NoSuchClip)?;
                p.tracks[ti].clips[ci].name = name.trim().to_string();
                Ok(Outcome::Clip(clip))
            }
            Self::RenderInPlace { clip, media } => {
                let (ti, ci) = editable(p, clip)?;
                let item = p.media(media).ok_or(EditError::NoSuchMedia)?;
                if !item.source.fits(p.tracks[ti].kind) {
                    return Err(EditError::WrongTrackKind);
                }
                let c = &p.tracks[ti].clips[ci];
                // A render can come out a frame or two short from rounding.
                if item
                    .length
                    .is_some_and(|l| c.duration > l.saturating_add(2))
                {
                    return Err(EditError::OutOfRange);
                }
                let c = &mut p.tracks[ti].clips[ci];
                c.media = media;
                c.source_in = 0;
                c.speed = 1.0;
                c.reverse = false;
                c.freeze = false;
                c.grade = ColorGrade::default();
                c.nodes.clear();
                c.effects.clear();
                c.keyframes.clear();
                c.transform = crate::Transform::default();
                Ok(Outcome::Clip(clip))
            }
            Self::SetReverse { clip, reverse } => {
                let (ti, ci) = editable(p, clip)?;
                p.tracks[ti].clips[ci].reverse = reverse;
                Ok(Outcome::Clip(clip))
            }
            Self::SetFreeze { clip, frame } => set_freeze(p, clip, frame),
            Self::NewTimeline { name } => new_timeline(p, name),
            Self::ImportTimeline {
                name,
                tracks,
                markers,
            } => import_timeline(p, name, tracks, markers),
            Self::SetMediaTimecode { media, timecode } => {
                let item = p
                    .media
                    .iter_mut()
                    .find(|m| m.id == media)
                    .ok_or(EditError::NoSuchMedia)?;
                item.timecode = timecode;
                Ok(Outcome::Nothing)
            }
            Self::RelinkMedia { media, path } => {
                let path = relink_path(&path)?;
                let item = p
                    .media
                    .iter_mut()
                    .find(|m| m.id == media)
                    .ok_or(EditError::NoSuchMedia)?;
                match &mut item.source {
                    MediaSource::Video { path: at }
                    | MediaSource::Audio { path: at }
                    | MediaSource::Image { path: at } => *at = path,
                    _ => {
                        return Err(EditError::Invalid(
                            "that media is made by the program and has no file to relink",
                        ))
                    }
                }
                Ok(Outcome::Nothing)
            }
            Self::AlignClips { clips } => align_clips(p, &clips),
            Self::PlaceClips { starts } => place_clips(p, &starts),
            Self::CreateCompound { clips, name } => create_compound(p, &clips, name),
            Self::CreateEffectsClip { start, name } => create_effects_clip(p, start, name),
            Self::DecomposeCompound { clip } => decompose_compound(p, clip),
            Self::SwitchTimeline { id } => switch_timeline(p, id),
            Self::RenameTimeline { id, name } => {
                let name = timeline_name(&name)?;
                if id == p.timeline_id {
                    p.timeline_name = name;
                } else {
                    let t = p
                        .timelines
                        .iter_mut()
                        .find(|t| t.id == id)
                        .ok_or(EditError::Invalid("there is no such timeline"))?;
                    t.name = name;
                }
                Ok(Outcome::Nothing)
            }
            Self::DeleteTimeline { id } => {
                if p.timelines.iter().any(|t| t.id == id && t.compound) {
                    return Err(EditError::Invalid(
                        "that timeline belongs to a compound clip",
                    ));
                }
                let Some(next) = p.timelines.iter().find(|t| !t.compound).map(|t| t.id) else {
                    return Err(EditError::Invalid("a project keeps at least one timeline"));
                };
                if id == p.timeline_id {
                    switch_timeline(p, next)?;
                }
                let before = p.timelines.len();
                p.timelines.retain(|t| t.id != id);
                if p.timelines.len() == before {
                    return Err(EditError::Invalid("there is no such timeline"));
                }
                Ok(Outcome::Nothing)
            }
            Self::InsertGap { track, at, frames } => {
                let ti = p
                    .tracks
                    .iter()
                    .position(|t| t.id == track)
                    .ok_or(EditError::NoSuchTrack)?;
                if p.tracks[ti].locked {
                    return Err(EditError::Locked);
                }
                let mut q = p.clone();
                cut_at(&mut q, ti, at);
                for c in q.tracks[ti].clips.iter_mut().filter(|c| c.start >= at) {
                    c.start = c
                        .start
                        .checked_add(frames)
                        .filter(|s| s.saturating_add(c.duration) <= MAX_FRAMES)
                        .ok_or(EditError::OutOfRange)?;
                }
                *p = q;
                Ok(Outcome::Nothing)
            }
            Self::SlipClip { clip, source_in } => slip_clip(p, clip, source_in),
            Self::SlideClip { clip, start } => slide_clip(p, clip, start),
            Self::SetClipTransform { clip, transform } => {
                // Like a grade, a picture's place can change on a locked track.
                let (ti, ci) = p.locate(clip).ok_or(EditError::NoSuchClip)?;
                p.tracks[ti].clips[ci].transform = transform.sanitized();
                Ok(Outcome::Nothing)
            }
            Self::SetBlend { clip, blend } => {
                let (ti, ci) = p.locate(clip).ok_or(EditError::NoSuchClip)?;
                p.tracks[ti].clips[ci].blend = blend;
                Ok(Outcome::Nothing)
            }
            Self::SetClipSpeed { clip, speed } => set_speed(p, clip, speed),
            Self::SpeedRamp { clip, speeds } => speed_ramp(p, clip, &speeds),
            Self::SetKeyframe {
                clip,
                param,
                at,
                value,
                ease,
            } => {
                // Like a grade, animation can change on a locked track.
                let (ti, ci) = p.locate(clip).ok_or(EditError::NoSuchClip)?;
                let c = &mut p.tracks[ti].clips[ci];
                if at >= c.duration {
                    return Err(EditError::Invalid("the keyframe must be inside the clip"));
                }
                if value.is_some_and(|v| !v.is_finite()) {
                    return Err(EditError::Invalid("the keyframe's value"));
                }
                let index = match c.keyframes.iter().position(|t| t.param == param) {
                    Some(i) => i,
                    None => {
                        c.keyframes.push(crate::KeyTrack {
                            param,
                            keys: Vec::new(),
                        });
                        c.keyframes.len() - 1
                    }
                };
                let track = &mut c.keyframes[index];
                track.keys.retain(|k| k.at != at);
                if let Some(value) = value {
                    if track.keys.len() >= crate::MAX_KEYFRAMES {
                        return Err(EditError::Invalid("too many keyframes"));
                    }
                    track.keys.push(crate::Keyframe { at, value, ease });
                }
                *track = track.clone().sanitized();
                c.keyframes.retain(|t| !t.keys.is_empty());
                Ok(Outcome::Clip(clip))
            }
            Self::SetTrackFx { track, fx } => {
                // Like the mix, a locked track's sound can still be shaped.
                let t = p.track_mut(track).ok_or(EditError::NoSuchTrack)?;
                t.fx = fx.sanitized();
                Ok(Outcome::Nothing)
            }
            Self::AddEffect { clip, kind } => {
                // Like a grade, effects can change on a locked track.
                let (ti, ci) = p.locate(clip).ok_or(EditError::NoSuchClip)?;
                let c = &mut p.tracks[ti].clips[ci];
                if c.effects.len() >= crate::MAX_EFFECTS {
                    return Err(EditError::Invalid("a clip has at most 16 effects"));
                }
                c.effects.push(crate::Effect::new(kind));
                Ok(Outcome::Clip(clip))
            }
            Self::SetEffect {
                clip,
                index,
                effect,
            } => {
                let (ti, ci) = p.locate(clip).ok_or(EditError::NoSuchClip)?;
                let slot = p.tracks[ti].clips[ci]
                    .effects
                    .get_mut(index)
                    .ok_or(EditError::Invalid("there is no such effect"))?;
                *slot = effect.sanitized();
                Ok(Outcome::Clip(clip))
            }
            Self::RemoveEffect { clip, index } => {
                let (ti, ci) = p.locate(clip).ok_or(EditError::NoSuchClip)?;
                let effects = &mut p.tracks[ti].clips[ci].effects;
                if index >= effects.len() {
                    return Err(EditError::Invalid("there is no such effect"));
                }
                effects.remove(index);
                Ok(Outcome::Clip(clip))
            }
            Self::SetEffects { clip, effects } => {
                let (ti, ci) = p.locate(clip).ok_or(EditError::NoSuchClip)?;
                if effects.len() > crate::MAX_EFFECTS {
                    return Err(EditError::Invalid("a clip has at most 16 effects"));
                }
                p.tracks[ti].clips[ci].effects = effects;
                Ok(Outcome::Clip(clip))
            }
            Self::ClearKeyframes { clip, param } => {
                let (ti, ci) = p.locate(clip).ok_or(EditError::NoSuchClip)?;
                p.tracks[ti].clips[ci]
                    .keyframes
                    .retain(|t| t.param != param);
                Ok(Outcome::Clip(clip))
            }
            Self::SetClipAudio { clip, audio } => {
                // Like a grade, a clip's sound can change on a locked track.
                let (ti, ci) = p.locate(clip).ok_or(EditError::NoSuchClip)?;
                // A picture whose sound has a linked clip of its own passes the settings on to
                // it, since that clip is the one heard.
                let mut targets = vec![(ti, ci)];
                if !p.tracks[ti].clips[ci].sound {
                    targets.extend(
                        p.linked_to(clip)
                            .into_iter()
                            .filter(|id| p.clip(*id).is_some_and(|(_, c)| c.sound))
                            .filter_map(|id| p.locate(id)),
                    );
                }
                for (ti, ci) in targets {
                    let c = &mut p.tracks[ti].clips[ci];
                    c.audio = audio.sanitized(c.duration);
                }
                Ok(Outcome::Nothing)
            }
            Self::SetTrackMix {
                track,
                gain_db,
                pan,
            } => {
                if !gain_db.is_finite() || !pan.is_finite() {
                    return Err(EditError::Invalid("the track mix"));
                }
                let t = p.track_mut(track).ok_or(EditError::NoSuchTrack)?;
                t.gain_db = gain_db.clamp(crate::MIN_GAIN_DB, crate::MAX_GAIN_DB);
                t.pan = pan.clamp(-1.0, 1.0);
                Ok(Outcome::Nothing)
            }
            Self::PasteClip {
                track,
                media,
                name,
                start,
                source_in,
                duration,
                opacity,
                grade,
                enabled,
                audio,
                speed,
                transform,
                blend,
                sound,
                nodes,
                reverse,
                freeze,
            } => {
                if !opacity.is_finite() {
                    return Err(EditError::Invalid("the opacity"));
                }
                if nodes.len() >= MAX_NODES {
                    return Err(EditError::Invalid("too many nodes"));
                }
                if !opacity.is_finite() {
                    return Err(EditError::Invalid("the opacity"));
                }
                let item = p.media(media).ok_or(EditError::NoSuchMedia)?;
                if item.length.is_some_and(|l| {
                    let span = if freeze {
                        1
                    } else {
                        (duration as f64 * f64::from(speed)).ceil() as u64
                    };
                    source_in.saturating_add(span) > l
                }) {
                    return Err(EditError::OutOfRange);
                }
                let target = p.track(track).ok_or(EditError::NoSuchTrack)?;
                if target.locked {
                    return Err(EditError::Locked);
                }
                if !item.source.fits(target.kind) {
                    return Err(EditError::WrongTrackKind);
                }
                if duration == 0 || source_in > MAX_FRAMES {
                    return Err(EditError::Invalid("a clip needs at least one frame"));
                }
                if !p.can_place(track, start, duration, None) {
                    return Err(
                        if start.checked_add(duration).is_none_or(|e| e > MAX_FRAMES) {
                            EditError::OutOfRange
                        } else {
                            EditError::Overlap
                        },
                    );
                }
                let id = ClipId(p.fresh_id());
                let clip = Clip {
                    id,
                    media,
                    name,
                    start,
                    source_in,
                    duration,
                    opacity: opacity.clamp(0.0, 1.0),
                    grade: grade.sanitized(),
                    enabled,
                    audio: audio.sanitized(duration),
                    speed: if speed.is_finite() {
                        speed.clamp(crate::MIN_SPEED, crate::MAX_SPEED)
                    } else {
                        1.0
                    },
                    transform: transform.sanitized(),
                    blend,
                    link: None,
                    sound,
                    nodes: nodes.into_iter().map(ColorGrade::sanitized).collect(),
                    reverse,
                    freeze,
                    transition: None,
                    locked: false,
                    color: None,
                    keyframes: Vec::new(),
                    effects: Vec::new(),
                };
                if let Some(t) = p.track_mut(track) {
                    t.clips.push(clip);
                    sort_clips(t);
                }
                Ok(Outcome::Clip(id))
            }
            Self::SetMarker { frame, name, color } => {
                if frame > MAX_FRAMES || color >= MARKER_COLORS {
                    return Err(EditError::Invalid("the marker"));
                }
                match p.markers.binary_search_by_key(&frame, |m| m.frame) {
                    Ok(i) => p.markers[i] = Marker { frame, name, color },
                    Err(i) => {
                        if p.markers.len() >= MAX_MARKERS {
                            return Err(EditError::Invalid("too many markers"));
                        }
                        p.markers.insert(i, Marker { frame, name, color });
                    }
                }
                Ok(Outcome::Nothing)
            }
            Self::DeleteMarker { frame } => {
                let i = p
                    .markers
                    .binary_search_by_key(&frame, |m| m.frame)
                    .map_err(|_| EditError::Invalid("there is no marker there"))?;
                p.markers.remove(i);
                Ok(Outcome::Nothing)
            }
            Self::ClearMarkers => {
                p.markers.clear();
                Ok(Outcome::Nothing)
            }
            Self::SetMarks { mark_in, mark_out } => {
                if [mark_in, mark_out]
                    .into_iter()
                    .flatten()
                    .any(|m| m > MAX_FRAMES)
                {
                    return Err(EditError::OutOfRange);
                }
                if let (Some(a), Some(b)) = (mark_in, mark_out) {
                    if a >= b {
                        return Err(EditError::Invalid(
                            "the in mark must be before the out mark",
                        ));
                    }
                }
                (p.mark_in, p.mark_out) = (mark_in, mark_out);
                Ok(Outcome::Nothing)
            }
        }
    }
}

fn one() -> f32 {
    1.0
}

/// The most colour nodes one clip may have.
pub const MAX_NODES: usize = 16;

fn yes() -> bool {
    true
}

/// How a video's sound is put on its audio track, matching the edit that put the picture down.
#[derive(Clone, Copy)]
enum SoundEdit {
    /// Into free space: the first unlocked audio track with room, else a new one.
    Place,
    /// Pushing later clips of the first unlocked audio track along.
    Insert,
    /// Over what is on the first unlocked audio track.
    Overwrite,
}

/// Runs `edit` (which puts a clip down) on a copy of the project and, when `with_sound` is set
/// and the clip is a video's picture, gives its sound a linked clip on an audio track. The
/// project changes only if both work.
fn with_linked_sound(
    p: &mut Project,
    with_sound: bool,
    how: SoundEdit,
    edit: impl FnOnce(&mut Project) -> Result<Outcome, EditError>,
) -> Result<Outcome, EditError> {
    if !with_sound {
        return edit(p);
    }
    let mut q = p.clone();
    let out = edit(&mut q)?;
    if let Outcome::Clip(id) = out {
        add_sound(&mut q, id, how)?;
    }
    *p = q;
    Ok(out)
}

/// Gives the sound of video clip `clip` a clip of its own on an audio track, linked to it, and
/// silences the picture's own sound.
fn add_sound(p: &mut Project, clip: ClipId, how: SoundEdit) -> Result<(), EditError> {
    let (ti, ci) = p.locate(clip).ok_or(EditError::NoSuchClip)?;
    let picture = p.tracks[ti].clips[ci].clone();
    let is_video = p
        .media(picture.media)
        .is_some_and(|m| matches!(m.source, MediaSource::Video { .. }));
    if p.tracks[ti].kind != TrackKind::Video || !is_video {
        return Ok(());
    }
    let (start, duration) = (picture.start, picture.duration);
    let free = |t: &Track| {
        t.kind == TrackKind::Audio
            && !t.locked
            && match how {
                SoundEdit::Place => p.can_place(t.id, start, duration, None),
                SoundEdit::Insert | SoundEdit::Overwrite => true,
            }
    };
    let ati = match p.tracks.iter().position(free) {
        Some(i) => i,
        None => {
            let Outcome::Track(id) = (Command::AddTrack {
                kind: TrackKind::Audio,
            })
            .apply(p)?
            else {
                return Err(EditError::Invalid("no audio track"));
            };
            p.tracks
                .iter()
                .position(|t| t.id == id)
                .ok_or(EditError::NoSuchTrack)?
        }
    };
    let link = p.fresh_id();
    let mut sound = picture.clone();
    sound.id = ClipId(p.fresh_id());
    sound.link = Some(link);
    sound.sound = true;
    sound.grade = ColorGrade::default();
    sound.nodes.clear();
    sound.transform = Transform::default();
    sound.opacity = 1.0;
    sound.blend = BlendMode::Normal;
    match how {
        SoundEdit::Place => {}
        SoundEdit::Insert => {
            cut_at(p, ati, start);
            for c in p.tracks[ati].clips.iter_mut().filter(|c| c.start >= start) {
                c.start = c
                    .start
                    .checked_add(duration)
                    .filter(|s| s.saturating_add(c.duration) <= MAX_FRAMES)
                    .ok_or(EditError::OutOfRange)?;
            }
        }
        SoundEdit::Overwrite => {
            let end = start.saturating_add(duration);
            cut_at(p, ati, start);
            cut_at(p, ati, end);
            p.tracks[ati]
                .clips
                .retain(|c| !(c.start >= start && c.end() <= end));
        }
    }
    p.tracks[ati].clips.push(sound);
    sort_clips(&mut p.tracks[ati]);
    let c = &mut p.tracks[ti].clips[ci];
    c.link = Some(link);
    c.sound = false;
    Ok(())
}

/// Runs `edit` on clip `clip` and on every clip linked to it, on a copy of the project; the
/// project changes only if all of them work.
fn with_partners(
    p: &mut Project,
    clip: ClipId,
    edit: impl Fn(&mut Project, ClipId) -> Result<(), EditError>,
) -> Result<Outcome, EditError> {
    let partners = p.partners(clip);
    if partners.is_empty() {
        edit(p, clip)?;
        return Ok(Outcome::Nothing);
    }
    let mut q = p.clone();
    edit(&mut q, clip)?;
    for id in partners {
        edit(&mut q, id)?;
    }
    *p = q;
    Ok(Outcome::Nothing)
}

/// `at` moved by `to - from`, if that stays at or after frame 0.
fn shifted(at: u64, from: u64, to: u64) -> Option<u64> {
    let moved = i128::from(at) + i128::from(to) - i128::from(from);
    u64::try_from(moved).ok()
}

/// Moves a clip, and its linked clips by the same time on their own tracks.
fn linked_move(
    p: &mut Project,
    clip: ClipId,
    track: TrackId,
    start: u64,
) -> Result<Outcome, EditError> {
    let partners = p.partners(clip);
    if partners.is_empty() {
        return move_clip(p, clip, track, start);
    }
    let from = p.clip(clip).ok_or(EditError::NoSuchClip)?.1.start;
    let mut q = p.clone();
    let out = move_clip(&mut q, clip, track, start)?;
    for id in partners {
        let (t, c) = q.clip(id).ok_or(EditError::NoSuchClip)?;
        let (own_track, to) = (
            t.id,
            shifted(c.start, from, start).ok_or(EditError::OutOfRange)?,
        );
        move_clip(&mut q, id, own_track, to)?;
    }
    *p = q;
    Ok(out)
}

/// Trims the start or the end of a clip. A linked clip whose same edge is at the same frame is
/// trimmed with it; one whose edge is elsewhere (a J-cut or an L-cut) is left as it is.
fn linked_trim(
    p: &mut Project,
    clip: ClipId,
    start: Option<u64>,
    end: Option<u64>,
) -> Result<Outcome, EditError> {
    let (old_start, old_end) = {
        let (_, c) = p.clip(clip).ok_or(EditError::NoSuchClip)?;
        (c.start, c.end())
    };
    let run = |q: &mut Project, id: ClipId| match (start, end) {
        (Some(s), _) => trim_start(q, id, s),
        (_, Some(e)) => trim_end(q, id, e),
        _ => Ok(Outcome::Nothing),
    };
    let partners: Vec<ClipId> = p
        .partners(clip)
        .into_iter()
        .filter(|id| {
            p.clip(*id).is_some_and(|(_, c)| {
                (start.is_some() && c.start == old_start) || (end.is_some() && c.end() == old_end)
            })
        })
        .collect();
    if partners.is_empty() {
        return run(p, clip);
    }
    let mut q = p.clone();
    let out = run(&mut q, clip)?;
    for id in partners {
        run(&mut q, id)?;
    }
    *p = q;
    Ok(out)
}

/// Splits a clip, and every linked clip the cut goes through. The right halves are linked to
/// each other, the left halves stay linked as they were.
fn linked_split(p: &mut Project, clip: ClipId, at: u64) -> Result<Outcome, EditError> {
    let partners: Vec<ClipId> = p
        .partners(clip)
        .into_iter()
        .filter(|id| {
            p.clip(*id)
                .is_some_and(|(_, c)| c.start < at && at < c.end())
        })
        .collect();
    if partners.is_empty() {
        let out = split(p, clip, at)?;
        // Nothing linked was cut with it: the right half is linked to nothing, and the left half
        // stays with what it was linked to.
        if let Outcome::Split(_, right) = out {
            if let Some((ti, ci)) = p.locate(right) {
                p.tracks[ti].clips[ci].link = None;
            }
        }
        return Ok(out);
    }
    let mut q = p.clone();
    let out = split(&mut q, clip, at)?;
    let mut rights = Vec::new();
    if let Outcome::Split(_, right) = out {
        rights.push(right);
    }
    for id in partners {
        if let Outcome::Split(_, right) = split(&mut q, id, at)? {
            rights.push(right);
        }
    }
    let link = q.fresh_id();
    for id in rights {
        if let Some((ti, ci)) = q.locate(id) {
            q.tracks[ti].clips[ci].link = Some(link);
        }
    }
    *p = q;
    Ok(out)
}

/// Links the clips, and everything already linked to them, under one new link.
fn link_clips(p: &mut Project, clips: &[ClipId]) -> Result<Outcome, EditError> {
    let mut ids: Vec<ClipId> = clips.to_vec();
    ids.sort_by_key(|c| c.0);
    ids.dedup();
    if ids.len() < 2 {
        return Err(EditError::Invalid("linking needs two clips or more"));
    }
    if ids.iter().any(|id| p.clip(*id).is_none()) {
        return Err(EditError::NoSuchClip);
    }
    let old: Vec<u64> = ids
        .iter()
        .filter_map(|id| p.clip(*id).and_then(|(_, c)| c.link))
        .collect();
    let link = p.fresh_id();
    for c in p.tracks.iter_mut().flat_map(|t| t.clips.iter_mut()) {
        if ids.contains(&c.id) || c.link.is_some_and(|l| old.contains(&l)) {
            c.link = Some(link);
        }
    }
    Ok(Outcome::Nothing)
}

/// Where the clip is, if it exists and its track is not locked.
fn editable(p: &Project, clip: ClipId) -> Result<(usize, usize), EditError> {
    let (ti, ci) = p.locate(clip).ok_or(EditError::NoSuchClip)?;
    if p.tracks[ti].locked {
        return Err(EditError::Locked);
    }
    if p.tracks[ti].clips[ci].locked {
        return Err(EditError::ClipLocked);
    }
    Ok((ti, ci))
}

fn sort_clips(track: &mut Track) {
    track.clips.sort_by_key(|c| c.start);
}

/// A clip of `media` at normal speed, with no grade, effects or transition.
fn plain_clip(id: ClipId, media: MediaId, name: String, start: u64, duration: u64) -> Clip {
    Clip {
        id,
        media,
        name,
        start,
        source_in: 0,
        duration,
        opacity: 1.0,
        grade: ColorGrade::default(),
        enabled: true,
        audio: ClipAudio::default(),
        speed: 1.0,
        transform: Transform::default(),
        blend: BlendMode::Normal,
        link: None,
        sound: true,
        nodes: Vec::new(),
        reverse: false,
        freeze: false,
        transition: None,
        locked: false,
        color: None,
        keyframes: Vec::new(),
        effects: Vec::new(),
    }
}

fn add_clip(
    p: &mut Project,
    track: TrackId,
    media: MediaId,
    start: u64,
    duration: Option<u64>,
) -> Result<Outcome, EditError> {
    let item = p.media(media).ok_or(EditError::NoSuchMedia)?.clone();
    let target = p.track(track).ok_or(EditError::NoSuchTrack)?;
    if target.locked {
        return Err(EditError::Locked);
    }
    if !item.source.fits(target.kind) {
        return Err(EditError::WrongTrackKind);
    }
    let duration = duration.unwrap_or(item.default_duration);
    if item.length.is_some_and(|l| duration > l) {
        return Err(EditError::OutOfRange);
    }
    if duration == 0 {
        return Err(EditError::Invalid("a clip needs at least one frame"));
    }
    if !p.can_place(track, start, duration, None) {
        return Err(
            if start.checked_add(duration).is_none_or(|e| e > MAX_FRAMES) {
                EditError::OutOfRange
            } else {
                EditError::Overlap
            },
        );
    }
    let id = ClipId(p.fresh_id());
    let clip = plain_clip(id, media, item.name, start, duration);
    if let Some(t) = p.track_mut(track) {
        t.clips.push(clip);
        sort_clips(t);
    }
    Ok(Outcome::Clip(id))
}

/// Checks what the insert and overwrite edits share and returns the clip to put down (its id is
/// given by the caller, once the edit is sure to work).
fn new_clip(
    p: &Project,
    track: TrackId,
    media: MediaId,
    start: u64,
    duration: Option<u64>,
) -> Result<Clip, EditError> {
    let item = p.media(media).ok_or(EditError::NoSuchMedia)?.clone();
    let target = p.track(track).ok_or(EditError::NoSuchTrack)?;
    if target.locked {
        return Err(EditError::Locked);
    }
    if !item.source.fits(target.kind) {
        return Err(EditError::WrongTrackKind);
    }
    let duration = duration.unwrap_or(item.default_duration);
    if duration == 0 {
        return Err(EditError::Invalid("a clip needs at least one frame"));
    }
    if item.length.is_some_and(|l| duration > l)
        || start.checked_add(duration).is_none_or(|e| e > MAX_FRAMES)
    {
        return Err(EditError::OutOfRange);
    }
    Ok(Clip {
        id: ClipId(0),
        media,
        name: item.name,
        start,
        source_in: 0,
        duration,
        opacity: 1.0,
        grade: ColorGrade::default(),
        enabled: true,
        audio: ClipAudio::default(),
        speed: 1.0,
        transform: Transform::default(),
        blend: BlendMode::Normal,
        link: None,
        sound: true,
        nodes: Vec::new(),
        reverse: false,
        freeze: false,
        transition: None,
        locked: false,
        color: None,
        keyframes: Vec::new(),
        effects: Vec::new(),
    })
}

/// Cuts the clip of track `ti` that `at` lies strictly inside into two, and returns nothing.
fn cut_at(p: &mut Project, ti: usize, at: u64) {
    let Some(ci) = p.tracks[ti]
        .clips
        .iter()
        .position(|c| c.start < at && at < c.end())
    else {
        return;
    };
    let c = &p.tracks[ti].clips[ci];
    let left = at - c.start;
    let mut right = c.clone();
    let advance = c.source_frames(left);
    let reverse = c.reverse;
    let right_span = c.source_span().saturating_sub(advance);
    right.id = ClipId(p.fresh_id());
    right.start = at;
    right.transition = None;
    if !reverse {
        right.source_in += advance;
    }
    right.duration -= left;
    right.shift_keys(-(left as i64));
    right.audio.fade_in = 0;
    right.audio = right.audio.sanitized(right.duration);
    let track = &mut p.tracks[ti];
    if reverse {
        track.clips[ci].source_in += right_span;
    }
    track.clips[ci].duration = left;
    track.clips[ci].audio.fade_out = 0;
    track.clips[ci].audio = track.clips[ci].audio.sanitized(left);
    track.clips.insert(ci + 1, right);
}

fn insert_clip(
    p: &mut Project,
    track: TrackId,
    media: MediaId,
    start: u64,
    duration: Option<u64>,
) -> Result<Outcome, EditError> {
    let clip = new_clip(p, track, media, start, duration)?;
    place_inserting(p, track, clip)
}

/// Puts `clip` on `track`, pushing everything from its start on later.
fn place_inserting(p: &mut Project, track: TrackId, mut clip: Clip) -> Result<Outcome, EditError> {
    let start = clip.start;
    let mut q = p.clone();
    clip.id = ClipId(q.fresh_id());
    let id = clip.id;
    let ti = q
        .tracks
        .iter()
        .position(|t| t.id == track)
        .ok_or(EditError::NoSuchTrack)?;
    cut_at(&mut q, ti, start);
    for c in q.tracks[ti].clips.iter_mut().filter(|c| c.start >= start) {
        c.start = c
            .start
            .checked_add(clip.duration)
            .filter(|s| s.saturating_add(c.duration) <= MAX_FRAMES)
            .ok_or(EditError::OutOfRange)?;
    }
    clip.start = start;
    q.tracks[ti].clips.push(clip);
    sort_clips(&mut q.tracks[ti]);
    *p = q;
    Ok(Outcome::Clip(id))
}

fn overwrite_clip(
    p: &mut Project,
    track: TrackId,
    media: MediaId,
    start: u64,
    duration: Option<u64>,
) -> Result<Outcome, EditError> {
    let clip = new_clip(p, track, media, start, duration)?;
    place_over(p, track, clip)
}

/// Puts `clip` on `track`, cutting away whatever it covers.
fn place_over(p: &mut Project, track: TrackId, mut clip: Clip) -> Result<Outcome, EditError> {
    let start = clip.start;
    let mut q = p.clone();
    clip.id = ClipId(q.fresh_id());
    let (id, end) = (clip.id, clip.end());
    let ti = q
        .tracks
        .iter()
        .position(|t| t.id == track)
        .ok_or(EditError::NoSuchTrack)?;
    // A clip that spans the whole new clip is cut in two first; then every overlapped clip is
    // either inside (removed) or sticks out on one side (shortened).
    cut_at(&mut q, ti, start);
    cut_at(&mut q, ti, end);
    q.tracks[ti]
        .clips
        .retain(|c| !(c.start >= start && c.end() <= end));
    q.tracks[ti].clips.push(clip);
    sort_clips(&mut q.tracks[ti]);
    *p = q;
    Ok(Outcome::Clip(id))
}

fn fit_to_fill(
    p: &mut Project,
    track: TrackId,
    media: MediaId,
    start: u64,
    duration: u64,
) -> Result<Outcome, EditError> {
    if duration == 0 {
        return Err(EditError::Invalid("a clip needs at least one frame"));
    }
    if start.checked_add(duration).is_none_or(|e| e > MAX_FRAMES) {
        return Err(EditError::OutOfRange);
    }
    let mut clip = new_clip(p, track, media, start, Some(1))?;
    let item = p.media(media).ok_or(EditError::NoSuchMedia)?;
    // A video or sound plays all its frames; a still its usual length.
    let span = item.length.unwrap_or(item.default_duration).max(1);
    let speed = span as f32 / duration as f32;
    if !(crate::MIN_SPEED..=crate::MAX_SPEED).contains(&speed) {
        return Err(EditError::Invalid("the speed"));
    }
    clip.duration = duration;
    clip.speed = speed;
    // Rounding may ask for one frame past the end: slow down by the smallest step until not.
    for _ in 0..16 {
        if item.length.is_none_or(|l| clip.source_span() <= l) {
            break;
        }
        clip.speed = f32::from_bits(clip.speed.to_bits() - 1);
    }
    if item.length.is_some_and(|l| clip.source_span() > l) {
        return Err(EditError::OutOfRange);
    }
    place_over(p, track, clip)
}

/// The track Place on Top and an effects clip use for `media` at `start`: the first unlocked track
/// of its kind above every clip there that is free for all `duration` frames (the media's own
/// default when it is `None`), added when there is none.
fn top_free_track(
    p: &mut Project,
    media: MediaId,
    start: u64,
    duration: Option<u64>,
) -> Result<TrackId, EditError> {
    let item = p.media(media).ok_or(EditError::NoSuchMedia)?;
    let kind = item.source.track_kind();
    let duration = duration.unwrap_or(item.default_duration).max(1);
    let end = start.saturating_add(duration);
    let busy = |t: &Track| t.clips.iter().any(|c| c.start < end && c.end() > start);
    let above = p
        .tracks
        .iter()
        .rposition(|t| t.kind == kind && busy(t))
        .map_or(0, |i| i + 1);
    let free = p
        .tracks
        .iter()
        .skip(above)
        .find(|t| t.kind == kind && !t.locked && !busy(t))
        .map(|t| t.id);
    match free {
        Some(id) => Ok(id),
        None => match (Command::AddTrack { kind }).apply(p)? {
            Outcome::Track(id) => Ok(id),
            _ => Err(EditError::NoSuchTrack),
        },
    }
}

fn replace_clip(p: &mut Project, clip: ClipId, media: MediaId) -> Result<Outcome, EditError> {
    let (ti, ci) = editable(p, clip)?;
    let item = p.media(media).ok_or(EditError::NoSuchMedia)?.clone();
    if !item.source.fits(p.tracks[ti].kind) {
        return Err(EditError::WrongTrackKind);
    }
    let c = &mut p.tracks[ti].clips[ci];
    if item.length.is_some_and(|l| c.source_span() > l) {
        return Err(EditError::OutOfRange);
    }
    c.media = media;
    c.name = item.name;
    c.source_in = 0;
    Ok(Outcome::Clip(clip))
}

fn roll_trim(p: &mut Project, left: ClipId, right: ClipId, to: u64) -> Result<Outcome, EditError> {
    let (lt, li) = editable(p, left)?;
    let (rt, ri) = editable(p, right)?;
    if lt != rt {
        return Err(EditError::Invalid("the clips are not on the same track"));
    }
    let (l, r) = (&p.tracks[lt].clips[li], &p.tracks[rt].clips[ri]);
    if l.end() != r.start {
        return Err(EditError::Invalid("the clips are not next to each other"));
    }
    if to <= l.start || to >= r.end() {
        return Err(EditError::Invalid("a clip needs at least one frame"));
    }
    let still = |c: &Clip| p.media(c.media).is_none_or(|m| m.length.is_none());
    // Left clip: its end moves, so it must still have footage to show.
    let new_left_duration = to - l.start;
    let left_length = p.media(l.media).and_then(|m| m.length);
    if left_length.is_some_and(|len| {
        l.source_in
            .saturating_add(l.source_frames(new_left_duration))
            > len
    }) {
        return Err(EditError::OutOfRange);
    }
    // Right clip: its start moves, so its first frame moves with it.
    let new_right_source_in = if to >= r.start {
        r.source_in + r.source_frames(to - r.start)
    } else {
        let back = r.source_frames(r.start - to);
        if still(r) {
            r.source_in.saturating_sub(back)
        } else {
            r.source_in.checked_sub(back).ok_or(EditError::OutOfRange)?
        }
    };
    let right_end = r.end();
    let (rc, lc) = (ri, li);
    let c = &mut p.tracks[rt].clips[rc];
    c.shift_keys(c.start as i64 - to as i64);
    (c.start, c.source_in, c.duration) = (to, new_right_source_in, right_end - to);
    c.audio = c.audio.sanitized(c.duration);
    let c = &mut p.tracks[lt].clips[lc];
    c.duration = new_left_duration;
    c.audio = c.audio.sanitized(c.duration);
    Ok(Outcome::Clip(left))
}

fn create_compound(p: &mut Project, ids: &[ClipId], name: String) -> Result<Outcome, EditError> {
    let name = timeline_name(&name)?;
    if ids.is_empty() {
        return Err(EditError::Invalid("select clips first"));
    }
    if p.timelines.len() + 1 >= crate::MAX_TIMELINES {
        return Err(EditError::Invalid("too many timelines"));
    }
    let mut q = p.clone();
    // The clips by track, in the order of the tracks.
    let mut by_track: Vec<(usize, Vec<Clip>)> = Vec::new();
    for id in ids {
        let (ti, ci) = editable(&q, *id)?;
        let clip = q.tracks[ti].clips[ci].clone();
        match by_track.iter_mut().find(|(t, _)| *t == ti) {
            Some((_, clips)) => {
                if !clips.iter().any(|c| c.id == clip.id) {
                    clips.push(clip);
                }
            }
            None => by_track.push((ti, vec![clip])),
        }
    }
    by_track.sort_by_key(|(ti, _)| *ti);
    let start = by_track
        .iter()
        .flat_map(|(_, c)| c.iter().map(|c| c.start))
        .min()
        .unwrap_or(0);
    let end = by_track
        .iter()
        .flat_map(|(_, c)| c.iter().map(Clip::end))
        .max()
        .unwrap_or(0);
    let base = by_track
        .iter()
        .find(|(ti, _)| q.tracks[*ti].kind == TrackKind::Video)
        .map(|(ti, _)| q.tracks[*ti].id)
        .ok_or(EditError::Invalid("a compound clip needs a video clip"))?;
    let mut nested = Vec::new();
    for (ti, mut clips) in by_track {
        let gone: Vec<ClipId> = clips.iter().map(|c| c.id).collect();
        q.tracks[ti].clips.retain(|c| !gone.contains(&c.id));
        for c in &mut clips {
            c.start -= start;
        }
        clips.sort_by_key(|c| c.start);
        let t = &q.tracks[ti];
        let (name, kind) = (t.name.clone(), t.kind);
        nested.push(Track {
            id: TrackId(q.fresh_id()),
            name,
            kind,
            clips,
            muted: false,
            locked: false,
            solo: false,
            picture: true,
            gain_db: 0.0,
            pan: 0.0,
            fx: Default::default(),
        });
    }
    let timeline = q.fresh_id();
    q.timelines.push(crate::Timeline {
        id: timeline,
        name: name.clone(),
        tracks: nested,
        markers: Vec::new(),
        mark_in: None,
        mark_out: None,
        compound: true,
    });
    let media = MediaId(q.fresh_id());
    let length = end - start;
    q.media.push(MediaItem {
        id: media,
        name,
        source: MediaSource::Compound { timeline },
        length: Some(length),
        default_duration: length,
        bin: None,
        timecode: None,
    });
    let out = add_clip(&mut q, base, media, start, Some(length))?;
    *p = q;
    Ok(out)
}

/// Creates an effects clip: a transparent clip whose grade and effects apply to the composite
/// below. The clip is placed at `start` on a free video track above the clips there, and lasts
/// for the current timeline's duration (or five seconds if the timeline is empty).
fn create_effects_clip(p: &mut Project, start: u64, name: String) -> Result<Outcome, EditError> {
    let name = timeline_name(&name)?;
    // The effects clip has no media timeline of its own, so it does not add a timeline.
    let mut q = p.clone();
    let duration = q.duration().max(start).saturating_sub(start).max(
        q.tracks
            .iter()
            .flat_map(|t| t.clips.iter().filter(|c| c.start >= start))
            .map(|c| c.end().saturating_sub(start))
            .max()
            .unwrap_or(0),
    );
    let duration = duration.max(q.fps.nominal().max(1) * 5);
    let media = MediaId(q.fresh_id());
    q.media.push(MediaItem {
        id: media,
        name,
        source: MediaSource::EffectsClip,
        length: None,
        default_duration: 1,
        bin: None,
        timecode: None,
    });
    let track = top_free_track(&mut q, media, start, Some(duration))?;
    let out = add_clip(&mut q, track, media, start, Some(duration))?;
    *p = q;
    Ok(out)
}

fn decompose_compound(p: &mut Project, clip: ClipId) -> Result<Outcome, EditError> {
    let (ti, ci) = editable(p, clip)?;
    let outer = p.tracks[ti].clips[ci].clone();
    let Some(MediaSource::Compound { timeline }) = p.media(outer.media).map(|m| m.source.clone())
    else {
        return Err(EditError::Invalid("that is not a compound clip"));
    };
    if outer.speed != 1.0 || outer.reverse || outer.freeze {
        return Err(EditError::Invalid(
            "play the compound clip at normal speed, forwards, first",
        ));
    }
    let nested = p
        .timeline_tracks(timeline)
        .ok_or(EditError::Invalid("a compound clip has no timeline"))?
        .to_vec();
    let mut q = p.clone();
    q.tracks[ti].clips.remove(ci);
    let (first, last) = (outer.source_in, outer.source_in + outer.duration);
    let mut links: Vec<(u64, u64)> = Vec::new();
    // Nested tracks of each kind go on the outer tracks of that kind in order: the video ones
    // from the compound clip's own track up, the audio ones from the first audio track on.
    let next_of_kind = |q: &mut Project, kind: TrackKind, n: usize| -> Result<TrackId, EditError> {
        let from = if kind == TrackKind::Video { ti } else { 0 };
        let found = q
            .tracks
            .iter()
            .enumerate()
            .filter(|(i, t)| t.kind == kind && *i >= from)
            .nth(n)
            .map(|(_, t)| t.id);
        match found {
            Some(id) => Ok(id),
            None => match (Command::AddTrack { kind }).apply(q)? {
                Outcome::Track(id) => Ok(id),
                _ => Err(EditError::NoSuchTrack),
            },
        }
    };
    let mut used = [0usize; 2];
    for track in &nested {
        let k = usize::from(track.kind == TrackKind::Audio);
        let target = next_of_kind(&mut q, track.kind, used[k])?;
        used[k] += 1;
        for c in &track.clips {
            let (a, b) = (c.start.max(first), c.end().min(last));
            if a >= b {
                continue;
            }
            let mut n = c.clone();
            n.source_in = source_in_after_trim(&q, c, a, b - a)?;
            n.duration = b - a;
            n.start = outer.start + (a - first);
            n.id = ClipId(q.fresh_id());
            n.audio = n.audio.sanitized(n.duration);
            if let Some(old) = c.link {
                n.link = Some(match links.iter().find(|(o, _)| *o == old) {
                    Some((_, new)) => *new,
                    None => {
                        let new = q.fresh_id();
                        links.push((old, new));
                        new
                    }
                });
            }
            if !q.can_place(target, n.start, n.duration, None) {
                return Err(EditError::Overlap);
            }
            if let Some(t) = q.track_mut(target) {
                t.clips.push(n);
                sort_clips(t);
            }
        }
    }
    *p = q;
    Ok(Outcome::Nothing)
}

/// The longest path a media item keeps.
const MAX_PATH_CHARS: usize = 4096;

/// A path to relink media to: the one given, trimmed, when it is a path this program can keep.
fn relink_path(path: &str) -> Result<String, EditError> {
    let path = path.trim();
    if path.is_empty() {
        return Err(EditError::Invalid("the file to relink to"));
    }
    if path.chars().count() > MAX_PATH_CHARS {
        return Err(EditError::Invalid("the file to relink to"));
    }
    if path.contains(['\n', '\r', '\0']) {
        return Err(EditError::Invalid("the file to relink to"));
    }
    Ok(path.to_string())
}

/// A timeline name: trimmed, not empty, at most 200 characters.
fn timeline_name(name: &str) -> Result<String, EditError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 200 {
        return Err(EditError::Invalid("the timeline name"));
    }
    Ok(name.to_string())
}

fn new_timeline(p: &mut Project, name: String) -> Result<Outcome, EditError> {
    let name = timeline_name(&name)?;
    if p.timelines.len() + 1 >= crate::MAX_TIMELINES {
        return Err(EditError::Invalid("too many timelines"));
    }
    // The new timeline has the tracks a new project starts with.
    let template = Project::new("");
    let mut fresh = crate::Timeline {
        id: p.fresh_id(),
        name,
        tracks: Vec::new(),
        markers: Vec::new(),
        mark_in: None,
        mark_out: None,
        compound: false,
    };
    for t in template.tracks {
        fresh.tracks.push(Track {
            id: TrackId(p.fresh_id()),
            ..t
        });
    }
    p.swap_timeline(&mut fresh);
    p.timelines.push(fresh);
    Ok(Outcome::Nothing)
}

/// See [`Command::ImportTimeline`]. The timeline is built on a copy, so a refusal (too many
/// timelines, a bad name) leaves the project as it was.
fn import_timeline(
    p: &mut Project,
    name: String,
    tracks: Vec<crate::ImportedTrack>,
    markers: Vec<Marker>,
) -> Result<Outcome, EditError> {
    let mut q = p.clone();
    new_timeline(&mut q, name)?;
    // The n-th imported track of a kind goes on the n-th track of that kind.
    let mut used = [0usize; 2];
    for imported in tracks {
        let slot = usize::from(imported.kind == TrackKind::Audio);
        let n = used[slot];
        used[slot] += 1;
        let existing = q
            .tracks
            .iter()
            .filter(|t| t.kind == imported.kind)
            .nth(n)
            .map(|t| t.id);
        let track = match existing {
            Some(id) => id,
            None => match (Command::AddTrack {
                kind: imported.kind,
            })
            .apply(&mut q)
            {
                Ok(Outcome::Track(id)) => id,
                // Past the most tracks a timeline may have: the rest is left out.
                _ => break,
            },
        };
        // The clips are taken in time order, and one fits when it starts at or after the end of
        // the one kept before it: a sort for the track, rather than a search of the track and a
        // sort for every clip, which a file of many clips made slow enough to hang the window.
        // The track is the new timeline's, so it starts empty (anything on it is kept before).
        let mut clips = imported.clips;
        clips.sort_by_key(|c| c.start);
        let mut end = q
            .track(track)
            .and_then(|t| t.clips.iter().map(Clip::end).max())
            .unwrap_or(0);
        let mut placed: Vec<Clip> = Vec::with_capacity(clips.len());
        for c in clips {
            let Some(item) = q.media(c.media) else {
                continue;
            };
            let speed = if c.speed.is_finite()
                && (crate::MIN_SPEED..=crate::MAX_SPEED).contains(&c.speed)
            {
                c.speed
            } else {
                1.0
            };
            let mut clip = plain_clip(ClipId(0), c.media, c.name, c.start, c.duration);
            (clip.source_in, clip.speed) = (c.source_in, speed);
            (clip.reverse, clip.freeze, clip.enabled) = (c.reverse, c.freeze, c.enabled);
            let fits = item.source.fits(imported.kind)
                && item
                    .length
                    .is_none_or(|l| c.source_in.saturating_add(clip.source_span()) <= l);
            let clip_end = clip.start.checked_add(clip.duration);
            let free = clip.duration > 0
                && clip.start >= end
                && clip_end.is_some_and(|e| e <= MAX_FRAMES);
            if !fits || !free {
                continue;
            }
            end = clip_end.unwrap_or(end);
            clip.transition = c
                .transition
                .filter(|t| (2..=crate::MAX_TRANSITION).contains(&t.frames));
            clip.id = ClipId(q.fresh_id());
            placed.push(clip);
        }
        if let Some(t) = q.track_mut(track) {
            t.clips.extend(placed);
            sort_clips(t);
        }
        if imported.muted {
            if let Some(t) = q.track_mut(track) {
                t.muted = true;
            }
        }
    }
    link_imported_sound(&mut q);
    let mut markers: Vec<Marker> = markers
        .into_iter()
        .filter(|m| m.frame < MAX_FRAMES)
        .map(|m| Marker {
            color: m.color % crate::MARKER_COLORS,
            ..m
        })
        .collect();
    markers.sort_by_key(|m| m.frame);
    markers.dedup_by_key(|m| m.frame);
    markers.truncate(MAX_MARKERS);
    q.markers = markers;
    *p = q;
    Ok(Outcome::Nothing)
}

/// Links each picture of an imported timeline to the sound clips of the same file at the same
/// place (start, length and in point), as the timeline links a video and its sound: other
/// editors keep a video's sound on a track of its own, and unlinked the picture would play its
/// sound too, so it was heard twice.
fn link_imported_sound(q: &mut Project) {
    use std::collections::HashMap;
    type Place = (MediaId, u64, u64, u64);
    let place = |c: &Clip| -> Place { (c.media, c.start, c.duration, c.source_in) };
    // The sound clips by place, then each picture takes those at its place: one pass over the
    // clips each way, however many there are.
    let mut sounds: HashMap<Place, Vec<ClipId>> = HashMap::new();
    for c in q
        .tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Audio)
        .flat_map(|t| &t.clips)
        .filter(|c| c.link.is_none())
    {
        sounds.entry(place(c)).or_default().push(c.id);
    }
    let pictures: Vec<(ClipId, Place)> = q
        .tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Video)
        .flat_map(|t| &t.clips)
        .filter(|c| c.link.is_none())
        .map(|c| (c.id, place(c)))
        .collect();
    let mut links: HashMap<ClipId, u64> = HashMap::new();
    for (picture, at) in pictures {
        let Some(partners) = sounds.remove(&at) else {
            continue;
        };
        let link = q.fresh_id();
        links.insert(picture, link);
        links.extend(partners.into_iter().map(|id| (id, link)));
    }
    for c in q.tracks.iter_mut().flat_map(|t| t.clips.iter_mut()) {
        if let Some(link) = links.get(&c.id) {
            c.link = Some(*link);
        }
    }
}

fn switch_timeline(p: &mut Project, id: u64) -> Result<Outcome, EditError> {
    if id == p.timeline_id {
        return Ok(Outcome::Nothing);
    }
    if p.timelines.iter().any(|t| t.id == id && t.compound) {
        return Err(EditError::Invalid(
            "that timeline belongs to a compound clip",
        ));
    }
    let i = p
        .timelines
        .iter()
        .position(|t| t.id == id)
        .ok_or(EditError::Invalid("there is no such timeline"))?;
    let mut other = p.timelines.remove(i);
    p.swap_timeline(&mut other);
    p.timelines.insert(i, other);
    Ok(Outcome::Nothing)
}

fn set_freeze(p: &mut Project, clip: ClipId, frame: Option<u64>) -> Result<Outcome, EditError> {
    let (ti, ci) = editable(p, clip)?;
    let length = p.media(p.tracks[ti].clips[ci].media).and_then(|m| m.length);
    let c = &mut p.tracks[ti].clips[ci];
    match frame {
        Some(frame) => {
            if length.is_some_and(|l| frame >= l) {
                return Err(EditError::OutOfRange);
            }
            (c.freeze, c.source_in) = (true, frame);
        }
        None => {
            c.freeze = false;
            // Played again from the held frame, as far as the footage lasts.
            if let Some(l) = length {
                let span = c.source_span();
                if span > l {
                    c.freeze = true;
                    return Err(EditError::OutOfRange);
                }
                c.source_in = c.source_in.min(l - span);
            }
        }
    }
    Ok(Outcome::Clip(clip))
}

fn slip_clip(p: &mut Project, clip: ClipId, source_in: u64) -> Result<Outcome, EditError> {
    let (ti, ci) = editable(p, clip)?;
    let c = &p.tracks[ti].clips[ci];
    let length = p.media(c.media).and_then(|m| m.length);
    if length.is_some_and(|l| source_in.checked_add(c.source_span()).is_none_or(|e| e > l)) {
        return Err(EditError::OutOfRange);
    }
    p.tracks[ti].clips[ci].source_in = source_in;
    Ok(Outcome::Clip(clip))
}

fn slide_clip(p: &mut Project, clip: ClipId, start: u64) -> Result<Outcome, EditError> {
    let (ti, ci) = editable(p, clip)?;
    let c = p.tracks[ti].clips[ci].clone();
    let end = start
        .checked_add(c.duration)
        .filter(|e| *e <= MAX_FRAMES)
        .ok_or(EditError::OutOfRange)?;
    let mut q = p.clone();
    let track = &mut q.tracks[ti];
    // The clip that ends where this one starts, and the one that starts where it ends.
    let left = track
        .clips
        .iter()
        .position(|o| o.end() == c.start && o.id != clip);
    let right = track
        .clips
        .iter()
        .position(|o| o.start == c.end() && o.id != clip);
    if let Some(li) = left {
        let l = &track.clips[li];
        if start <= l.start {
            return Err(EditError::Invalid("a clip needs at least one frame"));
        }
        let duration = start - l.start;
        let length = p.media(l.media).and_then(|m| m.length);
        if length.is_some_and(|len| l.source_in.saturating_add(l.source_frames(duration)) > len) {
            return Err(EditError::OutOfRange);
        }
        let l = &mut track.clips[li];
        l.duration = duration;
        l.audio = l.audio.sanitized(duration);
    }
    if let Some(ri) = right {
        let r = &track.clips[ri];
        let r_end = r.end();
        if end >= r_end {
            return Err(EditError::Invalid("a clip needs at least one frame"));
        }
        let still = p.media(r.media).is_none_or(|m| m.length.is_none());
        let source_in = if end >= r.start {
            r.source_in + r.source_frames(end - r.start)
        } else {
            let back = r.source_frames(r.start - end);
            if still {
                r.source_in.saturating_sub(back)
            } else {
                r.source_in.checked_sub(back).ok_or(EditError::OutOfRange)?
            }
        };
        let r = &mut track.clips[ri];
        (r.start, r.source_in, r.duration) = (end, source_in, r_end - end);
        r.audio = r.audio.sanitized(r.duration);
    }
    let me = track
        .clips
        .iter_mut()
        .find(|o| o.id == clip)
        .ok_or(EditError::NoSuchClip)?;
    me.start = start;
    sort_clips(track);
    if track.clips.windows(2).any(|w| w[0].end() > w[1].start) {
        return Err(EditError::Overlap);
    }
    *p = q;
    Ok(Outcome::Clip(clip))
}

fn move_clip(
    p: &mut Project,
    clip: ClipId,
    track: TrackId,
    start: u64,
) -> Result<Outcome, EditError> {
    let (from_ti, ci) = editable(p, clip)?;
    let duration = p.tracks[from_ti].clips[ci].duration;
    let dest = p.track(track).ok_or(EditError::NoSuchTrack)?;
    if dest.locked {
        return Err(EditError::Locked);
    }
    if dest.kind != p.tracks[from_ti].kind {
        return Err(EditError::WrongTrackKind);
    }
    if !p.can_place(track, start, duration, Some(clip)) {
        return Err(
            if start.checked_add(duration).is_none_or(|e| e > MAX_FRAMES) {
                EditError::OutOfRange
            } else {
                EditError::Overlap
            },
        );
    }
    let mut moved = p.tracks[from_ti].clips.remove(ci);
    moved.start = start;
    if let Some(t) = p.track_mut(track) {
        t.clips.push(moved);
        sort_clips(t);
    }
    Ok(Outcome::Clip(clip))
}

/// Moves clips so that the same source timecode falls on the same timeline frame: the clip with
/// the earliest timecode stays where it is and the others line up with it.
fn align_clips(p: &mut Project, clips: &[ClipId]) -> Result<Outcome, EditError> {
    let mut ids: Vec<ClipId> = clips.to_vec();
    ids.sort_by_key(|c| c.0);
    ids.dedup();
    if ids.len() < 2 {
        return Err(EditError::Invalid("aligning needs two clips or more"));
    }
    // The source timecode of each clip's first frame, in project frames.
    let mut timed: Vec<(ClipId, u64)> = Vec::with_capacity(ids.len());
    for id in &ids {
        let (ti, ci) = editable(p, *id)?;
        let clip = &p.tracks[ti].clips[ci];
        let item = p.media(clip.media).ok_or(EditError::NoSuchMedia)?;
        let start_tc = item
            .timecode
            .as_deref()
            .ok_or(EditError::Invalid("a clip's media has no timecode"))?;
        let base = p
            .fps
            .parse_timecode(start_tc)
            .ok_or(EditError::Invalid("the media's timecode"))?;
        // The source frame the clip shows first, so a slipped, reversed or frozen clip lines up
        // by the frame it really shows.
        let abs = base
            .checked_add(clip.source_frame_at(clip.start))
            .ok_or(EditError::OutOfRange)?;
        timed.push((*id, abs));
    }
    // The earliest timecode anchors; the others take the same distance from it on the timeline.
    let (anchor_abs, anchor_start) = timed
        .iter()
        .map(|(id, abs)| (*abs, p.clip(*id).map_or(0, |(_, c)| c.start)))
        .min_by_key(|(abs, _)| *abs)
        .ok_or(EditError::Invalid("aligning needs two clips or more"))?;
    let mut targets: Vec<(ClipId, u64)> = Vec::with_capacity(timed.len());
    for (id, abs) in &timed {
        let start = anchor_start
            .checked_add(abs - anchor_abs)
            .ok_or(EditError::OutOfRange)?;
        targets.push((*id, start));
    }
    move_groups(p, &targets)
}

/// Moves each clip to the frame given and its linked sound by the same distance, all or nothing.
fn place_clips(p: &mut Project, starts: &[(ClipId, u64)]) -> Result<Outcome, EditError> {
    let mut ids: Vec<ClipId> = starts.iter().map(|(id, _)| *id).collect();
    ids.sort_by_key(|c| c.0);
    ids.dedup();
    if ids.len() < 2 || ids.len() != starts.len() {
        return Err(EditError::Invalid("placing needs two clips or more, each once"));
    }
    for id in &ids {
        editable(p, *id)?;
    }
    move_groups(p, starts)
}

/// The moving of [`align_clips`] and [`place_clips`]: every clip goes to its frame, with the
/// clips it is linked to, or none moves.
fn move_groups(p: &mut Project, targets: &[(ClipId, u64)]) -> Result<Outcome, EditError> {
    let mut q = p.clone();
    // Take every clip out first, so one on its way cannot overlap one still to move. A clip's
    // linked sound goes with it, by the same distance.
    let mut moving: Vec<(TrackId, Clip, u64)> = Vec::with_capacity(targets.len());
    let mut taken: Vec<ClipId> = Vec::new();
    for (id, start) in targets {
        let start = *start;
        let Some((_, c)) = q.clip(*id) else {
            continue;
        };
        let from = c.start;
        let mut group = vec![*id];
        group.extend(q.partners(*id));
        for cid in group {
            if taken.contains(&cid) {
                continue;
            }
            let (ti, ci) = q.locate(cid).ok_or(EditError::NoSuchClip)?;
            if q.tracks[ti].locked {
                return Err(EditError::Locked);
            }
            let clip = q.tracks[ti].clips.remove(ci);
            let to = shifted(clip.start, from, start).ok_or(EditError::OutOfRange)?;
            taken.push(cid);
            moving.push((q.tracks[ti].id, clip, to));
        }
    }
    for (track, mut clip, start) in moving {
        if start
            .checked_add(clip.duration)
            .is_none_or(|e| e > MAX_FRAMES)
        {
            return Err(EditError::OutOfRange);
        }
        if !q.can_place(track, start, clip.duration, None) {
            return Err(EditError::Overlap);
        }
        clip.start = start;
        if let Some(t) = q.track_mut(track) {
            t.clips.push(clip);
            sort_clips(t);
        }
    }
    *p = q;
    Ok(Outcome::Nothing)
}

/// The most pieces a speed ramp makes.
pub const MAX_RAMP_STEPS: usize = 64;

fn speed_ramp(p: &mut Project, clip: ClipId, speeds: &[f32]) -> Result<Outcome, EditError> {
    if !(2..=MAX_RAMP_STEPS).contains(&speeds.len())
        || speeds
            .iter()
            .any(|s| !s.is_finite() || !(crate::MIN_SPEED..=crate::MAX_SPEED).contains(s))
    {
        return Err(EditError::Invalid("the speeds of the ramp"));
    }
    editable(p, clip)?;
    let mut q = p.clone();
    let mut ids = vec![clip];
    ids.extend(q.linked_to(clip));
    // Each piece of every clip shares a link with the same piece of the others.
    let links: Vec<Option<u64>> = (0..speeds.len())
        .map(|_| (ids.len() > 1).then(|| q.fresh_id()))
        .collect();
    for id in ids {
        ramp_one(&mut q, id, speeds, &links)?;
    }
    *p = q;
    Ok(Outcome::Clip(clip))
}

/// Ramps one clip; see [`Command::SpeedRamp`].
fn ramp_one(
    p: &mut Project,
    clip: ClipId,
    speeds: &[f32],
    links: &[Option<u64>],
) -> Result<(), EditError> {
    let (ti, ci) = editable(p, clip)?;
    let c = p.tracks[ti].clips[ci].clone();
    if c.freeze || c.reverse {
        return Err(EditError::Invalid(
            "a frozen or reversed clip cannot be ramped",
        ));
    }
    let span = c.source_span();
    let n = speeds.len() as u64;
    if span < n {
        return Err(EditError::Invalid(
            "the clip is too short for that many steps",
        ));
    }
    let length = p.media(c.media).and_then(|m| m.length);
    let mut pieces = Vec::with_capacity(speeds.len());
    let mut at = c.start;
    for (i, (&speed, link)) in speeds.iter().zip(links).enumerate() {
        let i = i as u64;
        let from = span * i / n;
        let to = span * (i + 1) / n;
        let mut piece = c.clone();
        if i > 0 {
            piece.id = ClipId(p.fresh_id());
            piece.transition = None;
            piece.audio.fade_in = 0;
        }
        if i + 1 < n {
            piece.audio.fade_out = 0;
        }
        piece.source_in = c.source_in + from;
        piece.speed = speed;
        piece.duration = (((to - from) as f64 / f64::from(speed)).round() as u64).max(1);
        // Rounding may reach past the media: slow down until it fits.
        for _ in 0..16 {
            if length.is_none_or(|l| piece.source_in + piece.source_span() <= l) {
                break;
            }
            piece.speed = f32::from_bits(piece.speed.to_bits() - 1);
        }
        if length.is_some_and(|l| piece.source_in + piece.source_span() > l) {
            return Err(EditError::OutOfRange);
        }
        piece.start = at;
        piece.link = if c.link.is_some() { *link } else { None };
        piece.audio = piece.audio.sanitized(piece.duration);
        at = at
            .checked_add(piece.duration)
            .ok_or(EditError::OutOfRange)?;
        pieces.push(piece);
    }
    if at > MAX_FRAMES {
        return Err(EditError::OutOfRange);
    }
    let old_end = c.end();
    let track = &mut p.tracks[ti];
    // The clips after it move by the change in length.
    for later in track.clips.iter_mut().filter(|o| o.start >= old_end) {
        later.start = (later.start + at).saturating_sub(old_end);
    }
    track.clips.splice(ci..=ci, pieces);
    sort_clips(track);
    Ok(())
}

fn set_speed(p: &mut Project, clip: ClipId, speed: f32) -> Result<Outcome, EditError> {
    if !speed.is_finite() || !(crate::MIN_SPEED..=crate::MAX_SPEED).contains(&speed) {
        return Err(EditError::Invalid("the speed"));
    }
    let (ti, ci) = editable(p, clip)?;
    let c = &p.tracks[ti].clips[ci];
    // The same part of the media takes proportionally more or less time.
    let duration =
        ((c.duration as f64 * f64::from(c.speed) / f64::from(speed)).round() as u64).max(1);
    let length = p.media(c.media).and_then(|m| m.length);
    let span = (duration as f64 * f64::from(speed)).ceil() as u64;
    if length.is_some_and(|l| c.source_in.saturating_add(span) > l) {
        return Err(EditError::OutOfRange);
    }
    if !p.can_place(p.tracks[ti].id, c.start, duration, Some(clip)) {
        return Err(if c.start.saturating_add(duration) > MAX_FRAMES {
            EditError::OutOfRange
        } else {
            EditError::Overlap
        });
    }
    let c = &mut p.tracks[ti].clips[ci];
    c.speed = speed;
    c.duration = duration;
    c.audio = c.audio.sanitized(duration);
    Ok(Outcome::Clip(clip))
}

fn trim_start(p: &mut Project, clip: ClipId, start: u64) -> Result<Outcome, EditError> {
    let (ti, ci) = editable(p, clip)?;
    let c = &p.tracks[ti].clips[ci];
    let end = c.end();
    if start >= end {
        return Err(EditError::Invalid("a clip needs at least one frame"));
    }
    let new_duration = end - start;
    let new_source_in = source_in_after_trim(p, c, start, new_duration)?;
    if !p.can_place(p.tracks[ti].id, start, new_duration, Some(clip)) {
        return Err(EditError::Overlap);
    }
    let c = &mut p.tracks[ti].clips[ci];
    // The animation stays with the picture.
    c.shift_keys(c.start as i64 - start as i64);
    (c.start, c.source_in, c.duration) = (start, new_source_in, new_duration);
    Ok(Outcome::Clip(clip))
}

fn trim_end(p: &mut Project, clip: ClipId, end: u64) -> Result<Outcome, EditError> {
    let (ti, ci) = editable(p, clip)?;
    let c = &p.tracks[ti].clips[ci];
    if end <= c.start {
        return Err(EditError::Invalid("a clip needs at least one frame"));
    }
    let duration = end - c.start;
    let source_in = source_in_after_trim(p, c, c.start, duration)?;
    if !p.can_place(p.tracks[ti].id, c.start, duration, Some(clip)) {
        return Err(EditError::Overlap);
    }
    let c = &mut p.tracks[ti].clips[ci];
    (c.source_in, c.duration) = (source_in, duration);
    Ok(Outcome::Clip(clip))
}

/// The first frame of media of clip `c` once it is trimmed to `start` and `duration`, keeping
/// every frame it still shows where it was. The edge that moves along the footage is the start,
/// or the end for a reversed clip; the other edge keeps its frame.
fn source_in_after_trim(
    p: &Project,
    c: &Clip,
    start: u64,
    duration: u64,
) -> Result<u64, EditError> {
    let length = p.media(c.media).and_then(|m| m.length);
    // How many frames the footage edge moves inward (positive) or outward (negative).
    let (old_edge, new_edge) = if c.reverse {
        (c.end(), start.saturating_add(duration))
    } else {
        (c.start, start)
    };
    let source_in = if (new_edge >= old_edge) != c.reverse {
        // Inward: the clip's first frame of media comes later.
        c.source_in + c.source_frames(new_edge.abs_diff(old_edge))
    } else {
        let back = c.source_frames(new_edge.abs_diff(old_edge));
        match length {
            // A still can be held for any time; footage cannot start before its first frame.
            None => c.source_in.saturating_sub(back),
            Some(_) => c.source_in.checked_sub(back).ok_or(EditError::OutOfRange)?,
        }
    };
    let mut trimmed = c.clone();
    trimmed.duration = duration;
    if length.is_some_and(|l| source_in.saturating_add(trimmed.source_span()) > l) {
        return Err(EditError::OutOfRange);
    }
    Ok(source_in)
}

/// Cuts the multicam clip's video at `at` and goes on with `angle` there.
fn switch_angle(
    p: &mut Project,
    multicam: u64,
    angle: usize,
    at: u64,
) -> Result<Outcome, EditError> {
    let mc = p
        .multicams
        .iter()
        .find(|m| m.id == multicam)
        .ok_or(EditError::Invalid("there is no such multicam clip"))?
        .clone();
    let target = mc
        .angles
        .get(angle)
        .ok_or(EditError::Invalid("there is no such angle"))?;
    // The topmost video clip that shows one of the angles.
    let (ti, ci) = p
        .tracks
        .iter()
        .enumerate()
        .rev()
        .filter(|(_, t)| t.kind == TrackKind::Video)
        .find_map(|(ti, t)| {
            t.clips
                .iter()
                .position(|c| c.contains(at) && mc.angle_of(c.media).is_some())
                .map(|ci| (ti, ci))
        })
        .ok_or(EditError::Invalid(
            "no angle of this clip is on the timeline there",
        ))?;
    let c = p.tracks[ti].clips[ci].clone();
    let current = mc.angle_of(c.media).ok_or(EditError::Invalid(
        "no angle of this clip is on the timeline there",
    ))?;
    if current == angle {
        return Ok(Outcome::Clip(c.id));
    }
    editable(p, c.id)?;
    if c.speed != 1.0 || c.reverse || c.freeze {
        return Err(EditError::Invalid(
            "angles switch only on clips at normal speed",
        ));
    }
    let sync = c.source_frame_at(at) as i64 - mc.angles[current].offset;
    let new_in = u64::try_from(sync + target.offset).map_err(|_| EditError::OutOfRange)?;
    let remaining = c.end() - at;
    let item = p.media(target.media).ok_or(EditError::NoSuchMedia)?;
    let name = item.name.clone();
    let duration = match item.length {
        Some(l) if new_in >= l => return Err(EditError::OutOfRange),
        Some(l) => remaining.min(l - new_in),
        None => remaining,
    };
    let id = if at > c.start {
        match split(p, c.id, at)? {
            Outcome::Split(_, right) => right,
            _ => return Err(EditError::Invalid("the cut failed")),
        }
    } else {
        c.id
    };
    let (ti, ci) = p.locate(id).ok_or(EditError::NoSuchClip)?;
    let n = &mut p.tracks[ti].clips[ci];
    n.media = target.media;
    n.name = name;
    n.source_in = new_in;
    n.link = None;
    if duration < n.duration {
        n.duration = duration;
        n.audio = n.audio.sanitized(duration);
    }
    Ok(Outcome::Clip(id))
}

fn split(p: &mut Project, clip: ClipId, at: u64) -> Result<Outcome, EditError> {
    let (ti, ci) = editable(p, clip)?;
    let c = &p.tracks[ti].clips[ci];
    if at <= c.start || at >= c.end() {
        return Err(EditError::Invalid("the cut must be inside the clip"));
    }
    let left = at - c.start;
    let mut right = c.clone();
    let advance = c.source_frames(left);
    let reverse = c.reverse;
    let right_span = c.source_span().saturating_sub(advance);
    right.id = ClipId(p.fresh_id());
    right.start = at;
    right.transition = None;
    if reverse {
        // Backwards, the left half shows the later footage.
        p.tracks[ti].clips[ci].source_in += right_span;
    } else {
        right.source_in += advance;
    }
    right.duration -= left;
    right.shift_keys(-(left as i64));
    // The fade in stays with the left half, the fade out with the right half.
    right.audio.fade_in = 0;
    right.audio = right.audio.sanitized(right.duration);
    let right_id = right.id;
    let track = &mut p.tracks[ti];
    track.clips[ci].duration = left;
    track.clips[ci].audio.fade_out = 0;
    track.clips[ci].audio = track.clips[ci].audio.sanitized(left);
    track.clips.insert(ci + 1, right);
    Ok(Outcome::Split(clip, right_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Demo project: V1 holds three clips of 96 frames at 0, 96 and 192.
    fn demo() -> (Project, TrackId, TrackId, [ClipId; 3]) {
        let p = Project::demo();
        let ids = [
            p.tracks[0].clips[0].id,
            p.tracks[0].clips[1].id,
            p.tracks[0].clips[2].id,
        ];
        (p.clone(), p.tracks[0].id, p.tracks[1].id, ids)
    }

    fn starts(p: &Project, t: usize) -> Vec<(u64, u64)> {
        p.tracks[t]
            .clips
            .iter()
            .map(|c| (c.start, c.duration))
            .collect()
    }

    /// Runs a command that must fail and checks that nothing changed.
    fn refused(p: &mut Project, c: Command) -> EditError {
        let before = p.clone();
        let err = c.apply(p).unwrap_err();
        assert_eq!(*p, before, "a refused command must not change the project");
        err
    }

    #[test]
    fn add_clip_uses_the_default_duration_and_sorts() {
        let (mut p, v1, _, _) = demo();
        let media = p.media[0].id;
        let Outcome::Clip(id) = (Command::AddClip {
            track: v1,
            media,
            start: 1000,
            duration: None,
            with_sound: false,
        })
        .apply(&mut p)
        .unwrap() else {
            panic!("expected a clip")
        };
        assert_eq!(p.clip(id).unwrap().1.duration, 96);
        let Outcome::Clip(early) = (Command::AddClip {
            track: v1,
            media,
            start: 300,
            duration: Some(10),
            with_sound: false,
        })
        .apply(&mut p)
        .unwrap() else {
            panic!("expected a clip")
        };
        assert_eq!(p.tracks[0].clips[3].id, early);
        assert!(p.clone().validate().is_ok());
    }

    #[test]
    fn add_clip_refusals() {
        let (mut p, v1, _, _) = demo();
        let media = p.media[0].id;
        let add = |track, media, start, duration| Command::AddClip {
            track,
            media,
            start,
            duration,
            with_sound: false,
        };
        assert_eq!(
            refused(&mut p, add(v1, media, 50, None)),
            EditError::Overlap
        );
        assert_eq!(
            refused(&mut p, add(v1, MediaId(77), 400, None)),
            EditError::NoSuchMedia
        );
        assert_eq!(
            refused(&mut p, add(TrackId(77), media, 400, None)),
            EditError::NoSuchTrack
        );
        assert!(matches!(
            refused(&mut p, add(v1, media, 400, Some(0))),
            EditError::Invalid(_)
        ));
        assert_eq!(
            refused(&mut p, add(v1, media, u64::MAX - 1, Some(5))),
            EditError::OutOfRange
        );
        Command::SetTrackLocked {
            track: v1,
            locked: true,
        }
        .apply(&mut p)
        .unwrap();
        assert_eq!(
            refused(&mut p, add(v1, media, 400, None)),
            EditError::Locked
        );
    }

    #[test]
    fn a_track_can_be_deleted_but_not_the_last_of_its_kind() {
        let (mut p, v1, _, _) = demo();
        let videos = |p: &Project| -> Vec<TrackId> {
            p.tracks
                .iter()
                .filter(|t| t.kind == TrackKind::Video)
                .map(|t| t.id)
                .collect()
        };
        if videos(&p).len() < 2 {
            (Command::AddTrack {
                kind: TrackKind::Video,
            })
            .apply(&mut p)
            .unwrap();
        }
        // V1 goes, with its clips.
        (Command::RemoveTrack { track: v1 }).apply(&mut p).unwrap();
        assert!(p.track(v1).is_none());
        let rest = videos(&p);
        for t in &rest[1..] {
            (Command::RemoveTrack { track: *t }).apply(&mut p).unwrap();
        }
        assert!(matches!(
            refused(&mut p, Command::RemoveTrack { track: rest[0] }),
            EditError::Invalid(_)
        ));
    }

    #[test]
    fn audio_tracks_do_not_take_video() {
        let (mut p, _, _, _) = demo();
        let Outcome::Track(a1) = (Command::AddTrack {
            kind: TrackKind::Audio,
        })
        .apply(&mut p)
        .unwrap() else {
            panic!()
        };
        assert_eq!(p.track(a1).unwrap().name, "A1");
        let media = p.media[0].id;
        assert_eq!(
            refused(
                &mut p,
                Command::AddClip {
                    track: a1,
                    media,
                    start: 0,
                    duration: None,
                    with_sound: false,
                }
            ),
            EditError::WrongTrackKind
        );
    }

    #[test]
    fn sound_files_go_on_audio_tracks_only() {
        let (mut p, v1, _, _) = demo();
        let Outcome::Track(a1) = (Command::AddTrack {
            kind: TrackKind::Audio,
        })
        .apply(&mut p)
        .unwrap() else {
            panic!()
        };
        let Outcome::Media(sound) = (Command::AddMedia {
            name: "music".into(),
            source: MediaSource::Audio {
                path: "music.wav".into(),
            },
            length: Some(200),
            default_duration: 200,
        })
        .apply(&mut p)
        .unwrap() else {
            panic!()
        };
        let add = |track, start| Command::AddClip {
            track,
            media: sound,
            start,
            duration: None,
            with_sound: false,
        };
        assert_eq!(refused(&mut p, add(v1, 1000)), EditError::WrongTrackKind);
        assert!(add(a1, 0).apply(&mut p).is_ok());
        // A sound clip cannot be moved to a video track either.
        let clip = p.track(a1).unwrap().clips[0].id;
        assert_eq!(
            refused(
                &mut p,
                Command::MoveClip {
                    clip,
                    track: v1,
                    start: 1000
                }
            ),
            EditError::WrongTrackKind
        );
    }

    #[test]
    fn move_to_another_track_and_back() {
        let (mut p, v1, v2, [a, ..]) = demo();
        Command::MoveClip {
            clip: a,
            track: v2,
            start: 10,
        }
        .apply(&mut p)
        .unwrap();
        assert_eq!(starts(&p, 1), vec![(10, 96)]);
        assert_eq!(p.tracks[0].clips.len(), 2);
        Command::MoveClip {
            clip: a,
            track: v1,
            start: 0,
        }
        .apply(&mut p)
        .unwrap();
        assert_eq!(starts(&p, 0), vec![(0, 96), (96, 96), (192, 96)]);
    }

    #[test]
    fn move_within_a_track_may_overlap_itself_but_not_others() {
        let (mut p, v1, _, [a, ..]) = demo();
        assert_eq!(
            refused(
                &mut p,
                Command::MoveClip {
                    clip: a,
                    track: v1,
                    start: 50
                }
            ),
            EditError::Overlap
        );
        assert_eq!(
            refused(
                &mut p,
                Command::MoveClip {
                    clip: a,
                    track: v1,
                    start: u64::MAX
                }
            ),
            EditError::OutOfRange
        );
        Command::MoveClip {
            clip: a,
            track: v1,
            start: 288,
        }
        .apply(&mut p)
        .unwrap();
        assert_eq!(p.tracks[0].clips.last().unwrap().id, a);
        assert!(p.clone().validate().is_ok());
    }

    #[test]
    fn trim_start_shrinks_and_extends() {
        let (mut p, _, _, [_, b, _]) = demo();
        Command::TrimStart {
            clip: b,
            start: 120,
        }
        .apply(&mut p)
        .unwrap();
        let c = p.clip(b).unwrap().1;
        assert_eq!(
            (c.start, c.source_in, c.duration, c.end()),
            (120, 24, 72, 192)
        );
        // Extending back to the left neighbour's end restores it, but not past it.
        Command::TrimStart { clip: b, start: 96 }
            .apply(&mut p)
            .unwrap();
        assert_eq!(p.clip(b).unwrap().1.source_in, 0);
        assert_eq!(
            refused(&mut p, Command::TrimStart { clip: b, start: 90 }),
            EditError::Overlap
        );
        assert!(matches!(
            refused(
                &mut p,
                Command::TrimStart {
                    clip: b,
                    start: 192
                }
            ),
            EditError::Invalid(_)
        ));
    }

    #[test]
    fn trim_start_cannot_go_before_the_first_frame_of_footage_but_a_still_can() {
        let (mut p, v1, _, [a, ..]) = demo();
        let Outcome::Media(footage) = (Command::AddMedia {
            name: "shot".into(),
            source: MediaSource::ColorBars,
            length: Some(100),
            default_duration: 10,
        })
        .apply(&mut p)
        .unwrap() else {
            panic!()
        };
        let Outcome::Clip(c) = (Command::AddClip {
            track: v1,
            media: footage,
            start: 500,
            duration: Some(10),
            with_sound: false,
        })
        .apply(&mut p)
        .unwrap() else {
            panic!()
        };
        assert_eq!(
            refused(
                &mut p,
                Command::TrimStart {
                    clip: c,
                    start: 490
                }
            ),
            EditError::OutOfRange
        );
        // The first clip is a still: it can grow to the left of its start (frame 0 here is as far as
        // the timeline goes, so move it right first).
        Command::TrimEnd { clip: a, end: 50 }.apply(&mut p).unwrap();
        Command::MoveClip {
            clip: a,
            track: v1,
            start: 20,
        }
        .apply(&mut p)
        .unwrap();
        Command::TrimStart { clip: a, start: 0 }
            .apply(&mut p)
            .unwrap();
        let grown = p.clip(a).unwrap().1;
        assert_eq!((grown.start, grown.source_in, grown.duration), (0, 0, 70));
    }

    #[test]
    fn trim_end_shrinks_extends_and_respects_media_length() {
        let (mut p, _, _, [a, ..]) = demo();
        Command::TrimEnd { clip: a, end: 50 }.apply(&mut p).unwrap();
        assert_eq!(p.clip(a).unwrap().1.duration, 50);
        assert_eq!(
            refused(&mut p, Command::TrimEnd { clip: a, end: 97 }),
            EditError::Overlap
        );
        assert!(matches!(
            refused(&mut p, Command::TrimEnd { clip: a, end: 0 }),
            EditError::Invalid(_)
        ));
        // A clip of a media with a length cannot run past it.
        let Outcome::Media(m) = (Command::AddMedia {
            name: "shot".into(),
            source: MediaSource::ColorBars,
            length: Some(30),
            default_duration: 30,
        })
        .apply(&mut p)
        .unwrap() else {
            panic!()
        };
        let track = p.tracks[1].id;
        let Outcome::Clip(c) = (Command::AddClip {
            track,
            media: m,
            start: 0,
            duration: None,
            with_sound: false,
        })
        .apply(&mut p)
        .unwrap() else {
            panic!()
        };
        assert_eq!(
            refused(&mut p, Command::TrimEnd { clip: c, end: 31 }),
            EditError::OutOfRange
        );
        assert_eq!(
            refused(
                &mut p,
                Command::AddClip {
                    track,
                    media: m,
                    start: 100,
                    duration: Some(31),
                    with_sound: false,
                }
            ),
            EditError::OutOfRange
        );
    }

    #[test]
    fn split_makes_two_clips_that_cover_the_same_frames() {
        let (mut p, _, _, [a, ..]) = demo();
        let Outcome::Split(left, right) =
            (Command::Split { clip: a, at: 30 }).apply(&mut p).unwrap()
        else {
            panic!()
        };
        assert_eq!(left, a);
        let (l, r) = (
            p.clip(left).unwrap().1.clone(),
            p.clip(right).unwrap().1.clone(),
        );
        assert_eq!((l.start, l.duration, l.source_in), (0, 30, 0));
        assert_eq!((r.start, r.duration, r.source_in), (30, 66, 30));
        assert_eq!(l.media, r.media);
        assert_ne!(left, right);
        assert!(p.clone().validate().is_ok());
    }

    #[test]
    fn split_keeps_the_grade_on_both_halves() {
        let (mut p, _, _, [a, ..]) = demo();
        let grade = ColorGrade {
            saturation: 0.5,
            ..Default::default()
        };
        Command::SetGrade {
            clip: a,
            grade: grade.clone(),
            node: 0,
        }
        .apply(&mut p)
        .unwrap();
        let Outcome::Split(_, right) = (Command::Split { clip: a, at: 10 }).apply(&mut p).unwrap()
        else {
            panic!()
        };
        assert_eq!(p.clip(right).unwrap().1.grade, grade);
    }

    #[test]
    fn split_refusals() {
        let (mut p, _, _, [a, ..]) = demo();
        for at in [0, 96, 500] {
            assert!(
                matches!(
                    refused(&mut p, Command::Split { clip: a, at }),
                    EditError::Invalid(_)
                ),
                "at {at}"
            );
        }
        assert_eq!(
            refused(
                &mut p,
                Command::Split {
                    clip: ClipId(9999),
                    at: 5
                }
            ),
            EditError::NoSuchClip
        );
    }

    #[test]
    fn delete_leaves_a_gap_and_ripple_delete_closes_it() {
        let (mut p, _, _, [a, b, c]) = demo();
        Command::Delete { clip: b }.apply(&mut p).unwrap();
        assert_eq!(starts(&p, 0), vec![(0, 96), (192, 96)]);
        Command::RippleDelete { clip: a }.apply(&mut p).unwrap();
        assert_eq!(starts(&p, 0), vec![(96, 96)]);
        assert_eq!(p.tracks[0].clips[0].id, c);
        assert_eq!(
            refused(&mut p, Command::Delete { clip: a }),
            EditError::NoSuchClip
        );
    }

    #[test]
    fn locked_tracks_refuse_edits_but_not_grades() {
        let (mut p, v1, _, [a, ..]) = demo();
        Command::SetTrackLocked {
            track: v1,
            locked: true,
        }
        .apply(&mut p)
        .unwrap();
        for c in [
            Command::Delete { clip: a },
            Command::Split { clip: a, at: 5 },
            Command::TrimEnd { clip: a, end: 5 },
            Command::SetOpacity {
                clip: a,
                opacity: 0.5,
            },
        ] {
            assert_eq!(refused(&mut p, c), EditError::Locked);
        }
        Command::SetGrade {
            clip: a,
            grade: ColorGrade {
                saturation: 0.0,
                ..Default::default()
            },
            node: 0,
        }
        .apply(&mut p)
        .unwrap();
    }

    #[test]
    fn a_track_can_be_soloed_and_its_picture_turned_off() {
        let (mut p, v1, a1, _) = demo();
        assert!(!p.any_soloed());
        Command::SetTrackSolo {
            track: a1,
            solo: true,
        }
        .apply(&mut p)
        .unwrap();
        assert!(p.track(a1).unwrap().solo);
        assert!(p.any_soloed());
        Command::SetTrackPicture {
            track: v1,
            picture: false,
        }
        .apply(&mut p)
        .unwrap();
        assert!(!p.track(v1).unwrap().picture);
        assert!(p.track(a1).unwrap().picture, "only V1 lost its picture");
        assert_eq!(
            refused(
                &mut p,
                Command::SetTrackSolo {
                    track: TrackId(u64::MAX),
                    solo: true
                }
            ),
            EditError::NoSuchTrack
        );
        Command::SetTrackSolo {
            track: a1,
            solo: false,
        }
        .apply(&mut p)
        .unwrap();
        Command::SetTrackPicture {
            track: v1,
            picture: true,
        }
        .apply(&mut p)
        .unwrap();
        assert!(!p.any_soloed());
        assert!(p.track(v1).unwrap().picture);
    }

    #[test]
    fn opacity_is_clamped_and_nan_refused() {
        let (mut p, _, _, [a, ..]) = demo();
        Command::SetOpacity {
            clip: a,
            opacity: 3.0,
        }
        .apply(&mut p)
        .unwrap();
        assert_eq!(p.clip(a).unwrap().1.opacity, 1.0);
        assert!(matches!(
            refused(
                &mut p,
                Command::SetOpacity {
                    clip: a,
                    opacity: f32::NAN
                }
            ),
            EditError::Invalid(_)
        ));
    }

    #[test]
    fn media_in_use_cannot_be_removed() {
        let (mut p, _, _, _) = demo();
        let used = p.media[0].id;
        assert_eq!(
            refused(&mut p, Command::RemoveMedia { media: used }),
            EditError::MediaInUse
        );
        let Outcome::Media(spare) = (Command::AddMedia {
            name: "x".into(),
            source: MediaSource::Solid { rgb: [0; 3] },
            length: None,
            default_duration: 10,
        })
        .apply(&mut p)
        .unwrap() else {
            panic!()
        };
        Command::RemoveMedia { media: spare }.apply(&mut p).unwrap();
        assert_eq!(
            refused(&mut p, Command::RemoveMedia { media: spare }),
            EditError::NoSuchMedia
        );
    }

    #[test]
    fn set_format_validates() {
        let (mut p, ..) = demo();
        Command::SetFormat {
            width: 3840,
            height: 2160,
            fps: Fps::FPS_60,
        }
        .apply(&mut p)
        .unwrap();
        assert_eq!((p.width, p.height, p.fps), (3840, 2160, Fps::FPS_60));
        for (width, height, fps) in [
            (0, 10, Fps::FPS_24),
            (99_999, 10, Fps::FPS_24),
            (10, 10, Fps { num: 1, den: 0 }),
        ] {
            assert!(matches!(
                refused(&mut p, Command::SetFormat { width, height, fps }),
                EditError::Invalid(_)
            ));
        }
    }

    #[test]
    fn command_ids_are_unique_and_serde_round_trips() {
        let all = [
            Command::RenameProject { name: "a".into() },
            Command::SetFormat {
                width: 1,
                height: 1,
                fps: Fps::FPS_24,
            },
            Command::AddMedia {
                name: "m".into(),
                source: MediaSource::ColorBars,
                length: None,
                default_duration: 1,
            },
            Command::RemoveMedia { media: MediaId(1) },
            Command::AddTrack {
                kind: TrackKind::Video,
            },
            Command::SetTrackLocked {
                track: TrackId(1),
                locked: true,
            },
            Command::SetTrackMuted {
                track: TrackId(1),
                muted: true,
            },
            Command::SetTrackSolo {
                track: TrackId(1),
                solo: true,
            },
            Command::SetTrackPicture {
                track: TrackId(1),
                picture: false,
            },
            Command::AddClip {
                track: TrackId(1),
                media: MediaId(1),
                start: 0,
                duration: None,
                with_sound: false,
            },
            Command::MoveClip {
                clip: ClipId(1),
                track: TrackId(1),
                start: 0,
            },
            Command::TrimStart {
                clip: ClipId(1),
                start: 0,
            },
            Command::TrimEnd {
                clip: ClipId(1),
                end: 1,
            },
            Command::Split {
                clip: ClipId(1),
                at: 1,
            },
            Command::Delete { clip: ClipId(1) },
            Command::RippleDelete { clip: ClipId(1) },
            Command::SetOpacity {
                clip: ClipId(1),
                opacity: 1.0,
            },
            Command::SetGrade {
                clip: ClipId(1),
                grade: ColorGrade::default(),
                node: 0,
            },
            Command::SetClipEnabled {
                clip: ClipId(1),
                enabled: false,
            },
            Command::PasteClip {
                track: TrackId(1),
                media: MediaId(1),
                name: "c".into(),
                start: 0,
                source_in: 0,
                duration: 1,
                opacity: 1.0,
                grade: ColorGrade::default(),
                enabled: true,
                audio: ClipAudio::default(),
                speed: 1.0,
                transform: Transform::default(),
                blend: BlendMode::Normal,
                sound: true,
                nodes: Vec::new(),
                reverse: false,
                freeze: false,
            },
            Command::SetMarker {
                frame: 1,
                name: "m".into(),
                color: 2,
            },
            Command::DeleteMarker { frame: 1 },
            Command::ClearMarkers,
            Command::SetMarks {
                mark_in: Some(1),
                mark_out: None,
            },
        ];
        let ids: std::collections::HashSet<_> = all.iter().map(Command::id).collect();
        assert_eq!(ids.len(), all.len());
        for c in all {
            let text = serde_json::to_string(&c).unwrap();
            assert_eq!(serde_json::from_str::<Command>(&text).unwrap(), c);
            assert!(!c.label().is_empty());
        }
    }

    #[test]
    fn disabling_a_clip_keeps_it_and_is_refused_on_a_locked_track() {
        let (mut p, v1, _, [a, ..]) = demo();
        Command::SetClipEnabled {
            clip: a,
            enabled: false,
        }
        .apply(&mut p)
        .unwrap();
        assert!(!p.clip(a).unwrap().1.enabled);
        Command::SetClipEnabled {
            clip: a,
            enabled: true,
        }
        .apply(&mut p)
        .unwrap();
        Command::SetTrackLocked {
            track: v1,
            locked: true,
        }
        .apply(&mut p)
        .unwrap();
        assert_eq!(
            refused(
                &mut p,
                Command::SetClipEnabled {
                    clip: a,
                    enabled: false
                }
            ),
            EditError::Locked
        );
    }

    fn paste(track: TrackId, media: MediaId, start: u64) -> Command {
        Command::PasteClip {
            track,
            media,
            name: "pasted".into(),
            start,
            source_in: 3,
            duration: 10,
            opacity: 0.5,
            grade: ColorGrade {
                saturation: 0.2,
                ..Default::default()
            },
            enabled: false,
            audio: ClipAudio::default(),
            speed: 1.0,
            transform: Transform::default(),
            blend: BlendMode::Normal,
            sound: true,
            nodes: Vec::new(),
            reverse: false,
            freeze: false,
        }
    }

    #[test]
    fn paste_keeps_the_look_and_checks_the_place() {
        let (mut p, v1, v2, _) = demo();
        let media = p.media[0].id;
        let Outcome::Clip(id) = paste(v2, media, 40).apply(&mut p).unwrap() else {
            panic!()
        };
        let c = p.clip(id).unwrap().1;
        assert_eq!(
            (c.start, c.source_in, c.duration, c.opacity, c.enabled),
            (40, 3, 10, 0.5, false)
        );
        assert_eq!(c.grade.saturation, 0.2);
        assert_eq!(refused(&mut p, paste(v1, media, 10)), EditError::Overlap);
        assert_eq!(refused(&mut p, paste(v2, media, 45)), EditError::Overlap);
        assert_eq!(
            refused(&mut p, paste(v2, MediaId(9), 500)),
            EditError::NoSuchMedia
        );
        assert_eq!(
            refused(&mut p, paste(TrackId(9), media, 500)),
            EditError::NoSuchTrack
        );
        assert_eq!(
            refused(&mut p, paste(v2, media, u64::MAX - 2)),
            EditError::OutOfRange
        );
        let nan = Command::PasteClip {
            track: v2,
            media,
            name: "x".into(),
            start: 500,
            source_in: 0,
            duration: 5,
            opacity: f32::NAN,
            grade: ColorGrade::default(),
            enabled: true,
            audio: ClipAudio::default(),
            speed: 1.0,
            transform: Transform::default(),
            blend: BlendMode::Normal,
            sound: true,
            nodes: Vec::new(),
            reverse: false,
            freeze: false,
        };
        assert!(matches!(refused(&mut p, nan), EditError::Invalid(_)));
        assert!(p.clone().validate().is_ok());
    }

    #[test]
    fn markers_stay_sorted_and_one_per_frame() {
        let (mut p, ..) = demo();
        for (frame, name) in [(50, "b"), (10, "a"), (90, "c")] {
            Command::SetMarker {
                frame,
                name: name.into(),
                color: 1,
            }
            .apply(&mut p)
            .unwrap();
        }
        assert_eq!(
            p.markers.iter().map(|m| m.frame).collect::<Vec<_>>(),
            vec![10, 50, 90]
        );
        Command::SetMarker {
            frame: 50,
            name: "renamed".into(),
            color: 4,
        }
        .apply(&mut p)
        .unwrap();
        assert_eq!(p.markers.len(), 3);
        assert_eq!(
            (p.markers[1].name.as_str(), p.markers[1].color),
            ("renamed", 4)
        );
        assert!(matches!(
            refused(
                &mut p,
                Command::SetMarker {
                    frame: 5,
                    name: "x".into(),
                    color: 8
                }
            ),
            EditError::Invalid(_)
        ));
        assert!(matches!(
            refused(&mut p, Command::DeleteMarker { frame: 77 }),
            EditError::Invalid(_)
        ));
        Command::DeleteMarker { frame: 10 }.apply(&mut p).unwrap();
        assert_eq!(p.markers.len(), 2);
        Command::ClearMarkers.apply(&mut p).unwrap();
        assert!(p.markers.is_empty());
        assert!(p.clone().validate().is_ok());
    }

    #[test]
    fn marks_need_in_before_out() {
        let (mut p, ..) = demo();
        Command::SetMarks {
            mark_in: Some(10),
            mark_out: Some(50),
        }
        .apply(&mut p)
        .unwrap();
        assert_eq!((p.mark_in, p.mark_out), (Some(10), Some(50)));
        assert!(matches!(
            refused(
                &mut p,
                Command::SetMarks {
                    mark_in: Some(50),
                    mark_out: Some(50)
                }
            ),
            EditError::Invalid(_)
        ));
        assert_eq!(
            refused(
                &mut p,
                Command::SetMarks {
                    mark_in: Some(u64::MAX),
                    mark_out: None
                }
            ),
            EditError::OutOfRange
        );
        Command::SetMarks {
            mark_in: None,
            mark_out: Some(5),
        }
        .apply(&mut p)
        .unwrap();
        assert_eq!((p.mark_in, p.mark_out), (None, Some(5)));
    }

    #[test]
    fn clip_audio_is_made_safe_and_fits_the_clip() {
        let (mut p, _, _, [a, ..]) = demo();
        Command::SetClipAudio {
            clip: a,
            audio: ClipAudio {
                gain_db: 500.0,
                fade_in: 80,
                fade_out: 80,
            },
        }
        .apply(&mut p)
        .unwrap();
        let audio = p.clip(a).unwrap().1.audio;
        // The clip has 96 frames: the gain is capped and the fades share the clip.
        assert_eq!(audio.gain_db, crate::MAX_GAIN_DB);
        assert_eq!((audio.fade_in, audio.fade_out), (80, 16));
        Command::SetClipAudio {
            clip: a,
            audio: ClipAudio {
                gain_db: f32::NAN,
                fade_in: 0,
                fade_out: 0,
            },
        }
        .apply(&mut p)
        .unwrap();
        assert_eq!(p.clip(a).unwrap().1.audio.gain_db, 0.0);
        // It can change on a locked track, like a grade.
        p.tracks[0].locked = true;
        assert!(Command::SetClipAudio {
            clip: a,
            audio: ClipAudio {
                gain_db: -6.0,
                ..ClipAudio::default()
            }
        }
        .apply(&mut p)
        .is_ok());
    }

    #[test]
    fn splitting_keeps_each_fade_with_its_end() {
        let (mut p, _, _, [a, ..]) = demo();
        Command::SetClipAudio {
            clip: a,
            audio: ClipAudio {
                gain_db: -3.0,
                fade_in: 10,
                fade_out: 20,
            },
        }
        .apply(&mut p)
        .unwrap();
        let Outcome::Split(left, right) =
            (Command::Split { clip: a, at: 40 }).apply(&mut p).unwrap()
        else {
            panic!()
        };
        let (l, r) = (
            p.clip(left).unwrap().1.audio,
            p.clip(right).unwrap().1.audio,
        );
        assert_eq!((l.fade_in, l.fade_out, l.gain_db), (10, 0, -3.0));
        assert_eq!((r.fade_in, r.fade_out, r.gain_db), (0, 20, -3.0));
    }

    #[test]
    fn track_mix_is_checked() {
        let (mut p, v1, _, _) = demo();
        Command::SetTrackMix {
            track: v1,
            gain_db: -12.0,
            pan: 3.0,
        }
        .apply(&mut p)
        .unwrap();
        let t = p.track(v1).unwrap();
        assert_eq!((t.gain_db, t.pan), (-12.0, 1.0));
        assert_eq!(
            refused(
                &mut p,
                Command::SetTrackMix {
                    track: v1,
                    gain_db: f32::INFINITY,
                    pan: 0.0
                }
            ),
            EditError::Invalid("the track mix")
        );
        assert_eq!(
            refused(
                &mut p,
                Command::SetTrackMix {
                    track: TrackId(9999),
                    gain_db: 0.0,
                    pan: 0.0
                }
            ),
            EditError::NoSuchTrack
        );
    }

    #[test]
    fn files_without_audio_settings_load_and_hostile_ones_are_repaired() {
        let p = Project::demo();
        let mut json: serde_json::Value = serde_json::from_str(&p.to_json().unwrap()).unwrap();
        // An older file has no audio settings at all.
        for t in json["tracks"].as_array_mut().unwrap() {
            t.as_object_mut().unwrap().remove("gain_db");
            t.as_object_mut().unwrap().remove("pan");
            for c in t["clips"].as_array_mut().unwrap() {
                c.as_object_mut().unwrap().remove("audio");
            }
        }
        let loaded = Project::from_json(&json.to_string()).unwrap();
        assert_eq!(loaded, p);
        // Out-of-range values in a file are brought back into range.
        json["tracks"][0]["gain_db"] = serde_json::json!(1e9);
        json["tracks"][0]["pan"] = serde_json::json!(-40.0);
        json["tracks"][0]["clips"][0]["audio"] =
            serde_json::json!({"gain_db": -1e9, "fade_in": 1000000, "fade_out": 5});
        let fixed = Project::from_json(&json.to_string()).unwrap();
        assert_eq!(fixed.tracks[0].gain_db, crate::MAX_GAIN_DB);
        assert_eq!(fixed.tracks[0].pan, -1.0);
        let a = fixed.tracks[0].clips[0].audio;
        assert_eq!(a.gain_db, crate::MIN_GAIN_DB);
        assert_eq!((a.fade_in, a.fade_out), (96, 0));
    }

    /// The (start, duration) of the clips of track `t`, and the first source frame of each.
    fn layout(p: &Project, t: usize) -> Vec<(u64, u64, u64)> {
        p.tracks[t]
            .clips
            .iter()
            .map(|c| (c.start, c.duration, c.source_in))
            .collect()
    }

    fn footage(p: &mut Project, length: u64) -> MediaId {
        let Outcome::Media(m) = (Command::AddMedia {
            name: "footage".into(),
            source: MediaSource::Video {
                path: "f.mp4".into(),
            },
            length: Some(length),
            default_duration: 30,
        })
        .apply(p)
        .unwrap() else {
            panic!()
        };
        m
    }

    #[test]
    fn insert_pushes_later_clips_and_splits_the_one_it_lands_in() {
        let (mut p, v1, _, _) = demo();
        let m = footage(&mut p, 500);
        // V1: 0..96, 96..192, 192..288. Insert 30 frames in the middle of the second clip.
        Command::InsertClip {
            track: v1,
            media: m,
            start: 120,
            duration: None,
            with_sound: false,
        }
        .apply(&mut p)
        .unwrap();
        assert_eq!(
            layout(&p, 0),
            [
                (0, 96, 0),
                (96, 24, 0),
                (120, 30, 0),
                (150, 72, 24),
                (222, 96, 0)
            ]
        );
        // At the start of the track everything moves; nothing is cut.
        Command::InsertClip {
            track: v1,
            media: m,
            start: 0,
            duration: Some(10),
            with_sound: false,
        }
        .apply(&mut p)
        .unwrap();
        assert_eq!(layout(&p, 0)[..2], [(0, 10, 0), (10, 96, 0)]);
        // Other tracks are not touched, a locked track refuses, and a failed insert changes nothing.
        assert!(p.tracks[1].clips.is_empty());
        p.tracks[0].locked = true;
        assert_eq!(
            refused(
                &mut p,
                Command::InsertClip {
                    track: v1,
                    media: m,
                    start: 0,
                    duration: None,
                    with_sound: false,
                }
            ),
            EditError::Locked
        );
        p.tracks[0].locked = false;
        assert_eq!(
            refused(
                &mut p,
                Command::InsertClip {
                    track: v1,
                    media: m,
                    start: MAX_FRAMES - 5,
                    duration: Some(10),
                    with_sound: false,
                }
            ),
            EditError::OutOfRange
        );
    }

    #[test]
    fn overwrite_covers_shortens_and_splits() {
        let (mut p, v1, _, _) = demo();
        let m = footage(&mut p, 500);
        // Covers the whole second clip and the first 10 frames of the third: 96..202.
        Command::OverwriteClip {
            track: v1,
            media: m,
            start: 96,
            duration: Some(106),
            with_sound: false,
        }
        .apply(&mut p)
        .unwrap();
        assert_eq!(
            layout(&p, 0),
            [(0, 96, 0), (96, 106, 0), (202, 86, 10)],
            "the second clip is gone, the third starts later and shows later footage"
        );
        // Lands inside one clip: that clip is split around it.
        let (mut p, v1, _, _) = demo();
        Command::OverwriteClip {
            track: v1,
            media: m_of(&mut p),
            start: 20,
            duration: Some(10),
            with_sound: false,
        }
        .apply(&mut p)
        .unwrap();
        assert_eq!(
            layout(&p, 0),
            [
                (0, 20, 0),
                (20, 10, 0),
                (30, 66, 30),
                (96, 96, 0),
                (192, 96, 0)
            ]
        );
        // Ends in a clip's start: shortens the one before.
        let (mut p, v1, _, _) = demo();
        let m = m_of(&mut p);
        Command::OverwriteClip {
            track: v1,
            media: m,
            start: 90,
            duration: Some(12),
            with_sound: false,
        }
        .apply(&mut p)
        .unwrap();
        assert_eq!(
            layout(&p, 0),
            [(0, 90, 0), (90, 12, 0), (102, 90, 6), (192, 96, 0)]
        );
    }

    /// A video media item of 500 frames in `p`.
    fn m_of(p: &mut Project) -> MediaId {
        footage(p, 500)
    }

    #[test]
    fn replace_keeps_place_and_look_and_checks_the_length() {
        let (mut p, _, _, [a, ..]) = demo();
        Command::SetOpacity {
            clip: a,
            opacity: 0.5,
        }
        .apply(&mut p)
        .unwrap();
        let long = footage(&mut p, 500);
        Command::ReplaceClip {
            clip: a,
            media: long,
        }
        .apply(&mut p)
        .unwrap();
        let c = p.clip(a).unwrap().1;
        assert_eq!(
            (c.media, c.start, c.duration, c.opacity),
            (long, 0, 96, 0.5)
        );
        assert_eq!(c.name, "footage");
        let short = footage(&mut p, 50);
        assert_eq!(
            refused(
                &mut p,
                Command::ReplaceClip {
                    clip: a,
                    media: short
                }
            ),
            EditError::OutOfRange
        );
        assert_eq!(
            refused(
                &mut p,
                Command::ReplaceClip {
                    clip: a,
                    media: MediaId(9999)
                }
            ),
            EditError::NoSuchMedia
        );
    }

    #[test]
    fn fit_to_fill_changes_the_speed_to_fit() {
        let (mut p, v1, _, _) = demo();
        let m = footage(&mut p, 100);
        let Outcome::Clip(id) = (Command::FitToFill {
            track: v1,
            media: m,
            start: 50,
            duration: 50,
            with_sound: false,
        })
        .apply(&mut p)
        .unwrap() else {
            panic!()
        };
        let c = p.clip(id).unwrap().1;
        assert_eq!((c.start, c.duration, c.source_in), (50, 50, 0));
        assert_eq!(
            p.tracks[0].clips.last().unwrap().end(),
            288,
            "an overwrite: the clips after it do not move"
        );
        assert_eq!(c.speed, 2.0, "100 frames of footage in 50 frames");
        assert!(c.source_span() <= 100);
        // It overwrote what it covers: nothing else overlaps it, and the track has no overlaps.
        let clips = &p.tracks[0].clips;
        assert!(clips
            .iter()
            .all(|o| o.id == id || o.end() <= 50 || o.start >= 100));
        assert!(clips.windows(2).all(|w| w[0].end() <= w[1].start));
        let odd = footage(&mut p, 100);
        let Outcome::Clip(id) = (Command::FitToFill {
            track: v1,
            media: odd,
            start: 1000,
            duration: 33,
            with_sound: false,
        })
        .apply(&mut p)
        .unwrap() else {
            panic!()
        };
        assert!(p.clip(id).unwrap().1.source_span() <= 100);
        assert_eq!(
            refused(
                &mut p,
                Command::FitToFill {
                    track: v1,
                    media: m,
                    start: 2000,
                    duration: 1,
                    with_sound: false,
                }
            ),
            EditError::Invalid("the speed"),
            "100 times faster is past the fastest speed"
        );
    }

    #[test]
    fn place_on_top_finds_or_adds_a_free_track() {
        let (mut p, v1, _, _) = demo();
        let media = p.media[0].id;
        let videos = |p: &Project| {
            p.tracks
                .iter()
                .filter(|t| t.kind == TrackKind::Video)
                .count()
        };
        let n = videos(&p);
        let Outcome::Clip(a) = (Command::PlaceOnTop {
            media,
            start: 10,
            with_sound: false,
        })
        .apply(&mut p)
        .unwrap() else {
            panic!()
        };
        assert_eq!(videos(&p), n, "the empty track above V1 is used");
        assert_ne!(p.clip(a).unwrap().0.id, v1);
        let Outcome::Clip(b) = (Command::PlaceOnTop {
            media,
            start: 20,
            with_sound: false,
        })
        .apply(&mut p)
        .unwrap() else {
            panic!()
        };
        assert_eq!(
            videos(&p),
            n + 1,
            "every track is busy there, so one is added"
        );
        assert_ne!(p.clip(a).unwrap().0.id, p.clip(b).unwrap().0.id);
        // Past the end of every clip, the lowest track is free.
        let Outcome::Clip(c) = (Command::PlaceOnTop {
            media,
            start: 5000,
            with_sound: false,
        })
        .apply(&mut p)
        .unwrap() else {
            panic!()
        };
        assert_eq!(p.clip(c).unwrap().0.id, v1);
    }

    #[test]
    fn reverse_and_freeze_change_which_frame_is_shown() {
        let (mut p, v1, _, _) = demo();
        let m = footage(&mut p, 100);
        let Outcome::Clip(id) = (Command::AddClip {
            track: v1,
            media: m,
            start: 1000,
            duration: Some(10),
            with_sound: false,
        })
        .apply(&mut p)
        .unwrap() else {
            panic!()
        };
        let at = |p: &Project, f| p.clip(id).unwrap().1.source_frame_at(f);
        assert_eq!((at(&p, 1000), at(&p, 1009)), (0, 9));
        Command::SetReverse {
            clip: id,
            reverse: true,
        }
        .apply(&mut p)
        .unwrap();
        assert_eq!((at(&p, 1000), at(&p, 1009)), (9, 0));
        // Cut in two, the parts still play the footage backwards, without a jump.
        Command::Split { clip: id, at: 1004 }.apply(&mut p).unwrap();
        let shown: Vec<u64> = (1000..1010)
            .map(|f| {
                let t = &p.tracks[0];
                Project::clip_at(t, f).unwrap().source_frame_at(f)
            })
            .collect();
        assert_eq!(shown, [9, 8, 7, 6, 5, 4, 3, 2, 1, 0]);
        Command::SetFreeze {
            clip: id,
            frame: Some(42),
        }
        .apply(&mut p)
        .unwrap();
        assert_eq!((at(&p, 1000), at(&p, 1003)), (42, 42));
        assert_eq!(
            refused(
                &mut p,
                Command::SetFreeze {
                    clip: id,
                    frame: Some(100)
                }
            ),
            EditError::OutOfRange
        );
        Command::SetFreeze {
            clip: id,
            frame: None,
        }
        .apply(&mut p)
        .unwrap();
        assert!(!p.clip(id).unwrap().1.freeze);
    }

    #[test]
    fn trimming_a_reversed_clip_keeps_the_frames_it_still_shows() {
        let (mut p, v1, _, _) = demo();
        let m = footage(&mut p, 100);
        let Outcome::Clip(id) = (Command::AddClip {
            track: v1,
            media: m,
            start: 1000,
            duration: Some(10),
            with_sound: false,
        })
        .apply(&mut p)
        .unwrap() else {
            panic!()
        };
        Command::SlipClip {
            clip: id,
            source_in: 20,
        }
        .apply(&mut p)
        .unwrap();
        Command::SetReverse {
            clip: id,
            reverse: true,
        }
        .apply(&mut p)
        .unwrap();
        let at = |p: &Project, f| p.clip(id).unwrap().1.source_frame_at(f);
        assert_eq!((at(&p, 1000), at(&p, 1009)), (29, 20));
        Command::TrimEnd {
            clip: id,
            end: 1007,
        }
        .apply(&mut p)
        .unwrap();
        assert_eq!((at(&p, 1000), at(&p, 1006)), (29, 23));
        Command::TrimStart {
            clip: id,
            start: 1002,
        }
        .apply(&mut p)
        .unwrap();
        assert_eq!((at(&p, 1002), at(&p, 1006)), (27, 23));
        Command::TrimEnd {
            clip: id,
            end: 1012,
        }
        .apply(&mut p)
        .unwrap();
        assert_eq!((at(&p, 1002), at(&p, 1011)), (27, 18));
    }

    #[test]
    fn keyframes_animate_and_survive_a_split() {
        use crate::KeyParam;
        let (mut p, _, _, [a, _, _]) = demo();
        let start = p.clip(a).unwrap().1.start;
        for (at, v) in [(0, 0.0), (80, 1.0)] {
            Command::SetKeyframe {
                clip: a,
                param: KeyParam::Opacity,
                at,
                value: Some(v),
                ease: false,
            }
            .apply(&mut p)
            .unwrap();
        }
        let c = p.clip(a).unwrap().1.clone();
        assert_eq!(c.opacity_at(start + 40), 0.5);
        assert_eq!(c.opacity_at(start + 90), 1.0);
        assert!(Command::SetKeyframe {
            clip: a,
            param: KeyParam::Opacity,
            at: 500,
            value: Some(1.0),
            ease: false,
        }
        .apply(&mut p)
        .is_err());
        // Cut in the middle: both halves show what the clip showed.
        let Outcome::Split(_, right) = Command::Split {
            clip: a,
            at: start + 20,
        }
        .apply(&mut p)
        .unwrap() else {
            panic!("split");
        };
        let r = p.clip(right).unwrap().1.clone();
        assert!((r.opacity_at(start + 40) - 0.5).abs() < 1e-6);
        assert!((p.clip(a).unwrap().1.opacity_at(start + 10) - 0.125).abs() < 1e-6);
        assert_eq!(Project::from_json(&p.to_json().unwrap()).unwrap(), p);
        Command::ClearKeyframes {
            clip: right,
            param: KeyParam::Opacity,
        }
        .apply(&mut p)
        .unwrap();
        assert!(p.clip(right).unwrap().1.keyframes.is_empty());
    }

    #[test]
    fn a_speed_ramp_cuts_the_clip_into_faster_pieces() {
        let (mut p, _, _, [a, b, _]) = demo();
        let (before, b_start) = {
            let (_, c) = p.clip(a).unwrap();
            (c.clone(), p.clip(b).unwrap().1.start)
        };
        Command::SpeedRamp {
            clip: a,
            speeds: vec![1.0, 2.0, 4.0],
        }
        .apply(&mut p)
        .unwrap();
        let pieces: Vec<_> = p.tracks[0]
            .clips
            .iter()
            .filter(|c| c.start < p.clip(b).unwrap().1.start)
            .cloned()
            .collect();
        assert_eq!(pieces.len(), 3);
        assert_eq!(pieces[0].id, a, "the first piece keeps the id");
        assert_eq!(
            pieces.iter().map(|c| c.speed).collect::<Vec<_>>(),
            vec![1.0, 2.0, 4.0]
        );
        // The same media, in order, without gaps.
        let span: u64 = pieces.iter().map(|c| c.source_span()).sum();
        assert_eq!(span, before.source_span());
        for w in pieces.windows(2) {
            assert_eq!(w[0].end(), w[1].start);
            assert_eq!(w[0].source_in + w[0].source_span(), w[1].source_in);
        }
        // 32 + 16 + 8 frames: the next clip moved up by 40.
        let new_end = pieces[2].end();
        assert_eq!(new_end, before.start + 56);
        assert_eq!(p.clip(b).unwrap().1.start, b_start - 40);
        assert!(Command::SpeedRamp {
            clip: b,
            speeds: vec![1.0]
        }
        .apply(&mut p)
        .is_err());
    }

    #[test]
    fn bins_keep_media_and_smart_bins_find_it() {
        let (mut p, _, _, _) = demo();
        Command::AddBin {
            name: "B-roll\n".into(),
        }
        .apply(&mut p)
        .unwrap();
        let bin = p.bins[0].clone();
        assert_eq!(bin.name, "B-roll");
        let media = p.media[0].id;
        Command::SetMediaBin {
            media,
            bin: Some(bin.id),
        }
        .apply(&mut p)
        .unwrap();
        assert_eq!(p.media[0].bin, Some(bin.id));
        Command::RenameBin {
            id: bin.id,
            name: "Shots".into(),
        }
        .apply(&mut p)
        .unwrap();
        assert_eq!(Project::from_json(&p.to_json().unwrap()).unwrap(), p);
        Command::DeleteBin { id: bin.id }.apply(&mut p).unwrap();
        assert!(
            p.bins.is_empty() && p.media[0].bin.is_none(),
            "back in Master"
        );
        assert_eq!(
            refused(
                &mut p,
                Command::SetMediaBin {
                    media,
                    bin: Some(999)
                }
            ),
            EditError::Invalid("there is no such bin")
        );
        Command::AddSmartBin {
            name: "Pictures".into(),
            contains: String::new(),
            kind: Some(crate::media_kind(&p.media[0].source).to_string()),
        }
        .apply(&mut p)
        .unwrap();
        assert!(p.smart_bins[0].matches(&p.media[0]));
    }

    #[test]
    fn stills_keep_a_grade_to_apply_to_another_clip() {
        let (mut p, _, _, [a, b, _]) = demo();
        let warm = ColorGrade {
            saturation: 1.4,
            ..Default::default()
        };
        let png = crate::base64::encode(&[1, 2, 3]);
        Command::GrabStill {
            name: "1.1.1".into(),
            frame: 10,
            grade: warm.clone(),
            nodes: vec![ColorGrade::default()],
            png: png.clone(),
        }
        .apply(&mut p)
        .unwrap();
        let id = p.stills[0].id;
        Command::ApplyStill { still: id, clip: b }
            .apply(&mut p)
            .unwrap();
        let (_, clip) = p.clip(b).unwrap();
        assert_eq!((clip.grade.saturation, clip.node_count()), (1.4, 2));
        assert_eq!(
            p.clip(a).unwrap().1.node_count(),
            1,
            "other clips keep theirs"
        );
        assert_eq!(Project::from_json(&p.to_json().unwrap()).unwrap(), p);
        assert_eq!(
            refused(
                &mut p,
                Command::GrabStill {
                    name: "x".into(),
                    frame: 0,
                    grade: warm,
                    nodes: vec![],
                    png: "@@@".into(),
                }
            ),
            EditError::Invalid("the still's picture")
        );
        Command::DeleteStill { id }.apply(&mut p).unwrap();
        assert!(p.stills.is_empty());
    }

    #[test]
    fn titles_are_kept_in_range_and_edited() {
        let (mut p, _, _, _) = demo();
        let long = crate::TitleText {
            text: "x".repeat(5000),
            size: 9999,
            x: 5000,
            ..Default::default()
        };
        let Outcome::Media(m) = (Command::AddMedia {
            name: "Text".into(),
            source: MediaSource::Title(long),
            length: None,
            default_duration: 120,
        })
        .apply(&mut p)
        .unwrap() else {
            panic!()
        };
        let MediaSource::Title(t) = &p.media(m).unwrap().source else {
            panic!()
        };
        assert_eq!(
            (t.text.len(), t.size, t.x),
            (crate::MAX_TITLE_CHARS, 500, 1000)
        );
        Command::SetTitle {
            media: m,
            title: crate::TitleText {
                text: "Hello".into(),
                ..Default::default()
            },
        }
        .apply(&mut p)
        .unwrap();
        assert!(matches!(&p.media(m).unwrap().source, MediaSource::Title(t) if t.text == "Hello"));
        let bars = p.media[0].id;
        assert_eq!(
            refused(
                &mut p,
                Command::SetTitle {
                    media: bars,
                    title: crate::TitleText::default()
                }
            ),
            EditError::Invalid("that media is not a title")
        );
        assert_eq!(
            refused(
                &mut p,
                Command::AddMedia {
                    name: "c".into(),
                    source: MediaSource::Compound { timeline: 1 },
                    length: Some(5),
                    default_duration: 5,
                }
            ),
            EditError::Invalid("a compound clip is made from clips")
        );
        assert_eq!(Project::from_json(&p.to_json().unwrap()).unwrap(), p);
    }

    #[test]
    fn transitions_need_a_neighbour_and_play_around_the_cut() {
        let (mut p, _, _, [a, b, _]) = demo();
        let dissolve = Some(crate::Transition {
            kind: crate::TransitionKind::CrossDissolve,
            frames: 24,
        });
        assert_eq!(
            refused(
                &mut p,
                Command::SetTransition {
                    clip: a,
                    transition: dissolve
                }
            ),
            EditError::Invalid("a transition needs a clip right before this one")
        );
        Command::SetTransition {
            clip: b,
            transition: dissolve,
        }
        .apply(&mut p)
        .unwrap();
        let t = &p.tracks[0];
        // Clip b starts at 96: the transition plays from 84 to 107.
        assert!(crate::transition_at(t, 83).is_none());
        let (l, r, _, start) = crate::transition_at(t, 84).unwrap();
        assert_eq!((l.id, r.id), (a, b));
        assert!(start < 0.05);
        let (_, _, _, middle) = crate::transition_at(t, 96).unwrap();
        assert!((middle - 0.52).abs() < 0.01, "{middle}");
        assert!(crate::transition_at(t, 107).is_some());
        assert!(crate::transition_at(t, 108).is_none());
        // Saved and opened again, it is kept.
        assert_eq!(Project::from_json(&p.to_json().unwrap()).unwrap(), p);
    }

    #[test]
    fn a_compound_clip_holds_clips_and_gives_them_back() {
        let (mut p, v1, _, [a, b, c]) = demo();
        let before = p.tracks[0].clips.clone();
        let Outcome::Clip(compound) = (Command::CreateCompound {
            clips: vec![b, c],
            name: "Ending".into(),
        })
        .apply(&mut p)
        .unwrap() else {
            panic!()
        };
        let (track, clip) = p.clip(compound).unwrap();
        assert_eq!((track.id, clip.start, clip.duration), (v1, 96, 192));
        assert_eq!(
            p.tracks[0].clips.len(),
            2,
            "the first clip and the compound clip"
        );
        assert!(p.clip(a).is_some() && p.clip(b).is_none());
        let item = p.media(clip.media).unwrap().clone();
        let MediaSource::Compound { timeline } = item.source else {
            panic!()
        };
        assert_eq!(p.timeline_tracks(timeline).unwrap()[0].clips[0].start, 0);
        assert_eq!(
            p.timeline_list().len(),
            1,
            "a compound's timeline is not listed"
        );
        assert_eq!(
            refused(&mut p, Command::SwitchTimeline { id: timeline }),
            EditError::Invalid("that timeline belongs to a compound clip")
        );
        // Saved and opened again, it is the same.
        assert_eq!(Project::from_json(&p.to_json().unwrap()).unwrap(), p);
        // Decomposed, the clips are back where they were, with new ids.
        Command::DecomposeCompound { clip: compound }
            .apply(&mut p)
            .unwrap();
        let spans = |clips: &[Clip]| {
            clips
                .iter()
                .map(|c| (c.start, c.duration, c.source_in, c.media))
                .collect::<Vec<_>>()
        };
        assert_eq!(spans(&p.tracks[0].clips), spans(&before));
        // The compound clip's media can then be removed, and its timeline goes with it.
        Command::RemoveMedia { media: item.id }
            .apply(&mut p)
            .unwrap();
        assert!(p.timeline_tracks(timeline).is_none());
        assert!(p.clone().validate().is_ok());
    }

    #[test]
    fn a_trimmed_compound_decomposes_to_what_it_shows() {
        let (mut p, _, _, [a, b, c]) = demo();
        let Outcome::Clip(compound) = (Command::CreateCompound {
            clips: vec![a, b, c],
            name: "All".into(),
        })
        .apply(&mut p)
        .unwrap() else {
            panic!()
        };
        Command::TrimStart {
            clip: compound,
            start: 50,
        }
        .apply(&mut p)
        .unwrap();
        Command::TrimEnd {
            clip: compound,
            end: 150,
        }
        .apply(&mut p)
        .unwrap();
        Command::DecomposeCompound { clip: compound }
            .apply(&mut p)
            .unwrap();
        let spans: Vec<(u64, u64, u64)> = layout(&p, 0);
        assert_eq!(spans, [(50, 46, 50), (96, 54, 0)]);
        assert_eq!(
            refused(
                &mut p,
                Command::CreateCompound {
                    clips: vec![],
                    name: "x".into()
                }
            ),
            EditError::Invalid("select clips first")
        );
    }

    #[test]
    fn timelines_are_kept_apart_and_switched() {
        let (mut p, _, _, _) = demo();
        let first = p.timeline_id;
        let clips = p.tracks[0].clips.len();
        let media = p.media[0].id;
        Command::NewTimeline {
            name: " Cut 2 ".into(),
        }
        .apply(&mut p)
        .unwrap();
        assert_eq!(p.timeline_name, "Cut 2");
        assert!(p.tracks.iter().all(|t| t.clips.is_empty()));
        assert_eq!(p.timeline_list().len(), 2);
        // Media used by a timeline that is not being edited is still in use.
        assert_eq!(
            refused(&mut p, Command::RemoveMedia { media }),
            EditError::MediaInUse
        );
        let second = p.timeline_id;
        Command::SwitchTimeline { id: first }.apply(&mut p).unwrap();
        assert_eq!(
            (p.timeline_name.as_str(), p.tracks[0].clips.len()),
            ("Timeline 1", clips)
        );
        Command::RenameTimeline {
            id: second,
            name: "Final".into(),
        }
        .apply(&mut p)
        .unwrap();
        assert_eq!(p.timeline_list()[1].1, "Final");
        // Saved and opened again, both are there.
        let mut back = Project::from_json(&p.to_json().unwrap()).unwrap();
        assert_eq!(back, p);
        Command::DeleteTimeline { id: first }
            .apply(&mut back)
            .unwrap();
        assert_eq!(back.timeline_name, "Final");
        assert_eq!(
            refused(&mut back, Command::DeleteTimeline { id: second }),
            EditError::Invalid("a project keeps at least one timeline")
        );
        assert_eq!(
            refused(&mut back, Command::NewTimeline { name: "  ".into() }),
            EditError::Invalid("the timeline name")
        );
    }

    #[test]
    fn a_gap_cuts_the_clip_there_and_moves_the_rest() {
        let (mut p, v1, _, _) = demo();
        Command::InsertGap {
            track: v1,
            at: 50,
            frames: 10,
        }
        .apply(&mut p)
        .unwrap();
        let spans: Vec<(u64, u64)> = layout(&p, 0).iter().map(|(s, d, _)| (*s, *d)).collect();
        assert_eq!(spans, [(0, 50), (60, 46), (106, 96), (202, 96)]);
    }

    #[test]
    fn slip_shows_other_footage_in_the_same_place() {
        let (mut p, v1, _, _) = demo();
        let m = footage(&mut p, 100);
        let Outcome::Clip(id) = (Command::AddClip {
            track: v1,
            media: m,
            start: 1000,
            duration: Some(40),
            with_sound: false,
        })
        .apply(&mut p)
        .unwrap() else {
            panic!()
        };
        Command::SlipClip {
            clip: id,
            source_in: 60,
        }
        .apply(&mut p)
        .unwrap();
        let c = p.clip(id).unwrap().1;
        assert_eq!((c.start, c.duration, c.source_in), (1000, 40, 60));
        assert_eq!(
            refused(
                &mut p,
                Command::SlipClip {
                    clip: id,
                    source_in: 61
                }
            ),
            EditError::OutOfRange,
            "the clip would run past the end of its footage"
        );
    }

    #[test]
    fn slide_moves_the_clip_between_its_neighbours() {
        let (mut p, _, _, [a, b, c]) = demo();
        assert_eq!(layout(&p, 0), [(0, 96, 0), (96, 96, 0), (192, 96, 0)]);
        Command::SlideClip {
            clip: b,
            start: 120,
        }
        .apply(&mut p)
        .unwrap();
        assert_eq!(
            layout(&p, 0),
            [(0, 120, 0), (120, 96, 0), (216, 72, 24)],
            "the left clip grows, the right one starts later and later in its media"
        );
        Command::SlideClip { clip: b, start: 50 }
            .apply(&mut p)
            .unwrap();
        assert_eq!(layout(&p, 0), [(0, 50, 0), (50, 96, 0), (146, 142, 0)]);
        assert_eq!(
            refused(&mut p, Command::SlideClip { clip: b, start: 0 }),
            EditError::Invalid("a clip needs at least one frame")
        );
        assert_eq!(
            refused(
                &mut p,
                Command::SlideClip {
                    clip: b,
                    start: 200
                }
            ),
            EditError::Invalid("a clip needs at least one frame")
        );
        let _ = (a, c);
    }

    #[test]
    fn roll_moves_the_cut_and_nothing_else() {
        let (mut p, _, _, [a, b, c]) = demo();
        // The demo media are generated stills, which can be held for any time.
        Command::RollTrim {
            left: a,
            right: b,
            to: 80,
        }
        .apply(&mut p)
        .unwrap();
        assert_eq!(
            layout(&p, 0),
            [(0, 80, 0), (80, 112, 0), (192, 96, 0)],
            "the second clip starts earlier; stills may hold before their first frame"
        );
        Command::RollTrim {
            left: a,
            right: b,
            to: 100,
        }
        .apply(&mut p)
        .unwrap();
        assert_eq!(layout(&p, 0), [(0, 100, 0), (100, 92, 20), (192, 96, 0)]);
        // Not neighbours, not on a cut, or no frame left.
        assert_eq!(
            refused(
                &mut p,
                Command::RollTrim {
                    left: a,
                    right: c,
                    to: 50
                }
            ),
            EditError::Invalid("the clips are not next to each other")
        );
        assert_eq!(
            refused(
                &mut p,
                Command::RollTrim {
                    left: a,
                    right: b,
                    to: 192
                }
            ),
            EditError::Invalid("a clip needs at least one frame")
        );
        // Footage cannot be rolled past its own ends.
        let (mut p, v1, _, _) = demo();
        let long = footage(&mut p, 500);
        let x = {
            let Outcome::Clip(x) = (Command::AddClip {
                track: v1,
                media: long,
                start: 400,
                duration: Some(50),
                with_sound: false,
            })
            .apply(&mut p)
            .unwrap() else {
                panic!()
            };
            x
        };
        let y = {
            let Outcome::Clip(y) = (Command::AddClip {
                track: v1,
                media: long,
                start: 450,
                duration: Some(50),
                with_sound: false,
            })
            .apply(&mut p)
            .unwrap() else {
                panic!()
            };
            y
        };
        // y starts at source frame 0: rolling the cut left needs footage before it.
        assert_eq!(
            refused(
                &mut p,
                Command::RollTrim {
                    left: x,
                    right: y,
                    to: 440
                }
            ),
            EditError::OutOfRange
        );
    }

    #[test]
    fn speed_changes_the_length_and_keeps_the_media_range() {
        let (mut p, v1, _, _) = demo();
        let m = footage(&mut p, 500);
        let Outcome::Clip(c) = (Command::AddClip {
            track: v1,
            media: m,
            start: 400,
            duration: Some(100),
            with_sound: false,
        })
        .apply(&mut p)
        .unwrap() else {
            panic!()
        };
        // Twice as fast: the same 100 frames of footage take 50 frames.
        Command::SetClipSpeed {
            clip: c,
            speed: 2.0,
        }
        .apply(&mut p)
        .unwrap();
        let clip = p.clip(c).unwrap().1;
        assert_eq!((clip.start, clip.duration, clip.speed), (400, 50, 2.0));
        assert_eq!(clip.source_span(), 100);
        assert_eq!(
            clip.source_frame_at(410),
            20,
            "10 frames in is 20 of footage"
        );
        // At twice the speed, 300 frames of timeline would need 600 frames of footage.
        assert_eq!(
            refused(&mut p, Command::TrimEnd { clip: c, end: 700 }),
            EditError::OutOfRange
        );
        // Splitting moves the second half's first frame by the speed.
        let Outcome::Split(l, r) = (Command::Split { clip: c, at: 420 }).apply(&mut p).unwrap()
        else {
            panic!()
        };
        assert_eq!(p.clip(l).unwrap().1.duration, 20);
        assert_eq!(p.clip(r).unwrap().1.source_in, 40, "20 frames at 2x");
        assert_eq!(p.clip(r).unwrap().1.speed, 2.0);
        // Not a usable speed.
        for bad in [0.0, -1.0, f32::NAN, 100.0] {
            assert_eq!(
                refused(
                    &mut p,
                    Command::SetClipSpeed {
                        clip: l,
                        speed: bad
                    }
                ),
                EditError::Invalid("the speed")
            );
        }
    }

    #[test]
    fn a_slower_clip_that_would_overlap_its_neighbour_is_refused() {
        let (mut p, _, _, [a, ..]) = demo();
        // The demo clips touch each other and are stills; half speed doubles the first clip.
        assert_eq!(
            refused(
                &mut p,
                Command::SetClipSpeed {
                    clip: a,
                    speed: 0.5
                }
            ),
            EditError::Overlap
        );
        Command::SetClipSpeed {
            clip: a,
            speed: 2.0,
        }
        .apply(&mut p)
        .unwrap();
        assert_eq!(p.clip(a).unwrap().1.duration, 48);
    }

    #[test]
    fn transforms_and_blends_are_stored_and_made_safe() {
        let (mut p, _, _, [a, ..]) = demo();
        Command::SetClipTransform {
            clip: a,
            transform: Transform {
                x: 0.25,
                scale_x: 1e9,
                scale_y: f32::NAN,
                rotation: 45.0,
                crop_left: 0.9,
                crop_right: 0.9,
                ..Transform::default()
            },
        }
        .apply(&mut p)
        .unwrap();
        let t = p.clip(a).unwrap().1.transform;
        assert_eq!(
            (t.x, t.scale_x, t.scale_y, t.rotation),
            (0.25, 16.0, 1.0, 45.0)
        );
        assert!(
            t.crop_left + t.crop_right <= 0.98 + 1e-6,
            "some picture is left"
        );
        assert!(!t.is_identity());
        Command::SetBlend {
            clip: a,
            blend: BlendMode::Screen,
        }
        .apply(&mut p)
        .unwrap();
        assert_eq!(p.clip(a).unwrap().1.blend, BlendMode::Screen);
        // It round-trips through a file, and files without these settings still open.
        assert_eq!(Project::from_json(&p.to_json().unwrap()).unwrap(), p);
        let mut json: serde_json::Value = serde_json::from_str(&p.to_json().unwrap()).unwrap();
        for tr in json["tracks"].as_array_mut().unwrap() {
            for c in tr["clips"].as_array_mut().unwrap() {
                let o = c.as_object_mut().unwrap();
                o.remove("speed");
                o.remove("transform");
                o.remove("blend");
            }
        }
        let old = Project::from_json(&json.to_string()).unwrap();
        let c = old.clip(a).unwrap().1;
        assert_eq!(
            (c.speed, c.blend, c.transform.is_identity()),
            (1.0, BlendMode::Normal, true)
        );
        // A hostile speed in a file is repaired.
        json["tracks"][0]["clips"][0]["speed"] = serde_json::json!(1e30);
        let fixed = Project::from_json(&json.to_string()).unwrap();
        assert_eq!(fixed.tracks[0].clips[0].speed, crate::MAX_SPEED);
    }

    // ----- linked sound: J-cuts and L-cuts -----

    /// A project with a 1000-frame video on V1 at frame 0 for 100 frames, placed with its sound,
    /// and the ids of the picture and its sound clip.
    fn linked() -> (Project, ClipId, ClipId) {
        let mut p = Project::new("t");
        let Outcome::Media(m) = (Command::AddMedia {
            name: "v".into(),
            source: MediaSource::Video {
                path: "v.mp4".into(),
            },
            length: Some(1000),
            default_duration: 100,
        })
        .apply(&mut p)
        .unwrap() else {
            panic!()
        };
        let v1 = p.tracks[0].id;
        let Outcome::Clip(picture) = (Command::AddClip {
            track: v1,
            media: m,
            start: 0,
            duration: None,
            with_sound: true,
        })
        .apply(&mut p)
        .unwrap() else {
            panic!()
        };
        let sound = p.linked_to(picture)[0];
        (p, picture, sound)
    }

    fn at(p: &Project, id: ClipId) -> (u64, u64, u64) {
        let (_, c) = p.clip(id).unwrap();
        (c.start, c.duration, c.source_in)
    }

    #[test]
    fn a_video_placed_with_sound_gets_a_linked_sound_clip_and_goes_quiet() {
        let (p, picture, sound) = linked();
        let (track, s) = p.clip(sound).unwrap();
        assert_eq!(track.kind, TrackKind::Audio);
        assert_eq!(track.name, "A1", "the first sound makes an audio track");
        assert_eq!(at(&p, sound), at(&p, picture));
        assert!(s.sound);
        assert!(!p.clip(picture).unwrap().1.sound, "heard once, from A1");
        assert_eq!(p.clip(picture).unwrap().1.link, s.link);
        assert!(s.link.is_some());
    }

    #[test]
    fn moving_one_moves_both_unless_linked_selection_is_off() {
        let (mut p, picture, sound) = linked();
        let v1 = p.tracks[0].id;
        Command::MoveClip {
            clip: picture,
            track: v1,
            start: 50,
        }
        .apply(&mut p)
        .unwrap();
        assert_eq!((at(&p, picture).0, at(&p, sound).0), (50, 50));
        Command::SetLinkedSelection { on: false }
            .apply(&mut p)
            .unwrap();
        Command::MoveClip {
            clip: picture,
            track: v1,
            start: 70,
        }
        .apply(&mut p)
        .unwrap();
        assert_eq!((at(&p, picture).0, at(&p, sound).0), (70, 50));
    }

    #[test]
    fn splitting_cuts_both_and_links_the_halves() {
        let (mut p, picture, sound) = linked();
        let Outcome::Split(_, right) = (Command::Split {
            clip: picture,
            at: 40,
        })
        .apply(&mut p)
        .unwrap() else {
            panic!()
        };
        assert_eq!(at(&p, sound), (0, 40, 0));
        let right_sound = p.linked_to(right);
        assert_eq!(right_sound.len(), 1);
        assert_eq!(at(&p, right_sound[0]), (40, 60, 40));
        assert_eq!(p.linked_to(picture), vec![sound], "left halves stay linked");
    }

    #[test]
    fn ripple_delete_takes_both_and_closes_both_gaps() {
        let (mut p, picture, _) = linked();
        let Outcome::Split(left, right) = (Command::Split {
            clip: picture,
            at: 40,
        })
        .apply(&mut p)
        .unwrap() else {
            panic!()
        };
        let right_sound = p.linked_to(right)[0];
        Command::RippleDelete { clip: left }.apply(&mut p).unwrap();
        assert_eq!(starts(&p, 0), vec![(0, 60)]);
        let a1 = p
            .tracks
            .iter()
            .position(|t| t.kind == TrackKind::Audio)
            .unwrap();
        assert_eq!(starts(&p, a1), vec![(0, 60)]);
        assert_eq!(at(&p, right_sound).0, 0);
    }

    #[test]
    fn a_j_cut_keeps_the_sound_edge_where_it_was() {
        let (mut p, picture, sound) = linked();
        // Trimming the start of both at once while the edges meet.
        Command::TrimStart {
            clip: picture,
            start: 20,
        }
        .apply(&mut p)
        .unwrap();
        assert_eq!((at(&p, picture).0, at(&p, sound).0), (20, 20));
        // J-cut: with Linked Selection off, the sound starts earlier than the picture.
        Command::SetLinkedSelection { on: false }
            .apply(&mut p)
            .unwrap();
        Command::TrimStart {
            clip: sound,
            start: 10,
        }
        .apply(&mut p)
        .unwrap();
        Command::SetLinkedSelection { on: true }
            .apply(&mut p)
            .unwrap();
        assert_eq!(at(&p, sound), (10, 90, 10));
        // Now trimming the picture's start leaves the sound's own edge alone...
        Command::TrimStart {
            clip: picture,
            start: 30,
        }
        .apply(&mut p)
        .unwrap();
        assert_eq!((at(&p, picture).0, at(&p, sound).0), (30, 10));
        // ...and moving still keeps the two together.
        let v1 = p.tracks[0].id;
        Command::MoveClip {
            clip: picture,
            track: v1,
            start: 40,
        }
        .apply(&mut p)
        .unwrap();
        assert_eq!((at(&p, picture).0, at(&p, sound).0), (40, 20));
    }

    #[test]
    fn an_insert_with_sound_pushes_the_audio_track_along_too() {
        let (mut p, picture, sound) = linked();
        let (v1, media) = (p.tracks[0].id, p.clip(picture).unwrap().1.media);
        let Outcome::Clip(new) = (Command::InsertClip {
            track: v1,
            media,
            start: 0,
            duration: Some(30),
            with_sound: true,
        })
        .apply(&mut p)
        .unwrap() else {
            panic!()
        };
        assert_eq!((at(&p, picture).0, at(&p, sound).0), (30, 30));
        assert_eq!(at(&p, p.linked_to(new)[0]), (0, 30, 0));
    }

    #[test]
    fn sound_settings_on_a_quiet_picture_go_to_its_sound_clip() {
        let (mut p, picture, sound) = linked();
        let audio = ClipAudio {
            gain_db: -6.0,
            fade_in: 5,
            fade_out: 0,
        };
        Command::SetClipAudio {
            clip: picture,
            audio,
        }
        .apply(&mut p)
        .unwrap();
        assert_eq!(p.clip(sound).unwrap().1.audio, audio);
    }

    #[test]
    fn link_and_unlink() {
        let (mut p, picture, sound) = linked();
        Command::UnlinkClips {
            clips: vec![picture],
        }
        .apply(&mut p)
        .unwrap();
        assert!(p.linked_to(picture).is_empty() && p.linked_to(sound).is_empty());
        refused(
            &mut p,
            Command::UnlinkClips {
                clips: vec![picture],
            },
        );
        refused(
            &mut p,
            Command::LinkClips {
                clips: vec![picture, picture],
            },
        );
        Command::LinkClips {
            clips: vec![picture, sound],
        }
        .apply(&mut p)
        .unwrap();
        assert_eq!(p.linked_to(picture), vec![sound]);
        // Unlinking kept the picture quiet, so the sound is still heard once.
        assert!(!p.clip(picture).unwrap().1.sound);
    }

    #[test]
    fn a_locked_audio_track_refuses_the_whole_linked_move() {
        let (mut p, picture, sound) = linked();
        let a1 = p.clip(sound).unwrap().0.id;
        Command::SetTrackLocked {
            track: a1,
            locked: true,
        }
        .apply(&mut p)
        .unwrap();
        let v1 = p.tracks[0].id;
        refused(
            &mut p,
            Command::MoveClip {
                clip: picture,
                track: v1,
                start: 10,
            },
        );
    }

    #[test]
    fn projects_from_before_links_still_open_with_their_sound() {
        let (p, picture, _) = linked();
        let mut json: serde_json::Value = serde_json::from_str(&p.to_json().unwrap()).unwrap();
        let map = json["tracks"][0]["clips"][0].as_object_mut().unwrap();
        map.remove("link");
        map.remove("sound");
        json.as_object_mut().unwrap().remove("linked_selection");
        let old = Project::from_json(&json.to_string()).unwrap();
        let (_, c) = old.clip(picture).unwrap();
        assert!(c.sound && c.link.is_none());
        assert!(old.linked_selection);
    }

    #[test]
    fn projects_from_before_solo_and_picture_still_open() {
        let p = Project::new("t");
        let mut json: serde_json::Value = serde_json::from_str(&p.to_json().unwrap()).unwrap();
        for track in json["tracks"].as_array_mut().unwrap() {
            let track = track.as_object_mut().unwrap();
            track.remove("solo");
            track.remove("picture");
        }
        let old = Project::from_json(&json.to_string()).unwrap();
        for track in &old.tracks {
            assert!(!track.solo, "no track starts soloed");
            assert!(track.picture, "every track shows its picture");
        }
        assert!(!old.any_soloed());
    }

    // ----- serial colour nodes -----

    fn first_clip(p: &Project) -> ClipId {
        p.tracks[0].clips[0].id
    }

    fn nodes(p: &Project) -> &Clip {
        p.clip(first_clip(p)).unwrap().1
    }

    #[test]
    fn parallel_nodes_add_their_change_and_layers_mix() {
        use picasso_color::NodeMode;
        let (mut p, ..) = demo();
        let clip = first_clip(&p);
        let grey = |c: &Clip| {
            let mut px = [100u8, 100, 100, 255];
            c.apply_nodes(&mut px, 1, 1);
            px[0]
        };
        let lift = |v: f32, mode, mix| ColorGrade {
            offset: [v, v, v],
            mode,
            mix,
            ..ColorGrade::default()
        };
        // Two serial +0.1 offsets add up; a parallel one with the same input adds too.
        for (i, g) in [
            lift(0.1, NodeMode::Serial, 100.0),
            lift(0.1, NodeMode::Parallel, 100.0),
        ]
        .into_iter()
        .enumerate()
        {
            if i > 0 {
                Command::AddSerialNode { clip, after: i - 1 }
                    .apply(&mut p)
                    .unwrap();
            }
            Command::SetGrade {
                clip,
                grade: g,
                node: i,
            }
            .apply(&mut p)
            .unwrap();
        }
        let both = grey(nodes(&p));
        assert!((i32::from(both) - 151).abs() <= 2, "{both}");
        // A layer at half strength brings half of its change (126 + 51 / 2).
        Command::SetGrade {
            clip,
            grade: lift(0.2, NodeMode::Layer, 50.0),
            node: 1,
        }
        .apply(&mut p)
        .unwrap();
        let layered = grey(nodes(&p));
        assert!((i32::from(layered) - 151).abs() <= 2, "{layered}");
    }

    #[test]
    fn switching_angle_cuts_and_keeps_the_sync_moment() {
        let mut p = Project::new("multi");
        let mut media = Vec::new();
        for (n, offset) in [("A", 0i64), ("B", 24)] {
            let id = MediaId(p.fresh_id());
            p.media.push(MediaItem {
                id,
                name: n.into(),
                source: MediaSource::Video {
                    path: format!("{n}.mp4"),
                },
                length: Some(1000),
                default_duration: 240,
                bin: None,
                timecode: None,
            });
            media.push((id, offset));
        }
        Command::CreateMulticam {
            name: "Cams".into(),
            angles: media.clone(),
        }
        .apply(&mut p)
        .unwrap();
        let mc = p.multicams[0].id;
        let v1 = p.tracks[0].id;
        let Outcome::Clip(clip) = (Command::AddClip {
            track: v1,
            media: media[0].0,
            start: 100,
            duration: Some(240),
            with_sound: false,
        })
        .apply(&mut p)
        .unwrap() else {
            panic!("no clip")
        };
        // Camera B shows the sync moment at its frame 24.
        let Outcome::Clip(second) = (Command::SwitchAngle {
            multicam: mc,
            angle: 1,
            at: 150,
        })
        .apply(&mut p)
        .unwrap() else {
            panic!("no clip")
        };
        let clips = &p.tracks[0].clips;
        assert_eq!(clips.len(), 2);
        assert_eq!((clips[0].id, clips[0].duration), (clip, 50));
        let b = clips.iter().find(|c| c.id == second).unwrap();
        assert_eq!((b.media, b.start, b.duration), (media[1].0, 150, 190));
        // Frame 150 is 50 frames into A (source 50), so B shows its frame 50 + 24.
        assert_eq!(b.source_in, 74);
        // Switching to the angle already shown changes nothing; a missing one is refused.
        let before = p.clone();
        Command::SwitchAngle {
            multicam: mc,
            angle: 1,
            at: 200,
        }
        .apply(&mut p)
        .unwrap();
        assert_eq!(p, before);
        let mut q = p.clone();
        refused(
            &mut q,
            Command::SwitchAngle {
                multicam: mc,
                angle: 5,
                at: 200,
            },
        );
        // Media of a multicam clip stays while the clip exists.
        refused(&mut q, Command::RemoveMedia { media: media[1].0 });
    }

    #[test]
    fn multicam_clips_need_two_distinct_video_angles() {
        let (mut p, ..) = demo();
        let m = p.media[0].id;
        refused(
            &mut p,
            Command::CreateMulticam {
                name: "x".into(),
                angles: vec![(m, 0)],
            },
        );
        refused(
            &mut p,
            Command::CreateMulticam {
                name: "x".into(),
                angles: vec![(m, 0), (m, 0)],
            },
        );
        // The demo's media are generators, not video.
        let n = p.media[1].id;
        refused(
            &mut p,
            Command::CreateMulticam {
                name: "x".into(),
                angles: vec![(m, 0), (n, 0)],
            },
        );
    }

    #[test]
    fn rendering_in_place_swaps_the_footage_and_drops_what_was_baked() {
        let (mut p, _, _, [a, ..]) = demo();
        let media = MediaId(p.fresh_id());
        p.media.push(MediaItem {
            id: media,
            name: "Render".into(),
            source: MediaSource::Video {
                path: "r.mov".into(),
            },
            length: Some(96),
            default_duration: 96,
            bin: None,
            timecode: None,
        });
        Command::SetClipSpeed {
            clip: a,
            speed: 1.0,
        }
        .apply(&mut p)
        .unwrap();
        Command::SetOpacity {
            clip: a,
            opacity: 0.5,
        }
        .apply(&mut p)
        .unwrap();
        Command::SetGrade {
            clip: a,
            grade: ColorGrade {
                gain: [2.0, 1.0, 1.0],
                ..ColorGrade::default()
            },
            node: 0,
        }
        .apply(&mut p)
        .unwrap();
        Command::RenderInPlace { clip: a, media }
            .apply(&mut p)
            .unwrap();
        let c = p.clip(a).unwrap().1;
        assert_eq!((c.media, c.source_in, c.speed), (media, 0, 1.0));
        assert!(!c.is_graded());
        assert_eq!(c.opacity, 0.5, "opacity stays a live control");
        // A render shorter than the clip is refused.
        let short = MediaId(p.fresh_id());
        p.media.push(MediaItem {
            id: short,
            name: "Short".into(),
            source: MediaSource::Video {
                path: "s.mov".into(),
            },
            length: Some(10),
            default_duration: 10,
            bin: None,
            timecode: None,
        });
        refused(
            &mut p,
            Command::RenderInPlace {
                clip: a,
                media: short,
            },
        );
    }

    #[test]
    fn clips_can_be_renamed() {
        let (mut p, _, _, [a, ..]) = demo();
        Command::RenameClip {
            clip: a,
            name: "  Hero shot ".into(),
        }
        .apply(&mut p)
        .unwrap();
        assert_eq!(p.clip(a).unwrap().1.name, "Hero shot");
        refused(
            &mut p,
            Command::RenameClip {
                clip: a,
                name: "  ".into(),
            },
        );
    }

    #[test]
    fn serial_nodes_are_added_after_the_current_one_and_deleted() {
        let (mut p, ..) = demo();
        let clip = first_clip(&p);
        assert_eq!(nodes(&p).node_count(), 1);
        Command::AddSerialNode { clip, after: 0 }
            .apply(&mut p)
            .unwrap();
        let red = ColorGrade {
            gain: [2.0, 1.0, 1.0],
            ..ColorGrade::default()
        };
        Command::SetGrade {
            clip,
            grade: red.clone(),
            node: 1,
        }
        .apply(&mut p)
        .unwrap();
        // A node between 01 and the red one.
        Command::AddSerialNode { clip, after: 0 }
            .apply(&mut p)
            .unwrap();
        assert_eq!(nodes(&p).node_count(), 3);
        assert_eq!(nodes(&p).node(2), Some(&red), "the red node moved to 03");
        assert!(nodes(&p).node(1).unwrap().is_identity());
        Command::DeleteNode { clip, node: 1 }.apply(&mut p).unwrap();
        assert_eq!(nodes(&p).node(1), Some(&red));
        // Deleting node 01 makes the next one the first.
        Command::DeleteNode { clip, node: 0 }.apply(&mut p).unwrap();
        assert_eq!(nodes(&p).grade, red);
        refused(&mut p, Command::DeleteNode { clip, node: 0 });
        refused(&mut p, Command::AddSerialNode { clip, after: 5 });
        refused(
            &mut p,
            Command::SetGrade {
                clip,
                grade: ColorGrade::default(),
                node: 3,
            },
        );
    }

    #[test]
    fn a_clip_has_at_most_sixteen_nodes() {
        let (mut p, ..) = demo();
        let clip = first_clip(&p);
        for _ in 1..MAX_NODES {
            Command::AddSerialNode { clip, after: 0 }
                .apply(&mut p)
                .unwrap();
        }
        assert_eq!(nodes(&p).node_count(), MAX_NODES);
        refused(&mut p, Command::AddSerialNode { clip, after: 0 });
    }

    #[test]
    fn nodes_apply_in_order_and_a_switched_off_node_does_nothing() {
        let (mut p, ..) = demo();
        let clip = first_clip(&p);
        Command::AddSerialNode { clip, after: 0 }
            .apply(&mut p)
            .unwrap();
        // Node 01 doubles, node 02 adds a tenth: order matters (0.2 -> 0.4 -> 0.5).
        let double = ColorGrade {
            gain: [2.0; 3],
            ..ColorGrade::default()
        };
        let lift = ColorGrade {
            offset: [0.1; 3],
            ..ColorGrade::default()
        };
        for (node, grade) in [(0, double), (1, lift)] {
            Command::SetGrade { clip, grade, node }
                .apply(&mut p)
                .unwrap();
        }
        let grey = |p: &Project| {
            let mut px = [51u8, 51, 51, 255];
            nodes(p).apply_nodes(&mut px, 1, 1);
            px[0]
        };
        assert_eq!(grey(&p), 128, "0.2 doubled then lifted by 0.1");
        assert!(nodes(&p).is_graded());
        Command::SetNodeEnabled {
            clip,
            node: 1,
            enabled: false,
        }
        .apply(&mut p)
        .unwrap();
        assert_eq!(grey(&p), 102, "only the doubling is left");
        Command::SetNodeEnabled {
            clip,
            node: 0,
            enabled: false,
        }
        .apply(&mut p)
        .unwrap();
        assert_eq!(grey(&p), 51);
        assert!(!nodes(&p).is_graded(), "settings kept, but nothing changes");
        assert!(!nodes(&p).grade.is_identity());
        Command::ResetNodes { clip }.apply(&mut p).unwrap();
        assert_eq!(nodes(&p).node_count(), 1);
        assert_eq!(nodes(&p).grade, ColorGrade::default());
    }

    #[test]
    fn nodes_survive_saving_and_old_files_have_one_node() {
        let (mut p, ..) = demo();
        let clip = first_clip(&p);
        Command::AddSerialNode { clip, after: 0 }
            .apply(&mut p)
            .unwrap();
        Command::SetNodeEnabled {
            clip,
            node: 1,
            enabled: false,
        }
        .apply(&mut p)
        .unwrap();
        let back = Project::from_json(&p.to_json().unwrap()).unwrap();
        assert_eq!(back, p);
        let mut json: serde_json::Value = serde_json::from_str(&p.to_json().unwrap()).unwrap();
        let map = json["tracks"][0]["clips"][0].as_object_mut().unwrap();
        map.remove("nodes");
        map["grade"].as_object_mut().unwrap().remove("enabled");
        let old = Project::from_json(&json.to_string()).unwrap();
        assert_eq!(nodes(&old).node_count(), 1);
        assert!(nodes(&old).grade.enabled);
    }

    #[test]
    fn a_file_with_too_many_nodes_is_cut_down() {
        let (p, ..) = demo();
        let mut json: serde_json::Value = serde_json::from_str(&p.to_json().unwrap()).unwrap();
        let node = serde_json::to_value(ColorGrade::default()).unwrap();
        json["tracks"][0]["clips"][0]["nodes"] = serde_json::Value::Array(vec![node; 500]);
        let read = Project::from_json(&json.to_string()).unwrap();
        assert_eq!(nodes(&read).node_count(), MAX_NODES);
    }

    /// A media item of a colour with a timecode, and a clip of it on `track`.
    fn timed_clip(
        p: &mut Project,
        track: TrackId,
        tc: Option<&str>,
        start: u64,
        source_in: u64,
    ) -> ClipId {
        let Outcome::Media(media) = (Command::AddMedia {
            name: "m".into(),
            source: MediaSource::ColorBars,
            length: Some(1000),
            default_duration: 100,
        })
        .apply(p)
        .unwrap() else {
            panic!("expected media")
        };
        Command::SetMediaTimecode {
            media,
            timecode: tc.map(str::to_string),
        }
        .apply(p)
        .unwrap();
        let Outcome::Clip(id) = (Command::AddClip {
            track,
            media,
            start,
            duration: Some(100),
            with_sound: false,
        })
        .apply(p)
        .unwrap() else {
            panic!("expected a clip")
        };
        if source_in > 0 {
            Command::SlipClip {
                clip: id,
                source_in,
            }
            .apply(p)
            .unwrap();
        }
        id
    }

    #[test]
    fn clips_align_to_the_earliest_timecode() {
        let mut p = Project::new("align");
        let v1 = p.tracks[0].id;
        let v2 = p.tracks[1].id;
        let early = timed_clip(&mut p, v1, Some("01:00:00:00"), 500, 0);
        let late = timed_clip(&mut p, v2, Some("01:00:01:00"), 10, 0);
        Command::AlignClips {
            clips: vec![early, late],
        }
        .apply(&mut p)
        .unwrap();
        // The earliest clip stays; the later one moves a second (24 frames) after it.
        assert_eq!(p.clip(early).unwrap().1.start, 500);
        assert_eq!(p.clip(late).unwrap().1.start, 524);
    }

    #[test]
    fn source_in_counts_toward_a_clips_timecode() {
        let mut p = Project::new("align");
        let v1 = p.tracks[0].id;
        let v2 = p.tracks[1].id;
        // The second clip's media starts a second earlier, but it is slipped a second forward,
        // so the frames both clips show begin at the same timecode.
        let a = timed_clip(&mut p, v1, Some("01:00:01:00"), 100, 0);
        let b = timed_clip(&mut p, v2, Some("01:00:00:00"), 400, 24);
        Command::AlignClips { clips: vec![a, b] }
            .apply(&mut p)
            .unwrap();
        assert_eq!(p.clip(a).unwrap().1.start, 100);
        assert_eq!(p.clip(b).unwrap().1.start, 100);
    }

    #[test]
    fn aligning_takes_the_linked_sound_with_the_picture() {
        let mut p = Project::new("align");
        let v1 = p.tracks[0].id;
        let v2 = p.tracks[1].id;
        // Two video clips with their own sound, the second a second later in its file.
        let clip = |p: &mut Project, track: TrackId, tc: &str, start: u64| {
            let Outcome::Media(media) = (Command::AddMedia {
                name: "m".into(),
                source: MediaSource::Video {
                    path: "/media/m.mov".into(),
                },
                length: Some(1000),
                default_duration: 100,
            })
            .apply(p)
            .unwrap() else {
                panic!("expected media")
            };
            Command::SetMediaTimecode {
                media,
                timecode: Some(tc.to_string()),
            }
            .apply(p)
            .unwrap();
            let Outcome::Clip(id) = (Command::AddClip {
                track,
                media,
                start,
                duration: Some(100),
                with_sound: true,
            })
            .apply(p)
            .unwrap() else {
                panic!("expected a clip")
            };
            id
        };
        let early = clip(&mut p, v1, "01:00:00:00", 0);
        let late = clip(&mut p, v2, "01:00:01:00", 0);
        // The sound of the late clip, which has to travel with it.
        let sound = p.linked_to(late);
        assert_eq!(sound.len(), 1, "the picture has its own sound");
        let (_, before) = p.clip(sound[0]).unwrap();
        assert_eq!(before.start, 0);
        Command::AlignClips {
            clips: vec![early, late],
        }
        .apply(&mut p)
        .unwrap();
        let (_, picture) = p.clip(late).unwrap();
        let (_, sound) = p.clip(sound[0]).unwrap();
        assert_eq!(picture.start, 24, "a second later than the early clip");
        assert_eq!(sound.start, picture.start, "the sound moved with the picture");
    }

    #[test]
    fn aligning_needs_two_clips_with_timecode() {
        let mut p = Project::new("align");
        let v1 = p.tracks[0].id;
        let v2 = p.tracks[1].id;
        let a = timed_clip(&mut p, v1, Some("01:00:00:00"), 0, 0);
        let b = timed_clip(&mut p, v2, None, 0, 0);
        assert_eq!(
            refused(&mut p, Command::AlignClips { clips: vec![a, b] }),
            EditError::Invalid("a clip's media has no timecode")
        );
        assert_eq!(
            refused(&mut p, Command::AlignClips { clips: vec![a] }),
            EditError::Invalid("aligning needs two clips or more")
        );
    }

    #[test]
    fn clips_are_placed_where_the_sound_says_and_the_linked_sound_goes_along() {
        let mut p = Project::new("place");
        let v1 = p.tracks[0].id;
        let v2 = p.tracks[1].id;
        let a = timed_clip(&mut p, v1, None, 100, 0);
        let b = timed_clip(&mut p, v2, None, 400, 0);
        Command::PlaceClips {
            starts: vec![(a, 100), (b, 130)],
        }
        .apply(&mut p)
        .unwrap();
        assert_eq!(p.clip(a).unwrap().1.start, 100);
        assert_eq!(p.clip(b).unwrap().1.start, 130);
        // One undo step brings it back.
        let mut q = Project::new("place");
        let (w1, w2) = (q.tracks[0].id, q.tracks[1].id);
        let a = timed_clip(&mut q, w1, None, 0, 0);
        let b = timed_clip(&mut q, w2, None, 50, 0);
        let mut session = crate::session::Session::new(q);
        session
            .dispatch(Command::PlaceClips {
                starts: vec![(a, 0), (b, 10)],
            })
            .unwrap();
        assert_eq!(session.project().clip(b).unwrap().1.start, 10);
        assert!(session.undo());
        assert_eq!(session.project().clip(b).unwrap().1.start, 50);
    }

    #[test]
    fn placing_needs_two_different_clips_and_overlaps_are_refused() {
        let mut p = Project::new("place");
        let v1 = p.tracks[0].id;
        let a = timed_clip(&mut p, v1, None, 0, 0);
        let b = timed_clip(&mut p, v1, None, 200, 0);
        assert_eq!(
            refused(&mut p, Command::PlaceClips { starts: vec![(a, 5)] }),
            EditError::Invalid("placing needs two clips or more, each once")
        );
        assert_eq!(
            refused(&mut p, Command::PlaceClips { starts: vec![(a, 5), (a, 9)] }),
            EditError::Invalid("placing needs two clips or more, each once")
        );
        // On one track the second would land on the first: nothing moves at all.
        assert_eq!(
            refused(&mut p, Command::PlaceClips { starts: vec![(a, 0), (b, 10)] }),
            EditError::Overlap
        );
        assert_eq!(p.clip(a).unwrap().1.start, 0);
        assert_eq!(p.clip(b).unwrap().1.start, 200);
    }

    #[test]
    fn relinking_points_every_clip_of_the_media_at_another_file() {
        let mut p = Project::new("relink");
        let v1 = p.tracks[0].id;
        let Outcome::Media(media) = (Command::AddMedia {
            name: "shot".into(),
            source: MediaSource::Video {
                path: "/cards/a/shot.mov".into(),
            },
            length: Some(100),
            default_duration: 50,
        })
        .apply(&mut p)
        .unwrap() else {
            panic!()
        };
        let Outcome::Clip(clip) = (Command::AddClip {
            track: v1,
            media,
            start: 0,
            duration: Some(50),
            with_sound: false,
        })
        .apply(&mut p)
        .unwrap() else {
            panic!()
        };
        Command::RelinkMedia {
            media,
            path: "  /backup/shot.mov  ".into(),
        }
        .apply(&mut p)
        .unwrap();
        let moved = MediaSource::Video {
            path: "/backup/shot.mov".into(),
        };
        assert_eq!(p.media(media).unwrap().source, moved, "trimmed and kept");
        // A relink changes no timing: the clip still shows that media where it was.
        let (_, c) = p.clip(clip).unwrap();
        assert_eq!((c.media, c.start, c.duration), (media, 0, 50));
        // Saved and opened again, the new path is what is kept.
        let again = Project::from_json(&p.to_json().unwrap()).unwrap();
        assert_eq!(again.media(media).unwrap().source, moved);
    }

    #[test]
    fn relinking_refuses_generated_media_and_bad_paths() {
        let mut p = Project::new("relink");
        let Outcome::Media(title) = (Command::AddMedia {
            name: "Title".into(),
            source: MediaSource::Title(crate::TitleText::default()),
            length: None,
            default_duration: 48,
        })
        .apply(&mut p)
        .unwrap() else {
            panic!()
        };
        assert_eq!(
            refused(
                &mut p,
                Command::RelinkMedia {
                    media: title,
                    path: "/x.mov".into(),
                }
            ),
            EditError::Invalid("that media is made by the program and has no file to relink")
        );
        let Outcome::Media(video) = (Command::AddMedia {
            name: "v".into(),
            source: MediaSource::Video {
                path: "/a.mov".into(),
            },
            length: Some(10),
            default_duration: 10,
        })
        .apply(&mut p)
        .unwrap() else {
            panic!()
        };
        assert_eq!(
            refused(
                &mut p,
                Command::RelinkMedia {
                    media: video,
                    path: "   ".into(),
                }
            ),
            EditError::Invalid("the file to relink to")
        );
        assert_eq!(
            refused(
                &mut p,
                Command::RelinkMedia {
                    media: MediaId(9999),
                    path: "/x.mov".into(),
                }
            ),
            EditError::NoSuchMedia
        );
    }

    #[test]
    fn an_effects_clip_is_placed_above_clips_and_is_transparent() {
        let (mut p, _, _, _) = demo();
        let timeline_before = p.timelines.len();
        let Outcome::Clip(clip) = (Command::CreateEffectsClip {
            start: 0,
            name: "Effects Clip 1".into(),
        })
        .apply(&mut p)
        .unwrap() else {
            panic!()
        };
        // The clip is on its own track, above the video tracks.
        let (track, c) = p.clip(clip).unwrap();
        assert_eq!(c.name, "Effects Clip 1");
        assert_eq!(c.start, 0);
        assert!(c.duration > 0);
        assert!(track.kind == TrackKind::Video);
        let item = p.media(c.media).unwrap();
        assert_eq!(item.source, MediaSource::EffectsClip);
        // It does not add a timeline (unlike a compound clip).
        assert_eq!(p.timelines.len(), timeline_before);
        // It is listed as an effects clip, not a generator.
        assert_eq!(crate::media_kind(&item.source), "Effects Clip");
        // Saved and opened again, it is the same.
        assert_eq!(Project::from_json(&p.to_json().unwrap()).unwrap(), p);
    }

    #[test]
    fn an_effects_clip_goes_where_it_fits_for_all_its_frames() {
        let (mut p, _, v2, _) = demo();
        let media = p.media[0].id;
        let fps = p.fps.nominal();
        // A clip later on V2, inside the span the effects clip will take.
        Command::AddClip {
            track: v2,
            media,
            start: fps * 8,
            duration: None,
            with_sound: false,
        }
        .apply(&mut p)
        .unwrap();
        let Outcome::Clip(clip) = (Command::CreateEffectsClip {
            start: 0,
            name: "Effects Clip 1".into(),
        })
        .apply(&mut p)
        .unwrap() else {
            panic!("an effects clip should still be made")
        };
        let (track, c) = p.clip(clip).unwrap();
        assert_ne!(track.id, v2, "V2 is busy later on");
        assert!(
            track
                .clips
                .iter()
                .all(|o| o.id == c.id || o.end() <= c.start || o.start >= c.end()),
            "it overlaps nothing on its track"
        );
    }

    #[test]
    fn an_effects_clip_name_is_validated() {
        let (mut p, _, _, _) = demo();
        refused(
            &mut p,
            Command::CreateEffectsClip {
                start: 0,
                name: "".into(),
            },
        );
    }
}
