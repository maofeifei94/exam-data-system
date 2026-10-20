// 考试数据处理系统 —— 后端 API 类型定义
// 对应后端 base http://127.0.0.1:8710, 前缀 /api

export type ColumnDtype = 'text' | 'number' | 'date' | 'image';

export interface DatasetColumn {
  name: string;
  dtype: ColumnDtype;
}

export interface Dataset {
  id: string;
  name: string;
  kind: string;
  source_file: string;
  origin: string;
  project_id: string;
  row_count: number;
  columns: DatasetColumn[];
  created_at: string;
  updated_at: string;
}

// ---------------- 考试项目 ----------------

export interface ExamProject {
  id: string;
  name: string;
  description: string;
  folder: string;
  config_file: string;
  flow_id: string;
  pdf_template_id: string;
  created_at: string;
  updated_at: string;
}

export interface ExamProjectInput {
  name: string;
  description?: string;
  folder?: string;
  flow_id?: string;
  pdf_template_id?: string;
}

/** global = 全局库；project = 某考试项目的本地副本 */
export type Scope = 'global' | 'project';

export interface DuplicateInput {
  project_id: string;
  name?: string;
}

export interface StandardField {
  id: string;
  name: string;
  label: string;
  dtype: ColumnDtype;
  description: string;
  system: boolean;
}

export interface StandardFieldInput {
  name: string;
  label: string;
  dtype: ColumnDtype;
  description: string;
}

export interface AliasMapping {
  id: string;
  alias: string;
  standard: string;
  updated_at: string;
}

export interface AliasMappingInput {
  alias: string;
  standard: string;
}

export type MatchedBy = 'exact_standard' | 'alias' | 'none';

export interface ColumnSuggestion {
  source: string;
  standard: string | null;
  matched_by: MatchedBy;
}

export interface RowPage {
  columns: DatasetColumn[];
  rows: (string | number | null)[][];
  total: number;
}

export interface OperatorParam {
  key: string;
  label: string;
  type:
    | 'text'
    | 'number'
    | 'bool'
    | 'enum'
    | 'column'
    | 'columns'
    | 'expr'
    | 'fields'
    | 'mappings';
  required: boolean;
  default?: any;
  options?: string[];
  placeholder?: string;
  help?: string;
}

export interface OperatorDef {
  type: string;
  label: string;
  group: string;
  description: string;
  params: OperatorParam[];
}

export interface StepPos {
  x: number;
  y: number;
}

export interface FlowStep {
  id: string;
  operator: string;
  label: string;
  enabled: boolean;
  params: Record<string, any>;
  /** 流程图节点在画布上的坐标；缺省时前端自动排布 */
  pos?: StepPos;
}

export interface Flow {
  id: string;
  name: string;
  description: string;
  steps: FlowStep[];
  scope: Scope;
  project_id: string;
  source_id: string;
  created_at: string;
  updated_at: string;
}

export interface FlowInput {
  name: string;
  description: string;
  steps: FlowStep[];
  scope?: Scope;
  project_id?: string;
}

export interface RunLogEntry {
  step_index: number;
  operator: string;
  label: string;
  status: 'ok' | 'skipped' | 'failed';
  rows: number;
  sql?: string | null;
  message?: string | null;
}

export interface RunResult {
  run_id: string;
  status: 'success' | 'failed';
  message?: string | null;
  log: RunLogEntry[];
  output_dataset_id: string | null;
  output: RowPage | null;
}

export interface FlowRun {
  id: string;
  flow_id: string;
  dataset_id: string;
  output_dataset_id: string | null;
  status: string;
  created_at: string;
  message?: string | null;
}

export interface FlowValidateResult {
  ok: boolean;
  errors: { step_index: number; message: string }[];
}

export interface PageSetup {
  size: 'A4' | 'A3' | 'A5' | 'Letter';
  orientation: 'portrait' | 'landscape';
  margin_mm: { top: number; right: number; bottom: number; left: number };
}

export interface MatrixSetup {
  rows: number;
  cols: number;
  gap_x_mm: number;
  gap_y_mm: number;
  cell_h_mm: number;
  /** 矩阵网格距内容区顶部的偏移（用于给页面级标题留位置） */
  origin_y_mm: number;
}

export interface TemplateElement {
  id: string;
  type: 'text' | 'field' | 'image' | 'line' | 'rect';
  x_mm: number;
  y_mm: number;
  w_mm: number;
  h_mm: number;
  text: string;
  field: string;
  font_size: number;
  bold: boolean;
  align: 'left' | 'center' | 'right';
  color: string;
  fill: string;
  border: boolean;
  border_radius_mm: number;
  /** 矩阵模板中：true 表示该元素属于整页（每页只画一次），坐标相对内容区左上角 */
  page_level: boolean;
}

export interface PdfTemplate {
  id: string;
  name: string;
  kind: 'single' | 'matrix';
  description: string;
  page: PageSetup;
  matrix: MatrixSetup;
  elements: TemplateElement[];
  scope: Scope;
  project_id: string;
  source_id: string;
  created_at: string;
  updated_at: string;
}

export interface PdfTemplateInput {
  name: string;
  kind: 'single' | 'matrix';
  description: string;
  page: PageSetup;
  matrix: MatrixSetup;
  elements: TemplateElement[];
  scope?: Scope;
  project_id?: string;
}

export interface ImportResult {
  dataset: Dataset;
  mapping: ColumnSuggestion[];
  preview: RowPage;
}

export interface HealthResponse {
  ok: boolean;
  version: string;
}

export interface OkResponse {
  ok: boolean;
}

export interface SavedResponse {
  saved: number;
}
