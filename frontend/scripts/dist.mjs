#!/usr/bin/env node
// 按当前运行环境一键编译打包安装包。
//
//   npm run dist
//
// 平台分支：
//   · Windows        → NSIS 安装程序（electron-builder 内置 NSIS，图标/版本信息原生写入）
//   · Ubuntu / Linux → AppImage（本机原生编译 Rust 后端，不做任何交叉编译）
//
// 前置条件：
//   Windows：Node 18+、Rust（默认 MSVC 工具链）+ Visual Studio Build Tools
//   Linux  ：Node 18+、Rust（本机工具链）

import { spawnSync } from 'node:child_process';
import { chmodSync, copyFileSync, existsSync, mkdirSync, readdirSync, rmSync } from 'node:fs';
import { platform } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const IS_WINDOWS = platform() === 'win32';
const IS_LINUX = platform() === 'linux';

// 后端产物名与打包目标随平台变化；数据库/日志均落在用户数据目录，与安装目录无关。
const BACKEND_EXE_NAME = IS_WINDOWS ? 'exam-backend.exe' : 'exam-backend';

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const projectDir = path.resolve(scriptDir, '..');
const backendDir = path.resolve(projectDir, '..', 'backend');
const stagingDir = path.join(projectDir, 'build', 'backend');
const releaseDir = path.join(projectDir, 'release');

function run(command, args, options = {}) {
  console.log(`\n$ ${command} ${args.join(' ')}`);
  const result = spawnSync(command, args, { stdio: 'inherit', cwd: projectDir, shell: IS_WINDOWS, ...options });
  if (result.status !== 0) {
    throw new Error(`命令执行失败：${command}（退出码 ${result.status}）`);
  }
}

function step1Frontend() {
  console.log('\n=== 1/3 构建前端 ===');
  run('npm', ['run', 'build']);
}

function step2Backend() {
  console.log('\n=== 2/3 编译 Rust 后端（本机目标，不交叉编译） ===');
  run('cargo', ['build', '--release', '--manifest-path', path.join(backendDir, 'Cargo.toml')], { cwd: backendDir });

  const built = path.join(backendDir, 'target', 'release', BACKEND_EXE_NAME);
  if (!existsSync(built)) {
    throw new Error(`未找到后端产物：${built}`);
  }

  // 清空暂存目录，避免残留另一平台的旧二进制被打进包里。
  rmSync(stagingDir, { recursive: true, force: true });
  mkdirSync(stagingDir, { recursive: true });
  const staged = path.join(stagingDir, BACKEND_EXE_NAME);
  copyFileSync(built, staged);
  if (!IS_WINDOWS) {
    chmodSync(staged, 0o755);
  }
  console.log(`后端已暂存至：${staged}`);
}

function step3Package() {
  if (IS_WINDOWS) {
    console.log('\n=== 3/3 打包 Windows 安装程序（NSIS） ===');
    run('npx', ['electron-builder', '--win', '--x64']);
  } else if (IS_LINUX) {
    console.log('\n=== 3/3 打包 Ubuntu 可运行包（AppImage） ===');
    run('npx', ['electron-builder', '--linux', '--x64']);
  } else {
    throw new Error(`不支持的平台：${platform()}（目前仅支持 Windows 与 Linux）`);
  }
  console.log(`\n安装包已生成于：${releaseDir}`);
}

step1Frontend();
step2Backend();
step3Package();
