//! Measures the CPU cost of the full chain on 10 s of synthetic singing,
//! with and without following a song, and the cost of analysing a song.
//!
//! cargo run --release -p soundtune-dsp --example bench

use std::f32::consts::PI;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;

use soundtune_dsp::{
    analyse_harmony, apply_sing_mode, Chord, GuideMelody, GuideNote, Harmony, HarmonySegment, Key,
    Meters, Params, Processor, Voice,
};

/// A 3 minute song map at `sr`: a chord every 0.5 s and, in every other
/// 4 s stretch, a melody note every 0.25 s.
fn song_map(sr: f32) -> (Harmony, GuideMelody) {
    let key = Key {
        root: 9,
        minor: true,
    };
    let half = (sr / 2.0) as u64;
    let segments = (0..360)
        .map(|i| HarmonySegment {
            start: i * half,
            scale: key.mask(),
            chord: Some(Chord {
                root: [9, 5, 0, 7][i as usize % 4],
                minor: i % 4 == 0,
            }),
        })
        .collect();
    let quarter = half / 2;
    let notes = (0..720)
        .filter(|i| (i / 16) % 2 == 0)
        .map(|i| GuideNote {
            start: i * quarter,
            end: (i + 1) * quarter,
            note: 57 + (i % 12) as u8,
        })
        .collect();
    let harmony = Harmony {
        rate: sr,
        key: Some(key),
        segments,
        length: 360 * half,
    };
    (harmony, GuideMelody { rate: sr, notes })
}

fn main() {
    let sr = 48_000.0f32;
    let secs = 10.0;
    // A vowel gliding around A3 with vibrato, plus a little noise.
    let mut phase = 0.0f32;
    let mut noise = 1u32;
    let input: Vec<f32> = (0..(sr * secs) as usize)
        .map(|n| {
            let t = n as f32 / sr;
            let hz = 220.0 * (1.0 + 0.1 * (0.3 * t).sin() + 0.01 * (2.0 * PI * 5.5 * t).sin());
            phase += 2.0 * PI * hz / sr;
            noise = noise.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let hiss = (noise >> 9) as f32 / (1u32 << 23) as f32 - 0.5;
            (1..=6)
                .map(|k| 0.2 / k as f32 * (k as f32 * phase).sin())
                .sum::<f32>()
                + 0.002 * hiss
        })
        .collect();

    let report = |name: &str, setup: &dyn Fn(&Params)| {
        let params = Arc::new(Params::default());
        setup(&params);
        let mut proc = Processor::new(sr, Arc::clone(&params), Arc::new(Meters::default()));
        let mut buf = input.clone();
        let start = Instant::now();
        for chunk in buf.chunks_mut(256) {
            proc.process(chunk);
            // What the output callback does while the song plays.
            params
                .song
                .position
                .fetch_add(chunk.len() as u64, Ordering::Relaxed);
        }
        let used = start.elapsed().as_secs_f32();
        println!("{name:<32} {:>5.2}% of one core", 100.0 * used / secs);
    };

    report("bypass", &|_| {});
    report("Pop Princess (polish 100%)", &|p| {
        apply_sing_mode(p, Voice::PopPrincess, 1.0)
    });
    report("everything on", &|p| {
        apply_sing_mode(p, Voice::PopPrincess, 1.0);
        p.pitch_on.set(true);
        p.dist_on.set(true);
        p.auto_key.set(true);
    });
    report("Pop Princess following a song", &|p| {
        apply_sing_mode(p, Voice::PopPrincess, 1.0);
        let (harmony, guide) = song_map(sr);
        p.song.harmony.store(Some(Arc::new(harmony)));
        p.song.guide.store(Some(Arc::new(guide)));
        p.song.follow.set(true);
        p.song.playing.set(true);
    });

    // Offline analysis of a 3 minute song (Am F C G, a chord per 2 s).
    let rate = 44_100.0f32;
    let chords: [&[f32]; 4] = [
        &[45.0, 57.0, 60.0, 64.0],
        &[41.0, 57.0, 60.0, 65.0],
        &[48.0, 55.0, 60.0, 64.0],
        &[43.0, 55.0, 59.0, 62.0],
    ];
    let song: Vec<f32> = (0..(rate * 180.0) as usize)
        .map(|n| {
            let t = n as f32 / rate;
            let notes = chords[(t / 2.0) as usize % 4];
            notes
                .iter()
                .map(|&m| 0.1 * (2.0 * PI * 440.0 * ((m - 69.0) / 12.0).exp2() * t).sin())
                .sum()
        })
        .collect();
    let start = Instant::now();
    let harmony = analyse_harmony(&song, rate);
    println!(
        "analysing a 3 min song: {:.2} s ({} chord changes, key {})",
        start.elapsed().as_secs_f32(),
        harmony.segments.len(),
        harmony.key.map_or("?".into(), |k| k.name()),
    );
}
