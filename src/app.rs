//! Application bootstrap.
//!
//! This module creates the libadwaita application object, loads persisted user
//! configuration, and hands control to the UI module. Keeping this separate from
//! `ui::app` makes startup easier to understand: this file owns the application
//! lifecycle, while `src/ui/app.rs` owns windows and widgets.

use std::cell::RefCell;
use std::rc::Rc;

use adw;
use adw::prelude::*;

use crate::config::read_config;
use crate::state::AppState;
use crate::ui;

const APP_ID: &str = "com.airgradient.desktop";

pub fn run() {
    let app = adw::Application::builder().application_id(APP_ID).build();

    // GTK applications build their UI from the `activate` signal. The callback
    // runs on the GTK main thread, which is also where all widget updates must
    // happen.
    app.connect_activate(build_ui);
    app.run();
}

fn build_ui(app: &adw::Application) {
    let config = read_config().ok().unwrap_or_default();
    let server_url = config.server_url;
    let refresh_interval_secs = config.refresh_interval_secs;
    // `Rc<RefCell<_>>` is the common gtk-rs pattern for shared mutable state on
    // the main thread. `Rc` lets signal handlers hold references to the same
    // state, and `RefCell` performs borrow checks at runtime.
    let state = Rc::new(RefCell::new(AppState::new(
        server_url,
        refresh_interval_secs,
    )));
    let window = ui::build_main_window(app, state);
    window.present();
}
