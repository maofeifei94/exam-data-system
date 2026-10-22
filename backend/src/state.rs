//! 应用状态：SQLite 连接池 + 阻塞任务隔离。
//!
//! 设计说明：
//! - rusqlite 是同步 API，直接放在 axum 的 async handler 里会阻塞 tokio 工作线程；
//!   所有数据库操作都通过 `spawn_blocking` 调度到阻塞线程池。
//! - 旧的「单连接 + 全局 Mutex」会让所有请求串行排队；这里改为固定容量连接池，
//!   配合 WAL 模式实现「多读一写」的并发模型。
//! - 执行闭包时捕获 panic，确保连接一定会被归还或丢弃，不会泄漏池容量。

use rusqlite::Connection;
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use crate::error::{AppError, AppResult};

/// 连接池配置。
#[derive(Debug, Clone, Copy)]
pub struct PoolConfig {
    /// 池内最大连接数（至少 1）。
    pub size: usize,
    /// 等待空闲连接的最长时间。
    pub acquire_timeout: Duration,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            size: 4,
            acquire_timeout: Duration::from_secs(15),
        }
    }
}

struct PoolInner {
    idle: Vec<Connection>,
    /// 已创建（空闲 + 使用中）的连接数。
    total: usize,
    closed: bool,
}

pub struct DbPool {
    path: String,
    config: PoolConfig,
    inner: Mutex<PoolInner>,
    available: Condvar,
}

impl DbPool {
    pub fn open(path: &str, config: PoolConfig) -> anyhow::Result<Arc<Self>> {
        let config = PoolConfig {
            size: config.size.clamp(1, 16),
            acquire_timeout: config.acquire_timeout.max(Duration::from_secs(1)),
        };
        let pool = Arc::new(Self {
            path: path.to_string(),
            config,
            inner: Mutex::new(PoolInner {
                idle: Vec::new(),
                total: 0,
                closed: false,
            }),
            available: Condvar::new(),
        });
        // 预热一条连接：既验证数据库文件可打开，也避免首个请求承担建连成本。
        let conn = pool.open_connection()?;
        let mut inner = pool
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("连接池锁被污染"))?;
        inner.idle.push(conn);
        inner.total = 1;
        drop(inner);
        Ok(pool)
    }

    fn open_connection(&self) -> anyhow::Result<Connection> {
        let conn = Connection::open(&self.path)?;
        crate::db::configure(&conn)?;
        Ok(conn)
    }

    fn lock_inner(&self) -> AppResult<std::sync::MutexGuard<'_, PoolInner>> {
        self.inner
            .lock()
            .map_err(|_| AppError::internal("数据库连接池锁被污染"))
    }

    /// 取得一条连接。池满时阻塞等待（带超时），调用方应确保运行在阻塞线程中。
    ///
    /// 必须通过 [`DbPool::release`] 归还；`AppState::db` 已封装了这一步。
    pub fn acquire(&self) -> AppResult<Connection> {
        let deadline = Instant::now() + self.config.acquire_timeout;
        let mut inner = self.lock_inner()?;
        loop {
            if inner.closed {
                return Err(AppError::internal("数据库连接池已关闭"));
            }
            if let Some(conn) = inner.idle.pop() {
                return Ok(conn);
            }
            if inner.total < self.config.size {
                inner.total += 1;
                drop(inner);
                return match self.open_connection() {
                    Ok(conn) => Ok(conn),
                    Err(e) => {
                        // 建连失败必须把占用的名额还回去，否则池会永久少一条连接。
                        let mut inner = self.lock_inner()?;
                        inner.total = inner.total.saturating_sub(1);
                        self.available.notify_one();
                        Err(AppError::internal_with_context("创建数据库连接失败", e))
                    }
                };
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(AppError::internal("数据库繁忙，请稍后重试"));
            }
            let (guard, timeout) = self
                .available
                .wait_timeout(inner, deadline - now)
                .map_err(|_| AppError::internal("数据库连接池锁被污染"))?;
            inner = guard;
            if timeout.timed_out() && inner.idle.is_empty() {
                return Err(AppError::internal("数据库繁忙，请稍后重试"));
            }
        }
    }

    /// 归还连接。`reusable=false` 用于 panic 等连接状态不可信的场景。
    pub fn release(&self, conn: Connection, reusable: bool) {
        match self.inner.lock() {
            Ok(mut inner) if reusable && !inner.closed => {
                inner.idle.push(conn);
                self.available.notify_one();
            }
            Ok(mut inner) => {
                inner.total = inner.total.saturating_sub(1);
                self.available.notify_one();
            }
            // 锁被污染时连接随值一起释放，池已不可用。
            Err(_) => {}
        }
    }

    /// 关闭池：拒绝新请求，丢弃空闲连接。使用中的连接归还时会被直接丢弃。
    pub fn close(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.closed = true;
            inner.idle.clear();
            self.available.notify_all();
        }
    }
}

pub struct AppState {
    pool: Arc<DbPool>,
}

impl AppState {
    pub fn new(path: &str, config: PoolConfig) -> anyhow::Result<Arc<Self>> {
        Ok(Arc::new(Self {
            pool: DbPool::open(path, config)?,
        }))
    }

    pub fn pool(&self) -> &DbPool {
        &self.pool
    }

    /// 在阻塞线程池中执行一段需要数据库连接的逻辑。
    ///
    /// 闭包拿到的是 `&mut Connection`，需要多语句原子性时可直接开启事务。
    pub async fn db<T, F>(self: &Arc<Self>, f: F) -> AppResult<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> AppResult<T> + Send + 'static,
    {
        let pool = Arc::clone(&self.pool);
        self.blocking(move || {
            let mut conn = pool.acquire()?;
            // 捕获 panic：先归还/丢弃连接，再把 panic 继续抛给 tokio。
            match catch_unwind(AssertUnwindSafe(|| f(&mut conn))) {
                Ok(result) => {
                    pool.release(conn, true);
                    result
                }
                Err(payload) => {
                    pool.release(conn, false);
                    resume_unwind(payload)
                }
            }
        })
        .await
    }

    /// 在阻塞线程池中执行纯 CPU / IO 任务（文件解析、模板写入等）。
    pub async fn blocking<T, F>(&self, f: F) -> AppResult<T>
    where
        T: Send + 'static,
        F: FnOnce() -> AppResult<T> + Send + 'static,
    {
        tokio::task::spawn_blocking(f)
            .await
            .map_err(|e| AppError::internal_with_context("后台任务异常退出", e))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db() -> (Arc<DbPool>, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!("exam_pool_{}.db", crate::db::new_id()));
        let pool = DbPool::open(
            path.to_str().unwrap(),
            PoolConfig {
                size: 2,
                acquire_timeout: Duration::from_secs(2),
            },
        )
        .expect("open pool");
        (pool, path)
    }

    #[test]
    fn pool_reuses_and_limits_connections() {
        let (pool, path) = temp_db();
        let a = pool.acquire().expect("first");
        let b = pool.acquire().expect("second");
        // 两条连接都被占用，第三次获取应超时失败而不是无限等待。
        let err = pool.acquire().expect_err("pool should be exhausted");
        assert_eq!(err.status, axum::http::StatusCode::INTERNAL_SERVER_ERROR);
        pool.release(a, true);
        pool.release(b, true);
        let c = pool
            .acquire()
            .expect("released connection should be reusable");
        pool.release(c, true);
        pool.close();
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn panicking_worker_does_not_leak_pool_slot() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (pool, path) = temp_db();
        let state = Arc::new(AppState {
            pool: Arc::clone(&pool),
        });
        let result = rt.block_on(state.db(|_conn| -> AppResult<()> { panic!("boom") }));
        // spawn_blocking 会把 panic 转成 JoinError，这里应拿到内部错误而不是挂死。
        assert!(result.is_err());
        let conn = pool.acquire().expect("slot must be released after panic");
        pool.release(conn, true);
        let _ = std::fs::remove_file(path);
    }
}
