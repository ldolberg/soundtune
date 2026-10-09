//! Harmonic analysis of a backing track: key, chords and the notes the
//! autotune may use at each moment. Runs offline on a loader thread.

use crate::chroma::{chromagram, Chromagram};
use crate::key::best_key;
use crate::{Key, NOTE_NAMES};

/// Seconds per analysis frame.
pub const FRAME_SECS: f32 = 0.25;
/// Minimum cosine similarity between a frame and a triad to call it.
const CHORD_THRESHOLD: f32 = 0.6;
/// Bonus for chords whose notes all belong to the song's key.
const DIATONIC_BONUS: f32 = 0.06;
/// Cost of changing chord between two frames, which keeps short glitches
/// (a passing bass note, a drum hit) from flickering the chord.
const SWITCH_PENALTY: f32 = 0.25;

/// A major or minor triad.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chord {
    pub root: u32,
    pub minor: bool,
}

impl Chord {
    /// Chord tones, one bit per pitch class (bit 0 = C).
    pub fn mask(self) -> u32 {
        let third = if self.minor { 3 } else { 4 };
        [0, third, 7]
            .iter()
            .fold(0, |m, i| m | 1 << ((self.root + i) % 12))
    }

    /// Like "C" or "F#m".
    pub fn name(self) -> String {
        format!(
            "{}{}",
            NOTE_NAMES[self.root as usize % 12],
            if self.minor { "m" } else { "" }
        )
    }

    fn all() -> impl Iterator<Item = Chord> {
        (0..24).map(|v| Chord {
            root: v % 12,
            minor: v >= 12,
        })
    }
}

/// A stretch of the song with the same notes allowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HarmonySegment {
    /// First sample (at `Harmony::rate`).
    pub start: u64,
    /// Notes of the song's key.
    pub scale: u32,
    /// The chord sounding, if one was recognised.
    pub chord: Option<Chord>,
}

impl HarmonySegment {
    pub fn chord_mask(&self) -> u32 {
        self.chord.map_or(0, Chord::mask)
    }

    /// Every note the autotune may use here: the key's scale plus the
    /// chord tones (a borrowed chord may add a note outside the key).
    pub fn mask(&self) -> u32 {
        self.scale | self.chord_mask()
    }
}

/// Key and chord timeline of a song.
#[derive(Debug, Clone)]
pub struct Harmony {
    /// Sample rate the segment starts are counted in.
    pub rate: f32,
    pub key: Option<Key>,
    /// Sorted by start, the first one starts at 0.
    pub segments: Vec<HarmonySegment>,
    /// Song length in samples.
    pub length: u64,
}

impl Harmony {
    /// The segment playing at sample `pos`, `None` past the end.
    pub fn at(&self, pos: u64) -> Option<&HarmonySegment> {
        if pos >= self.length {
            return None;
        }
        let i = self.segments.partition_point(|s| s.start <= pos);
        i.checked_sub(1).map(|i| &self.segments[i])
    }

    /// The allowed note masks over time: `(start_sample, mask)`.
    pub fn timeline(&self) -> Vec<(u64, u32)> {
        self.segments.iter().map(|s| (s.start, s.mask())).collect()
    }
}

/// The key that best explains the whole chromagram (Krumhansl-Schmuckler).
pub fn estimate_key(frames: &[[f32; 12]]) -> Option<Key> {
    let mut hist = [0.0f32; 12];
    for f in frames {
        for (h, c) in hist.iter_mut().zip(f) {
            *h += c;
        }
    }
    best_key(&hist).map(|(k, _)| k)
}

fn cosine(a: &[f32; 12], b: &[f32; 12]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na > 0.0 && nb > 0.0 {
        dot / (na * nb)
    } else {
        0.0
    }
}

/// Recognises a triad per frame by template matching, then picks the most
/// likely chord sequence (Viterbi) with a cost for every change, so the
/// result does not flicker. `None` where no triad fits (silence, drums).
pub fn detect_chords(frames: &[[f32; 12]], key: Option<Key>) -> Vec<Option<Chord>> {
    let chords: Vec<Chord> = Chord::all().collect();
    let templates: Vec<[f32; 12]> = chords
        .iter()
        .map(|c| std::array::from_fn(|pc| ((c.mask() >> pc) & 1) as f32))
        .collect();
    let key_mask = key.map_or(0xFFF, Key::mask);
    let states = chords.len() + 1; // last state = no chord

    let score = |i: usize| -> Vec<f32> {
        // Average with the neighbours to steady the estimate.
        let lo = i.saturating_sub(1);
        let hi = (i + 2).min(frames.len());
        let mut avg = [0.0f32; 12];
        for f in &frames[lo..hi] {
            for (a, c) in avg.iter_mut().zip(f) {
                *a += c;
            }
        }
        let silent = frames[i].iter().all(|&c| c == 0.0);
        let mut s: Vec<f32> = chords
            .iter()
            .zip(&templates)
            .map(|(c, t)| {
                if silent {
                    return 0.0;
                }
                let bonus = if c.mask() & !key_mask == 0 {
                    DIATONIC_BONUS
                } else {
                    0.0
                };
                cosine(&avg, t) + bonus
            })
            .collect();
        s.push(CHORD_THRESHOLD);
        s
    };

    if frames.is_empty() {
        return Vec::new();
    }
    let mut total = score(0);
    let mut back: Vec<Vec<u8>> = Vec::with_capacity(frames.len());
    back.push(vec![0; states]);
    for i in 1..frames.len() {
        let s = score(i);
        let (best_prev, best_total) = total
            .iter()
            .cloned()
            .enumerate()
            .fold((0, f32::MIN), |b, (j, t)| if t > b.1 { (j, t) } else { b });
        let mut next = vec![0.0; states];
        let mut from = vec![0u8; states];
        for st in 0..states {
            let stay = total[st];
            let (prev, t) = if stay >= best_total - SWITCH_PENALTY {
                (st, stay)
            } else {
                (best_prev, best_total - SWITCH_PENALTY)
            };
            next[st] = t + s[st];
            from[st] = prev as u8;
        }
        total = next;
        back.push(from);
    }

    let mut st = total
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))
        .map_or(states - 1, |(i, _)| i);
    let mut out = vec![None; frames.len()];
    for i in (0..frames.len()).rev() {
        out[i] = chords.get(st).copied();
        st = back[i][st] as usize;
    }
    out
}

/// A key and its relative (A minor and C major) share their notes, so the
/// pitch class profile alone often cannot tell them apart. The one whose
/// tonic chord sounds longer, and opens and closes the song, wins.
fn settle_relative(key: Key, chords: &[Option<Chord>]) -> Key {
    let relative = if key.minor {
        Key {
            root: (key.root + 3) % 12,
            minor: false,
        }
    } else {
        Key {
            root: (key.root + 9) % 12,
            minor: true,
        }
    };
    let voiced: Vec<Chord> = chords.iter().flatten().copied().collect();
    let score = |k: Key| {
        let tonic = Chord {
            root: k.root,
            minor: k.minor,
        };
        let held = voiced.iter().filter(|&&c| c == tonic).count() as f32;
        let ends = [voiced.first(), voiced.last()]
            .iter()
            .filter(|c| **c == Some(&tonic))
            .count() as f32;
        // Opening or closing on the tonic is worth 2 s of it.
        held + ends * 2.0 / FRAME_SECS
    };
    if score(relative) > score(key) {
        relative
    } else {
        key
    }
}

/// Builds the harmony timeline from an already computed chromagram.
pub fn harmony_from_chroma(chroma: &Chromagram, length: u64) -> Harmony {
    let key = estimate_key(&chroma.frames);
    let scale = key.map_or(0xFFF, Key::mask);
    let chords = detect_chords(&chroma.frames, key);
    let key = key.map(|k| settle_relative(k, &chords));
    let mut segments: Vec<HarmonySegment> = Vec::new();
    for (i, chord) in chords.into_iter().enumerate() {
        let seg = HarmonySegment {
            start: (i * chroma.hop) as u64,
            scale,
            chord,
        };
        match segments.last() {
            Some(last) if last.chord == seg.chord => {}
            _ => segments.push(seg),
        }
    }
    if segments.is_empty() {
        segments.push(HarmonySegment {
            start: 0,
            scale,
            chord: None,
        });
    }
    Harmony {
        rate: chroma.rate,
        key,
        segments,
        length,
    }
}

/// Analyses a mono recording: chromagram, key and chord timeline.
pub fn analyse_harmony(samples: &[f32], rate: f32) -> Harmony {
    let chroma = chromagram(samples, rate, FRAME_SECS);
    harmony_from_chroma(&chroma, samples.len() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chroma::tests::chord;

    fn progression(chords: &[&[f32]], rate: f32, secs: f32) -> Vec<f32> {
        chords
            .iter()
            .flat_map(|notes| chord(notes, rate, secs))
            .collect()
    }

    fn ch(root: u32, minor: bool) -> Chord {
        Chord { root, minor }
    }

    #[test]
    fn chord_masks_and_names() {
        assert_eq!(ch(0, false).mask(), 1 | 1 << 4 | 1 << 7);
        assert_eq!(ch(9, true).mask(), 1 << 9 | 1 | 1 << 4);
        assert_eq!(ch(6, true).name(), "F#m");
        assert_eq!(ch(7, false).name(), "G");
    }

    #[test]
    fn c_major_progression() {
        let rate = 22_050.0;
        // C - F - G - C, two seconds each, with the bass an octave down.
        let audio = progression(
            &[
                &[48.0, 60.0, 64.0, 67.0],
                &[41.0, 60.0, 65.0, 69.0],
                &[43.0, 59.0, 62.0, 67.0],
                &[48.0, 60.0, 64.0, 67.0],
            ],
            rate,
            2.0,
        );
        let h = analyse_harmony(&audio, rate);
        let key = h.key.expect("key");
        assert_eq!(
            key,
            Key {
                root: 0,
                minor: false
            },
            "got {}",
            key.name()
        );

        let names: Vec<String> = h
            .segments
            .iter()
            .map(|s| s.chord.map_or("-".into(), Chord::name))
            .collect();
        assert_eq!(names, ["C", "F", "G", "C"]);
        // Changes land within a frame of the real chord changes.
        for (seg, at) in h.segments.iter().zip([0.0, 2.0, 4.0, 6.0]) {
            let t = seg.start as f32 / rate;
            assert!((t - at).abs() <= FRAME_SECS + 1e-3, "{t} vs {at}");
        }
        // Mid-way through the F chord: scale notes plus F A C preferred.
        let seg = h.at((3.0 * rate) as u64).unwrap();
        assert_eq!(
            seg.scale,
            Key {
                root: 0,
                minor: false
            }
            .mask()
        );
        assert_eq!(seg.chord_mask(), 1 << 5 | 1 << 9 | 1);
        assert!(h.at(h.length).is_none());
        assert_eq!(h.timeline().len(), 4);
    }

    #[test]
    fn a_minor_progression_with_a_major_dominant() {
        let rate = 22_050.0;
        // Am - Dm - E - Am: the E chord brings in G#.
        let audio = progression(
            &[
                &[45.0, 57.0, 60.0, 64.0],
                &[50.0, 57.0, 62.0, 65.0],
                &[40.0, 56.0, 59.0, 64.0],
                &[45.0, 57.0, 60.0, 64.0],
            ],
            rate,
            2.0,
        );
        let h = analyse_harmony(&audio, rate);
        let key = h.key.expect("key");
        assert_eq!(
            key,
            Key {
                root: 9,
                minor: true
            },
            "got {}",
            key.name()
        );
        let e = h.at((5.0 * rate) as u64).unwrap();
        assert_eq!(e.chord, Some(ch(4, false)));
        // G# is allowed over the E chord although it is not in A minor.
        assert_ne!(e.mask() & 1 << 8, 0);
        assert_eq!(e.scale & 1 << 8, 0);
    }

    #[test]
    fn relative_keys_follow_the_tonic_chord() {
        let rate = 22_050.0;
        // Am F C G Am: the C major notes, but it opens and closes on Am.
        let audio = progression(
            &[
                &[45.0, 57.0, 60.0, 64.0],
                &[41.0, 57.0, 60.0, 65.0],
                &[48.0, 55.0, 60.0, 64.0],
                &[43.0, 55.0, 59.0, 62.0],
                &[45.0, 57.0, 60.0, 64.0],
            ],
            rate,
            2.0,
        );
        let h = analyse_harmony(&audio, rate);
        let key = h.key.expect("key");
        assert_eq!(
            key,
            Key {
                root: 9,
                minor: true
            },
            "got {}",
            key.name()
        );
    }

    #[test]
    fn silence_has_no_chord() {
        let frames = vec![[0.0f32; 12]; 8];
        assert!(detect_chords(&frames, None).iter().all(Option::is_none));
        assert_eq!(estimate_key(&frames), None);
        let h = analyse_harmony(&vec![0.0; 22_050], 22_050.0);
        assert_eq!(h.segments.len(), 1);
        assert_eq!(h.at(0).unwrap().mask(), 0xFFF);
    }
}
