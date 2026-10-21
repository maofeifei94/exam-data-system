const { app, BrowserWindow, dialog, ipcMain, shell } = require('electron');
const path = require('path');
const fs = require('fs');
const http = require('http');
const { spawn } = require('child_process');

const DEV_SERVER_URL = process.env.VITE_DEV_SERVER_URL;

// 后端固定监听本机回环地址，端口需与 preload.cjs 暴露的 baseUrl 保持一致。
const BACKEND_PORT = process.env.EXAM_PORT || '8710';
const BACKEND_HOST = '127.0.0.1';

let backendProcess = null;
let mainWindow = null;

/**
 * 打包运行时拉起随包发布的后端可执行文件。
 * 数据库写入用户数据目录（安装目录通常只读），保证首次运行即可建表。
 * 开发模式（VITE_DEV_SERVER_URL 存在）下不拉起，由开发者自行运行后端。
 */
function startBackend() {
  if (DEV_SERVER_URL) {
    return;
  }
  const exeName = process.platform === 'win32' ? 'exam-backend.exe' : 'exam-backend';
  const exePath = path.join(process.resourcesPath, 'backend', exeName);
  if (!fs.existsSync(exePath)) {
    console.error('未找到后端可执行文件：', exePath);
    return;
  }
  const dataDir = app.getPath('userData');
  fs.mkdirSync(dataDir, { recursive: true });

  backendProcess = spawn(exePath, [], {
    cwd: dataDir,
    env: {
      ...process.env,
      EXAM_ADDR: `${BACKEND_HOST}:${BACKEND_PORT}`,
      EXAM_DB: path.join(dataDir, 'exam_data.db'),
    },
    stdio: ['ignore', 'pipe', 'pipe'],
    windowsHide: true,
  });
  backendProcess.stdout.on('data', (buf) => process.stdout.write(`[backend] ${buf}`));
  backendProcess.stderr.on('data', (buf) => process.stderr.write(`[backend] ${buf}`));
  backendProcess.on('exit', (code, signal) => {
    console.log(`后端进程已退出 code=${code} signal=${signal}`);
    backendProcess = null;
  });
}

function stopBackend() {
  if (backendProcess) {
    try {
      backendProcess.kill();
    } catch (err) {
      console.error('停止后端进程失败：', err);
    }
    backendProcess = null;
  }
}

/** 轮询 /api/health，等待后端就绪；超时也继续，避免白屏。 */
function waitForBackend(timeoutMs = 20000) {
  const deadline = Date.now() + timeoutMs;
  return new Promise((resolve) => {
    const attempt = () => {
      const req = http.get(
        { host: BACKEND_HOST, port: BACKEND_PORT, path: '/api/health', timeout: 2000 },
        (res) => {
          res.resume();
          resolve(true);
        },
      );
      req.on('error', () => {
        if (Date.now() >= deadline) {
          resolve(false);
        } else {
          setTimeout(attempt, 300);
        }
      });
      req.on('timeout', () => req.destroy());
    };
    attempt();
  });
}

function createWindow() {
  const win = new BrowserWindow({
    width: 1440,
    height: 900,
    minWidth: 1024,
    minHeight: 640,
    title: '考试数据处理系统',
    // 无边框窗口：标题栏与窗口按钮由渲染进程自绘，像原生桌面软件而不是网页
    frame: false,
    show: false,
    backgroundColor: '#f4f6f9',
    webPreferences: {
      preload: path.join(__dirname, 'preload.cjs'),
      contextIsolation: true,
      nodeIntegration: false,
    },
  });

  mainWindow = win;

  // 避免白屏闪烁：等首帧准备好再显示
  win.once('ready-to-show', () => win.show());

  const notifyMaximized = () => {
    if (!win.isDestroyed()) {
      win.webContents.send('window:maximized-changed', win.isMaximized());
    }
  };
  win.on('maximize', notifyMaximized);
  win.on('unmaximize', notifyMaximized);

  // Open external links in the user's default browser.
  win.webContents.setWindowOpenHandler(({ url }) => {
    shell.openExternal(url);
    return { action: 'deny' };
  });

  if (DEV_SERVER_URL) {
    win.loadURL(DEV_SERVER_URL);
    win.webContents.openDevTools({ mode: 'detach' });
  } else {
    win.loadFile(path.join(__dirname, '..', 'dist', 'index.html'));
  }

  return win;
}

// ---------------------------------------------------------------------------
// 无边框窗口控制 + 系统原生对话框（渲染进程通过 preload 暴露的 API 调用）
// ---------------------------------------------------------------------------

function registerIpc() {
  ipcMain.handle('window:minimize', (event) => {
    BrowserWindow.fromWebContents(event.sender)?.minimize();
  });

  ipcMain.handle('window:toggle-maximize', (event) => {
    const win = BrowserWindow.fromWebContents(event.sender);
    if (!win) return false;
    if (win.isMaximized()) {
      win.unmaximize();
    } else {
      win.maximize();
    }
    return win.isMaximized();
  });

  ipcMain.handle('window:close', (event) => {
    BrowserWindow.fromWebContents(event.sender)?.close();
  });

  ipcMain.handle('window:is-maximized', (event) => {
    return BrowserWindow.fromWebContents(event.sender)?.isMaximized() ?? false;
  });

  // 选择项目文件夹：弹出系统文件夹选择窗口
  ipcMain.handle('dialog:select-folder', async (event, defaultPath) => {
    const win = BrowserWindow.fromWebContents(event.sender);
    const result = await dialog.showOpenDialog(win, {
      title: '选择项目文件夹',
      defaultPath: typeof defaultPath === 'string' && defaultPath ? defaultPath : undefined,
      properties: ['openDirectory', 'createDirectory'],
      buttonLabel: '选择此文件夹',
    });
    if (result.canceled || result.filePaths.length === 0) return null;
    return result.filePaths[0];
  });

  // 选择要导入的数据文件：弹出系统文件选择窗口
  ipcMain.handle('dialog:select-file', async (event, options) => {
    const win = BrowserWindow.fromWebContents(event.sender);
    const opts = options && typeof options === 'object' ? options : {};
    const result = await dialog.showOpenDialog(win, {
      title: opts.title || '选择文件',
      properties: ['openFile'],
      filters: opts.filters || [
        { name: '表格文件', extensions: ['xlsx', 'xls', 'csv'] },
        { name: '全部文件', extensions: ['*'] },
      ],
    });
    if (result.canceled || result.filePaths.length === 0) return null;
    return result.filePaths[0];
  });

  // 导出文件时选择保存位置
  ipcMain.handle('dialog:save-file', async (event, options) => {
    const win = BrowserWindow.fromWebContents(event.sender);
    const opts = options && typeof options === 'object' ? options : {};
    const result = await dialog.showSaveDialog(win, {
      title: opts.title || '保存文件',
      defaultPath: opts.defaultPath || undefined,
      filters: opts.filters,
    });
    if (result.canceled || !result.filePath) return null;
    return result.filePath;
  });

  // 在系统文件管理器中打开某个目录
  ipcMain.handle('shell:open-path', async (_event, target) => {
    if (typeof target !== 'string' || !target) return false;
    const err = await shell.openPath(target);
    return err === '';
  });
}

app.whenReady().then(async () => {
  registerIpc();
  startBackend();
  await waitForBackend();
  createWindow();

  app.on('activate', () => {
    if (BrowserWindow.getAllWindows().length === 0) {
      createWindow();
    }
  });
});

app.on('window-all-closed', () => {
  if (process.platform !== 'darwin') {
    app.quit();
  }
});

app.on('before-quit', () => {
  stopBackend();
});

app.on('will-quit', () => {
  stopBackend();
});
