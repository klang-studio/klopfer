//! Klopfer: a 16 pad drum sampler.

// Triggered by the VST3 bindings' macros, not by our code
#![allow(semicolon_in_expressions_from_non_local_macros)]

use std::num::NonZeroU32;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use klopfer_dsp::{Engine, PadSettings, Sample, NUM_OUTPUTS, NUM_PADS};
use nih_plug::prelude::*;

mod bank;
mod editor;
pub mod kit;
pub mod loader;
mod params;

use bank::{Preview, SampleBank};
use params::KlopferParams;

/// Pad meters fall by 20 dB within this time.
const METER_DECAY_MS: f32 = 300.0;

const AUX_OUTPUTS: [NonZeroU32; NUM_OUTPUTS - 1] = [new_nonzero_u32(2); NUM_OUTPUTS - 1];
const AUX_NAMES: [&str; NUM_OUTPUTS - 1] = [
    "Out 2", "Out 3", "Out 4", "Out 5", "Out 6", "Out 7", "Out 8", "Out 9", "Out 10", "Out 11",
    "Out 12", "Out 13", "Out 14", "Out 15", "Out 16",
];

pub enum Task {
    Load { pad: usize, path: Option<String> },
}

pub struct Klopfer {
    params: Arc<KlopferParams>,
    bank: Arc<SampleBank>,
    engine: Engine,
    /// The audio thread's own view of the loaded samples, see [`SampleBank`]
    slots: [Option<Arc<Sample>>; NUM_PADS],
    settings: [PadSettings; NUM_PADS],
    notes: [u8; NUM_PADS],
    meter_decay: f32,
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
            let names = kit::pad_names(&state);
            for (pad, (path, name)) in kit::sample_paths(&state).into_iter().zip(names).enumerate()
            {
                *params.pads[pad].sample_path.write() = path;
                *params.pads[pad].name.write() = name;
            }
        }
        Err(e) => nih_error!("Cannot load KLOPFER_KIT: {e}"),
    }
}

impl Default for Klopfer {
    fn default() -> Self {
        let params = KlopferParams::default();
        #[cfg(feature = "standalone")]
        preload_dev_kit(&params);

        Self {
            params: Arc::new(params),
            bank: Arc::new(SampleBank::default()),
            engine: Engine::new(44100.0),
            slots: Default::default(),
            settings: [PadSettings::default(); NUM_PADS],
            notes: [0; NUM_PADS],
            meter_decay: 0.0,
        }
    }
}

impl Klopfer {
    fn update_settings(&mut self) {
        let sample_rate = self.engine.sample_rate();
        for (i, pad) in self.params.pads.iter().enumerate() {
            self.settings[i] = PadSettings::new(&pad.to_dsp(), sample_rate);
            self.notes[i] = pad.note.value() as u8;
        }
    }

    fn note_on(&mut self, note: u8, velocity: f32) {
        // Several pads may share a note, they're layered then
        for pad in 0..NUM_PADS {
            if self.notes[pad] == note {
                self.engine
                    .note_on(pad, velocity, &self.slots, &self.settings);
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
            aux_input_ports: &[],
            aux_output_ports: &AUX_OUTPUTS,
            names: PortNames {
                layout: Some("Multi-Out"),
                main_input: None,
                main_output: Some("Main"),
                aux_inputs: &[],
                aux_outputs: &AUX_NAMES,
            },
        },
        AudioIOLayout {
            main_input_channels: None,
            main_output_channels: NonZeroU32::new(2),
            names: PortNames {
                layout: Some("Stereo"),
                main_output: Some("Main"),
                ..PortNames::const_default()
            },
            ..AudioIOLayout::const_default()
        },
    ];

    const MIDI_INPUT: MidiConfig = MidiConfig::Basic;
    const SAMPLE_ACCURATE_AUTOMATION: bool = false;

    type SysExMessage = ();
    type BackgroundTask = Task;

    fn task_executor(&mut self) -> TaskExecutor<Self> {
        let bank = self.bank.clone();
        Box::new(move |task| match task {
            Task::Load { pad, path } => bank.load(pad, path),
        })
    }

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        editor::create(self.params.clone(), self.bank.clone(), async_executor)
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        context: &mut impl InitContext<Self>,
    ) -> bool {
        self.engine = Engine::new(buffer_config.sample_rate);
        self.meter_decay = 0.1f32.powf(1.0 / (METER_DECAY_MS / 1000.0 * buffer_config.sample_rate));

        // Load samples synchronously so offline renders never start without them. This is a
        // no-op for pads whose sample is already loaded.
        for (pad, p) in self.params.pads.iter().enumerate() {
            let path = p.sample_path.read().clone();
            if self.bank.request(pad, path.clone()) {
                context.execute(Task::Load { pad, path });
            }
        }
        let engine = &mut self.engine;
        self.bank
            .apply_swaps(&mut self.slots, |pad| engine.kill_pad(pad));

        self.update_settings();
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
        let engine = &mut self.engine;
        self.bank
            .apply_swaps(&mut self.slots, |pad| engine.kill_pad(pad));
        self.update_settings();

        while let Some(preview) = self.bank.previews.pop() {
            match preview {
                Preview::On { pad, velocity } => {
                    self.engine
                        .note_on(pad, velocity, &self.slots, &self.settings)
                }
                Preview::Off { pad } => self.engine.note_off(pad, &self.settings),
            }
        }

        let num_samples = buffer.samples();
        let main = buffer.as_slice();
        let has_aux = !aux.outputs.is_empty();
        for out in aux.outputs.iter_mut() {
            for ch in out.as_slice() {
                ch.fill(0.0);
            }
        }

        let mut next_event = context.next_event();
        for i in 0..num_samples {
            while let Some(event) = next_event {
                if event.timing() as usize > i {
                    break;
                }
                match event {
                    NoteEvent::NoteOn { note, velocity, .. } => self.note_on(note, velocity),
                    NoteEvent::NoteOff { note, .. } => self.note_off(note),
                    NoteEvent::Choke { note, .. } => self.note_off(note),
                    _ => {}
                }
                next_event = context.next_event();
            }

            let mut bus = [[0.0f32; 2]; NUM_OUTPUTS];
            self.engine
                .render_frame(&self.slots, &self.settings, &mut bus);
            let master = self.params.master.smoothed.next();

            if !has_aux {
                // Stereo layout: fold all buses into the main output
                for b in 1..NUM_OUTPUTS {
                    bus[0][0] += bus[b][0];
                    bus[0][1] += bus[b][1];
                }
            }
            for (c, ch) in main.iter_mut().enumerate().take(2) {
                ch[i] = bus[0][c] * master;
            }
            for (k, out) in aux.outputs.iter_mut().enumerate() {
                for (c, ch) in out.as_slice().iter_mut().enumerate().take(2) {
                    ch[i] = bus[k + 1][c] * master;
                }
            }
        }

        let peaks = self.engine.take_pad_peaks();
        if self.params.editor_state.is_open() {
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
    const CLAP_DESCRIPTION: Option<&'static str> = Some("A 16 pad drum sampler");
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

nih_export_clap!(Klopfer);
nih_export_vst3!(Klopfer);
