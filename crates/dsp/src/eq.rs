//! "Brightness" EQ for a glossy pop vocal: low cut, presence boost and air.

use std::f32::consts::PI;

/// Second order IIR section (RBJ cookbook), transposed direct form II.
#[derive(Clone, Copy)]
pub struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Biquad {
    fn from_raw(b: [f32; 3], a: [f32; 3]) -> Self {
        let mut f = Self::identity();
        f.set_raw(b, a);
        f
    }

    pub fn identity() -> Self {
        Self { b0: 1.0, b1: 0.0, b2: 0.0, a1: 0.0, a2: 0.0, z1: 0.0, z2: 0.0 }
    }

    /// Replaces the coefficients but keeps the filter state, so changes
    /// while running do not click.
    fn set_raw(&mut self, b: [f32; 3], a: [f32; 3]) {
        self.b0 = b[0] / a[0];
        self.b1 = b[1] / a[0];
        self.b2 = b[2] / a[0];
        self.a1 = a[1] / a[0];
        self.a2 = a[2] / a[0];
    }

    fn copy_coefs(&mut self, other: &Biquad) {
        self.b0 = other.b0;
        self.b1 = other.b1;
        self.b2 = other.b2;
        self.a1 = other.a1;
        self.a2 = other.a2;
    }

    fn omega(freq: f32, sample_rate: f32) -> (f32, f32) {
        let w = 2.0 * PI * freq.min(sample_rate * 0.45) / sample_rate;
        (w.cos(), w.sin())
    }

    pub fn highpass(freq: f32, q: f32, sample_rate: f32) -> Self {
        let (c, s) = Self::omega(freq, sample_rate);
        let alpha = s / (2.0 * q);
        Self::from_raw(
            [(1.0 + c) / 2.0, -(1.0 + c), (1.0 + c) / 2.0],
            [1.0 + alpha, -2.0 * c, 1.0 - alpha],
        )
    }

    pub fn peaking(freq: f32, q: f32, gain_db: f32, sample_rate: f32) -> Self {
        let (c, s) = Self::omega(freq, sample_rate);
        let a = 10f32.powf(gain_db / 40.0);
        let alpha = s / (2.0 * q);
        Self::from_raw(
            [1.0 + alpha * a, -2.0 * c, 1.0 - alpha * a],
            [1.0 + alpha / a, -2.0 * c, 1.0 - alpha / a],
        )
    }

    /// High shelf with slope 1.
    pub fn high_shelf(freq: f32, gain_db: f32, sample_rate: f32) -> Self {
        let (c, s) = Self::omega(freq, sample_rate);
        let a = 10f32.powf(gain_db / 40.0);
        let alpha = s / 2.0 * 2f32.sqrt();
        let sq = 2.0 * a.sqrt() * alpha;
        Self::from_raw(
            [
                a * ((a + 1.0) + (a - 1.0) * c + sq),
                -2.0 * a * ((a - 1.0) + (a + 1.0) * c),
                a * ((a + 1.0) + (a - 1.0) * c - sq),
            ],
            [
                (a + 1.0) - (a - 1.0) * c + sq,
                2.0 * ((a - 1.0) - (a + 1.0) * c),
                (a + 1.0) - (a - 1.0) * c - sq,
            ],
        )
    }

    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

/// Low cut at 90 Hz, presence peak at 3.2 kHz and an "air" shelf at 10 kHz.
/// `amount` 0..1 scales the boosts (up to +6 dB presence, +9 dB air).
pub struct Brightness {
    sample_rate: f32,
    amount: f32,
    low_cut: Biquad,
    presence: Biquad,
    air: Biquad,
}

impl Brightness {
    pub fn new(sample_rate: f32) -> Self {
        let mut b = Self {
            sample_rate,
            amount: -1.0,
            low_cut: Biquad::highpass(90.0, 0.707, sample_rate),
            presence: Biquad::identity(),
            air: Biquad::identity(),
        };
        b.set(0.5);
        b
    }

    /// Call once per block; coefficients are only recomputed on change.
    pub fn set(&mut self, amount: f32) {
        let amount = amount.clamp(0.0, 1.0);
        if (amount - self.amount).abs() < 1e-4 {
            return;
        }
        self.amount = amount;
        let sr = self.sample_rate;
        self.presence.copy_coefs(&Biquad::peaking(3200.0, 0.9, 6.0 * amount, sr));
        self.air.copy_coefs(&Biquad::high_shelf(10_000.0, 9.0 * amount, sr));
    }

    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        self.air.process(self.presence.process(self.low_cut.process(x)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Steady state gain in dB of `f` at `hz`.
    fn gain_db(mut f: impl FnMut(f32) -> f32, hz: f32, sr: f32) -> f32 {
        let len = sr as usize;
        let (mut peak_in, mut peak_out) = (0.0f32, 0.0f32);
        for n in 0..len {
            let x = (2.0 * PI * hz * n as f32 / sr).sin();
            let y = f(x);
            if n > len / 2 {
                peak_in = peak_in.max(x.abs());
                peak_out = peak_out.max(y.abs());
            }
        }
        20.0 * (peak_out / peak_in).log10()
    }

    #[test]
    fn brightness_shapes_the_spectrum() {
        let sr = 48_000.0;
        let at = |hz: f32, amount: f32| {
            let mut eq = Brightness::new(sr);
            eq.set(amount);
            gain_db(|x| eq.process(x), hz, sr)
        };
        assert!(at(40.0, 1.0) < -9.0, "low cut");
        assert!(at(1000.0, 1.0).abs() < 1.5, "mids untouched");
        assert!(at(3200.0, 1.0) > 5.0, "presence");
        assert!(at(14_000.0, 1.0) > 7.0, "air");
        assert!(at(14_000.0, 0.0).abs() < 0.5, "flat highs at zero");
    }

    #[test]
    fn works_at_low_sample_rates() {
        let sr = 16_000.0;
        let mut eq = Brightness::new(sr);
        eq.set(1.0);
        let g = gain_db(|x| eq.process(x), 5000.0, sr);
        assert!(g.is_finite() && g > 0.0 && g < 20.0, "{g}");
    }
}
