use std::process::{Command, Stdio};
use std::path::PathBuf;
use std::fs;
use std::sync::Mutex;
use std::collections::HashMap;
use tauri::{AppHandle, Manager, Emitter};

fn data_dir(app: &AppHandle) -> PathBuf { app.path().app_data_dir().unwrap() }
fn qemu_dir(app: &AppHandle) -> PathBuf { data_dir(app).join("qemu") }
fn images_dir(app: &AppHandle) -> PathBuf { data_dir(app).join("images") }
fn instances_dir(app: &AppHandle) -> PathBuf { data_dir(app).join("instances") }
fn qemu_exe(app: &AppHandle) -> PathBuf { qemu_dir(app).join("qemu-system-i386.exe") }
fn qemu_img_exe(app: &AppHandle) -> PathBuf { qemu_dir(app).join("qemu-img.exe") }
fn adb_exe(app: &AppHandle) -> PathBuf { qemu_dir(app).join("adb.exe") }
fn base_img(app: &AppHandle) -> PathBuf { images_dir(app).join("android.img") }

fn send(app: &AppHandle, msg: &str, pct: i32) {
    app.emit("setup-progress", serde_json::json!({ "msg": msg, "pct": pct })).ok();
}

#[tauri::command]
fn check_setup(app: AppHandle) -> serde_json::Value {
    serde_json::json!({
        "qemu": qemu_exe(&app).exists(),
        "adb": adb_exe(&app).exists(),
        "image": base_img(&app).exists(),
    })
}

#[tauri::command]
async fn run_setup(app: AppHandle) -> serde_json::Value {
    let app2 = app.clone();
    std::thread::spawn(move || {
        let app = app2;
        fs::create_dir_all(qemu_dir(&app)).ok();
        fs::create_dir_all(images_dir(&app)).ok();
        fs::create_dir_all(instances_dir(&app)).ok();

        send(&app, "Starting setup...", 1);

        // 1. Download QEMU
        if !qemu_exe(&app).exists() {
            send(&app, "Downloading QEMU...", 5);
            let zip_path = data_dir(&app).join("qemu.zip");
            if download_file(
                "https://github.com/dirkarnez/qemu-portable/releases/download/20240822/qemu-portable-20240822-windows-amd64.zip",
                &zip_path,
                &app, 5, 25
            ).is_err() {
                send(&app, "QEMU download failed!", -1);
                return;
            }
            send(&app, "Extracting QEMU...", 26);
            extract_zip(&zip_path, &qemu_dir(&app));
            fs::remove_file(&zip_path).ok();

            // Find and copy exe if nested
            if let Some(found) = find_file(&qemu_dir(&app), "qemu-system-i386.exe") {
                if found != qemu_exe(&app) {
                    fs::copy(&found, qemu_exe(&app)).ok();
                }
            }
        }
        send(&app, "QEMU ready ✓", 30);

        // 2. Download ADB
        if !adb_exe(&app).exists() {
            send(&app, "Downloading ADB tools...", 32);
            let zip_path = data_dir(&app).join("adb.zip");
            if download_file(
                "https://dl.google.com/android/repository/platform-tools-latest-windows.zip",
                &zip_path,
                &app, 32, 50
            ).is_err() {
                send(&app, "ADB download failed!", -1);
                return;
            }
            send(&app, "Extracting ADB...", 51);
            let pt_dir = data_dir(&app).join("platform-tools-extract");
            extract_zip(&zip_path, &pt_dir);
            let pt = pt_dir.join("platform-tools");
            for f in &["adb.exe", "AdbWinApi.dll", "AdbWinUsbApi.dll"] {
                let src = pt.join(f);
                if src.exists() { fs::copy(&src, qemu_dir(&app).join(f)).ok(); }
            }
            fs::remove_dir_all(&pt_dir).ok();
            fs::remove_file(&zip_path).ok();
        }
        send(&app, "ADB ready ✓", 55);

        // 3. Download Android image
        if !base_img(&app).exists() {
            send(&app, "Downloading Android-x86 image (~300MB)...", 57);
            let iso_path = images_dir(&app).join("android.iso");
            if download_file(
                "https://sourceforge.net/projects/android-x86/files/Release%204.4-r5/android-x86-4.4-r5.iso/download",
                &iso_path,
                &app, 57, 93
            ).is_err() {
                send(&app, "Android image download failed!", -1);
                return;
            }
            send(&app, "Creating disk image...", 94);
            Command::new(qemu_img_exe(&app))
                .args(["create", "-f", "raw", base_img(&app).to_str().unwrap(), "4G"])
                .output().ok();
            fs::remove_file(&iso_path).ok();
        }

        send(&app, "All done! 🚀", 100);
    });
    serde_json::json!({ "success": true })
}

fn download_file(url: &str, dest: &PathBuf, app: &AppHandle, pct_start: i32, pct_end: i32) -> Result<(), String> {
    let output = Command::new("powershell")
        .args([
            "-Command",
            &format!(
                "$ProgressPreference='SilentlyContinue'; Invoke-WebRequest -Uri '{}' -OutFile '{}'",
                url,
                dest.to_str().unwrap()
            )
        ])
        .output()
        .map_err(|e| e.to_string())?;

    if output.status.success() {
        app.emit("setup-progress", serde_json::json!({ "msg": "Download complete", "pct": pct_end })).ok();
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).to_string())
    }
}

fn extract_zip(zip: &PathBuf, dest: &PathBuf) {
    fs::create_dir_all(dest).ok();
    Command::new("powershell")
        .args([
            "-Command",
            &format!(
                "$ProgressPreference='SilentlyContinue'; Expand-Archive -Path '{}' -DestinationPath '{}' -Force",
                zip.to_str().unwrap(),
                dest.to_str().unwrap()
            )
        ])
        .output().ok();
}

fn find_file(dir: &PathBuf, name: &str) -> Option<PathBuf> {
    if !dir.exists() { return None; }
    for entry in fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(f) = find_file(&path, name) { return Some(f); }
        } else if path.file_name()?.to_str()? == name {
            return Some(path);
        }
    }
    None
}

#[tauri::command]
fn create_overlay(app: AppHandle, id: String) -> bool {
    let inst_dir = instances_dir(&app);
    fs::create_dir_all(&inst_dir).ok();
    let overlay = inst_dir.join(format!("{}.qcow2", id));
    if overlay.exists() { return true; }
    Command::new(qemu_img_exe(&app))
        .args(["create", "-f", "qcow2", "-b", base_img(&app).to_str().unwrap(), "-F", "raw", overlay.to_str().unwrap()])
        .output().map(|o| o.status.success()).unwrap_or(false)
}

#[tauri::command]
fn start_qemu(app: AppHandle, id: String, index: u32) -> bool {
    let overlay = instances_dir(&app).join(format!("{}.qcow2", id));
    let adb_port = 5554 + index * 2;
    Command::new(qemu_exe(&app))
        .args([
            "-m", "512", "-smp", "1",
            "-drive", &format!("file={},format=qcow2", overlay.to_str().unwrap()),
            "-net", "nic",
            "-net", &format!("user,hostfwd=tcp:127.0.0.1:{}-:5555", adb_port),
            "-vga", "std", "-usb", "-device", "usb-tablet",
            "-no-reboot", "-nographic",
        ])
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
        .spawn().is_ok()
}

#[tauri::command]
fn stop_instance(_app: AppHandle, _id: String) -> bool { true }

#[tauri::command]
fn run_adb(app: AppHandle, args: Vec<String>) -> String {
    match Command::new(adb_exe(&app)).args(&args).output() {
        Ok(out) => String::from_utf8_lossy(&out.stdout).to_string(),
        Err(e) => format!("Error: {}", e),
    }
}

#[tauri::command]
fn capture_screen(app: AppHandle, id: String, adb_port: u32) -> String {
    let tmp = std::env::temp_dir().join(format!("void_{}.png", id));
    let device = format!("127.0.0.1:{}", adb_port);
    let adb = adb_exe(&app);
    Command::new(&adb).args(["-s", &device, "shell", "screencap", "-p", "/sdcard/sc.png"]).output().ok();
    Command::new(&adb).args(["-s", &device, "pull", "/sdcard/sc.png", tmp.to_str().unwrap()]).output().ok();
    if tmp.exists() {
        let data = fs::read(&tmp).unwrap_or_default();
        base64_encode(&data)
    } else { String::new() }
}

fn base64_encode(data: &[u8]) -> String {
    let chars = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    data.chunks(3).map(|c| {
        let n = match c.len() {
            3 => (c[0] as u32) << 16 | (c[1] as u32) << 8 | c[2] as u32,
            2 => (c[0] as u32) << 16 | (c[1] as u32) << 8,
            _ => (c[0] as u32) << 16,
        };
        let mut s = String::new();
        s.push(chars[(n >> 18 & 63) as usize] as char);
        s.push(chars[(n >> 12 & 63) as usize] as char);
        s.push(if c.len() > 1 { chars[(n >> 6 & 63) as usize] as char } else { '=' });
        s.push(if c.len() > 2 { chars[(n & 63) as usize] as char } else { '=' });
        s
    }).collect()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_http::init())
        .invoke_handler(tauri::generate_handler![
            check_setup, run_setup, create_overlay,
            start_qemu, stop_instance, run_adb, capture_screen,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}