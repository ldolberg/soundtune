//! MIDI guide melody: the exact notes the singer should hit.

use midly::{MetaMessage, MidiMessage, Smf, Timing, TrackEventKind};

/// MIDI channel 10 (index 9) is reserved for drums.
const DRUM_CHANNEL: u8 = 9;
/// Notes a singer can reach (C3..C6), used to find the melody part.
const VOCAL_RANGE: std::ops::RangeInclusive<u8> = 48..=84;
/// Default tempo: 120 bpm.
const DEFAULT_TEMPO: u32 = 500_000;

/// A note with times in seconds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimedNote {
    pub start: f64,
    pub end: f64,
    pub note: u8,
}

/// The notes of one channel of one MIDI track.
#[derive(Debug, Clone)]
pub struct MidiPart {
    pub track: usize,
    pub channel: u8,
    /// Track name from the file, may be empty.
    pub name: String,
    /// Sorted by start time.
    pub notes: Vec<TimedNote>,
}

impl MidiPart {
    pub fn is_drums(&self) -> bool {
        self.channel == DRUM_CHANNEL
    }

    /// Notes inside a singable range.
    pub fn vocal_notes(&self) -> usize {
        self.notes
            .iter()
            .filter(|n| VOCAL_RANGE.contains(&n.note))
            .count()
    }

    /// Fraction (0..1) of notes that start while another is still held.
    pub fn polyphony(&self) -> f32 {
        if self.notes.is_empty() {
            return 0.0;
        }
        let mut held_until = f64::MIN;
        let mut overlapping = 0;
        for n in &self.notes {
            // A few ms of legato overlap is still a melody.
            if n.start < held_until - 0.03 {
                overlapping += 1;
            }
            held_until = held_until.max(n.end);
        }
        overlapping as f32 / self.notes.len() as f32
    }

    /// Like "Track 2: Lead (ch 1, 120 notes)".
    pub fn label(&self) -> String {
        let name = if self.name.trim().is_empty() {
            String::new()
        } else {
            format!(": {}", self.name.trim())
        };
        format!(
            "Track {}{name} (ch {}, {} notes)",
            self.track + 1,
            self.channel + 1,
            self.notes.len()
        )
    }

    /// Turns the part into a guide melody with sample positions at
    /// `rate`. Chords are reduced to their top note and overlapping notes
    /// are cut where the next one starts, so at most one note is active.
    pub fn guide(&self, rate: f32) -> GuideMelody {
        let rate = rate as f64;
        let mut notes: Vec<GuideNote> = Vec::with_capacity(self.notes.len());
        for n in &self.notes {
            let start = (n.start * rate).round().max(0.0) as u64;
            let end = (n.end * rate).round().max(0.0) as u64;
            if end <= start {
                continue;
            }
            if let Some(last) = notes.last_mut() {
                if last.start == start {
                    // Same onset: keep the highest note.
                    if n.note > last.note {
                        *last = GuideNote {
                            start,
                            end,
                            note: n.note,
                        };
                    }
                    continue;
                }
                last.end = last.end.min(start);
            }
            notes.push(GuideNote {
                start,
                end,
                note: n.note,
            });
        }
        GuideMelody {
            rate: rate as f32,
            notes,
        }
    }
}

/// A parsed MIDI file, split into parts.
#[derive(Debug, Clone)]
pub struct MidiSong {
    /// Parts with at least one note.
    pub parts: Vec<MidiPart>,
}

impl MidiSong {
    /// The part most likely to be the vocal melody: not drums, mostly one
    /// note at a time, with the most notes in a singable range.
    pub fn default_part(&self) -> Option<usize> {
        self.parts
            .iter()
            .enumerate()
            .filter(|(_, p)| !p.is_drums())
            .map(|(i, p)| {
                let score = p.vocal_notes() as f32 * (1.0 - 0.8 * p.polyphony());
                (i, score)
            })
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }

    /// End of the last note in seconds.
    pub fn length(&self) -> f64 {
        self.parts
            .iter()
            .flat_map(|p| p.notes.iter().map(|n| n.end))
            .fold(0.0, f64::max)
    }
}

/// Converts MIDI ticks to seconds through the file's tempo map.
struct TempoMap {
    /// (tick, seconds at that tick, seconds per tick from there on)
    segments: Vec<(u64, f64, f64)>,
}

impl TempoMap {
    fn new(timing: Timing, mut changes: Vec<(u64, u32)>) -> Self {
        match timing {
            Timing::Timecode(fps, sub) => {
                let per_tick = 1.0 / (fps.as_f32() as f64 * sub.max(1) as f64);
                Self {
                    segments: vec![(0, 0.0, per_tick)],
                }
            }
            Timing::Metrical(ppq) => {
                let ppq = ppq.as_int().max(1) as f64;
                changes.sort_by_key(|c| c.0);
                let mut segments = vec![(0, 0.0, DEFAULT_TEMPO as f64 / 1e6 / ppq)];
                for (tick, tempo) in changes {
                    let last = segments.len() - 1;
                    let (t0, s0, per) = segments[last];
                    let seg = (
                        tick,
                        s0 + (tick - t0) as f64 * per,
                        tempo as f64 / 1e6 / ppq,
                    );
                    if tick == t0 {
                        // Several changes at the same tick: the last one wins.
                        segments[last] = seg;
                    } else {
                        segments.push(seg);
                    }
                }
                Self { segments }
            }
        }
    }

    fn seconds(&self, tick: u64) -> f64 {
        let i = self.segments.partition_point(|s| s.0 <= tick).max(1) - 1;
        let (t0, s0, per) = self.segments[i];
        s0 + (tick - t0) as f64 * per
    }
}

/// Parses a standard MIDI file into parts (one per track and channel),
/// with note times in seconds following the tempo changes.
pub fn parse_midi(bytes: &[u8]) -> Result<MidiSong, String> {
    let smf = Smf::parse(bytes).map_err(|e| format!("not a MIDI file: {e}"))?;

    // Tempo changes may sit in any track (usually the first).
    let mut tempos = Vec::new();
    for track in &smf.tracks {
        let mut tick = 0u64;
        for ev in track {
            tick += ev.delta.as_int() as u64;
            if let TrackEventKind::Meta(MetaMessage::Tempo(t)) = ev.kind {
                tempos.push((tick, t.as_int()));
            }
        }
    }
    let map = TempoMap::new(smf.header.timing, tempos);

    let mut parts = Vec::new();
    for (index, track) in smf.tracks.iter().enumerate() {
        let mut name = String::new();
        // Notes being held, per channel and key: start tick.
        let mut held: Vec<Vec<Vec<u64>>> = vec![vec![Vec::new(); 128]; 16];
        let mut notes: Vec<Vec<(u64, u64, u8)>> = vec![Vec::new(); 16];
        let mut tick = 0u64;
        for ev in track {
            tick += ev.delta.as_int() as u64;
            match ev.kind {
                TrackEventKind::Meta(MetaMessage::TrackName(n)) => {
                    name = String::from_utf8_lossy(n).into_owned();
                }
                TrackEventKind::Midi { channel, message } => {
                    let ch = channel.as_int() as usize;
                    let (key, on) = match message {
                        MidiMessage::NoteOn { key, vel } => (key.as_int(), vel.as_int() > 0),
                        MidiMessage::NoteOff { key, .. } => (key.as_int(), false),
                        _ => continue,
                    };
                    if on {
                        held[ch][key as usize].push(tick);
                    } else if let Some(start) = held[ch][key as usize].pop() {
                        notes[ch].push((start, tick, key));
                    }
                }
                _ => {}
            }
        }
        // Notes never released end with the track.
        for (ch, keys) in held.iter().enumerate() {
            for (key, starts) in keys.iter().enumerate() {
                for &start in starts {
                    notes[ch].push((start, tick.max(start + 1), key as u8));
                }
            }
        }
        for (ch, mut list) in notes.into_iter().enumerate() {
            if list.is_empty() {
                continue;
            }
            list.sort_by_key(|n| (n.0, n.2));
            parts.push(MidiPart {
                track: index,
                channel: ch as u8,
                name: name.clone(),
                notes: list
                    .into_iter()
                    .map(|(s, e, note)| TimedNote {
                        start: map.seconds(s),
                        end: map.seconds(e),
                        note,
                    })
                    .collect(),
            });
        }
    }
    if parts.is_empty() {
        return Err("the MIDI file has no notes".into());
    }
    Ok(MidiSong { parts })
}

/// One guide note, in samples at `GuideMelody::rate`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuideNote {
    pub start: u64,
    pub end: u64,
    pub note: u8,
}

/// A monophonic melody: sorted, non-overlapping notes.
#[derive(Debug, Clone, Default)]
pub struct GuideMelody {
    pub rate: f32,
    pub notes: Vec<GuideNote>,
}

impl GuideMelody {
    /// The note sounding at sample `pos`, if any. Allocation free.
    pub fn note_at(&self, pos: u64) -> Option<u8> {
        let i = self.notes.partition_point(|n| n.start <= pos);
        let n = self.notes.get(i.checked_sub(1)?)?;
        (pos < n.end).then_some(n.note)
    }

    /// The timeline as `(start_sample, end_sample, midi_note)`.
    pub fn timeline(&self) -> Vec<(u64, u64, u8)> {
        self.notes
            .iter()
            .map(|n| (n.start, n.end, n.note))
            .collect()
    }

    /// End of the last note in samples.
    pub fn length(&self) -> u64 {
        self.notes.last().map_or(0, |n| n.end)
    }
}

/// `note` moved by whole octaves to be as close as possible to `sung`
/// (both MIDI numbers), so the guide works for low and high voices.
pub fn nearest_octave(note: f32, sung: f32) -> f32 {
    note + 12.0 * ((sung - note) / 12.0).round()
}

#[cfg(test)]
mod tests {
    use super::*;
    use midly::num::{u15, u24, u28, u4, u7};
    use midly::{Format, Header, TrackEvent};

    fn ev(delta: u32, kind: TrackEventKind<'static>) -> TrackEvent<'static> {
        TrackEvent {
            delta: u28::new(delta),
            kind,
        }
    }

    fn note(ch: u8, key: u8, on: bool) -> TrackEventKind<'static> {
        let (key, vel) = (u7::new(key), u7::new(if on { 100 } else { 0 }));
        TrackEventKind::Midi {
            channel: u4::new(ch),
            message: if on {
                MidiMessage::NoteOn { key, vel }
            } else {
                MidiMessage::NoteOff { key, vel }
            },
        }
    }

    fn tempo(us: u32) -> TrackEventKind<'static> {
        TrackEventKind::Meta(MetaMessage::Tempo(u24::new(us)))
    }

    fn end() -> TrackEvent<'static> {
        ev(0, TrackEventKind::Meta(MetaMessage::EndOfTrack))
    }

    /// Format 1, 480 PPQ: a conductor track (120 bpm, then 60 bpm from
    /// beat 2), a piano part with chords and a monophonic lead.
    fn test_file() -> Vec<u8> {
        let mut smf = Smf::new(Header::new(
            Format::Parallel,
            Timing::Metrical(u15::new(480)),
        ));
        smf.tracks.push(vec![
            ev(0, tempo(500_000)),
            ev(960, tempo(1_000_000)),
            end(),
        ]);
        // Piano: C major chords, one per beat, low register.
        let mut piano = vec![ev(
            0,
            TrackEventKind::Meta(MetaMessage::TrackName(b"Piano")),
        )];
        for _ in 0..4 {
            for k in [36u8, 40, 43, 48] {
                piano.push(ev(0, note(0, k, true)));
            }
            piano.push(ev(480, note(0, 36, false)));
            for k in [40u8, 43, 48] {
                piano.push(ev(0, note(0, k, false)));
            }
        }
        piano.push(end());
        smf.tracks.push(piano);
        // Lead: A4 for 2 beats, rest a beat, then C5 (NoteOn vel 0 ends it).
        smf.tracks.push(vec![
            ev(0, TrackEventKind::Meta(MetaMessage::TrackName(b"Lead"))),
            ev(0, note(1, 69, true)),
            ev(960, note(1, 69, false)),
            ev(480, note(1, 72, true)),
            ev(480, note(1, 72, false)),
            end(),
        ]);
        let mut out = Vec::new();
        smf.write_std(&mut out).unwrap();
        out
    }

    #[test]
    fn parses_tempo_changes_and_picks_the_lead() {
        let song = parse_midi(&test_file()).unwrap();
        assert_eq!(song.parts.len(), 2);
        let lead_idx = song.default_part().unwrap();
        let lead = &song.parts[lead_idx];
        assert_eq!(lead.name, "Lead");
        assert_eq!(lead.track, 2);
        assert!(lead.label().contains("Lead"));
        assert_eq!(lead.polyphony(), 0.0);
        assert!(song
            .parts
            .iter()
            .any(|p| p.name == "Piano" && p.polyphony() > 0.5));

        // Beats 0..2 at 120 bpm = 0..1 s. Beat 3 starts at 1 s + 1 beat at
        // 60 bpm = 2 s and lasts one 60 bpm beat (1 s).
        let n = &lead.notes;
        assert_eq!(n.len(), 2);
        assert!((n[0].start - 0.0).abs() < 1e-9 && (n[0].end - 1.0).abs() < 1e-9);
        assert!((n[1].start - 2.0).abs() < 1e-9 && (n[1].end - 3.0).abs() < 1e-9);
        assert_eq!((n[0].note, n[1].note), (69, 72));
        assert!((song.length() - 3.0).abs() < 1e-9);

        let guide = lead.guide(48_000.0);
        assert_eq!(
            guide.timeline(),
            vec![(0, 48_000, 69), (96_000, 144_000, 72)]
        );
        assert_eq!(guide.note_at(0), Some(69));
        assert_eq!(guide.note_at(47_999), Some(69));
        assert_eq!(guide.note_at(48_000), None);
        assert_eq!(guide.note_at(100_000), Some(72));
        assert_eq!(guide.note_at(200_000), None);
        assert_eq!(guide.length(), 144_000);
    }

    #[test]
    fn chords_become_a_single_line() {
        let song = parse_midi(&test_file()).unwrap();
        let piano = song.parts.iter().find(|p| p.name == "Piano").unwrap();
        let guide = piano.guide(1000.0);
        assert_eq!(guide.notes.len(), 4);
        assert!(guide.notes.iter().all(|n| n.note == 48));
        assert!(guide.notes.windows(2).all(|w| w[0].end <= w[1].start));
    }

    #[test]
    fn timecode_files_and_errors() {
        let mut smf = Smf::new(Header::new(
            Format::SingleTrack,
            Timing::Timecode(midly::Fps::Fps25, 40),
        ));
        smf.tracks.push(vec![
            ev(0, note(0, 60, true)),
            ev(500, note(0, 60, false)),
            end(),
        ]);
        let mut bytes = Vec::new();
        smf.write_std(&mut bytes).unwrap();
        let song = parse_midi(&bytes).unwrap();
        // 25 fps * 40 ticks per frame = 1000 ticks per second.
        assert!((song.parts[0].notes[0].end - 0.5).abs() < 1e-9);
        assert!(parse_midi(b"not midi").is_err());
    }

    #[test]
    fn octave_follows_the_singer() {
        // Guide A4 (69): a man singing around A2..A3 gets an A3.
        assert_eq!(nearest_octave(69.0, 56.2), 57.0);
        assert_eq!(nearest_octave(69.0, 68.6), 69.0);
        assert_eq!(nearest_octave(69.0, 80.0), 81.0);
        assert_eq!(nearest_octave(60.0, 45.0), 48.0);
    }
}
