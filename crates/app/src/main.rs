// Hide the console window for release builds on Windows.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod audio;
mod widgets;
mod ui;

use gtk::glib;
use gtk::prelude::*;

const APP_ID: &str = "io.github.soundtune.SoundTune";

fn main() -> glib::ExitCode {
    let app = gtk::Application::builder().application_id(APP_ID).build();
    app.connect_startup(|_| ui::load_css());
    app.connect_activate(ui::build);
    app.run()
}
