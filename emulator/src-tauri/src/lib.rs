use std::process::{Command, Stdio};
use std::path::PathBuf;
use std::fs;
use tauri::{AppHandle, Emitter};

const INSTALL_DIR: &str = "C:\\Program Files\\VoidEmulator";
const DATA_DIR: &str = "C:\\Program Files\\VoidEmulator\\data";
const QEMU_DIR: &str = "C:\\Program Files\\VoidEmulator\\data\\qemu";
const IMAGES_DIR: &str = "C:\\Program Files\\VoidEmulator\\data\\images";
const INSTANCES_DIR: &str = "C:\\Program Files\\VoidEmulator\\data\\instances";
const RELEASE_JSON: &str = "https://raw.githubusercontent.com/darkflareplays8/VoidEm/main/release.json";
const CURRENT_VERSION: &str = "2.0.2";

fn qemu_exe() -> PathBuf { PathBuf::from(QEMU_DIR).join("qemu-system-i386.exe") }
fn qemu_img() -> PathBuf { PathBuf::from(QEMU_DIR).join("qemu-img.exe") }
fn adb_exe() -> PathBuf { PathBuf::from(QEMU_DIR).join("adb.exe") }
fn base_img() -> PathBuf { PathBuf::from(IMAGES_DIR).join("android.img") }

#[tauri::command]
fn check_update() -> serde_json::Value {
    let json = ps_fetch(RELEASE_JSON);
    match serde_json::from_str::<serde_json::Value>(&json) {
        Ok(j) => {
            let remote = j["version"].as_str().unwrap_or("0.0.0");
            serde_json::json!({
                "has_update": remote != CURRENT_VERSION,
                "version": remote,
                "url": j["url"].as_str().unwrap_or("")
            })
        }
        Err(_) => serde_json::json!({ "has_update": false })
    }
}

#[tauri::command]
async fn do_update(app: AppHandle, url: String) -> bool {
    let app2 = app.clone();
    std::thread::spawn(move || {
        let app = app2;
        app.emit("update-progress", serde_json::json!({ "msg": "Downloading update...", "pct": 10 })).ok();
        
        // Download new exe to temp
        let tmp = PathBuf::from(std::env::temp_dir()).join("VoidEmulator_new.exe");
        let result = Command::new("powershell")
            .args(["-Command", &format!(
                "$ProgressPreference='SilentlyContinue'; Invoke-WebRequest -Uri '{}' -OutFile '{}'",
                url, tmp.to_str().unwrap()
            )])
            .output();
        
        if result.map(|o| o.status.success()).unwrap_or(false) {
            app.emit("update-progress", serde_json::json!({ "msg": "Installing update...", "pct": 90 })).ok();
            
            // Use powershell to replace exe and relaunch (after a short delay so this process can exit)
            let current = PathBuf::from(INSTALL_DIR).join("VoidEmulator.exe");
            Command::new("powershell")
                .args(["-Command", &format!(
                    "Start-Sleep -Seconds 2; Copy-Item '{}' '{}' -Force; Start-Process '{}'",
                    tmp.to_str().unwrap(),
                    current.to_str().unwrap(),
                    current.to_str().unwrap()
                )])
                .spawn().ok();
            
            app.emit("update-progress", serde_json::json!({ "msg": "Restarting...", "pct": 100 })).ok();
            std::thread::sleep(std::time::Duration::from_secs(1));
            std::process::exit(0);
        } else {
            app.emit("update-progress", serde_json::json!({ "msg": "Update failed!", "pct": -1 })).ok();
        }
    });
    true
}

#[tauri::command]
fn check_setup() -> serde_json::Value {
    serde_json::json!({
        "qemu": qemu_exe().exists(),
        "adb": adb_exe().exists(),
        "image": base_img().exists(),
    })
}

#[tauri::command]
fn create_overlay(id: String) -> bool {
    fs::create_dir_all(INSTANCES_DIR).ok();
    let overlay = PathBuf::from(INSTANCES_DIR).join(format!("{}.qcow2", id));
    if overlay.exists() { return true; }
    Command::new(qemu_img())
        .args(["create", "-f", "qcow2", "-b", base_img().to_str().unwrap(), "-F", "raw", overlay.to_str().unwrap()])
        .output().map(|o| o.status.success()).unwrap_or(false)
}

#[tauri::command]
fn start_qemu(id: String, index: u32) -> bool {
    let overlay = PathBuf::from(INSTANCES_DIR).join(format!("{}.qcow2", id));
    let adb_port = 5554 + index * 2;
    Command::new(qemu_exe())
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
fn stop_instance(_id: String) -> bool { true }

#[tauri::command]
fn run_adb(args: Vec<String>) -> String {
    match Command::new(adb_exe()).args(&args).output() {
        Ok(out) => String::from_utf8_lossy(&out.stdout).to_string(),
        Err(e) => format!("Error: {}", e),
    }
}

#[tauri::command]
fn capture_screen(id: String, adb_port: u32) -> String {
    let tmp = std::env::temp_dir().join(format!("void_{}.png", id));
    let device = format!("127.0.0.1:{}", adb_port);
    Command::new(adb_exe()).args(["-s", &device, "shell", "screencap", "-p", "/sdcard/sc.png"]).output().ok();
    Command::new(adb_exe()).args(["-s", &device, "pull", "/sdcard/sc.png", tmp.to_str().unwrap()]).output().ok();
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

fn ps_fetch(url: &str) -> String {
    Command::new("powershell")
        .args(["-Command", &format!(
            "$ProgressPreference='SilentlyContinue'; (Invoke-WebRequest -Uri '{}').Content", url
        )])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![
            check_update, do_update, check_setup,
            create_overlay, start_qemu, stop_instance, run_adb, capture_screen,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}