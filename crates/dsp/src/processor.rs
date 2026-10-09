use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::{
    hz_to_midi, snap_to_mask, Brightness, Compressor, Distortion, Doubler, FeedbackGuard, Key,
    KeyDetector, Limiter, Meters, NoiseGate, Params, PitchShifter, PitchTracker, Reverb,
};

/// Pitch estimates per second from the tracker (one every ~10 ms).
const ESTIMATES_PER_SECOND: f32 = 100.0;
/// Seconds of singing the auto key detection remembers.
const KEY_WINDOW: f32 = 12.0;

/// The full effects chain:
/// gate -> autotune / high pitch -> compressor -> brightness -> doubler ->
/// distortion -> reverb -> master -> feedback guard -> limiter.
pub struct Processor {
    params: Arc<Params>,
    meters: Arc<Meters>,
    sample_rate: f32,
    gate: NoiseGate,
    shifter: PitchShifter,
    tracker: PitchTracker,
    keys: KeyDetector,
    compressor: Compressor,
    brightness: Brightness,
    doubler: Doubler,
    distortion: Distortion,
    reverb: Reverb,
    guard: FeedbackGuard,
    limiter: Limiter,
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
            gate: NoiseGate::new(sample_rate),
            shifter: PitchShifter::new(sample_rate),
            tracker: PitchTracker::new(sample_rate),
            keys: KeyDetector::new(ESTIMATES_PER_SECOND, KEY_WINDOW),
            compressor: Compressor::new(sample_rate),
            brightness: Brightness::new(sample_rate),
            doubler: Doubler::new(sample_rate),
            distortion: Distortion::new(sample_rate),
            reverb: Reverb::new(sample_rate),
            guard: FeedbackGuard::new(sample_rate),
            limiter: Limiter::new(sample_rate),
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
        let base = if pitch_on {
            p.pitch_semitones.get()
        } else {
            0.0
        };
        let auto_key = p.auto_key.get();
        let manual_mask = p.tune_mask.load(Ordering::Relaxed);
        let amount = p.tune_amount.get().clamp(0.0, 1.0);
        let gate_on = p.gate_on.get();
        let gate_threshold = p.gate_threshold.get();
        let comp_on = p.comp_on.get();
        let comp_threshold = p.comp_threshold.get();
        let comp_ratio = p.comp_ratio.get().max(1.0);
        let makeup = Compressor::makeup(comp_threshold, comp_ratio);
        let bright_on = p.bright_on.get();
        let double_on = p.double_on.get();
        let double_mix = p.double_mix.get().clamp(0.0, 1.0);
        let double_detune = p.double_detune.get().clamp(0.0, 50.0);
        let double_norm = 1.0 / (1.0 + 0.5 * double_mix);
        let dist_on = p.dist_on.get();
        let dist_mix = p.dist_mix.get().clamp(0.0, 1.0);
        let verb_on = p.verb_on.get();
        let (room, damping) = (p.room.get(), p.damping.get());
        let verb_mix = p.verb_mix.get().clamp(0.0, 1.0);
        let master = p.master.get();
        let guard_on = p.feedback_guard.get();

        let use_shift = pitch_on || tune_on;
        if !use_shift && self.shifting {
            self.shifter.reset();
            self.shift = 0.0;
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
        if bright_on {
            self.brightness.set(p.brightness.get());
        }
        if !tune_on {
            self.target = base;
        }

        let mut in_peak = 0.0f32;
        let mut out_peak = 0.0f32;
        let mut reduction = 0.0f32;
        for x in buf.iter_mut() {
            let dry = *x;
            in_peak = in_peak.max(dry.abs());

            let mut y = dry;
            if gate_on {
                y = self.gate.process(y, gate_threshold);
            }

            // Always track: the UI shows the sung note, and the shifter
            // needs the input period to stay phase coherent.
            if self.tracker.push(y) {
                let hz = self.tracker.pitch();
                self.period = hz.map(|hz| self.sample_rate / hz);
                self.meters.pitch_hz.set(hz.unwrap_or(0.0));
                if let Some(hz) = hz {
                    if self.keys.push(hz_to_midi(hz)) {
                        let key = self.keys.key().map_or(Key::NONE, Key::encode);
                        self.meters.key.store(key, Ordering::Relaxed);
                    }
                }
                if tune_on {
                    let mask = if auto_key {
                        // Until a key is found, snap to the nearest semitone.
                        self.keys.key().map_or(0xFFF, Key::mask)
                    } else {
                        manual_mask
                    };
                    self.target = match hz {
                        Some(hz) => {
                            let wanted = hz_to_midi(hz) + base;
                            base + (snap_to_mask(wanted, mask) - wanted) * amount
                        }
                        None => base,
                    };
                }
            }

            let mut ratio = 1.0;
            if use_shift {
                self.shift += (self.target - self.shift) * alpha;
                ratio = (self.shift / 12.0).exp2();
                y = self.shifter.process(y, ratio, self.period);
            }
            if comp_on {
                y = self
                    .compressor
                    .process(y, comp_threshold, comp_ratio, makeup);
                reduction = reduction.max(self.compressor.reduction());
            }
            if bright_on {
                y = self.brightness.process(y);
            }
            if double_on {
                let period = self.period.map(|t| t / ratio);
                let wet = self.doubler.process(y, double_detune, period);
                y = (y + wet * double_mix) * double_norm;
            }
            if dist_on {
                y += (self.distortion.process(y) - y) * dist_mix;
            }
            if verb_on {
                let wet = self.reverb.process(y, room, damping);
                y = y * (1.0 - verb_mix) + wet * verb_mix;
            }
            y *= master;
            if guard_on {
                y = self.guard.process(y);
            }
            y = self.limiter.process(y).clamp(-1.0, 1.0);

            out_peak = out_peak.max(y.abs());
            *x = y;
        }

        self.meters.input.fetch_max(in_peak);
        self.meters.output.fetch_max(out_peak);
        self.meters.reduction.fetch_max(reduction);
        self.meters.feedback.set(guard_on && self.guard.active());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    fn sine(hz: f32, sr: f32, len: usize) -> Vec<f32> {
        (0..len)
            .map(|n| 0.4 * (2.0 * PI * hz * n as f32 / sr).sin())
            .collect()
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
    fn correction_amount_pulls_part_way() {
        let sr = 48_000.0;
        for (amount, expect) in [(0.0, 430.0), (0.5, 435.0), (1.0, 440.0)] {
            let params = Arc::new(Params::default());
            params.tune_on.set(true);
            params.tune_speed.set(0.0);
            params.tune_amount.set(amount);
            let mut proc = Processor::new(sr, params, Arc::new(Meters::default()));
            let mut buf = sine(430.0, sr, sr as usize);
            for chunk in buf.chunks_mut(512) {
                proc.process(chunk);
            }
            let hz = crate::pitch::tests::measure_hz(&buf[24_000..], sr);
            assert!((hz - expect).abs() < 2.0, "amount {amount}: got {hz} Hz");
        }
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
        params.gate_on.set(true);
        params.comp_on.set(true);
        params.bright_on.set(true);
        params.brightness.set(1.0);
        params.double_on.set(true);
        params.double_mix.set(1.0);
        let mut proc = Processor::new(sr, params, Arc::new(Meters::default()));
        let mut buf = sine(150.0, sr, 2 * sr as usize);
        for chunk in buf.chunks_mut(256) {
            proc.process(chunk);
        }
        assert!(buf.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
        assert!(buf[44_100..].iter().any(|s| s.abs() > 0.05));
    }

    /// A sung vowel: `hz` with decaying harmonics.
    fn vowel(hz: f32, sr: f32, len: usize, amp: f32) -> Vec<f32> {
        (0..len)
            .map(|n| {
                let t = n as f32 / sr;
                (1..=6)
                    .map(|k| amp / k as f32 * (2.0 * PI * hz * k as f32 * t).sin())
                    .sum()
            })
            .collect()
    }

    fn rms(s: &[f32]) -> f32 {
        (s.iter().map(|x| x * x).sum::<f32>() / s.len() as f32).sqrt()
    }

    #[test]
    fn pop_princess_chain_tunes_and_silences_breaths() {
        let sr = 48_000.0;
        let params = Arc::new(Params::default());
        crate::apply_sing_mode(&params, crate::Voice::PopPrincess, 1.0);
        params.tune_mask.store(0xFFF, Ordering::Relaxed);
        let mut proc = Processor::new(sr, Arc::clone(&params), Arc::new(Meters::default()));

        // Breath and room noise at about -60 dBFS, then a flat A3 (216 Hz).
        let mut buf = crate::dynamics::tests::noise(sr as usize, 0.0015);
        let take = vowel(216.0, sr, 2 * sr as usize, 0.06);
        buf.extend(&take);
        for chunk in buf.chunks_mut(480) {
            proc.process(chunk);
        }
        assert!(buf.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
        assert!(rms(&buf[4800..48_000]) < 1e-4, "noise got through");
        let hz = crate::pitch::tests::measure_hz(&buf[2 * 48_000..], sr);
        assert!((hz - 220.0).abs() < 3.0, "got {hz} Hz");
        // The compressor's makeup brings a quiet take up.
        let (dry, wet) = (rms(&take[48_000..]), rms(&buf[2 * 48_000..]));
        assert!(wet > 2.0 * dry, "{dry} -> {wet}");
    }

    #[test]
    fn auto_key_follows_the_singing() {
        let sr = 44_100.0;
        let params = Arc::new(Params::default());
        params.tune_on.set(true);
        params.auto_key.set(true);
        let meters = Arc::new(Meters::default());
        let mut proc = Processor::new(sr, Arc::clone(&params), Arc::clone(&meters));
        assert_eq!(meters.key.load(Ordering::Relaxed), Key::NONE);

        // D major: D F# A D A F# E C# D
        for midi in [
            62.0, 66.0, 69.0, 74.0, 69.0, 66.0, 64.0, 61.0, 62.0, 66.0, 69.0, 62.0,
        ] {
            let hz = 440.0 * ((midi - 69.0f32) / 12.0).exp2();
            let mut buf = vowel(hz, sr, (sr * 0.35) as usize, 0.2);
            for chunk in buf.chunks_mut(256) {
                proc.process(chunk);
            }
        }
        let key = Key::decode(meters.key.load(Ordering::Relaxed)).expect("key found");
        assert_eq!(
            key.mask(),
            Key {
                root: 2,
                minor: false
            }
            .mask(),
            "got {}",
            key.name()
        );
    }

    #[test]
    fn feedback_is_turned_down() {
        let sr = 48_000.0;
        let params = Arc::new(Params::default());
        let meters = Arc::new(Meters::default());
        let mut proc = Processor::new(sr, params, Arc::clone(&meters));
        let mut buf = sine(2500.0, sr, 2 * sr as usize)
            .iter()
            .map(|s| s * 2.0)
            .collect::<Vec<_>>();
        for chunk in buf.chunks_mut(512) {
            proc.process(chunk);
        }
        assert!(meters.feedback.get());
        assert!(buf[(1.5 * sr) as usize..].iter().all(|s| s.abs() < 0.15));
    }
}
