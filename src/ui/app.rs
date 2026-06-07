//! Main GTK window and application UI flow.
//!
//! This module owns the visible application shell: header bar, page stack,
//! settings page, timers, and measurement fetching. It intentionally keeps the
//! lower-level dashboard widgets in separate modules.

use std::cell::RefCell;
use std::collections::HashMap;
use std::process::Command;
use std::rc::Rc;
use std::sync::mpsc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use adw::{
    self, prelude::*, ActionRow, ComboRow, EntryRow, HeaderBar, PreferencesGroup, PreferencesPage,
    SpinRow, StatusPage,
};
use gio;
use gtk4::{
    Align, Box as GtkBox, Button, Image, Label, Orientation, Stack, StackTransitionType, StringList,
    Switch,
};
use reqwest::blocking::Client;
use serde_json::Value;
use url::Url;

use crate::ui::{
    build_dashboard_page, load_dashboard_css, register_resources, DashboardPageWidgets,
};
use crate::{
    config::{self, AppConfig},
    sensors::{parse_air_measurements, AirMeasureSnapshot},
    state::{AppState, Page, ThemeMode},
};

const DEFAULT_WIDTH: i32 = 1180;
const DEFAULT_HEIGHT: i32 = 780;
const REQUEST_TIMEOUT_SECS: u64 = 8;
const MIN_REFRESH_INTERVAL_SECS: u64 = 5;
const ALERT_COOLDOWN_SECS: u64 = 20 * 60;
const ALERT_CONSECUTIVE_READINGS: u8 = 2;
const APP_NAME: &str = "Air Monitor";
static NOTIFICATION_SEQUENCE: AtomicU64 = AtomicU64::new(1);

type FetchResult<T> = Result<T, String>;
type SharedAlertMonitor = Rc<RefCell<AlertMonitor>>;

pub fn build_main_window(
    app: &adw::Application,
    state: Rc<RefCell<AppState>>,
    run_minimized: bool,
) -> adw::ApplicationWindow {
    let style_manager = adw::StyleManager::default();
    {
        let model = state.borrow();
        apply_color_scheme(&style_manager, model.theme_mode);
    }

    register_resources();
    load_dashboard_css();

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title(APP_NAME)
        .default_width(DEFAULT_WIDTH)
        .default_height(DEFAULT_HEIGHT)
        .build();

    let last_updated_label = Label::new(Some("Last updated: not yet"));
    last_updated_label.set_halign(Align::Start);
    // `last_updated` is shared between the fetch callback and a once-per-second
    // timer that turns the timestamp into text such as "17s ago".
    let last_updated = Rc::new(RefCell::new(None::<Instant>));
    // Store the active auto-refresh source so changing Settings can remove the
    // old timer before installing a new one.
    let auto_refresh_source = Rc::new(RefCell::new(None));

    let (dashboard_page, dashboard_widgets) = build_dashboard_page();
    let alert_monitor = Rc::new(RefCell::new(AlertMonitor::new(
        state.borrow().notifications_enabled,
    )));

    let (page_area, stack) = create_page_area(
        app.clone(),
        state.clone(),
        dashboard_page,
        dashboard_widgets.clone(),
        last_updated.clone(),
        last_updated_label.clone(),
        auto_refresh_source.clone(),
        alert_monitor.clone(),
    );

    let root = GtkBox::new(Orientation::Vertical, 0);
    root.add_css_class("app-shell");
    update_dark_shell_class(&root, style_manager.is_dark());
    // When System theme is selected, libadwaita can change between light and
    // dark while the app is running. Keep our custom root CSS class in sync.
    style_manager.connect_dark_notify({
        let root = root.clone();
        move |manager| update_dark_shell_class(&root, manager.is_dark())
    });
    root.append(&create_header_bar(
        app.clone(),
        state.clone(),
        dashboard_widgets.clone(),
        stack.clone(),
        last_updated_label.clone(),
        last_updated.clone(),
        alert_monitor.clone(),
    ));
    root.append(&page_area);
    window.set_content(Some(&root));
    install_app_actions(app, &window, &stack, state.clone());
    install_system_tray(app, &window, &stack, state.clone());
    window.connect_close_request({
        let window = window.clone();
        move |_| {
            window.hide();
            glib::Propagation::Stop
        }
    });
    // The close button now means "keep running in the background". `app.quit`
    // is the intentional exit path and can be used by a future tray menu.
    // Keep the application alive even when its only window is hidden. This
    // intentionally lives until process exit; `app.quit` remains the explicit
    // shutdown path.
    std::mem::forget(app.hold());

    if state.borrow().has_server_url() {
        // If config already contains a server URL, open directly into useful
        // data instead of waiting for manual refresh.
        trigger_fetch_current_measures(
            app.clone(),
            state.clone(),
            dashboard_widgets.clone(),
            last_updated.clone(),
            last_updated_label.clone(),
            alert_monitor.clone(),
        );
    }

    start_last_updated_timer(last_updated.clone(), last_updated_label.clone());
    start_auto_refresh_timer(
        app.clone(),
        state.clone(),
        dashboard_widgets.clone(),
        auto_refresh_source,
        last_updated.clone(),
        last_updated_label,
        alert_monitor,
    );

    if !(run_minimized || state.borrow().start_minimized) {
        window.present();
    }
    window
}

#[derive(Debug, Clone, Copy)]
enum TrayCommand {
    ShowDashboard,
    HideWindow,
    Quit,
}

struct AirMonitorTray {
    sender: mpsc::Sender<TrayCommand>,
}

impl ksni::Tray for AirMonitorTray {
    fn id(&self) -> String {
        "airgradient-desktop".into()
    }

    fn title(&self) -> String {
        APP_NAME.into()
    }

    fn icon_name(&self) -> String {
        "airgradient-desktop".into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        static TRAY_ICON: OnceLock<ksni::Icon> = OnceLock::new();

        let icon = TRAY_ICON
            .get_or_init(|| {
                let pixbuf = gdk_pixbuf::Pixbuf::from_read(std::io::Cursor::new(
                    include_bytes!("../../assets/airgradient-tray.png").as_slice(),
                ))
                .expect("embedded tray icon should be a valid PNG");
                let width = pixbuf.width();
                let height = pixbuf.height();
                let channels = pixbuf.n_channels() as usize;
                let rowstride = pixbuf.rowstride() as usize;
                let pixels = pixbuf.read_pixel_bytes();
                let pixels = pixels.as_ref();
                let mut data = Vec::with_capacity(width as usize * height as usize * 4);

                for y in 0..height as usize {
                    let row_start = y * rowstride;
                    let row_end = row_start + width as usize * channels;
                    for pixel in pixels[row_start..row_end].chunks_exact(channels) {
                        let red = pixel[0];
                        let green = pixel[1];
                        let blue = pixel[2];
                        let alpha = if channels >= 4 { pixel[3] } else { 255 };
                        data.extend_from_slice(&[alpha, red, green, blue]);
                    }
                }

                ksni::Icon {
                    width,
                    height,
                    data,
                }
            })
            .clone();

        vec![icon]
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.sender.send(TrayCommand::ShowDashboard);
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::{MenuItem, StandardItem};

        vec![
            StandardItem {
                label: "Show Dashboard".into(),
                icon_name: "go-home-symbolic".into(),
                activate: Box::new(|tray: &mut Self| {
                    let _ = tray.sender.send(TrayCommand::ShowDashboard);
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Hide Window".into(),
                icon_name: "window-close-symbolic".into(),
                activate: Box::new(|tray: &mut Self| {
                    let _ = tray.sender.send(TrayCommand::HideWindow);
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                icon_name: "application-exit".into(),
                activate: Box::new(|tray: &mut Self| {
                    let _ = tray.sender.send(TrayCommand::Quit);
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}

fn create_header_bar(
    app: adw::Application,
    state: Rc<RefCell<AppState>>,
    dashboard_widgets: DashboardPageWidgets,
    stack: Stack,
    last_updated_label: Label,
    last_updated: Rc<RefCell<Option<Instant>>>,
    alert_monitor: SharedAlertMonitor,
) -> HeaderBar {
    let header = HeaderBar::builder()
        .title_widget(&Label::new(Some(APP_NAME)))
        .build();
    header.add_css_class("flat");
    header.add_css_class("flat-header");

    let home_button = Button::builder()
        .icon_name("go-home-symbolic")
        .tooltip_text("Home")
        .build();
    let refresh_button = Button::builder()
        .icon_name("view-refresh-symbolic")
        .tooltip_text("Refresh measurements")
        .build();
    let settings_button = Button::builder()
        .icon_name("preferences-system-symbolic")
        .tooltip_text("Settings")
        .build();
    let help_button = Button::builder()
        .icon_name("help-about-symbolic")
        .tooltip_text("Help")
        .build();

    refresh_button.connect_clicked({
        let state = state.clone();
        let dashboard_widgets = dashboard_widgets.clone();
        let last_updated = last_updated.clone();
        let last_updated_label = last_updated_label.clone();
        let app = app.clone();
        let alert_monitor = alert_monitor.clone();
        move |_| {
            {
                let mut model = state.borrow_mut();
                model.register_action();
            }
            trigger_fetch_current_measures(
                app.clone(),
                state.clone(),
                dashboard_widgets.clone(),
                last_updated.clone(),
                last_updated_label.clone(),
                alert_monitor.clone(),
            );
        }
    });

    home_button.connect_clicked({
        let state = state.clone();
        let stack = stack.clone();
        let dashboard_widgets = dashboard_widgets.clone();
        let last_updated = last_updated.clone();
        let last_updated_label = last_updated_label.clone();
        let app = app.clone();
        let alert_monitor = alert_monitor.clone();
        move |_| {
            // The home button means "default page". Before setup that is
            // Welcome; after setup it is Dashboard.
            let target = if state.borrow().has_server_url() {
                Page::Dashboard
            } else {
                Page::Welcome
            };
            switch_to_page(
                target,
                &state,
                &stack,
                &dashboard_widgets,
                &last_updated,
                &last_updated_label,
                &app,
                &alert_monitor,
            );
        }
    });

    settings_button.connect_clicked({
        let state = state.clone();
        let stack = stack.clone();
        let dashboard_widgets = dashboard_widgets.clone();
        let last_updated = last_updated.clone();
        let last_updated_label = last_updated_label.clone();
        let app = app.clone();
        let alert_monitor = alert_monitor.clone();
        move |_| {
            switch_to_page(
                Page::Settings,
                &state,
                &stack,
                &dashboard_widgets,
                &last_updated,
                &last_updated_label,
                &app,
                &alert_monitor,
            );
        }
    });

    help_button.connect_clicked({
        let state = state.clone();
        let stack = stack.clone();
        let dashboard_widgets = dashboard_widgets.clone();
        let last_updated = last_updated.clone();
        let last_updated_label = last_updated_label.clone();
        let app = app.clone();
        let alert_monitor = alert_monitor.clone();
        move |_| {
            switch_to_page(
                Page::Help,
                &state,
                &stack,
                &dashboard_widgets,
                &last_updated,
                &last_updated_label,
                &app,
                &alert_monitor,
            );
        }
    });

    header.pack_start(&home_button);
    header.pack_start(&refresh_button);
    header.pack_start(&last_updated_label);
    header.pack_end(&help_button);
    header.pack_end(&settings_button);

    header
}

fn create_page_area(
    app: adw::Application,
    state: Rc<RefCell<AppState>>,
    dashboard_page: GtkBox,
    dashboard_widgets: DashboardPageWidgets,
    last_updated: Rc<RefCell<Option<Instant>>>,
    last_updated_label: Label,
    auto_refresh_source: Rc<RefCell<Option<glib::SourceId>>>,
    alert_monitor: SharedAlertMonitor,
) -> (GtkBox, Stack) {
    let container = GtkBox::new(Orientation::Vertical, 12);
    container.set_margin_top(12);
    container.set_margin_bottom(12);
    container.set_margin_start(12);
    container.set_margin_end(12);
    container.set_vexpand(true);

    let stack = Stack::builder()
        .vexpand(true)
        .hexpand(true)
        .transition_type(StackTransitionType::SlideLeftRight)
        .build();

    // `gtk4::Stack` keeps all pages alive and switches visibility by name.
    // This is simple for an app with a few pages and avoids rebuilding Settings
    // every time the user navigates.
    let welcome_page = build_welcome_page(stack.clone());
    let settings_page = build_settings_page(
        app,
        state.clone(),
        stack.clone(),
        dashboard_widgets.clone(),
        last_updated.clone(),
        last_updated_label.clone(),
        auto_refresh_source.clone(),
        alert_monitor,
    );
    let help_page = build_help_page();

    stack.add_titled(
        &welcome_page,
        Some(Page::Welcome.id()),
        Page::Welcome.title(),
    );
    stack.add_titled(
        &dashboard_page,
        Some(Page::Dashboard.id()),
        Page::Dashboard.title(),
    );
    stack.add_titled(
        &settings_page,
        Some(Page::Settings.id()),
        Page::Settings.title(),
    );
    stack.add_titled(&help_page, Some(Page::Help.id()), Page::Help.title());

    container.append(&stack);

    let current_page = state.borrow().current_page;
    let _ = stack.set_visible_child_name(current_page.id());

    (container, stack)
}

fn switch_to_page(
    page: Page,
    state: &Rc<RefCell<AppState>>,
    stack: &Stack,
    dashboard_widgets: &DashboardPageWidgets,
    last_updated: &Rc<RefCell<Option<Instant>>>,
    last_updated_label: &Label,
    app: &adw::Application,
    alert_monitor: &SharedAlertMonitor,
) {
    {
        let mut model = state.borrow_mut();
        model.set_page(page);
    }
    let _ = stack.set_visible_child_name(page.id());

    // Navigating back to the dashboard is a useful moment to refresh, because
    // the user is explicitly asking to see current measurements.
    if page == Page::Dashboard && state.borrow().has_server_url() {
        trigger_fetch_current_measures(
            app.clone(),
            state.clone(),
            dashboard_widgets.clone(),
            last_updated.clone(),
            last_updated_label.clone(),
            alert_monitor.clone(),
        );
    }
}

fn build_settings_page(
    app: adw::Application,
    state: Rc<RefCell<AppState>>,
    stack: Stack,
    dashboard_widgets: DashboardPageWidgets,
    last_updated: Rc<RefCell<Option<Instant>>>,
    last_updated_label: Label,
    auto_refresh_source: Rc<RefCell<Option<glib::SourceId>>>,
    alert_monitor: SharedAlertMonitor,
) -> gtk4::Widget {
    let page = PreferencesPage::builder()
        .title("Settings")
        .icon_name("preferences-system-symbolic")
        .build();

    let theme_options = StringList::new(&["System", "Light", "Dark"]);
    let current_mode = state.borrow().theme_mode;
    let theme_row = ComboRow::builder()
        .title("Style")
        .subtitle("Use the system preference or force a light or dark appearance")
        .model(&theme_options)
        .selected(theme_mode_index(current_mode))
        .build();
    theme_row.connect_selected_notify({
        let style_manager = adw::StyleManager::default();
        let state = state.clone();
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
        .text(state.borrow().server_url().unwrap_or_default())
        .build();
    let url_icon = Image::from_icon_name("network-wired-symbolic");
    url_row.add_prefix(&url_icon);

    let refresh_row = SpinRow::with_range(MIN_REFRESH_INTERVAL_SECS as f64, 3600.0, 1.0);
    refresh_row.set_title("Refresh Interval");
    refresh_row.set_subtitle("Seconds between automatic measurement updates");
    refresh_row.set_value(state.borrow().refresh_interval_secs as f64);
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
        .active(state.borrow().notifications_enabled)
        .build();
    notifications_row.add_suffix(&notifications_switch);
    notifications_row.set_activatable_widget(Some(&notifications_switch));

    let start_minimized_row = ActionRow::builder()
        .title("Start Minimized")
        .subtitle("Start hidden and keep polling in the background on next launch")
        .build();
    let start_minimized_switch = Switch::builder()
        .valign(Align::Center)
        .active(state.borrow().start_minimized)
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
        let app = app.clone();
        let test_notification_row = test_notification_row.clone();
        move |_| {
            let result = send_air_quality_notification(
                &app,
                AlertNotification {
                    id: "airgradient-test-notification".into(),
                    title: "Air Monitor test notification".into(),
                    body: "Notifications are working. Click this notification to open the dashboard."
                        .into(),
                    severity: AlertSeverity::Notice,
                },
            );
            match result {
                Ok(()) => test_notification_row.set_subtitle("Test notification sent."),
                Err(err) => test_notification_row.set_subtitle(&format!("Test notification failed: {err}")),
            }
        }
    });

    let status_row = ActionRow::builder()
        .title("Status")
        .subtitle("Enter a URL like http://192.168.1.201")
        .build();

    let save_row = ActionRow::builder()
        .title("Save Settings")
        .subtitle("Save the server URL and restart the refresh timer")
        .activatable(true)
        .build();
    save_row.add_suffix(&Image::from_icon_name("document-save-symbolic"));

    save_row.connect_activated({
        let state = state.clone();
        let url_row = url_row.clone();
        let refresh_row = refresh_row.clone();
        let notifications_switch = notifications_switch.clone();
        let start_minimized_switch = start_minimized_switch.clone();
        let dashboard_widgets = dashboard_widgets.clone();
        let stack = stack.clone();
        let auto_refresh_source = auto_refresh_source.clone();
        let last_updated = last_updated.clone();
        let status_row = status_row.clone();
        let alert_monitor = alert_monitor.clone();
        let app = app.clone();

        move |_| {
            // The UI stores a user-entered string, while the config file stores
            // a normalized base URL. Keeping normalization here makes fetch
            // code simpler and avoids saving unusable values.
            let normalized = match parse_server_url(&url_row.text()) {
                Ok(url) => url,
                Err(err) => {
                    status_row.set_subtitle(&format!("Invalid URL: {err}"));
                    return;
                }
            };

            let raw_interval = (refresh_row.value().round() as u64).max(MIN_REFRESH_INTERVAL_SECS);
            let config = AppConfig {
                server_url: normalized,
                refresh_interval_secs: raw_interval,
                notifications_enabled: notifications_switch.is_active(),
                start_minimized: start_minimized_switch.is_active(),
            };

            if let Err(err) = config::write_config(&config) {
                status_row.set_subtitle(&format!("Failed to save: {err}"));
                return;
            }

            {
                let mut model = state.borrow_mut();
                model.set_server_url(config.server_url.clone().unwrap_or_default());
                model.set_refresh_interval(config.refresh_interval_secs);
                model.set_notifications_enabled(config.notifications_enabled);
                model.set_start_minimized(config.start_minimized);
            }
            alert_monitor
                .borrow_mut()
                .set_enabled(config.notifications_enabled);

            let has_url = config.server_url.is_some();
            if has_url {
                stack.set_visible_child_name(Page::Dashboard.id());
                dashboard_widgets.server_label.set_text(&format!(
                    "Server URL: {}",
                    config.server_url.unwrap_or_default()
                ));
                status_row.set_subtitle("Saved. Refreshing dashboard.");
            } else {
                stack.set_visible_child_name(Page::Welcome.id());
                dashboard_widgets
                    .server_label
                    .set_text("Server URL: Not configured");
                dashboard_widgets
                    .fetch_status_label
                    .set_text("Server URL removed.");
                status_row.set_subtitle("Cleared URL. Returning to Welcome.");
            }

            start_auto_refresh_timer(
                app.clone(),
                state.clone(),
                dashboard_widgets.clone(),
                auto_refresh_source.clone(),
                last_updated.clone(),
                last_updated_label.clone(),
                alert_monitor.clone(),
            );
            {
                let mut last = last_updated.borrow_mut();
                *last = None;
            }

            if has_url {
                // Saving a valid URL should make the dashboard useful
                // immediately, so fetch once instead of waiting for the next
                // interval tick.
                trigger_fetch_current_measures(
                    app.clone(),
                    state.clone(),
                    dashboard_widgets.clone(),
                    last_updated.clone(),
                    last_updated_label.clone(),
                    alert_monitor.clone(),
                );
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

fn build_welcome_page(stack: Stack) -> gtk4::Widget {
    let open_settings_button = Button::builder().label("Open Settings").build();
    open_settings_button.add_css_class("suggested-action");
    open_settings_button.connect_clicked(move |_| {
        stack.set_visible_child_name(Page::Settings.id());
    });

    let actions = GtkBox::new(Orientation::Horizontal, 0);
    actions.set_halign(Align::Center);
    actions.append(&open_settings_button);

    let page = StatusPage::builder()
        .icon_name("network-wireless-symbolic")
        .title("Connect Device")
        .description(
            "Configure the local-server base URL to start showing measurements. \
             Accepted formats include http://192.168.1.201, 192.168.1.201, and http://192.168.1.201:80.",
        )
        .child(&actions)
        .build();

    page.upcast()
}

fn build_help_page() -> gtk4::Widget {
    let page = PreferencesPage::builder()
        .title("Help")
        .icon_name("help-about-symbolic")
        .build();

    let setup_group = PreferencesGroup::builder()
        .title("Setup")
        .description("How to connect the app to an AirGradient local server.")
        .build();
    setup_group.add(
        &ActionRow::builder()
            .title("Configure the Server")
            .subtitle("Open Settings and enter the local-server base URL.")
            .build(),
    );
    setup_group.add(
        &ActionRow::builder()
            .title("Fetch Measurements")
            .subtitle("Save settings to poll /measures/current automatically.")
            .build(),
    );
    setup_group.add(
        &ActionRow::builder()
            .title("Refresh Manually")
            .subtitle("Use the refresh button in the header bar for an immediate update.")
            .build(),
    );

    page.add(&setup_group);
    page.upcast()
}

fn start_last_updated_timer(last_updated: Rc<RefCell<Option<Instant>>>, label: Label) {
    let update = {
        let last_updated = last_updated.clone();
        let label = label.clone();
        move || {
            update_last_updated_text(&last_updated, &label);
            glib::ControlFlow::Continue
        }
    };

    // This timer updates only text. It does not fetch data.
    let _ = glib::timeout_add_seconds_local(1, update);
}

fn start_auto_refresh_timer(
    app: adw::Application,
    state: Rc<RefCell<AppState>>,
    dashboard_widgets: DashboardPageWidgets,
    auto_refresh_source: Rc<RefCell<Option<glib::SourceId>>>,
    last_updated: Rc<RefCell<Option<Instant>>>,
    last_updated_label: Label,
    alert_monitor: SharedAlertMonitor,
) {
    if let Some(source) = auto_refresh_source.borrow_mut().take() {
        // Removing the old SourceId prevents multiple refresh loops after the
        // user changes the interval in Settings.
        source.remove();
    }

    let (interval_secs, has_server_url) = {
        let model = state.borrow();
        (model.refresh_interval_secs, model.has_server_url())
    };

    let interval_secs = normalized_refresh_interval(interval_secs);
    if !has_server_url {
        // No URL means there is nothing safe to poll.
        return;
    }

    let timer_state = state.clone();
    let timer_widgets = dashboard_widgets.clone();
    let timer_last_updated = last_updated.clone();
    let timer_last_updated_label = last_updated_label.clone();
    let timer_app = app.clone();
    let timer_alert_monitor = alert_monitor.clone();

    let source = glib::timeout_add_seconds_local(interval_secs as u32, move || {
        if !timer_state.borrow().has_server_url() {
            // Stop the timer if the URL was cleared after the timer was created.
            return glib::ControlFlow::Break;
        }

        trigger_fetch_current_measures(
            timer_app.clone(),
            timer_state.clone(),
            timer_widgets.clone(),
            timer_last_updated.clone(),
            timer_last_updated_label.clone(),
            timer_alert_monitor.clone(),
        );
        glib::ControlFlow::Continue
    });

    *auto_refresh_source.borrow_mut() = Some(source);
}

fn normalized_refresh_interval(raw: u64) -> u64 {
    raw.max(MIN_REFRESH_INTERVAL_SECS)
}

fn install_app_actions(
    app: &adw::Application,
    window: &adw::ApplicationWindow,
    stack: &Stack,
    state: Rc<RefCell<AppState>>,
) {
    let show_dashboard = gio::SimpleAction::new("show-dashboard", None);
    show_dashboard.connect_activate({
        let window = window.clone();
        let stack = stack.clone();
        let state = state.clone();
        move |_, _| {
            let target = if state.borrow().has_server_url() {
                Page::Dashboard
            } else {
                Page::Settings
            };
            state.borrow_mut().set_page(target);
            let _ = stack.set_visible_child_name(target.id());
            window.present();
        }
    });
    app.add_action(&show_dashboard);

    let quit = gio::SimpleAction::new("quit", None);
    quit.connect_activate({
        let app = app.clone();
        move |_, _| app.quit()
    });
    app.add_action(&quit);
}

fn install_system_tray(
    app: &adw::Application,
    window: &adw::ApplicationWindow,
    stack: &Stack,
    state: Rc<RefCell<AppState>>,
) {
    let (sender, receiver) = mpsc::channel();
    let tray = AirMonitorTray { sender };
    match ksni::blocking::TrayMethods::assume_sni_available(tray, true).spawn() {
        Ok(handle) => {
            // Dropping the handle shuts down the tray service. The tray should
            // live until explicit app quit.
            std::mem::forget(handle);
        }
        Err(err) => {
            eprintln!("System tray unavailable: {err}");
            return;
        }
    }

    let app = app.clone();
    let window = window.clone();
    let stack = stack.clone();
    glib::timeout_add_local(Duration::from_millis(250), move || {
        while let Ok(command) = receiver.try_recv() {
            match command {
                TrayCommand::ShowDashboard => {
                    let target = if state.borrow().has_server_url() {
                        Page::Dashboard
                    } else {
                        Page::Settings
                    };
                    state.borrow_mut().set_page(target);
                    let _ = stack.set_visible_child_name(target.id());
                    window.present();
                }
                TrayCommand::HideWindow => window.hide(),
                TrayCommand::Quit => app.quit(),
            }
        }
        glib::ControlFlow::Continue
    });
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
enum AlertKind {
    Co2,
    Aqi,
    Pm25,
    Tvoc,
    Nox,
    HumidityLow,
    HumidityHigh,
    DeviceOffline,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd)]
enum AlertSeverity {
    Notice,
    Warning,
    Critical,
}

struct AlertNotification {
    id: String,
    title: String,
    body: String,
    severity: AlertSeverity,
}

struct AlertMonitor {
    enabled: bool,
    consecutive: HashMap<AlertKind, u8>,
    active_severity: HashMap<AlertKind, AlertSeverity>,
    last_sent: HashMap<AlertKind, Instant>,
    fetch_failures: u8,
}

impl AlertMonitor {
    fn new(enabled: bool) -> Self {
        Self {
            enabled,
            consecutive: HashMap::new(),
            active_severity: HashMap::new(),
            last_sent: HashMap::new(),
            fetch_failures: 0,
        }
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.consecutive.clear();
            self.active_severity.clear();
            self.last_sent.clear();
            self.fetch_failures = 0;
        }
    }

    fn evaluate(&mut self, snapshot: &AirMeasureSnapshot) -> Vec<AlertNotification> {
        if !self.enabled {
            return Vec::new();
        }

        self.fetch_failures = 0;
        let mut alerts = Vec::new();

        self.push_if_alert(
            &mut alerts,
            AlertKind::Co2,
            snapshot.co2.and_then(classify_co2),
            |severity| match severity {
                AlertSeverity::Notice => (
                    "CO2 is above 800 ppm",
                    "Ventilation may be low. Open a window or increase fresh-air ventilation.",
                ),
                AlertSeverity::Warning => (
                    "CO2 is high",
                    "CO2 is above 1200 ppm. Ventilate now if possible or reduce room occupancy.",
                ),
                AlertSeverity::Critical => (
                    "CO2 is very high",
                    "CO2 is above 2000 ppm. Leave briefly or improve ventilation immediately if possible.",
                ),
            },
        );
        self.push_if_alert(
            &mut alerts,
            AlertKind::Aqi,
            snapshot.aqi.and_then(classify_aqi),
            |severity| match severity {
                AlertSeverity::Notice => (
                    "AQI is unhealthy for sensitive groups",
                    "Reduce exposure if you are sensitive. Consider filtration or source control.",
                ),
                AlertSeverity::Warning => (
                    "AQI is unhealthy",
                    "Air quality may affect everyone. Reduce pollutant sources and improve filtration.",
                ),
                AlertSeverity::Critical => (
                    "AQI is very unhealthy",
                    "Limit exposure. Use filtration and avoid adding indoor pollution sources.",
                ),
            },
        );
        self.push_if_alert(
            &mut alerts,
            AlertKind::Pm25,
            snapshot.pm25.and_then(classify_pm25),
            |severity| match severity {
                AlertSeverity::Notice => (
                    "PM2.5 is elevated",
                    "Run an air purifier or improve HVAC filtration; reduce cooking, smoke, or dust sources.",
                ),
                AlertSeverity::Warning => (
                    "PM2.5 is high",
                    "Particle pollution is high. Use filtration and avoid activities that create particles.",
                ),
                AlertSeverity::Critical => (
                    "PM2.5 is very high",
                    "Limit exposure and use strong filtration. Check whether outdoor smoke or indoor sources are present.",
                ),
            },
        );
        self.push_if_alert(
            &mut alerts,
            AlertKind::Tvoc,
            snapshot.tvoc.and_then(classify_tvoc),
            |severity| match severity {
                AlertSeverity::Notice | AlertSeverity::Warning => (
                    "VOC level is elevated",
                    "Ventilate and check recent sources: cleaning products, paint, adhesives, or hobby materials.",
                ),
                AlertSeverity::Critical => (
                    "VOC level is high",
                    "Ventilate now and remove or seal likely chemical sources if safe to do so.",
                ),
            },
        );
        self.push_if_alert(
            &mut alerts,
            AlertKind::Nox,
            snapshot.nox.and_then(classify_nox),
            |severity| match severity {
                AlertSeverity::Notice | AlertSeverity::Warning => (
                    "NOx level is elevated",
                    "If cooking or using combustion appliances, use exhaust ventilation or open a window.",
                ),
                AlertSeverity::Critical => (
                    "NOx level is high",
                    "Increase ventilation and check combustion sources such as gas cooking or heaters.",
                ),
            },
        );
        self.push_if_alert(
            &mut alerts,
            AlertKind::HumidityLow,
            snapshot.humidity.and_then(classify_humidity_low),
            |_| (
                "Humidity is low",
                "Air is dry. Consider humidification if the room feels uncomfortable.",
            ),
        );
        self.push_if_alert(
            &mut alerts,
            AlertKind::HumidityHigh,
            snapshot.humidity.and_then(classify_humidity_high),
            |severity| match severity {
                AlertSeverity::Notice | AlertSeverity::Warning => (
                    "Humidity is high",
                    "Ventilate or dehumidify to reduce dampness and mold risk.",
                ),
                AlertSeverity::Critical => (
                    "Humidity is very high",
                    "Dehumidify or ventilate now and check for dampness or leaks.",
                ),
            },
        );

        alerts
    }

    fn record_fetch_error(&mut self, error: &str) -> Option<AlertNotification> {
        if !self.enabled {
            return None;
        }

        self.fetch_failures = self.fetch_failures.saturating_add(1);
        if self.fetch_failures < 3 {
            return None;
        }

        self.make_alert(
            AlertKind::DeviceOffline,
            AlertSeverity::Warning,
            "AirGradient device is unreachable",
            &format!("No fresh sensor data after repeated attempts. Last error: {error}"),
        )
    }

    fn push_if_alert<F>(
        &mut self,
        alerts: &mut Vec<AlertNotification>,
        kind: AlertKind,
        severity: Option<AlertSeverity>,
        text: F,
    ) where
        F: FnOnce(AlertSeverity) -> (&'static str, &'static str),
    {
        let Some(severity) = severity else {
            self.consecutive.remove(&kind);
            self.active_severity.remove(&kind);
            return;
        };

        let count = self.consecutive.entry(kind).or_insert(0);
        *count = count.saturating_add(1);
        if *count < ALERT_CONSECUTIVE_READINGS {
            return;
        }

        let (title, body) = text(severity);
        if let Some(alert) = self.make_alert(kind, severity, title, body) {
            alerts.push(alert);
        }
    }

    fn make_alert(
        &mut self,
        kind: AlertKind,
        severity: AlertSeverity,
        title: &str,
        body: &str,
    ) -> Option<AlertNotification> {
        let now = Instant::now();
        let escalated = self
            .active_severity
            .get(&kind)
            .is_some_and(|active| severity > *active);
        let cooled_down = self
            .last_sent
            .get(&kind)
            .is_none_or(|last| now.saturating_duration_since(*last).as_secs() >= ALERT_COOLDOWN_SECS);

        if !(escalated || cooled_down) {
            return None;
        }

        self.active_severity.insert(kind, severity);
        self.last_sent.insert(kind, now);
        Some(AlertNotification {
            id: format!("airgradient-{kind:?}").to_lowercase(),
            title: title.to_string(),
            body: body.to_string(),
            severity,
        })
    }
}

fn send_air_quality_notification(
    app: &adw::Application,
    alert: AlertNotification,
) -> Result<(), String> {
    let urgency = match alert.severity {
        AlertSeverity::Notice => "normal",
        AlertSeverity::Warning => "normal",
        AlertSeverity::Critical => "critical",
    };
    let expire_time = match alert.severity {
        AlertSeverity::Notice => "8000",
        AlertSeverity::Warning | AlertSeverity::Critical => "0",
    };

    let notify_send_result = Command::new("notify-send")
        .arg("--app-name")
        .arg(APP_NAME)
        .arg("--icon")
        .arg("airgradient-desktop")
        .arg("--urgency")
        .arg(urgency)
        .arg("--expire-time")
        .arg(expire_time)
        .arg(&alert.title)
        .arg(&alert.body)
        .status();

    match notify_send_result {
        Ok(status) if status.success() => return Ok(()),
        Ok(status) => eprintln!("notify-send exited with status {status}"),
        Err(err) => eprintln!("notify-send failed to start: {err}"),
    }

    let system_result = notify_rust::Notification::new()
        .appname(APP_NAME)
        .summary(&alert.title)
        .body(&alert.body)
        .icon("airgradient-desktop")
        .hint(notify_rust::Hint::DesktopEntry(
            "com.airgradient.desktop".into(),
        ))
        .hint(notify_rust::Hint::Category("device".into()))
        .urgency(match alert.severity {
            AlertSeverity::Notice => notify_rust::Urgency::Normal,
            AlertSeverity::Warning => notify_rust::Urgency::Normal,
            AlertSeverity::Critical => notify_rust::Urgency::Critical,
        })
        .timeout(match alert.severity {
            AlertSeverity::Notice => notify_rust::Timeout::Milliseconds(8_000),
            AlertSeverity::Warning | AlertSeverity::Critical => notify_rust::Timeout::Never,
        })
        .show();

    if let Err(err) = system_result {
        eprintln!("notify-rust failed, falling back to GNotification: {err}");
    } else {
        return Ok(());
    }

    let notification = gio::Notification::new(&alert.title);
    notification.set_body(Some(&alert.body));
    notification.set_default_action("app.show-dashboard");
    notification.add_button("Open Dashboard", "app.show-dashboard");
    notification.set_priority(match alert.severity {
        AlertSeverity::Notice => gio::NotificationPriority::Normal,
        AlertSeverity::Warning => gio::NotificationPriority::High,
        AlertSeverity::Critical => gio::NotificationPriority::Urgent,
    });
    let unique_id = format!(
        "{}-{}",
        alert.id,
        NOTIFICATION_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    app.send_notification(Some(&unique_id), &notification);
    Ok(())
}

fn classify_co2(value: f32) -> Option<AlertSeverity> {
    if value > 2000.0 {
        Some(AlertSeverity::Critical)
    } else if value > 1200.0 {
        Some(AlertSeverity::Warning)
    } else if value > 800.0 {
        Some(AlertSeverity::Notice)
    } else {
        None
    }
}

fn classify_aqi(value: f32) -> Option<AlertSeverity> {
    if value > 200.0 {
        Some(AlertSeverity::Critical)
    } else if value > 150.0 {
        Some(AlertSeverity::Warning)
    } else if value > 100.0 {
        Some(AlertSeverity::Notice)
    } else {
        None
    }
}

fn classify_pm25(value: f32) -> Option<AlertSeverity> {
    if value > 150.0 {
        Some(AlertSeverity::Critical)
    } else if value > 55.0 {
        Some(AlertSeverity::Warning)
    } else if value > 35.0 {
        Some(AlertSeverity::Notice)
    } else {
        None
    }
}

fn classify_tvoc(value: f32) -> Option<AlertSeverity> {
    if value > 660.0 {
        Some(AlertSeverity::Critical)
    } else if value > 220.0 {
        Some(AlertSeverity::Warning)
    } else {
        None
    }
}

fn classify_nox(value: f32) -> Option<AlertSeverity> {
    if value > 150.0 {
        Some(AlertSeverity::Critical)
    } else if value > 50.0 {
        Some(AlertSeverity::Warning)
    } else {
        None
    }
}

fn classify_humidity_low(value: f32) -> Option<AlertSeverity> {
    if value < 30.0 {
        Some(AlertSeverity::Notice)
    } else {
        None
    }
}

fn classify_humidity_high(value: f32) -> Option<AlertSeverity> {
    if value > 75.0 {
        Some(AlertSeverity::Critical)
    } else if value > 65.0 {
        Some(AlertSeverity::Notice)
    } else {
        None
    }
}

fn trigger_fetch_current_measures(
    app: adw::Application,
    state: Rc<RefCell<AppState>>,
    dashboard_widgets: DashboardPageWidgets,
    last_updated: Rc<RefCell<Option<Instant>>>,
    last_updated_label: Label,
    alert_monitor: SharedAlertMonitor,
) {
    let base_url = match state.borrow().server_url() {
        Some(url) => url.to_string(),
        None => {
            dashboard_widgets
                .fetch_status_label
                .set_text("No server URL configured.");
            return;
        }
    };

    dashboard_widgets
        .server_label
        .set_text(&format!("Server URL: {base_url}"));
    dashboard_widgets
        .fetch_status_label
        .set_text("Fetching measurements...");

    let dashboard_widgets_for_ui = dashboard_widgets.clone();
    let last_updated_for_ui = last_updated.clone();
    let last_updated_label_for_ui = last_updated_label.clone();
    let app_for_ui = app.clone();

    glib::MainContext::default().spawn_local(async move {
        // `reqwest::blocking` would freeze the GTK main loop if called directly.
        // `gio::spawn_blocking` runs it on a worker thread and resumes here with
        // the result so the UI can be updated safely.
        let result = gio::spawn_blocking(move || fetch_current_measurements(&base_url)).await;
        let result = match result {
            Ok(result) => result,
            Err(_) => Err("Background fetch task failed to execute.".to_string()),
        };

        match result {
            Ok(measurement) => {
                dashboard_widgets_for_ui.apply_measurements(&measurement);
                let alerts = alert_monitor.borrow_mut().evaluate(&measurement);
                for alert in alerts {
                    if let Err(err) = send_air_quality_notification(&app_for_ui, alert) {
                        eprintln!("System notification failed: {err}");
                    }
                }
                *last_updated_for_ui.borrow_mut() = Some(Instant::now());
                update_last_updated_text(&last_updated_for_ui, &last_updated_label_for_ui);
                dashboard_widgets_for_ui
                    .fetch_status_label
                    .set_text("Latest measurements loaded.");
            }
            Err(err) => {
                dashboard_widgets_for_ui
                    .fetch_status_label
                    .set_text(&format!("Fetch failed: {err}"));
                if let Some(alert) = alert_monitor.borrow_mut().record_fetch_error(&err) {
                    if let Err(err) = send_air_quality_notification(&app_for_ui, alert) {
                        eprintln!("System notification failed: {err}");
                    }
                }
            }
        }
    });
}

fn fetch_current_measurements(base_url: &str) -> FetchResult<AirMeasureSnapshot> {
    let normalized_base_url =
        parse_server_url(base_url)?.ok_or_else(|| "No server URL configured.".to_string())?;
    let url = format!(
        "{}/measures/current",
        normalized_base_url.trim_end_matches('/')
    );

    // The client is small enough to create per request. If the app grows into a
    // high-frequency poller, this could be moved into shared state.
    let client = Client::builder()
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .build()
        .map_err(|err| format!("HTTP client error: {err}"))?;

    let response = client
        .get(url)
        .send()
        .map_err(|err| format!("Request failed: {err}"))?;
    if !response.status().is_success() {
        return Err(format!("Server returned HTTP {}", response.status()));
    }

    let payload: Value = response
        .json()
        .map_err(|err| format!("Invalid JSON response: {err}"))?;
    Ok(parse_air_measurements(&payload))
}

fn parse_server_url(raw: &str) -> FetchResult<Option<String>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    let candidate = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        // Users commonly paste just an IP address. Default to HTTP because the
        // AirGradient local server is normally plain HTTP on the local network.
        format!("http://{trimmed}")
    };

    let mut parsed = Url::parse(&candidate).map_err(|err| format!("Invalid URL: {err}"))?;

    match parsed.scheme() {
        "http" | "https" => {}
        scheme => {
            return Err(format!("Invalid URL scheme '{scheme}'. Use http or https."));
        }
    }

    if parsed.host().is_none() {
        return Err("URL missing host component.".to_string());
    }

    // Store only the base URL. Fetching always appends `/measures/current`.
    parsed.set_path("");
    parsed.set_query(None);
    parsed.set_fragment(None);

    Ok(Some(parsed.to_string().trim_end_matches('/').to_string()))
}

fn update_last_updated_text(last_updated: &Rc<RefCell<Option<Instant>>>, label: &Label) {
    let text = match *last_updated.borrow() {
        Some(last) => {
            let elapsed = Instant::now().saturating_duration_since(last);
            let seconds = elapsed.as_secs();
            if seconds < 5 {
                "Last updated: just now".to_string()
            } else if seconds < 60 {
                format!("Last updated: {seconds}s ago")
            } else {
                let minutes = seconds / 60;
                let secs = seconds % 60;
                format!("Last updated: {minutes}m {secs}s ago")
            }
        }
        None => "Last updated: not yet".to_string(),
    };
    label.set_text(&text);
}

fn apply_color_scheme(style_manager: &adw::StyleManager, theme_mode: ThemeMode) {
    let scheme = match theme_mode {
        ThemeMode::System => adw::ColorScheme::Default,
        ThemeMode::Light => adw::ColorScheme::ForceLight,
        ThemeMode::Dark => adw::ColorScheme::ForceDark,
    };
    style_manager.set_color_scheme(scheme);
}

fn theme_mode_index(theme_mode: ThemeMode) -> u32 {
    match theme_mode {
        ThemeMode::System => 0,
        ThemeMode::Light => 1,
        ThemeMode::Dark => 2,
    }
}

fn update_dark_shell_class(root: &GtkBox, is_dark: bool) {
    // Libadwaita handles the general theme; this class is only for the app's
    // custom dashboard shell background.
    if is_dark {
        root.add_css_class("dark-app-shell");
    } else {
        root.remove_css_class("dark-app-shell");
    }
}
