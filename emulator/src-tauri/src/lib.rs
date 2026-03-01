use std::collections::HashMap;
use std::process::{Command, Stdio};
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::fs;
use std::sync::Mutex;
use tauri::State;

const CURRENT_VERSION: &str = "2.6.5";
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

// vnc_port = 5900 + index, ws_port = 6900 + index, adb_port = 5554 + index*2
fn vnc_port(index: u32) -> u16 { (5900 + index) as u16 }
fn ws_port(index: u32) -> u16 { (6900 + index) as u16 }
fn adb_port(index: u32) -> u16 { (5554 + index * 2) as u16 }

struct AppState {
    qemu_pids: Mutex<HashMap<String, u32>>,
    ws_handles: Mutex<HashMap<String, tokio::task::JoinHandle<()>>>,
}

// ─── Setup ────────────────────────────────────────────
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

// ─── Instances ────────────────────────────────────────
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
    if let Ok(mut map) = state.ws_handles.lock() {
        if let Some(h) = map.remove(&name) { h.abort(); }
    }
    fs::remove_dir_all(instance_dir(&name)).ok();
    true
}

// ─── Start QEMU + VNC + WS bridge ────────────────────
#[tauri::command]
async fn start_qemu(name: String, index: u32, state: State<'_, AppState>) -> Result<bool, String> {
    let overlay = overlay_path(&name);
    if !overlay.exists() { return Ok(false); }

    // Kill existing
    {
        let mut pids = state.qemu_pids.lock().unwrap();
        if let Some(pid) = pids.remove(&name) { kill_pid(pid); }
    }
    {
        let mut handles = state.ws_handles.lock().unwrap();
        if let Some(h) = handles.remove(&name) { h.abort(); }
    }

    let vnc = vnc_port(index);
    let ws = ws_port(index);
    let adb = adb_port(index);

    // Start QEMU with VNC
    let child = Command::new(qemu_exe())
        .args([
            "-m", "1024", "-smp", "2",
            "-drive", &format!("file={},format=qcow2", overlay.to_str().unwrap()),
            "-net", "nic",
            "-net", &format!("user,hostfwd=tcp:127.0.0.1:{}-:5555", adb),
            "-vga", "std", "-usb", "-device", "usb-tablet",
            "-no-reboot",
            "-vnc", &format!("127.0.0.1:{}", index), // :0 = port 5900, :1 = 5901 etc
        ])
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| e.to_string())?;

    let pid = child.id();
    state.qemu_pids.lock().unwrap().insert(name.clone(), pid);

    // Start TCP→WebSocket bridge for VNC
    let handle = tokio::spawn(async move {
        run_ws_bridge(vnc, ws).await;
    });
    state.ws_handles.lock().unwrap().insert(name, handle);

    Ok(true)
}

// TCP→WebSocket bridge: listens on ws_port, proxies to vnc_port
async fn run_ws_bridge(vnc_port: u16, ws_port: u16) {
    use tokio::net::{TcpListener, TcpStream};
    use tokio::io::copy_bidirectional;

    let listener = match TcpListener::bind(format!("127.0.0.1:{}", ws_port)).await {
        Ok(l) => l,
        Err(_) => return,
    };

    loop {
        let (ws_stream, _) = match listener.accept().await {
            Ok(s) => s,
            Err(_) => break,
        };

        // Connect to QEMU VNC
        let vnc_stream = match TcpStream::connect(format!("127.0.0.1:{}", vnc_port)).await {
            Ok(s) => s,
            Err(_) => continue,
        };

        tokio::spawn(async move {
            // Do WebSocket handshake, then proxy raw bytes
            ws_proxy(ws_stream, vnc_stream).await;
        });
    }
}

async fn ws_proxy(mut ws_conn: tokio::net::TcpStream, mut vnc_conn: tokio::net::TcpStream) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    // Read the HTTP upgrade request from the browser
    let mut buf = vec![0u8; 4096];
    let n = match ws_conn.read(&mut buf).await {
        Ok(n) if n > 0 => n,
        _ => return,
    };
    let req = String::from_utf8_lossy(&buf[..n]);

    // Extract Sec-WebSocket-Key
    let key = req.lines()
        .find(|l| l.to_lowercase().starts_with("sec-websocket-key:"))
        .and_then(|l| l.splitn(2, ':').nth(1))
        .map(|s| s.trim().to_string());

    let key = match key {
        Some(k) => k,
        None => return, // not a WS handshake
    };

    // Compute accept key
    let accept = ws_accept_key(&key);

    // Send 101 upgrade response
    let response = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\nSec-WebSocket-Protocol: binary\r\n\r\n",
        accept
    );
    if ws_conn.write_all(response.as_bytes()).await.is_err() { return; }

    // Now proxy WebSocket frames ↔ raw VNC TCP
    // We need to unwrap WS frames going to VNC, and wrap VNC data into WS frames going to browser
    ws_vnc_bridge(ws_conn, vnc_conn).await;
}

async fn ws_vnc_bridge(ws: tokio::net::TcpStream, vnc: tokio::net::TcpStream) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let (mut ws_r, mut ws_w) = tokio::io::split(ws);
    let (mut vnc_r, mut vnc_w) = tokio::io::split(vnc);

    // WS → VNC: unwrap WebSocket frames, send raw to VNC
    let ws_to_vnc = tokio::spawn(async move {
        let mut header = [0u8; 14];
        loop {
            // Read WS frame header (at least 2 bytes)
            if ws_r.read_exact(&mut header[..2]).await.is_err() { break; }
            let fin = header[0] & 0x80 != 0;
            let opcode = header[0] & 0x0F;
            if opcode == 8 { break; } // close frame
            let masked = header[1] & 0x80 != 0;
            let mut payload_len = (header[1] & 0x7F) as u64;
            if payload_len == 126 {
                if ws_r.read_exact(&mut header[..2]).await.is_err() { break; }
                payload_len = u16::from_be_bytes([header[0], header[1]]) as u64;
            } else if payload_len == 127 {
                if ws_r.read_exact(&mut header[..8]).await.is_err() { break; }
                payload_len = u64::from_be_bytes(header[..8].try_into().unwrap());
            }
            let mut mask = [0u8; 4];
            if masked {
                if ws_r.read_exact(&mut mask).await.is_err() { break; }
            }
            let mut payload = vec![0u8; payload_len as usize];
            if ws_r.read_exact(&mut payload).await.is_err() { break; }
            if masked {
                for (i, b) in payload.iter_mut().enumerate() { *b ^= mask[i % 4]; }
            }
            if vnc_w.write_all(&payload).await.is_err() { break; }
        }
    });

    // VNC → WS: wrap raw VNC data into binary WebSocket frames
    let vnc_to_ws = tokio::spawn(async move {
        let mut buf = vec![0u8; 65536];
        loop {
            let n = match vnc_r.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            // Build WS binary frame (no masking for server→client)
            let mut frame = Vec::with_capacity(n + 10);
            frame.push(0x82u8); // FIN + binary opcode
            if n <= 125 {
                frame.push(n as u8);
            } else if n <= 65535 {
                frame.push(126);
                frame.extend_from_slice(&(n as u16).to_be_bytes());
            } else {
                frame.push(127);
                frame.extend_from_slice(&(n as u64).to_be_bytes());
            }
            frame.extend_from_slice(&buf[..n]);
            if ws_w.write_all(&frame).await.is_err() { break; }
        }
    });

    let _ = tokio::join!(ws_to_vnc, vnc_to_ws);
}

fn ws_accept_key(key: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    // Proper SHA1 + base64 for WebSocket handshake
    let magic = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
    let combined = format!("{}{}", key, magic);
    let hash = sha1(&combined);
    base64_encode(&hash)
}

fn sha1(input: &str) -> [u8; 20] {
    // SHA1 implementation
    let bytes = input.as_bytes();
    let mut h: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];
    let bit_len = (bytes.len() as u64) * 8;
    let mut msg = bytes.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 { msg.push(0); }
    msg.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 80];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([chunk[i*4], chunk[i*4+1], chunk[i*4+2], chunk[i*4+3]]);
        }
        for i in 16..80 {
            w[i] = (w[i-3] ^ w[i-8] ^ w[i-14] ^ w[i-16]).rotate_left(1);
        }
        let (mut a, mut b, mut c, mut d, mut e) = (h[0], h[1], h[2], h[3], h[4]);
        for i in 0..80 {
            let (f, k) = match i {
                0..=19  => ((b & c) | ((!b) & d), 0x5A827999u32),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _       => (b ^ c ^ d, 0xCA62C1D6),
            };
            let temp = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(w[i]);
            e = d; d = c; c = b.rotate_left(30); b = a; a = temp;
        }
        h[0] = h[0].wrapping_add(a); h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c); h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
    }
    let mut out = [0u8; 20];
    for i in 0..5 { out[i*4..(i+1)*4].copy_from_slice(&h[i].to_be_bytes()); }
    out
}

// ─── Stop ─────────────────────────────────────────────
#[tauri::command]
fn stop_instance(name: String, state: State<AppState>) -> bool {
    if let Ok(mut map) = state.qemu_pids.lock() {
        if let Some(pid) = map.remove(&name) { kill_pid(pid); }
    }
    if let Ok(mut map) = state.ws_handles.lock() {
        if let Some(h) = map.remove(&name) { h.abort(); }
    }
    true
}

fn kill_pid(pid: u32) {
    Command::new("taskkill").args(["/F", "/PID", &pid.to_string()])
        .creation_flags(CREATE_NO_WINDOW).output().ok();
}

// ─── ADB ──────────────────────────────────────────────
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

// ─── Tauri entry ──────────────────────────────────────
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(AppState {
            qemu_pids: Mutex::new(HashMap::new()),
            ws_handles: Mutex::new(HashMap::new()),
        })
        .invoke_handler(tauri::generate_handler![
            check_setup, open_discord, load_instances, save_instances,
            create_overlay, delete_instance, start_qemu, stop_instance,
            run_adb, adb_pull_to_downloads, adb_push_bytes,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}