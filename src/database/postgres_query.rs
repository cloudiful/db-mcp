use anyhow::{Result, bail};
use futures_util::TryStreamExt;
use serde_json::Value;
use sqlx::pool::PoolConnection;
use sqlx::postgres::{PgConnection, PgPool};
use sqlx::{Column, Connection, Executor, Row, SqlSafeStr, TypeInfo};
use std::time::Instant;

use super::{
    CLEANUP_TIMEOUT, Deadline, ExplainResultData, Limits, Operation, QueryColumn, QueryResult,
    RowCollector, empty_result,
};

pub async fn execute_query(
    pool: &PgPool,
    read_only: bool,
    sql: &str,
    requested_max_rows: usize,
    limits: &Limits,
) -> Result<QueryResult> {
    let deadline = limits.deadline();
    execute_query_within(pool, read_only, sql, requested_max_rows, limits, deadline).await
}

pub async fn execute_query_within(
    pool: &PgPool,
    read_only: bool,
    sql: &str,
    requested_max_rows: usize,
    limits: &Limits,
    deadline: Deadline,
) -> Result<QueryResult> {
    let mut conn = deadline
        .run(Operation::Acquire, "postgres acquire", pool.acquire())
        .await?;
    let mut tx = deadline
        .run(Operation::Begin, "postgres begin", conn.begin())
        .await?;

    let outcome = async {
        if read_only {
            deadline
                .run(
                    Operation::Metadata,
                    "postgres set read only",
                    tx.execute(sqlx::query_file!(
                        "sql/postgres/set_transaction_read_only.sql"
                    )),
                )
                .await?;
        }
        execute_sql_with_connection(&mut tx, sql, requested_max_rows, limits, &deadline).await
    }
    .await;

    match outcome {
        Err(err) => {
            // A failed statement must never be committed. Bound the rollback;
            // an unconfirmed rollback leaves the physical connection in an
            // unknown state, so it is closed instead of returned to the pool.
            if deadline
                .run(Operation::Rollback, "postgres rollback", tx.rollback())
                .await
                .is_err()
            {
                evict_connection(conn).await;
            }
            Err(err)
        }
        Ok(value) if read_only => {
            if deadline
                .run(Operation::Rollback, "postgres rollback", tx.rollback())
                .await
                .is_err()
            {
                evict_connection(conn).await;
            }
            Ok(value)
        }
        Ok(value) => {
            if let Err(err) = deadline
                .run(Operation::Commit, "postgres commit", tx.commit())
                .await
            {
                // The commit outcome is unknown; never hand the connection back
                // to the pool in an unconfirmed state.
                evict_connection(conn).await;
                Err(err)
            } else {
                Ok(value)
            }
        }
    }
}

pub async fn execute_sql_with_connection(
    conn: &mut PgConnection,
    sql: &str,
    requested_max_rows: usize,
    limits: &Limits,
    deadline: &Deadline,
) -> Result<QueryResult> {
    let describe = deadline
        .run(
            Operation::Describe,
            "postgres describe",
            (&mut *conn).describe(sqlx::AssertSqlSafe(sql).into_sql_str()),
        )
        .await?;
    if describe.columns().is_empty() {
        let started = Instant::now();
        let result = deadline
            .run(
                Operation::Execute,
                "query",
                (&mut *conn).execute(sqlx::AssertSqlSafe(sql)),
            )
            .await?;
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
    execute_query_rows(
        &mut *conn,
        sql,
        requested_max_rows,
        limits,
        deadline,
        columns,
    )
    .await
}

pub async fn explain_query(
    pool: &PgPool,
    read_only: bool,
    sql: &str,
    limits: &Limits,
) -> Result<ExplainResultData> {
    let deadline = limits.deadline();
    explain_query_within(pool, read_only, sql, limits, deadline).await
}

pub async fn explain_query_within(
    pool: &PgPool,
    read_only: bool,
    sql: &str,
    limits: &Limits,
    deadline: Deadline,
) -> Result<ExplainResultData> {
    let mut conn = deadline
        .run(Operation::Acquire, "postgres acquire", pool.acquire())
        .await?;
    let mut tx = deadline
        .run(Operation::Begin, "postgres begin", conn.begin())
        .await?;

    let outcome = async {
        if read_only {
            deadline
                .run(
                    Operation::Metadata,
                    "postgres set read only",
                    tx.execute(sqlx::query_file!(
                        "sql/postgres/set_transaction_read_only.sql"
                    )),
                )
                .await?;
        }
        let rows = deadline
            .run(
                Operation::Explain,
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
        Ok(ExplainResultData {
            format: "text".to_string(),
            plan_lines: lines,
            notice: truncated
                .then(|| "explain output truncated to protect MCP context".to_string()),
        })
    }
    .await;

    // EXPLAIN never commits; always roll back so any plan writes are discarded.
    // A failed rollback leaves the connection state unconfirmed.
    if deadline
        .run(Operation::Rollback, "postgres rollback", tx.rollback())
        .await
        .is_err()
    {
        evict_connection(conn).await;
    }
    outcome
}

async fn execute_query_rows(
    conn: &mut PgConnection,
    sql: &str,
    requested_max_rows: usize,
    limits: &Limits,
    deadline: &Deadline,
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

    loop {
        // Bound the whole stream with the shared deadline, not each row: a slow
        // query cannot drip rows forever.
        let next = deadline
            .run(Operation::Fetch, "query", row_stream.try_next())
            .await?;
        let Some(row) = next else {
            break;
        };
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

/// Close a physical connection whose transaction state cannot be confirmed so
/// it is never reused. The close itself is bounded.
async fn evict_connection(conn: PoolConnection<sqlx::Postgres>) {
    let _ = tokio::time::timeout(CLEANUP_TIMEOUT, conn.close()).await;
}
