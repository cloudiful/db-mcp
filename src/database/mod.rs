use anyhow::Result;

mod diagnostics;
mod manager;
mod mcp_support;
pub mod oracle;
mod oracle_jdbc;
mod oracle_metadata;
mod oracle_query;
pub mod postgres;
mod postgres_query;
mod rows;
pub mod sqlite;
mod sqlite_dsn;
mod types;
mod unavailable;

pub use diagnostics::*;
pub use manager::*;
pub use mcp_support::*;
pub use rows::*;
pub(crate) use sqlite_dsn::sqlite_dsn_is_read_only;
pub use types::*;

pub fn chrono_like_now() -> String {
    chrono::Utc::now().to_rfc3339()
}

pub fn call_tool_ok<T: serde::Serialize>(
    text: impl Into<String>,
    value: &T,
) -> Result<rmcp::model::CallToolResult> {
    call_tool_json(text, value)
}
