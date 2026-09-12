use rmcp::model::Tool;
use serde_json::{Map, Value};

use crate::database::{self, query_annotations, read_only_annotations};
use crate::mcp_types::*;

use super::DbMcpService;
use super::server::{RunToolResult, parse_or_error};

/// The original thirteen-tool surface, kept verbatim for clients that have not
/// migrated to the canonical profile.
pub fn build_legacy_tools() -> Vec<Tool> {
    vec![
        database::tool_schema::<ListConnectionsInput, ListConnectionsOutput>(
            "list_connections",
            "List configured database connections.",
            read_only_annotations(),
        ),
        database::tool_schema::<DiagnoseConnectionInput, DiagnoseConnectionOutput>(
            "diagnose_connection",
            "Run staged connection diagnostics for one or all configured databases.",
            read_only_annotations(),
        ),
        database::tool_schema::<ListInput, ListSchemasOutput>(
            "list_schemas",
            "List visible schemas for a named connection. Call list_connections first.",
            read_only_annotations(),
        ),
        database::tool_schema::<ListInput, ListTablesOutput>(
            "list_tables",
            "List tables and views in a schema for a named connection.",
            read_only_annotations(),
        ),
        database::tool_schema::<SearchColumnsInput, SearchColumnsOutput>(
            "search_columns",
            "Search visible columns by name within a schema for a named connection.",
            read_only_annotations(),
        ),
        database::tool_schema::<ListConstraintsInput, ListIndexesOutput>(
            "list_indexes",
            "List visible indexes for a schema or table on a named connection.",
            read_only_annotations(),
        ),
        database::tool_schema::<ListConstraintsInput, ListConstraintsOutput>(
            "list_constraints",
            "List visible table constraints for a schema or table on a named connection.",
            read_only_annotations(),
        ),
        database::tool_schema::<TableInput, GetTableCommentsOutput>(
            "get_table_comments",
            "Get table and column comments for a table or view on a named connection.",
            read_only_annotations(),
        ),
        database::tool_schema::<TableInput, DescribeTableOutput>(
            "describe_table",
            "Describe table or view columns on a named connection.",
            read_only_annotations(),
        ),
        database::tool_schema::<TableInput, GetTableDDLOutput>(
            "get_table_ddl",
            "Return a table or view DDL definition in a normalized read-only form for a named connection.",
            read_only_annotations(),
        ),
        database::tool_schema::<ExecuteQueryInput, ExecuteQueryOutput>(
            "execute_query",
            "Run one single-statement query on a named connection. A trailing semicolon is allowed; SQL scripts are not supported. Results are truncated to protect MCP context.",
            query_annotations(),
        ),
        database::tool_schema::<ExecuteQueriesInput, ExecuteQueriesOutput>(
            "execute_queries",
            "Run 1 to 8 read-only single-statement queries sequentially on a named connection. Each query may span multiple lines and end with a semicolon; SQL scripts are not supported. Results are returned separately and execution stops at the first failure.",
            query_annotations(),
        ),
        database::tool_schema::<ExplainQueryInput, ExplainQueryOutput>(
            "explain_query",
            "Explain a query on a named connection and return the execution plan.",
            read_only_annotations(),
        ),
    ]
}

pub(super) async fn dispatch_legacy(
    service: &DbMcpService,
    name: &str,
    arguments: Option<Map<String, Value>>,
) -> Option<RunToolResult> {
    macro_rules! legacy_call {
        ($input:ty, $method:ident) => {{
            let input: $input = match parse_or_error(arguments) {
                Ok(input) => input,
                Err(error) => return Some(error),
            };
            let result = service
                .$method(input)
                .await
                .and_then(|output| database::call_tool_json(&output));
            RunToolResult::from_handler(result)
        }};
    }

    let result = match name {
        "list_connections" => legacy_call!(ListConnectionsInput, list_connections),
        "diagnose_connection" => legacy_call!(DiagnoseConnectionInput, diagnose_connection),
        "list_schemas" => legacy_call!(ListInput, list_schemas),
        "list_tables" => legacy_call!(ListInput, list_tables),
        "search_columns" => legacy_call!(SearchColumnsInput, search_columns),
        "list_indexes" => legacy_call!(ListConstraintsInput, list_indexes),
        "list_constraints" => legacy_call!(ListConstraintsInput, list_constraints),
        "get_table_comments" => legacy_call!(TableInput, get_table_comments),
        "describe_table" => legacy_call!(TableInput, describe_table),
        "get_table_ddl" => legacy_call!(TableInput, get_table_ddl),
        "execute_query" => legacy_call!(ExecuteQueryInput, execute_query),
        "execute_queries" => legacy_call!(ExecuteQueriesInput, execute_queries),
        "explain_query" => legacy_call!(ExplainQueryInput, explain_query),
        _ => return None,
    };
    Some(result)
}
