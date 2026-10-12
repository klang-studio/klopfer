//! Persisted per-pad layer configuration: up to four samples with velocity ranges.

use klopfer_dsp::{LayerInfo, LayerMode, PadLayers, NUM_LAYERS};
use serde::{Deserialize, Deserializer, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LayerModeConfig {
    #[default]
    Velocity,
    RoundRobin,
    Random,
}

impl LayerModeConfig {
    pub const ALL: [(LayerModeConfig, &'static str); 3] = [
        (LayerModeConfig::Velocity, "Velocity"),
        (LayerModeConfig::RoundRobin, "Round Robin"),
        (LayerModeConfig::Random, "Random"),
    ];

    pub fn name(self) -> &'static str {
        Self::ALL
            .iter()
            .find(|(m, _)| *m == self)
            .map(|(_, n)| *n)
            .unwrap_or("")
    }
}

impl From<LayerModeConfig> for LayerMode {
    fn from(m: LayerModeConfig) -> Self {
        match m {
            LayerModeConfig::Velocity => LayerMode::Velocity,
            LayerModeConfig::RoundRobin => LayerMode::RoundRobin,
            LayerModeConfig::Random => LayerMode::Random,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LayerSlotConfig {
    /// Absolute path of the sample
    pub path: Option<String>,
    /// Inclusive MIDI velocity range
    pub vel_lo: u8,
    pub vel_hi: u8,
    pub gain_db: f32,
    /// Semitones
    pub tune: f32,
}

impl Default for LayerSlotConfig {
    fn default() -> Self {
        Self {
            path: None,
            vel_lo: 1,
            vel_hi: 127,
            gain_db: 0.0,
            tune: 0.0,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LayerConfig {
    pub mode: LayerModeConfig,
    #[serde(deserialize_with = "any_number_of_layers")]
    pub layers: [LayerSlotConfig; NUM_LAYERS],
}

/// Accepts fewer or more layers than [`NUM_LAYERS`], so kits stay loadable if that ever changes.
fn any_number_of_layers<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<[LayerSlotConfig; NUM_LAYERS], D::Error> {
    let mut layers: [LayerSlotConfig; NUM_LAYERS] = Default::default();
    for (dst, src) in layers
        .iter_mut()
        .zip(Vec::<LayerSlotConfig>::deserialize(d)?)
    {
        *dst = src;
    }
    Ok(layers)
}

impl LayerConfig {
    pub fn to_dsp(&self) -> PadLayers {
        PadLayers {
            mode: self.mode.into(),
            layers: std::array::from_fn(|i| {
                let l = &self.layers[i];
                LayerInfo {
                    loaded: l.path.is_some(),
                    vel_lo: l.vel_lo.clamp(1, 127),
                    vel_hi: l.vel_hi.clamp(1, 127),
                    gain: 10f32.powf(l.gain_db / 20.0),
                    tune: l.tune,
                }
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_fields_get_defaults() {
        let cfg: LayerConfig =
            serde_json::from_str(r#"{"layers":[{"path":"/a.wav"},{"vel_lo":64}]}"#).unwrap();
        assert_eq!(cfg.mode, LayerModeConfig::Velocity);
        assert_eq!(cfg.layers[0].path.as_deref(), Some("/a.wav"));
        assert_eq!(cfg.layers[0].vel_hi, 127);
        assert_eq!(cfg.layers[1].vel_lo, 64);
        assert!(cfg.layers[3].path.is_none());
    }
}
