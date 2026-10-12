//! Klopfer: a 32 pad drum sampler.

// Triggered by the VST3 bindings' macros, not by our code
#![allow(semicolon_in_expressions_from_non_local_macros)]

use std::num::NonZeroU32;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use klopfer_dsp::{
    slot_index, Engine, PadLayers, PadSettings, Sample, SampleSource, NUM_LAYERS, NUM_OUTPUTS,
    NUM_PADS,
};
use nice_plug::prelude::*;
use nice_plug::midi::Key;
use nice_plug::util::permit_alloc;
use nice_plug_egui::EguiEditorState;

mod bank;
mod editor;
pub mod kit;
mod layers;
pub mod loader;
mod midi_file;
mod midi_learn;
mod params;
mod recording;
mod settings;
pub mod sfz;

use bank::{AudioSlots, Audition, Preview, SampleBank, StretchKey};
use midi_learn::{prune_overrides, set_override, MidiLearn, Override, Overrides};
use params::KlopferParams;
use recording::{Recorder, RecorderAudio};

/// Time constant of the master gain smoothing
const MASTER_SMOOTHING_MS: f32 = 10.0;
/// Upper bound for parameters overridden by MIDI learn at the same time
const MAX_OVERRIDES: usize = 128;
/// Tempo for synced LFOs when the host doesn't report one
const DEFAULT_TEMPO: f64 = 120.0;

/// Pad meters fall by 20 dB within this time.
const METER_DECAY_MS: f32 = 300.0;

const AUX_OUTPUTS: [NonZeroU32; NUM_OUTPUTS - 1] = [new_nonzero_u32(2); NUM_OUTPUTS - 1];
/// Stereo sidechain input for recording
const SIDECHAIN: [NonZeroU32; 1] = [new_nonzero_u32(2)];
const AUX_NAMES: [&str; NUM_OUTPUTS - 1] = [
    "Out 2", "Out 3", "Out 4", "Out 5", "Out 6", "Out 7", "Out 8", "Out 9", "Out 10", "Out 11",
    "Out 12", "Out 13", "Out 14", "Out 15", "Out 16",
];

/// The parts of the host transport the LFOs need.
#[derive(Debug, Clone, Copy)]
struct TransportInfo {
    tempo: f64,
    playing: bool,
    pos_beats: Option<f64>,
}

impl TransportInfo {
    const STOPPED: Self = Self {
        tempo: DEFAULT_TEMPO,
        playing: false,
        pos_beats: None,
    };

    fn from_host(t: &Transport) -> Self {
        Self {
            tempo: t.tempo.unwrap_or(DEFAULT_TEMPO),
            playing: t.playing,
            pos_beats: t.pos_beats(),
        }
    }
}

pub enum Task {
    /// Decode a file and audition it, see [`SampleBank::audition()`]
    Audition(std::path::PathBuf),
    /// Load a sample slot, see [`klopfer_dsp::slot_index()`]
    Load { slot: usize, path: Option<String> },
    /// Render the time stretched version of a sample slot
    Stretch {
        slot: usize,
        key: StretchKey,
        generation: u64,
    },
    /// Store the MIDI learn assignments in the plugin state
    PersistMidiMap,
}

pub struct Klopfer {
    params: Arc<KlopferParams>,
    editor_state: Arc<EguiEditorState>,
    /// The [`KlopferParams::state_gen`] last applied
    state_gen: u64,
    bank: Arc<SampleBank>,
    learn: Arc<MidiLearn>,
    recorder: Arc<Recorder>,
    recorder_audio: RecorderAudio,
    engine: Engine,
    /// The audio thread's own view of the loaded samples, see [`SampleBank`]
    audio: AudioSlots,
    /// Browser previews, mixed into the main output
    audition: Audition,
    /// Pads whose time stretching is on
    stretch_on: [bool; NUM_PADS],
    process_mode: ProcessMode,
    settings: [PadSettings; NUM_PADS],
    layers: [PadLayers; NUM_PADS],
    notes: [u8; NUM_PADS],
    /// MIDI learn values the host doesn't know about yet
    overrides: Vec<Override>,
    meter_decay: f32,
    /// Master gain target (with MIDI learn applied), the current smoothed gain and the
    /// per-sample smoothing coefficient
    master_target: f32,
    master_gain: f32,
    master_coeff: f32,
}

/// Development helper for the standalone build: `KLOPFER_KIT=path/to/kit.klopfer` preloads a
/// kit's samples, since the standalone can't restore plugin state on its own.
#[cfg(feature = "standalone")]
fn preload_dev_kit(params: &KlopferParams) {
    let Ok(path) = std::env::var("KLOPFER_KIT") else {
        return;
    };
    match kit::load(std::path::Path::new(&path)) {
        Ok(state) => {
            for (pad, name) in kit::pad_names(&state).into_iter().enumerate() {
                *params.pads[pad].layers.write() = kit::pad_layers(&state, pad);
                *params.pads[pad].name.write() = name;
            }
        }
        Err(e) => nice_error!("Cannot load KLOPFER_KIT: {e}"),
    }
}

impl Default for Klopfer {
    fn default() -> Self {
        let params = KlopferParams::default();
        #[cfg(feature = "standalone")]
        preload_dev_kit(&params);

        let params = Arc::new(params);
        // Built after moving the parameters into the Arc, the learn table keeps pointers to them
        let learn = Arc::new(MidiLearn::new(&*params));

        Self {
            params,
            editor_state: editor::editor_state(),
            state_gen: 0,
            bank: Arc::new(SampleBank::default()),
            learn,
            recorder: Arc::new(Recorder::default()),
            recorder_audio: RecorderAudio::default(),
            engine: Engine::new(44100.0),
            audio: AudioSlots::default(),
            audition: Audition::default(),
            stretch_on: [false; NUM_PADS],
            process_mode: ProcessMode::Realtime,
            settings: [PadSettings::default(); NUM_PADS],
            layers: [PadLayers::default(); NUM_PADS],
            notes: [0; NUM_PADS],
            overrides: Vec::with_capacity(MAX_OVERRIDES),
            meter_decay: 0.0,
            master_target: 1.0,
            master_gain: 1.0,
            master_coeff: 0.0,
        }
    }
}

/// The samples voices play: the stretched version where time stretching is on and rendered,
/// the original otherwise.
struct Playing<'a> {
    audio: &'a AudioSlots,
    stretch_on: &'a [bool; NUM_PADS],
}

impl SampleSource for Playing<'_> {
    #[inline]
    fn sample(&self, slot: usize) -> Option<&Sample> {
        let original = self.audio.originals.get(slot)?.as_deref();
        if self.stretch_on[slot / NUM_LAYERS] {
            self.audio.stretched[slot].as_deref().or(original)
        } else {
            original
        }
    }
}

impl Klopfer {
    /// Requests renderings for pads whose stretch settings changed. Each change is requested
    /// once; `request` runs or schedules the task.
    fn request_stretches(&mut self, tempo: f64, request: &mut impl FnMut(Task)) {
        let v = Overrides(&self.overrides);
        for (pad, p) in self.params.pads.iter().enumerate() {
            let mode = v.enumv(&p.stretch_mode).to_dsp();
            self.stretch_on[pad] = mode.is_some();
            let Some(mode) = mode else {
                continue;
            };

            let tune = v.int(&p.tune) as f32 + v.float(&p.fine) / 100.0;
            for layer in 0..NUM_LAYERS {
                let slot = slot_index(pad, layer);
                let Some(original) = self.audio.originals[slot].as_deref() else {
                    continue;
                };
                let ratio = if v.bool(&p.stretch_sync) {
                    // Fit the playback region to the target length
                    let region = (v.float(&p.end) - v.float(&p.start)).abs().max(1e-3)
                        * original.duration_secs();
                    let target = v.enumv(&p.stretch_beats).beats() * 60.0 / tempo;
                    target as f32 / region
                } else {
                    v.float(&p.stretch_length)
                };
                let pitch = tune + self.layers[pad].layers[layer].tune;

                let key = StretchKey::new(mode, ratio, pitch);
                if self.audio.stretch_requested[slot] != Some(key) {
                    self.audio.stretch_requested[slot] = Some(key);
                    let generation = self.bank.next_stretch_gen(slot);
                    request(Task::Stretch {
                        slot,
                        key,
                        generation,
                    });
                }
            }
        }
    }

    fn update_settings(&mut self, transport: TransportInfo) {
        let sample_rate = self.engine.sample_rate();
        let tempo = transport.tempo;
        let v = Overrides(&self.overrides);
        self.master_target = v.float(&self.params.master);

        for (i, pad) in self.params.pads.iter().enumerate() {
            self.settings[i] = PadSettings::new(&pad.to_dsp(&v, tempo), sample_rate);
            self.notes[i] = v.int(&pad.note) as u8;
            // The GUI holds this lock only briefly; keep last block's layers if it's busy
            if let Some(layers) = pad.layers.try_read() {
                self.layers[i] = layers.to_dsp();
            }

            // Free-running synced LFOs follow the song position
            if v.bool(&pad.lfo_sync) && !v.bool(&pad.lfo_retrigger) && transport.playing {
                if let Some(beats) = transport.pos_beats {
                    let cycle = v.enumv(&pad.lfo_division).beats();
                    self.engine.set_lfo_phase(i, (beats / cycle).fract() as f32);
                }
            }
        }
    }

    fn note_on(&mut self, note: u8, velocity: f32) {
        // Several pads may share a note, they're layered then
        for pad in 0..NUM_PADS {
            if self.notes[pad] == note {
                let samples = Playing {
                    audio: &self.audio,
                    stretch_on: &self.stretch_on,
                };
                self.engine
                    .note_on(pad, velocity, &samples, &self.settings, &self.layers);
            }
        }
    }

    fn note_off(&mut self, note: u8) {
        for pad in 0..NUM_PADS {
            if self.notes[pad] == note {
                self.engine.note_off(pad, &self.settings);
            }
        }
    }
}

impl Plugin for Klopfer {
    const NAME: &'static str = "Klopfer";
    const VENDOR: &'static str = "Klopfer";
    const URL: &'static str = "https://example.com/klopfer";
    const EMAIL: &'static str = "info@example.com";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[
        AudioIOLayout {
            main_input_channels: None,
            main_output_channels: NonZeroU32::new(2),
            aux_input_ports: &SIDECHAIN,
            aux_output_ports: &AUX_OUTPUTS,
            names: PortNames {
                layout: Some("Multi-Out"),
                main_input: None,
                main_output: Some("Main"),
                aux_inputs: &["Sidechain"],
                aux_outputs: &AUX_NAMES,
            },
        },
        AudioIOLayout {
            main_input_channels: None,
            main_output_channels: NonZeroU32::new(2),
            aux_input_ports: &SIDECHAIN,
            names: PortNames {
                layout: Some("Stereo"),
                main_output: Some("Main"),
                aux_inputs: &["Sidechain"],
                ..PortNames::const_default()
            },
            ..AudioIOLayout::const_default()
        },
    ];

    // CCs are needed for MIDI learn
    const MIDI_INPUT: MidiConfig = MidiConfig::MidiCCs;
    const SAMPLE_ACCURATE_AUTOMATION: bool = false;

    type Editor = nice_plug_egui::EguiEditor<editor::KlopferEditor>;
    type SysExMessage = ();
    type BackgroundTask = Task;

    fn task_executor(&mut self) -> TaskExecutor<Self> {
        let bank = self.bank.clone();
        let params = self.params.clone();
        let learn = self.learn.clone();
        Box::new(move |task| match task {
            Task::Load { slot, path } => bank.load(slot, path),
            Task::Audition(path) => match loader::load_sample(&path) {
                Ok(sample) => bank.audition(Some(sample)),
                Err(e) => nice_warn!("Cannot audition {}: {e}", path.display()),
            },
            Task::Stretch {
                slot,
                key,
                generation,
            } => bank.render_stretch(slot, key, generation),
            Task::PersistMidiMap => *params.midi_map.write() = learn.assignments(),
        })
    }

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, async_executor: AsyncExecutor<Self>) -> Option<Self::Editor> {
        editor::create(
            self.editor_state.clone(),
            self.params.clone(),
            self.bank.clone(),
            self.learn.clone(),
            self.recorder.clone(),
            async_executor,
        )
    }

    fn activate(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        context: &mut impl ActivateContext<Self>,
    ) -> bool {
        self.engine = Engine::new(buffer_config.sample_rate);
        self.recorder_audio
            .set_sample_rate(&self.recorder, buffer_config.sample_rate);
        self.meter_decay = 0.1f32.powf(1.0 / (METER_DECAY_MS / 1000.0 * buffer_config.sample_rate));
        self.master_coeff =
            (-1.0 / (MASTER_SMOOTHING_MS / 1000.0 * buffer_config.sample_rate)).exp();

        self.overrides.clear();
        // Load samples synchronously so offline renders never start without them
        self.state_gen = self.params.state_gen.load(Ordering::Acquire);
        sync_state(&self.params, &self.bank, &self.learn);
        let engine = &mut self.engine;
        self.bank
            .apply_swaps(&mut self.audio, |slot| engine.kill_slot(slot));
        self.update_settings(TransportInfo::STOPPED);

        // Render time stretched samples right away as well
        self.process_mode = buffer_config.process_mode;
        self.request_stretches(DEFAULT_TEMPO, &mut |task| context.execute(task));
        let engine = &mut self.engine;
        self.bank
            .apply_swaps(&mut self.audio, |slot| engine.kill_slot(slot));
        true
    }

    fn reset(&mut self) {
        self.engine.reset();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        // A state was loaded while active. nih-plug reinitialized the plugin for that, nice-plug
        // doesn't, so apply it here just as synchronously: renders must start with the samples.
        // A no-op for samples the editor loaded already.
        let state_loaded = self.params.state_gen.load(Ordering::Acquire) != self.state_gen;
        if state_loaded {
            self.state_gen = self.params.state_gen.load(Ordering::Acquire);
            self.overrides.clear();
            permit_alloc(|| sync_state(&self.params, &self.bank, &self.learn));
        }

        let engine = &mut self.engine;
        self.bank
            .apply_swaps(&mut self.audio, |slot| engine.kill_slot(slot));
        self.bank.apply_audition(&mut self.audition);
        prune_overrides(&mut self.overrides);
        let transport = TransportInfo::from_host(context.transport());
        self.update_settings(transport);

        if self.process_mode == ProcessMode::Offline || state_loaded {
            // Offline renders can't wait for the background thread: render here and use the
            // result in this very block
            let bank = self.bank.clone();
            self.request_stretches(transport.tempo, &mut |task| {
                if let Task::Stretch {
                    slot,
                    key,
                    generation,
                } = task
                {
                    permit_alloc(|| bank.render_stretch(slot, key, generation));
                }
            });
            let engine = &mut self.engine;
            self.bank
                .apply_swaps(&mut self.audio, |slot| engine.kill_slot(slot));
        } else {
            self.request_stretches(transport.tempo, &mut |task| {
                context.execute_background(task)
            });
        }

        while let Some(preview) = self.bank.previews.pop() {
            match preview {
                Preview::On { pad, velocity } => {
                    let samples = Playing {
                        audio: &self.audio,
                        stretch_on: &self.stretch_on,
                    };
                    self.engine
                        .note_on(pad, velocity, &samples, &self.settings, &self.layers)
                }
                Preview::Off { pad } => self.engine.note_off(pad, &self.settings),
            }
        }

        let num_samples = buffer.samples();
        if let Some(input) = aux.inputs.first_mut() {
            let channels = input.as_slice();
            if let Some((left, rest)) = channels.split_first() {
                let right = rest.first().unwrap_or(left);
                self.recorder_audio.process(&self.recorder, left, right);
            }
        }

        let main = buffer.as_slice();
        let has_aux = !aux.outputs.is_empty();
        for out in aux.outputs.iter_mut() {
            for ch in out.as_slice() {
                ch.fill(0.0);
            }
        }

        let sample_rate = self.engine.sample_rate();
        let mut next_event = context.next_event();
        for i in 0..num_samples {
            while let Some(event) = next_event {
                if event.timing() as usize > i {
                    break;
                }
                match event {
                    NoteEvent::NoteOn {
                        key: Key::Number(note),
                        velocity,
                        ..
                    } => self.note_on(note, velocity),
                    NoteEvent::NoteOff { key, .. } => match key.number() {
                        Some(note) => self.note_off(note),
                        // Wildcards address all notes
                        None => (0..128).for_each(|note| self.note_off(note)),
                    },
                    NoteEvent::Choke { key, .. } => match key.number() {
                        Some(note) => self.note_off(note),
                        None => self.engine.choke_all(),
                    },
                    // All sound off / all notes off
                    NoteEvent::MidiCC { cc: 120 | 123, .. } => self.engine.choke_all(),
                    NoteEvent::MidiCC { cc, value, .. } => {
                        if let Some((ptr, value)) = self.learn.handle_cc(cc, value) {
                            set_override(&mut self.overrides, ptr, value);
                            self.update_settings(transport);
                        }
                    }
                    _ => {}
                }
                next_event = context.next_event();
            }

            let mut bus = [[0.0f32; 2]; NUM_OUTPUTS];
            let samples = Playing {
                audio: &self.audio,
                stretch_on: &self.stretch_on,
            };
            self.engine.render_frame(&samples, &self.settings, &mut bus);
            self.master_gain =
                self.master_target + (self.master_gain - self.master_target) * self.master_coeff;
            let master = self.master_gain;

            if !has_aux {
                // Stereo layout: fold all buses into the main output
                for b in 1..NUM_OUTPUTS {
                    bus[0][0] += bus[b][0];
                    bus[0][1] += bus[b][1];
                }
            }
            let audition = self.audition.next(sample_rate);
            bus[0][0] += audition[0];
            bus[0][1] += audition[1];
            for (c, ch) in main.iter_mut().enumerate().take(2) {
                ch[i] = bus[0][c] * master;
            }
            for (k, out) in aux.outputs.iter_mut().enumerate() {
                for (c, ch) in out.as_slice().iter_mut().enumerate().take(2) {
                    ch[i] = bus[k + 1][c] * master;
                }
            }
        }

        self.engine.advance_lfo_phases(&self.settings, num_samples);
        if self
            .learn
            .map_changed
            .swap(false, std::sync::atomic::Ordering::AcqRel)
        {
            context.execute_gui(Task::PersistMidiMap);
        }

        let peaks = self.engine.take_pad_peaks();
        if self.editor_state.is_open() {
            let samples = Playing {
                audio: &self.audio,
                stretch_on: &self.stretch_on,
            };
            let heads = self.engine.playheads(&samples);
            for (head, pos) in self.bank.playheads.iter().zip(heads) {
                head.store(pos, Ordering::Relaxed);
            }

            let decay = self.meter_decay.powi(num_samples as i32);
            for (meter, peak) in self.bank.meters.iter().zip(peaks) {
                let old = meter.load(Ordering::Relaxed);
                meter.store(peak.max(old * decay), Ordering::Relaxed);
            }
        }

        ProcessStatus::Normal
    }
}

impl ClapPlugin for Klopfer {
    const CLAP_ID: &'static str = "com.klopfer.drum-sampler";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("A 32 pad drum sampler");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::Instrument,
        ClapFeature::Sampler,
        ClapFeature::DrumMachine,
        ClapFeature::Stereo,
    ];
}

impl Vst3Plugin for Klopfer {
    const VST3_CLASS_ID: [u8; 16] = *b"KlopferDrumSmplr";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[
        Vst3SubCategory::Instrument,
        Vst3SubCategory::Drum,
        Vst3SubCategory::Sampler,
    ];
}

nice_export_clap!(Klopfer);
nice_export_vst3!(Klopfer);

/// Applies the loaded state: MIDI learn assignments, single-sample paths from old states, and the
/// samples. Blocking; slots that already have their sample are skipped.
fn sync_state(params: &KlopferParams, bank: &SampleBank, learn: &MidiLearn) {
    learn.set_assignments(&params.midi_map.read());
    for (pad, p) in params.pads.iter().enumerate() {
        // States from before layers existed have a single sample path
        if let Some(path) = p.sample_path.write().take() {
            let mut layers = p.layers.write();
            if layers.layers[0].path.is_none() {
                layers.layers[0].path = Some(path);
            }
        }

        let paths: Vec<Option<String>> = p
            .layers
            .read()
            .layers
            .iter()
            .map(|l| l.path.clone())
            .collect();
        for (layer, path) in paths.into_iter().enumerate().take(NUM_LAYERS) {
            let slot = slot_index(pad, layer);
            if bank.request(slot, path.clone()) {
                bank.load(slot, path);
            }
        }
    }
}
