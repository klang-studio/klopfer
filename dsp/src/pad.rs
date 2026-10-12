//! Per-pad settings. [`PadParams`] holds user-facing values, [`PadSettings`] the derived,
//! sample-rate dependent coefficients the voices use.

use crate::biquad::BiquadCoeffs;
use crate::envelope::{exp_coeff, AhdsrCoeffs};
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
    /// Gate mode releases the amp envelope on note off, one-shot mode ignores note offs
    pub gate: bool,

    pub attack_ms: f32,
    pub hold_ms: f32,
    pub decay_ms: f32,
    /// `0..1`
    pub sustain: f32,
    pub release_ms: f32,
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
            attack_ms: 0.0,
            hold_ms: 0.0,
            decay_ms: 500.0,
            sustain: 1.0,
            release_ms: 200.0,
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
            choke_group: 0,
            output: 0,
        }
    }
}

/// Derived per-pad values, recomputed once per processing block.
#[derive(Debug, Clone, Copy)]
pub struct PadSettings {
    pub gain: f32,
    pub pan: f32,
    pub tune: f32,
    pub start: f32,
    pub end: f32,
    pub reverse: bool,
    pub gate: bool,
    pub amp: AhdsrCoeffs,
    pub velocity_sens: f32,

    pub filter: Option<SvfMode>,
    pub cutoff_hz: f32,
    pub resonance: f32,
    pub filter_env_octaves: f32,
    pub filter_env_coeff: f32,
    /// Pre-gain into the saturator, `1.0` means no drive
    pub drive_gain: f32,

    pub pitch_env_semis: f32,
    pub pitch_env_coeff: f32,

    pub eq: BiquadCoeffs,

    pub bits: f32,
    pub downsample: f32,

    pub choke_group: u8,
    pub output: usize,
}

impl PadSettings {
    pub fn new(p: &PadParams, sample_rate: f32) -> Self {
        let ms = |v: f32| v / 1000.0;
        let (start, end) = if p.start <= p.end {
            (p.start, p.end)
        } else {
            (p.end, p.start)
        };

        Self {
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
            amp: AhdsrCoeffs::new(
                ms(p.attack_ms),
                ms(p.hold_ms),
                ms(p.decay_ms),
                p.sustain,
                ms(p.release_ms),
                sample_rate,
            ),
            velocity_sens: p.velocity_sens.clamp(0.0, 1.0),
            filter: p.filter_mode.svf_mode(),
            cutoff_hz: p.cutoff_hz,
            resonance: p.resonance,
            filter_env_octaves: p.filter_env_octaves,
            filter_env_coeff: exp_coeff(ms(p.filter_env_decay_ms), sample_rate),
            drive_gain: 1.0 + 15.0 * p.drive.clamp(0.0, 1.0).powi(2),
            pitch_env_semis: p.pitch_env_semis,
            pitch_env_coeff: exp_coeff(ms(p.pitch_env_decay_ms), sample_rate),
            eq: BiquadCoeffs::peaking(p.eq_freq_hz, p.eq_q, p.eq_gain_db, sample_rate),
            bits: p.bits,
            downsample: p.downsample,
            choke_group: p.choke_group,
            output: p.output as usize,
        }
    }
}

impl Default for PadSettings {
    fn default() -> Self {
        Self::new(&PadParams::default(), 44100.0)
    }
}
