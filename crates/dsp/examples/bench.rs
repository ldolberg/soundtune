//! Measures the CPU cost of the full chain on 10 s of synthetic singing.
//!
//! cargo run --release -p soundtune-dsp --example bench

use std::f32::consts::PI;
use std::sync::Arc;
use std::time::Instant;

use soundtune_dsp::{apply_sing_mode, Meters, Params, Processor, Voice};

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
            (1..=6).map(|k| 0.2 / k as f32 * (k as f32 * phase).sin()).sum::<f32>() + 0.002 * hiss
        })
        .collect();

    let mut report = |name: &str, setup: &dyn Fn(&Params)| {
        let params = Arc::new(Params::default());
        setup(&params);
        let mut proc = Processor::new(sr, params, Arc::new(Meters::default()));
        let mut buf = input.clone();
        let start = Instant::now();
        for chunk in buf.chunks_mut(256) {
            proc.process(chunk);
        }
        let used = start.elapsed().as_secs_f32();
        println!("{name:<28} {:>5.2}% of one core", 100.0 * used / secs);
    };

    report("bypass", &|_| {});
    report("Pop Princess (polish 100%)", &|p| apply_sing_mode(p, Voice::PopPrincess, 1.0));
    report("everything on", &|p| {
        apply_sing_mode(p, Voice::PopPrincess, 1.0);
        p.pitch_on.set(true);
        p.dist_on.set(true);
        p.auto_key.set(true);
    });
}
