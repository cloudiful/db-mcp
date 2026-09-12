use schemars::JsonSchema;
use serde::Deserialize;

use crate::database;

pub use crate::config::MAX_BATCH_QUERIES;

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListConnectionsInput {
    pub refresh: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DiagnoseConnectionInput {
    pub connection: Option<String>,
    pub deep: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListInput {
    pub connection: Option<String>,
    pub schema: Option<String>,
    pub filter: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SearchColumnsInput {
    pub connection: Option<String>,
    pub schema: Option<String>,
    pub filter: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListConstraintsInput {
    pub connection: Option<String>,
    pub schema: Option<String>,
    pub table: Option<String>,
    pub filter: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TableInput {
    pub connection: Option<String>,
    pub schema: Option<String>,
    pub table: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExecuteQueryInput {
    pub connection: Option<String>,
    pub sql: String,
    pub max_rows: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExecuteQueryItem {
    pub sql: String,
    pub max_rows: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExecuteQueriesInput {
    pub connection: Option<String>,
    #[schemars(length(min = 1, max = MAX_BATCH_QUERIES))]
    pub queries: Vec<ExecuteQueryItem>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExplainQueryInput {
    pub connection: Option<String>,
    pub sql: String,
}

#[derive(Debug, Clone, serde::Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListConnectionsOutput {
    pub connections: Vec<database::ConnectionHealth>,
}

#[derive(Debug, Clone, serde::Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DiagnoseConnectionOutput {
    pub diagnostics: Vec<database::ConnectionDiagnostic>,
}

#[derive(Debug, Clone, serde::Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListSchemasOutput {
    pub connection: String,
    pub schemas: Vec<database::SchemaInfo>,
}

#[derive(Debug, Clone, serde::Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListTablesOutput {
    pub connection: String,
    pub tables: Vec<database::TableInfo>,
}

#[derive(Debug, Clone, serde::Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SearchColumnsOutput {
    pub connection: String,
    pub matches: Vec<database::ColumnMatchInfo>,
}

#[derive(Debug, Clone, serde::Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListConstraintsOutput {
    pub connection: String,
    pub constraints: Vec<database::ConstraintInfo>,
}

#[derive(Debug, Clone, serde::Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListIndexesOutput {
    pub connection: String,
    pub indexes: Vec<database::IndexInfo>,
}

#[derive(Debug, Clone, serde::Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetTableCommentsOutput {
    pub connection: String,
    pub comments: database::TableCommentInfo,
}

#[derive(Debug, Clone, serde::Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DescribeTableOutput {
    pub connection: String,
    pub schema: String,
    pub table: String,
    pub columns: Vec<database::ColumnInfo>,
}

#[derive(Debug, Clone, serde::Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetTableDDLOutput {
    pub connection: String,
    pub ddl: database::TableDDLInfo,
}

#[derive(Debug, Clone, serde::Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExecuteQueryOutput {
    pub connection: String,
    pub result: database::QueryResult,
}

#[derive(Debug, Clone, serde::Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExecuteQueriesResult {
    pub index: usize,
    pub result: database::QueryResult,
}

#[derive(Debug, Clone, serde::Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExecuteQueriesError {
    pub index: usize,
    pub message: String,
    pub code: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancellation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_unknown: Option<bool>,
}

#[derive(Debug, Clone, serde::Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExecuteQueriesOutput {
    pub connection: String,
    pub status: String,
    pub statement_count: usize,
    pub results: Vec<ExecuteQueriesResult>,
    pub error: Option<ExecuteQueriesError>,
}

#[derive(Debug, Clone, serde::Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExplainQueryOutput {
    pub connection: String,
    pub explain: database::ExplainResultData,
}
