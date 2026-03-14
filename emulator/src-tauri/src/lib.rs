use std::collections::HashMap;
use std::process::{Command, Stdio};
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::fs;
use std::sync::Mutex;
use tauri::State;

const CURRENT_VERSION: &str = "2.0.2";
const CREATE_NO_WINDOW: u32 = 0x08000000;

fn install_dir() -> PathBuf {
    PathBuf::from(std::env::var("APPDATA").unwrap_or_else(|_| "C:\\Users\\Public".into())).join("VoidEmulator")
}
fn qemu_dir() -> PathBuf { install_dir().join("data").join("qemu") }
fn images_dir() -> PathBuf { install_dir().join("data").join("images") }
fn instances_dir() -> PathBuf { install_dir().join("data").join("instances") }
fn qemu_exe() -> PathBuf { qemu_dir().join("qemu-system-i386.exe") }
fn qemu_img_exe() -> PathBuf { qemu_dir().join("qemu-img.exe") }
fn adb_exe() -> PathBuf { qemu_dir().join("adb.exe") }
fn base_img() -> PathBuf { images_dir().join("android.img") }
fn instance_dir(name: &str) -> PathBuf { instances_dir().join(name) }
fn overlay_path(name: &str) -> PathBuf { instance_dir(name).join("overlay.qcow2") }
fn instances_json() -> PathBuf { install_dir().join("instances.json") }
fn downloads_dir() -> PathBuf {
    PathBuf::from(std::env::var("USERPROFILE").unwrap_or_else(|_| "C:\\Users\\Public".into())).join("Downloads")
}
fn adb_port(index: u32) -> u16 { (5554 + index * 2) as u16 }

struct AppState {
    qemu_pids: Mutex<HashMap<String, u32>>,
}

#[tauri::command]
fn check_setup() -> serde_json::Value {
    let qemu_ok = qemu_exe().exists();
    let adb_ok = adb_exe().exists();
    let image_ok = base_img().exists();
    if qemu_ok && adb_ok && image_ok {
        fs::remove_dir_all(install_dir().join("downloads")).ok();
    }
    serde_json::json!({
        "qemu": qemu_ok, "adb": adb_ok, "image": image_ok,
        "qemu_path": qemu_exe().to_str().unwrap_or(""),
        "image_path": base_img().to_str().unwrap_or(""),
    })
}

#[tauri::command]
fn load_instances() -> serde_json::Value {
    if let Ok(data) = fs::read_to_string(instances_json()) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&data) { return v; }
    }
    serde_json::json!([])
}

#[tauri::command]
fn save_instances(instances: serde_json::Value) -> bool {
    fs::create_dir_all(install_dir()).ok();
    fs::write(instances_json(), serde_json::to_string_pretty(&instances).unwrap_or_default()).is_ok()
}

#[tauri::command]
fn open_discord() -> Result<(), String> {
    Command::new("cmd").args(["/c", "start", "", "https://discord.gg/XUe82svaAr"])
        .creation_flags(CREATE_NO_WINDOW).spawn().map(|_| ()).map_err(|e| e.to_string())
}

#[tauri::command]
fn create_overlay(name: String) -> bool {
    let dir = instance_dir(&name);
    let overlay = overlay_path(&name);
    fs::create_dir_all(&dir).ok();
    if overlay.exists() { return true; }
    Command::new(qemu_img_exe())
        .args(["create", "-f", "qcow2", "-b", base_img().to_str().unwrap(), "-F", "raw", overlay.to_str().unwrap()])
        .creation_flags(CREATE_NO_WINDOW)
        .output().map(|o| o.status.success()).unwrap_or(false)
}

#[tauri::command]
fn delete_instance(name: String, state: State<AppState>) -> bool {
    if let Ok(mut map) = state.qemu_pids.lock() {
        if let Some(pid) = map.remove(&name) { kill_pid(pid); }
    }
    fs::remove_dir_all(instance_dir(&name)).ok();
    true
}

#[tauri::command]
fn start_qemu(name: String, index: u32, state: State<AppState>) -> bool {
    let overlay = overlay_path(&name);
    if !overlay.exists() { return false; }
    if let Ok(mut map) = state.qemu_pids.lock() {
        if let Some(pid) = map.remove(&name) { kill_pid(pid); }
    }
    let adb = adb_port(index);
    match Command::new(qemu_exe())
        .args([
            "-m", "1024", "-smp", "2",
            "-drive", &format!("file={},format=qcow2", overlay.to_str().unwrap()),
            "-net", "nic",
            "-net", &format!("user,hostfwd=tcp:127.0.0.1:{}-:5555", adb),
            "-vga", "std", "-usb", "-device", "usb-tablet",
            "-no-reboot",
            "-name", &format!("{} view", name),
        ])
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => {
            let pid = child.id();
            if let Ok(mut map) = state.qemu_pids.lock() { map.insert(name, pid); }
            true
        }
        Err(_) => false
    }
}

#[tauri::command]
fn stop_instance(name: String, state: State<AppState>) -> bool {
    if let Ok(mut map) = state.qemu_pids.lock() {
        if let Some(pid) = map.remove(&name) { kill_pid(pid); return true; }
    }
    false
}

fn kill_pid(pid: u32) {
    Command::new("taskkill").args(["/F", "/PID", &pid.to_string()])
        .creation_flags(CREATE_NO_WINDOW).output().ok();
}

#[tauri::command]
fn run_adb(args: Vec<String>) -> String {
    match Command::new(adb_exe()).args(&args).creation_flags(CREATE_NO_WINDOW).output() {
        Ok(out) => String::from_utf8_lossy(&out.stdout).to_string(),
        Err(e) => format!("Error: {}", e),
    }
}

#[tauri::command]
fn adb_pull_to_downloads(adb_port: u32, remote: String, filename: String) -> bool {
    let dest = downloads_dir().join(&filename);
    fs::create_dir_all(downloads_dir()).ok();
    let device = format!("127.0.0.1:{}", adb_port);
    Command::new(adb_exe()).args(["-s", &device, "pull", &remote, dest.to_str().unwrap()])
        .creation_flags(CREATE_NO_WINDOW).output().map(|o| o.status.success()).unwrap_or(false)
}

#[tauri::command]
fn adb_push_bytes(adb_port: u32, bytes: Vec<u8>, dest: String) -> bool {
    let tmp = std::env::temp_dir().join("void_upload_tmp");
    if fs::write(&tmp, &bytes).is_err() { return false; }
    let device = format!("127.0.0.1:{}", adb_port);
    let ok = Command::new(adb_exe()).args(["-s", &device, "push", tmp.to_str().unwrap(), &dest])
        .creation_flags(CREATE_NO_WINDOW).output().map(|o| o.status.success()).unwrap_or(false);
    fs::remove_file(&tmp).ok();
    ok
}

fn base64_encode(data: &[u8]) -> String {
    let chars = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let n = match chunk.len() {
            3 => (chunk[0] as u32) << 16 | (chunk[1] as u32) << 8 | chunk[2] as u32,
            2 => (chunk[0] as u32) << 16 | (chunk[1] as u32) << 8,
            _ => (chunk[0] as u32) << 16,
        };
        out.push(chars[(n >> 18 & 63) as usize] as char);
        out.push(chars[(n >> 12 & 63) as usize] as char);
        out.push(if chunk.len() > 1 { chars[(n >> 6 & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { chars[(n & 63) as usize] as char } else { '=' });
    }
    out
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(AppState { qemu_pids: Mutex::new(HashMap::new()) })
        .invoke_handler(tauri::generate_handler![
            check_setup, open_discord, load_instances, save_instances,
            create_overlay, delete_instance, start_qemu, stop_instance,
            run_adb, adb_pull_to_downloads, adb_push_bytes,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}