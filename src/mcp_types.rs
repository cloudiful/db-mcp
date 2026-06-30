use schemars::JsonSchema;
use serde::Deserialize;

use crate::database;

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
pub struct ExplainQueryInput {
    pub connection: Option<String>,
    pub sql: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListConnectionsOutput {
    pub connections: Vec<database::ConnectionHealth>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnoseConnectionOutput {
    pub diagnostics: Vec<database::ConnectionDiagnostic>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListSchemasOutput {
    pub connection: String,
    pub schemas: Vec<database::SchemaInfo>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListTablesOutput {
    pub connection: String,
    pub tables: Vec<database::TableInfo>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchColumnsOutput {
    pub connection: String,
    pub matches: Vec<database::ColumnMatchInfo>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListConstraintsOutput {
    pub connection: String,
    pub constraints: Vec<database::ConstraintInfo>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListIndexesOutput {
    pub connection: String,
    pub indexes: Vec<database::IndexInfo>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GetTableCommentsOutput {
    pub connection: String,
    pub comments: database::TableCommentInfo,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DescribeTableOutput {
    pub connection: String,
    pub schema: String,
    pub table: String,
    pub columns: Vec<database::ColumnInfo>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GetTableDDLOutput {
    pub connection: String,
    pub ddl: database::TableDDLInfo,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecuteQueryOutput {
    pub connection: String,
    pub result: database::QueryResult,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExplainQueryOutput {
    pub connection: String,
    pub explain: database::ExplainResultData,
}
