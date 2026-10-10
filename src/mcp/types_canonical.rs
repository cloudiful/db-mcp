use anyhow::{Result, bail};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::config::MAX_BATCH_QUERIES;
use crate::mcp_types::{
    DescribeTableOutput, DiagnoseConnectionOutput, ExecuteQueriesOutput, ExecuteQueryOutput,
    GetTableCommentsOutput, GetTableDDLOutput, ListConnectionsOutput, ListConstraintsOutput,
    ListIndexesOutput, ListSchemasOutput, ListTablesOutput,
};

/// Selector for the canonical `db_connections` tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionAction {
    List,
    Diagnose,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
pub struct DbConnectionsInput {
    pub action: ConnectionAction,
    pub connection: Option<String>,
    pub refresh: Option<bool>,
    pub deep: Option<bool>,
}

/// Selector for the canonical `db_introspect` tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IntrospectAction {
    Schemas,
    Tables,
    Columns,
    Constraints,
    Indexes,
    Comments,
    Ddl,
}

impl IntrospectAction {
    /// Table-scoped actions need an explicit `table` argument.
    pub fn requires_table(self) -> bool {
        matches!(self, Self::Columns | Self::Comments | Self::Ddl)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
pub struct DbIntrospectInput {
    pub action: IntrospectAction,
    pub connection: Option<String>,
    pub schema: Option<String>,
    pub table: Option<String>,
    pub filter: Option<String>,
    pub limit: Option<usize>,
}

impl DbIntrospectInput {
    pub fn validate(&self) -> Result<()> {
        if self.action.requires_table()
            && self
                .table
                .as_deref()
                .map(str::trim)
                .unwrap_or_default()
                .is_empty()
        {
            bail!(
                "db_introspect action {:?} requires a non-empty table",
                self.action
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
pub struct DbQueryItem {
    pub sql: String,
}

/// Canonical `db_query` input. `sql` and `queries` are mutually exclusive and
/// exactly one must be present; `queries` carries the 1-8 batch bound.
///
/// The exclusivity rule is enforced by [`DbQueryInput::validate`] rather than
/// by a top-level schema combinator: Anthropic and Bedrock reject
/// `input_schema` whose root carries `oneOf`, `allOf` or `anyOf`, so such a
/// schema makes the tool unusable on those upstreams.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
pub struct DbQueryInput {
    pub connection: Option<String>,
    #[schemars(
        description = "Mutually exclusive with `queries`: provide exactly one of `sql` or `queries`."
    )]
    pub sql: Option<String>,
    #[schemars(
        length(min = 1, max = MAX_BATCH_QUERIES),
        description = "Mutually exclusive with `sql`: 1 to 8 statements run in order. Provide exactly one of `sql` or `queries`."
    )]
    pub queries: Option<Vec<DbQueryItem>>,
    pub max_rows: Option<usize>,
}

impl DbQueryInput {
    /// Server-side validation of the contract encoded in the JSON Schema.
    /// MCP clients may skip schema validation, so dispatch must call this.
    pub fn validate(&self) -> Result<()> {
        match (&self.sql, &self.queries) {
            (Some(_), Some(_)) => bail!("db_query requires either sql or queries, not both"),
            (None, None) => bail!("db_query requires sql or queries"),
            (Some(_), None) => Ok(()),
            (None, Some(queries)) => {
                if !(1..=MAX_BATCH_QUERIES).contains(&queries.len()) {
                    bail!("queries must contain between 1 and {MAX_BATCH_QUERIES} statements")
                }
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
pub struct DbExplainInput {
    pub connection: Option<String>,
    pub sql: String,
}

/// Canonical `db_connections` output. Reuses the existing typed outputs behind
/// a `kind`/`data` serde discriminator.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum DbConnectionsOutput {
    List(ListConnectionsOutput),
    Diagnose(DiagnoseConnectionOutput),
}

/// Canonical `db_introspect` output. Each action reuses the typed output of
/// the legacy metadata entry point it replaces.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum DbIntrospectOutput {
    Schemas(ListSchemasOutput),
    Tables(ListTablesOutput),
    Columns(DescribeTableOutput),
    Constraints(ListConstraintsOutput),
    Indexes(ListIndexesOutput),
    Comments(GetTableCommentsOutput),
    Ddl(GetTableDDLOutput),
}

/// Canonical `db_query` output, distinguishing a single statement from a batch.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum DbQueryOutput {
    Single(ExecuteQueryOutput),
    Batch(ExecuteQueriesOutput),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database;
    use serde_json::{Value, json};

    fn action_values(schema: &serde_json::Map<String, Value>) -> Vec<String> {
        let mut action = &schema["properties"]["action"];
        if let Some(reference) = action["$ref"].as_str() {
            action = &schema["$defs"][reference.rsplit('/').next().expect("definition name")];
        }
        action["enum"]
            .as_array()
            .expect("action enum array")
            .iter()
            .map(|value| value.as_str().expect("variant name").to_string())
            .collect()
    }

    fn query_item() -> DbQueryItem {
        DbQueryItem {
            sql: "select 1".to_string(),
        }
    }

    fn empty_query() -> DbQueryInput {
        DbQueryInput {
            connection: None,
            sql: None,
            queries: None,
            max_rows: None,
        }
    }

    fn introspect(action: IntrospectAction, table: Option<&str>) -> DbIntrospectInput {
        DbIntrospectInput {
            action,
            connection: None,
            schema: None,
            table: table.map(str::to_string),
            filter: None,
            limit: None,
        }
    }

    #[test]
    fn action_schemas_list_two_and_seven_variants() {
        let connections = database::to_json_object::<DbConnectionsInput>();
        assert_eq!(action_values(&connections), ["list", "diagnose"]);

        let introspect = database::to_json_object::<DbIntrospectInput>();
        assert_eq!(
            action_values(&introspect),
            [
                "schemas",
                "tables",
                "columns",
                "constraints",
                "indexes",
                "comments",
                "ddl"
            ]
        );
    }

    #[test]
    fn db_query_schema_encodes_batch_bounds() {
        let schema = database::to_json_object::<DbQueryInput>();
        assert_eq!(
            schema["properties"]["queries"]["minItems"].as_u64(),
            Some(1)
        );
        assert_eq!(
            schema["properties"]["queries"]["maxItems"].as_u64(),
            Some(8)
        );
    }

    #[test]
    fn canonical_inputs_deserialize_and_reject_unknown_actions() {
        let query: DbQueryInput = serde_json::from_value(json!({"sql": "select 1", "max_rows": 5}))
            .expect("deserialize db_query input");
        assert_eq!(query.sql.as_deref(), Some("select 1"));
        assert_eq!(query.max_rows, Some(5));

        let connections: DbConnectionsInput =
            serde_json::from_value(json!({"action": "diagnose", "deep": true}))
                .expect("deserialize db_connections input");
        assert_eq!(connections.action, ConnectionAction::Diagnose);

        assert!(serde_json::from_value::<DbConnectionsInput>(json!({"action": "ping"})).is_err());
        assert!(serde_json::from_value::<DbConnectionsInput>(json!({})).is_err());
        assert!(
            serde_json::from_value::<DbIntrospectInput>(json!({"action": "procedures"})).is_err()
        );
    }

    #[test]
    fn db_query_accepts_valid_single_and_batch_forms() {
        let single = DbQueryInput {
            sql: Some("select 1".to_string()),
            ..empty_query()
        };
        assert!(single.validate().is_ok());

        for size in [1, 8] {
            let batch = DbQueryInput {
                queries: Some((0..size).map(|_| query_item()).collect()),
                ..empty_query()
            };
            assert!(batch.validate().is_ok(), "batch of {size} should be valid");
        }
    }

    #[test]
    fn db_query_rejects_conflicting_missing_and_oversized_forms() {
        let both = DbQueryInput {
            sql: Some("select 1".to_string()),
            queries: Some(vec![query_item()]),
            ..empty_query()
        };
        assert!(both.validate().is_err());
        assert!(empty_query().validate().is_err());

        let empty = DbQueryInput {
            queries: Some(vec![]),
            ..empty_query()
        };
        assert!(empty.validate().is_err());

        let oversized = DbQueryInput {
            queries: Some((0..9).map(|_| query_item()).collect()),
            ..empty_query()
        };
        assert!(oversized.validate().is_err());
    }

    #[test]
    fn introspect_table_actions_require_table() {
        for action in [
            IntrospectAction::Columns,
            IntrospectAction::Comments,
            IntrospectAction::Ddl,
        ] {
            assert!(introspect(action, None).validate().is_err(), "{action:?}");
            assert!(introspect(action, Some("orders")).validate().is_ok());
        }

        for action in [
            IntrospectAction::Schemas,
            IntrospectAction::Tables,
            IntrospectAction::Constraints,
            IntrospectAction::Indexes,
        ] {
            assert!(introspect(action, None).validate().is_ok(), "{action:?}");
        }
    }

    #[test]
    fn outputs_keep_kind_data_discriminator() {
        let connections = DbConnectionsOutput::List(ListConnectionsOutput {
            connections: vec![],
        });
        let value = serde_json::to_value(&connections).expect("serialize connections output");
        assert_eq!(value["kind"], "list");
        assert_eq!(value["data"]["connections"], json!([]));

        let batch = DbQueryOutput::Batch(ExecuteQueriesOutput {
            connection: "sqlite".to_string(),
            status: "completed".to_string(),
            statement_count: 0,
            results: vec![],
            error: None,
        });
        let value = serde_json::to_value(&batch).expect("serialize query output");
        assert_eq!(value["kind"], "batch");
        assert_eq!(value["data"]["connection"], "sqlite");
    }
}
