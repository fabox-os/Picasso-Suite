//! The Color page (the dots tab), laid out like the reference screen: the stills gallery, the
//! viewer and the node area at the top, the clips in the middle, and below them the palette
//! row, the primary color wheels with their values and the scopes.
//!
//! Positions are written in pixels of the 2000-pixel-wide reference and scaled to the
//! 1872-point window (`K`). The lower half keeps its reference size, anchored to the page bar;
//! the upper half takes what is left.

#![allow(clippy::too_many_arguments)]

use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

use egui::{
    pos2, vec2, Align2, Color32, ColorImage, Context, Painter, Pos2, Rect, Sense, Stroke,
    StrokeKind, TextureHandle, TextureOptions, Ui,
};
use picasso_color::curves::{Curves, HUE_MODES, SOFT_CLIP_NONE};
use picasso_color::scopes::{curve_histogram, histogram, parade, vectorscope, waveform};
use picasso_color::{ColorGrade, CurveInput, Histogram, Lut};
use picasso_core::Command;
use picasso_render::Frame;

use crate::app::StudioApp;
use crate::capture::fit;
use crate::chrome::{Screen, Tool, R};
use crate::icons::Icon;
use crate::menu::Act;
use crate::panels::Split;
use crate::theme;
use crate::wheel::{color_wheel, master, set_master};

mod secondary;
use secondary::{Palette, WindowDrag};

/// Points per reference pixel.
const K: f32 = 0.936;

/// A video track row in the mini timeline: name, picture on, and its clips' (id, in, out).
type TimelineRow = (String, bool, Vec<(picasso_core::ClipId, u64, u64)>);

const BG: Color32 = Color32::from_rgb(40, 40, 45);
const BAR: Color32 = Color32::from_rgb(33, 33, 37);
const TOOLBAR2: Color32 = Color32::from_rgb(30, 30, 34);
const VIEW: Color32 = Color32::from_rgb(34, 34, 38);
const LINE: Color32 = Color32::from_rgb(16, 16, 19);
const TEXT: Color32 = Color32::from_rgb(214, 214, 218);
const LABEL: Color32 = Color32::from_rgb(130, 130, 136);
const GREY: Color32 = Color32::from_rgb(150, 150, 154);
const ICON: Color32 = Color32::from_rgb(118, 118, 122);
const FIELD: Color32 = Color32::from_rgb(22, 22, 25);

/// Reference rows: the end of the second toolbar, the top of the palette row, the page bar.
const TOOLBAR2_END: f32 = 100.0;
const PALETTE_TOP: f32 = 535.0;
const PAGE_BAR: f32 = 1040.0;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum ScopeKind {
    #[default]
    Parade,
    Waveform,
    Histogram,
    Vectorscope,
}

impl ScopeKind {
    fn label(self) -> &'static str {
        match self {
            ScopeKind::Parade => "Parade",
            ScopeKind::Waveform => "Waveform",
            ScopeKind::Histogram => "Histogram",
            ScopeKind::Vectorscope => "Vectorscope",
        }
    }
    fn next(self) -> Self {
        match self {
            ScopeKind::Parade => ScopeKind::Waveform,
            ScopeKind::Waveform => ScopeKind::Histogram,
            ScopeKind::Histogram => ScopeKind::Vectorscope,
            ScopeKind::Vectorscope => ScopeKind::Parade,
        }
    }
}

/// What a click in the viewer samples, after one of the pickers was chosen.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Picker {
    /// Makes the clicked colour neutral grey, keeping its brightness.
    Neutral,
    /// Makes the clicked colour black.
    Black,
    /// Makes the clicked colour white.
    White,
    /// Keys the clicked colour with the node's qualifier.
    Qualify,
}

/// The scope shown, what it measured last, and a picker waiting for a click.
pub(crate) struct ColorPage {
    kind: ScopeKind,
    /// The viewer picture and scope kind the data below was measured from.
    measured: Option<(u64, ScopeKind)>,
    texture: Option<TextureHandle>,
    histogram: Option<Histogram>,
    picker: Option<Picker>,
    /// The still chosen in the gallery, and whether it is shown beside the picture.
    still: Option<u64>,
    pub(crate) playing_still: bool,
    /// The gallery's pictures, made from the stills' PNGs.
    still_textures: std::collections::HashMap<u64, TextureHandle>,
    /// The palette under the clips: the wheels, the qualifier or the windows.
    palette: Palette,
    /// Shows the node's key in the viewer (its graded part in colour, the rest grey).
    pub(crate) highlight: bool,
    /// The window chosen in the Window palette, and what a drag on the viewer changes.
    window: usize,
    window_drag: Option<WindowDrag>,
    /// The histogram behind the curves, and the viewer picture it was measured from.
    curve_histogram: Option<Histogram>,
    /// The hue/sat/lum histogram behind the curve graph, measured with the curve being edited.
    curve_input: Option<[u32; 256]>,
    /// The viewer picture and mode the data above was measured from, so a new picture or a
    /// changed mode is measured again.
    curve_measured: Option<(u64, usize)>,
    /// When (egui's clock, seconds) the scope and the curves' histogram were last measured.
    /// While a clip plays they are measured at most [`SCOPE_RATE`] times a second, not once per
    /// picture: each measure reads every pixel, and the eye cannot follow faster.
    scope_time: f64,
    curve_time: f64,
    /// Curves > Edit: which of the Y, R, G and B curves a drag changes, and whether they change
    /// together (each takes the edited curve's points). The curves, their intensities and the
    /// soft clip are in the node's grade.
    curve_channels: [bool; 4],
    curve_link: bool,
    /// The curve being edited: 0 is the custom Y/R/G/B curves, 1 to 6 the hue/sat/lum ones.
    curve_mode: usize,
    /// The point of the edited curve being dragged.
    curve_drag: Option<usize>,
    /// The background thread that measures scopes, so the interface thread is not blocked by
    /// pixel reads. `None` on WASM (where threads are unavailable and measurement is synchronous).
    scope_worker: Option<(Sender<ScopeTask>, Receiver<ScopeResult>)>,
}

/// How many times a second the scopes follow a playing picture.
const SCOPE_RATE: f64 = 20.0;

/// A measurement task for the scope worker thread: measure one frame for a scope or a curve
/// histogram, so the pixel reads don't block the interface thread.
enum ScopeTask {
    /// Measure a scope (parade, waveform, vectorscope, or histogram).
    Scope {
        frame: Arc<Frame>,
        kind: ScopeKind,
        generation: u64,
        ctx: Context,
    },
    /// Measure a curve's backing histogram (mode 0) or hue/sat/lum input (1..=6).
    Curve {
        frame: Arc<Frame>,
        mode: usize,
        generation: u64,
        ctx: Context,
    },
}

/// A completed measurement from the scope worker thread.
enum ScopeResult {
    Scope {
        kind: ScopeKind,
        generation: u64,
        image: Option<Box<ColorImage>>,
        histogram: Option<Histogram>,
    },
    Curve {
        mode: usize,
        generation: u64,
        histogram: Option<Histogram>,
        input: Option<Box<[u32; 256]>>,
    },
}

/// Spawns the background thread that measures scopes, returning the task sender and result
/// receiver. On WASM (no threads) returns `None`.
#[cfg(not(target_arch = "wasm32"))]
fn spawn_scope_worker() -> Option<(Sender<ScopeTask>, Receiver<ScopeResult>)> {
    let (task_tx, task_rx) = channel::<ScopeTask>();
    let (result_tx, result_rx) = channel::<ScopeResult>();
    std::thread::spawn(move || {
        while let Ok(first) = task_rx.recv() {
            // Skip superseded frames: keep the latest task of each kind, so a scope waiting
            // behind a curve histogram (or the other way round) is not thrown away.
            let (mut scope, mut curve) = (None, None);
            for task in std::iter::once(first).chain(task_rx.try_iter()) {
                match task {
                    t @ ScopeTask::Scope { .. } => scope = Some(t),
                    t @ ScopeTask::Curve { .. } => curve = Some(t),
                }
            }
            for task in [scope, curve].into_iter().flatten() {
                let (result, ctx) = run_scope_task(task);
                if result_tx.send(result).is_err() {
                    return;
                }
                // The page only looks for results when it is drawn.
                ctx.request_repaint();
            }
        }
    });
    Some((task_tx, result_rx))
}

/// Measures one task, on the worker thread; also hands back the context to redraw.
#[cfg(not(target_arch = "wasm32"))]
fn run_scope_task(task: ScopeTask) -> (ScopeResult, Context) {
    match task {
        ScopeTask::Scope {
            frame,
            kind,
            generation,
            ctx,
        } => {
            let (image, histogram) = measure_scope_pixels(&frame, kind);
            let result = ScopeResult::Scope {
                kind,
                generation,
                image: image.map(Box::new),
                histogram,
            };
            (result, ctx)
        }
        ScopeTask::Curve {
            frame,
            mode,
            generation,
            ctx,
        } => {
            let (histogram, input) = measure_curve_pixels(&frame, mode);
            let result = ScopeResult::Curve {
                mode,
                generation,
                histogram,
                input: input.map(Box::new),
            };
            (result, ctx)
        }
    }
}

/// The curve mode buttons: an x position, an icon and a name. Index 0 is the custom curves;
/// 1..=`HUE_MODES` are the hue/sat/lum curves, with `HUE_MODES[i - 1]` the curve they edit.
const CURVE_MODES: [(f32, Icon, &str); 7] = [
    (1140.0, Icon::FxCurves, "Custom"),
    (1168.0, Icon::WheelDot, "Hue vs Hue"),
    (1196.0, Icon::WheelHdr, "Hue vs Sat"),
    (1224.0, Icon::WheelLog, "Hue vs Lum"),
    (1252.0, Icon::Spline, "Lum vs Sat"),
    (1280.0, Icon::Circles3, "Sat vs Sat"),
    (1308.0, Icon::Hexagon, "Sat vs Lum"),
];
/// The names of the hue/sat/lum curves, in the order of the buttons that edit them.
const HUE_MODE_NAMES: [&str; HUE_MODES] = [
    "Hue vs Hue",
    "Hue vs Sat",
    "Hue vs Lum",
    "Lum vs Sat",
    "Sat vs Sat",
    "Sat vs Lum",
];

/// The input a hue/sat/lum curve keys on, for a mode button index 1..=`HUE_MODES`.
fn curve_input_of(mode: usize) -> CurveInput {
    match mode {
        1..=3 => CurveInput::Hue,
        4 => CurveInput::Luminance,
        _ => CurveInput::Saturation,
    }
}

/// The colour a hue/sat/lum curve is drawn in.
fn hue_curve_color(mode: usize) -> Color32 {
    match mode {
        0 => Color32::from_rgb(120, 210, 235),
        1 => Color32::from_rgb(235, 210, 80),
        2 => Color32::from_rgb(235, 140, 60),
        3 => Color32::from_rgb(235, 90, 160),
        4 => Color32::from_rgb(60, 210, 130),
        _ => Color32::from_rgb(150, 90, 235),
    }
}

/// Whether a measure made at `last` (egui seconds) is due again; when it is not, a repaint is
/// asked for when it will be, so the last picture of a playback is still measured.
fn scope_due(ctx: &Context, last: f64) -> bool {
    let wait = last + 1.0 / SCOPE_RATE - ctx.input(|i| i.time);
    if wait > 0.0 {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait));
        return false;
    }
    true
}

impl Default for ColorPage {
    fn default() -> Self {
        Self {
            kind: ScopeKind::default(),
            measured: None,
            texture: None,
            histogram: None,
            picker: None,
            still: None,
            playing_still: false,
            still_textures: std::collections::HashMap::new(),
            palette: Palette::default(),
            highlight: false,
            window: 0,
            window_drag: None,
            curve_histogram: None,
            curve_input: None,
            curve_measured: None,
            scope_time: f64::NEG_INFINITY,
            curve_time: f64::NEG_INFINITY,
            curve_channels: [true; 4],
            curve_link: true,
            curve_mode: 0,
            curve_drag: None,
            scope_worker: spawn_scope_worker(),
        }
    }
}

/// Builds the scope image and/or histogram from a frame. This is the pixel-heavy work that runs on
/// the worker thread so it doesn't block the interface.
fn measure_scope_pixels(frame: &Frame, kind: ScopeKind) -> (Option<ColorImage>, Option<Histogram>) {
    let (w, h) = (frame.width as usize, frame.height as usize);
    let glow = |v: f32, base: [f32; 3], top: [f32; 3]| {
        Color32::from_rgb(
            (base[0] + v * top[0]) as u8,
            (base[1] + v * top[1]) as u8,
            (base[2] + v * top[2]) as u8,
        )
    };
    match kind {
        ScopeKind::Parade => {
            let p = parade(&frame.rgba, w, h, w.min(512));
            let columns = p.columns;
            // A narrow black gap between the three channels.
            let gap = (columns / 60).max(2);
            let width = columns * 3 + gap * 2;
            let mut pixels = vec![Color32::BLACK; width * 256];
            // Faint traces stay visible (a log scale) and dense ones burn towards white.
            let tints: [[f32; 3]; 3] = [
                [255.0, 62.0, 62.0],
                [60.0, 245.0, 82.0],
                [72.0, 92.0, 255.0],
            ];
            let top = (1.0 + p.max.max(1) as f32).ln();
            for (c, counts) in p.counts.iter().enumerate() {
                let left = c * (columns + gap);
                for column in 0..columns {
                    for level in 0..256 {
                        let count = counts[column * 256 + level];
                        if count == 0 {
                            continue;
                        }
                        let v = ((1.0 + count as f32).ln() / top).clamp(0.0, 1.0);
                        let light = 0.32 + 0.68 * v.powf(0.65);
                        let white = ((v - 0.55) / 0.45).clamp(0.0, 1.0) * 0.55;
                        let px = tints[c].map(|t| {
                            ((t * light) * (1.0 - white) + 255.0 * white).min(255.0) as u8
                        });
                        pixels[(255 - level) * width + left + column] =
                            Color32::from_rgb(px[0], px[1], px[2]);
                    }
                }
            }
            (Some(ColorImage::new([width, 256], pixels)), None)
        }
        ScopeKind::Waveform => {
            let wf = waveform(&frame.rgba, w, h, w.min(512));
            let mut pixels = vec![Color32::BLACK; wf.columns * 256];
            for column in 0..wf.columns {
                for level in 0..256 {
                    let count = wf.counts[column * 256 + level];
                    if count > 0 {
                        let v = (count as f32 / wf.max.max(1) as f32).sqrt();
                        pixels[(255 - level) * wf.columns + column] =
                            glow(v, [30.0, 60.0, 50.0], [70.0, 195.0, 90.0]);
                    }
                }
            }
            (Some(ColorImage::new([wf.columns, 256], pixels)), None)
        }
        ScopeKind::Vectorscope => {
            let vs = vectorscope(&frame.rgba, w, h, 256);
            let mut pixels = vec![Color32::BLACK; vs.size * vs.size];
            for (px, count) in pixels.iter_mut().zip(&vs.counts) {
                if *count > 0 {
                    let v = (*count as f32 / vs.max.max(1) as f32).sqrt();
                    *px = glow(v, [70.0, 140.0, 130.0], [150.0, 115.0, 110.0]);
                }
            }
            (Some(ColorImage::new([vs.size, vs.size], pixels)), None)
        }
        ScopeKind::Histogram => {
            (None, Some(histogram(&frame.rgba, w, h)))
        }
    }
}

/// Measures the curve histogram (mode 0) or hue/sat/lum input (1..=6) from a frame. Runs on the
/// worker thread.
fn measure_curve_pixels(frame: &Frame, mode: usize) -> (Option<Histogram>, Option<[u32; 256]>) {
    let (w, h) = (frame.width as usize, frame.height as usize);
    if mode == 0 {
        (Some(histogram(&frame.rgba, w, h)), None)
    } else {
        (
            None,
            Some(curve_histogram(
                &frame.rgba,
                w,
                h,
                curve_input_of(mode),
            )),
        )
    }
}

impl ColorPage {
    /// Picks up finished measurements from the worker thread and applies them (creates the
    /// texture, stores the histograms). Non-blocking: drains whatever is ready, scopes and
    /// curve histograms alike, so neither reader throws away the other's results.
    fn apply_worker_results(&mut self, ctx: &Context) {
        #[cfg(not(target_arch = "wasm32"))]
        while let Some(result) = self
            .scope_worker
            .as_ref()
            .and_then(|(_, rx)| rx.try_recv().ok())
        {
            match result {
                ScopeResult::Scope {
                    kind,
                    generation,
                    image,
                    histogram,
                } => {
                    if kind != self.kind {
                        continue;
                    }
                    if let Some(h) = histogram {
                        self.histogram = Some(h);
                    }
                    if let Some(img) = image {
                        match &mut self.texture {
                            Some(t) => t.set(*img, TextureOptions::LINEAR),
                            None => {
                                self.texture =
                                    Some(ctx.load_texture("scope", *img, TextureOptions::LINEAR))
                            }
                        }
                    }
                    self.measured = Some((generation, self.kind));
                }
                ScopeResult::Curve {
                    mode,
                    generation,
                    histogram,
                    input,
                } => {
                    if mode != self.curve_mode {
                        continue;
                    }
                    if let Some(h) = histogram {
                        self.curve_histogram = Some(h);
                        self.curve_input = None;
                    }
                    if let Some(input) = input {
                        self.curve_input = Some(*input);
                        self.curve_histogram = None;
                    }
                    self.curve_measured = Some((generation, mode));
                }
            }
        }
        #[cfg(target_arch = "wasm32")]
        let _ = ctx;
    }

    /// Measures the histogram behind the curves once for each new picture, so it follows
    /// playback. `mode` is the curve being edited (0 the custom one, 1..=6 a hue/sat/lum one).
    fn measure_curve(
        &mut self,
        ctx: &Context,
        frame: Option<Arc<Frame>>,
        generation: u64,
        mode: usize,
    ) {
        // Picks up finished results from the worker thread.
        self.apply_worker_results(ctx);
        if self.curve_measured == Some((generation, mode)) || !scope_due(ctx, self.curve_time) {
            return;
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let (Some((send, _)), Some(frame)) = (&self.scope_worker, &frame) {
            self.curve_time = ctx.input(|i| i.time);
            let _ = send.send(ScopeTask::Curve {
                frame: frame.clone(),
                mode,
                generation,
                ctx: ctx.clone(),
            });
            return;
        }
        // Fallback (WASM, or the worker thread has ended): measure here, on the interface thread.
        let Some(frame) = frame.as_deref() else {
            return;
        };
        self.curve_time = ctx.input(|i| i.time);
        let (histogram, input) = measure_curve_pixels(frame, mode);
        if let Some(h) = histogram {
            self.curve_histogram = Some(h);
            self.curve_input = None;
        }
        if let Some(input) = input {
            self.curve_input = Some(input);
            self.curve_histogram = None;
        }
        self.curve_measured = Some((generation, mode));
    }

    fn measure(&mut self, ctx: &Context, frame: Option<Arc<Frame>>, generation: u64) {
        // Picks up finished results from the worker thread.
        self.apply_worker_results(ctx);
        if self.measured == Some((generation, self.kind)) {
            return;
        }
        // A new scope kind is shown at once; a new picture waits its turn.
        let same_kind = self.measured.is_some_and(|(_, k)| k == self.kind);
        if same_kind && !scope_due(ctx, self.scope_time) {
            return;
        }
        // On native, hand the frame to the worker thread so the pixel reads don't block the
        // interface. On WASM (or if the worker has ended), measure here.
        #[cfg(not(target_arch = "wasm32"))]
        if let (Some((send, _)), Some(frame)) = (&self.scope_worker, &frame) {
            self.scope_time = ctx.input(|i| i.time);
            let _ = send.send(ScopeTask::Scope {
                frame: frame.clone(),
                kind: self.kind,
                generation,
                ctx: ctx.clone(),
            });
            return;
        }
        let Some(frame) = frame.as_deref() else {
            return;
        };
        self.scope_time = ctx.input(|i| i.time);
        let (image, histogram) = measure_scope_pixels(frame, self.kind);
        if let Some(h) = histogram {
            self.histogram = Some(h);
        }
        if let Some(img) = image {
            match &mut self.texture {
                Some(t) => t.set(img, TextureOptions::LINEAR),
                None => {
                    self.texture =
                        Some(ctx.load_texture("scope", img, TextureOptions::LINEAR))
                }
            }
        }
        self.measured = Some((generation, self.kind));
    }
}

/// The height in points of the lower half (the palette row and the panels): rows 656 to 1004 of
/// Resolve at 1920 x 1080.
const LOWER: f32 = 348.0 * R;
/// Resolve's window height at 1080p: positions of the lower half count up to there.
const FRESH_HEIGHT: f32 = 1052.0;

/// Where the upper half's columns of the older reference land on Resolve's at 1080p (x, in its
/// pixels): the gallery's tools, the viewer's and the node area's bar...
const TB2_X: [(f32, f32); 31] = [
    (3.0, 0.0),
    (30.0, 18.0),
    (89.0, 57.0),
    (209.0, 280.0),
    (385.0, 390.0),
    (432.0, 419.0),
    (491.0, 456.0),
    (535.0, 485.0),
    (592.0, 521.0),
    (650.0, 559.0),
    (707.0, 596.0),
    (738.0, 614.0),
    (777.0, 640.0),
    (819.0, 667.0),
    (868.0, 698.0),
    (910.0, 740.0),
    (950.0, 800.0),
    (1124.0, 893.0),
    (1142.0, 910.0),
    (1368.0, 1052.0),
    (1387.0, 1066.0),
    (1438.0, 1101.0),
    (1497.0, 1287.0),
    (1525.0, 1305.0),
    (1553.0, 1324.0),
    (1587.0, 1345.0),
    (1638.0, 1388.0),
    (1675.0, 1417.0),
    (1803.0, 1605.0),
    (1838.0, 1627.0),
    (2000.0, 1919.0),
];
/// ...and the gallery, the viewer with its transport row and the node area.
const BODY_X: [(f32, f32); 18] = [
    (3.0, 0.0),
    (738.0, 614.0),
    (765.0, 636.0),
    (785.0, 645.0),
    (816.0, 665.0),
    (857.0, 691.0),
    (907.0, 724.0),
    (968.0, 858.0),
    (1020.0, 892.0),
    (1068.0, 923.0),
    (1117.0, 954.0),
    (1170.0, 988.0),
    (1229.0, 1026.0),
    (1487.0, 1281.0),
    (1525.0, 1305.0),
    (1798.0, 1601.0),
    (1828.0, 1620.0),
    (2000.0, 1919.0),
];
/// The older reference's rows of the upper half that land on Resolve's: the bars, the viewer's
/// picture, its scrub bar and transport row, the node area's end, the clips.
const ROWS_C: [(f32, f32); 10] = [
    (62.0, 33.0),
    (100.0, 58.0),
    (145.0, 87.0),
    (272.0, 396.0),
    (292.0, 410.0),
    (318.0, 426.0),
    (337.0, 438.0),
    (365.0, 456.0),
    (535.0, 656.0),
    (1040.0, 1000.0),
];
/// Points per reference pixel for text and icons, and for widths, in the upper half.
const SZU: f32 = 0.65 * R;
const QU: f32 = 0.64 * R;

fn piece(map: &[(f32, f32)], v: f32) -> f32 {
    for w in map.windows(2) {
        let ((a0, b0), (a1, b1)) = (w[0], w[1]);
        if v <= a1 {
            return b0 + (v - a0) / (a1 - a0) * (b1 - b0);
        }
    }
    let (a, b) = map[map.len() - 1];
    b + (v - a)
}

/// A drawing surface that takes reference pixels.
#[derive(Clone, Copy)]
struct Cv<'a> {
    p: &'a Painter,
    s: &'a Screen,
    /// The height in points of the upper half (reference rows 100 to 535).
    upper: f32,
    /// Coordinates are pixels of the newer palette reference (a pixel is a point), whose page
    /// bar is at row 1047, instead of the older full-page reference.
    fresh: bool,
}

/// How much bigger or smaller than usual a color wheel is drawn when its panel became `taller`
/// and `wider` than usual: as much as the smaller of the two, so it stays round and inside the
/// panel, and never so small it cannot be grabbed or so big it covers the values around it.
fn wheel_fit(taller: f32, wider: f32) -> f32 {
    let fit = taller.min(wider);
    if fit.is_finite() {
        fit.clamp(0.4, 1.4)
    } else {
        1.0
    }
}

impl<'a> Cv<'a> {
    /// A point given in pixels of Resolve at 1080p, for the upper half: its rows from the second
    /// toolbar to the palette row fill the window's.
    fn rv(&self, x: f32, y: f32) -> Pos2 {
        let (x, y) = self.rv_points(x, y);
        self.s.p(x, y)
    }
    /// `rv` as the screen's own points, before the dividers move them.
    fn rv_points(&self, x: f32, y: f32) -> (f32, f32) {
        let top = 92.0;
        let bottom = (self.s.h - (FRESH_HEIGHT - 656.0) * R).max(top + 300.0);
        (x * R, top + (y - 33.0) * (bottom - top) / (656.0 - 33.0))
    }
    fn rrect(&self, x0: f32, y0: f32, x1: f32, y1: f32) -> Rect {
        self.s
            .between(self.rv_points(x0, y0), self.rv_points(x1, y1))
    }
    /// Whether a row belongs to the upper half of the older reference.
    fn upper_row(&self, y: f32) -> bool {
        !self.fresh && y < PALETTE_TOP
    }
    fn new(p: &'a Painter, s: &'a Screen) -> Self {
        let top = 92.0 + (TOOLBAR2_END - 62.0) * K;
        let bottom = s.h - 46.8 - LOWER;
        Cv {
            p,
            s,
            upper: (bottom - top).max(120.0),
            fresh: false,
        }
    }
    /// The same surface, taking coordinates of the newer palette reference.
    fn fresh(&self) -> Self {
        Cv {
            fresh: true,
            ..*self
        }
    }
    /// The size of the text in value boxes.
    fn value_font(&self) -> f32 {
        if self.fresh {
            9.5 * R
        } else {
            17.0
        }
    }
    fn y(&self, r: f32) -> f32 {
        let top = 92.0 + (TOOLBAR2_END - 62.0) * K;
        if r <= TOOLBAR2_END {
            92.0 + (r - 62.0) * K
        } else if r <= PALETTE_TOP {
            top + (r - TOOLBAR2_END) / (PALETTE_TOP - TOOLBAR2_END) * self.upper
        } else {
            // The older palettes are squeezed into the lower half the newer reference gives.
            let squeeze = LOWER / ((PAGE_BAR - PALETTE_TOP) * K);
            self.s.h - 46.8 - (PAGE_BAR - r) * K * squeeze
        }
    }
    fn at(&self, x: f32, y: f32) -> Pos2 {
        let (x, y) = self.at_points(x, y);
        self.s.p(x, y)
    }
    /// `at` as the screen's own points, before the dividers move them.
    fn at_points(&self, x: f32, y: f32) -> (f32, f32) {
        if self.fresh {
            (x * R, self.s.h - (FRESH_HEIGHT - y) * R)
        } else if y < PALETTE_TOP {
            let map = if y <= TOOLBAR2_END {
                &TB2_X[..]
            } else {
                &BODY_X[..]
            };
            self.rv_points(piece(map, x), piece(&ROWS_C, y))
        } else {
            (x * K, self.y(y))
        }
    }
    fn rect(&self, x0: f32, y0: f32, x1: f32, y1: f32) -> Rect {
        self.s
            .between(self.at_points(x0, y0), self.at_points(x1, y1))
    }
    fn band(&self, x0: f32, y0: f32, x1: f32, y1: f32, c: Color32) {
        self.p.rect_filled(self.rect(x0, y0, x1, y1), 0.0, c);
    }
    fn text(&self, x: f32, y: f32, align: Align2, text: &str, width: f32, c: Color32, bold: bool) {
        crate::i18n::paint_text(&self.p,
            self.at(x, y),
            align,
            text,
            fit(
                self.p,
                text,
                width * if self.upper_row(y) { QU } else { K } * self.shrink(x, y),
                bold,
            ),
            c,
        );
    }
    /// How much smaller than usual text and icons at a reference point are drawn: as much as
    /// the dividers squeezed the panel there, so they stay inside it.
    fn shrink(&self, x: f32, y: f32) -> f32 {
        let (x, y) = self.at_points(x, y);
        self.s.shrink(x, y)
    }
    /// Text at a fixed size in points.
    fn label(&self, x: f32, y: f32, align: Align2, text: &str, size: f32, c: Color32, bold: bool) {
        // The newer reference gives sizes in its pixels.
        let size = if self.fresh {
            size * R
        } else if self.upper_row(y) {
            size * SZU
        } else {
            size
        } * self.shrink(x, y);
        let font = if bold {
            theme::semibold(size)
        } else {
            theme::regular(size)
        };
        crate::i18n::paint_text(&self.p,self.at(x, y), align, text, font, c);
    }
    fn line(&self, a: (f32, f32), b: (f32, f32), w: f32, c: Color32) {
        let ends = self
            .s
            .segment(self.at_points(a.0, a.1), self.at_points(b.0, b.1));
        self.p.line_segment(ends, Stroke::new(w, c));
    }
    fn vline(&self, x: f32, y0: f32, y1: f32) {
        self.line((x, y0), (x, y1), 1.0, LINE);
    }
    fn icon(&self, x: f32, y: f32, icon: Icon, c: Color32, size: f32) {
        let size = if self.fresh {
            size * R
        } else if self.upper_row(y) {
            size * SZU
        } else {
            size
        } * self.shrink(x, y);
        crate::icons::paint_scaled(self.p, self.at(x, y), icon, c, size);
    }
    fn chevron(&self, x: f32, y: f32, c: Color32) {
        if self.upper_row(y) {
            let c0 = self.at(x, y);
            let u = SZU;
            self.p.add(egui::Shape::line(
                vec![
                    c0 + vec2(-6.0 * u, -3.5 * u),
                    c0 + vec2(0.0, 3.0 * u),
                    c0 + vec2(6.0 * u, -3.5 * u),
                ],
                Stroke::new(1.3, c),
            ));
            return;
        }
        self.p.add(egui::Shape::line(
            vec![
                self.at(x - 6.0, y - 3.5),
                self.at(x, y + 3.0),
                self.at(x + 6.0, y - 3.5),
            ],
            Stroke::new(1.5, c),
        ));
    }
    fn dots(&self, x: f32, y: f32, c: Color32) {
        if self.upper_row(y) {
            let c0 = self.at(x, y);
            for dx in [-10.0, 0.0, 10.0] {
                self.p.circle_filled(c0 + vec2(dx * SZU, 0.0), 2.8 * SZU, c);
            }
            return;
        }
        for dx in [-10.0, 0.0, 10.0] {
            self.p.circle_filled(self.at(x + dx, y), 2.8 * K, c);
        }
    }
}

/// A box with a value that changes as it is dragged sideways; a double-click puts back
/// `neutral`. Returns the new value when it changed.
fn value_box(
    ui: &mut Ui,
    c: &Cv,
    id: (&str, usize),
    r: (f32, f32, f32, f32),
    value: f32,
    text: &str,
    per_point: f32,
    neutral: f32,
) -> Option<f32> {
    let rect = c.rect(r.0, r.1, r.2, r.3);
    c.p.rect_filled(rect, 2.0, FIELD);
    c.p.rect_stroke(
        rect,
        2.0,
        Stroke::new(1.0, Color32::from_rgb(58, 58, 64)),
        StrokeKind::Inside,
    );
    c.label(
        (r.0 + r.2) / 2.0,
        (r.1 + r.3) / 2.0,
        Align2::CENTER_CENTER,
        text,
        c.value_font(),
        TEXT,
        false,
    );
    let response = ui
        .interact(rect, ui.id().with(id), Sense::click_and_drag())
        .on_hover_cursor(egui::CursorIcon::ResizeHorizontal)
        .on_hover_text("Drag sideways to change, double-click to reset");
    if response.double_clicked() {
        return Some(neutral);
    }
    if response.dragged() {
        let dx = response.drag_delta().x;
        if dx != 0.0 {
            return Some(value + dx * per_point);
        }
    }
    None
}

/// A thin bar under a value, shading from `left` to `right`.
fn underline_thin(c: &Cv, x0: f32, x1: f32, y: f32, left: Color32, right: Color32) {
    let rect = c.rect(x0, y, x1, y + 2.0);
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.left_top(), left);
    mesh.colored_vertex(rect.right_top(), right);
    mesh.colored_vertex(rect.right_bottom(), right);
    mesh.colored_vertex(rect.left_bottom(), left);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    c.p.add(egui::Shape::mesh(mesh));
}

/// Shortens a caption to `max` characters.
fn shorten_caption(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        let cut: String = text.chars().take(max.saturating_sub(1)).collect();
        format!("{cut}...")
    }
}

/// A chain link, lit when the channels move together.
fn paint_link(c: &Cv, x: f32, y: f32, on: bool) {
    let col = if on { theme::WHITE } else { GREY };
    let at = c.at(x, y);
    for (dx, dy) in [(-2.2, 2.2), (2.2, -2.2)] {
        c.p.rect_stroke(
            Rect::from_center_size(at + vec2(dx, dy), vec2(6.5, 4.5))
                .rotate_bb(egui::emath::Rot2::from_angle(-0.785)),
            2.2,
            Stroke::new(1.2, col),
            StrokeKind::Middle,
        );
    }
}

/// The histogram behind the curves: the brightness as a grey area with a light edge, and the
/// red, green and blue as faint lines, as Resolve draws it.
fn paint_curve_histogram(p: &Painter, graph: Rect, h: &Histogram) {
    // Smoothed over three bins; the top is the busiest bin away from pure black and white.
    let smooth = |bins: &[u32; 256]| -> Vec<f32> {
        (0..256)
            .map(|i| {
                let a = bins[i.max(1) - 1] as f32;
                let b = bins[i] as f32;
                let c = bins[(i + 1).min(255)] as f32;
                (a + 2.0 * b + c) / 4.0
            })
            .collect()
    };
    let luma = smooth(&h.luma);
    let channels = [smooth(&h.r), smooth(&h.g), smooth(&h.b)];
    let top = luma[2..254]
        .iter()
        .chain(channels.iter().flat_map(|c| c[2..254].iter()))
        .fold(1.0f32, |m, v| m.max(*v));
    let point = |i: usize, v: f32| {
        pos2(
            graph.left() + graph.width() * i as f32 / 255.0,
            graph.bottom() - graph.height() * (v / top).min(1.0) * 0.98,
        )
    };
    let mut mesh = egui::Mesh::default();
    let fill = Color32::from_rgba_unmultiplied(120, 120, 126, 110);
    for (i, v) in luma.iter().enumerate() {
        mesh.colored_vertex(point(i, *v), fill);
        mesh.colored_vertex(pos2(point(i, 0.0).x, graph.bottom()), fill);
    }
    for i in 0..255u32 {
        let k = i * 2;
        mesh.add_triangle(k, k + 1, k + 2);
        mesh.add_triangle(k + 1, k + 3, k + 2);
    }
    p.add(egui::Shape::mesh(mesh));
    for (bins, color) in channels.iter().zip([
        Color32::from_rgba_unmultiplied(220, 80, 80, 130),
        Color32::from_rgba_unmultiplied(90, 200, 110, 130),
        Color32::from_rgba_unmultiplied(90, 130, 235, 130),
    ]) {
        let line: Vec<Pos2> = bins.iter().enumerate().map(|(i, v)| point(i, *v)).collect();
        p.add(egui::Shape::line(line, Stroke::new(1.0, color)));
    }
    let edge: Vec<Pos2> = luma.iter().enumerate().map(|(i, v)| point(i, *v)).collect();
    p.add(egui::Shape::line(
        edge,
        Stroke::new(1.2, Color32::from_rgb(196, 196, 200)),
    ));
}

/// The distribution of a hue, saturation or luminance as a filled bar, drawn behind a
/// hue/sat/lum curve whose x-axis is that same value.
fn paint_curve_input_histogram(p: &Painter, graph: Rect, bins: &[u32; 256], color: Color32) {
    let top = bins.iter().copied().max().unwrap_or(0).max(1) as f32;
    let smooth = |i: usize| -> f32 {
        let a = bins[i.max(1) - 1] as f32;
        let b = bins[i] as f32;
        let c = bins[(i + 1).min(255)] as f32;
        (a + 2.0 * b + c) / 4.0 / top
    };
    let point = |i: usize, v: f32| {
        pos2(
            graph.left() + graph.width() * i as f32 / 255.0,
            graph.bottom() - graph.height() * (v / top).min(1.0) * 0.98,
        )
    };
    let mut mesh = egui::Mesh::default();
    let fill = color.gamma_multiply(0.5);
    let fill = Color32::from_rgba_unmultiplied(fill.r(), fill.g(), fill.b(), 180);
    for i in 0..256 {
        let v = smooth(i);
        mesh.colored_vertex(point(i, v), fill);
        mesh.colored_vertex(pos2(point(i, 0.0).x, graph.bottom()), fill);
    }
    for i in 0..255u32 {
        let k = i * 2;
        mesh.add_triangle(k, k + 1, k + 2);
        mesh.add_triangle(k + 1, k + 3, k + 2);
    }
    p.add(egui::Shape::mesh(mesh));
    let edge: Vec<Pos2> = (0..256).map(|i| point(i, smooth(i))).collect();
    p.add(egui::Shape::line(edge, Stroke::new(1.2, Color32::from_rgb(236, 236, 238))));
}

/// One wheel: its name, the value of an untouched channel, how far the puck reaches and how
/// fast the jog bar moves the master.
struct WheelSpec {
    label: &'static str,
    neutral: f32,
    range: f32,
    jog: f32,
}

const WHEELS: [WheelSpec; 4] = [
    WheelSpec {
        label: "Lift",
        neutral: 0.0,
        range: 0.3,
        jog: 0.002,
    },
    WheelSpec {
        label: "Gamma",
        neutral: 1.0,
        range: 0.5,
        jog: 0.004,
    },
    WheelSpec {
        label: "Gain",
        neutral: 1.0,
        range: 0.5,
        jog: 0.004,
    },
    WheelSpec {
        label: "Offset",
        neutral: 0.0,
        range: 0.3,
        jog: 0.002,
    },
];

/// The channels a wheel moves.
fn wheel_channels(g: &mut ColorGrade, n: usize) -> &mut [f32; 3] {
    match n {
        0 => &mut g.lift,
        1 => &mut g.gamma,
        2 => &mut g.gain,
        _ => &mut g.offset,
    }
}

/// How a wheel's channel value is shown: gamma as its distance from 1, offset around 25.
fn shown(n: usize, v: f32) -> f32 {
    match n {
        1 => v - 1.0,
        3 => 25.0 + v * 100.0,
        _ => v,
    }
}

fn from_shown(n: usize, v: f32) -> f32 {
    match n {
        1 => v + 1.0,
        3 => (v - 25.0) / 100.0,
        _ => v,
    }
}

impl StudioApp {
    /// The first toolbar of the Color page.
    pub(crate) fn color_toolbar1(&mut self, ui: &mut Ui, ctx: &Context, s: &Screen, y: f32) {
        // (x in reference pixels, icon, hint, size)
        let tools: [(f32, Icon, &str, f32); 9] = [
            (50.0, Icon::Gallery, "Gallery", 0.95),
            (128.0, Icon::LutGrid, "LUTs", 0.95),
            (209.0, Icon::PhotoMusic, "Media Pool", 1.0),
            (288.0, Icon::ClipFrame, "Clips", 0.95),
            (1646.0, Icon::Export, "Quick Export", 0.9),
            (1726.0, Icon::PageEdit, "Timeline", 0.95),
            (1806.0, Icon::NodeTree, "Nodes", 0.95),
            (1886.0, Icon::FxBox, "Effects", 0.9),
            (1964.0, Icon::Lightbox, "Lightbox", 0.9),
        ];
        let mut planned: Option<&str> = None;
        for (x, icon, hint, size) in tools {
            let color = if matches!(icon, Icon::ClipFrame | Icon::NodeTree) {
                Color32::from_rgb(200, 200, 204)
            } else {
                theme::ICON
            };
            let tool = Tool::new(hint).size(size).color(color);
            if self.tool(ui, s.p(x * K, y), icon, tool).clicked() {
                match icon {
                    Icon::LutGrid => self.lut_dialog(ctx),
                    Icon::PhotoMusic => self.import_dialog(ctx),
                    Icon::Export => self.page = crate::app::Page::Deliver,
                    Icon::PageEdit => self.page = crate::app::Page::Edit,
                    Icon::FxBox => self.page = crate::app::Page::Effects,
                    _ => planned = Some(hint),
                }
            }
        }
        let p = ui.painter();
        p.add(egui::Shape::line(
            vec![
                s.p(326.0 * K, y - 3.0),
                s.p(332.0 * K, y + 3.0),
                s.p(338.0 * K, y - 3.0),
            ],
            Stroke::new(1.5, GREY),
        ));
        if let Some(what) = planned {
            self.planned(what);
        }
    }

    pub(crate) fn color_page(&mut self, ui: &mut Ui, ctx: &Context, s: &Screen) {
        let painter = ui.painter().clone();
        // The dividers: the gallery, the viewer and the node area side by side; the clips
        // strip under them; the palettes under all of that; the scopes beside the curves. The
        // color wheels are round: they grow and shrink with the panel they sit in (the smaller
        // of its two stretches), so they are neither stretched nor squeezed out of shape.
        let splits = {
            let c = Cv::new(&painter, s);
            let f = c.fresh();
            let (left, right) = (c.at(3.0, 200.0).x, c.at(2000.0, 200.0).x);
            let (gallery, nodes) = (c.at(737.5, 200.0).x, c.at(1525.0, 200.0).x);
            let (top, palettes) = (c.at(3.0, TOOLBAR2_END).y, f.at(0.0, 656.0).y);
            [
                Split::column(
                    "color gallery",
                    s,
                    c.at(737.5, TOOLBAR2_END),
                    c.at(737.5, 365.0),
                    left,
                    nodes,
                ),
                Split::column(
                    "color nodes",
                    s,
                    c.at(1525.0, TOOLBAR2_END),
                    c.at(1525.0, 365.0),
                    gallery,
                    right,
                ),
                Split::row(
                    "color clips",
                    s,
                    c.rv(0.0, 456.0),
                    c.rv(1919.0, 456.0),
                    top,
                    palettes,
                ),
                // Above the palettes: drag it to give the wheels and curves more room, or the
                // viewer. The row with the palettes' names moves with it unstretched.
                Split::row(
                    "color palettes",
                    s,
                    f.at(0.0, 656.0),
                    f.at(1919.0, 656.0),
                    top,
                    // Where the page bar under the palettes starts.
                    f.at(0.0, 1004.0).y,
                )
                .keeping(37.0 * R),
                Split::column(
                    "color scopes",
                    s,
                    f.at(1439.5, 693.0),
                    f.at(1439.5, 1047.0),
                    f.at(720.0, 693.0).x,
                    right,
                ),
            ]
        };
        let warped = self.warped(s, &splits);
        let s = &warped;
        let c = Cv::new(&painter, s);
        c.band(3.0, 62.0, 2000.0, PAGE_BAR, BG);

        self.color_toolbar2(ui, &c);
        self.color_gallery(ui, ctx, &c);
        self.color_viewer(ui, ctx, &c);
        self.color_nodes(ui, &c);
        self.color_clips(ui, ctx, &c);
        self.color_palette_row(ui, ctx, &c);
        match self.color.palette {
            Palette::Wheels => {
                self.color_primaries(ui, ctx, &c);
                self.color_curves(ui, &c);
            }
            Palette::Qualifier => self.color_qualifier(ui, ctx, &c),
            Palette::Window => self.color_window(ui, ctx, &c),
        }
        self.color_scopes(ui, ctx, &c);
        self.dividers(ui, s, &splits);
    }

    fn color_toolbar2(&mut self, ui: &mut Ui, c: &Cv) {
        c.band(3.0, 62.0, 2000.0, TOOLBAR2_END, TOOLBAR2);
        c.band(3.0, TOOLBAR2_END - 1.0, 2000.0, TOOLBAR2_END + 1.0, LINE);
        let y = 81.0;
        // The gallery's tools.
        c.icon(30.0, y, Icon::Sidebar, ICON, 0.9);
        c.icon(89.0, y, Icon::Download, ICON, 0.9);
        c.line((209.0, y), (385.0, y), 1.0, Color32::from_rgb(70, 70, 76));
        c.p.circle_filled(c.at(209.0, y), 5.5 * QU, GREY);
        c.icon(432.0, y, Icon::Sort, GREY, 0.9);
        c.icon(491.0, y, Icon::ThumbGrid, theme::WHITE, 0.95);
        c.icon(535.0, y, Icon::ListBullets, GREY, 0.75);
        c.icon(592.0, y, Icon::Search, GREY, 0.85);
        c.icon(650.0, y, Icon::Expand, ICON, 0.8);
        c.dots(707.0, y, GREY);
        for x in [60.0, 118.0, 405.0, 460.0, 562.0, 622.0, 680.0, 737.0] {
            c.vline(x, 62.0, TOOLBAR2_END);
        }
        // The viewer's header: the zoom it is shown at.
        let zoom = self.color_zoom(c);
        c.label(
            777.0,
            y,
            Align2::CENTER_CENTER,
            &format!("{zoom}%"),
            17.5,
            TEXT,
            true,
        );
        c.chevron(819.0, y, GREY);
        c.icon(868.0, y, Icon::ViewerSplit, ICON, 0.85);
        c.chevron(910.0, y, GREY);
        c.p.circle_stroke(
            c.at(950.0, y - 2.0),
            2.0,
            Stroke::new(1.0, Color32::from_rgb(70, 70, 76)),
        );
        let name = self.project().timeline_name.clone();
        let shown: String = if name.chars().count() > 8 {
            let chars: Vec<char> = name.chars().collect();
            let head: String = chars.iter().take(3).collect();
            let tail: String = chars[chars.len() - 3..].iter().collect();
            format!("{head}...{tail}")
        } else {
            name
        };
        c.label(1124.0, y, Align2::RIGHT_CENTER, &shown, 17.5, TEXT, true);
        c.chevron(1142.0, y, GREY);
        let time = self.tc(self.playhead as i64);
        c.label(1368.0, y, Align2::RIGHT_CENTER, &time, 17.5, TEXT, true);
        c.chevron(1387.0, y, GREY);
        let picker = ui
            .interact(
                c.rect(1020.0, 64.0, 1160.0, 98.0),
                ui.id().with("color_timeline_pick"),
                Sense::click(),
            )
            .on_hover_text("Timelines of the project");
        let mut chosen: Option<Option<u64>> = None;
        egui::Popup::menu(&picker).show(|ui| {
            let current = self.project().timeline_id;
            for (id, name) in self.project().timeline_list() {
                if ui.selectable_label(id == current, name).clicked() {
                    chosen = Some(Some(id));
                }
            }
            ui.separator();
            if ui.button("New Timeline...").clicked() {
                chosen = Some(None);
            }
        });
        match chosen {
            Some(Some(id)) => self.switch_timeline(id),
            Some(None) => self.new_timeline(),
            None => {}
        }
        c.icon(1438.0, y, Icon::ColorWipe, theme::WHITE, 0.9);
        c.dots(1497.0, y, GREY);
        for x in [838.0, 930.0, 1410.0, 1466.0, 1525.0] {
            c.vline(x, 62.0, TOOLBAR2_END);
        }
        // The node area's header.
        c.icon(1553.0, y, Icon::Pointer, GREY, 0.85);
        c.chevron(1587.0, y, GREY);
        c.icon(1638.0, y, Icon::Layers, GREY, 0.85);
        c.chevron(1675.0, y, GREY);
        c.label(
            1803.0,
            y,
            Align2::CENTER_CENTER,
            "Clip",
            17.5,
            theme::WHITE,
            true,
        );
        c.chevron(1838.0, y, theme::WHITE);
        c.line((1980.0, y), (1990.0, y - 4.0), 1.2, ICON);
        c.line((1980.0, y), (1990.0, y + 4.0), 1.2, ICON);
        for x in [1610.0, 1700.0, 1960.0] {
            c.vline(x, 62.0, TOOLBAR2_END);
        }
        if ui
            .interact(
                c.rect(67.0, 64.0, 111.0, 98.0),
                ui.id().with("color_grab_still"),
                Sense::click(),
            )
            .on_hover_text("Grab still (Ctrl+Alt+G)")
            .clicked()
        {
            self.grab_still();
        }
        // The tools that do nothing yet say so when clicked.
        let planned: [(f32, &str); 12] = [
            (30.0, "Gallery sidebar"),
            (432.0, "Sort stills"),
            (491.0, "Thumbnail view"),
            (535.0, "List view"),
            (592.0, "Search stills"),
            (650.0, "Larger gallery"),
            (707.0, "Gallery options"),
            (868.0, "Viewer split"),
            (1438.0, "Split screen wipe"),
            (1497.0, "Viewer options"),
            (1553.0, "Node selection tools"),
            (1638.0, "Node layers"),
        ];
        for (x, name) in planned {
            if ui
                .interact(
                    c.rect(x - 22.0, 64.0, x + 22.0, 98.0),
                    ui.id().with(("color_tb2", x as i32)),
                    Sense::click(),
                )
                .on_hover_text(name)
                .clicked()
            {
                self.planned(name);
            }
        }
    }

    /// The zoom the viewer shows the picture at, in percent of its full size.
    fn color_zoom(&self, c: &Cv) -> u32 {
        let area = c.rect(738.0, 145.0, 1525.0, 272.0).shrink(8.0);
        let (w, h) = (self.project().width as f32, self.project().height as f32);
        if w < 1.0 || h < 1.0 {
            return 100;
        }
        let scale = (area.width() / w).min(area.height() / h);
        (scale * 100.0).round().clamp(1.0, 999.0) as u32
    }

    /// Grabs the picture at the playhead, with the grade of its clip, into the gallery.
    pub(crate) fn grab_still(&mut self) {
        let Some(clip) = self.current_clip() else {
            self.fail("there is no clip at the playhead");
            return;
        };
        let Some((grade, nodes)) = self
            .project()
            .clip(clip)
            .map(|(_, c)| (c.grade.clone(), c.nodes.clone()))
        else {
            return;
        };
        let (w, h) = (self.project().width.max(1), self.project().height.max(1));
        let height = (320 * h / w).clamp(16, 640);
        let picture =
            picasso_render::render_frame(self.project(), &self.cache, self.playhead, 320, height)
                .ok()
                .and_then(|f| picasso_render::encode_png(&f).ok());
        let Some(png) = picture else {
            self.fail("could not make the still's picture");
            return;
        };
        let name = format!("1.{}.1", self.project().stills.len() + 1);
        if self
            .run(Command::GrabStill {
                name: name.clone(),
                frame: self.playhead,
                grade,
                nodes,
                png: picasso_core::base64::encode(&png),
            })
            .is_some()
        {
            self.color.still = self.project().stills.last().map(|s| s.id);
            self.info(format!("Grabbed still {name}"));
        }
    }

    /// Shows the chosen still beside the picture, or stops showing it.
    pub(crate) fn toggle_play_still(&mut self) {
        if self.color.still.is_none() {
            self.color.still = self.project().stills.last().map(|s| s.id);
        }
        self.color.playing_still = !self.color.playing_still && self.color.still.is_some();
    }

    /// The texture of still `id`, made from its PNG the first time.
    fn still_texture(&mut self, ctx: &Context, id: u64) -> Option<egui::TextureId> {
        if let Some(t) = self.color.still_textures.get(&id) {
            return Some(t.id());
        }
        let still = self.project().stills.iter().find(|s| s.id == id)?;
        let bytes = picasso_core::base64::decode(&still.png)?;
        let decoded = picasso_render::Decoded::from_bytes(&bytes).ok()?;
        let image = egui::ColorImage::from_rgba_unmultiplied(
            [decoded.width as usize, decoded.height as usize],
            &decoded.rgba,
        );
        let texture = ctx.load_texture(format!("still{id}"), image, TextureOptions::LINEAR);
        let tid = texture.id();
        self.color.still_textures.insert(id, texture);
        Some(tid)
    }

    /// The gallery: the stills as pictures with their names; a click chooses one, a double
    /// click gives its grade to the clip at the playhead, a right click has the rest.
    fn color_gallery(&mut self, ui: &mut Ui, ctx: &Context, c: &Cv) {
        c.band(3.0, TOOLBAR2_END + 1.0, 737.0, 365.0, BG);
        c.vline(737.0, TOOLBAR2_END, 365.0);
        let stills: Vec<(u64, String)> = self
            .project()
            .stills
            .iter()
            .map(|s| (s.id, s.name.clone()))
            .collect();
        // Forget the pictures of stills that are gone (undone or deleted).
        self.color
            .still_textures
            .retain(|id, _| stills.iter().any(|(s, _)| s == id));
        if stills.is_empty() {
            c.text(
                361.0,
                235.0,
                Align2::CENTER_CENTER,
                "No stills created",
                347.0,
                Color32::from_rgb(88, 88, 94),
                false,
            );
            return;
        }
        let (w, h) = (150.0, 84.0);
        let columns = 4;
        let mut apply = None;
        let mut delete = None;
        let mut play = None;
        for (n, (id, name)) in stills.iter().enumerate() {
            let (col, row) = ((n % columns) as f32, (n / columns) as f32);
            let x0 = 24.0 + col * (w + 24.0);
            let y0 = TOOLBAR2_END + 16.0 + row * (h + 40.0);
            if y0 + h > 365.0 {
                break;
            }
            let pic = c.rect(x0, y0, x0 + w, y0 + h);
            c.p.rect_filled(pic, 2.0, Color32::from_rgb(14, 14, 16));
            if let Some(tex) = self.still_texture(ctx, *id) {
                c.p.image(
                    tex,
                    pic,
                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            let chosen = self.color.still == Some(*id);
            c.p.rect_stroke(
                pic,
                2.0,
                if chosen {
                    Stroke::new(2.0, Color32::from_rgb(214, 128, 60))
                } else {
                    Stroke::new(1.0, Color32::from_rgb(58, 58, 64))
                },
                StrokeKind::Inside,
            );
            c.label(
                x0 + w / 2.0,
                y0 + h + 16.0,
                Align2::CENTER_CENTER,
                name,
                15.0,
                TEXT,
                false,
            );
            let r = ui
                .interact(pic, ui.id().with(("still", *id)), Sense::click())
                .on_hover_text("Double-click to apply its grade; right click for more");
            if r.clicked() {
                self.color.still = Some(*id);
            }
            if r.double_clicked() {
                apply = Some(*id);
            }
            r.context_menu(|ui| {
                if ui.button("Apply Grade").clicked() {
                    apply = Some(*id);
                    ui.close();
                }
                if ui.button("Play Still").clicked() {
                    play = Some(*id);
                    ui.close();
                }
                ui.separator();
                if ui.button("Delete Still").clicked() {
                    delete = Some(*id);
                    ui.close();
                }
            });
        }
        if let Some(id) = apply {
            match self.current_clip() {
                Some(clip) => {
                    self.run(Command::ApplyStill { still: id, clip });
                }
                None => self.fail("there is no clip at the playhead to grade"),
            }
        }
        if let Some(id) = play {
            self.color.still = Some(id);
            self.color.playing_still = true;
        }
        if let Some(id) = delete {
            if self.run(Command::DeleteStill { id }).is_some() && self.color.still == Some(id) {
                self.color.still = None;
                self.color.playing_still = false;
            }
        }
    }

    fn color_viewer(&mut self, ui: &mut Ui, ctx: &Context, c: &Cv) {
        c.band(738.0, TOOLBAR2_END + 1.0, 1525.0, 145.0, BG);
        c.band(738.0, 272.0, 1525.0, 365.0, BG);
        // The picture, or an empty frame when there is no clip at the playhead.
        let area = c.rect(738.0, 145.0, 1525.0, 272.0);
        let has_clip = self.current_clip().is_some();
        let aspect = self.project().width as f32 / self.project().height.max(1) as f32;
        let shown = self
            .viewer
            .paint(ui, area, aspect, VIEW, has_clip, self.overlays);
        if !has_clip {
            c.p.rect_filled(area, 0.0, VIEW);
        }
        if self.color.playing_still {
            if let Some(tex) = self.color.still.and_then(|id| self.still_texture(ctx, id)) {
                let half = Rect::from_min_max(pos2(shown.center().x, shown.top()), shown.max);
                c.p.image(
                    tex,
                    half,
                    Rect::from_min_max(pos2(0.5, 0.0), pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
                c.p.line_segment(
                    [half.left_top(), half.left_bottom()],
                    Stroke::new(1.5, Color32::from_rgb(230, 230, 232)),
                );
            } else {
                self.color.playing_still = false;
            }
        }
        // A click in the picture samples it for a picker.
        let response = ui
            .interact(shown, ui.id().with("color_viewer"), Sense::click())
            .on_hover_cursor(if self.color.picker.is_some() {
                egui::CursorIcon::Crosshair
            } else {
                egui::CursorIcon::Default
            });
        if let (Some(picker), true) = (self.color.picker, response.clicked()) {
            if let Some(pos) = response.interact_pointer_pos() {
                let u = (pos.x - shown.left()) / shown.width().max(1.0);
                let v = (pos.y - shown.top()) / shown.height().max(1.0);
                self.pick(ctx, picker, u, v);
            }
            self.color.picker = None;
        }
        self.window_overlay(ui, ctx, c, shown);

        // The playhead over the whole timeline.
        let duration = self.project().duration().max(1);
        let bar = c.rect(775.0, 292.0, 1488.0, 296.0);
        c.p.rect_filled(bar, 1.0, Color32::from_rgb(70, 70, 76));
        let frac = (self.playhead as f32 / duration as f32).clamp(0.0, 1.0);
        let x = bar.left() + bar.width() * frac;
        c.p.rect_filled(
            Rect::from_center_size(pos2(x, bar.center().y), vec2(3.5, 13.0)),
            1.0,
            theme::WHITE,
        );
        let scrub = ui.interact(
            bar.expand2(vec2(6.0, 9.0)),
            ui.id().with("color_scrub"),
            Sense::click_and_drag(),
        );
        if scrub.clicked() || scrub.dragged() {
            if let Some(p) = scrub.interact_pointer_pos() {
                let f = ((p.x - bar.left()) / bar.width()).clamp(0.0, 1.0);
                self.playing = false;
                self.go_to((f * duration as f32) as u64);
            }
        }

        // The transport row.
        let y = 337.0;
        let neutral_on = self.color.picker == Some(Picker::Neutral);
        c.icon(
            785.0,
            y,
            Icon::Picker,
            if neutral_on { theme::WHITE } else { GREY },
            0.85,
        );
        c.chevron(816.0, y, GREY);
        c.icon(857.0, y, Icon::Layers, ICON, 0.8);
        c.icon(907.0, y, Icon::Speaker, theme::WHITE, 0.8);
        let buttons: [(f32, Icon, Act); 6] = [
            (968.0, Icon::ToStart, Act::GoFirst),
            (1020.0, Icon::PlayReverse, Act::PreviousFrame),
            (1068.0, Icon::Stop, Act::Stop),
            (1117.0, Icon::Play, Act::TogglePlay),
            (1170.0, Icon::ToEnd, Act::GoLast),
            (1229.0, Icon::Loop, Act::ToggleLoop),
        ];
        let tc = self.tc(self.playhead as i64);
        c.label(
            1487.0,
            y,
            Align2::RIGHT_CENTER,
            &tc,
            19.5,
            theme::WHITE,
            true,
        );
        for (x, icon, act) in buttons {
            let lit = (icon == Icon::Play && self.playing) || (icon == Icon::Loop && self.looping);
            c.icon(x, y, icon, if lit { theme::WHITE } else { GREY }, 0.9);
            if ui
                .interact(
                    c.rect(x - 22.0, y - 20.0, x + 22.0, y + 20.0),
                    ui.id().with(("color_transport", x as i32)),
                    Sense::click(),
                )
                .clicked()
            {
                self.do_act(ctx, act);
            }
        }
        if ui
            .interact(
                c.rect(765.0, y - 20.0, 826.0, y + 20.0),
                ui.id().with("color_neutral_picker"),
                Sense::click(),
            )
            .on_hover_text("Pick a colour in the viewer to make neutral")
            .clicked()
        {
            self.color.picker = if neutral_on {
                None
            } else {
                Some(Picker::Neutral)
            };
        }
        for (x, name) in [(857.0, "Viewer layers"), (907.0, "Viewer audio")] {
            if ui
                .interact(
                    c.rect(x - 20.0, y - 20.0, x + 20.0, y + 20.0),
                    ui.id().with(("color_vt", x as i32)),
                    Sense::click(),
                )
                .clicked()
            {
                self.planned(name);
            }
        }
        c.vline(1525.0, TOOLBAR2_END, 365.0);
    }

    /// Changes the grade so that the colour at (`u`, `v`) of the viewer (0..1 across and down)
    /// becomes neutral, black or white.
    fn pick(&mut self, ctx: &Context, picker: Picker, u: f32, v: f32) {
        let Some(id) = self.current_clip() else {
            return;
        };
        let Some(frame) = self.viewer.frame.as_ref() else {
            return;
        };
        let (w, h) = (frame.width as usize, frame.height as usize);
        let x = ((u.clamp(0.0, 1.0) * w as f32) as usize).min(w.saturating_sub(1));
        let y = ((v.clamp(0.0, 1.0) * h as f32) as usize).min(h.saturating_sub(1));
        let Some(px) = frame.rgba.get((y * w + x) * 4..(y * w + x) * 4 + 3) else {
            return;
        };
        let sample = [0, 1, 2].map(|c| (f32::from(px[c]) / 255.0).max(1.0 / 255.0));
        let Some((_, clip)) = self.project().clip(id) else {
            return;
        };
        let mut grade = clip
            .node(self.node_for(clip.id))
            .cloned()
            .unwrap_or_default();
        match picker {
            Picker::Neutral => {
                let y = picasso_color::LUMA
                    .iter()
                    .zip(sample)
                    .map(|(l, s)| l * s)
                    .sum::<f32>();
                for (gain, s) in grade.gain.iter_mut().zip(sample) {
                    *gain = (*gain * y / s).clamp(0.0, 10.0);
                }
            }
            Picker::Black => {
                for (offset, s) in grade.offset.iter_mut().zip(sample) {
                    *offset = (*offset - s).clamp(-2.0, 2.0);
                }
            }
            Picker::White => {
                for (gain, s) in grade.gain.iter_mut().zip(sample) {
                    *gain = (*gain / s).clamp(0.0, 10.0);
                }
            }
            Picker::Qualify => {
                let keep = grade.qualifier.as_ref().map(|q| (q.invert, q.blur));
                let mut q = picasso_color::Qualifier::pick(sample);
                if let Some((invert, blur)) = keep {
                    (q.invert, q.blur) = (invert, blur);
                }
                grade.qualifier = Some(q);
                self.color.highlight = true;
            }
        }
        self.run(Command::SetGrade {
            clip: id,
            grade,
            node: self.node_for(id),
        });
        ctx.request_repaint();
    }

    fn color_nodes(&mut self, ui: &mut Ui, c: &Cv) {
        c.band(
            1526.0,
            TOOLBAR2_END + 1.0,
            2000.0,
            318.0,
            Color32::from_rgb(34, 34, 38),
        );
        let grid = Color32::from_rgb(40, 40, 45);
        for i in 0..6 {
            let x = 1580.0 + 80.0 * i as f32;
            c.line((x, TOOLBAR2_END + 1.0), (x, 318.0), 1.0, grid);
        }
        for y in [130.0, 180.0, 230.0, 280.0] {
            c.line((1526.0, y), (2000.0, y), 1.0, grid);
        }
        c.band(1526.0, 318.0, 2000.0, 365.0, BG);
        c.line((1526.0, 318.0), (2000.0, 318.0), 1.0, LINE);
        c.p.circle_filled(c.at(1798.0, 341.0), 3.0 * R, theme::WHITE);
        c.p.circle_filled(c.at(1828.0, 341.0), 2.5 * R, Color32::from_rgb(64, 64, 70));

        // The clip's serial nodes, left to right from the source (green) to the output
        // (blue). A click picks the node the wheels edit; a right click has the node's menu.
        let Some(id) = self.current_clip() else {
            c.text(
                1763.0,
                207.0,
                Align2::CENTER_CENTER,
                "No clip to grade",
                130.0,
                GREY,
                false,
            );
            return;
        };
        let Some((_, clip)) = self.project().clip(id) else {
            return;
        };
        let nodes: Vec<(bool, bool, Option<String>)> = (0..clip.node_count())
            .filter_map(|n| clip.node(n))
            .map(|g| {
                (
                    g.enabled,
                    g.has_effect(),
                    (!g.label.is_empty())
                        .then(|| g.label.clone())
                        .or_else(|| g.lut.as_ref().map(|l| l.title.clone())),
                )
            })
            .collect();
        let modes: Vec<picasso_color::NodeMode> = (0..clip.node_count())
            .filter_map(|n| clip.node(n))
            .map(|g| g.mode)
            .collect();
        let current = self.node_for(id);
        let count = nodes.len().max(1);
        let thumb = self
            .project()
            .clip(id)
            .map(|(_, clip)| clip.media)
            .and_then(|m| self.media_thumb(ui.ctx(), m));
        let green = Color32::from_rgb(150, 200, 40);
        let blue = Color32::from_rgb(70, 140, 230);
        // Resolve's pixels at 1080p: the source at the lower left, the output at the lower right,
        // the nodes between them, each 98 pixels wide.
        let node_x = |n: usize| 1355.0 + 140.0 * n as f32;
        let wire = Color32::from_rgb(120, 120, 126);
        c.p.circle_filled(c.rv(1323.0, 263.0), 4.0 * R, green);
        c.p.circle_filled(c.rv(1901.0, 263.0), 4.0 * R, green);
        c.p.line_segment(
            [c.rv(1323.0, 259.0), c.rv(node_x(0) - 8.0, 143.0)],
            Stroke::new(1.0, wire),
        );
        c.p.line_segment(
            [c.rv(node_x(count - 1) + 105.0, 143.0), c.rv(1897.0, 260.0)],
            Stroke::new(1.0, wire),
        );
        let ctx = ui.ctx().clone();
        for (n, (enabled, graded, lut)) in nodes.into_iter().enumerate() {
            let x0 = node_x(n);
            let x1 = x0 + 98.0;
            let (top, bottom) = (122.0, 196.0);
            if n > 0 {
                c.p.line_segment(
                    [c.rv(node_x(n - 1) + 105.0, 143.0), c.rv(x0 - 8.0, 143.0)],
                    Stroke::new(1.0, wire),
                );
            }
            let rect = c.rrect(x0, top, x1, bottom);
            let fill = if enabled {
                Color32::from_rgb(58, 58, 64)
            } else {
                Color32::from_rgb(44, 44, 48)
            };
            c.p.rect_filled(rect, 2.0, fill);
            // The picture, a blue line under it, the number under that.
            let pic = c.rrect(x0 + 5.0, top + 4.0, x1 - 5.0, 172.0);
            c.p.rect_filled(pic, 1.0, Color32::from_rgb(14, 14, 16));
            if let Some(tex) = thumb {
                c.p.image(
                    tex,
                    pic,
                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    if enabled {
                        Color32::WHITE
                    } else {
                        Color32::from_gray(90)
                    },
                );
            }
            c.p.line_segment(
                [c.rv(x0 + 5.0, 175.0), c.rv(x1 - 5.0, 175.0)],
                Stroke::new(1.5, Color32::from_rgb(110, 140, 190)),
            );
            let outline = if n == current {
                Stroke::new(1.5, Color32::from_rgb(226, 62, 52))
            } else {
                Stroke::new(1.0, Color32::from_rgb(92, 92, 98))
            };
            c.p.rect_stroke(rect, 2.0, outline, StrokeKind::Outside);
            let label = if enabled { TEXT } else { GREY };
            crate::i18n::paint_text(&c.p,
                c.rv(x0 + 10.0, 186.0),
                Align2::LEFT_CENTER,
                format!("{:02}", n + 1),
                theme::regular(9.0 * R),
                label,
            );
            // Inputs: a green and a blue triangle; outputs: a green and a blue square.
            for (yy, col) in [(143.0, green), (175.0, blue)] {
                let p = c.rv(x0 - 8.0, yy);
                let u = R;
                c.p.add(egui::Shape::convex_polygon(
                    vec![
                        p + vec2(-3.0 * u, -3.5 * u),
                        p + vec2(3.0 * u, 0.0),
                        p + vec2(-3.0 * u, 3.5 * u),
                    ],
                    col,
                    Stroke::NONE,
                ));
                c.p.rect_filled(
                    Rect::from_center_size(c.rv(x1 + 6.0, yy), vec2(6.0 * u, 6.0 * u)),
                    0.0,
                    col,
                );
            }
            if let Some(mode) = modes
                .get(n)
                .filter(|m| **m != picasso_color::NodeMode::Serial)
            {
                crate::i18n::paint_text(&c.p,
                    c.rv(x0 + 8.0, top + 9.0),
                    Align2::LEFT_CENTER,
                    mode.name().to_uppercase(),
                    theme::regular(6.5 * R),
                    theme::ACCENT,
                );
            }
            // A small mark shows that the node changes the picture.
            if graded {
                c.p.circle_filled(c.rv(x1 - 10.0, 186.0), 2.0 * R, theme::ACCENT);
            }
            let caption = if !enabled {
                Some("off".to_string())
            } else {
                lut
            };
            if let Some(caption) = caption {
                crate::i18n::paint_text(&c.p,
                    c.rv((x0 + x1) / 2.0 + 6.0, 186.0),
                    Align2::CENTER_CENTER,
                    shorten_caption(&caption, 10),
                    theme::regular(7.5 * R),
                    GREY,
                );
            }
            let resp = ui
                .interact(rect, ui.id().with(("color_node", n)), Sense::click())
                .on_hover_text("Click to grade this node; right click for more");
            if resp.clicked() || resp.secondary_clicked() {
                self.color_node = n;
            }
            let has_lut = self
                .project()
                .clip(id)
                .and_then(|(_, c)| c.node(n))
                .is_some_and(|g| g.lut.is_some());
            resp.context_menu(|ui| {
                if ui.button("Add Serial Node").clicked() {
                    self.color_node = n;
                    self.do_act(&ctx, Act::AddSerialNode);
                    ui.close();
                }
                if ui.button("Label Node").clicked() {
                    self.color_node = n;
                    self.open_label_dialog();
                    ui.close();
                }
                if n > 0 {
                    if let Some(mut grade) = self
                        .project()
                        .clip(id)
                        .and_then(|(_, c)| c.node(n))
                        .cloned()
                    {
                        let before = (grade.mode, grade.mix);
                        ui.menu_button("Node Mode", |ui| {
                            for mode in [
                                picasso_color::NodeMode::Serial,
                                picasso_color::NodeMode::Parallel,
                                picasso_color::NodeMode::Layer,
                            ] {
                                ui.selectable_value(&mut grade.mode, mode, mode.name());
                            }
                        });
                        if grade.mode != picasso_color::NodeMode::Serial {
                            ui.add(
                                egui::Slider::new(&mut grade.mix, 0.0..=100.0)
                                    .text("Mix")
                                    .suffix("%"),
                            );
                        }
                        if (grade.mode, grade.mix) != before {
                            self.run_gesture(
                                &ctx,
                                Command::SetGrade {
                                    clip: id,
                                    grade,
                                    node: n,
                                },
                            );
                        }
                    }
                }
                let toggle = if enabled {
                    "Disable Node"
                } else {
                    "Enable Node"
                };
                if ui.button(toggle).clicked() {
                    self.run(Command::SetNodeEnabled {
                        clip: id,
                        node: n,
                        enabled: !enabled,
                    });
                    ui.close();
                }
                if ui.button("Reset Node Grade").clicked() {
                    self.run(Command::SetGrade {
                        clip: id,
                        grade: ColorGrade::default(),
                        node: n,
                    });
                    ui.close();
                }
                if has_lut && ui.button("Remove LUT").clicked() {
                    if let Some(mut grade) = self
                        .project()
                        .clip(id)
                        .and_then(|(_, c)| c.node(n))
                        .cloned()
                    {
                        grade.lut = None;
                        self.run(Command::SetGrade {
                            clip: id,
                            grade,
                            node: n,
                        });
                    }
                    ui.close();
                }
                ui.separator();
                if ui
                    .add_enabled(count > 1, egui::Button::new("Delete Node"))
                    .clicked()
                {
                    if self
                        .run(Command::DeleteNode { clip: id, node: n })
                        .is_some()
                    {
                        self.color_node = n.saturating_sub(1);
                    }
                    ui.close();
                }
            });
        }
    }

    /// The clips of the timeline with their codec, and under them the whole timeline with its
    /// ruler, as in Resolve at 1080p; a click goes to a clip and grades it.
    fn color_clips(&mut self, ui: &mut Ui, ctx: &Context, c: &Cv) {
        c.p.rect_filled(c.rrect(0.0, 456.0, 1919.0, 656.0), 0.0, BG);
        c.p.line_segment(
            [c.rv(0.0, 456.0), c.rv(1919.0, 456.0)],
            Stroke::new(1.0, LINE),
        );
        let clips = self.strip_clips();
        let at = self.playhead;
        let current = self
            .project()
            .tracks
            .iter()
            .rev()
            .filter(|t| t.kind == picasso_core::TrackKind::Video && !t.muted && t.picture)
            .find_map(|t| picasso_core::Project::clip_at(t, at))
            .map(|clip| clip.id);
        let mut go = None;
        for (n, clip) in clips.iter().enumerate() {
            let x = 8.0 + 140.0 * n as f32;
            if x > 1800.0 {
                break;
            }
            let number = Rect::from_min_max(c.rv(x, 463.0), c.rv(x + 25.0, 478.0));
            c.p.rect_filled(number, 2.0, Color32::from_rgb(20, 20, 24));
            c.p.rect_stroke(
                number,
                2.0,
                Stroke::new(1.0, Color32::from_rgb(70, 70, 76)),
                StrokeKind::Inside,
            );
            crate::i18n::paint_text(&c.p,
                number.center(),
                Align2::CENTER_CENTER,
                format!("{:02}", n + 1),
                theme::semibold(9.5 * R),
                TEXT,
            );
            let length = self
                .project()
                .fps
                .timecode(clip.end.saturating_sub(clip.start));
            crate::i18n::paint_text(&c.p,
                c.rv(x + 30.0, 470.0),
                Align2::LEFT_CENTER,
                length,
                theme::semibold(9.5 * R),
                TEXT,
            );
            crate::i18n::paint_text(&c.p,
                c.rv(x + 111.0, 470.0),
                Align2::LEFT_CENTER,
                &clip.track,
                theme::regular(9.5 * R),
                LABEL,
            );
            let pic = Rect::from_min_max(c.rv(x, 486.0), c.rv(x + 129.0, 557.0));
            c.p.rect_filled(pic, 2.0, Color32::from_rgb(12, 12, 14));
            if let Some(tex) = self.media_thumb(ctx, clip.media) {
                c.p.image(
                    tex,
                    pic.shrink(1.5),
                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            c.p.rect_stroke(
                pic,
                2.0,
                if current == Some(clip.id) {
                    Stroke::new(1.5, Color32::from_rgb(226, 62, 52))
                } else {
                    Stroke::new(1.0, Color32::from_rgb(54, 54, 60))
                },
                StrokeKind::Inside,
            );
            let codec = self.codec_label(clip.media);
            crate::i18n::paint_text(&c.p,
                c.rv(x - 1.0, 570.0),
                Align2::LEFT_CENTER,
                codec,
                theme::regular(10.0 * R),
                TEXT,
            );
            if ui
                .interact(
                    Rect::from_min_max(number.min, pic.max),
                    ui.id().with(("strip", clip.id.0)),
                    Sense::click(),
                )
                .on_hover_text(format!(
                    "{}  {}",
                    self.tc(clip.start as i64),
                    self.tc(clip.end as i64)
                ))
                .clicked()
            {
                go = Some((clip.id, clip.start));
            }
        }
        if let Some((id, start)) = go {
            self.playing = false;
            self.go_to(start);
            self.select_only(id);
        }

        // The whole timeline: its ruler, a row of clips for each picture track, and the playhead.
        let duration = self.project().duration().max(1);
        let span = (1913.0 - 40.0) as f32;
        c.p.rect_filled(
            c.rrect(0.0, 590.0, 1919.0, 626.0),
            0.0,
            Color32::from_rgb(36, 36, 40),
        );
        let ticks = (0..10).map(|n| {
            let x = 40.0 + 200.0 * n as f32;
            (x, ((x - 40.0) / span * duration as f32) as u64)
        });
        for (x, frame) in ruler_ticks(ticks) {
            c.p.line_segment(
                [c.rv(x, 593.0), c.rv(x, 615.0)],
                Stroke::new(1.0, Color32::from_rgb(70, 70, 76)),
            );
            crate::i18n::paint_text(&c.p,
                c.rv(x + 6.0, 603.0),
                Align2::LEFT_CENTER,
                self.tc(frame as i64),
                theme::regular(8.5 * R),
                LABEL,
            );
        }
        // Each picture track as a row of its clips where they sit on the timeline, V1 at the
        // bottom as on the Edit page; up to three rows share the strip. The clip being graded
        // has a red edge, and a track whose picture is off is dimmed.
        let to_x = |frame: u64| 40.0 + (frame as f32 / duration as f32).clamp(0.0, 1.0) * span;
        let videos: Vec<TimelineRow> = self
            .project()
            .tracks
            .iter()
            .filter(|t| t.kind == picasso_core::TrackKind::Video)
            .take(3)
            .map(|t| {
                (
                    t.name.clone(),
                    !t.muted && t.picture,
                    t.clips
                        .iter()
                        .map(|clip| (clip.id, clip.start, clip.start + clip.duration))
                        .collect(),
                )
            })
            .collect();
        let rows = videos.len().max(1) as f32;
        let (band_top, band_bottom) = (629.0, 647.0);
        let row_h = (band_bottom - band_top) / rows;
        for (i, (name, on, clips)) in videos.iter().enumerate() {
            let bottom = band_bottom - row_h * i as f32;
            let top = bottom - row_h + 1.0;
            crate::i18n::paint_text(&c.p,
                c.rv(6.0, (top + bottom) / 2.0),
                Align2::LEFT_CENTER,
                name,
                theme::regular((8.5 * R).min(row_h * 0.8)),
                LABEL,
            );
            let fill = if *on {
                Color32::from_rgb(52, 116, 170)
            } else {
                Color32::from_rgb(52, 62, 74)
            };
            for (id, start, end) in clips {
                // At least a point wide, and a gap between clips that touch.
                let (x0, x1) = (to_x(*start), to_x(*end));
                let rect = c.rrect(x0, top, (x1 - 1.0).max(x0 + 1.0), bottom);
                c.p.rect_filled(rect, 1.0, fill);
                if current == Some(*id) {
                    c.p.rect_stroke(
                        rect,
                        1.0,
                        Stroke::new(1.2, Color32::from_rgb(226, 62, 52)),
                        StrokeKind::Inside,
                    );
                }
            }
        }
        let x = to_x(self.playhead);
        c.p.line_segment(
            [c.rv(x, 594.0), c.rv(x, 648.0)],
            Stroke::new(1.0, Color32::from_rgb(222, 64, 52)),
        );
        c.p.circle_filled(c.rv(x, 598.0), 4.0 * R, Color32::from_rgb(222, 64, 52));
        let scrub = ui.interact(
            c.rrect(33.0, 592.0, 1913.0, 650.0),
            ui.id().with("color_timeline_scrub"),
            Sense::click_and_drag(),
        );
        if scrub.clicked() || scrub.dragged() {
            if let Some(p) = scrub.interact_pointer_pos() {
                let (a, b) = (c.rv(40.0, 0.0).x, c.rv(1913.0, 0.0).x);
                let f = ((p.x - a) / (b - a)).clamp(0.0, 1.0);
                self.playing = false;
                self.go_to((f * duration as f32) as u64);
            }
        }
    }

    fn color_palette_row(&mut self, ui: &mut Ui, ctx: &Context, c: &Cv) {
        // Laid out on Resolve at 1080p (`docs/reference/resolve-1080p/color.png`): the primary
        // palettes on the left, the secondary ones in the middle, keyframes, scopes and info on
        // the right.
        let c = c.fresh();
        c.band(0.0, 656.0, 1919.0, 692.0, BAR);
        c.line((0.0, 655.5), (1919.0, 655.5), 1.0, LINE);
        c.line((0.0, 692.5), (1919.0, 692.5), 1.0, LINE);
        let y = 674.0;
        let palettes: [(f32, Icon, &str); 16] = [
            (31.5, Icon::CamRaw, "Camera Raw"),
            (91.0, Icon::ColorMatch, "Color Match"),
            (150.0, Icon::WheelDot, "Color Wheels"),
            (211.0, Icon::WheelHdr, "HDR Wheels"),
            (268.0, Icon::RgbMixer, "RGB Mixer"),
            (331.5, Icon::MotionFx, "Motion Effects"),
            (780.0, Icon::FxCurves, "Curves"),
            (837.0, Icon::Hexagon, "Color Warper"),
            (900.0, Icon::Qualifier, "Qualifier"),
            (962.0, Icon::CircleWindow, "Window"),
            (1020.0, Icon::Tracker, "Tracker"),
            (1081.0, Icon::MagicMask, "Magic Mask"),
            (1140.0, Icon::BlurBox, "Blur"),
            (1200.0, Icon::Needle, "Key"),
            (1260.0, Icon::Sizing, "Sizing"),
            (1320.0, Icon::Box3D, "3D"),
        ];
        let mut planned: Option<&str> = None;
        for (x, icon, name) in palettes {
            let r = ui
                .interact(
                    c.rect(x - 28.0, 657.0, x + 28.0, 691.0),
                    ui.id().with(("color_palette", x as i32)),
                    Sense::click(),
                )
                .on_hover_text(name);
            let palette = match icon {
                Icon::WheelDot | Icon::FxCurves => Some(Palette::Wheels),
                Icon::Qualifier => Some(Palette::Qualifier),
                Icon::CircleWindow => Some(Palette::Window),
                _ => None,
            };
            let selected = palette == Some(self.color.palette)
                && (icon != Icon::FxCurves || self.color.palette == Palette::Wheels);
            let lit = icon == Icon::WheelDot || icon == Icon::FxCurves;
            let color = if lit && selected {
                theme::WHITE
            } else if r.hovered() {
                theme::ICON_HOVER
            } else {
                ICON
            };
            c.icon(x, y, icon, color, 0.62);
            if r.clicked() {
                match palette {
                    Some(p) => self.color.palette = p,
                    None => planned = Some(name),
                }
            }
        }
        c.icon(1772.0, y, Icon::Diamonds3, ICON, 0.62);
        let scopes = ui
            .interact(
                c.rect(1810.0, 657.0, 1850.0, 691.0),
                ui.id().with("color_scopes_toggle"),
                Sense::click(),
            )
            .on_hover_text("Scopes");
        c.icon(
            1829.0,
            y,
            Icon::ScopeGraph,
            if self.scopes_visible {
                theme::WHITE
            } else {
                ICON
            },
            0.62,
        );
        if scopes.clicked() {
            self.do_act(ctx, Act::ToggleScopes);
        }
        // Info: a ring with an "i".
        let info = c.at(1889.0, y);
        c.p.circle_stroke(info, 7.0 * R, Stroke::new(1.2, ICON));
        crate::i18n::paint_text(&c.p,
            info,
            Align2::CENTER_CENTER,
            "i",
            theme::semibold(10.0 * R),
            ICON,
        );
        if ui
            .interact(
                c.rect(1752.0, 657.0, 1792.0, 691.0),
                ui.id().with("color_keyframes"),
                Sense::click(),
            )
            .clicked()
        {
            planned = Some("Keyframes");
        }
        if let Some(name) = planned {
            self.planned(name);
        }
    }

    /// The primary wheels: the header, the row of tone values, four wheels with their
    /// channel values and jog bars, and the row of extra values at the bottom.
    fn color_primaries(&mut self, ui: &mut Ui, ctx: &Context, c: &Cv) {
        let c = &c.fresh();
        c.band(0.0, 693.0, 719.0, 717.0, BAR);
        c.label(
            11.0,
            705.0,
            Align2::LEFT_CENTER,
            "Primaries - Color Wheels",
            11.5,
            theme::WHITE,
            true,
        );
        c.icon(606.0, 705.0, Icon::WheelDot, theme::WHITE, 0.5);
        c.icon(634.0, 705.0, Icon::BarsVert, GREY, 0.5);
        c.icon(662.0, 705.0, Icon::WheelLog, GREY, 0.5);
        c.icon(700.0, 705.0, Icon::Reset, GREY, 0.5);
        c.vline(719.5, 693.0, 1004.0);
        c.line((0.0, 717.5), (719.0, 717.5), 1.0, LINE);
        c.line((0.0, 752.5), (719.0, 752.5), 1.0, LINE);
        for (x, name) in [(634.0, "Primaries bars"), (662.0, "Log wheels")] {
            if ui
                .interact(
                    c.rect(x - 12.0, 695.0, x + 12.0, 715.0),
                    ui.id().with(("color_mode", x as i32)),
                    Sense::click(),
                )
                .on_hover_text(name)
                .clicked()
            {
                self.planned(name);
            }
        }

        let id = self.current_clip();
        let before = id
            .and_then(|id| self.project().clip(id))
            .map(|(_, clip)| {
                clip.node(self.node_for(clip.id))
                    .cloned()
                    .unwrap_or_default()
            })
            .unwrap_or_default();
        let mut grade = before.clone();

        if ui
            .interact(
                c.rect(688.0, 695.0, 712.0, 715.0),
                ui.id().with("color_reset_all"),
                Sense::click(),
            )
            .on_hover_text("Reset the grade")
            .clicked()
        {
            let lut = grade.lut.take();
            grade = ColorGrade {
                lut,
                ..ColorGrade::default()
            };
        }

        // Auto balance and the neutral picker.
        c.icon(24.0, 733.0, Icon::AutoA, GREY, 0.5);
        let pick_on = self.color.picker == Some(Picker::Neutral);
        c.icon(
            57.0,
            733.0,
            Icon::Picker,
            if pick_on { theme::WHITE } else { GREY },
            0.5,
        );
        if ui
            .interact(
                c.rect(12.0, 723.0, 36.0, 743.0),
                ui.id().with("color_auto"),
                Sense::click(),
            )
            .on_hover_text("Auto balance")
            .clicked()
        {
            self.auto_balance(&mut grade);
        }
        if ui
            .interact(
                c.rect(45.0, 723.0, 69.0, 743.0),
                ui.id().with("color_pick_neutral"),
                Sense::click(),
            )
            .on_hover_text("Pick a colour in the viewer to make neutral")
            .clicked()
        {
            self.color.picker = if pick_on { None } else { Some(Picker::Neutral) };
        }

        // Temp, Tint, Contrast, Pivot and Mid/Detail: the label's right edge, then the box.
        let top: [(&str, f32, f32, f32); 5] = [
            ("Temp", 151.0, 156.0, 198.0),
            ("Tint", 274.0, 279.0, 321.0),
            ("Contrast", 398.0, 402.0, 445.0),
            ("Pivot", 523.0, 527.0, 568.0),
            ("Mid/Detail", 646.0, 652.0, 692.0),
        ];
        for (n, (label, lx, x0, x1)) in top.into_iter().enumerate() {
            c.label(lx, 733.0, Align2::RIGHT_CENTER, label, 10.5, LABEL, false);
            let (value, text, per, neutral) = match n {
                0 => (
                    grade.temperature,
                    format!("{:.1}", grade.temperature),
                    10.0,
                    0.0,
                ),
                1 => (grade.tint, format!("{:.2}", grade.tint), 0.25, 0.0),
                2 => (grade.contrast, format!("{:.3}", grade.contrast), 0.005, 1.0),
                3 => (grade.pivot, format!("{:.3}", grade.pivot), 0.002, 0.435),
                _ => (
                    grade.midtone_detail,
                    format!("{:.2}", grade.midtone_detail),
                    0.5,
                    0.0,
                ),
            };
            if let Some(v) = value_box(
                ui,
                c,
                ("color_top", n),
                (x0, 723.0, x1, 741.0),
                value,
                &text,
                per,
                neutral,
            ) {
                match n {
                    0 => grade.temperature = v,
                    1 => grade.tint = v,
                    2 => grade.contrast = v.clamp(0.0, 2.0),
                    3 => grade.pivot = v.clamp(0.05, 0.95),
                    _ => grade.midtone_detail = v,
                }
            }
            let (l, r) = match n {
                0 => (
                    Color32::from_rgb(60, 110, 220),
                    Color32::from_rgb(230, 150, 50),
                ),
                1 => (
                    Color32::from_rgb(60, 190, 90),
                    Color32::from_rgb(200, 70, 190),
                ),
                2 => (
                    Color32::from_rgb(10, 10, 10),
                    Color32::from_rgb(230, 230, 230),
                ),
                _ => (Color32::from_rgb(70, 70, 76), Color32::from_rgb(70, 70, 76)),
            };
            underline_thin(c, x0 + 6.0, x1 - 6.0, 743.0, l, r);
        }

        // The four wheels: centre, picker, name and reset icons.
        let centers = [92.0, 270.0, 448.0, 626.0];
        let names = [96.0, 272.0, 451.0, 628.0];
        let resets = [158.0, 336.0, 514.0, 692.0];
        let pickers = [32.0, 0.0, 389.0, 0.0];
        for (n, spec) in WHEELS.iter().enumerate() {
            let cx = centers[n];
            c.label(
                names[n],
                773.0,
                Align2::CENTER_CENTER,
                spec.label,
                11.0,
                TEXT,
                false,
            );
            // Lift and gain have pickers that make a colour black or white.
            if n == 0 || n == 2 {
                let px = pickers[n];
                let picker = if n == 0 { Picker::Black } else { Picker::White };
                let on = self.color.picker == Some(picker);
                c.icon(
                    px,
                    773.0,
                    Icon::PlusDot,
                    if on { theme::WHITE } else { GREY },
                    0.5,
                );
                if ui
                    .interact(
                        c.rect(px - 10.0, 763.0, px + 10.0, 783.0),
                        ui.id().with(("color_point_picker", n)),
                        Sense::click(),
                    )
                    .on_hover_text(if n == 0 {
                        "Pick a colour in the viewer to make black"
                    } else {
                        "Pick a colour in the viewer to make white"
                    })
                    .clicked()
                {
                    self.color.picker = if on { None } else { Some(picker) };
                }
            }
            let rx = resets[n];
            c.icon(rx, 773.0, Icon::Reset, GREY, 0.5);
            if ui
                .interact(
                    c.rect(rx - 10.0, 763.0, rx + 10.0, 783.0),
                    ui.id().with(("color_wheel_reset", n)),
                    Sense::click(),
                )
                .on_hover_text(format!("Reset {}", spec.label))
                .clicked()
            {
                *wheel_channels(&mut grade, n) = [spec.neutral; 3];
            }

            let fit = {
                let (x0, y0) = c.at_points(cx - 89.0, 700.0);
                let (x1, y1) = c.at_points(cx + 89.0, 990.0);
                let (taller, wider) = c.s.grown(x0, y0, x1, y1);
                wheel_fit(taller, wider)
            };
            color_wheel(
                ui,
                ui.id().with(("color_wheel", n)),
                c.at(cx, 836.0),
                47.5 * R * fit,
                wheel_channels(&mut grade, n),
                spec.neutral,
                spec.range,
                // Gain's light sits at the top left, the others' at the bottom right.
                if n == 2 { 150.0 } else { -50.0 },
            );

            // The values: master and channels, or the three channels of offset.
            let rgb = *wheel_channels(&mut grade, n);
            let colors = [
                Color32::from_rgb(220, 220, 222),
                Color32::from_rgb(206, 52, 52),
                Color32::from_rgb(52, 170, 84),
                Color32::from_rgb(52, 96, 214),
            ];
            let boxes: Vec<(usize, f32, f32)> = if n == 3 {
                vec![(1, 555.0, 598.0), (2, 605.0, 648.0), (3, 655.0, 698.0)]
            } else {
                (0..4)
                    .map(|i| {
                        let mid = cx - 57.0 + i as f32 * 38.0;
                        (i, mid - 17.0, mid + 17.0)
                    })
                    .collect()
            };
            for (which, x0, x1) in boxes {
                let value = if which == 0 {
                    master(rgb)
                } else {
                    rgb[which - 1]
                };
                let text = format!("{:.2}", shown(n, value));
                if let Some(v) = value_box(
                    ui,
                    c,
                    ("color_value", n * 4 + which),
                    (x0, 898.0, x1, 914.0),
                    shown(n, value),
                    &text,
                    if n == 3 { 0.1 } else { 0.005 },
                    shown(n, spec.neutral),
                ) {
                    let v = from_shown(n, v);
                    let channels = wheel_channels(&mut grade, n);
                    if which == 0 {
                        set_master(channels, v);
                    } else {
                        channels[which - 1] = v;
                    }
                }
                underline_thin(c, x0 + 2.0, x1 - 2.0, 920.0, colors[which], colors[which]);
            }

            // The jog bar: drag it to move the master.
            let (j0, j1) = (cx - 73.0, cx + 74.0);
            let jog = c.rect(j0, 934.0, j1, 948.0);
            c.p.rect_filled(jog, 6.0, Color32::from_rgb(16, 16, 18));
            let ridges = 46;
            for i in 0..ridges {
                let x = j0 + 6.0 + (j1 - j0 - 12.0) * i as f32 / (ridges - 1) as f32;
                c.line((x, 937.0), (x, 945.0), 1.0, Color32::from_rgb(72, 72, 78));
            }
            let r = ui
                .interact(jog, ui.id().with(("color_jog", n)), Sense::click_and_drag())
                .on_hover_cursor(egui::CursorIcon::ResizeHorizontal)
                .on_hover_text(format!(
                    "Drag to change the {} master, double-click to reset",
                    spec.label.to_lowercase()
                ));
            let channels = wheel_channels(&mut grade, n);
            if r.double_clicked() {
                set_master(channels, spec.neutral);
            } else if r.dragged() {
                let m = master(*channels) + r.drag_delta().x * spec.jog;
                set_master(channels, m);
            }
        }

        // The extra values under the wheels.
        let bottom: [(&str, f32, f32, f32); 6] = [
            ("Col Boost", 70.0, 80.0, 112.0),
            ("Shad", 184.0, 192.0, 230.0),
            ("Hi/Light", 300.0, 307.0, 345.0),
            ("Sat", 416.0, 421.0, 462.0),
            ("Hue", 531.0, 535.0, 577.0),
            ("L. Mix", 646.0, 649.0, 695.0),
        ];
        for (n, (label, lx, x0, x1)) in bottom.into_iter().enumerate() {
            c.label(lx, 978.0, Align2::RIGHT_CENTER, label, 10.5, LABEL, false);
            let (value, neutral) = match n {
                0 => (grade.color_boost, 0.0),
                1 => (grade.shadows, 0.0),
                2 => (grade.highlights, 0.0),
                3 => (grade.saturation * 50.0, 50.0),
                4 => (grade.hue, 50.0),
                _ => (grade.luma_mix, 100.0),
            };
            if let Some(v) = value_box(
                ui,
                c,
                ("color_bottom", n),
                (x0, 969.0, x1, 987.0),
                value,
                &format!("{value:.2}"),
                0.25,
                neutral,
            ) {
                match n {
                    0 => grade.color_boost = v,
                    1 => grade.shadows = v,
                    2 => grade.highlights = v,
                    3 => grade.saturation = (v / 50.0).clamp(0.0, 2.0),
                    4 => grade.hue = v.clamp(0.0, 100.0),
                    _ => grade.luma_mix = v.clamp(0.0, 100.0),
                }
            }
            let (l, r) = match n {
                0 | 3 => (
                    Color32::from_rgb(70, 70, 76),
                    Color32::from_rgb(200, 60, 160),
                ),
                4 => (
                    Color32::from_rgb(220, 60, 60),
                    Color32::from_rgb(60, 110, 220),
                ),
                _ => (
                    Color32::from_rgb(70, 70, 76),
                    Color32::from_rgb(220, 220, 222),
                ),
            };
            underline_thin(c, x0 + 4.0, x1 - 4.0, 989.0, l, r);
        }

        let grade = grade.sanitized();
        if let (Some(id), true) = (id, grade != before) {
            self.run_gesture(
                ctx,
                Command::SetGrade {
                    clip: id,
                    grade,
                    node: self.node_for(id),
                },
            );
        }
    }

    /// Sets the gains so that the frame's average colour becomes neutral.
    fn auto_balance(&self, grade: &mut ColorGrade) {
        let Some(frame) = self.viewer.frame.as_ref() else {
            return;
        };
        let mut sum = [0u64; 3];
        let pixels = (frame.rgba.len() / 4).max(1) as u64;
        for px in frame.rgba.chunks_exact(4) {
            for (total, v) in sum.iter_mut().zip(px) {
                *total += u64::from(*v);
            }
        }
        let mean = sum.map(|t| (t as f32 / pixels as f32).max(1.0));
        let grey = (mean[0] + mean[1] + mean[2]) / 3.0;
        for (gain, m) in grade.gain.iter_mut().zip(mean) {
            *gain = (*gain * grey / m).clamp(0.25, 4.0);
        }
    }

    /// The Curves palette beside the wheels: the custom curve over a live histogram of the
    /// picture, then the Edit intensities and the Soft Clip settings.
    fn color_curves(&mut self, ui: &mut Ui, c: &Cv) {
        let c = &c.fresh();
        // The curves of the node being graded: the panel edits a copy of them only (not the
        // whole grade, which can hold a large table) and hands them back when they changed.
        let id = self.current_clip();
        let before = id
            .and_then(|id| self.project().clip(id))
            .and_then(|(_, clip)| clip.node(self.node_for(clip.id)))
            .map(|g| g.curves.clone())
            .unwrap_or_default();
        let mut curves = before.clone();
        c.band(720.0, 693.0, 1439.0, 717.0, BAR);
        // The mode being edited: 0 the custom curves, 1..=`HUE_MODES` a hue/sat/lum curve.
        let mode = self.color.curve_mode.min(HUE_MODES);
        // (`then_some` would subtract first, and mode 0 has nothing to subtract from.)
        let hue_mode = mode.checked_sub(1);
        let title = match hue_mode {
            None => "Curves - Custom".to_string(),
            Some(m) => format!("Curves - {}", HUE_MODE_NAMES[m]),
        };
        c.label(
            731.0,
            705.0,
            Align2::LEFT_CENTER,
            &title,
            11.5,
            theme::WHITE,
            true,
        );
        for (i, (x, icon, name)) in CURVE_MODES.iter().enumerate() {
            let selected = i == mode;
            c.icon(
                *x,
                705.0,
                *icon,
                if selected { theme::WHITE } else { GREY },
                if selected { 0.55 } else { 0.45 },
            );
            if ui
                .interact(
                    c.rect(x - 11.0, 695.0, x + 11.0, 715.0),
                    ui.id().with(("curve_mode", i)),
                    Sense::click(),
                )
                .on_hover_text(*name)
                .clicked()
            {
                self.color.curve_mode = i;
                self.color.curve_drag = None;
            }
        }
        c.icon(1347.0, 705.0, Icon::Expand, GREY, 0.45);
        c.icon(1384.0, 705.0, Icon::Reset, GREY, 0.5);
        if ui
            .interact(
                c.rect(1374.0, 695.0, 1394.0, 715.0),
                ui.id().with("curves_reset_all"),
                Sense::click(),
            )
            .on_hover_text("Reset all curves and the soft clip")
            .clicked()
        {
            curves = Curves::default();
        }
        c.dots(1421.0, 705.0, GREY);
        c.vline(1197.5, 718.0, 1004.0);
        c.vline(1439.0, 693.0, 1004.0);

        // The graph: a grid, the histogram of the picture on screen (it follows playback), and
        // the curve, which is straight until it is edited.
        let graph = c.rect(740.0, 737.0, 1177.0, 977.0);
        c.p.rect_filled(graph, 0.0, Color32::from_rgb(38, 38, 43));
        let grid = Stroke::new(1.0, Color32::from_rgb(50, 50, 56));
        for i in 1..8 {
            let x = graph.left() + graph.width() * i as f32 / 8.0;
            c.p.line_segment([pos2(x, graph.top()), pos2(x, graph.bottom())], grid);
        }
        for i in 1..5 {
            let y = graph.top() + graph.height() * i as f32 / 5.0;
            c.p.line_segment([pos2(graph.left(), y), pos2(graph.right(), y)], grid);
        }
        if self.current_clip().is_some() {
            self.color
                .measure_curve(ui.ctx(), self.viewer.frame.clone(), self.viewer.generation, mode);
            match (hue_mode, &self.color.curve_input, &self.color.curve_histogram) {
                (Some(m), Some(bins), _) => {
                    paint_curve_input_histogram(c.p, graph, bins, hue_curve_color(m));
                }
                (None, _, Some(h)) => paint_curve_histogram(c.p, graph, h),
                _ => {}
            }
        }
        c.p.rect_filled(
            Rect::from_center_size(c.at(739.0, 743.0), vec2(8.0 * R, 12.0 * R)),
            2.0,
            Color32::from_rgb(236, 236, 238),
        );
        // The curves: the edited one bright with its points, the other bent ones faint in
        // their colours. A press on a point drags it, a press elsewhere adds a point there and
        // drags it, a click adds one, and a right click on a point removes it.
        let area = Rect::from_min_max(c.at(746.0, 743.0), c.at(1171.0, 972.0));
        let to_screen = |x: f32, y: f32| {
            pos2(
                area.left() + area.width() * x,
                area.bottom() - area.height() * y,
            )
        };
        let to_curve = |p: egui::Pos2| {
            (
                ((p.x - area.left()) / area.width().max(1.0)).clamp(0.0, 1.0),
                ((area.bottom() - p.y) / area.height().max(1.0)).clamp(0.0, 1.0),
            )
        };
        let colors = [
            Color32::from_rgb(238, 232, 226),
            Color32::from_rgb(232, 70, 70),
            Color32::from_rgb(70, 200, 90),
            Color32::from_rgb(80, 130, 245),
        ];
        let targets: Vec<usize> = (0..4).filter(|&i| self.color.curve_channels[i]).collect();
        let main = targets.first().copied();
        let together = self.color.curve_link;

        let response = ui
            .interact(
                area.expand(6.0),
                ui.id().with("curve_graph"),
                Sense::click_and_drag(),
            )
            .on_hover_text(
                "Click to add a point, drag a point to move it, right-click a point to remove it",
            );
        let editing = id.is_some() && (hue_mode.is_some() || main.is_some());
        if editing {
            let aspect = area.width() / area.height().max(1.0);
            let radius = 9.0 / area.height().max(1.0);
            let k = main.unwrap_or(0);
            let near = |curves: &Curves, p: egui::Pos2| -> Option<usize> {
                let (x, y) = to_curve(p);
                match hue_mode {
                    Some(m) => curves.hue_point_near(m, x, y, radius, aspect),
                    None => curves.point_near(k, x, y, radius, aspect),
                }
            };
            let push = |curves: &mut Curves, x: f32, y: f32| -> Option<usize> {
                match hue_mode {
                    Some(m) => curves.add_hue_point(m, x, y),
                    None => curves.add_point(k, x, y),
                }
            };
            let drag = |curves: &mut Curves, i: usize, x: f32, y: f32| {
                match hue_mode {
                    Some(m) => curves.move_hue_point(m, i, x, y),
                    None => curves.move_point(k, i, x, y),
                }
            };
            let pop = |curves: &mut Curves, i: usize| {
                match hue_mode {
                    Some(m) => curves.remove_hue_point(m, i),
                    None => curves.remove_point(k, i),
                }
            };
            // Curves edited together take the edited curve's points.
            let sync = |curves: &mut Curves| {
                if hue_mode.is_none() && together {
                    for &t in &targets {
                        if t != k {
                            curves.points[t] = curves.points[k].clone();
                        }
                    }
                }
            };
            if response.drag_started() {
                if let Some(p) = response.interact_pointer_pos() {
                    let (x, y) = to_curve(p);
                    self.color.curve_drag = near(&curves, p).or_else(|| push(&mut curves, x, y));
                    sync(&mut curves);
                }
            }
            if response.dragged() {
                if let (Some(i), Some(p)) = (self.color.curve_drag, response.interact_pointer_pos()) {
                    let (x, y) = to_curve(p);
                    drag(&mut curves, i, x, y);
                    sync(&mut curves);
                }
            }
            if response.drag_stopped() {
                self.color.curve_drag = None;
            }
            if response.clicked() {
                if let Some(p) = response.interact_pointer_pos() {
                    if near(&curves, p).is_none() {
                        let (x, y) = to_curve(p);
                        push(&mut curves, x, y);
                        sync(&mut curves);
                    }
                }
            }
            if response.secondary_clicked() {
                if let Some(i) = response.interact_pointer_pos().and_then(|p| near(&curves, p)) {
                    pop(&mut curves, i);
                    sync(&mut curves);
                }
            }
            if let Some(p) = response.hover_pos() {
                if near(&curves, p).is_some() || self.color.curve_drag.is_some() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                }
            }
        }
        let painter = c.p.with_clip_rect(area.expand(6.0));
        let line = |curve: usize| -> Vec<egui::Pos2> {
            (0..=96)
                .map(|i| {
                    let x = i as f32 / 96.0;
                    to_screen(x, curves.eval(curve, x))
                })
                .collect()
        };
        // The custom page shows the other bent channels faintly; the hue/sat/lum graphs show
        // only the one being edited.
        if hue_mode.is_none() {
            for (k, color) in colors.iter().enumerate() {
                if Some(k) == main || curves.is_straight(k) {
                    continue;
                }
                painter.add(egui::Shape::line(
                    line(k),
                    Stroke::new(1.0, color.gamma_multiply(0.6)),
                ));
            }
        }
        let bright = match hue_mode {
            None => {
                let k = main.unwrap_or(0);
                if main.is_some() {
                    colors[k]
                } else {
                    colors[0].gamma_multiply(0.5)
                }
            }
            Some(m) => hue_curve_color(m),
        };
        let bright_line: Vec<Pos2> = match hue_mode {
            None => line(main.unwrap_or(0)),
            Some(m) => (0..=96)
                .map(|i| {
                    let x = i as f32 / 96.0;
                    to_screen(x, curves.eval_hue(m, x))
                })
                .collect(),
        };
        painter.add(egui::Shape::line(bright_line, Stroke::new(1.5, bright)));
        match hue_mode {
            None => {
                if main.is_some() {
                    let k = main.unwrap_or(0);
                    for (i, p) in curves.points[k].iter().enumerate() {
                        let at = to_screen(p[0], p[1]);
                        let held = self.color.curve_drag == Some(i);
                        painter.circle_filled(at, if held { 5.0 } else { 4.0 } * R, bright);
                        painter.circle_stroke(
                            at,
                            4.0 * R,
                            Stroke::new(1.0, Color32::from_rgb(30, 30, 34)),
                        );
                    }
                }
            }
            Some(m) => {
                for (i, p) in curves.hue_points[m].iter().enumerate() {
                    let at = to_screen(p[0], p[1]);
                    let held = self.color.curve_drag == Some(i);
                    painter.circle_filled(at, if held { 5.0 } else { 4.0 } * R, bright);
                    painter.circle_stroke(
                        at,
                        4.0 * R,
                        Stroke::new(1.0, Color32::from_rgb(30, 30, 34)),
                    );
                }
            }
        }
        if hue_mode.is_none() {
        // Edit: which channels a drag on the curve changes, and how strongly each curve acts.
        c.label(
            1208.0,
            738.0,
            Align2::LEFT_CENTER,
            "Edit",
            10.5,
            TEXT,
            false,
        );
        paint_link(c, 1275.0, 738.0, self.color.curve_link);
        if ui
            .interact(
                c.rect(1266.0, 729.0, 1284.0, 747.0),
                ui.id().with("curve_link"),
                Sense::click(),
            )
            .on_hover_text("Edit the channels together")
            .clicked()
        {
            self.color.curve_link ^= true;
        }
        let channels = [
            (
                "Y",
                Color32::from_rgb(214, 214, 216),
                Color32::from_rgb(30, 30, 32),
            ),
            ("R", Color32::from_rgb(226, 40, 40), theme::WHITE),
            ("G", Color32::from_rgb(40, 172, 64), theme::WHITE),
            ("B", Color32::from_rgb(46, 92, 230), theme::WHITE),
        ];
        for (i, (name, fill, text)) in channels.iter().enumerate() {
            let x = 1298.0 + 26.7 * i as f32;
            let rect = c.rect(x - 10.0, 729.0, x + 10.0, 747.0);
            let on = self.color.curve_channels[i];
            c.p.rect_filled(
                rect,
                2.0,
                if on { *fill } else { fill.gamma_multiply(0.35) },
            );
            crate::i18n::paint_text(&c.p,
                rect.center(),
                Align2::CENTER_CENTER,
                *name,
                theme::semibold(9.5 * R),
                *text,
            );
            if ui
                .interact(rect, ui.id().with(("curve_channel", i)), Sense::click())
                .clicked()
            {
                self.color.curve_channels[i] ^= true;
            }
        }
        c.icon(1421.0, 738.0, Icon::Reset, GREY, 0.5);
        if ui
            .interact(
                c.rect(1411.0, 728.0, 1431.0, 748.0),
                ui.id().with("curve_reset"),
                Sense::click(),
            )
            .on_hover_text("Reset the chosen curves")
            .clicked()
        {
            let straight = Curves::default();
            for &t in &targets {
                curves.points[t] = straight.points[t].clone();
                curves.intensity[t] = 100.0;
            }
        }
        let knobs = [
            Color32::from_rgb(232, 232, 234),
            Color32::from_rgb(226, 40, 40),
            Color32::from_rgb(40, 172, 64),
            Color32::from_rgb(46, 92, 230),
        ];
        for (i, knob) in knobs.into_iter().enumerate() {
            let y = 770.0 + 25.0 * i as f32;
            let value = curves.intensity[i];
            c.line((1211.0, y), (1332.0, y), 1.0, Color32::from_rgb(70, 70, 76));
            let kx = 1211.0 + 115.0 * (value / 100.0).clamp(0.0, 1.0);
            c.p.circle_filled(c.at(kx, y), 3.6 * R, knob);
            if let Some(v) = value_box(
                ui,
                c,
                ("curve_intensity", i),
                (1342.0, y - 9.0, 1390.0, y + 9.0),
                value,
                &format!("{value:.0}"),
                0.5,
                100.0,
            ) {
                curves.intensity[i] = v.clamp(0.0, 100.0);
            }
            c.icon(1421.0, y, Icon::Reset, GREY, 0.5);
            if ui
                .interact(
                    c.rect(1411.0, y - 10.0, 1431.0, y + 10.0),
                    ui.id().with(("curve_intensity_reset", i)),
                    Sense::click(),
                )
                .clicked()
            {
                curves.intensity[i] = 100.0;
            }
        }

        // Soft Clip: how the curve rolls off at black and white, as four numbers.
        c.label(
            1207.0,
            888.0,
            Align2::LEFT_CENTER,
            "Soft Clip",
            10.5,
            TEXT,
            false,
        );
        paint_link(c, 1302.0, 888.0, true);
        for (i, (name, fill, _)) in channels.iter().skip(1).enumerate() {
            let x = 1325.0 + 27.0 * i as f32;
            let rect = c.rect(x - 10.0, 879.0, x + 10.0, 897.0);
            c.p.rect_filled(rect, 2.0, *fill);
            crate::i18n::paint_text(&c.p,
                rect.center(),
                Align2::CENTER_CENTER,
                *name,
                theme::semibold(9.5 * R),
                theme::WHITE,
            );
        }
        c.icon(1421.0, 888.0, Icon::Reset, GREY, 0.5);
        if ui
            .interact(
                c.rect(1411.0, 878.0, 1431.0, 898.0),
                ui.id().with("soft_clip_reset"),
                Sense::click(),
            )
            .clicked()
        {
            curves.soft_clip = SOFT_CLIP_NONE;
        }
        // Low, High, then L.S. and H.S. (low soft and high soft), as numbers from 0 to 100.
        for (i, (name, label_right, x0, x1, y)) in [
            ("Low", 1243.0, 1252.0, 1297.0, 931.0),
            ("High", 1352.0, 1360.0, 1405.0, 931.0),
            ("L.S.", 1243.0, 1252.0, 1297.0, 957.0),
            ("H.S.", 1352.0, 1360.0, 1405.0, 957.0),
        ]
        .into_iter()
        .enumerate()
        {
            // The array is low, low soft, high, high soft; the screen is low, high, L.S., H.S.
            let slot = [0, 2, 1, 3][i];
            c.label(
                label_right,
                y,
                Align2::RIGHT_CENTER,
                name,
                10.5,
                LABEL,
                false,
            );
            let value = curves.soft_clip[slot] * 100.0;
            if let Some(v) = value_box(
                ui,
                c,
                ("soft_clip", i),
                (x0, y - 9.0, x1, y + 9.0),
                value,
                &format!("{value:.1}"),
                0.5,
                SOFT_CLIP_NONE[slot] * 100.0,
            ) {
                curves.soft_clip[slot] = (v / 100.0).clamp(0.0, 1.0);
            }
        }
        }
        else {
            // Hue/sat/lum curves get one intensity slider for the curve being edited.
            let m = hue_mode.unwrap_or(0);
            c.label(
                1208.0,
                770.0,
                Align2::LEFT_CENTER,
                HUE_MODE_NAMES[m],
                10.5,
                TEXT,
                false,
            );
            let value = curves.hue_intensity[m];
            c.line((1211.0, 770.0), (1332.0, 770.0), 1.0, Color32::from_rgb(70, 70, 76));
            let kx = 1211.0 + 115.0 * (value / 100.0).clamp(0.0, 1.0);
            c.p.circle_filled(c.at(kx, 770.0), 3.6 * R, hue_curve_color(m));
            if let Some(v) = value_box(
                ui,
                c,
                ("hue_intensity", m),
                (1342.0, 761.0, 1390.0, 779.0),
                value,
                &format!("{value:.0}"),
                0.5,
                100.0,
            ) {
                curves.hue_intensity[m] = v.clamp(0.0, 100.0);
            }
            c.icon(1421.0, 770.0, Icon::Reset, GREY, 0.5);
            if ui
                .interact(
                    c.rect(1411.0, 760.0, 1431.0, 780.0),
                    ui.id().with(("hue_intensity_reset", m)),
                    Sense::click(),
                )
                .clicked()
            {
                let straight = Curves::default();
                curves.hue_points[m] = straight.hue_points[m].clone();
                curves.hue_intensity[m] = 100.0;
            }
        }

        if let (Some(id), true) = (id, curves != before) {
            let node = self.node_for(id);
            if let Some(mut grade) = self
                .project()
                .clip(id)
                .and_then(|(_, clip)| clip.node(node))
                .cloned()
            {
                grade.curves = curves.sanitized();
                self.run_gesture(
                    ui.ctx(),
                    Command::SetGrade {
                        clip: id,
                        grade,
                        node,
                    },
                );
            }
        }
    }

    fn color_scopes(&mut self, ui: &mut Ui, ctx: &Context, c: &Cv) {
        // Laid out on Resolve at 1080p, from 1440 to the right edge.
        let c = &c.fresh();
        c.band(1440.0, 693.0, 1919.0, 717.0, BAR);
        c.label(
            1450.0,
            705.0,
            Align2::LEFT_CENTER,
            "Scopes",
            11.5,
            theme::WHITE,
            true,
        );
        let kind = self.color.kind;
        c.label(
            1799.0,
            705.0,
            Align2::RIGHT_CENTER,
            kind.label(),
            11.0,
            theme::WHITE,
            true,
        );
        c.chevron(1809.0, 705.0, theme::WHITE);
        c.icon(1827.0, 705.0, Icon::Sliders, GREY, 0.45);
        c.icon(1864.0, 705.0, Icon::Expand, GREY, 0.45);
        c.dots(1901.0, 705.0, GREY);
        if ui
            .interact(
                c.rect(1735.0, 695.0, 1815.0, 715.0),
                ui.id().with("color_scope_kind"),
                Sense::click(),
            )
            .on_hover_text("Click for the next scope")
            .clicked()
        {
            self.color.kind = kind.next();
        }
        if ui
            .interact(
                c.rect(1815.0, 695.0, 1840.0, 715.0),
                ui.id().with("color_scope_settings"),
                Sense::click(),
            )
            .clicked()
        {
            self.planned("Scope settings");
        }

        let area = c.rect(1440.5, 718.0, 1919.0, 1004.0);
        c.p.rect_filled(area, 0.0, Color32::BLACK);
        let olive = Color32::from_rgb(64, 58, 28);
        let plot = c.rect(1490.0, 725.0, 1916.0, 975.0);
        if !self.scopes_visible {
            return;
        }
        if matches!(kind, ScopeKind::Parade | ScopeKind::Waveform) {
            let levels = [1023, 896, 768, 640, 512, 384, 256, 128, 0];
            for (i, level) in levels.into_iter().enumerate() {
                let y = 725.0 + 31.25 * i as f32;
                c.label(
                    1479.0,
                    y,
                    Align2::RIGHT_CENTER,
                    &level.to_string(),
                    10.5,
                    Color32::from_rgb(214, 196, 120),
                    false,
                );
                c.line((1490.0, y), (1916.0, y), 1.0, Color32::from_rgb(34, 32, 22));
            }
            for i in 0..=40 {
                let y = 725.0 + 250.0 * i as f32 / 40.0;
                c.line((1485.0, y), (1490.0, y), 1.0, olive);
            }
        } else if kind == ScopeKind::Vectorscope {
            let center = plot.center();
            let r = plot.height().min(plot.width()) / 2.0;
            for f in [0.25, 0.5, 0.75, 1.0] {
                c.p.circle_stroke(center, r * f, Stroke::new(1.0, olive));
            }
        }

        let generation = self.viewer.generation;
        self.color
            .measure(ctx, self.viewer.frame.clone(), generation);
        if self.current_clip().is_none() {
            return;
        }
        match kind {
            ScopeKind::Parade | ScopeKind::Waveform => {
                if let Some(t) = &self.color.texture {
                    c.p.image(
                        t.id(),
                        plot,
                        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                }
            }
            ScopeKind::Vectorscope => {
                if let Some(t) = &self.color.texture {
                    let side = plot.height().min(plot.width());
                    c.p.image(
                        t.id(),
                        Rect::from_center_size(plot.center(), vec2(side, side)),
                        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                }
            }
            ScopeKind::Histogram => {
                if let Some(h) = &self.color.histogram {
                    let max = h.max().max(1) as f32;
                    for (bins, color) in [
                        (&h.r, Color32::from_rgba_unmultiplied(235, 80, 80, 150)),
                        (&h.g, Color32::from_rgba_unmultiplied(80, 220, 110, 150)),
                        (&h.b, Color32::from_rgba_unmultiplied(90, 140, 245, 150)),
                        (&h.luma, Color32::from_rgba_unmultiplied(230, 230, 230, 110)),
                    ] {
                        let points: Vec<_> = bins
                            .iter()
                            .enumerate()
                            .map(|(i, v)| {
                                pos2(
                                    plot.left() + plot.width() * i as f32 / 255.0,
                                    plot.bottom() - plot.height() * (*v as f32 / max).sqrt(),
                                )
                            })
                            .collect();
                        c.p.add(egui::Shape::line(points, Stroke::new(1.5, color)));
                    }
                }
            }
        }
    }

    /// Reads a `.cube` file and puts it on the clip being graded, replacing any table it had.
    pub(crate) fn apply_lut(&mut self, name: &str, bytes: &[u8]) {
        let Some(id) = self.target_clip() else {
            self.fail(format!("{name}: select a clip to put the LUT on"));
            return;
        };
        let title = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
        let lut = match Lut::parse_cube(&String::from_utf8_lossy(bytes), title) {
            Ok(lut) => lut,
            Err(e) => {
                self.fail(format!("{name}: {e}"));
                return;
            }
        };
        let Some((_, clip)) = self.project().clip(id) else {
            return;
        };
        let mut grade = clip
            .node(self.node_for(clip.id))
            .cloned()
            .unwrap_or_default();
        let shown = lut.title.clone();
        grade.lut = Some(lut);
        self.run(Command::SetGrade {
            clip: id,
            grade,
            node: self.node_for(id),
        });
        self.info(format!("Applied the LUT {shown}"));
    }
}

/// The mini timeline's ruler ticks, `(x, frame)`, without the ones that repeat the time before
/// them: an empty timeline, or one shorter than the strip, would otherwise show the same time
/// all the way across.
fn ruler_ticks(ticks: impl IntoIterator<Item = (f32, u64)>) -> Vec<(f32, u64)> {
    let mut out: Vec<(f32, u64)> = Vec::new();
    for tick in ticks {
        if out.last().map(|&(_, frame)| frame) != Some(tick.1) {
            out.push(tick);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{ruler_ticks, wheel_fit};

    #[test]
    fn a_wheel_follows_the_smaller_stretch_of_its_panel() {
        assert_eq!(wheel_fit(1.0, 1.0), 1.0);
        // Taller but not wider: it cannot grow past the width.
        assert_eq!(wheel_fit(1.3, 1.0), 1.0);
        assert_eq!(wheel_fit(0.7, 1.2), 0.7);
        // Never vanishing, never huge; nonsense gives the usual size.
        assert_eq!(wheel_fit(0.1, 0.1), 0.4);
        assert_eq!(wheel_fit(3.0, 3.0), 1.4);
        assert_eq!(wheel_fit(f32::NAN, 1.0), 1.0);
    }

    /// The ticks the page draws for a timeline `duration` frames long: ten 200-point steps
    /// from 40 across the strip's 1873 points.
    fn ticks(duration: u64) -> Vec<(f32, u64)> {
        let span = 1873.0;
        ruler_ticks((0..10).map(|n| {
            let x = 40.0 + 200.0 * n as f32;
            (x, ((x - 40.0) / span * duration as f32) as u64)
        }))
    }

    #[test]
    fn a_timeline_long_enough_keeps_every_tick_where_it_was() {
        let ticks = ticks(24_000);
        assert_eq!(ticks.len(), 10);
        assert_eq!(ticks.first().map(|&(x, _)| x), Some(40.0));
        assert_eq!(ticks.last().map(|&(x, _)| x), Some(1840.0));
        let frames: Vec<u64> = ticks.iter().map(|&(_, frame)| frame).collect();
        assert!(frames.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn an_empty_timeline_has_one_tick_at_its_start() {
        // The page takes an empty project's duration as 1 frame, so the ruler is a point.
        assert_eq!(ticks(1), vec![(40.0, 0)]);
    }

    #[test]
    fn a_timeline_of_a_few_frames_does_not_repeat_a_time() {
        // Three frames over ten ticks: the times only move on three times.
        let ticks = ticks(3);
        assert_eq!(ticks.len(), 3);
        let frames: Vec<u64> = ticks.iter().map(|&(_, frame)| frame).collect();
        assert!(frames.windows(2).all(|w| w[0] < w[1]));
    }
}
