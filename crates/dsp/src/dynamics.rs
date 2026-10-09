//! Level processors: noise gate and compressor.

/// One-pole smoothing coefficient for a time constant in seconds.
pub(crate) fn coef(seconds: f32, sample_rate: f32) -> f32 {
    if seconds <= 0.0 {
        1.0
    } else {
        1.0 - (-1.0 / (seconds * sample_rate)).exp()
    }
}

pub(crate) fn db_to_lin(db: f32) -> f32 {
    (db * (std::f32::consts::LN_10 / 20.0)).exp()
}

/// Noise gate that mutes breaths and room noise between phrases.
///
/// A peak follower opens the gate instantly above the threshold; it closes
/// 6 dB below it (hysteresis) after a short hold, with a soft release so
/// word endings are not chopped.
pub struct NoiseGate {
    env: f32,
    gain: f32,
    open: bool,
    hold: u32,
    hold_len: u32,
    env_rel: f32,
    attack: f32,
    release: f32,
}

impl NoiseGate {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            env: 0.0,
            gain: 0.0,
            open: false,
            hold: 0,
            hold_len: (sample_rate * 0.06) as u32,
            env_rel: 1.0 - coef(0.03, sample_rate),
            attack: coef(0.001, sample_rate),
            release: coef(0.08, sample_rate),
        }
    }

    /// True while the gate lets the signal through.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// `threshold_db` is the opening level in dBFS.
    #[inline]
    pub fn process(&mut self, x: f32, threshold_db: f32) -> f32 {
        let a = x.abs();
        self.env = if a > self.env {
            a
        } else {
            self.env * self.env_rel
        };

        let open_at = db_to_lin(threshold_db);
        if self.env > open_at {
            self.open = true;
            self.hold = self.hold_len;
        } else if self.env < open_at * 0.5 {
            if self.hold > 0 {
                self.hold -= 1;
            } else {
                self.open = false;
            }
        }

        let (target, speed) = if self.open {
            (1.0, self.attack)
        } else {
            (0.0, self.release)
        };
        self.gain += (target - self.gain) * speed;
        x * self.gain
    }
}

/// Smooth feed-forward compressor with a soft knee and automatic makeup
/// gain, so quiet and loud singing come out at a similar level.
pub struct Compressor {
    /// Mean square level (smoothed).
    ms: f32,
    /// Current gain change in dB (0 or negative).
    gr: f32,
    ms_coef: f32,
    attack: f32,
    release: f32,
}

const KNEE_DB: f32 = 6.0;

impl Compressor {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            ms: 0.0,
            gr: 0.0,
            ms_coef: coef(0.01, sample_rate),
            attack: coef(0.005, sample_rate),
            release: coef(0.12, sample_rate),
        }
    }

    /// Static curve: gain change in dB for an input level in dB.
    pub fn curve(level_db: f32, threshold_db: f32, ratio: f32) -> f32 {
        let slope = 1.0 / ratio.max(1.0) - 1.0;
        let over = level_db - threshold_db;
        if 2.0 * over < -KNEE_DB {
            0.0
        } else if 2.0 * over <= KNEE_DB {
            slope * (over + KNEE_DB / 2.0).powi(2) / (2.0 * KNEE_DB)
        } else {
            slope * over
        }
    }

    /// Makeup gain in dB: half of the reduction a full scale signal gets.
    pub fn makeup(threshold_db: f32, ratio: f32) -> f32 {
        -Self::curve(0.0, threshold_db, ratio) * 0.5
    }

    /// Current gain reduction in dB (positive number).
    pub fn reduction(&self) -> f32 {
        -self.gr
    }

    /// `makeup_db` is normally `Compressor::makeup(threshold, ratio)`,
    /// computed once per block.
    #[inline]
    pub fn process(&mut self, x: f32, threshold_db: f32, ratio: f32, makeup_db: f32) -> f32 {
        self.ms += (x * x - self.ms) * self.ms_coef;
        let level_db = 10.0 * (self.ms + 1e-10).log10();
        let target = Self::curve(level_db, threshold_db, ratio);
        let speed = if target < self.gr {
            self.attack
        } else {
            self.release
        };
        self.gr += (target - self.gr) * speed;
        x * db_to_lin(self.gr + makeup_db)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::f32::consts::PI;

    fn rms(s: &[f32]) -> f32 {
        (s.iter().map(|x| x * x).sum::<f32>() / s.len() as f32).sqrt()
    }

    fn db(x: f32) -> f32 {
        20.0 * x.log10()
    }

    /// Deterministic white-ish noise in [-amp, amp].
    pub(crate) fn noise(len: usize, amp: f32) -> Vec<f32> {
        let mut state = 0x1234_5678u32;
        (0..len)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                (state as f32 / u32::MAX as f32 * 2.0 - 1.0) * amp
            })
            .collect()
    }

    #[test]
    fn gate_mutes_quiet_noise_and_passes_singing() {
        let sr = 48_000.0;
        let mut gate = NoiseGate::new(sr);
        let hiss = noise(24_000, 0.002);
        let out: Vec<f32> = hiss.iter().map(|&x| gate.process(x, -45.0)).collect();
        assert!(!gate.is_open());
        assert!(db(rms(&out[4800..])) < db(rms(&hiss)) - 40.0);

        let tone: Vec<f32> = (0..24_000)
            .map(|n| 0.3 * (2.0 * PI * 220.0 * n as f32 / sr).sin())
            .collect();
        let out: Vec<f32> = tone.iter().map(|&x| gate.process(x, -45.0)).collect();
        assert!(gate.is_open());
        assert!((rms(&out[2400..]) / rms(&tone[2400..]) - 1.0).abs() < 0.01);
    }

    #[test]
    fn gate_releases_after_phrase() {
        let sr = 44_100.0;
        let mut gate = NoiseGate::new(sr);
        for n in 0..4410 {
            gate.process(0.3 * (n as f32 * 0.05).sin(), -40.0);
        }
        assert!(gate.is_open());
        let tail: Vec<f32> = noise(22_050, 0.001)
            .iter()
            .map(|&x| gate.process(x, -40.0))
            .collect();
        assert!(!gate.is_open());
        assert!(tail[17_640..].iter().all(|s| s.abs() < 1e-4));
    }

    #[test]
    fn compressor_curve_and_makeup() {
        assert_eq!(Compressor::curve(-40.0, -20.0, 4.0), 0.0);
        assert!((Compressor::curve(0.0, -20.0, 4.0) + 15.0).abs() < 1e-4);
        assert!(Compressor::curve(-20.0, -20.0, 4.0) < 0.0); // inside the knee
        assert!((Compressor::makeup(-20.0, 4.0) - 7.5).abs() < 1e-4);
        assert_eq!(Compressor::makeup(-20.0, 1.0), 0.0);
    }

    #[test]
    fn compressor_evens_out_quiet_and_loud_singing() {
        let sr = 48_000.0;
        let (thr, ratio) = (-24.0, 4.0);
        let makeup = Compressor::makeup(thr, ratio);
        let level = |amp: f32| {
            let mut c = Compressor::new(sr);
            let out: Vec<f32> = (0..48_000)
                .map(|n| {
                    c.process(
                        amp * (2.0 * PI * 200.0 * n as f32 / sr).sin(),
                        thr,
                        ratio,
                        makeup,
                    )
                })
                .collect();
            assert!(out.iter().all(|s| s.is_finite()));
            db(rms(&out[24_000..]))
        };
        let (quiet, loud) = (level(0.03), level(0.5));
        // 24 dB apart at the input, much closer at the output.
        assert!(loud - quiet < 14.0, "quiet {quiet} dB, loud {loud} dB");
        // Makeup lifts the quiet take.
        assert!(quiet > db(0.03 / 2f32.sqrt()) + 3.0);
    }
}
