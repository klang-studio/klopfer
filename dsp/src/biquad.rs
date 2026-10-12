//! Stereo biquad, used for the per-pad parametric EQ band.

use std::f32::consts::PI;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BiquadCoeffs {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

impl BiquadCoeffs {
    pub const IDENTITY: Self = Self {
        b0: 1.0,
        b1: 0.0,
        b2: 0.0,
        a1: 0.0,
        a2: 0.0,
    };

    /// RBJ cookbook peaking EQ.
    pub fn peaking(freq: f32, q: f32, gain_db: f32, sample_rate: f32) -> Self {
        if gain_db.abs() < 0.01 {
            return Self::IDENTITY;
        }

        let a = 10f32.powf(gain_db / 40.0);
        let w0 = 2.0 * PI * freq.clamp(20.0, sample_rate * 0.45) / sample_rate;
        let alpha = w0.sin() / (2.0 * q.max(0.05));
        let cos = w0.cos();
        let a0 = 1.0 + alpha / a;

        Self {
            b0: (1.0 + alpha * a) / a0,
            b1: (-2.0 * cos) / a0,
            b2: (1.0 - alpha * a) / a0,
            a1: (-2.0 * cos) / a0,
            a2: (1.0 - alpha / a) / a0,
        }
    }

    pub fn is_identity(&self) -> bool {
        *self == Self::IDENTITY
    }
}

/// Transposed direct form II state for two channels.
#[derive(Debug, Clone, Copy, Default)]
pub struct Biquad {
    s1: [f32; 2],
    s2: [f32; 2],
}

impl Biquad {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    #[inline]
    pub fn process(&mut self, c: &BiquadCoeffs, input: (f32, f32)) -> (f32, f32) {
        if c.is_identity() {
            return input;
        }

        let mut out = [0.0; 2];
        for (ch, x) in [input.0, input.1].into_iter().enumerate() {
            let y = c.b0 * x + self.s1[ch];
            self.s1[ch] = c.b1 * x - c.a1 * y + self.s2[ch];
            self.s2[ch] = c.b2 * x - c.a2 * y;
            out[ch] = y;
        }
        (out[0], out[1])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peak_boosts_at_center_frequency() {
        let sr = 48000.0;
        let c = BiquadCoeffs::peaking(1000.0, 1.0, 12.0, sr);
        let mut f = Biquad::default();
        let mut peak = 0.0f32;
        for i in 0..48000 {
            let x = (2.0 * PI * 1000.0 * i as f32 / sr).sin();
            let (y, _) = f.process(&c, (x, x));
            if i > 24000 {
                peak = peak.max(y.abs());
            }
        }
        // +12 dB is a factor of ~3.98
        assert!((peak - 3.98).abs() < 0.1, "peak was {peak}");
    }
}
