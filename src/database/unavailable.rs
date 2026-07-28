use anyhow::Result;
use async_trait::async_trait;

use super::{
    ColumnInfo, ColumnMatchInfo, ConnectionDiagnostic, ConnectionInfo, ConstraintInfo,
    DatabaseConnection, ExplainResultData, IndexInfo, Limits, QueryResult, SchemaInfo,
    TableCommentInfo, TableDDLInfo, TableInfo, diagnostic_from_checks, probe_check_result,
};

#[derive(Clone)]
pub struct FailedConnection {
    name: String,
    driver: String,
    default_schema: String,
    error: String,
}

impl FailedConnection {
    pub fn new(name: String, driver: String, default_schema: String, error: String) -> Self {
        Self {
            name,
            driver,
            default_schema,
            error,
        }
    }

    fn unavailable_error(&self, op: &str) -> anyhow::Error {
        anyhow::anyhow!(
            "connection unavailable op={} name={} driver={} error={}",
            op,
            self.name,
            self.driver,
            self.error
        )
    }
}

#[async_trait]
impl DatabaseConnection for FailedConnection {
    fn info(&self) -> ConnectionInfo {
        ConnectionInfo {
            name: self.name.clone(),
            driver: self.driver.clone(),
            default_schema: self.default_schema.clone(),
            status: "down".to_string(),
            available: false,
            last_checked_at: None,
            last_successful_at: None,
            last_error: Some(self.error.clone()),
            failure_phase: Some("config".to_string()),
            last_error_chain: vec![self.error.clone()],
            probe_checks: vec![],
        }
    }

    fn target_summary(&self) -> String {
        "<unavailable>".to_string()
    }

    fn supports_read_only_batch(&self) -> bool {
        false
    }

    async fn ping(&self) -> Result<()> {
        Err(self.unavailable_error("ping"))
    }

    async fn diagnose(&self, _deep: bool) -> ConnectionDiagnostic {
        let error = self.unavailable_error("diagnose");
        diagnostic_from_checks(
            self.info(),
            vec![probe_check_result(
                "config",
                self.target_summary(),
                std::time::Duration::ZERO,
                Err(error),
            )],
        )
    }

    async fn list_schemas(&self, _filter: Option<&str>, _limit: usize) -> Result<Vec<SchemaInfo>> {
        Err(self.unavailable_error("list_schemas"))
    }

    async fn list_tables(
        &self,
        _schema: Option<&str>,
        _filter: Option<&str>,
        _limit: usize,
    ) -> Result<Vec<TableInfo>> {
        Err(self.unavailable_error("list_tables"))
    }

    async fn search_columns(
        &self,
        _schema: Option<&str>,
        _filter: Option<&str>,
        _limit: usize,
    ) -> Result<Vec<ColumnMatchInfo>> {
        Err(self.unavailable_error("search_columns"))
    }

    async fn list_constraints(
        &self,
        _schema: Option<&str>,
        _table: Option<&str>,
        _limit: usize,
    ) -> Result<Vec<ConstraintInfo>> {
        Err(self.unavailable_error("list_constraints"))
    }

    async fn list_indexes(
        &self,
        _schema: Option<&str>,
        _table: Option<&str>,
        _limit: usize,
    ) -> Result<Vec<IndexInfo>> {
        Err(self.unavailable_error("list_indexes"))
    }

    async fn get_table_comments(
        &self,
        _schema: Option<&str>,
        _table: &str,
    ) -> Result<TableCommentInfo> {
        Err(self.unavailable_error("get_table_comments"))
    }

    async fn describe_table(&self, _schema: Option<&str>, _table: &str) -> Result<Vec<ColumnInfo>> {
        Err(self.unavailable_error("describe_table"))
    }

    async fn get_table_ddl(&self, _schema: Option<&str>, _table: &str) -> Result<TableDDLInfo> {
        Err(self.unavailable_error("get_table_ddl"))
    }

    async fn execute_query(
        &self,
        _sql: &str,
        _max_rows: usize,
        _limits: &Limits,
    ) -> Result<QueryResult> {
        Err(self.unavailable_error("execute_query"))
    }

    async fn explain_query(&self, _sql: &str, _limits: &Limits) -> Result<ExplainResultData> {
        Err(self.unavailable_error("explain_query"))
    }
}
