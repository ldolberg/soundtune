//! Automatic key detection from the sung pitch.

use crate::{MusicScale, NOTE_NAMES};

/// A musical key: root pitch class (0 = C) and mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Key {
    pub root: u32,
    pub minor: bool,
}

impl Key {
    /// Value stored in `Meters::key` when no key is known.
    pub const NONE: u32 = u32::MAX;

    pub fn scale(self) -> MusicScale {
        if self.minor {
            MusicScale::Minor
        } else {
            MusicScale::Major
        }
    }

    /// Notes of the key's scale, one bit per pitch class.
    pub fn mask(self) -> u32 {
        self.scale().mask(self.root)
    }

    /// Like "A minor".
    pub fn name(self) -> String {
        format!(
            "{} {}",
            NOTE_NAMES[self.root as usize % 12],
            if self.minor { "minor" } else { "major" }
        )
    }

    pub fn encode(self) -> u32 {
        self.root % 12 + if self.minor { 12 } else { 0 }
    }

    pub fn decode(v: u32) -> Option<Key> {
        (v < 24).then_some(Key {
            root: v % 12,
            minor: v >= 12,
        })
    }
}

// Krumhansl-Kessler key profiles.
const MAJOR: [f32; 12] = [
    6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88,
];
const MINOR: [f32; 12] = [
    6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17,
];

/// Pitch estimates between two key estimates.
const EVALUATE_EVERY: u32 = 25;
/// How much better (in correlation) a new key must fit before we switch.
const SWITCH_MARGIN: f32 = 0.04;

/// Keeps a pitch class histogram of the last few seconds of singing and
/// finds the major or minor key whose Krumhansl profile fits it best.
pub struct KeyDetector {
    hist: [f32; 12],
    /// Per estimate decay, so old notes fade out over the window.
    decay: f32,
    /// Histogram weight needed before guessing (seconds of singing).
    min_weight: f32,
    since: u32,
    key: Option<Key>,
    profiles: [[f32; 12]; 2],
}

/// Removes the mean and scales to unit length, so a dot product of two
/// normalised vectors is their Pearson correlation.
fn normalise(v: &[f32; 12]) -> [f32; 12] {
    let mean = v.iter().sum::<f32>() / 12.0;
    let mut out = v.map(|x| x - mean);
    let norm = out.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        out.iter_mut().for_each(|x| *x /= norm);
    }
    out
}

/// Correlation of a normalised pitch class histogram with `key`'s profile.
fn fit(profiles: &[[f32; 12]; 2], hist: &[f32; 12], key: Key) -> f32 {
    let profile = &profiles[key.minor as usize];
    (0..12)
        .map(|pc| hist[pc] * profile[(pc + 12 - key.root as usize) % 12])
        .sum()
}

fn best_key_normalised(profiles: &[[f32; 12]; 2], hist: &[f32; 12]) -> (Key, f32) {
    let mut best = Key {
        root: 0,
        minor: false,
    };
    let mut best_fit = f32::MIN;
    for v in 0..24 {
        let key = Key::decode(v).unwrap_or(best);
        let f = fit(profiles, hist, key);
        if f > best_fit {
            best_fit = f;
            best = key;
        }
    }
    (best, best_fit)
}

/// The major or minor key whose Krumhansl-Kessler profile correlates best
/// with a pitch class histogram (bin 0 = C), with that correlation (-1..1).
/// `None` for an empty or flat histogram.
pub fn best_key(hist: &[f32; 12]) -> Option<(Key, f32)> {
    let norm = normalise(hist);
    if norm.iter().all(|&x| x == 0.0) {
        return None;
    }
    let profiles = [normalise(&MAJOR), normalise(&MINOR)];
    Some(best_key_normalised(&profiles, &norm))
}

impl KeyDetector {
    /// `rate` is how many pitch estimates arrive per second while singing,
    /// `window` the memory in seconds.
    pub fn new(rate: f32, window: f32) -> Self {
        Self {
            hist: [0.0; 12],
            decay: (-1.0 / (rate * window)).exp(),
            min_weight: rate * 1.5,
            since: 0,
            key: None,
            profiles: [normalise(&MAJOR), normalise(&MINOR)],
        }
    }

    pub fn key(&self) -> Option<Key> {
        self.key
    }

    pub fn reset(&mut self) {
        self.hist = [0.0; 12];
        self.key = None;
        self.since = 0;
    }

    /// Feeds one voiced pitch estimate (MIDI note number, fractional).
    /// Returns true when the detected key changed.
    pub fn push(&mut self, midi: f32) -> bool {
        self.hist.iter_mut().for_each(|h| *h *= self.decay);
        // Weight notes sung close to a semitone more than slides between them.
        let nearest = midi.round();
        let weight = 1.0 - (midi - nearest).abs();
        self.hist[(nearest as i32).rem_euclid(12) as usize] += weight;

        self.since += 1;
        if self.since < EVALUATE_EVERY {
            return false;
        }
        self.since = 0;
        self.evaluate()
    }

    fn evaluate(&mut self) -> bool {
        if self.hist.iter().sum::<f32>() < self.min_weight {
            return false;
        }
        let hist = normalise(&self.hist);
        let (best, best_fit) = best_key_normalised(&self.profiles, &hist);
        let switch = match self.key {
            None => true,
            Some(cur) => cur != best && best_fit > fit(&self.profiles, &hist, cur) + SWITCH_MARGIN,
        };
        if switch {
            self.key = Some(best);
        }
        switch
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sings `notes` (MIDI numbers) for `secs` seconds each, at 100 estimates/s.
    fn sing(det: &mut KeyDetector, notes: &[f32], secs: f32) {
        for &n in notes {
            for _ in 0..(secs * 100.0) as usize {
                det.push(n);
            }
        }
    }

    #[test]
    fn encode_decode_roundtrip() {
        for v in 0..24 {
            assert_eq!(Key::decode(v).unwrap().encode(), v);
        }
        assert_eq!(Key::decode(Key::NONE), None);
        assert_eq!(
            Key {
                root: 9,
                minor: true
            }
            .name(),
            "A minor"
        );
        // Relative keys share their notes.
        assert_eq!(
            Key {
                root: 9,
                minor: true
            }
            .mask(),
            Key {
                root: 0,
                minor: false
            }
            .mask()
        );
    }

    #[test]
    fn needs_some_singing_first() {
        let mut det = KeyDetector::new(100.0, 10.0);
        sing(&mut det, &[60.0], 0.5);
        assert_eq!(det.key(), None);
    }

    #[test]
    fn finds_g_major_from_a_melody() {
        let mut det = KeyDetector::new(100.0, 10.0);
        // G A B C D E F# G, a bit out of tune, then a G major arpeggio.
        let melody = [
            67.1, 69.0, 70.8, 72.0, 74.2, 76.0, 77.9, 79.0, 67.0, 71.0, 74.0, 71.0, 67.0,
        ];
        sing(&mut det, &melody, 0.4);
        let key = det.key().expect("a key");
        assert_eq!(
            key.mask(),
            Key {
                root: 7,
                minor: false
            }
            .mask(),
            "got {}",
            key.name()
        );
    }

    #[test]
    fn finds_a_minor_and_follows_a_key_change() {
        let mut det = KeyDetector::new(100.0, 6.0);
        // A minor arpeggio and scale fragments, leaning on A, C and E.
        let a_minor = [
            57.0, 60.0, 64.0, 69.0, 64.0, 60.0, 57.0, 59.0, 62.0, 65.0, 64.0, 57.0,
        ];
        sing(&mut det, &a_minor, 0.4);
        assert_eq!(
            det.key(),
            Some(Key {
                root: 9,
                minor: true
            })
        );

        // Then a long stretch in E major (G#, C#, D#, F#).
        let e_major = [
            64.0, 68.0, 71.0, 76.0, 73.0, 75.0, 66.0, 64.0, 68.0, 71.0, 63.0, 64.0,
        ];
        sing(&mut det, &e_major, 1.0);
        let key = det.key().unwrap();
        assert_eq!(
            key.mask(),
            Key {
                root: 4,
                minor: false
            }
            .mask(),
            "got {}",
            key.name()
        );
    }
}
