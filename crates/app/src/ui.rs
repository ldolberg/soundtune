use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use gtk::prelude::*;
use gtk::{gdk, glib};
use soundtune_dsp::{hz_to_midi, snap_to_mask, Meters, MusicScale, Params, NOTE_NAMES};

use crate::audio::{self, Engine};
use crate::widgets::{Keyboard, Knob, PitchMeter};

const DEFAULT_DEVICE: &str = "System default";

const CSS: &str = "
window.console { background-color: #1b1e23; }
headerbar { background: #262a31; box-shadow: none; border-bottom: 1px solid #111316; }
.brand { font-weight: 800; letter-spacing: 3px; font-size: 1.1em; }
.strip { background: #23272d; border-bottom: 1px solid #111316; padding: 8px 18px; }
.section-title { color: #8a9099; font-size: 0.75em; font-weight: 700; letter-spacing: 1px; }
.knob-title { color: #d6d9de; font-size: 0.85em; }
.knob-title.big { font-size: 1.1em; }
.knob-value { color: #4a9eff; font-size: 0.85em; }
.knob-value.big { font-size: 1.05em; }
.knob-value:disabled { color: #5c6168; }
.field-title { color: #d6d9de; font-size: 0.85em; }
button.power { min-width: 22px; min-height: 22px; padding: 2px; border-radius: 99px; background: none; color: #6b7179; box-shadow: none; }
button.power:checked { color: #4a9eff; }
button.main-power:checked { color: #ffffff; background: #3d7fe0; }
.scale-panel { background: #23272d; border: 1px solid #343941; border-radius: 10px; padding: 10px 16px; }
.cents { color: #8a9099; font-size: 0.8em; }
button.note { min-width: 38px; padding: 4px 0; background: #2d3239; color: #c3c7cd; border: 1px solid #3b4048; box-shadow: none; }
button.note:checked { background: #e8952c; color: #1b1e23; border-color: #e8952c; font-weight: 700; }
button.set { min-width: 84px; background: #2d3239; box-shadow: none; }
.footer { background: #23272d; border-top: 1px solid #111316; padding: 6px 14px; }
.footer label { color: #8a9099; font-size: 0.85em; }
levelbar trough { min-height: 6px; }
";

pub fn load_css() {
    if let Some(settings) = gtk::Settings::default() {
        settings.set_gtk_application_prefer_dark_theme(true);
    }
    let provider = gtk::CssProvider::new();
    provider.load_from_string(CSS);
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

/// Notes the autotune snaps to, mirrored in the note buttons and keyboard.
struct ScaleState {
    params: Arc<Params>,
    mask: Cell<u32>,
    key: Cell<u32>,
    scale: Cell<MusicScale>,
    buttons: Vec<gtk::ToggleButton>,
    keyboard: Keyboard,
    updating: Cell<bool>,
}

impl ScaleState {
    fn set_mask(&self, mask: u32) {
        if self.updating.get() {
            return;
        }
        self.updating.set(true);
        self.mask.set(mask);
        self.params.tune_mask.store(mask, Ordering::Relaxed);
        for (pc, b) in self.buttons.iter().enumerate() {
            b.set_active(mask & (1 << pc) != 0);
        }
        self.keyboard.set_mask(mask);
        self.updating.set(false);
    }

    fn set_scale(&self, key: u32, scale: MusicScale) {
        self.key.set(key);
        self.scale.set(scale);
        self.set_mask(scale.mask(key));
    }
}

/// Widgets presets need to change.
struct Controls {
    tune_on: gtk::ToggleButton,
    pitch_on: gtk::ToggleButton,
    dist_on: gtk::ToggleButton,
    verb_on: gtk::ToggleButton,
    retune: Knob,
    pitch: Knob,
    drive: Knob,
    tone: Knob,
    dist_mix: Knob,
    room: Knob,
    damping: Knob,
    verb_mix: Knob,
    key: gtk::DropDown,
    scale: Rc<ScaleState>,
}

struct Preset {
    name: &'static str,
    pitch: Option<f64>,
    /// key, scale, retune speed (0..1)
    tune: Option<(u32, MusicScale, f64)>,
    /// drive, tone, mix
    dist: Option<(f64, f64, f64)>,
    /// room, damping, mix
    verb: Option<(f64, f64, f64)>,
}

const PRESETS: [Preset; 6] = [
    Preset {
        name: "Clean",
        pitch: None,
        tune: None,
        dist: None,
        verb: None,
    },
    Preset {
        name: "Chipmunk",
        pitch: Some(10.0),
        tune: None,
        dist: None,
        verb: Some((0.3, 0.5, 0.12)),
    },
    Preset {
        name: "Robot Tune",
        pitch: None,
        tune: Some((0, MusicScale::Major, 0.0)),
        dist: None,
        verb: Some((0.5, 0.5, 0.15)),
    },
    Preset {
        name: "Pop Star",
        pitch: None,
        tune: Some((0, MusicScale::Chromatic, 0.35)),
        dist: None,
        verb: Some((0.75, 0.4, 0.3)),
    },
    Preset {
        name: "Megaphone",
        pitch: None,
        tune: None,
        dist: Some((0.55, 0.25, 1.0)),
        verb: None,
    },
    Preset {
        name: "Cathedral",
        pitch: None,
        tune: None,
        dist: None,
        verb: Some((0.95, 0.25, 0.5)),
    },
];

impl Controls {
    fn apply(&self, p: &Preset) {
        self.pitch_on.set_active(p.pitch.is_some());
        if let Some(semis) = p.pitch {
            self.pitch.set_value(semis);
        }
        self.tune_on.set_active(p.tune.is_some());
        if let Some((key, scale, speed)) = p.tune {
            self.key.set_selected(key);
            self.scale.set_scale(key, scale);
            self.retune.set_value(speed);
        }
        self.dist_on.set_active(p.dist.is_some());
        if let Some((drive, tone, mix)) = p.dist {
            self.drive.set_value(drive);
            self.tone.set_value(tone);
            self.dist_mix.set_value(mix);
        }
        self.verb_on.set_active(p.verb.is_some());
        if let Some((room, damping, mix)) = p.verb {
            self.room.set_value(room);
            self.damping.set_value(damping);
            self.verb_mix.set_value(mix);
        }
    }
}

/// Keeps a float parameter in sync with a knob.
macro_rules! bind_knob {
    ($knob:expr, $params:expr, $field:ident) => {{
        let p = Arc::clone(&$params);
        p.$field.set($knob.value() as f32);
        $knob.connect_changed(move |v| p.$field.set(v as f32));
    }};
}

/// Keeps an effect's on/off parameter in sync with its power button, and
/// greys out the effect's knobs while it is off.
macro_rules! bind_power {
    ($button:expr, $params:expr, $field:ident, [$($knob:expr),*]) => {{
        let p = Arc::clone(&$params);
        let knobs: Vec<Knob> = vec![$($knob.clone()),*];
        let sync = move |b: &gtk::ToggleButton| {
            p.$field.set(b.is_active());
            knobs.iter().for_each(|k| k.set_active(b.is_active()));
        };
        sync(&$button);
        $button.connect_toggled(sync);
    }};
}

fn percent(v: f64) -> String {
    format!("{:.0}%", v * 100.0)
}

fn power_button(tooltip: &str) -> gtk::ToggleButton {
    let b = gtk::ToggleButton::new();
    b.set_icon_name("system-shutdown-symbolic");
    b.set_tooltip_text(Some(tooltip));
    b.add_css_class("power");
    b.set_valign(gtk::Align::Center);
    b
}

/// A titled group in the top strip.
fn section(title: &str, power: Option<&gtk::ToggleButton>, children: &[&gtk::Widget]) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    head.set_halign(gtk::Align::Center);
    if let Some(p) = power {
        head.append(p);
    }
    let l = gtk::Label::new(Some(&title.to_uppercase()));
    l.add_css_class("section-title");
    head.append(&l);
    b.append(&head);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    row.set_halign(gtk::Align::Center);
    for c in children {
        row.append(*c);
    }
    b.append(&row);
    b
}

fn labeled(title: &str, w: &impl IsA<gtk::Widget>) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 6);
    b.set_valign(gtk::Align::Center);
    let l = gtk::Label::new(Some(title));
    l.add_css_class("field-title");
    b.append(&l);
    b.append(w);
    b
}

fn set_devices(dd: &gtk::DropDown, names: &[String]) {
    let mut items: Vec<&str> = vec![DEFAULT_DEVICE];
    items.extend(names.iter().map(String::as_str));
    dd.set_model(Some(&gtk::StringList::new(&items)));
    dd.set_selected(0);
}

fn selected_device(dd: &gtk::DropDown) -> Option<String> {
    if dd.selected() == 0 {
        return None;
    }
    dd.selected_item()
        .and_downcast::<gtk::StringObject>()
        .map(|s| s.string().to_string())
}

fn separator() -> gtk::Separator {
    let s = gtk::Separator::new(gtk::Orientation::Vertical);
    s.set_margin_top(4);
    s.set_margin_bottom(4);
    s
}

pub fn build(app: &gtk::Application) {
    let params = Arc::new(Params::default());
    let meters = Arc::new(Meters::default());
    let engine: Rc<RefCell<Option<Engine>>> = Rc::new(RefCell::new(None));

    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("SoundTune")
        .default_width(1080)
        .default_height(720)
        .build();
    window.add_css_class("console");

    // Header: power, presets, brand, device settings.
    let header = gtk::HeaderBar::new();
    let power = power_button("Start / stop audio");
    power.add_css_class("main-power");
    let preset_dd = gtk::DropDown::from_strings(&PRESETS.map(|p| p.name));
    preset_dd.set_tooltip_text(Some("Presets"));
    let brand = gtk::Label::new(Some("SOUNDTUNE"));
    brand.add_css_class("brand");
    header.set_title_widget(Some(&brand));
    header.pack_start(&power);
    header.pack_start(&preset_dd);

    let input_dd = gtk::DropDown::from_strings(&[DEFAULT_DEVICE]);
    let output_dd = gtk::DropDown::from_strings(&[DEFAULT_DEVICE]);
    set_devices(&input_dd, &audio::input_device_names());
    set_devices(&output_dd, &audio::output_device_names());
    let refresh = gtk::Button::with_label("Rescan devices");
    {
        let (input_dd, output_dd) = (input_dd.clone(), output_dd.clone());
        refresh.connect_clicked(move |_| {
            set_devices(&input_dd, &audio::input_device_names());
            set_devices(&output_dd, &audio::output_device_names());
        });
    }
    let devices = gtk::Grid::new();
    devices.set_row_spacing(8);
    devices.set_column_spacing(10);
    devices.set_margin_top(8);
    devices.set_margin_bottom(8);
    devices.set_margin_start(8);
    devices.set_margin_end(8);
    devices.attach(&gtk::Label::new(Some("Microphone")), 0, 0, 1, 1);
    devices.attach(&input_dd, 1, 0, 1, 1);
    devices.attach(&gtk::Label::new(Some("Output")), 0, 1, 1, 1);
    devices.attach(&output_dd, 1, 1, 1, 1);
    devices.attach(&refresh, 1, 2, 1, 1);
    let popover = gtk::Popover::new();
    popover.set_child(Some(&devices));
    let settings = gtk::MenuButton::new();
    settings.set_icon_name("emblem-system-symbolic");
    settings.set_tooltip_text(Some("Audio devices"));
    settings.set_popover(Some(&popover));
    header.pack_end(&settings);
    window.set_titlebar(Some(&header));

    // Knobs and power buttons.
    let tune_on = power_button("Autotune on / off");
    let pitch_on = power_button("High pitch on / off");
    let dist_on = power_button("Distortion on / off");
    let verb_on = power_button("Reverb on / off");

    let retune = Knob::new("Retune Speed", 0.0, 1.0, 0.1, 120, |v| {
        format!("{:.0} ms", v * 250.0)
    });
    let pitch = Knob::new("High Pitch", -12.0, 12.0, 7.0, 120, |v| {
        format!("{v:+.0} st")
    })
    .bipolar()
    .stepped(1.0);
    let drive = Knob::new("Distortion", 0.0, 1.0, 0.5, 120, percent);
    let room = Knob::new("Reverb", 0.0, 1.0, 0.7, 120, percent);
    retune.prepend_header(&tune_on);
    pitch.prepend_header(&pitch_on);
    drive.prepend_header(&dist_on);
    room.prepend_header(&verb_on);

    let tone = Knob::new("Tone", 0.0, 1.0, 0.5, 44, percent);
    let dist_mix = Knob::new("Mix", 0.0, 1.0, 1.0, 44, percent);
    let damping = Knob::new("Damping", 0.0, 1.0, 0.5, 44, percent);
    let verb_mix = Knob::new("Mix", 0.0, 1.0, 0.3, 44, percent);
    let volume = Knob::new("Volume", 0.0, 2.0, 1.0, 44, percent);

    bind_knob!(retune, params, tune_speed);
    bind_knob!(pitch, params, pitch_semitones);
    bind_knob!(drive, params, drive);
    bind_knob!(tone, params, tone);
    bind_knob!(dist_mix, params, dist_mix);
    bind_knob!(room, params, room);
    bind_knob!(damping, params, damping);
    bind_knob!(verb_mix, params, verb_mix);
    bind_knob!(volume, params, master);
    bind_power!(tune_on, params, tune_on, [retune]);
    bind_power!(pitch_on, params, pitch_on, [pitch]);
    bind_power!(dist_on, params, dist_on, [drive, tone, dist_mix]);
    bind_power!(verb_on, params, verb_on, [room, damping, verb_mix]);

    // Scale: note buttons, keyboard and "set" buttons share one mask.
    let keyboard = Keyboard::new(96);
    let buttons: Vec<gtk::ToggleButton> = NOTE_NAMES
        .iter()
        .map(|n| {
            let b = gtk::ToggleButton::with_label(n);
            b.add_css_class("note");
            b
        })
        .collect();
    let scale = Rc::new(ScaleState {
        params: Arc::clone(&params),
        mask: Cell::new(0),
        key: Cell::new(0),
        scale: Cell::new(MusicScale::Major),
        buttons: buttons.clone(),
        keyboard: keyboard.clone(),
        updating: Cell::new(false),
    });
    scale.set_scale(0, MusicScale::Major);
    for (pc, b) in buttons.iter().enumerate() {
        let s = Rc::downgrade(&scale);
        b.connect_toggled(move |b| {
            let Some(s) = s.upgrade() else { return };
            if !s.updating.get() {
                let bit = 1 << pc;
                s.set_mask(if b.is_active() {
                    s.mask.get() | bit
                } else {
                    s.mask.get() & !bit
                });
            }
        });
    }
    {
        let s = Rc::downgrade(&scale);
        keyboard.connect_toggle(move |pc| {
            if let Some(s) = s.upgrade() {
                s.set_mask(s.mask.get() ^ (1 << pc));
            }
        });
    }

    let key_dd = gtk::DropDown::from_strings(&NOTE_NAMES);
    {
        let s = Rc::clone(&scale);
        key_dd.connect_selected_notify(move |d| s.set_scale(d.selected(), s.scale.get()));
    }

    // Top strip.
    let strip = gtk::Box::new(gtk::Orientation::Horizontal, 22);
    strip.add_css_class("strip");
    let key_box = labeled("Key", &key_dd);
    strip.append(&section("Autotune", None, &[key_box.upcast_ref()]));
    strip.append(&separator());
    strip.append(&section(
        "Distortion",
        None,
        &[tone.widget().upcast_ref(), dist_mix.widget().upcast_ref()],
    ));
    strip.append(&separator());
    strip.append(&section(
        "Reverb",
        None,
        &[
            damping.widget().upcast_ref(),
            verb_mix.widget().upcast_ref(),
        ],
    ));
    strip.append(&separator());
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    strip.append(&spacer);
    strip.append(&separator());
    strip.append(&section("Output", None, &[volume.widget().upcast_ref()]));

    // Centre: big knobs around the pitch meter.
    let meter = PitchMeter::new(290);
    let centre = gtk::Box::new(gtk::Orientation::Horizontal, 26);
    centre.set_halign(gtk::Align::Center);
    centre.set_valign(gtk::Align::Center);
    centre.set_vexpand(true);
    centre.set_margin_top(14);
    centre.set_margin_bottom(14);
    for k in [&retune, &pitch] {
        k.widget().set_valign(gtk::Align::Center);
        centre.append(k.widget());
    }
    centre.append(meter.widget());
    for k in [&drive, &room] {
        k.widget().set_valign(gtk::Align::Center);
        centre.append(k.widget());
    }

    // Scale panel.
    let set_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
    set_box.set_valign(gtk::Align::Center);
    for sc in [
        MusicScale::Major,
        MusicScale::Minor,
        MusicScale::Pentatonic,
        MusicScale::Chromatic,
    ] {
        let label = if sc == MusicScale::Chromatic {
            "All"
        } else {
            sc.name()
        };
        let b = gtk::Button::with_label(label);
        b.add_css_class("set");
        let s = Rc::clone(&scale);
        b.connect_clicked(move |_| s.set_scale(s.key.get(), sc));
        set_box.append(&b);
    }
    let notes = gtk::Grid::new();
    notes.set_column_spacing(6);
    notes.set_row_spacing(6);
    notes.set_valign(gtk::Align::Center);
    for (pc, b) in buttons.iter().enumerate() {
        let cents = gtk::Label::new(Some(&(pc * 100).to_string()));
        cents.add_css_class("cents");
        notes.attach(&cents, pc as i32, 0, 1, 1);
        notes.attach(b, pc as i32, 1, 1, 1);
    }
    let hint = gtk::Label::new(Some(
        "Autotune snaps to the highlighted notes.\nClick notes or piano keys to edit the scale.",
    ));
    hint.add_css_class("cents");
    hint.set_justify(gtk::Justification::Center);
    let panel = gtk::Box::new(gtk::Orientation::Horizontal, 24);
    panel.add_css_class("scale-panel");
    panel.set_halign(gtk::Align::Center);
    panel.set_margin_bottom(12);
    let set_title = gtk::Label::new(Some("SET"));
    set_title.add_css_class("section-title");
    panel.append(&set_title);
    panel.append(&set_box);
    panel.append(&notes);
    panel.append(&hint);

    // Footer: levels and status.
    let footer = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    footer.add_css_class("footer");
    let in_meter = gtk::LevelBar::for_interval(0.0, 1.0);
    let out_meter = gtk::LevelBar::for_interval(0.0, 1.0);
    for m in [&in_meter, &out_meter] {
        m.set_size_request(140, -1);
        m.set_valign(gtk::Align::Center);
    }
    let status = gtk::Label::new(Some("Stopped. Use headphones to avoid feedback."));
    status.set_hexpand(true);
    status.set_xalign(1.0);
    status.set_ellipsize(gtk::pango::EllipsizeMode::Start);
    footer.append(&gtk::Label::new(Some("IN")));
    footer.append(&in_meter);
    footer.append(&gtk::Label::new(Some("OUT")));
    footer.append(&out_meter);
    footer.append(&status);

    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.append(&strip);
    root.append(&centre);
    root.append(&panel);
    root.append(keyboard.widget());
    root.append(&footer);
    window.set_child(Some(&root));

    let controls = Rc::new(Controls {
        tune_on,
        pitch_on,
        dist_on,
        verb_on,
        retune,
        pitch,
        drive,
        tone,
        dist_mix,
        room,
        damping,
        verb_mix,
        key: key_dd,
        scale: Rc::clone(&scale),
    });
    {
        let controls = Rc::clone(&controls);
        preset_dd.connect_selected_notify(move |d| controls.apply(&PRESETS[d.selected() as usize]));
    }

    // Start / stop.
    {
        let (engine, status, settings) = (Rc::clone(&engine), status.clone(), settings.clone());
        let (params, meters) = (Arc::clone(&params), Arc::clone(&meters));
        power.connect_toggled(move |btn| {
            if btn.is_active() {
                let input = selected_device(&input_dd);
                let output = selected_device(&output_dd);
                let started = Engine::start(
                    input.as_deref(),
                    output.as_deref(),
                    Arc::clone(&params),
                    Arc::clone(&meters),
                );
                match started {
                    Ok(e) => {
                        status.set_text(&e.description);
                        *engine.borrow_mut() = Some(e);
                        settings.set_sensitive(false);
                    }
                    Err(err) => {
                        btn.set_active(false);
                        status.set_text(&format!("Could not start audio: {err}"));
                    }
                }
            } else {
                let was_running = engine.borrow_mut().take().is_some();
                settings.set_sensitive(true);
                if was_running {
                    status.set_text("Stopped.");
                }
            }
        });
    }

    // Meter refresh.
    {
        let (in_level, out_level) = (Cell::new(0.0f32), Cell::new(0.0f32));
        let shown_cents = Cell::new(0.0f64);
        glib::timeout_add_local(Duration::from_millis(33), move || {
            // Peak meters with a gentle fall-off.
            in_level.set(meters.input.take().max(in_level.get() * 0.8));
            out_level.set(meters.output.take().max(out_level.get() * 0.8));
            in_meter.set_value(in_level.get().min(1.0) as f64);
            out_meter.set_value(out_level.get().min(1.0) as f64);

            let hz = meters.pitch_hz.get();
            if hz > 0.0 {
                let sung = hz_to_midi(hz);
                let target = if params.tune_on.get() {
                    snap_to_mask(sung, params.tune_mask.load(Ordering::Relaxed))
                } else {
                    sung.round()
                };
                let cents = ((sung - target) * 100.0) as f64;
                shown_cents.set(shown_cents.get() + (cents - shown_cents.get()) * 0.5);
                let letter = NOTE_NAMES[(target as i32).rem_euclid(12) as usize];
                meter.update(true, letter, shown_cents.get(), hz as f64);
                keyboard.set_current(Some(target as i32));
            } else {
                meter.update(false, "-", 0.0, 0.0);
                keyboard.set_current(None);
            }
            glib::ControlFlow::Continue
        });
    }

    window.present();
}
