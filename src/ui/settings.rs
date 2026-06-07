//! Settings page construction and persistence callbacks.

use std::cell::RefCell;
use std::rc::Rc;

use adw::{prelude::*, ActionRow, ComboRow, EntryRow, PreferencesGroup, PreferencesPage, SpinRow};
use gtk4::{Align, Button, Image, StringList, Switch};

use super::app::{
    apply_color_scheme, start_auto_refresh_timer, theme_mode_index, NavigationContext,
};
use crate::{
    alerts::{AlertNotification, AlertSeverity},
    config::{
        self, AppConfig, RefreshInterval, MAX_REFRESH_INTERVAL_SECS, MIN_REFRESH_INTERVAL_SECS,
    },
    device::DeviceBaseUrl,
    notifications::send_air_quality_notification,
    state::{Page, ThemeMode},
};

pub(super) fn build_settings_page(
    navigation: NavigationContext,
    auto_refresh_source: Rc<RefCell<Option<glib::SourceId>>>,
) -> gtk4::Widget {
    let page = PreferencesPage::builder()
        .title("Settings")
        .icon_name("preferences-system-symbolic")
        .build();

    let theme_options = StringList::new(&["System", "Light", "Dark"]);
    let current_mode = navigation.ui.state.borrow().theme_mode;
    let theme_row = ComboRow::builder()
        .title("Style")
        .subtitle("Use the system preference or force a light or dark appearance")
        .model(&theme_options)
        .selected(theme_mode_index(current_mode))
        .build();
    theme_row.connect_selected_notify({
        let style_manager = adw::StyleManager::default();
        let state = navigation.ui.state.clone();
        move |row| {
            let theme_mode = match row.selected() {
                1 => ThemeMode::Light,
                2 => ThemeMode::Dark,
                _ => ThemeMode::System,
            };
            {
                let mut model = state.borrow_mut();
                model.theme_mode = theme_mode;
            }
            // Theme changes apply immediately. They are not persisted yet; the
            // current app state is enough for this session.
            apply_color_scheme(&style_manager, theme_mode);
        }
    });

    let url_row = EntryRow::builder()
        .title("Local-server Base URL")
        .text(
            navigation
                .ui
                .state
                .borrow()
                .server_url()
                .unwrap_or_default(),
        )
        .build();
    let url_icon = Image::from_icon_name("network-wired-symbolic");
    url_row.add_prefix(&url_icon);

    let refresh_row = SpinRow::with_range(MIN_REFRESH_INTERVAL_SECS as f64, 3600.0, 1.0);
    refresh_row.set_title("Refresh Interval");
    refresh_row.set_subtitle("Seconds between automatic measurement updates");
    refresh_row.set_value(navigation.ui.state.borrow().refresh_interval.as_secs() as f64);
    refresh_row.set_numeric(true);
    refresh_row.set_tooltip_text(Some(
        "Refresh interval in seconds. Minimum value is 5 seconds.",
    ));

    let notifications_row = ActionRow::builder()
        .title("Air Quality Notifications")
        .subtitle("Notify when CO2, AQI, particles, VOC, NOx, or humidity need attention")
        .build();
    let notifications_switch = Switch::builder()
        .valign(Align::Center)
        .active(navigation.ui.state.borrow().notifications_enabled)
        .build();
    notifications_row.add_suffix(&notifications_switch);
    notifications_row.set_activatable_widget(Some(&notifications_switch));

    let start_minimized_row = ActionRow::builder()
        .title("Start Minimized")
        .subtitle("Start hidden and keep polling in the background on next launch")
        .build();
    let start_minimized_switch = Switch::builder()
        .valign(Align::Center)
        .active(navigation.ui.state.borrow().start_minimized)
        .build();
    start_minimized_row.add_suffix(&start_minimized_switch);
    start_minimized_row.set_activatable_widget(Some(&start_minimized_switch));

    let test_notification_row = ActionRow::builder()
        .title("Test Notification")
        .subtitle("Send a sample alert and test click-to-open behavior")
        .build();
    let test_notification_button = Button::builder()
        .label("Send")
        .valign(Align::Center)
        .build();
    test_notification_button.add_css_class("suggested-action");
    test_notification_row.add_suffix(&test_notification_button);
    test_notification_row.set_activatable_widget(Some(&test_notification_button));
    test_notification_button.connect_clicked({
        let app = navigation.ui.app.clone();
        let test_notification_row = test_notification_row.clone();
        move |_| {
            let result = send_air_quality_notification(
                &app,
                AlertNotification {
                    id: "airgradient-test-notification".into(),
                    title: "Air Monitor test notification".into(),
                    body:
                        "Notifications are working. Click this notification to open the dashboard."
                            .into(),
                    severity: AlertSeverity::Notice,
                },
            );
            match result {
                Ok(()) => test_notification_row.set_subtitle("Test notification sent."),
                Err(err) => {
                    test_notification_row.set_subtitle(&format!("Test notification failed: {err}"))
                }
            }
        }
    });

    let status_row = ActionRow::builder()
        .title("Status")
        .subtitle(
            navigation
                .ui
                .state
                .borrow()
                .startup_notice
                .as_ref()
                .map(|notice| notice.user_message())
                .unwrap_or_else(|| "Enter a URL like http://192.168.1.201".to_string()),
        )
        .build();

    let save_row = ActionRow::builder()
        .title("Save Settings")
        .subtitle("Save the server URL and restart the refresh timer")
        .activatable(true)
        .build();
    save_row.add_suffix(&Image::from_icon_name("document-save-symbolic"));

    save_row.connect_activated({
        let navigation = navigation.clone();
        let url_row = url_row.clone();
        let refresh_row = refresh_row.clone();
        let notifications_switch = notifications_switch.clone();
        let start_minimized_switch = start_minimized_switch.clone();
        let auto_refresh_source = auto_refresh_source.clone();
        let status_row = status_row.clone();

        move |_| {
            // The UI stores a user-entered string, while the config file stores
            // a normalized base URL. Keeping normalization here makes fetch
            // code simpler and avoids saving unusable values.
            let server_url = match DeviceBaseUrl::parse(&url_row.text()) {
                Ok(url) => url,
                Err(err) => {
                    status_row.set_subtitle(&format!("Invalid URL: {err}"));
                    return;
                }
            };

            let raw_interval = (refresh_row.value().round() as u64)
                .clamp(MIN_REFRESH_INTERVAL_SECS, MAX_REFRESH_INTERVAL_SECS);
            let refresh_interval = match RefreshInterval::new(raw_interval) {
                Ok(interval) => interval,
                Err(err) => {
                    status_row.set_subtitle(&format!("Invalid interval: {err}"));
                    return;
                }
            };
            let config = AppConfig {
                server_url,
                refresh_interval,
                notifications_enabled: notifications_switch.is_active(),
                start_minimized: start_minimized_switch.is_active(),
            };

            if let Err(err) = config::write_config(&config) {
                status_row.set_subtitle(&format!("Failed to save: {err}"));
                return;
            }

            {
                let mut model = navigation.ui.state.borrow_mut();
                model.set_device_base_url(config.server_url.clone());
                model.set_refresh_interval(config.refresh_interval);
                model.set_notifications_enabled(config.notifications_enabled);
                model.set_start_minimized(config.start_minimized);
            }
            navigation
                .ui
                .alert_monitor
                .borrow_mut()
                .set_enabled(config.notifications_enabled);

            let has_url = config.server_url.is_some();
            if has_url {
                navigation
                    .stack
                    .set_visible_child_name(Page::Dashboard.id());
                navigation
                    .ui
                    .dashboard_widgets
                    .server_label
                    .set_text(&format!(
                        "Server URL: {}",
                        config
                            .server_url
                            .as_ref()
                            .map(DeviceBaseUrl::as_str)
                            .unwrap_or_default()
                    ));
                status_row.set_subtitle("Saved. Refreshing dashboard.");
            } else {
                navigation.stack.set_visible_child_name(Page::Welcome.id());
                navigation
                    .ui
                    .dashboard_widgets
                    .server_label
                    .set_text("Server URL: Not configured");
                navigation
                    .ui
                    .dashboard_widgets
                    .fetch_status_label
                    .set_text("Server URL removed.");
                status_row.set_subtitle("Cleared URL. Returning to Welcome.");
            }

            start_auto_refresh_timer(navigation.ui.clone(), auto_refresh_source.clone());
            {
                let mut last = navigation.ui.last_updated.borrow_mut();
                *last = None;
            }

            if has_url {
                // Saving a valid URL should make the dashboard useful
                // immediately, so fetch once instead of waiting for the next
                // interval tick.
                navigation.ui.trigger_fetch();
            }
        }
    });

    let appearance_group = PreferencesGroup::builder()
        .title("Appearance")
        .description("GNOME apps should follow the system style by default.")
        .build();
    appearance_group.add(&theme_row);

    let server_group = PreferencesGroup::builder()
        .title("Device")
        .description("Configure the AirGradient local-server endpoint.")
        .build();
    server_group.add(&url_row);
    server_group.add(&refresh_row);
    server_group.add(&notifications_row);
    server_group.add(&start_minimized_row);
    server_group.add(&test_notification_row);
    server_group.add(&save_row);

    let status_group = PreferencesGroup::new();
    status_group.add(&status_row);

    page.add(&appearance_group);
    page.add(&server_group);
    page.add(&status_group);

    page.upcast()
}
