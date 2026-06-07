//! Measurement fetch orchestration for the GTK UI.
//!
//! The window module owns widgets and timers. This module owns the asynchronous
//! fetch flow, including status text, alert evaluation, and notification
//! delivery.

use std::time::Instant;

use super::app::UiContext;
use crate::device::fetch_current_measurements;
use crate::notifications::send_air_quality_notification;

pub(super) fn trigger_fetch_current_measures(ui: UiContext) {
    let base_url = match ui.state.borrow().server_url.clone() {
        Some(url) => url,
        None => {
            ui.dashboard_widgets
                .fetch_status_label
                .set_text("No server URL configured.");
            return;
        }
    };

    ui.dashboard_widgets
        .server_label
        .set_text(&format!("Server URL: {base_url}"));
    ui.dashboard_widgets
        .fetch_status_label
        .set_text("Fetching measurements...");

    glib::MainContext::default().spawn_local(async move {
        // `reqwest::blocking` would freeze the GTK main loop if called directly.
        // `gio::spawn_blocking` runs it on a worker thread and resumes here with
        // the result so the UI can be updated safely.
        let result = gio::spawn_blocking(move || fetch_current_measurements(&base_url))
            .await
            .map_err(|_| "Background fetch task failed to execute.".to_string())
            .and_then(|result| result.map_err(|err| err.to_string()));
        let now = Instant::now();

        match result {
            Ok(measurement) => {
                ui.dashboard_widgets.apply_measurements(&measurement);
                let alerts = ui.alert_monitor.borrow_mut().evaluate_at(&measurement, now);
                for alert in alerts {
                    if let Err(err) = send_air_quality_notification(&ui.app, alert) {
                        eprintln!("System notification failed: {err}");
                    }
                }
                *ui.last_updated.borrow_mut() = Some(now);
                super::app::update_last_updated_text(&ui.last_updated, &ui.last_updated_label);
                ui.dashboard_widgets
                    .fetch_status_label
                    .set_text("Latest measurements loaded.");
            }
            Err(err) => {
                ui.dashboard_widgets
                    .fetch_status_label
                    .set_text(&format!("Fetch failed: {err}"));
                if let Some(alert) = ui
                    .alert_monitor
                    .borrow_mut()
                    .record_fetch_error_at(&err, now)
                {
                    if let Err(err) = send_air_quality_notification(&ui.app, alert) {
                        eprintln!("System notification failed: {err}");
                    }
                }
            }
        }
    });
}
