use std::io::Read;
use std::process::ExitCode;
use std::sync::Arc;

use anyhow::Result;
use serde::Serialize;
use serde_json::Value;

use crate::config::{Config, OutputFormat};
use crate::database::{DatabaseConnection, Manager};
use crate::mcp::types_canonical::{
    DbConnectionsOutput, DbIntrospectInput, DbIntrospectOutput, DbQueryOutput, IntrospectAction,
};
use crate::mcp_types::{
    DescribeTableOutput, DiagnoseConnectionOutput, ExecuteQueryOutput, ExplainQueryOutput,
    GetTableCommentsOutput, GetTableDDLOutput, ListConnectionsOutput, ListConstraintsOutput,
    ListIndexesOutput, ListSchemasOutput, ListTablesOutput,
};

use super::exit::{CliError, CliErrorKind};
use super::{
    Command, ConnectionsAction, ConnectionsCommand, ExplainCommand, IntrospectCommand,
    QueryCommand, SqlInputArgs, format,
};

const READ_ONLY_REQUIRED: &str = "query requires a read-only connection; SQLite connections also require a read-only DSN such as mode=ro";

/// Execute one progressive CLI command against the shared database domain.
///
/// Success writes the business object to stdout; failures are returned so the
/// runtime can render the `{error, kind, command}` payload on stderr.
pub async fn run(
    command: Command,
    format: OutputFormat,
    cfg: Config,
) -> Result<ExitCode, CliError> {
    let manager = Manager::new(&cfg)
        .await
        .map_err(|err| CliError::with_kind(CliErrorKind::Internal, err))?;

    let value = match command {
        Command::Serve(_) => {
            return Err(CliError::internal(
                "serve is handled by the runtime, not the CLI dispatcher",
            ));
        }
        Command::Connections(command) => run_connections(command, &manager).await?,
        Command::Introspect(command) => run_introspect(command, &manager).await?,
        Command::Query(command) => run_query(command, &manager).await?,
        Command::Explain(command) => run_explain(command, &manager).await?,
    };

    let rendered = format::render_success(format, &value)?;
    println!("{rendered}");
    Ok(ExitCode::SUCCESS)
}

pub fn command_name(command: &Command) -> &'static str {
    match command {
        Command::Serve(_) => "serve",
        Command::Connections(_) => "connections list",
        Command::Introspect(_) => "introspect",
        Command::Query(_) => "query",
        Command::Explain(_) => "explain",
    }
}

async fn run_connections(
    command: ConnectionsCommand,
    manager: &Manager,
) -> Result<Value, CliError> {
    match command.action {
        ConnectionsAction::List(args) => {
            let output = if args.deep {
                let diagnostics = manager
                    .diagnose(None, true)
                    .await
                    .map_err(CliError::from_anyhow)?;
                DbConnectionsOutput::Diagnose(DiagnoseConnectionOutput { diagnostics })
            } else {
                let health = manager
                    .health(None, args.refresh)
                    .await
                    .map_err(CliError::from_anyhow)?;
                DbConnectionsOutput::List(ListConnectionsOutput {
                    connections: health.connections,
                })
            };
            to_json(&output)
        }
    }
}

async fn run_introspect(command: IntrospectCommand, manager: &Manager) -> Result<Value, CliError> {
    let input = DbIntrospectInput {
        action: command.action.into(),
        connection: Some(command.connection),
        schema: command.schema,
        table: command.table,
        filter: command.filter,
        limit: command.limit,
    };
    input
        .validate()
        .map_err(|err| CliError::invalid_args(err.to_string()))?;

    let conn = resolve_connection(manager, input.connection.as_deref().unwrap_or_default())?;
    let name = conn.info().name;
    let schema = input.schema.as_deref();
    let table = input.table.as_deref().unwrap_or_default();
    let filter = input.filter.as_deref();
    let limit = input.limit.unwrap_or(0);

    let output = match input.action {
        IntrospectAction::Schemas => DbIntrospectOutput::Schemas(ListSchemasOutput {
            connection: name,
            schemas: conn
                .list_schemas(filter, limit)
                .await
                .map_err(CliError::from_anyhow)?,
        }),
        IntrospectAction::Tables => DbIntrospectOutput::Tables(ListTablesOutput {
            connection: name,
            tables: conn
                .list_tables(schema, filter, limit)
                .await
                .map_err(CliError::from_anyhow)?,
        }),
        IntrospectAction::Columns => DbIntrospectOutput::Columns(DescribeTableOutput {
            connection: name,
            schema: input
                .schema
                .clone()
                .unwrap_or_else(|| conn.info().default_schema),
            table: table.to_string(),
            columns: conn
                .describe_table(schema, table)
                .await
                .map_err(CliError::from_anyhow)?,
        }),
        IntrospectAction::Constraints => DbIntrospectOutput::Constraints(ListConstraintsOutput {
            connection: name,
            constraints: conn
                .list_constraints(schema, input.table.as_deref(), limit)
                .await
                .map_err(CliError::from_anyhow)?,
        }),
        IntrospectAction::Indexes => DbIntrospectOutput::Indexes(ListIndexesOutput {
            connection: name,
            indexes: conn
                .list_indexes(schema, input.table.as_deref(), limit)
                .await
                .map_err(CliError::from_anyhow)?,
        }),
        IntrospectAction::Comments => DbIntrospectOutput::Comments(GetTableCommentsOutput {
            connection: name,
            comments: conn
                .get_table_comments(schema, table)
                .await
                .map_err(CliError::from_anyhow)?,
        }),
        IntrospectAction::Ddl => DbIntrospectOutput::Ddl(GetTableDDLOutput {
            connection: name,
            ddl: conn
                .get_table_ddl(schema, table)
                .await
                .map_err(CliError::from_anyhow)?,
        }),
    };
    to_json(&output)
}

async fn run_query(command: QueryCommand, manager: &Manager) -> Result<Value, CliError> {
    let sql = read_sql(&command.input)?;
    let conn = resolve_connection(manager, &command.connection)?;
    if !conn.supports_read_only_batch() {
        return Err(CliError::invalid_args(READ_ONLY_REQUIRED));
    }
    let result = conn
        .execute_query(&sql, command.max_rows.unwrap_or(0), manager.limits())
        .await
        .map_err(CliError::from_anyhow)?;
    to_json(&DbQueryOutput::Single(ExecuteQueryOutput {
        connection: conn.info().name,
        result,
    }))
}

async fn run_explain(command: ExplainCommand, manager: &Manager) -> Result<Value, CliError> {
    let sql = read_sql(&command.input)?;
    let conn = resolve_connection(manager, &command.connection)?;
    let explain = conn
        .explain_query(&sql, manager.limits())
        .await
        .map_err(CliError::from_anyhow)?;
    to_json(&ExplainQueryOutput {
        connection: conn.info().name,
        explain,
    })
}

fn resolve_connection(
    manager: &Manager,
    name: &str,
) -> Result<Arc<dyn DatabaseConnection>, CliError> {
    manager
        .connection(Some(name))
        .map_err(CliError::from_anyhow)
}

/// Resolve exactly one of `--sql`, `--file`, or `--stdin` into SQL text.
fn read_sql(input: &SqlInputArgs) -> Result<String, CliError> {
    let sources = [input.sql.is_some(), input.file.is_some(), input.stdin]
        .into_iter()
        .filter(|present| *present)
        .count();
    if sources != 1 {
        return Err(CliError::invalid_args(
            "exactly one of --sql, --file, or --stdin is required",
        ));
    }

    let raw = if let Some(sql) = &input.sql {
        sql.clone()
    } else if let Some(path) = &input.file {
        std::fs::read_to_string(path)
            .map_err(|err| CliError::internal(format!("read SQL file {}: {err}", path.display())))?
    } else {
        let mut buffer = String::new();
        std::io::stdin()
            .read_to_string(&mut buffer)
            .map_err(|err| CliError::internal(format!("read SQL from stdin: {err}")))?;
        buffer
    };

    if raw.trim().is_empty() {
        return Err(CliError::invalid_args("SQL input is empty"));
    }
    Ok(raw)
}

fn to_json<T: Serialize>(value: &T) -> Result<Value, CliError> {
    serde_json::to_value(value).map_err(|err| CliError::internal(err.to_string()))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::cli::ServeArgs;

    fn input(sql: Option<&str>, file: Option<&str>, stdin: bool) -> SqlInputArgs {
        SqlInputArgs {
            sql: sql.map(str::to_string),
            file: file.map(PathBuf::from),
            stdin,
        }
    }

    #[test]
    fn read_sql_accepts_inline_text() {
        let sql = read_sql(&input(Some("select 1"), None, false)).expect("inline sql");
        assert_eq!(sql, "select 1");
    }

    #[test]
    fn read_sql_requires_exactly_one_source() {
        let none = read_sql(&input(None, None, false)).expect_err("no source");
        assert_eq!(none.kind, CliErrorKind::InvalidArgs);

        let both =
            read_sql(&input(Some("select 1"), Some("query.sql"), false)).expect_err("two sources");
        assert_eq!(both.kind, CliErrorKind::InvalidArgs);
    }

    #[test]
    fn read_sql_rejects_empty_inline_text() {
        let err = read_sql(&input(Some("   "), None, false)).expect_err("empty sql");
        assert_eq!(err.kind, CliErrorKind::InvalidArgs);
    }

    #[test]
    fn command_names_are_stable() {
        assert_eq!(command_name(&Command::Serve(ServeArgs {})), "serve");
    }
}
