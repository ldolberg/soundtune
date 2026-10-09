//! Real-time voice effects used by the SoundTune console.
//!
//! Everything the processor and the song player run is allocation free once
//! constructed, so it is safe inside an audio callback. The song analysis
//! (chromagram, harmony, MIDI parsing) runs offline on a loader thread.

mod chroma;
mod distortion;
mod doubler;
mod dynamics;
mod eq;
mod guard;
mod guide;
mod harmony;
mod key;
mod music;
mod params;
mod pitch;
mod processor;
mod reverb;
mod sing;
mod tracker;

pub use chroma::{chromagram, Chromagram, CHROMA_HIGH_HZ, CHROMA_LOW_HZ};
pub use distortion::Distortion;
pub use doubler::Doubler;
pub use dynamics::{Compressor, NoiseGate};
pub use eq::{Biquad, Brightness};
pub use guard::{FeedbackGuard, Limiter};
pub use guide::{
    nearest_octave, parse_midi, GuideMelody, GuideNote, MidiPart, MidiSong, TimedNote,
};
pub use harmony::{
    analyse_harmony, detect_chords, estimate_key, harmony_from_chroma, Chord, Harmony,
    HarmonySegment, FRAME_SECS,
};
pub use key::{best_key, Key, KeyDetector};
pub use music::{
    hz_to_midi, note_name, snap_preferring, snap_to_mask, snap_to_scale, MusicScale, NOTE_NAMES,
};
pub use params::{AtomicF32, Meters, Params, Toggle};
pub use pitch::PitchShifter;
pub use processor::Processor;
pub use reverb::Reverb;
pub use sing::{apply_sing_mode, bypass_all, Voice};
pub use tracker::PitchTracker;
