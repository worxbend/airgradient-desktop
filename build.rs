use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
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
