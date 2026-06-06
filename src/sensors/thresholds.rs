//! Sensor threshold color mapping.
//!
//! These helpers convert measurements into `gdk::RGBA` colors used by the
//! dashboard. Color values come from the GNOME palette so the UI remains aligned
//! with the GNOME Human Interface Guidelines.

use gtk4::gdk;

#[inline]
fn rgba_u8(r: u8, g: u8, b: u8) -> gdk::RGBA {
    // GTK stores color channels as floats from 0.0 to 1.0. The GNOME palette is
    // easier for humans to read as 8-bit RGB, so this helper converts it.
    gdk::RGBA::new(
        f32::from(r) / 255.0,
        f32::from(g) / 255.0,
        f32::from(b) / 255.0,
        1.0,
    )
}

pub fn co2_status_color(value: f32) -> gdk::RGBA {
    // CO2 thresholds: excellent/acceptable/moderate/high.
    match value {
        x if x < 800.0 => rgba_u8(51, 209, 122),  // Green 3
        x if x < 1200.0 => rgba_u8(245, 194, 17), // Yellow 4
        x if x < 2000.0 => rgba_u8(255, 120, 0),  // Orange 3
        _ => rgba_u8(237, 51, 59),                // Red 2
    }
}

pub fn pm25_status_color(value: f32) -> gdk::RGBA {
    // PM2.5 thresholds use common AQI breakpoints.
    match value {
        x if x < 12.0 => rgba_u8(51, 209, 122), // Green 3
        x if x < 35.0 => rgba_u8(245, 194, 17), // Yellow 4
        x if x < 55.0 => rgba_u8(255, 120, 0),  // Orange 3
        _ => rgba_u8(237, 51, 59),              // Red 2
    }
}

pub fn tvoc_status_color(value: f32) -> gdk::RGBA {
    match value {
        x if x < 65.0 => rgba_u8(51, 209, 122),
        x if x < 220.0 => rgba_u8(245, 194, 17),
        x if x < 660.0 => rgba_u8(255, 120, 0),
        _ => rgba_u8(237, 51, 59),
    }
}

pub fn nox_status_color(value: f32) -> gdk::RGBA {
    match value {
        x if x < 20.0 => rgba_u8(51, 209, 122),
        x if x < 50.0 => rgba_u8(245, 194, 17),
        x if x < 150.0 => rgba_u8(255, 120, 0),
        _ => rgba_u8(237, 51, 59),
    }
}

pub fn aqi_status_color(value: f32) -> gdk::RGBA {
    match value {
        x if x <= 50.0 => rgba_u8(51, 209, 122),
        x if x <= 100.0 => rgba_u8(245, 194, 17),
        x if x <= 150.0 => rgba_u8(255, 120, 0),
        x if x <= 200.0 => rgba_u8(237, 51, 59),
        x if x <= 300.0 => rgba_u8(145, 65, 172),
        _ => rgba_u8(94, 92, 100),
    }
}
