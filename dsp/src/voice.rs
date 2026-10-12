//! A single playing sample.

use crate::biquad::Biquad;
use crate::envelope::{Ahdsr, DecayEnv};
use crate::lofi::LoFi;
use crate::pad::PadSettings;
use crate::sample::Sample;
use crate::svf::Svf;

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Voice {
    pub active: bool,
    pub pad: usize,
    /// Monotonic trigger counter, used to find the oldest voice when stealing
    pub id: u64,

    pos: f64,
    velocity_gain: f32,
    /// `1.0` while playing normally, fades to `0.0` when choked
    choke_gain: f32,
    choking: bool,

    amp: Ahdsr,
    pitch_env: DecayEnv,
    filter_env: DecayEnv,
    lofi: LoFi,
    svf: Svf,
    eq: Biquad,
}

impl Voice {
    pub fn start(&mut self, pad: usize, id: u64, velocity: f32, sample: &Sample, s: &PadSettings) {
        let len = sample.len() as f64;
        *self = Self {
            active: true,
            pad,
            id,
            pos: if s.reverse {
                s.end as f64 * len - 1.0
            } else {
                s.start as f64 * len
            },
            velocity_gain: 1.0 - s.velocity_sens * (1.0 - velocity.clamp(0.0, 1.0)),
            choke_gain: 1.0,
            ..Self::default()
        };
        self.amp.trigger();
        self.pitch_env.trigger();
        self.filter_env.trigger();
    }

    pub fn release(&mut self) {
        self.amp.release();
    }

    pub fn choke(&mut self) {
        self.choking = true;
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
        let len = sample.len() as f64;
        let (start, end) = (s.start as f64 * len, s.end as f64 * len);
        if self.pos < start || self.pos >= end || end - start < 1.0 {
            self.active = false;
            return (0.0, 0.0);
        }

        let mut x = sample.read(self.pos);

        let pitch = s.tune + s.pitch_env_semis * self.pitch_env.next(s.pitch_env_coeff);
        let rate = (sample.sample_rate / sample_rate) as f64 * (pitch / 12.0).exp2() as f64;
        if s.reverse {
            self.pos -= rate;
        } else {
            self.pos += rate;
        }

        x = self.lofi.process(s.bits, s.downsample, x);

        if s.drive_gain > 1.0 {
            let k = s.drive_gain;
            let norm = k.sqrt().recip();
            x = ((x.0 * k).tanh() * norm, (x.1 * k).tanh() * norm);
        }

        let filter_env = self.filter_env.next(s.filter_env_coeff);
        if let Some(mode) = s.filter {
            let cutoff = s.cutoff_hz * (s.filter_env_octaves * filter_env).exp2();
            x = self.svf.process(mode, cutoff, s.resonance, sample_rate, x);
        }

        x = self.eq.process(&s.eq, x);

        let amp = self.amp.next(&s.amp);
        if self.choking {
            self.choke_gain -= choke_step;
        }
        if self.amp.is_idle() || self.choke_gain <= 0.0 {
            self.active = false;
            return (0.0, 0.0);
        }

        let gain = amp * self.velocity_gain * self.choke_gain * s.gain;
        let pan_l = (1.0 - s.pan).min(1.0);
        let pan_r = (1.0 + s.pan).min(1.0);
        (x.0 * gain * pan_l, x.1 * gain * pan_r)
    }
}
