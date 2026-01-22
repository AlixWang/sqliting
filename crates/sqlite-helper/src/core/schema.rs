use rusqlite::{Connection, Row};

use crate::core::types::ColumnMeta;
use crate::error::{AppError, AppResult};

pub fn list_tables(conn: &Connection) -> AppResult<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )?;
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn list_columns(conn: &Connection, table: &str) -> AppResult<Vec<ColumnMeta>> {
    // Use PRAGMA table_info; table name is not parameterizable in SQLite, so we must
    // validate it as an identifier to prevent injection.
    if !is_safe_identifier(table) {
        return Err(AppError::InvalidRequest(format!(
            "invalid table identifier: {table}"
        )));
    }

    let sql = format!("PRAGMA table_info({table})");
    let mut stmt = conn.prepare(&sql)?;
    let cols = stmt
        .query_map([], |row: &Row<'_>| {
            let name: String = row.get("name")?;
            let decl_type: Option<String> = row.get("type")?;
            Ok(ColumnMeta {
                name,
                decl_type,
                sqlite_type: None,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(cols)
}

pub(crate) fn is_safe_identifier(s: &str) -> bool {
    // Minimal safe subset: [A-Za-z_][A-Za-z0-9_]*
    let mut chars = s.chars();
    let Some(first) = chars.next() else { return false };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

pub(crate) fn is_safe_table_ref(s: &str) -> bool {
    // Allow either `table` or `schema.table`, both segments must be safe identifiers.
    let mut parts = s.split('.');
    let Some(first) = parts.next() else { return false };
    if !is_safe_identifier(first) {
        return false;
    }
    match parts.next() {
        None => true,
        Some(second) => {
            // Only allow one dot for now; keep it simple.
            if parts.next().is_some() {
                return false;
            }
            is_safe_identifier(second)
        }
    }
}

