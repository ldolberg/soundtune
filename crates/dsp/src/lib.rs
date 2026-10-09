//! Real-time voice effects used by the SoundTune console.
//!
//! Everything here is allocation free once constructed, so it is safe to run
//! inside an audio callback.

mod distortion;
mod music;
mod params;
mod pitch;
mod processor;
mod reverb;
mod tracker;

pub use distortion::Distortion;
pub use music::{hz_to_midi, note_name, snap_to_mask, snap_to_scale, MusicScale, NOTE_NAMES};
pub use params::{AtomicF32, Meters, Params, Toggle};
pub use pitch::PitchShifter;
pub use processor::Processor;
pub use reverb::Reverb;
pub use tracker::PitchTracker;
