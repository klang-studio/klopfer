//! Stereo state variable filter (Cytomic / Andrew Simper's trapezoidal SVF).

use std::f32::consts::PI;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SvfMode {
    #[default]
    LowPass,
    HighPass,
    BandPass,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Svf {
    ic1: [f32; 2],
    ic2: [f32; 2],
}

impl Svf {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Processes one stereo frame. `resonance` is in `[0, 1]`.
    #[inline]
    pub fn process(
        &mut self,
        mode: SvfMode,
        cutoff_hz: f32,
        resonance: f32,
        sample_rate: f32,
        input: (f32, f32),
    ) -> (f32, f32) {
        let fc = cutoff_hz.clamp(20.0, sample_rate * 0.45);
        let g = (PI * fc / sample_rate).tan();
        // Q from 0.5 to ~25
        let k = 2.0 - 1.92 * resonance.clamp(0.0, 1.0);
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;

        let mut out = [0.0; 2];
        for (ch, v0) in [input.0, input.1].into_iter().enumerate() {
            let v3 = v0 - self.ic2[ch];
            let v1 = a1 * self.ic1[ch] + a2 * v3;
            let v2 = self.ic2[ch] + a2 * self.ic1[ch] + a3 * v3;
            self.ic1[ch] = 2.0 * v1 - self.ic1[ch];
            self.ic2[ch] = 2.0 * v2 - self.ic2[ch];

            out[ch] = match mode {
                SvfMode::LowPass => v2,
                SvfMode::BandPass => v1,
                SvfMode::HighPass => v0 - k * v1 - v2,
            };
        }

        (out[0], out[1])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rms_of_sine(mode: SvfMode, cutoff: f32, freq: f32) -> f32 {
        let sr = 48000.0;
        let mut f = Svf::default();
        let n = 48000;
        let mut acc = 0.0;
        for i in 0..n {
            let x = (2.0 * PI * freq * i as f32 / sr).sin();
            let (y, _) = f.process(mode, cutoff, 0.0, sr, (x, x));
            if i > n / 2 {
                acc += y * y;
            }
        }
        (acc / (n / 2) as f32).sqrt()
    }

    #[test]
    fn lowpass_attenuates_highs() {
        assert!(rms_of_sine(SvfMode::LowPass, 200.0, 5000.0) < 0.01);
        assert!(rms_of_sine(SvfMode::LowPass, 5000.0, 100.0) > 0.6);
    }

    #[test]
    fn highpass_attenuates_lows() {
        assert!(rms_of_sine(SvfMode::HighPass, 5000.0, 100.0) < 0.01);
    }
}
