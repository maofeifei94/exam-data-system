//! 考试项目接口。
//!
//! 项目是「数据集 + 流程 + PDF 模板」的绑定容器，同时在项目文件夹里
//! 落一份 `exam-project.json`，方便用户直接查看/备份项目配置。
//! 数据库记录是唯一事实来源，配置文件是可再生的投影。

use crate::db::{self, new_id, now};
use crate::error::{AppError, AppResult};
use crate::models::{ExamProject, ExamProjectInput};
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::Json;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;

/// 每个考试项目在指定文件夹下写出一个配置文件，记录它引用哪套数据处理流程与 PDF 模板。
pub const CONFIG_FILE_NAME: &str = "exam-project.json";

const PROJ_COLS: &str = "id, name, description, folder, config_file, flow_id, pdf_template_id, dataset_id, created_at, updated_at";

fn row_to_project(r: &rusqlite::Row<'_>) -> rusqlite::Result<ExamProject> {
    Ok(ExamProject {
        id: r.get(0)?,
        name: r.get(1)?,
        description: r.get(2)?,
        folder: r.get(3)?,
        config_file: r.get(4)?,
        flow_id: r.get(5)?,
        pdf_template_id: r.get(6)?,
        dataset_id: r.get(7)?,
        created_at: r.get(8)?,
        updated_at: r.get(9)?,
    })
}

fn fetch_project(conn: &Connection, id: &str) -> AppResult<ExamProject> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {PROJ_COLS} FROM exam_projects WHERE id = ?1"
    ))?;
    stmt.query_row(params![id], row_to_project)
        .optional()?
        .ok_or_else(|| AppError::not_found("考试项目不存在"))
}

#[derive(Debug, Deserialize, Serialize)]
struct ConfigFile {
    id: String,
    name: String,
    description: String,
    flow_id: String,
    pdf_template_id: String,
    dataset_id: String,
    updated_at: String,
}

/// 把项目配置写到项目文件夹下的 exam-project.json；返回配置文件路径。folder 为空则跳过。
fn write_config(project: &ExamProject) -> AppResult<String> {
    let folder = project.folder.trim();
    if folder.is_empty() {
        return Ok(String::new());
    }
    let dir = PathBuf::from(folder);
    std::fs::create_dir_all(&dir)
        .map_err(|e| AppError::bad_request(format!("无法创建项目文件夹 {folder}: {e}")))?;
    let path = dir.join(CONFIG_FILE_NAME);
    let content = ConfigFile {
        id: project.id.clone(),
        name: project.name.clone(),
        description: project.description.clone(),
        flow_id: project.flow_id.clone(),
        pdf_template_id: project.pdf_template_id.clone(),
        dataset_id: project.dataset_id.clone(),
        updated_at: project.updated_at.clone(),
    };
    let text = serde_json::to_string_pretty(&content)?;
    std::fs::write(&path, text)
        .map_err(|e| AppError::bad_request(format!("写入项目配置文件失败: {e}")))?;
    Ok(path.to_string_lossy().to_string())
}

/// 名称校验：空名称无法在列表中区分，过长名称会撑坏界面布局。
fn validate_name(raw: &str) -> AppResult<String> {
    let name = raw.trim().to_string();
    if name.is_empty() {
        return Err(AppError::validation("考试项目名称不能为空"));
    }
    if name.chars().count() > 120 {
        return Err(AppError::validation("考试项目名称不能超过 120 个字符"));
    }
    Ok(name)
}

pub async fn list(State(state): State<Arc<AppState>>) -> AppResult<Json<Vec<ExamProject>>> {
    let projects = state
        .db(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {PROJ_COLS} FROM exam_projects ORDER BY updated_at DESC, id ASC"
            ))?;
            let rows = stmt.query_map([], row_to_project)?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
        })
        .await?;
    Ok(Json(projects))
}

pub async fn get_one(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> AppResult<Json<ExamProject>> {
    let project = state.db(move |conn| fetch_project(conn, &id)).await?;
    Ok(Json(project))
}

pub async fn create(
    State(state): State<Arc<AppState>>,
    Json(input): Json<ExamProjectInput>,
) -> AppResult<Json<ExamProject>> {
    let name = validate_name(&input.name)?;
    let project = ExamProject {
        id: new_id(),
        name,
        description: input.description,
        folder: input.folder.trim().to_string(),
        config_file: String::new(),
        flow_id: input.flow_id,
        pdf_template_id: input.pdf_template_id,
        dataset_id: input.dataset_id,
        created_at: now(),
        updated_at: String::new(),
    };
    // 写成时间与 created_at 一致，避免两次取时间出现毫秒级差异。
    let project = ExamProject {
        updated_at: project.created_at.clone(),
        ..project
    };

    let saved = state
        .db(move |conn| {
            db::with_tx(conn, |tx| {
                tx.execute(
                    "INSERT INTO exam_projects
                       (id, name, description, folder, config_file, flow_id, pdf_template_id, dataset_id, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, '', ?5, ?6, ?7, ?8, ?9)",
                    params![
                        project.id,
                        project.name,
                        project.description,
                        project.folder,
                        project.flow_id,
                        project.pdf_template_id,
                        project.dataset_id,
                        project.created_at,
                        project.updated_at
                    ],
                )?;
                // 配置文件写入失败时整个事务回滚，不会留下「配置指向不存在项目」的脏数据。
                let mut saved = fetch_project(tx, &project.id)?;
                saved.config_file = write_config(&saved)?;
                tx.execute(
                    "UPDATE exam_projects SET config_file = ?1 WHERE id = ?2",
                    params![saved.config_file, saved.id],
                )?;
                Ok(saved)
            })
        })
        .await?;
    Ok(Json(saved))
}

pub async fn update(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(input): Json<ExamProjectInput>,
) -> AppResult<Json<ExamProject>> {
    let name = validate_name(&input.name)?;

    let saved = state
        .db(move |conn| {
            db::with_tx(conn, |tx| {
                let mut project = fetch_project(tx, &id)?;
                project.name = name;
                project.description = input.description;
                project.folder = input.folder.trim().to_string();
                project.flow_id = input.flow_id;
                project.pdf_template_id = input.pdf_template_id;
                project.dataset_id = input.dataset_id;
                project.updated_at = now();
                project.config_file = write_config(&project)?;

                tx.execute(
                    "UPDATE exam_projects
                     SET name = ?1, description = ?2, folder = ?3, config_file = ?4, flow_id = ?5,
                         pdf_template_id = ?6, dataset_id = ?7, updated_at = ?8
                     WHERE id = ?9",
                    params![
                        project.name,
                        project.description,
                        project.folder,
                        project.config_file,
                        project.flow_id,
                        project.pdf_template_id,
                        project.dataset_id,
                        project.updated_at,
                        project.id
                    ],
                )?;
                fetch_project(tx, &project.id)
            })
        })
        .await?;
    Ok(Json(saved))
}

pub async fn remove(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    // 事务内完成「解除数据集归属 + 删除项目」，避免中途失败留下半删除状态。
    let config_file = state
        .db(move |conn| {
            db::with_tx(conn, |tx| {
                let project = fetch_project(tx, &id)?;
                tx.execute(
                    "UPDATE datasets SET project_id = '', updated_at = ?1 WHERE project_id = ?2",
                    params![now(), id],
                )?;
                tx.execute("DELETE FROM exam_projects WHERE id = ?1", params![id])?;
                Ok(project.config_file)
            })
        })
        .await?;

    // 数据库已提交后再清理配置文件：失败也不影响数据一致性。
    if !config_file.trim().is_empty() {
        let _ = std::fs::remove_file(FsPath::new(&config_file));
    }
    Ok(Json(json!({ "ok": true })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_project(folder: &str) -> ExamProject {
        ExamProject {
            id: "p1".to_string(),
            name: "2026 春季考试".to_string(),
            description: "描述".to_string(),
            folder: folder.to_string(),
            config_file: String::new(),
            flow_id: "f1".to_string(),
            pdf_template_id: "t1".to_string(),
            dataset_id: "d1".to_string(),
            created_at: "2026-01-01 00:00:00".to_string(),
            updated_at: "2026-01-01 00:00:00".to_string(),
        }
    }

    #[test]
    fn project_name_is_trimmed_and_validated() {
        assert_eq!(validate_name("  考试  ").expect("valid"), "考试");
        assert_eq!(
            validate_name("   ").expect_err("empty").code,
            "validation_error"
        );
        let long = "名".repeat(121);
        assert_eq!(
            validate_name(&long).expect_err("too long").code,
            "validation_error"
        );
    }

    #[test]
    fn write_config_persists_project_binding() {
        let dir = std::env::temp_dir().join(format!("exam_proj_{}", new_id()));
        let project = sample_project(dir.to_string_lossy().as_ref());
        let path = write_config(&project).expect("write config");

        assert!(path.ends_with(CONFIG_FILE_NAME));
        let text = std::fs::read_to_string(&path).expect("read config");
        let parsed: ConfigFile = serde_json::from_str(&text).expect("valid json");
        assert_eq!(parsed.id, "p1");
        assert_eq!(parsed.flow_id, "f1");
        assert_eq!(parsed.pdf_template_id, "t1");
        assert_eq!(parsed.dataset_id, "d1");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_folder_skips_config_file() {
        let project = sample_project("   ");
        assert_eq!(write_config(&project).expect("skip"), "");
    }
}
