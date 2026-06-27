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

    pub(super) async fn connect(&self) -> Result<Connection> {
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
        tokio::time::timeout(
            self.limits.query_timeout,
            Connection::connect_with_config(cfg),
        )
        .await
        .with_context(|| {
            format!(
                "oracle connect timeout name={} target={}",
                self.name, self.spec.target
            )
        })?
        .with_context(|| {
            format!(
                "oracle connect failed name={} target={}",
                self.name, self.spec.target
            )
        })
    }

    pub(super) async fn query(&self, sql: &str) -> Result<oracle_rs::QueryResult> {
        let conn = self.connect().await?;
        let binds: Vec<OracleValue> = vec![];
        let result = tokio::time::timeout(self.limits.query_timeout, conn.query(sql, &binds))
            .await
            .with_context(|| format!("oracle query timeout name={}", self.name))?
            .with_context(|| format!("oracle query failed name={}", self.name))?;
        let _ = conn.close().await;
        Ok(result)
    }

    pub(super) async fn query_optional_string(&self, sql: &str) -> Result<Option<String>> {
        let result = self.query(sql).await?;
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

    pub(super) async fn refresh_schema(&self) -> Result<()> {
        let schema = self
            .query_optional_string("select sys_context('USERENV', 'CURRENT_SCHEMA') from dual")
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

    async fn ping(&self) -> Result<()> {
        self.query("select 1 from dual").await?;
        self.refresh_schema().await?;
        Ok(())
    }

    async fn diagnose(&self, deep: bool) -> ConnectionDiagnostic {
        let target = self.target_summary();
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
                let conn = self.connect().await?;
                let _ = conn.close().await;
                Ok(())
            })
            .await,
        );
        if checks.last().is_some_and(|check| check.status != "up") {
            return diagnostic_from_checks(self.info(), checks);
        }

        checks.push(
            timed_probe("pingQuery", target.clone(), || async {
                self.query("select 1 from dual").await.map(|_| ())
            })
            .await,
        );
        if checks.last().is_some_and(|check| check.status != "up") {
            return diagnostic_from_checks(self.info(), checks);
        }

        checks.push(
            timed_probe("defaultSchema", target.clone(), || async {
                self.refresh_schema()
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
                self.query("select count(*) from all_users")
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

    async fn explain_query(&self, sql: &str, limits: &Limits) -> Result<ExplainResultData> {
        let sql = strip_trailing_sql_terminator(sql);
        oracle_query::explain_query(self, sql, limits).await
    }
}
