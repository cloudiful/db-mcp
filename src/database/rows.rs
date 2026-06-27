use anyhow::{Context, Result};
use base64::Engine;
use futures_util::TryStreamExt;
use serde_json::{Map, Value};
use sqlx::{
    Any, AnyConnection, AnyPool, Column, Executor, Row, TypeInfo, ValueRef, any::AnyQueryResult,
    any::AnyRow,
};
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

pub fn any_row_to_json(row: &AnyRow) -> Result<Map<String, Value>> {
    let mut map = Map::new();
    for (idx, col) in row.columns().iter().enumerate() {
        let raw = row.try_get_raw(idx)?;
        let value = if raw.is_null() {
            Value::Null
        } else {
            use sqlx::any::AnyTypeInfoKind::*;
            match raw.type_info().kind() {
                Bool => Value::Bool(row.try_get::<bool, _>(idx)?),
                SmallInt | Integer | BigInt => Value::from(row.try_get::<i64, _>(idx)?),
                Real | Double => Value::from(row.try_get::<f64, _>(idx)?),
                Text => Value::String(row.try_get::<String, _>(idx)?),
                Blob => {
                    let bytes: Vec<u8> = row.try_get(idx)?;
                    Value::String(base64::engine::general_purpose::STANDARD.encode(bytes))
                }
                Null => Value::Null,
            }
        };
        map.insert(col.name().to_string(), value);
    }
    Ok(map)
}

pub async fn collect_rows(
    pool: &AnyPool,
    sql: &str,
    requested_max_rows: usize,
    limits: &Limits,
) -> Result<QueryResult> {
    let mut conn = pool.acquire().await?;
    execute_sql_with_connection(&mut conn, sql, requested_max_rows, limits).await
}

pub async fn execute_sql_with_connection(
    conn: &mut AnyConnection,
    sql: &str,
    requested_max_rows: usize,
    limits: &Limits,
) -> Result<QueryResult> {
    let describe = (&mut *conn).describe(sql).await?;
    if describe.columns().is_empty() {
        let started = Instant::now();
        let result = tokio::time::timeout(limits.query_timeout, (&mut *conn).execute(sql))
            .await
            .context("query timeout")??;
        return Ok(empty_result_from_query_result(result, limits, started));
    }

    collect_rows_with_executor(&mut *conn, sql, requested_max_rows, limits).await
}

pub async fn collect_rows_with_executor<'e, E>(
    executor: E,
    sql: &str,
    requested_max_rows: usize,
    limits: &Limits,
) -> Result<QueryResult>
where
    E: Executor<'e, Database = Any>,
{
    let applied_max_rows = limits.normalize_max_rows(requested_max_rows);
    let started = Instant::now();
    let mut rows = vec![];
    let mut row_count = 0usize;
    let mut columns = vec![];
    let mut row_stream = sqlx::query(sql).fetch(executor);
    let mut truncated = false;
    let mut reasons = vec![];
    let mut total_chars = 0usize;

    while let Some(row) = tokio::time::timeout(limits.query_timeout, row_stream.try_next())
        .await
        .context("query timeout")??
    {
        if rows.len() >= applied_max_rows {
            truncated = true;
            reasons.push("row limit reached".to_string());
            break;
        }

        if columns.is_empty() {
            columns = row
                .columns()
                .iter()
                .take(limits.max_columns)
                .map(|col| QueryColumn {
                    name: col.name().to_string(),
                    data_type: col.type_info().name().to_string(),
                })
                .collect();
        }

        let mut row_map = Map::new();
        let raw_map = any_row_to_json(&row)?;
        for (key, value) in raw_map.into_iter().take(limits.max_columns) {
            let (value, was_truncated) = serialize_value(&value, limits.max_cell_chars);
            if was_truncated {
                truncated = true;
                reasons.push(format!("cell {key} truncated"));
            }
            total_chars += serde_json::to_string(&value)?.chars().count();
            if total_chars > limits.max_total_chars {
                truncated = true;
                reasons.push("total character limit reached".to_string());
                break;
            }
            row_map.insert(key, value);
        }
        rows.push(row_map);
        row_count += 1;
        if total_chars > limits.max_total_chars {
            break;
        }
    }

    Ok(QueryResult {
        columns,
        rows,
        row_count,
        applied_max_rows,
        truncated,
        truncation_reasons: reasons,
        notice: if truncated {
            Some("result truncated to protect MCP context".to_string())
        } else {
            None
        },
        execution_time: format!("{:?}", started.elapsed()),
    })
}

fn empty_result_from_query_result(
    result: AnyQueryResult,
    limits: &Limits,
    started: Instant,
) -> QueryResult {
    QueryResult {
        columns: vec![],
        rows: vec![],
        row_count: result.rows_affected() as usize,
        applied_max_rows: limits.normalize_max_rows(0),
        truncated: false,
        truncation_reasons: vec![],
        notice: None,
        execution_time: format!("{:?}", started.elapsed()),
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
