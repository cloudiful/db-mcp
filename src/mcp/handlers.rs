use anyhow::Result;
use rmcp::model::CallToolResult;

use crate::{database, mcp_types::*};

use super::DbMcpService;
use super::batch::execute_query_batch;

impl DbMcpService {
    pub(super) async fn list_connections(
        &self,
        input: ListConnectionsInput,
    ) -> Result<CallToolResult> {
        let refresh = input.refresh.unwrap_or(false);
        let health = self.state.manager.health(None, refresh).await?;
        database::call_tool_json(&ListConnectionsOutput {
            connections: health.connections,
        })
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
        database::call_tool_json(&DiagnoseConnectionOutput { diagnostics })
    }

    pub(super) async fn list_schemas(&self, input: ListInput) -> Result<CallToolResult> {
        let conn = self.state.manager.connection(input.connection.as_deref())?;
        let schemas = conn
            .list_schemas(input.filter.as_deref(), input.limit.unwrap_or(0))
            .await?;
        database::call_tool_json(&ListSchemasOutput {
            connection: conn.info().name,
            schemas,
        })
    }

    pub(super) async fn list_tables(&self, input: ListInput) -> Result<CallToolResult> {
        let conn = self.state.manager.connection(input.connection.as_deref())?;
        let schema = input.schema.as_deref();
        let tables = conn
            .list_tables(schema, input.filter.as_deref(), input.limit.unwrap_or(0))
            .await?;
        database::call_tool_json(&ListTablesOutput {
            connection: conn.info().name,
            tables,
        })
    }

    pub(super) async fn search_columns(&self, input: SearchColumnsInput) -> Result<CallToolResult> {
        let conn = self.state.manager.connection(input.connection.as_deref())?;
        let schema = input.schema.as_deref();
        let matches = conn
            .search_columns(schema, input.filter.as_deref(), input.limit.unwrap_or(0))
            .await?;
        database::call_tool_json(&SearchColumnsOutput {
            connection: conn.info().name,
            matches,
        })
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
        database::call_tool_json(&ListConstraintsOutput {
            connection: conn.info().name,
            constraints,
        })
    }

    pub(super) async fn list_indexes(&self, input: ListConstraintsInput) -> Result<CallToolResult> {
        let conn = self.state.manager.connection(input.connection.as_deref())?;
        let schema = input.schema.as_deref();
        let indexes = conn
            .list_indexes(schema, input.table.as_deref(), input.limit.unwrap_or(0))
            .await?;
        database::call_tool_json(&ListIndexesOutput {
            connection: conn.info().name,
            indexes,
        })
    }

    pub(super) async fn get_table_comments(&self, input: TableInput) -> Result<CallToolResult> {
        let conn = self.state.manager.connection(input.connection.as_deref())?;
        let comments = conn
            .get_table_comments(input.schema.as_deref(), &input.table)
            .await?;
        database::call_tool_json(&GetTableCommentsOutput {
            connection: conn.info().name,
            comments,
        })
    }

    pub(super) async fn describe_table(&self, input: TableInput) -> Result<CallToolResult> {
        let conn = self.state.manager.connection(input.connection.as_deref())?;
        let columns = conn
            .describe_table(input.schema.as_deref(), &input.table)
            .await?;
        database::call_tool_json(&DescribeTableOutput {
            connection: conn.info().name,
            schema: input
                .schema
                .clone()
                .unwrap_or_else(|| conn.info().default_schema),
            table: input.table,
            columns,
        })
    }

    pub(super) async fn get_table_ddl(&self, input: TableInput) -> Result<CallToolResult> {
        let conn = self.state.manager.connection(input.connection.as_deref())?;
        let ddl = conn
            .get_table_ddl(input.schema.as_deref(), &input.table)
            .await?;
        database::call_tool_json(&GetTableDDLOutput {
            connection: conn.info().name,
            ddl,
        })
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
        database::call_tool_json(&ExecuteQueryOutput {
            connection: conn.info().name,
            result,
        })
    }

    pub(super) async fn execute_queries(
        &self,
        input: ExecuteQueriesInput,
    ) -> Result<CallToolResult> {
        let conn = self.state.manager.connection(input.connection.as_deref())?;
        let output = execute_query_batch(&*conn, input, self.state.manager.limits()).await?;
        database::call_tool_json(&output)
    }

    pub(super) async fn explain_query(&self, input: ExplainQueryInput) -> Result<CallToolResult> {
        let conn = self.state.manager.connection(input.connection.as_deref())?;
        let explain = conn
            .explain_query(&input.sql, self.state.manager.limits())
            .await?;
        database::call_tool_json(&ExplainQueryOutput {
            connection: conn.info().name,
            explain,
        })
    }
}
