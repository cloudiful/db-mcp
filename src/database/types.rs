use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::config;

use super::diagnostics::{ConnectionDiagnostic, ProbeCheck};

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthSnapshot {
    pub status: String,
    pub connections: Vec<ConnectionHealth>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaInfo {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableInfo {
    pub schema: String,
    pub name: String,
    pub table_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnMatchInfo {
    pub schema: String,
    pub table: String,
    pub column: String,
    pub data_type: String,
    pub position: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnCommentInfo {
    pub name: String,
    pub comment: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableCommentInfo {
    pub schema: String,
    pub table: String,
    pub table_comment: Option<String>,
    pub columns: Vec<ColumnCommentInfo>,
    pub notice: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableDDLInfo {
    pub schema: String,
    pub table: String,
    pub object_type: String,
    pub ddl: String,
    pub notice: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExplainResultData {
    pub format: String,
    pub plan_lines: Vec<String>,
    pub notice: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryColumn {
    pub name: String,
    pub data_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
}

#[async_trait]
pub trait DatabaseConnection: Send + Sync {
    fn info(&self) -> ConnectionInfo;
    fn target_summary(&self) -> String;
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
    async fn explain_query(&self, sql: &str, limits: &Limits) -> Result<ExplainResultData>;
}
