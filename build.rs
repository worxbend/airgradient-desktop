//! Build script for GTK resources.
//!
//! Cargo runs `build.rs` before compiling the crate. Here we call
//! `glib-compile-resources` so GTK can load the app's symbolic SVG icons from an
//! embedded GResource instead of loose files on disk.

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    // Cargo provides OUT_DIR for generated build artifacts. The compiled
    // resource is later included by `gio::resources_register_include!`.
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR is set by Cargo"));
    let target = out_dir.join("airgradient.gresource");

    let status = Command::new("glib-compile-resources")
        .arg("resources/airgradient.gresource.xml")
        .arg("--target")
        .arg(&target)
        .arg("--sourcedir")
        .arg("resources")
        .status()
        .expect("failed to run glib-compile-resources");

    if !status.success() {
        panic!("glib-compile-resources failed with status {status}");
    }

    // These lines tell Cargo when to rerun the build script. Without them,
    // editing an SVG might not rebuild the embedded resource.
    println!("cargo:rerun-if-changed=resources/airgradient.gresource.xml");
    for icon in [
        "airgradient-temperature-symbolic.svg",
        "airgradient-humidity-symbolic.svg",
        "airgradient-air-quality-symbolic.svg",
        "airgradient-co2-symbolic.svg",
        "airgradient-voc-symbolic.svg",
        "airgradient-nox-symbolic.svg",
        "airgradient-particles-symbolic.svg",
    ] {
        println!("cargo:rerun-if-changed=resources/icons/scalable/status/{icon}");
    }
}
