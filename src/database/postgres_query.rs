use anyhow::{Context, Result, bail};
use futures_util::TryStreamExt;
use serde_json::{Map, Value};
use sqlx::{AnyPool, Executor, Row};

use super::{
    ExplainResultData, Limits, QueryColumn, QueryResult, execute_sql_with_connection,
    serialize_value,
};

pub async fn execute_query(
    pool: &AnyPool,
    read_only: bool,
    sql: &str,
    requested_max_rows: usize,
    limits: &Limits,
) -> Result<QueryResult> {
    let mut tx = pool.begin().await?;
    if read_only {
        let sql = format!("select * from ({sql}) as __dbmcp_read_only_guard");
        tx.execute("set transaction read only").await?;
        let result = execute_query_rows(&mut *tx, &sql, requested_max_rows, limits).await;
        let _ = tx.rollback().await;
        result
    } else {
        let result = execute_sql_with_connection(&mut *tx, sql, requested_max_rows, limits).await;
        tx.commit().await?;
        result
    }
}

pub async fn explain_query(
    pool: &AnyPool,
    read_only: bool,
    sql: &str,
    limits: &Limits,
) -> Result<ExplainResultData> {
    let mut tx = pool.begin().await?;
    if read_only {
        tx.execute("set transaction read only").await?;
    }
    let rows = sqlx::query(&format!("EXPLAIN {sql}"))
        .fetch_all(&mut *tx)
        .await?;
    let mut lines = vec![];
    for row in rows.into_iter().take(limits.explain_max_lines) {
        let line: String = row.try_get::<String, _>(0).unwrap_or_default();
        lines.push(line);
    }
    tx.rollback().await?;
    Ok(ExplainResultData {
        format: "text".to_string(),
        plan_lines: lines,
        notice: None,
    })
}

async fn execute_query_rows<'e, E>(
    executor: E,
    sql: &str,
    requested_max_rows: usize,
    limits: &Limits,
) -> Result<QueryResult>
where
    E: Executor<'e, Database = sqlx::Any>,
{
    let applied_max_rows = limits.normalize_max_rows(requested_max_rows);
    let started = std::time::Instant::now();
    let fetch_limit = applied_max_rows.saturating_add(1);
    let wrapped = format!(
        "select row_to_json(__dbmcp_row)::text as __dbmcp_row_json from ({sql}) as __dbmcp_row limit {fetch_limit}"
    );
    let mut row_stream = sqlx::query(&wrapped).fetch(executor);
    let mut rows = vec![];
    let mut columns = vec![];
    let mut row_count = 0usize;
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

        let row_json: String = row.try_get("__dbmcp_row_json")?;
        let raw_map = match serde_json::from_str::<Value>(&row_json)? {
            Value::Object(map) => map,
            other => bail!("postgres row_to_json returned non-object value: {other}"),
        };

        if columns.is_empty() {
            columns = raw_map
                .iter()
                .take(limits.max_columns)
                .map(|(name, value)| QueryColumn {
                    name: name.clone(),
                    data_type: json_value_type(value).to_string(),
                })
                .collect();
        }

        let mut row_map = Map::new();
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

fn json_value_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}
