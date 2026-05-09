use anyhow::Result;

use super::*;

#[derive(Clone)]
pub struct OracleConnection {
    name: String,
    _config: config::OracleConfig,
    _limits: Limits,
}

impl OracleConnection {
    pub fn new(name: String, config: config::OracleConfig, limits: Limits) -> Self {
        Self {
            name,
            _config: config,
            _limits: limits,
        }
    }
}

#[async_trait::async_trait]
impl DatabaseConnection for OracleConnection {
    fn info(&self) -> ConnectionInfo {
        ConnectionInfo {
            name: self.name.clone(),
            driver: "oracle".to_string(),
            default_schema: self._config.username.to_uppercase(),
            status: "down".to_string(),
            available: false,
            last_checked_at: None,
            last_successful_at: None,
            last_error: None,
        }
    }

    async fn ping(&self) -> Result<()> {
        anyhow::bail!("oracle support is not enabled in this build");
    }

    async fn list_schemas(&self, _filter: Option<&str>, _limit: usize) -> Result<Vec<SchemaInfo>> {
        anyhow::bail!("oracle support is not enabled in this build");
    }

    async fn list_tables(
        &self,
        _schema: Option<&str>,
        _filter: Option<&str>,
        _limit: usize,
    ) -> Result<Vec<TableInfo>> {
        anyhow::bail!("oracle support is not enabled in this build");
    }

    async fn search_columns(
        &self,
        _schema: Option<&str>,
        _filter: Option<&str>,
        _limit: usize,
    ) -> Result<Vec<ColumnMatchInfo>> {
        anyhow::bail!("oracle support is not enabled in this build");
    }

    async fn list_constraints(
        &self,
        _schema: Option<&str>,
        _table: Option<&str>,
        _limit: usize,
    ) -> Result<Vec<ConstraintInfo>> {
        anyhow::bail!("oracle support is not enabled in this build");
    }

    async fn list_indexes(
        &self,
        _schema: Option<&str>,
        _table: Option<&str>,
        _limit: usize,
    ) -> Result<Vec<IndexInfo>> {
        anyhow::bail!("oracle support is not enabled in this build");
    }

    async fn get_table_comments(
        &self,
        _schema: Option<&str>,
        _table: &str,
    ) -> Result<TableCommentInfo> {
        anyhow::bail!("oracle support is not enabled in this build");
    }

    async fn describe_table(&self, _schema: Option<&str>, _table: &str) -> Result<Vec<ColumnInfo>> {
        anyhow::bail!("oracle support is not enabled in this build");
    }

    async fn get_table_ddl(&self, _schema: Option<&str>, _table: &str) -> Result<TableDDLInfo> {
        anyhow::bail!("oracle support is not enabled in this build");
    }

    async fn execute_query(
        &self,
        _sql: &str,
        _max_rows: usize,
        _limits: &Limits,
    ) -> Result<QueryResult> {
        anyhow::bail!("oracle support is not enabled in this build");
    }

    async fn explain_query(&self, _sql: &str, _limits: &Limits) -> Result<ExplainResultData> {
        anyhow::bail!("oracle support is not enabled in this build");
    }
}
