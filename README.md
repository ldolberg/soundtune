# SoundTune

A small live voice effects console written in Rust with GTK4. Talk or sing
into your microphone and hear yourself through a full pop vocal chain. Even
if you sing out of tune, one button gets you a tight, glossy, early 2000s
pop sound: snapped pitch, bright and breathy tone, doubled vocals and a lush
reverb.

| Effect      | What it does                                                   | Controls                                   |
|-------------|----------------------------------------------------------------|--------------------------------------------|
| Gate        | Mutes breaths and room noise between phrases                   | Threshold                                  |
| Autotune    | Detects your pitch (YIN) and snaps it to a scale               | Key, scale, auto key, retune speed, correction amount |
| High Pitch  | Shifts your voice by -12 to +12 semitones                      | Shift                                      |
| Compressor  | Evens out quiet and loud singing, automatic makeup gain        | Threshold, ratio                           |
| Brightness  | 90 Hz low cut, 3.2 kHz presence boost and 10 kHz "air" shelf   | Amount                                     |
| Doubler     | Two copies detuned up and down by a few cents, delayed 18/29 ms | Mix, detune                                |
| Distortion  | tanh overdrive with a tone filter                              | Drive, tone, mix                           |
| Reverb      | Freeverb style room                                            | Room size, damping, mix                    |

Signal flow: gate, autotune / high pitch, compressor, brightness, doubler,
distortion, reverb, volume, feedback guard, limiter.

Autotune and High Pitch combine: with both on, the shifted voice is still
snapped to the chosen scale. **Correction** sets how far notes are pulled
onto the scale (100% = exactly on the note, 50% = halfway), **retune speed**
how fast.

**Auto key** listens to what you sing (a pitch class histogram of the last
~12 seconds, matched against Krumhansl major and minor key profiles) and
makes the autotune follow the detected key. Until it has heard enough (about
1.5 seconds of singing), it snaps to the nearest semitone.

### Sing along with a song

Open a backing track (MP3, WAV, FLAC or Ogg Vorbis) and SoundTune plays it
with your voice and keeps you on key with it. Optionally also open the
song's melody as a MIDI file.

* **Analysis** (on a background thread, a fraction of a second for a 3
  minute song): a chromagram every 0.25 s (FFT of a 0.3 s window, 60 Hz to
  5 kHz folded onto the 12 pitch classes), the song key (Krumhansl-Schmuckler
  over the whole song; a key and its relative, like A minor and C major, are
  told apart by which tonic chord sounds longer and opens or closes the
  song) and the chords (major and minor triad templates, smoothed with a
  Viterbi pass so they do not flicker). The result is a timeline of allowed
  notes: the key's scale plus the current chord tones.
* **MIDI melody**: tempo changes are followed, every track and channel is a
  part you can pick, and by default the part that looks most like a vocal
  line is used (not drums, mostly one note at a time, most notes in a
  singable range). Chords in it are reduced to their top note.
* **Follow song** (on by default): at the current song position the
  autotune targets, in this order:
  1. the melody note, if a MIDI melody is loaded and a note is sounding,
     in the octave nearest to your voice (so low and high voices both
     work), plus the melody **Shift** in semitones;
  2. otherwise the key's notes, with chord tones preferred (a note sung
     between a chord tone and a passing note lands on the chord tone);
  3. otherwise (past the end, or no song) the scale set by hand or by
     Auto key.

  Paused, the autotune keeps following the paused position. The melody
  **Offset** (ms) moves the melody later or earlier if it does not line up
  with the recording.
* **Playback**: the track is mixed into the output with its own volume (the
  Volume knob only affects your voice), resampled on the fly to the device
  rate. Its clock drives the song position the autotune reads.

### Safety

* A peak limiter (about -1 dBFS) always runs on the output.
* The **howl guard** watches for acoustic feedback: a loud, sustained,
  almost pure tone (regular zero crossings and no harmonics). After about
  0.8 s of howling it turns the output down by 20 dB for 3 seconds and the
  window shows a red warning. It can be switched off in Advanced, Safety.

## Interface

The window opens in **Sing** mode, made for people who just want to sing:

* **MAKE ME A POP STAR**: one big switch that turns on the whole chain (and
  starts the audio if it is stopped). Off means your plain voice.
* **Polish**: one knob that drives everything at once: faster, tighter
  retuning, harder gate and compression, more brightness, more doubling and
  more reverb.
* **Voice**: Natural (gentle, transparent), Pop Princess (glossy early 2000s
  pop), Robot (instant hard tuning with a metallic double) or Chipmunk.
* **Pitch correction** slider: from off to exactly on the note.
* **Auto key** switch and the detected key, plus a tuner showing the note
  you are singing.
* **Song bar**: Open song, Open melody (MIDI), back to start, play/pause
  (starts the audio if needed), stop, time and progress, the track volume
  and the **Follow song** switch. The line below shows the file, the song's
  key, the current chord and melody note; with a melody loaded, the melody
  part, Offset and Shift.

The **Advanced** tab is the full console:

* **Header**: power button (start/stop audio), preset menu (Chipmunk, Robot
  Tune, Pop Princess, Pop Star, Radio Voice, Megaphone, Cathedral), audio
  device settings (gear).
* **Top strip**: key, auto key, correction amount, secondary knobs (tone,
  mixes, damping) and volume.
* **Vocal chain strip**: gate, compressor, brightness and doubler, each with
  a power button, a compressor gain reduction meter and the howl guard.
* **Centre**: big knobs, each with its own power button, around a tuner that
  shows the target note and how many cents you are off.
* **Scale panel and keyboard**: highlighted notes are the ones autotune snaps
  to. Use Major / Minor / Pentatonic / All, or click note buttons or piano
  keys to build a custom scale. The note you are singing lights up in blue.
  While following a song they show the notes the song allows right now
  instead (chord tones dotted, the melody note in pink); your own scale is
  kept for when following stops.

Knobs: drag up/down or scroll to change, double click to reset.

Sing mode and the Advanced controls share the same settings: Sing mode sets
the advanced knobs, and you can fine tune from there.

**Use headphones**, otherwise the speakers feed back into the microphone.

Files can also be given on the command line:

```sh
soundtune [--advanced] [song.mp3] [melody.mid]
```

## Layout

```
crates/dsp   pure Rust DSP and song analysis (rustfft, midly, arc-swap; unit tested)
crates/app   GTK4 UI (custom drawn knobs, tuner, keyboard), audio I/O via cpal,
             song decoding via symphonia
packaging/   Windows bundle/installer scripts, Linux .desktop file
```

## Requirements

A recent stable Rust (install with [rustup](https://rustup.rs)); distro
packaged Rust 1.75 is too old for the gtk4-rs dependencies.

## Linux

```sh
sudo apt install libgtk-4-dev libasound2-dev build-essential   # Debian/Ubuntu
# Fedora: sudo dnf install gtk4-devel alsa-lib-devel
cargo run --release -p soundtune
```

Audio goes through ALSA, which on modern distros is routed to PipeWire or
PulseAudio. "System default" is usually the right device.

Install for the current user:

```sh
cargo install --path crates/app
cp packaging/linux/io.github.soundtune.SoundTune.desktop ~/.local/share/applications/
```

## Windows

GTK4 apps on Windows are built with MSYS2 and shipped together with the GTK
runtime DLLs.

1. Install [MSYS2](https://www.msys2.org/) and open the **MSYS2 UCRT64** shell.
2. From the repository root run:

   ```sh
   bash packaging/windows/package.sh
   ```

   This installs the toolchain and GTK4, builds the release exe and produces
   `dist/SoundTune/` plus `dist/SoundTune-windows-x64.zip`: a portable folder,
   run `SoundTune.bat` or `bin\soundtune.exe`.
3. Optional installer: with [Inno Setup 6](https://jrsoftware.org/isinfo.php)
   installed, run `iscc packaging\windows\soundtune.iss` to get
   `dist\SoundTune-setup-x64.exe`.

The GitHub Actions workflow (`.github/workflows/build.yml`) does all of this
automatically and uploads the zip and installer as build artifacts, so you can
also get Windows builds without a Windows machine.

## Tests

```sh
cargo test -p soundtune-dsp
```

The DSP tests check pitch detection accuracy, octave shifting, autotune
correction of a flat note (fully and part way), gate and compressor
behaviour, the brightness EQ response, doubler detune and delay, key
detection, feedback detection without false alarms on loud singing, the
Sing mode settings, the full Pop Princess chain, reverb decay and output
bounds. The song tests check the chromagram of synthetic chords (a C major
triad lights up C, E and G), key and chord detection on synthetic
progressions (including a relative key and a chord outside the key), MIDI
parsing of files built in the test (tempo changes, timecode timing,
choosing the melody part, reducing chords), guide lookup and octave choice,
backing track playback and resampling, and follow mode in the processor (a
430 Hz note with an A4 melody note comes out at 440 Hz; offset, shift,
chord fallback and the static scale fallback).

`cargo test -p soundtune` also decodes WAV files written by the test
(stereo and mono) and analyses them. One more test runs the real audio
engine silently (volumes at 0) through a short song; it needs audio
devices, so it is ignored by default:

```sh
cargo test -p soundtune -- --ignored
```

CPU benchmark of the chain on 10 s of synthetic singing:

```sh
cargo run --release -p soundtune-dsp --example bench
```

Everything on costs about 2% of one core; following a song adds nothing
measurable (one timeline lookup per audio block). Analysing a 3 minute song
takes about 0.05 s.

## Development

CI (`.github/workflows/build.yml`) runs these on every push and pull request,
so run them locally before pushing:

```sh
cargo fmt --all --check                               # formatting
cargo clippy --workspace --all-targets -- -D warnings # lints, warnings fail
cargo test --workspace                                # unit tests
```

`cargo fmt --all` fixes formatting in place. The app crate needs the GTK4
and ALSA development packages listed above even for clippy.

### Cutting a release

1. Bump `version` in `crates/app/Cargo.toml` (and `crates/dsp/Cargo.toml` if
   it changed), commit and push to `main`, and wait for CI to pass.
2. Tag the commit and push the tag:

   ```sh
   git tag v0.2.0
   git push origin v0.2.0
   ```

The tag build runs the same jobs and then creates a GitHub Release with
`soundtune-linux-x64.tar.gz`, `SoundTune-windows-x64.zip` and
`SoundTune-setup-x64.exe` attached. The installer version is taken from the
tag name.

## Notes

* Latency is roughly the audio buffer size plus ~40 ms (30 ms grain for the
  pitch shifter, small safety buffer). The doubler's copies come 18 and 29 ms
  after the main voice on purpose.
* All DSP is allocation and lock free in the audio callback; the UI talks to
  it through atomics.
* Songs are decoded and analysed on a loader thread. The decoded track and
  the timelines are immutable and handed to the audio callbacks through
  `arc-swap` pointers (no mutex); replaced ones are freed a few seconds later
  on the UI thread, never in a callback. The song clock is an atomic sample
  counter (input rate) advanced by the output callback.
* The chord analysis knows major and minor triads only; sevenths, sus and
  power chords map to the nearest triad or to "no chord" (then the key's
  scale alone applies). The MIDI melody has to match the recording's
  timing; use Offset for a constant shift.
* The pitch shifter is pitch synchronous: it uses the detected period to
  align its grains, which avoids the warbly "beating" of a naive delay line
  shifter.

## License

MIT, see [LICENSE](LICENSE).
