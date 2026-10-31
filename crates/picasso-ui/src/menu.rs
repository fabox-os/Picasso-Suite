//! The menus, as data: one table draws them, binds their keys and lists the shortcuts.
//!
//! The structure follows `Menu-Spec.md`. An entry without an [`Act`] is planned: it is shown
//! (disabled, with its key as the spec gives it) but does nothing and never takes a key, so a key
//! two entries share always belongs to the one that works.
//!
//! Keyboard Customization can give any act other keys; those replace the table's keys of that act
//! everywhere (the menus, the shortcut list and what the keys do).

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use egui::{Key, KeyboardShortcut, Modifiers};
use picasso_core::EffectKind;

/// Everything a menu entry or a key can do.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Act {
    // File
    ProjectManager,
    NewProject,
    OpenProject,
    SaveProject,
    SaveProjectAs,
    SaveCopy,
    Revert,
    ImportMedia,
    ProjectSettings,
    MediaManagement,
    QuickExport,
    Quit,
    // Edit
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    Duplicate,
    DeleteSelected,
    RippleDeleteSelected,
    SelectAll,
    DeselectAll,
    SelectForward,
    ShowShortcuts,
    FitToFill,
    PlaceOnTop,
    PasteInsert,
    PasteMix,
    // Trim
    RippleStart,
    RippleEnd,
    TrimStartToPlayhead,
    TrimEndToPlayhead,
    ExtendEdit,
    SlipLeft,
    SlipRight,
    SlideLeft,
    SlideRight,
    // Timeline
    NewTimeline,
    SwitchTimeline,
    DeleteTimeline,
    CreateCompound,
    CreateEffectsClip,
    DecomposeCompound,
    AddCrossDissolve,
    AddDipToBlack,
    AddCrossFade,
    AddTransitionToAll,
    AddTitle,
    GrabStill,
    PlayStill,
    RemoveTransition,
    SplitClip,
    ToggleSnap,
    // Clip
    ToggleEnableClip,
    FreezeFrame,
    ReverseClip,
    // Mark
    MarkIn,
    MarkOut,
    MarkClip,
    ClearIn,
    ClearOut,
    ClearInOut,
    AddMarker,
    AddMarkerDialog,
    ModifyMarker,
    DeleteMarkerHere,
    DeleteAllMarkers,
    PreviousMarker,
    NextMarker,
    // View
    FullScreenViewer,
    CinemaViewer,
    OverlaySafeArea,
    OverlayCrosshair,
    OverlayCheckerboard,
    OverlayGrid,
    OverlayAll,
    ShowScopes,
    // Playback
    TogglePlay,
    PlayForward,
    PlayReverse,
    Stop,
    PlayAround,
    PlayInToOut,
    ToggleLoop,
    ProxyOff,
    ProxyHalf,
    ProxyQuarter,
    ProxyEighth,
    ToggleOptimisedMedia,
    ToggleHardwareDecoding,
    DeleteRenderCache,
    LinkClips,
    UnlinkClips,
    ToggleLinkedSelection,
    ToggleAudioScrubbing,
    AddSerialNode,
    ToggleNode,
    GoFirst,
    GoLast,
    GoIn,
    GoOut,
    PreviousFrame,
    NextFrame,
    PreviousEdit,
    NextEdit,
    SecondBack,
    SecondForward,
    // Effects
    GenerateSolid,
    GenerateBars,
    GenerateGradient,
    // Color
    ResetAllGrades,
    ResetCurrentGrade,
    ApplyLut,
    ToggleScopes,
    // Workspace
    ResetLayout,
    SaveLayout,
    LoadLayout,
    PageMedia,
    PageCut,
    PageEdit,
    PageColor,
    PageDeliver,
    PageAudio,
    // Added entries
    ToggleHighlight,
    MarkTimeline,
    SelectNearestEdit,
    PositionLock,
    SelectionFollowsPlayhead,
    PageEffects,
    ShowInspector,
    ShowMixer,
    ShowMediaPool,
    ShowEditIndex,
    TimecodeWindow,
    PlayAroundMark,
    ChangeClipSpeed,
    ChangeClipDuration,
    PreviousNodeGraph,
    NextNodeGraph,
    AddNodeBefore,
    RenderInPlace,
    ClipAttributes,
    DynamicTrim,
    LoopTrim,
    AsymmetricTrim,
    AddOutsideNode,
    TimelineFromBin,
    BounceMix,
    BounceSelectedTracks,
    NewMulticam,
    MulticamViewer,
    AddParallelNode,
    AddLayerNode,
    LabelNode,
    TimelineFromSelection,
    Manual,
    LockClip,
    SetClipColor(Option<u8>),
    NormalizeAudio,
    AutoAlignTimecode,
    AutoAlignWaveform,
    ExportEdl,
    ExportOtio,
    /// The `xmeml` timeline format (true: named for the Premiere XML render preset, which
    /// writes the same file).
    ExportXml(bool),
    ExportAaf,
    ImportTimeline,
    ExportSubtitles,
    CreateSubtitleTrack,
    CreateSubtitleRegion,
    StillToPool,
    FolderAsBin,
    MatchFrame,
    TrimMode,
    SelectionMode,
    SpeedRamp,
    AddTextPlus,
    AddEffect(picasso_core::EffectKind),
    TrackEq,
    TrackDynamics,
    Preferences,
    // Help
    About,
    CheckForUpdates,
    /// Saves this session's log (see [`crate::session_log`]).
    ExportLog,
    /// Shows the menus in this language (Help > Language), kept for the next start.
    Language(crate::i18n::Language),
    // Not in a menu
    LeaveViewer,
}

pub(crate) enum Entry {
    /// `shown` replaces the keys in the menu when they are not the whole story (e.g. "Space / L").
    Item {
        label: &'static str,
        shown: &'static str,
        keys: &'static [&'static str],
        act: Option<Act>,
    },
    Sep,
    Sub {
        label: &'static str,
        items: &'static [Entry],
    },
}

const SEP: Entry = Entry::Sep;

const fn item(label: &'static str, keys: &'static [&'static str], act: Act) -> Entry {
    Entry::Item {
        label,
        shown: "",
        keys,
        act: Some(act),
    }
}

const fn item_shown(
    label: &'static str,
    shown: &'static str,
    keys: &'static [&'static str],
    act: Act,
) -> Entry {
    Entry::Item {
        label,
        shown,
        keys,
        act: Some(act),
    }
}

/// An entry that is not built yet; `shown` is the key the spec gives it.
const fn planned(label: &'static str, shown: &'static str) -> Entry {
    Entry::Item {
        label,
        shown,
        keys: &[],
        act: None,
    }
}

const fn sub(label: &'static str, items: &'static [Entry]) -> Entry {
    Entry::Sub { label, items }
}

const FILE: &[Entry] = &[
    planned("New Project Library...", ""),
    planned("Open Project Library...", ""),
    planned("Show Current Project Library in File Explorer", ""),
    SEP,
    item("Project Manager", &["Shift+1"], Act::ProjectManager),
    item("New Project", &["Ctrl+N"], Act::NewProject),
    item("Open Project...", &["Ctrl+O"], Act::OpenProject),
    item("Import Project...", &[], Act::OpenProject),
    item("Export Project...", &[], Act::SaveCopy),
    planned("Restore Project Archive...", ""),
    SEP,
    item("Save Project", &["Ctrl+S"], Act::SaveProject),
    item("Save Project As...", &["Ctrl+Shift+S"], Act::SaveProjectAs),
    item("Save Copy As...", &[], Act::SaveCopy),
    item("Revert to Last Saved Version", &[], Act::Revert),
    SEP,
    sub(
        "Import",
        &[
            item("Media...", &["Ctrl+I"], Act::ImportMedia),
            item("Folder as Bin...", &[], Act::FolderAsBin),
            item(
                "Timeline: AAF, EDL, XML...",
                &["Ctrl+Shift+I"],
                Act::ImportTimeline,
            ),
            planned("Timeline: Pre-conformed EDL...", ""),
            item("Subtitle...", &[], Act::ImportTimeline),
            planned("Metadata to Media Pool...", ""),
            planned("Bin...", ""),
        ],
    ),
    sub(
        "Export",
        &[
            item("Timeline: EDL...", &[], Act::ExportEdl),
            item("Timeline: OpenTimelineIO...", &[], Act::ExportOtio),
            item("Timeline: XML...", &[], Act::ExportXml(false)),
            item("Timeline: AAF...", &[], Act::ExportAaf),
            item("Subtitle...", &[], Act::ExportSubtitles),
            planned("Bin...", ""),
            planned("Metadata from Media Pool...", ""),
        ],
    ),
    item("Media Management...", &["Ctrl+M"], Act::MediaManagement),
    SEP,
    item("Project Settings...", &["Shift+9"], Act::ProjectSettings),
    item("System Preferences...", &["Ctrl+,"], Act::Preferences),
    SEP,
    item("Quick Export...", &["Ctrl+Shift+E"], Act::QuickExport),
    SEP,
    item("Quit", &["Ctrl+Q"], Act::Quit),
];

const EDIT: &[Entry] = &[
    item("Undo", &["Ctrl+Z"], Act::Undo),
    item_shown(
        "Redo",
        "Ctrl+Shift+Z",
        &["Ctrl+Shift+Z", "Ctrl+Y"],
        Act::Redo,
    ),
    SEP,
    item("Cut", &["Ctrl+X"], Act::Cut),
    item("Copy", &["Ctrl+C"], Act::Copy),
    item("Paste", &["Ctrl+V"], Act::Paste),
    item("Paste Insert", &["Ctrl+Shift+V"], Act::PasteInsert),
    item("Paste Mix to Current Track", &[], Act::PasteMix),
    item("Duplicate", &["Alt+C"], Act::Duplicate),
    // The edits of the viewer that need keys of their own.
    item("Fit to Fill", &["Shift+F11"], Act::FitToFill),
    item("Place on Top", &["F12"], Act::PlaceOnTop),
    item_shown(
        "Delete Selected",
        "Backspace",
        &["Backspace"],
        Act::DeleteSelected,
    ),
    item_shown(
        "Delete Selected with Ripple",
        "Del",
        &["Delete", "Shift+Delete", "Shift+Backspace"],
        Act::RippleDeleteSelected,
    ),
    SEP,
    item("Select All", &["Ctrl+A"], Act::SelectAll),
    item("Deselect All", &["Ctrl+Shift+A"], Act::DeselectAll),
    item("Select Clips Forward", &[], Act::SelectForward),
    item("Find...", &[], Act::ShowMediaPool),
    SEP,
    // Filled with the projects last used when the menu is drawn.
    sub(
        "Recent Projects",
        &[item("Project Manager...", &[], Act::ProjectManager)],
    ),
    item(
        "Keyboard Customization",
        &["Ctrl+Alt+K"],
        Act::ShowShortcuts,
    ),
];

const TRIM: &[Entry] = &[
    item("Select Nearest Edit Point", &["V"], Act::SelectNearestEdit),
    item("Selection Mode", &["A"], Act::SelectionMode),
    item("Trim Mode", &["T"], Act::TrimMode),
    item("Dynamic Trim Mode", &["Alt+W"], Act::DynamicTrim),
    item("Ripple Start", &["Ctrl+["], Act::RippleStart),
    item("Ripple End", &["Ctrl+]"], Act::RippleEnd),
    sub(
        "Ripple Trim Previous / Next",
        &[
            item(
                "Ripple Trim Previous Edit to Playhead",
                &[],
                Act::RippleStart,
            ),
            item("Ripple Trim Next Edit to Playhead", &[], Act::RippleEnd),
        ],
    ),
    item("Trim Start to Playhead", &[], Act::TrimStartToPlayhead),
    item("Trim End to Playhead", &[], Act::TrimEndToPlayhead),
    item("Extend Edit", &["E"], Act::ExtendEdit),
    sub(
        "Slip Clip",
        &[
            item("Slip One Frame Earlier", &[","], Act::SlipLeft),
            item("Slip One Frame Later", &["."], Act::SlipRight),
        ],
    ),
    sub(
        "Slide Clip",
        &[
            item("Slide One Frame Left", &["Alt+,"], Act::SlideLeft),
            item("Slide One Frame Right", &["Alt+."], Act::SlideRight),
        ],
    ),
    SEP,
    item("Loop Trim", &[], Act::LoopTrim),
    item("Asymmetric Trimming", &[], Act::AsymmetricTrim),
    planned("Trim Editor", ""),
];

const TIMELINE: &[Entry] = &[
    item("New Timeline...", &["Ctrl+Alt+N"], Act::NewTimeline),
    item(
        "New Timeline Using Selected Clips",
        &[],
        Act::TimelineFromSelection,
    ),
    item(
        "Create New Compound Clip...",
        &["Alt+G"],
        Act::CreateCompound,
    ),
    item(
        "New Effects Clip...",
        &["Alt+Shift+E"],
        Act::CreateEffectsClip,
    ),
    sub(
        "Decompose in Place",
        &[
            item("Decompose Compound Clip", &[], Act::DecomposeCompound),
            item("Decompose Nested Timeline", &[], Act::DecomposeCompound),
        ],
    ),
    SEP,
    // Not in the spec, but cutting a clip is basic editing.
    item_shown(
        "Split Clip",
        "Ctrl+\\",
        &["Ctrl+\\", "Ctrl+B", "S"],
        Act::SplitClip,
    ),
    SEP,
    item("Create Timeline Using Bin...", &[], Act::TimelineFromBin),
    item("Import Timeline...", &[], Act::ImportTimeline),
    item("Export Timeline...", &[], Act::ExportEdl),
    SEP,
    item(
        "Timeline Resolution and Frame Rate...",
        &[],
        Act::ProjectSettings,
    ),
    sub(
        "Timeline Settings",
        &[
            item("Snap", &["N"], Act::ToggleSnap),
            item(
                "Linked Selection",
                &["Ctrl+Shift+L"],
                Act::ToggleLinkedSelection,
            ),
            item("Position Lock", &[], Act::PositionLock),
            item(
                "Selection Follows Playhead",
                &[],
                Act::SelectionFollowsPlayhead,
            ),
        ],
    ),
    SEP,
    item("Add Marker", &["M"], Act::AddMarker),
    item(
        "Add Marker and Show Dialog",
        &["Alt+M"],
        Act::AddMarkerDialog,
    ),
    item("Delete All Markers", &[], Act::DeleteAllMarkers),
    SEP,
    item("Create Subtitle Track", &[], Act::CreateSubtitleTrack),
    item("Create Subtitle Region", &[], Act::CreateSubtitleRegion),
    item(
        "Add Transition to All Edits",
        &["Ctrl+T"],
        Act::AddTransitionToAll,
    ),
    SEP,
    item("Switch Timeline", &["Ctrl+Alt+."], Act::SwitchTimeline),
    planned("Open Timeline in New Window", ""),
    item("Delete Timeline", &[], Act::DeleteTimeline),
];

const CLIP: &[Entry] = &[
    item("Enable Clip", &["D"], Act::ToggleEnableClip),
    item("Match Frame", &["F"], Act::MatchFrame),
    item("Lock Clip", &[], Act::LockClip),
    SEP,
    item("New Multicam Clip...", &[], Act::NewMulticam),
    item("Multicam Viewer", &[], Act::MulticamViewer),
    item("Add to Media Pool as Stills", &[], Act::StillToPool),
    item("Change Clip Speed...", &["Ctrl+R"], Act::ChangeClipSpeed),
    item("Change Clip Duration...", &[], Act::ChangeClipDuration),
    sub(
        "Retime Controls",
        &[
            item("Retime Clip", &[], Act::ChangeClipSpeed),
            item("Freeze Frame", &["Shift+R"], Act::FreezeFrame),
            item("Speed Ramp...", &[], Act::SpeedRamp),
            item("Reverse Clip", &[], Act::ReverseClip),
        ],
    ),
    SEP,
    item_shown(
        "Create New Compound Clip",
        "Alt+G",
        &[],
        Act::CreateCompound,
    ),
    item("Decompose in Place", &[], Act::DecomposeCompound),
    SEP,
    item("Link Clips", &["Ctrl+L"], Act::LinkClips),
    item("Unlink Audio and Video", &[], Act::UnlinkClips),
    SEP,
    item("Enable / Disable Audio Scrubbing", &[], Act::ToggleAudioScrubbing),
    sub(
        "Auto Align Clips",
        &[
            item("Based on Timecode", &[], Act::AutoAlignTimecode),
            item("Based on Waveform", &[], Act::AutoAlignWaveform),
        ],
    ),
    SEP,
    item("Normalize Audio Levels...", &[], Act::NormalizeAudio),
    item("Render in Place", &[], Act::RenderInPlace),
    // The shortcut is on the Timeline menu's entry; one key is not given to two entries.
    item("New Effects Clip", &[], Act::CreateEffectsClip),
    item("New Compound Clip", &[], Act::CreateCompound),
    item("Clip Attributes...", &[], Act::ClipAttributes),
];

const MARK: &[Entry] = &[
    item("Mark In", &["I"], Act::MarkIn),
    item("Mark Out", &["O"], Act::MarkOut),
    item("Mark Clip", &["X"], Act::MarkClip),
    item("Mark Timeline In/Out", &[], Act::MarkTimeline),
    SEP,
    item("Clear In", &["Alt+I"], Act::ClearIn),
    item("Clear Out", &["Alt+O"], Act::ClearOut),
    item("Clear In and Out", &["Alt+X"], Act::ClearInOut),
    SEP,
    item("Add Marker", &["M"], Act::AddMarker),
    item(
        "Add Marker and Show Dialog",
        &["Alt+M"],
        Act::AddMarkerDialog,
    ),
    item("Modify Marker...", &[], Act::ModifyMarker),
    item(
        "Delete Marker at Current Position",
        &[],
        Act::DeleteMarkerHere,
    ),
    item("Delete All Markers in Timeline", &[], Act::DeleteAllMarkers),
    SEP,
    item("Go to Previous Marker", &["Shift+Up"], Act::PreviousMarker),
    item("Go to Next Marker", &["Shift+Down"], Act::NextMarker),
    SEP,
    sub(
        "Set Clip Color",
        &[
            item("Default", &[], Act::SetClipColor(None)),
            item("Orange", &[], Act::SetClipColor(Some(0))),
            item("Apricot", &[], Act::SetClipColor(Some(1))),
            item("Yellow", &[], Act::SetClipColor(Some(2))),
            item("Lime", &[], Act::SetClipColor(Some(3))),
            item("Olive", &[], Act::SetClipColor(Some(4))),
            item("Green", &[], Act::SetClipColor(Some(5))),
            item("Teal", &[], Act::SetClipColor(Some(6))),
            item("Navy", &[], Act::SetClipColor(Some(7))),
            item("Blue", &[], Act::SetClipColor(Some(8))),
            item("Purple", &[], Act::SetClipColor(Some(9))),
            item("Violet", &[], Act::SetClipColor(Some(10))),
            item("Pink", &[], Act::SetClipColor(Some(11))),
            item("Tan", &[], Act::SetClipColor(Some(12))),
            item("Beige", &[], Act::SetClipColor(Some(13))),
            item("Brown", &[], Act::SetClipColor(Some(14))),
            item("Chocolate", &[], Act::SetClipColor(Some(15))),
        ],
    ),
];

const VIEW: &[Entry] = &[
    planned("Show Current Frame on Playback", ""),
    item("Safe Area Overlay", &[], Act::OverlaySafeArea),
    item("Show Grid / Guides", &[], Act::OverlayGrid),
    SEP,
    planned("Show Toolbar", ""),
    item("Show Full Screen Viewer", &["P"], Act::FullScreenViewer),
    item("Show Cinema Viewer", &["Ctrl+F"], Act::CinemaViewer),
    SEP,
    sub(
        "Viewer Overlays",
        &[
            planned("Transform / Crop / Dynamic Zoom", ""),
            item("Safe Area", &[], Act::OverlaySafeArea),
            item("Center Crosshair", &[], Act::OverlayCrosshair),
            item(
                "Checkerboard Background (Alpha)",
                &[],
                Act::OverlayCheckerboard,
            ),
            item("Show All Overlays", &[], Act::OverlayAll),
        ],
    ),
    SEP,
    item("Timecode Window", &[], Act::TimecodeWindow),
    item("Show Scopes", &["Shift+H"], Act::ShowScopes),
    SEP,
    item("Show Audio Meters", &[], Act::ShowMixer),
    item("Show Audio Mixer", &[], Act::ShowMixer),
    item("Show Inspector", &[], Act::ShowInspector),
];

const PLAYBACK: &[Entry] = &[
    item_shown("Play Forward", "Space / L", &["L"], Act::PlayForward),
    item("Play Reverse", &["J"], Act::PlayReverse),
    item("Stop", &["K"], Act::Stop),
    SEP,
    item("Play Around Current Frame", &["/"], Act::PlayAround),
    item("Play In to Out", &[], Act::PlayInToOut),
    item("Play Around In / Around Out", &[], Act::PlayAroundMark),
    item("Loop Playback", &["Ctrl+W"], Act::ToggleLoop),
    SEP,
    item("Go to First Frame", &["Home"], Act::GoFirst),
    item("Go to Last Frame", &["End"], Act::GoLast),
    item("Go to In", &["Q"], Act::GoIn),
    item("Go to Out", &["W"], Act::GoOut),
    item_shown("Previous Frame", "Left", &["Left"], Act::PreviousFrame),
    item_shown("Next Frame", "Right", &["Right"], Act::NextFrame),
    item_shown("Previous Edit Point", "Up", &["Up"], Act::PreviousEdit),
    item_shown("Next Edit Point", "Down", &["Down"], Act::NextEdit),
    SEP,
    sub(
        "Timeline Playback Resolution",
        &[
            item("Half", &[], Act::ProxyHalf),
            item("Quarter", &[], Act::ProxyQuarter),
        ],
    ),
    planned("Effects Memory Cache", ""),
    sub(
        "Timeline Proxy Mode",
        &[
            item("Off", &[], Act::ProxyOff),
            item("Half", &[], Act::ProxyHalf),
            item("Quarter", &[], Act::ProxyQuarter),
            item("Eighth", &[], Act::ProxyEighth),
        ],
    ),
    // The copies made in File > Media Management: smaller, and quick to find a picture in.
    item("Use Optimised Media", &[], Act::ToggleOptimisedMedia),
    // The same switch as System Preferences > Decode Options, where playback is judged.
    item("Hardware Decoding", &[], Act::ToggleHardwareDecoding),
    item("Delete Render Cache", &[], Act::DeleteRenderCache),
];

const EFFECTS: &[Entry] = &[
    sub(
        "Video Transitions",
        &[
            item("Cross Dissolve", &[], Act::AddCrossDissolve),
            item("Dip to Color", &[], Act::AddDipToBlack),
        ],
    ),
    sub(
        "Audio Transitions",
        &[item("Cross Fade", &[], Act::AddCrossFade)],
    ),
    item("Remove Transition", &[], Act::RemoveTransition),
    sub(
        "OpenFX",
        &[
            sub(
                "Blur",
                &[
                    item(
                        "Gaussian Blur",
                        &[],
                        Act::AddEffect(EffectKind::GaussianBlur),
                    ),
                    item("Sharpen", &[], Act::AddEffect(EffectKind::Sharpen)),
                ],
            ),
            sub(
                "Color",
                &[
                    item("Invert Colors", &[], Act::AddEffect(EffectKind::Invert)),
                    item(
                        "Black and White",
                        &[],
                        Act::AddEffect(EffectKind::BlackAndWhite),
                    ),
                ],
            ),
            sub(
                "Distort",
                &[item("Mosaic", &[], Act::AddEffect(EffectKind::Mosaic))],
            ),
        ],
    ),
    sub(
        "Audio FX",
        &[
            item("EQ", &[], Act::TrackEq),
            item("Dynamics", &[], Act::TrackDynamics),
        ],
    ),
    sub(
        "Picasso FX",
        &[
            sub(
                "Color",
                &[
                    item("Sepia", &[], Act::AddEffect(EffectKind::Sepia)),
                    item(
                        "RGB Mixer",
                        &[],
                        Act::AddEffect(EffectKind::RgbMixer),
                    ),
                    item(
                        "Color Temperature",
                        &[],
                        Act::AddEffect(EffectKind::ColorTemperature),
                    ),
                ],
            ),
            sub(
                "Blur",
                &[
                    item(
                        "Soft Blur",
                        &[],
                        Act::AddEffect(EffectKind::GaussianBlur),
                    ),
                    item(
                        "Directional Blur",
                        &[],
                        Act::AddEffect(EffectKind::DirectionalBlur),
                    ),
                ],
            ),
            sub(
                "Light",
                &[
                    item("Glow", &[], Act::AddEffect(EffectKind::Glow)),
                    item(
                        "Diffusion",
                        &[],
                        Act::AddEffect(EffectKind::Diffusion),
                    ),
                ],
            ),
            sub(
                "Stylize",
                &[
                    item("Vignette", &[], Act::AddEffect(EffectKind::Vignette)),
                    item("Film Grain", &[], Act::AddEffect(EffectKind::FilmGrain)),
                ],
            ),
        ],
    ),
    sub("Compositing Effects", &[planned("Nodes", "")]),
    sub(
        "Generators",
        &[
            item("Solid Color", &[], Act::GenerateSolid),
            item("Color Bars", &[], Act::GenerateBars),
            item("Gradient", &[], Act::GenerateGradient),
        ],
    ),
    sub(
        "Titles",
        &[
            item("Text", &[], Act::AddTitle),
            item("Text+", &[], Act::AddTextPlus),
        ],
    ),
    SEP,
    planned("Effects Library Search", ""),
    planned("Add Tool (Compositing)", ""),
];

const COLOR: &[Entry] = &[
    item(
        "Reset All Grades and Nodes",
        &["Shift+Home"],
        Act::ResetAllGrades,
    ),
    item("Reset Current Node Grade", &[], Act::ResetCurrentGrade),
    item("Enable/Disable Current Node", &["Ctrl+D"], Act::ToggleNode),
    item("Grab Still", &["Ctrl+Alt+G"], Act::GrabStill),
    item("Play Still", &[], Act::PlayStill),
    SEP,
    sub(
        "Nodes",
        &[
            item("Add Serial Node", &["Alt+S"], Act::AddSerialNode),
            item("Add Serial Before", &[], Act::AddNodeBefore),
            item("Add Parallel Node", &["Alt+P"], Act::AddParallelNode),
            item("Add Layer Mixer", &["Alt+L"], Act::AddLayerNode),
            planned("Add Splitter / Combiner", ""),
            item("Add Outside Node", &[], Act::AddOutsideNode),
            item("Label Node", &[], Act::LabelNode),
        ],
    ),
    SEP,
    sub(
        "Timeline",
        &[
            item("Previous Node Graph", &[], Act::PreviousNodeGraph),
            item("Next Node Graph", &[], Act::NextNodeGraph),
        ],
    ),
    SEP,
    sub("LUT", &[item("Apply LUT from list", &[], Act::ApplyLut)]),
    sub(
        "Gallery",
        &[
            item("Grab Still", &[], Act::GrabStill),
            item("Play Still", &[], Act::PlayStill),
        ],
    ),
    SEP,
    item("Scopes On/Off", &[], Act::ToggleScopes),
    item("Highlight", &[], Act::ToggleHighlight),
    planned("Lightbox", ""),
];

const AUDIO: &[Entry] = &[
    sub("ADR", &[planned("Automated Dialogue Replacement", "")]),
    SEP,
    item("Bounce Mix to Track", &[], Act::BounceMix),
    item("Bounce Selected Tracks", &[], Act::BounceSelectedTracks),
    SEP,
    planned("Cache Audio Effects", ""),
    SEP,
    sub(
        "Audio Link",
        &[planned("Link Group", ""), planned("Decompose", "")],
    ),
    planned("Loop Jog", ""),
    item("Show Audio Timeline / Mixer", &[], Act::ShowMixer),
];

const WORKSPACE: &[Entry] = &[
    item("Full Screen Viewer", &["P"], Act::FullScreenViewer),
    sub(
        "Show Panel in Workspace",
        &[
            item("Media Pool", &[], Act::ShowMediaPool),
            item("Effects Library", &[], Act::PageEffects),
            item("Inspector", &[], Act::ShowInspector),
            item("Mixer", &[], Act::ShowMixer),
            item("Metadata", &[], Act::ShowMediaPool),
            item("Edit Index", &[], Act::ShowEditIndex),
            planned("Sound Library", ""),
        ],
    ),
    sub(
        "Layout",
        &[
            item("Reset UI Layout", &[], Act::ResetLayout),
            item("Save Layout", &[], Act::SaveLayout),
            item("Load Layout", &[], Act::LoadLayout),
        ],
    ),
    sub(
        "Switch to Page",
        &[
            item("Media", &["Shift+2"], Act::PageMedia),
            item("Cut", &["Shift+3"], Act::PageCut),
            item("Edit", &["Shift+4"], Act::PageEdit),
            item("Effects", &["Shift+5"], Act::PageEffects),
            item("Color", &["Shift+6"], Act::PageColor),
            item("Audio", &["Shift+7"], Act::PageAudio),
            item("Deliver", &["Shift+8"], Act::PageDeliver),
        ],
    ),
    planned("Dual Screen On/Off", ""),
    planned("Video Clean Feed", ""),
];

/// Help > Language: each language by its own name, English (the default) first.
const LANGUAGES: &[Entry] = &[
    language(crate::i18n::Language::English),
    language(crate::i18n::Language::Spanish),
    language(crate::i18n::Language::Portuguese),
    language(crate::i18n::Language::French),
    language(crate::i18n::Language::German),
    language(crate::i18n::Language::Italian),
    language(crate::i18n::Language::Dutch),
    language(crate::i18n::Language::Polish),
    language(crate::i18n::Language::Turkish),
    language(crate::i18n::Language::Swedish),
    language(crate::i18n::Language::Norwegian),
    language(crate::i18n::Language::Russian),
    language(crate::i18n::Language::Ukrainian),
    language(crate::i18n::Language::Greek),
    language(crate::i18n::Language::Danish),
    language(crate::i18n::Language::Finnish),
    language(crate::i18n::Language::Indonesian),
    language(crate::i18n::Language::Czech),
    language(crate::i18n::Language::Slovak),
    language(crate::i18n::Language::Romanian),
    language(crate::i18n::Language::Hungarian),
    language(crate::i18n::Language::Croatian),
    language(crate::i18n::Language::Bosnian),
    language(crate::i18n::Language::Serbian),
    language(crate::i18n::Language::Catalan),
    language(crate::i18n::Language::Estonian),
    language(crate::i18n::Language::Latvian),
    language(crate::i18n::Language::Lithuanian),
    language(crate::i18n::Language::Icelandic),
    language(crate::i18n::Language::Luxembourgish),
    language(crate::i18n::Language::Albanian),
    language(crate::i18n::Language::Filipino),
    language(crate::i18n::Language::Vietnamese),
    language(crate::i18n::Language::Korean),
    language(crate::i18n::Language::Bengali),
    language(crate::i18n::Language::Thai),
    language(crate::i18n::Language::Chinese),
    language(crate::i18n::Language::Japanese),
    language(crate::i18n::Language::Arabic),
    language(crate::i18n::Language::Hindi),
];

const fn language(language: crate::i18n::Language) -> Entry {
    item(language.name(), &[], Act::Language(language))
}

const HELP: &[Entry] = &[
    item("Picasso Suite Manual", &["F1"], Act::Manual),
    planned("Picasso Suite Tutorials", ""),
    item("Keyboard Customization", &[], Act::ShowShortcuts),
    SEP,
    sub("Language", LANGUAGES),
    SEP,
    item("Check for Updates...", &[], Act::CheckForUpdates),
    SEP,
    item("Export Log File...", &[], Act::ExportLog),
    SEP,
    item("About Picasso Suite", &[], Act::About),
];

/// The menus of the title row: name, left edge of its label in the title row, and entries.
pub(crate) const MENUS: [(&str, f32, &[Entry]); 13] = [
    ("File", 216.0, FILE),
    ("Edit", 267.0, EDIT),
    ("Trim", 322.0, TRIM),
    ("Timeline", 384.0, TIMELINE),
    ("Clip", 480.0, CLIP),
    ("Mark", 535.0, MARK),
    ("View", 601.0, VIEW),
    ("Playback", 665.0, PLAYBACK),
    ("Effects", 763.0, EFFECTS),
    ("Color", 842.0, COLOR),
    ("Audio", 911.0, AUDIO),
    ("Workspace", 1003.0, WORKSPACE),
    ("Help", 1122.0, HELP),
];

/// Keys that have no menu entry of their own.
const HIDDEN_KEYS: [(&str, Act); 4] = [
    ("Space", Act::TogglePlay),
    ("Shift+Left", Act::SecondBack),
    ("Shift+Right", Act::SecondForward),
    ("Escape", Act::LeaveViewer),
];

/// Whether an entry (or anything below it) does something.
pub(crate) fn is_live(entries: &[Entry]) -> bool {
    entries.iter().any(|e| match e {
        Entry::Item { act, .. } => act.is_some(),
        Entry::Sep => false,
        Entry::Sub { items, .. } => is_live(items),
    })
}

/// The keys of an entry as the menu shows them.
pub(crate) fn shown_keys(shown: &str, keys: &[&str]) -> String {
    if shown.is_empty() {
        keys.first().map_or(String::new(), |k| (*k).to_string())
    } else {
        shown.to_string()
    }
}

/// Reads `Ctrl+Shift+Z`, `Alt+I`, `Shift+Up`, `Ctrl+[`, `Space` and the like.
pub(crate) fn parse_keys(text: &str) -> Option<KeyboardShortcut> {
    let mut modifiers = Modifiers::NONE;
    let mut key = None;
    for part in text
        .split('+')
        .filter(|p| !p.is_empty())
        .chain(text.ends_with("++").then_some("+"))
    {
        match part {
            "Ctrl" | "Cmd" => modifiers.command = true,
            "Shift" => modifiers.shift = true,
            "Alt" => modifiers.alt = true,
            "[" => key = Some(Key::OpenBracket),
            "]" => key = Some(Key::CloseBracket),
            "," => key = Some(Key::Comma),
            "." => key = Some(Key::Period),
            "/" => key = Some(Key::Slash),
            "\\" => key = Some(Key::Backslash),
            "Del" | "Delete" => key = Some(Key::Delete),
            "Left" => key = Some(Key::ArrowLeft),
            "Right" => key = Some(Key::ArrowRight),
            "Up" => key = Some(Key::ArrowUp),
            "Down" => key = Some(Key::ArrowDown),
            other => key = Some(Key::from_name(other)?),
        }
    }
    key.map(|k| KeyboardShortcut::new(modifiers, k))
}

/// A key bound to an act.
pub(crate) struct Binding {
    pub shortcut: KeyboardShortcut,
    #[cfg_attr(not(test), allow(dead_code))] // read by the tests that report a conflict
    pub text: String,
    pub act: Act,
}

fn collect(entries: &'static [Entry], out: &mut Vec<(&'static str, Act)>) {
    for e in entries {
        match e {
            Entry::Item {
                keys,
                act: Some(act),
                ..
            } => out.extend(keys.iter().map(|k| (*k, *act))),
            Entry::Sub { items, .. } => collect(items, out),
            _ => {}
        }
    }
}

/// The text and act of every key in the menus and the hidden ones.
pub(crate) fn key_texts() -> Vec<(&'static str, Act)> {
    let mut all = Vec::new();
    for (_, _, entries) in MENUS {
        collect(entries, &mut all);
    }
    all.extend(HIDDEN_KEYS);
    all
}

/// Every binding with the keys of `custom` in place of the table's for those acts, the most
/// specific (most modifiers) first, so `Shift+Home` is tried before `Home`.
pub(crate) fn bindings_with(custom: &HashMap<Act, Vec<String>>) -> Vec<Binding> {
    let table = key_texts()
        .into_iter()
        .filter(|(_, act)| !custom.contains_key(act))
        .map(|(text, act)| (text.to_string(), act));
    let changed = custom
        .iter()
        .flat_map(|(act, keys)| keys.iter().map(|k| (k.clone(), *act)));
    let mut all: Vec<Binding> = table
        .chain(changed)
        .filter_map(|(text, act)| {
            parse_keys(&text).map(|shortcut| Binding {
                shortcut,
                text,
                act,
            })
        })
        .collect();
    all.sort_by_key(|b| {
        std::cmp::Reverse(
            usize::from(b.shortcut.modifiers.command)
                + usize::from(b.shortcut.modifiers.shift)
                + usize::from(b.shortcut.modifiers.alt),
        )
    });
    all
}

/// The keys set in Keyboard Customization and the bindings they make.
struct Keymap {
    custom: HashMap<Act, Vec<String>>,
    bindings: Arc<Vec<Binding>>,
}

static KEYMAP: RwLock<Option<Keymap>> = RwLock::new(None);

fn with_keymap<R>(f: impl FnOnce(&Keymap) -> R) -> R {
    {
        let map = KEYMAP.read().unwrap_or_else(|e| e.into_inner());
        if let Some(map) = map.as_ref() {
            return f(map);
        }
    }
    let mut map = KEYMAP.write().unwrap_or_else(|e| e.into_inner());
    let map = map.get_or_insert_with(|| Keymap {
        custom: HashMap::new(),
        bindings: Arc::new(bindings_with(&HashMap::new())),
    });
    f(map)
}

/// Every binding in use: the table's keys, and those set in Keyboard Customization.
pub(crate) fn bindings() -> Arc<Vec<Binding>> {
    with_keymap(|m| m.bindings.clone())
}

/// The acts whose keys were changed, and their keys.
pub(crate) fn custom_keys() -> HashMap<Act, Vec<String>> {
    with_keymap(|m| m.custom.clone())
}

/// Uses `custom` for the acts in it from now on; keys that cannot be read are left out.
pub(crate) fn set_custom_keys(mut custom: HashMap<Act, Vec<String>>) {
    for keys in custom.values_mut() {
        keys.retain(|k| parse_keys(k).is_some());
    }
    let bindings = Arc::new(bindings_with(&custom));
    *KEYMAP.write().unwrap_or_else(|e| e.into_inner()) = Some(Keymap { custom, bindings });
}

/// The table's keys of an act.
pub(crate) fn default_keys(act: Act) -> Vec<&'static str> {
    let mut keys: Vec<&'static str> = Vec::new();
    for (text, a) in key_texts() {
        if a == act && !keys.contains(&text) {
            keys.push(text);
        }
    }
    keys
}

/// The keys an act has with the changed keys `custom`.
pub(crate) fn keys_in(custom: &HashMap<Act, Vec<String>>, act: Act) -> Vec<String> {
    custom
        .get(&act)
        .cloned()
        .unwrap_or_else(|| default_keys(act).into_iter().map(str::to_string).collect())
}

/// The keys an act has now.
pub(crate) fn keys_of(act: Act) -> Vec<String> {
    with_keymap(|m| keys_in(&m.custom, act))
}

/// The keys a menu entry shows: those set in Keyboard Customization for its act, else the
/// table's.
pub(crate) fn display_keys(act: Option<Act>, shown: &str, keys: &[&str]) -> String {
    match act.and_then(|a| with_keymap(|m| m.custom.get(&a).cloned())) {
        Some(custom) => custom.join(" / "),
        None => shown_keys(shown, keys),
    }
}

/// The name an act is saved under.
pub(crate) fn act_name(act: Act) -> String {
    format!("{act:?}")
}

/// The act saved under `name`, among those that have a menu entry or a key.
pub(crate) fn act_named(name: &str) -> Option<Act> {
    rows()
        .into_iter()
        .filter_map(|r| r.act)
        .find(|a| act_name(*a) == name)
}

/// Whether a key is Shift, Ctrl, Alt or the system key on its own, which egui also reports as a
/// key: those are held with a key, not one themselves.
pub(crate) fn is_modifier(key: Key) -> bool {
    matches!(
        key,
        Key::ShiftLeft
            | Key::ShiftRight
            | Key::ControlLeft
            | Key::ControlRight
            | Key::AltLeft
            | Key::AltRight
            | Key::SuperLeft
            | Key::SuperRight
    )
}

/// The text of a key as the table writes it ("Ctrl+Shift+Z", "["), when it is one the table can
/// read back.
pub(crate) fn key_text(modifiers: Modifiers, key: Key) -> Option<String> {
    if is_modifier(key) {
        return None;
    }
    let name = match key {
        Key::OpenBracket => "[",
        Key::CloseBracket => "]",
        Key::Comma => ",",
        Key::Period => ".",
        Key::Slash => "/",
        Key::Backslash => "\\",
        Key::Delete => "Delete",
        Key::ArrowLeft => "Left",
        Key::ArrowRight => "Right",
        Key::ArrowUp => "Up",
        Key::ArrowDown => "Down",
        other => other.name(),
    };
    let mut text = String::new();
    if modifiers.command || modifiers.ctrl {
        text.push_str("Ctrl+");
    }
    if modifiers.alt {
        text.push_str("Alt+");
    }
    if modifiers.shift {
        text.push_str("Shift+");
    }
    text.push_str(name);
    let read = parse_keys(&text)?;
    (read.logical_key == key).then_some(text)
}

/// Whether two key texts are the same key ("Del" and "Delete" are).
pub(crate) fn same_key(a: &str, b: &str) -> bool {
    match (parse_keys(a), parse_keys(b)) {
        (Some(x), Some(y)) => x == y,
        _ => false,
    }
}

/// The acts other than `act` that `text` starts now.
pub(crate) fn key_users(text: &str, act: Act) -> Vec<Act> {
    key_users_in(&bindings(), text, act)
}

/// The acts other than `act` that `text` starts among `bindings`.
pub(crate) fn key_users_in(bindings: &[Binding], text: &str, act: Act) -> Vec<Act> {
    let mut users = Vec::new();
    for b in bindings {
        if b.act != act && same_key(&b.text, text) && !users.contains(&b.act) {
            users.push(b.act);
        }
    }
    users
}

/// One line of Keyboard Customization: where the command is, its name, its act (none when it is
/// planned) and the keys the table gives it.
pub(crate) struct KeyRow {
    pub menu: &'static str,
    pub label: &'static str,
    pub act: Option<Act>,
    pub default: String,
}

/// The names of the keys that have no menu entry.
fn hidden_label(act: Act) -> &'static str {
    match act {
        Act::TogglePlay => "Play / Stop",
        Act::SecondBack => "Back one second",
        Act::SecondForward => "Forward one second",
        Act::LeaveViewer => "Leave the full-screen viewer",
        _ => "",
    }
}

/// Every menu entry that has a key or works, then the keys without an entry (under "Other").
pub(crate) fn rows() -> Vec<KeyRow> {
    fn walk(menu: &'static str, entries: &'static [Entry], out: &mut Vec<KeyRow>) {
        for e in entries {
            match e {
                Entry::Item {
                    label,
                    shown,
                    keys,
                    act,
                } => {
                    let default = shown_keys(shown, keys);
                    if !default.is_empty() || act.is_some() {
                        out.push(KeyRow {
                            menu,
                            label,
                            act: *act,
                            default,
                        });
                    }
                }
                Entry::Sub { items, .. } => walk(menu, items, out),
                Entry::Sep => {}
            }
        }
    }
    let mut out = Vec::new();
    for (menu, _, entries) in MENUS {
        walk(menu, entries, &mut out);
    }
    for (text, act) in HIDDEN_KEYS {
        out.push(KeyRow {
            menu: "Other",
            label: hidden_label(act),
            act: Some(act),
            default: text.to_string(),
        });
    }
    out
}

/// The name of an act for messages: its first menu entry's label.
pub(crate) fn act_label(act: Act) -> &'static str {
    rows()
        .into_iter()
        .find(|r| r.act == Some(act))
        .map_or("another command", |r| r.label)
}

/// How many entries the menus have, and how many of them work.
pub(crate) fn counts() -> (usize, usize) {
    fn walk(entries: &[Entry], total: &mut usize, built: &mut usize) {
        for e in entries {
            match e {
                Entry::Item { act, .. } => {
                    *total += 1;
                    *built += usize::from(act.is_some());
                }
                Entry::Sub { items, .. } => walk(items, total, built),
                Entry::Sep => {}
            }
        }
    }
    let (mut total, mut built) = (0, 0);
    for (_, _, entries) in MENUS {
        walk(entries, &mut total, &mut built);
    }
    (total, built)
}

/// A Markdown report of every menu entry, its key and whether it works yet. Generated from the
/// table the menus are drawn from, so it is never out of date.
pub(crate) fn status_markdown() -> String {
    fn walk(path: &str, entries: &[Entry], out: &mut String) {
        for e in entries {
            match e {
                Entry::Item {
                    label,
                    shown,
                    keys,
                    act,
                } => {
                    let keys = shown_keys(shown, keys);
                    let status = if act.is_some() { "works" } else { "planned" };
                    out.push_str(&format!(
                        "| {path}{label} | {} | {status} |\n",
                        if keys.is_empty() {
                            "-".to_string()
                        } else {
                            format!("`{keys}`")
                        }
                    ));
                }
                Entry::Sub { label, items } => walk(&format!("{path}{label} > "), items, out),
                Entry::Sep => {}
            }
        }
    }
    let (total, built) = counts();
    let mut out = format!(
        "# Menu status\n\nGenerated by `picasso-suite --menu-status` from the table the menus are drawn from.\n\n{built} of {total} entries work; the rest are planned and shown disabled.\n"
    );
    for (name, _, entries) in MENUS {
        out.push_str(&format!(
            "\n## {name}\n\n| Entry | Key | Status |\n|---|---|---|\n"
        ));
        walk("", entries, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn labels(entries: &'static [Entry], out: &mut Vec<&'static str>) {
        for e in entries {
            match e {
                Entry::Item { label, .. } => out.push(label),
                Entry::Sub { label, items } => {
                    out.push(label);
                    labels(items, out);
                }
                Entry::Sep => {}
            }
        }
    }

    #[test]
    fn every_key_of_a_built_entry_can_be_read() {
        for (text, act) in key_texts() {
            assert!(parse_keys(text).is_some(), "{text} ({act:?}) is not a key");
        }
    }

    #[test]
    fn one_key_never_does_two_things() {
        let mut seen: HashMap<String, Act> = HashMap::new();
        for b in bindings().iter() {
            let id = format!("{:?}", b.shortcut);
            if let Some(other) = seen.insert(id, b.act) {
                assert_eq!(
                    other, b.act,
                    "{} is bound to {other:?} and {:?}",
                    b.text, b.act
                );
            }
        }
    }

    #[test]
    fn parsing_reads_modifiers_and_symbols() {
        let s = parse_keys("Ctrl+Shift+Z").unwrap();
        assert!(s.modifiers.command && s.modifiers.shift && !s.modifiers.alt);
        assert_eq!(s.logical_key, Key::Z);
        assert_eq!(parse_keys("Ctrl+[").unwrap().logical_key, Key::OpenBracket);
        assert_eq!(parse_keys("Ctrl+\\").unwrap().logical_key, Key::Backslash);
        assert_eq!(parse_keys("/").unwrap().logical_key, Key::Slash);
        assert_eq!(parse_keys("Shift+Up").unwrap().logical_key, Key::ArrowUp);
        assert_eq!(parse_keys("Shift+9").unwrap().logical_key, Key::Num9);
        assert!(parse_keys("Ctrl+Nonsense").is_none());
        assert!(parse_keys("").is_none());
    }

    #[test]
    fn custom_keys_replace_the_tables_keys_of_their_act_only() {
        let mut custom = HashMap::new();
        custom.insert(Act::Undo, vec!["Ctrl+U".to_string()]);
        custom.insert(Act::Redo, Vec::new());
        let all = bindings_with(&custom);
        let undo: Vec<&str> = all
            .iter()
            .filter(|b| b.act == Act::Undo)
            .map(|b| b.text.as_str())
            .collect();
        assert_eq!(undo, ["Ctrl+U"], "Ctrl+Z no longer undoes");
        assert!(all.iter().all(|b| b.act != Act::Redo), "Redo has no key");
        assert!(
            all.iter()
                .any(|b| b.act == Act::SaveProject && b.text == "Ctrl+S"),
            "the others keep theirs"
        );
    }

    #[test]
    fn keys_read_back_as_they_are_written() {
        let ctrl = Modifiers {
            ctrl: true,
            command: true,
            ..Modifiers::NONE
        };
        let shift = Modifiers::SHIFT;
        for (m, key, text) in [
            (ctrl, Key::Z, "Ctrl+Z"),
            (Modifiers::NONE, Key::OpenBracket, "["),
            (shift | Modifiers::ALT, Key::ArrowLeft, "Alt+Shift+Left"),
            (Modifiers::NONE, Key::Delete, "Delete"),
            (Modifiers::NONE, Key::F5, "F5"),
            (Modifiers::NONE, Key::Space, "Space"),
            (shift, Key::Num9, "Shift+9"),
        ] {
            assert_eq!(key_text(m, key).as_deref(), Some(text));
            assert!(same_key(text, &key_text(m, key).unwrap_or_default()));
        }
        assert_eq!(
            key_text(ctrl, Key::ControlLeft),
            None,
            "Ctrl alone is not a key"
        );
        assert!(same_key("Del", "Delete"));
        assert!(!same_key("Ctrl+Z", "Z"));
    }

    #[test]
    fn every_act_is_saved_and_found_by_its_name() {
        for row in rows() {
            if let Some(act) = row.act {
                assert_eq!(act_named(&act_name(act)), Some(act), "{}", row.label);
            }
        }
        assert_eq!(act_named("NoSuchCommand"), None);
        assert!(rows()
            .iter()
            .any(|r| r.label == "Play / Stop" && r.default == "Space"));
        assert_eq!(act_label(Act::Undo), "Undo");
    }

    #[test]
    fn the_most_specific_binding_comes_first() {
        let all = bindings();
        let home = all.iter().position(|b| b.text == "Home").unwrap();
        let shift_home = all.iter().position(|b| b.text == "Shift+Home").unwrap();
        assert!(shift_home < home);
    }

    #[test]
    fn no_other_product_is_named_in_a_menu() {
        let mut all = Vec::new();
        for (name, _, entries) in MENUS {
            all.push(name);
            labels(entries, &mut all);
        }
        for label in all {
            for banned in [
                "Resolve",
                "DaVinci",
                "Fusion",
                "Fairlight",
                "PowerGrade",
                "Premiere",
                "Final Cut",
            ] {
                assert!(!label.contains(banned), "{label} names another product");
            }
        }
    }

    #[test]
    fn the_menu_bar_has_the_spec_s_thirteen_menus_in_order() {
        let names: Vec<&str> = MENUS.iter().map(|m| m.0).collect();
        assert_eq!(
            names,
            [
                "File",
                "Edit",
                "Trim",
                "Timeline",
                "Clip",
                "Mark",
                "View",
                "Playback",
                "Effects",
                "Color",
                "Audio",
                "Workspace",
                "Help"
            ]
        );
        assert!(MENUS.iter().all(|(_, _, e)| !e.is_empty()));
    }

    #[test]
    fn planned_entries_never_take_a_key() {
        for (_, _, entries) in MENUS {
            fn check(entries: &[Entry]) {
                for e in entries {
                    match e {
                        Entry::Item {
                            keys,
                            act: None,
                            label,
                            ..
                        } => assert!(keys.is_empty(), "{label} is planned but has keys"),
                        Entry::Sub { items, .. } => check(items),
                        _ => {}
                    }
                }
            }
            check(entries);
        }
    }

    #[test]
    fn the_status_report_counts_what_works() {
        let (total, built) = counts();
        assert!(built > 80 && total > built, "{built} of {total}");
        let report = status_markdown();
        assert!(report.contains(&format!("{built} of {total} entries work")));
        assert!(report.contains("| Ripple Start | `Ctrl+[` | works |"));
        assert!(report.contains("| Trim Mode | `T` | works |"));
        assert!(report.contains("| Dynamic Trim Mode | `Alt+W` | works |"));
        assert!(report.contains("| Import > Media... | `Ctrl+I` | works |"));
        assert_eq!(report.matches("\n## ").count(), 13);
    }

    /// `Menu-Status.md` is generated from the table above, so it must match it. Regenerate it with
    /// `cargo run -p picasso-suite -- --menu-status > Menu-Status.md` after changing a menu entry.
    #[test]
    fn the_menu_status_file_is_current() {
        // Git on Windows may check the file out with CRLF; the report is written with LF. The shell
        // redirect that writes the file adds a newline of its own, so trailing space is ignored.
        let committed = include_str!("../../../Menu-Status.md").replace("\r\n", "\n");
        assert_eq!(
            committed.trim_end(),
            status_markdown().trim_end(),
            "Menu-Status.md is out of date: run `cargo run -p picasso-suite -- --menu-status > Menu-Status.md`"
        );
    }

    #[test]
    fn the_listing_marks_what_is_built() {
        let list = rows();
        assert!(list.iter().any(|r| r.menu == "Edit"
            && r.label == "Undo"
            && r.default == "Ctrl+Z"
            && r.act == Some(Act::Undo)));
        assert!(list.iter().any(|r| r.menu == "File"
            && r.label == "Media Management..."
            && r.default == "Ctrl+M"
            && r.act == Some(Act::MediaManagement)));
        // A planned entry with no key is no key binding, so it is not listed.
        assert!(!list.iter().any(|r| r.label == "Restore Project Archive..."));
        // The report still says it is planned.
        assert!(status_markdown().contains("| Restore Project Archive... | - | planned |"));
    }
}

