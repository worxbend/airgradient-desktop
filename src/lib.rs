//! Library entry point for AirGradient Desktop.
//!
//! Keeping module declarations here lets the binary stay small and gives
//! integration tests access to the same code paths the application uses.

pub mod alerts;
pub mod app;
pub mod app_info;
pub mod config;
pub mod device;
pub mod notifications;
pub mod sensors;
pub mod state;
pub mod ui;

pub use app::run;
