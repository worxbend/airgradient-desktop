use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use airgradient_desktop::config::{
    read_config_from_path, write_config_to_path, AppConfig, ConfigStartupNotice, RefreshInterval,
};
use airgradient_desktop::device::DeviceBaseUrl;

#[test]
fn config_round_trip_uses_validated_values() {
    let path = unique_config_path("round-trip");
    let config = AppConfig {
        server_url: Some(
            DeviceBaseUrl::parse("192.168.1.201")
                .expect("URL should parse")
                .expect("URL should be configured"),
        ),
        refresh_interval: RefreshInterval::new(45).expect("interval should be valid"),
        notifications_enabled: false,
        start_minimized: true,
    };

    write_config_to_path(&path, &config).expect("config should write");
    let loaded = read_config_from_path(&path);

    assert_eq!(loaded.startup_notice, None);
    assert_eq!(
        loaded.config.server_url.as_ref().map(DeviceBaseUrl::as_str),
        Some("http://192.168.1.201")
    );
    assert_eq!(loaded.config.refresh_interval.as_secs(), 45);
    assert!(!loaded.config.notifications_enabled);
    assert!(loaded.config.start_minimized);
}

#[test]
fn missing_config_reports_first_launch_with_defaults() {
    let path = unique_config_path("missing");

    let loaded = read_config_from_path(&path);

    assert_eq!(
        loaded.startup_notice,
        Some(ConfigStartupNotice::FirstLaunch)
    );
    assert_eq!(loaded.config.server_url, None);
    assert_eq!(loaded.config.refresh_interval, RefreshInterval::default());
}

#[test]
fn malformed_config_reports_parse_failure_with_defaults() {
    let path = unique_config_path("malformed");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("test dir should be created");
    }
    fs::write(&path, "{ not json").expect("malformed config should be written");

    let loaded = read_config_from_path(&path);

    assert!(matches!(
        loaded.startup_notice,
        Some(ConfigStartupNotice::ParseFailed(_))
    ));
    assert_eq!(loaded.config.server_url, None);
    assert_eq!(loaded.config.refresh_interval, RefreshInterval::default());
}

fn unique_config_path(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after epoch")
        .as_nanos();
    std::env::temp_dir()
        .join(format!("airgradient-desktop-{name}-{nanos}"))
        .join("config.json")
}
