use anyhow::Result;

use crate::mcp::types_canonical::{DbIntrospectInput, DbIntrospectOutput, IntrospectAction};
use crate::mcp_types::*;

use super::DbMcpService;

impl DbMcpService {
    /// Map a canonical `db_introspect` action onto the existing metadata
    /// handlers. Table-scoped actions are validated before dispatch, so the
    /// empty table fallback here is unreachable and never queried.
    pub(super) async fn introspect(&self, input: DbIntrospectInput) -> Result<DbIntrospectOutput> {
        let connection = input.connection.clone();
        let schema = input.schema.clone();
        let table = input.table.clone();
        let filter = input.filter.clone();
        let limit = input.limit;

        Ok(match input.action {
            IntrospectAction::Schemas => DbIntrospectOutput::Schemas(
                self.list_schemas(ListInput {
                    connection,
                    schema,
                    filter,
                    limit,
                })
                .await?,
            ),
            IntrospectAction::Tables => DbIntrospectOutput::Tables(
                self.list_tables(ListInput {
                    connection,
                    schema,
                    filter,
                    limit,
                })
                .await?,
            ),
            IntrospectAction::Columns => DbIntrospectOutput::Columns(
                self.describe_table(TableInput {
                    connection,
                    schema,
                    table: table.unwrap_or_default(),
                })
                .await?,
            ),
            IntrospectAction::Constraints => DbIntrospectOutput::Constraints(
                self.list_constraints(ListConstraintsInput {
                    connection,
                    schema,
                    table,
                    filter,
                    limit,
                })
                .await?,
            ),
            IntrospectAction::Indexes => DbIntrospectOutput::Indexes(
                self.list_indexes(ListConstraintsInput {
                    connection,
                    schema,
                    table,
                    filter,
                    limit,
                })
                .await?,
            ),
            IntrospectAction::Comments => DbIntrospectOutput::Comments(
                self.get_table_comments(TableInput {
                    connection,
                    schema,
                    table: table.unwrap_or_default(),
                })
                .await?,
            ),
            IntrospectAction::Ddl => DbIntrospectOutput::Ddl(
                self.get_table_ddl(TableInput {
                    connection,
                    schema,
                    table: table.unwrap_or_default(),
                })
                .await?,
            ),
        })
    }
}
