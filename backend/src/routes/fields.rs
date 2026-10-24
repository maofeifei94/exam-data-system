//! 标准字段字典、字段分组与别名映射接口。
//!
//! 这三张表共同服务于「导入后的字段映射」：标准字段是目标集合，
//! 别名表记录历史映射经验，分组只影响展示顺序。

use crate::db::{self, new_id, now};
use crate::error::{AppError, AppResult};
use crate::models::{
    AliasInput, AliasMapping, ColumnSuggestion, FieldGroup, FieldGroupInput, StandardField,
    StandardFieldInput, SuggestInput,
};
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::Json;
use rusqlite::{params, Connection};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;

const FIELD_COLS: &str = "id, name, label, dtype, precision, group_id, description, system";

/// 允许的字段类型。照片不作为数据类型：照片文件名按文本存储，
/// 由 PDF 模板的照片元素负责「目录 + 名称 + 后缀」解析。
///
/// number 为历史类型，保留兼容；新数据统一用 decimal / integer。
const ALLOWED_DTYPES: &[&str] = &[
    "text", "number", "integer", "decimal", "percent", "date", "datetime", "boolean",
];

fn row_to_field(r: &rusqlite::Row<'_>) -> rusqlite::Result<StandardField> {
    Ok(StandardField {
        id: r.get(0)?,
        name: r.get(1)?,
        label: r.get(2)?,
        dtype: r.get(3)?,
        precision: r.get(4)?,
        group_id: r.get(5)?,
        description: r.get(6)?,
        system: r.get::<_, i64>(7)? != 0,
    })
}

fn fetch_field(conn: &Connection, id: &str) -> AppResult<StandardField> {
    use rusqlite::OptionalExtension;
    let mut stmt = conn.prepare(&format!(
        "SELECT {FIELD_COLS} FROM standard_fields WHERE id = ?1"
    ))?;
    stmt.query_row(params![id], row_to_field)
        .optional()?
        .ok_or_else(|| AppError::not_found("标准字段不存在"))
}

fn validate_field_name(raw: &str) -> AppResult<String> {
    let name = raw.trim().to_string();
    if name.is_empty() {
        return Err(AppError::validation("标准字段名不能为空"));
    }
    if name.chars().count() > 80 {
        return Err(AppError::validation("标准字段名不能超过 80 个字符"));
    }
    Ok(name)
}

fn normalize_dtype(raw: &str) -> AppResult<String> {
    let dtype = raw.trim().to_ascii_lowercase();
    if dtype.is_empty() {
        return Ok("text".to_string());
    }
    if !ALLOWED_DTYPES.contains(&dtype.as_str()) {
        return Err(AppError::validation(format!(
            "不支持的字段类型 [{dtype}]，可选：{}",
            ALLOWED_DTYPES.join("、")
        )));
    }
    Ok(dtype)
}

fn is_unique_violation(e: &rusqlite::Error) -> bool {
    matches!(
        e,
        rusqlite::Error::SqliteFailure(err, _)
            if err.code == rusqlite::ErrorCode::ConstraintViolation
    )
}

pub async fn list_standard(
    State(state): State<Arc<AppState>>,
) -> AppResult<Json<Vec<StandardField>>> {
    let fields = state
        .db(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {FIELD_COLS} FROM standard_fields ORDER BY name ASC"
            ))?;
            let rows = stmt.query_map([], row_to_field)?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
        })
        .await?;
    Ok(Json(fields))
}

pub async fn create_standard(
    State(state): State<Arc<AppState>>,
    Json(input): Json<StandardFieldInput>,
) -> AppResult<Json<StandardField>> {
    let name = validate_field_name(&input.name)?;
    let dtype = normalize_dtype(&input.dtype)?;
    let precision = input.precision.unwrap_or(2).clamp(0, 6);
    let group_id = input.group_id.unwrap_or_default();
    let id = new_id();

    let field = state
        .db(move |conn| {
            db::with_tx(conn, |tx| {
                tx.execute(
                    "INSERT INTO standard_fields
                       (id, name, label, dtype, precision, group_id, description, system)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0)",
                    params![
                        id,
                        name,
                        input.label,
                        dtype,
                        precision,
                        group_id,
                        input.description
                    ],
                )
                .map_err(|e| {
                    if is_unique_violation(&e) {
                        AppError::conflict(format!("标准字段 [{name}] 已存在"))
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

pub async fn update_standard(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(input): Json<StandardFieldInput>,
) -> AppResult<Json<StandardField>> {
    let name = validate_field_name(&input.name)?;
    let dtype = normalize_dtype(&input.dtype)?;
    let precision = input.precision.unwrap_or(2).clamp(0, 6);
    let group_id = input.group_id.unwrap_or_default();

    let field = state
        .db(move |conn| {
            db::with_tx(conn, |tx| {
                let old = fetch_field(tx, &id)?;
                if old.name != name {
                    // 重命名标准字段必须同步别名表的引用，否则别名卡片会「留在旧名上」，
                    // 前端看起来就像新建了一个字段。
                    tx.execute(
                        "UPDATE alias_mappings SET standard = ?1, updated_at = ?2
                         WHERE standard = ?3",
                        params![name, now(), old.name],
                    )?;
                }
                tx.execute(
                    "UPDATE standard_fields
                     SET name = ?1, label = ?2, dtype = ?3, precision = ?4, group_id = ?5,
                         description = ?6
                     WHERE id = ?7",
                    params![
                        name,
                        input.label,
                        dtype,
                        precision,
                        group_id,
                        input.description,
                        id
                    ],
                )
                .map_err(|e| {
                    if is_unique_violation(&e) {
                        AppError::conflict(format!("标准字段 [{name}] 已存在"))
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

/// 删除标准字段时同步清掉指向它的别名，避免导入建议引用已不存在的字段。
pub async fn delete_standard(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    state
        .db(move |conn| {
            db::with_tx(conn, |tx| {
                let field = fetch_field(tx, &id)?;
                tx.execute(
                    "DELETE FROM alias_mappings WHERE standard = ?1",
                    params![field.name],
                )?;
                tx.execute("DELETE FROM standard_fields WHERE id = ?1", params![id])?;
                Ok(())
            })
        })
        .await?;
    Ok(Json(json!({ "ok": true })))
}

// ---------------------------------------------------------------------------
// 字段分组
// ---------------------------------------------------------------------------

fn row_to_group(r: &rusqlite::Row<'_>) -> rusqlite::Result<FieldGroup> {
    Ok(FieldGroup {
        id: r.get(0)?,
        name: r.get(1)?,
        sort_order: r.get(2)?,
    })
}

fn validate_group_name(raw: &str) -> AppResult<String> {
    let name = raw.trim().to_string();
    if name.is_empty() {
        return Err(AppError::validation("分组名称不能为空"));
    }
    if name.chars().count() > 60 {
        return Err(AppError::validation("分组名称不能超过 60 个字符"));
    }
    Ok(name)
}

fn fetch_group(conn: &Connection, id: &str) -> AppResult<FieldGroup> {
    use rusqlite::OptionalExtension;
    let mut stmt = conn.prepare("SELECT id, name, sort_order FROM field_groups WHERE id = ?1")?;
    stmt.query_row(params![id], row_to_group)
        .optional()?
        .ok_or_else(|| AppError::not_found("分组不存在"))
}

pub async fn list_groups(State(state): State<Arc<AppState>>) -> AppResult<Json<Vec<FieldGroup>>> {
    let groups = state
        .db(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, name, sort_order FROM field_groups ORDER BY sort_order ASC, name ASC",
            )?;
            let rows = stmt.query_map([], row_to_group)?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
        })
        .await?;
    Ok(Json(groups))
}

pub async fn create_group(
    State(state): State<Arc<AppState>>,
    Json(input): Json<FieldGroupInput>,
) -> AppResult<Json<FieldGroup>> {
    let name = validate_group_name(&input.name)?;
    let id = new_id();

    let group = state
        .db(move |conn| {
            db::with_tx(conn, |tx| {
                let sort_order = match input.sort_order {
                    Some(v) => v,
                    None => tx.query_row(
                        "SELECT COALESCE(MAX(sort_order), 0) + 10 FROM field_groups",
                        [],
                        |r| r.get::<_, i64>(0),
                    )?,
                };
                tx.execute(
                    "INSERT INTO field_groups (id, name, sort_order, created_at)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![id, name, sort_order, now()],
                )?;
                fetch_group(tx, &id)
            })
        })
        .await?;
    Ok(Json(group))
}

pub async fn update_group(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(input): Json<FieldGroupInput>,
) -> AppResult<Json<FieldGroup>> {
    let name = validate_group_name(&input.name)?;

    let group = state
        .db(move |conn| {
            db::with_tx(conn, |tx| {
                tx.execute(
                    "UPDATE field_groups SET name = ?1, sort_order = COALESCE(?2, sort_order)
                     WHERE id = ?3",
                    params![name, input.sort_order, id],
                )?;
                fetch_group(tx, &id)
            })
        })
        .await?;
    Ok(Json(group))
}

/// 删除分组只解除归属，不删除字段本身。
pub async fn delete_group(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    state
        .db(move |conn| {
            db::with_tx(conn, |tx| {
                fetch_group(tx, &id)?;
                tx.execute(
                    "UPDATE standard_fields SET group_id = '' WHERE group_id = ?1",
                    params![id],
                )?;
                tx.execute("DELETE FROM field_groups WHERE id = ?1", params![id])?;
                Ok(())
            })
        })
        .await?;
    Ok(Json(json!({ "ok": true })))
}

pub async fn list_aliases(
    State(state): State<Arc<AppState>>,
) -> AppResult<Json<Vec<AliasMapping>>> {
    let aliases = state
        .db(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, alias, standard, updated_at FROM alias_mappings ORDER BY alias ASC",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok(AliasMapping {
                    id: r.get(0)?,
                    alias: r.get(1)?,
                    standard: r.get(2)?,
                    updated_at: r.get(3)?,
                })
            })?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
        })
        .await?;
    Ok(Json(aliases))
}

/// 批量保存别名：整批放在一个事务里，失败时不会写入一半。
pub async fn save_aliases(
    State(state): State<Arc<AppState>>,
    Json(items): Json<Vec<AliasInput>>,
) -> AppResult<Json<serde_json::Value>> {
    let saved = state
        .db(move |conn| {
            db::with_tx(conn, |tx| {
                let mut saved = 0i64;
                for it in items.iter() {
                    let alias = it.alias.trim();
                    let standard = it.standard.trim();
                    if alias.is_empty() || standard.is_empty() {
                        continue;
                    }
                    if alias.chars().count() > 120 {
                        return Err(AppError::validation(format!("别名过长: {alias}")));
                    }
                    tx.execute(
                        "INSERT INTO alias_mappings (id, alias, standard, updated_at)
                         VALUES (?1, ?2, ?3, ?4)
                         ON CONFLICT(alias) DO UPDATE SET
                             standard = excluded.standard,
                             updated_at = excluded.updated_at",
                        params![new_id(), alias, standard, now()],
                    )?;
                    saved += 1;
                }
                Ok(saved)
            })
        })
        .await?;
    Ok(Json(json!({ "saved": saved })))
}

pub async fn delete_alias(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    let removed = state
        .db(move |conn| {
            Ok(conn.execute("DELETE FROM alias_mappings WHERE id = ?1", params![id])? as i64)
        })
        .await?;
    if removed == 0 {
        return Err(AppError::not_found("别名不存在"));
    }
    Ok(Json(json!({ "ok": true })))
}

/// 根据标准字段与别名表为导入列给出映射建议。
pub async fn suggest(
    State(state): State<Arc<AppState>>,
    Json(input): Json<SuggestInput>,
) -> AppResult<Json<Vec<ColumnSuggestion>>> {
    let out = state
        .db(move |conn| {
            let mut stmt = conn.prepare("SELECT alias, standard FROM alias_mappings")?;
            let alias_map: HashMap<String, String> = stmt
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .collect();

            let mut stmt2 = conn.prepare("SELECT name FROM standard_fields")?;
            let standards: Vec<String> = stmt2
                .query_map([], |r| r.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;

            Ok(input
                .columns
                .iter()
                .map(|c| {
                    if standards.iter().any(|s| s == c) {
                        ColumnSuggestion {
                            source: c.clone(),
                            standard: Some(c.clone()),
                            matched_by: "exact_standard".into(),
                        }
                    } else if let Some(s) = alias_map.get(c) {
                        ColumnSuggestion {
                            source: c.clone(),
                            standard: Some(s.clone()),
                            matched_by: "alias".into(),
                        }
                    } else {
                        ColumnSuggestion {
                            source: c.clone(),
                            standard: None,
                            matched_by: "none".into(),
                        }
                    }
                })
                .collect())
        })
        .await?;
    Ok(Json(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dtype_is_normalized_and_whitelisted() {
        assert_eq!(normalize_dtype("").expect("default"), "text");
        assert_eq!(normalize_dtype(" NUMBER ").expect("upper"), "number");
        assert_eq!(normalize_dtype("percent").expect("percent"), "percent");
        let err = normalize_dtype("blob").expect_err("illegal dtype");
        assert_eq!(err.code, "validation_error");
        // 照片不再是数据类型，必须明确拒绝
        assert!(normalize_dtype("image").is_err());
    }
}
