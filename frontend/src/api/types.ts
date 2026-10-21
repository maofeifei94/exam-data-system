// 考试数据处理系统 —— 后端 API 类型定义
// 对应后端 base http://127.0.0.1:8710, 前缀 /api

/** 字段数据类型，参考 Excel 的数值/日期分类：整数与小数分开，小数带精度 */
export type ColumnDtype =
  | 'text'
  | 'integer'
  | 'decimal'
  | 'percent'
  | 'date'
  | 'datetime'
  | 'boolean'
  | 'image';

/** 需要「精度（保留小数位）」的类型 */
export const PRECISION_DTYPES: ColumnDtype[] = ['decimal', 'percent'];

/** 兼容后端历史数据里把数值统一记成 number 的情况 */
export function normalizeDtype(dtype: string | undefined): ColumnDtype {
  if (!dtype || dtype === 'number') {
    return dtype === 'number' ? 'decimal' : 'text';
  }
  return dtype as ColumnDtype;
}

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
  /** 项目流程「数据导入」节点绑定的数据集 */
  dataset_id: string;
  created_at: string;
  updated_at: string;
}

export interface ExamProjectInput {
  name: string;
  description?: string;
  folder?: string;
  flow_id?: string;
  pdf_template_id?: string;
  dataset_id?: string;
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
  /** 小数类型的精度（保留小数位） */
  precision: number;
  /** 所属字段分组；空字符串表示未分组 */
  group_id: string;
  description: string;
  system: boolean;
}

export interface StandardFieldInput {
  name: string;
  label: string;
  dtype: ColumnDtype;
  precision?: number;
  group_id?: string;
  description: string;
}

export interface FieldGroup {
  id: string;
  name: string;
  sort_order: number;
}

export interface FieldGroupInput {
  name: string;
  sort_order?: number;
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
  /** 可视分支组标识；为空表示主链步骤 */
  branch?: string;
  /** 该分支挂载在哪个步骤之后（步骤 id） */
  branch_of?: string;
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

export interface StepFlow {
  step_index: number;
  input_fields: string[];
  output_fields: string[];
  errors: string[];
}

export interface FlowValidateResult {
  ok: boolean;
  errors: { step_index: number; message: string }[];
  /** 每一步的数据流快照（进入/离开该步骤时的字段集合） */
  steps: StepFlow[];
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
  /** 编辑器中是否显示网格辅助线 */
  show_grid: boolean;
  /** 是否显示 / 输出单元格边框 */
  show_border: boolean;
}

export interface TemplateElement {
  id: string;
  type: 'text' | 'field' | 'image' | 'line' | 'rect' | 'matrix';
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
  /** 子元素：坐标相对父元素左上角；矩阵的子元素会复制到每个单元格 */
  children: TemplateElement[];
  /** type=matrix 时的网格配置 */
  matrix?: MatrixSetup;
  /** 兼容旧模板：整页元素标记，新结构不再使用 */
  page_level?: boolean;
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
