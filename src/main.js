const { app, BrowserWindow, ipcMain, shell } = require('electron');
const path = require('path');
const fs = require('fs');
const os = require('os');
const { spawn, execSync } = require('child_process');
const https = require('https');
const http = require('http');

// App data dir — user's AppData/Roaming/VoidEmulator
const DATA_DIR = path.join(app.getPath('userData'), 'data');
const QEMU_DIR = path.join(DATA_DIR, 'qemu');
const IMAGES_DIR = path.join(DATA_DIR, 'images');
const INSTANCES_DIR = path.join(DATA_DIR, 'instances');
const BASE_IMG = path.join(IMAGES_DIR, 'android.img');
const QEMU_EXE = path.join(QEMU_DIR, 'qemu-system-i386.exe');
const QEMU_IMG = path.join(QEMU_DIR, 'qemu-img.exe');
const ADB_EXE = path.join(QEMU_DIR, 'adb.exe');

[DATA_DIR, QEMU_DIR, IMAGES_DIR, INSTANCES_DIR].forEach(d => {
  if (!fs.existsSync(d)) fs.mkdirSync(d, { recursive: true });
});

// Track running instances
const instances = {};
let mainWindow;

function createWindow() {
  mainWindow = new BrowserWindow({
    width: 1280,
    height: 800,
    minWidth: 900,
    minHeight: 600,
    backgroundColor: '#07030f',
    titleBarStyle: 'hidden',
    titleBarOverlay: {
      color: '#09070f',
      symbolColor: '#c044ff',
      height: 40
    },
    webPreferences: {
      nodeIntegration: false,
      contextIsolation: true,
      preload: path.join(__dirname, 'preload.js')
    },
    icon: path.join(__dirname, '..', 'assets', 'icon.ico')
  });

  mainWindow.loadFile(path.join(__dirname, 'index.html'));
}

app.whenReady().then(createWindow);
app.on('window-all-closed', () => {
  // Kill all QEMU processes
  Object.values(instances).forEach(inst => {
    try { inst.process?.kill(); } catch {}
  });
  app.quit();
});

// ─── SETUP / DOWNLOAD ───────────────────────────────────────────────

function download(url, dest, onProgress) {
  return new Promise((resolve, reject) => {
    const file = fs.createWriteStream(dest);
    const protocol = url.startsWith('https') ? https : http;

    const request = (u) => {
      protocol.get(u, (res) => {
        if (res.statusCode === 301 || res.statusCode === 302) {
          return request(res.headers.location);
        }
        const total = parseInt(res.headers['content-length'] || '0');
        let received = 0;
        res.on('data', chunk => {
          received += chunk.length;
          if (total) onProgress?.(Math.round(received / total * 100), received, total);
        });
        res.pipe(file);
        file.on('finish', () => { file.close(); resolve(); });
        res.on('error', reject);
      }).on('error', reject);
    };
    request(url);
  });
}

async function extractZip(zipPath, destDir) {
  const StreamZip = require('node-stream-zip');
  const zip = new StreamZip.async({ file: zipPath });
  await zip.extract(null, destDir);
  await zip.close();
}

ipcMain.handle('check-setup', async () => {
  return {
    qemu: fs.existsSync(QEMU_EXE),
    adb: fs.existsSync(ADB_EXE),
    image: fs.existsSync(BASE_IMG),
    qemuImg: fs.existsSync(QEMU_IMG)
  };
});

ipcMain.handle('run-setup', async (event) => {
  const send = (msg, pct) => {
    console.log(`[SETUP ${pct}%] ${msg}`);
    mainWindow.webContents.send('setup-progress', { msg, pct });
  };

  try {
    send('Setup started...', 1);
    // 1. Download & silently install QEMU
    if (!fs.existsSync(QEMU_EXE)) {
      send('Downloading QEMU installer...', 2);
      const qemuInstaller = path.join(DATA_DIR, 'qemu-setup.exe');
      await download(
        'https://qemu.weilnetz.de/w64/qemu-w64-setup-20251217.exe',
        qemuInstaller,
        (pct, recv, total) => {
          const mb = (recv / 1024 / 1024).toFixed(1);
          const tot = (total / 1024 / 1024).toFixed(1);
          send(`Downloading QEMU... ${mb} / ${tot} MB`, 2 + pct * 0.2);
        }
      );
      send('Installing QEMU silently...', 23);
      // Silent install to our QEMU dir
      execSync(`"${qemuInstaller}" /S /D=${QEMU_DIR}`, { timeout: 120000 });
      fs.unlinkSync(qemuInstaller);

      // Also check default install location and copy if needed
      if (!fs.existsSync(QEMU_EXE)) {
        const defaultPath = 'C:\Program Files\qemu\qemu-system-i386.exe';
        if (fs.existsSync(defaultPath)) {
          const qemuDefaultDir = 'C:\Program Files\qemu';
          ['qemu-system-i386.exe', 'qemu-img.exe', 'qemu-system-x86_64.exe'].forEach(f => {
            const src = path.join(qemuDefaultDir, f);
            if (fs.existsSync(src)) fs.copyFileSync(src, path.join(QEMU_DIR, f));
          });
        }
      }
      send('Verifying QEMU...', 28);
    } else {
      send('QEMU already installed — skipping...', 28);
    }
    send('QEMU ready ✓', 30);

    // 2. Download ADB
    if (!fs.existsSync(ADB_EXE)) {
      send('Downloading Android Debug Bridge (ADB)...', 32);
      const adbZip = path.join(DATA_DIR, 'adb.zip');
      await download(
        'https://dl.google.com/android/repository/platform-tools-latest-windows.zip',
        adbZip,
        (pct, recv, total) => {
          const mb = (recv / 1024 / 1024).toFixed(1);
          const tot = (total / 1024 / 1024).toFixed(1);
          send(`Downloading ADB tools... ${mb} / ${tot} MB`, 32 + pct * 0.18);
        }
      );
      send('Extracting ADB tools...', 50);
      await extractZip(adbZip, DATA_DIR);
      const ptDir = path.join(DATA_DIR, 'platform-tools');
      ['adb.exe', 'AdbWinApi.dll', 'AdbWinUsbApi.dll'].forEach(f => {
        const src = path.join(ptDir, f);
        if (fs.existsSync(src)) fs.copyFileSync(src, path.join(QEMU_DIR, f));
      });
      fs.rmSync(ptDir, { recursive: true, force: true });
      fs.unlinkSync(adbZip);
      send('Verifying ADB...', 53);
    } else {
      send('ADB already installed — skipping...', 53);
    }
    send('ADB ready ✓', 55);

    // 3. Download Android-x86 image
    if (!fs.existsSync(BASE_IMG)) {
      send('Downloading Android-x86 9.0 image — this may take a few minutes...', 57);
      const isoPath = path.join(IMAGES_DIR, 'android.iso');
      await download(
        'https://sourceforge.net/projects/android-x86/files/Release%204.4-r5/android-x86-4.4-r5.iso/download',
        isoPath,
        (pct, recv, total) => {
          const mb = (recv / 1024 / 1024).toFixed(0);
          const tot = (total / 1024 / 1024).toFixed(0);
          send(`Downloading Android 9.0... ${mb} / ${tot} MB (${pct}%)`, 57 + pct * 0.38);
        }
      );
      send('Creating virtual disk image...', 96);
      execSync(`"${QEMU_IMG}" create -f raw "${BASE_IMG}" 4G`);
      send('Cleaning up...', 98);
      fs.unlinkSync(isoPath);
      send('Android image ready ✓', 99);
    } else {
      send('Android image already exists — skipping...', 99);
    }

    send('All done! Launching VoidEmulator... 🚀', 100);
    return { success: true };
  } catch (err) {
    send(`Setup failed: ${err.message}`, -1);
    return { success: false, error: err.message };
  }
});

// ─── INSTANCE MANAGEMENT ────────────────────────────────────────────

ipcMain.handle('create-instance', async (event, { id, name, index }) => {
  const overlayImg = path.join(INSTANCES_DIR, `${id}.qcow2`);

  if (!fs.existsSync(overlayImg)) {
    execSync(`"${QEMU_IMG}" create -f qcow2 -b "${BASE_IMG}" -F raw "${overlayImg}"`);
  }

  instances[id] = { id, name, index, process: null, state: 'stopped', overlayImg };
  return { success: true };
});

ipcMain.handle('start-instance', async (event, { id }) => {
  const inst = instances[id];
  if (!inst) return { success: false, error: 'Instance not found' };

  const adbPort = 5554 + inst.index * 2;
  const args = [
    '-m', '512',
    '-smp', '1',
    '-drive', `file=${inst.overlayImg},format=qcow2`,
    '-net', 'nic',
    '-net', `user,hostfwd=tcp:127.0.0.1:${adbPort}-:5555`,
    '-vga', 'std',
    '-usb', '-device', 'usb-tablet',
    '-no-reboot',
    '-nographic',
  ];

  const proc = spawn(QEMU_EXE, args, { stdio: 'pipe' });
  inst.process = proc;
  inst.adbPort = adbPort;
  inst.state = 'starting';

  proc.stdout.on('data', d => mainWindow.webContents.send('instance-log', { id, msg: d.toString() }));
  proc.stderr.on('data', d => mainWindow.webContents.send('instance-log', { id, msg: d.toString() }));
  proc.on('exit', () => {
    inst.state = 'stopped';
    mainWindow.webContents.send('instance-state', { id, state: 'stopped' });
  });

  mainWindow.webContents.send('instance-state', { id, state: 'starting' });

  // Poll ADB
  setTimeout(async () => {
    for (let i = 0; i < 30; i++) {
      try {
        const result = execSync(`"${ADB_EXE}" connect 127.0.0.1:${adbPort}`, { timeout: 3000 }).toString();
        if (result.includes('connected')) {
          inst.state = 'running';
          mainWindow.webContents.send('instance-state', { id, state: 'running' });
          startScreenCapture(id);
          return;
        }
      } catch {}
      await new Promise(r => setTimeout(r, 3000));
    }
  }, 5000);

  return { success: true };
});

ipcMain.handle('stop-instance', async (event, { id }) => {
  const inst = instances[id];
  if (!inst) return;
  try { inst.process?.kill(); } catch {}
  inst.state = 'stopped';
  inst.process = null;
  clearInterval(inst.captureInterval);
  return { success: true };
});

ipcMain.handle('adb-tap', async (event, { id, x, y }) => {
  const inst = instances[id];
  if (!inst?.adbPort) return;
  try { execSync(`"${ADB_EXE}" -s 127.0.0.1:${inst.adbPort} shell input tap ${x} ${y}`, { timeout: 2000 }); } catch {}
});

ipcMain.handle('adb-key', async (event, { id, key }) => {
  const inst = instances[id];
  if (!inst?.adbPort) return;
  try { execSync(`"${ADB_EXE}" -s 127.0.0.1:${inst.adbPort} shell input keyevent ${key}`, { timeout: 2000 }); } catch {}
});

ipcMain.handle('adb-swipe', async (event, { id, x1, y1, x2, y2 }) => {
  const inst = instances[id];
  if (!inst?.adbPort) return;
  try { execSync(`"${ADB_EXE}" -s 127.0.0.1:${inst.adbPort} shell input swipe ${x1} ${y1} ${x2} ${y2} 200`, { timeout: 3000 }); } catch {}
});

function startScreenCapture(id) {
  const inst = instances[id];
  inst.captureInterval = setInterval(async () => {
    if (inst.state !== 'running') return;
    try {
      const tmpFile = path.join(DATA_DIR, `screen_${id}.png`);
      execSync(`"${ADB_EXE}" -s 127.0.0.1:${inst.adbPort} shell screencap -p /sdcard/sc.png`, { timeout: 3000 });
      execSync(`"${ADB_EXE}" -s 127.0.0.1:${inst.adbPort} pull /sdcard/sc.png "${tmpFile}"`, { timeout: 5000 });
      if (fs.existsSync(tmpFile)) {
        const data = fs.readFileSync(tmpFile).toString('base64');
        mainWindow.webContents.send('screen-update', { id, data });
      }
    } catch {}
  }, 800);
}