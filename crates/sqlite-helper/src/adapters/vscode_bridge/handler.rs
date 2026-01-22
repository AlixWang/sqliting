use std::path::PathBuf;

use crate::{
    cli::Args,
    core::{connection::ConnectionManager, limits::effective_limit},
    error::{AppError, AppResult},
};

use super::protocol::*;

pub struct BridgeHandler {
    args: Args,
    cm: ConnectionManager,
    active_db: Option<PathBuf>,
}

impl BridgeHandler {
    pub fn new(args: Args) -> Self {
        Self {
            args,
            cm: ConnectionManager::new(),
            active_db: None,
        }
    }

    pub async fn handle(&mut self, req: BridgeRequest) -> BridgeResponse<serde_json::Value> {
        if req.v != 1 {
            return BridgeResponse::err(
                req.v,
                req.id,
                "INVALID_REQUEST",
                format!("unsupported protocol version: {}", req.v),
            );
        }

        match req.cmd.as_str() {
            "connect" => self.handle_connect(req).await,
            "query" => self.handle_query(req).await,
            "execute" => self.handle_execute(req).await,
            "tables" => self.handle_tables(req).await,
            "columns" => self.handle_columns(req).await,
            other => BridgeResponse::err(
                req.v,
                req.id,
                "INVALID_REQUEST",
                format!("unknown cmd: {other}"),
            ),
        }
    }

    async fn handle_connect(&mut self, req: BridgeRequest) -> BridgeResponse<serde_json::Value> {
        let p: ConnectPayload = match serde_json::from_value(req.payload) {
            Ok(v) => v,
            Err(e) => return err(req, AppError::InvalidRequest(e.to_string())),
        };
        let path = PathBuf::from(p.path);
        self.active_db = Some(path.clone());
        match self.cm.ensure_worker(&path) {
            Ok(_) => ok(req, serde_json::Value::Bool(true)),
            Err(e) => err(req, e),
        }
    }

    async fn handle_query(&mut self, req: BridgeRequest) -> BridgeResponse<serde_json::Value> {
        let p: QueryPayload = match serde_json::from_value(req.payload) {
            Ok(v) => v,
            Err(e) => return err(req, AppError::InvalidRequest(e.to_string())),
        };
        let db_path = match self.resolve_db_path(p.path) {
            Ok(p) => p,
            Err(e) => return err(req, e),
        };
        let worker = match self.cm.ensure_worker(&db_path) {
            Ok(w) => w,
            Err(e) => return err(req, e),
        };
        let limits = effective_limit(p.limit, self.args.max_rows);
        match worker.query(p.sql, limits.max_rows, p.offset).await {
            Ok(qr) => ok(
                req,
                serde_json::to_value(qr).unwrap_or_else(|_| serde_json::Value::Null),
            ),
            Err(e) => err(req, e),
        }
    }

    async fn handle_execute(&mut self, req: BridgeRequest) -> BridgeResponse<serde_json::Value> {
        let p: ExecutePayload = match serde_json::from_value(req.payload) {
            Ok(v) => v,
            Err(e) => return err(req, AppError::InvalidRequest(e.to_string())),
        };
        let db_path = match self.resolve_db_path(p.path) {
            Ok(p) => p,
            Err(e) => return err(req, e),
        };
        let worker = match self.cm.ensure_worker(&db_path) {
            Ok(w) => w,
            Err(e) => return err(req, e),
        };
        match worker.execute(p.sql).await {
            Ok(er) => ok(
                req,
                serde_json::to_value(er).unwrap_or_else(|_| serde_json::Value::Null),
            ),
            Err(e) => err(req, e),
        }
    }

    async fn handle_tables(&mut self, req: BridgeRequest) -> BridgeResponse<serde_json::Value> {
        let p: TablesPayload = match serde_json::from_value(req.payload) {
            Ok(v) => v,
            Err(e) => return err(req, AppError::InvalidRequest(e.to_string())),
        };
        let db_path = match self.resolve_db_path(p.path) {
            Ok(p) => p,
            Err(e) => return err(req, e),
        };
        let worker = match self.cm.ensure_worker(&db_path) {
            Ok(w) => w,
            Err(e) => return err(req, e),
        };
        match worker.tables().await {
            Ok(v) => ok(
                req,
                serde_json::to_value(v).unwrap_or_else(|_| serde_json::Value::Null),
            ),
            Err(e) => err(req, e),
        }
    }

    async fn handle_columns(&mut self, req: BridgeRequest) -> BridgeResponse<serde_json::Value> {
        let p: ColumnsPayload = match serde_json::from_value(req.payload) {
            Ok(v) => v,
            Err(e) => return err(req, AppError::InvalidRequest(e.to_string())),
        };
        let db_path = match self.resolve_db_path(p.path) {
            Ok(p) => p,
            Err(e) => return err(req, e),
        };
        let worker = match self.cm.ensure_worker(&db_path) {
            Ok(w) => w,
            Err(e) => return err(req, e),
        };
        match worker.columns(p.table).await {
            Ok(v) => ok(
                req,
                serde_json::to_value(v).unwrap_or_else(|_| serde_json::Value::Null),
            ),
            Err(e) => err(req, e),
        }
    }

    fn resolve_db_path(&self, payload_path: Option<String>) -> AppResult<PathBuf> {
        if let Some(p) = payload_path {
            return Ok(PathBuf::from(p));
        }
        self.active_db
            .clone()
            .ok_or_else(|| AppError::InvalidRequest("no active db; call connect first or pass path".into()))
    }
}

fn ok(req: BridgeRequest, data: serde_json::Value) -> BridgeResponse<serde_json::Value> {
    BridgeResponse::ok(req.v, req.id, data)
}

fn err(req: BridgeRequest, e: AppError) -> BridgeResponse<serde_json::Value> {
    BridgeResponse::err(req.v, req.id, e.code(), e.to_string())
}

