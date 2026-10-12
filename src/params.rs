//! Plugin parameters. Parameter IDs are part of saved projects and must never change.

use klopfer_dsp::modulation::NUM_SLOTS;
use klopfer_dsp::{
    EnvParams, FilterMode, LfoShape, ModSlot, ModSource, ModTarget, NUM_OUTPUTS, NUM_PADS,
};
use nih_plug::prelude::*;
use nih_plug_egui::EguiState;
use parking_lot::RwLock;
use std::sync::Arc;

use crate::layers::LayerConfig;
use crate::midi_learn::Overrides;

/// First MIDI note of the pad grid, C1 in the General MIDI drum map.
pub const BASE_NOTE: i32 = 36;

/// Note name with middle C (60) as C3, the convention of the GM drum map.
pub fn note_name(note: i32) -> String {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    format!(
        "{}{}",
        NAMES[note.rem_euclid(12) as usize],
        note.div_euclid(12) - 2
    )
}

#[derive(Params)]
pub struct KlopferParams {
    #[persist = "editor-state"]
    pub editor_state: Arc<EguiState>,

    #[persist = "kit-name"]
    pub kit_name: RwLock<String>,

    /// MIDI learn assignments as `(cc, parameter id)`
    #[persist = "midi-map"]
    pub midi_map: RwLock<Vec<(u8, String)>>,

    #[id = "master"]
    pub master: FloatParam,

    /// Boxed: with all pads the parameters are several hundred KB, too much for the stack
    #[nested(array, group = "Pad")]
    pub pads: Box<[PadParams]>,
}

impl Default for KlopferParams {
    fn default() -> Self {
        Self {
            editor_state: EguiState::from_size(crate::editor::WIDTH, crate::editor::HEIGHT),
            kit_name: RwLock::new(String::from("Init")),
            midi_map: RwLock::new(Vec::new()),
            master: FloatParam::new(
                "Master",
                util::db_to_gain(0.0),
                FloatRange::Skewed {
                    min: util::db_to_gain(-60.0),
                    max: util::db_to_gain(12.0),
                    factor: FloatRange::gain_skew_factor(-60.0, 12.0),
                },
            )
            .with_unit(" dB")
            .with_value_to_string(formatters::v2s_f32_gain_to_db(1))
            .with_string_to_value(formatters::s2v_f32_gain_to_db()),
            pads: (0..NUM_PADS).map(PadParams::new).collect(),
        }
    }
}

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayMode {
    #[name = "One Shot"]
    #[id = "one_shot"]
    OneShot,
    #[name = "Gate"]
    #[id = "gate"]
    Gate,
}

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterType {
    #[name = "Off"]
    #[id = "off"]
    Off,
    #[name = "Low Pass"]
    #[id = "low_pass"]
    LowPass,
    #[name = "High Pass"]
    #[id = "high_pass"]
    HighPass,
    #[name = "Band Pass"]
    #[id = "band_pass"]
    BandPass,
}

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum LfoShapeParam {
    #[name = "Sine"]
    #[id = "sine"]
    Sine,
    #[name = "Triangle"]
    #[id = "triangle"]
    Triangle,
    #[name = "Saw Up"]
    #[id = "saw_up"]
    SawUp,
    #[name = "Saw Down"]
    #[id = "saw_down"]
    SawDown,
    #[name = "Square"]
    #[id = "square"]
    Square,
    #[name = "S&H"]
    #[id = "sample_hold"]
    SampleHold,
}

impl From<LfoShapeParam> for LfoShape {
    fn from(s: LfoShapeParam) -> Self {
        match s {
            LfoShapeParam::Sine => LfoShape::Sine,
            LfoShapeParam::Triangle => LfoShape::Triangle,
            LfoShapeParam::SawUp => LfoShape::SawUp,
            LfoShapeParam::SawDown => LfoShape::SawDown,
            LfoShapeParam::Square => LfoShape::Square,
            LfoShapeParam::SampleHold => LfoShape::SampleHold,
        }
    }
}

/// Tempo-synced LFO cycle lengths.
#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncDivision {
    #[name = "4/1"]
    #[id = "bars4"]
    Bars4,
    #[name = "2/1"]
    #[id = "bars2"]
    Bars2,
    #[name = "1/1"]
    #[id = "whole"]
    Whole,
    #[name = "1/2"]
    #[id = "half"]
    Half,
    #[name = "1/4"]
    #[id = "quarter"]
    Quarter,
    #[name = "1/8"]
    #[id = "eighth"]
    Eighth,
    #[name = "1/16"]
    #[id = "sixteenth"]
    Sixteenth,
    #[name = "1/32"]
    #[id = "thirty_second"]
    ThirtySecond,
    #[name = "1/4 T"]
    #[id = "quarter_triplet"]
    QuarterTriplet,
    #[name = "1/8 T"]
    #[id = "eighth_triplet"]
    EighthTriplet,
    #[name = "1/16 T"]
    #[id = "sixteenth_triplet"]
    SixteenthTriplet,
    #[name = "1/4 D"]
    #[id = "quarter_dotted"]
    QuarterDotted,
    #[name = "1/8 D"]
    #[id = "eighth_dotted"]
    EighthDotted,
}

impl SyncDivision {
    /// Length of one LFO cycle in quarter notes.
    pub fn beats(self) -> f64 {
        match self {
            SyncDivision::Bars4 => 16.0,
            SyncDivision::Bars2 => 8.0,
            SyncDivision::Whole => 4.0,
            SyncDivision::Half => 2.0,
            SyncDivision::Quarter => 1.0,
            SyncDivision::Eighth => 0.5,
            SyncDivision::Sixteenth => 0.25,
            SyncDivision::ThirtySecond => 0.125,
            SyncDivision::QuarterTriplet => 2.0 / 3.0,
            SyncDivision::EighthTriplet => 1.0 / 3.0,
            SyncDivision::SixteenthTriplet => 1.0 / 6.0,
            SyncDivision::QuarterDotted => 1.5,
            SyncDivision::EighthDotted => 0.75,
        }
    }
}

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModSourceParam {
    #[name = "Off"]
    #[id = "off"]
    Off,
    #[name = "LFO"]
    #[id = "lfo"]
    Lfo,
    #[name = "Mod Env 1"]
    #[id = "mod_env1"]
    ModEnv1,
    #[name = "Mod Env 2"]
    #[id = "mod_env2"]
    ModEnv2,
    #[name = "Amp Env"]
    #[id = "amp_env"]
    AmpEnv,
    #[name = "Velocity"]
    #[id = "velocity"]
    Velocity,
    #[name = "Random"]
    #[id = "random"]
    Random,
    #[name = "Noise"]
    #[id = "noise"]
    Noise,
}

impl From<ModSourceParam> for ModSource {
    fn from(s: ModSourceParam) -> Self {
        match s {
            ModSourceParam::Off => ModSource::Off,
            ModSourceParam::Lfo => ModSource::Lfo,
            ModSourceParam::ModEnv1 => ModSource::ModEnv1,
            ModSourceParam::ModEnv2 => ModSource::ModEnv2,
            ModSourceParam::AmpEnv => ModSource::AmpEnv,
            ModSourceParam::Velocity => ModSource::Velocity,
            ModSourceParam::Random => ModSource::Random,
            ModSourceParam::Noise => ModSource::Noise,
        }
    }
}

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModTargetParam {
    #[name = "Off"]
    #[id = "off"]
    Off,
    #[name = "Pitch"]
    #[id = "pitch"]
    Pitch,
    #[name = "Cutoff"]
    #[id = "cutoff"]
    Cutoff,
    #[name = "Resonance"]
    #[id = "resonance"]
    Resonance,
    #[name = "Drive"]
    #[id = "drive"]
    Drive,
    #[name = "Pan"]
    #[id = "pan"]
    Pan,
    #[name = "Volume"]
    #[id = "volume"]
    Volume,
    #[name = "Start"]
    #[id = "sample_start"]
    SampleStart,
    #[name = "End"]
    #[id = "sample_end"]
    SampleEnd,
    #[name = "LFO Rate"]
    #[id = "lfo_rate"]
    LfoRate,
    #[name = "Bits"]
    #[id = "bits"]
    Bits,
    #[name = "Downsample"]
    #[id = "downsample"]
    Downsample,
    #[name = "Amp Attack"]
    #[id = "amp_attack"]
    AmpAttack,
    #[name = "Amp Decay"]
    #[id = "amp_decay"]
    AmpDecay,
}

impl From<ModTargetParam> for ModTarget {
    fn from(t: ModTargetParam) -> Self {
        match t {
            ModTargetParam::Off => ModTarget::Off,
            ModTargetParam::Pitch => ModTarget::Pitch,
            ModTargetParam::Cutoff => ModTarget::Cutoff,
            ModTargetParam::Resonance => ModTarget::Resonance,
            ModTargetParam::Drive => ModTarget::Drive,
            ModTargetParam::Pan => ModTarget::Pan,
            ModTargetParam::Volume => ModTarget::Volume,
            ModTargetParam::SampleStart => ModTarget::SampleStart,
            ModTargetParam::SampleEnd => ModTarget::SampleEnd,
            ModTargetParam::LfoRate => ModTarget::LfoRate,
            ModTargetParam::Bits => ModTarget::Bits,
            ModTargetParam::Downsample => ModTarget::Downsample,
            ModTargetParam::AmpAttack => ModTarget::AmpAttack,
            ModTargetParam::AmpDecay => ModTarget::AmpDecay,
        }
    }
}

impl From<FilterType> for FilterMode {
    fn from(t: FilterType) -> Self {
        match t {
            FilterType::Off => FilterMode::Off,
            FilterType::LowPass => FilterMode::LowPass,
            FilterType::HighPass => FilterMode::HighPass,
            FilterType::BandPass => FilterMode::BandPass,
        }
    }
}

/// One modulation matrix slot.
#[derive(Params)]
pub struct ModSlotParams {
    #[id = "src"]
    pub source: EnumParam<ModSourceParam>,
    #[id = "dst"]
    pub target: EnumParam<ModTargetParam>,
    #[id = "amt"]
    pub amount: FloatParam,
}

impl ModSlotParams {
    fn new(pad: usize, slot: usize) -> Self {
        let n = |s: &str| format!("P{} Mod {} {s}", pad + 1, slot + 1);
        Self {
            source: EnumParam::new(n("Source"), ModSourceParam::Off),
            target: EnumParam::new(n("Target"), ModTargetParam::Off),
            amount: FloatParam::new(
                n("Amount"),
                0.0,
                FloatRange::Linear {
                    min: -1.0,
                    max: 1.0,
                },
            )
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),
        }
    }

    fn to_dsp(&self, v: &Overrides) -> ModSlot {
        ModSlot {
            source: v.enumv(&self.source).into(),
            target: v.enumv(&self.target).into(),
            amount: v.float(&self.amount),
        }
    }
}

/// AHDSR parameters, used for the two modulation envelopes.
pub struct EnvelopeParams<'a> {
    pub attack: &'a FloatParam,
    pub hold: &'a FloatParam,
    pub decay: &'a FloatParam,
    pub sustain: &'a FloatParam,
    pub release: &'a FloatParam,
}

impl EnvelopeParams<'_> {
    fn to_dsp(&self, v: &Overrides) -> EnvParams {
        EnvParams {
            attack_ms: v.float(self.attack),
            hold_ms: v.float(self.hold),
            decay_ms: v.float(self.decay),
            sustain: v.float(self.sustain),
            release_ms: v.float(self.release),
        }
    }
}

#[derive(Params)]
pub struct PadParams {
    /// Single-sample path from before layers existed, migrated into `layers` on load
    #[persist = "path"]
    pub sample_path: RwLock<Option<String>>,
    /// Samples, velocity ranges and layer mode
    #[persist = "layers"]
    pub layers: RwLock<LayerConfig>,
    /// Display name, defaults to the file name
    #[persist = "name"]
    pub name: RwLock<String>,

    #[id = "note"]
    pub note: IntParam,
    #[id = "vol"]
    pub volume: FloatParam,
    #[id = "pan"]
    pub pan: FloatParam,
    #[id = "mute"]
    pub mute: BoolParam,
    #[id = "out"]
    pub output: IntParam,
    #[id = "choke"]
    pub choke: IntParam,
    #[id = "mode"]
    pub mode: EnumParam<PlayMode>,

    #[id = "tune"]
    pub tune: IntParam,
    #[id = "fine"]
    pub fine: FloatParam,
    #[id = "start"]
    pub start: FloatParam,
    #[id = "end"]
    pub end: FloatParam,
    #[id = "rev"]
    pub reverse: BoolParam,

    #[id = "att"]
    pub attack: FloatParam,
    #[id = "hold"]
    pub hold: FloatParam,
    #[id = "dec"]
    pub decay: FloatParam,
    #[id = "sus"]
    pub sustain: FloatParam,
    #[id = "rel"]
    pub release: FloatParam,
    #[id = "velsens"]
    pub velocity_sens: FloatParam,

    #[id = "ftype"]
    pub filter_type: EnumParam<FilterType>,
    #[id = "cutoff"]
    pub cutoff: FloatParam,
    #[id = "reso"]
    pub resonance: FloatParam,
    #[id = "fenv"]
    pub filter_env: FloatParam,
    #[id = "fdec"]
    pub filter_decay: FloatParam,
    #[id = "drive"]
    pub drive: FloatParam,

    #[id = "penv"]
    pub pitch_env: FloatParam,
    #[id = "pdec"]
    pub pitch_decay: FloatParam,

    #[id = "eqfreq"]
    pub eq_freq: FloatParam,
    #[id = "eqgain"]
    pub eq_gain: FloatParam,
    #[id = "eqq"]
    pub eq_q: FloatParam,

    #[id = "bits"]
    pub bits: FloatParam,
    #[id = "srr"]
    pub downsample: FloatParam,

    #[id = "lfoshape"]
    pub lfo_shape: EnumParam<LfoShapeParam>,
    #[id = "lforate"]
    pub lfo_rate: FloatParam,
    #[id = "lfosync"]
    pub lfo_sync: BoolParam,
    #[id = "lfodiv"]
    pub lfo_division: EnumParam<SyncDivision>,
    #[id = "lforetrig"]
    pub lfo_retrigger: BoolParam,

    #[id = "m1att"]
    pub env1_attack: FloatParam,
    #[id = "m1hold"]
    pub env1_hold: FloatParam,
    #[id = "m1dec"]
    pub env1_decay: FloatParam,
    #[id = "m1sus"]
    pub env1_sustain: FloatParam,
    #[id = "m1rel"]
    pub env1_release: FloatParam,

    #[id = "m2att"]
    pub env2_attack: FloatParam,
    #[id = "m2hold"]
    pub env2_hold: FloatParam,
    #[id = "m2dec"]
    pub env2_decay: FloatParam,
    #[id = "m2sus"]
    pub env2_sustain: FloatParam,
    #[id = "m2rel"]
    pub env2_release: FloatParam,

    #[nested(array, group = "Mod Slot")]
    pub slots: [ModSlotParams; NUM_SLOTS],
}

fn time_param(name: String, default_ms: f32, max_ms: f32) -> FloatParam {
    FloatParam::new(
        name,
        default_ms,
        FloatRange::Skewed {
            min: 0.0,
            max: max_ms,
            factor: FloatRange::skew_factor(-2.0),
        },
    )
    .with_unit(" ms")
    .with_value_to_string(Arc::new(|v| {
        if v >= 1000.0 {
            format!("{:.2} s", v / 1000.0)
        } else {
            format!("{v:.1} ms")
        }
    }))
}

fn percent_param(name: String, default: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max: 1.0 })
        .with_unit("%")
        .with_value_to_string(formatters::v2s_f32_percentage(0))
        .with_string_to_value(formatters::s2v_f32_percentage())
}

impl PadParams {
    pub fn new(index: usize) -> Self {
        let n = |s: &str| format!("P{} {s}", index + 1);

        Self {
            sample_path: RwLock::new(None),
            layers: RwLock::new(LayerConfig::default()),
            name: RwLock::new(String::new()),

            note: IntParam::new(
                n("Note"),
                BASE_NOTE + index as i32,
                IntRange::Linear { min: 0, max: 127 },
            )
            .with_value_to_string(Arc::new(|v| format!("{} ({v})", note_name(v))))
            .non_automatable(),
            volume: FloatParam::new(
                n("Volume"),
                0.0,
                FloatRange::Linear {
                    min: -60.0,
                    max: 12.0,
                },
            )
            .with_unit(" dB")
            .with_step_size(0.1),
            pan: FloatParam::new(
                n("Pan"),
                0.0,
                FloatRange::Linear {
                    min: -1.0,
                    max: 1.0,
                },
            )
            .with_value_to_string(Arc::new(|v| {
                let p = (v * 100.0).round() as i32;
                match p {
                    0 => String::from("C"),
                    p if p < 0 => format!("{}L", -p),
                    p => format!("{p}R"),
                }
            })),
            mute: BoolParam::new(n("Mute"), false),
            output: IntParam::new(
                n("Output"),
                1,
                IntRange::Linear {
                    min: 1,
                    max: NUM_OUTPUTS as i32,
                },
            )
            .with_value_to_string(Arc::new(|v| {
                if v == 1 {
                    String::from("Main")
                } else {
                    format!("Out {v}")
                }
            })),
            choke: IntParam::new(n("Choke"), 0, IntRange::Linear { min: 0, max: 16 })
                .with_value_to_string(Arc::new(|v| {
                    if v == 0 {
                        String::from("Off")
                    } else {
                        v.to_string()
                    }
                })),
            mode: EnumParam::new(n("Mode"), PlayMode::OneShot),

            tune: IntParam::new(n("Tune"), 0, IntRange::Linear { min: -48, max: 48 })
                .with_unit(" st"),
            fine: FloatParam::new(
                n("Fine"),
                0.0,
                FloatRange::Linear {
                    min: -100.0,
                    max: 100.0,
                },
            )
            .with_unit(" ct")
            .with_step_size(1.0),
            start: percent_param(n("Start"), 0.0),
            end: percent_param(n("End"), 1.0),
            reverse: BoolParam::new(n("Reverse"), false),

            attack: time_param(n("Attack"), 0.0, 2000.0),
            hold: time_param(n("Hold"), 0.0, 2000.0),
            decay: time_param(n("Decay"), 500.0, 10000.0),
            sustain: percent_param(n("Sustain"), 1.0),
            release: time_param(n("Release"), 200.0, 10000.0),
            velocity_sens: percent_param(n("Velocity"), 1.0),

            filter_type: EnumParam::new(n("Filter"), FilterType::Off),
            cutoff: FloatParam::new(
                n("Cutoff"),
                20000.0,
                FloatRange::Skewed {
                    min: 20.0,
                    max: 20000.0,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_value_to_string(formatters::v2s_f32_hz_then_khz(1))
            .with_string_to_value(formatters::s2v_f32_hz_then_khz()),
            resonance: percent_param(n("Resonance"), 0.0),
            filter_env: FloatParam::new(
                n("Filter Env"),
                0.0,
                FloatRange::Linear {
                    min: -8.0,
                    max: 8.0,
                },
            )
            .with_unit(" oct")
            .with_step_size(0.01),
            filter_decay: time_param(n("Filter Decay"), 200.0, 5000.0),
            drive: percent_param(n("Drive"), 0.0),

            pitch_env: FloatParam::new(
                n("Pitch Env"),
                0.0,
                FloatRange::Linear {
                    min: -48.0,
                    max: 48.0,
                },
            )
            .with_unit(" st")
            .with_step_size(0.1),
            pitch_decay: time_param(n("Pitch Decay"), 50.0, 2000.0),

            eq_freq: FloatParam::new(
                n("EQ Freq"),
                1000.0,
                FloatRange::Skewed {
                    min: 20.0,
                    max: 20000.0,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_value_to_string(formatters::v2s_f32_hz_then_khz(1))
            .with_string_to_value(formatters::s2v_f32_hz_then_khz()),
            eq_gain: FloatParam::new(
                n("EQ Gain"),
                0.0,
                FloatRange::Linear {
                    min: -18.0,
                    max: 18.0,
                },
            )
            .with_unit(" dB")
            .with_step_size(0.1),
            eq_q: FloatParam::new(
                n("EQ Q"),
                0.7,
                FloatRange::Skewed {
                    min: 0.1,
                    max: 10.0,
                    factor: FloatRange::skew_factor(-1.0),
                },
            )
            .with_step_size(0.01),

            bits: FloatParam::new(
                n("Bits"),
                24.0,
                FloatRange::Linear {
                    min: 2.0,
                    max: 24.0,
                },
            )
            .with_step_size(0.1)
            .with_value_to_string(Arc::new(|v| {
                if v >= 24.0 {
                    String::from("Off")
                } else {
                    format!("{v:.1} bit")
                }
            })),
            downsample: FloatParam::new(
                n("Downsample"),
                1.0,
                FloatRange::Skewed {
                    min: 1.0,
                    max: 32.0,
                    factor: FloatRange::skew_factor(-1.0),
                },
            )
            .with_step_size(0.01)
            .with_value_to_string(Arc::new(|v| {
                if v <= 1.0 {
                    String::from("Off")
                } else {
                    format!("{v:.1}x")
                }
            })),

            lfo_shape: EnumParam::new(n("LFO Shape"), LfoShapeParam::Sine),
            lfo_rate: FloatParam::new(
                n("LFO Rate"),
                2.0,
                FloatRange::Skewed {
                    min: 0.01,
                    max: 50.0,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            lfo_sync: BoolParam::new(n("LFO Sync"), false),
            lfo_division: EnumParam::new(n("LFO Division"), SyncDivision::Quarter),
            lfo_retrigger: BoolParam::new(n("LFO Retrigger"), true),

            env1_attack: time_param(n("Mod Env 1 Attack"), 0.0, 2000.0),
            env1_hold: time_param(n("Mod Env 1 Hold"), 0.0, 2000.0),
            env1_decay: time_param(n("Mod Env 1 Decay"), 300.0, 10000.0),
            env1_sustain: percent_param(n("Mod Env 1 Sustain"), 0.0),
            env1_release: time_param(n("Mod Env 1 Release"), 200.0, 10000.0),

            env2_attack: time_param(n("Mod Env 2 Attack"), 0.0, 2000.0),
            env2_hold: time_param(n("Mod Env 2 Hold"), 0.0, 2000.0),
            env2_decay: time_param(n("Mod Env 2 Decay"), 300.0, 10000.0),
            env2_sustain: percent_param(n("Mod Env 2 Sustain"), 0.0),
            env2_release: time_param(n("Mod Env 2 Release"), 200.0, 10000.0),

            slots: std::array::from_fn(|slot| ModSlotParams::new(index, slot)),
        }
    }

    pub fn env1(&self) -> EnvelopeParams<'_> {
        EnvelopeParams {
            attack: &self.env1_attack,
            hold: &self.env1_hold,
            decay: &self.env1_decay,
            sustain: &self.env1_sustain,
            release: &self.env1_release,
        }
    }

    pub fn env2(&self) -> EnvelopeParams<'_> {
        EnvelopeParams {
            attack: &self.env2_attack,
            hold: &self.env2_hold,
            decay: &self.env2_decay,
            sustain: &self.env2_sustain,
            release: &self.env2_release,
        }
    }

    /// LFO frequency in Hz, from the rate or, when synced, from the tempo.
    pub fn lfo_hz(&self, v: &Overrides, tempo: f64) -> f32 {
        if v.bool(&self.lfo_sync) {
            (tempo / 60.0 / v.enumv(&self.lfo_division).beats()) as f32
        } else {
            v.float(&self.lfo_rate)
        }
    }

    /// Snapshot of the current values for the DSP engine. `v` holds MIDI learn values that the
    /// host doesn't know about yet.
    pub fn to_dsp(&self, v: &Overrides, tempo: f64) -> klopfer_dsp::PadParams {
        klopfer_dsp::PadParams {
            volume_db: v.float(&self.volume),
            pan: v.float(&self.pan),
            mute: v.bool(&self.mute),
            tune: v.int(&self.tune) as f32 + v.float(&self.fine) / 100.0,
            start: v.float(&self.start),
            end: v.float(&self.end),
            reverse: v.bool(&self.reverse),
            gate: v.enumv(&self.mode) == PlayMode::Gate,
            amp: EnvParams {
                attack_ms: v.float(&self.attack),
                hold_ms: v.float(&self.hold),
                decay_ms: v.float(&self.decay),
                sustain: v.float(&self.sustain),
                release_ms: v.float(&self.release),
            },
            velocity_sens: v.float(&self.velocity_sens),
            filter_mode: v.enumv(&self.filter_type).into(),
            cutoff_hz: v.float(&self.cutoff),
            resonance: v.float(&self.resonance),
            filter_env_octaves: v.float(&self.filter_env),
            filter_env_decay_ms: v.float(&self.filter_decay),
            drive: v.float(&self.drive),
            pitch_env_semis: v.float(&self.pitch_env),
            pitch_env_decay_ms: v.float(&self.pitch_decay),
            eq_freq_hz: v.float(&self.eq_freq),
            eq_gain_db: v.float(&self.eq_gain),
            eq_q: v.float(&self.eq_q),
            bits: v.float(&self.bits),
            downsample: v.float(&self.downsample),
            lfo_shape: v.enumv(&self.lfo_shape).into(),
            lfo_rate_hz: self.lfo_hz(v, tempo),
            lfo_retrigger: v.bool(&self.lfo_retrigger),
            mod_env1: self.env1().to_dsp(v),
            mod_env2: self.env2().to_dsp(v),
            slots: std::array::from_fn(|i| self.slots[i].to_dsp(v)),
            choke_group: v.int(&self.choke) as u8,
            output: (v.int(&self.output) - 1) as u8,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The test kit generator and saved kits rely on these IDs.
    #[test]
    fn nested_parameter_ids() {
        let params = KlopferParams::default();
        let ids: Vec<String> = params
            .param_map()
            .into_iter()
            .map(|(id, _, _)| id)
            .collect();
        for id in [
            "vol_1",
            "choke_32",
            "lforate_13",
            "src_1_13",
            "amt_8_32",
            "m2rel_5",
        ] {
            assert!(ids.iter().any(|i| i == id), "missing parameter id {id}");
        }
        // 32 pads with 70 parameters each, plus master
        assert_eq!(ids.len(), 32 * 70 + 1);
    }

    #[test]
    fn enum_ids_are_stable_strings() {
        assert_eq!(ModTargetParam::ids().unwrap()[1], "pitch");
        assert_eq!(LfoShapeParam::ids().unwrap()[5], "sample_hold");
    }
}
