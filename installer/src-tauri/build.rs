use std::fs;

fn main() {
    // Read version from release.json at root of workspace
    let release = fs::read_to_string("../../release.json")
        .expect("release.json not found");
    let json: serde_json::Value = serde_json::from_str(&release)
        .expect("Invalid release.json");
    let version = json["version"].as_str().expect("No version in release.json");

    // Expose as env var to the rest of the build
    println!("cargo:rustc-env=APP_VERSION={}", version);
    println!("cargo:rerun-if-changed=../../release.json");

    tauri_build::build()
}