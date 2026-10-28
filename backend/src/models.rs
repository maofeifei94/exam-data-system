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

/// 流程中某个节点的数据快照。
///
/// `step_index` 为 0 基下标：执行完该下标对应的步骤后的数据；
/// 等于步骤总数时表示整条流程的最终输出（含排序 / 截断）。
#[derive(Debug, Serialize)]
pub struct StepData {
    pub step_index: i64,
    pub label: String,
    pub operator: String,
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
    /// 只保留这些字段（来自流程「必需字段校验」），其余列从数据集中剔除
    #[serde(default)]
    pub keep_fields: Vec<String>,
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

// ---------------- 项目本地字段 ----------------

/// 某个考试项目自己的字段副本。与全局 `standard_fields` 完全独立，
/// 修改只影响本项目，不会回写全局字段库。
#[derive(Debug, Clone, Serialize)]
pub struct ProjectField {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub label: String,
    pub dtype: String,
    pub precision: i64,
    pub group_id: String,
    pub description: String,
    /// 从全局字段复制时的来源字段 id；手工新建时为空
    pub source_id: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
pub struct ProjectFieldInput {
    pub project_id: String,
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
    #[serde(default)]
    pub source_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ProjectFieldUpdate {
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

/// 把选中的全局字段复制到项目：按全局字段 id 挑选。
#[derive(Debug, Deserialize)]
pub struct CopyProjectFieldsInput {
    pub project_id: String,
    #[serde(default)]
    pub field_ids: Vec<String>,
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
    /// 可视分支组标识；为空表示主链步骤
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub branch: String,
    /// 该分支挂载在哪个步骤之后（步骤 id）
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub branch_of: String,
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

/// 单个步骤的数据流快照：进入该步骤时可用字段、执行后字段、该步骤的问题。
#[derive(Debug, Serialize)]
pub struct StepFlow {
    pub step_index: i64,
    pub input_fields: Vec<String>,
    pub output_fields: Vec<String>,
    pub errors: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ValidateResult {
    pub ok: bool,
    pub errors: Vec<ValidateError>,
    pub steps: Vec<StepFlow>,
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
    Margins {
        top: 15.0,
        right: 12.0,
        bottom: 12.0,
        left: 12.0,
    }
}
fn default_matrix() -> MatrixSetup {
    MatrixSetup {
        rows: 5,
        cols: 6,
        gap_x_mm: 2.0,
        gap_y_mm: 2.0,
        cell_h_mm: 42.0,
        origin_y_mm: 0.0,
        show_grid: true,
        show_border: true,
    }
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
    /// 编辑器中是否显示网格辅助线（不影响 PDF 内容）
    #[serde(default = "default_true")]
    pub show_grid: bool,
    /// 是否输出单元格边框（同时影响编辑器预览与 PDF）
    #[serde(default = "default_true")]
    pub show_border: bool,
}
impl Default for MatrixSetup {
    fn default() -> Self {
        Self {
            rows: default_matrix_rows(),
            cols: default_matrix_cols(),
            gap_x_mm: 0.0,
            gap_y_mm: 0.0,
            cell_h_mm: default_cell_h(),
            origin_y_mm: 0.0,
            show_grid: true,
            show_border: true,
        }
    }
}
fn default_true() -> bool {
    true
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
    /// 子元素：坐标相对父元素左上角；矩阵的子元素会被复制到每个单元格
    #[serde(default)]
    pub children: Vec<TemplateElement>,
    /// type = "matrix" 时的网格配置
    #[serde(default)]
    pub matrix: Option<MatrixSetup>,
    /// 兼容旧模板：true 表示整页元素（每页只画一次）
    #[serde(default)]
    pub page_level: bool,
    /// type = "image" 时：照片所在目录；为空时回退到渲染请求的 photo_dir
    #[serde(default)]
    pub photo_dir: String,
    /// type = "image" 时：照片名模板，支持 `{字段}` 占位符与固定字符串组合
    #[serde(default)]
    pub photo_name: String,
    /// type = "image" 时：按顺序尝试的候选后缀（不带点，如 jpg、png）
    #[serde(default)]
    pub photo_exts: Vec<String>,
}

impl TemplateElement {
    pub(crate) fn empty(id: &str, etype: &str) -> Self {
        TemplateElement {
            id: id.to_string(),
            etype: etype.to_string(),
            x_mm: 0.0,
            y_mm: 0.0,
            w_mm: 0.0,
            h_mm: 0.0,
            text: String::new(),
            field: String::new(),
            font_size: default_font_size(),
            bold: false,
            align: default_align(),
            color: default_color(),
            fill: String::new(),
            border: false,
            border_radius_mm: 0.0,
            children: Vec::new(),
            matrix: None,
            page_level: false,
            photo_dir: String::new(),
            photo_name: String::new(),
            photo_exts: Vec::new(),
        }
    }
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

/// 递归迁移照片元素：旧模板把照片文件名存在 `field`（绑定单个数据列），
/// 新结构改为 `photo_name`（支持 `{字段}` 占位符的组合模板）。
///
/// 这里不主动删除 `field`，保留原值便于旧客户端回读。
fn migrate_photo_elements(elements: &mut [TemplateElement]) {
    for el in elements.iter_mut() {
        if el.etype == "image" && el.photo_name.trim().is_empty() && !el.field.trim().is_empty() {
            el.photo_name = format!("{{{}}}", el.field.trim());
        }
        migrate_photo_elements(&mut el.children);
    }
}

impl TemplateBody {
    /// 把旧版「扁平元素 + 页面级矩阵」模板迁移为嵌套元素结构：
    /// 页面元素保持在根，单元格元素挂到新建的 matrix 元素下。
    pub fn normalize(&mut self, kind: &str) {
        // 照片元素迁移：旧模板用 field 绑定单个字段，新结构改为「名称模板 + 后缀」。
        migrate_photo_elements(&mut self.elements);
        if self.elements.iter().any(|e| e.etype == "matrix") {
            return;
        }
        if kind != "matrix" {
            return;
        }
        let (pw, ph) = match self.page.size.as_str() {
            "A3" => (297.0, 420.0),
            "A5" => (148.0, 210.0),
            "Letter" => (215.9, 279.4),
            _ => (210.0, 297.0),
        };
        let page_w = if self.page.orientation == "landscape" {
            ph
        } else {
            pw
        };
        let content_w = (page_w - self.page.margin_mm.left - self.page.margin_mm.right).max(10.0);
        let mx = self.matrix.clone();
        let cols = mx.cols.max(1) as f32;
        let rows = mx.rows.max(1) as f32;
        let gap_x = mx.gap_x_mm.max(0.0);
        let gap_y = mx.gap_y_mm.max(0.0);
        let cell_w = ((content_w - gap_x * (cols - 1.0)) / cols).max(1.0);
        let cell_h = if mx.cell_h_mm > 0.0 {
            mx.cell_h_mm
        } else {
            30.0
        };
        let matrix_w = cell_w * cols + gap_x * (cols - 1.0);
        let matrix_h = cell_h * rows + gap_y * (rows - 1.0);

        let old = std::mem::take(&mut self.elements);
        let mut page_elements = Vec::new();
        let mut cell_elements = Vec::new();
        for mut el in old {
            let was_page_level = el.page_level;
            el.page_level = false;
            el.children.clear();
            if was_page_level {
                page_elements.push(el);
            } else {
                cell_elements.push(el);
            }
        }

        let mut matrix_el = TemplateElement::empty("matrix", "matrix");
        matrix_el.x_mm = 0.0;
        matrix_el.y_mm = mx.origin_y_mm;
        matrix_el.w_mm = matrix_w;
        matrix_el.h_mm = matrix_h;
        matrix_el.matrix = Some(mx);
        matrix_el.children = cell_elements;
        page_elements.push(matrix_el);
        self.elements = page_elements;
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
