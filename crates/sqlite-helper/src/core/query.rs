use std::collections::HashMap;

use rusqlite::{types::ValueRef, Connection, Row};

use crate::core::types::{ColumnMeta, ExecResult, QueryResult};
use crate::error::AppResult;

pub fn run_query(
    conn: &Connection,
    sql: &str,
    limit: usize,
    offset: Option<usize>,
) -> AppResult<QueryResult> {
    // v0: implement offset by wrapping query if provided. This avoids relying on client SQL edits,
    // but still keeps things simple. For complex queries, user should provide LIMIT/OFFSET in SQL.
    let effective_sql = if let Some(off) = offset {
        format!("SELECT * FROM ({sql}) LIMIT {limit} OFFSET {off}")
    } else {
        sql.to_string()
    };

    let mut stmt = conn.prepare(&effective_sql)?;
    let col_names: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();

    let mut columns = Vec::with_capacity(col_names.len());
    for (i, name) in col_names.iter().enumerate() {
        let decl_type = stmt.column_decltype(i).map(|s| s.to_string());
        columns.push(ColumnMeta {
            name: name.clone(),
            decl_type,
            sqlite_type: decl_type.clone(),
        });
    }

    let mut rows = Vec::new();
    let mut truncated = false;
    let mut next_offset = None;

    let mut r = stmt.query([])?;
    while let Some(row) = r.next()? {
        if rows.len() >= limit {
            truncated = true;
            next_offset = Some(offset.unwrap_or(0) + rows.len());
            break;
        }

        rows.push(row_to_json_object(row, &col_names)?);
    }

    Ok(QueryResult {
        columns,
        rows,
        truncated,
        next_offset,
    })
}

pub fn run_execute(conn: &Connection, sql: &str) -> AppResult<ExecResult> {
    let changes = conn.execute(sql, [])?;
    let last_id = conn.last_insert_rowid();
    Ok(ExecResult {
        changes: changes as u64,
        last_insert_rowid: Some(last_id),
    })
}

fn row_to_json_object(row: &Row<'_>, col_names: &[String]) -> AppResult<HashMap<String, serde_json::Value>> {
    let mut out = HashMap::with_capacity(col_names.len());
    for (i, name) in col_names.iter().enumerate() {
        let v = match row.get_ref(i)? {
            ValueRef::Null => serde_json::Value::Null,
            ValueRef::Integer(x) => serde_json::Value::from(x),
            ValueRef::Real(x) => serde_json::Value::from(x),
            ValueRef::Text(t) => serde_json::Value::from(String::from_utf8_lossy(t).to_string()),
            ValueRef::Blob(b) => serde_json::json!({
                "$type": "blob",
                "base64": base64::encode(b),
                "size": b.len()
            }),
        };
        out.insert(name.clone(), v);
    }
    Ok(out)
}

// NOTE: base64 is used for BLOB encoding. Keep it minimal.
mod base64 {
    pub fn encode(bytes: &[u8]) -> String {
        base64_simd::STANDARD.encode_to_string(bytes)
    }
}

mod base64_simd {
    pub use base64_simd_impl::*;
    mod base64_simd_impl {
        // Minimal embedded base64 encoder to avoid adding dependency right now.
        // This is intentionally simple and not optimized; can be replaced by `base64` crate later.
        const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

        pub struct Standard;
        pub const STANDARD: Standard = Standard;

        impl Standard {
            pub fn encode_to_string(&self, input: &[u8]) -> String {
                let mut out = String::with_capacity(((input.len() + 2) / 3) * 4);
                let mut i = 0;
                while i + 3 <= input.len() {
                    let n = ((input[i] as u32) << 16) | ((input[i + 1] as u32) << 8) | (input[i + 2] as u32);
                    out.push(TABLE[((n >> 18) & 63) as usize] as char);
                    out.push(TABLE[((n >> 12) & 63) as usize] as char);
                    out.push(TABLE[((n >> 6) & 63) as usize] as char);
                    out.push(TABLE[(n & 63) as usize] as char);
                    i += 3;
                }
                let rem = input.len() - i;
                if rem == 1 {
                    let n = (input[i] as u32) << 16;
                    out.push(TABLE[((n >> 18) & 63) as usize] as char);
                    out.push(TABLE[((n >> 12) & 63) as usize] as char);
                    out.push('=');
                    out.push('=');
                } else if rem == 2 {
                    let n = ((input[i] as u32) << 16) | ((input[i + 1] as u32) << 8);
                    out.push(TABLE[((n >> 18) & 63) as usize] as char);
                    out.push(TABLE[((n >> 12) & 63) as usize] as char);
                    out.push(TABLE[((n >> 6) & 63) as usize] as char);
                    out.push('=');
                }
                out
            }
        }
    }
}

