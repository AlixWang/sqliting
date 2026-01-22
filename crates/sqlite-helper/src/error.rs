use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("invalid request: {0}")]
    InvalidRequest(String),

    #[error("path not allowed: {0}")]
    PathNotAllowed(PathBuf),

    #[error("failed to open database: {path}: {source}")]
    DbOpenFailed {
        path: PathBuf,
        #[source]
        source: rusqlite::Error,
    },

    #[error("sql error: {0}")]
    SqlError(String),

    #[error("query is not read-only")]
    NotReadonly,

    #[error("timeout")]
    Timeout,

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("internal error: {0}")]
    Internal(String),
}

impl From<rusqlite::Error> for AppError {
    fn from(e: rusqlite::Error) -> Self {
        AppError::SqlError(e.to_string())
    }
}

impl AppError {
    pub fn code(&self) -> &'static str {
        match self {
            AppError::InvalidRequest(_) => "INVALID_REQUEST",
            AppError::PathNotAllowed(_) => "PATH_NOT_ALLOWED",
            AppError::DbOpenFailed { .. } => "DB_OPEN_FAILED",
            AppError::SqlError(_) => "SQL_ERROR",
            AppError::NotReadonly => "NOT_READONLY",
            AppError::Timeout => "TIMEOUT",
            AppError::Io(_) => "IO_ERROR",
            AppError::Json(_) => "JSON_ERROR",
            AppError::Internal(_) => "INTERNAL",
        }
    }
}

pub type AppResult<T> = Result<T, AppError>;

