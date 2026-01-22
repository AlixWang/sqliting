use std::path::{Component, Path, PathBuf};

use serde_json::Value;
use tokio::io::{self, AsyncBufReadExt, AsyncWriteExt};

use crate::{
    cli::Args,
    core::{connection::ConnectionManager, limits::effective_limit, schema},
    error::{AppError, AppResult},
};

/// MCP server (JSON-RPC 2.0 over stdio).
///
/// Implements the minimal set required by RFC-001/RFC-002:
/// - initialize
/// - tools/list
/// - tools/call: read_query, write_query, get_schema
/// - resources/read (sqlite://.../tables/...)
/// - prompts/list, prompts/get (analyze-db-health)
pub async fn run(args: Args) -> AppResult<()> {
    let cm = ConnectionManager::new();

    let mut stdin = io::BufReader::new(io::stdin());
    let mut stdout = io::BufWriter::new(io::stdout());
    let mut line = String::new();

    loop {
        line.clear();
        let n = stdin.read_line(&mut line).await?;
        if n == 0 {
            break;
        }
        let raw = line.trim();
        if raw.is_empty() {
            continue;
        }

        let msg: Value = match serde_json::from_str(raw) {
            Ok(v) => v,
            Err(e) => {
                let resp = jsonrpc_error(Value::Null, -32700, format!("parse error: {e}"), None);
                write_line(&mut stdout, &resp).await?;
                continue;
            }
        };

        // Notifications (no id) are ignored.
        let id = msg.get("id").cloned().unwrap_or(Value::Null);
        if id.is_null() {
            continue;
        }

        let Some(method) = msg.get("method").and_then(|m| m.as_str()) else {
            let resp = jsonrpc_error(id, -32600, "invalid request: missing method".into(), None);
            write_line(&mut stdout, &resp).await?;
            continue;
        };

        let params = msg.get("params").cloned().unwrap_or(Value::Null);

        let resp = match method {
            "initialize" => handle_initialize(id),
            "tools/list" => handle_tools_list(id),
            "tools/call" => handle_tools_call(id, params, &args, &cm).await,
            "resources/list" => handle_resources_list(id),
            "resources/read" => handle_resources_read(id, params, &args, &cm).await,
            "prompts/list" => handle_prompts_list(id),
            "prompts/get" => handle_prompts_get(id, params),
            _ => jsonrpc_error(id, -32601, format!("method not found: {method}"), None),
        };

        write_line(&mut stdout, &resp).await?;
    }

    Ok(())
}

fn handle_initialize(id: Value) -> Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "serverInfo": {
                "name": "sqlite-helper",
                "version": env!("CARGO_PKG_VERSION")
            },
            "capabilities": {
                "tools": { "listChanged": false },
                "resources": { },
                "prompts": { }
            }
        }
    })
}

fn handle_tools_list(id: Value) -> Value {
    // MCP tools/list: https://modelcontextprotocol.io/specification/.../server/tools
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "tools": [
                {
                    "name": "read_query",
                    "description": "Execute a read-only SQL query (SELECT/PRAGMA/EXPLAIN) to analyze data.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "db_path": { "type": "string" },
                            "sql": { "type": "string" },
                            "limit": { "type": "integer", "minimum": 1 },
                            "offset": { "type": "integer", "minimum": 0 }
                        },
                        "required": ["db_path", "sql"]
                    }
                },
                {
                    "name": "write_query",
                    "description": "Execute a write SQL query (INSERT/UPDATE/DELETE/DDL). Requires user confirmation in the client.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "db_path": { "type": "string" },
                            "sql": { "type": "string" }
                        },
                        "required": ["db_path", "sql"]
                    }
                },
                {
                    "name": "get_schema",
                    "description": "Get database structure (tables and columns).",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "db_path": { "type": "string" }
                        },
                        "required": ["db_path"]
                    }
                },
                {
                    "name": "analyze_db_health",
                    "description": "Run PRAGMA integrity_check and return a health report.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "db_path": { "type": "string" }
                        },
                        "required": ["db_path"]
                    }
                }
            ]
        }
    })
}

async fn handle_tools_call(id: Value, params: Value, args: &Args, cm: &ConnectionManager) -> Value {
    let Some(name) = params.get("name").and_then(|v| v.as_str()) else {
        return jsonrpc_error(id, -32602, "invalid params: missing name".into(), None);
    };
    let arguments = params.get("arguments").cloned().unwrap_or(Value::Null);

    let res = match name {
        "read_query" => tool_read_query(arguments, args, cm).await,
        "write_query" => tool_write_query(arguments, args, cm).await,
        "get_schema" => tool_get_schema(arguments, args, cm).await,
        "analyze_db_health" => tool_analyze_db_health(arguments, args, cm).await,
        other => Err(AppError::InvalidRequest(format!("unknown tool: {other}"))),
    };

    match res {
        Ok((text, structured)) => serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "content": [{ "type": "text", "text": text }],
                "structuredContent": structured,
                "isError": false
            }
        }),
        Err(e) => serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "content": [{ "type": "text", "text": format!("{}: {}", e.code(), e) }],
                "isError": true
            }
        }),
    }
}

fn handle_resources_list(id: Value) -> Value {
    // Resources are dynamic (depend on db path), so we don't enumerate here.
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": { "resources": [] }
    })
}

async fn handle_resources_read(id: Value, params: Value, args: &Args, cm: &ConnectionManager) -> Value {
    let Some(uri) = params.get("uri").and_then(|v| v.as_str()) else {
        return jsonrpc_error(id, -32602, "invalid params: missing uri".into(), None);
    };

    match read_sqlite_table_resource(uri, args, cm).await {
        Ok((text, structured)) => serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "contents": [{
                    "uri": uri,
                    "mimeType": "application/json",
                    "text": text
                }],
                "structuredContent": structured
            }
        }),
        Err(e) => serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": -32000, "message": format!("{}: {}", e.code(), e) }
        }),
    }
}

fn handle_prompts_list(id: Value) -> Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "prompts": [
                { "name": "analyze-db-health", "description": "Run PRAGMA integrity_check and return a health report." }
            ]
        }
    })
}

fn handle_prompts_get(id: Value, params: Value) -> Value {
    let Some(name) = params.get("name").and_then(|v| v.as_str()) else {
        return jsonrpc_error(id, -32602, "invalid params: missing name".into(), None);
    };
    if name != "analyze-db-health" {
        return jsonrpc_error(id, -32602, format!("unknown prompt: {name}"), None);
    }

    // Keep it simple: provide instructions; client/tool caller provides db_path and runs tools.
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "description": "Analyze database health.",
            "messages": [
                {
                    "role": "user",
                    "content": {
                        "type": "text",
                        "text": "Run PRAGMA integrity_check; list tables and column counts; summarize any issues."
                    }
                }
            ]
        }
    })
}

async fn tool_read_query(arguments: Value, args: &Args, cm: &ConnectionManager) -> AppResult<(String, Value)> {
    let db_path = get_string(&arguments, "db_path")?;
    let sql = get_string(&arguments, "sql")?;
    let limit = arguments.get("limit").and_then(|v| v.as_u64()).map(|n| n as usize);
    let offset = arguments.get("offset").and_then(|v| v.as_u64()).map(|n| n as usize);

    let db_path = validate_db_path(Path::new(&db_path), &args.allowed_dir)?;
    let worker = cm.ensure_worker(&db_path)?;
    let limits = effective_limit(limit, args.max_rows);
    let qr = worker.read_query(sql, limits.max_rows, offset).await?;

    let structured = serde_json::to_value(&qr)?;
    let text = serde_json::to_string_pretty(&structured).unwrap_or_else(|_| "<result>".into());
    Ok((text, structured))
}

async fn tool_write_query(arguments: Value, args: &Args, cm: &ConnectionManager) -> AppResult<(String, Value)> {
    let db_path = get_string(&arguments, "db_path")?;
    let sql = get_string(&arguments, "sql")?;

    let db_path = validate_db_path(Path::new(&db_path), &args.allowed_dir)?;
    let worker = cm.ensure_worker(&db_path)?;
    let er = worker.execute(sql).await?;

    let structured = serde_json::to_value(&er)?;
    let text = serde_json::to_string_pretty(&structured).unwrap_or_else(|_| "<result>".into());
    Ok((text, structured))
}

async fn tool_get_schema(arguments: Value, args: &Args, cm: &ConnectionManager) -> AppResult<(String, Value)> {
    let db_path = get_string(&arguments, "db_path")?;
    let db_path = validate_db_path(Path::new(&db_path), &args.allowed_dir)?;
    let worker = cm.ensure_worker(&db_path)?;

    let tables = worker.tables().await?;
    let mut out_tables = Vec::with_capacity(tables.len());
    for t in tables {
        let cols = worker.columns(t.clone()).await?;
        out_tables.push(serde_json::json!({ "name": t, "columns": cols }));
    }

    let structured = serde_json::json!({ "tables": out_tables });
    let text = serde_json::to_string_pretty(&structured).unwrap_or_else(|_| "<result>".into());
    Ok((text, structured))
}

async fn tool_analyze_db_health(arguments: Value, args: &Args, cm: &ConnectionManager) -> AppResult<(String, Value)> {
    let db_path = get_string(&arguments, "db_path")?;
    let db_path = validate_db_path(Path::new(&db_path), &args.allowed_dir)?;
    let worker = cm.ensure_worker(&db_path)?;

    // PRAGMA integrity_check returns rows like [{ "integrity_check": "ok" }] or multiple rows with errors.
    let integrity = worker
        .read_query("PRAGMA integrity_check".to_string(), 50, None)
        .await?;

    let file_size = std::fs::metadata(&db_path).map(|m| m.len()).ok();
    let tables = worker.tables().await?;
    let mut table_summaries = Vec::with_capacity(tables.len());
    for t in tables {
        let cols = worker.columns(t.clone()).await?;
        table_summaries.push(serde_json::json!({
            "name": t,
            "column_count": cols.len(),
            "columns": cols
        }));
    }

    let structured = serde_json::json!({
        "db_path": db_path,
        "file_size_bytes": file_size,
        "integrity_check": integrity,
        "schema": { "tables": table_summaries }
    });
    let text = serde_json::to_string_pretty(&structured).unwrap_or_else(|_| "<result>".into());
    Ok((text, structured))
}

async fn read_sqlite_table_resource(uri: &str, args: &Args, cm: &ConnectionManager) -> AppResult<(String, Value)> {
    // RFC-001 URI: sqlite://{abs_path_to_db}/tables/{table_name}
    let (db_path, table) = parse_sqlite_table_uri(uri)?;
    let db_path = validate_db_path(&db_path, &args.allowed_dir)?;
    let worker = cm.ensure_worker(&db_path)?;

    // Preview first 50 rows.
    if !schema::is_safe_table_ref(&table) {
        return Err(AppError::InvalidRequest(format!(
            "invalid table name in resource uri: {table}"
        )));
    }
    let sql = format!("SELECT * FROM {table} LIMIT 50");
    let qr = worker.read_query(sql, 50, None).await?;
    let structured = serde_json::to_value(&qr)?;
    let text = serde_json::to_string_pretty(&structured).unwrap_or_else(|_| "<result>".into());
    Ok((text, structured))
}

fn parse_sqlite_table_uri(uri: &str) -> AppResult<(PathBuf, String)> {
    let uri = uri.strip_prefix("sqlite://").ok_or_else(|| {
        AppError::InvalidRequest("resource uri must start with sqlite://".into())
    })?;
    let parts: Vec<&str> = uri.split("/tables/").collect();
    if parts.len() != 2 {
        return Err(AppError::InvalidRequest(
            "resource uri must be sqlite://{abs_path}/tables/{table}".into(),
        ));
    }
    let db_path = parts[0];
    let table = parts[1];
    if table.is_empty() {
        return Err(AppError::InvalidRequest("missing table name".into()));
    }
    Ok((PathBuf::from(db_path), table.to_string()))
}

fn get_string(obj: &Value, key: &str) -> AppResult<String> {
    obj.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| AppError::InvalidRequest(format!("missing or invalid field: {key}")))
}

fn validate_db_path(db_path: &Path, allowed_dirs: &[PathBuf]) -> AppResult<PathBuf> {
    let abs = if db_path.is_absolute() {
        db_path.to_path_buf()
    } else {
        std::env::current_dir()?.join(db_path)
    };

    if allowed_dirs.is_empty() {
        return Ok(normalize_lexical(&abs));
    }

    let abs_norm = normalize_lexical(&abs);
    for d in allowed_dirs {
        let d = normalize_lexical(d);
        if abs_norm.starts_with(&d) {
            return Ok(abs_norm);
        }
    }
    Err(AppError::PathNotAllowed(abs_norm))
}

fn normalize_lexical(p: &Path) -> PathBuf {
    // Normalize lexically (remove `.` and resolve `..`) without touching filesystem,
    // so it works even if DB file doesn't exist yet.
    let mut out = PathBuf::new();

    for comp in p.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                // Pop only if we have something to pop and the last isn't a prefix/root.
                let popped = out.pop();
                if !popped {
                    // Can't go above root/prefix; keep as-is (still prevents escaping during starts_with checks).
                }
            }
            Component::Prefix(prefix) => out.push(prefix.as_os_str()),
            Component::RootDir => out.push(comp.as_os_str()),
            Component::Normal(c) => out.push(c),
        }
    }

    out
}

async fn write_line(w: &mut io::BufWriter<io::Stdout>, v: &Value) -> AppResult<()> {
    serde_json::to_writer(w, v)?;
    w.write_all(b"\n").await?;
    w.flush().await?;
    Ok(())
}

fn jsonrpc_error(id: Value, code: i64, message: String, data: Option<Value>) -> Value {
    let mut err = serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message }
    });
    if let Some(d) = data {
        if let Some(obj) = err.get_mut("error").and_then(|v| v.as_object_mut()) {
            obj.insert("data".to_string(), d);
        }
    }
    err
}

