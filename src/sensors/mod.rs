//! Sensor parsing and threshold logic.
//!
//! The UI should not need to know every possible JSON key emitted by different
//! AirGradient firmware versions. This module exposes a normalized snapshot and
//! color helpers instead.

pub mod air_quality;
pub mod thresholds;

pub use air_quality::{parse_air_measurements, AirMeasureSnapshot};
