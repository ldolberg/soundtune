use crate::PitchShifter;

/// Delays (seconds) of the two doubled voices.
const DELAYS: [f32; 2] = [0.018, 0.029];

/// Vocal doubler: two copies of the voice, one detuned up and one down by a
/// few cents and slightly delayed, like a second and third take sung on top.
pub struct Doubler {
    buf: Vec<f32>,
    mask: usize,
    write: usize,
    delays: [usize; 2],
    shifters: [PitchShifter; 2],
}

impl Doubler {
    pub fn new(sample_rate: f32) -> Self {
        let delays = DELAYS.map(|d| (d * sample_rate) as usize);
        let size = (delays[1] + 1).next_power_of_two();
        Self {
            buf: vec![0.0; size],
            mask: size - 1,
            write: 0,
            delays,
            shifters: [PitchShifter::new(sample_rate), PitchShifter::new(sample_rate)],
        }
    }

    /// Returns the doubled voices only (no dry signal). `cents` is the
    /// detune of each copy; `period` is the input pitch period, if voiced.
    #[inline]
    pub fn process(&mut self, x: f32, cents: f32, period: Option<f32>) -> f32 {
        self.buf[self.write] = x;
        let ratio = (cents / 1200.0).exp2();
        let mut out = 0.0;
        for (v, (shifter, &delay)) in self.shifters.iter_mut().zip(&self.delays).enumerate() {
            let tap = self.buf[(self.write + self.buf.len() - delay) & self.mask];
            let r = if v == 0 { ratio } else { 1.0 / ratio };
            out += shifter.process(tap, r, period);
        }
        self.write = (self.write + 1) & self.mask;
        out * 0.5
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    #[test]
    fn copies_are_detuned_both_ways() {
        let sr = 48_000.0;
        let input: Vec<f32> = (0..sr as usize).map(|n| 0.5 * (2.0 * PI * 220.0 * n as f32 / sr).sin()).collect();
        // Each voice on its own: run one doubler and read the shifters'
        // pitch through a large detune so the tracker can resolve it.
        for (cents, expect) in [(100.0, 220.0 * 2f32.powf(1.0 / 12.0)), (-100.0, 220.0 / 2f32.powf(1.0 / 12.0))] {
            let mut d = Doubler::new(sr);
            let out: Vec<f32> = input
                .iter()
                .map(|&x| {
                    d.buf[d.write] = x;
                    let tap = d.buf[(d.write + d.buf.len() - d.delays[0]) & d.mask];
                    d.write = (d.write + 1) & d.mask;
                    d.shifters[0].process(tap, (cents / 1200.0f32).exp2(), Some(sr / 220.0))
                })
                .collect();
            let hz = crate::pitch::tests::measure_hz(&out[9600..], sr);
            assert!((hz - expect).abs() < 2.0, "{cents} cents: got {hz} Hz, want {expect}");
        }
    }

    #[test]
    fn doubled_signal_is_delayed_and_bounded() {
        let sr = 44_100.0;
        let mut d = Doubler::new(sr);
        let mut first = None;
        for n in 0..(sr as usize) {
            let x = if n < 4410 { (2.0 * PI * 300.0 * n as f32 / sr).sin() } else { 0.0 };
            let y = d.process(x, 12.0, Some(sr / 300.0));
            assert!(y.is_finite() && y.abs() <= 1.0);
            if first.is_none() && y.abs() > 1e-3 {
                first = Some(n);
            }
        }
        // Nothing comes out before the shortest copy's delay.
        assert!(first.unwrap() as f32 >= DELAYS[0] * sr);
    }
}
