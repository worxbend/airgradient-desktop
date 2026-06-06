//! User configuration persistence.
//!
//! Air Monitor stores durable settings as JSON under the XDG config directory.
//! Runtime-only values, such as the last measurements kept for trend display,
//! intentionally stay in memory and are not written here.

use std::env;
use std::fs::{create_dir_all, read_to_string, write};
use std::io;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

const DEFAULT_REFRESH_INTERVAL_SECS: u64 = 30;

#[derive(Debug, Serialize, Deserialize)]
pub struct AppConfig {
    /// Base URL of the local AirGradient device, for example
    /// `http://192.168.1.201`.
    pub server_url: Option<String>,
    /// Seconds between automatic refreshes. The default is used when older
    /// config files do not contain the field.
    #[serde(default = "default_refresh_interval")]
    pub refresh_interval_secs: u64,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            server_url: None,
            refresh_interval_secs: DEFAULT_REFRESH_INTERVAL_SECS,
        }
    }
}

fn default_refresh_interval() -> u64 {
    DEFAULT_REFRESH_INTERVAL_SECS
}

pub fn read_config() -> io::Result<AppConfig> {
    let config_path = config_path();
    match read_to_string(config_path) {
        Ok(raw) => serde_json::from_str(&raw).map_err(io::Error::other),
        // First launch is not an error. Treat a missing file as defaults.
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(AppConfig::default()),
        Err(err) => Err(err),
    }
}

pub fn write_config(config: &AppConfig) -> io::Result<()> {
    let config_dir = config_dir();
    create_dir_all(&config_dir)?;
    let raw = serde_json::to_string_pretty(config).map_err(io::Error::other)?;
    write(config_path(), raw)
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
