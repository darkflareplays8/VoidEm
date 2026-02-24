const { contextBridge, ipcRenderer } = require('electron');

contextBridge.exposeInMainWorld('dm', {
  checkSetup: () => ipcRenderer.invoke('check-setup'),
  runSetup: () => ipcRenderer.invoke('run-setup'),
  createInstance: (data) => ipcRenderer.invoke('create-instance', data),
  startInstance: (id) => ipcRenderer.invoke('start-instance', { id }),
  stopInstance: (id) => ipcRenderer.invoke('stop-instance', { id }),
  adbTap: (id, x, y) => ipcRenderer.invoke('adb-tap', { id, x, y }),
  adbKey: (id, key) => ipcRenderer.invoke('adb-key', { id, key }),
  adbSwipe: (id, x1, y1, x2, y2) => ipcRenderer.invoke('adb-swipe', { id, x1, y1, x2, y2 }),
  onSetupProgress: (cb) => ipcRenderer.on('setup-progress', (_, d) => cb(d)),
  onInstanceLog: (cb) => ipcRenderer.on('instance-log', (_, d) => cb(d)),
  onInstanceState: (cb) => ipcRenderer.on('instance-state', (_, d) => cb(d)),
  onScreenUpdate: (cb) => ipcRenderer.on('screen-update', (_, d) => cb(d)),
});