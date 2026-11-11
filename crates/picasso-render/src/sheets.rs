//! Contact sheets of the newer effects, transitions and generators, drawn by the same code the
//! program uses. Not a check: `SHEET_DIR=/some/folder cargo test -p picasso-render --release
//! sheets -- --ignored --nocapture` writes three PNG files into the folder.

use std::path::PathBuf;

use picasso_color::{Effect, EffectKind};
use picasso_core::{
    GeneratorKind, MediaId, MediaItem, MediaSource, Project, TitleText, TransitionKind,
};

use crate::cache::MediaCache;
use crate::compose::render_thumbnail;

const TW: usize = 480;
const TH: usize = 270;

/// A made-up scene with sky, sun, hills, saturated boxes, a face and fine texture, so that the
/// effects have something to show.
fn scene(palette: usize) -> Vec<u8> {
    let mut v = vec![0u8; TW * TH * 4];
    let (sky_top, sky_low, ground) = match palette {
        0 => ([40u8, 90, 190], [250u8, 170, 90], [30u8, 70, 50]),
        _ => ([120u8, 30, 150], [90u8, 220, 200], [70u8, 40, 30]),
    };
    for y in 0..TH {
        for x in 0..TW {
            let t = y as f32 / (TH as f32 * 0.65);
            let horizon = (TH as f32 * 0.62)
                + ((x as f32 * 0.03).sin() * 12.0)
                + ((x as f32 * 0.011).cos() * 16.0);
            let (r, g, b) = if (y as f32) < horizon {
                let t = t.min(1.0);
                (0..3).fold((0.0, 0.0, 0.0), |acc, i| {
                    let c = f32::from(sky_top[i]) * (1.0 - t) + f32::from(sky_low[i]) * t;
                    match i {
                        0 => (c, acc.1, acc.2),
                        1 => (acc.0, c, acc.2),
                        _ => (acc.0, acc.1, c),
                    }
                })
            } else {
                // Ground with fine texture.
                let n = (((x * 13) ^ (y * 29)) % 37) as f32;
                (
                    f32::from(ground[0]) + n,
                    f32::from(ground[1]) + n * 1.2,
                    f32::from(ground[2]) + n * 0.8,
                )
            };
            let i = (y * TW + x) * 4;
            v[i..i + 4].copy_from_slice(&[
                r.min(255.0) as u8,
                g.min(255.0) as u8,
                b.min(255.0) as u8,
                255,
            ]);
        }
    }
    let disc = |v: &mut Vec<u8>, cx: f32, cy: f32, r: f32, c: [u8; 3]| {
        for y in 0..TH {
            for x in 0..TW {
                if ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt() < r {
                    let i = (y * TW + x) * 4;
                    v[i..i + 3].copy_from_slice(&c);
                }
            }
        }
    };
    let rect = |v: &mut Vec<u8>, x0: usize, y0: usize, w: usize, h: usize, c: [u8; 3]| {
        for y in y0..(y0 + h).min(TH) {
            for x in x0..(x0 + w).min(TW) {
                let i = (y * TW + x) * 4;
                v[i..i + 3].copy_from_slice(&c);
            }
        }
    };
    disc(&mut v, 380.0, 70.0, 22.0, [255, 250, 235]); // the sun
    rect(&mut v, 40, 170, 60, 50, [210, 40, 40]);
    rect(&mut v, 110, 160, 50, 60, [40, 190, 70]);
    rect(&mut v, 170, 180, 55, 40, [40, 80, 220]);
    disc(&mut v, 300.0, 190.0, 34.0, [205, 150, 120]); // a face
    disc(&mut v, 289.0, 183.0, 4.0, [30, 25, 25]);
    disc(&mut v, 311.0, 183.0, 4.0, [30, 25, 25]);
    // Two bright lamps.
    disc(&mut v, 90.0, 60.0, 6.0, [255, 255, 245]);
    disc(&mut v, 200.0, 40.0, 5.0, [255, 245, 220]);
    v
}

/// Writes the label over the bottom left of a tile.
fn labelled(mut tile: Vec<u8>, w: usize, h: usize, text: &str) -> Vec<u8> {
    let title = TitleText {
        text: text.into(),
        size: 62,
        x: 0,
        y: 940,
        bold: true,
        style: picasso_core::TextStyle {
            background: true,
            background_color: [0, 0, 0],
            background_opacity: 65,
            outline: 0,
            align: picasso_core::TextAlign::Left,
            ..picasso_core::TextStyle::default()
        },
        ..TitleText::default()
    };
    // The title's x is the middle of its text; put the middle where it fits.
    let mut t = title;
    t.x = (text.chars().count() as u32 * 18).clamp(120, 880);
    crate::text::draw_title(&mut tile, w, h, &t);
    tile
}

fn sheet(tiles: Vec<Vec<u8>>, cols: usize, w: usize, h: usize, name: &str) {
    let rows = tiles.len().div_ceil(cols);
    let gap = 6;
    let (sw, sh) = (cols * w + (cols + 1) * gap, rows * h + (rows + 1) * gap);
    let mut out = vec![0u8; sw * sh * 4];
    for px in out.chunks_exact_mut(4) {
        px.copy_from_slice(&[18, 18, 22, 255]);
    }
    for (n, tile) in tiles.iter().enumerate() {
        let (cx, cy) = (n % cols, n / cols);
        let (ox, oy) = (gap + cx * (w + gap), gap + cy * (h + gap));
        for y in 0..h {
            let src = &tile[y * w * 4..(y + 1) * w * 4];
            let dst = ((oy + y) * sw + ox) * 4;
            out[dst..dst + w * 4].copy_from_slice(src);
        }
    }
    let dir = PathBuf::from(std::env::var("SHEET_DIR").unwrap_or_else(|_| ".".into()));
    let path = dir.join(name);
    let img = image::RgbaImage::from_raw(sw as u32, sh as u32, out).expect("sheet size");
    img.save(&path).expect("write the sheet");
    eprintln!("wrote {}", path.display());
}

#[test]
#[ignore]
fn sheets() {
    let base = scene(0);
    let other = scene(1);

    // ---- Effects ----
    let kinds = [
        EffectKind::Bloom,
        EffectKind::LightRays,
        EffectKind::LensFlare,
        EffectKind::ApertureDiffraction,
        EffectKind::Watercolor,
        EffectKind::StylizeInk,
        EffectKind::Emboss,
        EffectKind::EdgeDetect,
        EffectKind::LensBarrel,
        EffectKind::Ripple,
        EffectKind::CornerPinLeft,
        EffectKind::ChromaticAberration,
        EffectKind::FilmDamage,
        EffectKind::AnalogDamage,
        EffectKind::FalseColorThermal,
    ];
    let mut tiles = vec![labelled(base.clone(), TW, TH, "Original")];
    for kind in kinds {
        let mut t = base.clone();
        Effect::new(kind).apply_at(&mut t, TW, TH, 7, 24.0);
        tiles.push(labelled(t, TW, TH, kind.label()));
    }
    sheet(tiles, 4, TW, TH, "new-effects.png");

    // ---- Transitions, half way ----
    let kinds = [
        TransitionKind::ArrowIris,
        TransitionKind::StarShape,
        TransitionKind::HeartShape,
        TransitionKind::EyeIris,
        TransitionKind::BarnDoorVertical,
        TransitionKind::SplitHorizontal,
        TransitionKind::ClockWipe,
        TransitionKind::SpiralWipe,
        TransitionKind::VenetianBlindWipe,
        TransitionKind::BoxUpperLeft,
        TransitionKind::SlideOutLeft,
        TransitionKind::Glitch,
        TransitionKind::BurnAway,
        TransitionKind::BlurDissolve,
        TransitionKind::AdditiveDissolve,
        TransitionKind::HexagonIrisVertical,
    ];
    let mut tiles = Vec::new();
    for kind in kinds {
        let mut second = other.clone();
        crate::transitions::mix_into(kind, &base, &mut second, TW, TH, 0.5);
        tiles.push(labelled(second, TW, TH, kind.label()));
    }
    sheet(tiles, 4, TW, TH, "new-transitions.png");

    // ---- Generators ----
    let mut project = Project::new("sheets");
    let cache = MediaCache::new();
    let mut tiles = Vec::new();
    for (n, kind) in GeneratorKind::ALL.into_iter().enumerate() {
        let item = MediaItem {
            id: MediaId(100 + n as u64),
            name: kind.label().into(),
            source: MediaSource::Generator {
                pattern: kind,
                colors: kind.default_colors(),
            },
            length: None,
            default_duration: 96,
            bin: None,
            timecode: None,
        };
        project.media.push(item.clone());
        let frame =
            render_thumbnail(&project, &item, &cache, TW as u32, TH as u32).expect("thumbnail");
        // Over a grey so the see-through ones show.
        let mut tile = vec![0u8; TW * TH * 4];
        for (o, s) in tile.chunks_exact_mut(4).zip(frame.rgba.chunks_exact(4)) {
            let a = u32::from(s[3]);
            for c in 0..3 {
                o[c] = ((u32::from(s[c]) * a + 90 * (255 - a)) / 255) as u8;
            }
            o[3] = 255;
        }
        tiles.push(labelled(tile, TW, TH, kind.label()));
    }
    sheet(tiles, 4, TW, TH, "new-generators.png");
}

/// A scene for the keys: a subject on a green screen.
fn green_screen_scene() -> Vec<u8> {
    let mut v = vec![0u8; TW * TH * 4];
    for px in v.chunks_exact_mut(4) {
        px.copy_from_slice(&[0, 177, 64, 255]);
    }
    let disc = |v: &mut Vec<u8>, cx: f32, cy: f32, r: f32, c: [u8; 3]| {
        for y in 0..TH {
            for x in 0..TW {
                if ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt() < r {
                    let i = (y * TW + x) * 4;
                    v[i..i + 3].copy_from_slice(&c);
                }
            }
        }
    };
    disc(&mut v, 240.0, 150.0, 75.0, [205, 150, 120]); // a face, with a green cast at the edge
    disc(&mut v, 215.0, 135.0, 9.0, [30, 25, 25]);
    disc(&mut v, 265.0, 135.0, 9.0, [30, 25, 25]);
    disc(&mut v, 240.0, 80.0, 40.0, [200, 40, 40]); // a red hat
    for y in 200..TH {
        for x in 150..330 {
            let i = (y * TW + x) * 4;
            v[i..i + 3].copy_from_slice(&[40, 80, 200]);
        }
    }
    v
}

/// The picture over a chequerboard, so what a key made see-through shows.
fn over_checks(rgba: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; rgba.len()];
    for (n, (o, s)) in out
        .chunks_exact_mut(4)
        .zip(rgba.chunks_exact(4))
        .enumerate()
    {
        let (x, y) = (n % TW, n / TW);
        let back = if (x / 16 + y / 16) % 2 == 0 {
            70u32
        } else {
            110
        };
        let a = u32::from(s[3]);
        for c in 0..3 {
            o[c] = ((u32::from(s[c]) * a + back * (255 - a)) / 255) as u8;
        }
        o[3] = 255;
    }
    out
}

fn effect_tiles(base: &[u8], kinds: &[EffectKind]) -> Vec<Vec<u8>> {
    let mut tiles = vec![labelled(base.to_vec(), TW, TH, "Original")];
    for kind in kinds {
        let mut t = base.to_vec();
        Effect::new(*kind).apply_at(&mut t, TW, TH, 7, 24.0);
        tiles.push(labelled(t, TW, TH, kind.label()));
    }
    tiles
}

#[test]
#[ignore]
fn sections() {
    use EffectKind as K;
    let base = scene(0);

    let section = |name: &str, kinds: &[EffectKind]| {
        sheet(
            effect_tiles(&base, kinds),
            4,
            TW,
            TH,
            &format!("section-{name}.png"),
        );
    };

    // Key: every key on the subject over a green screen, then over a chequerboard.
    let screen = green_screen_scene();
    let mut tiles = vec![labelled(screen.clone(), TW, TH, "Original (green screen)")];
    for kind in [
        K::GreenScreen,
        K::BlueScreen,
        K::RedScreen,
        K::HslKeyGreen,
        K::HslKeyBlue,
        K::Keyer3dGreen,
        K::Keyer3dGreenAndBlue,
        K::LumaKey,
    ] {
        let mut t = screen.clone();
        Effect::new(kind).apply_at(&mut t, TW, TH, 7, 24.0);
        tiles.push(labelled(over_checks(&t), TW, TH, kind.label()));
    }
    for kind in [K::MatteShrink, K::MatteGrow, K::MatteSoften] {
        let mut t = screen.clone();
        Effect::new(K::GreenScreen).apply_at(&mut t, TW, TH, 7, 24.0);
        Effect::new(kind).apply_at(&mut t, TW, TH, 7, 24.0);
        tiles.push(labelled(
            over_checks(&t),
            TW,
            TH,
            &format!("Green Screen + {}", kind.label()),
        ));
    }
    sheet(tiles, 4, TW, TH, "section-key.png");

    section(
        "restore",
        &[
            K::ContrastPop,
            K::FalseColorExposure,
            K::FalseColorThermal,
            K::FalseColorRainbow,
            K::Dehaze,
            K::Deband,
            K::Beauty,
            K::Defringe,
            K::DespillGreen,
            K::GamutLimiterRec709,
            K::DeadPixelFixer,
        ],
    );
    section(
        "warp",
        &[
            K::LensBarrel,
            K::LensPincushion,
            K::Ripple,
            K::Wave,
            K::VerticalWave,
            K::HorizontalBow,
            K::VerticalBow,
            K::SquareBulge,
            K::SquarePinch,
            K::CornerPinLeft,
            K::CornerPinRight,
            K::CornerPinTop,
        ],
    );
    section(
        "light",
        &[
            K::Glow,
            K::Diffusion,
            K::LightRays,
            K::Bloom,
            K::Halation,
            K::LensFlare,
            K::ApertureDiffraction,
            K::LensReflections,
        ],
    );
    section(
        "texture",
        &[
            K::FastNoise,
            K::TexturePop,
            K::DetailRecovery,
            K::AnalogDamage,
            K::FilmDamage,
            K::JpegDamage,
        ],
    );
    section(
        "transform",
        &[
            K::FeatheredCrop,
            K::FeatheredCropLeft,
            K::FeatheredCropTop,
            K::Collage2x2,
            K::Collage3x3,
            K::Collage4x4,
        ],
    );
    section(
        "sharpen",
        &[
            K::Sharpen,
            K::UnsharpMask,
            K::SharpenEdges,
            K::SoftenAndSharpen,
        ],
    );
    section(
        "stylize",
        &[
            K::Vignette,
            K::Scanlines,
            K::Halftone,
            K::PencilSketch,
            K::Abstraction,
            K::Prism,
            K::ColoredBorder,
            K::ColoredBorderRounded,
            K::ChromaticAberration,
            K::EdgeDetect,
            K::Emboss,
            K::Stroke,
            K::Watercolor,
            K::StylizeRelief,
            K::StylizeInk,
            K::StylizeCutout,
        ],
    );
    section(
        "blur",
        &[
            K::GaussianBlur,
            K::BoxBlur,
            K::RadialBlur,
            K::ZoomBlur,
            K::TiltShift,
            K::LensBlur,
            K::NoiseReduction,
        ],
    );
    section(
        "color",
        &[
            K::Exposure,
            K::Posterize,
            K::Vibrance,
            K::Duotone,
            K::Sepia,
            K::BlackAndWhite,
            K::ColorTemperature,
        ],
    );
    section(
        "distort",
        &[
            K::Twirl,
            K::Bulge,
            K::Pinch,
            K::Kaleidoscope,
            K::MirrorSplit,
            K::Tile,
            K::CameraShake,
        ],
    );

    // Time: a ball moving across the scene; every effect on the fifth frame of the clip.
    let ball = |i: i64| {
        let mut v = scene(0);
        let cx = 80.0 + i as f32 * 38.0;
        for y in 0..TH {
            for x in 0..TW {
                if ((x as f32 - cx).powi(2) + (y as f32 - 120.0).powi(2)).sqrt() < 30.0 {
                    let n = (y * TW + x) * 4;
                    v[n..n + 3].copy_from_slice(&[250, 220, 40]);
                }
            }
        }
        v
    };
    let current = ball(0);
    let mut tiles = vec![labelled(
        current.clone(),
        TW,
        TH,
        "Original (a ball moving right)",
    )];
    for kind in [
        K::Echo,
        K::Strobe,
        K::MotionTrails,
        K::Smear,
        K::MotionBlur,
        K::Deflicker,
        K::ColorStabilizer,
        K::FrameReplacerMix,
        K::DirtRemoval,
    ] {
        let a = kind.default_amount() / 100.0;
        let offsets = crate::temporal::offsets(kind, a, 5);
        let frames: Vec<Vec<u8>> = offsets.iter().map(|o| ball(*o)).collect();
        let mut t = current.clone();
        crate::temporal::apply(kind, a, &mut t, (TW, TH), 5, &frames);
        tiles.push(labelled(t, TW, TH, kind.label()));
    }
    sheet(tiles, 4, TW, TH, "section-time.png");
}
