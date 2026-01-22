use rusqlite::{ffi, Connection};

use crate::error::AppResult;

pub fn is_sql_readonly(conn: &Connection, sql: &str) -> AppResult<bool> {
    let mut stmt = conn.prepare(sql)?;
    // SAFETY: rusqlite Statement exposes a valid sqlite3_stmt pointer.
    let ptr = stmt.as_ptr();
    let ro = unsafe { ffi::sqlite3_stmt_readonly(ptr) };
    Ok(ro != 0)
}

