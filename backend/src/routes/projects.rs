use crate::db::{new_id, now};
use crate::error::{AppError, AppResult};
use crate::models::{ExamProject, ExamProjectInput};
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::Json;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;

/// 每个考试项目在指定文件夹下写出一个配置文件，记录它引用哪套数据处理流程与 PDF 模板。
pub const CONFIG_FILE_NAME: &str = "exam-project.json";

const PROJ_COLS: &str = "id, name, description, folder, config_file, flow_id, pdf_template_id, created_at, updated_at";

fn row_to_project(r: &rusqlite::Row<'_>) -> rusqlite::Result<ExamProject> {
    Ok(ExamProject {
        id: r.get(0)?,
        name: r.get(1)?,
        description: r.get(2)?,
        folder: r.get(3)?,
        config_file: r.get(4)?,
        flow_id: r.get(5)?,
        pdf_template_id: r.get(6)?,
        created_at: r.get(7)?,
        updated_at: r.get(8)?,
    })
}

fn fetch_project(conn: &rusqlite::Connection, id: &str) -> AppResult<ExamProject> {
    let mut stmt = conn.prepare(&format!("SELECT {PROJ_COLS} FROM exam_projects WHERE id = ?1"))?;
    stmt.query_row(params![id], |r| row_to_project(r))
        .map_err(|_| AppError::not_found("考试项目不存在"))
}

#[derive(Debug, Deserialize, Serialize)]
struct ConfigFile {
    id: String,
    name: String,
    description: String,
    flow_id: String,
    pdf_template_id: String,
    updated_at: String,
}

/// 把项目配置写到项目文件夹下的 exam-project.json；返回配置文件路径。folder 为空则跳过。
fn write_config(project: &ExamProject) -> AppResult<String> {
    let folder = project.folder.trim();
    if folder.is_empty() {
        return Ok(String::new());
    }
    let dir: PathBuf = FsPath::new(folder).to_path_buf();
    std::fs::create_dir_all(&dir)
        .map_err(|e| AppError::bad_request(format!("无法创建项目文件夹 {folder}: {e}")))?;
    let path = dir.join(CONFIG_FILE_NAME);
    let content = ConfigFile {
        id: project.id.clone(),
        name: project.name.clone(),
        description: project.description.clone(),
        flow_id: project.flow_id.clone(),
        pdf_template_id: project.pdf_template_id.clone(),
        updated_at: project.updated_at.clone(),
    };
    let text = serde_json::to_string_pretty(&content)?;
    std::fs::write(&path, text)
        .map_err(|e| AppError::bad_request(format!("写入项目配置文件失败: {e}")))?;
    Ok(path.to_string_lossy().to_string())
}

pub async fn list(State(state): State<Arc<AppState>>) -> AppResult<Json<Vec<ExamProject>>> {
    let conn = state.lock()?;
    let mut stmt = conn.prepare(&format!("SELECT {PROJ_COLS} FROM exam_projects ORDER BY updated_at DESC"))?;
    let rows = stmt.query_map([], |r| row_to_project(r))?;
    Ok(Json(rows.collect::<Result<Vec<_>, _>>()?))
}

pub async fn get_one(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> AppResult<Json<ExamProject>> {
    let conn = state.lock()?;
    Ok(Json(fetch_project(&conn, &id)?))
}

pub async fn create(
    State(state): State<Arc<AppState>>,
    Json(input): Json<ExamProjectInput>,
) -> AppResult<Json<ExamProject>> {
    let name = input.name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::bad_request("考试项目名称不能为空"));
    }
    let id = new_id();
    let ts = now();
    let mut project = ExamProject {
        id,
        name,
        description: input.description,
        folder: input.folder.trim().to_string(),
        config_file: String::new(),
        flow_id: input.flow_id,
        pdf_template_id: input.pdf_template_id,
        created_at: ts.clone(),
        updated_at: ts,
    };
    // 先写配置文件，成功后再落库，避免出现配置写失败的脏记录
    project.config_file = write_config(&project)?;

    let conn = state.lock()?;
    conn.execute(
        "INSERT INTO exam_projects
           (id, name, description, folder, config_file, flow_id, pdf_template_id, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
        params![
            project.id,
            project.name,
            project.description,
            project.folder,
            project.config_file,
            project.flow_id,
            project.pdf_template_id,
            project.created_at
        ],
    )?;
    Ok(Json(fetch_project(&conn, &project.id)?))
}

pub async fn update(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(input): Json<ExamProjectInput>,
) -> AppResult<Json<ExamProject>> {
    let name = input.name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::bad_request("考试项目名称不能为空"));
    }
    let conn = state.lock()?;
    let mut project = fetch_project(&conn, &id)?;
    project.name = name;
    project.description = input.description;
    project.folder = input.folder.trim().to_string();
    project.flow_id = input.flow_id;
    project.pdf_template_id = input.pdf_template_id;
    project.updated_at = now();
    project.config_file = write_config(&project)?;

    conn.execute(
        "UPDATE exam_projects
         SET name = ?1, description = ?2, folder = ?3, config_file = ?4, flow_id = ?5, pdf_template_id = ?6, updated_at = ?7
         WHERE id = ?8",
        params![
            project.name,
            project.description,
            project.folder,
            project.config_file,
            project.flow_id,
            project.pdf_template_id,
            project.updated_at,
            id
        ],
    )?;
    Ok(Json(fetch_project(&conn, &id)?))
}

pub async fn remove(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    let conn = state.lock()?;
    // 删除项目时把其下数据集改为“未归属”，避免数据从项目视图里消失
    conn.execute("UPDATE datasets SET project_id = '' WHERE project_id = ?1", params![id])?;
    conn.execute("DELETE FROM exam_projects WHERE id = ?1", params![id])?;
    Ok(Json(json!({ "ok": true })))
}
