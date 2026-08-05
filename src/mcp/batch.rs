use anyhow::Result;

use crate::{database, mcp_types::*};

pub(super) async fn execute_query_batch(
    conn: &dyn database::DatabaseConnection,
    input: ExecuteQueriesInput,
    limits: &database::Limits,
) -> Result<ExecuteQueriesOutput> {
    let statement_count = input.queries.len();
    if !(1..=MAX_BATCH_QUERIES).contains(&statement_count) {
        anyhow::bail!("queries must contain between 1 and {MAX_BATCH_QUERIES} statements")
    }
    if !conn.supports_read_only_batch() {
        anyhow::bail!(
            "execute_queries requires a read-only connection; SQLite connections also require a read-only DSN such as mode=ro"
        )
    }

    let connection = conn.info().name;
    let mut results = Vec::with_capacity(statement_count);
    for (index, query) in input.queries.into_iter().enumerate() {
        match conn
            .execute_query(&query.sql, query.max_rows.unwrap_or(0), limits)
            .await
        {
            Ok(result) => results.push(ExecuteQueriesResult { index, result }),
            Err(error) => {
                return Ok(ExecuteQueriesOutput {
                    connection,
                    status: "failed".to_string(),
                    statement_count,
                    results,
                    error: Some(ExecuteQueriesError {
                        index,
                        message: format!("{error:#}"),
                    }),
                });
            }
        }
    }

    Ok(ExecuteQueriesOutput {
        connection,
        status: "completed".to_string(),
        statement_count,
        results,
        error: None,
    })
}

#[cfg(test)]
mod tests {
    use std::{
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::execute_query_batch;
    use crate::{
        config::{Config, SQLiteConfig},
        database::{DatabaseConnection, Limits, sqlite::SQLiteConnection, to_json_object},
        mcp_types::{ExecuteQueriesInput, ExecuteQueryItem},
    };

    static DB_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn limits() -> Limits {
        Limits::from_config(&Config::default()).expect("default limits")
    }

    fn test_db_path() -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let sequence = DB_COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "dbmcp-batch-{}-{sequence}-{suffix}.db",
            std::process::id()
        ))
    }

    async fn read_only_connection() -> (SQLiteConnection, Limits, PathBuf) {
        let path = test_db_path();
        let writable_dsn = format!("sqlite://{}?mode=rwc", path.display());
        let pool = sqlx::SqlitePool::connect(&writable_dsn)
            .await
            .expect("create sqlite database");
        sqlx::query("create table if not exists batch_test (value integer)")
            .execute(&pool)
            .await
            .expect("initialize sqlite database");
        pool.close().await;

        let limits = limits();
        let dsn = format!("sqlite://{}?mode=ro", path.display());
        let conn = SQLiteConnection::new(
            "sqlite-test".to_string(),
            SQLiteConfig {
                dsn,
                read_only: true,
            },
            limits.clone(),
        )
        .expect("read-only sqlite connection");
        (conn, limits, path)
    }

    fn input(sql: &[&str]) -> ExecuteQueriesInput {
        ExecuteQueriesInput {
            connection: None,
            queries: sql
                .iter()
                .map(|sql| ExecuteQueryItem {
                    sql: (*sql).to_string(),
                    max_rows: Some(5),
                })
                .collect(),
        }
    }

    #[tokio::test]
    async fn runs_queries_in_order_and_preserves_separate_results() {
        let (conn, limits, path) = read_only_connection().await;
        assert!(conn.supports_read_only_batch());

        let output = execute_query_batch(
            &conn,
            input(&["select\n1 as value;", "select 2 as value;"]),
            &limits,
        )
        .await
        .expect("batch query");

        assert_eq!(output.status, "completed");
        assert_eq!(output.statement_count, 2);
        assert_eq!(output.results.len(), 2);
        assert_eq!(output.results[0].index, 0);
        assert_eq!(output.results[1].index, 1);
        assert_eq!(output.results[0].result.rows[0]["value"].as_i64(), Some(1));
        assert_eq!(output.results[1].result.rows[0]["value"].as_i64(), Some(2));

        drop(conn);
        std::fs::remove_file(path).expect("remove test database");
    }

    #[tokio::test]
    async fn stops_after_the_first_failed_statement() {
        let (conn, limits, path) = read_only_connection().await;

        let output = execute_query_batch(
            &conn,
            input(&[
                "select 1 as value",
                "select * from missing_batch_table",
                "select 3 as value",
            ]),
            &limits,
        )
        .await
        .expect("batch failure should be structured");

        assert_eq!(output.status, "failed");
        assert_eq!(output.statement_count, 3);
        assert_eq!(output.results.len(), 1);
        assert_eq!(output.results[0].index, 0);
        assert_eq!(output.error.as_ref().expect("batch error").index, 1);

        drop(conn);
        std::fs::remove_file(path).expect("remove test database");
    }

    #[tokio::test]
    async fn rejects_non_read_only_sqlite_batches() {
        let limits = limits();
        let conn = SQLiteConnection::new(
            "sqlite-test".to_string(),
            SQLiteConfig {
                dsn: "sqlite::memory:".to_string(),
                read_only: true,
            },
            limits.clone(),
        )
        .expect("sqlite connection");

        let error = execute_query_batch(&conn, input(&["select 1"]), &limits)
            .await
            .expect_err("memory sqlite should fail the read-only DSN gate");
        assert!(error.to_string().contains("read-only DSN"));
    }

    #[test]
    fn schema_requires_one_to_eight_queries() {
        let schema = to_json_object::<ExecuteQueriesInput>();
        let queries = &schema["properties"]["queries"];
        assert_eq!(queries["minItems"].as_u64(), Some(1));
        assert_eq!(queries["maxItems"].as_u64(), Some(8));
    }

    #[tokio::test]
    async fn rejects_empty_and_oversized_batches() {
        let (conn, limits, path) = read_only_connection().await;

        let empty = ExecuteQueriesInput {
            connection: None,
            queries: vec![],
        };
        assert!(
            execute_query_batch(&conn, empty, &limits)
                .await
                .expect_err("empty batch")
                .to_string()
                .contains("between 1 and 8")
        );

        let oversized = ExecuteQueriesInput {
            connection: None,
            queries: (0..9)
                .map(|_| ExecuteQueryItem {
                    sql: "select 1".to_string(),
                    max_rows: None,
                })
                .collect(),
        };
        assert!(
            execute_query_batch(&conn, oversized, &limits)
                .await
                .expect_err("oversized batch")
                .to_string()
                .contains("between 1 and 8")
        );

        drop(conn);
        std::fs::remove_file(path).expect("remove test database");
    }
}
