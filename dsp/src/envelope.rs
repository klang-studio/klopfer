//! Envelope generators. Coefficients are precomputed per pad in [`crate::pad::PadSettings`] so the
//! per-sample work is just a few multiply-adds.

/// Below this level an envelope counts as silent.
pub const SILENCE: f32 = 1e-4;

/// `ln(1000)`: an exponential segment of length `T` covers 60 dB.
const LN_1000: f32 = 6.907_755;

/// Per-sample coefficient for an exponential segment that decays by 60 dB in `secs`.
pub fn exp_coeff(secs: f32, sample_rate: f32) -> f32 {
    if secs <= 0.0 {
        0.0
    } else {
        (-LN_1000 / (secs * sample_rate)).exp()
    }
}

/// Precomputed AHDSR coefficients.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AhdsrCoeffs {
    /// Level increment per sample during the (linear) attack. `>= 1.0` means instant.
    pub attack_inc: f32,
    pub hold_samples: u32,
    pub decay_coeff: f32,
    pub sustain: f32,
    pub release_coeff: f32,
}

impl AhdsrCoeffs {
    pub fn new(
        attack_s: f32,
        hold_s: f32,
        decay_s: f32,
        sustain: f32,
        release_s: f32,
        sample_rate: f32,
    ) -> Self {
        Self {
            attack_inc: if attack_s <= 0.0 {
                1.0
            } else {
                1.0 / (attack_s * sample_rate)
            },
            hold_samples: (hold_s.max(0.0) * sample_rate) as u32,
            decay_coeff: exp_coeff(decay_s, sample_rate),
            sustain: sustain.clamp(0.0, 1.0),
            release_coeff: exp_coeff(release_s, sample_rate),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Idle,
    Attack,
    Hold(u32),
    Decay,
    Sustain,
    Release,
}

/// Attack (linear), hold, decay (exponential), sustain, release (exponential).
#[derive(Debug, Clone, Copy)]
pub struct Ahdsr {
    stage: Stage,
    level: f32,
}

impl Default for Ahdsr {
    fn default() -> Self {
        Self {
            stage: Stage::Idle,
            level: 0.0,
        }
    }
}

impl Ahdsr {
    pub fn trigger(&mut self) {
        self.stage = Stage::Attack;
        self.level = 0.0;
    }

    pub fn release(&mut self) {
        if self.stage != Stage::Idle {
            self.stage = Stage::Release;
        }
    }

    pub fn is_idle(&self) -> bool {
        self.stage == Stage::Idle
    }

    pub fn is_releasing(&self) -> bool {
        self.stage == Stage::Release
    }

    pub fn level(&self) -> f32 {
        self.level
    }

    #[inline]
    pub fn next(&mut self, c: &AhdsrCoeffs) -> f32 {
        match self.stage {
            Stage::Idle => {}
            Stage::Attack => {
                self.level += c.attack_inc;
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.stage = Stage::Hold(0);
                }
            }
            Stage::Hold(n) => {
                if n >= c.hold_samples {
                    self.stage = Stage::Decay;
                } else {
                    self.stage = Stage::Hold(n + 1);
                }
            }
            Stage::Decay => {
                self.level = c.sustain + (self.level - c.sustain) * c.decay_coeff;
                if (self.level - c.sustain).abs() < SILENCE {
                    self.level = c.sustain;
                    self.stage = if c.sustain <= SILENCE {
                        Stage::Idle
                    } else {
                        Stage::Sustain
                    };
                }
            }
            Stage::Sustain => {
                // Follow sustain changes smoothly while holding
                self.level = c.sustain + (self.level - c.sustain) * c.decay_coeff;
            }
            Stage::Release => {
                self.level *= c.release_coeff;
                if self.level < SILENCE {
                    self.level = 0.0;
                    self.stage = Stage::Idle;
                }
            }
        }

        self.level
    }
}

/// A simple exponential decay from 1 to 0, used for pitch and filter envelopes.
#[derive(Debug, Clone, Copy, Default)]
pub struct DecayEnv {
    level: f32,
}

impl DecayEnv {
    pub fn trigger(&mut self) {
        self.level = 1.0;
    }

    #[inline]
    pub fn next(&mut self, coeff: f32) -> f32 {
        let out = self.level;
        self.level *= coeff;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 1000.0;

    fn run(env: &mut Ahdsr, c: &AhdsrCoeffs, n: usize) -> f32 {
        (0..n).map(|_| env.next(c)).last().unwrap_or(0.0)
    }

    #[test]
    fn attack_is_linear() {
        let c = AhdsrCoeffs::new(0.1, 0.0, 1.0, 1.0, 0.1, SR);
        let mut env = Ahdsr::default();
        env.trigger();
        assert!((run(&mut env, &c, 50) - 0.5).abs() < 1e-3);
    }

    #[test]
    fn decays_to_sustain_and_releases_to_idle() {
        let c = AhdsrCoeffs::new(0.0, 0.0, 0.05, 0.5, 0.05, SR);
        let mut env = Ahdsr::default();
        env.trigger();
        assert!((run(&mut env, &c, 200) - 0.5).abs() < 1e-3);
        env.release();
        run(&mut env, &c, 200);
        assert!(env.is_idle());
    }

    #[test]
    fn zero_sustain_ends_after_decay() {
        let c = AhdsrCoeffs::new(0.0, 0.0, 0.05, 0.0, 1.0, SR);
        let mut env = Ahdsr::default();
        env.trigger();
        run(&mut env, &c, 200);
        assert!(env.is_idle());
    }

    #[test]
    fn exp_coeff_hits_minus_60_db() {
        let k = exp_coeff(1.0, SR);
        assert!((k.powi(1000) - 0.001).abs() < 1e-5);
    }
}
