use anyhow::Result;

use crate::mcp_types::*;

use super::DbMcpService;
use super::batch::execute_query_batch;

impl DbMcpService {
    pub(super) async fn list_connections(
        &self,
        input: ListConnectionsInput,
    ) -> Result<ListConnectionsOutput> {
        let health = self
            .state
            .manager
            .health(None, input.refresh.unwrap_or(false))
            .await?;
        Ok(ListConnectionsOutput {
            connections: health.connections,
        })
    }

    pub(super) async fn diagnose_connection(
        &self,
        input: DiagnoseConnectionInput,
    ) -> Result<DiagnoseConnectionOutput> {
        let diagnostics = self
            .state
            .manager
            .diagnose(input.connection.as_deref(), input.deep.unwrap_or(false))
            .await?;
        Ok(DiagnoseConnectionOutput { diagnostics })
    }

    pub(super) async fn list_schemas(&self, input: ListInput) -> Result<ListSchemasOutput> {
        let conn = self.state.manager.connection(input.connection.as_deref())?;
        let schemas = conn
            .list_schemas(input.filter.as_deref(), input.limit.unwrap_or(0))
            .await?;
        Ok(ListSchemasOutput {
            connection: conn.info().name,
            schemas,
        })
    }

    pub(super) async fn list_tables(&self, input: ListInput) -> Result<ListTablesOutput> {
        let conn = self.state.manager.connection(input.connection.as_deref())?;
        let schema = input.schema.as_deref();
        let tables = conn
            .list_tables(schema, input.filter.as_deref(), input.limit.unwrap_or(0))
            .await?;
        Ok(ListTablesOutput {
            connection: conn.info().name,
            tables,
        })
    }

    pub(super) async fn search_columns(
        &self,
        input: SearchColumnsInput,
    ) -> Result<SearchColumnsOutput> {
        let conn = self.state.manager.connection(input.connection.as_deref())?;
        let schema = input.schema.as_deref();
        let matches = conn
            .search_columns(schema, input.filter.as_deref(), input.limit.unwrap_or(0))
            .await?;
        Ok(SearchColumnsOutput {
            connection: conn.info().name,
            matches,
        })
    }

    pub(super) async fn list_constraints(
        &self,
        input: ListConstraintsInput,
    ) -> Result<ListConstraintsOutput> {
        let conn = self.state.manager.connection(input.connection.as_deref())?;
        let schema = input.schema.as_deref();
        let constraints = conn
            .list_constraints(schema, input.table.as_deref(), input.limit.unwrap_or(0))
            .await?;
        Ok(ListConstraintsOutput {
            connection: conn.info().name,
            constraints,
        })
    }

    pub(super) async fn list_indexes(
        &self,
        input: ListConstraintsInput,
    ) -> Result<ListIndexesOutput> {
        let conn = self.state.manager.connection(input.connection.as_deref())?;
        let schema = input.schema.as_deref();
        let indexes = conn
            .list_indexes(schema, input.table.as_deref(), input.limit.unwrap_or(0))
            .await?;
        Ok(ListIndexesOutput {
            connection: conn.info().name,
            indexes,
        })
    }

    pub(super) async fn get_table_comments(
        &self,
        input: TableInput,
    ) -> Result<GetTableCommentsOutput> {
        let conn = self.state.manager.connection(input.connection.as_deref())?;
        let comments = conn
            .get_table_comments(input.schema.as_deref(), &input.table)
            .await?;
        Ok(GetTableCommentsOutput {
            connection: conn.info().name,
            comments,
        })
    }

    pub(super) async fn describe_table(&self, input: TableInput) -> Result<DescribeTableOutput> {
        let conn = self.state.manager.connection(input.connection.as_deref())?;
        let columns = conn
            .describe_table(input.schema.as_deref(), &input.table)
            .await?;
        Ok(DescribeTableOutput {
            connection: conn.info().name,
            schema: input
                .schema
                .clone()
                .unwrap_or_else(|| conn.info().default_schema),
            table: input.table,
            columns,
        })
    }

    pub(super) async fn get_table_ddl(&self, input: TableInput) -> Result<GetTableDDLOutput> {
        let conn = self.state.manager.connection(input.connection.as_deref())?;
        let ddl = conn
            .get_table_ddl(input.schema.as_deref(), &input.table)
            .await?;
        Ok(GetTableDDLOutput {
            connection: conn.info().name,
            ddl,
        })
    }

    pub(super) async fn execute_query(
        &self,
        input: ExecuteQueryInput,
    ) -> Result<ExecuteQueryOutput> {
        let conn = self.state.manager.connection(input.connection.as_deref())?;
        let result = conn
            .execute_query(
                &input.sql,
                input.max_rows.unwrap_or(0),
                self.state.manager.limits(),
            )
            .await?;
        Ok(ExecuteQueryOutput {
            connection: conn.info().name,
            result,
        })
    }

    pub(super) async fn execute_queries(
        &self,
        input: ExecuteQueriesInput,
    ) -> Result<ExecuteQueriesOutput> {
        let conn = self.state.manager.connection(input.connection.as_deref())?;
        execute_query_batch(&*conn, input, self.state.manager.limits()).await
    }

    pub(super) async fn explain_query(
        &self,
        input: ExplainQueryInput,
    ) -> Result<ExplainQueryOutput> {
        let conn = self.state.manager.connection(input.connection.as_deref())?;
        let explain = conn
            .explain_query(&input.sql, self.state.manager.limits())
            .await?;
        Ok(ExplainQueryOutput {
            connection: conn.info().name,
            explain,
        })
    }
}
