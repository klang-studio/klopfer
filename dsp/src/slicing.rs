//! Finding slice points in loops: transient detection, a tempo guess and a beat grid.

use crate::sample::Sample;

/// Analysis hop for the onset detector
const HOP: usize = 256;
/// Slices start this much before the detected onset so attacks stay intact
const PRE_ROLL_SECS: f32 = 0.002;

fn mono(sample: &Sample, i: usize) -> f32 {
    match &sample.right {
        Some(r) => (sample.left[i] + r[i]) * 0.5,
        None => sample.left[i],
    }
}

/// Hops to look back for the local minimum an onset rises from (about 25 ms)
const LOOKBACK_HOPS: usize = 4;

/// Start frames of the slices found by an energy based onset detector. The first slice always
/// starts at frame 0. `sensitivity` is in `0..1`, higher finds more slices.
///
/// An onset is a rise of the short-term energy above its minimum over the last ~25 ms. Comparing
/// with that minimum instead of the previous hop keeps the random level changes in noisy decays
/// (snares, hats) from counting as hits.
pub fn detect_transients(sample: &Sample, sensitivity: f32, min_gap_secs: f32) -> Vec<usize> {
    let len = sample.len();
    let mut starts = vec![0];
    if len < HOP * (LOOKBACK_HOPS + 2) {
        return starts;
    }

    // Energy per hop in dB, of the first difference: a gentle high-pass that lets hats and
    // attacks stand out against the low end of kicks still ringing
    let energy: Vec<f32> = (0..len / HOP)
        .map(|h| {
            let e: f32 = (h * HOP..(h + 1) * HOP)
                .map(|i| (mono(sample, i) - mono(sample, i.saturating_sub(1))).powi(2))
                .sum();
            10.0 * (e / HOP as f32 + 1e-12).log10()
        })
        .collect();
    let rise: Vec<f32> = (0..energy.len())
        .map(|h| {
            let from = h.saturating_sub(LOOKBACK_HOPS);
            let floor = energy[from..h.max(from + 1)]
                .iter()
                .cloned()
                .fold(f32::MAX, f32::min);
            (energy[h] - floor).max(0.0)
        })
        .collect();

    // Required rise: 15 dB at sensitivity 0 down to 3 dB at 1
    let threshold = 15.0 - 12.0 * sensitivity.clamp(0.0, 1.0);
    // Ignore onsets in near silence
    let floor = energy.iter().cloned().fold(f32::MIN, f32::max) - 50.0;

    let min_gap = ((min_gap_secs * sample.sample_rate) as usize / HOP).max(1);
    let pre_roll = (PRE_ROLL_SECS * sample.sample_rate) as usize;
    let mut last_hop = 0;
    let mut h = 1;
    while h < rise.len() {
        if rise[h] > threshold && energy[h] > floor && h - last_hop >= min_gap {
            // The onset is where the rise starts; the slice starts slightly before it
            let mut onset = h;
            while onset > 1 && rise[onset - 1] > threshold * 0.5 && h - onset < LOOKBACK_HOPS {
                onset -= 1;
            }
            let frame = (onset * HOP).saturating_sub(pre_roll);
            if frame > *starts.last().unwrap_or(&0) {
                starts.push(frame);
            }
            last_hop = h;
            // Skip the rest of this rise
            while h < rise.len() && rise[h] > threshold {
                h += 1;
            }
        }
        h += 1;
    }
    starts
}

/// Start frames of equally long slices, `beats_per_slice` beats each at `bpm`.
pub fn slice_by_beats(sample: &Sample, bpm: f32, beats_per_slice: f32) -> Vec<usize> {
    let step = (60.0 / bpm.max(1.0) * beats_per_slice * sample.sample_rate) as usize;
    if step == 0 {
        return vec![0];
    }
    (0..sample.len()).step_by(step).collect()
}

/// Guesses the tempo of a loop from its length, assuming it spans a whole number of 4/4 bars.
/// Picks the bar count that puts the tempo closest to 120 BPM within 80..160.
pub fn estimate_bpm(sample: &Sample) -> f32 {
    let secs = sample.duration_secs();
    if secs <= 0.0 {
        return 120.0;
    }
    [1.0f32, 2.0, 4.0, 8.0, 16.0, 32.0]
        .iter()
        .map(|bars| bars * 4.0 * 60.0 / secs)
        .filter(|bpm| (80.0..160.0).contains(bpm))
        .min_by(|a, b| (a - 120.0).abs().total_cmp(&(b - 120.0).abs()))
        .unwrap_or_else(|| 4.0 * 60.0 / secs)
}

/// Turns start frames into normalized `(start, end)` regions covering the whole sample.
pub fn regions(starts: &[usize], len: usize) -> Vec<(f32, f32)> {
    if len == 0 {
        return Vec::new();
    }
    starts
        .iter()
        .enumerate()
        .map(|(i, &s)| {
            let end = starts.get(i + 1).copied().unwrap_or(len);
            (s as f32 / len as f32, end as f32 / len as f32)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 44100.0;

    /// Noise bursts with exponential decay at the given times.
    fn hits(times: &[f32], secs: f32) -> Sample {
        let len = (secs * SR) as usize;
        let mut data = vec![0.0; len];
        let mut seed = 1u32;
        for &t in times {
            let start = (t * SR) as usize;
            for i in start..len.min(start + (0.15 * SR) as usize) {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                let noise = seed as f32 / u32::MAX as f32 * 2.0 - 1.0;
                data[i] += noise * (-((i - start) as f32) / (0.03 * SR)).exp();
            }
        }
        Sample::mono(SR, data)
    }

    #[test]
    fn finds_drum_hits() {
        let times = [0.0, 0.5, 1.0, 1.25, 1.5];
        let starts = detect_transients(&hits(&times, 2.0), 0.5, 0.05);
        assert_eq!(starts.len(), times.len(), "{starts:?}");
        for (s, t) in starts.iter().zip(times) {
            let err = (*s as f32 / SR - t).abs();
            assert!(err < 0.01, "slice at {} for hit at {t}", *s as f32 / SR);
        }
    }

    #[test]
    fn noisy_decays_dont_count_as_hits() {
        // Eighth notes at 120 BPM with loud and quiet hits, like a drum loop
        let times: Vec<f32> = (0..8).map(|i| i as f32 * 0.25).collect();
        let mut s = hits(&times, 2.0);
        // Make every other hit quieter, as hats between kicks and snares
        for (i, x) in s.left.iter_mut().enumerate() {
            let t = i as f32 / SR;
            if ((t / 0.25) as usize) % 2 == 1 {
                *x *= 0.3;
            }
        }
        let starts = detect_transients(&s, 0.5, 0.05);
        assert_eq!(starts.len(), 8, "{starts:?}");
    }

    #[test]
    fn beat_grid_and_regions() {
        let s = Sample::mono(SR, vec![0.0; (2.0 * SR) as usize]);
        // 2 s at 120 BPM = 4 beats, sliced in eighths
        let starts = slice_by_beats(&s, 120.0, 0.5);
        assert_eq!(starts.len(), 8);
        let r = regions(&starts, s.len());
        assert_eq!(r[0].0, 0.0);
        assert_eq!(r[7].1, 1.0);
        assert!((r[1].0 - 0.125).abs() < 1e-4);
    }

    #[test]
    fn tempo_guess_from_loop_length() {
        // One bar at 95 BPM
        let s = Sample::mono(SR, vec![0.0; (4.0 * 60.0 / 95.0 * SR) as usize]);
        assert!((estimate_bpm(&s) - 95.0).abs() < 0.1);
        // Two bars at 140 BPM
        let s = Sample::mono(SR, vec![0.0; (8.0 * 60.0 / 140.0 * SR) as usize]);
        assert!((estimate_bpm(&s) - 140.0).abs() < 0.1);
    }
}
