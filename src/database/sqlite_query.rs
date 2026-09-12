use anyhow::Result;
use base64::Engine;
use futures_util::TryStreamExt;
use serde_json::{Map, Value};
use sqlx::pool::PoolConnection;
use sqlx::sqlite::{SqliteConnection, SqlitePool, SqliteRow};
use sqlx::{Column, Executor, Row, SqlSafeStr, TypeInfo, ValueRef};
use std::time::Instant;

use super::{
    CLEANUP_TIMEOUT, Deadline, Limits, Operation, QueryColumn, QueryResult, RowCollector,
    empty_result, timeout_operation,
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
    let result =
        execute_sql_with_connection(&mut conn, sql, requested_max_rows, limits, &deadline).await;
    if let Err(err) = &result
        && timeout_operation(err).is_some()
    {
        // A driver-level timeout leaves the statement state unknown on this
        // physical connection; close it instead of returning it to the pool.
        evict_connection(conn).await;
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
    collect_rows_with_connection(
        &mut *conn,
        sql,
        requested_max_rows,
        limits,
        deadline,
        columns,
    )
    .await
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
