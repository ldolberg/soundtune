//! "Sing mode": one voice choice plus one Polish amount drive the whole chain.

use crate::Params;

/// Voice characters offered in Sing mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Voice {
    /// Gentle pitch correction and a clean, slightly brighter sound.
    Natural,
    /// Early 2000s pop: tight glossy tuning, breathy brightness, doubled
    /// vocals and a lush reverb.
    PopPrincess,
    /// Instant, hard tuning with a metallic double.
    Robot,
    /// Pitched up and tuned.
    Chipmunk,
}

impl Voice {
    pub const ALL: [Voice; 4] = [
        Voice::Natural,
        Voice::PopPrincess,
        Voice::Robot,
        Voice::Chipmunk,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Voice::Natural => "Natural",
            Voice::PopPrincess => "Pop Princess",
            Voice::Robot => "Robot",
            Voice::Chipmunk => "Chipmunk",
        }
    }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Sets every effect for `voice` at `polish` (0 = subtle, 1 = full
/// studio gloss). The scale (`tune_mask`, `auto_key`) and the master volume
/// are left alone.
pub fn apply_sing_mode(p: &Params, voice: Voice, polish: f32) {
    let t = polish.clamp(0.0, 1.0);

    // Shared clean-up chain: the gate keeps breaths and room noise out of
    // the autotune and reverb, the compressor evens out the level.
    p.gate_on.set(true);
    p.gate_threshold.set(lerp(-56.0, -44.0, t));
    p.comp_on.set(true);
    p.dist_on.set(false);
    p.pitch_on.set(false);
    p.tune_on.set(true);
    p.bright_on.set(true);
    p.verb_on.set(true);
    p.damping.set(0.4);

    match voice {
        Voice::Natural => {
            p.tune_speed.set(lerp(0.6, 0.25, t));
            p.comp_threshold.set(lerp(-14.0, -22.0, t));
            p.comp_ratio.set(lerp(1.8, 3.0, t));
            p.brightness.set(lerp(0.1, 0.35, t));
            p.double_on.set(false);
            p.room.set(0.55);
            p.verb_mix.set(lerp(0.08, 0.18, t));
        }
        Voice::PopPrincess => {
            p.tune_speed.set(lerp(0.3, 0.0, t));
            p.comp_threshold.set(lerp(-16.0, -28.0, t));
            p.comp_ratio.set(lerp(2.5, 6.0, t));
            p.brightness.set(lerp(0.4, 0.95, t));
            p.double_on.set(true);
            p.double_mix.set(lerp(0.25, 0.7, t));
            p.double_detune.set(lerp(8.0, 14.0, t));
            p.room.set(lerp(0.7, 0.85, t));
            p.damping.set(0.3);
            p.verb_mix.set(lerp(0.16, 0.32, t));
        }
        Voice::Robot => {
            p.tune_speed.set(0.0);
            p.comp_threshold.set(lerp(-18.0, -26.0, t));
            p.comp_ratio.set(4.0);
            p.brightness.set(lerp(0.3, 0.6, t));
            p.double_on.set(true);
            p.double_mix.set(lerp(0.2, 0.5, t));
            p.double_detune.set(25.0);
            p.room.set(0.5);
            p.verb_mix.set(lerp(0.08, 0.15, t));
        }
        Voice::Chipmunk => {
            p.pitch_on.set(true);
            p.pitch_semitones.set(10.0);
            p.tune_speed.set(lerp(0.4, 0.05, t));
            p.comp_threshold.set(lerp(-16.0, -24.0, t));
            p.comp_ratio.set(3.0);
            p.brightness.set(lerp(0.2, 0.5, t));
            p.double_on.set(t > 0.5);
            p.double_mix.set(lerp(0.0, 0.4, (t - 0.5).max(0.0) * 2.0));
            p.double_detune.set(12.0);
            p.room.set(0.45);
            p.verb_mix.set(lerp(0.08, 0.18, t));
        }
    }
}

/// Turns every effect off (volume, scale and safety settings are kept).
pub fn bypass_all(p: &Params) {
    for t in [
        &p.pitch_on,
        &p.tune_on,
        &p.dist_on,
        &p.verb_on,
        &p.gate_on,
        &p.comp_on,
        &p.bright_on,
        &p.double_on,
    ] {
        t.set(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polish_turns_up_the_gloss() {
        let p = Params::default();
        apply_sing_mode(&p, Voice::PopPrincess, 0.2);
        let soft = (
            p.tune_speed.get(),
            p.comp_ratio.get(),
            p.brightness.get(),
            p.double_mix.get(),
            p.verb_mix.get(),
        );
        apply_sing_mode(&p, Voice::PopPrincess, 1.0);
        assert!(p.tune_on.get() && p.gate_on.get() && p.comp_on.get());
        assert!(p.bright_on.get() && p.double_on.get() && p.verb_on.get());
        assert!(!p.dist_on.get() && !p.pitch_on.get());
        assert!(p.tune_speed.get() < soft.0 && p.tune_speed.get() < 0.01);
        assert!(p.comp_ratio.get() > soft.1);
        assert!(p.brightness.get() > soft.2);
        assert!(p.double_mix.get() > soft.3);
        assert!(p.verb_mix.get() > soft.4);
        let detune = p.double_detune.get();
        assert!((8.0..=15.0).contains(&detune));
    }

    #[test]
    fn every_voice_is_in_range_and_bypass_clears() {
        let p = Params::default();
        for v in Voice::ALL {
            for t in [0.0, 0.5, 1.0] {
                apply_sing_mode(&p, v, t);
                assert!(p.tune_on.get(), "{}", v.name());
                assert!((0.0..=1.0).contains(&p.tune_speed.get()));
                assert!(p.comp_ratio.get() >= 1.0);
                assert!((0.0..=1.0).contains(&p.verb_mix.get()));
            }
        }
        apply_sing_mode(&p, Voice::Chipmunk, 1.0);
        assert!(p.pitch_on.get() && p.pitch_semitones.get() > 0.0);
        bypass_all(&p);
        assert!(!p.tune_on.get() && !p.pitch_on.get() && !p.double_on.get() && !p.gate_on.get());
        assert!(p.feedback_guard.get());
    }
}
