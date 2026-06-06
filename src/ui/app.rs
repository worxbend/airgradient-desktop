//! Main GTK window and application UI flow.
//!
//! This module owns the visible application shell: header bar, page stack,
//! settings page, timers, and measurement fetching. It intentionally keeps the
//! lower-level dashboard widgets in separate modules.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use adw::{
    self, prelude::*, ActionRow, ComboRow, EntryRow, HeaderBar, PreferencesGroup, PreferencesPage,
    SpinRow, StatusPage,
};
use gio;
use gtk4::{
    Align, Box as GtkBox, Button, Image, Label, Orientation, Stack, StackTransitionType, StringList,
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
const APP_NAME: &str = "Air Monitor";

type FetchResult<T> = Result<T, String>;

pub fn build_main_window(
    app: &adw::Application,
    state: Rc<RefCell<AppState>>,
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

    let (page_area, stack) = create_page_area(
        state.clone(),
        dashboard_page,
        dashboard_widgets.clone(),
        last_updated.clone(),
        last_updated_label.clone(),
        auto_refresh_source.clone(),
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
        state.clone(),
        dashboard_widgets.clone(),
        stack.clone(),
        last_updated_label.clone(),
        last_updated.clone(),
    ));
    root.append(&page_area);
    window.set_content(Some(&root));

    if state.borrow().has_server_url() {
        // If config already contains a server URL, open directly into useful
        // data instead of waiting for manual refresh.
        trigger_fetch_current_measures(
            state.clone(),
            dashboard_widgets.clone(),
            last_updated.clone(),
            last_updated_label.clone(),
        );
    }

    start_last_updated_timer(last_updated.clone(), last_updated_label.clone());
    start_auto_refresh_timer(
        state.clone(),
        dashboard_widgets.clone(),
        auto_refresh_source,
        last_updated.clone(),
        last_updated_label,
    );

    window.present();
    window
}

fn create_header_bar(
    state: Rc<RefCell<AppState>>,
    dashboard_widgets: DashboardPageWidgets,
    stack: Stack,
    last_updated_label: Label,
    last_updated: Rc<RefCell<Option<Instant>>>,
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
        move |_| {
            {
                let mut model = state.borrow_mut();
                model.register_action();
            }
            trigger_fetch_current_measures(
                state.clone(),
                dashboard_widgets.clone(),
                last_updated.clone(),
                last_updated_label.clone(),
            );
        }
    });

    home_button.connect_clicked({
        let state = state.clone();
        let stack = stack.clone();
        let dashboard_widgets = dashboard_widgets.clone();
        let last_updated = last_updated.clone();
        let last_updated_label = last_updated_label.clone();
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
            );
        }
    });

    settings_button.connect_clicked({
        let state = state.clone();
        let stack = stack.clone();
        let dashboard_widgets = dashboard_widgets.clone();
        let last_updated = last_updated.clone();
        let last_updated_label = last_updated_label.clone();
        move |_| {
            switch_to_page(
                Page::Settings,
                &state,
                &stack,
                &dashboard_widgets,
                &last_updated,
                &last_updated_label,
            );
        }
    });

    help_button.connect_clicked({
        let state = state.clone();
        let stack = stack.clone();
        let dashboard_widgets = dashboard_widgets.clone();
        let last_updated = last_updated.clone();
        let last_updated_label = last_updated_label.clone();
        move |_| {
            switch_to_page(
                Page::Help,
                &state,
                &stack,
                &dashboard_widgets,
                &last_updated,
                &last_updated_label,
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
    state: Rc<RefCell<AppState>>,
    dashboard_page: GtkBox,
    dashboard_widgets: DashboardPageWidgets,
    last_updated: Rc<RefCell<Option<Instant>>>,
    last_updated_label: Label,
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
    let welcome_page = build_welcome_page(stack.clone());
    let settings_page = build_settings_page(
        state.clone(),
        stack.clone(),
        dashboard_widgets.clone(),
        last_updated.clone(),
        last_updated_label.clone(),
        auto_refresh_source.clone(),
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
            state.clone(),
            dashboard_widgets.clone(),
            last_updated.clone(),
            last_updated_label.clone(),
        );
    }
}

fn build_settings_page(
    state: Rc<RefCell<AppState>>,
    stack: Stack,
    dashboard_widgets: DashboardPageWidgets,
    last_updated: Rc<RefCell<Option<Instant>>>,
    last_updated_label: Label,
    auto_refresh_source: Rc<RefCell<Option<glib::SourceId>>>,
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
        let dashboard_widgets = dashboard_widgets.clone();
        let stack = stack.clone();
        let auto_refresh_source = auto_refresh_source.clone();
        let last_updated = last_updated.clone();
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
            };

            if let Err(err) = config::write_config(&config) {
                status_row.set_subtitle(&format!("Failed to save: {err}"));
                return;
            }

            {
                let mut model = state.borrow_mut();
                model.set_server_url(config.server_url.clone().unwrap_or_default());
                model.set_refresh_interval(config.refresh_interval_secs);
            }

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
                state.clone(),
                dashboard_widgets.clone(),
                auto_refresh_source.clone(),
                last_updated.clone(),
                last_updated_label.clone(),
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
                    state.clone(),
                    dashboard_widgets.clone(),
                    last_updated.clone(),
                    last_updated_label.clone(),
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
    state: Rc<RefCell<AppState>>,
    dashboard_widgets: DashboardPageWidgets,
    auto_refresh_source: Rc<RefCell<Option<glib::SourceId>>>,
    last_updated: Rc<RefCell<Option<Instant>>>,
    last_updated_label: Label,
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

    let source = glib::timeout_add_seconds_local(interval_secs as u32, move || {
        if !timer_state.borrow().has_server_url() {
            // Stop the timer if the URL was cleared after the timer was created.
            return glib::ControlFlow::Break;
        }

        trigger_fetch_current_measures(
            timer_state.clone(),
            timer_widgets.clone(),
            timer_last_updated.clone(),
            timer_last_updated_label.clone(),
        );
        glib::ControlFlow::Continue
    });

    *auto_refresh_source.borrow_mut() = Some(source);
}

fn normalized_refresh_interval(raw: u64) -> u64 {
    raw.max(MIN_REFRESH_INTERVAL_SECS)
}

fn trigger_fetch_current_measures(
    state: Rc<RefCell<AppState>>,
    dashboard_widgets: DashboardPageWidgets,
    last_updated: Rc<RefCell<Option<Instant>>>,
    last_updated_label: Label,
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
