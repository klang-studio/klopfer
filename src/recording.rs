//! Recording from the sidechain input into a pad layer.
//!
//! The editor allocates a [`Take`] and hands it to the audio thread, which only writes into it.
//! When the take is full or stopped, it travels back to the editor, which trims it, saves it as a
//! WAV file and loads it. No allocations on the audio thread.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use atomic_float::AtomicF32;
use crossbeam_queue::ArrayQueue;

pub const MAX_SECONDS: f32 = 60.0;

/// Peak level after normalizing, -1 dBFS
const NORMALIZE_PEAK: f32 = 0.891;
/// Input meter falls by 20 dB within this time
const METER_DECAY_SECS: f32 = 0.3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum State {
    Idle = 0,
    /// Armed, waiting for the input to cross the threshold
    Waiting = 1,
    Recording = 2,
}

/// A recording in progress. Preallocated for [`MAX_SECONDS`].
pub struct Take {
    pub left: Vec<f32>,
    pub right: Vec<f32>,
    pub len: usize,
    pub sample_rate: f32,
    pub pad: usize,
    pub layer: usize,
    /// Start only when the input exceeds this linear level
    pub threshold: Option<f32>,
    pub normalize: bool,
    started: bool,
}

impl Take {
    pub fn new(
        sample_rate: f32,
        pad: usize,
        layer: usize,
        threshold: Option<f32>,
        normalize: bool,
    ) -> Box<Self> {
        let capacity = (sample_rate * MAX_SECONDS) as usize;
        Box::new(Self {
            left: vec![0.0; capacity],
            right: vec![0.0; capacity],
            len: 0,
            sample_rate,
            pad,
            layer,
            threshold,
            normalize,
            started: threshold.is_none(),
        })
    }

    fn is_full(&self) -> bool {
        self.len >= self.left.len()
    }

    /// The recorded audio, normalized if requested.
    pub fn audio(&self) -> (Vec<f32>, Vec<f32>) {
        let (mut l, mut r) = (
            self.left[..self.len].to_vec(),
            self.right[..self.len].to_vec(),
        );
        if self.normalize {
            let peak = l.iter().chain(&r).fold(0.0f32, |m, x| m.max(x.abs()));
            if peak > 1e-6 {
                let gain = NORMALIZE_PEAK / peak;
                l.iter_mut().chain(r.iter_mut()).for_each(|x| *x *= gain);
            }
        }
        (l, r)
    }

    /// Writes the take as a 32-bit float stereo WAV file.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("cannot create folder: {e}"))?;
        }
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: self.sample_rate as u32,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let mut w = hound::WavWriter::create(path, spec).map_err(|e| e.to_string())?;
        let (l, r) = self.audio();
        for (a, b) in l.iter().zip(&r) {
            w.write_sample(*a).map_err(|e| e.to_string())?;
            w.write_sample(*b).map_err(|e| e.to_string())?;
        }
        w.finalize().map_err(|e| e.to_string())
    }
}

/// A file name for a new recording, e.g. `Klopfer 2026-10-12 15-04-05.wav`.
pub fn file_name() -> PathBuf {
    PathBuf::from(format!(
        "Klopfer {}.wav",
        chrono::Local::now().format("%Y-%m-%d %H-%M-%S")
    ))
}

pub struct Recorder {
    to_audio: ArrayQueue<Box<Take>>,
    from_audio: ArrayQueue<Box<Take>>,
    stop: AtomicBool,
    state: AtomicU8,
    /// Decaying peak level of the sidechain input
    pub level: AtomicF32,
    /// Recorded fraction of the take, `0..1`
    pub progress: AtomicF32,
    /// The host's sample rate, for allocating takes
    pub sample_rate: AtomicF32,
}

impl Default for Recorder {
    fn default() -> Self {
        Self {
            to_audio: ArrayQueue::new(1),
            from_audio: ArrayQueue::new(2),
            stop: AtomicBool::new(false),
            state: AtomicU8::new(State::Idle as u8),
            level: AtomicF32::new(0.0),
            progress: AtomicF32::new(0.0),
            sample_rate: AtomicF32::new(44100.0),
        }
    }
}

impl Recorder {
    pub fn state(&self) -> State {
        match self.state.load(Ordering::Acquire) {
            1 => State::Waiting,
            2 => State::Recording,
            _ => State::Idle,
        }
    }

    /// Editor: starts a take. Returns `false` if one is already running.
    pub fn start(&self, take: Box<Take>) -> bool {
        if self.state() != State::Idle {
            return false;
        }
        self.stop.store(false, Ordering::Release);
        self.progress.store(0.0, Ordering::Relaxed);
        let waiting = take.threshold.is_some();
        if self.to_audio.push(take).is_err() {
            return false;
        }
        let state = if waiting {
            State::Waiting
        } else {
            State::Recording
        };
        self.state.store(state as u8, Ordering::Release);
        true
    }

    /// Editor: ends the take, it comes back through [`Recorder::take_finished()`].
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Release);
    }

    /// Editor: a finished take, if there is one.
    pub fn take_finished(&self) -> Option<Box<Take>> {
        self.from_audio.pop()
    }
}

/// The audio thread's side of the recorder.
#[derive(Default)]
pub struct RecorderAudio {
    take: Option<Box<Take>>,
    meter_decay: f32,
}

impl RecorderAudio {
    pub fn set_sample_rate(&mut self, recorder: &Recorder, sample_rate: f32) {
        recorder.sample_rate.store(sample_rate, Ordering::Relaxed);
        self.meter_decay = 0.1f32.powf(1.0 / (METER_DECAY_SECS * sample_rate));
    }

    /// Records one block of the stereo input. Never allocates or deallocates.
    pub fn process(&mut self, recorder: &Recorder, left: &[f32], right: &[f32]) {
        let peak = left.iter().chain(right).fold(0.0f32, |m, x| m.max(x.abs()));
        let old = recorder.level.load(Ordering::Relaxed);
        let decayed = old * self.meter_decay.powi(left.len() as i32);
        recorder.level.store(peak.max(decayed), Ordering::Relaxed);

        if self.take.is_none() {
            self.take = recorder.to_audio.pop();
        }
        let Some(take) = self.take.as_mut() else {
            return;
        };

        let mut finished = recorder.stop.load(Ordering::Acquire);
        if !finished {
            for (l, r) in left.iter().zip(right) {
                if !take.started {
                    let threshold = take.threshold.unwrap_or(0.0);
                    if l.abs().max(r.abs()) < threshold {
                        continue;
                    }
                    take.started = true;
                    recorder
                        .state
                        .store(State::Recording as u8, Ordering::Release);
                }
                take.left[take.len] = *l;
                take.right[take.len] = *r;
                take.len += 1;
                if take.is_full() {
                    finished = true;
                    break;
                }
            }
            recorder
                .progress
                .store(take.len as f32 / take.left.len() as f32, Ordering::Relaxed);
        }

        if finished {
            // Hand the take back; if the editor hasn't collected the previous one yet, try again
            // next block instead of dropping (and deallocating) it here
            if let Some(take) = self.take.take() {
                if let Err(take) = recorder.from_audio.push(take) {
                    self.take = Some(take);
                    return;
                }
            }
            recorder.stop.store(false, Ordering::Release);
            recorder.state.store(State::Idle as u8, Ordering::Release);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn audio(rec: &mut RecorderAudio, r: &Recorder, values: &[f32]) {
        rec.process(r, values, values);
    }

    #[test]
    fn records_until_stopped() {
        let recorder = Recorder::default();
        let mut audio_side = RecorderAudio::default();
        audio_side.set_sample_rate(&recorder, 100.0);

        assert!(recorder.start(Take::new(100.0, 3, 1, None, false)));
        assert!(!recorder.start(Take::new(100.0, 3, 1, None, false)));
        audio(&mut audio_side, &recorder, &[0.1, 0.2, 0.3]);
        assert_eq!(recorder.state(), State::Recording);
        recorder.stop();
        audio(&mut audio_side, &recorder, &[0.9]);

        let take = recorder.take_finished().unwrap();
        assert_eq!((take.pad, take.layer), (3, 1));
        assert_eq!(take.audio().0, vec![0.1, 0.2, 0.3]);
        assert_eq!(recorder.state(), State::Idle);
    }

    #[test]
    fn waits_for_threshold_and_stops_when_full() {
        let recorder = Recorder::default();
        let mut audio_side = RecorderAudio::default();
        // 60 s at 1 Hz: 60 samples of room
        audio_side.set_sample_rate(&recorder, 1.0);
        assert!(recorder.start(Take::new(1.0, 0, 0, Some(0.5), true)));
        audio(&mut audio_side, &recorder, &[0.1, 0.2]);
        assert_eq!(recorder.state(), State::Waiting);

        let loud = vec![0.25f32; 70];
        audio(&mut audio_side, &recorder, &[0.6]);
        audio(&mut audio_side, &recorder, &loud);
        let take = recorder.take_finished().unwrap();
        assert_eq!(take.len, 60);
        // Normalized to -1 dBFS
        let (l, _) = take.audio();
        assert!((l[0] - NORMALIZE_PEAK).abs() < 1e-6);
    }

    #[test]
    fn saves_wav() {
        let dir = std::env::temp_dir().join(format!("klopfer-rec-{}", std::process::id()));
        let recorder = Recorder::default();
        let mut audio_side = RecorderAudio::default();
        audio_side.set_sample_rate(&recorder, 1000.0);
        recorder.start(Take::new(1000.0, 0, 0, None, false));
        audio(&mut audio_side, &recorder, &[0.5; 100]);
        recorder.stop();
        audio(&mut audio_side, &recorder, &[]);
        let take = recorder.take_finished().unwrap();

        let path = dir.join(file_name());
        take.save(&path).unwrap();
        let loaded = crate::loader::load_sample(&path).unwrap();
        assert_eq!(loaded.len(), 100);
        assert!(loaded.is_stereo());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
