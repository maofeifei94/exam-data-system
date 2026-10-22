//! 考试数据处理系统 —— 后端服务入口。
//!
//! 启动流程：打开 SQLite → 建表/写入种子数据 → 构建路由 → 监听 127.0.0.1:8710。
//! 可通过环境变量覆盖：
//!   EXAM_DB    数据库文件路径（默认 exam_data.db）
//!   EXAM_ADDR  监听地址（默认 127.0.0.1:8710）
//!   EXAM_DATA_DIR   PDF 照片目录（默认当前目录下的 photos/）

mod db;
mod engine;
mod error;
mod importer;
mod models;
mod pdfgen;
mod routes;
mod state;
mod store;
mod util;

use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let db_path = std::env::var("EXAM_DB").unwrap_or_else(|_| "exam_data.db".to_string());

    // 建表 / 迁移 / 种子数据只在启动时做一次，用完即释放连接，
    // 之后所有请求都走连接池，避免再出现「全局单连接」的并发瓶颈。
    {
        let conn = db::open(&db_path)?;
        db::init(&conn)?;
    }
    let state = state::AppState::new(&db_path, state::PoolConfig::default())?;

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = routes::router().with_state(Arc::clone(&state)).layer(cors);

    let addr: SocketAddr = std::env::var("EXAM_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:8710".to_string())
        .parse()
        .map_err(|e| anyhow::anyhow!("监听地址解析失败: {e}"))?;

    let listener = tokio::net::TcpListener::bind(addr).await?;
    println!("exam-backend 已启动: http://{addr}  (db: {db_path})");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    // 停止接收新请求后清空空闲连接，让 SQLite 正常关闭。
    state.pool().close();
    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}
