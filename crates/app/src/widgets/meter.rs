use std::cell::{Cell, RefCell};
use std::f64::consts::PI;
use std::rc::Rc;

use gtk::cairo;
use gtk::prelude::*;

use super::{centered_text, rgb, ACCENT, MUTED, ORANGE};

#[derive(Default)]
struct State {
    voiced: Cell<bool>,
    cents: Cell<f64>,
    hz: Cell<f64>,
    note: RefCell<String>,
}

/// Round tuner display: target note in the middle, deviation in cents on
/// a tick ring from -100 to +100.
#[derive(Clone)]
pub struct PitchMeter {
    area: gtk::DrawingArea,
    state: Rc<State>,
}

/// Angle of a cents value: 0 at the top, +-100 at +-135 degrees.
fn angle(cents: f64) -> f64 {
    -PI / 2.0 + cents / 100.0 * 0.75 * PI
}

impl PitchMeter {
    pub fn new(size: i32) -> Self {
        let area = gtk::DrawingArea::new();
        area.set_content_width(size);
        area.set_content_height(size);
        area.set_halign(gtk::Align::Center);
        area.set_valign(gtk::Align::Center);
        let state = Rc::new(State::default());
        state.note.replace("-".into());
        let s = Rc::clone(&state);
        area.set_draw_func(move |_, cr, w, h| draw(&s, cr, w as f64, h as f64));
        Self { area, state }
    }

    pub fn widget(&self) -> &gtk::DrawingArea {
        &self.area
    }

    /// `note` is the target note name, `cents` how far the input is from it.
    pub fn update(&self, voiced: bool, note: &str, cents: f64, hz: f64) {
        self.state.voiced.set(voiced);
        self.state.cents.set(cents.clamp(-100.0, 100.0));
        self.state.hz.set(hz);
        if *self.state.note.borrow() != note {
            self.state.note.replace(note.to_string());
        }
        self.area.queue_draw();
    }
}

fn draw(s: &State, cr: &cairo::Context, w: f64, h: f64) {
    let (cx, cy) = (w / 2.0, h / 2.0);
    let r = w.min(h) / 2.0 - 28.0;
    let voiced = s.voiced.get();
    let cents = s.cents.get();

    // Bezel and face.
    let grad = cairo::RadialGradient::new(cx, cy - r * 0.3, r * 0.1, cx, cy, r * 1.05);
    grad.add_color_stop_rgb(0.0, 0.17, 0.18, 0.21);
    grad.add_color_stop_rgb(1.0, 0.07, 0.08, 0.09);
    let _ = cr.set_source(&grad);
    cr.arc(cx, cy, r * 1.04, 0.0, 2.0 * PI);
    let _ = cr.fill();

    // Tick ring: ticks between 0 and the current deviation light up.
    cr.set_line_cap(cairo::LineCap::Butt);
    for i in 0..=80 {
        let c = -100.0 + i as f64 * 2.5;
        let major = i % 10 == 0;
        let lit = voiced && ((c >= 0.0 && c <= cents) || (c <= 0.0 && c >= cents));
        let current = voiced && (c - cents).abs() <= 1.25;
        if current {
            cr.set_source_rgb(1.0, 0.85, 0.55);
        } else if lit {
            rgb(cr, ORANGE);
        } else if major {
            cr.set_source_rgb(0.55, 0.57, 0.60);
        } else {
            cr.set_source_rgb(0.30, 0.32, 0.35);
        }
        cr.set_line_width(if current { 3.5 } else { 2.0 });
        let a = angle(c);
        let inner = if major { r * 0.74 } else { r * 0.80 };
        cr.move_to(cx + a.cos() * inner, cy + a.sin() * inner);
        cr.line_to(cx + a.cos() * r * 0.96, cy + a.sin() * r * 0.96);
        let _ = cr.stroke();
    }

    // Scale labels outside the ring.
    cr.select_font_face("Sans", cairo::FontSlant::Normal, cairo::FontWeight::Normal);
    cr.set_font_size(11.0);
    cr.set_source_rgb(0.62, 0.64, 0.67);
    for c in (-100..=100).step_by(25) {
        let a = angle(c as f64);
        let text = if c > 0 {
            format!("+{c}")
        } else {
            c.to_string()
        };
        centered_text(
            cr,
            &text,
            cx + a.cos() * (r + 17.0),
            cy + a.sin() * (r + 17.0),
        );
    }

    // Inner face.
    cr.arc(cx, cy, r * 0.66, 0.0, 2.0 * PI);
    cr.set_source_rgb(0.12, 0.13, 0.15);
    let _ = cr.fill_preserve();
    cr.set_source_rgb(0.05, 0.05, 0.06);
    cr.set_line_width(2.0);
    let _ = cr.stroke();

    // Note name and frequency.
    cr.select_font_face("Sans", cairo::FontSlant::Normal, cairo::FontWeight::Normal);
    cr.set_font_size(r * 0.42);
    rgb(cr, if voiced { ACCENT } else { MUTED });
    centered_text(cr, &s.note.borrow(), cx, cy - r * 0.04);

    cr.set_font_size(12.0);
    cr.set_source_rgb(0.62, 0.64, 0.67);
    let info = if voiced {
        format!("{:.1} Hz  {:+.0} ct", s.hz.get(), cents)
    } else {
        "no pitch".to_string()
    };
    centered_text(cr, &info, cx, cy + r * 0.40);
}
