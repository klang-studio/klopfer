//! A single playing sample.

use crate::biquad::Biquad;
use crate::envelope::{Ahdsr, AhdsrCoeffs, DecayEnv};
use crate::lofi::LoFi;
use crate::modulation::{
    Lfo, ModValues, Rng, Sources, BITS_RANGE, CUTOFF_RANGE_OCT, DOWNSAMPLE_RANGE_OCT,
    LFO_RATE_RANGE_OCT, PITCH_RANGE_SEMIS, TIME_RANGE_OCT,
};
use crate::pad::PadSettings;
use crate::sample::Sample;
use crate::svf::Svf;

/// What a voice plays: a pad, one of its layers, and the layer's offsets.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct VoiceStart {
    pub pad: usize,
    /// Index into the engine's sample slots
    pub slot: usize,
    pub velocity: f32,
    pub layer_gain: f32,
    pub layer_tune: f32,
    pub lfo_phase: f32,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Voice {
    pub active: bool,
    pub pad: usize,
    pub slot: usize,
    /// Monotonic trigger counter, used to find the oldest voice when stealing
    pub id: u64,

    pos: f64,
    velocity: f32,
    velocity_gain: f32,
    layer_gain: f32,
    layer_tune: f32,
    /// Per-note value for the random mod source
    random: f32,
    /// Normalized start offset from note-on modulation
    start_offset: f32,
    /// Amp envelope coefficients when attack/decay are modulated at note on
    own_amp: Option<AhdsrCoeffs>,
    /// `1.0` while playing normally, fades to `0.0` when choked
    choke_gain: f32,
    choking: bool,
    /// LFO rate modulation from the previous sample (the LFO can't modulate itself instantly)
    lfo_rate_mod: f32,

    rng: Rng,
    lfo: Lfo,
    amp: Ahdsr,
    env1: Ahdsr,
    env2: Ahdsr,
    pitch_env: DecayEnv,
    filter_env: DecayEnv,
    lofi: LoFi,
    svf: Svf,
    eq: Biquad,
}

impl Voice {
    pub fn start(&mut self, id: u64, v: VoiceStart, sample: &Sample, s: &PadSettings, seed: u32) {
        let mut rng = Rng::new(seed);
        *self = Self {
            active: true,
            pad: v.pad,
            slot: v.slot,
            id,
            velocity: v.velocity.clamp(0.0, 1.0),
            velocity_gain: 1.0 - s.velocity_sens * (1.0 - v.velocity.clamp(0.0, 1.0)),
            layer_gain: v.layer_gain,
            layer_tune: v.layer_tune,
            random: rng.unipolar(),
            choke_gain: 1.0,
            ..Self::default()
        };
        self.lfo
            .reset(if s.lfo_retrigger { 0.0 } else { v.lfo_phase }, &mut rng);
        self.rng = rng;

        if s.has_note_on_mod {
            // Sources as they are at the very first sample
            let m = ModValues::evaluate(
                s.active_slots(),
                &self.sources(self.lfo.value(s.lfo_shape), 0.0, 0.0),
            );
            self.start_offset = m.start;
            if m.amp_attack != 0.0 || m.amp_decay != 0.0 {
                let mut p = s.amp_params;
                p.attack_ms *= (m.amp_attack * TIME_RANGE_OCT).exp2();
                p.decay_ms *= (m.amp_decay * TIME_RANGE_OCT).exp2();
                self.own_amp = Some(p.coeffs(s.sample_rate));
            }
        }

        let len = sample.len() as f64;
        self.pos = if s.reverse {
            s.end as f64 * len - 1.0
        } else {
            (s.start + self.start_offset).clamp(0.0, 1.0) as f64 * len
        };

        self.amp.trigger();
        self.env1.trigger();
        self.env2.trigger();
        self.pitch_env.trigger();
        self.filter_env.trigger();
    }

    /// Playback position in frames of the playing sample.
    pub fn position(&self) -> f64 {
        self.pos
    }

    pub fn release(&mut self) {
        self.amp.release();
        self.env1.release();
        self.env2.release();
    }

    pub fn choke(&mut self) {
        self.choking = true;
    }

    #[inline]
    fn sources(&self, lfo: f32, env1: f32, env2: f32) -> Sources {
        Sources {
            lfo,
            env1,
            env2,
            amp: self.amp.level(),
            velocity: self.velocity,
            random: self.random,
            noise: 0.0,
        }
    }

    /// Renders one frame. Deactivates the voice once it has finished.
    #[inline]
    pub fn render(
        &mut self,
        sample: &Sample,
        s: &PadSettings,
        sample_rate: f32,
        choke_step: f32,
    ) -> (f32, f32) {
        // Modulation sources and matrix
        let m = if s.num_slots > 0 {
            let lfo_rate = s.lfo_rate_hz * (self.lfo_rate_mod * LFO_RATE_RANGE_OCT).exp2();
            let lfo = self
                .lfo
                .next(s.lfo_shape, lfo_rate, sample_rate, &mut self.rng);
            let env1 = self.env1.next(&s.mod_env1);
            let env2 = self.env2.next(&s.mod_env2);
            let mut sources = self.sources(lfo, env1, env2);
            sources.noise = self.rng.bipolar();
            let m = ModValues::evaluate(s.active_slots(), &sources);
            self.lfo_rate_mod = m.lfo_rate;
            m
        } else {
            ModValues::default()
        };

        let len = sample.len() as f64;
        let start = (s.start + self.start_offset).clamp(0.0, 1.0) as f64 * len;
        let end = (s.end + m.end).clamp(0.0, 1.0) as f64 * len;
        if self.pos < start || self.pos >= end || end - start < 1.0 {
            self.active = false;
            return (0.0, 0.0);
        }

        let mut x = sample.read(self.pos);

        let pitch = s.tune + self.layer_tune - sample.baked_pitch
            + s.pitch_env_semis * self.pitch_env.next(s.pitch_env_coeff)
            + m.pitch * PITCH_RANGE_SEMIS;
        let rate = (sample.sample_rate / sample_rate) as f64 * (pitch / 12.0).exp2() as f64;
        if s.reverse {
            self.pos -= rate;
        } else {
            self.pos += rate;
        }

        let bits = (s.bits + m.bits * BITS_RANGE).clamp(2.0, 24.0);
        let downsample =
            (s.downsample * (m.downsample * DOWNSAMPLE_RANGE_OCT).exp2()).clamp(1.0, 64.0);
        x = self.lofi.process(bits, downsample, x);

        let drive = (s.drive + m.drive).clamp(0.0, 1.0);
        if drive > 0.0 {
            let k = 1.0 + 15.0 * drive * drive;
            let norm = k.sqrt().recip();
            x = ((x.0 * k).tanh() * norm, (x.1 * k).tanh() * norm);
        }

        let filter_env = self.filter_env.next(s.filter_env_coeff);
        if let Some(mode) = s.filter {
            let cutoff = s.cutoff_hz
                * (s.filter_env_octaves * filter_env + m.cutoff * CUTOFF_RANGE_OCT).exp2();
            let resonance = (s.resonance + m.resonance).clamp(0.0, 1.0);
            x = self.svf.process(mode, cutoff, resonance, sample_rate, x);
        }

        x = self.eq.process(&s.eq, x);

        let amp = match &self.own_amp {
            Some(c) => self.amp.next(c),
            None => self.amp.next(&s.amp),
        };
        if self.choking {
            self.choke_gain -= choke_step;
        }
        if self.amp.is_idle() || self.choke_gain <= 0.0 {
            self.active = false;
            return (0.0, 0.0);
        }

        let gain = amp
            * self.velocity_gain
            * self.layer_gain
            * self.choke_gain
            * s.gain
            * (1.0 + m.volume).max(0.0);
        let pan = (s.pan + m.pan).clamp(-1.0, 1.0);
        let pan_l = (1.0 - pan).min(1.0);
        let pan_r = (1.0 + pan).min(1.0);
        (x.0 * gain * pan_l, x.1 * gain * pan_r)
    }
}
