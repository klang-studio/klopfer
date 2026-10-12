//! Voice management: layer selection, allocation, stealing, choke groups and output routing.

use std::sync::Arc;

use crate::layers::{select_layers, LayerCursor, PadLayers, NUM_LAYERS};
use crate::modulation::Rng;
use crate::pad::PadSettings;
use crate::sample::Sample;
use crate::voice::{Voice, VoiceStart};

pub const NUM_PADS: usize = 32;
/// Pads shown at once in the pad grid
pub const PADS_PER_BANK: usize = 16;
/// One sample slot per pad layer, indexed by [`slot_index()`]
pub const NUM_SAMPLE_SLOTS: usize = NUM_PADS * NUM_LAYERS;
/// Stereo output buses: main + 15 aux
pub const NUM_OUTPUTS: usize = 16;
pub const MAX_VOICES: usize = 64;

/// Fade time for choked voices
const CHOKE_FADE_SECS: f32 = 0.005;

#[inline]
pub fn slot_index(pad: usize, layer: usize) -> usize {
    pad * NUM_LAYERS + layer
}

/// Where the engine gets the sample for a slot from.
pub trait SampleSource {
    fn sample(&self, slot: usize) -> Option<&Sample>;
}

impl SampleSource for [Option<Arc<Sample>>] {
    #[inline]
    fn sample(&self, slot: usize) -> Option<&Sample> {
        self.get(slot).and_then(|s| s.as_deref())
    }
}

impl SampleSource for Vec<Option<Arc<Sample>>> {
    #[inline]
    fn sample(&self, slot: usize) -> Option<&Sample> {
        self.as_slice().sample(slot)
    }
}

pub struct Engine {
    voices: [Voice; MAX_VOICES],
    sample_rate: f32,
    choke_step: f32,
    next_id: u64,
    rng: Rng,
    cursors: [LayerCursor; NUM_PADS],
    /// Free-running LFO phase per pad, for LFOs that don't retrigger
    lfo_phases: [f32; NUM_PADS],
    pad_peaks: [f32; NUM_PADS],
}

impl Engine {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            voices: [Voice::default(); MAX_VOICES],
            sample_rate,
            choke_step: 1.0 / (CHOKE_FADE_SECS * sample_rate),
            next_id: 0,
            rng: Rng::default(),
            cursors: [LayerCursor::default(); NUM_PADS],
            lfo_phases: [0.0; NUM_PADS],
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

    /// Sets a pad's free-running LFO phase, e.g. from the host's transport for synced LFOs.
    pub fn set_lfo_phase(&mut self, pad: usize, phase: f32) {
        self.lfo_phases[pad] = phase.rem_euclid(1.0);
    }

    /// Advances the free-running LFO phases by `frames` samples.
    pub fn advance_lfo_phases(&mut self, settings: &[PadSettings], frames: usize) {
        for (phase, s) in self.lfo_phases.iter_mut().zip(settings) {
            *phase = (*phase + s.lfo_rate_hz * frames as f32 / self.sample_rate).fract();
        }
    }

    /// Plays `pad`, choosing its layers by velocity and layer mode.
    pub fn note_on<S: SampleSource + ?Sized>(
        &mut self,
        pad: usize,
        velocity: f32,
        samples: &S,
        settings: &[PadSettings],
        layers: &[PadLayers],
    ) {
        let mut pad_layers = layers[pad];
        for (layer, info) in pad_layers.layers.iter_mut().enumerate() {
            info.loaded = info.loaded && samples.sample(slot_index(pad, layer)).is_some();
        }
        let play = select_layers(&pad_layers, velocity, &mut self.cursors[pad], &mut self.rng);
        if !play.iter().any(|p| *p) {
            return;
        }

        let s = &settings[pad];
        if s.choke_group > 0 {
            for v in self.voices.iter_mut().filter(|v| v.active && v.pad != pad) {
                if settings[v.pad].choke_group == s.choke_group {
                    v.choke();
                }
            }
        }

        for (layer, info) in pad_layers.layers.iter().enumerate() {
            if !play[layer] {
                continue;
            }
            let slot = slot_index(pad, layer);
            let Some(sample) = samples.sample(slot) else {
                continue;
            };

            let index = self.free_voice();
            self.next_id += 1;
            let seed = (self.next_id as u32).wrapping_mul(0x9e37_79b9) ^ 0x5bd1_e995;
            let start = VoiceStart {
                pad,
                slot,
                velocity,
                layer_gain: info.gain,
                layer_tune: info.tune,
                lfo_phase: self.lfo_phases[pad],
            };
            self.voices[index].start(self.next_id, start, sample, s, seed);
        }
    }

    fn free_voice(&self) -> usize {
        match self.voices.iter().position(|v| !v.active) {
            Some(i) => i,
            None => self
                .voices
                .iter()
                .enumerate()
                .min_by_key(|(_, v)| v.id)
                .map(|(i, _)| i)
                .unwrap_or(0),
        }
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

    /// Immediately stops all voices playing sample `slot`. Needed before the sample gets replaced.
    pub fn kill_slot(&mut self, slot: usize) {
        for v in self.voices.iter_mut().filter(|v| v.slot == slot) {
            v.active = false;
        }
    }

    /// Renders one frame and adds it to the output buses.
    #[inline]
    pub fn render_frame<S: SampleSource + ?Sized>(
        &mut self,
        samples: &S,
        settings: &[PadSettings],
        bus: &mut [[f32; 2]; NUM_OUTPUTS],
    ) {
        for v in self.voices.iter_mut().filter(|v| v.active) {
            let Some(sample) = samples.sample(v.slot) else {
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
    use crate::layers::{LayerInfo, LayerMode};
    use crate::modulation::{ModSlot, ModSource, ModTarget};
    use crate::pad::PadParams;

    const SR: f32 = 48000.0;

    struct Setup {
        samples: Vec<Option<Arc<Sample>>>,
        settings: Vec<PadSettings>,
        layers: Vec<PadLayers>,
    }

    /// Every pad gets one constant 0.5 sample on layer 0.
    fn setup(params: &[(usize, PadParams)]) -> Setup {
        let mut samples = vec![None; NUM_SAMPLE_SLOTS];
        let mut layers = vec![PadLayers::default(); NUM_PADS];
        for pad in 0..NUM_PADS {
            samples[slot_index(pad, 0)] = Some(Arc::new(Sample::mono(SR, vec![0.5; 4800])));
            layers[pad].layers[0].loaded = true;
        }
        let mut settings = vec![PadSettings::new(&PadParams::default(), SR); NUM_PADS];
        for (pad, p) in params {
            settings[*pad] = PadSettings::new(p, SR);
        }
        Setup {
            samples,
            settings,
            layers,
        }
    }

    impl Setup {
        fn hit(&self, e: &mut Engine, pad: usize, velocity: f32) {
            e.note_on(pad, velocity, &self.samples, &self.settings, &self.layers);
        }

        fn render(&self, e: &mut Engine, n: usize) -> [[f32; 2]; NUM_OUTPUTS] {
            let mut bus = [[0.0; 2]; NUM_OUTPUTS];
            for _ in 0..n {
                bus = [[0.0; 2]; NUM_OUTPUTS];
                e.render_frame(&self.samples, &self.settings, &mut bus);
            }
            bus
        }
    }

    #[test]
    fn plays_sample_until_end() {
        let t = setup(&[]);
        let mut e = Engine::new(SR);
        t.hit(&mut e, 0, 1.0);
        let bus = t.render(&mut e, 10);
        assert!((bus[0][0] - 0.5).abs() < 1e-3);
        t.render(&mut e, 4800);
        assert_eq!(e.active_voices(), 0);
    }

    #[test]
    fn velocity_scales_level() {
        let t = setup(&[]);
        let mut e = Engine::new(SR);
        t.hit(&mut e, 0, 0.5);
        let bus = t.render(&mut e, 10);
        assert!((bus[0][0] - 0.25).abs() < 1e-3);
    }

    #[test]
    fn routes_to_selected_output() {
        let p = PadParams {
            output: 3,
            ..PadParams::default()
        };
        let t = setup(&[(1, p)]);
        let mut e = Engine::new(SR);
        t.hit(&mut e, 1, 1.0);
        let bus = t.render(&mut e, 10);
        assert_eq!(bus[0], [0.0, 0.0]);
        assert!(bus[3][0] > 0.4);
    }

    #[test]
    fn choke_group_silences_other_pads() {
        let p = PadParams {
            choke_group: 1,
            ..PadParams::default()
        };
        let t = setup(&[(2, p), (3, p)]);
        let mut e = Engine::new(SR);
        t.hit(&mut e, 2, 1.0);
        t.render(&mut e, 10);
        t.hit(&mut e, 3, 1.0);
        // 5 ms fade at 48 kHz
        t.render(&mut e, 300);
        assert_eq!(e.active_voices(), 1);
    }

    #[test]
    fn gate_mode_releases_on_note_off() {
        let mut p = PadParams {
            gate: true,
            ..PadParams::default()
        };
        p.amp.release_ms = 1.0;
        let t = setup(&[(0, p)]);
        let mut e = Engine::new(SR);
        t.hit(&mut e, 0, 1.0);
        t.render(&mut e, 10);
        e.note_off(0, &t.settings);
        t.render(&mut e, 200);
        assert_eq!(e.active_voices(), 0);
    }

    #[test]
    fn steals_oldest_voice() {
        let t = setup(&[]);
        let mut e = Engine::new(SR);
        for i in 0..MAX_VOICES + 4 {
            t.hit(&mut e, i % NUM_PADS, 1.0);
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
        let t = setup(&[(0, p)]);
        let mut e = Engine::new(SR);
        t.hit(&mut e, 0, 1.0);
        // One octave up: 4800 frames play in 2400
        t.render(&mut e, 2390);
        assert_eq!(e.active_voices(), 1);
        t.render(&mut e, 20);
        assert_eq!(e.active_voices(), 0);
    }

    #[test]
    fn velocity_layers_pick_their_sample() {
        let mut t = setup(&[]);
        t.samples[slot_index(0, 1)] = Some(Arc::new(Sample::mono(SR, vec![0.25; 4800])));
        t.layers[0].layers[0] = LayerInfo {
            loaded: true,
            vel_lo: 64,
            vel_hi: 127,
            ..LayerInfo::default()
        };
        t.layers[0].layers[1] = LayerInfo {
            loaded: true,
            vel_lo: 1,
            vel_hi: 63,
            gain: 2.0,
            ..LayerInfo::default()
        };
        let p = PadParams {
            velocity_sens: 0.0,
            ..PadParams::default()
        };
        t.settings[0] = PadSettings::new(&p, SR);

        let mut e = Engine::new(SR);
        t.hit(&mut e, 0, 0.2);
        // Soft layer: 0.25 * layer gain 2.0
        assert!((t.render(&mut e, 10)[0][0] - 0.5).abs() < 1e-3);
        assert_eq!(e.active_voices(), 1);
    }

    #[test]
    fn velocity_mode_layers_overlapping_ranges() {
        let mut t = setup(&[]);
        t.samples[slot_index(0, 1)] = Some(Arc::new(Sample::mono(SR, vec![0.25; 4800])));
        t.layers[0].layers[1].loaded = true;
        t.layers[0].mode = LayerMode::Velocity;
        let mut e = Engine::new(SR);
        t.hit(&mut e, 0, 1.0);
        assert!((t.render(&mut e, 10)[0][0] - 0.75).abs() < 1e-3);
    }

    #[test]
    fn matrix_modulates_volume() {
        let mut p = PadParams::default();
        p.slots[0] = ModSlot {
            source: ModSource::Velocity,
            target: ModTarget::Volume,
            amount: -1.0,
        };
        p.velocity_sens = 0.0;
        let t = setup(&[(0, p)]);
        let mut e = Engine::new(SR);
        // Velocity 1.0 * -1.0: volume factor 0
        t.hit(&mut e, 0, 1.0);
        assert!(t.render(&mut e, 10)[0][0].abs() < 1e-6);
    }

    #[test]
    fn note_on_start_modulation_shortens_playback() {
        let mut p = PadParams::default();
        p.slots[0] = ModSlot {
            source: ModSource::Velocity,
            target: ModTarget::SampleStart,
            amount: 0.5,
        };
        let t = setup(&[(0, p)]);
        let mut e = Engine::new(SR);
        t.hit(&mut e, 0, 1.0);
        // Starts halfway: 2400 frames left
        t.render(&mut e, 2390);
        assert_eq!(e.active_voices(), 1);
        t.render(&mut e, 20);
        assert_eq!(e.active_voices(), 0);
    }
}
