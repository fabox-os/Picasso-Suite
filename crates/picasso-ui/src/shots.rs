//! Pictures of the program's own menus, drawn without a window: the menus are opened the way a
//! person opens them (pointer moves and clicks sent to the interface), the shapes egui makes are
//! turned into pixels by the small software renderer below, and the pictures are saved as PNG.
//! Not a check: `SHOT_DIR=/some/folder cargo test -p picasso-ui shots -- --ignored --nocapture`.

use std::collections::HashMap;
use std::path::PathBuf;

use egui::epaint::{ImageData, Primitive, Shape};
use egui::{Context, Event, Pos2, RawInput, Rect, TextureId};

use crate::app::StudioApp;

/// A texture the interface uploaded (the font atlas, icons): premultiplied RGBA.
struct Texture {
    w: usize,
    h: usize,
    px: Vec<[u8; 4]>,
}

impl Texture {
    /// Bilinear sample at uv (0 to 1), as floats 0 to 1.
    fn sample(&self, u: f32, v: f32) -> [f32; 4] {
        let x = (u * self.w as f32 - 0.5).clamp(0.0, (self.w - 1) as f32);
        let y = (v * self.h as f32 - 0.5).clamp(0.0, (self.h - 1) as f32);
        let (x0, y0) = (x as usize, y as usize);
        let (x1, y1) = ((x0 + 1).min(self.w - 1), (y0 + 1).min(self.h - 1));
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let at = |x: usize, y: usize, c: usize| f32::from(self.px[y * self.w + x][c]) / 255.0;
        [0, 1, 2, 3].map(|c| {
            let top = at(x0, y0, c) * (1.0 - fx) + at(x1, y0, c) * fx;
            let bottom = at(x0, y1, c) * (1.0 - fx) + at(x1, y1, c) * fx;
            top * (1.0 - fy) + bottom * fy
        })
    }
}

/// An interface being driven frame by frame.
struct Driver {
    size: egui::Vec2,
    scale: f32,
    ctx: Context,
    app: StudioApp,
    time: f64,
    textures: HashMap<TextureId, Texture>,
    last: Option<egui::FullOutput>,
}

const SIZE: egui::Vec2 = egui::vec2(1920.0, 1700.0);
const SCALE: f32 = 2.0;

impl Driver {
    fn new() -> Self {
        Self::sized(SIZE, SCALE)
    }

    fn sized(size: egui::Vec2, scale: f32) -> Self {
        let ctx = Context::default();
        ctx.set_pixels_per_point(scale);
        let mut app = StudioApp::new(&ctx);
        app.manager.open = false;
        app.load_demo();
        let mut d = Self {
            size,
            scale,
            ctx,
            app,
            time: 0.0,
            textures: HashMap::new(),
            last: None,
        };
        for _ in 0..6 {
            d.frame(vec![]);
        }
        d
    }

    fn frame(&mut self, events: Vec<Event>) {
        self.time += 0.016;
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
            events,
            time: Some(self.time),
            ..RawInput::default()
        };
        let mut out = self.ctx.run_ui(input, |ui| self.app.ui(ui));
        for (id, deltas) in out.textures_delta.set.drain() {
            for delta in deltas {
                let ImageData::Color(image) = &delta.image;
                let [w, h] = image.size;
                let pixels: Vec<[u8; 4]> = image.pixels.iter().map(|c| c.to_array()).collect();
                let texture = self.textures.entry(id).or_insert_with(|| Texture {
                    w,
                    h,
                    px: vec![[0; 4]; w * h],
                });
                match delta.pos {
                    None => {
                        texture.w = w;
                        texture.h = h;
                        texture.px = pixels;
                    }
                    Some([x0, y0]) => {
                        for y in 0..h {
                            for x in 0..w {
                                let (tx, ty) = (x0 + x, y0 + y);
                                if tx < texture.w && ty < texture.h {
                                    texture.px[ty * texture.w + tx] = pixels[y * w + x];
                                }
                            }
                        }
                    }
                }
            }
        }
        self.last = Some(out);
    }

    fn frames(&mut self, n: usize, events: Vec<Event>) {
        for i in 0..n {
            self.frame(if i == 0 { events.clone() } else { vec![] });
        }
    }

    fn move_to(&mut self, p: Pos2) {
        // Slowly, so a menu that opens on hover has the time to.
        self.frames(3, vec![Event::PointerMoved(p)]);
        self.frames(28, vec![]);
    }

    fn click(&mut self, p: Pos2) {
        self.frames(3, vec![Event::PointerMoved(p)]);
        let button = |pressed| Event::PointerButton {
            pos: p,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        self.frame(vec![button(true)]);
        self.frame(vec![button(false)]);
        self.frames(6, vec![]);
    }

    /// Where the last frame drew a label, the topmost one (popups come after the page).
    fn find(&self, label: &str) -> Option<Pos2> {
        fn walk(shape: &Shape, label: &str, found: &mut Option<Pos2>) {
            match shape {
                Shape::Text(t) if t.galley.job.text.trim() == label => {
                    *found = Some(t.pos + t.galley.rect.center().to_vec2());
                }
                Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, label, found)),
                _ => {}
            }
        }
        let mut found = None;
        for clipped in &self.last.as_ref()?.shapes {
            walk(&clipped.shape, label, &mut found);
        }
        found
    }

    /// The last frame as pixels (premultiplied RGBA over the dark window colour).
    fn pixels(&self) -> (usize, usize, Vec<u8>) {
        let out = self.last.as_ref().expect("a frame");
        let ppp = out.pixels_per_point;
        let (w, h) = ((self.size.x * ppp) as usize, (self.size.y * ppp) as usize);
        let mut img = vec![0u8; w * h * 4];
        for px in img.chunks_exact_mut(4) {
            px.copy_from_slice(&[12, 12, 14, 255]);
        }
        let primitives = self.ctx.tessellate(out.shapes.clone(), ppp);
        for clipped in &primitives {
            let Primitive::Mesh(mesh) = &clipped.primitive else {
                continue;
            };
            let Some(texture) = self.textures.get(&mesh.texture_id) else {
                continue;
            };
            let clip = (
                (clipped.clip_rect.min.x * ppp).max(0.0),
                (clipped.clip_rect.min.y * ppp).max(0.0),
                (clipped.clip_rect.max.x * ppp).min(w as f32),
                (clipped.clip_rect.max.y * ppp).min(h as f32),
            );
            for tri in mesh.indices.chunks_exact(3) {
                let v = [
                    mesh.vertices[tri[0] as usize],
                    mesh.vertices[tri[1] as usize],
                    mesh.vertices[tri[2] as usize],
                ];
                let p = v.map(|v| (v.pos.x * ppp, v.pos.y * ppp));
                let (min_x, max_x) = (
                    p.iter().map(|q| q.0).fold(f32::MAX, f32::min).max(clip.0),
                    p.iter().map(|q| q.0).fold(f32::MIN, f32::max).min(clip.2),
                );
                let (min_y, max_y) = (
                    p.iter().map(|q| q.1).fold(f32::MAX, f32::min).max(clip.1),
                    p.iter().map(|q| q.1).fold(f32::MIN, f32::max).min(clip.3),
                );
                let area =
                    (p[1].0 - p[0].0) * (p[2].1 - p[0].1) - (p[2].0 - p[0].0) * (p[1].1 - p[0].1);
                if area.abs() < 1e-9 || min_x >= max_x || min_y >= max_y {
                    continue;
                }
                for y in (min_y.floor() as usize)..(max_y.ceil() as usize).min(h) {
                    for x in (min_x.floor() as usize)..(max_x.ceil() as usize).min(w) {
                        let (cx, cy) = (x as f32 + 0.5, y as f32 + 0.5);
                        let w0 =
                            ((p[1].0 - cx) * (p[2].1 - cy) - (p[2].0 - cx) * (p[1].1 - cy)) / area;
                        let w1 =
                            ((p[2].0 - cx) * (p[0].1 - cy) - (p[0].0 - cx) * (p[2].1 - cy)) / area;
                        let w2 = 1.0 - w0 - w1;
                        if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                            continue;
                        }
                        let u = w0 * v[0].uv.x + w1 * v[1].uv.x + w2 * v[2].uv.x;
                        let t = w0 * v[0].uv.y + w1 * v[1].uv.y + w2 * v[2].uv.y;
                        let tex = texture.sample(u, t);
                        let col = |c: usize| {
                            (w0 * f32::from(v[0].color.to_array()[c])
                                + w1 * f32::from(v[1].color.to_array()[c])
                                + w2 * f32::from(v[2].color.to_array()[c]))
                                / 255.0
                        };
                        let src = [0, 1, 2, 3].map(|c| col(c) * tex[c]);
                        let i = (y * w + x) * 4;
                        for c in 0..4 {
                            let dst = f32::from(img[i + c]) / 255.0;
                            let out = src[c] + dst * (1.0 - src[3]);
                            img[i + c] = (out.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                        }
                    }
                }
            }
        }
        (w, h, img)
    }
}

/// Opens Effects, then each entry of `path` in turn (the pointer rests on it so a submenu opens),
/// and saves the part of the window that the open menus cover.
fn shot(path: &[&str], name: &str) {
    let mut d = Driver::new();
    let (_, _, before) = d.pixels();
    d.click(Pos2::new(763.0 + 14.0, 19.0));
    for label in path {
        let Some(at) = d.find(label) else {
            panic!("no \"{label}\" in the open menu (path {path:?})");
        };
        d.move_to(at);
    }
    let (w, h, after) = d.pixels();
    // What the menus cover: where the picture is not what it was before.
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0usize, 0usize);
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) * 4;
            if before[i..i + 3] != after[i..i + 3] {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    assert!(x1 > x0 && y1 > y0, "{name}: nothing opened");
    let pad = 16usize;
    let (x0, y0) = (x0.saturating_sub(pad), y0.saturating_sub(pad));
    let (x1, y1) = ((x1 + pad).min(w), (y1 + pad).min(h));
    let mut out = Vec::with_capacity((x1 - x0) * (y1 - y0) * 4);
    for y in y0..y1 {
        out.extend_from_slice(&after[(y * w + x0) * 4..(y * w + x1) * 4]);
    }
    let dir = PathBuf::from(std::env::var("SHOT_DIR").unwrap_or_else(|_| ".".into()));
    let file = dir.join(format!("{name}.png"));
    image::RgbaImage::from_raw((x1 - x0) as u32, (y1 - y0) as u32, out)
        .expect("size")
        .save(&file)
        .expect("write the picture");
    eprintln!("wrote {}", file.display());
}

#[test]
#[ignore]
fn shots() {
    shot(&["Video Transitions", "Wipe"], "menu-transitions-wipe");
    shot(&["Video Transitions", "Iris"], "menu-transitions-iris");
    shot(&["Video Transitions", "Shape"], "menu-transitions-shape");
    shot(&["Video Transitions", "Motion"], "menu-transitions-motion");
    shot(
        &["Video Transitions", "More Transitions"],
        "menu-transitions-more",
    );
    shot(&["Picasso FX"], "menu-fx-groups");
    for (group, slug) in [
        ("Key", "key"),
        ("Restore", "restore"),
        ("Time", "time"),
        ("Warp", "warp"),
        ("Light", "light"),
        ("Texture", "texture"),
        ("Transform", "transform"),
        ("Sharpen", "sharpen"),
        ("Stylize", "stylize"),
        ("Blur", "blur"),
        ("Color", "color"),
    ] {
        shot(&["Picasso FX", group], &format!("menu-fx-{slug}"));
    }
    shot(&["Generators"], "menu-generators");
    shot(
        &["Generators", "More Gradients"],
        "menu-generators-gradients",
    );
    shot(
        &["Generators", "Test Patterns"],
        "menu-generators-test-patterns",
    );
    shot(
        &["Generators", "Patterns and Utilities"],
        "menu-generators-patterns",
    );
}

/// The Choose Video Layout window on a new project, for each platform, as pictures.
#[test]
#[ignore]
fn video_layout_shots() {
    let dir = PathBuf::from(std::env::var("SHOT_DIR").unwrap_or_else(|_| ".".into()));
    let mut d = Driver::new();
    d.app.new_project();
    d.frames(6, vec![]);
    for (platform, name) in [
        (picasso_core::video_layout::Platform::YouTube, "video-layout-youtube"),
        (picasso_core::video_layout::Platform::Instagram, "video-layout-instagram"),
        (picasso_core::video_layout::Platform::TikTok, "video-layout-tiktok"),
    ] {
        if let Some(c) = d.app.extra.video_layout.as_mut() {
            c.pick_platform(platform);
        }
        d.frames(4, vec![]);
        let (w, h, px) = d.pixels();
        let file = dir.join(format!("{name}.png"));
        image::RgbaImage::from_raw(w as u32, h as u32, px)
            .expect("size")
            .save(&file)
            .expect("write the picture");
        eprintln!("wrote {}", file.display());
    }
}

/// The Color page with its palettes divider dragged up and down, and the Deliver page with its
/// render settings column narrowed, as pictures (for the issue about panels that do not scale).
#[test]
#[ignore]
fn responsive_shots() {
    use crate::app::Page;
    let dir = PathBuf::from(std::env::var("SHOT_DIR").unwrap_or_else(|_| ".".into()));
    let save = |d: &mut Driver, name: &str| {
        d.frames(6, vec![]);
        let (w, h, px) = d.pixels();
        let file = dir.join(format!("{name}.png"));
        image::RgbaImage::from_raw(w as u32, h as u32, px)
            .expect("size")
            .save(&file)
            .expect("write the picture");
        eprintln!("wrote {}", file.display());
    };
    let size = egui::vec2(1920.0, 1080.0);
    for (moved, name) in [(0, "usual"), (-150, "up"), (120, "down")] {
        let mut d = Driver::sized(size, 1.0);
        d.app.set_page(Page::Color);
        d.app.extra.prefs.panels.moved.clear();
        if moved != 0 {
            d.app
                .extra
                .prefs
                .panels
                .moved
                .insert("color palettes".to_string(), moved);
        }
        save(&mut d, &format!("color-palettes-{name}"));
    }
    for (moved, name) in [(0, "usual"), (-150, "narrow"), (-250, "narrower"), (-500, "narrowest")] {
        let mut d = Driver::sized(size, 1.0);
        d.app.set_page(Page::Deliver);
        d.app.extra.prefs.panels.moved.clear();
        if moved != 0 {
            d.app
                .extra
                .prefs
                .panels
                .moved
                .insert("deliver settings".to_string(), moved);
        }
        save(&mut d, &format!("deliver-form-{name}"));
    }
}
