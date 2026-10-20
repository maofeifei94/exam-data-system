use rusqlite::Connection;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::error::{AppError, AppResult};

pub struct AppState {
    pub conn: Mutex<Connection>,
}

impl AppState {
    pub fn new(conn: Connection) -> Arc<Self> {
        Arc::new(AppState { conn: Mutex::new(conn) })
    }

    pub fn lock(&self) -> AppResult<MutexGuard<'_, Connection>> {
        self.conn
            .lock()
            .map_err(|_| AppError::internal("数据库连接被污染（panic 过）"))
    }
}
