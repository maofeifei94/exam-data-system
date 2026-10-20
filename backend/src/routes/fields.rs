use crate::db::{new_id, now};
use crate::error::{AppError, AppResult};
use crate::models::{AliasInput, AliasMapping, ColumnSuggestion, StandardField, StandardFieldInput, SuggestInput};
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::Json;
use rusqlite::params;
use serde_json::json;
use std::sync::Arc;

fn row_to_field(r: &rusqlite::Row<'_>) -> rusqlite::Result<StandardField> {
    Ok(StandardField {
        id: r.get(0)?,
        name: r.get(1)?,
        label: r.get(2)?,
        dtype: r.get(3)?,
        description: r.get(4)?,
        system: r.get::<_, i64>(5)? != 0,
    })
}

pub async fn list_standard(State(state): State<Arc<AppState>>) -> AppResult<Json<Vec<StandardField>>> {
    let conn = state.lock()?;
    let mut stmt = conn.prepare(
        "SELECT id, name, label, dtype, description, system FROM standard_fields ORDER BY system DESC, name ASC",
    )?;
    let rows = stmt.query_map([], |r| row_to_field(r))?;
    Ok(Json(rows.collect::<Result<Vec<_>, _>>()?))
}

pub async fn create_standard(
    State(state): State<Arc<AppState>>,
    Json(input): Json<StandardFieldInput>,
) -> AppResult<Json<StandardField>> {
    let name = input.name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::bad_request("标准字段名不能为空"));
    }
    let conn = state.lock()?;
    let id = new_id();
    conn.execute(
        "INSERT INTO standard_fields (id, name, label, dtype, description, system)
         VALUES (?1, ?2, ?3, ?4, ?5, 0)",
        params![id, name, input.label, input.dtype, input.description],
    )
    .map_err(|e| {
        if e.to_string().contains("UNIQUE") {
            AppError::bad_request(format!("标准字段 [{name}] 已存在"))
        } else {
            AppError::from(e)
        }
    })?;
    let mut stmt = conn.prepare(
        "SELECT id, name, label, dtype, description, system FROM standard_fields WHERE id = ?1",
    )?;
    let f = stmt.query_row(params![id], |r| row_to_field(r))?;
    Ok(Json(f))
}

pub async fn update_standard(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(input): Json<StandardFieldInput>,
) -> AppResult<Json<StandardField>> {
    let conn = state.lock()?;
    let n = conn.execute(
        "UPDATE standard_fields SET name = ?1, label = ?2, dtype = ?3, description = ?4 WHERE id = ?5",
        params![input.name.trim(), input.label, input.dtype, input.description, id],
    )?;
    if n == 0 {
        return Err(AppError::not_found("标准字段不存在"));
    }
    let mut stmt = conn.prepare(
        "SELECT id, name, label, dtype, description, system FROM standard_fields WHERE id = ?1",
    )?;
    let f = stmt.query_row(params![id], |r| row_to_field(r))?;
    Ok(Json(f))
}

pub async fn delete_standard(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    let conn = state.lock()?;
    conn.execute("DELETE FROM standard_fields WHERE id = ?1", params![id])?;
    Ok(Json(json!({ "ok": true })))
}

pub async fn list_aliases(State(state): State<Arc<AppState>>) -> AppResult<Json<Vec<AliasMapping>>> {
    let conn = state.lock()?;
    let mut stmt =
        conn.prepare("SELECT id, alias, standard, updated_at FROM alias_mappings ORDER BY alias ASC")?;
    let rows = stmt.query_map([], |r| {
        Ok(AliasMapping {
            id: r.get(0)?,
            alias: r.get(1)?,
            standard: r.get(2)?,
            updated_at: r.get(3)?,
        })
    })?;
    Ok(Json(rows.collect::<Result<Vec<_>, _>>()?))
}

pub async fn save_aliases(
    State(state): State<Arc<AppState>>,
    Json(items): Json<Vec<AliasInput>>,
) -> AppResult<Json<serde_json::Value>> {
    let conn = state.lock()?;
    let mut saved = 0i64;
    for it in items {
        let alias = it.alias.trim();
        let standard = it.standard.trim();
        if alias.is_empty() || standard.is_empty() {
            continue;
        }
        conn.execute(
            "INSERT INTO alias_mappings (id, alias, standard, updated_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(alias) DO UPDATE SET standard = excluded.standard, updated_at = excluded.updated_at",
            params![new_id(), alias, standard, now()],
        )?;
        saved += 1;
    }
    Ok(Json(json!({ "saved": saved })))
}

pub async fn delete_alias(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    let conn = state.lock()?;
    conn.execute("DELETE FROM alias_mappings WHERE id = ?1", params![id])?;
    Ok(Json(json!({ "ok": true })))
}

pub async fn suggest(
    State(state): State<Arc<AppState>>,
    Json(input): Json<SuggestInput>,
) -> AppResult<Json<Vec<ColumnSuggestion>>> {
    let conn = state.lock()?;
    let mut stmt = conn.prepare("SELECT alias, standard FROM alias_mappings")?;
    let alias_map: std::collections::HashMap<String, String> = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .collect();
    let mut stmt2 = conn.prepare("SELECT name FROM standard_fields")?;
    let standards: Vec<String> = stmt2
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    let out = input
        .columns
        .iter()
        .map(|c| {
            if standards.iter().any(|s| s == c) {
                ColumnSuggestion { source: c.clone(), standard: Some(c.clone()), matched_by: "exact_standard".into() }
            } else if let Some(s) = alias_map.get(c) {
                ColumnSuggestion { source: c.clone(), standard: Some(s.clone()), matched_by: "alias".into() }
            } else {
                ColumnSuggestion { source: c.clone(), standard: None, matched_by: "none".into() }
            }
        })
        .collect();
    Ok(Json(out))
}
