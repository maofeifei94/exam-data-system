const { contextBridge, ipcRenderer } = require('electron');

// 后端地址：页面从 file:// 加载时必须用绝对 URL
contextBridge.exposeInMainWorld('examapi', {
  baseUrl: 'http://127.0.0.1:8710',
});

// 桌面能力：无边框窗口控制 + 系统原生文件/文件夹对话框
contextBridge.exposeInMainWorld('examDesktop', {
  platform: process.platform,
  minimize: () => ipcRenderer.invoke('window:minimize'),
  toggleMaximize: () => ipcRenderer.invoke('window:toggle-maximize'),
  close: () => ipcRenderer.invoke('window:close'),
  isMaximized: () => ipcRenderer.invoke('window:is-maximized'),
  onMaximizedChange: (callback) => {
    const listener = (_event, maximized) => callback(Boolean(maximized));
    ipcRenderer.on('window:maximized-changed', listener);
    return () => ipcRenderer.removeListener('window:maximized-changed', listener);
  },
  selectFolder: (defaultPath) => ipcRenderer.invoke('dialog:select-folder', defaultPath),
  selectFile: (options) => ipcRenderer.invoke('dialog:select-file', options),
  saveFile: (options) => ipcRenderer.invoke('dialog:save-file', options),
  openPath: (target) => ipcRenderer.invoke('shell:open-path', target),
});
