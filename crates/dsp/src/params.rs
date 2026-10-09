use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

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

    pub dist_on: Toggle,
    pub drive: AtomicF32,
    pub tone: AtomicF32,
    pub dist_mix: AtomicF32,

    pub verb_on: Toggle,
    pub room: AtomicF32,
    pub damping: AtomicF32,
    pub verb_mix: AtomicF32,
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
            dist_on: Toggle::new(false),
            drive: AtomicF32::new(0.5),
            tone: AtomicF32::new(0.5),
            dist_mix: AtomicF32::new(1.0),
            verb_on: Toggle::new(false),
            room: AtomicF32::new(0.7),
            damping: AtomicF32::new(0.5),
            verb_mix: AtomicF32::new(0.3),
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
}

impl Default for Meters {
    fn default() -> Self {
        Self {
            input: AtomicF32::new(0.0),
            output: AtomicF32::new(0.0),
            pitch_hz: AtomicF32::new(0.0),
        }
    }
}
