mod commands;
mod exit;
mod format;

pub use commands::{command_name, run};
pub use exit::{CliError, CliErrorKind};
pub use format::{error_json, render_success};

use std::path::PathBuf;

use clap::{Args, Subcommand};

/// Progressive CLI command tree. Omitting the subcommand preserves the current
/// MCP server startup behaviour; `serve` is the explicit form.
#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    Serve(ServeArgs),
    Connections(ConnectionsCommand),
    Introspect(IntrospectCommand),
    Query(QueryCommand),
    Explain(ExplainCommand),
}

/// Start the MCP server; this is also the default when no subcommand is given.
#[derive(Debug, Clone, Args)]
pub struct ServeArgs {}

/// Inspect configured database connections.
#[derive(Debug, Clone, Args)]
pub struct ConnectionsCommand {
    #[command(subcommand)]
    pub action: ConnectionsAction,
}

#[derive(Debug, Clone, Subcommand)]
pub enum ConnectionsAction {
    List(ConnectionsListArgs),
}

#[derive(Debug, Clone, Args)]
pub struct ConnectionsListArgs {
    /// Re-probe connection health before listing.
    #[arg(long)]
    pub refresh: bool,
    /// Run staged connection diagnostics instead of the cached health list.
    #[arg(long)]
    pub deep: bool,
}

/// Inspect database metadata by action.
#[derive(Debug, Clone, Args)]
pub struct IntrospectCommand {
    /// Metadata action: schemas, tables, columns, constraints, indexes,
    /// comments, or ddl.
    #[arg(value_enum)]
    pub action: IntrospectActionArg,
    #[arg(long)]
    pub connection: String,
    #[arg(long)]
    pub schema: Option<String>,
    #[arg(long)]
    pub table: Option<String>,
    #[arg(long)]
    pub filter: Option<String>,
    #[arg(long)]
    pub limit: Option<usize>,
}

/// Run one read-only SQL statement on a named connection.
#[derive(Debug, Clone, Args)]
pub struct QueryCommand {
    #[arg(long)]
    pub connection: String,
    #[command(flatten)]
    pub input: SqlInputArgs,
    #[arg(long)]
    pub max_rows: Option<usize>,
}

/// Explain one SQL statement on a named connection.
#[derive(Debug, Clone, Args)]
pub struct ExplainCommand {
    #[arg(long)]
    pub connection: String,
    #[command(flatten)]
    pub input: SqlInputArgs,
}

/// The three mutually exclusive SQL input sources for `query` and `explain`.
#[derive(Debug, Clone, Args)]
pub struct SqlInputArgs {
    /// Inline SQL text.
    #[arg(long, conflicts_with_all = ["file", "stdin"])]
    pub sql: Option<String>,
    /// Read SQL from a file.
    #[arg(long, conflicts_with_all = ["sql", "stdin"])]
    pub file: Option<PathBuf>,
    /// Read SQL from standard input.
    #[arg(long, conflicts_with_all = ["sql", "file"])]
    pub stdin: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum IntrospectActionArg {
    Schemas,
    Tables,
    Columns,
    Constraints,
    Indexes,
    Comments,
    Ddl,
}

impl From<IntrospectActionArg> for crate::mcp::types_canonical::IntrospectAction {
    fn from(value: IntrospectActionArg) -> Self {
        use crate::mcp::types_canonical::IntrospectAction;
        match value {
            IntrospectActionArg::Schemas => IntrospectAction::Schemas,
            IntrospectActionArg::Tables => IntrospectAction::Tables,
            IntrospectActionArg::Columns => IntrospectAction::Columns,
            IntrospectActionArg::Constraints => IntrospectAction::Constraints,
            IntrospectActionArg::Indexes => IntrospectAction::Indexes,
            IntrospectActionArg::Comments => IntrospectAction::Comments,
            IntrospectActionArg::Ddl => IntrospectAction::Ddl,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Cli, OutputFormat};
    use clap::Parser;

    fn parse(args: &[&str]) -> Cli {
        Cli::try_parse_from(args).expect("parse cli")
    }

    #[test]
    fn no_subcommand_preserves_serve_default() {
        let cli = parse(&["dbmcp"]);
        assert!(cli.command.is_none());
        assert_eq!(cli.format, OutputFormat::Json);
    }

    #[test]
    fn serve_subcommand_parses() {
        let cli = parse(&["dbmcp", "serve"]);
        assert!(matches!(cli.command, Some(Command::Serve(_))));
    }

    #[test]
    fn connections_list_parses_flags() {
        let cli = parse(&["dbmcp", "connections", "list", "--refresh", "--deep"]);
        match cli.command {
            Some(Command::Connections(ConnectionsCommand {
                action: ConnectionsAction::List(args),
            })) => {
                assert!(args.refresh);
                assert!(args.deep);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn introspect_parses_action_and_arguments() {
        let cli = parse(&[
            "dbmcp",
            "introspect",
            "columns",
            "--connection",
            "sqlite",
            "--schema",
            "main",
            "--table",
            "orders",
            "--limit",
            "10",
        ]);
        match cli.command {
            Some(Command::Introspect(args)) => {
                assert_eq!(args.action, IntrospectActionArg::Columns);
                assert_eq!(args.connection, "sqlite");
                assert_eq!(args.table.as_deref(), Some("orders"));
                assert_eq!(args.limit, Some(10));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn query_parses_inline_sql_and_max_rows() {
        let cli = parse(&[
            "dbmcp",
            "query",
            "--connection",
            "sqlite",
            "--sql",
            "select 1",
            "--max-rows",
            "3",
        ]);
        match cli.command {
            Some(Command::Query(args)) => {
                assert_eq!(args.connection, "sqlite");
                assert_eq!(args.input.sql.as_deref(), Some("select 1"));
                assert_eq!(args.max_rows, Some(3));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn explain_rejects_conflicting_input_sources() {
        let both = Cli::try_parse_from([
            "dbmcp",
            "explain",
            "--connection",
            "sqlite",
            "--sql",
            "select 1",
            "--file",
            "query.sql",
        ]);
        assert!(both.is_err(), "conflicting sources must be rejected");
    }

    #[test]
    fn format_flag_is_global_and_validated() {
        let cli = parse(&[
            "dbmcp",
            "query",
            "--connection",
            "sqlite",
            "--sql",
            "q",
            "--format",
            "text",
        ]);
        assert_eq!(cli.format, OutputFormat::Text);
        assert!(Cli::try_parse_from(["dbmcp", "--format", "yaml"]).is_err());
    }

    #[test]
    fn unknown_subcommand_is_rejected() {
        assert!(Cli::try_parse_from(["dbmcp", "bogus"]).is_err());
    }
}
