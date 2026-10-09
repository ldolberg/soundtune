use std::cell::{Cell, RefCell};
use std::f64::consts::PI;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{cairo, glib};

use super::{rgb, ACCENT, MUTED};

const START: f64 = 0.75 * PI;
const SWEEP: f64 = 1.5 * PI;

/// Called with the new value whenever the knob changes.
type ValueCallback = Box<dyn Fn(f64)>;

struct Inner {
    value: Cell<f64>,
    default: Cell<f64>,
    min: f64,
    max: f64,
    step: Cell<f64>,
    bipolar: Cell<bool>,
    format: Box<dyn Fn(f64) -> String>,
    callbacks: RefCell<Vec<ValueCallback>>,
    area: gtk::DrawingArea,
    label: gtk::Label,
}

impl Inner {
    fn set(&self, v: f64) {
        let mut v = v.clamp(self.min, self.max);
        let step = self.step.get();
        if step > 0.0 {
            v = (v / step).round() * step;
        }
        if v == self.value.get() {
            return;
        }
        self.value.set(v);
        self.label.set_text(&(self.format)(v));
        self.area.queue_draw();
        for cb in self.callbacks.borrow().iter() {
            cb(v);
        }
    }

    fn draw(&self, cr: &cairo::Context, w: f64, h: f64) {
        let (cx, cy) = (w / 2.0, h / 2.0);
        let r = w.min(h) / 2.0 - 4.0;
        let active = self.area.is_sensitive();
        let t = (self.value.get() - self.min) / (self.max - self.min);
        let angle = START + SWEEP * t;

        cr.set_line_cap(cairo::LineCap::Round);
        cr.set_line_width((r * 0.05).max(2.0));
        cr.set_source_rgb(0.25, 0.27, 0.30);
        cr.arc(cx, cy, r, START, START + SWEEP);
        let _ = cr.stroke();

        let from = if self.bipolar.get() {
            START + SWEEP / 2.0
        } else {
            START
        };
        let (a0, a1) = if angle < from {
            (angle, from)
        } else {
            (from, angle)
        };
        if a1 - a0 > 1e-3 {
            rgb(cr, if active { ACCENT } else { MUTED });
            cr.arc(cx, cy, r, a0, a1);
            let _ = cr.stroke();
        }

        // Body with a soft top-left highlight.
        let br = r * 0.78;
        let grad = cairo::RadialGradient::new(cx - br * 0.35, cy - br * 0.35, br * 0.1, cx, cy, br);
        grad.add_color_stop_rgb(0.0, 0.25, 0.27, 0.30);
        grad.add_color_stop_rgb(1.0, 0.11, 0.12, 0.14);
        let _ = cr.set_source(&grad);
        cr.arc(cx, cy, br, 0.0, 2.0 * PI);
        let _ = cr.fill_preserve();
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.7);
        cr.set_line_width(1.5);
        let _ = cr.stroke();

        cr.set_line_width((r * 0.07).max(2.0));
        if active {
            cr.set_source_rgb(0.95, 0.95, 0.95);
        } else {
            rgb(cr, MUTED);
        }
        let (c, s) = (angle.cos(), angle.sin());
        cr.move_to(cx + c * br * 0.45, cy + s * br * 0.45);
        cr.line_to(cx + c * br * 0.85, cy + s * br * 0.85);
        let _ = cr.stroke();
    }
}

/// Rotary knob: drag up/down or scroll to change, double click to reset.
#[derive(Clone)]
pub struct Knob {
    root: gtk::Box,
    header: gtk::Box,
    inner: Rc<Inner>,
}

impl Knob {
    pub fn new(
        title: &str,
        min: f64,
        max: f64,
        value: f64,
        size: i32,
        format: impl Fn(f64) -> String + 'static,
    ) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 4);
        root.set_halign(gtk::Align::Center);
        root.add_css_class("knob");
        let big = size >= 90;

        let header = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        header.set_halign(gtk::Align::Center);
        let title_label = gtk::Label::new(Some(title));
        title_label.add_css_class("knob-title");
        if big {
            title_label.add_css_class("big");
        }
        header.append(&title_label);

        let area = gtk::DrawingArea::new();
        area.set_content_width(size);
        area.set_content_height(size);
        area.set_halign(gtk::Align::Center);

        let label = gtk::Label::new(Some(&format(value)));
        label.add_css_class("knob-value");
        if big {
            label.add_css_class("big");
        }

        root.append(&header);
        root.append(&area);
        root.append(&label);

        let inner = Rc::new(Inner {
            value: Cell::new(value),
            default: Cell::new(value),
            min,
            max,
            step: Cell::new(0.0),
            bipolar: Cell::new(false),
            format: Box::new(format),
            callbacks: RefCell::new(Vec::new()),
            area: area.clone(),
            label,
        });

        // The draw func owns the state, so the knob lives as long as its widget.
        let strong = Rc::clone(&inner);
        area.set_draw_func(move |_, cr, w, h| strong.draw(cr, w as f64, h as f64));

        // Full range over ~200 px of vertical drag.
        let drag = gtk::GestureDrag::new();
        let start = Rc::new(Cell::new(value));
        {
            let (weak, start) = (Rc::downgrade(&inner), Rc::clone(&start));
            drag.connect_drag_begin(move |_, _, _| {
                if let Some(i) = weak.upgrade() {
                    start.set(i.value.get());
                }
            });
        }
        {
            let weak = Rc::downgrade(&inner);
            drag.connect_drag_update(move |_, _, dy| {
                if let Some(i) = weak.upgrade() {
                    i.set(start.get() - dy * (i.max - i.min) / 200.0);
                }
            });
        }
        area.add_controller(drag);

        let click = gtk::GestureClick::new();
        {
            let weak = Rc::downgrade(&inner);
            click.connect_pressed(move |_, n, _, _| {
                if let Some(i) = weak.upgrade() {
                    if n == 2 {
                        i.set(i.default.get());
                    }
                }
            });
        }
        area.add_controller(click);

        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        {
            let weak = Rc::downgrade(&inner);
            scroll.connect_scroll(move |_, _, dy| {
                if let Some(i) = weak.upgrade() {
                    let step = if i.step.get() > 0.0 {
                        i.step.get()
                    } else {
                        (i.max - i.min) / 50.0
                    };
                    i.set(i.value.get() - dy.signum() * step);
                }
                glib::Propagation::Stop
            });
        }
        area.add_controller(scroll);

        Self {
            root,
            header,
            inner,
        }
    }

    /// Draws the value arc from the centre; double click resets to the centre.
    pub fn bipolar(self) -> Self {
        self.inner.bipolar.set(true);
        self.inner
            .default
            .set((self.inner.min + self.inner.max) / 2.0);
        self
    }

    /// Quantises values to multiples of `step`.
    pub fn stepped(self, step: f64) -> Self {
        self.inner.step.set(step);
        self
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    /// Puts a widget (for example a power button) before the title.
    pub fn prepend_header(&self, w: &impl IsA<gtk::Widget>) {
        self.header.prepend(w);
    }

    pub fn value(&self) -> f64 {
        self.inner.value.get()
    }

    pub fn set_value(&self, v: f64) {
        self.inner.set(v);
    }

    pub fn connect_changed(&self, f: impl Fn(f64) + 'static) {
        self.inner.callbacks.borrow_mut().push(Box::new(f));
    }

    /// Greys the knob out (and blocks input) when its effect is off.
    pub fn set_active(&self, active: bool) {
        self.inner.area.set_sensitive(active);
        self.inner.label.set_sensitive(active);
        self.inner.area.queue_draw();
    }
}
