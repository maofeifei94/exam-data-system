//! 登录、退出、当前用户与操作员管理。

use crate::auth::{bearer_token, create_session, make_password, record, verify_password, AuthUser};
use crate::db::{new_id, now};
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::Extension;
use axum::Json;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

#[derive(Debug, Deserialize)]
pub struct LoginInput {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub token: String,
    pub expires_at: String,
    pub user: AuthUser,
}

pub async fn login(
    State(state): State<Arc<AppState>>,
    Json(input): Json<LoginInput>,
) -> AppResult<Json<LoginResponse>> {
    let username = input.username.trim().to_string();
    if username.is_empty() || input.password.is_empty() {
        return Err(AppError::validation("请输入用户名和密码"));
    }
    let password = input.password;

    let result = state
        .db(move |conn| {
            let row: Option<(String, String, String, String, String)> = conn
                .query_row(
                    "SELECT id, username, display_name, role, password_hash
                     FROM users WHERE username = ?1 AND disabled = 0",
                    params![username],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
                )
                .optional()?;
            let Some((id, username, display_name, role, password_hash)) = row else {
                return Err(AppError::unauthorized("用户名或密码错误"));
            };
            if !verify_password(&password, &password_hash) {
                return Err(AppError::unauthorized("用户名或密码错误"));
            }
            let (token, expires_at) = create_session(conn, &id)?;
            conn.execute(
                "UPDATE users SET last_login_at = ?1 WHERE id = ?2",
                params![now(), id],
            )?;
            let user = AuthUser {
                id,
                username,
                display_name,
                role,
            };
            record(conn, &user, "登录系统", ("session", "", ""), "", "登录成功")
                .map_err(AppError::from)?;
            Ok(LoginResponse {
                token,
                expires_at,
                user,
            })
        })
        .await?;
    Ok(Json(result))
}

pub async fn logout(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    headers: HeaderMap,
) -> AppResult<Json<serde_json::Value>> {
    let token = bearer_token(&headers);
    state
        .db(move |conn| {
            if let Some(token) = token {
                conn.execute("DELETE FROM sessions WHERE token = ?1", params![token])?;
            }
            record(conn, &user, "退出登录", ("session", "", ""), "", "退出成功")
                .map_err(AppError::from)?;
            Ok(())
        })
        .await?;
    Ok(Json(json!({ "ok": true })))
}

pub async fn me(Extension(user): Extension<AuthUser>) -> Json<AuthUser> {
    Json(user)
}

#[derive(Debug, Serialize)]
pub struct UserAccount {
    pub id: String,
    pub username: String,
    pub display_name: String,
    pub role: String,
    pub disabled: bool,
    pub created_at: String,
    pub last_login_at: String,
}

/// 操作员列表：用于日志筛选与用户管理。所有登录用户可见（不含口令信息）。
pub async fn list_users(State(state): State<Arc<AppState>>) -> AppResult<Json<Vec<UserAccount>>> {
    let users = state
        .db(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, username, display_name, role, disabled, created_at, last_login_at
                 FROM users ORDER BY created_at ASC",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok(UserAccount {
                    id: r.get(0)?,
                    username: r.get(1)?,
                    display_name: r.get(2)?,
                    role: r.get(3)?,
                    disabled: r.get::<_, i64>(4)? != 0,
                    created_at: r.get(5)?,
                    last_login_at: r.get(6)?,
                })
            })?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
        })
        .await?;
    Ok(Json(users))
}

#[derive(Debug, Deserialize)]
pub struct CreateUserInput {
    pub username: String,
    pub display_name: String,
    pub password: String,
    #[serde(default)]
    pub role: String,
}

pub async fn create_user(
    State(state): State<Arc<AppState>>,
    Extension(admin): Extension<AuthUser>,
    Json(input): Json<CreateUserInput>,
) -> AppResult<Json<UserAccount>> {
    if admin.role != "admin" {
        return Err(AppError::forbidden("仅管理员可以新增操作员"));
    }
    let username = input.username.trim().to_string();
    if username.len() < 2 || username.len() > 32 || username.chars().any(char::is_whitespace) {
        return Err(AppError::validation("用户名需为 2~32 个字符且不能包含空格"));
    }
    if input.password.chars().count() < 6 {
        return Err(AppError::validation("密码至少 6 位"));
    }
    let display_name = {
        let name = input.display_name.trim().to_string();
        if name.is_empty() {
            username.clone()
        } else {
            name
        }
    };
    let role = if input.role.trim() == "admin" {
        "admin".to_string()
    } else {
        "operator".to_string()
    };
    let display_name_for_log = display_name.clone();

    let user = state
        .db(move |conn| {
            let exists: i64 = conn.query_row(
                "SELECT COUNT(*) FROM users WHERE username = ?1",
                params![username],
                |r| r.get(0),
            )?;
            if exists > 0 {
                return Err(AppError::conflict(format!("用户名 {username} 已存在")));
            }
            let id = new_id();
            let ts = now();
            conn.execute(
                "INSERT INTO users (id, username, display_name, password_hash, role, disabled, created_at, last_login_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6, '')",
                params![
                    id,
                    username,
                    display_name,
                    make_password(&input.password),
                    role,
                    ts
                ],
            )?;
            record(
                conn,
                &admin,
                "新增操作员",
                ("user", &id, &display_name_for_log),
                "",
                &format!("用户名：{username}；角色：{role}"),
            )
            .map_err(AppError::from)?;
            Ok(UserAccount {
                id,
                username,
                display_name: display_name_for_log,
                role,
                disabled: false,
                created_at: ts,
                last_login_at: String::new(),
            })
        })
        .await?;
    Ok(Json(user))
}
