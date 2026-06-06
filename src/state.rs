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
    pub const fn id(self) -> &'static str {
        match self {
            Self::Welcome => "welcome",
            Self::Dashboard => "dashboard",
            Self::Settings => "settings",
            Self::Help => "help",
        }
    }

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
    pub server_url: Option<String>,
    pub refresh_interval_secs: u64,
}

impl AppState {
    pub fn new(server_url: Option<String>, refresh_interval_secs: u64) -> Self {
        Self {
            current_page: if server_url
                .as_ref()
                .is_some_and(|url| !url.trim().is_empty())
            {
                Page::Dashboard
            } else {
                Page::Welcome
            },
            action_count: 0,
            theme_mode: ThemeMode::System,
            server_url,
            refresh_interval_secs,
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
        matches!(&self.server_url, Some(url) if !url.trim().is_empty())
    }

    pub fn set_server_url(&mut self, raw_server_url: String) {
        let trimmed = raw_server_url.trim().to_string();
        self.server_url = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        };

        if self.has_server_url() {
            self.current_page = Page::Dashboard;
        } else {
            self.current_page = Page::Welcome;
        }
    }

    pub fn server_url(&self) -> Option<&str> {
        self.server_url.as_deref()
    }

    pub fn set_refresh_interval(&mut self, secs: u64) {
        self.refresh_interval_secs = secs;
    }
}
