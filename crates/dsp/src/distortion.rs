use std::f32::consts::PI;

/// Saturating overdrive followed by a one-pole tone filter.
pub struct Distortion {
    sample_rate: f32,
    gain: f32,
    lp_coef: f32,
    lp: f32,
}

impl Distortion {
    pub fn new(sample_rate: f32) -> Self {
        let mut d = Self {
            sample_rate,
            gain: 1.0,
            lp_coef: 1.0,
            lp: 0.0,
        };
        d.set(0.5, 0.5);
        d
    }

    /// `drive` and `tone` are 0..1. Call once per block, not per sample.
    pub fn set(&mut self, drive: f32, tone: f32) {
        let drive = drive.clamp(0.0, 1.0);
        self.gain = 1.0 + drive * drive * 60.0;
        let cutoff = (800.0 * 15f32.powf(tone.clamp(0.0, 1.0))).min(self.sample_rate * 0.45);
        self.lp_coef = 1.0 - (-2.0 * PI * cutoff / self.sample_rate).exp();
    }

    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        let wet = (x * self.gain).tanh() * 0.6;
        self.lp += self.lp_coef * (wet - self.lp);
        self.lp
    }
}
