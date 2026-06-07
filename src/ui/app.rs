//! Main GTK window and application UI flow.
//!
//! This module owns the visible application shell: header bar, page stack,
//! settings page, timers, and measurement fetching. It intentionally keeps the
//! lower-level dashboard widgets in separate modules.

use std::cell::RefCell;
use std::process::Command;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use adw::{
    prelude::*, ActionRow, ComboRow, EntryRow, HeaderBar, PreferencesGroup, PreferencesPage,
    SpinRow, StatusPage,
};
use gtk4::{
    Align, Box as GtkBox, Button, Image, Label, Orientation, Stack, StackTransitionType,
    StringList, Switch,
};
use reqwest::blocking::Client;
use serde_json::Value;
use url::Url;

use crate::ui::{
    build_dashboard_page, load_dashboard_css, register_resources, DashboardPageWidgets,
};
use crate::{
    alerts::{AlertMonitor, AlertNotification, AlertSeverity},
    config::{self, AppConfig},
    sensors::{parse_air_measurements, AirMeasureSnapshot},
    state::{AppState, Page, ThemeMode},
};

const DEFAULT_WIDTH: i32 = 1180;
const DEFAULT_HEIGHT: i32 = 780;
const REQUEST_TIMEOUT_SECS: u64 = 8;
const MIN_REFRESH_INTERVAL_SECS: u64 = 5;
const APP_NAME: &str = "Air Monitor";
static NOTIFICATION_SEQUENCE: AtomicU64 = AtomicU64::new(1);

type FetchResult<T> = Result<T, String>;
type SharedAlertMonitor = Rc<RefCell<AlertMonitor>>;

#[derive(Clone)]
struct UiContext {
    app: adw::Application,
    state: Rc<RefCell<AppState>>,
    dashboard_widgets: DashboardPageWidgets,
    last_updated: Rc<RefCell<Option<Instant>>>,
    last_updated_label: Label,
    alert_monitor: SharedAlertMonitor,
}

impl UiContext {
    fn trigger_fetch(&self) {
        trigger_fetch_current_measures(self.clone());
    }
}

#[derive(Clone)]
struct NavigationContext {
    ui: UiContext,
    stack: Stack,
}

impl NavigationContext {
    fn switch_to_page(&self, page: Page) {
        {
            let mut model = self.ui.state.borrow_mut();
            model.set_page(page);
        }
        self.stack.set_visible_child_name(page.id());

        // Navigating back to the dashboard is a useful moment to refresh, because
        // the user is explicitly asking to see current measurements.
        if page == Page::Dashboard && self.ui.state.borrow().has_server_url() {
            self.ui.trigger_fetch();
        }
    }
}

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
    let ui = UiContext {
        app: app.clone(),
        state: state.clone(),
        dashboard_widgets: dashboard_widgets.clone(),
        last_updated: last_updated.clone(),
        last_updated_label: last_updated_label.clone(),
        alert_monitor,
    };

    let (page_area, stack) =
        create_page_area(ui.clone(), dashboard_page, auto_refresh_source.clone());
    let navigation = NavigationContext {
        ui: ui.clone(),
        stack: stack.clone(),
    };

    let root = GtkBox::new(Orientation::Vertical, 0);
    root.add_css_class("app-shell");
    update_dark_shell_class(&root, style_manager.is_dark());
    // When System theme is selected, libadwaita can change between light and
    // dark while the app is running. Keep our custom root CSS class in sync.
    style_manager.connect_dark_notify({
        let root = root.clone();
        move |manager| update_dark_shell_class(&root, manager.is_dark())
    });
    root.append(&create_header_bar(navigation.clone()));
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
        ui.trigger_fetch();
    }

    start_last_updated_timer(last_updated.clone(), last_updated_label.clone());
    start_auto_refresh_timer(ui, auto_refresh_source);

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

fn create_header_bar(navigation: NavigationContext) -> HeaderBar {
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
        let ui = navigation.ui.clone();
        move |_| {
            {
                let mut model = ui.state.borrow_mut();
                model.register_action();
            }
            ui.trigger_fetch();
        }
    });

    home_button.connect_clicked({
        let navigation = navigation.clone();
        move |_| {
            // The home button means "default page". Before setup that is
            // Welcome; after setup it is Dashboard.
            let target = if navigation.ui.state.borrow().has_server_url() {
                Page::Dashboard
            } else {
                Page::Welcome
            };
            navigation.switch_to_page(target);
        }
    });

    settings_button.connect_clicked({
        let navigation = navigation.clone();
        move |_| {
            navigation.switch_to_page(Page::Settings);
        }
    });

    help_button.connect_clicked({
        let navigation = navigation.clone();
        move |_| {
            navigation.switch_to_page(Page::Help);
        }
    });

    header.pack_start(&home_button);
    header.pack_start(&refresh_button);
    header.pack_start(&navigation.ui.last_updated_label);
    header.pack_end(&help_button);
    header.pack_end(&settings_button);

    header
}

fn create_page_area(
    ui: UiContext,
    dashboard_page: GtkBox,
    auto_refresh_source: Rc<RefCell<Option<glib::SourceId>>>,
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
    let navigation = NavigationContext {
        ui,
        stack: stack.clone(),
    };
    let welcome_page = build_welcome_page(stack.clone());
    let settings_page = build_settings_page(navigation.clone(), auto_refresh_source.clone());
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

    let current_page = navigation.ui.state.borrow().current_page;
    stack.set_visible_child_name(current_page.id());

    (container, stack)
}

fn build_settings_page(
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
    refresh_row.set_value(navigation.ui.state.borrow().refresh_interval_secs as f64);
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
        .subtitle("Enter a URL like http://192.168.1.201")
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
                let mut model = navigation.ui.state.borrow_mut();
                model.set_server_url(config.server_url.clone().unwrap_or_default());
                model.set_refresh_interval(config.refresh_interval_secs);
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
                        config.server_url.unwrap_or_default()
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
    ui: UiContext,
    auto_refresh_source: Rc<RefCell<Option<glib::SourceId>>>,
) {
    if let Some(source) = auto_refresh_source.borrow_mut().take() {
        // Removing the old SourceId prevents multiple refresh loops after the
        // user changes the interval in Settings.
        source.remove();
    }

    let (interval_secs, has_server_url) = {
        let model = ui.state.borrow();
        (model.refresh_interval_secs, model.has_server_url())
    };

    let interval_secs = normalized_refresh_interval(interval_secs);
    if !has_server_url {
        // No URL means there is nothing safe to poll.
        return;
    }

    let source = glib::timeout_add_seconds_local(interval_secs as u32, move || {
        if !ui.state.borrow().has_server_url() {
            // Stop the timer if the URL was cleared after the timer was created.
            return glib::ControlFlow::Break;
        }

        ui.trigger_fetch();
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
            stack.set_visible_child_name(target.id());
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
                    stack.set_visible_child_name(target.id());
                    window.present();
                }
                TrayCommand::HideWindow => window.hide(),
                TrayCommand::Quit => app.quit(),
            }
        }
        glib::ControlFlow::Continue
    });
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

fn trigger_fetch_current_measures(ui: UiContext) {
    let base_url = match ui.state.borrow().server_url() {
        Some(url) => url.to_string(),
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
        let result = gio::spawn_blocking(move || fetch_current_measurements(&base_url)).await;
        let result = match result {
            Ok(result) => result,
            Err(_) => Err("Background fetch task failed to execute.".to_string()),
        };

        match result {
            Ok(measurement) => {
                ui.dashboard_widgets.apply_measurements(&measurement);
                let alerts = ui.alert_monitor.borrow_mut().evaluate(&measurement);
                for alert in alerts {
                    if let Err(err) = send_air_quality_notification(&ui.app, alert) {
                        eprintln!("System notification failed: {err}");
                    }
                }
                *ui.last_updated.borrow_mut() = Some(Instant::now());
                update_last_updated_text(&ui.last_updated, &ui.last_updated_label);
                ui.dashboard_widgets
                    .fetch_status_label
                    .set_text("Latest measurements loaded.");
            }
            Err(err) => {
                ui.dashboard_widgets
                    .fetch_status_label
                    .set_text(&format!("Fetch failed: {err}"));
                if let Some(alert) = ui.alert_monitor.borrow_mut().record_fetch_error(&err) {
                    if let Err(err) = send_air_quality_notification(&ui.app, alert) {
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

#[cfg(test)]
mod tests {
    use super::{normalized_refresh_interval, parse_server_url, MIN_REFRESH_INTERVAL_SECS};

    #[test]
    fn parse_server_url_accepts_empty_value_as_not_configured() {
        assert_eq!(parse_server_url("   ").expect("empty URL is valid"), None);
    }

    #[test]
    fn parse_server_url_defaults_bare_host_to_http() {
        let normalized = parse_server_url("192.168.1.201").expect("bare host should parse");

        assert_eq!(normalized.as_deref(), Some("http://192.168.1.201"));
    }

    #[test]
    fn parse_server_url_keeps_scheme_host_and_port_only() {
        let normalized =
            parse_server_url(" https://airgradient.local:8443/measures/current?x=1#readings ")
                .expect("URL with path should parse");

        assert_eq!(
            normalized.as_deref(),
            Some("https://airgradient.local:8443")
        );
    }

    #[test]
    fn parse_server_url_rejects_unsupported_schemes() {
        let err = parse_server_url("ftp://airgradient.local").expect_err("ftp is unsupported");

        assert!(err.contains("Invalid URL scheme 'ftp'"));
    }

    #[test]
    fn normalized_refresh_interval_enforces_minimum() {
        assert_eq!(normalized_refresh_interval(1), MIN_REFRESH_INTERVAL_SECS);
        assert_eq!(normalized_refresh_interval(60), 60);
    }
}
