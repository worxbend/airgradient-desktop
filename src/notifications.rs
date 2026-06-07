//! Desktop notification delivery.
//!
//! The alert policy decides *what* should notify. This infrastructure module
//! owns *how* to deliver it through available Linux desktop notification paths.

use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use adw::prelude::ApplicationExt;

use crate::alerts::{AlertNotification, AlertSeverity};
use crate::app_info::{APP_ID, APP_NAME};

static NOTIFICATION_SEQUENCE: AtomicU64 = AtomicU64::new(1);

pub fn send_air_quality_notification(
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
        .hint(notify_rust::Hint::DesktopEntry(APP_ID.into()))
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
