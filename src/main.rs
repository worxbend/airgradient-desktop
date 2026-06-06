//! Binary entry point.
//!
//! Rust programs start in `main()`. This file only declares the project modules
//! and delegates real application startup to `app::run()`, keeping the entry
//! point easy to scan.

mod app;
mod config;
mod sensors;
mod state;
mod ui;

fn main() {
    app::run();
}
