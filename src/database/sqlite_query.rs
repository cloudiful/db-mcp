use anyhow::Result;
use base64::Engine;
use futures_util::TryStreamExt;
use serde_json::{Map, Value};
use sqlx::pool::PoolConnection;
use sqlx::sqlite::{SqliteConnection, SqlitePool, SqliteRow};
use sqlx::{Column, Executor, Row, SqlSafeStr, TypeInfo, ValueRef};
use std::time::Instant;

use super::{
    CLEANUP_TIMEOUT, Cancellation, Deadline, Limits, Operation, QueryColumn, QueryResult,
    RowCollector, SQLITE_PROGRESS_INTERVAL, attach_cancellation, cancellation_report, empty_result,
    timeout_operation,
};

// SQLx checked query macros resolve one database per crate from the compile-time
// DATABASE_URL, so SQLite probes cannot use query_file! in a mixed-backend crate.
// They stay in .sql files and are loaded at runtime instead.
const PING_SQL: &str = include_str!("../../sql/sqlite/ping.sql");

pub async fn ping(pool: &SqlitePool) -> Result<i64> {
    Ok(sqlx::query(sqlx::AssertSqlSafe(PING_SQL))
        .fetch_one(pool)
        .await?
        .try_get::<i64, _>(0)?)
}

pub async fn collect_rows(
    pool: &SqlitePool,
    sql: &str,
    requested_max_rows: usize,
    limits: &Limits,
) -> Result<QueryResult> {
    let deadline = limits.deadline();
    collect_rows_within(pool, sql, requested_max_rows, limits, deadline).await
}

pub async fn collect_rows_within(
    pool: &SqlitePool,
    sql: &str,
    requested_max_rows: usize,
    limits: &Limits,
    deadline: Deadline,
) -> Result<QueryResult> {
    let mut conn = deadline
        .run(Operation::Acquire, "sqlite acquire", pool.acquire())
        .await?;
    // Driver-level cancellation: SQLite invokes the progress handler during
    // long-running VM steps. It is armed just inside the client deadline, so an
    // interrupt normally confirms the stop before the client timeout drops the
    // in-flight future. The handle is locked while the handler is installed so
    // the worker is quiescent.
    {
        let mut handle = deadline
            .run(
                Operation::Configure,
                "sqlite lock handle",
                conn.lock_handle(),
            )
            .await?;
        handle.set_progress_handler(SQLITE_PROGRESS_INTERVAL, move || {
            !deadline.interrupt_expired()
        });
    }
    let result =
        execute_sql_with_connection(&mut conn, sql, requested_max_rows, limits, &deadline).await;
    let unconfirmed = result.as_ref().err().is_some_and(|error| {
        matches!(
            cancellation_report(error).map(|report| report.cancellation()),
            Some(Cancellation::Unconfirmed)
        )
    });
    if unconfirmed {
        // The client-side timeout dropped the future while the worker may still
        // be stepping. Leave the handler armed so it stops the worker, and
        // discard the connection instead of returning it to the pool.
        evict_connection(conn).await;
    } else {
        // The command completed and the handler is cleared while the worker is
        // locked out, so the connection can be reused cleanly. A failed or
        // timed-out lock means the worker is unusable, so the connection is
        // discarded.
        let cleared = match tokio::time::timeout(CLEANUP_TIMEOUT, conn.lock_handle()).await {
            Ok(Ok(mut handle)) => {
                handle.remove_progress_handler();
                true
            }
            Ok(Err(_)) | Err(_) => false,
        };
        if !cleared {
            evict_connection(conn).await;
        }
    }
    result
}

pub async fn execute_sql_with_connection(
    conn: &mut SqliteConnection,
    sql: &str,
    requested_max_rows: usize,
    limits: &Limits,
    deadline: &Deadline,
) -> Result<QueryResult> {
    let describe = deadline
        .run(
            Operation::Describe,
            "sqlite describe",
            (&mut *conn).describe(sqlx::AssertSqlSafe(sql).into_sql_str()),
        )
        .await
        .map_err(|error| classify_deadline_error(error, Operation::Describe, false))?;
    if describe.columns().is_empty() {
        let started = Instant::now();
        let result = deadline
            .run(
                Operation::Execute,
                "query",
                (&mut *conn).execute(sqlx::AssertSqlSafe(sql)),
            )
            .await
            .map_err(|error| classify_deadline_error(error, Operation::Execute, true))?;
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
    collect_rows_with_connection(
        &mut *conn,
        sql,
        requested_max_rows,
        limits,
        deadline,
        columns,
    )
    .await
    .map_err(|error| classify_deadline_error(error, Operation::Fetch, false))
}

async fn collect_rows_with_connection(
    conn: &mut SqliteConnection,
    sql: &str,
    requested_max_rows: usize,
    limits: &Limits,
    deadline: &Deadline,
    columns: Vec<QueryColumn>,
) -> Result<QueryResult> {
    let applied_max_rows = limits.normalize_max_rows(requested_max_rows);
    let started = Instant::now();
    let mut collector = RowCollector::new(applied_max_rows, started);
    collector.set_columns(columns);
    let mut row_stream = sqlx::query(sqlx::AssertSqlSafe(sql)).fetch(&mut *conn);

    loop {
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
        if collector.push(sqlite_row_to_json(&row)?, limits)? {
            break;
        }
    }

    Ok(collector.finish())
}

/// SQLite extended result code reported when the progress handler aborts the
/// statement (SQLITE_INTERRUPT). `sqlx` exposes the numeric code, not a typed
/// variant.
const SQLITE_INTERRUPT_CODE: &str = "9";

fn is_sqlite_interrupt(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause.downcast_ref::<sqlx::Error>().is_some_and(|error| {
            matches!(
                error,
                sqlx::Error::Database(database)
                    if database.code().as_deref() == Some(SQLITE_INTERRUPT_CODE)
            )
        })
    })
}

/// Classify a stage error once the logical deadline was reached. A driver
/// interrupt is a confirmed stop; a client-side expiry leaves the statement
/// state unknown. Ordinary errors pass through untouched.
fn classify_deadline_error(
    error: anyhow::Error,
    operation: Operation,
    result_unknown: bool,
) -> anyhow::Error {
    if is_sqlite_interrupt(&error) {
        attach_cancellation(error, operation, Cancellation::Confirmed, result_unknown)
    } else if timeout_operation(&error).is_some() {
        attach_cancellation(error, operation, Cancellation::Unconfirmed, result_unknown)
    } else {
        error
    }
}

async fn evict_connection(conn: PoolConnection<sqlx::Sqlite>) {
    let _ = tokio::time::timeout(CLEANUP_TIMEOUT, conn.close()).await;
}

fn sqlite_row_to_json(row: &SqliteRow) -> Result<Map<String, Value>> {
    let mut map = Map::new();
    for (idx, col) in row.columns().iter().enumerate() {
        let raw = row.try_get_raw(idx)?;
        let value = if raw.is_null() {
            Value::Null
        } else {
            match raw.type_info().name() {
                "TEXT" | "DATE" | "TIME" | "DATETIME" | "NUMERIC" => {
                    Value::String(row.try_get::<String, _>(idx)?)
                }
                "INTEGER" | "INT4" => Value::from(row.try_get::<i64, _>(idx)?),
                "REAL" => Value::from(row.try_get::<f64, _>(idx)?),
                "BOOLEAN" => Value::Bool(row.try_get::<bool, _>(idx)?),
                "BLOB" => {
                    let bytes: Vec<u8> = row.try_get(idx)?;
                    Value::String(base64::engine::general_purpose::STANDARD.encode(bytes))
                }
                _ => Value::String(row.try_get::<String, _>(idx)?),
            }
        };
        map.insert(col.name().to_string(), value);
    }
    Ok(map)
}
