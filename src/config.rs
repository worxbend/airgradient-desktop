//! User configuration persistence.
//!
//! Air Monitor stores durable settings as JSON under the XDG config directory.
//! Runtime-only values, such as the last measurements kept for trend display,
//! intentionally stay in memory and are not written here.

use std::env;
use std::fmt;
use std::fs::{create_dir_all, read_to_string, write};
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::device::DeviceBaseUrl;

pub const DEFAULT_REFRESH_INTERVAL_SECS: u64 = 30;
pub const MIN_REFRESH_INTERVAL_SECS: u64 = 5;
pub const MAX_REFRESH_INTERVAL_SECS: u64 = 3600;

#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd)]
pub struct RefreshInterval(u64);

impl RefreshInterval {
    pub const DEFAULT: Self = Self(DEFAULT_REFRESH_INTERVAL_SECS);

    pub fn new(secs: u64) -> Result<Self, RefreshIntervalError> {
        if secs < MIN_REFRESH_INTERVAL_SECS {
            return Err(RefreshIntervalError::TooShort(secs));
        }
        if secs > MAX_REFRESH_INTERVAL_SECS {
            return Err(RefreshIntervalError::TooLong(secs));
        }
        Ok(Self(secs))
    }

    pub fn clamped(secs: u64) -> Self {
        Self(secs.clamp(MIN_REFRESH_INTERVAL_SECS, MAX_REFRESH_INTERVAL_SECS))
    }

    pub const fn as_secs(self) -> u64 {
        self.0
    }
}

impl Default for RefreshInterval {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl Serialize for RefreshInterval {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for RefreshInterval {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let secs = u64::deserialize(deserializer)?;
        Self::new(secs).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum RefreshIntervalError {
    TooShort(u64),
    TooLong(u64),
}

impl fmt::Display for RefreshIntervalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooShort(secs) => write!(
                f,
                "refresh interval {secs}s is below the {MIN_REFRESH_INTERVAL_SECS}s minimum"
            ),
            Self::TooLong(secs) => write!(
                f,
                "refresh interval {secs}s is above the {MAX_REFRESH_INTERVAL_SECS}s maximum"
            ),
        }
    }
}

impl std::error::Error for RefreshIntervalError {}

#[derive(Debug, Serialize, Deserialize)]
pub struct AppConfig {
    /// Base URL of the local AirGradient device, for example
    /// `http://192.168.1.201`.
    pub server_url: Option<DeviceBaseUrl>,
    /// Seconds between automatic refreshes. The default is used when older
    /// config files do not contain the field.
    #[serde(rename = "refresh_interval_secs", default = "default_refresh_interval")]
    pub refresh_interval: RefreshInterval,
    /// Whether desktop notifications should be sent for air-quality insights.
    #[serde(default = "default_notifications_enabled")]
    pub notifications_enabled: bool,
    /// Whether the app should start hidden and keep polling in the background.
    #[serde(default)]
    pub start_minimized: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            server_url: None,
            refresh_interval: RefreshInterval::DEFAULT,
            notifications_enabled: default_notifications_enabled(),
            start_minimized: false,
        }
    }
}

fn default_refresh_interval() -> RefreshInterval {
    RefreshInterval::DEFAULT
}

fn default_notifications_enabled() -> bool {
    true
}

#[derive(Debug)]
pub struct LoadedConfig {
    pub config: AppConfig,
    pub startup_notice: Option<ConfigStartupNotice>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum ConfigStartupNotice {
    FirstLaunch,
    ReadFailed(String),
    ParseFailed(String),
}

impl ConfigStartupNotice {
    pub fn user_message(&self) -> String {
        match self {
            Self::FirstLaunch => "No saved settings yet. Defaults are loaded.".to_string(),
            Self::ReadFailed(err) => {
                format!("Settings could not be read, so defaults were loaded: {err}")
            }
            Self::ParseFailed(err) => {
                format!("Settings could not be parsed, so defaults were loaded: {err}")
            }
        }
    }
}

pub fn read_config() -> LoadedConfig {
    read_config_from_path(&config_path())
}

pub fn read_config_from_path(path: &Path) -> LoadedConfig {
    match read_to_string(path) {
        Ok(raw) => match serde_json::from_str(&raw) {
            Ok(config) => LoadedConfig {
                config,
                startup_notice: None,
            },
            Err(err) => LoadedConfig {
                config: AppConfig::default(),
                startup_notice: Some(ConfigStartupNotice::ParseFailed(err.to_string())),
            },
        },
        Err(err) if err.kind() == io::ErrorKind::NotFound => LoadedConfig {
            config: AppConfig::default(),
            startup_notice: Some(ConfigStartupNotice::FirstLaunch),
        },
        Err(err) => LoadedConfig {
            config: AppConfig::default(),
            startup_notice: Some(ConfigStartupNotice::ReadFailed(err.to_string())),
        },
    }
}

pub fn write_config(config: &AppConfig) -> io::Result<()> {
    let config_dir = config_dir();
    create_dir_all(&config_dir)?;
    write_config_to_path(&config_path(), config)
}

pub fn write_config_to_path(path: &Path, config: &AppConfig) -> io::Result<()> {
    if let Some(config_dir) = path.parent() {
        create_dir_all(config_dir)?;
    }
    let raw = serde_json::to_string_pretty(config).map_err(io::Error::other)?;
    write(path, raw)
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.json")
}

pub fn config_dir() -> PathBuf {
    // Follow the XDG Base Directory convention first. If it is unavailable,
    // fall back to `$HOME/.config`, then finally to the current directory so the
    // app still has a deterministic path in unusual environments.
    env::var("XDG_CONFIG_HOME")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            env::var("HOME")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .map(|home| PathBuf::from(home).join(".config"))
        })
        .unwrap_or_else(|| PathBuf::from("."))
        .join("airgradient-desktop")
}
