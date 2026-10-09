/// Monophonic pitch detector based on the YIN algorithm.
///
/// Input is decimated by 2 at common sample rates to keep the analysis cheap;
/// a new estimate is produced every ~10 ms.
pub struct PitchTracker {
    decim: usize,
    acc: f32,
    acc_n: usize,
    rate: f32,
    ring: Vec<f32>,
    pos: usize,
    filled: usize,
    since_hop: usize,
    hop: usize,
    window: usize,
    tau_min: usize,
    tau_max: usize,
    frame: Vec<f32>,
    diff: Vec<f32>,
    pitch: Option<f32>,
}

const MIN_HZ: f32 = 70.0;
const MAX_HZ: f32 = 1000.0;
const THRESHOLD: f32 = 0.15;
const MIN_RMS: f32 = 0.01;

impl PitchTracker {
    pub fn new(sample_rate: f32) -> Self {
        let decim = if sample_rate >= 32_000.0 { 2 } else { 1 };
        let rate = sample_rate / decim as f32;
        let tau_min = ((rate / MAX_HZ).floor() as usize).max(2);
        let tau_max = (rate / MIN_HZ).ceil() as usize;
        let window = tau_max;
        let len = window + tau_max + 1;
        Self {
            decim,
            acc: 0.0,
            acc_n: 0,
            rate,
            ring: vec![0.0; len],
            pos: 0,
            filled: 0,
            since_hop: 0,
            hop: ((rate * 0.01) as usize).max(1),
            window,
            tau_min,
            tau_max,
            frame: vec![0.0; len],
            diff: vec![0.0; tau_max + 2],
            pitch: None,
        }
    }

    /// Latest estimate in Hz, `None` when the input is silent or unvoiced.
    pub fn pitch(&self) -> Option<f32> {
        self.pitch
    }

    /// Feeds one sample; returns true when a new estimate is available.
    pub fn push(&mut self, x: f32) -> bool {
        self.acc += x;
        self.acc_n += 1;
        if self.acc_n < self.decim {
            return false;
        }
        let sample = self.acc / self.decim as f32;
        self.acc = 0.0;
        self.acc_n = 0;

        self.ring[self.pos] = sample;
        self.pos = (self.pos + 1) % self.ring.len();
        self.filled = (self.filled + 1).min(self.ring.len());
        self.since_hop += 1;
        if self.since_hop < self.hop || self.filled < self.ring.len() {
            return false;
        }
        self.since_hop = 0;
        self.pitch = self.analyze();
        true
    }

    fn analyze(&mut self) -> Option<f32> {
        let len = self.ring.len();
        for k in 0..len {
            self.frame[k] = self.ring[(self.pos + k) % len];
        }
        let frame = &self.frame;

        let energy: f32 = frame[..self.window].iter().map(|s| s * s).sum();
        if (energy / self.window as f32).sqrt() < MIN_RMS {
            return None;
        }

        // Difference function, then cumulative mean normalisation.
        let d = &mut self.diff;
        d[0] = 1.0;
        let mut running = 0.0;
        for tau in 1..=self.tau_max {
            let mut sum = 0.0;
            for j in 0..self.window {
                let delta = frame[j] - frame[j + tau];
                sum += delta * delta;
            }
            running += sum;
            d[tau] = if running > 0.0 {
                sum * tau as f32 / running
            } else {
                1.0
            };
        }

        let mut tau = self.tau_min;
        while tau < self.tau_max {
            if d[tau] < THRESHOLD {
                while tau + 1 < self.tau_max && d[tau + 1] < d[tau] {
                    tau += 1;
                }
                break;
            }
            tau += 1;
        }
        if tau >= self.tau_max {
            return None;
        }

        // Parabolic interpolation around the minimum.
        let (s0, s1, s2) = (d[tau - 1], d[tau], d[tau + 1]);
        let denom = s0 - 2.0 * s1 + s2;
        let shift = if denom.abs() > 1e-9 {
            0.5 * (s0 - s2) / denom
        } else {
            0.0
        };
        Some(self.rate / (tau as f32 + shift.clamp(-1.0, 1.0)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    fn detect(hz: f32, sr: f32) -> Option<f32> {
        let mut t = PitchTracker::new(sr);
        for n in 0..(sr as usize / 4) {
            let x = 0.5 * (2.0 * PI * hz * n as f32 / sr).sin()
                + 0.2 * (4.0 * PI * hz * n as f32 / sr).sin();
            t.push(x);
        }
        t.pitch()
    }

    #[test]
    fn detects_voice_range_pitches() {
        for &(hz, sr) in &[
            (110.0, 48_000.0),
            (220.0, 44_100.0),
            (523.25, 48_000.0),
            (90.0, 16_000.0),
        ] {
            let got = detect(hz, sr).expect("voiced");
            assert!((got - hz).abs() / hz < 0.01, "{hz} Hz at {sr}: got {got}");
        }
    }

    #[test]
    fn silence_is_unvoiced() {
        let mut t = PitchTracker::new(48_000.0);
        for _ in 0..24_000 {
            t.push(0.0);
        }
        assert_eq!(t.pitch(), None);
    }
}
