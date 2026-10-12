//! Sample storage shared between the GUI, the background loader and the audio thread.
//!
//! Samples live in slots, one per pad layer (see [`klopfer_dsp::slot_index()`]). The audio thread
//! owns its own `[Option<Arc<Sample>>; NUM_SAMPLE_SLOTS]`. New samples reach it through a
//! lock-free queue, and replaced samples travel back through a second queue so they're never
//! deallocated on the audio thread.

use std::path::Path;
use std::sync::Arc;

use atomic_float::AtomicF32;
use crossbeam_queue::ArrayQueue;
use klopfer_dsp::{Sample, NUM_PADS, NUM_SAMPLE_SLOTS};
use parking_lot::RwLock;

use crate::loader;

const QUEUE_SIZE: usize = 128;
const PEAK_BUCKETS: usize = 600;

#[derive(Debug, Clone, Default, PartialEq)]
pub enum PadStatus {
    #[default]
    Empty,
    Loaded,
    /// The file could not be found, the user needs to relink it
    Missing,
    Error(String),
}

/// What the GUI knows about a sample slot.
#[derive(Default)]
pub struct PadSlot {
    /// The path that should be loaded. Used to drop stale loads and to skip redundant ones.
    pub requested: Option<String>,
    pub status: PadStatus,
    pub sample: Option<Arc<Sample>>,
    /// Min/max pairs for the waveform display
    pub peaks: Arc<Vec<(f32, f32)>>,
}

/// A pad preview triggered from the GUI.
#[derive(Debug, Clone, Copy)]
pub enum Preview {
    On { pad: usize, velocity: f32 },
    Off { pad: usize },
}

pub struct SampleBank {
    pub slots: Vec<RwLock<PadSlot>>,
    to_audio: ArrayQueue<(usize, Option<Arc<Sample>>)>,
    garbage: ArrayQueue<Arc<Sample>>,
    /// GUI → audio thread
    pub previews: ArrayQueue<Preview>,
    /// Audio thread → GUI, decaying peak level per pad
    pub meters: [AtomicF32; NUM_PADS],
}

impl Default for SampleBank {
    fn default() -> Self {
        Self {
            slots: (0..NUM_SAMPLE_SLOTS)
                .map(|_| RwLock::new(PadSlot::default()))
                .collect(),
            to_audio: ArrayQueue::new(QUEUE_SIZE),
            garbage: ArrayQueue::new(QUEUE_SIZE),
            previews: ArrayQueue::new(QUEUE_SIZE),
            meters: std::array::from_fn(|_| AtomicF32::new(0.0)),
        }
    }
}

impl SampleBank {
    /// Marks `path` as the wanted sample for `slot`. Returns `true` if it differs from what's
    /// already requested, i.e. if a load is needed.
    pub fn request(&self, slot: usize, path: Option<String>) -> bool {
        let mut slot = self.slots[slot].write();
        if slot.requested == path {
            return false;
        }
        slot.requested = path;
        true
    }

    /// Loads the requested sample for `slot`. Blocking, call from a background or GUI thread.
    pub fn load(&self, slot: usize, path: Option<String>) {
        self.collect_garbage();

        let Some(path) = path else {
            self.install(slot, None, None, PadStatus::Empty);
            return;
        };

        let result = if Path::new(&path).exists() {
            loader::load_sample(Path::new(&path)).map_err(PadStatus::Error)
        } else {
            Err(PadStatus::Missing)
        };

        match result {
            Ok(sample) => self.install(slot, Some(&path), Some(sample), PadStatus::Loaded),
            Err(status) => self.install(slot, Some(&path), None, status),
        }
    }

    fn install(&self, index: usize, path: Option<&str>, sample: Option<Sample>, status: PadStatus) {
        let sample = sample.map(Arc::new);
        {
            let mut slot = self.slots[index].write();
            // A newer request superseded this one while decoding
            if slot.requested.as_deref() != path {
                return;
            }
            slot.peaks = Arc::new(
                sample
                    .as_ref()
                    .map(|s| s.peaks(PEAK_BUCKETS))
                    .unwrap_or_default(),
            );
            slot.sample = sample.clone();
            slot.status = status;
        }

        let mut msg = (index, sample);
        // The audio thread drains this every block, so this only spins when no audio is running
        for _ in 0..100 {
            match self.to_audio.push(msg) {
                Ok(()) => return,
                Err(m) => {
                    msg = m;
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
            }
        }
    }

    /// Drops samples the audio thread has handed back.
    pub fn collect_garbage(&self) {
        while self.garbage.pop().is_some() {}
    }

    /// Audio thread: applies pending sample swaps to `slots`. `on_swap` is called with the slot
    /// index before its sample changes. Never allocates or deallocates.
    pub fn apply_swaps(&self, slots: &mut [Option<Arc<Sample>>], mut on_swap: impl FnMut(usize)) {
        // Only take new samples while the old ones can be handed back
        while self.garbage.len() < self.garbage.capacity() {
            let Some((index, sample)) = self.to_audio.pop() else {
                break;
            };
            on_swap(index);
            if let Some(old) = std::mem::replace(&mut slots[index], sample) {
                // Can't fail, we checked for room and we're the only producer
                let _ = self.garbage.push(old);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swaps_reach_audio_and_old_samples_come_back() {
        let bank = SampleBank::default();
        let mut slots = vec![None; NUM_SAMPLE_SLOTS];

        bank.request(0, Some(String::from("a")));
        bank.install(
            0,
            Some("a"),
            Some(Sample::mono(48000.0, vec![0.0; 4])),
            PadStatus::Loaded,
        );
        bank.apply_swaps(&mut slots, |_| {});
        assert!(slots[0].is_some());

        bank.request(0, None);
        bank.install(0, None, None, PadStatus::Empty);
        let mut swapped = vec![];
        bank.apply_swaps(&mut slots, |p| swapped.push(p));
        assert!(slots[0].is_none());
        assert_eq!(swapped, vec![0]);
        assert_eq!(bank.garbage.len(), 1);
    }

    #[test]
    fn stale_loads_are_dropped() {
        let bank = SampleBank::default();
        let mut slots = vec![None; NUM_SAMPLE_SLOTS];
        bank.request(1, Some(String::from("new")));
        bank.install(
            1,
            Some("old"),
            Some(Sample::mono(48000.0, vec![0.0; 4])),
            PadStatus::Loaded,
        );
        bank.apply_swaps(&mut slots, |_| {});
        assert!(slots[1].is_none());
    }

    #[test]
    fn missing_files_are_flagged() {
        let bank = SampleBank::default();
        let path = Some(String::from("/nope/kick.wav"));
        assert!(bank.request(2, path.clone()));
        assert!(!bank.request(2, path.clone()));
        bank.load(2, path);
        assert_eq!(bank.slots[2].read().status, PadStatus::Missing);
    }
}
