use anyhow::Result;

use crate::{database, mcp_types::*};

use super::errors::{classify, log_failure};

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
    // One shared budget for the whole batch: later statements see only the
    // remaining time, and a failure keeps its statement index.
    let deadline = limits.batch_deadline();
    let mut results = Vec::with_capacity(statement_count);
    for (index, query) in input.queries.into_iter().enumerate() {
        match conn
            .execute_query_within(&query.sql, query.max_rows.unwrap_or(0), limits, deadline)
            .await
        {
            Ok(result) => results.push(ExecuteQueriesResult { index, result }),
            Err(error) => {
                // The failing statement keeps its index and the same stable
                // error fields as a tool-level failure; the full chain goes to
                // the controlled log only.
                log_failure(&error, Some(&connection));
                let payload = classify(&error, Some(&connection)).payload;
                return Ok(ExecuteQueriesOutput {
                    connection,
                    status: "failed".to_string(),
                    statement_count,
                    results,
                    error: Some(ExecuteQueriesError {
                        index,
                        message: payload.message,
                        code: payload.code,
                        kind: payload.kind,
                        operation: payload.operation,
                        timeout: payload.timeout,
                        cancellation: payload.cancellation,
                        result_unknown: payload.result_unknown,
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
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    use super::execute_query_batch;
    use crate::{
        config::{Config, SQLiteConfig},
        database::{
            DatabaseConnection, Limits, Operation, sqlite::SQLiteConnection, timeout_operation,
            to_json_object,
        },
        mcp_types::{ExecuteQueriesInput, ExecuteQueryItem},
    };

    static DB_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn limits() -> Limits {
        Limits::from_config(&Config::default()).expect("default limits")
    }

    fn limits_with_timeout(timeout: Duration) -> Limits {
        let mut limits = limits();
        limits.query_timeout = timeout;
        limits
    }

    /// A deterministic CPU-bound query that always outlives a short test
    /// budget, then completes so the sqlite worker is not stuck indefinitely.
    const SLOW_SQL: &str = "with recursive counter(value) as (\
        select 1 union all select value + 1 from counter where value < 5000000\
    ) select count(*) as value from counter";

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
        let error = output.error.as_ref().expect("batch error");
        assert_eq!(error.index, 1);
        assert_eq!(error.code, "backend_error");
        assert_eq!(error.kind, "backend");
        assert!(error.message.contains("missing_batch_table"), "{error:?}");
        assert!(error.timeout.is_none());
        assert!(error.cancellation.is_none());

        drop(conn);
        std::fs::remove_file(path).expect("remove test database");
    }

    #[tokio::test]
    async fn batch_shares_one_budget_and_keeps_the_failing_index() {
        let (conn, _default_limits, path) = read_only_connection().await;
        let limits = limits_with_timeout(Duration::from_millis(300));

        let output = execute_query_batch(&conn, input(&[SLOW_SQL, "select 2 as value"]), &limits)
            .await
            .expect("batch failure should be structured");

        assert_eq!(output.status, "failed");
        assert!(
            output.results.is_empty(),
            "the slow statement runs first and must stop the batch"
        );
        let error = output.error.expect("batch error");
        assert_eq!(error.index, 0);
        assert_eq!(error.code, "operation_timeout");
        assert_eq!(error.kind, "timeout");
        assert_eq!(error.timeout, Some(true));
        assert!(
            error.cancellation.is_some(),
            "a deadline timeout must carry a typed cancellation state: {error:?}"
        );
        assert!(
            error.message.contains("timeout"),
            "unexpected error: {}",
            error.message
        );

        drop(conn);
        std::fs::remove_file(path).expect("remove test database");
    }

    #[tokio::test]
    async fn shared_deadline_timeout_carries_typed_operation() {
        let (conn, default_limits, path) = read_only_connection().await;
        // Warm the pool first so acquire/describe do not consume the short
        // budget and the timeout lands on the streaming fetch.
        conn.execute_query("select 1 as value", 5, &default_limits)
            .await
            .expect("warm-up query");
        let limits = limits_with_timeout(Duration::from_millis(300));
        let deadline = limits.batch_deadline();

        let err = conn
            .execute_query_within(SLOW_SQL, 5, &limits, deadline)
            .await
            .expect_err("slow query should exceed the shared deadline");

        let operation = timeout_operation(&err).expect("timeout should be typed");
        assert!(
            matches!(
                operation,
                Operation::Describe | Operation::Execute | Operation::Fetch
            ),
            "unexpected operation: {operation:?}"
        );
        assert!(format!("{err:#}").contains("timeout"));

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
