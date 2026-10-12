//! Per-pad settings. [`PadParams`] holds user-facing values, [`PadSettings`] the derived,
//! sample-rate dependent coefficients the voices use.

use crate::biquad::BiquadCoeffs;
use crate::envelope::{exp_coeff, AhdsrCoeffs};
use crate::modulation::{LfoShape, ModSlot, NUM_SLOTS};
use crate::svf::SvfMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FilterMode {
    #[default]
    Off,
    LowPass,
    HighPass,
    BandPass,
}

impl FilterMode {
    fn svf_mode(self) -> Option<SvfMode> {
        match self {
            FilterMode::Off => None,
            FilterMode::LowPass => Some(SvfMode::LowPass),
            FilterMode::HighPass => Some(SvfMode::HighPass),
            FilterMode::BandPass => Some(SvfMode::BandPass),
        }
    }
}

/// AHDSR times in milliseconds, sustain `0..1`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EnvParams {
    pub attack_ms: f32,
    pub hold_ms: f32,
    pub decay_ms: f32,
    pub sustain: f32,
    pub release_ms: f32,
}

impl EnvParams {
    pub fn coeffs(&self, sample_rate: f32) -> AhdsrCoeffs {
        AhdsrCoeffs::new(
            self.attack_ms / 1000.0,
            self.hold_ms / 1000.0,
            self.decay_ms / 1000.0,
            self.sustain,
            self.release_ms / 1000.0,
            sample_rate,
        )
    }
}

impl Default for EnvParams {
    fn default() -> Self {
        Self {
            attack_ms: 0.0,
            hold_ms: 0.0,
            decay_ms: 500.0,
            sustain: 1.0,
            release_ms: 200.0,
        }
    }
}

/// User-facing pad parameters in their natural units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PadParams {
    pub volume_db: f32,
    /// `-1` (left) to `1` (right)
    pub pan: f32,
    pub mute: bool,
    /// Coarse + fine tuning in semitones
    pub tune: f32,
    /// Normalized playback region, `start < end`
    pub start: f32,
    pub end: f32,
    pub reverse: bool,
    /// Gate mode releases the envelopes on note off, one-shot mode ignores note offs
    pub gate: bool,

    pub amp: EnvParams,
    /// `0..1`: how much velocity affects the level
    pub velocity_sens: f32,

    pub filter_mode: FilterMode,
    pub cutoff_hz: f32,
    /// `0..1`
    pub resonance: f32,
    /// Filter envelope depth in octaves
    pub filter_env_octaves: f32,
    pub filter_env_decay_ms: f32,
    /// `0..1`
    pub drive: f32,

    /// Pitch envelope depth in semitones
    pub pitch_env_semis: f32,
    pub pitch_env_decay_ms: f32,

    pub eq_freq_hz: f32,
    pub eq_gain_db: f32,
    pub eq_q: f32,

    /// 24 = off
    pub bits: f32,
    /// Sample-and-hold factor, 1 = off
    pub downsample: f32,

    pub lfo_shape: LfoShape,
    /// Already resolved from the tempo when synced
    pub lfo_rate_hz: f32,
    /// Restart the LFO on every hit instead of running freely
    pub lfo_retrigger: bool,
    pub mod_env1: EnvParams,
    pub mod_env2: EnvParams,
    pub slots: [ModSlot; NUM_SLOTS],

    /// `0` = no choke group
    pub choke_group: u8,
    /// Output bus index, `0` = main
    pub output: u8,
}

impl Default for PadParams {
    fn default() -> Self {
        Self {
            volume_db: 0.0,
            pan: 0.0,
            mute: false,
            tune: 0.0,
            start: 0.0,
            end: 1.0,
            reverse: false,
            gate: false,
            amp: EnvParams::default(),
            velocity_sens: 1.0,
            filter_mode: FilterMode::Off,
            cutoff_hz: 20000.0,
            resonance: 0.0,
            filter_env_octaves: 0.0,
            filter_env_decay_ms: 200.0,
            drive: 0.0,
            pitch_env_semis: 0.0,
            pitch_env_decay_ms: 50.0,
            eq_freq_hz: 1000.0,
            eq_gain_db: 0.0,
            eq_q: 0.7,
            bits: 24.0,
            downsample: 1.0,
            lfo_shape: LfoShape::Sine,
            lfo_rate_hz: 2.0,
            lfo_retrigger: true,
            mod_env1: EnvParams::default(),
            mod_env2: EnvParams::default(),
            slots: [ModSlot::default(); NUM_SLOTS],
            choke_group: 0,
            output: 0,
        }
    }
}

/// Derived per-pad values, recomputed once per processing block.
#[derive(Debug, Clone, Copy)]
pub struct PadSettings {
    pub sample_rate: f32,
    pub gain: f32,
    pub pan: f32,
    pub tune: f32,
    pub start: f32,
    pub end: f32,
    pub reverse: bool,
    pub gate: bool,
    pub amp_params: EnvParams,
    pub amp: AhdsrCoeffs,
    pub velocity_sens: f32,

    pub filter: Option<SvfMode>,
    pub cutoff_hz: f32,
    pub resonance: f32,
    pub filter_env_octaves: f32,
    pub filter_env_coeff: f32,
    /// `0..1`
    pub drive: f32,

    pub pitch_env_semis: f32,
    pub pitch_env_coeff: f32,

    pub eq: BiquadCoeffs,

    pub bits: f32,
    pub downsample: f32,

    pub lfo_shape: LfoShape,
    pub lfo_rate_hz: f32,
    pub lfo_retrigger: bool,
    pub mod_env1: AhdsrCoeffs,
    pub mod_env2: AhdsrCoeffs,
    /// The active slots are moved to the front, `num_slots` of them
    pub slots: [ModSlot; NUM_SLOTS],
    pub num_slots: usize,
    /// Whether any active slot targets a note-on-only destination
    pub has_note_on_mod: bool,

    pub choke_group: u8,
    pub output: usize,
}

impl PadSettings {
    pub fn new(p: &PadParams, sample_rate: f32) -> Self {
        let (start, end) = if p.start <= p.end {
            (p.start, p.end)
        } else {
            (p.end, p.start)
        };

        let mut slots = [ModSlot::default(); NUM_SLOTS];
        let mut num_slots = 0;
        for slot in p.slots.iter().filter(|s| s.is_active()) {
            slots[num_slots] = *slot;
            num_slots += 1;
        }
        let has_note_on_mod = slots[..num_slots]
            .iter()
            .any(|s| s.target.is_note_on_only());

        Self {
            sample_rate,
            gain: if p.mute {
                0.0
            } else {
                10f32.powf(p.volume_db / 20.0)
            },
            pan: p.pan.clamp(-1.0, 1.0),
            tune: p.tune,
            start: start.clamp(0.0, 1.0),
            end: end.clamp(0.0, 1.0),
            reverse: p.reverse,
            gate: p.gate,
            amp_params: p.amp,
            amp: p.amp.coeffs(sample_rate),
            velocity_sens: p.velocity_sens.clamp(0.0, 1.0),
            filter: p.filter_mode.svf_mode(),
            cutoff_hz: p.cutoff_hz,
            resonance: p.resonance,
            filter_env_octaves: p.filter_env_octaves,
            filter_env_coeff: exp_coeff(p.filter_env_decay_ms / 1000.0, sample_rate),
            drive: p.drive.clamp(0.0, 1.0),
            pitch_env_semis: p.pitch_env_semis,
            pitch_env_coeff: exp_coeff(p.pitch_env_decay_ms / 1000.0, sample_rate),
            eq: BiquadCoeffs::peaking(p.eq_freq_hz, p.eq_q, p.eq_gain_db, sample_rate),
            bits: p.bits,
            downsample: p.downsample,
            lfo_shape: p.lfo_shape,
            lfo_rate_hz: p.lfo_rate_hz,
            lfo_retrigger: p.lfo_retrigger,
            mod_env1: p.mod_env1.coeffs(sample_rate),
            mod_env2: p.mod_env2.coeffs(sample_rate),
            slots,
            num_slots,
            has_note_on_mod,
            choke_group: p.choke_group,
            output: p.output as usize,
        }
    }

    #[inline]
    pub fn active_slots(&self) -> &[ModSlot] {
        &self.slots[..self.num_slots]
    }
}

impl Default for PadSettings {
    fn default() -> Self {
        Self::new(&PadParams::default(), 44100.0)
    }
}
