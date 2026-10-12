//! Bit depth and sample rate reduction.

#[derive(Debug, Clone, Copy, Default)]
pub struct LoFi {
    phase: f32,
    held: (f32, f32),
}

impl LoFi {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// `bits >= 24` disables quantization, `downsample <= 1` disables sample-and-hold.
    #[inline]
    pub fn process(&mut self, bits: f32, downsample: f32, input: (f32, f32)) -> (f32, f32) {
        let mut x = input;

        if downsample > 1.0 {
            if self.phase <= 0.0 {
                self.held = x;
                self.phase += downsample;
            }
            self.phase -= 1.0;
            x = self.held;
        }

        if bits < 24.0 {
            let steps = 2f32.powf(bits - 1.0);
            x = ((x.0 * steps).round() / steps, (x.1 * steps).round() / steps);
        }

        x
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_is_transparent() {
        let mut l = LoFi::default();
        assert_eq!(l.process(24.0, 1.0, (0.123, -0.456)), (0.123, -0.456));
    }

    #[test]
    fn quantizes_to_bit_depth() {
        let mut l = LoFi::default();
        // 2 bits: steps of 0.5
        assert_eq!(l.process(2.0, 1.0, (0.3, -0.8)), (0.5, -1.0));
    }

    #[test]
    fn holds_samples() {
        let mut l = LoFi::default();
        let out: Vec<f32> = (0..4)
            .map(|i| l.process(24.0, 2.0, (i as f32, 0.0)).0)
            .collect();
        assert_eq!(out, vec![0.0, 0.0, 2.0, 2.0]);
    }
}
