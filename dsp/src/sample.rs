//! Decoded audio data for a single pad.

use crate::interp::hermite;

/// A decoded sample at its original sample rate. Mono samples have no right channel.
#[derive(Debug, Clone)]
pub struct Sample {
    pub sample_rate: f32,
    pub left: Vec<f32>,
    pub right: Option<Vec<f32>>,
}

impl Sample {
    pub fn mono(sample_rate: f32, data: Vec<f32>) -> Self {
        Self {
            sample_rate,
            left: data,
            right: None,
        }
    }

    pub fn stereo(sample_rate: f32, left: Vec<f32>, right: Vec<f32>) -> Self {
        debug_assert_eq!(left.len(), right.len());
        Self {
            sample_rate,
            left,
            right: Some(right),
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.left.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.left.is_empty()
    }

    #[inline]
    pub fn is_stereo(&self) -> bool {
        self.right.is_some()
    }

    pub fn duration_secs(&self) -> f32 {
        self.len() as f32 / self.sample_rate
    }

    /// Reads an interpolated frame at a fractional position. Positions outside of the sample are
    /// clamped to the first/last frame.
    #[inline]
    pub fn read(&self, pos: f64) -> (f32, f32) {
        let last = self.len() as isize - 1;
        if last < 0 {
            return (0.0, 0.0);
        }

        let i = pos.floor() as isize;
        let t = (pos - i as f64) as f32;
        let idx = |o: isize| (i + o).clamp(0, last) as usize;
        let (a, b, c, d) = (idx(-1), idx(0), idx(1), idx(2));

        let l = &self.left;
        let left = hermite(l[a], l[b], l[c], l[d], t);
        match &self.right {
            Some(r) => (left, hermite(r[a], r[b], r[c], r[d], t)),
            None => (left, left),
        }
    }

    /// Min/max pairs over `buckets` equally sized slices of the sample, for waveform displays.
    /// Stereo samples are summed to mono.
    pub fn peaks(&self, buckets: usize) -> Vec<(f32, f32)> {
        let len = self.len();
        if len == 0 || buckets == 0 {
            return Vec::new();
        }

        (0..buckets)
            .map(|b| {
                let from = b * len / buckets;
                let to = ((b + 1) * len / buckets).max(from + 1).min(len);
                (from..to).fold((f32::MAX, f32::MIN), |(lo, hi), i| {
                    let v = match &self.right {
                        Some(r) => (self.left[i] + r[i]) * 0.5,
                        None => self.left[i],
                    };
                    (lo.min(v), hi.max(v))
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_exact_frames() {
        let s = Sample::stereo(
            44100.0,
            vec![0.0, 1.0, 2.0, 3.0],
            vec![0.0, -1.0, -2.0, -3.0],
        );
        assert_eq!(s.read(1.0), (1.0, -1.0));
        assert_eq!(s.read(3.0), (3.0, -3.0));
    }

    #[test]
    fn mono_is_duplicated() {
        let s = Sample::mono(44100.0, vec![0.5; 8]);
        assert_eq!(s.read(2.5), (0.5, 0.5));
    }

    #[test]
    fn peaks_cover_whole_sample() {
        let s = Sample::mono(44100.0, vec![-1.0, 0.0, 0.0, 1.0]);
        assert_eq!(s.peaks(2), vec![(-1.0, 0.0), (0.0, 1.0)]);
    }
}
