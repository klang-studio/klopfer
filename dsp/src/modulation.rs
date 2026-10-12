//! Modulation: LFO, sources, the per-pad matrix and how matrix targets are scaled.

use std::f32::consts::TAU;

pub const NUM_SLOTS: usize = 8;

/// Full-scale ranges of the matrix targets at amount 1.0 and a source value of 1.0
pub const PITCH_RANGE_SEMIS: f32 = 48.0;
pub const CUTOFF_RANGE_OCT: f32 = 8.0;
pub const LFO_RATE_RANGE_OCT: f32 = 4.0;
pub const TIME_RANGE_OCT: f32 = 4.0;
pub const BITS_RANGE: f32 = 22.0;
pub const DOWNSAMPLE_RANGE_OCT: f32 = 5.0;

/// Small, fast PRNG (xorshift32) for noise, S&H and per-note random values.
#[derive(Debug, Clone, Copy)]
pub struct Rng(u32);

impl Default for Rng {
    fn default() -> Self {
        Self(0x9e37_79b9)
    }
}

impl Rng {
    pub fn new(seed: u32) -> Self {
        Self(seed.max(1))
    }

    #[inline]
    fn next_u32(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0
    }

    /// Uniform in `[0, 1)`
    #[inline]
    pub fn unipolar(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 / (1u32 << 24) as f32
    }

    /// Uniform in `[-1, 1)`
    #[inline]
    pub fn bipolar(&mut self) -> f32 {
        self.unipolar() * 2.0 - 1.0
    }

    /// Uniform in `0..n`, `n > 0`
    #[inline]
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u32() as usize) % n
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LfoShape {
    #[default]
    Sine,
    Triangle,
    SawUp,
    SawDown,
    Square,
    SampleHold,
}

/// A bipolar LFO, phase in `[0, 1)`.
#[derive(Debug, Clone, Copy, Default)]
pub struct Lfo {
    phase: f32,
    held: f32,
}

impl Lfo {
    pub fn reset(&mut self, phase: f32, rng: &mut Rng) {
        self.phase = phase.rem_euclid(1.0);
        self.held = rng.bipolar();
    }

    /// The value at the current phase without advancing.
    #[inline]
    pub fn value(&self, shape: LfoShape) -> f32 {
        let p = self.phase;
        match shape {
            LfoShape::Sine => (TAU * p).sin(),
            LfoShape::Triangle => 1.0 - 4.0 * (p - 0.5).abs(),
            LfoShape::SawUp => 2.0 * p - 1.0,
            LfoShape::SawDown => 1.0 - 2.0 * p,
            LfoShape::Square => {
                if p < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            LfoShape::SampleHold => self.held,
        }
    }

    /// Returns the current value and advances by one sample.
    #[inline]
    pub fn next(&mut self, shape: LfoShape, freq_hz: f32, sample_rate: f32, rng: &mut Rng) -> f32 {
        let out = self.value(shape);
        self.phase += freq_hz / sample_rate;
        if self.phase >= 1.0 {
            self.phase = self.phase.fract();
            self.held = rng.bipolar();
        }
        out
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ModSource {
    #[default]
    Off,
    Lfo,
    ModEnv1,
    ModEnv2,
    AmpEnv,
    Velocity,
    /// A random value per note
    Random,
    /// White noise, changes every sample
    Noise,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ModTarget {
    #[default]
    Off,
    Pitch,
    Cutoff,
    Resonance,
    Drive,
    Pan,
    Volume,
    /// Evaluated once at note on
    SampleStart,
    SampleEnd,
    LfoRate,
    Bits,
    Downsample,
    /// Evaluated once at note on
    AmpAttack,
    /// Evaluated once at note on
    AmpDecay,
}

impl ModTarget {
    pub fn is_note_on_only(self) -> bool {
        matches!(
            self,
            ModTarget::SampleStart | ModTarget::AmpAttack | ModTarget::AmpDecay
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ModSlot {
    pub source: ModSource,
    pub target: ModTarget,
    /// `-1..1`
    pub amount: f32,
}

impl ModSlot {
    pub fn is_active(&self) -> bool {
        self.source != ModSource::Off && self.target != ModTarget::Off && self.amount != 0.0
    }
}

/// Current source values for one voice. Envelopes, velocity and random are unipolar, LFO and noise
/// bipolar.
#[derive(Debug, Clone, Copy, Default)]
pub struct Sources {
    pub lfo: f32,
    pub env1: f32,
    pub env2: f32,
    pub amp: f32,
    pub velocity: f32,
    pub random: f32,
    pub noise: f32,
}

impl Sources {
    #[inline]
    fn get(&self, source: ModSource) -> f32 {
        match source {
            ModSource::Off => 0.0,
            ModSource::Lfo => self.lfo,
            ModSource::ModEnv1 => self.env1,
            ModSource::ModEnv2 => self.env2,
            ModSource::AmpEnv => self.amp,
            ModSource::Velocity => self.velocity,
            ModSource::Random => self.random,
            ModSource::Noise => self.noise,
        }
    }
}

/// Summed `source * amount` per target, in normalized units (`-1..1` per slot).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ModValues {
    pub pitch: f32,
    pub cutoff: f32,
    pub resonance: f32,
    pub drive: f32,
    pub pan: f32,
    pub volume: f32,
    pub start: f32,
    pub end: f32,
    pub lfo_rate: f32,
    pub bits: f32,
    pub downsample: f32,
    pub amp_attack: f32,
    pub amp_decay: f32,
}

impl ModValues {
    #[inline]
    pub fn evaluate(slots: &[ModSlot], sources: &Sources) -> Self {
        let mut m = Self::default();
        for slot in slots {
            let v = sources.get(slot.source) * slot.amount;
            let field = match slot.target {
                ModTarget::Off => continue,
                ModTarget::Pitch => &mut m.pitch,
                ModTarget::Cutoff => &mut m.cutoff,
                ModTarget::Resonance => &mut m.resonance,
                ModTarget::Drive => &mut m.drive,
                ModTarget::Pan => &mut m.pan,
                ModTarget::Volume => &mut m.volume,
                ModTarget::SampleStart => &mut m.start,
                ModTarget::SampleEnd => &mut m.end,
                ModTarget::LfoRate => &mut m.lfo_rate,
                ModTarget::Bits => &mut m.bits,
                ModTarget::Downsample => &mut m.downsample,
                ModTarget::AmpAttack => &mut m.amp_attack,
                ModTarget::AmpDecay => &mut m.amp_decay,
            };
            *field += v;
        }
        m
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lfo_shapes_cover_full_range() {
        let mut rng = Rng::default();
        for shape in [
            LfoShape::Sine,
            LfoShape::Triangle,
            LfoShape::SawUp,
            LfoShape::SawDown,
            LfoShape::Square,
        ] {
            let mut lfo = Lfo::default();
            lfo.reset(0.0, &mut rng);
            let values: Vec<f32> = (0..1000)
                .map(|_| lfo.next(shape, 1.0, 1000.0, &mut rng))
                .collect();
            let (lo, hi) = values
                .iter()
                .fold((f32::MAX, f32::MIN), |(l, h), v| (l.min(*v), h.max(*v)));
            assert!(lo < -0.95 && hi > 0.95, "{shape:?}: {lo}..{hi}");
        }
    }

    #[test]
    fn sample_hold_changes_once_per_cycle() {
        let mut rng = Rng::new(42);
        let mut lfo = Lfo::default();
        lfo.reset(0.0, &mut rng);
        let values: Vec<f32> = (0..20)
            .map(|_| lfo.next(LfoShape::SampleHold, 100.0, 1000.0, &mut rng))
            .collect();
        assert!(values[..10].iter().all(|v| *v == values[0]));
        assert_ne!(values[0], values[10]);
    }

    #[test]
    fn matrix_sums_slots_per_target() {
        let slots = [
            ModSlot {
                source: ModSource::Velocity,
                target: ModTarget::Cutoff,
                amount: 0.5,
            },
            ModSlot {
                source: ModSource::Lfo,
                target: ModTarget::Cutoff,
                amount: -1.0,
            },
            ModSlot {
                source: ModSource::Velocity,
                target: ModTarget::Off,
                amount: 1.0,
            },
        ];
        let sources = Sources {
            velocity: 1.0,
            lfo: 0.25,
            ..Sources::default()
        };
        let m = ModValues::evaluate(&slots, &sources);
        assert!((m.cutoff - 0.25).abs() < 1e-6);
        assert_eq!(m.pitch, 0.0);
    }

    #[test]
    fn rng_ranges() {
        let mut rng = Rng::new(7);
        for _ in 0..10_000 {
            let u = rng.unipolar();
            let b = rng.bipolar();
            assert!((0.0..1.0).contains(&u));
            assert!((-1.0..1.0).contains(&b));
            assert!(rng.below(3) < 3);
        }
    }
}
