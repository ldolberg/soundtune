//! Chromagram: how much of each pitch class (C, C#, ... B) sounds in each
//! short frame of a recording. Runs offline (it allocates), not in the
//! audio callback.

use std::f32::consts::PI;

use rustfft::num_complex::Complex;
use rustfft::FftPlanner;

use crate::hz_to_midi;

/// Lowest and highest frequencies that count towards the chroma.
pub const CHROMA_LOW_HZ: f32 = 60.0;
pub const CHROMA_HIGH_HZ: f32 = 5000.0;
/// Frames quieter than this (RMS) count as silence.
const SILENCE_RMS: f32 = 1e-3;

/// Pitch class energy per frame.
#[derive(Debug, Clone)]
pub struct Chromagram {
    pub rate: f32,
    /// Samples per frame: frame `i` covers `i * hop .. (i + 1) * hop`.
    pub hop: usize,
    /// One vector per frame (bin 0 = C), scaled so the largest bin is 1.
    /// Silent frames are all zero.
    pub frames: Vec<[f32; 12]>,
    /// RMS level of each frame.
    pub energy: Vec<f32>,
}

/// FFT size for `rate`: about 0.3 s, enough to tell semitones apart at
/// 60 Hz.
fn window_size(rate: f32) -> usize {
    ((rate * 0.3) as usize).next_power_of_two().max(1024)
}

/// Computes the chromagram of a mono signal with one frame every
/// `hop_secs` seconds. Each frame's spectrum comes from a Hann window of
/// about 0.3 s centred on the frame. FFT bins from 60 Hz to 5 kHz are
/// added to the nearest pitch class, weighted down the further they are
/// from the semitone centre.
pub fn chromagram(samples: &[f32], rate: f32, hop_secs: f32) -> Chromagram {
    let hop = ((rate * hop_secs).round() as usize).max(1);
    let size = window_size(rate);
    let window: Vec<f32> = (0..size)
        .map(|n| 0.5 - 0.5 * (2.0 * PI * n as f32 / size as f32).cos())
        .collect();
    let bins: Vec<(usize, usize, f32)> = (1..size / 2)
        .filter_map(|k| {
            let hz = k as f32 * rate / size as f32;
            if !(CHROMA_LOW_HZ..=CHROMA_HIGH_HZ).contains(&hz) {
                return None;
            }
            let midi = hz_to_midi(hz);
            let nearest = midi.round();
            let weight = (PI * (midi - nearest)).cos().powi(2);
            Some((k, (nearest as i32).rem_euclid(12) as usize, weight))
        })
        .collect();

    let fft = FftPlanner::<f32>::new().plan_fft_forward(size);
    let mut buf = vec![Complex::new(0.0, 0.0); size];
    let mut scratch = vec![Complex::new(0.0, 0.0); fft.get_inplace_scratch_len()];
    let count = samples.len().div_ceil(hop);
    let mut frames = Vec::with_capacity(count);
    let mut energy = Vec::with_capacity(count);
    for i in 0..count {
        let own = &samples[i * hop..((i + 1) * hop).min(samples.len())];
        let rms = (own.iter().map(|x| x * x).sum::<f32>() / own.len() as f32).sqrt();
        energy.push(rms);
        if rms < SILENCE_RMS {
            frames.push([0.0; 12]);
            continue;
        }

        let start = (i * hop + hop / 2) as isize - (size / 2) as isize;
        for (j, c) in buf.iter_mut().enumerate() {
            let idx = start + j as isize;
            let x = if idx >= 0 && (idx as usize) < samples.len() {
                samples[idx as usize]
            } else {
                0.0
            };
            *c = Complex::new(x * window[j], 0.0);
        }
        fft.process_with_scratch(&mut buf, &mut scratch);

        let mut chroma = [0.0f32; 12];
        for &(k, pc, w) in &bins {
            chroma[pc] += w * buf[k].norm();
        }
        let max = chroma.iter().cloned().fold(0.0, f32::max);
        if max > 0.0 {
            chroma.iter_mut().for_each(|c| *c /= max);
        }
        frames.push(chroma);
    }
    Chromagram {
        rate,
        hop,
        frames,
        energy,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Notes (MIDI numbers) played together with a few harmonics, like a
    /// piano or guitar chord.
    pub fn chord(notes: &[f32], rate: f32, secs: f32) -> Vec<f32> {
        let len = (rate * secs) as usize;
        (0..len)
            .map(|n| {
                let t = n as f32 / rate;
                notes
                    .iter()
                    .map(|&m| {
                        let hz = 440.0 * ((m - 69.0) / 12.0).exp2();
                        (1..=4)
                            .map(|k| 0.1 / k as f32 * (2.0 * PI * hz * k as f32 * t).sin())
                            .sum::<f32>()
                    })
                    .sum()
            })
            .collect()
    }

    fn top3(c: &[f32; 12]) -> Vec<usize> {
        let mut idx: Vec<usize> = (0..12).collect();
        idx.sort_by(|&a, &b| c[b].total_cmp(&c[a]));
        let mut top = idx[..3].to_vec();
        top.sort();
        top
    }

    #[test]
    fn c_major_triad_lights_up_c_e_g() {
        let rate = 44_100.0;
        // C3 E3 G3 C4 E4 G4, low enough to test the bass resolution too.
        let audio = chord(&[48.0, 52.0, 55.0, 60.0, 64.0, 67.0], rate, 2.0);
        let chroma = chromagram(&audio, rate, 0.25);
        assert_eq!(chroma.frames.len(), 8);
        for frame in &chroma.frames[1..7] {
            assert_eq!(top3(frame), vec![0, 4, 7], "{frame:?}");
            // Everything else is well below the chord tones.
            for pc in [1, 3, 6, 8, 10] {
                assert!(frame[pc] < 0.3, "pc {pc}: {frame:?}");
            }
        }
    }

    #[test]
    fn a_minor_and_silence() {
        let rate = 48_000.0;
        let mut audio = chord(&[57.0, 60.0, 64.0], rate, 1.0);
        audio.resize(audio.len() + 24_000, 0.0);
        let chroma = chromagram(&audio, rate, 0.5);
        assert_eq!(chroma.hop, 24_000);
        assert_eq!(top3(&chroma.frames[0]), vec![0, 4, 9]);
        assert!(chroma.frames[2].iter().all(|&c| c == 0.0));
        assert!(chroma.energy[0] > 0.05 && chroma.energy[2] == 0.0);
    }
}
