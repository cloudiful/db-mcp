use anyhow::Result;
use rmcp::model::CallToolResult;

use crate::{database, mcp_types::*};

use super::DbMcpService;

impl DbMcpService {
    pub(super) async fn list_connections(
        &self,
        input: ListConnectionsInput,
    ) -> Result<CallToolResult> {
        let refresh = input.refresh.unwrap_or(true);
        let health = self.state.manager.health(None, refresh).await?;
        let output = ListConnectionsOutput {
            default_connection: health.default_connection.clone(),
            connections: health.connections,
        };
        database::call_tool_json(
            format!(
                "Configured {} connection(s). Default is {}.",
                output.connections.len(),
                output.default_connection
            ),
            &output,
        )
    }

    pub(super) async fn diagnose_connection(
        &self,
        input: DiagnoseConnectionInput,
    ) -> Result<CallToolResult> {
        let diagnostics = self
            .state
            .manager
            .diagnose(input.connection.as_deref(), input.deep.unwrap_or(false))
            .await?;
        database::call_tool_json(
            format!(
                "Returned diagnostic data for {} connection(s).",
                diagnostics.len()
            ),
            &DiagnoseConnectionOutput { diagnostics },
        )
    }

    pub(super) async fn list_schemas(&self, input: ListInput) -> Result<CallToolResult> {
        let conn = self.state.manager.connection(input.connection.as_deref())?;
        let schemas = conn
            .list_schemas(input.filter.as_deref(), input.limit.unwrap_or(0))
            .await?;
        let output = ListSchemasOutput {
            connection: conn.info().name,
            schemas,
        };
        database::call_tool_json(
            format!("Returned {} schema(s).", output.schemas.len()),
            &output,
        )
    }

    pub(super) async fn list_tables(&self, input: ListInput) -> Result<CallToolResult> {
        let conn = self.state.manager.connection(input.connection.as_deref())?;
        let schema = input.schema.as_deref();
        let tables = conn
            .list_tables(schema, input.filter.as_deref(), input.limit.unwrap_or(0))
            .await?;
        let output = ListTablesOutput {
            connection: conn.info().name,
            tables,
        };
        database::call_tool_json(
            format!("Returned {} table/view entries.", output.tables.len()),
            &output,
        )
    }

    pub(super) async fn search_columns(&self, input: SearchColumnsInput) -> Result<CallToolResult> {
        let conn = self.state.manager.connection(input.connection.as_deref())?;
        let schema = input.schema.as_deref();
        let matches = conn
            .search_columns(schema, input.filter.as_deref(), input.limit.unwrap_or(0))
            .await?;
        let output = SearchColumnsOutput {
            connection: conn.info().name,
            matches,
        };
        database::call_tool_json(
            format!("Returned {} column match(es).", output.matches.len()),
            &output,
        )
    }

    pub(super) async fn list_constraints(
        &self,
        input: ListConstraintsInput,
    ) -> Result<CallToolResult> {
        let conn = self.state.manager.connection(input.connection.as_deref())?;
        let schema = input.schema.as_deref();
        let constraints = conn
            .list_constraints(schema, input.table.as_deref(), input.limit.unwrap_or(0))
            .await?;
        let output = ListConstraintsOutput {
            connection: conn.info().name,
            constraints,
        };
        database::call_tool_json(
            format!("Returned {} constraint row(s).", output.constraints.len()),
            &output,
        )
    }

    pub(super) async fn list_indexes(&self, input: ListConstraintsInput) -> Result<CallToolResult> {
        let conn = self.state.manager.connection(input.connection.as_deref())?;
        let schema = input.schema.as_deref();
        let indexes = conn
            .list_indexes(schema, input.table.as_deref(), input.limit.unwrap_or(0))
            .await?;
        let output = ListIndexesOutput {
            connection: conn.info().name,
            indexes,
        };
        database::call_tool_json(
            format!("Returned {} index row(s).", output.indexes.len()),
            &output,
        )
    }

    pub(super) async fn get_table_comments(&self, input: TableInput) -> Result<CallToolResult> {
        let conn = self.state.manager.connection(input.connection.as_deref())?;
        let comments = conn
            .get_table_comments(input.schema.as_deref(), &input.table)
            .await?;
        database::call_tool_json(
            format!("Loaded comments for {}.{}", comments.schema, comments.table),
            &GetTableCommentsOutput {
                connection: conn.info().name,
                comments,
            },
        )
    }

    pub(super) async fn describe_table(&self, input: TableInput) -> Result<CallToolResult> {
        let conn = self.state.manager.connection(input.connection.as_deref())?;
        let columns = conn
            .describe_table(input.schema.as_deref(), &input.table)
            .await?;
        let output = DescribeTableOutput {
            connection: conn.info().name,
            schema: input
                .schema
                .clone()
                .unwrap_or_else(|| conn.info().default_schema),
            table: input.table,
            columns,
        };
        database::call_tool_json(
            format!("Described {}.{}.", output.schema, output.table),
            &output,
        )
    }

    pub(super) async fn get_table_ddl(&self, input: TableInput) -> Result<CallToolResult> {
        let conn = self.state.manager.connection(input.connection.as_deref())?;
        let ddl = conn
            .get_table_ddl(input.schema.as_deref(), &input.table)
            .await?;
        database::call_tool_json(
            format!(
                "Loaded {} DDL for {}.{}.",
                ddl.object_type, ddl.schema, ddl.table
            ),
            &GetTableDDLOutput {
                connection: conn.info().name,
                ddl,
            },
        )
    }

    pub(super) async fn execute_query(&self, input: ExecuteQueryInput) -> Result<CallToolResult> {
        let conn = self.state.manager.connection(input.connection.as_deref())?;
        let result = conn
            .execute_query(
                &input.sql,
                input.max_rows.unwrap_or(0),
                self.state.manager.limits(),
            )
            .await?;
        let text = format!(
            "Returned {} row(s) with {} column(s) from {} in {}.",
            result.row_count,
            result.columns.len(),
            conn.info().name,
            result.execution_time
        );
        database::call_tool_json(
            text,
            &ExecuteQueryOutput {
                connection: conn.info().name,
                result,
            },
        )
    }

    pub(super) async fn explain_query(&self, input: ExplainQueryInput) -> Result<CallToolResult> {
        let conn = self.state.manager.connection(input.connection.as_deref())?;
        let explain = conn
            .explain_query(&input.sql, self.state.manager.limits())
            .await?;
        database::call_tool_json(
            format!(
                "Explained query on {} with {} plan line(s).",
                conn.info().name,
                explain.plan_lines.len()
            ),
            &ExplainQueryOutput {
                connection: conn.info().name,
                explain,
            },
        )
    }
}
