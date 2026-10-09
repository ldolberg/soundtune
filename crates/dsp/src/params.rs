use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use crate::SongState;

/// An `f32` that can be shared between the UI thread and the audio thread.
#[derive(Debug)]
pub struct AtomicF32(AtomicU32);

impl AtomicF32 {
    pub fn new(v: f32) -> Self {
        Self(AtomicU32::new(v.to_bits()))
    }

    pub fn get(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }

    pub fn set(&self, v: f32) {
        self.0.store(v.to_bits(), Ordering::Relaxed);
    }

    /// Returns the stored value and resets it to zero.
    pub fn take(&self) -> f32 {
        f32::from_bits(self.0.swap(0, Ordering::Relaxed))
    }

    /// Keeps the larger of the stored value and `v`. Only valid for
    /// non-negative values, whose bit patterns sort like the floats do.
    pub fn fetch_max(&self, v: f32) {
        self.0.fetch_max(v.max(0.0).to_bits(), Ordering::Relaxed);
    }
}

/// A shared on/off flag.
#[derive(Debug)]
pub struct Toggle(AtomicBool);

impl Toggle {
    pub fn new(v: bool) -> Self {
        Self(AtomicBool::new(v))
    }

    pub fn get(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    pub fn set(&self, v: bool) {
        self.0.store(v, Ordering::Relaxed);
    }

    /// Returns the flag and clears it.
    pub fn take(&self) -> bool {
        self.0.swap(false, Ordering::Relaxed)
    }
}

/// Effect settings written by the UI and read by the audio thread.
#[derive(Debug)]
pub struct Params {
    /// Output gain, linear.
    pub master: AtomicF32,

    pub pitch_on: Toggle,
    /// Pitch shift in semitones (positive is higher).
    pub pitch_semitones: AtomicF32,

    pub tune_on: Toggle,
    /// Notes the autotune may snap to, one bit per pitch class (bit 0 = C).
    pub tune_mask: AtomicU32,
    /// 0 = instant (robotic) correction, 1 = slow and natural.
    pub tune_speed: AtomicF32,
    /// How far notes are pulled to the scale: 0 = not at all, 1 = fully.
    pub tune_amount: AtomicF32,

    pub dist_on: Toggle,
    pub drive: AtomicF32,
    pub tone: AtomicF32,
    pub dist_mix: AtomicF32,

    pub verb_on: Toggle,
    pub room: AtomicF32,
    pub damping: AtomicF32,
    pub verb_mix: AtomicF32,

    pub gate_on: Toggle,
    /// Level (dBFS) above which the noise gate opens.
    pub gate_threshold: AtomicF32,

    pub comp_on: Toggle,
    /// Compressor threshold in dBFS.
    pub comp_threshold: AtomicF32,
    /// Compression ratio, 1 = none.
    pub comp_ratio: AtomicF32,

    pub bright_on: Toggle,
    /// Presence and air boost, 0..1.
    pub brightness: AtomicF32,

    pub double_on: Toggle,
    /// Level of the doubled voices, 0..1.
    pub double_mix: AtomicF32,
    /// Detune of each doubled voice in cents.
    pub double_detune: AtomicF32,

    /// Autotune follows the key detected from the singing instead of
    /// `tune_mask`.
    pub auto_key: Toggle,
    /// Turn the output down when acoustic feedback is detected.
    pub feedback_guard: Toggle,

    /// Backing track, song timelines and transport.
    pub song: SongState,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            master: AtomicF32::new(1.0),
            pitch_on: Toggle::new(false),
            pitch_semitones: AtomicF32::new(7.0),
            tune_on: Toggle::new(false),
            tune_mask: AtomicU32::new(0xFFF),
            tune_speed: AtomicF32::new(0.1),
            tune_amount: AtomicF32::new(1.0),
            dist_on: Toggle::new(false),
            drive: AtomicF32::new(0.5),
            tone: AtomicF32::new(0.5),
            dist_mix: AtomicF32::new(1.0),
            verb_on: Toggle::new(false),
            room: AtomicF32::new(0.7),
            damping: AtomicF32::new(0.5),
            verb_mix: AtomicF32::new(0.3),
            gate_on: Toggle::new(false),
            gate_threshold: AtomicF32::new(-48.0),
            comp_on: Toggle::new(false),
            comp_threshold: AtomicF32::new(-20.0),
            comp_ratio: AtomicF32::new(3.0),
            bright_on: Toggle::new(false),
            brightness: AtomicF32::new(0.5),
            double_on: Toggle::new(false),
            double_mix: AtomicF32::new(0.5),
            double_detune: AtomicF32::new(10.0),
            auto_key: Toggle::new(false),
            feedback_guard: Toggle::new(true),
            song: SongState::default(),
        }
    }
}

/// Values the audio thread reports back to the UI.
#[derive(Debug)]
pub struct Meters {
    /// Input peak since the UI last read it.
    pub input: AtomicF32,
    /// Output peak since the UI last read it.
    pub output: AtomicF32,
    /// Detected input pitch in Hz, 0 when silent or unvoiced.
    pub pitch_hz: AtomicF32,
    /// Key found by auto key detection (`Key::encode`), `Key::NONE` if none.
    pub key: AtomicU32,
    /// Compressor gain reduction in dB (peak since the UI last read it).
    pub reduction: AtomicF32,
    /// Set while the feedback guard is turning the output down.
    pub feedback: Toggle,
    /// Note (MIDI number) the autotune is steering the voice to, in the
    /// singer's own pitch (before High Pitch), 0 when there is none
    /// (silence). Only updated while the autotune is on.
    pub target_note: AtomicF32,
    /// While following a song: the notes allowed right now (bit 0 = C),
    /// 0 when not following.
    pub song_mask: AtomicU32,
    /// While following a song: the current chord's notes, 0 if none.
    pub song_chord: AtomicU32,
    /// While following a song: the guide note (MIDI number, after the
    /// shift), `NO_NOTE` when no guide note is sounding.
    pub guide_note: AtomicU32,
}

/// `Meters::guide_note` when there is no guide note.
pub const NO_NOTE: u32 = u32::MAX;

impl Default for Meters {
    fn default() -> Self {
        Self {
            input: AtomicF32::new(0.0),
            output: AtomicF32::new(0.0),
            pitch_hz: AtomicF32::new(0.0),
            key: AtomicU32::new(crate::Key::NONE),
            reduction: AtomicF32::new(0.0),
            feedback: Toggle::new(false),
            target_note: AtomicF32::new(0.0),
            song_mask: AtomicU32::new(0),
            song_chord: AtomicU32::new(0),
            guide_note: AtomicU32::new(NO_NOTE),
        }
    }
}
