const { app, BrowserWindow, shell } = require('electron');
const path = require('path');
const fs = require('fs');
const http = require('http');
const { spawn } = require('child_process');

const DEV_SERVER_URL = process.env.VITE_DEV_SERVER_URL;

// 后端固定监听本机回环地址，端口需与 preload.cjs 暴露的 baseUrl 保持一致。
const BACKEND_PORT = process.env.EXAM_PORT || '8710';
const BACKEND_HOST = '127.0.0.1';

let backendProcess = null;

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
    minWidth: 1100,
    minHeight: 700,
    title: '考试数据处理系统',
    webPreferences: {
      preload: path.join(__dirname, 'preload.cjs'),
      contextIsolation: true,
      nodeIntegration: false,
    },
  });

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

app.whenReady().then(async () => {
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
