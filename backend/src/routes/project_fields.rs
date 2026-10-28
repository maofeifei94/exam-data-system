//! 项目本地字段模板。
//!
//! 项目可以「从全局字段库挑选」复制出自己的一份字段副本，之后随意改名、
//! 改类型都不会影响全局字典。复制关系只记在 `source_id` 上，用于回溯来源。

use crate::db::{self, new_id, now};
use crate::error::{AppError, AppResult};
use crate::models::{
    CopyProjectFieldsInput, ProjectAlias, ProjectAliasInput, ProjectField, ProjectFieldInput,
    ProjectFieldUpdate,
};
use crate::routes::fields::{normalize_dtype, validate_field_name};
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::Json;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

const FIELD_COLS: &str = "id, project_id, name, label, dtype, precision, group_id, description, source_id, created_at, updated_at";

fn row_to_field(r: &rusqlite::Row<'_>) -> rusqlite::Result<ProjectField> {
    Ok(ProjectField {
        id: r.get(0)?,
        project_id: r.get(1)?,
        name: r.get(2)?,
        label: r.get(3)?,
        dtype: r.get(4)?,
        precision: r.get(5)?,
        group_id: r.get(6)?,
        description: r.get(7)?,
        source_id: r.get(8)?,
        created_at: r.get(9)?,
        updated_at: r.get(10)?,
    })
}

fn fetch_field(conn: &Connection, id: &str) -> AppResult<ProjectField> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {FIELD_COLS} FROM project_fields WHERE id = ?1"
    ))?;
    stmt.query_row(params![id], row_to_field)
        .optional()?
        .ok_or_else(|| AppError::not_found("项目字段不存在"))
}

fn require_project(raw: &str) -> AppResult<String> {
    let project_id = raw.trim().to_string();
    if project_id.is_empty() {
        return Err(AppError::bad_request("必须指定考试项目"));
    }
    Ok(project_id)
}

fn is_unique_violation(e: &rusqlite::Error) -> bool {
    matches!(
        e,
        rusqlite::Error::SqliteFailure(err, _)
            if err.code == rusqlite::ErrorCode::ConstraintViolation
    )
}

#[derive(Debug, Deserialize)]
pub struct ProjectFieldsQuery {
    #[serde(default)]
    pub project_id: Option<String>,
}

/// 列出某个项目的本地字段；不传 project_id 时返回空列表，
/// 避免误把全局字典当作项目字段展示。
pub async fn list(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ProjectFieldsQuery>,
) -> AppResult<Json<Vec<ProjectField>>> {
    let project_id = q.project_id.unwrap_or_default().trim().to_string();
    if project_id.is_empty() {
        return Ok(Json(Vec::new()));
    }
    let fields = state
        .db(move |conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {FIELD_COLS} FROM project_fields WHERE project_id = ?1 ORDER BY name ASC"
            ))?;
            let rows = stmt.query_map(params![project_id], row_to_field)?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
        })
        .await?;
    Ok(Json(fields))
}

pub async fn create(
    State(state): State<Arc<AppState>>,
    Json(input): Json<ProjectFieldInput>,
) -> AppResult<Json<ProjectField>> {
    let project_id = require_project(&input.project_id)?;
    let name = validate_field_name(&input.name)?;
    let dtype = normalize_dtype(&input.dtype)?;
    let precision = input.precision.unwrap_or(2).clamp(0, 6);
    let group_id = input.group_id.unwrap_or_default();
    let source_id = input.source_id.unwrap_or_default();
    let id = new_id();
    let ts = now();

    let field = state
        .db(move |conn| {
            db::with_tx(conn, |tx| {
                tx.execute(
                    "INSERT INTO project_fields
                       (id, project_id, name, label, dtype, precision, group_id, description,
                        source_id, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)",
                    params![
                        id,
                        project_id,
                        name,
                        input.label,
                        dtype,
                        precision,
                        group_id,
                        input.description,
                        source_id,
                        ts
                    ],
                )
                .map_err(|e| {
                    if is_unique_violation(&e) {
                        AppError::conflict(format!("本项目已有字段 [{name}]"))
                    } else {
                        AppError::from(e)
                    }
                })?;
                fetch_field(tx, &id)
            })
        })
        .await?;
    Ok(Json(field))
}

pub async fn update(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(input): Json<ProjectFieldUpdate>,
) -> AppResult<Json<ProjectField>> {
    let name = validate_field_name(&input.name)?;
    let dtype = normalize_dtype(&input.dtype)?;
    let precision = input.precision.unwrap_or(2).clamp(0, 6);
    let group_id = input.group_id.unwrap_or_default();

    let field = state
        .db(move |conn| {
            db::with_tx(conn, |tx| {
                let old = fetch_field(tx, &id)?;
                tx.execute(
                    "UPDATE project_fields
                     SET name = ?1, label = ?2, dtype = ?3, precision = ?4, group_id = ?5,
                         description = ?6, updated_at = ?7
                     WHERE id = ?8",
                    params![
                        name,
                        input.label,
                        dtype,
                        precision,
                        group_id,
                        input.description,
                        now(),
                        id
                    ],
                )
                .map_err(|e| {
                    if is_unique_violation(&e) {
                        AppError::conflict(format!("本项目已有字段 [{name}]"))
                    } else {
                        AppError::from(e)
                    }
                })?;
                // 字段改名时同步项目内的别名归属，卡片不会残留在旧名上。
                if old.name != name {
                    tx.execute(
                        "UPDATE project_alias_mappings SET standard = ?1, updated_at = ?2
                         WHERE project_id = ?3 AND standard = ?4",
                        params![name, now(), old.project_id, old.name],
                    )?;
                }
                fetch_field(tx, &id)
            })
        })
        .await?;
    Ok(Json(field))
}

pub async fn remove(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    let removed = state
        .db(move |conn| {
            db::with_tx(conn, |tx| {
                let field = fetch_field(tx, &id)?;
                tx.execute(
                    "DELETE FROM project_alias_mappings WHERE project_id = ?1 AND standard = ?2",
                    params![field.project_id, field.name],
                )?;
                Ok(tx.execute("DELETE FROM project_fields WHERE id = ?1", params![id])? as i64)
            })
        })
        .await?;
    if removed == 0 {
        return Err(AppError::not_found("项目字段不存在"));
    }
    Ok(Json(json!({ "ok": true })))
}

const ALIAS_COLS: &str = "id, project_id, alias, standard, updated_at";

fn row_to_alias(r: &rusqlite::Row<'_>) -> rusqlite::Result<ProjectAlias> {
    Ok(ProjectAlias {
        id: r.get(0)?,
        project_id: r.get(1)?,
        alias: r.get(2)?,
        standard: r.get(3)?,
        updated_at: r.get(4)?,
    })
}

/// 列出项目内的全部别名（服务于项目字段卡片）。
pub async fn list_aliases(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ProjectFieldsQuery>,
) -> AppResult<Json<Vec<ProjectAlias>>> {
    let project_id = q.project_id.unwrap_or_default().trim().to_string();
    if project_id.is_empty() {
        return Ok(Json(Vec::new()));
    }
    let aliases = state
        .db(move |conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {ALIAS_COLS} FROM project_alias_mappings
                 WHERE project_id = ?1 ORDER BY updated_at ASC"
            ))?;
            let rows = stmt.query_map(params![project_id], row_to_alias)?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
        })
        .await?;
    Ok(Json(aliases))
}

/// 新增或更新一条项目别名（同一项目内别名唯一）。
pub async fn save_alias(
    State(state): State<Arc<AppState>>,
    Json(input): Json<ProjectAliasInput>,
) -> AppResult<Json<ProjectAlias>> {
    let project_id = require_project(&input.project_id)?;
    let alias = input.alias.trim().to_string();
    let standard = input.standard.trim().to_string();
    if alias.is_empty() || standard.is_empty() {
        return Err(AppError::validation("别名与字段名不能为空"));
    }
    if alias.chars().count() > 120 {
        return Err(AppError::validation(format!("别名过长: {alias}")));
    }

    let saved = state
        .db(move |conn| {
            db::with_tx(conn, |tx| {
                // 只允许给本项目已有的字段添加别名，避免项目别名指向全局字段。
                let exists: Option<String> = tx
                    .query_row(
                        "SELECT id FROM project_fields WHERE project_id = ?1 AND name = ?2",
                        params![project_id, standard],
                        |r| r.get(0),
                    )
                    .optional()?;
                if exists.is_none() {
                    return Err(AppError::not_found(format!("本项目没有字段 [{standard}]")));
                }
                let ts = now();
                tx.execute(
                    "INSERT INTO project_alias_mappings (id, project_id, alias, standard, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT(project_id, alias) DO UPDATE SET
                         standard = excluded.standard,
                         updated_at = excluded.updated_at",
                    params![new_id(), project_id, alias, standard, ts],
                )?;
                tx.query_row(
                    &format!(
                        "SELECT {ALIAS_COLS} FROM project_alias_mappings
                         WHERE project_id = ?1 AND alias = ?2"
                    ),
                    params![project_id, alias],
                    row_to_alias,
                )
                .map_err(AppError::from)
            })
        })
        .await?;
    Ok(Json(saved))
}

pub async fn delete_alias(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    let removed = state
        .db(move |conn| {
            Ok(conn.execute(
                "DELETE FROM project_alias_mappings WHERE id = ?1",
                params![id],
            )? as i64)
        })
        .await?;
    if removed == 0 {
        return Err(AppError::not_found("别名不存在"));
    }
    Ok(Json(json!({ "ok": true })))
}

/// 把选中的全局字段复制进项目。
///
/// 同名冲突时跳过（幂等），返回实际复制的数量，方便前端提示。
pub async fn copy_from_global(
    State(state): State<Arc<AppState>>,
    Json(input): Json<CopyProjectFieldsInput>,
) -> AppResult<Json<serde_json::Value>> {
    let project_id = require_project(&input.project_id)?;
    let field_ids = input.field_ids;

    let copied = state
        .db(move |conn| db::with_tx(conn, |tx| copy_fields(tx, &project_id, &field_ids)))
        .await?;
    Ok(Json(json!({ "copied": copied })))
}

fn copy_fields(conn: &Connection, project_id: &str, field_ids: &[String]) -> AppResult<i64> {
    let mut copied = 0i64;
    let ts = now();
    for field_id in field_ids {
        let mut stmt = conn.prepare(
            "SELECT name, label, dtype, precision, group_id, description
             FROM standard_fields WHERE id = ?1",
        )?;
        let source = stmt
            .query_row(params![field_id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                ))
            })
            .optional()?;
        let Some((name, label, dtype, precision, group_id, description)) = source else {
            continue;
        };
        copied += conn.execute(
            "INSERT INTO project_fields
               (id, project_id, name, label, dtype, precision, group_id, description,
                source_id, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)
             ON CONFLICT(project_id, name) DO NOTHING",
            params![
                new_id(),
                project_id,
                name,
                label,
                dtype,
                precision,
                group_id,
                description,
                field_id,
                ts
            ],
        )? as i64;

        // 别名整体复制到项目，保证项目字段的编辑界面与全局完全一致。
        let mut alias_stmt =
            conn.prepare("SELECT alias FROM alias_mappings WHERE standard = ?1")?;
        let aliases: Vec<String> = alias_stmt
            .query_map(params![name], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        for alias in aliases {
            conn.execute(
                "INSERT INTO project_alias_mappings (id, project_id, alias, standard, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(project_id, alias) DO NOTHING",
                params![new_id(), project_id, alias, name, ts],
            )?;
        }
    }
    Ok(copied)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory_db() -> Connection {
        let conn = Connection::open_in_memory().expect("memory db");
        db::init(&conn).expect("init");
        conn
    }

    fn seeded_field_id(conn: &Connection, name: &str) -> String {
        conn.query_row(
            "SELECT id FROM standard_fields WHERE name = ?1",
            params![name],
            |r| r.get(0),
        )
        .expect("seed field")
    }

    #[test]
    fn copy_is_idempotent_and_scoped_per_project() {
        let conn = memory_db();
        let id = seeded_field_id(&conn, "身份证号");

        assert_eq!(
            copy_fields(&conn, "p1", std::slice::from_ref(&id)).expect("first copy"),
            1
        );
        // 同名重复复制被跳过，不会报错
        assert_eq!(
            copy_fields(&conn, "p1", std::slice::from_ref(&id)).expect("dup copy"),
            0
        );
        // 另一个项目有自己独立的副本
        assert_eq!(
            copy_fields(&conn, "p2", std::slice::from_ref(&id)).expect("other project"),
            1
        );

        let project_of: String = conn
            .query_row(
                "SELECT project_id FROM project_fields WHERE name = '身份证号' AND project_id = 'p2'",
                [],
                |r| r.get(0),
            )
            .expect("project row");
        assert_eq!(project_of, "p2");
    }

    #[test]
    fn copy_brings_aliases_with_the_field() {
        let conn = memory_db();
        let id = seeded_field_id(&conn, "身份证号");
        let global_aliases: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM alias_mappings WHERE standard = '身份证号'",
                [],
                |r| r.get(0),
            )
            .expect("global aliases");
        assert!(global_aliases > 0, "seed 数据应包含别名");

        copy_fields(&conn, "p1", &[id]).expect("copy");
        let project_aliases: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM project_alias_mappings
                 WHERE project_id = 'p1' AND standard = '身份证号'",
                [],
                |r| r.get(0),
            )
            .expect("project aliases");
        assert_eq!(project_aliases, global_aliases);

        // 另一个项目不会共享别名。
        let other: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM project_alias_mappings WHERE project_id = 'p2'",
                [],
                |r| r.get(0),
            )
            .expect("other aliases");
        assert_eq!(other, 0);
    }

    #[test]
    fn copy_skips_unknown_field_ids() {
        let conn = memory_db();
        assert_eq!(
            copy_fields(&conn, "p1", &["missing".to_string()]).expect("copy"),
            0
        );
    }
}
