//! The Render Settings presets of the Deliver page, as DaVinci Resolve lists them: each sets the
//! format, codec, size and sound of the render. Some have a submenu of resolutions. Final Cut
//! Pro 7, Premiere XML and AVID AAF write the timeline for another editor instead of rendering;
//! Pro Tools renders the timeline's audio as baked .wav files and writes an AAF that references
//! them.

use egui::{pos2, vec2, Align2, Color32, Painter, Pos2, Rect, Sense, Stroke, StrokeKind, Ui};
use picasso_export::{AudioCodec, Options, Preset, Profile, Quality, Rate};

use crate::app::StudioApp;
use crate::icons::{paint, Icon};
use crate::theme::regular;

/// How a preset's icon looks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Badge {
    Logo,
    /// A film frame with a play sign and a codec label in this colour.
    Film(&'static str, Color32),
    YouTube,
    Vimeo,
    TikTok,
    Presentations,
    Dropbox,
    FinalCut,
    Premiere,
    Audio,
    Avid,
    ProTools,
}

/// One entry of the list.
pub(crate) struct Entry {
    pub name: &'static str,
    pub badge: Badge,
    /// The resolutions of its submenu (vertical ones for TikTok), empty when it has none.
    pub sizes: &'static [&'static str],
    /// Picasso Suite can render it.
    pub available: bool,
}

const WEB: &[&str] = &["720p", "1080p", "2160p"];

pub(crate) const ENTRIES: [Entry; 16] = [
    Entry {
        name: "Custom Export",
        badge: Badge::Logo,
        sizes: &[],
        available: true,
    },
    Entry {
        name: "H.264 Master",
        badge: Badge::Film("H.264", Color32::from_rgb(42, 96, 140)),
        sizes: &[],
        available: true,
    },
    Entry {
        name: "HyperDeck",
        badge: Badge::Film("H.264", Color32::from_rgb(42, 96, 140)),
        sizes: &[],
        available: true,
    },
    Entry {
        name: "H.265 Master",
        badge: Badge::Film("H.265", Color32::from_rgb(150, 30, 40)),
        sizes: &[],
        available: true,
    },
    Entry {
        name: "ProRes 422 HQ",
        badge: Badge::Film("ProRes", Color32::from_rgb(58, 58, 64)),
        sizes: &[],
        available: true,
    },
    Entry {
        name: "YouTube",
        badge: Badge::YouTube,
        sizes: WEB,
        available: true,
    },
    Entry {
        name: "Vimeo",
        badge: Badge::Vimeo,
        sizes: WEB,
        available: true,
    },
    Entry {
        name: "TikTok",
        badge: Badge::TikTok,
        sizes: &["720p", "1080p"],
        available: true,
    },
    Entry {
        name: "Presentations",
        badge: Badge::Presentations,
        sizes: &[],
        available: true,
    },
    Entry {
        name: "Dropbox",
        badge: Badge::Dropbox,
        sizes: WEB,
        available: true,
    },
    Entry {
        name: "IMF Generic",
        badge: Badge::Film("IMF", Color32::from_rgb(58, 58, 64)),
        sizes: &[],
        available: true,
    },
    Entry {
        name: "Final Cut Pro 7",
        badge: Badge::FinalCut,
        sizes: &[],
        available: true,
    },
    Entry {
        name: "Premiere XML",
        badge: Badge::Premiere,
        sizes: &[],
        available: true,
    },
    Entry {
        name: "Audio Only",
        badge: Badge::Audio,
        sizes: &[],
        available: true,
    },
    Entry {
        name: "AVID AAF",
        badge: Badge::Avid,
        sizes: &[],
        available: true,
    },
    Entry {
        name: "Pro Tools",
        badge: Badge::ProTools,
        sizes: &[],
        available: true,
    },
];

/// The name shown for a preset with a size: "YouTube 1080p".
pub(crate) fn title(entry: &Entry, size: Option<&str>) -> String {
    match size {
        Some(size) => format!("{} {size}", entry.name),
        None => entry.name.to_string(),
    }
}

/// The settings a preset gives: the codec, the resolution (an index as `DeliverUi::res_mode`
/// counts, with a custom size), and the encoder options.
pub(crate) struct Settings {
    pub preset: Preset,
    pub res_mode: usize,
    pub custom: Option<(u32, u32)>,
    pub options: Options,
}

/// What `entry` (at `size`, for one with a submenu) sets; `None` for a custom export, which keeps
/// the settings as they are.
pub(crate) fn settings(entry: &Entry, size: Option<&str>) -> Option<Settings> {
    // `res_mode`: 0 the timeline's size, then `RESOLUTIONS` (720p, 1080p, 1440p, 2160p, vertical
    // 1080 x 1920, square), then 7 for a custom size.
    let web_res = match size {
        Some("720p") => 1,
        Some("2160p") => 4,
        _ => 2,
    };
    let mut options = Options::default();
    let mut s = Settings {
        preset: Preset::H264Web,
        res_mode: 0,
        custom: None,
        options: Options::default(),
    };
    // `options` is filled below and moved into `s` at the end.
    match entry.name {
        "H.264 Master" => {
            options.profile = Profile::High;
            options.rate = Rate::Quality(Quality::Best);
        }
        "HyperDeck" => {
            options.profile = Profile::High;
            s.res_mode = 2;
        }
        "H.265 Master" => s.preset = Preset::H265,
        "ProRes 422 HQ" => {
            s.preset = Preset::ProRes422;
            options.prores = 3;
            options.audio_codec = AudioCodec::Pcm;
        }
        "YouTube" | "Vimeo" | "Dropbox" => {
            s.res_mode = web_res;
            options.profile = Profile::High;
            options.audio_codec = AudioCodec::Aac;
            options.audio_kbps = 320;
        }
        "TikTok" => {
            if size == Some("720p") {
                s.res_mode = 7;
                s.custom = Some((720, 1280));
            } else {
                s.res_mode = 5;
            }
            options.audio_codec = AudioCodec::Aac;
        }
        "Presentations" => {
            s.res_mode = 2;
            options.rate = Rate::Quality(Quality::High);
            options.audio_kbps = 192;
        }
        "Audio Only" => {
            s.preset = Preset::Wav;
            options.video = false;
        }
        "IMF Generic" => {
            s.preset = Preset::Mxf;
            options.audio_codec = AudioCodec::Pcm;
        }
        "Pro Tools" => {
            s.preset = Preset::ProTools;
            options.audio_codec = AudioCodec::Pcm;
        }
        _ => return None,
    }
    s.options = options;
    Some(s)
}

/// Paints a preset's icon, 28 points wide, centred on `c`.
pub(crate) fn paint_badge(p: &Painter, c: Pos2, badge: Badge) {
    let tile = |fill: Color32| {
        let r = Rect::from_center_size(c, vec2(28.0, 28.0));
        p.rect_filled(r, 6.0, fill);
        r
    };
    let white = Color32::WHITE;
    let play = |at: Pos2, size: f32, col: Color32| {
        p.add(egui::Shape::convex_polygon(
            vec![
                at + vec2(-size * 0.45, -size * 0.55),
                at + vec2(size * 0.6, 0.0),
                at + vec2(-size * 0.45, size * 0.55),
            ],
            col,
            Stroke::NONE,
        ));
    };
    match badge {
        Badge::Logo => {
            tile(Color32::from_rgb(40, 40, 46));
            crate::chrome::paint_logo(p.ctx(), p, c, 24.0);
        }
        Badge::Film(label, color) => {
            let r = Rect::from_center_size(c + vec2(0.0, -2.0), vec2(34.0, 24.0));
            p.rect_filled(r, 3.0, Color32::from_rgb(44, 30, 34));
            p.rect_filled(
                Rect::from_min_max(r.min, pos2(r.max.x, r.top() + 4.0)),
                2.0,
                Color32::from_rgb(110, 50, 58),
            );
            play(r.center() + vec2(-6.0, -1.0), 9.0, white);
            let tag =
                Rect::from_min_size(pos2(r.left() + 13.0, r.bottom() - 6.0), vec2(26.0, 11.0));
            p.rect_filled(tag, 2.0, color);
            crate::i18n::paint_text(p,
                tag.center(),
                Align2::CENTER_CENTER,
                label,
                regular(8.5),
                white,
            );
        }
        Badge::YouTube => {
            tile(Color32::from_rgb(230, 33, 23));
            play(c, 11.0, white);
        }
        Badge::Vimeo => {
            tile(Color32::from_rgb(26, 183, 234));
            crate::i18n::paint_text(p,
                c,
                Align2::CENTER_CENTER,
                "v",
                crate::theme::semibold(22.0),
                white,
            );
        }
        Badge::TikTok => {
            tile(Color32::from_rgb(10, 10, 12));
            for (shift, col) in [
                (vec2(-1.0, -1.0), Color32::from_rgb(37, 244, 238)),
                (vec2(1.0, 1.0), Color32::from_rgb(254, 44, 85)),
                (vec2(0.0, 0.0), white),
            ] {
                let at = c + shift;
                p.circle_filled(at + vec2(-3.0, 5.0), 3.6, col);
                p.line_segment(
                    [at + vec2(0.4, 5.0), at + vec2(0.4, -8.0)],
                    Stroke::new(2.4, col),
                );
                p.line_segment(
                    [at + vec2(0.4, -8.0), at + vec2(6.0, -4.5)],
                    Stroke::new(2.4, col),
                );
            }
        }
        Badge::Presentations => {
            let r = tile(Color32::from_rgb(36, 74, 150));
            p.rect_filled(
                Rect::from_center_size(c + vec2(0.0, 2.0), vec2(20.0, 14.0)),
                2.0,
                Color32::from_rgb(20, 40, 90),
            );
            play(c + vec2(0.0, 2.0), 8.0, white);
            p.circle_filled(
                r.right_top() + vec2(-4.0, 4.0),
                4.0,
                Color32::from_rgb(90, 200, 120),
            );
        }
        Badge::Dropbox => {
            tile(Color32::from_rgb(0, 97, 254));
            for (dx, dy) in [
                (-4.5, -5.0),
                (4.5, -5.0),
                (-4.5, 1.5),
                (4.5, 1.5),
                (0.0, 7.0),
            ] {
                let size = if dy > 5.0 { 2.6 } else { 3.6 };
                let m = c + vec2(dx, dy);
                p.add(egui::Shape::convex_polygon(
                    vec![
                        m + vec2(0.0, -size * 0.8),
                        m + vec2(size, 0.0),
                        m + vec2(0.0, size * 0.8),
                        m + vec2(-size, 0.0),
                    ],
                    white,
                    Stroke::NONE,
                ));
            }
        }
        Badge::FinalCut => {
            tile(Color32::from_rgb(112, 52, 190));
            let film = Rect::from_center_size(c, vec2(18.0, 14.0));
            p.rect_stroke(
                film,
                2.0,
                Stroke::new(1.6, Color32::from_rgb(230, 200, 255)),
                StrokeKind::Inside,
            );
            for i in 0..3 {
                let y = film.top() + 3.5 + 3.5 * i as f32;
                p.line_segment(
                    [pos2(film.left() + 3.0, y), pos2(film.right() - 3.0, y)],
                    Stroke::new(1.0, Color32::from_rgb(255, 120, 200)),
                );
            }
        }
        Badge::Premiere => {
            let r = tile(Color32::from_rgb(0, 0, 52));
            p.rect_stroke(
                r.shrink(1.0),
                5.0,
                Stroke::new(1.2, Color32::from_rgb(153, 153, 255)),
                StrokeKind::Inside,
            );
            crate::i18n::paint_text(p,
                c,
                Align2::CENTER_CENTER,
                "Pr",
                crate::theme::semibold(15.0),
                Color32::from_rgb(153, 153, 255),
            );
        }
        Badge::Audio => {
            tile(Color32::from_rgb(70, 140, 60));
            p.circle_filled(c + vec2(-4.0, 5.0), 3.0, white);
            p.circle_filled(c + vec2(5.0, 3.0), 3.0, white);
            p.line_segment(
                [c + vec2(-1.3, 5.0), c + vec2(-1.3, -7.0)],
                Stroke::new(1.8, white),
            );
            p.line_segment(
                [c + vec2(7.7, 3.0), c + vec2(7.7, -9.0)],
                Stroke::new(1.8, white),
            );
            p.line_segment(
                [c + vec2(-1.3, -7.0), c + vec2(7.7, -9.0)],
                Stroke::new(2.6, white),
            );
        }
        Badge::Avid => {
            tile(Color32::from_rgb(110, 70, 220));
            p.circle_stroke(c, 8.5, Stroke::new(1.8, white));
            p.rect_stroke(
                Rect::from_center_size(c, vec2(10.0, 8.0)),
                1.5,
                Stroke::new(1.4, white),
                StrokeKind::Inside,
            );
        }
        Badge::ProTools => {
            tile(Color32::from_rgb(124, 58, 237));
            p.circle_stroke(c, 8.5, Stroke::new(1.8, white));
            p.line_segment(
                [c + vec2(-4.5, 5.0), c + vec2(0.0, -4.0)],
                Stroke::new(1.8, white),
            );
            p.line_segment(
                [c + vec2(0.0, -4.0), c + vec2(4.5, 5.0)],
                Stroke::new(1.8, white),
            );
        }
    }
}

const ROW_H: f32 = 54.0;
const MENU_W: f32 = 376.0;

/// One row of the list: icon, name, and a ">" when it opens a submenu. Returns its response.
fn row(
    ui: &mut Ui,
    text: &str,
    badge: Option<Badge>,
    chevron: bool,
    usable: bool,
    selected: bool,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(
        vec2(MENU_W, ROW_H),
        if usable {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let p = ui.painter();
    if resp.hovered() && usable {
        p.rect_filled(rect, 4.0, Color32::from_rgb(52, 52, 60));
    }
    let mut x = rect.left() + 18.0;
    if let Some(badge) = badge {
        paint_badge(p, pos2(rect.left() + 42.0, rect.center().y), badge);
        x = rect.left() + 82.0;
    }
    crate::i18n::paint_text(p,
        pos2(x, rect.center().y),
        Align2::LEFT_CENTER,
        text,
        regular(19.0),
        match (usable, selected || resp.hovered()) {
            (false, _) => Color32::from_rgb(96, 96, 102),
            (true, true) => Color32::from_rgb(236, 236, 240),
            (true, false) => Color32::from_rgb(196, 196, 202),
        },
    );
    if chevron {
        paint(
            p,
            pos2(rect.right() - 22.0, rect.center().y),
            Icon::ChevRight,
            Color32::from_rgb(150, 150, 156),
        );
    }
    if !usable {
        resp.clone()
            .on_hover_text("Not available yet: needs an interchange writer (on the roadmap)");
    }
    resp
}

impl StudioApp {
    /// The list under the Preset box; applies the preset that is picked.
    pub(crate) fn preset_menu(&mut self, ui: &mut Ui) {
        ui.spacing_mut().item_spacing.y = 0.0;
        let current = self.deliver_ui.preset_name.clone();
        let mut picked: Option<(usize, Option<&'static str>)> = None;
        for (i, entry) in ENTRIES.iter().enumerate() {
            let selected =
                current == entry.name || current.starts_with(&format!("{} ", entry.name));
            // Resolve names the entries with a submenu after their 1080p choice.
            let shown = if entry.sizes.is_empty() {
                entry.name.to_string()
            } else {
                title(entry, Some("1080p"))
            };
            let resp = row(
                ui,
                &shown,
                Some(entry.badge),
                !entry.sizes.is_empty(),
                entry.available,
                selected,
            );
            if entry.sizes.is_empty() {
                if resp.clicked() {
                    picked = Some((i, None));
                }
                continue;
            }
            egui::containers::menu::SubMenu::new().show(ui, &resp, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for size in entry.sizes {
                    let name = title(entry, Some(size));
                    if row(ui, &name, None, false, true, current == name).clicked() {
                        picked = Some((i, Some(size)));
                        ui.close();
                    }
                }
            });
        }
        let Some((i, size)) = picked else { return };
        ui.close();
        self.apply_render_preset(i, size);
    }

    pub(crate) fn apply_render_preset(&mut self, index: usize, size: Option<&str>) {
        let entry = &ENTRIES[index];
        if !entry.available {
            return;
        }
        // The interchange presets write the timeline for another editor, pointing at the
        // original media (nothing is rendered).
        let ctx = self.ctx.clone();
        match entry.name {
            "Final Cut Pro 7" => return self.export_xml(&ctx, false),
            "Premiere XML" => return self.export_xml(&ctx, true),
            "AVID AAF" => return self.export_aaf(&ctx),
            _ => {}
        }
        self.deliver_ui.preset_name = title(entry, size);
        let Some(s) = settings(entry, size) else {
            return;
        };
        self.deliver.preset = s.preset;
        self.deliver.options = Options {
            clock_start: self.deliver.options.clock_start,
            ..s.options
        };
        self.deliver_ui.res_mode = s.res_mode;
        if let Some(custom) = s.custom {
            self.deliver_ui.custom = custom;
            self.deliver_ui.vertical = false;
        }
        self.deliver_ui.fps_choice = crate::deliver::DEFAULT_FPS_CHOICE;
        self.info(format!("Render preset: {}", self.deliver_ui.preset_name));
    }

    /// The badge of the preset in use, for the Preset box.
    pub(crate) fn preset_badge(&self) -> Badge {
        let name = &self.deliver_ui.preset_name;
        ENTRIES
            .iter()
            .find(|e| name == e.name || name.starts_with(&format!("{} ", e.name)))
            .map_or(Badge::Logo, |e| e.badge)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_set_codec_and_size() {
        let yt = ENTRIES.iter().find(|e| e.name == "YouTube").unwrap();
        let s = settings(yt, Some("2160p")).unwrap();
        assert_eq!((s.preset, s.res_mode), (Preset::H264Web, 4));
        let pr = ENTRIES.iter().find(|e| e.name == "ProRes 422 HQ").unwrap();
        let s = settings(pr, None).unwrap();
        assert_eq!((s.preset, s.options.prores), (Preset::ProRes422, 3));
        let tt = ENTRIES.iter().find(|e| e.name == "TikTok").unwrap();
        assert_eq!(
            settings(tt, Some("720p")).unwrap().custom,
            Some((720, 1280))
        );
        let audio = ENTRIES.iter().find(|e| e.name == "Audio Only").unwrap();
        assert_eq!(settings(audio, None).unwrap().preset, Preset::Wav);
        let imf = ENTRIES.iter().find(|e| e.name == "IMF Generic").unwrap();
        assert!(imf.available);
        assert_eq!(settings(imf, None).unwrap().preset, Preset::Mxf);
        assert_eq!(
            settings(imf, None).unwrap().options.audio_codec,
            AudioCodec::Pcm
        );
        let pt = ENTRIES.iter().find(|e| e.name == "Pro Tools").unwrap();
        assert!(pt.available);
        assert_eq!(settings(pt, None).unwrap().preset, Preset::ProTools);
        assert_eq!(
            settings(pt, None).unwrap().options.audio_codec,
            AudioCodec::Pcm
        );
        assert!(settings(&ENTRIES[0], None).is_none());
        assert_eq!(title(yt, Some("1080p")), "YouTube 1080p");
        // Every entry that can be picked gives settings, except Custom Export and the ones that
        // write a timeline file.
        let writes_timeline = ["Final Cut Pro 7", "Premiere XML", "AVID AAF"];
        for e in ENTRIES
            .iter()
            .skip(1)
            .filter(|e| e.available && !writes_timeline.contains(&e.name))
        {
            assert!(
                settings(e, e.sizes.first().copied()).is_some(),
                "{}",
                e.name
            );
        }
    }
}
