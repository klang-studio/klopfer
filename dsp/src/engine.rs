//! Voice management: allocation, stealing, choke groups and output routing.

use std::sync::Arc;

use crate::pad::PadSettings;
use crate::sample::Sample;
use crate::voice::Voice;

pub const NUM_PADS: usize = 16;
/// Stereo output buses: main + 15 aux
pub const NUM_OUTPUTS: usize = 16;
pub const MAX_VOICES: usize = 64;

/// Fade time for choked voices
const CHOKE_FADE_SECS: f32 = 0.005;

pub struct Engine {
    voices: [Voice; MAX_VOICES],
    sample_rate: f32,
    choke_step: f32,
    next_id: u64,
    pad_peaks: [f32; NUM_PADS],
}

impl Engine {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            voices: [Voice::default(); MAX_VOICES],
            sample_rate,
            choke_step: 1.0 / (CHOKE_FADE_SECS * sample_rate),
            next_id: 0,
            pad_peaks: [0.0; NUM_PADS],
        }
    }

    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    pub fn reset(&mut self) {
        for v in &mut self.voices {
            v.active = false;
        }
        self.pad_peaks = [0.0; NUM_PADS];
    }

    pub fn active_voices(&self) -> usize {
        self.voices.iter().filter(|v| v.active).count()
    }

    /// Starts a voice for `pad`. Does nothing if the pad has no sample.
    pub fn note_on(
        &mut self,
        pad: usize,
        velocity: f32,
        samples: &[Option<Arc<Sample>>],
        settings: &[PadSettings],
    ) {
        let Some(sample) = samples.get(pad).and_then(|s| s.as_deref()) else {
            return;
        };
        let s = &settings[pad];

        if s.choke_group > 0 {
            for v in self.voices.iter_mut().filter(|v| v.active && v.pad != pad) {
                if settings[v.pad].choke_group == s.choke_group {
                    v.choke();
                }
            }
        }

        let slot = match self.voices.iter().position(|v| !v.active) {
            Some(i) => i,
            None => self
                .voices
                .iter()
                .enumerate()
                .min_by_key(|(_, v)| v.id)
                .map(|(i, _)| i)
                .unwrap_or(0),
        };

        self.next_id += 1;
        self.voices[slot].start(pad, self.next_id, velocity, sample, s);
    }

    /// Releases gate-mode voices of `pad`. One-shot voices ignore note offs.
    pub fn note_off(&mut self, pad: usize, settings: &[PadSettings]) {
        if !settings[pad].gate {
            return;
        }
        for v in self.voices.iter_mut().filter(|v| v.active && v.pad == pad) {
            v.release();
        }
    }

    /// Fades out all voices, e.g. for an all-notes-off message.
    pub fn choke_all(&mut self) {
        for v in self.voices.iter_mut().filter(|v| v.active) {
            v.choke();
        }
    }

    /// Immediately stops all voices of `pad`. Needed before its sample gets replaced.
    pub fn kill_pad(&mut self, pad: usize) {
        for v in self.voices.iter_mut().filter(|v| v.pad == pad) {
            v.active = false;
        }
    }

    /// Renders one frame and adds it to the output buses.
    #[inline]
    pub fn render_frame(
        &mut self,
        samples: &[Option<Arc<Sample>>],
        settings: &[PadSettings],
        bus: &mut [[f32; 2]; NUM_OUTPUTS],
    ) {
        for v in self.voices.iter_mut().filter(|v| v.active) {
            let Some(sample) = samples.get(v.pad).and_then(|s| s.as_deref()) else {
                v.active = false;
                continue;
            };
            let s = &settings[v.pad];
            let (l, r) = v.render(sample, s, self.sample_rate, self.choke_step);

            let out = &mut bus[s.output.min(NUM_OUTPUTS - 1)];
            out[0] += l;
            out[1] += r;

            let peak = &mut self.pad_peaks[v.pad];
            *peak = peak.max(l.abs()).max(r.abs());
        }
    }

    /// Returns the per-pad peak levels since the last call and resets them.
    pub fn take_pad_peaks(&mut self) -> [f32; NUM_PADS] {
        std::mem::replace(&mut self.pad_peaks, [0.0; NUM_PADS])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pad::PadParams;

    const SR: f32 = 48000.0;

    fn setup(params: &[(usize, PadParams)]) -> (Vec<Option<Arc<Sample>>>, Vec<PadSettings>) {
        let samples = (0..NUM_PADS)
            .map(|_| Some(Arc::new(Sample::mono(SR, vec![0.5; 4800]))))
            .collect();
        let mut settings = vec![PadSettings::new(&PadParams::default(), SR); NUM_PADS];
        for (pad, p) in params {
            settings[*pad] = PadSettings::new(p, SR);
        }
        (samples, settings)
    }

    fn render(
        e: &mut Engine,
        samples: &[Option<Arc<Sample>>],
        settings: &[PadSettings],
        n: usize,
    ) -> [[f32; 2]; NUM_OUTPUTS] {
        let mut bus = [[0.0; 2]; NUM_OUTPUTS];
        for _ in 0..n {
            bus = [[0.0; 2]; NUM_OUTPUTS];
            e.render_frame(samples, settings, &mut bus);
        }
        bus
    }

    #[test]
    fn plays_sample_until_end() {
        let (samples, settings) = setup(&[]);
        let mut e = Engine::new(SR);
        e.note_on(0, 1.0, &samples, &settings);
        let bus = render(&mut e, &samples, &settings, 10);
        assert!((bus[0][0] - 0.5).abs() < 1e-3);
        render(&mut e, &samples, &settings, 4800);
        assert_eq!(e.active_voices(), 0);
    }

    #[test]
    fn velocity_scales_level() {
        let (samples, settings) = setup(&[]);
        let mut e = Engine::new(SR);
        e.note_on(0, 0.5, &samples, &settings);
        let bus = render(&mut e, &samples, &settings, 10);
        assert!((bus[0][0] - 0.25).abs() < 1e-3);
    }

    #[test]
    fn routes_to_selected_output() {
        let p = PadParams {
            output: 3,
            ..PadParams::default()
        };
        let (samples, settings) = setup(&[(1, p)]);
        let mut e = Engine::new(SR);
        e.note_on(1, 1.0, &samples, &settings);
        let bus = render(&mut e, &samples, &settings, 10);
        assert_eq!(bus[0], [0.0, 0.0]);
        assert!(bus[3][0] > 0.4);
    }

    #[test]
    fn choke_group_silences_other_pads() {
        let p = PadParams {
            choke_group: 1,
            ..PadParams::default()
        };
        let (samples, settings) = setup(&[(2, p), (3, p)]);
        let mut e = Engine::new(SR);
        e.note_on(2, 1.0, &samples, &settings);
        render(&mut e, &samples, &settings, 10);
        e.note_on(3, 1.0, &samples, &settings);
        // 5 ms fade at 48 kHz
        render(&mut e, &samples, &settings, 300);
        assert_eq!(e.active_voices(), 1);
    }

    #[test]
    fn gate_mode_releases_on_note_off() {
        let p = PadParams {
            gate: true,
            release_ms: 1.0,
            ..PadParams::default()
        };
        let (samples, settings) = setup(&[(0, p)]);
        let mut e = Engine::new(SR);
        e.note_on(0, 1.0, &samples, &settings);
        render(&mut e, &samples, &settings, 10);
        e.note_off(0, &settings);
        render(&mut e, &samples, &settings, 200);
        assert_eq!(e.active_voices(), 0);
    }

    #[test]
    fn steals_oldest_voice() {
        let (samples, settings) = setup(&[]);
        let mut e = Engine::new(SR);
        for i in 0..MAX_VOICES + 4 {
            e.note_on(i % NUM_PADS, 1.0, &samples, &settings);
        }
        assert_eq!(e.active_voices(), MAX_VOICES);
    }

    #[test]
    fn reverse_and_pitch_change_length() {
        let p = PadParams {
            reverse: true,
            tune: 12.0,
            ..PadParams::default()
        };
        let (samples, settings) = setup(&[(0, p)]);
        let mut e = Engine::new(SR);
        e.note_on(0, 1.0, &samples, &settings);
        // One octave up: 4800 frames play in 2400
        render(&mut e, &samples, &settings, 2390);
        assert_eq!(e.active_voices(), 1);
        render(&mut e, &samples, &settings, 20);
        assert_eq!(e.active_voices(), 0);
    }
}
