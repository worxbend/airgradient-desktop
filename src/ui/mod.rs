//! GTK user-interface modules.
//!
//! `ui::app` builds the top-level window. The other files provide pages and
//! reusable dashboard widgets. This module also registers CSS and embedded icon
//! resources before widgets try to use them.

mod app;
mod aqi_widget;
mod dashboard;
mod humidity_widget;
mod sensor_card;
mod temperature_widget;

pub use app::build_main_window;
pub use dashboard::{build_dashboard_page, DashboardPageWidgets};

use gtk4::{gdk, CssProvider, IconTheme};

const ICON_RESOURCE_PATH: &str = "/com/airgradient/desktop/icons";

pub fn register_resources() {
    // The macro references the file produced by `build.rs`. Registering it
    // makes the SVG icons available through GTK's resource system.
    gio::resources_register_include!("airgradient.gresource")
        .expect("AirGradient resources should be compiled into the binary");

    if let Some(display) = gdk::Display::default() {
        // Adding the resource path lets `Image::from_icon_name()` find icons
        // such as `airgradient-temperature-symbolic`.
        IconTheme::for_display(&display).add_resource_path(ICON_RESOURCE_PATH);
    }
}

pub fn load_dashboard_css() {
    let css = include_str!("../../assets/dashboard.css");
    let provider = CssProvider::new();
    provider.load_from_data(css);

    if let Some(display) = gdk::Display::default() {
        // Application priority lets our CSS override default widget styling
        // while still allowing GTK/libadwaita to provide the base theme.
        gtk4::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}
