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
  | 'boolean';

/** 需要「精度（保留小数位）」的类型 */
export const PRECISION_DTYPES: ColumnDtype[] = ['decimal', 'percent'];

/**
 * 兼容后端历史数据里把数值统一记成 number 的情况。
 * 照片已不再是数据类型：历史 image 字段按文本处理。
 */
export function normalizeDtype(dtype: string | undefined): ColumnDtype {
  if (!dtype) return 'text';
  if (dtype === 'number') return 'decimal';
  if (dtype === 'image') return 'text';
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

/** 项目本地字段副本：从全局字段库复制而来，修改只影响本项目。 */
export interface ProjectField {
  id: string;
  project_id: string;
  name: string;
  label: string;
  dtype: ColumnDtype;
  precision: number;
  group_id: string;
  description: string;
  /** 复制来源的全局字段 id；手工新建时为空 */
  source_id: string;
  created_at: string;
  updated_at: string;
}

export interface ProjectFieldInput {
  project_id: string;
  name: string;
  label: string;
  dtype: ColumnDtype;
  precision?: number;
  group_id?: string;
  description: string;
  source_id?: string;
}

export interface ProjectFieldUpdate {
  name: string;
  label: string;
  dtype: ColumnDtype;
  precision?: number;
  group_id?: string;
  description: string;
}

/** 项目本地别名：复制全局字段时一并复制，之后可在项目内独立增删。 */
export interface ProjectAlias {
  id: string;
  project_id: string;
  alias: string;
  standard: string;
  updated_at: string;
}

export interface ProjectAliasInput {
  project_id: string;
  alias: string;
  standard: string;
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

/**
 * 流程中某个节点的数据快照。
 * `step_index` 等于流程步骤总数时表示整条流程的最终输出。
 */
export interface FlowStepData extends RowPage {
  step_index: number;
  label: string;
  operator: string;
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
    | 'mappings'
    | 'sorts'
    | 'aggregations'
    | 'dataset'
    | 'dataset_column'
    | 'dataset_fields';
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
  size: 'A4' | 'A3' | 'A5' | 'Letter' | 'Custom';
  orientation: 'portrait' | 'landscape';
  margin_mm: { top: number; right: number; bottom: number; left: number };
  /** 自定义纸张宽度（mm），size=Custom 时生效 */
  width_mm?: number;
  /** 自定义纸张高度（mm），size=Custom 时生效 */
  height_mm?: number;
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

/** 明细表列：绑定字段并设置表头、列宽与对齐。 */
export interface TableColumn {
  field: string;
  label: string;
  /** 列宽（mm），0 表示按剩余宽度均分 */
  width_mm: number;
  align: 'left' | 'center' | 'right';
  /** 汇总方式：留空不汇总，可选 sum / avg / count */
  summary?: string;
}

/** 明细表配置：数据按行高自动分页，每页重复表头。 */
export interface TableSetup {
  columns: TableColumn[];
  header_height_mm: number;
  row_height_mm: number;
  show_header: boolean;
  show_grid: boolean;
  /** 汇总行首列文字 */
  summary_label?: string;
  /** 单元格文字自动换行 */
  wrap?: boolean;
}

export interface TemplateElement {
  id: string;
  type: 'text' | 'field' | 'image' | 'qrcode' | 'barcode' | 'line' | 'rect' | 'matrix' | 'table';
  x_mm: number;
  y_mm: number;
  w_mm: number;
  h_mm: number;
  text: string;
  field: string;
  font_size: number;
  bold: boolean;
  /** 文本超出元素宽度时自动换行 */
  wrap?: boolean;
  /** 文本旋转角度：0 / 90 / 180 / 270 */
  rotate?: number;
  /** 渲染内容为空时隐藏该元素 */
  hide_if_empty?: boolean;
  align: 'left' | 'center' | 'right';
  color: string;
  fill: string;
  border: boolean;
  border_radius_mm: number;
  /** 子元素：坐标相对父元素左上角；矩阵的子元素会复制到每个单元格 */
  children: TemplateElement[];
  /** type=matrix 时的网格配置 */
  matrix?: MatrixSetup;
  /** type=table 时的明细表配置 */
  table?: TableSetup;
  /** 兼容旧模板：整页元素标记，新结构不再使用 */
  page_level?: boolean;
  /** type=image：照片所在目录，空则回退到渲染请求的 photo_dir */
  photo_dir?: string;
  /** type=image：照片名模板，支持 `{{字段}}` / 固定字符串组合 */
  photo_name?: string;
  /**
   * type=image：候选后缀（不带点），按顺序查找。
   * 例如 ['jpg', 'png'] 会依次尝试 `名称.jpg`、`名称.png`。
   */
  photo_exts?: string[];
  /** type=image：填充方式，stretch 拉伸铺满 / contain 等比缩放适应 */
  photo_fit?: 'stretch' | 'contain';
}

export interface PdfTemplate {
  id: string;
  name: string;
  kind: 'single' | 'matrix' | 'table';
  description: string;
  page: PageSetup;
  matrix: MatrixSetup;
  elements: TemplateElement[];
  /** 页面分组/排序依据（如考场号）：每个取值一页，页面按取值排序 */
  page_field: string;
  /** 渲染前排序字段与方向 */
  sort_field: string;
  sort_order: 'asc' | 'desc';
  /** 矩阵格子索引依据（如座位号）：取值 N 放入第 N 个格子 */
  matrix_index_field: string;
  /** 模板显式声明的必需字段 */
  required_fields: string[];
  /** 显式声明 + 子元素占位符合并后的完整必需字段 */
  all_required_fields: string[];
  /** 页面背景图路径，整页铺满并保持比例 */
  background_image: string;
  /** 页眉文字，支持 {字段} 占位符 */
  header_text: string;
  /** 页脚文字，支持 {字段} 占位符 */
  footer_text: string;
  /** 页眉 / 页脚对齐 */
  header_align: 'left' | 'center' | 'right';
  footer_align: 'left' | 'center' | 'right';
  /** 页眉页脚字号（pt） */
  chrome_font_size: number;
  /** 页眉页脚颜色 */
  chrome_color: string;
  /** 水印文字，为空表示不添加 */
  watermark: string;
  /** 水印平铺整页（默认单个居中水印） */
  watermark_tile: boolean;
  /** 页脚显示「第 X 页 / 共 Y 页」 */
  show_page_number: boolean;
  /** 页面边框 */
  page_border: boolean;
  page_border_width_mm: number;
  page_border_color: string;
  /** 页码起始值 */
  page_number_start: number;
  /** 首页隐藏页码 */
  page_number_skip_first: boolean;
  scope: Scope;
  project_id: string;
  source_id: string;
  created_at: string;
  updated_at: string;
}

export interface PdfTemplateInput {
  name: string;
  kind: 'single' | 'matrix' | 'table';
  description: string;
  page: PageSetup;
  matrix: MatrixSetup;
  elements: TemplateElement[];
  page_field?: string;
  sort_field?: string;
  sort_order?: 'asc' | 'desc';
  matrix_index_field?: string;
  required_fields?: string[];
  background_image?: string;
  header_text?: string;
  footer_text?: string;
  header_align?: 'left' | 'center' | 'right';
  footer_align?: 'left' | 'center' | 'right';
  chrome_font_size?: number;
  chrome_color?: string;
  watermark?: string;
  watermark_tile?: boolean;
  show_page_number?: boolean;
  page_border?: boolean;
  page_border_width_mm?: number;
  page_border_color?: string;
  page_number_start?: number;
  page_number_skip_first?: boolean;
  scope?: Scope;
  project_id?: string;
}

export interface SortKey {
  field: string;
  order: 'asc' | 'desc';
}

export interface AggregationSpec {
  func: string;
  field?: string;
  out?: string;
  decimals?: number;
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
