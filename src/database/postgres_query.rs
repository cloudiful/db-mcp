use anyhow::{Context, Result, bail};
use futures_util::TryStreamExt;
use serde_json::Value;
use sqlx::postgres::{PgConnection, PgPool};
use sqlx::{Executor, Row, SqlSafeStr};
use std::time::Instant;

use super::{ExplainResultData, Limits, QueryColumn, QueryResult, RowCollector, empty_result};

pub async fn execute_query(
    pool: &PgPool,
    read_only: bool,
    sql: &str,
    requested_max_rows: usize,
    limits: &Limits,
) -> Result<QueryResult> {
    let mut tx = pool.begin().await?;
    if read_only {
        tx.execute(sqlx::query_file!(
            "sql/postgres/set_transaction_read_only.sql"
        ))
        .await?;
        let result = execute_sql_with_connection(&mut tx, sql, requested_max_rows, limits).await;
        let _ = tx.rollback().await;
        result
    } else {
        let result = execute_sql_with_connection(&mut tx, sql, requested_max_rows, limits).await;
        tx.commit().await?;
        result
    }
}

pub async fn execute_sql_with_connection(
    conn: &mut PgConnection,
    sql: &str,
    requested_max_rows: usize,
    limits: &Limits,
) -> Result<QueryResult> {
    let describe = (&mut *conn)
        .describe(sqlx::AssertSqlSafe(sql).into_sql_str())
        .await?;
    if describe.columns().is_empty() {
        let started = Instant::now();
        let result = tokio::time::timeout(
            limits.query_timeout,
            (&mut *conn).execute(sqlx::AssertSqlSafe(sql)),
        )
        .await
        .context("query timeout")??;
        return Ok(empty_result(
            result.rows_affected() as usize,
            limits,
            started,
        ));
    }

    execute_query_rows(&mut *conn, sql, requested_max_rows, limits).await
}

pub async fn explain_query(
    pool: &PgPool,
    read_only: bool,
    sql: &str,
    limits: &Limits,
) -> Result<ExplainResultData> {
    let mut tx = pool.begin().await?;
    if read_only {
        tx.execute(sqlx::query_file!(
            "sql/postgres/set_transaction_read_only.sql"
        ))
        .await?;
    }
    let rows = sqlx::query(sqlx::AssertSqlSafe(format!("EXPLAIN {sql}")))
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

async fn execute_query_rows(
    conn: &mut PgConnection,
    sql: &str,
    requested_max_rows: usize,
    limits: &Limits,
) -> Result<QueryResult> {
    let applied_max_rows = limits.normalize_max_rows(requested_max_rows);
    let started = Instant::now();
    let fetch_limit = applied_max_rows.saturating_add(1);
    let wrapped = format!(
        "select row_to_json(__dbmcp_row)::text as __dbmcp_row_json from ({sql}) as __dbmcp_row limit {fetch_limit}"
    );
    let mut collector = RowCollector::new(applied_max_rows, started);
    let mut row_stream = sqlx::query(sqlx::AssertSqlSafe(wrapped)).fetch(&mut *conn);

    while let Some(row) = tokio::time::timeout(limits.query_timeout, row_stream.try_next())
        .await
        .context("query timeout")??
    {
        if collector.at_row_limit() {
            collector.mark_row_limit();
            break;
        }

        let row_json: String = row.try_get("__dbmcp_row_json")?;
        let raw_map = match serde_json::from_str::<Value>(&row_json)? {
            Value::Object(map) => map,
            other => bail!("postgres row_to_json returned non-object value: {other}"),
        };

        if collector.columns().is_empty() {
            collector.set_columns(
                raw_map
                    .iter()
                    .take(limits.max_columns)
                    .map(|(name, value)| QueryColumn {
                        name: name.clone(),
                        data_type: json_value_type(value).to_string(),
                    })
                    .collect(),
            );
        }

        if collector.push(raw_map, limits)? {
            break;
        }
    }

    Ok(collector.finish())
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
