use anyhow::{Context, Result};
use sqlx::postgres::PgPool;
use std::sync::{Arc, RwLock};

use crate::config;

use super::*;

#[derive(Clone)]
pub struct PostgresConnection {
    name: String,
    config: config::PostgresConfig,
    limits: Limits,
    pool: PgPool,
    default_schema: Arc<RwLock<String>>,
}

impl PostgresConnection {
    pub fn new(name: String, config: config::PostgresConfig, limits: Limits) -> Result<Self> {
        let pool = PgPool::connect_lazy(&config.dsn)?;
        let default_schema = if config.schema.trim().is_empty() {
            "public".to_string()
        } else {
            config.schema.trim().to_string()
        };
        Ok(Self {
            name,
            config,
            limits,
            pool,
            default_schema: Arc::new(RwLock::new(default_schema)),
        })
    }

    async fn schema(&self) -> Result<String> {
        Ok(self
            .default_schema
            .read()
            .expect("default_schema poisoned")
            .clone())
    }

    async fn refresh_schema(&self) -> Result<()> {
        if !self.config.schema.trim().is_empty() {
            *self
                .default_schema
                .write()
                .expect("default_schema poisoned") = self.config.schema.trim().to_string();
            return Ok(());
        }
        let schema: Option<String> = run_with_timeout(
            &self.limits,
            "postgres current_schema",
            sqlx::query_file_scalar!("sql/postgres/current_schema.sql").fetch_one(&self.pool),
        )
        .await?;
        if let Some(schema) = schema {
            *self
                .default_schema
                .write()
                .expect("default_schema poisoned") = schema;
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl DatabaseConnection for PostgresConnection {
    fn info(&self) -> ConnectionInfo {
        ConnectionInfo {
            name: self.name.clone(),
            driver: "postgres".to_string(),
            default_schema: self
                .default_schema
                .read()
                .expect("default_schema poisoned")
                .clone(),
            status: "down".to_string(),
            available: false,
            last_checked_at: None,
            last_successful_at: None,
            last_error: None,
            failure_phase: None,
            last_error_chain: vec![],
            probe_checks: vec![],
        }
    }

    fn target_summary(&self) -> String {
        redact_postgres_dsn(&self.config.dsn)
    }

    fn supports_read_only_batch(&self) -> bool {
        self.config.read_only
    }

    async fn ping(&self) -> Result<()> {
        tokio::time::timeout(
            self.limits.query_timeout,
            sqlx::query_file!("sql/postgres/ping.sql").fetch_one(&self.pool),
        )
        .await
        .with_context(|| {
            format!(
                "postgres ping timeout name={} target={}",
                self.name,
                redact_postgres_dsn(&self.config.dsn)
            )
        })?
        .with_context(|| {
            format!(
                "postgres ping failed name={} target={}",
                self.name,
                redact_postgres_dsn(&self.config.dsn)
            )
        })?;
        self.refresh_schema().await?;
        Ok(())
    }

    async fn diagnose(&self, deep: bool) -> ConnectionDiagnostic {
        let target = self.target_summary();
        let mut checks = vec![];
        checks.push(
            timed_probe("config", target.clone(), || async {
                if self.config.dsn.trim().is_empty() {
                    anyhow::bail!("postgres dsn is empty");
                }
                Ok(())
            })
            .await,
        );
        if checks.last().is_some_and(|check| check.status != "up") {
            return diagnostic_from_checks(self.info(), checks);
        }

        checks.push(
            timed_probe("connect/acquire", target.clone(), || async {
                tokio::time::timeout(self.limits.query_timeout, self.pool.acquire())
                    .await
                    .with_context(|| {
                        format!(
                            "postgres acquire timeout name={} target={}",
                            self.name,
                            redact_postgres_dsn(&self.config.dsn)
                        )
                    })?
                    .with_context(|| {
                        format!(
                            "postgres acquire failed name={} target={}",
                            self.name,
                            redact_postgres_dsn(&self.config.dsn)
                        )
                    })?;
                Ok(())
            })
            .await,
        );
        if checks.last().is_some_and(|check| check.status != "up") {
            return diagnostic_from_checks(self.info(), checks);
        }

        checks.push(
            timed_probe("pingQuery", target.clone(), || async {
                tokio::time::timeout(
                    self.limits.query_timeout,
                    sqlx::query_file!("sql/postgres/ping.sql").fetch_one(&self.pool),
                )
                .await
                .context("postgres ping query timeout")?
                .context("postgres ping query failed")?;
                Ok(())
            })
            .await,
        );
        if checks.last().is_some_and(|check| check.status != "up") {
            return diagnostic_from_checks(self.info(), checks);
        }

        checks.push(
            timed_probe("defaultSchema", target.clone(), || async {
                self.refresh_schema()
                    .await
                    .context("refresh postgres default schema")
            })
            .await,
        );
        if checks.last().is_some_and(|check| check.status != "up") || !deep {
            return diagnostic_from_checks(self.info(), checks);
        }

        checks.push(
            timed_probe("metadataSmokeTest", target, || async {
                run_with_timeout(
                    &self.limits,
                    "postgres metadata smoke test",
                    sqlx::query_file!("sql/postgres/metadata_smoke.sql").fetch_one(&self.pool),
                )
                .await?;
                Ok(())
            })
            .await,
        );
        diagnostic_from_checks(self.info(), checks)
    }

    async fn list_schemas(&self, filter: Option<&str>, limit: usize) -> Result<Vec<SchemaInfo>> {
        let limit = self.limits.normalize_list_limit(limit);
        let filter = filter.unwrap_or("");
        let rows = run_with_timeout(
            &self.limits,
            "postgres list_schemas",
            sqlx::query_file!(
                "sql/postgres/list_schemas.sql",
                format!("%{filter}%"),
                limit as i64
            )
            .fetch_all(&self.pool),
        )
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| SchemaInfo {
                name: row.schema_name.unwrap_or_default(),
            })
            .collect())
    }

    async fn list_tables(
        &self,
        schema: Option<&str>,
        filter: Option<&str>,
        limit: usize,
    ) -> Result<Vec<TableInfo>> {
        let limit = self.limits.normalize_list_limit(limit);
        let schema = schema.map(str::to_string).unwrap_or(self.schema().await?);
        let filter = filter.unwrap_or("");
        let rows = run_with_timeout(
            &self.limits,
            "postgres list_tables",
            sqlx::query_file!(
                "sql/postgres/list_tables.sql",
                &schema,
                format!("%{filter}%"),
                limit as i64
            )
            .fetch_all(&self.pool),
        )
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| TableInfo {
                schema: row.table_schema.unwrap_or_default(),
                name: row.table_name.unwrap_or_default(),
                table_type: row.table_type.unwrap_or_default(),
            })
            .collect())
    }

    async fn search_columns(
        &self,
        schema: Option<&str>,
        filter: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ColumnMatchInfo>> {
        let limit = self.limits.normalize_list_limit(limit);
        let schema = schema.map(str::to_string).unwrap_or(self.schema().await?);
        let filter = filter.unwrap_or("");
        let rows = run_with_timeout(
            &self.limits,
            "postgres search_columns",
            sqlx::query_file!(
                "sql/postgres/search_columns.sql",
                &schema,
                format!("%{filter}%"),
                limit as i64
            )
            .fetch_all(&self.pool),
        )
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| ColumnMatchInfo {
                schema: row.table_schema.unwrap_or_default(),
                table: row.table_name.unwrap_or_default(),
                column: row.column_name.unwrap_or_default(),
                data_type: row.data_type.unwrap_or_default(),
                position: row.ordinal_position.map(|p| p as i64).unwrap_or_default(),
            })
            .collect())
    }

    async fn list_constraints(
        &self,
        schema: Option<&str>,
        table: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ConstraintInfo>> {
        let schema = schema.map(str::to_string).unwrap_or(self.schema().await?);
        let table = table.context("table is required")?;
        let limit = self.limits.normalize_list_limit(limit);
        let rows = run_with_timeout(
            &self.limits,
            "postgres list_constraints",
            sqlx::query_file!(
                "sql/postgres/list_constraints.sql",
                &schema,
                table,
                limit as i64
            )
            .fetch_all(&self.pool),
        )
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| ConstraintInfo {
                name: row.name,
                schema: row.schema.unwrap_or_default(),
                table: row.table_name.unwrap_or_default(),
                constraint_type: row.constraint_type.unwrap_or_default(),
                columns: vec![],
                referenced_schema: row.referenced_schema,
                referenced_table: row.referenced_table,
                referenced_columns: vec![],
                check_expression: None,
                definition: row.definition,
            })
            .collect())
    }

    async fn list_indexes(
        &self,
        schema: Option<&str>,
        table: Option<&str>,
        limit: usize,
    ) -> Result<Vec<IndexInfo>> {
        let schema = schema.map(str::to_string).unwrap_or(self.schema().await?);
        let table = table.context("table is required")?;
        let limit = self.limits.normalize_list_limit(limit);
        let rows = run_with_timeout(
            &self.limits,
            "postgres list_indexes",
            sqlx::query_file!(
                "sql/postgres/list_indexes.sql",
                &schema,
                table,
                limit as i64
            )
            .fetch_all(&self.pool),
        )
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| {
                let definition = row.indexdef.unwrap_or_default();
                let columns = definition
                    .split('(')
                    .nth(1)
                    .and_then(|s| s.split(')').next())
                    .map(|s| s.split(',').map(|v| v.trim().to_string()).collect())
                    .unwrap_or_else(Vec::new);
                IndexInfo {
                    schema: row.schemaname.unwrap_or_default(),
                    table: row.tablename.unwrap_or_default(),
                    name: row.indexname.unwrap_or_default(),
                    index_type: Some("btree".to_string()),
                    unique: definition.to_uppercase().contains("UNIQUE INDEX"),
                    primary: definition.to_uppercase().contains("PRIMARY KEY"),
                    partial: None,
                    columns,
                    definition: Some(definition),
                }
            })
            .collect())
    }

    async fn get_table_comments(
        &self,
        schema: Option<&str>,
        table: &str,
    ) -> Result<TableCommentInfo> {
        let schema = schema.map(str::to_string).unwrap_or(self.schema().await?);
        let table_comment: Option<String> = run_with_timeout(
            &self.limits,
            "postgres get_table_comment",
            sqlx::query_file_scalar!("sql/postgres/get_table_comment.sql", &schema, table)
                .fetch_one(&self.pool),
        )
        .await
        .ok()
        .flatten();
        let rows = run_with_timeout(
            &self.limits,
            "postgres list_column_comments",
            sqlx::query_file!("sql/postgres/list_column_comments.sql", &schema, table)
                .fetch_all(&self.pool),
        )
        .await?;
        Ok(TableCommentInfo {
            schema,
            table: table.to_string(),
            table_comment,
            columns: rows
                .into_iter()
                .map(|row| ColumnCommentInfo {
                    name: row.name.unwrap_or_default(),
                    comment: row.comment.filter(|c| !c.is_empty()),
                })
                .collect(),
            notice: None,
        })
    }

    async fn describe_table(&self, schema: Option<&str>, table: &str) -> Result<Vec<ColumnInfo>> {
        let schema = schema.map(str::to_string).unwrap_or(self.schema().await?);
        let rows = run_with_timeout(
            &self.limits,
            "postgres describe_table",
            sqlx::query_file!("sql/postgres/describe_table.sql", &schema, table)
                .fetch_all(&self.pool),
        )
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| ColumnInfo {
                name: row.column_name.unwrap_or_default(),
                data_type: row.data_type.unwrap_or_default(),
                nullable: row.is_nullable.as_deref() == Some("YES"),
                position: row.ordinal_position.map(|p| p as i64).unwrap_or_default(),
                length: row.character_maximum_length.map(|v| v as i64),
                precision: row.numeric_precision.map(|v| v as i64),
                scale: row.numeric_scale.map(|v| v as i64),
            })
            .collect())
    }

    async fn get_table_ddl(&self, schema: Option<&str>, table: &str) -> Result<TableDDLInfo> {
        let schema = schema.map(str::to_string).unwrap_or(self.schema().await?);
        let object_type: Option<String> = run_with_timeout(
            &self.limits,
            "postgres get_table_type",
            sqlx::query_file_scalar!("sql/postgres/get_table_type.sql", &schema, table)
                .fetch_one(&self.pool),
        )
        .await?;
        let object_type = object_type
            .ok_or_else(|| anyhow::anyhow!("table or view {schema}.{table} not found"))?;
        let ddl: String = if object_type == "VIEW" {
            run_with_timeout(
                &self.limits,
                "postgres get_view_ddl",
                sqlx::query_file_scalar!("sql/postgres/get_view_ddl.sql", &schema, table)
                    .fetch_one(&self.pool),
            )
            .await?
            .unwrap_or_default()
        } else {
            let columns = self.describe_table(Some(&schema), table).await?;
            let mut ddl = format!(
                "CREATE TABLE {}.{} (\n",
                quote_ident(&schema),
                quote_ident(table)
            );
            for (idx, col) in columns.iter().enumerate() {
                let nullable = if col.nullable { "" } else { " NOT NULL" };
                ddl.push_str(&format!(
                    "  {} {}{}{}",
                    quote_ident(&col.name),
                    col.data_type,
                    nullable,
                    if idx + 1 == columns.len() {
                        "\n"
                    } else {
                        ",\n"
                    }
                ));
            }
            ddl.push(')');
            ddl
        };
        Ok(TableDDLInfo {
            schema,
            table: table.to_string(),
            object_type,
            ddl,
            notice: None,
        })
    }

    async fn execute_query(
        &self,
        sql: &str,
        max_rows: usize,
        limits: &Limits,
    ) -> Result<QueryResult> {
        let sql = strip_trailing_sql_terminator(sql);
        postgres_query::execute_query(&self.pool, self.config.read_only, sql, max_rows, limits)
            .await
    }

    async fn explain_query(&self, sql: &str, limits: &Limits) -> Result<ExplainResultData> {
        let sql = strip_trailing_sql_terminator(sql);
        postgres_query::explain_query(&self.pool, self.config.read_only, sql, limits).await
    }
}

fn quote_ident(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}
