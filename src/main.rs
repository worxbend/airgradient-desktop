//! Binary entry point.
//!
//! Rust programs start in `main()`. This file delegates real application
//! startup to the library crate, keeping the binary easy to scan.

fn main() {
    airgradient_desktop::run();
}
