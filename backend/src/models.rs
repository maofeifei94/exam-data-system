use serde::{Deserialize, Serialize};

fn t() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatasetColumn {
    pub name: String,
    pub dtype: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Dataset {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub source_file: String,
    pub origin: String,
    pub project_id: String,
    pub row_count: i64,
    pub columns: Vec<DatasetColumn>,
    pub created_at: String,
    pub updated_at: String,
}

// ---------------- 考试项目 ----------------

#[derive(Debug, Clone, Serialize)]
pub struct ExamProject {
    pub id: String,
    pub name: String,
    pub description: String,
    pub folder: String,
    pub config_file: String,
    pub flow_id: String,
    pub pdf_template_id: String,
    /// 项目流程「数据导入」节点绑定的数据集
    pub dataset_id: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
pub struct ExamProjectInput {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub folder: String,
    #[serde(default)]
    pub flow_id: String,
    #[serde(default)]
    pub pdf_template_id: String,
    #[serde(default)]
    pub dataset_id: String,
}

#[derive(Debug, Serialize)]
pub struct RowPage {
    pub columns: Vec<DatasetColumn>,
    pub rows: Vec<Vec<serde_json::Value>>,
    pub total: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ColumnSuggestion {
    pub source: String,
    pub standard: Option<String>,
    pub matched_by: String,
}

#[derive(Debug, Serialize)]
pub struct ImportResult {
    pub dataset: Dataset,
    pub mapping: Vec<ColumnSuggestion>,
    pub preview: RowPage,
}

#[derive(Debug, Deserialize)]
pub struct ApplyMappingInput {
    #[serde(default)]
    pub mappings: Vec<MappingPair>,
    #[serde(default)]
    pub persist_aliases: bool,
}

#[derive(Debug, Deserialize)]
pub struct MappingPair {
    pub from: String,
    pub to: String,
}

// ---------------- 标准字段 / 别名 ----------------

#[derive(Debug, Clone, Serialize)]
pub struct StandardField {
    pub id: String,
    pub name: String,
    pub label: String,
    pub dtype: String,
    /// 小数类型的精度（保留小数位）；非小数类型该值无意义
    pub precision: i64,
    pub group_id: String,
    pub description: String,
    pub system: bool,
}

#[derive(Debug, Deserialize)]
pub struct StandardFieldInput {
    pub name: String,
    #[serde(default)]
    pub label: String,
    #[serde(default = "default_dtype")]
    pub dtype: String,
    #[serde(default)]
    pub precision: Option<i64>,
    #[serde(default)]
    pub group_id: Option<String>,
    #[serde(default)]
    pub description: String,
}

fn default_dtype() -> String {
    "text".to_string()
}

#[derive(Debug, Clone, Serialize)]
pub struct FieldGroup {
    pub id: String,
    pub name: String,
    pub sort_order: i64,
}

#[derive(Debug, Deserialize)]
pub struct FieldGroupInput {
    pub name: String,
    #[serde(default)]
    pub sort_order: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AliasMapping {
    pub id: String,
    pub alias: String,
    pub standard: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
pub struct AliasInput {
    pub alias: String,
    pub standard: String,
}

#[derive(Debug, Deserialize)]
pub struct SuggestInput {
    #[serde(default)]
    pub columns: Vec<String>,
}

// ---------------- 流程 ----------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepPos {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowStep {
    pub id: String,
    pub operator: String,
    #[serde(default)]
    pub label: String,
    #[serde(default = "t")]
    pub enabled: bool,
    #[serde(default)]
    pub params: serde_json::Value,
    /// 流程图节点在画布上的位置；为空时前端按顺序自动排列。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pos: Option<StepPos>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Flow {
    pub id: String,
    pub name: String,
    pub description: String,
    pub steps: Vec<FlowStep>,
    pub scope: String,
    pub project_id: String,
    pub source_id: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
pub struct FlowInput {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub steps: Vec<FlowStep>,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DuplicateInput {
    pub project_id: String,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RunFlowInput {
    pub dataset_id: String,
}

#[derive(Debug, Serialize)]
pub struct RunLogEntry {
    pub step_index: i64,
    pub operator: String,
    pub label: String,
    pub status: String,
    pub rows: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sql: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RunResult {
    pub run_id: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub log: Vec<RunLogEntry>,
    pub output_dataset_id: Option<String>,
    pub output: Option<RowPage>,
}

#[derive(Debug, Serialize)]
pub struct FlowRun {
    pub id: String,
    pub flow_id: String,
    pub dataset_id: String,
    pub output_dataset_id: String,
    pub status: String,
    pub message: String,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
pub struct ValidateFlowInput {
    #[serde(default)]
    pub steps: Vec<FlowStep>,
    #[serde(default)]
    pub columns: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ValidateError {
    pub step_index: i64,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct ValidateResult {
    pub ok: bool,
    pub errors: Vec<ValidateError>,
}

// ---------------- 操作符目录 ----------------

#[derive(Debug, Serialize)]
pub struct OperatorParam {
    pub key: String,
    pub label: String,
    #[serde(rename = "type")]
    pub ptype: String,
    pub required: bool,
    pub default: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct OperatorDef {
    #[serde(rename = "type")]
    pub otype: String,
    pub label: String,
    pub group: String,
    pub description: String,
    pub params: Vec<OperatorParam>,
}

// ---------------- PDF 模板 ----------------

fn default_page_size() -> String {
    "A4".to_string()
}
fn default_orientation() -> String {
    "portrait".to_string()
}
fn default_margin() -> Margins {
    Margins { top: 15.0, right: 12.0, bottom: 12.0, left: 12.0 }
}
fn default_matrix() -> MatrixSetup {
    MatrixSetup { rows: 5, cols: 6, gap_x_mm: 2.0, gap_y_mm: 2.0, cell_h_mm: 42.0, origin_y_mm: 0.0 }
}
fn default_font_size() -> f32 {
    11.0
}
fn default_align() -> String {
    "left".to_string()
}
fn default_color() -> String {
    "#000000".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Margins {
    #[serde(default = "default_m_top")]
    pub top: f32,
    #[serde(default = "default_m_right")]
    pub right: f32,
    #[serde(default = "default_m_bottom")]
    pub bottom: f32,
    #[serde(default = "default_m_left")]
    pub left: f32,
}
fn default_m_top() -> f32 {
    15.0
}
fn default_m_right() -> f32 {
    12.0
}
fn default_m_bottom() -> f32 {
    12.0
}
fn default_m_left() -> f32 {
    12.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageSetup {
    #[serde(default = "default_page_size")]
    pub size: String,
    #[serde(default = "default_orientation")]
    pub orientation: String,
    #[serde(default = "default_margin")]
    pub margin_mm: Margins,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatrixSetup {
    #[serde(default = "default_matrix_rows")]
    pub rows: i64,
    #[serde(default = "default_matrix_cols")]
    pub cols: i64,
    #[serde(default = "default_gap")]
    pub gap_x_mm: f32,
    #[serde(default = "default_gap")]
    pub gap_y_mm: f32,
    #[serde(default = "default_cell_h")]
    pub cell_h_mm: f32,
    /// 矩阵网格距内容区顶部的偏移（用于给页面级标题留位置）
    #[serde(default)]
    pub origin_y_mm: f32,
}
fn default_matrix_rows() -> i64 {
    5
}
fn default_matrix_cols() -> i64 {
    6
}
fn default_gap() -> f32 {
    2.0
}
fn default_cell_h() -> f32 {
    42.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateElement {
    pub id: String,
    #[serde(rename = "type")]
    pub etype: String,
    #[serde(default)]
    pub x_mm: f32,
    #[serde(default)]
    pub y_mm: f32,
    #[serde(default)]
    pub w_mm: f32,
    #[serde(default)]
    pub h_mm: f32,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub field: String,
    #[serde(default = "default_font_size")]
    pub font_size: f32,
    #[serde(default)]
    pub bold: bool,
    #[serde(default = "default_align")]
    pub align: String,
    #[serde(default = "default_color")]
    pub color: String,
    #[serde(default)]
    pub fill: String,
    #[serde(default)]
    pub border: bool,
    #[serde(default)]
    pub border_radius_mm: f32,
    /// 矩阵模板中：true 表示该元素属于整页（每页只画一次），坐标相对内容区左上角
    #[serde(default)]
    pub page_level: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateBody {
    #[serde(default = "default_page")]
    pub page: PageSetup,
    #[serde(default = "default_matrix")]
    pub matrix: MatrixSetup,
    #[serde(default)]
    pub elements: Vec<TemplateElement>,
}
fn default_page() -> PageSetup {
    PageSetup {
        size: default_page_size(),
        orientation: default_orientation(),
        margin_mm: default_margin(),
    }
}

impl Default for TemplateBody {
    fn default() -> Self {
        TemplateBody {
            page: default_page(),
            matrix: default_matrix(),
            elements: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PdfTemplate {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub description: String,
    pub page: PageSetup,
    pub matrix: MatrixSetup,
    pub elements: Vec<TemplateElement>,
    pub scope: String,
    pub project_id: String,
    pub source_id: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
pub struct PdfTemplateInput {
    pub name: String,
    #[serde(default = "default_kind")]
    pub kind: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_page")]
    pub page: PageSetup,
    #[serde(default = "default_matrix")]
    pub matrix: MatrixSetup,
    #[serde(default)]
    pub elements: Vec<TemplateElement>,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
}
fn default_kind() -> String {
    "single".to_string()
}

#[derive(Debug, Deserialize)]
pub struct RenderInput {
    pub dataset_id: String,
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub page: Option<i64>,
    #[serde(default)]
    pub photo_dir: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PreviewInput {
    pub template: PdfTemplateInput,
    pub dataset_id: String,
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub photo_dir: Option<String>,
}
