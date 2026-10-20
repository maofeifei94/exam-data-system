# 打包指南

本文说明如何把「考试数据处理系统」编译并打包成可直接安装运行的程序。

## 1. 项目结构

```
.
├── backend/                Rust 后端（Axum + SQLite）
│   ├── src/                导入、字段映射、流程引擎、PDF 生成
│   ├── assets/             内嵌中文字体 NotoSansSC.ttf
│   └── Cargo.toml
├── frontend/               React + Vite 前端 + Electron 桌面外壳
│   ├── electron/           主进程 main.cjs、preload.cjs
│   ├── build/              打包资源（icon.png / icon.svg）
│   ├── scripts/            打包脚本 dist.mjs、after-pack.cjs
│   └── package.json
├── document/               项目文档
└── .gitignore
```

前端负责界面，后端负责数据处理，两者由 Electron 主进程在运行时串联（见第 6 节）。

## 2. 环境要求

| 平台 | 需要安装 |
| --- | --- |
| 通用 | Node.js 18+（含 npm）、Rust stable（含 cargo） |
| Windows | Visual Studio Build Tools 的「使用 C++ 的桌面开发」工作负载 |
| Ubuntu | 无额外编译工具；运行 AppImage 需要 libfuse2 |

Windows 需要 C/C++ 编译器，是因为后端依赖的 `rusqlite` 打开了 `bundled` 特性，会在编译期编译 SQLite 源码，需要 `cl.exe` / `link.exe`。

## 3. 一键打包

```bash
cd frontend
npm install
npm run dist
```

`npm run dist` 会按当前运行的操作系统自动选择打包目标，**不做交叉编译**：

| 运行平台 | 打包目标 | 产物 |
| --- | --- | --- |
| Windows | NSIS 安装程序 | `frontend/release/ExamDataSetup-0.1.0.exe` |
| Ubuntu / Linux | AppImage | `frontend/release/ExamData-0.1.0-linux.AppImage` |

> 首次 `npm install` 需要下载 Electron 二进制，若网络较慢可通过 `ELECTRON_MIRROR` 指定镜像。

## 4. 打包流程

脚本 [dist.mjs](../frontend/scripts/dist.mjs) 依次执行三步：

1. **构建前端**：`npm run build`（先 `tsc --noEmit` 类型检查，再由 Vite 产出 `frontend/dist`）
2. **编译后端**：在本机目标上 `cargo build --release`，然后把可执行文件暂存到 `frontend/build/backend/`
   - Windows 产出 `exam-backend.exe`
   - Linux 产出 `exam-backend`（并补上 0755 可执行位）
3. **打包**：调用 electron-builder
   - Windows：`electron-builder --win --x64`，用内置 NSIS 生成安装程序，图标与版本信息由 rcedit 原生写入
   - Linux：`electron-builder --linux --x64`，生成 AppImage；打包后由 [after-pack.cjs](../frontend/scripts/after-pack.cjs) 钩子给后端二进制补回可执行位

第 2 步会把编译产物统一暂存到 `frontend/build/backend/`，由 `package.json` 中的 `extraResources` 复制进安装包的 `resources/backend/`，因此两种平台的打包配置可以共用，无需分平台维护。

## 5. 运行安装包

**Windows**

双击 `ExamDataSetup-0.1.0.exe`，按向导安装（可选安装目录，会创建桌面与开始菜单快捷方式）。安装目录内的主程序为 `考试数据处理系统.exe`。

**Ubuntu**

```bash
chmod +x ExamData-0.1.0-linux.AppImage
./ExamData-0.1.0-linux.AppImage
```

若提示缺少 FUSE：

```bash
sudo apt install libfuse2
# 或跳过 FUSE 直接解包运行
./ExamData-0.1.0-linux.AppImage --appimage-extract-and-run
```

## 6. 运行期行为

- 应用启动时，主进程会自动拉起随包发布的 `resources/backend/exam-backend(.exe)`，等它就绪后再显示窗口，因此用户无需单独启动后端。
- 后端固定监听 `127.0.0.1:8710`，健康检查接口为 `/api/health`。该端口需保持空闲。
- **数据库写入用户数据目录，不写安装目录**（安装目录通常只读）：
  - Windows：`%APPDATA%\exam-data-frontend\exam_data.db`
  - Linux：`~/.config/exam-data-frontend/exam_data.db`
  - 目录名取自应用名；若调整 `package.json` 的 `name` / `productName`，该目录名会随之变化。
- 首次启动自动建表，无需手工初始化。
- 开发模式（存在 `VITE_DEV_SERVER_URL`）下主进程不拉起后端，改由开发者自行启动。

## 7. 常见问题

| 现象 | 处理 |
| --- | --- |
| Windows 编译后端报找不到 `cl.exe` / `link.exe` | 安装 VS Build Tools 的 C++ 工作负载，或改用 MSVC 工具链的 Rust 版本 |
| AppImage 报 FUSE 相关错误 | 安装 `libfuse2`，或用 `--appimage-extract-and-run` |
| 界面空白、后端未启动 | 检查 8710 端口是否被占用，以及安装包内是否存在 `resources/backend/exam-backend` |
| 想换应用图标 | 替换 `frontend/build/icon.png`（建议 512×512 以上）后重新打包 |
| 改了后端代码但没生效 | 重新执行 `npm run dist`，脚本会重新编译并覆盖 `frontend/build/backend/` |

## 8. 手动分步执行（可选）

需要单独调试某一步时，可手动执行等价命令：

```bash
cd frontend
npm run build                       # 1. 前端
cd ../backend && cargo build --release   # 2. 后端
cp target/release/exam-backend* ../frontend/build/backend/   # 2. 暂存（Windows 取 exam-backend.exe）
cd ../frontend
npx electron-builder --win --x64    # 3. Windows
npx electron-builder --linux --x64  # 3. Ubuntu
```
