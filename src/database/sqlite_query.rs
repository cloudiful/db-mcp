use anyhow::{Context, Result};
use base64::Engine;
use futures_util::TryStreamExt;
use serde_json::{Map, Value};
use sqlx::sqlite::SqliteRow;
use sqlx::{Column, Executor, Row, SqlSafeStr, SqliteConnection, SqlitePool, TypeInfo, ValueRef};
use std::time::Instant;

use super::{Limits, QueryColumn, QueryResult, RowCollector, empty_result, run_with_timeout};

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
    let mut conn = run_with_timeout(limits, "sqlite acquire", pool.acquire()).await?;
    execute_sql_with_connection(&mut conn, sql, requested_max_rows, limits).await
}

pub async fn execute_sql_with_connection(
    conn: &mut SqliteConnection,
    sql: &str,
    requested_max_rows: usize,
    limits: &Limits,
) -> Result<QueryResult> {
    let describe = run_with_timeout(
        limits,
        "sqlite describe",
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

    collect_rows_with_connection(&mut *conn, sql, requested_max_rows, limits).await
}

async fn collect_rows_with_connection(
    conn: &mut SqliteConnection,
    sql: &str,
    requested_max_rows: usize,
    limits: &Limits,
) -> Result<QueryResult> {
    let applied_max_rows = limits.normalize_max_rows(requested_max_rows);
    let started = Instant::now();
    let mut collector = RowCollector::new(applied_max_rows, started);
    let mut row_stream = sqlx::query(sqlx::AssertSqlSafe(sql)).fetch(&mut *conn);

    while let Some(row) = tokio::time::timeout(limits.query_timeout, row_stream.try_next())
        .await
        .context("query timeout")??
    {
        if collector.at_row_limit() {
            collector.mark_row_limit();
            break;
        }
        if collector.columns().is_empty() {
            collector.set_columns(
                row.columns()
                    .iter()
                    .take(limits.max_columns)
                    .map(|col| QueryColumn {
                        name: col.name().to_string(),
                        data_type: col.type_info().name().to_string(),
                    })
                    .collect(),
            );
        }
        if collector.push(sqlite_row_to_json(&row)?, limits)? {
            break;
        }
    }

    Ok(collector.finish())
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
