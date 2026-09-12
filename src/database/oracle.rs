use anyhow::{Context, Result};
use oracle_rs::{Config as OracleRsConfig, Connection, Value as OracleValue};
use std::sync::{Arc, RwLock};

use crate::config;

use super::*;
use super::{
    oracle_jdbc::{OracleConnectSpec, OracleService, parse_oracle_jdbc_url},
    oracle_metadata, oracle_query,
};

#[derive(Clone)]
pub struct OracleConnection {
    pub(super) name: String,
    pub(super) config: config::OracleConfig,
    pub(super) limits: Limits,
    spec: OracleConnectSpec,
    default_schema: Arc<RwLock<String>>,
}

impl OracleConnection {
    pub fn new(name: String, config: config::OracleConfig, limits: Limits) -> Result<Self> {
        let spec = parse_oracle_jdbc_url(&config)?;
        let default_schema = config.username.trim().to_uppercase();
        Ok(Self {
            name,
            config,
            limits,
            spec,
            default_schema: Arc::new(RwLock::new(default_schema)),
        })
    }

    pub(super) async fn connect_within(&self, deadline: &Deadline) -> Result<Connection> {
        let mut cfg = match &self.spec.service {
            OracleService::ServiceName(service) => OracleRsConfig::new(
                &self.spec.host,
                self.spec.port,
                service,
                &self.spec.username,
                &self.spec.password,
            ),
            OracleService::Sid(sid) => OracleRsConfig::with_sid(
                &self.spec.host,
                self.spec.port,
                sid,
                &self.spec.username,
                &self.spec.password,
            ),
        };
        cfg = cfg.connect_timeout(self.limits.query_timeout);
        deadline
            .run(
                Operation::Connect,
                &format!(
                    "oracle connect name={} target={}",
                    self.name, self.spec.target
                ),
                Connection::connect_with_config(cfg),
            )
            .await
    }

    pub(super) async fn query(&self, sql: &str) -> Result<oracle_rs::QueryResult> {
        let deadline = self.limits.deadline();
        self.query_within(sql, &deadline).await
    }

    pub(super) async fn query_within(
        &self,
        sql: &str,
        deadline: &Deadline,
    ) -> Result<oracle_rs::QueryResult> {
        let conn = self.connect_within(deadline).await?;
        let binds: Vec<OracleValue> = vec![];
        let result = deadline
            .run(
                Operation::Query,
                &format!("oracle query name={}", self.name),
                conn.query(sql, &binds),
            )
            .await;
        let cancelled = result
            .as_ref()
            .err()
            .is_some_and(|error| timeout_operation(error).is_some());
        close_connection(&conn, deadline).await;
        match result {
            Err(err) if cancelled => {
                let operation = timeout_operation(&err).unwrap_or(Operation::Query);
                Err(attach_cancellation(
                    err,
                    operation,
                    Cancellation::Unconfirmed,
                    !self.config.read_only,
                ))
            }
            other => other,
        }
    }

    pub(super) async fn query_optional_string(&self, sql: &str) -> Result<Option<String>> {
        let deadline = self.limits.deadline();
        self.query_optional_string_within(sql, &deadline).await
    }

    pub(super) async fn query_optional_string_within(
        &self,
        sql: &str,
        deadline: &Deadline,
    ) -> Result<Option<String>> {
        let result = self.query_within(sql, deadline).await?;
        Ok(result
            .rows
            .first()
            .and_then(|row| row.get(0))
            .and_then(oracle_query::value_to_string))
    }

    pub(super) async fn schema(&self) -> String {
        self.default_schema
            .read()
            .expect("default_schema poisoned")
            .clone()
    }

    pub(super) async fn refresh_schema_within(&self, deadline: &Deadline) -> Result<()> {
        let schema = self
            .query_optional_string_within(
                "select sys_context('USERENV', 'CURRENT_SCHEMA') from dual",
                deadline,
            )
            .await
            .context("oracle current schema")?
            .unwrap_or_else(|| self.config.username.trim().to_uppercase());
        *self
            .default_schema
            .write()
            .expect("default_schema poisoned") = schema.to_uppercase();
        Ok(())
    }
}

/// Best-effort bounded close for a dedicated Oracle session. Sessions are not
/// pooled, so an unconfirmed state cannot leak into a shared connection.
async fn close_connection(conn: &Connection, deadline: &Deadline) {
    let budget = deadline.remaining().max(CLEANUP_TIMEOUT);
    let _ = tokio::time::timeout(budget, conn.close()).await;
}

#[async_trait::async_trait]
impl DatabaseConnection for OracleConnection {
    fn info(&self) -> ConnectionInfo {
        ConnectionInfo {
            name: self.name.clone(),
            driver: "oracle".to_string(),
            default_schema: self
                .default_schema
                .read()
                .expect("default_schema poisoned")
                .clone(),
            status: "down".to_string(),
            available: false,
            last_checked_at: None,
            last_successful_at: None,
            last_error: None,
            failure_phase: None,
            last_error_chain: vec![],
            probe_checks: vec![],
        }
    }

    fn target_summary(&self) -> String {
        self.spec.target.clone()
    }

    fn supports_read_only_batch(&self) -> bool {
        self.config.read_only
    }

    async fn ping(&self) -> Result<()> {
        let deadline = self.limits.deadline();
        self.query_within("select 1 from dual", &deadline).await?;
        self.refresh_schema_within(&deadline).await?;
        Ok(())
    }

    async fn diagnose(&self, deep: bool) -> ConnectionDiagnostic {
        let target = self.target_summary();
        let deadline = self.limits.probe_deadline();
        let mut checks = vec![
            timed_probe("config", target.clone(), || async {
                parse_oracle_jdbc_url(&self.config).map(|_| ())
            })
            .await,
        ];
        if checks.last().is_some_and(|check| check.status != "up") {
            return diagnostic_from_checks(self.info(), checks);
        }

        checks.push(
            timed_probe("connect/acquire", target.clone(), || async {
                let conn = self.connect_within(&deadline).await?;
                close_connection(&conn, &deadline).await;
                Ok(())
            })
            .await,
        );
        if checks.last().is_some_and(|check| check.status != "up") {
            return diagnostic_from_checks(self.info(), checks);
        }

        checks.push(
            timed_probe("pingQuery", target.clone(), || async {
                self.query_within("select 1 from dual", &deadline)
                    .await
                    .map(|_| ())
            })
            .await,
        );
        if checks.last().is_some_and(|check| check.status != "up") {
            return diagnostic_from_checks(self.info(), checks);
        }

        checks.push(
            timed_probe("defaultSchema", target.clone(), || async {
                self.refresh_schema_within(&deadline)
                    .await
                    .context("refresh oracle default schema")
            })
            .await,
        );
        if checks.last().is_some_and(|check| check.status != "up") || !deep {
            return diagnostic_from_checks(self.info(), checks);
        }

        checks.push(
            timed_probe("metadataSmokeTest", target, || async {
                self.query_within("select count(*) from all_users", &deadline)
                    .await
                    .map(|_| ())
            })
            .await,
        );
        diagnostic_from_checks(self.info(), checks)
    }

    async fn list_schemas(&self, filter: Option<&str>, limit: usize) -> Result<Vec<SchemaInfo>> {
        oracle_metadata::list_schemas(self, filter, limit).await
    }

    async fn list_tables(
        &self,
        schema: Option<&str>,
        filter: Option<&str>,
        limit: usize,
    ) -> Result<Vec<TableInfo>> {
        oracle_metadata::list_tables(self, schema, filter, limit).await
    }

    async fn search_columns(
        &self,
        schema: Option<&str>,
        filter: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ColumnMatchInfo>> {
        oracle_metadata::search_columns(self, schema, filter, limit).await
    }

    async fn list_constraints(
        &self,
        schema: Option<&str>,
        table: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ConstraintInfo>> {
        oracle_metadata::list_constraints(self, schema, table, limit).await
    }

    async fn list_indexes(
        &self,
        schema: Option<&str>,
        table: Option<&str>,
        limit: usize,
    ) -> Result<Vec<IndexInfo>> {
        oracle_metadata::list_indexes(self, schema, table, limit).await
    }

    async fn get_table_comments(
        &self,
        schema: Option<&str>,
        table: &str,
    ) -> Result<TableCommentInfo> {
        oracle_metadata::get_table_comments(self, schema, table).await
    }

    async fn describe_table(&self, schema: Option<&str>, table: &str) -> Result<Vec<ColumnInfo>> {
        oracle_metadata::describe_table(self, schema, table).await
    }

    async fn get_table_ddl(&self, schema: Option<&str>, table: &str) -> Result<TableDDLInfo> {
        oracle_metadata::get_table_ddl(self, schema, table).await
    }

    async fn execute_query(
        &self,
        sql: &str,
        max_rows: usize,
        limits: &Limits,
    ) -> Result<QueryResult> {
        let sql = strip_trailing_sql_terminator(sql);
        oracle_query::execute_query(self, sql, max_rows, limits).await
    }

    async fn execute_query_within(
        &self,
        sql: &str,
        max_rows: usize,
        limits: &Limits,
        deadline: Deadline,
    ) -> Result<QueryResult> {
        let sql = strip_trailing_sql_terminator(sql);
        oracle_query::execute_query_within(self, sql, max_rows, limits, deadline).await
    }

    async fn explain_query(&self, sql: &str, limits: &Limits) -> Result<ExplainResultData> {
        let sql = strip_trailing_sql_terminator(sql);
        oracle_query::explain_query(self, sql, limits).await
    }
}
