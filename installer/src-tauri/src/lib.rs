use std::process::Command;
use std::path::PathBuf;
use std::fs;
use std::sync::Arc;
use std::io::{Write, Read};
use tauri::State;
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

const INSTALL_DIR: &str = "C:\\Program Files\\VoidEmulator";
const DATA_DIR: &str = "C:\\Program Files\\VoidEmulator\\data";
const QEMU_DIR: &str = "C:\\Program Files\\VoidEmulator\\data\\qemu";
const IMAGES_DIR: &str = "C:\\Program Files\\VoidEmulator\\data\\images";
const INSTANCES_DIR: &str = "C:\\Program Files\\VoidEmulator\\data\\instances";
const RELEASE_JSON: &str = "https://raw.githubusercontent.com/darkflareplays8/VoidEm/main/release.json";
const QEMU_URL: &str = "https://qemu.weilnetz.de/w64/2025/qemu-w64-setup-20251224.exe";

#[derive(Default)]
pub struct InstallState {
    pub log: std::sync::Mutex<Vec<(String, i32)>>,
    pub done: std::sync::Mutex<bool>,
}

#[tauri::command]
fn check_installed() -> bool {
    PathBuf::from(INSTALL_DIR).join("VoidEmulator.exe").exists()
}

#[tauri::command]
fn get_progress(state: State<Arc<InstallState>>) -> serde_json::Value {
    let log = state.log.lock().unwrap();
    let done = state.done.lock().unwrap();
    serde_json::json!({ "log": log.clone(), "done": *done })
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
            push("Downloading QEMU (174MB)...", 3);
            let installer = PathBuf::from(DATA_DIR).join("qemu-setup.exe");
            if let Err(e) = http_download(QEMU_URL, &installer) {
                push(&format!("QEMU download failed: {}", e), -1); return;
            }
            push("Installing QEMU...", 18);
            // Run NSIS installer silently
            Command::new(&installer)
                .args(["/S", &format!("/D={}", QEMU_DIR)])
                .status().ok();
            fs::remove_file(&installer).ok();
            // If not in our dir, find and copy from default location
            if !qemu_exe.exists() {
                let default_dir = PathBuf::from("C:\\Program Files\\qemu");
                if default_dir.exists() {
                    if let Ok(entries) = fs::read_dir(&default_dir) {
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
            if let Err(e) = http_download("https://dl.google.com/android/repository/platform-tools-latest-windows.zip", &zip) {
                push(&format!("ADB download failed: {}", e), -1); return;
            }
            push("Extracting ADB...", 43);
            ps_extract_hidden(&zip, &PathBuf::from(DATA_DIR).join("pt_tmp"));
            let pt = PathBuf::from(DATA_DIR).join("pt_tmp").join("platform-tools");
            for f in &["adb.exe", "AdbWinApi.dll", "AdbWinUsbApi.dll"] {
                let src = pt.join(f);
                if src.exists() { fs::copy(&src, PathBuf::from(QEMU_DIR).join(f)).ok(); }
            }
            fs::remove_dir_all(PathBuf::from(DATA_DIR).join("pt_tmp")).ok();
            fs::remove_file(&zip).ok();
        }
        push("ADB ready", 47);

        // 3. Android image
        let base_img = PathBuf::from(IMAGES_DIR).join("android.img");
        if !base_img.exists() {
            push("Downloading Android-x86 (~300MB)...", 49);
            let iso = PathBuf::from(IMAGES_DIR).join("android.iso");
            if let Err(e) = http_download("https://sourceforge.net/projects/android-x86/files/Release%204.4-r5/android-x86-4.4-r5.iso/download", &iso) {
                push(&format!("Android download failed: {}", e), -1); return;
            }
            push("Creating disk image...", 86);
            Command::new(PathBuf::from(QEMU_DIR).join("qemu-img.exe"))
                .args(["create", "-f", "raw", base_img.to_str().unwrap(), "4G"])
                .creation_flags(0x08000000) // CREATE_NO_WINDOW
                .output().ok();
            fs::remove_file(&iso).ok();
        }
        push("Android ready", 88);

        // 4. VoidEmulator.exe
        push("Fetching latest version...", 89);
        let json = match ureq::get(RELEASE_JSON).call() {
            Ok(r) => r.into_string().unwrap_or_default(),
            Err(e) => { push(&format!("release.json failed: {}", e), -1); return; }
        };
        let url = serde_json::from_str::<serde_json::Value>(&json)
            .ok().and_then(|j| j["url"].as_str().map(|s| s.to_string()))
            .unwrap_or_default();
        if url.is_empty() { push("No URL in release.json!", -1); return; }

        push("Downloading VoidEmulator.exe...", 90);
        let exe_dest = PathBuf::from(INSTALL_DIR).join("VoidEmulator.exe");
        if let Err(e) = http_download(&url, &exe_dest) {
            push(&format!("VoidEmulator download failed: {}", e), -1); return;
        }
        push("VoidEmulator installed", 97);

        // 5. Shortcuts
        push("Creating shortcuts...", 98);
        let target = PathBuf::from(INSTALL_DIR).join("VoidEmulator.exe");
        if let Ok(appdata) = std::env::var("APPDATA") {
            create_shortcut(&target, &PathBuf::from(appdata).join("Microsoft\\Windows\\Start Menu\\Programs\\VoidEmulator.lnk"));
        }
        if let Ok(profile) = std::env::var("USERPROFILE") {
            create_shortcut(&target, &PathBuf::from(profile).join("Desktop\\VoidEmulator.lnk"));
        }

        push("Installation complete!", 100);
        *state.done.lock().unwrap() = true;
    });
    true
}

#[tauri::command]
fn launch_app() {
    Command::new(PathBuf::from(INSTALL_DIR).join("VoidEmulator.exe")).spawn().ok();
}

fn http_download(url: &str, dest: &PathBuf) -> Result<(), String> {
    let resp = ureq::get(url).call().map_err(|e| e.to_string())?;
    let mut file = fs::File::create(dest).map_err(|e| e.to_string())?;
    let mut reader = resp.into_reader();
    let mut buf = vec![0u8; 65536];
    loop {
        let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 { break; }
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn ps_extract_hidden(zip: &PathBuf, dest: &PathBuf) {
    fs::create_dir_all(dest).ok();
    Command::new("powershell")
        .args(["-WindowStyle", "Hidden", "-Command", &format!(
            "$ProgressPreference='SilentlyContinue'; Expand-Archive -Path '{}' -DestinationPath '{}' -Force",
            zip.to_str().unwrap(), dest.to_str().unwrap()
        )])
        .creation_flags(0x08000000) // CREATE_NO_WINDOW
        .output().ok();
}

fn create_shortcut(target: &PathBuf, shortcut: &PathBuf) {
    Command::new("powershell")
        .args(["-WindowStyle", "Hidden", "-Command", &format!(
            "$ws = New-Object -ComObject WScript.Shell; $s = $ws.CreateShortcut('{}'); $s.TargetPath = '{}'; $s.Save()",
            shortcut.to_str().unwrap(), target.to_str().unwrap()
        )])
        .creation_flags(0x08000000)
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