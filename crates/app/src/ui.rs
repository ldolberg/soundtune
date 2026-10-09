use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use gtk::prelude::*;
use gtk::{gdk, glib};
use soundtune_dsp::{
    apply_sing_mode, bypass_all, hz_to_midi, snap_to_mask, Key, Meters, MusicScale, Params, Voice,
    NOTE_NAMES,
};

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
.strip.chain { background: #202328; }
button.popstar { font-size: 1.5em; font-weight: 800; letter-spacing: 2px; padding: 16px 40px; border-radius: 99px; background: #2d3239; color: #d6d9de; border: 1px solid #3b4048; box-shadow: none; }
button.popstar:checked { background: linear-gradient(90deg, #e0479e, #8b5cf6); color: #ffffff; border-color: #f07cc0; box-shadow: 0 0 22px rgba(224, 71, 158, 0.45); }
button.voice { min-width: 150px; padding: 8px 0; background: #2d3239; color: #c3c7cd; border: 1px solid #3b4048; box-shadow: none; }
button.voice:checked { background: #4a9eff; color: #1b1e23; border-color: #4a9eff; font-weight: 700; }
.sing-hint { color: #8a9099; }
.key-label { color: #d6d9de; font-size: 1.15em; font-weight: 700; }
.headphones { background: #2a2620; color: #e8b56c; border: 1px solid #4a3d28; border-radius: 99px; padding: 6px 16px; }
scale.correction trough { min-height: 8px; border-radius: 99px; background: #2d3239; }
scale.correction highlight { min-height: 8px; border-radius: 99px; background: linear-gradient(90deg, #4a9eff, #e0479e); }
scale.correction slider { min-width: 20px; min-height: 20px; border-radius: 99px; background: #f2f2f2; box-shadow: none; }
.correction-value { color: #4a9eff; font-weight: 700; }
scale.correction marks label { color: #8a9099; font-size: 0.8em; }
.feedback { background: #6b1f26; color: #ffe1e3; font-weight: 700; padding: 8px; }
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
    gate_on: gtk::ToggleButton,
    comp_on: gtk::ToggleButton,
    bright_on: gtk::ToggleButton,
    double_on: gtk::ToggleButton,
    retune: Knob,
    correction: Knob,
    pitch: Knob,
    drive: Knob,
    tone: Knob,
    dist_mix: Knob,
    room: Knob,
    damping: Knob,
    verb_mix: Knob,
    gate: Knob,
    comp_threshold: Knob,
    comp_ratio: Knob,
    brightness: Knob,
    double_mix: Knob,
    double_detune: Knob,
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
    /// gate threshold (dB)
    gate: Option<f64>,
    /// threshold (dB), ratio
    comp: Option<(f64, f64)>,
    /// brightness amount
    bright: Option<f64>,
    /// mix, detune (cents)
    double: Option<(f64, f64)>,
}

const CLEAN: Preset = Preset {
    name: "Clean",
    pitch: None,
    tune: None,
    dist: None,
    verb: None,
    gate: None,
    comp: None,
    bright: None,
    double: None,
};

const PRESETS: [Preset; 8] = [
    CLEAN,
    Preset { name: "Chipmunk", pitch: Some(10.0), verb: Some((0.3, 0.5, 0.12)), ..CLEAN },
    Preset { name: "Robot Tune", tune: Some((0, MusicScale::Major, 0.0)), verb: Some((0.5, 0.5, 0.15)), ..CLEAN },
    Preset {
        name: "Pop Princess",
        tune: Some((0, MusicScale::Major, 0.03)),
        verb: Some((0.82, 0.3, 0.28)),
        gate: Some(-46.0),
        comp: Some((-26.0, 5.0)),
        bright: Some(0.85),
        double: Some((0.6, 12.0)),
        ..CLEAN
    },
    Preset { name: "Pop Star", tune: Some((0, MusicScale::Chromatic, 0.35)), verb: Some((0.75, 0.4, 0.3)), ..CLEAN },
    Preset {
        name: "Radio Voice",
        gate: Some(-50.0),
        comp: Some((-24.0, 4.0)),
        bright: Some(0.5),
        verb: Some((0.4, 0.5, 0.06)),
        ..CLEAN
    },
    Preset { name: "Megaphone", dist: Some((0.55, 0.25, 1.0)), ..CLEAN },
    Preset { name: "Cathedral", verb: Some((0.95, 0.25, 0.5)), ..CLEAN },
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
        self.gate_on.set_active(p.gate.is_some());
        if let Some(threshold) = p.gate {
            self.gate.set_value(threshold);
        }
        self.comp_on.set_active(p.comp.is_some());
        if let Some((threshold, ratio)) = p.comp {
            self.comp_threshold.set_value(threshold);
            self.comp_ratio.set_value(ratio);
        }
        self.bright_on.set_active(p.bright.is_some());
        if let Some(amount) = p.bright {
            self.brightness.set_value(amount);
        }
        self.double_on.set_active(p.double.is_some());
        if let Some((mix, detune)) = p.double {
            self.double_mix.set_value(mix);
            self.double_detune.set_value(detune);
        }
    }

    /// Shows the current parameter values (after Sing mode changed them).
    fn sync(&self, p: &Params) {
        let toggles = [
            (&self.tune_on, &p.tune_on),
            (&self.pitch_on, &p.pitch_on),
            (&self.dist_on, &p.dist_on),
            (&self.verb_on, &p.verb_on),
            (&self.gate_on, &p.gate_on),
            (&self.comp_on, &p.comp_on),
            (&self.bright_on, &p.bright_on),
            (&self.double_on, &p.double_on),
        ];
        // Read everything first: updating a widget writes its value back.
        let states: Vec<bool> = toggles.iter().map(|(_, t)| t.get()).collect();
        let knobs = [
            (&self.retune, &p.tune_speed),
            (&self.correction, &p.tune_amount),
            (&self.pitch, &p.pitch_semitones),
            (&self.drive, &p.drive),
            (&self.tone, &p.tone),
            (&self.dist_mix, &p.dist_mix),
            (&self.room, &p.room),
            (&self.damping, &p.damping),
            (&self.verb_mix, &p.verb_mix),
            (&self.gate, &p.gate_threshold),
            (&self.comp_threshold, &p.comp_threshold),
            (&self.comp_ratio, &p.comp_ratio),
            (&self.brightness, &p.brightness),
            (&self.double_mix, &p.double_mix),
            (&self.double_detune, &p.double_detune),
        ];
        let values: Vec<f32> = knobs.iter().map(|(_, v)| v.get()).collect();
        for ((knob, _), v) in knobs.iter().zip(values) {
            knob.set_value(v as f64);
        }
        for ((button, _), on) in toggles.iter().zip(states) {
            button.set_active(on);
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

fn decibels(v: f64) -> String {
    format!("{v:.0} dB")
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
        .default_height(800)
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
    let stack = gtk::Stack::new();
    stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    let switcher = gtk::StackSwitcher::new();
    switcher.set_stack(Some(&stack));
    let title = gtk::Box::new(gtk::Orientation::Horizontal, 18);
    title.append(&brand);
    title.append(&switcher);
    header.set_title_widget(Some(&title));
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
    let gate_on = power_button("Noise gate on / off");
    let comp_on = power_button("Compressor on / off");
    let bright_on = power_button("Brightness on / off");
    let double_on = power_button("Doubler on / off");
    let guard_on = power_button("Feedback guard: turns the output down when the speakers howl");
    guard_on.set_active(params.feedback_guard.get());

    let retune = Knob::new("Retune Speed", 0.0, 1.0, 0.1, 120, |v| format!("{:.0} ms", v * 250.0));
    let pitch = Knob::new("High Pitch", -12.0, 12.0, 7.0, 120, |v| format!("{v:+.0} st"))
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
    let correction = Knob::new("Correction", 0.0, 1.0, 1.0, 44, percent);
    let gate = Knob::new("Threshold", -70.0, -20.0, -48.0, 44, decibels);
    let comp_threshold = Knob::new("Threshold", -40.0, 0.0, -20.0, 44, decibels);
    let comp_ratio = Knob::new("Ratio", 1.0, 10.0, 3.0, 44, |v| format!("{v:.1}:1"));
    let brightness = Knob::new("Amount", 0.0, 1.0, 0.5, 44, percent);
    let double_mix = Knob::new("Mix", 0.0, 1.0, 0.5, 44, percent);
    let double_detune = Knob::new("Detune", 0.0, 30.0, 10.0, 44, |v| format!("{v:.0} ct"));

    bind_knob!(retune, params, tune_speed);
    bind_knob!(pitch, params, pitch_semitones);
    bind_knob!(drive, params, drive);
    bind_knob!(tone, params, tone);
    bind_knob!(dist_mix, params, dist_mix);
    bind_knob!(room, params, room);
    bind_knob!(damping, params, damping);
    bind_knob!(verb_mix, params, verb_mix);
    bind_knob!(volume, params, master);
    bind_knob!(correction, params, tune_amount);
    bind_knob!(gate, params, gate_threshold);
    bind_knob!(comp_threshold, params, comp_threshold);
    bind_knob!(comp_ratio, params, comp_ratio);
    bind_knob!(brightness, params, brightness);
    bind_knob!(double_mix, params, double_mix);
    bind_knob!(double_detune, params, double_detune);
    bind_power!(tune_on, params, tune_on, [retune, correction]);
    bind_power!(pitch_on, params, pitch_on, [pitch]);
    bind_power!(dist_on, params, dist_on, [drive, tone, dist_mix]);
    bind_power!(verb_on, params, verb_on, [room, damping, verb_mix]);
    bind_power!(gate_on, params, gate_on, [gate]);
    bind_power!(comp_on, params, comp_on, [comp_threshold, comp_ratio]);
    bind_power!(bright_on, params, bright_on, [brightness]);
    bind_power!(double_on, params, double_on, [double_mix, double_detune]);
    bind_power!(guard_on, params, feedback_guard, []);

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
                s.set_mask(if b.is_active() { s.mask.get() | bit } else { s.mask.get() & !bit });
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
    let auto_key = gtk::ToggleButton::with_label("Auto");
    auto_key.add_css_class("set");
    auto_key.set_tooltip_text(Some("Detect the key from your singing"));
    {
        let p = Arc::clone(&params);
        let key_dd = key_dd.clone();
        auto_key.connect_toggled(move |b| {
            p.auto_key.set(b.is_active());
            key_dd.set_sensitive(!b.is_active());
        });
    }
    let auto_box = labeled("Detect", &auto_key);
    strip.append(&section(
        "Autotune",
        None,
        &[key_box.upcast_ref(), auto_box.upcast_ref(), correction.widget().upcast_ref()],
    ));
    strip.append(&separator());
    strip.append(&section("Distortion", None, &[tone.widget().upcast_ref(), dist_mix.widget().upcast_ref()]));
    strip.append(&separator());
    strip.append(&section("Reverb", None, &[damping.widget().upcast_ref(), verb_mix.widget().upcast_ref()]));
    strip.append(&separator());
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    strip.append(&spacer);
    strip.append(&separator());
    strip.append(&section("Output", None, &[volume.widget().upcast_ref()]));

    // Vocal chain strip: the clean-up and polish effects.
    let chain = gtk::Box::new(gtk::Orientation::Horizontal, 22);
    chain.add_css_class("strip");
    chain.add_css_class("chain");
    chain.append(&section("Gate", Some(&gate_on), &[gate.widget().upcast_ref()]));
    chain.append(&separator());
    chain.append(&section(
        "Compressor",
        Some(&comp_on),
        &[comp_threshold.widget().upcast_ref(), comp_ratio.widget().upcast_ref()],
    ));
    chain.append(&separator());
    chain.append(&section("Brightness", Some(&bright_on), &[brightness.widget().upcast_ref()]));
    chain.append(&separator());
    chain.append(&section(
        "Doubler",
        Some(&double_on),
        &[double_mix.widget().upcast_ref(), double_detune.widget().upcast_ref()],
    ));
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    chain.append(&spacer);
    chain.append(&separator());
    let reduction = gtk::LevelBar::for_interval(0.0, 20.0);
    reduction.set_size_request(90, -1);
    reduction.set_valign(gtk::Align::Center);
    reduction.set_tooltip_text(Some("Compressor gain reduction (0 to 20 dB)"));
    let reduction_box = labeled("Reduction", &reduction);
    chain.append(&section("Meter", None, &[reduction_box.upcast_ref()]));
    chain.append(&separator());
    let guard_label = gtk::Label::new(Some("Howl guard"));
    guard_label.add_css_class("field-title");
    let guard_box = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    guard_box.set_valign(gtk::Align::Center);
    guard_box.append(&guard_on);
    guard_box.append(&guard_label);
    chain.append(&section("Safety", None, &[guard_box.upcast_ref()]));

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
    for sc in [MusicScale::Major, MusicScale::Minor, MusicScale::Pentatonic, MusicScale::Chromatic] {
        let label = if sc == MusicScale::Chromatic { "All" } else { sc.name() };
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

    let advanced = gtk::Box::new(gtk::Orientation::Vertical, 0);
    advanced.append(&strip);
    advanced.append(&chain);
    advanced.append(&centre);
    advanced.append(&panel);
    advanced.append(keyboard.widget());

    // Sing mode: the easy page. One big switch, one Polish knob, a voice.
    let popstar = gtk::ToggleButton::with_label("MAKE ME A POP STAR");
    popstar.add_css_class("popstar");
    popstar.set_halign(gtk::Align::Center);
    popstar.set_tooltip_text(Some("Tuning, clean-up, shine, doubling and reverb in one click"));
    let polish = Knob::new("Polish", 0.0, 1.0, 0.7, 170, percent);
    polish.widget().set_valign(gtk::Align::Center);
    let sing_meter = PitchMeter::new(250);

    let voice_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
    voice_box.set_valign(gtk::Align::Center);
    let voice_title = gtk::Label::new(Some("VOICE"));
    voice_title.add_css_class("section-title");
    voice_box.append(&voice_title);
    let voice = Rc::new(Cell::new(Voice::PopPrincess));
    let voice_buttons: Vec<gtk::ToggleButton> = Voice::ALL
        .iter()
        .map(|v| {
            let b = gtk::ToggleButton::with_label(v.name());
            b.add_css_class("voice");
            voice_box.append(&b);
            b
        })
        .collect();
    for b in &voice_buttons[1..] {
        b.set_group(Some(&voice_buttons[0]));
    }
    for (b, v) in voice_buttons.iter().zip(Voice::ALL) {
        b.set_active(v == voice.get());
    }

    let sing_row = gtk::Box::new(gtk::Orientation::Horizontal, 48);
    sing_row.set_halign(gtk::Align::Center);
    sing_row.append(polish.widget());
    sing_row.append(sing_meter.widget());
    sing_row.append(&voice_box);

    // Pitch correction: how hard notes are pulled onto the scale.
    let correction_scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 0.01);
    correction_scale.add_css_class("correction");
    correction_scale.set_size_request(420, -1);
    correction_scale.set_value(correction.value());
    correction_scale.add_mark(0.0, gtk::PositionType::Bottom, Some("Off"));
    correction_scale.add_mark(0.5, gtk::PositionType::Bottom, Some("Gentle"));
    correction_scale.add_mark(1.0, gtk::PositionType::Bottom, Some("Exact"));
    correction_scale.set_tooltip_text(Some("How far your notes are pulled onto the right pitch"));
    let correction_value = gtk::Label::new(Some(&percent(correction.value())));
    correction_value.add_css_class("correction-value");
    correction_value.set_width_chars(5);
    correction_value.set_xalign(0.0);
    {
        let (correction_scale, correction_value) = (correction_scale.clone(), correction_value.clone());
        correction.connect_changed(move |v| {
            correction_scale.set_value(v);
            correction_value.set_text(&percent(v));
        });
    }
    {
        let (correction, tune_on) = (correction.clone(), tune_on.clone());
        correction_scale.connect_value_changed(move |s| {
            correction.set_value(s.value());
            // Moving the slider up means "tune me", even with Sing mode off.
            if s.value() > 0.0 && !tune_on.is_active() {
                tune_on.set_active(true);
            }
        });
    }
    let correction_label = gtk::Label::new(Some("Pitch correction"));
    correction_label.add_css_class("field-title");
    let correction_row = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    correction_row.set_halign(gtk::Align::Center);
    correction_row.append(&correction_label);
    correction_row.append(&correction_scale);
    correction_row.append(&correction_value);

    let auto_switch = gtk::Switch::new();
    auto_switch.set_valign(gtk::Align::Center);
    auto_switch
        .bind_property("active", &auto_key, "active")
        .bidirectional()
        .sync_create()
        .build();
    let auto_label = gtk::Label::new(Some("Auto key"));
    auto_label.add_css_class("field-title");
    let key_label = gtk::Label::new(Some("Key: C major"));
    key_label.add_css_class("key-label");
    key_label.set_width_chars(24);
    key_label.set_xalign(0.0);
    let key_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    key_row.set_halign(gtk::Align::Center);
    key_row.append(&auto_switch);
    key_row.append(&auto_label);
    key_row.append(&separator());
    key_row.append(&key_label);

    let headphones = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    headphones.add_css_class("headphones");
    headphones.set_halign(gtk::Align::Center);
    headphones.append(&gtk::Image::from_icon_name("audio-headphones-symbolic"));
    headphones.append(&gtk::Label::new(Some(
        "Use headphones: speakers feed your voice back into the microphone and howl.",
    )));

    let sing_hint = gtk::Label::new(Some(
        "Press the big button and sing. Turn Polish up for a tighter, shinier, studio sound.\n\
         Auto key listens for a few seconds, then keeps you on the notes of your song.",
    ));
    sing_hint.add_css_class("sing-hint");
    sing_hint.set_justify(gtk::Justification::Center);

    let sing = gtk::Box::new(gtk::Orientation::Vertical, 22);
    sing.set_valign(gtk::Align::Center);
    sing.set_vexpand(true);
    sing.set_margin_top(18);
    sing.set_margin_bottom(18);
    sing.append(&headphones);
    sing.append(&popstar);
    sing.append(&sing_row);
    sing.append(&correction_row);
    sing.append(&key_row);
    sing.append(&sing_hint);

    stack.add_titled(&sing, Some("sing"), "Sing");
    stack.add_titled(&advanced, Some("advanced"), "Advanced");
    stack.set_vexpand(true);
    {
        // Presets belong to the advanced view.
        let preset_dd = preset_dd.clone();
        let sync = move |s: &gtk::Stack| {
            preset_dd.set_visible(s.visible_child_name().as_deref() == Some("advanced"));
        };
        sync(&stack);
        stack.connect_visible_child_name_notify(sync);
    }

    let feedback = gtk::Label::new(Some(
        "Feedback detected: output turned down. Use headphones or lower the volume.",
    ));
    feedback.add_css_class("feedback");
    feedback.set_visible(false);

    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.append(&feedback);
    root.append(&stack);
    root.append(&footer);
    window.set_child(Some(&root));

    let controls = Rc::new(Controls {
        tune_on,
        pitch_on,
        dist_on,
        verb_on,
        gate_on,
        comp_on,
        bright_on,
        double_on,
        retune,
        correction,
        pitch,
        drive,
        tone,
        dist_mix,
        room,
        damping,
        verb_mix,
        gate,
        comp_threshold,
        comp_ratio,
        brightness,
        double_mix,
        double_detune,
        key: key_dd.clone(),
        scale: Rc::clone(&scale),
    });
    {
        let controls = Rc::clone(&controls);
        preset_dd.connect_selected_notify(move |d| controls.apply(&PRESETS[d.selected() as usize]));
    }

    // Sing mode drives every effect through the same parameters the
    // advanced controls use, then shows the result there.
    let apply_sing = {
        let (params, controls, popstar, polish) =
            (Arc::clone(&params), Rc::clone(&controls), popstar.clone(), polish.clone());
        let voice = Rc::clone(&voice);
        Rc::new(move || {
            if popstar.is_active() {
                apply_sing_mode(&params, voice.get(), polish.value() as f32);
            } else {
                bypass_all(&params);
            }
            controls.sync(&params);
        })
    };
    {
        let (apply_sing, power) = (Rc::clone(&apply_sing), power.clone());
        popstar.connect_toggled(move |b| {
            apply_sing();
            // One click should be enough: start the audio too.
            if b.is_active() && !power.is_active() {
                power.set_active(true);
            }
        });
    }
    {
        let (apply_sing, popstar) = (Rc::clone(&apply_sing), popstar.clone());
        polish.connect_changed(move |_| {
            if popstar.is_active() {
                apply_sing();
            }
        });
    }
    for (b, v) in voice_buttons.iter().zip(Voice::ALL) {
        let (apply_sing, popstar, voice) = (Rc::clone(&apply_sing), popstar.clone(), Rc::clone(&voice));
        b.connect_toggled(move |b| {
            if !b.is_active() {
                return;
            }
            voice.set(v);
            if popstar.is_active() {
                apply_sing();
            } else {
                popstar.set_active(true);
            }
        });
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
        let gr_level = Cell::new(0.0f32);
        let shown_key = Cell::new(Key::NONE);
        let key_dd = key_dd.clone();
        glib::timeout_add_local(Duration::from_millis(33), move || {
            gr_level.set(meters.reduction.take().max(gr_level.get() * 0.85));
            reduction.set_value(gr_level.get().min(20.0) as f64);
            feedback.set_visible(meters.feedback.get());

            // Auto key: show the detected key in the scale panel too, so
            // switching Auto off keeps it.
            let detected = meters.key.load(Ordering::Relaxed);
            if params.auto_key.get() {
                if detected != shown_key.get() {
                    shown_key.set(detected);
                    if let Some(k) = Key::decode(detected) {
                        scale.set_scale(k.root, k.scale());
                        key_dd.set_selected(k.root);
                    }
                }
                key_label.set_text(&match Key::decode(detected) {
                    Some(k) => format!("Key: {} (detected)", k.name()),
                    None => "Key: listening...".to_string(),
                });
            } else {
                shown_key.set(Key::NONE);
                let name = match scale.scale.get() {
                    MusicScale::Major => "major",
                    MusicScale::Minor => "minor",
                    MusicScale::Pentatonic => "pentatonic",
                    MusicScale::Chromatic => "chromatic",
                };
                key_label.set_text(&format!("Key: {} {name}", NOTE_NAMES[scale.key.get() as usize % 12]));
            }

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
                sing_meter.update(true, letter, shown_cents.get(), hz as f64);
                keyboard.set_current(Some(target as i32));
            } else {
                meter.update(false, "-", 0.0, 0.0);
                sing_meter.update(false, "-", 0.0, 0.0);
                keyboard.set_current(None);
            }
            glib::ControlFlow::Continue
        });
    }

    window.present();
}
