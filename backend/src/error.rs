use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

/// 统一错误类型。
///
/// - HTTP 响应固定为 `{"error": "...", "code": "..."}`；
/// - `code` 是给前端做分支判断的稳定标识，不随文案变化；
/// - 数据库/IO 的原始细节只写日志，不返回给客户端。
#[derive(Debug)]
pub struct AppError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
}

impl AppError {
    fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "bad_request", message)
    }

    /// 输入校验失败：语义上比 bad_request 更明确，便于前端统一提示。
    pub fn validation(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "validation_error", message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", message)
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, "conflict", message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", message)
    }

    /// 记录内部错误细节，对外只返回通用文案。
    pub fn internal_with_context(context: &str, detail: impl std::fmt::Display) -> Self {
        eprintln!("[exam-backend] {context}: {detail}");
        Self::internal("服务器内部错误，请查看服务端日志")
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for AppError {}

impl From<rusqlite::Error> for AppError {
    fn from(e: rusqlite::Error) -> Self {
        use rusqlite::ErrorCode;
        match &e {
            rusqlite::Error::SqliteFailure(err, _)
                if err.code == ErrorCode::ConstraintViolation =>
            {
                Self::conflict("数据冲突：违反唯一性或引用约束")
            }
            rusqlite::Error::QueryReturnedNoRows => Self::not_found("记录不存在"),
            _ => Self::internal_with_context("SQLite 执行失败", &e),
        }
    }
}

impl From<anyhow::Error> for AppError {
    fn from(e: anyhow::Error) -> Self {
        Self::internal_with_context("内部错误", format!("{e:#}"))
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        Self::internal_with_context("IO 操作失败", e)
    }
}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            "invalid_json",
            format!("JSON 解析失败: {e}"),
        )
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(json!({ "error": self.message, "code": self.code })),
        )
            .into_response()
    }
}

pub type AppResult<T> = Result<T, AppError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bad_request_uses_stable_code() {
        let e = AppError::bad_request("参数错误");
        assert_eq!(e.status, StatusCode::BAD_REQUEST);
        assert_eq!(e.code, "bad_request");
        assert_eq!(e.message, "参数错误");
    }

    #[test]
    fn constraint_violation_maps_to_conflict() {
        let raw = rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error {
                code: rusqlite::ErrorCode::ConstraintViolation,
                extended_code: 2067,
            },
            Some("UNIQUE constraint failed".to_string()),
        );
        let e = AppError::from(raw);
        assert_eq!(e.status, StatusCode::CONFLICT);
        assert_eq!(e.code, "conflict");
    }

    #[test]
    fn other_sqlite_errors_do_not_leak_details() {
        let raw = rusqlite::Error::InvalidQuery;
        let e = AppError::from(raw);
        assert_eq!(e.status, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(!e.message.contains("InvalidQuery"));
    }
}
