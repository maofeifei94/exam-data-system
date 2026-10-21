use anyhow::Result;
use rusqlite::Connection;

/// 建库语句。所有数据集（导入的、流程产出的）都共用这两张元数据表，
/// 物理数据表统一命名为 `data_<id>`，物理列名固定为 c0、c1、c2 ...
/// 这样列改名只是元数据操作，不受 SQLite 标识符限制。
pub const SCHEMA: &str = r#"
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS exam_projects (
    id             TEXT PRIMARY KEY,
    name           TEXT NOT NULL,
    description    TEXT NOT NULL DEFAULT '',
    folder         TEXT NOT NULL DEFAULT '',
    config_file    TEXT NOT NULL DEFAULT '',
    flow_id        TEXT NOT NULL DEFAULT '',
    pdf_template_id TEXT NOT NULL DEFAULT '',
    dataset_id     TEXT NOT NULL DEFAULT '',
    created_at     TEXT NOT NULL,
    updated_at     TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS datasets (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    kind        TEXT NOT NULL DEFAULT 'import',
    source_file TEXT NOT NULL DEFAULT '',
    origin      TEXT NOT NULL DEFAULT '',
    project_id  TEXT NOT NULL DEFAULT '',
    row_count   INTEGER NOT NULL DEFAULT 0,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS dataset_columns (
    dataset_id  TEXT NOT NULL,
    idx         INTEGER NOT NULL,
    name        TEXT NOT NULL,
    dtype       TEXT NOT NULL DEFAULT 'text',
    PRIMARY KEY (dataset_id, idx)
);

-- 字段分组：字段别名页按分组展示卡片，避免一屏几十张卡片混在一起
CREATE TABLE IF NOT EXISTS field_groups (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    sort_order  INTEGER NOT NULL DEFAULT 0,
    created_at  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS standard_fields (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL UNIQUE,
    label       TEXT NOT NULL DEFAULT '',
    dtype       TEXT NOT NULL DEFAULT 'text',
    precision   INTEGER NOT NULL DEFAULT 2,
    group_id    TEXT NOT NULL DEFAULT '',
    description TEXT NOT NULL DEFAULT '',
    system      INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS alias_mappings (
    id          TEXT PRIMARY KEY,
    alias       TEXT NOT NULL UNIQUE,
    standard    TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS flows (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    steps       TEXT NOT NULL DEFAULT '[]',
    scope       TEXT NOT NULL DEFAULT 'global',
    project_id  TEXT NOT NULL DEFAULT '',
    source_id   TEXT NOT NULL DEFAULT '',
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS flow_runs (
    id                  TEXT PRIMARY KEY,
    flow_id             TEXT NOT NULL,
    dataset_id          TEXT NOT NULL,
    output_dataset_id   TEXT NOT NULL DEFAULT '',
    status              TEXT NOT NULL,
    message             TEXT NOT NULL DEFAULT '',
    sql_text            TEXT NOT NULL DEFAULT '',
    created_at          TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS pdf_templates (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    kind        TEXT NOT NULL DEFAULT 'single',
    description TEXT NOT NULL DEFAULT '',
    template    TEXT NOT NULL DEFAULT '{}',
    scope       TEXT NOT NULL DEFAULT 'global',
    project_id  TEXT NOT NULL DEFAULT '',
    source_id   TEXT NOT NULL DEFAULT '',
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);
"#;

pub fn new_id() -> String {
    // 16 位十六进制，足够短且不会与 SQLite 保留字冲突
    uuid::Uuid::new_v4().simple().to_string()[..16].to_string()
}

pub fn now() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

pub fn table_name(dataset_id: &str) -> String {
    format!("data_{dataset_id}")
}

pub fn open(path: &str) -> Result<Connection> {
    let conn = Connection::open(path)?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;
    Ok(conn)
}

pub fn init(conn: &Connection) -> Result<()> {
    conn.execute_batch(SCHEMA)?;
    migrate(conn)?;
    seed(conn)?;
    Ok(())
}

/// 兼容旧库：为已存在的表补上新增列（CREATE TABLE IF NOT EXISTS 不会改动老表结构）。
fn migrate(conn: &Connection) -> Result<()> {
    let columns: &[(&str, &str, &str)] = &[
        ("datasets", "project_id", "TEXT NOT NULL DEFAULT ''"),
        ("exam_projects", "dataset_id", "TEXT NOT NULL DEFAULT ''"),
        ("standard_fields", "precision", "INTEGER NOT NULL DEFAULT 2"),
        ("standard_fields", "group_id", "TEXT NOT NULL DEFAULT ''"),
        ("flows", "scope", "TEXT NOT NULL DEFAULT 'global'"),
        ("flows", "project_id", "TEXT NOT NULL DEFAULT ''"),
        ("flows", "source_id", "TEXT NOT NULL DEFAULT ''"),
        ("pdf_templates", "scope", "TEXT NOT NULL DEFAULT 'global'"),
        ("pdf_templates", "project_id", "TEXT NOT NULL DEFAULT ''"),
        ("pdf_templates", "source_id", "TEXT NOT NULL DEFAULT ''"),
    ];
    for (table, col, decl) in columns {
        ensure_column(conn, table, col, decl)?;
    }
    Ok(())
}

fn ensure_column(conn: &Connection, table: &str, col: &str, decl: &str) -> Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let names: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    if !names.iter().any(|n| n == col) {
        conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {col} {decl};"))?;
    }
    Ok(())
}

/// 初始化标准字段字典 / 别名映射 / 示例流程 / 示例 PDF 模板。
/// 全部使用 INSERT OR IGNORE，用户可以自由修改而不会被覆盖。
fn seed(conn: &Connection) -> Result<()> {
    // 字段分组：让「字段别名」页按业务域分区展示
    let groups: &[(&str, &str, i64)] = &[
        ("grp_identity", "身份信息", 10),
        ("grp_score", "成绩", 20),
        ("grp_exam", "考务安排", 30),
    ];
    for (id, name, sort_order) in groups {
        conn.execute(
            "INSERT OR IGNORE INTO field_groups (id, name, sort_order, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![id, name, sort_order, now()],
        )?;
    }

    // (标准名, 显示标签, 数据类型, 小数精度, 所属分组, 描述)
    let fields: &[(&str, &str, &str, i64, &str, &str)] = &[
        ("身份证号", "身份证号", "text", 0, "grp_identity", "考生唯一标识，必填"),
        ("姓名", "姓名", "text", 0, "grp_identity", "考生姓名"),
        ("准考证号", "准考证号", "text", 0, "grp_identity", "准考证编号"),
        ("性别", "性别", "text", 0, "grp_identity", ""),
        ("手机号", "手机号", "text", 0, "grp_identity", ""),
        ("照片", "照片", "image", 0, "grp_identity", "考生照片文件路径或文件名"),
        ("出生日期", "出生日期", "date", 0, "grp_identity", ""),
        ("学历", "学历", "text", 0, "grp_identity", ""),
        ("毕业院校", "毕业院校", "text", 0, "grp_identity", ""),
        ("专业", "专业", "text", 0, "grp_identity", ""),
        ("笔试成绩", "笔试成绩", "decimal", 2, "grp_score", "笔试原始分"),
        ("面试成绩", "面试成绩", "decimal", 2, "grp_score", "面试原始分"),
        ("总成绩", "总成绩", "decimal", 2, "grp_score", "加权后的综合成绩"),
        ("排名", "排名", "integer", 0, "grp_score", "按总成绩排名"),
        ("报考岗位", "报考岗位", "text", 0, "grp_exam", "招聘岗位名称"),
        ("考场", "考场", "text", 0, "grp_exam", "考场编号"),
        ("座位号", "座位号", "integer", 0, "grp_exam", "考场内座位号"),
        ("体检结果", "体检结果", "text", 0, "grp_exam", ""),
    ];
    for (name, label, dtype, precision, group_id, desc) in fields {
        conn.execute(
            "INSERT OR IGNORE INTO standard_fields
               (id, name, label, dtype, precision, group_id, description, system)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1)",
            rusqlite::params![new_id(), name, label, dtype, precision, group_id, desc],
        )?;
    }

    let aliases: &[(&str, &str)] = &[
        ("szf", "身份证号"),
        ("sfz", "身份证号"),
        ("sfzh", "身份证号"),
        ("身份证", "身份证号"),
        ("身份证号码", "身份证号"),
        ("证件号码", "身份证号"),
        ("证件号", "身份证号"),
        ("公民身份号码", "身份证号"),
        ("xm", "姓名"),
        ("考生姓名", "姓名"),
        ("zkzh", "准考证号"),
        ("准考证", "准考证号"),
        ("笔试", "笔试成绩"),
        ("笔试分", "笔试成绩"),
        ("bs", "笔试成绩"),
        ("笔试分数", "笔试成绩"),
        ("面试", "面试成绩"),
        ("面试分", "面试成绩"),
        ("ms", "面试成绩"),
        ("面试分数", "面试成绩"),
        ("总分", "总成绩"),
        ("综合成绩", "总成绩"),
        ("总成绩", "总成绩"),
        ("名次", "排名"),
        ("名次排序", "排名"),
        ("报考单位", "报考岗位"),
        ("岗位", "报考岗位"),
        ("招聘岗位", "报考岗位"),
        ("联系电话", "手机号"),
        ("电话", "手机号"),
        ("相片", "照片"),
        ("报名照片", "照片"),
    ];
    for (alias, standard) in aliases {
        conn.execute(
            "INSERT OR IGNORE INTO alias_mappings (id, alias, standard, updated_at) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![new_id(), alias, standard, now()],
        )?;
    }

    let steps = serde_json::json!([
        { "id": "s_require", "operator": "require_fields", "label": "校验必要字段",
          "enabled": true,
          "params": { "fields": ["身份证号", "笔试成绩", "面试成绩"] } },
        { "id": "s_unique", "operator": "unique_check", "label": "身份证号唯一性校验",
          "enabled": true, "params": { "columns": ["身份证号"], "on_error": "fail" } },
        { "id": "s_total", "operator": "compute", "label": "加权计算总成绩",
          "enabled": true,
          "params": { "out": "总成绩", "expr": "[笔试成绩]*0.6 + [面试成绩]*0.4", "decimals": 2 } },
        { "id": "s_rank", "operator": "rank", "label": "按总成绩排名",
          "enabled": true,
          "params": { "by": "总成绩", "order": "desc", "out": "排名", "method": "rank",
                      "partition_by": [] } },
        { "id": "s_sort", "operator": "sort", "label": "按总成绩降序排列",
          "enabled": true, "params": { "by": "总成绩", "order": "desc" } },
        { "id": "s_out", "operator": "output", "label": "输出结果表",
          "enabled": true, "params": { "name": "成绩排名结果" } }
    ]);
    conn.execute(
        "INSERT OR IGNORE INTO flows (id, name, description, steps, created_at, updated_at)
         VALUES ('flow_score_rank', '成绩加权排名（标准流程）',
                 '导入成绩后：校验必填字段 → 计算加权总成绩 → 排名 → 输出结果表。可直接复用于每次考试。',
                 ?1, ?2, ?2)",
        rusqlite::params![steps.to_string(), now()],
    )?;

    // 示例：5 行 × 6 列 的考生座次矩阵模板
    let matrix_tpl = serde_json::json!({
        "page": { "size": "A4", "orientation": "portrait",
                  "margin_mm": { "top": 15, "right": 12, "bottom": 12, "left": 12 } },
        "matrix": { "rows": 5, "cols": 6, "gap_x_mm": 2, "gap_y_mm": 2, "cell_h_mm": 42,
                    "origin_y_mm": 12 },
        "elements": [
            { "id": "e_title", "type": "text", "x_mm": 0, "y_mm": 0, "w_mm": 186, "h_mm": 9,
              "text": "第 {{page}} 考场座次表（本页 {{count}} 人）", "field": "", "font_size": 14,
              "bold": true, "align": "center", "color": "#000000", "fill": "", "border": false,
              "border_radius_mm": 0, "page_level": true },
            { "id": "e_box", "type": "rect", "x_mm": 0, "y_mm": 0, "w_mm": 30, "h_mm": 42,
              "text": "", "field": "", "font_size": 10, "bold": false, "align": "left",
              "color": "#000000", "fill": "#FFFFFF", "border": true, "border_radius_mm": 1 },
            { "id": "e_photo", "type": "image", "x_mm": 2, "y_mm": 2, "w_mm": 16, "h_mm": 20,
              "text": "", "field": "照片", "font_size": 10, "bold": false, "align": "center",
              "color": "#000000", "fill": "", "border": false, "border_radius_mm": 0 },
            { "id": "e_seat", "type": "text", "x_mm": 20, "y_mm": 2, "w_mm": 8, "h_mm": 8,
              "text": "{{index}}", "field": "", "font_size": 12, "bold": true, "align": "center",
              "color": "#000000", "fill": "", "border": false, "border_radius_mm": 0 },
            { "id": "e_name", "type": "text", "x_mm": 2, "y_mm": 24, "w_mm": 26, "h_mm": 6,
              "text": "{姓名}", "field": "姓名", "font_size": 11, "bold": true, "align": "center",
              "color": "#000000", "fill": "", "border": false, "border_radius_mm": 0 },
            { "id": "e_id", "type": "text", "x_mm": 2, "y_mm": 30, "w_mm": 26, "h_mm": 5,
              "text": "{准考证号}", "field": "准考证号", "font_size": 9, "bold": false, "align": "center",
              "color": "#333333", "fill": "", "border": false, "border_radius_mm": 0 },
            { "id": "e_post", "type": "text", "x_mm": 2, "y_mm": 35, "w_mm": 26, "h_mm": 5,
              "text": "{报考岗位}", "field": "报考岗位", "font_size": 9, "bold": false, "align": "center",
              "color": "#333333", "fill": "", "border": false, "border_radius_mm": 0 }
        ]
    });
    conn.execute(
        "INSERT OR IGNORE INTO pdf_templates (id, name, kind, description, template, created_at, updated_at)
         VALUES ('tpl_seat_matrix', '考生座次表（5×6 矩阵）', 'matrix',
                 '每页 5 行 × 6 列，共 30 个座位，单元格内显示照片、姓名、准考证号、报考岗位。',
                 ?1, ?2, ?2)",
        rusqlite::params![matrix_tpl.to_string(), now()],
    )?;

    let single_tpl = serde_json::json!({
        "page": { "size": "A4", "orientation": "portrait",
                  "margin_mm": { "top": 20, "right": 20, "bottom": 20, "left": 20 } },
        "matrix": { "rows": 5, "cols": 6, "gap_x_mm": 2, "gap_y_mm": 2, "cell_h_mm": 42 },
        "elements": [
            { "id": "e_t", "type": "text", "x_mm": 0, "y_mm": 0, "w_mm": 170, "h_mm": 12,
              "text": "考生成绩单", "field": "", "font_size": 22, "bold": true, "align": "center",
              "color": "#000000", "fill": "", "border": false, "border_radius_mm": 0 },
            { "id": "e_photo", "type": "image", "x_mm": 128, "y_mm": 20, "w_mm": 35, "h_mm": 45,
              "text": "", "field": "照片", "font_size": 10, "bold": false, "align": "center",
              "color": "#000000", "fill": "", "border": true, "border_radius_mm": 0 },
            { "id": "e_l1", "type": "text", "x_mm": 0, "y_mm": 22, "w_mm": 60, "h_mm": 7,
              "text": "姓名：{姓名}", "field": "", "font_size": 13, "bold": false, "align": "left",
              "color": "#000000", "fill": "", "border": false, "border_radius_mm": 0 },
            { "id": "e_l2", "type": "text", "x_mm": 0, "y_mm": 32, "w_mm": 90, "h_mm": 7,
              "text": "身份证号：{身份证号}", "field": "", "font_size": 13, "bold": false, "align": "left",
              "color": "#000000", "fill": "", "border": false, "border_radius_mm": 0 },
            { "id": "e_l3", "type": "text", "x_mm": 0, "y_mm": 42, "w_mm": 90, "h_mm": 7,
              "text": "准考证号：{准考证号}", "field": "", "font_size": 13, "bold": false, "align": "left",
              "color": "#000000", "fill": "", "border": false, "border_radius_mm": 0 },
            { "id": "e_l4", "type": "text", "x_mm": 0, "y_mm": 54, "w_mm": 90, "h_mm": 7,
              "text": "报考岗位：{报考岗位}", "field": "", "font_size": 13, "bold": false, "align": "left",
              "color": "#000000", "fill": "", "border": false, "border_radius_mm": 0 },
            { "id": "e_l5", "type": "text", "x_mm": 0, "y_mm": 70, "w_mm": 90, "h_mm": 7,
              "text": "笔试成绩：{笔试成绩}", "field": "", "font_size": 13, "bold": false, "align": "left",
              "color": "#000000", "fill": "", "border": false, "border_radius_mm": 0 },
            { "id": "e_l6", "type": "text", "x_mm": 0, "y_mm": 80, "w_mm": 90, "h_mm": 7,
              "text": "面试成绩：{面试成绩}", "field": "", "font_size": 13, "bold": false, "align": "left",
              "color": "#000000", "fill": "", "border": false, "border_radius_mm": 0 },
            { "id": "e_l7", "type": "text", "x_mm": 0, "y_mm": 90, "w_mm": 90, "h_mm": 8,
              "text": "总成绩：{总成绩}    排名：{排名}", "field": "", "font_size": 15, "bold": true,
              "align": "left", "color": "#000000", "fill": "", "border": false, "border_radius_mm": 0 }
        ]
    });
    conn.execute(
        "INSERT OR IGNORE INTO pdf_templates (id, name, kind, description, template, created_at, updated_at)
         VALUES ('tpl_score_single', '考生成绩单（单人单页）', 'single',
                 '每行数据生成一页成绩单，包含照片与各项成绩。',
                 ?1, ?2, ?2)",
        rusqlite::params![single_tpl.to_string(), now()],
    )?;

    Ok(())
}
