//! AirGradient device access.
//!
//! This module owns local-server URL normalization and the blocking HTTP request
//! to `/measures/current`. Keeping it outside the GTK window code gives the UI a
//! small application-facing boundary to call from `gio::spawn_blocking`.

use std::fmt;
use std::time::Duration;

use reqwest::blocking::Client;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use url::Url;

use crate::sensors::{parse_air_measurements, AirMeasureSnapshot};

const REQUEST_TIMEOUT_SECS: u64 = 8;

pub type DeviceResult<T> = Result<T, DeviceError>;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DeviceBaseUrl(String);

impl DeviceBaseUrl {
    pub fn parse(raw: &str) -> DeviceResult<Option<Self>> {
        parse_server_url(raw).map(|url| url.map(Self))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Display for DeviceBaseUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for DeviceBaseUrl {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for DeviceBaseUrl {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Self::parse(&raw)
            .map_err(serde::de::Error::custom)?
            .ok_or_else(|| serde::de::Error::custom("device base URL cannot be empty"))
    }
}

#[derive(Debug)]
pub enum DeviceError {
    InvalidUrl(url::ParseError),
    UnsupportedScheme(String),
    MissingHost,
    NotConfigured,
    HttpClient(reqwest::Error),
    Request(reqwest::Error),
    HttpStatus(reqwest::StatusCode),
    Json(reqwest::Error),
}

impl fmt::Display for DeviceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidUrl(err) => write!(f, "Invalid URL: {err}"),
            Self::UnsupportedScheme(scheme) => {
                write!(f, "Invalid URL scheme '{scheme}'. Use http or https.")
            }
            Self::MissingHost => f.write_str("URL missing host component."),
            Self::NotConfigured => f.write_str("No server URL configured."),
            Self::HttpClient(err) => write!(f, "HTTP client error: {err}"),
            Self::Request(err) => write!(f, "Request failed: {err}"),
            Self::HttpStatus(status) => write!(f, "Server returned HTTP {status}"),
            Self::Json(err) => write!(f, "Invalid JSON response: {err}"),
        }
    }
}

impl std::error::Error for DeviceError {}

pub fn fetch_current_measurements(base_url: &DeviceBaseUrl) -> DeviceResult<AirMeasureSnapshot> {
    let url = format!(
        "{}/measures/current",
        base_url.as_str().trim_end_matches('/')
    );

    // The client is small enough to create per request. If the app grows into a
    // high-frequency poller, this could be moved into shared state.
    let client = Client::builder()
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .build()
        .map_err(DeviceError::HttpClient)?;

    let response = client.get(url).send().map_err(DeviceError::Request)?;
    if !response.status().is_success() {
        return Err(DeviceError::HttpStatus(response.status()));
    }

    let payload: Value = response.json().map_err(DeviceError::Json)?;
    Ok(parse_air_measurements(&payload))
}

pub fn parse_server_url(raw: &str) -> DeviceResult<Option<String>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    let candidate = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        // Users commonly paste just an IP address. Default to HTTP because the
        // AirGradient local server is normally plain HTTP on the local network.
        format!("http://{trimmed}")
    };

    let mut parsed = Url::parse(&candidate).map_err(DeviceError::InvalidUrl)?;

    match parsed.scheme() {
        "http" | "https" => {}
        scheme => return Err(DeviceError::UnsupportedScheme(scheme.to_string())),
    }

    if parsed.host().is_none() {
        return Err(DeviceError::MissingHost);
    }

    // Store only the base URL. Fetching always appends `/measures/current`.
    parsed.set_path("");
    parsed.set_query(None);
    parsed.set_fragment(None);

    Ok(Some(parsed.to_string().trim_end_matches('/').to_string()))
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread::{self, JoinHandle};

    use super::{fetch_current_measurements, parse_server_url, DeviceBaseUrl, DeviceError};

    #[test]
    fn parse_server_url_accepts_empty_value_as_not_configured() {
        assert_eq!(parse_server_url("   ").expect("empty URL is valid"), None);
    }

    #[test]
    fn parse_server_url_defaults_bare_host_to_http() {
        let normalized = parse_server_url("192.168.1.201").expect("bare host should parse");

        assert_eq!(normalized.as_deref(), Some("http://192.168.1.201"));
    }

    #[test]
    fn parse_server_url_keeps_scheme_host_and_port_only() {
        let normalized =
            parse_server_url(" https://airgradient.local:8443/measures/current?x=1#readings ")
                .expect("URL with path should parse");

        assert_eq!(
            normalized.as_deref(),
            Some("https://airgradient.local:8443")
        );
    }

    #[test]
    fn parse_server_url_rejects_unsupported_schemes() {
        let err = parse_server_url("ftp://airgradient.local").expect_err("ftp is unsupported");

        assert!(matches!(
            err,
            DeviceError::UnsupportedScheme(ref scheme) if scheme == "ftp"
        ));
        assert!(err.to_string().contains("Invalid URL scheme 'ftp'"));
    }

    #[test]
    fn parse_server_url_rejects_urls_without_host() {
        let err = parse_server_url("http://").expect_err("host is required");

        assert!(matches!(
            err,
            DeviceError::InvalidUrl(_) | DeviceError::MissingHost
        ));
    }

    #[test]
    fn fetch_current_measurements_requests_current_endpoint_and_parses_payload() {
        let body = r#"{"rco2":447,"pm02":7,"atmpCompensated":24.47,"rhumCompensated":49}"#;
        let (base_url, server) = serve_once("HTTP/1.1 200 OK", body);
        let base_url = DeviceBaseUrl::parse(&base_url)
            .expect("test server URL should parse")
            .expect("test server URL should be configured");

        let snapshot = fetch_current_measurements(&base_url).expect("fetch should parse payload");

        assert_eq!(snapshot.co2, Some(447.0));
        assert_eq!(snapshot.pm25, Some(7.0));
        assert_eq!(snapshot.temperature, Some(24.47));
        assert_eq!(snapshot.humidity, Some(49.0));
        server.join().expect("test server should complete");
    }

    #[test]
    fn fetch_current_measurements_reports_http_status_errors() {
        let (base_url, server) = serve_once("HTTP/1.1 503 Service Unavailable", "{}");
        let base_url = DeviceBaseUrl::parse(&base_url)
            .expect("test server URL should parse")
            .expect("test server URL should be configured");

        let err = fetch_current_measurements(&base_url).expect_err("503 should fail");

        assert!(matches!(err, DeviceError::HttpStatus(status) if status.as_u16() == 503));
        server.join().expect("test server should complete");
    }

    fn serve_once(status_line: &'static str, body: &'static str) -> (String, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let address = listener.local_addr().expect("read test server address");
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept one request");
            let mut request = [0_u8; 1024];
            let bytes_read = stream.read(&mut request).expect("read request");
            let request = String::from_utf8_lossy(&request[..bytes_read]);
            assert!(request.starts_with("GET /measures/current "));

            let response = format!(
                "{status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream
                .write_all(response.as_bytes())
                .expect("write response");
        });

        (format!("http://{address}"), handle)
    }
}
