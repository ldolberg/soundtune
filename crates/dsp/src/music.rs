/// Musical scales the autotune can snap to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MusicScale {
    Chromatic,
    Major,
    Minor,
    Pentatonic,
}

impl MusicScale {
    pub const ALL: [MusicScale; 4] = [
        MusicScale::Chromatic,
        MusicScale::Major,
        MusicScale::Minor,
        MusicScale::Pentatonic,
    ];

    pub fn name(self) -> &'static str {
        match self {
            MusicScale::Chromatic => "Chromatic",
            MusicScale::Major => "Major",
            MusicScale::Minor => "Minor",
            MusicScale::Pentatonic => "Pentatonic",
        }
    }

    /// Bit mask of the scale's pitch classes (bit 0 = C) in `key`.
    pub fn mask(self, key: u32) -> u32 {
        self.intervals()
            .iter()
            .fold(0, |m, &i| m | 1 << ((i + key as i32) % 12))
    }

    /// Semitone offsets from the key root that belong to the scale.
    pub fn intervals(self) -> &'static [i32] {
        match self {
            MusicScale::Chromatic => &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
            MusicScale::Major => &[0, 2, 4, 5, 7, 9, 11],
            MusicScale::Minor => &[0, 2, 3, 5, 7, 8, 10],
            MusicScale::Pentatonic => &[0, 2, 4, 7, 9],
        }
    }
}

pub const NOTE_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

pub fn hz_to_midi(hz: f32) -> f32 {
    69.0 + 12.0 * (hz / 440.0).log2()
}

/// Nearest note (as a MIDI number) to `midi` that belongs to `scale` in `key`.
pub fn snap_to_scale(midi: f32, key: u32, scale: MusicScale) -> f32 {
    snap_to_mask(midi, scale.mask(key))
}

/// Nearest note to `midi` whose pitch class is set in `mask` (bit 0 = C).
/// An empty mask leaves the pitch unchanged.
pub fn snap_to_mask(midi: f32, mask: u32) -> f32 {
    if mask & 0xFFF == 0 {
        return midi;
    }
    let base = midi.round() as i32;
    let mut best = base as f32;
    let mut best_dist = f32::MAX;
    for n in base - 6..=base + 6 {
        if mask & (1 << n.rem_euclid(12)) != 0 {
            let dist = (n as f32 - midi).abs();
            if dist < best_dist {
                best_dist = dist;
                best = n as f32;
            }
        }
    }
    best
}

/// Like [`snap_to_mask`], but notes in `preferred` count as `bias` semitones
/// closer than they are, so a note sung between a chord tone and a passing
/// note lands on the chord tone. Preferred notes need not be in `mask`.
pub fn snap_preferring(midi: f32, mask: u32, preferred: u32, bias: f32) -> f32 {
    let allowed = (mask | preferred) & 0xFFF;
    if allowed == 0 {
        return midi;
    }
    let base = midi.round() as i32;
    let mut best = base as f32;
    let mut best_cost = f32::MAX;
    for n in base - 7..=base + 7 {
        let bit = 1 << n.rem_euclid(12);
        if allowed & bit != 0 {
            let mut cost = (n as f32 - midi).abs();
            if preferred & bit != 0 {
                cost -= bias;
            }
            if cost < best_cost {
                best_cost = cost;
                best = n as f32;
            }
        }
    }
    best
}

/// Name like "A4" for a MIDI note number.
pub fn note_name(midi: f32) -> String {
    let n = midi.round() as i32;
    format!(
        "{}{}",
        NOTE_NAMES[n.rem_euclid(12) as usize],
        n.div_euclid(12) - 1
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a440_is_midi_69() {
        assert!((hz_to_midi(440.0) - 69.0).abs() < 1e-4);
        assert_eq!(note_name(69.0), "A4");
        assert_eq!(note_name(60.0), "C4");
    }

    #[test]
    fn snaps_to_nearest_scale_note() {
        // C# is not in C major: 61.2 should go to D (62), 60.9 to C (60).
        assert_eq!(snap_to_scale(61.2, 0, MusicScale::Major), 62.0);
        assert_eq!(snap_to_scale(60.9, 0, MusicScale::Major), 60.0);
        assert_eq!(snap_to_scale(61.4, 0, MusicScale::Chromatic), 61.0);
        // A minor pentatonic over key A (9): A C D E G -> 63.4 (D#) snaps to D or E.
        let s = snap_to_scale(63.4, 9, MusicScale::Minor);
        assert!(s == 62.0 || s == 64.0);
    }

    #[test]
    fn masks() {
        assert_eq!(MusicScale::Chromatic.mask(0), 0xFFF);
        // C major: C D E F G A B
        assert_eq!(MusicScale::Major.mask(0), 0b1010_1011_0101);
        // Only A enabled: everything goes to an A.
        assert_eq!(snap_to_mask(66.0, 1 << 9), 69.0);
        assert_eq!(snap_to_mask(61.3, 0), 61.3);
    }

    #[test]
    fn chord_tones_win_close_calls() {
        let c_major = MusicScale::Major.mask(0);
        let c_triad = 1 | 1 << 4 | 1 << 7;
        // D# sits between D and E: the chord tone E wins.
        assert_eq!(snap_preferring(63.0, c_major, c_triad, 0.4), 64.0);
        // A clean D or F stays (passing notes are still allowed).
        assert_eq!(snap_preferring(62.1, c_major, c_triad, 0.4), 62.0);
        assert_eq!(snap_preferring(65.0, c_major, c_triad, 0.4), 65.0);
        // Chord tones outside the scale are allowed (E major chord in C: G#).
        assert_eq!(
            snap_preferring(68.2, c_major, 1 << 4 | 1 << 8 | 1 << 11, 0.4),
            68.0
        );
        assert_eq!(snap_preferring(61.3, 0, 0, 0.4), 61.3);
    }
}
