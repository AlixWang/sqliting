use super::types::Limits;

pub fn effective_limit(requested: Option<usize>, max_rows: usize) -> Limits {
    let max_rows = requested.unwrap_or(max_rows).min(max_rows).max(1);
    Limits { max_rows }
}

