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

    app.connect_activate(build_ui);
    app.run();
}

fn build_ui(app: &adw::Application) {
    let config = read_config().ok().unwrap_or_default();
    let server_url = config.server_url;
    let refresh_interval_secs = config.refresh_interval_secs;
    let state = Rc::new(RefCell::new(AppState::new(
        server_url,
        refresh_interval_secs,
    )));
    let window = ui::build_main_window(app, state);
    window.present();
}
