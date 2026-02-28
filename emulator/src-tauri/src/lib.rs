use std::process::{Command, Stdio};
use std::path::PathBuf;
use std::fs;

const RELEASE_JSON: &str = "https://raw.githubusercontent.com/darkflareplays8/VoidEm/main/release.json";
const CURRENT_VERSION: &str = "2.0.2";

fn install_dir() -> PathBuf {
    PathBuf::from(std::env::var("APPDATA").unwrap_or_else(|_| "C:\\Users\\Public".into())).join("VoidEmulator")
}
fn qemu_dir() -> PathBuf { install_dir().join("data").join("qemu") }
fn images_dir() -> PathBuf { install_dir().join("data").join("images") }
fn instances_dir() -> PathBuf { install_dir().join("data").join("instances") }
fn qemu_exe() -> PathBuf { qemu_dir().join("qemu-system-i386.exe") }
fn qemu_img() -> PathBuf { qemu_dir().join("qemu-img.exe") }
fn adb_exe() -> PathBuf { qemu_dir().join("adb.exe") }
fn base_img() -> PathBuf { images_dir().join("android.img") }

#[tauri::command]
fn check_setup() -> serde_json::Value {
    let qemu_ok = qemu_exe().exists() 
        || PathBuf::from("C:\\Program Files\\qemu\\qemu-system-i386.exe").exists();
    let adb_ok = adb_exe().exists()
        || PathBuf::from("C:\\Program Files\\qemu\\adb.exe").exists();
    let image_ok = base_img().exists();
    // Return paths for debugging
    serde_json::json!({
        "qemu": qemu_ok,
        "adb": adb_ok,
        "image": image_ok,
        "qemu_path": qemu_exe().to_str().unwrap_or(""),
        "image_path": base_img().to_str().unwrap_or(""),
    })
}

#[tauri::command]
fn open_discord() -> Result<(), String> {
    Command::new("cmd")
        .args(["/c", "start", "", "https://discord.gg/XUe82svaAr"])
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn create_overlay(id: String) -> bool {
    fs::create_dir_all(instances_dir()).ok();
    let overlay = instances_dir().join(format!("{}.qcow2", id));
    if overlay.exists() { return true; }
    Command::new(qemu_img())
        .args(["create", "-f", "qcow2", "-b", base_img().to_str().unwrap(), "-F", "raw", overlay.to_str().unwrap()])
        .output().map(|o| o.status.success()).unwrap_or(false)
}

#[tauri::command]
fn start_qemu(id: String, index: u32) -> bool {
    let overlay = instances_dir().join(format!("{}.qcow2", id));
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![
            check_setup, open_discord,
            create_overlay, start_qemu, stop_instance, run_adb, capture_screen,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}