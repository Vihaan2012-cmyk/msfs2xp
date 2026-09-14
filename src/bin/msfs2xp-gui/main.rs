//! msfs2xp for Windows: a window over the command line converter. Convert
//! packages, fix converted textures, and validate, preview or inspect files.
#![cfg_attr(windows, windows_subsystem = "windows")]

mod logic;

#[cfg(windows)]
mod app;

#[cfg(windows)]
fn main() {
    app::run();
}

#[cfg(not(windows))]
fn main() {
    eprintln!("msfs2xp-gui runs on Windows; use the msfs2xp command line elsewhere.");
}
