use std::process::Command;
use std::path::PathBuf;
use std::fs;
use std::sync::Arc;
use std::io::{Write, Read};
use tauri::State;
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

const RELEASE_JSON: &str = "https://raw.githubusercontent.com/darkflareplays8/VoidEm/main/release.json";
const QEMU_URL: &str = "https://github.com/darkflareplays8/VoidEm/releases/download/Asset%2FQuemu/qemu.zip";

fn install_dir() -> PathBuf {
    PathBuf::from(std::env::var("APPDATA").unwrap_or_else(|_| "C:\\Users\\Public".into())).join("VoidEmulator")
}
fn qemu_dir() -> PathBuf { install_dir().join("data").join("qemu") }
fn images_dir() -> PathBuf { install_dir().join("data").join("images") }

#[derive(Default)]
pub struct InstallState {
    pub log: std::sync::Mutex<Vec<(String, i32)>>,
    pub done: std::sync::Mutex<bool>,
}

#[tauri::command]
fn check_installed() -> bool {
    install_dir().join("VoidEmulator.exe").exists()
        && qemu_dir().join("qemu-system-i386.exe").exists()
        && images_dir().join("android.img").exists()
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

        let install = install_dir();
        let qemu = qemu_dir();
        let images = images_dir();
        let instances = install.join("data").join("instances");

        for dir in &[&install, &images, &instances] {
            if let Err(e) = fs::create_dir_all(dir) {
                push(&format!("Failed to create dir {:?}: {}", dir, e), -1); return;
            }
        }
        push("Starting installation...", 1);

        // 1. QEMU - download portable zip, extract to AppData (no admin needed)
        let qemu_exe = qemu.join("qemu-system-i386.exe");
        if !qemu_exe.exists() {
            let downloads_dir = install_dir().join("downloads");
            fs::create_dir_all(&downloads_dir).ok();
            let zip = downloads_dir.join("qemu.zip");
            if let Err(e) = http_download_resume(QEMU_URL, &zip, "QEMU", 3, 20, &state) {
                push(&format!("QEMU download failed: {}", e), -1); return;
            }
            push("Extracting QEMU...", 21);
            let qemu_parent = qemu.parent().unwrap_or(&qemu);
            fs::create_dir_all(&qemu_parent).ok();
            let tar_result = Command::new("tar")
                .args(["-xf", zip.to_str().unwrap(), "-C", qemu_parent.to_str().unwrap()])
                .creation_flags(0x08000000)
                .output();
            match &tar_result {
                Ok(o) => {
                    let stdout = String::from_utf8_lossy(&o.stdout);
                    let stderr = String::from_utf8_lossy(&o.stderr);
                    push(&format!("tar exit:{} stdout:{} stderr:{}", o.status.code().unwrap_or(-1), stdout.trim(), stderr.trim()), 22);
                }
                Err(e) => { push(&format!("tar failed to run: {}", e), -1); return; }
            }
            // List what's actually in qemu dir and parent after extraction
            let parent_contents = fs::read_dir(&qemu_parent).map(|e| {
                e.flatten().map(|f| f.file_name().to_string_lossy().to_string()).collect::<Vec<_>>().join(", ")
            }).unwrap_or_else(|_| "unreadable".into());
            let qemu_contents = fs::read_dir(&qemu).map(|e| {
                e.flatten().map(|f| f.file_name().to_string_lossy().to_string()).collect::<Vec<_>>().join(", ")
            }).unwrap_or_else(|_| "missing".into());
            push(&format!("parent:[{}] qemu:[{}]", parent_contents, qemu_contents), 23);
            if !qemu_exe.exists() {
                push("QEMU extraction failed - qemu-system-i386.exe not found", -1); return;
            }
        }
        push("QEMU ready ✓", 25);

        // 2. ADB
        let adb_exe = qemu.join("adb.exe");
        if !adb_exe.exists() {
            let downloads_dir = install_dir().join("downloads");
            fs::create_dir_all(&downloads_dir).ok();
            let zip = downloads_dir.join("adb.zip");
            if let Err(e) = http_download_resume("https://dl.google.com/android/repository/platform-tools-latest-windows.zip", &zip, "ADB", 27, 44, &state) {
                push(&format!("ADB download failed: {}", e), -1); return;
            }
            push("Extracting ADB...", 45);
            let tmp = std::env::temp_dir().join("void_pt_tmp");
            fs::create_dir_all(&tmp).ok();
            Command::new("tar")
                .args(["-xf", zip.to_str().unwrap(), "-C", tmp.to_str().unwrap()])
                .creation_flags(0x08000000)
                .output().ok();
            let pt = tmp.join("platform-tools");
            for f in &["adb.exe", "AdbWinApi.dll", "AdbWinUsbApi.dll"] {
                let src = pt.join(f);
                if src.exists() { fs::copy(&src, qemu.join(f)).ok(); }
            }
            fs::remove_dir_all(&tmp).ok();
        }
        push("ADB ready ✓", 47);

        // 3. Android image
        let base_img = images.join("android.img");
        if !base_img.exists() {
            let downloads_dir = install_dir().join("downloads");
            fs::create_dir_all(&downloads_dir).ok();
            let iso = downloads_dir.join("android.iso");
            if let Err(e) = http_download_resume("https://www.fosshub.com/Android-x86.html/android-x86-4.4-r5.iso", &iso, "Android", 49, 86, &state) {
                push(&format!("Android download failed: {}", e), -1); return;
            }
            push("Creating disk image...", 87);
            let qemu_img = qemu.join("qemu-img.exe");
            if qemu_img.exists() {
                Command::new(&qemu_img)
                    .args(["create", "-f", "raw", base_img.to_str().unwrap(), "4G"])
                    .creation_flags(0x08000000)
                    .output().ok();
            } else {
                Command::new("fsutil")
                    .args(["file", "createnew", base_img.to_str().unwrap(), "4294967296"])
                    .creation_flags(0x08000000)
                    .output().ok();
            }
        }
        push("Android ready ✓", 88);

        // 4. VoidEmulator.exe
        push("Fetching latest version...", 89);
        let json = match ureq::get(RELEASE_JSON).call() {
            Ok(r) => r.into_string().unwrap_or_default(),
            Err(e) => { push(&format!("release.json failed: {}", e), -1); return; }
        };
        let parsed = serde_json::from_str::<serde_json::Value>(&json).unwrap_or_default();
        let version = parsed["version"].as_str().unwrap_or("").to_string();
        let url = parsed["url"].as_str().unwrap_or("").to_string();
        if url.is_empty() { push("No URL in release.json!", -1); return; }

        // Try current URL, if 404 fall back to previous version
        let final_url = if url_exists(&url) {
            push(&format!("Downloading VoidEmulator v{}...", version), 90);
            url.clone()
        } else {
            push(&format!("v{} not ready yet, trying previous version...", version), 90);
            match prev_version_url(&url, &version) {
                Some(prev) => {
                    if url_exists(&prev) {
                        push("Using previous version instead", 90);
                        prev
                    } else {
                        push("Could not find a working VoidEmulator download!", -1); return;
                    }
                }
                None => { push("Could not determine fallback URL!", -1); return; }
            }
        };

        let downloads_dir = install_dir().join("downloads");
        fs::create_dir_all(&downloads_dir).ok();
        let exe_tmp = downloads_dir.join("VoidEmulator.exe");
        let exe_dest = install.join("VoidEmulator.exe");
        if let Err(e) = http_download_progress(&final_url, &exe_tmp, "VoidEmulator", 90, 96, &state) {
            push(&format!("VoidEmulator download failed: {}", e), -1); return;
        }
        if let Err(e) = fs::copy(&exe_tmp, &exe_dest) {
            push(&format!("Copy failed: {}", e), -1); return;
        }
        push("VoidEmulator installed ✓", 97);

        push("Creating shortcuts...", 98);
        let exe_dest = install.join("VoidEmulator.exe");
        // Start menu
        create_shortcut(
            &exe_dest,
            &PathBuf::from(std::env::var("APPDATA").unwrap_or_default())
                .join("Microsoft\\Windows\\Start Menu\\Programs\\VoidEmulator.lnk")
        );
        // Desktop shortcut - use PowerShell to get real desktop path (handles OneDrive redirect)
        Command::new("powershell")
            .args(["-WindowStyle", "Hidden", "-Command", &format!(
                "$ws = New-Object -ComObject WScript.Shell; $d = [Environment]::GetFolderPath('Desktop'); $s = $ws.CreateShortcut($d + '\\VoidEmulator.lnk'); $s.TargetPath = '{}'; $s.Save()",
                exe_dest.to_str().unwrap()
            )])
            .creation_flags(0x08000000)
            .output().ok();

        push("Installation complete!", 100);
        *state.done.lock().unwrap() = true;
    });
    true
}

#[tauri::command]
fn launch_app() -> Result<(), String> {
    Command::new(install_dir().join("VoidEmulator.exe"))
        .creation_flags(0x08000000)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn open_discord() -> Result<(), String> {
    Command::new("cmd")
        .args(["/c", "start", "", "https://discord.gg/XUe82svaAr"])
        .creation_flags(0x08000000)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}


fn url_exists(url: &str) -> bool {
    match ureq::head(url).call() {
        Ok(r) => r.status() < 400,
        Err(_) => false,
    }
}

fn prev_version_url(url: &str, version: &str) -> Option<String> {
    // Parse semver e.g. "2.3.1" -> (2, 3, 1)
    let parts: Vec<u64> = version.split('.').filter_map(|p| p.parse().ok()).collect();
    if parts.len() != 3 { return None; }
    let (major, minor, patch) = (parts[0], parts[1], parts[2]);
    let prev_ver = if patch > 0 {
        format!("{}.{}.{}", major, minor, patch - 1)
    } else if minor > 0 {
        format!("{}.{}.9", major, minor - 1)
    } else {
        return None;
    };
    // Replace version string in URL e.g. v2.3.1 -> v2.3.0
    let prev_url = url
        .replace(&format!("v{}", version), &format!("v{}", prev_ver))
        .replace(&format!("/{}/", version), &format!("/{}/", prev_ver));
    Some(prev_url)
}

fn http_download_resume(url: &str, dest: &PathBuf, label: &str, pct_start: i32, pct_end: i32, state: &Arc<InstallState>) -> Result<(), String> {
    use std::io::{Write, Read, Seek, SeekFrom};
    // Check how much we already have
    let existing = dest.metadata().map(|m| m.len()).unwrap_or(0);
    // HEAD request to get total size
    let total = ureq::head(url).call()
        .ok()
        .and_then(|r| r.header("content-length").and_then(|v| v.parse::<u64>().ok()))
        .unwrap_or(0);
    // If already fully downloaded, skip
    if total > 0 && existing >= total {
        state.log.lock().unwrap().push((format!("{} already downloaded, skipping...", label), pct_end));
        return Ok(());
    }
    // Open file for append if resuming, create if new
    let mut file = if existing > 0 {
        state.log.lock().unwrap().push((format!("Resuming {} from {:.1} MB...", label, existing as f64 / 1_048_576.0), pct_start));
        fs::OpenOptions::new().append(true).open(dest).map_err(|e| e.to_string())?
    } else {
        fs::File::create(dest).map_err(|e| e.to_string())?
    };
    // Send Range header if resuming
    let resp = if existing > 0 {
        ureq::get(url)
            .set("Range", &format!("bytes={}-", existing))
            .call()
            .map_err(|e| e.to_string())?
    } else {
        ureq::get(url).call().map_err(|e| e.to_string())?
    };
    let mut downloaded = existing;
    let mut reader = resp.into_reader();
    let mut buf = vec![0u8; 65536];
    let mut last_pct = pct_start;
    loop {
        let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 { break; }
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        downloaded += n as u64;
        if total > 0 {
            let progress = downloaded as f64 / total as f64;
            let pct = pct_start + ((pct_end - pct_start) as f64 * progress) as i32;
            if pct > last_pct {
                last_pct = pct;
                let mb_done = downloaded as f64 / 1_048_576.0;
                let mb_total = total as f64 / 1_048_576.0;
                state.log.lock().unwrap().push((
                    format!("Downloading {}... {:.1}/{:.1} MB ({:.0}%)", label, mb_done, mb_total, progress * 100.0),
                    pct
                ));
            }
        } else {
            let mb = downloaded as f64 / 1_048_576.0;
            state.log.lock().unwrap().push((format!("Downloading {}... {:.1} MB", label, mb), pct_start));
        }
    }
    Ok(())
}

fn http_download_progress(url: &str, dest: &PathBuf, label: &str, pct_start: i32, pct_end: i32, state: &Arc<InstallState>) -> Result<(), String> {
    let resp = ureq::get(url).call().map_err(|e| e.to_string())?;
    let total = resp.header("content-length")
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0);
    let mut file = fs::File::create(dest).map_err(|e| e.to_string())?;
    let mut reader = resp.into_reader();
    let mut buf = vec![0u8; 65536];
    let mut downloaded: u64 = 0;
    let mut last_pct = pct_start;
    loop {
        let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 { break; }
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        downloaded += n as u64;
        if total > 0 {
            let progress = downloaded as f64 / total as f64;
            let pct = pct_start + ((pct_end - pct_start) as f64 * progress) as i32;
            if pct > last_pct {
                last_pct = pct;
                let mb_done = downloaded as f64 / 1_048_576.0;
                let mb_total = total as f64 / 1_048_576.0;
                state.log.lock().unwrap().push((
                    format!("Downloading {}... {:.1}/{:.1} MB ({:.0}%)", label, mb_done, mb_total, progress * 100.0),
                    pct
                ));
            }
        } else {
            let mb_done = downloaded as f64 / 1_048_576.0;
            state.log.lock().unwrap().push((format!("Downloading {}... {:.1} MB", label, mb_done), pct_start));
        }
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
        .creation_flags(0x08000000)
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

#[tauri::command]
fn close_installer(app: tauri::AppHandle) {
    app.exit(0);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let state = Arc::new(InstallState::default());
    tauri::Builder::default()
        .manage(state)
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![
            check_installed, start_install, get_progress, launch_app, open_discord, close_installer
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}