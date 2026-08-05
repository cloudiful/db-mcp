use anyhow::Result;
use serde_json::{Map, Value};
use std::time::Instant;

use super::{Limits, QueryColumn, QueryResult};

// MCP callers often send SQL copied from consoles with a trailing terminator.
// We accept that for single statements, but still do not support multi-statement scripts.
pub fn strip_trailing_sql_terminator(sql: &str) -> &str {
    let trimmed = sql.trim_end();
    if trimmed.ends_with(';') {
        trimmed.trim_end_matches(';').trim_end()
    } else {
        trimmed
    }
}

pub fn maybe_truncate_text(value: String, max_chars: usize) -> (String, bool) {
    if value.chars().count() <= max_chars {
        return (value, false);
    }
    let truncated = value.chars().take(max_chars).collect::<String>();
    (truncated, true)
}

pub fn serialize_value(value: &Value, max_chars: usize) -> (Value, bool) {
    match value {
        Value::String(s) => {
            let (s, truncated) = maybe_truncate_text(s.clone(), max_chars);
            (Value::String(s), truncated)
        }
        _ => (value.clone(), false),
    }
}

pub fn empty_result(rows_affected: usize, limits: &Limits, started: Instant) -> QueryResult {
    QueryResult {
        columns: vec![],
        rows: vec![],
        row_count: rows_affected,
        applied_max_rows: limits.normalize_max_rows(0),
        truncated: false,
        truncation_reasons: vec![],
        notice: None,
        execution_time: format!("{:?}", started.elapsed()),
    }
}

/// Accumulates fetched rows while applying the shared truncation accounting:
/// per-cell character limits, a global character budget, and the row limit.
pub struct RowCollector {
    started: Instant,
    applied_max_rows: usize,
    columns: Vec<QueryColumn>,
    rows: Vec<Map<String, Value>>,
    row_count: usize,
    truncated: bool,
    reasons: Vec<String>,
    total_chars: usize,
}

impl RowCollector {
    pub fn new(applied_max_rows: usize, started: Instant) -> Self {
        Self {
            started,
            applied_max_rows,
            columns: vec![],
            rows: vec![],
            row_count: 0,
            truncated: false,
            reasons: vec![],
            total_chars: 0,
        }
    }

    pub fn at_row_limit(&self) -> bool {
        self.rows.len() >= self.applied_max_rows
    }

    pub fn mark_row_limit(&mut self) {
        self.truncated = true;
        self.reasons.push("row limit reached".to_string());
    }

    pub fn columns(&self) -> &[QueryColumn] {
        &self.columns
    }

    pub fn set_columns(&mut self, columns: Vec<QueryColumn>) {
        self.columns = columns;
    }

    pub fn push(&mut self, raw: Map<String, Value>, limits: &Limits) -> Result<bool> {
        let mut row_map = Map::new();
        for (key, value) in raw.into_iter().take(limits.max_columns) {
            let (value, was_truncated) = serialize_value(&value, limits.max_cell_chars);
            if was_truncated {
                self.truncated = true;
                self.reasons.push(format!("cell {key} truncated"));
            }
            self.total_chars += serde_json::to_string(&value)?.chars().count();
            if self.total_chars > limits.max_total_chars {
                self.truncated = true;
                self.reasons
                    .push("total character limit reached".to_string());
                break;
            }
            row_map.insert(key, value);
        }
        self.rows.push(row_map);
        self.row_count += 1;
        Ok(self.total_chars > limits.max_total_chars)
    }

    pub fn finish(self) -> QueryResult {
        QueryResult {
            columns: self.columns,
            rows: self.rows,
            row_count: self.row_count,
            applied_max_rows: self.applied_max_rows,
            truncated: self.truncated,
            truncation_reasons: self.reasons,
            notice: if self.truncated {
                Some("result truncated to protect MCP context".to_string())
            } else {
                None
            },
            execution_time: format!("{:?}", self.started.elapsed()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::strip_trailing_sql_terminator;

    #[test]
    fn strips_trailing_semicolon_and_whitespace() {
        assert_eq!(strip_trailing_sql_terminator("select 1;   \n"), "select 1");
    }

    #[test]
    fn keeps_query_without_trailing_semicolon() {
        assert_eq!(strip_trailing_sql_terminator("select ';'"), "select ';'");
    }

    #[test]
    fn strips_multiple_trailing_semicolons() {
        assert_eq!(strip_trailing_sql_terminator("select 1;;"), "select 1");
    }
}
