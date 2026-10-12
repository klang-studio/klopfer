//! Velocity layers and round robin: which of a pad's samples play for a given hit.

use crate::modulation::Rng;

pub const NUM_LAYERS: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LayerMode {
    /// All layers whose velocity range matches play together
    #[default]
    Velocity,
    /// Matching layers take turns
    RoundRobin,
    /// A random matching layer plays, avoiding immediate repeats
    Random,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayerInfo {
    /// Whether a sample is loaded for this layer
    pub loaded: bool,
    /// Inclusive MIDI velocity range, `1..=127`
    pub vel_lo: u8,
    pub vel_hi: u8,
    /// Linear gain
    pub gain: f32,
    /// Semitones
    pub tune: f32,
}

impl Default for LayerInfo {
    fn default() -> Self {
        Self {
            loaded: false,
            vel_lo: 1,
            vel_hi: 127,
            gain: 1.0,
            tune: 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PadLayers {
    pub mode: LayerMode,
    pub layers: [LayerInfo; NUM_LAYERS],
}

/// Per-pad state for round robin and random selection.
#[derive(Debug, Clone, Copy, Default)]
pub struct LayerCursor {
    next: usize,
    last: Option<usize>,
}

/// Returns which layers to play for a hit with `velocity` in `0..=1`.
pub fn select_layers(
    pad: &PadLayers,
    velocity: f32,
    cursor: &mut LayerCursor,
    rng: &mut Rng,
) -> [bool; NUM_LAYERS] {
    let vel = (velocity * 127.0).round().clamp(1.0, 127.0) as u8;
    let mut candidates = [0usize; NUM_LAYERS];
    let mut count = 0;
    for (i, l) in pad.layers.iter().enumerate() {
        if l.loaded && (l.vel_lo.min(l.vel_hi)..=l.vel_hi.max(l.vel_lo)).contains(&vel) {
            candidates[count] = i;
            count += 1;
        }
    }

    let mut play = [false; NUM_LAYERS];
    if count == 0 {
        return play;
    }

    let pick = match pad.mode {
        LayerMode::Velocity => {
            for &i in &candidates[..count] {
                play[i] = true;
            }
            return play;
        }
        LayerMode::RoundRobin => {
            let i = candidates[cursor.next % count];
            cursor.next = cursor.next.wrapping_add(1);
            i
        }
        LayerMode::Random => {
            let mut i = candidates[rng.below(count)];
            if count > 1 && Some(i) == cursor.last {
                // Shift to a different candidate instead of re-rolling, stays allocation and loop free
                let pos = candidates[..count]
                    .iter()
                    .position(|&c| c == i)
                    .unwrap_or(0);
                i = candidates[(pos + 1 + rng.below(count - 1)) % count];
            }
            i
        }
    };
    cursor.last = Some(pick);
    play[pick] = true;
    play
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pad(mode: LayerMode, ranges: &[(u8, u8)]) -> PadLayers {
        let mut p = PadLayers {
            mode,
            ..PadLayers::default()
        };
        for (i, (lo, hi)) in ranges.iter().enumerate() {
            p.layers[i] = LayerInfo {
                loaded: true,
                vel_lo: *lo,
                vel_hi: *hi,
                ..LayerInfo::default()
            };
        }
        p
    }

    fn playing(p: [bool; NUM_LAYERS]) -> Vec<usize> {
        (0..NUM_LAYERS).filter(|i| p[*i]).collect()
    }

    #[test]
    fn velocity_ranges_pick_and_layer() {
        let p = pad(LayerMode::Velocity, &[(1, 63), (64, 127), (100, 127)]);
        let (mut c, mut r) = (LayerCursor::default(), Rng::default());
        assert_eq!(playing(select_layers(&p, 0.3, &mut c, &mut r)), vec![0]);
        assert_eq!(playing(select_layers(&p, 0.6, &mut c, &mut r)), vec![1]);
        assert_eq!(playing(select_layers(&p, 1.0, &mut c, &mut r)), vec![1, 2]);
    }

    #[test]
    fn round_robin_cycles() {
        let p = pad(LayerMode::RoundRobin, &[(1, 127), (1, 127), (1, 127)]);
        let (mut c, mut r) = (LayerCursor::default(), Rng::default());
        let picks: Vec<usize> = (0..6)
            .map(|_| playing(select_layers(&p, 1.0, &mut c, &mut r))[0])
            .collect();
        assert_eq!(picks, vec![0, 1, 2, 0, 1, 2]);
    }

    #[test]
    fn random_never_repeats_immediately() {
        let p = pad(LayerMode::Random, &[(1, 127), (1, 127)]);
        let (mut c, mut r) = (LayerCursor::default(), Rng::new(3));
        let picks: Vec<usize> = (0..50)
            .map(|_| playing(select_layers(&p, 1.0, &mut c, &mut r))[0])
            .collect();
        assert!(picks.windows(2).all(|w| w[0] != w[1]));
    }

    #[test]
    fn unloaded_layers_are_skipped() {
        let mut p = pad(LayerMode::RoundRobin, &[(1, 127), (1, 127)]);
        p.layers[1].loaded = false;
        let (mut c, mut r) = (LayerCursor::default(), Rng::default());
        for _ in 0..3 {
            assert_eq!(playing(select_layers(&p, 1.0, &mut c, &mut r)), vec![0]);
        }
    }
}
