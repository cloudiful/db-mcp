use anyhow::{Context, Result, bail};
use futures_util::TryStreamExt;
use serde_json::Value;
use sqlx::postgres::{PgConnection, PgPool};
use sqlx::{Column, Executor, Row, SqlSafeStr, TypeInfo};
use std::time::Instant;

use super::{
    ExplainResultData, Limits, QueryColumn, QueryResult, RowCollector, empty_result,
    run_with_timeout,
};

pub async fn execute_query(
    pool: &PgPool,
    read_only: bool,
    sql: &str,
    requested_max_rows: usize,
    limits: &Limits,
) -> Result<QueryResult> {
    let mut tx = run_with_timeout(limits, "postgres begin", pool.begin()).await?;
    if read_only {
        run_with_timeout(
            limits,
            "postgres set read only",
            tx.execute(sqlx::query_file!(
                "sql/postgres/set_transaction_read_only.sql"
            )),
        )
        .await?;
        let result = execute_sql_with_connection(&mut tx, sql, requested_max_rows, limits).await;
        let _ = tx.rollback().await;
        result
    } else {
        let result = execute_sql_with_connection(&mut tx, sql, requested_max_rows, limits).await;
        run_with_timeout(limits, "postgres commit", tx.commit()).await?;
        result
    }
}

pub async fn execute_sql_with_connection(
    conn: &mut PgConnection,
    sql: &str,
    requested_max_rows: usize,
    limits: &Limits,
) -> Result<QueryResult> {
    let describe = run_with_timeout(
        limits,
        "postgres describe",
        (&mut *conn).describe(sqlx::AssertSqlSafe(sql).into_sql_str()),
    )
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

    let columns = describe
        .columns()
        .iter()
        .take(limits.max_columns)
        .map(|col| QueryColumn {
            name: col.name().to_string(),
            data_type: col.type_info().name().to_string(),
        })
        .collect();
    execute_query_rows(&mut *conn, sql, requested_max_rows, limits, columns).await
}

pub async fn explain_query(
    pool: &PgPool,
    read_only: bool,
    sql: &str,
    limits: &Limits,
) -> Result<ExplainResultData> {
    let mut tx = run_with_timeout(limits, "postgres begin", pool.begin()).await?;
    if read_only {
        run_with_timeout(
            limits,
            "postgres set read only",
            tx.execute(sqlx::query_file!(
                "sql/postgres/set_transaction_read_only.sql"
            )),
        )
        .await?;
    }
    let rows = run_with_timeout(
        limits,
        "postgres explain",
        sqlx::query(sqlx::AssertSqlSafe(format!("EXPLAIN {sql}"))).fetch_all(&mut *tx),
    )
    .await?;
    let mut lines = vec![];
    let mut total_chars = 0usize;
    let mut truncated = false;
    for row in rows.into_iter().take(limits.explain_max_lines) {
        let line: String = row.try_get::<String, _>(0).unwrap_or_default();
        total_chars += line.chars().count();
        if total_chars > limits.explain_max_total_chars {
            truncated = true;
            break;
        }
        lines.push(line);
    }
    tx.rollback().await?;
    Ok(ExplainResultData {
        format: "text".to_string(),
        plan_lines: lines,
        notice: truncated.then(|| "explain output truncated to protect MCP context".to_string()),
    })
}

async fn execute_query_rows(
    conn: &mut PgConnection,
    sql: &str,
    requested_max_rows: usize,
    limits: &Limits,
    columns: Vec<QueryColumn>,
) -> Result<QueryResult> {
    let applied_max_rows = limits.normalize_max_rows(requested_max_rows);
    let started = Instant::now();
    let fetch_limit = applied_max_rows.saturating_add(1);
    // The newline before the closing paren keeps a trailing `-- comment`
    // from commenting out the wrapper's tail.
    let wrapped = format!(
        "select row_to_json(__dbmcp_row)::text as __dbmcp_row_json from ({sql}\n) as __dbmcp_row limit {fetch_limit}"
    );
    let mut collector = RowCollector::new(applied_max_rows, started);
    collector.set_columns(columns);
    let mut row_stream = sqlx::query(sqlx::AssertSqlSafe(wrapped)).fetch(&mut *conn);
    // Bound the whole stream, not each row: the deadline is relative to the
    // start of streaming so a slow query cannot drip rows forever.
    let deadline = tokio::time::Instant::now() + limits.query_timeout;

    while let Some(row) = tokio::time::timeout_at(deadline, row_stream.try_next())
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

        if collector.push(raw_map, limits)? {
            break;
        }
    }

    Ok(collector.finish())
}
