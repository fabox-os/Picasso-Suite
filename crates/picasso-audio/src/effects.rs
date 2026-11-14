//! The signal processing of a track's sound effects ([`picasso_core::AudioEffectKind`]): rooms and
//! echoes, modulation, lo-fi, device and voice effects. All of it works on interleaved stereo
//! blocks at [`SAMPLE_RATE`], one block after the other, and keeps what it needs between blocks.
//!
//! Each effect is a [`Stage`] built from a few small parts (filters, delay lines, a pitch
//! shifter, a reverb): the knobs are read again for every block, so moving one while the sound
//! plays has no click beyond what the effect itself does.

use std::f32::consts::TAU;
use std::fmt;

use picasso_core::{AudioEffect, AudioEffectKind, MAX_PARAMS};

use crate::SAMPLE_RATE;

const SR: f32 = SAMPLE_RATE as f32;

/// The knobs of one effect, in the order of `AudioEffectKind::params`.
type Knobs = [f32; MAX_PARAMS];

/// The loudest a sample may get after an effect: effects with feedback can never run away.
const LIMIT: f32 = 8.0;

trait Stage: Send {
    /// Runs `buf` (interleaved stereo) through the effect.
    fn process(&mut self, k: &Knobs, buf: &mut [f32]);
}

// ---- parts -----------------------------------------------------------------------------------

/// A position in a cycle, in turns (0 to 1).
#[derive(Clone, Copy, Default)]
struct Lfo {
    phase: f32,
}

impl Lfo {
    /// Moves on by one sample at `hz`.
    fn advance(&mut self, hz: f32) {
        self.phase = (self.phase + hz / SR).fract();
    }

    /// 0 to 1, a sine that starts at 0.5 and rises.
    fn unipolar(&self, offset: f32) -> f32 {
        0.5 + 0.5 * (TAU * (self.phase + offset)).sin()
    }

    /// -1 to 1.
    fn bipolar(&self, offset: f32) -> f32 {
        (TAU * (self.phase + offset)).sin()
    }
}

/// A delay of up to a fixed time, read at any (fractional) delay.
#[derive(Clone)]
struct DelayLine {
    buf: Vec<f32>,
    pos: usize,
}

impl DelayLine {
    fn new(seconds: f32) -> Self {
        Self {
            buf: vec![0.0; (seconds * SR) as usize + 4],
            pos: 0,
        }
    }

    fn push(&mut self, x: f32) {
        self.buf[self.pos] = x;
        self.pos = (self.pos + 1) % self.buf.len();
    }

    /// The sound that was pushed `delay` samples ago (1 is the last one).
    fn read(&self, delay: f32) -> f32 {
        let len = self.buf.len();
        let d = delay.clamp(1.0, (len - 2) as f32);
        let whole = d as usize;
        let frac = d - whole as f32;
        let a = self.buf[(self.pos + len - whole) % len];
        let b = self.buf[(self.pos + len - whole - 1) % len];
        a + (b - a) * frac
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Shape {
    Low,
    High,
    Band,
}

/// A second-order filter for both channels (the formulas of the "Audio EQ Cookbook").
#[derive(Clone, Copy)]
struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
    z: [[f64; 2]; 2],
}

impl Biquad {
    fn new() -> Self {
        Self {
            b: [1.0, 0.0, 0.0],
            a: [0.0, 0.0],
            z: [[0.0; 2]; 2],
        }
    }

    /// Sets the filter's shape, keeping what it remembers.
    fn tune(&mut self, shape: Shape, freq: f32, q: f32) {
        let f = f64::from(freq.clamp(20.0, SR * 0.45));
        let w = std::f64::consts::TAU * f / f64::from(SR);
        let (sin, cos) = w.sin_cos();
        let alpha = sin / (2.0 * f64::from(q.max(0.05)));
        let (b, a0) = match shape {
            Shape::Low => ([(1.0 - cos) / 2.0, 1.0 - cos, (1.0 - cos) / 2.0], 1.0 + alpha),
            Shape::High => (
                [(1.0 + cos) / 2.0, -(1.0 + cos), (1.0 + cos) / 2.0],
                1.0 + alpha,
            ),
            Shape::Band => ([alpha, 0.0, -alpha], 1.0 + alpha),
        };
        self.b = [b[0] / a0, b[1] / a0, b[2] / a0];
        self.a = [-2.0 * cos / a0, (1.0 - alpha) / a0];
    }

    fn run(&mut self, ch: usize, x: f32) -> f32 {
        let x = f64::from(x);
        let z = &mut self.z[ch];
        let y = self.b[0] * x + z[0];
        z[0] = self.b[1] * x - self.a[0] * y + z[1];
        z[1] = self.b[2] * x - self.a[1] * y;
        y as f32
    }
}

/// How far a sound is lowered, in decibels, to the level: `20 * log10`.
fn to_db(x: f32) -> f32 {
    20.0 * x.max(1e-9).log10()
}

fn from_db(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// The per-sample step of a smoother that gets 63% of the way in `ms`.
fn smoothing(ms: f32) -> f32 {
    (-1.0 / (ms * SR / 1000.0).max(1.0)).exp()
}

/// Mixes the dry sound with the wet one by `amount` (0 all dry, 1 all wet).
fn blend(dry: f32, wet: f32, amount: f32) -> f32 {
    dry + (wet - dry) * amount
}

/// Quantises the sound to fewer levels and holds it for several samples.
#[derive(Clone, Copy, Default)]
struct Crusher {
    hold: [f32; 2],
    left: [u32; 2],
}

impl Crusher {
    /// `logarithmic` makes the steps grow with the level, so quiet sound is crushed as hard as
    /// loud sound.
    fn run(&mut self, ch: usize, x: f32, bits: f32, hold: u32, mix: f32, logarithmic: bool) -> f32 {
        if self.left[ch] == 0 {
            self.hold[ch] = if logarithmic {
                let magnitude = x.abs();
                if magnitude < 1e-6 {
                    0.0
                } else {
                    let steps = (bits * 4.0).max(1.0);
                    ((magnitude.log2() * steps).round() / steps).exp2().copysign(x)
                }
            } else {
                let levels = (bits - 1.0).exp2();
                (x * levels).round() / levels
            };
            self.left[ch] = hold.max(1);
        }
        self.left[ch] -= 1;
        blend(x, self.hold[ch], mix)
    }
}

/// A short random pop and a hiss are made from this.
#[derive(Clone, Copy)]
struct Noise(u32);

impl Noise {
    /// -1 to 1.
    fn next(&mut self) -> f32 {
        // xorshift32
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

// ---- space -----------------------------------------------------------------------------------

/// A room: eight parallel comb filters and four all-pass filters per ear (the arrangement of the
/// public-domain "Freeverb"), lengths in samples at 44.1 kHz.
struct Reverb {
    combs: [Vec<Comb>; 2],
    allpass: [Vec<AllPass>; 2],
}

#[derive(Clone)]
struct Comb {
    buf: Vec<f32>,
    pos: usize,
    store: f32,
}

#[derive(Clone)]
struct AllPass {
    buf: Vec<f32>,
    pos: usize,
}

const COMB_LENGTHS: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const ALLPASS_LENGTHS: [usize; 4] = [556, 441, 341, 225];
/// The right ear's filters are a little longer, which is where the width comes from.
const SPREAD: usize = 23;

impl Reverb {
    fn new() -> Self {
        let scale = SR / 44_100.0;
        let size = |n: usize| ((n as f32 * scale) as usize).max(8);
        let build = |extra: usize| {
            (
                COMB_LENGTHS
                    .iter()
                    .map(|&n| Comb {
                        buf: vec![0.0; size(n + extra)],
                        pos: 0,
                        store: 0.0,
                    })
                    .collect::<Vec<_>>(),
                ALLPASS_LENGTHS
                    .iter()
                    .map(|&n| AllPass {
                        buf: vec![0.0; size(n + extra)],
                        pos: 0,
                    })
                    .collect::<Vec<_>>(),
            )
        };
        let (cl, al) = build(0);
        let (cr, ar) = build(SPREAD);
        Self {
            combs: [cl, cr],
            allpass: [al, ar],
        }
    }
}

impl Stage for Reverb {
    fn process(&mut self, k: &Knobs, buf: &mut [f32]) {
        let feedback = k[0] * 0.28 + 0.7;
        let damp = k[1] * 0.4;
        let width = k[2];
        let mix = k[3];
        let wet1 = (1.0 + width) / 2.0;
        let wet2 = (1.0 - width) / 2.0;
        for frame in buf.chunks_exact_mut(2) {
            let input = (frame[0] + frame[1]) * 0.015;
            let mut ears = [0.0f32; 2];
            for (ear, out) in ears.iter_mut().enumerate() {
                let mut sum = 0.0;
                for c in &mut self.combs[ear] {
                    let y = c.buf[c.pos];
                    c.store = y * (1.0 - damp) + c.store * damp;
                    c.buf[c.pos] = input + c.store * feedback;
                    c.pos = (c.pos + 1) % c.buf.len();
                    sum += y;
                }
                for a in &mut self.allpass[ear] {
                    let held = a.buf[a.pos];
                    let y = -sum + held;
                    a.buf[a.pos] = sum + held * 0.5;
                    a.pos = (a.pos + 1) % a.buf.len();
                    sum = y;
                }
                *out = sum;
            }
            // The tail is brought up to the level of the dry sound.
            let wet = [
                (ears[0] * wet1 + ears[1] * wet2) * 3.0,
                (ears[1] * wet1 + ears[0] * wet2) * 3.0,
            ];
            frame[0] = frame[0] * (1.0 - mix) + wet[0] * mix;
            frame[1] = frame[1] * (1.0 - mix) + wet[1] * mix;
        }
    }
}

/// A short delay that moves: chorus (a voice that is a little off), and what Underwater uses.
struct Chorus {
    lines: [DelayLine; 2],
    lfo: Lfo,
}

impl Chorus {
    fn new() -> Self {
        Self {
            lines: [DelayLine::new(0.12), DelayLine::new(0.12)],
            lfo: Lfo::default(),
        }
    }

    fn run(&mut self, buf: &mut [f32], delay_ms: f32, rate: f32, depth_ms: f32, mix: f32) {
        let ms = SR / 1000.0;
        for frame in buf.chunks_exact_mut(2) {
            for (ch, x) in frame.iter_mut().enumerate() {
                // The ears sweep a quarter of a cycle apart.
                let swing = self.lfo.bipolar(ch as f32 * 0.25) * depth_ms;
                let wet = self.lines[ch].read((delay_ms + swing).max(0.1) * ms);
                self.lines[ch].push(*x);
                *x = *x * (1.0 - 0.5 * mix) + wet * 0.5 * mix;
            }
            self.lfo.advance(rate);
        }
    }
}

impl Stage for Chorus {
    fn process(&mut self, k: &Knobs, buf: &mut [f32]) {
        self.run(buf, k[0], k[1], k[2], k[3]);
    }
}

/// The comb sweep of a short delay with feedback.
struct Flanger {
    lines: [DelayLine; 2],
    lfo: Lfo,
}

impl Flanger {
    fn new() -> Self {
        Self {
            lines: [DelayLine::new(0.12), DelayLine::new(0.12)],
            lfo: Lfo::default(),
        }
    }

    /// `feedback` -0.95 to 0.95, `mix` 0 to 1, `phase` the right ear's offset in turns.
    #[allow(clippy::too_many_arguments)]
    fn run(
        &mut self,
        buf: &mut [f32],
        rate: f32,
        delay_ms: f32,
        depth_ms: f32,
        feedback: f32,
        mix: f32,
        phase: f32,
    ) {
        let ms = SR / 1000.0;
        // A wide mix does not simply get louder than a narrow one.
        let normalise = 1.0 / (1.0 + mix);
        for frame in buf.chunks_exact_mut(2) {
            for (ch, x) in frame.iter_mut().enumerate() {
                let offset = if ch == 0 { 0.0 } else { phase };
                let sweep = depth_ms * self.lfo.unipolar(offset);
                let wet = self.lines[ch].read(((delay_ms + sweep) * ms).max(1.0));
                self.lines[ch].push(*x + wet * feedback);
                *x = (*x + wet * mix) * normalise;
            }
            self.lfo.advance(rate);
        }
    }
}

impl Stage for Flanger {
    fn process(&mut self, k: &Knobs, buf: &mut [f32]) {
        self.run(buf, k[0], k[1], k[2], k[3] / 100.0, k[4] / 100.0, k[5] / 360.0);
    }
}

/// A sweeping notch filter: six all-pass filters whose frequency moves.
struct Phaser {
    /// One memory per filter and ear: the last input and the last output.
    state: [[[f32; 2]; 6]; 2],
    last: [f32; 2],
    lfo: Lfo,
}

impl Phaser {
    fn new() -> Self {
        Self {
            state: [[[0.0; 2]; 6]; 2],
            last: [0.0; 2],
            lfo: Lfo::default(),
        }
    }
}

impl Stage for Phaser {
    fn process(&mut self, k: &Knobs, buf: &mut [f32]) {
        let (rate, centre, depth, feedback, mix) = (k[0], k[1], k[2], k[3].min(0.9), k[4]);
        for frame in buf.chunks_exact_mut(2) {
            // Up to an octave either side of the centre.
            let freq = centre * (depth * self.lfo.bipolar(0.0)).exp2();
            let t = (std::f32::consts::PI * freq.clamp(20.0, SR * 0.45) / SR).tan();
            let a = (t - 1.0) / (t + 1.0);
            for (ch, x) in frame.iter_mut().enumerate() {
                let dry = *x;
                let mut v = dry + self.last[ch] * feedback;
                for s in &mut self.state[ch] {
                    let y = a * v + s[0] - a * s[1];
                    *s = [v, y];
                    v = y;
                }
                self.last[ch] = v;
                *x = blend(dry, (dry + v) * 0.5, mix);
            }
            self.lfo.advance(rate);
        }
    }
}

/// Repeats: what was heard comes back after `delay`, softer each time.
struct Echo {
    lines: [DelayLine; 2],
}

impl Echo {
    fn new() -> Self {
        Self {
            lines: [DelayLine::new(0.95), DelayLine::new(0.95)],
        }
    }

    fn run(&mut self, buf: &mut [f32], delay_ms: f32, feedback: f32, mix: f32) {
        let delay = delay_ms * SR / 1000.0;
        for frame in buf.chunks_exact_mut(2) {
            for (ch, x) in frame.iter_mut().enumerate() {
                let wet = self.lines[ch].read(delay);
                self.lines[ch].push(*x + wet * feedback);
                *x += wet * mix;
            }
        }
    }
}

impl Stage for Echo {
    fn process(&mut self, k: &Knobs, buf: &mut [f32]) {
        self.run(buf, k[0], k[1], k[2]);
    }
}

/// One short repeat, no more.
struct Slapback {
    lines: [DelayLine; 2],
}

impl Stage for Slapback {
    fn process(&mut self, k: &Knobs, buf: &mut [f32]) {
        let delay = k[0] * SR / 1000.0;
        for frame in buf.chunks_exact_mut(2) {
            for (ch, x) in frame.iter_mut().enumerate() {
                let wet = self.lines[ch].read(delay);
                self.lines[ch].push(*x);
                *x += wet * k[1];
            }
        }
    }
}

/// The level goes up and down.
#[derive(Default)]
struct Tremolo {
    lfo: Lfo,
}

impl Tremolo {
    fn run(&mut self, buf: &mut [f32], rate: f32, depth: f32) {
        for frame in buf.chunks_exact_mut(2) {
            // Starts at full level.
            let gain = 1.0 - depth + depth * self.lfo.unipolar(0.25);
            frame[0] *= gain;
            frame[1] *= gain;
            self.lfo.advance(rate);
        }
    }
}

impl Stage for Tremolo {
    fn process(&mut self, k: &Knobs, buf: &mut [f32]) {
        self.run(buf, k[0], k[1]);
    }
}

/// The sound moves from one speaker to the other: the ears are half a cycle apart.
#[derive(Default)]
struct AutoPan {
    lfo: Lfo,
}

impl Stage for AutoPan {
    fn process(&mut self, k: &Knobs, buf: &mut [f32]) {
        let amount = k[1];
        for frame in buf.chunks_exact_mut(2) {
            for (ch, x) in frame.iter_mut().enumerate() {
                let lfo = self.lfo.unipolar(ch as f32 * 0.5);
                *x *= 1.0 - amount + amount * lfo;
            }
            self.lfo.advance(k[0]);
        }
    }
}

/// Each ear hears the other one's sound a moment later and upside down, which pushes the picture
/// outward.
struct StereoWiden {
    lines: [DelayLine; 2],
}

impl Stage for StereoWiden {
    fn process(&mut self, k: &Knobs, buf: &mut [f32]) {
        let delay = k[0] * SR / 1000.0;
        let (feedback, crossfeed, dry) = (k[1], k[2], k[3]);
        for frame in buf.chunks_exact_mut(2) {
            let (l, r) = (frame[0], frame[1]);
            let dl = self.lines[0].read(delay);
            let dr = self.lines[1].read(delay);
            self.lines[0].push(l + dr * feedback);
            self.lines[1].push(r + dl * feedback);
            frame[0] = l * dry - dr * crossfeed;
            frame[1] = r * dry - dl * crossfeed;
        }
    }
}

// ---- texture ---------------------------------------------------------------------------------

/// Bitcrush and 8-Bit: few levels and few samples.
#[derive(Default)]
struct Bitcrush {
    crusher: Crusher,
    logarithmic: bool,
}

impl Stage for Bitcrush {
    fn process(&mut self, k: &Knobs, buf: &mut [f32]) {
        for frame in buf.chunks_exact_mut(2) {
            for (ch, x) in frame.iter_mut().enumerate() {
                *x = self
                    .crusher
                    .run(ch, *x, k[0], k[1] as u32, k[2], self.logarithmic);
            }
        }
    }
}

/// A sharpening step (what changed since the last sample is added back) and then a few levels.
#[derive(Default)]
struct Crystalizer {
    previous: [f32; 2],
}

impl Stage for Crystalizer {
    fn process(&mut self, k: &Knobs, buf: &mut [f32]) {
        let step = 2.0 / k[1].max(2.0);
        for frame in buf.chunks_exact_mut(2) {
            for (ch, x) in frame.iter_mut().enumerate() {
                let sharp = *x + (*x - self.previous[ch]) * k[0];
                self.previous[ch] = *x;
                *x = ((sharp / step).round() * step).clamp(-1.0, 1.0);
            }
        }
    }
}

/// A compressor that lets go slowly, and a saturation that rounds off the peaks.
#[derive(Default)]
struct Tape {
    envelope: f32,
}

impl Stage for Tape {
    fn process(&mut self, k: &Knobs, buf: &mut [f32]) {
        let (threshold, ratio) = (k[0], k[1]);
        let (attack, release) = (smoothing(k[2]), smoothing(k[3]));
        let drive = 1.0 + 4.0 * k[4];
        for frame in buf.chunks_exact_mut(2) {
            let level = to_db(frame[0].abs().max(frame[1].abs()));
            let target = if level > threshold {
                threshold + (level - threshold) / ratio - level
            } else {
                0.0
            };
            let c = if target < self.envelope { attack } else { release };
            self.envelope = target + c * (self.envelope - target);
            let gain = from_db(self.envelope);
            for x in frame.iter_mut() {
                // Small sounds pass as they are (the slope at zero is 1); loud ones round off.
                *x = (*x * gain * drive).tanh() / drive;
            }
        }
    }
}

/// An old record: the lows and highs are gone, the speed wanders and it crackles.
struct Vinyl {
    high_pass: Biquad,
    low_pass: Biquad,
    wobble: Wobble,
    noise: Noise,
    pop: f32,
}

impl Stage for Vinyl {
    fn process(&mut self, k: &Knobs, buf: &mut [f32]) {
        self.high_pass.tune(Shape::High, k[0], 0.707);
        self.low_pass.tune(Shape::Low, k[1], 0.707);
        self.wobble.run(buf, k[2], k[3]);
        // Up to about fifty pops a second.
        let chance = k[4] * 0.001;
        for frame in buf.chunks_exact_mut(2) {
            if (self.noise.next() * 0.5 + 0.5) < chance {
                self.pop = self.noise.next() * 0.25;
            }
            self.pop *= 0.92;
            for (ch, x) in frame.iter_mut().enumerate() {
                let y = self.low_pass.run(ch, self.high_pass.run(ch, *x));
                *x = y + self.pop;
            }
        }
    }
}

/// The pitch swings a little: a delay that moves around 2.5 ms.
struct Wobble {
    lines: [DelayLine; 2],
    lfo: Lfo,
}

impl Wobble {
    fn new() -> Self {
        Self {
            lines: [DelayLine::new(0.01), DelayLine::new(0.01)],
            lfo: Lfo::default(),
        }
    }

    fn run(&mut self, buf: &mut [f32], rate: f32, depth: f32) {
        let centre = 0.0025 * SR;
        for frame in buf.chunks_exact_mut(2) {
            let delay = (centre + centre * depth * self.lfo.bipolar(0.0)).max(1.0);
            for (ch, x) in frame.iter_mut().enumerate() {
                self.lines[ch].push(*x);
                *x = self.lines[ch].read(delay);
            }
            self.lfo.advance(rate);
        }
    }
}

impl Stage for Wobble {
    fn process(&mut self, k: &Knobs, buf: &mut [f32]) {
        self.run(buf, k[0], k[1]);
    }
}

// ---- transmission ----------------------------------------------------------------------------

/// A filter (or two) and a little crushing, which is most of what a device does to a voice.
struct Device {
    kind: AudioEffectKind,
    first: Biquad,
    second: Biquad,
    crusher: Crusher,
    chorus: Chorus,
}

impl Device {
    fn new(kind: AudioEffectKind) -> Self {
        Self {
            kind,
            first: Biquad::new(),
            second: Biquad::new(),
            crusher: Crusher::default(),
            chorus: Chorus::new(),
        }
    }
}

impl Stage for Device {
    fn process(&mut self, k: &Knobs, buf: &mut [f32]) {
        match self.kind {
            AudioEffectKind::Megaphone => {
                let q = (k[0] / k[1].max(1.0)).clamp(0.1, 20.0);
                self.first.tune(Shape::Band, k[0], q);
                for frame in buf.chunks_exact_mut(2) {
                    for (ch, x) in frame.iter_mut().enumerate() {
                        let y = self.first.run(ch, *x);
                        *x = self.crusher.run(ch, y, 8.0, 1, k[2], false);
                    }
                }
            }
            AudioEffectKind::Muffled => {
                self.first.tune(Shape::Low, k[0], 0.707);
                for frame in buf.chunks_exact_mut(2) {
                    for (ch, x) in frame.iter_mut().enumerate() {
                        *x = self.first.run(ch, *x) * k[1];
                    }
                }
            }
            AudioEffectKind::Telephone => {
                self.first.tune(Shape::High, k[0], 0.707);
                self.second.tune(Shape::Low, k[1], 0.707);
                for frame in buf.chunks_exact_mut(2) {
                    for (ch, x) in frame.iter_mut().enumerate() {
                        *x = self.second.run(ch, self.first.run(ch, *x));
                    }
                }
            }
            AudioEffectKind::WalkieTalkie => {
                self.first.tune(Shape::High, k[1], 0.707);
                self.second.tune(Shape::Low, k[2], 0.707);
                for frame in buf.chunks_exact_mut(2) {
                    for (ch, x) in frame.iter_mut().enumerate() {
                        let y = self.second.run(ch, self.first.run(ch, *x));
                        *x = self.crusher.run(ch, y, 6.0, 1, k[0], false);
                    }
                }
            }
            // Underwater: the highs are gone and the rest sways.
            _ => {
                self.first.tune(Shape::Low, k[0], 0.707);
                for frame in buf.chunks_exact_mut(2) {
                    for (ch, x) in frame.iter_mut().enumerate() {
                        *x = self.first.run(ch, *x);
                    }
                }
                self.chorus.run(buf, 50.0, k[2], 2.0, k[1]);
            }
        }
    }
}

// ---- utility ---------------------------------------------------------------------------------

/// Quiet sound is turned down further: below the threshold, the level falls `ratio - 1` decibels
/// for each decibel it is under.
#[derive(Default)]
struct Gate {
    envelope: f32,
}

impl Stage for Gate {
    fn process(&mut self, k: &Knobs, buf: &mut [f32]) {
        let (threshold, ratio) = (k[0], k[1]);
        let (attack, release) = (smoothing(k[2]), smoothing(k[3]));
        for frame in buf.chunks_exact_mut(2) {
            let level = to_db(frame[0].abs().max(frame[1].abs()));
            let target = if level < threshold {
                ((level - threshold) * (ratio - 1.0)).max(-80.0)
            } else {
                0.0
            };
            // Closing is slow (release), opening is fast (attack).
            let c = if target < self.envelope { release } else { attack };
            self.envelope = target + c * (self.envelope - target);
            let gain = from_db(self.envelope);
            frame[0] *= gain;
            frame[1] *= gain;
        }
    }
}

/// Turns down the "s" sounds: the band above `frequency` is followed, and taken out of the
/// sound by as much as it is loud. The band is what a one-pole low-pass leaves of the sound, so
/// taking all of it out leaves exactly that low-pass.
struct DeEsser {
    low: [f32; 2],
    envelope: [f32; 2],
}

impl Stage for DeEsser {
    fn process(&mut self, k: &Knobs, buf: &mut [f32]) {
        let (intensity, max_cut) = (k[0], k[1]);
        let g = 1.0 - (-TAU * k[2].clamp(20.0, SR * 0.45) / SR).exp();
        // Fast enough to catch an "s", slow enough not to chew the vowel after it.
        let (attack, release) = (smoothing(1.0), smoothing(60.0));
        for frame in buf.chunks_exact_mut(2) {
            for (ch, x) in frame.iter_mut().enumerate() {
                self.low[ch] += g * (*x - self.low[ch]);
                let band = *x - self.low[ch];
                let magnitude = band.abs();
                let c = if magnitude > self.envelope[ch] { attack } else { release };
                self.envelope[ch] = c * self.envelope[ch] + (1.0 - c) * magnitude;
                let cut = (intensity * self.envelope[ch] * 4.0).clamp(0.0, max_cut);
                *x -= band * cut;
            }
        }
    }
}

/// Brings a quiet voice up to a steady level, slowly, so the level walks rather than jumps.
struct Leveler {
    envelope: f32,
    gain: f32,
}

impl Stage for Leveler {
    fn process(&mut self, k: &Knobs, buf: &mut [f32]) {
        let (strength, peak) = (k[0], k[1]);
        let release = smoothing(400.0);
        let scale = 48_000.0 / SR;
        let (raise, fall) = (0.0005 * scale, 0.001 * scale);
        for frame in buf.chunks_exact_mut(2) {
            let magnitude = frame[0].abs().max(frame[1].abs());
            self.envelope = if magnitude > self.envelope {
                magnitude
            } else {
                release * self.envelope + (1.0 - release) * magnitude
            };
            let target = if self.envelope > 1e-4 {
                (peak / self.envelope).clamp(1.0, strength)
            } else {
                1.0
            };
            let step = target - self.gain;
            self.gain += step.clamp(-fall, raise);
            frame[0] *= self.gain;
            frame[1] *= self.gain;
        }
    }
}

/// Nothing gets over the ceiling: the gain drops at once and comes back over 50 ms. Both ears
/// share it, so the picture does not shift when one side peaks.
struct Limiter {
    gain: f32,
}

impl Stage for Limiter {
    fn process(&mut self, k: &Knobs, buf: &mut [f32]) {
        let drive = from_db(k[0]);
        let ceiling = from_db(k[1]).clamp(1e-4, 1.0);
        let release = 1.0 - smoothing(50.0);
        for frame in buf.chunks_exact_mut(2) {
            frame[0] *= drive;
            frame[1] *= drive;
            let peak = frame[0].abs().max(frame[1].abs());
            let wanted = if peak > ceiling { ceiling / peak } else { 1.0 };
            if wanted < self.gain {
                self.gain = wanted;
            } else {
                self.gain += (wanted - self.gain) * release;
            }
            frame[0] *= self.gain;
            frame[1] *= self.gain;
        }
    }
}

// ---- voice -----------------------------------------------------------------------------------

/// Changes the pitch without changing the speed: two taps of a delay that sweeps, crossfaded so
/// the jump of one is covered by the other.
struct PitchShift {
    lines: [DelayLine; 2],
    phase: f32,
}

/// How long a piece of sound one tap plays before it jumps back, in seconds.
const PITCH_WINDOW: f32 = 0.04;

impl PitchShift {
    fn new() -> Self {
        Self {
            lines: [DelayLine::new(PITCH_WINDOW + 0.01), DelayLine::new(PITCH_WINDOW + 0.01)],
            phase: 0.0,
        }
    }

    fn run(&mut self, buf: &mut [f32], ratio: f32) {
        let window = PITCH_WINDOW * SR;
        for frame in buf.chunks_exact_mut(2) {
            // The delay of a tap changes by `1 - ratio` samples a sample, so it reads the sound at
            // `ratio` times the speed.
            self.phase = (self.phase + (1.0 - ratio) / window).rem_euclid(1.0);
            let p1 = self.phase;
            let p2 = (p1 + 0.5).fract();
            let g1 = 0.5 - 0.5 * (TAU * p1).cos();
            let g2 = 0.5 - 0.5 * (TAU * p2).cos();
            for (ch, x) in frame.iter_mut().enumerate() {
                self.lines[ch].push(*x);
                *x = self.lines[ch].read(p1 * window + 1.0) * g1
                    + self.lines[ch].read(p2 * window + 1.0) * g2;
            }
        }
    }
}

/// Chipmunk and Deep Voice: only the pitch.
struct Pitch(PitchShift);

impl Stage for Pitch {
    fn process(&mut self, k: &Knobs, buf: &mut [f32]) {
        self.0.run(buf, k[0]);
    }
}

/// Lower, with an echo and a little crushing.
struct DarkLord {
    pitch: PitchShift,
    echo: Echo,
    crusher: Crusher,
}

impl Stage for DarkLord {
    fn process(&mut self, k: &Knobs, buf: &mut [f32]) {
        self.pitch.run(buf, k[0]);
        self.echo.run(buf, k[1], k[2] * 0.6, 0.8 * k[2] + 0.2);
        for frame in buf.chunks_exact_mut(2) {
            for (ch, x) in frame.iter_mut().enumerate() {
                *x = self.crusher.run(ch, *x, 10.0, 1, k[3], false);
            }
        }
    }
}

/// A flanger with everything wet.
struct Alien {
    flanger: Flanger,
}

impl Stage for Alien {
    fn process(&mut self, k: &Knobs, buf: &mut [f32]) {
        self.flanger
            .run(buf, k[0], k[1], k[2], k[3] / 100.0, 1.0, 0.25);
    }
}

/// A fast tremolo is a robot.
#[derive(Default)]
struct Robot(Tremolo);

impl Stage for Robot {
    fn process(&mut self, k: &Knobs, buf: &mut [f32]) {
        self.0.run(buf, k[0], k[1]);
    }
}

// ---- the chain -------------------------------------------------------------------------------

fn build(kind: AudioEffectKind) -> Box<dyn Stage> {
    use AudioEffectKind as K;
    match kind {
        K::Reverb => Box::new(Reverb::new()),
        K::Chorus => Box::new(Chorus::new()),
        K::Flanger => Box::new(Flanger::new()),
        K::Phaser => Box::new(Phaser::new()),
        K::Echo => Box::new(Echo::new()),
        K::Slapback => Box::new(Slapback {
            lines: [DelayLine::new(0.13), DelayLine::new(0.13)],
        }),
        K::Tremolo => Box::<Tremolo>::default(),
        K::AutoPan => Box::<AutoPan>::default(),
        K::StereoWiden => Box::new(StereoWiden {
            lines: [DelayLine::new(0.12), DelayLine::new(0.12)],
        }),
        K::Bitcrush => Box::<Bitcrush>::default(),
        K::EightBit => Box::new(Bitcrush {
            logarithmic: true,
            ..Default::default()
        }),
        K::Crystalizer => Box::<Crystalizer>::default(),
        K::TapeSaturation => Box::<Tape>::default(),
        K::Vinyl => Box::new(Vinyl {
            high_pass: Biquad::new(),
            low_pass: Biquad::new(),
            wobble: Wobble::new(),
            noise: Noise(0x9E37_79B9),
            pop: 0.0,
        }),
        K::Megaphone | K::Muffled | K::Telephone | K::Underwater | K::WalkieTalkie => {
            Box::new(Device::new(kind))
        }
        K::Gate => Box::<Gate>::default(),
        K::DeEsser => Box::new(DeEsser {
            low: [0.0; 2],
            envelope: [0.0; 2],
        }),
        K::Leveler => Box::new(Leveler {
            envelope: 0.0,
            gain: 1.0,
        }),
        K::Limiter => Box::new(Limiter { gain: 1.0 }),
        K::Alien => Box::new(Alien {
            flanger: Flanger::new(),
        }),
        K::Chipmunk | K::DeepVoice => Box::new(Pitch(PitchShift::new())),
        K::Robot => Box::<Robot>::default(),
        K::DarkLord => Box::new(DarkLord {
            pitch: PitchShift::new(),
            echo: Echo::new(),
            crusher: Crusher::default(),
        }),
        K::Wobble => Box::new(Wobble::new()),
    }
}

/// The memory of one track's sound effects between blocks.
#[derive(Default)]
pub struct EffectChain {
    stages: Vec<(AudioEffectKind, Box<dyn Stage>)>,
}

impl Clone for EffectChain {
    /// A copy starts with nothing remembered: it is only ever a copy of a mixer to start over.
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl fmt::Debug for EffectChain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list()
            .entries(self.stages.iter().map(|(kind, _)| kind))
            .finish()
    }
}

impl EffectChain {
    /// Runs `buf` (interleaved stereo) through `chain`, in order. The effects that are off are
    /// skipped. A stage is rebuilt (and starts silent) only when a different effect takes its place.
    pub fn process(&mut self, chain: &[AudioEffect], buf: &mut [f32]) {
        self.stages.truncate(chain.len());
        for (i, fx) in chain.iter().enumerate() {
            if self.stages.get(i).map(|(kind, _)| *kind) != Some(fx.kind) {
                self.stages.truncate(i);
                self.stages.push((fx.kind, build(fx.kind)));
            }
        }
        for (fx, (_, stage)) in chain.iter().zip(&mut self.stages) {
            if !fx.enabled {
                continue;
            }
            stage.process(&fx.sanitized().params, buf);
            // Nothing may run away or leave a broken number behind.
            for x in buf.iter_mut() {
                *x = if x.is_finite() { x.clamp(-LIMIT, LIMIT) } else { 0.0 };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(freq: f32, amp: f32, seconds: f32) -> Vec<f32> {
        let n = (seconds * SR) as usize;
        (0..n)
            .flat_map(|i| {
                let v = amp * (TAU * freq * i as f32 / SR).sin();
                [v, v]
            })
            .collect()
    }

    fn run_with(kind: AudioEffectKind, edit: impl Fn(&mut [f32; MAX_PARAMS]), input: &[f32]) -> Vec<f32> {
        let mut fx = AudioEffect::new(kind);
        edit(&mut fx.params);
        let mut out = input.to_vec();
        // In blocks, as the mixer does.
        let mut chain = EffectChain::default();
        for block in out.chunks_mut(960) {
            chain.process(&[fx], block);
        }
        out
    }

    fn run(kind: AudioEffectKind, input: &[f32]) -> Vec<f32> {
        run_with(kind, |_| {}, input)
    }

    /// The loudest sample of the left ear from `from` seconds on.
    fn peak_after(buf: &[f32], from: f32) -> f32 {
        let start = (from * SR) as usize * 2;
        buf[start..]
            .iter()
            .step_by(2)
            .fold(0.0f32, |m, v| m.max(v.abs()))
    }

    fn energy(buf: &[f32]) -> f32 {
        buf.iter().map(|v| v * v).sum::<f32>()
    }

    /// How much of the left ear's sound from `from` seconds on is a tone of `freq` hertz (the
    /// Goertzel sum), as a share of its energy.
    fn power_at(buf: &[f32], freq: f32, from: f32) -> f32 {
        let left: Vec<f32> = buf.iter().step_by(2).copied().collect();
        let start = (from * SR) as usize;
        let (mut re, mut im) = (0.0f64, 0.0f64);
        for (n, v) in left[start..].iter().enumerate() {
            let a = std::f64::consts::TAU * f64::from(freq) * (n as f64) / f64::from(SR);
            re += f64::from(*v) * a.cos();
            im += f64::from(*v) * a.sin();
        }
        let n = (left.len() - start) as f64;
        ((re * re + im * im) / (n * n)) as f32
    }

    #[test]
    fn every_effect_runs_on_a_voice_and_on_extreme_knobs_without_breaking() {
        let mut voice = tone(220.0, 0.4, 1.5);
        for (a, b) in voice.iter_mut().zip(tone(3300.0, 0.2, 1.5)) {
            *a += b;
        }
        for kind in AudioEffectKind::ALL {
            for pick in 0..3 {
                let out = run_with(
                    kind,
                    |p| {
                        for (slot, spec) in p.iter_mut().zip(kind.params()) {
                            *slot = match pick {
                                0 => spec.default,
                                1 => spec.min,
                                _ => spec.max,
                            };
                        }
                    },
                    &voice,
                );
                assert_eq!(out.len(), voice.len());
                assert!(out.iter().all(|v| v.is_finite()), "{kind:?} {pick}");
                assert!(peak_after(&out, 0.0) <= LIMIT, "{kind:?} {pick}");
            }
            // With its default knobs it changes the sound (but for the ones that start quiet).
            let out = run(kind, &voice);
            if !matches!(kind, AudioEffectKind::Gate | AudioEffectKind::Leveler | AudioEffectKind::Limiter) {
                assert!(
                    out.iter().zip(&voice).any(|(a, b)| (a - b).abs() > 1e-3),
                    "{kind:?} leaves the sound as it was"
                );
            }
        }
    }

    #[test]
    fn effects_that_are_off_or_not_there_leave_the_sound_alone() {
        let input = tone(440.0, 0.5, 0.2);
        let mut off = AudioEffect::new(AudioEffectKind::Reverb);
        off.enabled = false;
        let mut out = input.clone();
        EffectChain::default().process(&[off], &mut out);
        assert_eq!(out, input);
        let mut out = input.clone();
        EffectChain::default().process(&[], &mut out);
        assert_eq!(out, input);
    }

    #[test]
    fn a_stage_is_rebuilt_only_when_another_effect_takes_its_place() {
        let mut chain = EffectChain::default();
        let echo = AudioEffect::new(AudioEffectKind::Echo);
        // The default echo repeats after 250 ms, which is 12000 frames: the first frame of the
        // next block.
        let mut impulse = vec![0.0f32; 24_000];
        impulse[0] = 1.0;
        chain.process(&[echo], &mut impulse);
        let mut next = vec![0.0f32; 24_000];
        chain.process(&[echo, AudioEffect::new(AudioEffectKind::Tremolo)], &mut next);
        assert!(next[0] > 0.3, "{}", next[0]);
        // A different effect at the same place starts over: nothing is left of the echo.
        let mut other = vec![0.0f32; 24_000];
        chain.process(&[AudioEffect::new(AudioEffectKind::Muffled)], &mut other);
        assert!(other.iter().all(|v| v.abs() < 1e-6));
    }

    #[test]
    fn echo_repeats_after_its_delay_and_slapback_once() {
        let mut impulse = vec![0.0f32; 2 * 48_000];
        impulse[0] = 1.0;
        let out = run_with(
            AudioEffectKind::Echo,
            |p| *p = [100.0, 0.5, 1.0, 0.0, 0.0, 0.0],
            &impulse,
        );
        // 100 ms is 4800 samples; each repeat is half the last.
        assert!((out[2 * 4800] - 1.0).abs() < 1e-4, "{}", out[2 * 4800]);
        assert!((out[2 * 9600] - 0.5).abs() < 1e-4);
        assert!((out[2 * 14_400] - 0.25).abs() < 1e-4);
        let once = run_with(
            AudioEffectKind::Slapback,
            |p| *p = [50.0, 0.5, 0.0, 0.0, 0.0, 0.0],
            &impulse,
        );
        assert!((once[2 * 2400] - 0.5).abs() < 1e-4);
        assert!(once[2 * 2401..].iter().all(|v| v.abs() < 1e-4));
    }

    #[test]
    fn tremolo_swings_the_level_and_auto_pan_swaps_the_ears() {
        let out = run_with(
            AudioEffectKind::Tremolo,
            |p| *p = [5.0, 1.0, 0.0, 0.0, 0.0, 0.0],
            &tone(1000.0, 0.5, 1.0),
        );
        // The level (over 10 ms) goes from full to nearly nothing and back, five times a second.
        let levels: Vec<f32> = out
            .chunks(960)
            .map(|w| (energy(w) / w.len() as f32).sqrt())
            .collect();
        let (loud, quiet) = levels
            .iter()
            .fold((0.0f32, f32::MAX), |(hi, lo), v| (hi.max(*v), lo.min(*v)));
        assert!(loud > 0.3 && quiet < loud * 0.1, "{loud} {quiet}");
        // A steady sound: when the left is full the right is gone.
        let steady = vec![0.5f32; 2 * 48_000];
        let pan = run_with(AudioEffectKind::AutoPan, |p| *p = [0.5, 1.0, 0.0, 0.0, 0.0, 0.0], &steady);
        let (i, left) = pan
            .iter()
            .step_by(2)
            .enumerate()
            .fold((0, 0.0f32), |m, (i, v)| if *v > m.1 { (i, *v) } else { m });
        assert!(left > 0.49);
        assert!(pan[2 * i + 1] < 0.02, "{}", pan[2 * i + 1]);
    }

    #[test]
    fn filters_keep_what_they_pass_and_take_the_rest() {
        let gain = |kind, freq| {
            let out = run(kind, &tone(freq, 0.5, 0.5));
            peak_after(&out, 0.25) / 0.5
        };
        // Muffled opens up to 800 Hz.
        assert!(gain(AudioEffectKind::Muffled, 100.0) > 0.9);
        assert!(gain(AudioEffectKind::Muffled, 8000.0) < 0.05);
        // A telephone passes 300 to 3400 Hz.
        assert!(gain(AudioEffectKind::Telephone, 1000.0) > 0.9);
        assert!(gain(AudioEffectKind::Telephone, 60.0) < 0.15);
        assert!(gain(AudioEffectKind::Telephone, 12_000.0) < 0.15);
        // A megaphone is a band around 1.5 kHz (and a little grit).
        assert!(gain(AudioEffectKind::Megaphone, 1500.0) > 0.5);
        assert!(gain(AudioEffectKind::Megaphone, 100.0) < 0.1);
        // A walkie-talkie has its own band.
        assert!(gain(AudioEffectKind::WalkieTalkie, 1000.0) > 0.4);
        assert!(gain(AudioEffectKind::WalkieTalkie, 80.0) < 0.1);
    }

    #[test]
    fn bitcrush_quantises_and_holds() {
        let ramp: Vec<f32> = (0..2000).flat_map(|i| {
            let v = i as f32 / 1000.0 - 1.0;
            [v, v]
        }).collect();
        let out = run_with(AudioEffectKind::Bitcrush, |p| *p = [3.0, 1.0, 1.0, 0.0, 0.0, 0.0], &ramp);
        // Three bits: steps of a quarter, nine values at most.
        let mut values: Vec<i32> = out.iter().map(|v| (v * 1000.0).round() as i32).collect();
        values.sort_unstable();
        values.dedup();
        assert!(values.len() <= 9, "{values:?}");
        // Held for four samples at a time.
        let held = run_with(AudioEffectKind::Bitcrush, |p| *p = [16.0, 4.0, 1.0, 0.0, 0.0, 0.0], &ramp);
        for group in held.chunks(8) {
            assert!(group.iter().step_by(2).all(|v| (*v - group[0]).abs() < 1e-6));
        }
        // 8-Bit crushes quiet sounds as hard as loud ones.
        let source = tone(300.0, 0.01, 0.1);
        let quiet = run_with(AudioEffectKind::EightBit, |p| *p = [2.0, 1.0, 1.0, 0.0, 0.0, 0.0], &source);
        for (a, b) in source.iter().zip(&quiet).filter(|(a, _)| a.abs() > 1e-5) {
            // Every sample lands on the grid of eighths of an octave, within 4.5% of the source.
            let octaves = b.abs().log2() * 8.0;
            assert!((octaves - octaves.round()).abs() < 1e-3, "{b}");
            assert!((b / a - 1.0).abs() < 0.05, "{a} {b}");
        }
        // Crystalizer: only a few levels remain.
        let crystal = run_with(AudioEffectKind::Crystalizer, |p| *p = [0.0, 4.0, 0.0, 0.0, 0.0, 0.0], &tone(200.0, 0.9, 0.1));
        assert!(crystal.iter().all(|v| ((v / 0.5).round() * 0.5 - v).abs() < 1e-5));
    }

    #[test]
    fn the_limiter_never_goes_over_its_ceiling() {
        let loud = tone(300.0, 2.5, 0.5);
        let out = run_with(AudioEffectKind::Limiter, |p| *p = [0.0, -6.0, 0.0, 0.0, 0.0, 0.0], &loud);
        let ceiling = 10f32.powf(-6.0 / 20.0);
        assert!(peak_after(&out, 0.0) <= ceiling + 1e-4, "{}", peak_after(&out, 0.0));
        // And it does not touch what is under it.
        let quiet = tone(300.0, 0.2, 0.2);
        let same = run_with(AudioEffectKind::Limiter, |p| *p = [0.0, -0.5, 0.0, 0.0, 0.0, 0.0], &quiet);
        assert!(quiet.iter().zip(&same).all(|(a, b)| (a - b).abs() < 1e-5));
    }

    #[test]
    fn the_gate_shuts_on_quiet_sound_and_opens_for_loud() {
        let quiet = tone(300.0, 0.001, 1.0);
        let out = run(AudioEffectKind::Gate, &quiet);
        assert!(peak_after(&out, 0.5) < 0.001 * 0.1, "{}", peak_after(&out, 0.5));
        let loud = tone(300.0, 0.5, 0.5);
        let out = run(AudioEffectKind::Gate, &loud);
        assert!((peak_after(&out, 0.25) - 0.5).abs() < 0.03);
    }

    #[test]
    fn the_leveler_brings_a_quiet_voice_up() {
        let out = run_with(
            AudioEffectKind::Leveler,
            |p| *p = [20.0, 0.9, 0.0, 0.0, 0.0, 0.0],
            &tone(250.0, 0.1, 4.0),
        );
        let after = peak_after(&out, 3.0);
        assert!(after > 0.5 && after <= 1.0, "{after}");
    }

    #[test]
    fn the_de_esser_takes_out_the_hiss_and_leaves_the_voice() {
        let knobs = |p: &mut [f32; MAX_PARAMS]| *p = [1.0, 1.0, 5000.0, 0.0, 0.0, 0.0];
        let hiss = run_with(AudioEffectKind::DeEsser, knobs, &tone(9000.0, 0.5, 0.5));
        assert!(peak_after(&hiss, 0.25) < 0.3, "{}", peak_after(&hiss, 0.25));
        let voice = run_with(AudioEffectKind::DeEsser, knobs, &tone(200.0, 0.5, 0.5));
        assert!(peak_after(&voice, 0.25) > 0.48);
    }

    #[test]
    fn tape_rounds_off_peaks_and_leaves_small_sound_alone() {
        let loud = run(AudioEffectKind::TapeSaturation, &tone(200.0, 0.9, 0.5));
        assert!(peak_after(&loud, 0.25) < 0.7, "{}", peak_after(&loud, 0.25));
        let small = tone(200.0, 0.01, 0.2);
        let out = run(AudioEffectKind::TapeSaturation, &small);
        assert!((peak_after(&out, 0.1) - 0.01).abs() < 0.001);
    }

    #[test]
    fn voice_changers_move_the_pitch_by_their_ratio() {
        let input = tone(440.0, 0.5, 1.5);
        let up = run_with(AudioEffectKind::Chipmunk, |p| p[0] = 1.5, &input);
        let (new, old) = (power_at(&up, 660.0, 0.5), power_at(&up, 440.0, 0.5));
        assert!(new > old * 3.0, "{new} {old}");
        let down = run_with(AudioEffectKind::DeepVoice, |p| p[0] = 0.7, &input);
        let (new, old) = (power_at(&down, 308.0, 0.5), power_at(&down, 440.0, 0.5));
        assert!(new > old * 3.0, "{new} {old}");
        // The speed is not changed: it is as long and as loud.
        assert_eq!(up.len(), input.len());
        assert!(peak_after(&up, 0.5) > 0.3);
    }

    #[test]
    fn a_reverb_has_a_tail_and_none_when_it_is_dry() {
        let mut impulse = vec![0.0f32; 2 * 48_000];
        impulse[0] = 1.0;
        impulse[1] = 1.0;
        let wet = run_with(AudioEffectKind::Reverb, |p| *p = [0.8, 0.3, 1.0, 1.0, 0.0, 0.0], &impulse);
        // Half a second on there is still a tail.
        assert!(energy(&wet[2 * 24_000..2 * 30_000]) > 1e-6);
        let dry = run_with(AudioEffectKind::Reverb, |p| *p = [0.8, 0.3, 1.0, 0.0, 0.0, 0.0], &impulse);
        assert_eq!(dry, impulse);
        // A bigger room rings longer.
        let small = run_with(AudioEffectKind::Reverb, |p| *p = [0.0, 0.5, 1.0, 1.0, 0.0, 0.0], &impulse);
        let big = run_with(AudioEffectKind::Reverb, |p| *p = [1.0, 0.5, 1.0, 1.0, 0.0, 0.0], &impulse);
        assert!(energy(&big[2 * 40_000..]) > energy(&small[2 * 40_000..]));
    }

    #[test]
    fn stereo_widen_puts_sound_in_the_ear_that_had_none() {
        let mut left_only = tone(500.0, 0.5, 0.5);
        for frame in left_only.chunks_exact_mut(2) {
            frame[1] = 0.0;
        }
        let out = run(AudioEffectKind::StereoWiden, &left_only);
        let right: Vec<f32> = out.iter().skip(1).step_by(2).copied().collect();
        assert!(right.iter().map(|v| v * v).sum::<f32>() > 1.0);
    }

    #[test]
    fn modulation_effects_are_not_the_dry_sound_but_keep_its_level() {
        let input = tone(600.0, 0.4, 1.0);
        for kind in [
            AudioEffectKind::Chorus,
            AudioEffectKind::Flanger,
            AudioEffectKind::Phaser,
            AudioEffectKind::Wobble,
            AudioEffectKind::Alien,
            AudioEffectKind::Underwater,
        ] {
            let out = run(kind, &input);
            let (a, b) = (energy(&input), energy(&out));
            assert!(b > a * 0.05 && b < a * 6.0, "{kind:?} {a} {b}");
        }
    }

    #[test]
    fn a_vinyl_crackles_when_it_is_told_to_and_not_otherwise() {
        let silence = vec![0.0f32; 2 * 48_000];
        let crackling = run_with(AudioEffectKind::Vinyl, |p| p[4] = 1.0, &silence);
        assert!(peak_after(&crackling, 0.0) > 0.02);
        let clean = run_with(AudioEffectKind::Vinyl, |p| p[4] = 0.0, &silence);
        assert!(clean.iter().all(|v| *v == 0.0));
    }

    #[test]
    fn dark_lord_is_lower_and_echoes() {
        let input = tone(440.0, 0.5, 1.5);
        let out = run(AudioEffectKind::DarkLord, &input);
        let (new, old) = (power_at(&out, 352.0, 0.5), power_at(&out, 440.0, 0.5));
        assert!(new > old * 3.0, "{new} {old}");
    }
}
