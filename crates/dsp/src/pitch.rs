use std::f32::consts::PI;

/// Low latency pitch shifter built from two crossfaded, sweeping delay taps.
///
/// Each tap reads the input at a rate of `ratio`; when a tap's delay wraps
/// around it is faded out, so the jump is inaudible. If the input period is
/// known, a restarting tap is aligned to a whole number of periods of the
/// other tap, so the crossfade blends two in-phase signals (no beating and no
/// pitch drift).
pub struct PitchShifter {
    buf: Vec<f32>,
    mask: usize,
    write: usize,
    window: f32,
    phase: f32,
    /// Extra delay so period alignment never needs a negative delay.
    base: f32,
    offsets: [f32; 2],
}

impl PitchShifter {
    pub fn new(sample_rate: f32) -> Self {
        // 30 ms grains: short enough for low latency, long enough for voice.
        let window = (sample_rate * 0.03).max(64.0);
        // Half of the longest period the tracker reports (70 Hz).
        let base = sample_rate / 70.0 * 0.5 + 1.0;
        let size = (window as usize + 2 * base as usize + 4).next_power_of_two();
        Self {
            buf: vec![0.0; size],
            mask: size - 1,
            write: 0,
            window,
            phase: 0.0,
            base,
            offsets: [0.0; 2],
        }
    }

    pub fn reset(&mut self) {
        self.buf.iter_mut().for_each(|s| *s = 0.0);
        self.phase = 0.0;
        self.offsets = [0.0; 2];
    }

    /// Processes one sample. `ratio` is the frequency ratio (2.0 = one octave
    /// up); `period` is the input's pitch period in samples, if voiced.
    #[inline]
    pub fn process(&mut self, x: f32, ratio: f32, period: Option<f32>) -> f32 {
        self.buf[self.write] = x;

        let prev = self.phase;
        self.phase += (1.0 - ratio) / self.window;
        self.phase -= self.phase.floor();
        if self.phase >= 1.0 {
            self.phase = 0.0;
        }
        let other = (self.phase + 0.5).fract();
        let prev_other = (prev + 0.5).fract();

        let raw = [self.phase * self.window, other * self.window];
        let period = period.filter(|&t| t > 1.0 && t <= 2.0 * (self.base - 1.0));
        if (self.phase - prev).abs() > 0.5 {
            self.offsets[0] = align(raw[0], raw[1] + self.offsets[1], period);
        }
        if (other - prev_other).abs() > 0.5 {
            self.offsets[1] = align(raw[1], raw[0] + self.offsets[0], period);
        }

        // sin^2 + cos^2 = 1, and each tap is silent where its delay wraps.
        let g1 = (PI * self.phase).sin().powi(2);
        let y = self.tap(self.base + raw[0] + self.offsets[0]) * g1
            + self.tap(self.base + raw[1] + self.offsets[1]) * (1.0 - g1);

        self.write = (self.write + 1) & self.mask;
        y
    }

    #[inline]
    fn tap(&self, delay: f32) -> f32 {
        let pos = self.write as f32 - delay + self.buf.len() as f32;
        let i = pos.floor();
        let frac = pos - i;
        let i = i as usize;
        let a = self.buf[i & self.mask];
        let b = self.buf[(i + 1) & self.mask];
        a + (b - a) * frac
    }
}

/// Offset in [-period/2, period/2] that makes `raw + offset` congruent to
/// `target` modulo `period`.
fn align(raw: f32, target: f32, period: Option<f32>) -> f32 {
    match period {
        Some(t) => {
            let diff = target - raw;
            diff - t * (diff / t).round()
        }
        None => 0.0,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Median of the YIN estimates over `signal`.
    pub(crate) fn measure_hz(signal: &[f32], sample_rate: f32) -> f32 {
        let mut tracker = crate::PitchTracker::new(sample_rate);
        let mut found: Vec<f32> = signal
            .iter()
            .filter_map(|&x| if tracker.push(x) { tracker.pitch() } else { None })
            .collect();
        assert!(!found.is_empty(), "no pitch found");
        found.sort_by(|a, b| a.partial_cmp(b).unwrap());
        found[found.len() / 2]
    }

    #[test]
    fn shifts_an_octave_up() {
        let sr = 48_000.0;
        let mut ps = PitchShifter::new(sr);
        let out: Vec<f32> = (0..sr as usize)
            .map(|n| ps.process((2.0 * PI * 220.0 * n as f32 / sr).sin(), 2.0, Some(sr / 220.0)))
            .collect();
        let hz = measure_hz(&out[4800..], sr);
        assert!((hz - 440.0).abs() < 3.0, "got {hz} Hz");
    }

    #[test]
    fn unity_ratio_keeps_pitch() {
        let sr = 44_100.0;
        let mut ps = PitchShifter::new(sr);
        let out: Vec<f32> = (0..sr as usize)
            .map(|n| ps.process((2.0 * PI * 300.0 * n as f32 / sr).sin(), 1.0, None))
            .collect();
        let hz = measure_hz(&out[4410..], sr);
        assert!((hz - 300.0).abs() < 2.0, "got {hz} Hz");
    }
}
