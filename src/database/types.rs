use anyhow::{Context, Result};
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::fmt;
use std::time::Duration;

use crate::config;

/// Upper bound for best-effort transaction cleanup (rollback, connection
/// close) performed after a logical deadline has expired. Cleanup is never
/// allowed to wait unbounded.
pub const CLEANUP_TIMEOUT: Duration = Duration::from_secs(5);

use super::diagnostics::{ConnectionDiagnostic, ProbeCheck};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionHealth {
    pub name: String,
    pub driver: String,
    pub default_schema: String,
    pub status: String,
    pub available: bool,
    pub last_checked_at: Option<String>,
    pub last_successful_at: Option<String>,
    pub last_error: Option<String>,
    pub failure_phase: Option<String>,
    pub last_error_chain: Vec<String>,
    pub probe_checks: Vec<ProbeCheck>,
}

pub type ConnectionInfo = ConnectionHealth;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HealthSnapshot {
    pub status: String,
    pub connections: Vec<ConnectionHealth>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SchemaInfo {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TableInfo {
    pub schema: String,
    pub name: String,
    pub table_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ColumnInfo {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub position: i64,
    pub length: Option<i64>,
    pub precision: Option<i64>,
    pub scale: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ColumnMatchInfo {
    pub schema: String,
    pub table: String,
    pub column: String,
    pub data_type: String,
    pub position: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConstraintInfo {
    pub name: Option<String>,
    pub schema: String,
    pub table: String,
    pub constraint_type: String,
    pub columns: Vec<String>,
    pub referenced_schema: Option<String>,
    pub referenced_table: Option<String>,
    pub referenced_columns: Vec<String>,
    pub check_expression: Option<String>,
    pub definition: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct IndexInfo {
    pub schema: String,
    pub table: String,
    pub name: String,
    pub index_type: Option<String>,
    pub unique: bool,
    pub primary: bool,
    pub partial: Option<bool>,
    pub columns: Vec<String>,
    pub definition: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ColumnCommentInfo {
    pub name: String,
    pub comment: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TableCommentInfo {
    pub schema: String,
    pub table: String,
    pub table_comment: Option<String>,
    pub columns: Vec<ColumnCommentInfo>,
    pub notice: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TableDDLInfo {
    pub schema: String,
    pub table: String,
    pub object_type: String,
    pub ddl: String,
    pub notice: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExplainResultData {
    pub format: String,
    pub plan_lines: Vec<String>,
    pub notice: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct QueryColumn {
    pub name: String,
    pub data_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct QueryResult {
    pub columns: Vec<QueryColumn>,
    pub rows: Vec<Map<String, Value>>,
    pub row_count: usize,
    pub applied_max_rows: usize,
    pub truncated: bool,
    pub truncation_reasons: Vec<String>,
    pub notice: Option<String>,
    pub execution_time: String,
}

#[derive(Debug, Clone)]
pub struct Limits {
    pub default_max_rows: usize,
    pub hard_max_rows: usize,
    pub max_columns: usize,
    pub max_cell_chars: usize,
    pub max_total_chars: usize,
    pub explain_max_lines: usize,
    pub explain_max_total_chars: usize,
    pub query_timeout: std::time::Duration,
}

impl Limits {
    pub fn from_config(cfg: &config::Config) -> Result<Self> {
        Ok(Self {
            default_max_rows: cfg.limits.default_max_rows,
            hard_max_rows: cfg.limits.hard_max_rows,
            max_columns: cfg.limits.max_columns,
            max_cell_chars: cfg.limits.max_cell_chars,
            max_total_chars: cfg.limits.max_total_chars,
            explain_max_lines: cfg.limits.explain_max_lines,
            explain_max_total_chars: cfg.limits.explain_max_total_chars,
            query_timeout: cfg.query_timeout()?,
        })
    }

    pub fn normalize_max_rows(&self, requested: usize) -> usize {
        if requested == 0 {
            self.default_max_rows
        } else {
            requested.min(self.hard_max_rows)
        }
    }

    pub fn normalize_list_limit(&self, requested: usize) -> usize {
        if requested == 0 {
            self.default_max_rows
        } else {
            requested.min(self.hard_max_rows)
        }
    }

    /// Budget for one logical query/metadata operation. Every nested stage
    /// (acquire, begin, describe, execute, fetch, commit, rollback) shares it.
    pub fn deadline(&self) -> Deadline {
        Deadline::after(self.query_timeout)
    }

    /// Budget for a whole batch. Statements consume the same shared budget, so
    /// a batch cannot run for `query_timeout * statement_count`.
    pub fn batch_deadline(&self) -> Deadline {
        Deadline::after(self.query_timeout)
    }

    /// Budget for a connection probe. Diagnostics run a fixed stage sequence,
    /// so the stages share one budget instead of restarting the timer.
    pub fn probe_budget(&self) -> Duration {
        self.query_timeout.saturating_mul(4)
    }

    pub fn probe_deadline(&self) -> Deadline {
        Deadline::after(self.probe_budget())
    }
}

/// The logical stage a deadline bounds. Timeouts carry this typed information
/// so callers classify them without matching error strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Acquire,
    Connect,
    Ping,
    Begin,
    Describe,
    Execute,
    Fetch,
    Commit,
    Rollback,
    Query,
    Explain,
    Metadata,
}

impl Operation {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Acquire => "acquire",
            Self::Connect => "connect",
            Self::Ping => "ping",
            Self::Begin => "begin",
            Self::Describe => "describe",
            Self::Execute => "execute",
            Self::Fetch => "fetch",
            Self::Commit => "commit",
            Self::Rollback => "rollback",
            Self::Query => "query",
            Self::Explain => "explain",
            Self::Metadata => "metadata",
        }
    }
}

/// Typed timeout for one logical operation, kept in the error chain so it
/// survives human-readable context added for diagnostics.
#[derive(Debug, Clone, Copy)]
pub struct OperationTimeout {
    operation: Operation,
    budget: Duration,
}

impl OperationTimeout {
    pub fn new(operation: Operation, budget: Duration) -> Self {
        Self { operation, budget }
    }

    pub fn operation(&self) -> Operation {
        self.operation
    }

    pub fn budget(&self) -> Duration {
        self.budget
    }
}

impl fmt::Display for OperationTimeout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} operation exceeded its {:?} budget",
            self.operation.as_str(),
            self.budget
        )
    }
}

impl std::error::Error for OperationTimeout {}

/// A single logical-operation deadline. Nested stages share one deadline: each
/// wait gets only the remaining time and never restarts the timer.
#[derive(Debug, Clone, Copy)]
pub struct Deadline {
    at: tokio::time::Instant,
    budget: Duration,
}

impl Deadline {
    pub fn after(budget: Duration) -> Self {
        Self {
            at: tokio::time::Instant::now() + budget,
            budget,
        }
    }

    pub fn budget(&self) -> Duration {
        self.budget
    }

    pub fn remaining(&self) -> Duration {
        self.at
            .saturating_duration_since(tokio::time::Instant::now())
    }

    pub fn expired(&self) -> bool {
        self.remaining().is_zero()
    }

    /// Run `fut` within the remaining budget. Expiry produces a typed
    /// [`OperationTimeout`] wrapped in `{label} timeout` context; a functional
    /// failure keeps the existing `{label} failed` context chain.
    pub async fn run<T, E, F>(&self, operation: Operation, label: &str, fut: F) -> Result<T>
    where
        E: Into<anyhow::Error>,
        F: std::future::Future<Output = std::result::Result<T, E>>,
    {
        match tokio::time::timeout(self.remaining(), fut).await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(err)) => Err(err.into()).with_context(|| format!("{label} failed")),
            Err(_) => Err(anyhow::Error::new(OperationTimeout::new(
                operation,
                self.budget,
            )))
            .with_context(|| format!("{label} timeout")),
        }
    }
}

/// Classify an error as a timeout and return the bounded stage, walking the
/// full context chain so wrapped errors keep their typed information.
pub fn timeout_operation(error: &anyhow::Error) -> Option<Operation> {
    error.chain().find_map(|cause| {
        cause
            .downcast_ref::<OperationTimeout>()
            .map(OperationTimeout::operation)
    })
}

#[async_trait]
pub trait DatabaseConnection: Send + Sync {
    fn info(&self) -> ConnectionInfo;
    fn target_summary(&self) -> String;
    fn supports_read_only_batch(&self) -> bool;
    async fn ping(&self) -> Result<()>;
    async fn diagnose(&self, deep: bool) -> ConnectionDiagnostic;
    async fn list_schemas(&self, filter: Option<&str>, limit: usize) -> Result<Vec<SchemaInfo>>;
    async fn list_tables(
        &self,
        schema: Option<&str>,
        filter: Option<&str>,
        limit: usize,
    ) -> Result<Vec<TableInfo>>;
    async fn search_columns(
        &self,
        schema: Option<&str>,
        filter: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ColumnMatchInfo>>;
    async fn list_constraints(
        &self,
        schema: Option<&str>,
        table: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ConstraintInfo>>;
    async fn list_indexes(
        &self,
        schema: Option<&str>,
        table: Option<&str>,
        limit: usize,
    ) -> Result<Vec<IndexInfo>>;
    async fn get_table_comments(
        &self,
        schema: Option<&str>,
        table: &str,
    ) -> Result<TableCommentInfo>;
    async fn describe_table(&self, schema: Option<&str>, table: &str) -> Result<Vec<ColumnInfo>>;
    async fn get_table_ddl(&self, schema: Option<&str>, table: &str) -> Result<TableDDLInfo>;
    async fn execute_query(
        &self,
        sql: &str,
        max_rows: usize,
        limits: &Limits,
    ) -> Result<QueryResult>;

    /// Execute a query inside a caller-provided shared deadline. Backends that
    /// own their cancellation cleanup use it for every nested stage; the
    /// default preserves behavior for connections without shared-budget
    /// support.
    async fn execute_query_within(
        &self,
        sql: &str,
        max_rows: usize,
        limits: &Limits,
        _deadline: Deadline,
    ) -> Result<QueryResult> {
        self.execute_query(sql, max_rows, limits).await
    }

    async fn explain_query(&self, sql: &str, limits: &Limits) -> Result<ExplainResultData>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn run_returns_value_within_budget() {
        let deadline = Deadline::after(Duration::from_secs(1));
        let value = deadline
            .run(Operation::Query, "postgres query", async {
                Ok::<_, anyhow::Error>(7)
            })
            .await
            .expect("value within budget");
        assert_eq!(value, 7);
    }

    #[tokio::test]
    async fn run_maps_expiry_to_typed_operation() {
        let deadline = Deadline::after(Duration::from_millis(10));
        let err = deadline
            .run(Operation::Fetch, "postgres fetch", async {
                tokio::time::sleep(Duration::from_secs(30)).await;
                Ok::<_, anyhow::Error>(())
            })
            .await
            .expect_err("deadline should expire");
        assert_eq!(timeout_operation(&err), Some(Operation::Fetch));
        assert!(format!("{err:#}").contains("postgres fetch timeout"));
    }

    #[tokio::test]
    async fn run_keeps_failure_context_without_timeout_classification() {
        let deadline = Deadline::after(Duration::from_secs(1));
        let err = deadline
            .run(Operation::Query, "postgres query", async {
                Err::<(), _>(anyhow::anyhow!("syntax error"))
            })
            .await
            .expect_err("failure should propagate");
        assert_eq!(timeout_operation(&err), None);
        let message = format!("{err:#}");
        assert!(message.contains("postgres query failed"));
        assert!(message.contains("syntax error"));
    }

    #[test]
    fn remaining_time_shrinks_and_expires() {
        let deadline = Deadline::after(Duration::from_millis(50));
        let before = deadline.remaining();
        std::thread::sleep(Duration::from_millis(5));
        assert!(deadline.remaining() < before);
        assert!(!deadline.expired());

        let expired = Deadline::after(Duration::ZERO);
        assert!(expired.expired());
        assert_eq!(expired.remaining(), Duration::ZERO);
    }

    #[test]
    fn probe_budget_scales_with_query_timeout() {
        let limits = Limits {
            query_timeout: Duration::from_secs(2),
            ..Limits::from_config(&config::Config::default()).expect("default limits")
        };
        assert_eq!(limits.probe_budget(), Duration::from_secs(8));
        assert_eq!(limits.batch_deadline().budget(), Duration::from_secs(2));
    }
}
