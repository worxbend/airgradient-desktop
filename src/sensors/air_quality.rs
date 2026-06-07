//! AirGradient local-server payload parsing.
//!
//! AirGradient devices can expose slightly different field names depending on
//! hardware model and firmware. The parser accepts several candidate keys for
//! each measurement and returns a normalized `AirMeasureSnapshot` for the UI.

use serde_json::Value;

/// One parsed measurement response from `/measures/current`.
///
/// Every sensor value is optional because not all AirGradient models expose the
/// same fields. In the UI, `None` is displayed as `--`; it is not treated as
/// zero because zero can be a valid real measurement.
#[derive(Debug, Clone, Default)]
pub struct AirMeasureSnapshot {
    pub temperature: Option<f32>,
    pub humidity: Option<f32>,
    pub aqi: Option<f32>,
    pub co2: Option<f32>,
    pub nox: Option<f32>,
    pub nox_unit: Option<&'static str>,
    pub tvoc: Option<f32>,
    pub tvoc_unit: Option<&'static str>,
    pub pm1: Option<f32>,
    pub pm25: Option<f32>,
    pub pm10: Option<f32>,
    pub pm003_count: Option<f32>,
}

pub fn parse_air_measurements(raw: &Value) -> AirMeasureSnapshot {
    // AirGradient's current firmware exposes `noxIndex`, but accepting common
    // alternatives keeps the app usable if payload names change or if users test
    // with compatible local-server implementations.
    let nox = extract_measurement_value(raw, &["nox", "no2", "nox_ppb"])
        .or_else(|| extract_measurement_value(raw, &["noxIndex", "nox_index"]));
    let nox_unit = if nox.is_none() {
        None
    } else if has_any_key(raw, &["noxIndex", "nox_index"]) {
        Some("index")
    } else {
        Some("ppb")
    };

    let tvoc = extract_measurement_value(raw, &["tvoc", "tvoc_ppb", "tvoc_ppm", "voc"])
        .or_else(|| extract_measurement_value(raw, &["tvocIndex", "tvoc_index"]));
    let tvoc_unit = if tvoc.is_none() {
        None
    } else if has_any_key(raw, &["tvocIndex", "tvoc_index"]) {
        Some("index")
    } else {
        Some("ppb")
    };
    let pm25 = extract_measurement_value(raw, &["pm02", "pm2_5", "pm25", "pm2.5"]);

    AirMeasureSnapshot {
        // Prefer compensated temperature/humidity when available because the
        // device can apply model-specific correction before exposing values.
        temperature: extract_measurement_value(
            raw,
            &[
                "atmpCompensated",
                "temperatureCompensated",
                "temperature_compensated",
                "atmp",
                "temperature",
                "temp",
                "temp_c",
                "temperature_c",
                "temperatureC",
            ],
        ),
        humidity: extract_measurement_value(
            raw,
            &[
                "rhumCompensated",
                "humidityCompensated",
                "humidity_compensated",
                "rhum",
                "humidity",
                "hum",
                "relative_humidity",
                "rh",
                "humidity_pct",
            ],
        ),
        aqi: extract_measurement_value(raw, &["aqi", "air_quality_index"])
            .or_else(|| pm25.map(pm25_to_us_aqi)),
        co2: extract_measurement_value(raw, &["rco2", "co2", "co2_ppm"]),
        nox,
        nox_unit,
        tvoc,
        tvoc_unit,
        pm1: extract_measurement_value(raw, &["pm1", "pm1.0", "pm01", "pm_1_0"]),
        pm25,
        pm10: extract_measurement_value(raw, &["pm10", "pm10_0"]),
        pm003_count: extract_measurement_value(raw, &["pm003Count", "pm003_count", "pm0_3_count"]),
    }
}

/// Return the first numeric value found under any candidate key.
///
/// This searches top-level keys first, then recursively searches nested objects
/// and arrays. That makes the parser tolerant of payloads that wrap sensor
/// values in a `measurements` object.
pub fn extract_measurement_value(raw: &Value, candidates: &[&str]) -> Option<f32> {
    candidates.iter().find_map(|name| {
        if let Some(value) = raw.get(*name).and_then(as_f32) {
            return Some(value);
        }

        let lower = name.to_lowercase();
        if let Some(value) = raw.get(lower.as_str()).and_then(as_f32) {
            return Some(value);
        }

        raw.as_object().and_then(|obj| {
            obj.values()
                .find_map(|value| find_nested_key(value, name))
                .or_else(|| {
                    obj.values()
                        .find_map(|value| find_nested_key(value, lower.as_str()))
                })
        })
    })
}

fn as_f32(v: &Value) -> Option<f32> {
    match v {
        Value::Number(num) => num.to_string().parse::<f32>().ok(),
        Value::String(raw) => raw.parse::<f32>().ok(),
        _ => None,
    }
}

fn find_nested_key(raw: &Value, key: &str) -> Option<f32> {
    match raw {
        Value::Object(object) => {
            if let Some(value) = object.get(key) {
                return as_f32(value);
            }
            let lower = key.to_lowercase();
            if let Some(value) = object.get(&lower) {
                return as_f32(value);
            }
            for value in object.values() {
                if let Some(found) = find_nested_key(value, key) {
                    return Some(found);
                }
            }
            None
        }
        Value::Array(items) => {
            for item in items {
                if let Some(found) = find_nested_key(item, key) {
                    return Some(found);
                }
            }
            None
        }
        _ => None,
    }
}

fn has_any_key(raw: &Value, candidates: &[&str]) -> bool {
    candidates.iter().any(|key| {
        let lower = key.to_lowercase();
        has_nested_key(raw, key) || has_nested_key(raw, lower.as_str())
    })
}

fn has_nested_key(raw: &Value, key: &str) -> bool {
    match raw {
        Value::Object(object) => {
            object.contains_key(key) || object.values().any(|value| has_nested_key(value, key))
        }
        Value::Array(items) => items.iter().any(|value| has_nested_key(value, key)),
        _ => false,
    }
}

fn pm25_to_us_aqi(pm25: f32) -> f32 {
    // US AQI linear interpolation breakpoints for PM2.5 concentration. This is
    // used only when the device does not report an AQI value directly.
    const BREAKPOINTS: [(f32, f32, f32, f32); 6] = [
        (0.0, 12.0, 0.0, 50.0),
        (12.1, 35.4, 51.0, 100.0),
        (35.5, 55.4, 101.0, 150.0),
        (55.5, 150.4, 151.0, 200.0),
        (150.5, 250.4, 201.0, 300.0),
        (250.5, 500.4, 301.0, 500.0),
    ];

    for (c_low, c_high, i_low, i_high) in BREAKPOINTS {
        if pm25 >= c_low && pm25 <= c_high {
            return ((i_high - i_low) / (c_high - c_low)) * (pm25 - c_low) + i_low;
        }
    }

    if pm25 < 0.0 {
        0.0
    } else {
        500.0
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::parse_air_measurements;

    #[test]
    fn parses_airgradient_local_server_payload() {
        let payload = json!({
            "wifi": -46,
            "serialno": "ecda3b1eaaaf",
            "rco2": 447,
            "pm01": 3,
            "pm02": 7,
            "pm10": 8,
            "pm003Count": 442,
            "atmp": 25.87,
            "atmpCompensated": 24.47,
            "rhum": 43,
            "rhumCompensated": 49,
            "tvocIndex": 100,
            "tvocRaw": 33051,
            "noxIndex": 1,
            "noxRaw": 16307
        });

        let snapshot = parse_air_measurements(&payload);

        assert_eq!(snapshot.co2, Some(447.0));
        assert_eq!(snapshot.pm1, Some(3.0));
        assert_eq!(snapshot.pm25, Some(7.0));
        assert_eq!(snapshot.pm10, Some(8.0));
        assert_eq!(snapshot.pm003_count, Some(442.0));
        assert_eq!(snapshot.temperature, Some(24.47));
        assert_eq!(snapshot.humidity, Some(49.0));
        assert_eq!(snapshot.tvoc, Some(100.0));
        assert_eq!(snapshot.tvoc_unit, Some("index"));
        assert_eq!(snapshot.nox, Some(1.0));
        assert_eq!(snapshot.nox_unit, Some("index"));
        assert_eq!(snapshot.aqi.map(|value| value.round()), Some(29.0));
    }

    #[test]
    fn parses_nested_payloads_with_numeric_strings() {
        let payload = json!({
            "device": {
                "measurements": [
                    {
                        "rco2": "812",
                        "pm02": "13.2",
                        "atmpCompensated": "22.4",
                        "rhumCompensated": "45.5"
                    },
                    {
                        "tvocIndex": "110",
                        "noxIndex": "3",
                        "pm003Count": "1200"
                    }
                ]
            }
        });

        let snapshot = parse_air_measurements(&payload);

        assert_eq!(snapshot.co2, Some(812.0));
        assert_eq!(snapshot.pm25, Some(13.2));
        assert_eq!(snapshot.temperature, Some(22.4));
        assert_eq!(snapshot.humidity, Some(45.5));
        assert_eq!(snapshot.tvoc, Some(110.0));
        assert_eq!(snapshot.tvoc_unit, Some("index"));
        assert_eq!(snapshot.nox, Some(3.0));
        assert_eq!(snapshot.nox_unit, Some("index"));
        assert_eq!(snapshot.pm003_count, Some(1200.0));
    }
}
