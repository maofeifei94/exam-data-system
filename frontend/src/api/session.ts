/**
 * 登录 token 的本地存储与全局 401 事件。
 *
 * 单独成模块是为了让 `client.ts` 与认证上下文共享状态，同时避免循环依赖。
 */

const TOKEN_KEY = 'exam.auth.token';

/** 收到 401 时派发的事件名：认证上下文监听后清理登录态。 */
export const UNAUTHORIZED_EVENT = 'exam:unauthorized';

function storage(): Storage | null {
  try {
    return typeof window !== 'undefined' ? window.localStorage : null;
  } catch {
    // 隐私模式或存储被禁用时退化为内存外的空实现
    return null;
  }
}

let memoryToken = '';

export function getToken(): string {
  return storage()?.getItem(TOKEN_KEY) ?? memoryToken;
}

export function setToken(token: string): void {
  memoryToken = token;
  storage()?.setItem(TOKEN_KEY, token);
}

export function clearToken(): void {
  memoryToken = '';
  storage()?.removeItem(TOKEN_KEY);
}

export function emitUnauthorized(): void {
  if (typeof window !== 'undefined') {
    window.dispatchEvent(new Event(UNAUTHORIZED_EVENT));
  }
}
