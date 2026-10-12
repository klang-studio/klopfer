//! Realtime DSP building blocks for Klopfer.
//!
//! Nothing in here depends on the plugin framework, so everything can be unit tested in isolation.
//! None of the `process`/`render` style functions allocate.

pub mod biquad;
pub mod engine;
pub mod envelope;
pub mod interp;
pub mod layers;
pub mod lofi;
pub mod modulation;
pub mod pad;
pub mod sample;
pub mod slicing;
pub mod stretch;
pub mod svf;
mod voice;

pub use engine::{
    slot_index, Engine, SampleSource, MAX_VOICES, NUM_OUTPUTS, NUM_PADS, NUM_SAMPLE_SLOTS,
    PADS_PER_BANK,
};
pub use layers::{LayerInfo, LayerMode, PadLayers, NUM_LAYERS};
pub use modulation::{LfoShape, ModSlot, ModSource, ModTarget};
pub use pad::{EnvParams, FilterMode, PadParams, PadSettings};
pub use sample::Sample;
