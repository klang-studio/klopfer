//! Realtime DSP building blocks for Klopfer.
//!
//! Nothing in here depends on the plugin framework, so everything can be unit tested in isolation.
//! None of the `process`/`render` style functions allocate.

pub mod biquad;
pub mod engine;
pub mod envelope;
pub mod interp;
pub mod lofi;
pub mod pad;
pub mod sample;
pub mod svf;
mod voice;

pub use engine::{Engine, MAX_VOICES, NUM_OUTPUTS, NUM_PADS};
pub use pad::{FilterMode, PadParams, PadSettings};
pub use sample::Sample;
