// Hide the console window for release builds on Windows.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod audio;
mod song;
mod songbar;
mod ui;
mod widgets;

use std::path::PathBuf;

use gtk::glib;
use gtk::prelude::*;

const APP_ID: &str = "io.github.soundtune.SoundTune";

const USAGE: &str = "Usage: soundtune [--advanced] [SONG.mp3|wav|flac|ogg] [MELODY.mid]";

fn main() -> glib::ExitCode {
    let mut opts = ui::Options::default();
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--advanced" => opts.advanced = true,
            "-h" | "--help" => {
                println!("{USAGE}");
                return glib::ExitCode::SUCCESS;
            }
            _ if arg.starts_with('-') => {
                eprintln!("unknown option {arg}\n{USAGE}");
                return glib::ExitCode::FAILURE;
            }
            _ => {
                let path = PathBuf::from(arg);
                if songbar::StartFiles::is_midi(&path) {
                    opts.files.melody = Some(path);
                } else {
                    opts.files.song = Some(path);
                }
            }
        }
    }

    let app = gtk::Application::builder().application_id(APP_ID).build();
    app.connect_startup(|_| ui::load_css());
    app.connect_activate(move |app| ui::build(app, &opts));
    // Our arguments are handled above; GTK only gets the program name.
    let argv0: Vec<String> = std::env::args().take(1).collect();
    app.run_with_args(&argv0)
}
