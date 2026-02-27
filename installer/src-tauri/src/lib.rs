use std::process::Command;
use std::path::PathBuf;
use std::fs;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Manager, State};

const INSTALL_DIR: &str = "C:\\Program Files\\VoidEmulator";
const DATA_DIR: &str = "C:\\Program Files\\VoidEmulator\\data";
const QEMU_DIR: &str = "C:\\Program Files\\VoidEmulator\\data\\qemu";
const IMAGES_DIR: &str = "C:\\Program Files\\VoidEmulator\\data\\images";
const INSTANCES_DIR: &str = "C:\\Program Files\\VoidEmulator\\data\\instances";
const RELEASE_JSON: &str = "https://raw.githubusercontent.com/darkflareplays8/VoidEm/main/release.json";

#[derive(Default)]
pub struct InstallState {
    pub log: Mutex<Vec<(String, i32)>>,
    pub done: Mutex<bool>,
}

#[tauri::command]
fn check_installed() -> bool {
    PathBuf::from(INSTALL_DIR).join("VoidEmulator.exe").exists()
}

#[tauri::command]
fn get_progress(state: State<Arc<InstallState>>) -> serde_json::Value {
    let log = state.log.lock().unwrap();
    let done = state.done.lock().unwrap();
    serde_json::json!({
        "log": log.clone(),
        "done": *done
    })
}

#[tauri::command]
fn start_install(state: State<Arc<InstallState>>) -> bool {
    let state = Arc::clone(&state);
    std::thread::spawn(move || {
        let push = |msg: &str, pct: i32| {
            state.log.lock().unwrap().push((msg.to_string(), pct));
        };

        for dir in &[INSTALL_DIR, DATA_DIR, QEMU_DIR, IMAGES_DIR, INSTANCES_DIR] {
            fs::create_dir_all(dir).ok();
        }

        push("Starting installation...", 1);

        // 1. QEMU
        let qemu_exe = PathBuf::from(QEMU_DIR).join("qemu-system-i386.exe");
        if !qemu_exe.exists() {
            push("Downloading QEMU...", 5);
            let installer = PathBuf::from(DATA_DIR).join("qemu-setup.exe");
            if !ps_download("https://qemu.weilnetz.de/w64/qemu-w64-setup-20251217.exe", &installer) {
                push("QEMU download failed!", -1); return;
            }
            push("Installing QEMU silently...", 18);
            // Silent install to our dir
            let install_path = PathBuf::from(QEMU_DIR);
            Command::new(&installer)
                .args(["/S", &format!("/D={}", install_path.to_str().unwrap())])
                .output().ok();
            fs::remove_file(&installer).ok();
            // QEMU installer puts files directly in the dir
            if !qemu_exe.exists() {
                // Try default install location
                let default = PathBuf::from("C:\\Program Files\\qemu\\qemu-system-i386.exe");
                if default.exists() {
                    // Copy all qemu files over
                    if let Ok(entries) = fs::read_dir("C:\\Program Files\\qemu") {
                        for e in entries.flatten() {
                            fs::copy(e.path(), PathBuf::from(QEMU_DIR).join(e.file_name())).ok();
                        }
                    }
                }
            }
        }
        push("QEMU ready", 25);

        // 2. ADB
        let adb_exe = PathBuf::from(QEMU_DIR).join("adb.exe");
        if !adb_exe.exists() {
            push("Downloading ADB tools...", 27);
            let zip = PathBuf::from(DATA_DIR).join("adb.zip");
            if !ps_download("https://dl.google.com/android/repository/platform-tools-latest-windows.zip", &zip) {
                push("ADB download failed!", -1); return;
            }
            push("Extracting ADB...", 43);
            let tmp = PathBuf::from(DATA_DIR).join("pt_tmp");
            ps_extract(&zip, &tmp);
            let pt = tmp.join("platform-tools");
            for f in &["adb.exe", "AdbWinApi.dll", "AdbWinUsbApi.dll"] {
                let src = pt.join(f);
                if src.exists() { fs::copy(&src, PathBuf::from(QEMU_DIR).join(f)).ok(); }
            }
            fs::remove_dir_all(&tmp).ok();
            fs::remove_file(&zip).ok();
        }
        push("ADB ready", 47);

        // 3. Android image
        let base_img = PathBuf::from(IMAGES_DIR).join("android.img");
        if !base_img.exists() {
            push("Downloading Android-x86 (~300MB)...", 49);
            let iso = PathBuf::from(IMAGES_DIR).join("android.iso");
            if !ps_download("https://sourceforge.net/projects/android-x86/files/Release%204.4-r5/android-x86-4.4-r5.iso/download", &iso) {
                push("Android download failed!", -1); return;
            }
            push("Creating disk image...", 86);
            Command::new(PathBuf::from(QEMU_DIR).join("qemu-img.exe"))
                .args(["create", "-f", "raw", base_img.to_str().unwrap(), "4G"])
                .output().ok();
            fs::remove_file(&iso).ok();
        }
        push("Android ready", 88);

        // 4. VoidEmulator.exe
        push("Fetching latest version...", 89);
        let json = ps_fetch(RELEASE_JSON);
        let url = serde_json::from_str::<serde_json::Value>(&json)
            .ok()
            .and_then(|j| j["url"].as_str().map(|s| s.to_string()))
            .unwrap_or_default();
        if url.is_empty() { push("Failed to read release.json!", -1); return; }

        push("Downloading VoidEmulator.exe...", 90);
        let exe_dest = PathBuf::from(INSTALL_DIR).join("VoidEmulator.exe");
        if !ps_download(&url, &exe_dest) { push("VoidEmulator download failed!", -1); return; }
        push("VoidEmulator installed", 97);

        // 5. Shortcuts
        push("Creating shortcuts...", 98);
        create_shortcut(
            &PathBuf::from(INSTALL_DIR).join("VoidEmulator.exe"),
            &PathBuf::from(std::env::var("APPDATA").unwrap_or_default())
                .join("Microsoft\\Windows\\Start Menu\\Programs\\VoidEmulator.lnk")
        );
        create_shortcut(
            &PathBuf::from(INSTALL_DIR).join("VoidEmulator.exe"),
            &PathBuf::from(std::env::var("USERPROFILE").unwrap_or_default())
                .join("Desktop\\VoidEmulator.lnk")
        );

        push("Installation complete!", 100);
        *state.done.lock().unwrap() = true;
    });
    true
}

#[tauri::command]
fn launch_app() {
    Command::new(PathBuf::from(INSTALL_DIR).join("VoidEmulator.exe")).spawn().ok();
}

fn ps_download(url: &str, dest: &PathBuf) -> bool {
    Command::new("powershell")
        .args(["-Command", &format!(
            "$ProgressPreference='SilentlyContinue'; Invoke-WebRequest -Uri '{}' -OutFile '{}'",
            url, dest.to_str().unwrap()
        )])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn ps_extract(zip: &PathBuf, dest: &PathBuf) {
    fs::create_dir_all(dest).ok();
    Command::new("powershell")
        .args(["-Command", &format!(
            "$ProgressPreference='SilentlyContinue'; Expand-Archive -Path '{}' -DestinationPath '{}' -Force",
            zip.to_str().unwrap(), dest.to_str().unwrap()
        )])
        .output().ok();
}

fn ps_fetch(url: &str) -> String {
    Command::new("powershell")
        .args(["-Command", &format!(
            "$ProgressPreference='SilentlyContinue'; (Invoke-WebRequest -Uri '{}').Content", url
        )])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

fn find_file(dir: &PathBuf, name: &str) -> Option<PathBuf> {
    if !dir.exists() { return None; }
    for entry in fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.is_dir() { if let Some(f) = find_file(&path, name) { return Some(f); } }
        else if path.file_name()?.to_str()? == name { return Some(path); }
    }
    None
}

fn create_shortcut(target: &PathBuf, shortcut: &PathBuf) {
    Command::new("powershell")
        .args(["-Command", &format!(
            "$ws = New-Object -ComObject WScript.Shell; $s = $ws.CreateShortcut('{}'); $s.TargetPath = '{}'; $s.Save()",
            shortcut.to_str().unwrap(), target.to_str().unwrap()
        )])
        .output().ok();
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let state = Arc::new(InstallState::default());
    tauri::Builder::default()
        .manage(state)
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![
            check_installed, start_install, get_progress, launch_app
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}