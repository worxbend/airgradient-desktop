use std::env;
use std::fs::{create_dir_all, read_to_string, write};
use std::io;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

const DEFAULT_REFRESH_INTERVAL_SECS: u64 = 30;

#[derive(Debug, Serialize, Deserialize)]
pub struct AppConfig {
    pub server_url: Option<String>,
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
