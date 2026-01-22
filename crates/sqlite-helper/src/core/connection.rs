use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread,
};

use rusqlite::{Connection, OpenFlags};
use tokio::sync::oneshot;

use crate::{
    core::{query, readonly, schema, types::ExecResult, types::QueryResult},
    error::{AppError, AppResult},
};

#[derive(Debug, Clone)]
pub struct ConnectionManager {
    inner: Arc<Mutex<HashMap<PathBuf, WorkerHandle>>>,
    busy_timeout_ms: u64,
}

impl ConnectionManager {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            busy_timeout_ms: 2_000,
        }
    }

    pub fn ensure_worker(&self, db_path: &Path) -> AppResult<WorkerHandle> {
        let db_path = canonicalize_lossy(db_path)?;
        let mut guard = self.inner.lock().map_err(|_| AppError::Internal("poisoned lock".into()))?;
        if let Some(h) = guard.get(&db_path) {
            return Ok(h.clone());
        }

        let h = WorkerHandle::spawn(db_path.clone(), self.busy_timeout_ms)?;
        guard.insert(db_path, h.clone());
        Ok(h)
    }
}

#[derive(Debug, Clone)]
pub struct WorkerHandle {
    tx: std::sync::mpsc::Sender<DbTask>,
    pub db_path: PathBuf,
}

impl WorkerHandle {
    fn spawn(db_path: PathBuf, busy_timeout_ms: u64) -> AppResult<Self> {
        let (tx, rx) = std::sync::mpsc::channel::<DbTask>();
        let path_for_thread = db_path.clone();
        thread::spawn(move || db_worker_main(path_for_thread, busy_timeout_ms, rx));
        Ok(Self { tx, db_path })
    }

    pub async fn query(
        &self,
        sql: String,
        limit: usize,
        offset: Option<usize>,
    ) -> AppResult<QueryResult> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(DbTask::Query {
                sql,
                limit,
                offset,
                respond_to: tx,
            })
            .map_err(|_| AppError::Internal("db worker unavailable".into()))?;
        rx.await.map_err(|_| AppError::Internal("db worker dropped response".into()))?
    }

    /// Query with a readonly check performed inside the DB worker (for MCP read_query).
    pub async fn read_query(
        &self,
        sql: String,
        limit: usize,
        offset: Option<usize>,
    ) -> AppResult<QueryResult> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(DbTask::ReadQuery {
                sql,
                limit,
                offset,
                respond_to: tx,
            })
            .map_err(|_| AppError::Internal("db worker unavailable".into()))?;
        rx.await
            .map_err(|_| AppError::Internal("db worker dropped response".into()))?
    }

    pub async fn execute(&self, sql: String) -> AppResult<ExecResult> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(DbTask::Execute { sql, respond_to: tx })
            .map_err(|_| AppError::Internal("db worker unavailable".into()))?;
        rx.await.map_err(|_| AppError::Internal("db worker dropped response".into()))?
    }

    pub async fn tables(&self) -> AppResult<Vec<String>> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(DbTask::Tables { respond_to: tx })
            .map_err(|_| AppError::Internal("db worker unavailable".into()))?;
        rx.await.map_err(|_| AppError::Internal("db worker dropped response".into()))?
    }

    pub async fn columns(&self, table: String) -> AppResult<Vec<crate::core::types::ColumnMeta>> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(DbTask::Columns { table, respond_to: tx })
            .map_err(|_| AppError::Internal("db worker unavailable".into()))?;
        rx.await.map_err(|_| AppError::Internal("db worker dropped response".into()))?
    }
}

enum DbTask {
    Query {
        sql: String,
        limit: usize,
        offset: Option<usize>,
        respond_to: oneshot::Sender<AppResult<QueryResult>>,
    },
    ReadQuery {
        sql: String,
        limit: usize,
        offset: Option<usize>,
        respond_to: oneshot::Sender<AppResult<QueryResult>>,
    },
    Execute {
        sql: String,
        respond_to: oneshot::Sender<AppResult<ExecResult>>,
    },
    Tables {
        respond_to: oneshot::Sender<AppResult<Vec<String>>>,
    },
    Columns {
        table: String,
        respond_to: oneshot::Sender<AppResult<Vec<crate::core::types::ColumnMeta>>>,
    },
}

fn db_worker_main(db_path: PathBuf, busy_timeout_ms: u64, rx: std::sync::mpsc::Receiver<DbTask>) {
    let conn = match open_conn(&db_path, busy_timeout_ms) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error=%e, path=%db_path.display(), "failed to open db in worker; dropping tasks");
            // Drain tasks and respond error.
            while let Ok(task) = rx.recv() {
                respond_err(task, e.clone());
            }
            return;
        }
    };

    while let Ok(task) = rx.recv() {
        match task {
            DbTask::Query {
                sql,
                limit,
                offset,
                respond_to,
            } => {
                let res = query::run_query(&conn, &sql, limit, offset);
                let _ = respond_to.send(res);
            }
            DbTask::ReadQuery {
                sql,
                limit,
                offset,
                respond_to,
            } => {
                let res = match readonly::is_sql_readonly(&conn, &sql) {
                    Ok(true) => query::run_query(&conn, &sql, limit, offset),
                    Ok(false) => Err(AppError::NotReadonly),
                    Err(e) => Err(e),
                };
                let _ = respond_to.send(res);
            }
            DbTask::Execute { sql, respond_to } => {
                let res = query::run_execute(&conn, &sql);
                let _ = respond_to.send(res);
            }
            DbTask::Tables { respond_to } => {
                let res = schema::list_tables(&conn);
                let _ = respond_to.send(res);
            }
            DbTask::Columns { table, respond_to } => {
                let res = schema::list_columns(&conn, &table);
                let _ = respond_to.send(res);
            }
        }
    }
}

fn respond_err(task: DbTask, err: AppError) {
    match task {
        DbTask::Query { respond_to, .. } => {
            let _ = respond_to.send(Err(err));
        }
        DbTask::ReadQuery { respond_to, .. } => {
            let _ = respond_to.send(Err(err));
        }
        DbTask::Execute { respond_to, .. } => {
            let _ = respond_to.send(Err(err));
        }
        DbTask::Tables { respond_to } => {
            let _ = respond_to.send(Err(err));
        }
        DbTask::Columns { respond_to, .. } => {
            let _ = respond_to.send(Err(err));
        }
    }
}

fn open_conn(path: &Path, busy_timeout_ms: u64) -> AppResult<Connection> {
    let flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE;
    let conn = Connection::open_with_flags(path, flags)
        .map_err(|source| AppError::DbOpenFailed {
            path: path.to_path_buf(),
            source,
        })?;
    let _ = conn.busy_timeout(std::time::Duration::from_millis(busy_timeout_ms));
    Ok(conn)
}

fn canonicalize_lossy(path: &Path) -> AppResult<PathBuf> {
    // canonicalize requires file exists; SQLite DB might be created on open.
    // Use absolute path when possible.
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        let cwd = std::env::current_dir()?;
        Ok(cwd.join(path))
    }
}

