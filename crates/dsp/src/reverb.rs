/// Mono Freeverb (Schroeder/Moorer): 8 damped combs into 4 allpasses.
pub struct Reverb {
    combs: Vec<Comb>,
    allpasses: Vec<AllPass>,
}

const COMB_TUNING: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const ALLPASS_TUNING: [usize; 4] = [556, 441, 341, 225];
const INPUT_GAIN: f32 = 0.015;

struct Comb {
    buf: Vec<f32>,
    idx: usize,
    store: f32,
}

impl Comb {
    #[inline]
    fn process(&mut self, x: f32, feedback: f32, damp: f32) -> f32 {
        let out = self.buf[self.idx];
        self.store = out * (1.0 - damp) + self.store * damp;
        self.buf[self.idx] = x + self.store * feedback;
        self.idx = (self.idx + 1) % self.buf.len();
        out
    }
}

struct AllPass {
    buf: Vec<f32>,
    idx: usize,
}

impl AllPass {
    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let b = self.buf[self.idx];
        self.buf[self.idx] = x + b * 0.5;
        self.idx = (self.idx + 1) % self.buf.len();
        b - x
    }
}

impl Reverb {
    pub fn new(sample_rate: f32) -> Self {
        let scale = sample_rate / 44_100.0;
        let len = |n: usize| ((n as f32 * scale) as usize).max(1);
        Self {
            combs: COMB_TUNING
                .iter()
                .map(|&n| Comb {
                    buf: vec![0.0; len(n)],
                    idx: 0,
                    store: 0.0,
                })
                .collect(),
            allpasses: ALLPASS_TUNING
                .iter()
                .map(|&n| AllPass {
                    buf: vec![0.0; len(n)],
                    idx: 0,
                })
                .collect(),
        }
    }

    /// Returns the wet signal only. `room` and `damping` are 0..1.
    #[inline]
    pub fn process(&mut self, x: f32, room: f32, damping: f32) -> f32 {
        let feedback = room.clamp(0.0, 1.0) * 0.28 + 0.7;
        let damp = damping.clamp(0.0, 1.0) * 0.4;
        let input = x * INPUT_GAIN;
        let mut out = 0.0;
        for c in &mut self.combs {
            out += c.process(input, feedback, damp);
        }
        for a in &mut self.allpasses {
            out = a.process(out);
        }
        out * 3.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn impulse_leaves_a_decaying_tail() {
        let mut r = Reverb::new(48_000.0);
        let out: Vec<f32> = (0..96_000)
            .map(|n| r.process(if n == 0 { 1.0 } else { 0.0 }, 0.8, 0.5))
            .collect();
        let early: f32 = out[2_000..12_000].iter().map(|s| s * s).sum();
        let late: f32 = out[86_000..].iter().map(|s| s * s).sum();
        assert!(early > 0.0 && late < early);
        assert!(out.iter().all(|s| s.is_finite()));
    }
}
