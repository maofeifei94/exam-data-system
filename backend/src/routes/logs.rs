//! 操作日志查询。

use crate::error::AppResult;
use crate::state::AppState;
use axum::extract::{Query, State};
use axum::Json;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Deserialize)]
pub struct LogQuery {
    /// 操作员用户名（精确匹配）
    #[serde(default)]
    pub username: Option<String>,
    /// 操作类型（精确匹配，如「运行流程」）
    #[serde(default)]
    pub action: Option<String>,
    /// 关键字：匹配操作员、操作、对象名与详情
    #[serde(default)]
    pub q: Option<String>,
    #[serde(default)]
    pub offset: Option<i64>,
    #[serde(default)]
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct OperationLog {
    pub id: String,
    pub user_id: String,
    pub username: String,
    pub display_name: String,
    pub action: String,
    pub target_type: String,
    pub target_id: String,
    pub target_name: String,
    pub project_id: String,
    pub project_name: String,
    pub detail: String,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct LogPage {
    pub total: i64,
    pub items: Vec<OperationLog>,
}

pub async fn list(
    State(state): State<Arc<AppState>>,
    Query(q): Query<LogQuery>,
) -> AppResult<Json<LogPage>> {
    let username = q.username.unwrap_or_default().trim().to_string();
    let action = q.action.unwrap_or_default().trim().to_string();
    let keyword = q.q.unwrap_or_default().trim().to_string();
    let limit = q.limit.unwrap_or(100).clamp(1, 500);
    let offset = q.offset.unwrap_or(0).max(0);
    let like = format!("%{keyword}%");

    let page = state
        .db(move |conn| {
            let total: i64 = conn.query_row(
                "SELECT COUNT(*) FROM operation_logs l
                 WHERE (?1 = '' OR l.username = ?1)
                   AND (?2 = '' OR l.action = ?2)
                   AND (?3 = '' OR l.username LIKE ?3 OR l.action LIKE ?3
                        OR l.target_name LIKE ?3 OR l.detail LIKE ?3)",
                params![username, action, like],
                |r| r.get(0),
            )?;
            let mut stmt = conn.prepare(
                "SELECT l.id, l.user_id, l.username, COALESCE(u.display_name, ''),
                        l.action, l.target_type, l.target_id, l.target_name,
                        l.project_id, COALESCE(p.name, ''), l.detail, l.created_at
                 FROM operation_logs l
                 LEFT JOIN users u ON u.id = l.user_id
                 LEFT JOIN exam_projects p ON p.id = l.project_id
                 WHERE (?1 = '' OR l.username = ?1)
                   AND (?2 = '' OR l.action = ?2)
                   AND (?3 = '' OR l.username LIKE ?3 OR l.action LIKE ?3
                        OR l.target_name LIKE ?3 OR l.detail LIKE ?3)
                 ORDER BY l.created_at DESC, l.rowid DESC
                 LIMIT ?4 OFFSET ?5",
            )?;
            let rows = stmt.query_map(params![username, action, like, limit, offset], |r| {
                Ok(OperationLog {
                    id: r.get(0)?,
                    user_id: r.get(1)?,
                    username: r.get(2)?,
                    display_name: r.get(3)?,
                    action: r.get(4)?,
                    target_type: r.get(5)?,
                    target_id: r.get(6)?,
                    target_name: r.get(7)?,
                    project_id: r.get(8)?,
                    project_name: r.get(9)?,
                    detail: r.get(10)?,
                    created_at: r.get(11)?,
                })
            })?;
            let items = rows.collect::<Result<Vec<_>, _>>()?;
            Ok(LogPage { total, items })
        })
        .await?;
    Ok(Json(page))
}
