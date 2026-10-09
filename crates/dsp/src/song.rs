//! Backing track playback and the song timelines the autotune follows.
//!
//! Everything the audio threads read lives behind `ArcSwapOption`s: the UI
//! builds a new, immutable `Track`, `Harmony` or `GuideMelody` on a loader
//! thread and swaps it in; the audio callbacks only `load()` it, which never
//! blocks or allocates. The UI keeps replaced values alive for a moment so
//! the last reference is never dropped (freed) on an audio thread.

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

use arc_swap::ArcSwapOption;

use crate::{AtomicF32, GuideMelody, Harmony, Toggle, NO_NOTE};

/// A decoded backing track: mono or stereo at its own sample rate.
pub struct Track {
    pub rate: f32,
    /// 1 or 2.
    pub channels: usize,
    /// Interleaved samples.
    pub samples: Vec<f32>,
}

impl Track {
    pub fn frames(&self) -> usize {
        self.samples.len() / self.channels.max(1)
    }

    /// Length in seconds.
    pub fn duration(&self) -> f64 {
        self.frames() as f64 / self.rate as f64
    }

    fn frame(&self, i: usize) -> (f32, f32) {
        if self.channels >= 2 {
            let j = 2 * i;
            (self.samples[j], self.samples[j + 1])
        } else {
            (self.samples[i], self.samples[i])
        }
    }

    /// Left and right at a fractional frame position (linear
    /// interpolation), silence outside the track.
    #[inline]
    pub fn stereo_at(&self, pos: f64) -> (f32, f32) {
        if pos < 0.0 {
            return (0.0, 0.0);
        }
        let i = pos as usize;
        if i + 1 >= self.frames() {
            return if i < self.frames() {
                self.frame(i)
            } else {
                (0.0, 0.0)
            };
        }
        let t = (pos - i as f64) as f32;
        let (a, b) = (self.frame(i), self.frame(i + 1));
        (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
    }
}

/// Shared song state: what is loaded, transport and follow settings.
pub struct SongState {
    pub track: ArcSwapOption<Track>,
    /// Key and chord timeline analysed from the track.
    pub harmony: ArcSwapOption<Harmony>,
    /// Melody from a MIDI file.
    pub guide: ArcSwapOption<GuideMelody>,
    pub playing: Toggle,
    /// Set by the UI to go back to the start; the player clears it.
    pub rewind: Toggle,
    /// Backing track volume, linear.
    pub volume: AtomicF32,
    /// Song position in samples at `clock_rate` (the input sample rate the
    /// processor runs at). Written by the player.
    pub position: AtomicU64,
    /// Rate `position` counts in, set when the audio starts.
    pub clock_rate: AtomicF32,
    /// Song length in seconds (the track, or the guide without a track).
    pub length: AtomicF32,
    /// Autotune follows the song (guide notes, then chords and key).
    pub follow: Toggle,
    /// Moves the guide later (positive) or earlier, in milliseconds.
    pub guide_offset_ms: AtomicF32,
    /// Transposes the guide, in semitones.
    pub guide_shift: AtomicF32,
}

impl Default for SongState {
    fn default() -> Self {
        Self {
            track: ArcSwapOption::empty(),
            harmony: ArcSwapOption::empty(),
            guide: ArcSwapOption::empty(),
            playing: Toggle::new(false),
            rewind: Toggle::new(false),
            volume: AtomicF32::new(0.7),
            position: AtomicU64::new(0),
            clock_rate: AtomicF32::new(48_000.0),
            length: AtomicF32::new(0.0),
            follow: Toggle::new(true),
            guide_offset_ms: AtomicF32::new(0.0),
            guide_shift: AtomicF32::new(0.0),
        }
    }
}

impl fmt::Debug for SongState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SongState")
            .field("track", &self.track.load().is_some())
            .field("harmony", &self.harmony.load().is_some())
            .field("guide", &self.guide.load().is_some())
            .field("playing", &self.playing)
            .field("position", &self.position)
            .field("follow", &self.follow)
            .finish_non_exhaustive()
    }
}

/// What the song wants the singer to sing at some moment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SongTarget {
    /// Exactly this note (MIDI number, in whatever octave is closest).
    Guide(f32),
    /// The key's notes, preferring the chord tones.
    Notes { scale: u32, chord: u32 },
}

impl SongTarget {
    /// (allowed notes mask, chord mask, guide note or `NO_NOTE`) as shown
    /// in `Meters`; `(0, 0, NO_NOTE)` for `None`.
    pub fn display(target: Option<SongTarget>) -> (u32, u32, u32) {
        match target {
            None => (0, 0, NO_NOTE),
            Some(SongTarget::Guide(n)) => {
                let n = n.round() as i32;
                (1 << n.rem_euclid(12), 0, n.max(0) as u32)
            }
            Some(SongTarget::Notes { scale, chord }) => (scale | chord, chord, NO_NOTE),
        }
    }
}

impl SongState {
    /// Current position in seconds.
    pub fn seconds(&self) -> f64 {
        self.position.load(Ordering::Relaxed) as f64 / self.clock_rate.get().max(1.0) as f64
    }

    /// What the song wants at `secs`: the guide note if one is sounding
    /// (after the offset and shift), else the key and chord, else `None`.
    /// Lock and allocation free.
    pub fn target_at(&self, secs: f64) -> Option<SongTarget> {
        let guide = self.guide.load();
        if let Some(g) = guide.as_deref() {
            let t = secs - self.guide_offset_ms.get() as f64 / 1000.0;
            if t >= 0.0 {
                if let Some(note) = g.note_at((t * g.rate as f64) as u64) {
                    let shift = self.guide_shift.get().round();
                    return Some(SongTarget::Guide(note as f32 + shift));
                }
            }
        }
        let harmony = self.harmony.load();
        let seg = harmony
            .as_deref()
            .and_then(|h| h.at((secs.max(0.0) * h.rate as f64) as u64))?;
        Some(SongTarget::Notes {
            scale: seg.scale,
            chord: seg.chord_mask(),
        })
    }
}

/// Plays the backing track in the output callback and advances the song
/// clock. Allocation and lock free.
pub struct SongPlayer {
    in_rate: f64,
    /// Input-rate samples per output frame.
    step: f64,
    /// Position in input-rate samples.
    pos: f64,
}

impl SongPlayer {
    /// `in_rate` is the processor's (input) rate, `out_rate` the output
    /// device's. Continues from the song's current position.
    pub fn new(song: &SongState, in_rate: f32, out_rate: f32) -> Self {
        let old_rate = song.clock_rate.get().max(1.0) as f64;
        let pos = song.position.load(Ordering::Relaxed) as f64 * in_rate as f64 / old_rate;
        song.clock_rate.set(in_rate);
        song.position.store(pos as u64, Ordering::Relaxed);
        Self {
            in_rate: in_rate as f64,
            step: in_rate as f64 / out_rate as f64,
            pos,
        }
    }

    /// Adds the backing track to `out` (interleaved, `channels` per frame)
    /// and moves the song clock on.
    pub fn render(&mut self, song: &SongState, out: &mut [f32], channels: usize) {
        if song.rewind.take() {
            self.pos = 0.0;
        }
        if song.playing.get() {
            let end = song.length.get() as f64 * self.in_rate;
            let vol = song.volume.get();
            let track = song.track.load();
            let track = track.as_deref();
            let to_track = track.map_or(0.0, |t| t.rate as f64 / self.in_rate);
            let channels = channels.max(1);
            for frame in out.chunks_mut(channels) {
                if self.pos >= end {
                    song.playing.set(false);
                    break;
                }
                if let Some(t) = track {
                    let (l, r) = t.stereo_at(self.pos * to_track);
                    let (l, r) = (l * vol, r * vol);
                    match frame {
                        [mono] => *mono += 0.5 * (l + r),
                        [a, b, rest @ ..] => {
                            *a += l;
                            *b += r;
                            rest.iter_mut().for_each(|s| *s += 0.5 * (l + r));
                        }
                        [] => {}
                    }
                }
                self.pos += self.step;
            }
        }
        song.position.store(self.pos as u64, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn ramp_track(rate: f32, frames: usize) -> Track {
        Track {
            rate,
            channels: 2,
            samples: (0..frames).flat_map(|i| [i as f32, -(i as f32)]).collect(),
        }
    }

    #[test]
    fn interpolates_and_stays_silent_outside() {
        let t = ramp_track(10.0, 4);
        assert_eq!(t.frames(), 4);
        assert_eq!(t.stereo_at(1.5), (1.5, -1.5));
        assert_eq!(t.stereo_at(3.0), (3.0, -3.0));
        assert_eq!(t.stereo_at(7.0), (0.0, 0.0));
        assert_eq!(t.stereo_at(-1.0), (0.0, 0.0));
        let mono = Track {
            rate: 10.0,
            channels: 1,
            samples: vec![0.5, 1.0],
        };
        assert_eq!(mono.stereo_at(0.0), (0.5, 0.5));
        assert!((mono.duration() - 0.2).abs() < 1e-9);
    }

    #[test]
    fn plays_resampled_advances_and_stops_at_the_end() {
        let song = SongState::default();
        // A 1 s track at 24 kHz, processor at 48 kHz, output at 96 kHz.
        song.track
            .store(Some(Arc::new(ramp_track(24_000.0, 24_000))));
        song.length.set(1.0);
        song.volume.set(1.0);
        let mut player = SongPlayer::new(&song, 48_000.0, 96_000.0);
        assert_eq!(song.clock_rate.get(), 48_000.0);

        // Paused: nothing is added, the clock stands still.
        let mut out = vec![0.0f32; 2 * 960];
        player.render(&song, &mut out, 2);
        assert!(out.iter().all(|&s| s == 0.0));
        assert_eq!(song.position.load(Ordering::Relaxed), 0);

        song.playing.set(true);
        player.render(&song, &mut out, 2);
        // 960 output frames at 96 kHz = 10 ms = 480 input samples.
        assert_eq!(song.position.load(Ordering::Relaxed), 480);
        assert!((song.seconds() - 0.01).abs() < 1e-9);
        // Frame n plays track frame n / 4 (24 kHz vs 96 kHz).
        assert_eq!((out[2 * 400], out[2 * 400 + 1]), (100.0, -100.0));

        // Mono output gets the average of both sides.
        let mut mono = vec![0.25f32; 4];
        player.render(&song, &mut mono, 1);
        assert_eq!(mono[0], 0.25);

        for _ in 0..200 {
            player.render(&song, &mut out, 2);
        }
        assert!(!song.playing.get());
        let end = song.position.load(Ordering::Relaxed);
        assert!((48_000..48_001).contains(&end), "{end}");

        song.rewind.set(true);
        player.render(&song, &mut out, 2);
        assert_eq!(song.position.load(Ordering::Relaxed), 0);
        assert!(!song.rewind.get());
    }

    #[test]
    fn restarting_audio_at_another_rate_keeps_the_place() {
        let song = SongState::default();
        song.clock_rate.set(48_000.0);
        song.position.store(48_000, Ordering::Relaxed);
        let _player = SongPlayer::new(&song, 44_100.0, 44_100.0);
        assert_eq!(song.position.load(Ordering::Relaxed), 44_100);
        assert!((song.seconds() - 1.0).abs() < 1e-9);
    }
}
