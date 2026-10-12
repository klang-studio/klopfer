//! Sample storage shared between the GUI, the background loader and the audio thread.
//!
//! Samples live in slots, one per pad layer (see [`klopfer_dsp::slot_index()`]). The audio thread
//! owns its own copies in [`AudioSlots`]. New samples reach it through a lock-free queue, and
//! replaced samples travel back through a second queue so they're never deallocated on the audio
//! thread. Time stretched versions are rendered in the background and travel the same way.

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use atomic_float::AtomicF32;
use crossbeam_queue::ArrayQueue;
use klopfer_dsp::stretch::{stretch, StretchMode};
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

/// Identifies a time stretched rendering: re-rendered whenever any of this changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StretchKey {
    pub mode: StretchMode,
    /// Length factor in thousandths
    pub ratio_milli: u32,
    /// Pitch shift in cents
    pub pitch_cents: i32,
}

impl StretchKey {
    pub fn new(mode: StretchMode, ratio: f32, pitch_semis: f32) -> Self {
        Self {
            mode,
            ratio_milli: (ratio.clamp(0.05, 20.0) * 1000.0).round() as u32,
            pitch_cents: (pitch_semis * 100.0).round() as i32,
        }
    }
}

enum SlotUpdate {
    Original(usize, Option<Arc<Sample>>),
    Stretched(usize, StretchKey, Arc<Sample>),
}

/// The audio thread's samples. Only the audio thread (or `initialize()`) touches these.
pub struct AudioSlots {
    pub originals: Vec<Option<Arc<Sample>>>,
    pub stretched: Vec<Option<Arc<Sample>>>,
    /// Key of the installed stretched sample
    pub stretch_keys: Vec<Option<StretchKey>>,
    /// Key of the last requested rendering, to request each change only once
    pub stretch_requested: Vec<Option<StretchKey>>,
}

impl Default for AudioSlots {
    fn default() -> Self {
        Self {
            originals: vec![None; NUM_SAMPLE_SLOTS],
            stretched: vec![None; NUM_SAMPLE_SLOTS],
            stretch_keys: vec![None; NUM_SAMPLE_SLOTS],
            stretch_requested: vec![None; NUM_SAMPLE_SLOTS],
        }
    }
}

/// A pad preview triggered from the GUI.
#[derive(Debug, Clone, Copy)]
pub enum Preview {
    On { pad: usize, velocity: f32 },
    Off { pad: usize },
}

pub struct SampleBank {
    pub slots: Vec<RwLock<PadSlot>>,
    to_audio: ArrayQueue<SlotUpdate>,
    /// Incremented for every stretch request; renderings for older generations are dropped
    stretch_gen: Vec<AtomicU64>,
    /// Whether a stretch rendering for the slot is on its way, for the GUI
    pub stretching: Vec<AtomicBool>,
    garbage: ArrayQueue<Arc<Sample>>,
    /// GUI → audio thread
    pub previews: ArrayQueue<Preview>,
    /// Background thread → audio thread: the sample to audition from the browser, `None` stops
    auditions: ArrayQueue<Option<Arc<Sample>>>,
    /// Audio thread → GUI, decaying peak level per pad
    pub meters: [AtomicF32; NUM_PADS],
    /// Audio thread → GUI, playback position per slot relative to the sample length, `-1.0` when
    /// nothing plays. Only updated while the editor is open.
    pub playheads: Vec<AtomicF32>,
}

impl Default for SampleBank {
    fn default() -> Self {
        Self {
            slots: (0..NUM_SAMPLE_SLOTS)
                .map(|_| RwLock::new(PadSlot::default()))
                .collect(),
            to_audio: ArrayQueue::new(QUEUE_SIZE),
            stretch_gen: (0..NUM_SAMPLE_SLOTS).map(|_| AtomicU64::new(0)).collect(),
            stretching: (0..NUM_SAMPLE_SLOTS)
                .map(|_| AtomicBool::new(false))
                .collect(),
            garbage: ArrayQueue::new(QUEUE_SIZE),
            previews: ArrayQueue::new(QUEUE_SIZE),
            auditions: ArrayQueue::new(4),
            meters: std::array::from_fn(|_| AtomicF32::new(0.0)),
            playheads: (0..NUM_SAMPLE_SLOTS)
                .map(|_| AtomicF32::new(-1.0))
                .collect(),
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

        self.send(SlotUpdate::Original(index, sample));
    }

    fn send(&self, mut msg: SlotUpdate) {
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

    /// Audio thread: marks a new stretch request for `slot` and returns its generation.
    pub fn next_stretch_gen(&self, slot: usize) -> u64 {
        self.stretching[slot].store(true, Ordering::Release);
        self.stretch_gen[slot].fetch_add(1, Ordering::AcqRel) + 1
    }

    /// Renders the time stretched version of `slot`. Blocking, call from a background thread.
    /// Does nothing if a newer request came in, and gives up midway when one does.
    pub fn render_stretch(&self, slot: usize, key: StretchKey, generation: u64) {
        self.collect_garbage();
        let current = || self.stretch_gen[slot].load(Ordering::Acquire);
        if current() != generation {
            return;
        }
        let Some(original) = self.slots[slot].read().sample.clone() else {
            self.stretching[slot].store(false, Ordering::Release);
            return;
        };

        let ratio = key.ratio_milli as f32 / 1000.0;
        let pitch = key.pitch_cents as f32 / 100.0;
        let cancelled = || current() != generation;
        if let Some(stretched) = stretch(&original, ratio, pitch, key.mode, &cancelled) {
            if current() == generation {
                self.send(SlotUpdate::Stretched(slot, key, Arc::new(stretched)));
                self.stretching[slot].store(false, Ordering::Release);
            }
        }
    }

    /// Plays `sample` once in place of the current audition, `None` stops it. Call from a
    /// background or GUI thread.
    pub fn audition(&self, sample: Option<Sample>) {
        self.collect_garbage();
        // Only the latest request matters, older ones are dropped here
        let _ = self.auditions.force_push(sample.map(Arc::new));
    }

    /// Audio thread: switches `audition` to the latest requested sample. Never deallocates.
    pub fn apply_audition(&self, audition: &mut Audition) {
        while self.garbage.len() < self.garbage.capacity() {
            let Some(sample) = self.auditions.pop() else {
                break;
            };
            if let Some(old) = std::mem::replace(&mut audition.sample, sample) {
                // Can't fail, we checked for room and we're the only producer
                let _ = self.garbage.push(old);
            }
            audition.pos = 0.0;
        }
    }

    /// Audio thread: applies pending sample swaps. `on_swap` is called with the slot index before
    /// its sample changes. Never allocates or deallocates.
    pub fn apply_swaps(&self, audio: &mut AudioSlots, mut on_swap: impl FnMut(usize)) {
        // Each update hands back at most two samples; only take one while they fit
        while self.garbage.len() + 2 <= self.garbage.capacity() {
            let Some(update) = self.to_audio.pop() else {
                break;
            };
            let old = match update {
                SlotUpdate::Original(index, sample) => {
                    on_swap(index);
                    // A new original makes the stretched version stale
                    audio.stretch_keys[index] = None;
                    audio.stretch_requested[index] = None;
                    [
                        std::mem::replace(&mut audio.originals[index], sample),
                        audio.stretched[index].take(),
                    ]
                }
                SlotUpdate::Stretched(index, key, sample) => {
                    on_swap(index);
                    audio.stretch_keys[index] = Some(key);
                    [audio.stretched[index].replace(sample), None]
                }
            };
            for sample in old.into_iter().flatten() {
                // Can't fail, we checked for room and we're the only producer
                let _ = self.garbage.push(sample);
            }
        }
    }
}

/// Audio thread: plays a browser sample once, resampled to the output rate.
#[derive(Default)]
pub struct Audition {
    sample: Option<Arc<Sample>>,
    /// Position in the sample's frames
    pos: f64,
}

impl Audition {
    /// The next stereo frame at `sample_rate`, silence once the sample has ended.
    pub fn next(&mut self, sample_rate: f32) -> [f32; 2] {
        let Some(sample) = &self.sample else {
            return [0.0; 2];
        };
        let i = self.pos as usize;
        if i + 1 >= sample.left.len() {
            return [0.0; 2];
        }
        let t = (self.pos - i as f64) as f32;
        let lerp = |data: &[f32]| data[i] + (data[i + 1] - data[i]) * t;
        let left = lerp(&sample.left);
        let right = sample.right.as_deref().map_or(left, lerp);
        self.pos += (sample.sample_rate / sample_rate) as f64;
        [left, right]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swaps_reach_audio_and_old_samples_come_back() {
        let bank = SampleBank::default();
        let mut slots = AudioSlots::default();

        bank.request(0, Some(String::from("a")));
        bank.install(
            0,
            Some("a"),
            Some(Sample::mono(48000.0, vec![0.0; 4])),
            PadStatus::Loaded,
        );
        bank.apply_swaps(&mut slots, |_| {});
        assert!(slots.originals[0].is_some());

        bank.request(0, None);
        bank.install(0, None, None, PadStatus::Empty);
        let mut swapped = vec![];
        bank.apply_swaps(&mut slots, |p| swapped.push(p));
        assert!(slots.originals[0].is_none());
        assert_eq!(swapped, vec![0]);
        assert_eq!(bank.garbage.len(), 1);
    }

    #[test]
    fn stale_loads_are_dropped() {
        let bank = SampleBank::default();
        let mut slots = AudioSlots::default();
        bank.request(1, Some(String::from("new")));
        bank.install(
            1,
            Some("old"),
            Some(Sample::mono(48000.0, vec![0.0; 4])),
            PadStatus::Loaded,
        );
        bank.apply_swaps(&mut slots, |_| {});
        assert!(slots.originals[1].is_none());
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

    #[test]
    fn stretched_renderings_install_and_stale_ones_are_dropped() {
        let bank = SampleBank::default();
        let mut slots = AudioSlots::default();
        bank.request(3, Some(String::from("a")));
        bank.install(
            3,
            Some("a"),
            Some(Sample::mono(48000.0, vec![0.1; 4800])),
            PadStatus::Loaded,
        );
        bank.apply_swaps(&mut slots, |_| {});

        let key = StretchKey::new(StretchMode::Smooth, 2.0, 0.0);
        let stale = bank.next_stretch_gen(3);
        let current = bank.next_stretch_gen(3);
        bank.render_stretch(3, key, stale);
        bank.apply_swaps(&mut slots, |_| {});
        assert!(slots.stretched[3].is_none());

        bank.render_stretch(3, key, current);
        bank.apply_swaps(&mut slots, |_| {});
        assert_eq!(slots.stretched[3].as_ref().unwrap().len(), 9600);
        assert_eq!(slots.stretch_keys[3], Some(key));
        assert!(!bank.stretching[3].load(Ordering::Acquire));

        // A new original invalidates the stretched version
        bank.request(3, None);
        bank.install(3, None, None, PadStatus::Empty);
        bank.apply_swaps(&mut slots, |_| {});
        assert!(slots.stretched[3].is_none());
        assert_eq!(slots.stretch_keys[3], None);
    }

    #[test]
    fn auditions_play_once_at_the_output_rate() {
        let bank = SampleBank::default();
        let mut audition = Audition::default();
        bank.audition(Some(Sample::mono(24000.0, vec![0.0, 1.0, 0.0])));
        bank.apply_audition(&mut audition);

        // Half the output rate: every other frame is interpolated
        let left: Vec<f32> = (0..6).map(|_| audition.next(48000.0)[0]).collect();
        assert_eq!(left, vec![0.0, 0.5, 1.0, 0.5, 0.0, 0.0]);

        // A new audition replaces the old one, which goes back for deallocation
        bank.audition(None);
        bank.apply_audition(&mut audition);
        assert_eq!(audition.next(48000.0), [0.0; 2]);
        assert_eq!(bank.garbage.len(), 1);
    }
}
