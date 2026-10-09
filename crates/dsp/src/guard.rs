//! Output protection: a peak limiter and a feedback (howl) detector.

use crate::dynamics::coef;

/// Instant attack peak limiter. Signals below the ceiling pass untouched.
pub struct Limiter {
    gain: f32,
    release: f32,
}

/// Limiter ceiling, about -1 dBFS.
pub const CEILING: f32 = 0.89;

impl Limiter {
    pub fn new(sample_rate: f32) -> Self {
        Self { gain: 1.0, release: coef(0.1, sample_rate) }
    }

    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        let a = x.abs();
        let needed = if a * self.gain > CEILING { CEILING / a } else { 1.0 };
        if needed < self.gain {
            self.gain = needed;
        } else {
            self.gain += (1.0 - self.gain) * self.release;
        }
        x * self.gain
    }
}

/// Analysis window of the feedback detector, seconds.
const WINDOW: f32 = 0.1;
/// Consecutive howling windows before we act.
const TRIGGER_WINDOWS: u32 = 8;
/// RMS level (about -14 dBFS) below which nothing counts as howling.
const MIN_RMS: f32 = 0.2;
/// How long the output stays turned down after the howl stops, seconds.
const HOLD: f32 = 3.0;
/// Output gain while ducking (about -20 dB).
const DUCK: f32 = 0.1;

/// Detects acoustic feedback (speakers into the microphone) and turns the
/// output down until it stops.
///
/// Feedback is a loud, sustained, almost pure sine. Voice is not: its
/// harmonics make zero crossings irregular and put far more energy in the
/// second difference than a sine of the same frequency would have. Each
/// 100 ms window measures the zero crossing rhythm and compares the second
/// difference energy with the value a pure sine at that frequency predicts.
pub struct FeedbackGuard {
    window: u32,
    n: u32,
    sum_sq: f32,
    sum_d2: f32,
    x1: f32,
    x2: f32,
    last_cross: u32,
    crossings: u32,
    sum_iv: f32,
    sum_iv2: f32,
    howling: u32,
    hold: u32,
    hold_len: u32,
    gain: f32,
    attack: f32,
    release: f32,
}

impl FeedbackGuard {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            window: (sample_rate * WINDOW) as u32,
            n: 0,
            sum_sq: 0.0,
            sum_d2: 0.0,
            x1: 0.0,
            x2: 0.0,
            last_cross: 0,
            crossings: 0,
            sum_iv: 0.0,
            sum_iv2: 0.0,
            howling: 0,
            hold: 0,
            hold_len: (sample_rate * HOLD) as u32,
            gain: 1.0,
            attack: coef(0.05, sample_rate),
            release: coef(0.5, sample_rate),
        }
    }

    /// True while the output is turned down because of feedback.
    pub fn active(&self) -> bool {
        self.hold > 0
    }

    /// Analyses `x` (the output before protection) and returns it with the
    /// protection gain applied.
    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        let d2 = x - 2.0 * self.x1 + self.x2;
        self.sum_sq += x * x;
        self.sum_d2 += d2 * d2;
        if (x >= 0.0) != (self.x1 >= 0.0) {
            if self.crossings > 0 {
                let iv = (self.n - self.last_cross) as f32;
                self.sum_iv += iv;
                self.sum_iv2 += iv * iv;
            }
            self.crossings += 1;
            self.last_cross = self.n;
        }
        self.x2 = self.x1;
        self.x1 = x;
        self.n += 1;
        if self.n >= self.window {
            self.end_window();
        }

        if self.hold > 0 {
            self.hold -= 1;
        }
        let (target, speed) = if self.hold > 0 { (DUCK, self.attack) } else { (1.0, self.release) };
        self.gain += (target - self.gain) * speed;
        x * self.gain
    }

    fn end_window(&mut self) {
        if self.is_howl() {
            self.howling += 1;
            if self.howling >= TRIGGER_WINDOWS || self.hold > 0 {
                // Keep ducking while it howls, then hold a while longer.
                self.hold = self.hold_len;
            }
        } else {
            self.howling = 0;
        }
        self.n = 0;
        self.sum_sq = 0.0;
        self.sum_d2 = 0.0;
        self.crossings = 0;
        self.sum_iv = 0.0;
        self.sum_iv2 = 0.0;
        self.last_cross = 0;
    }

    fn is_howl(&self) -> bool {
        let rms = (self.sum_sq / self.n as f32).sqrt();
        // Loud, and at least 50 Hz worth of zero crossings.
        if rms < MIN_RMS || self.crossings < 10 {
            return false;
        }
        let intervals = (self.crossings - 1) as f32;
        let mean = self.sum_iv / intervals;
        let var = (self.sum_iv2 / intervals - mean * mean).max(0.0);
        if var.sqrt() > 0.1 * mean {
            return false;
        }
        // A sine of angular frequency w has E[d2^2] / E[x^2] = (2 - 2 cos w)^2.
        let w = std::f32::consts::PI / mean;
        let predicted = (2.0 - 2.0 * w.cos()).powi(2);
        let measured = self.sum_d2 / self.sum_sq;
        measured < predicted * 1.6
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    #[test]
    fn limiter_passes_quiet_and_catches_peaks() {
        let mut l = Limiter::new(48_000.0);
        assert_eq!(l.process(0.5), 0.5);
        assert!(l.process(3.0).abs() <= CEILING + 1e-6);
        // Recovers afterwards.
        let mut y = 0.0;
        for _ in 0..48_000 {
            y = l.process(0.5);
        }
        assert!((y - 0.5).abs() < 1e-3);
    }

    /// Runs `signal` through a guard; returns when it first triggered.
    fn trigger_time(signal: impl Iterator<Item = f32>, sr: f32) -> Option<f32> {
        let mut g = FeedbackGuard::new(sr);
        for (n, x) in signal.enumerate() {
            g.process(x);
            if g.active() {
                return Some(n as f32 / sr);
            }
        }
        None
    }

    #[test]
    fn detects_howl() {
        let sr = 48_000.0;
        // A 2.3 kHz tone building up, like speaker feedback.
        let howl = (0..3 * sr as usize).map(|n| {
            let t = n as f32 / sr;
            (0.2 + 0.25 * t).min(0.9) * (2.0 * PI * 2300.0 * t).sin()
        });
        let t = trigger_time(howl, sr).expect("feedback detected");
        assert!(t < 1.2, "took {t} s");
    }

    #[test]
    fn ducks_and_recovers() {
        let sr = 44_100.0;
        let mut g = FeedbackGuard::new(sr);
        let mut out = 0.0f32;
        for n in 0..(2 * sr as usize) {
            out = g.process(0.8 * (2.0 * PI * 900.0 * n as f32 / sr).sin()).abs().max(out * 0.999);
        }
        assert!(g.active());
        assert!(out < 0.2, "output still {out}");
        for _ in 0..(5 * sr as usize) {
            g.process(0.0);
        }
        assert!(!g.active());
        assert!(g.process(0.5) > 0.45);
    }

    #[test]
    fn loud_singing_is_not_feedback() {
        let sr = 48_000.0;
        // A steady, loud, autotuned-like vowel: 220 Hz with harmonics.
        let voice = (0..4 * sr as usize).map(|n| {
            let t = n as f32 / sr;
            (1..=8).map(|k| 0.35 / k as f32 * (2.0 * PI * 220.0 * k as f32 * t + k as f32).sin()).sum()
        });
        assert_eq!(trigger_time(voice, sr), None);

        // Breathy noise is not feedback either.
        let hiss = crate::dynamics::tests::noise(4 * sr as usize, 0.8).into_iter();
        assert_eq!(trigger_time(hiss, sr), None);
    }
}
