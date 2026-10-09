//! Loading songs and MIDI melodies. Runs on a loader thread, never in the
//! GTK main loop or an audio callback.

use std::fs::File;
use std::path::Path;

use soundtune_dsp::{analyse_harmony, parse_midi, Harmony, MidiSong, Track};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// File name patterns offered in the "Open song" dialog.
pub const AUDIO_PATTERNS: [&str; 5] = ["*.mp3", "*.wav", "*.flac", "*.ogg", "*.oga"];
pub const MIDI_PATTERNS: [&str; 2] = ["*.mid", "*.midi"];

/// A decoded file: the track for playback (mono or stereo) and a mono
/// mix for analysis, both at the file's sample rate.
pub struct Decoded {
    pub track: Track,
    pub mono: Vec<f32>,
}

/// Decodes an MP3, WAV, FLAC or Ogg Vorbis file. More than two channels
/// are folded into left and right.
pub fn decode(path: &Path) -> Result<Decoded, String> {
    let file = File::open(path).map_err(|e| format!("cannot open the file: {e}"))?;
    let stream = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            stream,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| format!("unsupported file: {e}"))?;
    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or("the file has no audio")?;
    let track_id = track.id;
    let mut rate = track.codec_params.sample_rate.unwrap_or(0);
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| format!("unsupported codec: {e}"))?;

    let mut stereo: Vec<f32> = Vec::new();
    let mut mono: Vec<f32> = Vec::new();
    let mut channels = 0usize;
    let mut buf: Option<SampleBuffer<f32>> = None;
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(Error::ResetRequired) => break,
            Err(e) => return Err(format!("read error: {e}")),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            // A damaged frame: skip it, like players do.
            Err(Error::DecodeError(_)) => continue,
            Err(e) => return Err(format!("decode error: {e}")),
        };
        let spec = *decoded.spec();
        rate = spec.rate;
        let n = spec.channels.count().max(1);
        channels = channels.max(n.min(2));
        let sb = match &mut buf {
            Some(b) if b.capacity() >= decoded.capacity() * n => b,
            _ => buf.insert(SampleBuffer::new(decoded.capacity() as u64, spec)),
        };
        sb.copy_interleaved_ref(decoded);
        for frame in sb.samples().chunks(n) {
            let (l, r) = match frame {
                [m] => (*m, *m),
                [l, r] => (*l, *r),
                _ => {
                    // Surround: fronts plus the rest shared by both sides.
                    let rest: f32 = frame[2..].iter().sum::<f32>() / (n - 1) as f32;
                    (frame[0] + rest, frame[1] + rest)
                }
            };
            stereo.push(l);
            stereo.push(r);
            mono.push(0.5 * (l + r));
        }
    }
    if mono.is_empty() || rate == 0 {
        return Err("the file has no audio".into());
    }
    let samples = if channels >= 2 {
        stereo
    } else {
        drop(stereo);
        mono.clone()
    };
    Ok(Decoded {
        track: Track {
            rate: rate as f32,
            channels: channels.clamp(1, 2),
            samples,
        },
        mono,
    })
}

/// A decoded and analysed song, ready to play and follow.
pub struct LoadedSong {
    pub name: String,
    pub track: Track,
    pub harmony: Harmony,
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Decodes `path` and analyses its key and chords.
pub fn load_song(path: &Path) -> Result<LoadedSong, String> {
    let decoded = decode(path)?;
    let harmony = analyse_harmony(&decoded.mono, decoded.track.rate);
    Ok(LoadedSong {
        name: file_name(path),
        track: decoded.track,
        harmony,
    })
}

/// A parsed MIDI file.
pub struct LoadedMidi {
    pub name: String,
    pub song: MidiSong,
}

pub fn load_midi(path: &Path) -> Result<LoadedMidi, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("cannot open the file: {e}"))?;
    Ok(LoadedMidi {
        name: file_name(path),
        song: parse_midi(&bytes)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    fn temp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("soundtune-test-{}-{name}", std::process::id()))
    }

    #[test]
    fn wav_roundtrip_stereo_and_analysis() {
        let path = temp_path("chord.wav");
        let rate = 22_050;
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&path, spec).unwrap();
        let frames = rate as usize * 2;
        // Left: an A minor chord, right: silence.
        for n in 0..frames {
            let t = n as f32 / rate as f32;
            let s: f32 = [220.0f32, 261.63, 329.63]
                .iter()
                .map(|hz| 0.2 * (2.0 * PI * hz * t).sin())
                .sum();
            w.write_sample((s * 32_767.0) as i16).unwrap();
            w.write_sample(0i16).unwrap();
        }
        w.finalize().unwrap();

        let song = load_song(&path).unwrap();
        std::fs::remove_file(&path).ok();
        assert_eq!(song.name, path.file_name().unwrap().to_string_lossy());
        let t = &song.track;
        assert_eq!((t.rate, t.channels, t.frames()), (22_050.0, 2, frames));
        assert!((t.duration() - 2.0).abs() < 1e-6);
        // Samples survive the 16 bit round trip.
        let expect: f32 = [220.0f32, 261.63, 329.63]
            .iter()
            .map(|hz| 0.2 * (2.0 * PI * hz * 100.0 / rate as f32).sin())
            .sum();
        assert!((t.samples[200] - expect).abs() < 1e-3);
        assert_eq!(t.samples[201], 0.0);
        // The analysis hears A minor (or its relative, C major).
        let key = song.harmony.key.expect("a key");
        assert_eq!(key.mask(), soundtune_dsp::MusicScale::Minor.mask(9));
        let chord = song.harmony.at(22_050).and_then(|s| s.chord);
        assert_eq!(chord.map(|c| c.name()), Some("Am".into()));
    }

    #[test]
    fn mono_wav_and_errors() {
        let path = temp_path("mono.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 8_000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let mut w = hound::WavWriter::create(&path, spec).unwrap();
        for n in 0..800 {
            w.write_sample(n as f32 / 1000.0).unwrap();
        }
        w.finalize().unwrap();
        let d = decode(&path).unwrap();
        std::fs::remove_file(&path).ok();
        assert_eq!((d.track.channels, d.track.frames()), (1, 800));
        assert_eq!(d.track.samples[400], 0.4);
        assert_eq!(d.mono[400], 0.4);

        let bogus = temp_path("bogus.wav");
        std::fs::write(&bogus, b"definitely not audio").unwrap();
        assert!(decode(&bogus).is_err());
        assert!(load_midi(&bogus).is_err());
        std::fs::remove_file(&bogus).ok();
        assert!(decode(Path::new("/nonexistent/song.mp3")).is_err());
    }
}
