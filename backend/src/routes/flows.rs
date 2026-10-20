use crate::db::{new_id, now};
use crate::engine;
use crate::error::{AppError, AppResult};
use crate::models::{
    DuplicateInput, Flow, FlowInput, FlowRun, FlowStep, RunFlowInput, RunResult, ValidateFlowInput,
    ValidateResult,
};
use crate::state::AppState;
use crate::store;
use axum::extract::{Path, Query, State};
use axum::Json;
use rusqlite::params;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

const FLOW_COLS: &str =
    "id, name, description, steps, scope, project_id, source_id, created_at, updated_at";

fn row_to_flow(r: &rusqlite::Row<'_>) -> rusqlite::Result<Flow> {
    let steps_raw: String = r.get(3)?;
    Ok(Flow {
        id: r.get(0)?,
        name: r.get(1)?,
        description: r.get(2)?,
        steps: serde_json::from_str(&steps_raw).unwrap_or_default(),
        scope: r.get(4)?,
        project_id: r.get(5)?,
        source_id: r.get(6)?,
        created_at: r.get(7)?,
        updated_at: r.get(8)?,
    })
}

fn fetch_flow(conn: &rusqlite::Connection, id: &str) -> AppResult<Flow> {
    let mut stmt = conn.prepare(&format!("SELECT {FLOW_COLS} FROM flows WHERE id = ?1"))?;
    stmt.query_row(params![id], |r| row_to_flow(r))
        .map_err(|_| AppError::not_found("流程不存在"))
}

pub async fn operators() -> AppResult<Json<Vec<crate::models::OperatorDef>>> {
    Ok(Json(engine::operators::catalog()))
}

#[derive(Debug, Deserialize)]
pub struct FlowQuery {
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
}

/// 列表：`project_id` 指定则返回该项目的流程；否则 `scope=global` 返回全局库；都不传返回全部。
pub async fn list(
    State(state): State<Arc<AppState>>,
    Query(q): Query<FlowQuery>,
) -> AppResult<Json<Vec<Flow>>> {
    let conn = state.lock()?;
    let mut sql = format!("SELECT {FLOW_COLS} FROM flows");
    let mut binds: Vec<String> = Vec::new();
    if let Some(pid) = q.project_id.as_deref().filter(|s| !s.trim().is_empty()) {
        sql.push_str(" WHERE project_id = ?1");
        binds.push(pid.to_string());
    } else if let Some(sc) = q.scope.as_deref().filter(|s| !s.trim().is_empty()) {
        sql.push_str(" WHERE scope = ?1");
        binds.push(sc.to_string());
    }
    sql.push_str(" ORDER BY updated_at DESC");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(binds.iter()), |r| row_to_flow(r))?;
    Ok(Json(rows.collect::<Result<Vec<_>, _>>()?))
}

pub async fn get_one(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> AppResult<Json<Flow>> {
    let conn = state.lock()?;
    Ok(Json(fetch_flow(&conn, &id)?))
}

fn normalize_steps(steps: &[FlowStep]) -> Vec<FlowStep> {
    steps
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let mut s = s.clone();
            if s.id.trim().is_empty() {
                s.id = format!("step_{}", i + 1);
            }
            if s.params.is_null() {
                s.params = engine::operators::default_params(&s.operator);
            }
            if s.label.trim().is_empty() {
                s.label = engine::operators::label_of(&s.operator);
            }
            s
        })
        .collect()
}

/// 决定新流程的作用域：显式传入优先，否则有 project_id 即为项目级。
fn resolve_scope(scope: Option<&str>, project_id: &str) -> String {
    match scope.map(|s| s.trim()) {
        Some("project") if !project_id.is_empty() => "project".to_string(),
        Some("global") => "global".to_string(),
        _ if !project_id.is_empty() => "project".to_string(),
        _ => "global".to_string(),
    }
}

pub async fn create(
    State(state): State<Arc<AppState>>,
    Json(input): Json<FlowInput>,
) -> AppResult<Json<Flow>> {
    let name = input.name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::bad_request("流程名称不能为空"));
    }
    let steps = normalize_steps(&input.steps);
    let project_id = input.project_id.clone().unwrap_or_default();
    let scope = resolve_scope(input.scope.as_deref(), &project_id);
    let id = new_id();
    let ts = now();
    let conn = state.lock()?;
    conn.execute(
        "INSERT INTO flows (id, name, description, steps, scope, project_id, source_id, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, '', ?7, ?7)",
        params![
            id,
            name,
            input.description,
            serde_json::to_string(&steps)?,
            scope,
            project_id,
            ts
        ],
    )?;
    Ok(Json(fetch_flow(&conn, &id)?))
}

/// 把（通常是全局库里的）流程复制成为一个考试项目的本地副本，之后全局改动不影响本项目。
pub async fn duplicate(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(input): Json<DuplicateInput>,
) -> AppResult<Json<Flow>> {
    let project_id = input.project_id.trim().to_string();
    if project_id.is_empty() {
        return Err(AppError::bad_request("必须指定目标考试项目"));
    }
    let conn = state.lock()?;
    let src = fetch_flow(&conn, &id)?;
    let new = new_id();
    let name = input
        .name
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| format!("{}（本项目）", src.name));
    let ts = now();
    conn.execute(
        "INSERT INTO flows (id, name, description, steps, scope, project_id, source_id, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, 'project', ?5, ?6, ?7, ?7)",
        params![
            new,
            name,
            src.description,
            serde_json::to_string(&src.steps)?,
            project_id,
            id,
            ts
        ],
    )?;
    Ok(Json(fetch_flow(&conn, &new)?))
}

pub async fn update(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(input): Json<FlowInput>,
) -> AppResult<Json<Flow>> {
    let steps = normalize_steps(&input.steps);
    let conn = state.lock()?;
    let n = conn.execute(
        "UPDATE flows SET name = ?1, description = ?2, steps = ?3, updated_at = ?4 WHERE id = ?5",
        params![
            input.name.trim(),
            input.description,
            serde_json::to_string(&steps)?,
            now(),
            id
        ],
    )?;
    if n == 0 {
        return Err(AppError::not_found("流程不存在"));
    }
    Ok(Json(fetch_flow(&conn, &id)?))
}

pub async fn remove(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    let conn = state.lock()?;
    conn.execute("DELETE FROM flow_runs WHERE flow_id = ?1", params![id])?;
    conn.execute("DELETE FROM flows WHERE id = ?1", params![id])?;
    Ok(Json(json!({ "ok": true })))
}

pub async fn run(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(input): Json<RunFlowInput>,
) -> AppResult<Json<RunResult>> {
    let conn = state.lock()?;
    let _ = store::get_dataset(&conn, &input.dataset_id)?;
    Ok(Json(engine::run_flow(&conn, &id, &input.dataset_id)?))
}

pub async fn runs(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> AppResult<Json<Vec<FlowRun>>> {
    let conn = state.lock()?;
    let mut stmt = conn.prepare(
        "SELECT id, flow_id, dataset_id, output_dataset_id, status, message, created_at
         FROM flow_runs WHERE flow_id = ?1 ORDER BY created_at DESC LIMIT 50",
    )?;
    let rows = stmt.query_map(params![id], |r| {
        Ok(FlowRun {
            id: r.get(0)?,
            flow_id: r.get(1)?,
            dataset_id: r.get(2)?,
            output_dataset_id: r.get(3)?,
            status: r.get(4)?,
            message: r.get(5)?,
            created_at: r.get(6)?,
        })
    })?;
    Ok(Json(rows.collect::<Result<Vec<_>, _>>()?))
}

pub async fn validate(Json(input): Json<ValidateFlowInput>) -> AppResult<Json<ValidateResult>> {
    let steps = normalize_steps(&input.steps);
    Ok(Json(engine::validate_steps(&steps, &input.columns)))
}
