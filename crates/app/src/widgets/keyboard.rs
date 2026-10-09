use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::cairo;
use gtk::prelude::*;

use super::{centered_text, rgb, ACCENT, ORANGE};

/// MIDI range shown: C2..C6.
const LOW: i32 = 36;
const HIGH: i32 = 84;

fn is_black(n: i32) -> bool {
    matches!(n.rem_euclid(12), 1 | 3 | 6 | 8 | 10)
}

#[derive(Default)]
struct State {
    mask: Cell<u32>,
    current: Cell<Option<i32>>,
    on_toggle: RefCell<Option<Box<dyn Fn(u32)>>>,
}

/// Piano keyboard: notes in the scale are orange, the sung note is blue.
/// Clicking a key toggles its pitch class in the scale.
#[derive(Clone)]
pub struct Keyboard {
    area: gtk::DrawingArea,
    state: Rc<State>,
}

struct Geometry {
    white_w: f64,
    black_w: f64,
    black_h: f64,
}

impl Geometry {
    fn new(w: f64, h: f64) -> Self {
        let whites = (LOW..=HIGH).filter(|&n| !is_black(n)).count() as f64;
        let white_w = w / whites;
        Self { white_w, black_w: white_w * 0.6, black_h: h * 0.62 }
    }

    /// Left edge of a white key, or of the white key a black key sits after.
    fn white_index(n: i32) -> f64 {
        (LOW..n).filter(|&m| !is_black(m)).count() as f64
    }

    fn black_x(&self, n: i32) -> f64 {
        Self::white_index(n) * self.white_w - self.black_w / 2.0
    }

    fn key_at(&self, x: f64, y: f64) -> Option<i32> {
        if y < self.black_h {
            let hit = (LOW..=HIGH)
                .filter(|&n| is_black(n))
                .find(|&n| x >= self.black_x(n) && x < self.black_x(n) + self.black_w);
            if hit.is_some() {
                return hit;
            }
        }
        let idx = (x / self.white_w).floor() as usize;
        (LOW..=HIGH).filter(|&n| !is_black(n)).nth(idx)
    }
}

impl Keyboard {
    pub fn new(height: i32) -> Self {
        let area = gtk::DrawingArea::new();
        area.set_content_height(height);
        area.set_hexpand(true);
        let state = Rc::new(State::default());

        let s = Rc::clone(&state);
        area.set_draw_func(move |_, cr, w, h| draw(&s, cr, w as f64, h as f64));

        let click = gtk::GestureClick::new();
        let (s, a) = (Rc::downgrade(&state), area.downgrade());
        click.connect_pressed(move |_, _, x, y| {
            let (Some(s), Some(a)) = (s.upgrade(), a.upgrade()) else { return };
            let geo = Geometry::new(a.width() as f64, a.height() as f64);
            if let Some(n) = geo.key_at(x, y) {
                if let Some(cb) = s.on_toggle.borrow().as_ref() {
                    cb(n.rem_euclid(12) as u32);
                }
            }
        });
        area.add_controller(click);

        Self { area, state }
    }

    pub fn widget(&self) -> &gtk::DrawingArea {
        &self.area
    }

    pub fn set_mask(&self, mask: u32) {
        self.state.mask.set(mask);
        self.area.queue_draw();
    }

    pub fn set_current(&self, note: Option<i32>) {
        if self.state.current.get() != note {
            self.state.current.set(note);
            self.area.queue_draw();
        }
    }

    /// Called with the pitch class (0 = C) of a clicked key.
    pub fn connect_toggle(&self, f: impl Fn(u32) + 'static) {
        self.state.on_toggle.replace(Some(Box::new(f)));
    }
}

fn draw(s: &State, cr: &cairo::Context, w: f64, h: f64) {
    let geo = Geometry::new(w, h);
    let mask = s.mask.get();
    let current = s.current.get();
    let in_scale = |n: i32| mask & (1 << n.rem_euclid(12)) != 0;

    cr.select_font_face("Sans", cairo::FontSlant::Normal, cairo::FontWeight::Bold);
    cr.set_font_size(10.0);
    for (i, n) in (LOW..=HIGH).filter(|&n| !is_black(n)).enumerate() {
        let x = i as f64 * geo.white_w;
        cr.rectangle(x + 0.5, 0.0, geo.white_w - 1.0, h);
        if current == Some(n) {
            rgb(cr, ACCENT);
        } else if in_scale(n) {
            rgb(cr, ORANGE);
        } else {
            cr.set_source_rgb(0.93, 0.93, 0.93);
        }
        let _ = cr.fill();
        if n % 12 == 0 {
            cr.set_source_rgb(0.15, 0.15, 0.17);
            centered_text(cr, &format!("C{}", n / 12 - 1), x + geo.white_w / 2.0, h - 10.0);
        }
    }

    for n in (LOW..=HIGH).filter(|&n| is_black(n)) {
        cr.rectangle(geo.black_x(n), 0.0, geo.black_w, geo.black_h);
        if current == Some(n) {
            rgb(cr, ACCENT);
        } else if in_scale(n) {
            cr.set_source_rgb(0.72, 0.42, 0.08);
        } else {
            cr.set_source_rgb(0.12, 0.12, 0.14);
        }
        let _ = cr.fill_preserve();
        cr.set_source_rgb(0.05, 0.05, 0.06);
        cr.set_line_width(1.0);
        let _ = cr.stroke();
    }
}
