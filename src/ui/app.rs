//! Main GTK window and application UI flow.
//!
//! This module owns the visible application shell: header bar, page stack,
//! settings page, timers, and measurement fetching. It intentionally keeps the
//! lower-level dashboard widgets in separate modules.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use adw::{prelude::*, ActionRow, HeaderBar, PreferencesGroup, PreferencesPage, StatusPage};
use gtk4::{Align, Box as GtkBox, Button, Label, Orientation, Stack, StackTransitionType};

use crate::config::{ConfigStartupNotice, RefreshInterval};
use crate::ui::settings::build_settings_page;
use crate::ui::tray::{install_system_tray, SystemTrayRuntime};
use crate::ui::{
    build_dashboard_page, load_dashboard_css, register_resources, DashboardPageWidgets,
};
use crate::{
    alerts::AlertMonitor,
    app_info::APP_NAME,
    state::{AppState, Page, ThemeMode},
};

const DEFAULT_WIDTH: i32 = 1180;
const DEFAULT_HEIGHT: i32 = 780;

type SharedAlertMonitor = Rc<RefCell<AlertMonitor>>;

#[derive(Clone)]
pub(super) struct UiContext {
    pub(super) app: adw::Application,
    pub(super) state: Rc<RefCell<AppState>>,
    pub(super) dashboard_widgets: DashboardPageWidgets,
    pub(super) last_updated: Rc<RefCell<Option<Instant>>>,
    pub(super) last_updated_label: Label,
    pub(super) alert_monitor: SharedAlertMonitor,
}

impl UiContext {
    pub(super) fn trigger_fetch(&self) {
        super::fetch::trigger_fetch_current_measures(self.clone());
    }
}

#[derive(Clone)]
pub(super) struct NavigationContext {
    pub(super) ui: UiContext,
    pub(super) stack: Stack,
}

struct AppRuntime {
    _hold_guard: gio::ApplicationHoldGuard,
    _tray: Option<SystemTrayRuntime>,
    last_updated_source: Option<glib::SourceId>,
    auto_refresh_source: Rc<RefCell<Option<glib::SourceId>>>,
}

impl Drop for AppRuntime {
    fn drop(&mut self) {
        if let Some(source) = self.last_updated_source.take() {
            source.remove();
        }
        if let Some(source) = self.auto_refresh_source.borrow_mut().take() {
            source.remove();
        }
    }
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
    let tray = install_system_tray(app, &window, &stack, state.clone());
    window.connect_close_request({
        let window = window.clone();
        move |_| {
            window.hide();
            glib::Propagation::Stop
        }
    });
    // The close button now means "keep running in the background". `app.quit`
    // is the intentional exit path from the tray menu or application action.
    let hold_guard = app.hold();

    if state.borrow().has_server_url() {
        // If config already contains a server URL, open directly into useful
        // data instead of waiting for manual refresh.
        ui.trigger_fetch();
    }

    let last_updated_source =
        start_last_updated_timer(last_updated.clone(), last_updated_label.clone());
    start_auto_refresh_timer(ui, auto_refresh_source.clone());

    let runtime = AppRuntime {
        _hold_guard: hold_guard,
        _tray: tray,
        last_updated_source: Some(last_updated_source),
        auto_refresh_source: auto_refresh_source.clone(),
    };
    // Store runtime resources on the window so they live exactly as long as the
    // main GTK object. GLib owns this qdata and drops it when the window is
    // finalized.
    unsafe {
        window.set_data("airgradient-runtime", runtime);
    }

    if !(run_minimized || state.borrow().start_minimized) {
        window.present();
    }
    window
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
    let startup_notice = navigation.ui.state.borrow().startup_notice.clone();
    let welcome_page = build_welcome_page(stack.clone(), startup_notice.as_ref());
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

fn build_welcome_page(stack: Stack, startup_notice: Option<&ConfigStartupNotice>) -> gtk4::Widget {
    let open_settings_button = Button::builder().label("Open Settings").build();
    open_settings_button.add_css_class("suggested-action");
    open_settings_button.connect_clicked(move |_| {
        stack.set_visible_child_name(Page::Settings.id());
    });

    let actions = GtkBox::new(Orientation::Horizontal, 0);
    actions.set_halign(Align::Center);
    actions.append(&open_settings_button);

    let description = match startup_notice {
        Some(ConfigStartupNotice::ReadFailed(_)) | Some(ConfigStartupNotice::ParseFailed(_)) => {
            format!(
                "{} {}",
                "Saved settings could not be loaded, so defaults are active.",
                "Open Settings to review and save a corrected configuration."
            )
        }
        _ => "Configure the local-server base URL to start showing measurements. Accepted formats include http://192.168.1.201, 192.168.1.201, and http://192.168.1.201:80.".to_string(),
    };

    let page = StatusPage::builder()
        .icon_name("network-wireless-symbolic")
        .title("Connect Device")
        .description(&description)
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

fn start_last_updated_timer(
    last_updated: Rc<RefCell<Option<Instant>>>,
    label: Label,
) -> glib::SourceId {
    let update = {
        let last_updated = last_updated.clone();
        let label = label.clone();
        move || {
            update_last_updated_text(&last_updated, &label);
            glib::ControlFlow::Continue
        }
    };

    // This timer updates only text. It does not fetch data.
    glib::timeout_add_seconds_local(1, update)
}

pub(super) fn start_auto_refresh_timer(
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
        (
            normalized_refresh_interval(model.refresh_interval.as_secs()),
            model.has_server_url(),
        )
    };

    if !has_server_url {
        // No URL means there is nothing safe to poll.
        return;
    }

    let source = glib::timeout_add_seconds_local(interval_secs.as_secs() as u32, move || {
        if !ui.state.borrow().has_server_url() {
            // Stop the timer if the URL was cleared after the timer was created.
            return glib::ControlFlow::Break;
        }

        ui.trigger_fetch();
        glib::ControlFlow::Continue
    });

    *auto_refresh_source.borrow_mut() = Some(source);
}

fn normalized_refresh_interval(raw: u64) -> RefreshInterval {
    RefreshInterval::clamped(raw)
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

pub(super) fn update_last_updated_text(last_updated: &Rc<RefCell<Option<Instant>>>, label: &Label) {
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

pub(super) fn apply_color_scheme(style_manager: &adw::StyleManager, theme_mode: ThemeMode) {
    let scheme = match theme_mode {
        ThemeMode::System => adw::ColorScheme::Default,
        ThemeMode::Light => adw::ColorScheme::ForceLight,
        ThemeMode::Dark => adw::ColorScheme::ForceDark,
    };
    style_manager.set_color_scheme(scheme);
}

pub(super) fn theme_mode_index(theme_mode: ThemeMode) -> u32 {
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
    use super::normalized_refresh_interval;
    use crate::config::MIN_REFRESH_INTERVAL_SECS;

    #[test]
    fn normalized_refresh_interval_enforces_minimum() {
        assert_eq!(
            normalized_refresh_interval(1).as_secs(),
            MIN_REFRESH_INTERVAL_SECS
        );
        assert_eq!(normalized_refresh_interval(60).as_secs(), 60);
    }
}
