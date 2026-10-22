import type {
  AliasMapping,
  AliasMappingInput,
  ColumnSuggestion,
  Dataset,
  DuplicateInput,
  ExamProject,
  ExamProjectInput,
  FieldGroup,
  FieldGroupInput,
  Flow,
  FlowInput,
  FlowRun,
  FlowStep,
  FlowValidateResult,
  HealthResponse,
  ImportResult,
  OkResponse,
  OperatorDef,
  PdfTemplate,
  PdfTemplateInput,
  RowPage,
  RunResult,
  SavedResponse,
  StandardField,
  StandardFieldInput,
} from './types';

declare global {
  interface Window {
    examapi?: { baseUrl: string };
  }
}

// Electron 里由 preload 注入绝对地址（页面从 file:// 加载，必须用绝对 URL）；
// 纯浏览器开发时留空，走同源路径，由 Vite dev server 的 /api 代理转发到后端。
// `window` 判空让该模块也能在 Node（单元测试）环境导入。
const BASE_URL: string = typeof window !== 'undefined' ? window.examapi?.baseUrl ?? '' : '';

/** 普通查询接口的超时：本机 SQLite 操作正常远低于该值。 */
const DEFAULT_TIMEOUT_MS = 30_000;
/** 导入、运行流程、渲染 PDF 等长耗时接口的超时。 */
const LONG_TIMEOUT_MS = 5 * 60_000;

/**
 * 统一的接口错误。
 *
 * - `status` 为 HTTP 状态码，网络层失败时为 0；
 * - `code` 与后端 `{"error","code"}` 契约保持一致，前端应据此分支，
 *   而不是匹配可变的提示文案。
 */
export class ApiError extends Error {
  readonly status: number;
  readonly code: string;

  constructor(message: string, status: number, code: string) {
    super(message);
    this.name = 'ApiError';
    this.status = status;
    this.code = code;
  }
}

export function isApiError(err: unknown): err is ApiError {
  return err instanceof ApiError;
}

export interface ApiRequestOptions extends RequestInit {
  /** 覆盖默认超时；传 0 表示不限制（谨慎使用）。 */
  timeoutMs?: number;
}

const STATUS_MESSAGES: Record<number, string> = {
  400: '请求参数有误',
  401: '没有访问权限',
  403: '没有访问权限',
  404: '请求的资源不存在',
  409: '数据冲突，请刷新后重试',
  413: '上传内容过大',
  415: '不支持的文件类型',
  422: '请求数据校验失败',
  429: '请求过于频繁，请稍后重试',
  500: '服务器内部错误',
  502: '后端服务暂时不可用',
  503: '后端服务暂时不可用',
  504: '后端服务响应超时',
};

function statusMessage(status: number): string {
  return STATUS_MESSAGES[status] ?? `请求失败 (HTTP ${status})`;
}

async function parseError(res: Response): Promise<ApiError> {
  let message = statusMessage(res.status);
  let code = `http_${res.status}`;
  // 读取失败（例如读取过程中连接中断）时保留状态码文案。
  const text = await res.text().catch(() => '');
  if (text.trim()) {
    try {
      const data: unknown = JSON.parse(text);
      if (data && typeof data === 'object') {
        const body = data as { error?: unknown; code?: unknown };
        if (typeof body.error === 'string' && body.error.trim()) {
          message = body.error;
        }
        if (typeof body.code === 'string' && body.code.trim()) {
          code = body.code;
        }
      }
    } catch {
      // 非 JSON（例如代理返回的 HTML 错误页）：使用状态码文案，不把响应体透给用户。
    }
  }
  return new ApiError(message, res.status, code);
}

function parseJsonBody<T>(res: Response): Promise<T> {
  return res.text().then((text) => {
    // 204 或后端主动返回空体时视为无数据，避免把空字符串当 JSON 解析。
    if (!text.trim()) {
      return undefined as T;
    }
    try {
      return JSON.parse(text) as T;
    } catch {
      throw new ApiError('服务器返回了无法解析的数据', res.status, 'invalid_response');
    }
  });
}

interface PreparedRequest {
  init: RequestInit;
  /** 是否因超时被中止（区分用户主动取消）。 */
  timedOut: () => boolean;
  cleanup: () => void;
}

function prepareRequest(init: ApiRequestOptions): PreparedRequest {
  const { timeoutMs = DEFAULT_TIMEOUT_MS, ...rest } = init;
  const controller = new AbortController();
  let timedOut = false;
  let timer: ReturnType<typeof setTimeout> | undefined;

  const external = rest.signal;
  const onAbort = () => controller.abort();
  if (external) {
    if (external.aborted) {
      controller.abort();
    } else {
      external.addEventListener('abort', onAbort, { once: true });
    }
  }
  if (timeoutMs > 0) {
    timer = setTimeout(() => {
      timedOut = true;
      controller.abort();
    }, timeoutMs);
  }

  return {
    init: { ...rest, signal: controller.signal },
    timedOut: () => timedOut,
    cleanup: () => {
      if (timer !== undefined) clearTimeout(timer);
      external?.removeEventListener('abort', onAbort);
    },
  };
}

async function send<T>(
  path: string,
  init: ApiRequestOptions,
  read: (res: Response) => Promise<T>,
): Promise<T> {
  const headers = new Headers(init.headers);
  if (init.body && !(init.body instanceof FormData) && !headers.has('Content-Type')) {
    headers.set('Content-Type', 'application/json');
  }
  const prepared = prepareRequest(init);
  try {
    const res = await fetch(`${BASE_URL}${path}`, { ...prepared.init, headers });
    if (!res.ok) {
      throw await parseError(res);
    }
    return await read(res);
  } catch (err) {
    if (prepared.timedOut()) {
      throw new ApiError('请求超时，请稍后重试', 0, 'timeout');
    }
    if (err instanceof ApiError) {
      throw err;
    }
    if (err instanceof DOMException && err.name === 'AbortError') {
      throw new ApiError('请求已取消', 0, 'aborted');
    }
    throw new ApiError('无法连接后端服务，请确认服务已启动', 0, 'network_error');
  } finally {
    prepared.cleanup();
  }
}

/** 低层 JSON 请求入口；业务接口都应基于它，保证错误/超时行为一致。 */
export function apiRequest<T>(path: string, init: ApiRequestOptions = {}): Promise<T> {
  return send(path, init, (res) => parseJsonBody<T>(res));
}

/** 低层二进制请求入口（PDF、CSV 下载）。 */
export function apiRequestBlob(path: string, init: ApiRequestOptions = {}): Promise<Blob> {
  return send(path, init, (res) => res.blob());
}

function triggerBrowserDownload(blob: Blob, filename: string): void {
  const url = URL.createObjectURL(blob);
  const link = document.createElement('a');
  link.href = url;
  link.download = filename;
  document.body.appendChild(link);
  link.click();
  link.remove();
  URL.revokeObjectURL(url);
}

// ---------------------------------------------------------------------------
// 健康检查
// ---------------------------------------------------------------------------

export function getHealth(): Promise<HealthResponse> {
  return apiRequest<HealthResponse>('/api/health');
}

// ---------------------------------------------------------------------------
// 考试项目 Exam Projects
// ---------------------------------------------------------------------------

export function listProjects(): Promise<ExamProject[]> {
  return apiRequest<ExamProject[]>('/api/projects');
}

export function getProject(id: string): Promise<ExamProject> {
  return apiRequest<ExamProject>(`/api/projects/${id}`);
}

export function createProject(input: ExamProjectInput): Promise<ExamProject> {
  return apiRequest<ExamProject>('/api/projects', {
    method: 'POST',
    body: JSON.stringify(input),
  });
}

export function updateProject(id: string, input: ExamProjectInput): Promise<ExamProject> {
  return apiRequest<ExamProject>(`/api/projects/${id}`, {
    method: 'PUT',
    body: JSON.stringify(input),
  });
}

export function deleteProject(id: string): Promise<OkResponse> {
  return apiRequest<OkResponse>(`/api/projects/${id}`, { method: 'DELETE' });
}

// ---------------------------------------------------------------------------
// 标准字段 Standard Fields
// ---------------------------------------------------------------------------

export function listStandardFields(): Promise<StandardField[]> {
  return apiRequest<StandardField[]>('/api/standard-fields');
}

export function createStandardField(input: StandardFieldInput): Promise<StandardField> {
  return apiRequest<StandardField>('/api/standard-fields', {
    method: 'POST',
    body: JSON.stringify(input),
  });
}

export function updateStandardField(id: string, input: StandardFieldInput): Promise<StandardField> {
  return apiRequest<StandardField>(`/api/standard-fields/${id}`, {
    method: 'PUT',
    body: JSON.stringify(input),
  });
}

export function deleteStandardField(id: string): Promise<OkResponse> {
  return apiRequest<OkResponse>(`/api/standard-fields/${id}`, { method: 'DELETE' });
}

// ---------------------------------------------------------------------------
// 字段分组 Field Groups
// ---------------------------------------------------------------------------

export function listFieldGroups(): Promise<FieldGroup[]> {
  return apiRequest<FieldGroup[]>('/api/field-groups');
}

export function createFieldGroup(input: FieldGroupInput): Promise<FieldGroup> {
  return apiRequest<FieldGroup>('/api/field-groups', {
    method: 'POST',
    body: JSON.stringify(input),
  });
}

export function updateFieldGroup(id: string, input: FieldGroupInput): Promise<FieldGroup> {
  return apiRequest<FieldGroup>(`/api/field-groups/${id}`, {
    method: 'PUT',
    body: JSON.stringify(input),
  });
}

export function deleteFieldGroup(id: string): Promise<OkResponse> {
  return apiRequest<OkResponse>(`/api/field-groups/${id}`, { method: 'DELETE' });
}

// ---------------------------------------------------------------------------
// 别名映射 Aliases
// ---------------------------------------------------------------------------

export function listAliases(): Promise<AliasMapping[]> {
  return apiRequest<AliasMapping[]>('/api/aliases');
}

export function saveAliases(mappings: AliasMappingInput[]): Promise<SavedResponse> {
  return apiRequest<SavedResponse>('/api/aliases', {
    method: 'POST',
    body: JSON.stringify(mappings),
  });
}

export function deleteAlias(id: string): Promise<OkResponse> {
  return apiRequest<OkResponse>(`/api/aliases/${id}`, { method: 'DELETE' });
}

export function suggestAliases(columns: string[]): Promise<ColumnSuggestion[]> {
  return apiRequest<ColumnSuggestion[]>('/api/aliases/suggest', {
    method: 'POST',
    body: JSON.stringify({ columns }),
  });
}

// ---------------------------------------------------------------------------
// 数据集 Datasets
// ---------------------------------------------------------------------------

export function listDatasets(projectId?: string): Promise<Dataset[]> {
  const query = projectId ? `?project_id=${encodeURIComponent(projectId)}` : '';
  return apiRequest<Dataset[]>(`/api/datasets${query}`);
}

export function getDataset(id: string): Promise<Dataset> {
  return apiRequest<Dataset>(`/api/datasets/${id}`);
}

export function getDatasetRows(id: string, offset = 0, limit = 50): Promise<RowPage> {
  const params = new URLSearchParams({ offset: String(offset), limit: String(limit) });
  return apiRequest<RowPage>(`/api/datasets/${id}/rows?${params.toString()}`);
}

export function deleteDataset(id: string): Promise<OkResponse> {
  return apiRequest<OkResponse>(`/api/datasets/${id}`, { method: 'DELETE' });
}

export function importDataset(
  file: File,
  name?: string,
  headerRow?: number,
  projectId?: string,
): Promise<ImportResult> {
  const form = new FormData();
  form.append('file', file);
  if (name) {
    form.append('name', name);
  }
  if (headerRow != undefined) {
    form.append('header_row', String(headerRow));
  }
  if (projectId) {
    form.append('project_id', projectId);
  }
  return apiRequest<ImportResult>('/api/datasets/import', {
    method: 'POST',
    body: form,
    // 大文件解析 + 建表可能持续数十秒，使用长任务超时。
    timeoutMs: LONG_TIMEOUT_MS,
  });
}

export function applyMapping(
  id: string,
  mappings: { from: string; to: string }[],
  persistAliases: boolean,
  keepFields: string[] = [],
): Promise<Dataset> {
  return apiRequest<Dataset>(`/api/datasets/${id}/apply_mapping`, {
    method: 'POST',
    body: JSON.stringify({ mappings, persist_aliases: persistAliases, keep_fields: keepFields }),
  });
}

/**
 * 触发浏览器下载。默认以 POST 提交 JSON body；导出 CSV 等 GET 端点可传入 'GET'。
 */
export async function downloadFile(
  path: string,
  body: unknown,
  filename: string,
  method: 'GET' | 'POST' = 'POST',
): Promise<void> {
  // 全量导出可能生成较大的文件，使用长任务超时。
  const init: ApiRequestOptions = { method, timeoutMs: LONG_TIMEOUT_MS };
  if (method === 'POST') {
    init.body = JSON.stringify(body ?? {});
  }
  const blob = await apiRequestBlob(path, init);
  triggerBrowserDownload(blob, filename);
}

// ---------------------------------------------------------------------------
// 算子 Operators
// ---------------------------------------------------------------------------

export function listOperators(): Promise<OperatorDef[]> {
  return apiRequest<OperatorDef[]>('/api/operators');
}

// ---------------------------------------------------------------------------
// 处理流程 Flows
// ---------------------------------------------------------------------------

export interface ScopeQuery {
  projectId?: string;
  scope?: 'global' | 'project';
}

function scopeQueryString(q: ScopeQuery): string {
  const params = new URLSearchParams();
  if (q.projectId) params.set('project_id', q.projectId);
  if (q.scope) params.set('scope', q.scope);
  const s = params.toString();
  return s ? `?${s}` : '';
}

export function listFlows(q: ScopeQuery = {}): Promise<Flow[]> {
  return apiRequest<Flow[]>(`/api/flows${scopeQueryString(q)}`);
}

export function createFlow(input: FlowInput): Promise<Flow> {
  return apiRequest<Flow>('/api/flows', {
    method: 'POST',
    body: JSON.stringify(input),
  });
}

export function updateFlow(id: string, input: FlowInput): Promise<Flow> {
  return apiRequest<Flow>(`/api/flows/${id}`, {
    method: 'PUT',
    body: JSON.stringify(input),
  });
}

export function duplicateFlow(id: string, input: DuplicateInput): Promise<Flow> {
  return apiRequest<Flow>(`/api/flows/${id}/duplicate`, {
    method: 'POST',
    body: JSON.stringify(input),
  });
}

export function deleteFlow(id: string): Promise<OkResponse> {
  return apiRequest<OkResponse>(`/api/flows/${id}`, { method: 'DELETE' });
}

export function runFlow(id: string, datasetId: string): Promise<RunResult> {
  return apiRequest<RunResult>(`/api/flows/${id}/run`, {
    method: 'POST',
    body: JSON.stringify({ dataset_id: datasetId }),
    // 流程会在整张数据集上执行 SQL，数据量大时需要更长超时。
    timeoutMs: LONG_TIMEOUT_MS,
  });
}

export function validateFlow(steps: FlowStep[], columns: string[]): Promise<FlowValidateResult> {
  return apiRequest<FlowValidateResult>('/api/flows/validate', {
    method: 'POST',
    body: JSON.stringify({ steps, columns }),
  });
}

export function getFlowRuns(id: string): Promise<FlowRun[]> {
  return apiRequest<FlowRun[]>(`/api/flows/${id}/runs`);
}

// ---------------------------------------------------------------------------
// PDF 模板 Pdf Templates
// ---------------------------------------------------------------------------

export function listPdfTemplates(q: ScopeQuery = {}): Promise<PdfTemplate[]> {
  return apiRequest<PdfTemplate[]>(`/api/pdf-templates${scopeQueryString(q)}`);
}

export function createPdfTemplate(input: PdfTemplateInput): Promise<PdfTemplate> {
  return apiRequest<PdfTemplate>('/api/pdf-templates', {
    method: 'POST',
    body: JSON.stringify(input),
  });
}

export function updatePdfTemplate(id: string, input: PdfTemplateInput): Promise<PdfTemplate> {
  return apiRequest<PdfTemplate>(`/api/pdf-templates/${id}`, {
    method: 'PUT',
    body: JSON.stringify(input),
  });
}

export function duplicatePdfTemplate(id: string, input: DuplicateInput): Promise<PdfTemplate> {
  return apiRequest<PdfTemplate>(`/api/pdf-templates/${id}/duplicate`, {
    method: 'POST',
    body: JSON.stringify(input),
  });
}

export function deletePdfTemplate(id: string): Promise<OkResponse> {
  return apiRequest<OkResponse>(`/api/pdf-templates/${id}`, { method: 'DELETE' });
}

export function renderPdfTemplate(id: string, datasetId: string, limit?: number, page?: number): Promise<Blob> {
  return apiRequestBlob(`/api/pdf-templates/${id}/render`, {
    method: 'POST',
    body: JSON.stringify({ dataset_id: datasetId, limit, page }),
    timeoutMs: LONG_TIMEOUT_MS,
  });
}

export function previewPdfTemplate(template: PdfTemplateInput, datasetId: string, limit?: number): Promise<Blob> {
  return apiRequestBlob('/api/pdf-templates/preview', {
    method: 'POST',
    body: JSON.stringify({ template, dataset_id: datasetId, limit }),
    timeoutMs: LONG_TIMEOUT_MS,
  });
}
