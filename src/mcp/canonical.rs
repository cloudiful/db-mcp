use anyhow::{Result, bail};
use rmcp::model::{CallToolResult, Tool};
use serde_json::{Map, Value};

use crate::database::{self, read_only_annotations};
use crate::mcp::types_canonical::*;
use crate::mcp_types::*;

use super::DbMcpService;
use super::batch::execute_query_batch;
use super::server::{RunToolResult, parse_or_error};

const READ_ONLY_REQUIRED: &str = "db_query requires a read-only connection; SQLite connections also require a read-only DSN such as mode=ro";

/// The canonical four-tool surface. All tools are read-only: `db_query`
/// enforces a read-only connection server-side rather than relying on the
/// tool annotation.
pub fn build_canonical_tools() -> Vec<Tool> {
    vec![
        database::tool_schema::<DbConnectionsInput, DbConnectionsOutput>(
            "db_connections",
            "List configured database connections or run staged connection diagnostics.",
            read_only_annotations(),
        ),
        database::tool_schema::<DbIntrospectInput, DbIntrospectOutput>(
            "db_introspect",
            "Inspect database metadata by action: schemas, tables, columns, constraints, indexes, comments, or ddl.",
            read_only_annotations(),
        ),
        database::tool_schema::<DbQueryInput, DbQueryOutput>(
            "db_query",
            "Run one query or a batch of 1 to 8 statements on a read-only connection. Results are truncated to protect MCP context.",
            read_only_annotations(),
        ),
        database::tool_schema::<DbExplainInput, ExplainQueryOutput>(
            "db_explain",
            "Explain a query on a named connection and return the execution plan.",
            read_only_annotations(),
        ),
    ]
}

pub(super) async fn dispatch_canonical(
    service: &DbMcpService,
    name: &str,
    arguments: Option<Map<String, Value>>,
) -> Option<RunToolResult> {
    let result = match name {
        "db_connections" => {
            let input: DbConnectionsInput = match parse_or_error(arguments) {
                Ok(input) => input,
                Err(error) => return Some(error),
            };
            service.canonical_connections(input).await
        }
        "db_introspect" => {
            let input: DbIntrospectInput = match parse_or_error(arguments) {
                Ok(input) => input,
                Err(error) => return Some(error),
            };
            if let Err(error) = input.validate() {
                return Some(RunToolResult::invalid_params(&error));
            }
            service
                .introspect(input)
                .await
                .and_then(|output| database::call_tool_json(&output))
        }
        "db_query" => {
            let input: DbQueryInput = match parse_or_error(arguments) {
                Ok(input) => input,
                Err(error) => return Some(error),
            };
            if let Err(error) = input.validate() {
                return Some(RunToolResult::invalid_params(&error));
            }
            service.canonical_query(input).await
        }
        "db_explain" => {
            let input: DbExplainInput = match parse_or_error(arguments) {
                Ok(input) => input,
                Err(error) => return Some(error),
            };
            service.canonical_explain(input).await
        }
        _ => return None,
    };
    Some(RunToolResult::from_handler(result))
}

impl DbMcpService {
    async fn canonical_connections(&self, input: DbConnectionsInput) -> Result<CallToolResult> {
        let output = match input.action {
            ConnectionAction::List => {
                let health = self
                    .state
                    .manager
                    .health(input.connection.as_deref(), input.refresh.unwrap_or(false))
                    .await?;
                DbConnectionsOutput::List(ListConnectionsOutput {
                    connections: health.connections,
                })
            }
            ConnectionAction::Diagnose => {
                let diagnostics = self
                    .state
                    .manager
                    .diagnose(input.connection.as_deref(), input.deep.unwrap_or(false))
                    .await?;
                DbConnectionsOutput::Diagnose(DiagnoseConnectionOutput { diagnostics })
            }
        };
        database::call_tool_json(&output)
    }

    /// Single and batch canonical queries both require a read-only connection;
    /// the driver-level read-only policy (`supports_read_only_batch`) is the
    /// server-side gate, not the MCP annotation.
    async fn canonical_query(&self, input: DbQueryInput) -> Result<CallToolResult> {
        let conn = self.state.manager.connection(input.connection.as_deref())?;
        if !conn.supports_read_only_batch() {
            bail!(READ_ONLY_REQUIRED);
        }

        let output = match (input.sql, input.queries) {
            (Some(sql), None) => {
                let single = self
                    .execute_query(ExecuteQueryInput {
                        connection: input.connection,
                        sql,
                        max_rows: input.max_rows,
                    })
                    .await?;
                DbQueryOutput::Single(single)
            }
            (None, Some(queries)) => {
                let batch = ExecuteQueriesInput {
                    connection: input.connection,
                    queries: queries
                        .into_iter()
                        .map(|query| ExecuteQueryItem {
                            sql: query.sql,
                            max_rows: input.max_rows,
                        })
                        .collect(),
                };
                let result =
                    execute_query_batch(&*conn, batch, self.state.manager.limits()).await?;
                DbQueryOutput::Batch(result)
            }
            // `DbQueryInput::validate` runs before dispatch and rejects both
            // forms, so this arm is defensive rather than reachable.
            _ => bail!("db_query requires either sql or queries, not both"),
        };
        database::call_tool_json(&output)
    }

    async fn canonical_explain(&self, input: DbExplainInput) -> Result<CallToolResult> {
        let explain = self
            .explain_query(ExplainQueryInput {
                connection: input.connection,
                sql: input.sql,
            })
            .await?;
        database::call_tool_json(&explain)
    }
}
