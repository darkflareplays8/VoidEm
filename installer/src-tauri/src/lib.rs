use std::process::Command;
const VERSION: &str = env!("APP_VERSION");
use std::path::PathBuf;
use std::fs;
use tauri::{AppHandle, Emitter};

const INSTALL_DIR: &str = "C:\\Program Files\\VoidEmulator";
const DATA_DIR: &str = "C:\\Program Files\\VoidEmulator\\data";
const QEMU_DIR: &str = "C:\\Program Files\\VoidEmulator\\data\\qemu";
const IMAGES_DIR: &str = "C:\\Program Files\\VoidEmulator\\data\\images";
const INSTANCES_DIR: &str = "C:\\Program Files\\VoidEmulator\\data\\instances";
const RELEASE_JSON: &str = "https://raw.githubusercontent.com/darkflareplays8/VoidEm/main/release.json";

fn send(app: &AppHandle, msg: &str, pct: i32) {
    app.emit("install-progress", serde_json::json!({ "msg": msg, "pct": pct })).ok();
}

#[tauri::command]
fn check_installed() -> bool {
    PathBuf::from(INSTALL_DIR).join("VoidEmulator.exe").exists()
}

#[tauri::command]
async fn run_install(app: AppHandle) -> serde_json::Value {
    let app2 = app.clone();
    std::thread::spawn(move || {
        let app = app2;

        // Create all dirs
        for dir in &[INSTALL_DIR, DATA_DIR, QEMU_DIR, IMAGES_DIR, INSTANCES_DIR] {
            fs::create_dir_all(dir).ok();
        }

        send(&app, "Starting installation...", 1);

        // 1. Download QEMU
        let qemu_exe = PathBuf::from(QEMU_DIR).join("qemu-system-i386.exe");
        if !qemu_exe.exists() {
            send(&app, "Downloading QEMU emulator...", 3);
            let zip = PathBuf::from(DATA_DIR).join("qemu.zip");
            if !ps_download(
                "https://github.com/dirkarnez/qemu-portable/releases/download/20240822/qemu-portable-20240822-windows-amd64.zip",
                &zip, &app, 3, 22
            ) { send(&app, "QEMU download failed!", -1); return; }
            send(&app, "Extracting QEMU...", 23);
            ps_extract(&zip, &PathBuf::from(QEMU_DIR));
            fs::remove_file(&zip).ok();
            if let Some(found) = find_file(&PathBuf::from(QEMU_DIR), "qemu-system-i386.exe") {
                if found != qemu_exe { fs::copy(&found, &qemu_exe).ok(); }
            }
        }
        send(&app, "QEMU ready ✓", 25);

        // 2. Download ADB
        let adb_exe = PathBuf::from(QEMU_DIR).join("adb.exe");
        if !adb_exe.exists() {
            send(&app, "Downloading ADB tools...", 27);
            let zip = PathBuf::from(DATA_DIR).join("adb.zip");
            if !ps_download(
                "https://dl.google.com/android/repository/platform-tools-latest-windows.zip",
                &zip, &app, 27, 42
            ) { send(&app, "ADB download failed!", -1); return; }
            send(&app, "Extracting ADB...", 43);
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
        send(&app, "ADB ready ✓", 47);

        // 3. Download Android image
        let base_img = PathBuf::from(IMAGES_DIR).join("android.img");
        if !base_img.exists() {
            send(&app, "Downloading Android-x86 4.4 image (~300MB)...", 49);
            let iso = PathBuf::from(IMAGES_DIR).join("android.iso");
            if !ps_download(
                "https://sourceforge.net/projects/android-x86/files/Release%204.4-r5/android-x86-4.4-r5.iso/download",
                &iso, &app, 49, 85
            ) { send(&app, "Android download failed!", -1); return; }
            send(&app, "Creating disk image...", 86);
            Command::new(PathBuf::from(QEMU_DIR).join("qemu-img.exe"))
                .args(["create", "-f", "raw", base_img.to_str().unwrap(), "4G"])
                .output().ok();
            fs::remove_file(&iso).ok();
        }
        send(&app, "Android ready ✓", 88);

        // 4. Download VoidEmulator.exe from release.json
        send(&app, "Fetching latest VoidEmulator version...", 89);
        let release_json = ps_fetch(RELEASE_JSON);
        let exe_url = match serde_json::from_str::<serde_json::Value>(&release_json) {
            Ok(j) => j["url"].as_str().unwrap_or("").to_string(),
            Err(_) => { send(&app, "Failed to read release.json!", -1); return; }
        };
        if exe_url.is_empty() { send(&app, "No download URL in release.json!", -1); return; }

        send(&app, "Downloading VoidEmulator.exe...", 90);
        let exe_dest = PathBuf::from(INSTALL_DIR).join("VoidEmulator.exe");
        if !ps_download(&exe_url, &exe_dest, &app, 90, 96) {
            send(&app, "VoidEmulator download failed!", -1);
            return;
        }
        send(&app, "VoidEmulator installed ✓", 97);

        // 5. Create shortcuts
        send(&app, "Creating shortcuts...", 98);
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

        send(&app, "Installation complete! 🚀", 100);
    });
    serde_json::json!({ "success": true })
}

#[tauri::command]
fn launch_app() {
    Command::new(PathBuf::from(INSTALL_DIR).join("VoidEmulator.exe"))
        .spawn().ok();
}

fn ps_download(url: &str, dest: &PathBuf, app: &AppHandle, pct_start: i32, pct_end: i32) -> bool {
    send(app, &format!("Downloading... 0%"), pct_start);
    let result = Command::new("powershell")
        .args(["-Command", &format!(
            "$ProgressPreference='SilentlyContinue'; Invoke-WebRequest -Uri '{}' -OutFile '{}'",
            url, dest.to_str().unwrap()
        )])
        .output();
    match result {
        Ok(o) if o.status.success() => {
            send(app, "Download complete", pct_end);
            true
        }
        _ => false
    }
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
            "$ProgressPreference='SilentlyContinue'; (Invoke-WebRequest -Uri '{}').Content",
            url
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
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![check_installed, run_install, launch_app])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}