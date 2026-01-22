use serde::{Deserialize, Serialize};

use crate::core::types::{ColumnMeta, ExecResult, QueryResult};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BridgeRequest {
    pub v: u32,
    pub id: String,
    pub cmd: String,
    #[serde(default)]
    pub payload: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct BridgeResponse<T> {
    pub v: u32,
    pub id: String,
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

impl<T> BridgeResponse<T> {
    pub fn ok(v: u32, id: String, data: T) -> Self {
        Self {
            v,
            id,
            status: "ok",
            data: Some(data),
            error: None,
            code: None,
            details: None,
        }
    }

    pub fn err(v: u32, id: String, code: &'static str, error: String) -> Self {
        Self {
            v,
            id,
            status: "error",
            data: None,
            error: Some(error),
            code: Some(code),
            details: None,
        }
    }
}

// Payloads

#[derive(Debug, Deserialize)]
pub struct ConnectPayload {
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct QueryPayload {
    pub sql: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub offset: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct ExecutePayload {
    pub sql: String,
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TablesPayload {
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ColumnsPayload {
    pub table: String,
    #[serde(default)]
    pub path: Option<String>,
}

// Response data wrappers (keeps protocol explicit)
pub type ConnectResult = bool;
pub type TablesResult = Vec<String>;
pub type ColumnsResult = Vec<ColumnMeta>;
pub type QueryResultData = QueryResult;
pub type ExecuteResultData = ExecResult;

