//! In-memory application state.
//!
//! Persisted settings live in `config.rs`. This module stores state that the UI
//! needs while the app is running, such as the active page and current theme.

use crate::config::{ConfigStartupNotice, RefreshInterval};
use crate::device::DeviceBaseUrl;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum Page {
    Welcome,
    Dashboard,
    Settings,
    Help,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ThemeMode {
    System,
    Light,
    Dark,
}

impl Page {
    /// Stable string used by `gtk4::Stack` to identify each page.
    pub const fn id(self) -> &'static str {
        match self {
            Self::Welcome => "welcome",
            Self::Dashboard => "dashboard",
            Self::Settings => "settings",
            Self::Help => "help",
        }
    }

    /// Human-readable title used when registering pages with GTK.
    pub const fn title(self) -> &'static str {
        match self {
            Self::Welcome => "Welcome",
            Self::Dashboard => "Dashboard",
            Self::Settings => "Settings",
            Self::Help => "Help",
        }
    }
}

#[derive(Debug, Clone)]
pub struct AppState {
    pub current_page: Page,
    pub action_count: u32,
    pub theme_mode: ThemeMode,
    pub server_url: Option<DeviceBaseUrl>,
    pub refresh_interval: RefreshInterval,
    pub notifications_enabled: bool,
    pub start_minimized: bool,
    pub startup_notice: Option<ConfigStartupNotice>,
}

impl AppState {
    pub fn new(
        server_url: Option<DeviceBaseUrl>,
        refresh_interval: RefreshInterval,
        notifications_enabled: bool,
        start_minimized: bool,
        startup_notice: Option<ConfigStartupNotice>,
    ) -> Self {
        Self {
            // A configured URL means the app can go straight to the dashboard.
            // Without one, the welcome page explains how to configure the app.
            current_page: if server_url.is_some() {
                Page::Dashboard
            } else {
                Page::Welcome
            },
            action_count: 0,
            theme_mode: ThemeMode::System,
            server_url,
            refresh_interval,
            notifications_enabled,
            start_minimized,
            startup_notice,
        }
    }

    pub fn set_page(&mut self, page: Page) {
        self.current_page = page;
    }

    pub fn register_action(&mut self) -> u32 {
        self.action_count += 1;
        self.action_count
    }

    pub fn has_server_url(&self) -> bool {
        self.server_url.is_some()
    }

    pub fn set_server_url(&mut self, raw_server_url: String) {
        self.server_url = DeviceBaseUrl::parse(&raw_server_url).ok().flatten();

        // Routing follows configuration: a valid URL unlocks the dashboard,
        // while clearing the URL returns the user to onboarding.
        if self.has_server_url() {
            self.current_page = Page::Dashboard;
        } else {
            self.current_page = Page::Welcome;
        }
    }

    pub fn server_url(&self) -> Option<&str> {
        self.server_url.as_ref().map(DeviceBaseUrl::as_str)
    }

    pub fn set_device_base_url(&mut self, server_url: Option<DeviceBaseUrl>) {
        self.server_url = server_url;
        if self.has_server_url() {
            self.current_page = Page::Dashboard;
        } else {
            self.current_page = Page::Welcome;
        }
    }

    pub fn set_refresh_interval(&mut self, refresh_interval: RefreshInterval) {
        self.refresh_interval = refresh_interval;
    }

    pub fn set_notifications_enabled(&mut self, enabled: bool) {
        self.notifications_enabled = enabled;
    }

    pub fn set_start_minimized(&mut self, enabled: bool) {
        self.start_minimized = enabled;
    }
}
