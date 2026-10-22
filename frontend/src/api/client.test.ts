import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  ApiError,
  apiRequest,
  apiRequestBlob,
  deleteProject,
  isApiError,
  listProjects,
} from './client';

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}

function mockFetch(handler: (input: RequestInfo | URL, init?: RequestInit) => Promise<Response>) {
  return vi.spyOn(globalThis, 'fetch').mockImplementation(handler);
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe('成功响应', () => {
  it('解析 JSON 并透传请求路径', async () => {
    const fetchMock = mockFetch(async () => jsonResponse([{ id: 'p1' }]));
    const projects = await listProjects();

    expect(projects).toEqual([{ id: 'p1' }]);
    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(String(fetchMock.mock.calls[0][0])).toContain('/api/projects');
  });

  it('204 / 空响应体返回 undefined，不因 JSON 解析报错', async () => {
    mockFetch(async () => new Response(null, { status: 204 }));
    await expect(deleteProject('p1')).resolves.toBeUndefined();
  });

  it('自动为 JSON 请求补充 Content-Type，但不对 FormData 生效', async () => {
    const fetchMock = mockFetch(async () => jsonResponse({ ok: true }));
    await apiRequest('/api/projects', { method: 'POST', body: JSON.stringify({ name: 'x' }) });

    const headers = new Headers(fetchMock.mock.calls[0][1]?.headers);
    expect(headers.get('Content-Type')).toBe('application/json');
  });
});

describe('错误契约', () => {
  it('保留后端稳定的错误码与提示文案', async () => {
    mockFetch(async () =>
      jsonResponse({ error: '数据冲突：违反唯一性约束', code: 'conflict' }, 409),
    );

    const err = await listProjects().catch((e: unknown) => e);
    expect(isApiError(err)).toBe(true);
    expect(err).toBeInstanceOf(ApiError);
    expect(err).toMatchObject({
      status: 409,
      code: 'conflict',
      message: '数据冲突：违反唯一性约束',
    });
  });

  it('非 JSON 错误页只使用状态码文案，不泄露响应体', async () => {
    mockFetch(async () => new Response('<html>stack trace secret</html>', { status: 500 }));

    const err = await listProjects().catch((e: unknown) => e);
    expect(err).toMatchObject({ status: 500, code: 'http_500', message: '服务器内部错误' });
    expect((err as Error).message).not.toContain('stack trace');
  });

  it('成功响应但不是合法 JSON 时给出明确错误码', async () => {
    mockFetch(async () => new Response('not-json', { status: 200 }));

    const err = await listProjects().catch((e: unknown) => e);
    expect(err).toMatchObject({ status: 200, code: 'invalid_response' });
  });

  it('二进制接口同样复用统一错误解析', async () => {
    mockFetch(async () => jsonResponse({ error: 'PDF 模板不存在', code: 'not_found' }, 404));

    await expect(apiRequestBlob('/api/pdf-templates/x/render')).rejects.toMatchObject({
      status: 404,
      code: 'not_found',
      message: 'PDF 模板不存在',
    });
  });
});

describe('网络层异常', () => {
  it('连接失败映射为 network_error', async () => {
    mockFetch(async () => {
      throw new TypeError('fetch failed');
    });

    const err = await listProjects().catch((e: unknown) => e);
    expect(err).toMatchObject({ status: 0, code: 'network_error' });
  });

  it('超时后主动中止请求，并给出 timeout 错误码', async () => {
    mockFetch(
      (_input, init) =>
        new Promise<Response>((_resolve, reject) => {
          init?.signal?.addEventListener('abort', () => {
            reject(new DOMException('The operation was aborted.', 'AbortError'));
          });
        }),
    );

    await expect(apiRequest('/api/slow', { timeoutMs: 10 })).rejects.toMatchObject({
      status: 0,
      code: 'timeout',
    });
  });

  it('调用方主动取消与超时区分开', async () => {
    mockFetch(
      (_input, init) =>
        new Promise<Response>((_resolve, reject) => {
          init?.signal?.addEventListener('abort', () => {
            reject(new DOMException('The operation was aborted.', 'AbortError'));
          });
        }),
    );

    const controller = new AbortController();
    const pending = apiRequest('/api/slow', { signal: controller.signal, timeoutMs: 0 });
    controller.abort();

    await expect(pending).rejects.toMatchObject({ status: 0, code: 'aborted' });
  });
});
