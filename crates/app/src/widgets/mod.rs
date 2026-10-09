//! Custom drawn widgets for the console look.

mod keyboard;
mod knob;
mod meter;

pub use keyboard::Keyboard;
pub use knob::Knob;
pub use meter::PitchMeter;

use gtk::cairo;

pub const ACCENT: (f64, f64, f64) = (0.29, 0.62, 1.0);
pub const ORANGE: (f64, f64, f64) = (0.94, 0.60, 0.19);
pub const MUTED: (f64, f64, f64) = (0.36, 0.38, 0.42);
pub const PINK: (f64, f64, f64) = (0.88, 0.28, 0.62);

fn rgb(cr: &cairo::Context, c: (f64, f64, f64)) {
    cr.set_source_rgb(c.0, c.1, c.2);
}

/// Draws `text` centred on (x, y).
fn centered_text(cr: &cairo::Context, text: &str, x: f64, y: f64) {
    if let Ok(te) = cr.text_extents(text) {
        cr.move_to(
            x - te.width() / 2.0 - te.x_bearing(),
            y - te.height() / 2.0 - te.y_bearing(),
        );
        let _ = cr.show_text(text);
    }
    // show_text leaves a current point that the next arc would connect to.
    cr.new_path();
}
