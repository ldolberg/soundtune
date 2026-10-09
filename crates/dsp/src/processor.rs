use std::sync::Arc;

use crate::{
    hz_to_midi, snap_to_mask, Distortion, Meters, Params, PitchShifter, PitchTracker, Reverb,
};

/// The full effects chain: autotune / high pitch -> distortion -> reverb -> master.
pub struct Processor {
    params: Arc<Params>,
    meters: Arc<Meters>,
    sample_rate: f32,
    shifter: PitchShifter,
    tracker: PitchTracker,
    distortion: Distortion,
    reverb: Reverb,
    /// Pitch shift currently applied, in semitones (smoothed).
    shift: f32,
    /// Pitch shift we are gliding towards.
    target: f32,
    shifting: bool,
    /// Input pitch period in samples, when voiced.
    period: Option<f32>,
}

impl Processor {
    pub fn new(sample_rate: f32, params: Arc<Params>, meters: Arc<Meters>) -> Self {
        Self {
            params,
            meters,
            sample_rate,
            shifter: PitchShifter::new(sample_rate),
            tracker: PitchTracker::new(sample_rate),
            distortion: Distortion::new(sample_rate),
            reverb: Reverb::new(sample_rate),
            shift: 0.0,
            target: 0.0,
            shifting: false,
            period: None,
        }
    }

    /// Smoothing factor for a one-pole glide with the given time constant.
    fn glide(&self, seconds: f32) -> f32 {
        if seconds <= 1e-4 {
            1.0
        } else {
            1.0 - (-1.0 / (seconds * self.sample_rate)).exp()
        }
    }

    /// Processes a block of mono samples in place.
    pub fn process(&mut self, buf: &mut [f32]) {
        let p = Arc::clone(&self.params);
        let pitch_on = p.pitch_on.get();
        let tune_on = p.tune_on.get();
        let base = if pitch_on { p.pitch_semitones.get() } else { 0.0 };
        let mask = p.tune_mask.load(std::sync::atomic::Ordering::Relaxed);
        let dist_on = p.dist_on.get();
        let dist_mix = p.dist_mix.get().clamp(0.0, 1.0);
        let verb_on = p.verb_on.get();
        let (room, damping) = (p.room.get(), p.damping.get());
        let verb_mix = p.verb_mix.get().clamp(0.0, 1.0);
        let master = p.master.get();

        let use_shift = pitch_on || tune_on;
        if !use_shift && self.shifting {
            self.shifter.reset();
            self.shift = 0.0;
            self.period = None;
        }
        self.shifting = use_shift;

        // Retune speed 0..1 maps to a 0..250 ms glide. Without autotune we
        // still glide a little so slider moves do not zipper.
        let alpha = if tune_on {
            self.glide(p.tune_speed.get().clamp(0.0, 1.0) * 0.25)
        } else {
            self.glide(0.02)
        };
        if dist_on {
            self.distortion.set(p.drive.get(), p.tone.get());
        }
        if !tune_on {
            self.target = base;
        }

        let mut in_peak = 0.0f32;
        let mut out_peak = 0.0f32;
        for x in buf.iter_mut() {
            let dry = *x;
            in_peak = in_peak.max(dry.abs());

            // Always track: the UI shows the sung note, and the shifter
            // needs the input period to stay phase coherent.
            if self.tracker.push(dry) {
                let hz = self.tracker.pitch();
                self.period = hz.map(|hz| self.sample_rate / hz);
                self.meters.pitch_hz.set(hz.unwrap_or(0.0));
                if tune_on {
                    self.target = match hz {
                        Some(hz) => {
                            let sung = hz_to_midi(hz);
                            snap_to_mask(sung + base, mask) - sung
                        }
                        None => base,
                    };
                }
            }

            let mut y = dry;
            if use_shift {
                self.shift += (self.target - self.shift) * alpha;
                y = self.shifter.process(y, (self.shift / 12.0).exp2(), self.period);
            }
            if dist_on {
                y += (self.distortion.process(y) - y) * dist_mix;
            }
            if verb_on {
                let wet = self.reverb.process(y, room, damping);
                y = y * (1.0 - verb_mix) + wet * verb_mix;
            }
            y = (y * master).clamp(-1.0, 1.0);

            out_peak = out_peak.max(y.abs());
            *x = y;
        }

        self.meters.input.fetch_max(in_peak);
        self.meters.output.fetch_max(out_peak);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    fn sine(hz: f32, sr: f32, len: usize) -> Vec<f32> {
        (0..len).map(|n| 0.4 * (2.0 * PI * hz * n as f32 / sr).sin()).collect()
    }

    #[test]
    fn bypass_is_transparent() {
        let params = Arc::new(Params::default());
        let mut proc = Processor::new(48_000.0, params, Arc::new(Meters::default()));
        let input = sine(220.0, 48_000.0, 4800);
        let mut buf = input.clone();
        proc.process(&mut buf);
        assert_eq!(buf, input);
    }

    #[test]
    fn autotune_pulls_a_flat_note_to_pitch() {
        let sr = 48_000.0;
        let params = Arc::new(Params::default());
        params.tune_on.set(true);
        params.tune_speed.set(0.0);
        let meters = Arc::new(Meters::default());
        let mut proc = Processor::new(sr, params, Arc::clone(&meters));

        // 430 Hz is ~40 cents flat of A4 (440 Hz).
        let mut buf = sine(430.0, sr, sr as usize);
        for chunk in buf.chunks_mut(512) {
            proc.process(chunk);
        }
        let hz = crate::pitch::tests::measure_hz(&buf[24_000..], sr);
        assert!((hz - 440.0).abs() < 3.0, "got {hz} Hz");
        assert!((meters.pitch_hz.get() - 430.0).abs() < 5.0);
    }

    #[test]
    fn all_effects_stay_bounded() {
        let sr = 44_100.0;
        let params = Arc::new(Params::default());
        params.pitch_on.set(true);
        params.tune_on.set(true);
        params.dist_on.set(true);
        params.verb_on.set(true);
        params.room.set(1.0);
        params.drive.set(1.0);
        let mut proc = Processor::new(sr, params, Arc::new(Meters::default()));
        let mut buf = sine(150.0, sr, 2 * sr as usize);
        for chunk in buf.chunks_mut(256) {
            proc.process(chunk);
        }
        assert!(buf.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
        assert!(buf[44_100..].iter().any(|s| s.abs() > 0.05));
    }
}
