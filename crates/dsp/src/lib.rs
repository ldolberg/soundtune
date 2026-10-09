//! Real-time voice effects used by the SoundTune console.
//!
//! Everything here is allocation free once constructed, so it is safe to run
//! inside an audio callback.

mod distortion;
mod doubler;
mod dynamics;
mod eq;
mod guard;
mod key;
mod music;
mod params;
mod pitch;
mod processor;
mod reverb;
mod sing;
mod tracker;

pub use distortion::Distortion;
pub use doubler::Doubler;
pub use dynamics::{Compressor, NoiseGate};
pub use eq::{Biquad, Brightness};
pub use guard::{FeedbackGuard, Limiter};
pub use key::{Key, KeyDetector};
pub use music::{hz_to_midi, note_name, snap_to_mask, snap_to_scale, MusicScale, NOTE_NAMES};
pub use params::{AtomicF32, Meters, Params, Toggle};
pub use pitch::PitchShifter;
pub use processor::Processor;
pub use reverb::Reverb;
pub use sing::{apply_sing_mode, bypass_all, Voice};
pub use tracker::PitchTracker;
