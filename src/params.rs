//! Plugin parameters. Parameter IDs are part of saved projects and must never change.

use klopfer_dsp::{FilterMode, NUM_OUTPUTS, NUM_PADS};
use nih_plug::prelude::*;
use nih_plug_egui::EguiState;
use parking_lot::RwLock;
use std::sync::Arc;

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

    #[id = "master"]
    pub master: FloatParam,

    #[nested(array, group = "Pad")]
    pub pads: [PadParams; NUM_PADS],
}

impl Default for KlopferParams {
    fn default() -> Self {
        Self {
            editor_state: EguiState::from_size(crate::editor::WIDTH, crate::editor::HEIGHT),
            kit_name: RwLock::new(String::from("Init")),
            master: FloatParam::new(
                "Master",
                util::db_to_gain(0.0),
                FloatRange::Skewed {
                    min: util::db_to_gain(-60.0),
                    max: util::db_to_gain(12.0),
                    factor: FloatRange::gain_skew_factor(-60.0, 12.0),
                },
            )
            .with_smoother(SmoothingStyle::Logarithmic(20.0))
            .with_unit(" dB")
            .with_value_to_string(formatters::v2s_f32_gain_to_db(1))
            .with_string_to_value(formatters::s2v_f32_gain_to_db()),
            pads: std::array::from_fn(PadParams::new),
        }
    }
}

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayMode {
    #[name = "One Shot"]
    OneShot,
    #[name = "Gate"]
    Gate,
}

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterType {
    #[name = "Off"]
    Off,
    #[name = "Low Pass"]
    LowPass,
    #[name = "High Pass"]
    HighPass,
    #[name = "Band Pass"]
    BandPass,
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

#[derive(Params)]
pub struct PadParams {
    /// Absolute path of the loaded sample
    #[persist = "path"]
    pub sample_path: RwLock<Option<String>>,
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
        }
    }

    /// Snapshot of the current values for the DSP engine.
    pub fn to_dsp(&self) -> klopfer_dsp::PadParams {
        klopfer_dsp::PadParams {
            volume_db: self.volume.value(),
            pan: self.pan.value(),
            mute: self.mute.value(),
            tune: self.tune.value() as f32 + self.fine.value() / 100.0,
            start: self.start.value(),
            end: self.end.value(),
            reverse: self.reverse.value(),
            gate: self.mode.value() == PlayMode::Gate,
            attack_ms: self.attack.value(),
            hold_ms: self.hold.value(),
            decay_ms: self.decay.value(),
            sustain: self.sustain.value(),
            release_ms: self.release.value(),
            velocity_sens: self.velocity_sens.value(),
            filter_mode: self.filter_type.value().into(),
            cutoff_hz: self.cutoff.value(),
            resonance: self.resonance.value(),
            filter_env_octaves: self.filter_env.value(),
            filter_env_decay_ms: self.filter_decay.value(),
            drive: self.drive.value(),
            pitch_env_semis: self.pitch_env.value(),
            pitch_env_decay_ms: self.pitch_decay.value(),
            eq_freq_hz: self.eq_freq.value(),
            eq_gain_db: self.eq_gain.value(),
            eq_q: self.eq_q.value(),
            bits: self.bits.value(),
            downsample: self.downsample.value(),
            choke_group: self.choke.value() as u8,
            output: (self.output.value() - 1) as u8,
        }
    }
}
