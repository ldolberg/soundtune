//! Microphone -> effects -> speakers, using cpal (ALSA/PipeWire on Linux,
//! WASAPI on Windows).

use std::sync::Arc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample, StreamConfig, SupportedStreamConfig};
use rtrb::{Consumer, Producer, RingBuffer};
use soundtune_dsp::{Meters, Params, Processor, SongPlayer};

/// Seconds of processed audio kept between input and output before we start
/// dropping samples to catch up.
const TARGET_LATENCY: f64 = 0.04;
const MAX_LATENCY: f64 = 0.12;

/// Running audio streams. Dropping it stops the audio.
pub struct Engine {
    _input: cpal::Stream,
    _output: cpal::Stream,
    pub description: String,
}

pub fn input_device_names() -> Vec<String> {
    cpal::default_host()
        .input_devices()
        .map(|devs| devs.filter_map(|d| d.name().ok()).collect())
        .unwrap_or_default()
}

pub fn output_device_names() -> Vec<String> {
    cpal::default_host()
        .output_devices()
        .map(|devs| devs.filter_map(|d| d.name().ok()).collect())
        .unwrap_or_default()
}

fn find_device(
    devices: Result<impl Iterator<Item = cpal::Device>, cpal::DevicesError>,
    name: &str,
) -> Result<cpal::Device, String> {
    devices
        .map_err(|e| e.to_string())?
        .find(|d| d.name().map(|n| n == name).unwrap_or(false))
        .ok_or_else(|| format!("device \"{name}\" not found"))
}

impl Engine {
    /// Starts processing. `None` selects the system default device.
    pub fn start(
        input: Option<&str>,
        output: Option<&str>,
        params: Arc<Params>,
        meters: Arc<Meters>,
    ) -> Result<Engine, String> {
        let host = cpal::default_host();
        let in_dev = match input {
            Some(name) => find_device(host.input_devices(), name)?,
            None => host.default_input_device().ok_or("no input device found")?,
        };
        let out_dev = match output {
            Some(name) => find_device(host.output_devices(), name)?,
            None => host
                .default_output_device()
                .ok_or("no output device found")?,
        };

        let in_cfg = in_dev.default_input_config().map_err(|e| e.to_string())?;
        let in_rate = in_cfg.sample_rate();
        let out_cfg = pick_output_config(&out_dev, in_rate)?;
        let out_rate = out_cfg.sample_rate();

        let rate = in_rate.0 as f64;
        let (producer, consumer) = RingBuffer::<f32>::new(rate as usize);
        let player = SongPlayer::new(&params.song, rate as f32, out_rate.0 as f32);
        let link = OutputLink {
            params: Arc::clone(&params),
            player,
            mix: Vec::with_capacity(1 << 16),
            consumer,
            step: rate / out_rate.0 as f64,
            target: (rate * TARGET_LATENCY) as usize,
            max: (rate * MAX_LATENCY) as usize,
            pos: 0.0,
            prev: 0.0,
            cur: 0.0,
        };

        let processor = Processor::new(rate as f32, params, meters);
        let input_stream = build_input(&in_dev, &in_cfg, processor, producer)?;
        let output_stream = build_output(&out_dev, &out_cfg, link)?;
        input_stream.play().map_err(|e| e.to_string())?;
        output_stream.play().map_err(|e| e.to_string())?;

        let description = format!(
            "{} ({} Hz) -> {} ({} Hz)",
            in_dev.name().unwrap_or_else(|_| "input".into()),
            in_rate.0,
            out_dev.name().unwrap_or_else(|_| "output".into()),
            out_rate.0,
        );
        Ok(Engine {
            _input: input_stream,
            _output: output_stream,
            description,
        })
    }
}

/// Prefers an output config at the input's sample rate so no resampling is needed.
fn pick_output_config(
    dev: &cpal::Device,
    rate: cpal::SampleRate,
) -> Result<SupportedStreamConfig, String> {
    let default = dev.default_output_config().map_err(|e| e.to_string())?;
    if default.sample_rate() == rate {
        return Ok(default);
    }
    if let Ok(mut configs) = dev.supported_output_configs() {
        if let Some(c) = configs.find(|c| {
            c.channels() == default.channels()
                && c.sample_format() == default.sample_format()
                && c.min_sample_rate() <= rate
                && rate <= c.max_sample_rate()
        }) {
            return Ok(c.with_sample_rate(rate));
        }
    }
    Ok(default)
}

macro_rules! dispatch_format {
    ($format:expr, $build:ident, $($args:expr),*) => {
        match $format {
            SampleFormat::F32 => $build::<f32>($($args),*),
            SampleFormat::F64 => $build::<f64>($($args),*),
            SampleFormat::I16 => $build::<i16>($($args),*),
            SampleFormat::I32 => $build::<i32>($($args),*),
            SampleFormat::U16 => $build::<u16>($($args),*),
            SampleFormat::U8 => $build::<u8>($($args),*),
            other => Err(format!("unsupported sample format {other}")),
        }
    };
}

fn build_input(
    dev: &cpal::Device,
    cfg: &SupportedStreamConfig,
    processor: Processor,
    producer: Producer<f32>,
) -> Result<cpal::Stream, String> {
    let config = cfg.config();
    dispatch_format!(
        cfg.sample_format(),
        input_stream,
        dev,
        &config,
        processor,
        producer
    )
}

fn input_stream<T>(
    dev: &cpal::Device,
    config: &StreamConfig,
    mut processor: Processor,
    mut producer: Producer<f32>,
) -> Result<cpal::Stream, String>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let channels = config.channels.max(1) as usize;
    let mut mono = Vec::with_capacity(16_384);
    dev.build_input_stream(
        config,
        move |data: &[T], _| {
            mono.clear();
            for frame in data.chunks(channels) {
                let sum: f32 = frame.iter().map(|&s| f32::from_sample(s)).sum();
                mono.push(sum / channels as f32);
            }
            processor.process(&mut mono);
            for &s in &mono {
                if producer.push(s).is_err() {
                    break;
                }
            }
        },
        |err| eprintln!("input stream error: {err}"),
        None,
    )
    .map_err(|e| e.to_string())
}

/// Reads processed audio from the ring buffer, keeps latency bounded and
/// linearly resamples when input and output rates differ. Mixes in the
/// backing track.
struct OutputLink {
    params: Arc<Params>,
    player: SongPlayer,
    /// Output block being built, preallocated.
    mix: Vec<f32>,
    consumer: Consumer<f32>,
    step: f64,
    target: usize,
    max: usize,
    pos: f64,
    prev: f32,
    cur: f32,
}

impl OutputLink {
    fn catch_up(&mut self) {
        let backlog = self.consumer.slots();
        if backlog > self.max {
            if let Ok(chunk) = self.consumer.read_chunk(backlog - self.target) {
                chunk.commit_all();
            }
        }
    }

    #[inline]
    fn next(&mut self) -> f32 {
        while self.pos >= 1.0 {
            self.prev = self.cur;
            // On underrun fade towards silence instead of clicking.
            self.cur = self.consumer.pop().unwrap_or(self.cur * 0.9);
            self.pos -= 1.0;
        }
        let y = self.prev + (self.cur - self.prev) * self.pos as f32;
        self.pos += self.step;
        y
    }
}

fn build_output(
    dev: &cpal::Device,
    cfg: &SupportedStreamConfig,
    link: OutputLink,
) -> Result<cpal::Stream, String> {
    let config = cfg.config();
    dispatch_format!(cfg.sample_format(), output_stream, dev, &config, link)
}

fn output_stream<T>(
    dev: &cpal::Device,
    config: &StreamConfig,
    mut link: OutputLink,
) -> Result<cpal::Stream, String>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = config.channels.max(1) as usize;
    dev.build_output_stream(
        config,
        move |data: &mut [T], _| {
            link.catch_up();
            // Voice first, then the backing track on top. `mix` only grows
            // if the device asks for a bigger buffer than ever before.
            let mut mix = std::mem::take(&mut link.mix);
            mix.clear();
            mix.resize(data.len(), 0.0);
            for frame in mix.chunks_mut(channels) {
                let v = link.next();
                frame.iter_mut().for_each(|s| *s = v);
            }
            link.player.render(&link.params.song, &mut mix, channels);
            for (d, &s) in data.iter_mut().zip(&mix) {
                *d = T::from_sample(s.clamp(-1.0, 1.0));
            }
            link.mix = mix;
        },
        |err| eprintln!("output stream error: {err}"),
        None,
    )
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use soundtune_dsp::{GuideMelody, GuideNote, Track, NO_NOTE};
    use std::sync::atomic::Ordering;
    use std::time::{Duration, Instant};

    /// Needs real audio devices, so it is ignored by default:
    /// cargo test -p soundtune -- --ignored
    /// Everything is silent: master and track volume are 0.
    #[test]
    #[ignore]
    fn engine_plays_and_follows_a_song_silently() {
        let params = Arc::new(Params::default());
        let meters = Arc::new(Meters::default());
        params.master.set(0.0);
        params.tune_on.set(true);
        let song = &params.song;
        song.volume.set(0.0);
        song.track.store(Some(Arc::new(Track {
            rate: 44_100.0,
            channels: 2,
            samples: vec![0.0; 2 * 44_100 * 2],
        })));
        // Guide: A4 for the first second, C5 for the second.
        song.guide.store(Some(Arc::new(GuideMelody {
            rate: 1000.0,
            notes: vec![
                GuideNote {
                    start: 0,
                    end: 1000,
                    note: 69,
                },
                GuideNote {
                    start: 1000,
                    end: 2000,
                    note: 72,
                },
            ],
        })));
        song.length.set(2.0);
        song.playing.set(true);

        let engine = Engine::start(None, None, Arc::clone(&params), Arc::clone(&meters))
            .expect("audio devices");
        let start = Instant::now();
        let mut seen = Vec::new();
        while song.playing.get() && start.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(50));
            seen.push(meters.guide_note.load(Ordering::Relaxed));
        }
        let took = start.elapsed().as_secs_f32();
        // Let the processor see the end of the song.
        std::thread::sleep(Duration::from_millis(100));
        drop(engine);
        println!("played 2 s in {took:.2} s");
        assert!(!song.playing.get(), "the song should end");
        assert!((1.8..3.0).contains(&took), "{took}");
        assert!((song.seconds() - 2.0).abs() < 0.05, "{}", song.seconds());
        assert!(seen.contains(&69) && seen.contains(&72), "{seen:?}");
        assert_eq!(meters.guide_note.load(Ordering::Relaxed), NO_NOTE);
    }
}
