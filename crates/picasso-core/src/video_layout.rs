//! The video layouts a project can be set up with: a platform (YouTube, Instagram, Facebook,
//! TikTok and more), a template of it (a video, a short, a reel, a story, a post...) and a quality
//! (4K to 720p), which together give the picture size of the project. One table and one piece
//! of arithmetic: the chooser window, the menu and the tests all use these.

/// Where the video is going.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Platform {
    YouTube,
    Instagram,
    Facebook,
    TikTok,
    More,
}

impl Platform {
    pub const ALL: [Self; 5] = [
        Self::YouTube,
        Self::Instagram,
        Self::Facebook,
        Self::TikTok,
        Self::More,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::YouTube => "YouTube",
            Self::Instagram => "Instagram",
            Self::Facebook => "Facebook",
            Self::TikTok => "TikTok",
            Self::More => "More",
        }
    }
}

/// The shape of the picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Aspect {
    /// 16:9, wide.
    Wide,
    /// 9:16, tall.
    Tall,
    /// 1:1.
    Square,
    /// 4:5.
    Portrait,
    /// 4:3.
    Classic,
}

impl Aspect {
    pub fn label(self) -> &'static str {
        match self {
            Self::Wide => "16:9",
            Self::Tall => "9:16",
            Self::Square => "1:1",
            Self::Portrait => "4:5",
            Self::Classic => "4:3",
        }
    }

    /// The proportions, width then height, for drawing the shape.
    pub fn ratio(self) -> (u32, u32) {
        match self {
            Self::Wide => (16, 9),
            Self::Tall => (9, 16),
            Self::Square => (1, 1),
            Self::Portrait => (4, 5),
            Self::Classic => (4, 3),
        }
    }
}

/// How sharp the picture is, by the short side of a 16:9 frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Quality {
    Uhd4k,
    P1440,
    P1080,
    P720,
}

impl Quality {
    pub const ALL: [Self; 4] = [Self::Uhd4k, Self::P1440, Self::P1080, Self::P720];

    pub fn label(self) -> &'static str {
        match self {
            Self::Uhd4k => "4K",
            Self::P1440 => "1440p",
            Self::P1080 => "1080p",
            Self::P720 => "720p",
        }
    }

    /// The two sides of a 16:9 frame at this quality: the short one and the long one.
    pub fn edges(self) -> (u32, u32) {
        match self {
            Self::Uhd4k => (2160, 3840),
            Self::P1440 => (1440, 2560),
            Self::P1080 => (1080, 1920),
            Self::P720 => (720, 1280),
        }
    }
}

/// One entry of a platform's list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Template {
    pub id: &'static str,
    pub platform: Platform,
    pub label: &'static str,
    pub aspect: Aspect,
}

const fn template(id: &'static str, platform: Platform, label: &'static str, aspect: Aspect) -> Template {
    Template {
        id,
        platform,
        label,
        aspect,
    }
}

/// Every template, in the order each platform lists them.
pub const TEMPLATES: [Template; 17] = [
    template("yt_video", Platform::YouTube, "YT Video", Aspect::Wide),
    template("yt_short", Platform::YouTube, "YT Short", Aspect::Tall),
    template("ig_reel", Platform::Instagram, "IG Reel", Aspect::Tall),
    template("ig_story", Platform::Instagram, "IG Story", Aspect::Tall),
    template("ig_post", Platform::Instagram, "IG Post", Aspect::Square),
    template("ig_feed", Platform::Instagram, "IG Feed", Aspect::Portrait),
    template("fb_reel", Platform::Facebook, "FB Reel", Aspect::Tall),
    template("fb_video", Platform::Facebook, "FB Video", Aspect::Wide),
    template("fb_story", Platform::Facebook, "FB Story", Aspect::Tall),
    template("tiktok", Platform::TikTok, "TikTok", Aspect::Tall),
    template("snapchat", Platform::More, "Snapchat", Aspect::Tall),
    template("x_video", Platform::More, "X / Twitter", Aspect::Wide),
    template("linkedin", Platform::More, "LinkedIn", Aspect::Wide),
    template("square", Platform::More, "Square", Aspect::Square),
    template("landscape", Platform::More, "Landscape", Aspect::Wide),
    template("portrait", Platform::More, "Portrait", Aspect::Tall),
    template("classic", Platform::More, "Classic", Aspect::Classic),
];

/// The templates of one platform, in order.
pub fn templates_of(platform: Platform) -> impl Iterator<Item = &'static Template> {
    TEMPLATES.iter().filter(move |t| t.platform == platform)
}

/// The template with this id.
pub fn template_by_id(id: &str) -> Option<&'static Template> {
    TEMPLATES.iter().find(|t| t.id == id)
}

/// What a new project is set up with before anyone chooses.
pub const DEFAULT: (&str, Quality) = ("yt_video", Quality::P1080);

/// The picture size, width and height in pixels, of `template` at `quality`: the short side is
/// the quality's, and the long side follows the shape.
pub fn size_for(template: &Template, quality: Quality) -> (u32, u32) {
    let (short, long) = quality.edges();
    match template.aspect {
        Aspect::Tall => (short, long),
        Aspect::Portrait => (short, short * 5 / 4),
        Aspect::Square => (short, short),
        Aspect::Classic => (short * 4 / 3, short),
        Aspect::Wide => (long, short),
    }
}

/// The template and quality whose size is nearest to a picture of `width` x `height`: what a
/// project that already has a size shows when its layout is chosen again. The first of equally
/// near ones wins.
pub fn nearest(width: u32, height: u32) -> (&'static Template, Quality) {
    let mut best = (&TEMPLATES[0], Quality::P1080);
    let mut best_gap = u64::MAX;
    for t in &TEMPLATES {
        for q in Quality::ALL {
            let (w, h) = size_for(t, q);
            let gap = u64::from(w.abs_diff(width)) + u64::from(h.abs_diff(height));
            if gap < best_gap {
                best_gap = gap;
                best = (t, q);
            }
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn size(id: &str, q: Quality) -> (u32, u32) {
        size_for(template_by_id(id).unwrap(), q)
    }

    #[test]
    fn the_templates_are_listed_once_and_every_platform_has_some() {
        let mut ids: Vec<&str> = TEMPLATES.iter().map(|t| t.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), TEMPLATES.len());
        for platform in Platform::ALL {
            assert!(templates_of(platform).count() >= 1, "{platform:?}");
        }
        assert_eq!(templates_of(Platform::YouTube).count(), 2);
        assert_eq!(templates_of(Platform::Instagram).count(), 4);
        assert_eq!(templates_of(Platform::Facebook).count(), 3);
        assert_eq!(templates_of(Platform::TikTok).count(), 1);
        assert_eq!(DEFAULT.0, TEMPLATES[0].id);
    }

    #[test]
    fn a_size_follows_the_shape_and_the_quality() {
        assert_eq!(size("yt_video", Quality::P1080), (1920, 1080));
        assert_eq!(size("yt_video", Quality::Uhd4k), (3840, 2160));
        assert_eq!(size("yt_short", Quality::P1080), (1080, 1920));
        assert_eq!(size("tiktok", Quality::P720), (720, 1280));
        assert_eq!(size("ig_post", Quality::P1440), (1440, 1440));
        assert_eq!(size("ig_feed", Quality::P1080), (1080, 1350));
        assert_eq!(size("classic", Quality::P1080), (1440, 1080));
        assert_eq!(size("classic", Quality::P720), (960, 720));
        assert_eq!(size("landscape", Quality::P1440), (2560, 1440));
    }

    #[test]
    fn every_size_is_one_a_project_accepts_and_has_its_shape() {
        for t in &TEMPLATES {
            for q in Quality::ALL {
                let (w, h) = size_for(t, q);
                assert!(w > 0 && h > 0 && w <= 16_384 && h <= 16_384, "{} {q:?}", t.id);
                let (rw, rh) = t.aspect.ratio();
                // The shape is right to a pixel.
                assert!(
                    (u64::from(w) * u64::from(rh)).abs_diff(u64::from(h) * u64::from(rw)) <= u64::from(rw.max(rh)),
                    "{} {q:?}: {w}x{h} is not {}",
                    t.id,
                    t.aspect.label()
                );
            }
        }
    }

    #[test]
    fn the_nearest_layout_of_a_size_is_found_again() {
        for t in &TEMPLATES {
            for q in Quality::ALL {
                let (w, h) = size_for(t, q);
                let (found, quality) = nearest(w, h);
                // Another template may be the same size (a video and a reel are both 9:16, say);
                // the size it gives is what must be the same.
                assert_eq!(size_for(found, quality), (w, h), "{} {q:?}", t.id);
            }
        }
        // The first of equal ones wins: 1920x1080 is a YouTube video, not a Facebook one.
        assert_eq!(nearest(1920, 1080).0.id, "yt_video");
        assert_eq!(nearest(1080, 1920).0.id, "yt_short");
        // A size between two is given the nearer.
        assert_eq!(nearest(1900, 1070), (&TEMPLATES[0], Quality::P1080));
    }
}
