//! Offline time stretching and pitch shifting. Runs on a background thread, never in the audio
//! callback.

use std::f32::consts::PI;

use crate::sample::Sample;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StretchMode {
    /// WSOLA: overlap-add of waveform-aligned frames, keeps drums tight
    #[default]
    Smooth,
    /// Fixed grains without alignment, the grainy sound of vintage samplers
    Cycle,
}

/// Smooth mode frame length
const FRAME_SECS: f32 = 0.03;
/// Cycle mode grain length and crossfade
const GRAIN_SECS: f32 = 0.035;
const GRAIN_FADE_SECS: f32 = 0.002;

fn channels(sample: &Sample) -> Vec<&[f32]> {
    match &sample.right {
        Some(r) => vec![&sample.left, r],
        None => vec![&sample.left],
    }
}

#[inline]
fn at(data: &[f32], i: isize) -> f32 {
    if i < 0 {
        0.0
    } else {
        data.get(i as usize).copied().unwrap_or(0.0)
    }
}

/// Changes the length by `time_ratio` (2.0 = twice as long) and the pitch by `pitch_semis`,
/// independently of each other. Returns `None` when `cancelled` returns true during processing.
/// The result's `baked_pitch` records the pitch shift.
pub fn stretch(
    sample: &Sample,
    time_ratio: f32,
    pitch_semis: f32,
    mode: StretchMode,
    cancelled: &dyn Fn() -> bool,
) -> Option<Sample> {
    let pitch = (pitch_semis / 12.0).exp2();
    // Stretch by the combined factor, then resample: the resampling restores the length and
    // shifts the pitch
    let ratio = (time_ratio * pitch).clamp(0.05, 20.0);
    let input = channels(sample);
    let stretched = match mode {
        StretchMode::Smooth => wsola(&input, sample.sample_rate, ratio, cancelled)?,
        StretchMode::Cycle => cycle(&input, sample.sample_rate, ratio, cancelled)?,
    };

    let mut out = resample(stretched, sample.sample_rate, pitch);
    out.baked_pitch = pitch_semis;
    Some(out)
}

/// Reads the channels at `rate` times the original speed.
fn resample(mut channels: Vec<Vec<f32>>, sample_rate: f32, rate: f32) -> Sample {
    let tmp = if channels.len() > 1 {
        let right = channels.pop().unwrap_or_default();
        Sample::stereo(sample_rate, channels.pop().unwrap_or_default(), right)
    } else {
        Sample::mono(sample_rate, channels.pop().unwrap_or_default())
    };
    if (rate - 1.0).abs() < 1e-6 {
        return tmp;
    }

    let out_len = (tmp.len() as f64 / rate as f64) as usize;
    let (mut left, mut right) = (Vec::with_capacity(out_len), Vec::with_capacity(out_len));
    for i in 0..out_len {
        let (l, r) = tmp.read(i as f64 * rate as f64);
        left.push(l);
        right.push(r);
    }
    if tmp.is_stereo() {
        Sample::stereo(sample_rate, left, right)
    } else {
        Sample::mono(sample_rate, left)
    }
}

/// Waveform similarity overlap-add.
fn wsola(
    input: &[&[f32]],
    sample_rate: f32,
    ratio: f32,
    cancelled: &dyn Fn() -> bool,
) -> Option<Vec<Vec<f32>>> {
    let len = input[0].len();
    let n = ((FRAME_SECS * sample_rate) as usize).max(64) & !1;
    let hs = n / 2;
    let ha = hs as f64 / ratio as f64;
    let tolerance = (n / 4) as isize;
    let out_len = (len as f64 * ratio as f64) as usize;

    // Periodic Hann: overlapping by half sums to one
    let window: Vec<f32> = (0..n)
        .map(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / n as f32).cos())
        .collect();
    let mono: Vec<f32> = (0..len)
        .map(|i| input.iter().map(|c| c[i]).sum::<f32>() / input.len() as f32)
        .collect();

    let mut out = vec![vec![0.0f32; out_len + n]; input.len()];
    let mut norm = vec![0.0f32; out_len + n];
    let mut prev: isize = 0;

    for k in 0..=out_len / hs {
        if k % 64 == 0 && cancelled() {
            return None;
        }

        let pos = if k == 0 {
            0
        } else {
            // Pick the input position near the nominal one whose waveform continues the previous
            // frame best (normalized cross-correlation over the overlap)
            let natural = prev + hs as isize;
            let nominal = (k as f64 * ha) as isize;
            let reference: Vec<f32> = (0..hs as isize).map(|i| at(&mono, natural + i)).collect();
            let mut best = (nominal, f32::MIN);
            let mut cand = nominal - tolerance;
            while cand <= nominal + tolerance {
                let (mut dot, mut energy) = (0.0f32, 1e-9f32);
                for (i, r) in reference.iter().enumerate().step_by(2) {
                    let x = at(&mono, cand + i as isize);
                    dot += r * x;
                    energy += x * x;
                }
                let score = dot / energy.sqrt();
                if score > best.1 {
                    best = (cand, score);
                }
                cand += 2;
            }
            best.0.max(0)
        };

        let out_pos = k * hs;
        for i in 0..n {
            let w = window[i];
            for (o, c) in out.iter_mut().zip(input) {
                o[out_pos + i] += at(c, pos + i as isize) * w;
            }
            norm[out_pos + i] += w;
        }
        prev = pos;
    }

    for o in &mut out {
        for (x, w) in o.iter_mut().zip(&norm) {
            if *w > 0.1 {
                *x /= w;
            }
        }
        o.truncate(out_len);
    }
    Some(out)
}

/// Fixed grains read at the stretched position, joined with short crossfades.
fn cycle(
    input: &[&[f32]],
    sample_rate: f32,
    ratio: f32,
    cancelled: &dyn Fn() -> bool,
) -> Option<Vec<Vec<f32>>> {
    let len = input[0].len();
    let grain = ((GRAIN_SECS * sample_rate) as usize).max(16);
    let fade = ((GRAIN_FADE_SECS * sample_rate) as usize).clamp(1, grain / 2);
    let out_len = (len as f64 * ratio as f64) as usize;
    let mut out = vec![vec![0.0f32; out_len + grain + fade]; input.len()];

    for k in 0..=out_len / grain {
        if k % 256 == 0 && cancelled() {
            return None;
        }
        let out_start = k * grain;
        let in_start = (out_start as f64 / ratio as f64) as isize;
        for i in 0..grain + fade {
            // Linear fade in over the first `fade` samples, out over the last ones
            let gain = if i < fade && k > 0 {
                i as f32 / fade as f32
            } else if i >= grain {
                1.0 - (i - grain) as f32 / fade as f32
            } else {
                1.0
            };
            for (o, c) in out.iter_mut().zip(input) {
                o[out_start + i] += at(c, in_start + i as isize) * gain;
            }
        }
    }
    for o in &mut out {
        o.truncate(out_len);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 44100.0;

    fn sine(freq: f32, secs: f32) -> Sample {
        Sample::mono(
            SR,
            (0..(secs * SR) as usize)
                .map(|i| (2.0 * PI * freq * i as f32 / SR).sin() * 0.5)
                .collect(),
        )
    }

    /// Dominant frequency from the zero-crossing rate of the middle part.
    fn frequency(s: &Sample) -> f32 {
        let d = &s.left[s.len() / 4..s.len() * 3 / 4];
        let crossings = d.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count();
        crossings as f32 / (d.len() as f32 / SR)
    }

    const NEVER: &dyn Fn() -> bool = &|| false;

    #[test]
    fn smooth_stretch_keeps_pitch() {
        let s = stretch(&sine(220.0, 1.0), 2.0, 0.0, StretchMode::Smooth, NEVER).unwrap();
        assert!(
            (s.duration_secs() - 2.0).abs() < 0.02,
            "{}",
            s.duration_secs()
        );
        assert!((frequency(&s) - 220.0).abs() < 5.0, "{}", frequency(&s));
    }

    #[test]
    fn pitch_shift_keeps_length() {
        let s = stretch(&sine(220.0, 1.0), 1.0, 12.0, StretchMode::Smooth, NEVER).unwrap();
        assert!(
            (s.duration_secs() - 1.0).abs() < 0.02,
            "{}",
            s.duration_secs()
        );
        assert!((frequency(&s) - 440.0).abs() < 10.0, "{}", frequency(&s));
        assert_eq!(s.baked_pitch, 12.0);
    }

    #[test]
    fn cycle_mode_shortens() {
        let s = stretch(&sine(220.0, 1.0), 0.5, 0.0, StretchMode::Cycle, NEVER).unwrap();
        assert!((s.duration_secs() - 0.5).abs() < 0.02);
        assert!((frequency(&s) - 220.0).abs() < 10.0, "{}", frequency(&s));
    }

    #[test]
    fn stereo_stays_stereo_and_can_be_cancelled() {
        let mono = sine(100.0, 0.5);
        let s = Sample::stereo(SR, mono.left.clone(), mono.left.clone());
        let out = stretch(&s, 1.5, 0.0, StretchMode::Smooth, NEVER).unwrap();
        assert!(out.is_stereo());
        assert!(stretch(&s, 1.5, 0.0, StretchMode::Smooth, &|| true).is_none());
    }
}
