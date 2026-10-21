// 桌面端能力封装：Electron 中走 IPC（系统原生对话框、无边框窗口控制），
// 纯浏览器开发时优雅降级，不抛异常。

export interface DesktopApi {
  platform: string;
  minimize: () => Promise<void>;
  toggleMaximize: () => Promise<boolean>;
  close: () => Promise<void>;
  isMaximized: () => Promise<boolean>;
  onMaximizedChange: (callback: (maximized: boolean) => void) => () => void;
  selectFolder: (defaultPath?: string) => Promise<string | null>;
  selectFile: (options?: {
    title?: string;
    filters?: { name: string; extensions: string[] }[];
  }) => Promise<string | null>;
  saveFile: (options?: {
    title?: string;
    defaultPath?: string;
    filters?: { name: string; extensions: string[] }[];
  }) => Promise<string | null>;
  openPath: (target: string) => Promise<boolean>;
}

declare global {
  interface Window {
    examDesktop?: DesktopApi;
  }
}

export function desktop(): DesktopApi | undefined {
  return window.examDesktop;
}

export function isDesktop(): boolean {
  return Boolean(window.examDesktop);
}

/** 弹出系统文件夹选择窗口；浏览器环境返回 null。 */
export async function pickFolder(defaultPath?: string): Promise<string | null> {
  const api = desktop();
  if (!api) return null;
  return api.selectFolder(defaultPath);
}

/** 在系统文件管理器中打开目录；浏览器环境返回 false。 */
export async function revealFolder(target: string): Promise<boolean> {
  const api = desktop();
  if (!api || !target) return false;
  return api.openPath(target);
}
