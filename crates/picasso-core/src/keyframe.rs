//! Keyframes: a clip property that changes over the clip. Each animated property has its own
//! list of keyframes (frames counted from the clip's first frame); between two keyframes the
//! value moves in a straight line (or eases in and out), before the first and after the last it
//! holds.

use crate::easing::KeyCurve;
use serde::{Deserialize, Serialize};

use crate::project::{Clip, Transform};

/// The most keyframes one property keeps.
pub const MAX_KEYFRAMES: usize = 1000;

/// The properties of a clip that can be animated.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum KeyParam {
    Opacity,
    PositionX,
    PositionY,
    /// Width and height together.
    Zoom,
    Rotation,
}

impl KeyParam {
    pub const ALL: [Self; 5] = [
        Self::Opacity,
        Self::PositionX,
        Self::PositionY,
        Self::Zoom,
        Self::Rotation,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Opacity => "Opacity",
            Self::PositionX => "Position X",
            Self::PositionY => "Position Y",
            Self::Zoom => "Zoom",
            Self::Rotation => "Rotation",
        }
    }

    /// The values the property can take.
    pub fn range(self) -> (f32, f32) {
        match self {
            Self::Opacity => (0.0, 1.0),
            Self::PositionX | Self::PositionY => (-4.0, 4.0),
            Self::Zoom => (0.01, 16.0),
            Self::Rotation => (-3600.0, 3600.0),
        }
    }

    /// The property's value on a clip that is not animated.
    pub fn base(self, clip: &Clip) -> f32 {
        let t = &clip.transform;
        match self {
            Self::Opacity => clip.opacity,
            Self::PositionX => t.x,
            Self::PositionY => t.y,
            Self::Zoom => (t.scale_x + t.scale_y) * 0.5,
            Self::Rotation => t.rotation,
        }
    }

    fn apply(self, value: f32, opacity: &mut f32, t: &mut Transform) {
        match self {
            Self::Opacity => *opacity = value,
            Self::PositionX => t.x = value,
            Self::PositionY => t.y = value,
            Self::Zoom => (t.scale_x, t.scale_y) = (value, value),
            Self::Rotation => t.rotation = value,
        }
    }
}

/// A value at a frame of the clip.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Keyframe {
    pub at: u64,
    pub value: f32,
    /// Eases in and out of this keyframe (slow near it) instead of moving at an even rate.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ease: bool,
    /// How the value moves from this keyframe to the next one (a straight line unless set).
    #[serde(default, skip_serializing_if = "KeyCurve::is_linear")]
    pub curve: KeyCurve,
}

/// The keyframes of one property, in time order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KeyTrack {
    pub param: KeyParam,
    pub keys: Vec<Keyframe>,
}

/// The value of `keys` (in time order, not empty) at clip frame `at`.
pub fn value_at(keys: &[Keyframe], at: u64) -> Option<f32> {
    let first = keys.first()?;
    if at <= first.at {
        return Some(first.value);
    }
    for pair in keys.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if at <= b.at {
            let span = (b.at - a.at).max(1) as f32;
            let mut t = (at - a.at) as f32 / span;
            // The curve of the keyframe the segment starts at; the older "ease" switch (on either
            // end) is a smooth step.
            if !a.curve.is_linear() {
                t = a.curve.eval(t);
            } else if a.ease || b.ease {
                t = KeyCurve::Smooth.eval(t);
            }
            return Some(a.value + (b.value - a.value) * t);
        }
    }
    keys.last().map(|k| k.value)
}

impl KeyTrack {
    /// The same track with its keyframes in order, one per frame, in range.
    pub fn sanitized(mut self) -> Self {
        let (lo, hi) = self.param.range();
        self.keys.retain(|k| k.value.is_finite());
        for k in &mut self.keys {
            k.value = k.value.clamp(lo, hi);
        }
        self.keys.sort_by_key(|k| k.at);
        self.keys.dedup_by_key(|k| k.at);
        self.keys.truncate(MAX_KEYFRAMES);
        self
    }
}

impl Clip {
    /// The keyframes of `param`, if it is animated.
    pub fn keys(&self, param: KeyParam) -> Option<&[Keyframe]> {
        self.keyframes
            .iter()
            .find(|t| t.param == param && !t.keys.is_empty())
            .map(|t| t.keys.as_slice())
    }

    /// The value of `param` at timeline frame `frame`.
    pub fn value_at(&self, param: KeyParam, frame: u64) -> f32 {
        let (lo, hi) = param.range();
        self.keys(param)
            .and_then(|k| value_at(k, frame.saturating_sub(self.start)))
            // A curve that overshoots (back, elastic) stays inside what the property can be.
            .map(|v| v.clamp(lo, hi))
            .unwrap_or_else(|| param.base(self))
    }

    /// The clip's opacity and transform at timeline frame `frame`, with its keyframes.
    pub fn animated(&self, frame: u64) -> (f32, Transform) {
        let (mut opacity, mut t) = (self.opacity, self.transform);
        for track in &self.keyframes {
            if let Some(v) = value_at(&track.keys, frame.saturating_sub(self.start)) {
                let (lo, hi) = track.param.range();
                track.param.apply(v.clamp(lo, hi), &mut opacity, &mut t);
            }
        }
        (opacity, t)
    }

    pub fn opacity_at(&self, frame: u64) -> f32 {
        self.animated(frame).0
    }

    pub fn transform_at(&self, frame: u64) -> Transform {
        self.animated(frame).1
    }

    /// Moves the keyframes `delta` frames later (earlier when negative), as when the clip's
    /// first frame moves within its media. What falls before the clip's start is replaced by a
    /// keyframe at its start holding the value there, so the animation shown does not change.
    pub fn shift_keys(&mut self, delta: i64) {
        for track in &mut self.keyframes {
            let at_start = (delta < 0)
                .then(|| value_at(&track.keys, delta.unsigned_abs()))
                .flatten();
            let mut keys: Vec<Keyframe> = track
                .keys
                .iter()
                .filter_map(|k| {
                    let at = k.at as i64 + delta;
                    (at > 0 || (at == 0 && at_start.is_none())).then_some(Keyframe {
                        at: at.max(0) as u64,
                        ..*k
                    })
                })
                .collect();
            if let Some(value) = at_start {
                keys.insert(
                    0,
                    Keyframe {
                        at: 0,
                        value,
                        ease: false,
                        curve: KeyCurve::Linear,
                    },
                );
            }
            track.keys = keys;
        }
        self.keyframes.retain(|t| !t.keys.is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(at: u64, value: f32) -> Keyframe {
        Keyframe {
            at,
            value,
            ease: false,
            curve: KeyCurve::Linear,
        }
    }

    #[test]
    fn values_move_between_keyframes_and_hold_outside() {
        let keys = [k(10, 0.0), k(20, 1.0)];
        assert_eq!(value_at(&keys, 0), Some(0.0));
        assert_eq!(value_at(&keys, 15), Some(0.5));
        assert_eq!(value_at(&keys, 99), Some(1.0));
        let eased = [
            Keyframe {
                ease: true,
                ..k(0, 0.0)
            },
            k(10, 1.0),
        ];
        let v = value_at(&eased, 2).unwrap();
        assert!(v < 0.2, "slow near the eased keyframe: {v}");
        assert_eq!(value_at(&eased, 5), Some(0.5));
        assert_eq!(value_at(&[], 5), None);
    }

    #[test]
    fn a_keyframe_can_ease_along_any_curve_to_the_next() {
        let with = |curve| {
            [
                Keyframe {
                    curve,
                    ..k(0, 0.0)
                },
                k(10, 1.0),
            ]
        };
        // A jump stays until the next keyframe.
        assert_eq!(value_at(&with(KeyCurve::Hold), 9), Some(0.0));
        assert_eq!(value_at(&with(KeyCurve::Hold), 10), Some(1.0));
        // Ease out is ahead of a straight line at the start, ease in behind it.
        let out = value_at(&with(KeyCurve::Bezier([0.33, 1.0, 0.68, 1.0])), 3).unwrap();
        let slow = value_at(&with(KeyCurve::Bezier([0.32, 0.0, 0.67, 0.0])), 3).unwrap();
        assert!(out > 0.3 && slow < 0.3, "{out} {slow}");
        // Bounce lands on the next value; elastic passes it on the way (so it is not capped).
        assert_eq!(value_at(&with(KeyCurve::BounceOut), 10), Some(1.0));
        assert!(
            (1..10).any(|at| value_at(&with(KeyCurve::ElasticOut), at).unwrap() > 1.0)
        );
        // The curve belongs to the keyframe it starts at: the next keyframe's does not matter.
        let two = [k(0, 0.0), Keyframe { curve: KeyCurve::Hold, ..k(10, 1.0) }];
        assert_eq!(value_at(&two, 5), Some(0.5));
    }

    #[test]
    fn a_property_stays_inside_its_range_when_a_curve_overshoots() {
        let mut c = crate::Project::demo().tracks[0].clips[0].clone();
        c.start = 0;
        c.keyframes = vec![KeyTrack {
            param: KeyParam::Opacity,
            keys: vec![
                Keyframe { curve: KeyCurve::ElasticOut, ..k(0, 0.0) },
                k(10, 1.0),
            ],
        }];
        for frame in 0..=10 {
            let o = c.opacity_at(frame);
            assert!((0.0..=1.0).contains(&o), "frame {frame}: {o}");
        }
    }

    #[test]
    fn shifting_keeps_what_is_shown() {
        let mut c = crate::Project::demo().tracks[0].clips[0].clone();
        c.keyframes = vec![KeyTrack {
            param: KeyParam::Opacity,
            keys: vec![k(0, 0.0), k(10, 1.0)],
        }];
        let before = c.value_at(KeyParam::Opacity, c.start + 6);
        c.shift_keys(-4);
        c.start += 4;
        assert_eq!(c.keys(KeyParam::Opacity).unwrap()[0], k(0, 0.4));
        assert!((c.value_at(KeyParam::Opacity, c.start + 2) - before).abs() < 1e-6);
        let t = KeyTrack {
            param: KeyParam::Zoom,
            keys: vec![k(5, f32::NAN), k(3, 99.0), k(3, 1.0)],
        }
        .sanitized();
        assert_eq!(t.keys, vec![k(3, 16.0)]);
    }
}
