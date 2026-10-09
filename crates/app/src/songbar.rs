//! The "Song" bar on the Sing page: open a backing track and a MIDI
//! melody, transport, track volume and the Follow switch.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::Ordering;
use std::sync::{mpsc, Arc};
use std::time::Duration;

use gtk::prelude::*;
use gtk::{gio, glib};
use soundtune_dsp::{note_name, Params, SongTarget};

use crate::song::{self, LoadedMidi, LoadedSong, AUDIO_PATTERNS, MIDI_PATTERNS};
use crate::widgets::Knob;

/// Sample rate guide melodies are built at (they carry their own rate).
const GUIDE_RATE: f32 = 48_000.0;

pub struct SongBar {
    root: gtk::Box,
    params: Arc<Params>,
    power: gtk::ToggleButton,
    open_song: gtk::Button,
    open_midi: gtk::Button,
    play: gtk::ToggleButton,
    time: gtk::Label,
    progress: gtk::ProgressBar,
    info: gtk::Label,
    follow: gtk::Switch,
    melody_box: gtk::Box,
    parts: gtk::DropDown,
    song_name: RefCell<Option<String>>,
    midi: RefCell<Option<LoadedMidi>>,
    /// Text last shown, to avoid needless relayouts.
    shown: RefCell<(String, String)>,
    /// Files being loaded.
    loading: Cell<u32>,
}

fn clock(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

fn icon_button<B: IsA<gtk::Button> + IsA<gtk::Widget>>(b: B, icon: &str, tip: &str) -> B {
    b.set_icon_name(icon);
    b.set_tooltip_text(Some(tip));
    b.add_css_class("transport");
    b.set_valign(gtk::Align::Center);
    b
}

fn file_filter(name: &str, patterns: &[&str]) -> gtk::FileFilter {
    let f = gtk::FileFilter::new();
    f.set_name(Some(name));
    for p in patterns {
        f.add_pattern(p);
        f.add_pattern(&p.to_uppercase());
    }
    f
}

/// Drops replaced song data a little later on the main loop, so an audio
/// callback still holding it never frees it.
fn retire<T: 'static>(old: Option<Arc<T>>) {
    if let Some(old) = old {
        glib::timeout_add_local_once(Duration::from_secs(2), move || drop(old));
    }
}

/// Runs `work` on a loader thread and hands its result to `done` on the
/// main loop.
fn load_in_background<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
    done: impl Fn(T) + 'static,
) {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(work());
    });
    glib::timeout_add_local(Duration::from_millis(50), move || match rx.try_recv() {
        Ok(v) => {
            done(v);
            glib::ControlFlow::Break
        }
        Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
        Err(mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
    });
}

impl SongBar {
    pub fn new(
        window: &gtk::ApplicationWindow,
        params: Arc<Params>,
        power: &gtk::ToggleButton,
    ) -> Rc<SongBar> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
        root.add_css_class("song-bar");
        root.set_halign(gtk::Align::Center);
        root.set_size_request(860, -1);

        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let title = gtk::Label::new(Some("SONG"));
        title.add_css_class("section-title");
        let open_song = gtk::Button::with_label("Open song");
        open_song.add_css_class("set");
        open_song.set_valign(gtk::Align::Center);
        open_song.set_tooltip_text(Some(
            "Load a backing track (MP3, WAV, FLAC or OGG) to sing along with",
        ));
        let open_midi = gtk::Button::with_label("Open melody");
        open_midi.add_css_class("set");
        open_midi.set_valign(gtk::Align::Center);
        open_midi.set_tooltip_text(Some(
            "Load the melody as a MIDI file: the autotune then sings its exact notes",
        ));
        let rewind = icon_button(
            gtk::Button::new(),
            "media-skip-backward-symbolic",
            "Back to the start",
        );
        let play = icon_button(
            gtk::ToggleButton::new(),
            "media-playback-start-symbolic",
            "Play / pause",
        );
        let stop = icon_button(gtk::Button::new(), "media-playback-stop-symbolic", "Stop");
        let time = gtk::Label::new(Some("0:00 / 0:00"));
        time.add_css_class("song-time");
        time.set_width_chars(11);
        let progress = gtk::ProgressBar::new();
        progress.add_css_class("song");
        progress.set_hexpand(true);
        progress.set_valign(gtk::Align::Center);
        let volume = Knob::new(
            "Track",
            0.0,
            1.0,
            params.song.volume.get() as f64,
            44,
            |v| format!("{:.0}%", v * 100.0),
        );
        {
            let p = Arc::clone(&params);
            volume.connect_changed(move |v| p.song.volume.set(v as f32));
        }
        let follow = gtk::Switch::new();
        follow.set_valign(gtk::Align::Center);
        follow.set_active(params.song.follow.get());
        follow.set_tooltip_text(Some(
            "Autotune follows the song: the melody notes if a MIDI melody is loaded, \
             otherwise the song's key and chords",
        ));
        {
            let p = Arc::clone(&params);
            follow.connect_active_notify(move |s| p.song.follow.set(s.is_active()));
        }
        let follow_label = gtk::Label::new(Some("Follow song"));
        follow_label.add_css_class("field-title");

        row.append(&title);
        row.append(&open_song);
        row.append(&open_midi);
        row.append(&rewind);
        row.append(&play);
        row.append(&stop);
        row.append(&time);
        row.append(&progress);
        row.append(volume.widget());
        row.append(&follow);
        row.append(&follow_label);

        let info_row = gtk::Box::new(gtk::Orientation::Horizontal, 14);
        let info = gtk::Label::new(None);
        info.add_css_class("song-info");
        info.set_hexpand(true);
        info.set_xalign(0.0);
        info.set_ellipsize(gtk::pango::EllipsizeMode::End);
        info_row.append(&info);

        // Melody options, shown once a MIDI file is loaded.
        let melody_box = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        melody_box.set_visible(false);
        let parts = gtk::DropDown::from_strings(&[]);
        parts.set_valign(gtk::Align::Center);
        parts.set_tooltip_text(Some("Which MIDI part is the melody"));
        let offset = Knob::new("Offset", -500.0, 500.0, 0.0, 36, |v| format!("{v:+.0} ms"))
            .bipolar()
            .stepped(10.0);
        let shift = Knob::new("Shift", -12.0, 12.0, 0.0, 36, |v| format!("{v:+.0} st"))
            .bipolar()
            .stepped(1.0);
        offset
            .widget()
            .set_tooltip_text(Some("Moves the melody later (+) or earlier (-)"));
        shift
            .widget()
            .set_tooltip_text(Some("Transposes the melody in semitones"));
        {
            let p = Arc::clone(&params);
            offset.connect_changed(move |v| p.song.guide_offset_ms.set(v as f32));
            let p = Arc::clone(&params);
            shift.connect_changed(move |v| p.song.guide_shift.set(v as f32));
        }
        let melody_label = gtk::Label::new(Some("Melody"));
        melody_label.add_css_class("field-title");
        melody_box.append(&melody_label);
        melody_box.append(&parts);
        melody_box.append(offset.widget());
        melody_box.append(shift.widget());
        info_row.append(&melody_box);

        root.append(&row);
        root.append(&info_row);

        let bar = Rc::new(SongBar {
            root,
            params,
            power: power.clone(),
            open_song: open_song.clone(),
            open_midi: open_midi.clone(),
            play: play.clone(),
            time,
            progress,
            info,
            follow,
            melody_box,
            parts: parts.clone(),
            song_name: RefCell::new(None),
            midi: RefCell::new(None),
            shown: RefCell::new(Default::default()),
            loading: Cell::new(0),
        });

        {
            let (bar, window) = (Rc::downgrade(&bar), window.clone());
            open_song.connect_clicked(move |_| {
                let bar = bar.clone();
                let filter = file_filter("Audio (MP3, WAV, FLAC, OGG)", &AUDIO_PATTERNS);
                choose_file(&window, "Open song", filter, move |path| {
                    if let Some(bar) = bar.upgrade() {
                        bar.load_song(path);
                    }
                });
            });
        }
        {
            let (bar, window) = (Rc::downgrade(&bar), window.clone());
            open_midi.connect_clicked(move |_| {
                let bar = bar.clone();
                let filter = file_filter("MIDI melody", &MIDI_PATTERNS);
                choose_file(&window, "Open melody", filter, move |path| {
                    if let Some(bar) = bar.upgrade() {
                        bar.load_midi(path);
                    }
                });
            });
        }
        {
            let bar = Rc::downgrade(&bar);
            play.connect_toggled(move |b| {
                if let Some(bar) = bar.upgrade() {
                    bar.set_playing(b.is_active());
                }
            });
        }
        {
            let bar = Rc::downgrade(&bar);
            stop.connect_clicked(move |_| {
                if let Some(bar) = bar.upgrade() {
                    bar.play.set_active(false);
                    bar.rewind();
                }
            });
        }
        {
            let bar = Rc::downgrade(&bar);
            rewind.connect_clicked(move |_| {
                if let Some(bar) = bar.upgrade() {
                    bar.rewind();
                }
            });
        }
        {
            let bar = Rc::downgrade(&bar);
            parts.connect_selected_notify(move |d| {
                if let Some(bar) = bar.upgrade() {
                    bar.use_part(d.selected() as usize);
                }
            });
        }
        bar.refresh();
        bar
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    fn song(&self) -> &soundtune_dsp::SongState {
        &self.params.song
    }

    /// True when a track or a melody is loaded.
    pub fn loaded(&self) -> bool {
        self.song().track.load().is_some() || self.song().guide.load().is_some()
    }

    /// Following is on and there is something to follow.
    pub fn following(&self) -> bool {
        self.follow.is_active() && self.loaded()
    }

    /// The analysed key of the backing track, like "A minor".
    pub fn key_name(&self) -> Option<String> {
        let h = self.song().harmony.load();
        h.as_deref().and_then(|h| h.key).map(|k| k.name())
    }

    fn rewind(&self) {
        let song = self.song();
        song.rewind.set(true);
        // Shown at once, also when the audio is stopped.
        song.position.store(0, Ordering::Relaxed);
    }

    fn set_playing(&self, on: bool) {
        let song = self.song();
        if on && !self.loaded() {
            self.play.set_active(false);
            self.set_info("Open a song or a melody first.");
            return;
        }
        if on && song.seconds() >= song.length.get() as f64 - 0.05 {
            self.rewind();
        }
        song.playing.set(on);
        self.play.set_icon_name(if on {
            "media-playback-pause-symbolic"
        } else {
            "media-playback-start-symbolic"
        });
        // The track plays through the audio engine: start it.
        if on && !self.power.is_active() {
            self.power.set_active(true);
        }
    }

    fn update_length(&self) {
        let track = self
            .song()
            .track
            .load()
            .as_deref()
            .map_or(0.0, |t| t.duration());
        let midi = self.midi.borrow().as_ref().map_or(0.0, |m| m.song.length());
        self.song().length.set(track.max(midi) as f32);
    }

    fn set_loading(&self, on: bool, what: &str) {
        let n = if on {
            self.loading.get() + 1
        } else {
            self.loading.get().saturating_sub(1)
        };
        self.loading.set(n);
        self.open_song.set_sensitive(n == 0);
        self.open_midi.set_sensitive(n == 0);
        if on {
            self.set_info(&format!("Loading {what}..."));
        }
    }

    fn set_info(&self, text: &str) {
        self.shown.borrow_mut().1 = text.to_string();
        self.info.set_text(text);
    }

    pub fn load_song(self: &Rc<Self>, path: PathBuf) {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
        self.set_loading(true, &name.unwrap_or_default());
        let bar = Rc::downgrade(self);
        load_in_background(
            move || song::load_song(&path),
            move |res: Result<LoadedSong, String>| {
                let Some(bar) = bar.upgrade() else { return };
                bar.set_loading(false, "");
                match res {
                    Ok(loaded) => bar.use_song(loaded),
                    Err(e) => bar.set_info(&format!("Could not load the song: {e}")),
                }
                bar.refresh();
            },
        );
    }

    fn use_song(&self, loaded: LoadedSong) {
        self.play.set_active(false);
        let song = self.song();
        retire(song.track.swap(Some(Arc::new(loaded.track))));
        retire(song.harmony.swap(Some(Arc::new(loaded.harmony))));
        *self.song_name.borrow_mut() = Some(loaded.name);
        self.update_length();
        self.rewind();
    }

    pub fn load_midi(self: &Rc<Self>, path: PathBuf) {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
        self.set_loading(true, &name.unwrap_or_default());
        let bar = Rc::downgrade(self);
        load_in_background(
            move || song::load_midi(&path),
            move |res: Result<LoadedMidi, String>| {
                let Some(bar) = bar.upgrade() else { return };
                bar.set_loading(false, "");
                match res {
                    Ok(midi) => bar.use_midi(midi),
                    Err(e) => bar.set_info(&format!("Could not load the melody: {e}")),
                }
                bar.refresh();
            },
        );
    }

    fn use_midi(&self, midi: LoadedMidi) {
        let labels: Vec<String> = midi.song.parts.iter().map(|p| p.label()).collect();
        let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
        let default = midi.song.default_part().unwrap_or(0);
        *self.midi.borrow_mut() = Some(midi);
        // Setting the model fires `selected` with the first part; choose
        // the melody part after it.
        self.parts.set_model(Some(&gtk::StringList::new(&labels)));
        self.parts.set_selected(default as u32);
        self.use_part(default);
        self.melody_box.set_visible(true);
        self.update_length();
    }

    fn use_part(&self, index: usize) {
        let guide = {
            let midi = self.midi.borrow();
            let Some(part) = midi.as_ref().and_then(|m| m.song.parts.get(index)) else {
                return;
            };
            part.guide(GUIDE_RATE)
        };
        retire(self.song().guide.swap(Some(Arc::new(guide))));
    }

    /// Updates the time, progress and info line. Call regularly.
    pub fn refresh(&self) {
        let song = self.song();
        // The clock only runs with the audio on.
        if !self.power.is_active() && song.playing.get() {
            song.playing.set(false);
        }
        if self.play.is_active() != song.playing.get() {
            self.play.set_active(song.playing.get());
        }

        let (pos, len) = (song.seconds(), song.length.get() as f64);
        let time = format!("{} / {}", clock(pos.min(len)), clock(len));
        self.progress.set_fraction(if len > 0.0 {
            (pos / len).clamp(0.0, 1.0)
        } else {
            0.0
        });
        if self.loading.get() > 0 {
            return;
        }

        let mut info: Vec<String> = Vec::new();
        if let Some(name) = self.song_name.borrow().as_ref() {
            info.push(name.clone());
            if let Some(key) = self.key_name() {
                info.push(format!("key {key}"));
            }
            let h = song.harmony.load();
            let chord = h
                .as_deref()
                .and_then(|h| h.at((pos * h.rate as f64) as u64))
                .and_then(|s| s.chord);
            if let Some(chord) = chord {
                info.push(format!("chord {}", chord.name()));
            }
        }
        if let Some(midi) = self.midi.borrow().as_ref() {
            match song.target_at(pos) {
                Some(SongTarget::Guide(n)) => info.push(format!("melody {}", note_name(n))),
                _ => info.push(midi.name.clone()),
            }
        }
        let info = if info.is_empty() {
            "No song loaded. Open an MP3, WAV, FLAC or OGG file to sing along, \
             and optionally its melody as a MIDI file."
                .to_string()
        } else {
            info.join("  \u{b7}  ")
        };

        let mut shown = self.shown.borrow_mut();
        if shown.0 != time {
            self.time.set_text(&time);
            shown.0 = time;
        }
        if shown.1 != info {
            self.info.set_text(&info);
            shown.1 = info;
        }
    }
}

/// Shows a file chooser and calls `then` with the chosen path.
fn choose_file(
    window: &gtk::ApplicationWindow,
    title: &str,
    filter: gtk::FileFilter,
    then: impl Fn(PathBuf) + 'static,
) {
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    let dialog = gtk::FileDialog::builder()
        .title(title)
        .modal(true)
        .filters(&filters)
        .default_filter(&filter)
        .build();
    dialog.open(Some(window), gio::Cancellable::NONE, move |res| {
        if let Some(path) = res.ok().and_then(|f| f.path()) {
            then(path);
        }
    });
}

/// Paths given on the command line.
#[derive(Debug, Default, Clone)]
pub struct StartFiles {
    pub song: Option<PathBuf>,
    pub melody: Option<PathBuf>,
}

impl StartFiles {
    pub fn load(&self, bar: &Rc<SongBar>) {
        if let Some(p) = &self.song {
            bar.load_song(p.clone());
        }
        if let Some(p) = &self.melody {
            bar.load_midi(p.clone());
        }
    }

    /// Is `path` a MIDI file (by extension)?
    pub fn is_midi(path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("mid") || e.eq_ignore_ascii_case("midi"))
    }
}
