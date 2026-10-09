# SoundTune

A small live voice effects console written in Rust with GTK4. Talk or sing
into your microphone and hear yourself through four effects:

| Effect     | What it does                                              | Controls                     |
|------------|-----------------------------------------------------------|------------------------------|
| Autotune   | Detects your pitch (YIN) and snaps it to a scale          | Key, scale, retune speed     |
| High Pitch | Shifts your voice by -12 to +12 semitones                 | Shift                        |
| Distortion | tanh overdrive with a tone filter                         | Drive, tone, mix             |
| Reverb     | Freeverb style room                                       | Room size, damping, mix      |

Autotune and High Pitch combine: with both on, the shifted voice is still
snapped to the chosen scale.

## Interface

* **Header**: power button (start/stop audio), preset menu (Chipmunk, Robot
  Tune, Pop Star, Megaphone, Cathedral), audio device settings (gear).
* **Top strip**: key, secondary knobs (tone, mixes, damping) and volume.
* **Centre**: big knobs, each with its own power button, around a tuner that
  shows the target note and how many cents you are off.
* **Scale panel and keyboard**: highlighted notes are the ones autotune snaps
  to. Use Major / Minor / Pentatonic / All, or click note buttons or piano
  keys to build a custom scale. The note you are singing lights up in blue.

Knobs: drag up/down or scroll to change, double click to reset.

**Use headphones**, otherwise the speakers feed back into the microphone.

## Layout

```
crates/dsp   pure Rust DSP (no dependencies, unit tested)
crates/app   GTK4 UI (custom drawn knobs, tuner, keyboard) + audio I/O via cpal
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
correction of a flat note, reverb decay and output bounds.

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
  pitch shifter, small safety buffer).
* The pitch shifter is pitch synchronous: it uses the detected period to
  align its grains, which avoids the warbly "beating" of a naive delay line
  shifter.

## License

MIT, see [LICENSE](LICENSE).
