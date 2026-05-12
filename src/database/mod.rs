use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use base64::Engine;
use futures_util::TryStreamExt;
use rmcp::model::{
    CallToolResult, Content, GetPromptResult, InitializeResult, ListPromptsResult,
    ListResourceTemplatesResult, ListResourcesResult, ListToolsResult, Prompt,
    PromptArgument, PromptMessage, ReadResourceResult, RawContent, RawResource,
    RawResourceTemplate, Resource, ResourceContents, ResourceTemplate, ServerCapabilities,
    ServerInfo, Tool, AnnotateAble,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sqlx::{AnyPool, Row, Column, TypeInfo, ValueRef, any::AnyRow};
use std::{collections::BTreeMap, sync::{Arc, Mutex, Once}, time::Instant};
use tracing::{info, warn};

use crate::{config, readonly_sql};

pub mod oracle;
pub mod postgres;
pub mod sqlite;

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
}

pub type ConnectionInfo = ConnectionHealth;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthSnapshot {
    pub status: String,
    pub default_connection: String,
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
    async fn ping(&self) -> Result<()>;
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
    async fn execute_query(&self, sql: &str, max_rows: usize, limits: &Limits)
        -> Result<QueryResult>;
    async fn explain_query(&self, sql: &str, limits: &Limits) -> Result<ExplainResultData>;
}

#[derive(Clone)]
struct Slot {
    conn: Arc<dyn DatabaseConnection>,
    health: Arc<Mutex<ConnectionHealth>>,
}

#[derive(Clone)]
struct FailedConnection {
    name: String,
    driver: String,
    default_schema: String,
    error: String,
}

impl FailedConnection {
    fn new(name: String, driver: String, default_schema: String, error: String) -> Self {
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
        }
    }

    async fn ping(&self) -> Result<()> {
        Err(self.unavailable_error("ping"))
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

#[derive(Clone)]
pub struct Manager {
    slots: Arc<BTreeMap<String, Slot>>,
    default_name: String,
    limits: Limits,
}

static SQLX_ANY_DRIVERS: Once = Once::new();

impl Manager {
    pub async fn new(cfg: &config::Config) -> Result<Self> {
        SQLX_ANY_DRIVERS.call_once(sqlx::any::install_default_drivers);
        let limits = Limits::from_config(cfg)?;
        let mut slots = BTreeMap::new();
        for conn_cfg in &cfg.connections {
            let target = connection_target_summary(conn_cfg);
            let conn = match open_connection(conn_cfg, &limits).await {
                Ok(conn) => {
                    info!(
                        "database connection initialized name={} driver={} target={}",
                        conn_cfg.name, conn_cfg.driver, target
                    );
                    conn
                }
                Err(err) => {
                    let error = format!("{:#}", err);
                    warn!(
                        "database connection unavailable name={} driver={} target={} error={}",
                        conn_cfg.name, conn_cfg.driver, target, error
                    );
                    Box::new(FailedConnection::new(
                        conn_cfg.name.clone(),
                        conn_cfg.driver.clone(),
                        default_schema_for_failed_connection(conn_cfg),
                        error,
                    ))
                }
            };
            let info = conn.info();
            let health = ConnectionHealth {
                name: info.name.clone(),
                driver: info.driver.clone(),
                default_schema: info.default_schema.clone(),
                status: "down".to_string(),
                available: false,
                last_checked_at: None,
                last_successful_at: None,
                last_error: None,
            };
            slots.insert(
                info.name.clone(),
                Slot {
                    conn: Arc::from(conn),
                    health: Arc::new(Mutex::new(health)),
                },
            );
        }

        let manager = Self {
            default_name: cfg.connections[0].name.clone(),
            slots: Arc::new(slots),
            limits,
        };
        manager.refresh_all().await;
        Ok(manager)
    }

    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    pub fn default_connection_name(&self) -> &str {
        &self.default_name
    }

    pub fn connection_infos(&self) -> Vec<ConnectionInfo> {
        self.slots
            .values()
            .map(|slot| slot.health.lock().expect("health poisoned").clone())
            .collect()
    }

    pub fn connection(&self, name: Option<&str>) -> Result<Arc<dyn DatabaseConnection>> {
        let name = name.unwrap_or(&self.default_name);
        self.slots
            .get(name)
            .map(|slot| slot.conn.clone())
            .with_context(|| format!("connection {name:?} not found"))
    }

    pub async fn health(&self, name: Option<&str>, refresh: bool) -> Result<HealthSnapshot> {
        if refresh {
            if let Some(name) = name {
                self.refresh_one(name).await?;
            } else {
                self.refresh_all().await;
            }
        }
        let mut connections = vec![];
        if let Some(name) = name {
            let slot = self
                .slots
                .get(name)
                .with_context(|| format!("connection {name:?} not found"))?;
            connections.push(slot.health.lock().expect("health poisoned").clone());
        } else {
            for slot in self.slots.values() {
                connections.push(slot.health.lock().expect("health poisoned").clone());
            }
        }
        connections.sort_by(|a, b| a.name.cmp(&b.name));
        let status = overall_status(&connections);
        Ok(HealthSnapshot {
            status,
            default_connection: self.default_name.clone(),
            connections,
        })
    }

    async fn refresh_all(&self) {
        let mut tasks = vec![];
        for slot in self.slots.values() {
            tasks.push(Self::refresh_slot(slot.clone()));
        }
        futures_util::future::join_all(tasks).await;
    }

    async fn refresh_one(&self, name: &str) -> Result<()> {
        let slot = self
            .slots
            .get(name)
            .with_context(|| format!("connection {name:?} not found"))?
            .clone();
        Self::refresh_slot(slot).await;
        Ok(())
    }

    async fn refresh_slot(slot: Slot) {
        let now = chrono_like_now();
        let ping_result = slot.conn.ping().await;
        let info = slot.conn.info();
        let mut health = slot.health.lock().expect("health poisoned");
        health.last_checked_at = Some(now.clone());
        match ping_result {
            Ok(()) => {
                health.available = true;
                health.status = "up".to_string();
                health.last_successful_at = Some(now);
                health.last_error = None;
                health.default_schema = info.default_schema;
            }
            Err(err) => {
                health.available = false;
                health.status = "down".to_string();
                health.last_error = Some(err.to_string());
            }
        }
    }
}

fn overall_status(connections: &[ConnectionHealth]) -> String {
    if connections.iter().all(|c| c.available) {
        "up".to_string()
    } else if connections.iter().any(|c| c.available) {
        "degraded".to_string()
    } else {
        "down".to_string()
    }
}

fn default_schema_for_failed_connection(cfg: &config::ConnectionConfig) -> String {
    match cfg.driver.as_str() {
        "postgres" => cfg
            .postgres
            .as_ref()
            .and_then(|c| {
                let schema = c.schema.trim();
                (!schema.is_empty()).then(|| schema.to_string())
            })
            .unwrap_or_else(|| "public".to_string()),
        "sqlite" => "main".to_string(),
        "oracle" => cfg
            .oracle
            .as_ref()
            .map(|c| c.username.to_uppercase())
            .unwrap_or_default(),
        _ => String::new(),
    }
}

fn connection_target_summary(conn: &config::ConnectionConfig) -> String {
    match conn.driver.as_str() {
        "oracle" => conn
            .oracle
            .as_ref()
            .map(|c| c.jdbc_url.clone())
            .unwrap_or_else(|| "oracle:<missing>".to_string()),
        "postgres" => conn
            .postgres
            .as_ref()
            .map(|c| redact_postgres_dsn(&c.dsn))
            .unwrap_or_else(|| "postgres:<missing>".to_string()),
        "sqlite" => conn
            .sqlite
            .as_ref()
            .map(|c| c.dsn.clone())
            .unwrap_or_else(|| "sqlite:<missing>".to_string()),
        _ => "<unknown>".to_string(),
    }
}

fn redact_postgres_dsn(raw: &str) -> String {
    if let Ok(mut url) = url::Url::parse(raw) {
        if url.password().is_some() {
            let username = url.username().to_string();
            let _ = url.set_password(Some("REDACTED"));
            if username.is_empty() {
                let _ = url.set_username("");
            }
        }
        return url.to_string();
    }
    raw.split_whitespace()
        .map(|part| {
            if part.to_lowercase().starts_with("password=") {
                "password=REDACTED".to_string()
            } else {
                part.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

async fn open_connection(
    cfg: &config::ConnectionConfig,
    limits: &Limits,
) -> Result<Box<dyn DatabaseConnection>> {
    match cfg.driver.as_str() {
        "postgres" => Ok(Box::new(postgres::PostgresConnection::new(
            cfg.name.clone(),
            cfg.postgres
                .as_ref()
                .with_context(|| format!("postgres config missing name={}", cfg.name))?
                .clone(),
            limits.clone(),
        )
        .with_context(|| {
            format!(
                "postgres connection init failed name={} target={}",
                cfg.name,
                connection_target_summary(cfg)
            )
        })?)),
        "sqlite" => Ok(Box::new(sqlite::SQLiteConnection::new(
            cfg.name.clone(),
            cfg.sqlite
                .as_ref()
                .with_context(|| format!("sqlite config missing name={}", cfg.name))?
                .clone(),
            limits.clone(),
        )
        .with_context(|| {
            format!(
                "sqlite connection init failed name={} target={}",
                cfg.name,
                connection_target_summary(cfg)
            )
        })?)),
        "oracle" => Ok(Box::new(oracle::OracleConnection::new(
            cfg.name.clone(),
            cfg.oracle
                .as_ref()
                .with_context(|| format!("oracle config missing name={}", cfg.name))?
                .clone(),
            limits.clone(),
        ))),
        other => bail!("unsupported driver {other}"),
    }
}

pub fn build_server_info(name: &str, version: &str) -> ServerInfo {
    InitializeResult::new(
        ServerCapabilities::builder()
            .enable_prompts()
            .enable_resources()
            .enable_tools()
            .enable_prompts_list_changed()
            .enable_resources_list_changed()
            .enable_tool_list_changed()
            .build(),
    )
    .with_server_info(
        rmcp::model::Implementation::new(name, version)
            .with_title("Database MCP")
            .with_description("Safe database discovery and read-only query access"),
    )
    .with_instructions(
        "Start with list_connections or db://connections, then inspect schemas and tables before running ad hoc SQL.".to_string(),
    )
}

pub fn text_content(text: impl Into<String>) -> Content {
    RawContent::text(text).no_annotation()
}

pub fn call_tool_text(text: impl Into<String>) -> CallToolResult {
    CallToolResult::success(vec![text_content(text)])
}

pub fn call_tool_json<T: Serialize>(text: impl Into<String>, value: &T) -> Result<CallToolResult> {
    let mut result = CallToolResult::success(vec![text_content(text)]);
    result.structured_content = Some(serde_json::to_value(value)?);
    Ok(result)
}

pub fn to_resource(uri: impl Into<String>, name: impl Into<String>, title: impl Into<String>, description: impl Into<String>, mime_type: impl Into<String>) -> Resource {
    RawResource::new(uri, name)
        .with_title(title)
        .with_description(description)
        .with_mime_type(mime_type)
        .no_annotation()
}

pub fn to_resource_template(
    uri_template: impl Into<String>,
    name: impl Into<String>,
    title: impl Into<String>,
    description: impl Into<String>,
    mime_type: impl Into<String>,
) -> ResourceTemplate {
    RawResourceTemplate::new(uri_template, name)
        .with_title(title)
        .with_description(description)
        .with_mime_type(mime_type)
        .no_annotation()
}

pub fn prompt(name: &str, description: &str) -> Prompt {
    Prompt::new(name, Some(description), None)
}

pub fn prompt_with_args(name: &str, description: &str, args: Vec<PromptArgument>) -> Prompt {
    Prompt::new(name, Some(description), Some(args))
}

pub fn get_prompt_result(description: impl Into<String>, messages: Vec<PromptMessage>) -> GetPromptResult {
    GetPromptResult::new(messages).with_description(description)
}

pub fn read_resource_result(contents: Vec<ResourceContents>) -> ReadResourceResult {
    ReadResourceResult::new(contents)
}

pub fn list_tools_result(tools: Vec<Tool>) -> ListToolsResult {
    ListToolsResult {
        tools,
        ..Default::default()
    }
}

pub fn list_prompts_result(prompts: Vec<Prompt>) -> ListPromptsResult {
    ListPromptsResult {
        prompts,
        ..Default::default()
    }
}

pub fn list_resources_result(resources: Vec<Resource>) -> ListResourcesResult {
    ListResourcesResult {
        resources,
        ..Default::default()
    }
}

pub fn list_resource_templates_result(resource_templates: Vec<ResourceTemplate>) -> ListResourceTemplatesResult {
    ListResourceTemplatesResult {
        resource_templates,
        ..Default::default()
    }
}

pub fn to_json_object<T: JsonSchema>() -> Arc<Map<String, Value>> {
    let schema = schemars::schema_for!(T);
    let value = serde_json::to_value(schema).expect("schema serializes");
    Arc::new(value.as_object().cloned().expect("schema is object"))
}

pub fn maybe_truncate_text(value: String, max_chars: usize) -> (String, bool) {
    if value.chars().count() <= max_chars {
        return (value, false);
    }
    let truncated = value.chars().take(max_chars).collect::<String>();
    (truncated, true)
}

pub fn serialize_value(value: &Value, max_chars: usize) -> (Value, bool) {
    match value {
        Value::String(s) => {
            let (s, truncated) = maybe_truncate_text(s.clone(), max_chars);
            (Value::String(s), truncated)
        }
        _ => (value.clone(), false),
    }
}

pub fn any_row_to_json(row: &AnyRow) -> Result<Map<String, Value>> {
    let mut map = Map::new();
    for (idx, col) in row.columns().iter().enumerate() {
        let raw = row.try_get_raw(idx)?;
        let value = if raw.is_null() {
            Value::Null
        } else {
            use sqlx::any::AnyTypeInfoKind::*;
            match raw.type_info().kind() {
                Bool => Value::Bool(row.try_get::<bool, _>(idx)?),
                SmallInt | Integer | BigInt => Value::from(row.try_get::<i64, _>(idx)?),
                Real | Double => Value::from(row.try_get::<f64, _>(idx)?),
                Text => Value::String(row.try_get::<String, _>(idx)?),
                Blob => {
                    let bytes: Vec<u8> = row.try_get(idx)?;
                    Value::String(base64::engine::general_purpose::STANDARD.encode(bytes))
                }
                Null => Value::Null,
            }
        };
        map.insert(col.name().to_string(), value);
    }
    Ok(map)
}

pub fn chrono_like_now() -> String {
    chrono::Utc::now().to_rfc3339()
}

pub async fn collect_rows(
    pool: &AnyPool,
    sql: &str,
    requested_max_rows: usize,
    limits: &Limits,
) -> Result<QueryResult> {
    let normalized = readonly_sql::normalize_read_only_query(sql)?;
    let applied_max_rows = limits.normalize_max_rows(requested_max_rows);
    let started = Instant::now();
    let mut rows = vec![];
    let mut row_count = 0usize;
    let mut columns = vec![];
    let mut row_stream = sqlx::query(&normalized).fetch(pool);
    let mut truncated = false;
    let mut reasons = vec![];
    let mut total_chars = 0usize;

    while let Some(row) = tokio::time::timeout(limits.query_timeout, row_stream.try_next())
        .await
        .context("query timeout")??
    {
        if rows.len() >= applied_max_rows {
            truncated = true;
            reasons.push("row limit reached".to_string());
            break;
        }

        if columns.is_empty() {
            columns = row
                .columns()
                .iter()
                .take(limits.max_columns)
                .map(|col| QueryColumn {
                    name: col.name().to_string(),
                    data_type: col.type_info().name().to_string(),
                })
                .collect();
        }

        let mut row_map = Map::new();
        let raw_map = any_row_to_json(&row)?;
        for (key, value) in raw_map.into_iter().take(limits.max_columns) {
            let (value, was_truncated) = serialize_value(&value, limits.max_cell_chars);
            if was_truncated {
                truncated = true;
                reasons.push(format!("cell {key} truncated"));
            }
            total_chars += serde_json::to_string(&value)?.chars().count();
            if total_chars > limits.max_total_chars {
                truncated = true;
                reasons.push("total character limit reached".to_string());
                break;
            }
            row_map.insert(key, value);
        }
        rows.push(row_map);
        row_count += 1;
        if total_chars > limits.max_total_chars {
            break;
        }
    }

    Ok(QueryResult {
        columns,
        rows,
        row_count,
        applied_max_rows,
        truncated,
        truncation_reasons: reasons,
        notice: if truncated {
            Some("result truncated to protect MCP context".to_string())
        } else {
            None
        },
        execution_time: format!("{:?}", started.elapsed()),
    })
}
