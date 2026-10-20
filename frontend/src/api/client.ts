import type {
  AliasMapping,
  AliasMappingInput,
  ColumnSuggestion,
  Dataset,
  DuplicateInput,
  ExamProject,
  ExamProjectInput,
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
const BASE_URL: string = window.examapi?.baseUrl ?? '';

async function parseError(res: Response): Promise<Error> {
  let message = `请求失败 (HTTP ${res.status})`;
  try {
    const data = await res.json();
    if (data && typeof data.error === 'string') {
      message = data.error;
    }
  } catch {
    // 响应不是 JSON，保留默认信息
  }
  return new Error(message);
}

async function request<T>(path: string, init: RequestInit = {}): Promise<T> {
  const headers = new Headers(init.headers);
  if (init.body && !(init.body instanceof FormData) && !headers.has('Content-Type')) {
    headers.set('Content-Type', 'application/json');
  }
  const res = await fetch(`${BASE_URL}${path}`, { ...init, headers });
  if (!res.ok) {
    throw await parseError(res);
  }
  if (res.status === 204) {
    return undefined as T;
  }
  return (await res.json()) as T;
}

async function requestBlob(path: string, init: RequestInit = {}): Promise<Blob> {
  const headers = new Headers(init.headers);
  if (init.body && !(init.body instanceof FormData) && !headers.has('Content-Type')) {
    headers.set('Content-Type', 'application/json');
  }
  const res = await fetch(`${BASE_URL}${path}`, { ...init, headers });
  if (!res.ok) {
    throw await parseError(res);
  }
  return await res.blob();
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
  return request<HealthResponse>('/api/health');
}

// ---------------------------------------------------------------------------
// 考试项目 Exam Projects
// ---------------------------------------------------------------------------

export function listProjects(): Promise<ExamProject[]> {
  return request<ExamProject[]>('/api/projects');
}

export function getProject(id: string): Promise<ExamProject> {
  return request<ExamProject>(`/api/projects/${id}`);
}

export function createProject(input: ExamProjectInput): Promise<ExamProject> {
  return request<ExamProject>('/api/projects', {
    method: 'POST',
    body: JSON.stringify(input),
  });
}

export function updateProject(id: string, input: ExamProjectInput): Promise<ExamProject> {
  return request<ExamProject>(`/api/projects/${id}`, {
    method: 'PUT',
    body: JSON.stringify(input),
  });
}

export function deleteProject(id: string): Promise<OkResponse> {
  return request<OkResponse>(`/api/projects/${id}`, { method: 'DELETE' });
}

// ---------------------------------------------------------------------------
// 标准字段 Standard Fields
// ---------------------------------------------------------------------------

export function listStandardFields(): Promise<StandardField[]> {
  return request<StandardField[]>('/api/standard-fields');
}

export function createStandardField(input: StandardFieldInput): Promise<StandardField> {
  return request<StandardField>('/api/standard-fields', {
    method: 'POST',
    body: JSON.stringify(input),
  });
}

export function updateStandardField(id: string, input: StandardFieldInput): Promise<StandardField> {
  return request<StandardField>(`/api/standard-fields/${id}`, {
    method: 'PUT',
    body: JSON.stringify(input),
  });
}

export function deleteStandardField(id: string): Promise<OkResponse> {
  return request<OkResponse>(`/api/standard-fields/${id}`, { method: 'DELETE' });
}

// ---------------------------------------------------------------------------
// 别名映射 Aliases
// ---------------------------------------------------------------------------

export function listAliases(): Promise<AliasMapping[]> {
  return request<AliasMapping[]>('/api/aliases');
}

export function saveAliases(mappings: AliasMappingInput[]): Promise<SavedResponse> {
  return request<SavedResponse>('/api/aliases', {
    method: 'POST',
    body: JSON.stringify(mappings),
  });
}

export function deleteAlias(id: string): Promise<OkResponse> {
  return request<OkResponse>(`/api/aliases/${id}`, { method: 'DELETE' });
}

export function suggestAliases(columns: string[]): Promise<ColumnSuggestion[]> {
  return request<ColumnSuggestion[]>('/api/aliases/suggest', {
    method: 'POST',
    body: JSON.stringify({ columns }),
  });
}

// ---------------------------------------------------------------------------
// 数据集 Datasets
// ---------------------------------------------------------------------------

export function listDatasets(projectId?: string): Promise<Dataset[]> {
  const query = projectId ? `?project_id=${encodeURIComponent(projectId)}` : '';
  return request<Dataset[]>(`/api/datasets${query}`);
}

export function getDataset(id: string): Promise<Dataset> {
  return request<Dataset>(`/api/datasets/${id}`);
}

export function getDatasetRows(id: string, offset = 0, limit = 50): Promise<RowPage> {
  const params = new URLSearchParams({ offset: String(offset), limit: String(limit) });
  return request<RowPage>(`/api/datasets/${id}/rows?${params.toString()}`);
}

export function deleteDataset(id: string): Promise<OkResponse> {
  return request<OkResponse>(`/api/datasets/${id}`, { method: 'DELETE' });
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
  return request<ImportResult>('/api/datasets/import', {
    method: 'POST',
    body: form,
  });
}

export function applyMapping(
  id: string,
  mappings: { from: string; to: string }[],
  persistAliases: boolean,
): Promise<Dataset> {
  return request<Dataset>(`/api/datasets/${id}/apply_mapping`, {
    method: 'POST',
    body: JSON.stringify({ mappings, persist_aliases: persistAliases }),
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
  const init: RequestInit = { method };
  if (method === 'POST') {
    init.body = JSON.stringify(body ?? {});
  }
  const blob = await requestBlob(path, init);
  triggerBrowserDownload(blob, filename);
}

// ---------------------------------------------------------------------------
// 算子 Operators
// ---------------------------------------------------------------------------

export function listOperators(): Promise<OperatorDef[]> {
  return request<OperatorDef[]>('/api/operators');
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
  return request<Flow[]>(`/api/flows${scopeQueryString(q)}`);
}

export function createFlow(input: FlowInput): Promise<Flow> {
  return request<Flow>('/api/flows', {
    method: 'POST',
    body: JSON.stringify(input),
  });
}

export function updateFlow(id: string, input: FlowInput): Promise<Flow> {
  return request<Flow>(`/api/flows/${id}`, {
    method: 'PUT',
    body: JSON.stringify(input),
  });
}

export function duplicateFlow(id: string, input: DuplicateInput): Promise<Flow> {
  return request<Flow>(`/api/flows/${id}/duplicate`, {
    method: 'POST',
    body: JSON.stringify(input),
  });
}

export function deleteFlow(id: string): Promise<OkResponse> {
  return request<OkResponse>(`/api/flows/${id}`, { method: 'DELETE' });
}

export function runFlow(id: string, datasetId: string): Promise<RunResult> {
  return request<RunResult>(`/api/flows/${id}/run`, {
    method: 'POST',
    body: JSON.stringify({ dataset_id: datasetId }),
  });
}

export function validateFlow(steps: FlowStep[], columns: string[]): Promise<FlowValidateResult> {
  return request<FlowValidateResult>('/api/flows/validate', {
    method: 'POST',
    body: JSON.stringify({ steps, columns }),
  });
}

export function getFlowRuns(id: string): Promise<FlowRun[]> {
  return request<FlowRun[]>(`/api/flows/${id}/runs`);
}

// ---------------------------------------------------------------------------
// PDF 模板 Pdf Templates
// ---------------------------------------------------------------------------

export function listPdfTemplates(q: ScopeQuery = {}): Promise<PdfTemplate[]> {
  return request<PdfTemplate[]>(`/api/pdf-templates${scopeQueryString(q)}`);
}

export function createPdfTemplate(input: PdfTemplateInput): Promise<PdfTemplate> {
  return request<PdfTemplate>('/api/pdf-templates', {
    method: 'POST',
    body: JSON.stringify(input),
  });
}

export function updatePdfTemplate(id: string, input: PdfTemplateInput): Promise<PdfTemplate> {
  return request<PdfTemplate>(`/api/pdf-templates/${id}`, {
    method: 'PUT',
    body: JSON.stringify(input),
  });
}

export function duplicatePdfTemplate(id: string, input: DuplicateInput): Promise<PdfTemplate> {
  return request<PdfTemplate>(`/api/pdf-templates/${id}/duplicate`, {
    method: 'POST',
    body: JSON.stringify(input),
  });
}

export function deletePdfTemplate(id: string): Promise<OkResponse> {
  return request<OkResponse>(`/api/pdf-templates/${id}`, { method: 'DELETE' });
}

export function renderPdfTemplate(id: string, datasetId: string, limit?: number, page?: number): Promise<Blob> {
  return requestBlob(`/api/pdf-templates/${id}/render`, {
    method: 'POST',
    body: JSON.stringify({ dataset_id: datasetId, limit, page }),
  });
}

export function previewPdfTemplate(template: PdfTemplateInput, datasetId: string, limit?: number): Promise<Blob> {
  return requestBlob('/api/pdf-templates/preview', {
    method: 'POST',
    body: JSON.stringify({ template, dataset_id: datasetId, limit }),
  });
}
