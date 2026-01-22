use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnMeta {
    pub name: String,
    #[serde(default)]
    pub decl_type: Option<String>,
    #[serde(default)]
    pub sqlite_type: Option<String>,
}

pub type DbRow = std::collections::HashMap<String, serde_json::Value>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryResult {
    pub columns: Vec<ColumnMeta>,
    pub rows: Vec<DbRow>,
    #[serde(default)]
    pub truncated: bool,
    #[serde(default)]
    pub next_offset: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecResult {
    pub changes: u64,
    #[serde(default)]
    pub last_insert_rowid: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct Limits {
    pub max_rows: usize,
}

