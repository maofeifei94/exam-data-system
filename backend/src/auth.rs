//! 登录认证与操作日志。
//!
//! - 口令使用随机盐 + 多轮 SHA-256 存储，不保存明文；
//! - 登录成功后签发随机 token，存 `sessions` 表，默认 7 天有效；
//! - 所有 `/api` 接口（健康检查、登录除外）都经过 [`require_auth`] 中间件；
//! - 写操作成功后自动记一条操作日志，流程运行 / 数据导入再补充业务明细。

use crate::db::{new_id, now};
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, Method};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Arc;

/// 登录会话有效期（天）。
const SESSION_DAYS: i64 = 7;
/// 口令哈希轮数：本地单机场景下兼顾安全与登录速度。
const HASH_ROUNDS: usize = 20_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthUser {
    pub id: String,
    pub username: String,
    pub display_name: String,
    pub role: String,
}

fn to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

pub fn new_salt() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// 加盐多轮哈希，返回十六进制摘要。
pub fn hash_password(password: &str, salt: &str) -> String {
    let mut digest = Sha256::digest(format!("{salt}:{password}").as_bytes());
    for _ in 0..HASH_ROUNDS {
        let mut hasher = Sha256::new();
        hasher.update(digest);
        hasher.update(salt.as_bytes());
        hasher.update(password.as_bytes());
        digest = hasher.finalize();
    }
    to_hex(&digest)
}

/// 生成 `盐$摘要` 形式的口令串。
pub fn make_password(password: &str) -> String {
    let salt = new_salt();
    format!("{salt}${}", hash_password(password, &salt))
}

pub fn verify_password(password: &str, stored: &str) -> bool {
    match stored.split_once('$') {
        Some((salt, expected)) => hash_password(password, salt) == expected,
        None => false,
    }
}

/// 首次启动时创建默认管理员；已存在时不覆盖（避免重置用户改过的密码）。
pub fn seed_admin(conn: &Connection) -> rusqlite::Result<()> {
    let exists: i64 = conn.query_row(
        "SELECT COUNT(*) FROM users WHERE username = 'admin'",
        [],
        |r| r.get(0),
    )?;
    if exists == 0 {
        conn.execute(
            "INSERT INTO users (id, username, display_name, password_hash, role, disabled, created_at, last_login_at)
             VALUES ('user_admin', 'admin', '系统管理员', ?1, 'admin', 0, ?2, '')",
            params![make_password("admin123"), now()],
        )?;
    }
    Ok(())
}

pub fn create_session(conn: &Connection, user_id: &str) -> AppResult<(String, String)> {
    // 顺带清理过期会话，避免 sessions 表无限增长。
    let _ = conn.execute(
        "DELETE FROM sessions WHERE expires_at <= ?1",
        params![now()],
    );
    let token = format!("{}{}", new_id(), new_id());
    let expires_at = (chrono::Local::now() + chrono::Duration::days(SESSION_DAYS))
        .format("%Y-%m-%d %H:%M:%S")
        .to_string();
    conn.execute(
        "INSERT INTO sessions (token, user_id, created_at, expires_at) VALUES (?1, ?2, ?3, ?4)",
        params![token, user_id, now(), expires_at],
    )?;
    Ok((token, expires_at))
}

pub fn user_for_token(conn: &Connection, token: &str) -> AppResult<Option<AuthUser>> {
    let mut stmt = conn.prepare(
        "SELECT u.id, u.username, u.display_name, u.role
         FROM sessions s JOIN users u ON u.id = s.user_id
         WHERE s.token = ?1 AND s.expires_at > ?2 AND u.disabled = 0",
    )?;
    let user = stmt
        .query_row(params![token, now()], |r| {
            Ok(AuthUser {
                id: r.get(0)?,
                username: r.get(1)?,
                display_name: r.get(2)?,
                role: r.get(3)?,
            })
        })
        .optional()?;
    Ok(user)
}

/// 从 `Authorization: Bearer xxx` 读取 token。
pub fn bearer_token(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let token = raw
        .strip_prefix("Bearer ")
        .or_else(|| raw.strip_prefix("bearer "))?;
    let token = token.trim();
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}

/// 写入一条操作日志。`target` 为（对象类型, 对象 ID, 对象名称）。
pub fn record(
    conn: &Connection,
    user: &AuthUser,
    action: &str,
    target: (&str, &str, &str),
    project_id: &str,
    detail: &str,
) -> rusqlite::Result<()> {
    let (target_type, target_id, target_name) = target;
    conn.execute(
        "INSERT INTO operation_logs
           (id, user_id, username, action, target_type, target_id, target_name, project_id, detail, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            new_id(),
            user.id,
            user.username,
            action,
            target_type,
            target_id,
            target_name,
            project_id,
            detail,
            now()
        ],
    )?;
    Ok(())
}

struct AuditSpec {
    action: &'static str,
    target_type: &'static str,
    target_id: String,
}

/// 根据请求方法与路径推断操作日志内容；返回 None 表示不需要记录。
///
/// 流程运行与数据导入有更详细的业务日志，这里跳过，避免重复。
fn audit_spec(method: &Method, path: &str) -> Option<AuditSpec> {
    if method != Method::POST && method != Method::PUT && method != Method::DELETE {
        return None;
    }
    let parts: Vec<&str> = path.trim_matches('/').split('/').collect();
    if parts.first() != Some(&"api") {
        return None;
    }
    let rest = &parts[1..];
    let id = |i: usize| rest.get(i).map(|s| s.to_string()).unwrap_or_default();
    let spec = |action, target_type, target_id: String| {
        Some(AuditSpec {
            action,
            target_type,
            target_id,
        })
    };
    let m = method.as_str();
    match (m, rest) {
        ("POST", ["projects"]) => spec("新建项目", "project", String::new()),
        ("PUT", ["projects", _]) => spec("修改项目", "project", id(1)),
        ("DELETE", ["projects", _]) => spec("删除项目", "project", id(1)),
        ("DELETE", ["datasets", _]) => spec("删除数据集", "dataset", id(1)),
        ("POST", ["datasets", _, "apply_mapping"]) => spec("字段映射", "dataset", id(1)),
        ("POST", ["flows"]) => spec("新建流程", "flow", String::new()),
        ("POST", ["flows", _, "duplicate"]) => spec("复制流程", "flow", id(1)),
        ("PUT", ["flows", _]) => spec("修改流程", "flow", id(1)),
        ("DELETE", ["flows", _]) => spec("删除流程", "flow", id(1)),
        ("POST", ["pdf-templates"]) => spec("新建 PDF 模板", "pdf_template", String::new()),
        ("POST", ["pdf-templates", _, "render"]) => spec("生成 PDF", "pdf_template", id(1)),
        ("POST", ["pdf-templates", _, "duplicate"]) => spec("复制 PDF 模板", "pdf_template", id(1)),
        ("PUT", ["pdf-templates", _]) => spec("修改 PDF 模板", "pdf_template", id(1)),
        ("DELETE", ["pdf-templates", _]) => spec("删除 PDF 模板", "pdf_template", id(1)),
        ("POST", ["standard-fields"]) => spec("新增标准字段", "field", String::new()),
        ("PUT", ["standard-fields", _]) => spec("修改标准字段", "field", id(1)),
        ("DELETE", ["standard-fields", _]) => spec("删除标准字段", "field", id(1)),
        ("POST", ["field-groups"]) => spec("新增字段分组", "field", String::new()),
        ("PUT", ["field-groups", _]) => spec("修改字段分组", "field", id(1)),
        ("DELETE", ["field-groups", _]) => spec("删除字段分组", "field", id(1)),
        ("POST", ["aliases"]) => spec("保存字段别名", "alias", String::new()),
        ("DELETE", ["aliases", _]) => spec("删除字段别名", "alias", id(1)),
        ("POST", ["project-fields"]) | ("POST", ["project-fields", "copy"]) => {
            spec("维护项目字段", "field", String::new())
        }
        ("PUT", ["project-fields", _]) => spec("修改项目字段", "field", id(1)),
        ("DELETE", ["project-fields", _]) => spec("删除项目字段", "field", id(1)),
        ("POST", ["project-aliases"]) => spec("保存项目别名", "alias", String::new()),
        ("DELETE", ["project-aliases", _]) => spec("删除项目别名", "alias", id(1)),
        _ => None,
    }
}

/// 认证中间件：校验 token，把当前用户注入请求扩展，并为写操作记录日志。
pub async fn require_auth(
    State(state): State<Arc<AppState>>,
    mut req: Request,
    next: Next,
) -> Response {
    let path = req.uri().path().to_string();
    if path == "/api/health" || path == "/api/auth/login" {
        return next.run(req).await;
    }
    let Some(token) = bearer_token(req.headers()) else {
        return AppError::unauthorized("请先登录").into_response();
    };
    let user = match state.db(move |conn| user_for_token(conn, &token)).await {
        Ok(Some(user)) => user,
        Ok(None) => {
            return AppError::unauthorized("登录已过期，请重新登录").into_response();
        }
        Err(err) => return err.into_response(),
    };

    let method = req.method().clone();
    req.extensions_mut().insert(user.clone());
    let response = next.run(req).await;

    if response.status().is_success() {
        if let Some(spec) = audit_spec(&method, &path) {
            let detail = format!("{} {}", method, path);
            let _ = state
                .db(move |conn| {
                    record(
                        conn,
                        &user,
                        spec.action,
                        (spec.target_type, &spec.target_id, ""),
                        "",
                        &detail,
                    )
                    .map_err(AppError::from)
                })
                .await;
        }
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_hash_roundtrip_and_rejects_wrong_password() {
        let stored = make_password("secret123");
        assert!(stored.contains('$'));
        assert!(verify_password("secret123", &stored));
        assert!(!verify_password("secret124", &stored));
        assert!(!verify_password("secret123", "no-salt-format"));
    }

    #[test]
    fn audit_spec_skips_reads_and_previews() {
        assert!(audit_spec(&Method::GET, "/api/projects").is_none());
        assert!(audit_spec(&Method::POST, "/api/pdf-templates/preview").is_none());
        let spec = audit_spec(&Method::PUT, "/api/flows/abc").expect("flow update");
        assert_eq!(spec.action, "修改流程");
        assert_eq!(spec.target_id, "abc");
        let run = audit_spec(&Method::POST, "/api/flows/abc/run");
        assert!(run.is_none(), "流程运行由业务日志单独记录");
    }
}
