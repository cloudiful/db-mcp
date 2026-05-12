use anyhow::{Context, Result};
use sqlx::{AnyPool, Row};
use std::sync::{Arc, RwLock};

use super::*;

#[derive(Clone)]
pub struct PostgresConnection {
    name: String,
    config: config::PostgresConfig,
    limits: Limits,
    pool: AnyPool,
    default_schema: Arc<RwLock<String>>,
}

impl PostgresConnection {
    pub fn new(name: String, config: config::PostgresConfig, limits: Limits) -> Result<Self> {
        let pool = AnyPool::connect_lazy(&config.dsn)?;
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
        let schema: Option<String> = sqlx::query_scalar("select current_schema()")
            .fetch_optional(&self.pool)
            .await
            .context("current_schema")?;
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
        }
    }

    async fn ping(&self) -> Result<()> {
        tokio::time::timeout(self.limits.query_timeout, self.pool.acquire())
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

    async fn list_schemas(&self, filter: Option<&str>, limit: usize) -> Result<Vec<SchemaInfo>> {
        let limit = self.limits.normalize_list_limit(limit);
        let filter = filter.unwrap_or("");
        let rows = sqlx::query(
            r#"
            select schema_name::text as schema_name
            from information_schema.schemata
            where schema_name not like 'pg_%'
              and schema_name <> 'information_schema'
              and lower(schema_name) like lower($1)
            order by schema_name
            limit $2
            "#,
        )
        .bind(format!("%{filter}%"))
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| SchemaInfo {
                name: row.try_get::<String, _>("schema_name").unwrap_or_default(),
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
        let rows = sqlx::query(
            r#"
            select table_schema::text as table_schema,
                   table_name::text as table_name,
                   table_type::text as table_type
            from information_schema.tables
            where table_schema = $1
              and lower(table_name) like lower($2)
              and table_type in ('BASE TABLE', 'VIEW')
            order by table_name
            limit $3
            "#,
        )
        .bind(&schema)
        .bind(format!("%{filter}%"))
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| TableInfo {
                schema: row
                    .try_get("table_schema")
                    .unwrap_or_else(|_| schema.clone()),
                name: row.try_get("table_name").unwrap_or_default(),
                table_type: row.try_get("table_type").unwrap_or_default(),
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
        let rows = sqlx::query(
            r#"
            select table_schema::text as table_schema,
                   table_name::text as table_name,
                   column_name::text as column_name,
                   data_type::text as data_type,
                   ordinal_position
            from information_schema.columns
            where table_schema = $1
              and lower(column_name) like lower($2)
            order by table_name, ordinal_position
            limit $3
            "#,
        )
        .bind(&schema)
        .bind(format!("%{filter}%"))
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| ColumnMatchInfo {
                schema: row
                    .try_get("table_schema")
                    .unwrap_or_else(|_| schema.clone()),
                table: row.try_get("table_name").unwrap_or_default(),
                column: row.try_get("column_name").unwrap_or_default(),
                data_type: row.try_get("data_type").unwrap_or_default(),
                position: row
                    .try_get::<i64, _>("ordinal_position")
                    .unwrap_or_default(),
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
        let rows = sqlx::query(
            r#"
            select
                c.conname::text as name,
                n.nspname::text as schema,
                rel.relname::text as table_name,
                case c.contype
                    when 'p' then 'PRIMARY KEY'
                    when 'u' then 'UNIQUE'
                    when 'f' then 'FOREIGN KEY'
                    when 'c' then 'CHECK'
                    else c.contype::text
                end as constraint_type,
                pg_get_constraintdef(c.oid, true)::text as definition,
                case when c.confrelid <> 0 then nconf.nspname::text end as referenced_schema,
                case when c.confrelid <> 0 then relconf.relname::text end as referenced_table
            from pg_constraint c
            join pg_class rel on rel.oid = c.conrelid
            join pg_namespace n on n.oid = rel.relnamespace
            left join pg_class relconf on relconf.oid = c.confrelid
            left join pg_namespace nconf on nconf.oid = relconf.relnamespace
            where n.nspname = $1 and rel.relname = $2
            order by c.conname
            limit $3
            "#,
        )
        .bind(&schema)
        .bind(table)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| ConstraintInfo {
                name: row.try_get("name").ok(),
                schema: row.try_get("schema").unwrap_or_else(|_| schema.clone()),
                table: row.try_get("table_name").unwrap_or_default(),
                constraint_type: row.try_get("constraint_type").unwrap_or_default(),
                columns: vec![],
                referenced_schema: row.try_get("referenced_schema").ok(),
                referenced_table: row.try_get("referenced_table").ok(),
                referenced_columns: vec![],
                check_expression: None,
                definition: row.try_get("definition").ok(),
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
        let rows = sqlx::query(
            r#"
            select schemaname::text as schemaname,
                   tablename::text as tablename,
                   indexname::text as indexname,
                   indexdef::text as indexdef
            from pg_indexes
            where schemaname = $1 and tablename = $2
            order by indexname
            limit $3
            "#,
        )
        .bind(&schema)
        .bind(table)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| {
                let definition: String = row.try_get("indexdef").unwrap_or_default();
                let columns = definition
                    .split('(')
                    .nth(1)
                    .and_then(|s| s.split(')').next())
                    .map(|s| s.split(',').map(|v| v.trim().to_string()).collect())
                    .unwrap_or_else(Vec::new);
                IndexInfo {
                    schema: row.try_get("schemaname").unwrap_or_else(|_| schema.clone()),
                    table: row.try_get("tablename").unwrap_or_default(),
                    name: row.try_get("indexname").unwrap_or_default(),
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
        let table_comment: Option<String> = sqlx::query_scalar(
            r#"
            select obj_description((quote_ident($1) || '.' || quote_ident($2))::regclass::oid, 'pg_class')
            "#,
        )
        .bind(&schema)
        .bind(table)
        .fetch_optional(&self.pool)
        .await
        .ok()
        .flatten();
        let rows = sqlx::query(
            r#"
            select a.attname::text as name, d.description::text as comment
            from pg_attribute a
            join pg_class c on c.oid = a.attrelid
            join pg_namespace n on n.oid = c.relnamespace
            left join pg_description d on d.objoid = a.attrelid and d.objsubid = a.attnum
            where n.nspname = $1 and c.relname = $2 and a.attnum > 0 and not a.attisdropped
            order by a.attnum
            "#,
        )
        .bind(&schema)
        .bind(table)
        .fetch_all(&self.pool)
        .await?;
        Ok(TableCommentInfo {
            schema,
            table: table.to_string(),
            table_comment,
            columns: rows
                .into_iter()
                .map(|row| ColumnCommentInfo {
                    name: row.try_get("name").unwrap_or_default(),
                    comment: row.try_get("comment").ok(),
                })
                .collect(),
            notice: None,
        })
    }

    async fn describe_table(&self, schema: Option<&str>, table: &str) -> Result<Vec<ColumnInfo>> {
        let schema = schema.map(str::to_string).unwrap_or(self.schema().await?);
        let rows = sqlx::query(
            r#"
            select column_name::text as column_name,
                   data_type::text as data_type,
                   is_nullable::text as is_nullable,
                   ordinal_position,
                   character_maximum_length, numeric_precision, numeric_scale
            from information_schema.columns
            where table_schema = $1 and table_name = $2
            order by ordinal_position
            "#,
        )
        .bind(&schema)
        .bind(table)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| ColumnInfo {
                name: row.try_get("column_name").unwrap_or_default(),
                data_type: row.try_get("data_type").unwrap_or_default(),
                nullable: row
                    .try_get::<String, _>("is_nullable")
                    .map(|s| s == "YES")
                    .unwrap_or(false),
                position: row.try_get("ordinal_position").unwrap_or_default(),
                length: row.try_get("character_maximum_length").ok(),
                precision: row.try_get("numeric_precision").ok(),
                scale: row.try_get("numeric_scale").ok(),
            })
            .collect())
    }

    async fn get_table_ddl(&self, schema: Option<&str>, table: &str) -> Result<TableDDLInfo> {
        let schema = schema.map(str::to_string).unwrap_or(self.schema().await?);
        let object_type: Option<String> = sqlx::query_scalar(
            r#"
            select table_type
            from information_schema.tables
            where table_schema = $1 and table_name = $2
            "#,
        )
        .bind(&schema)
        .bind(table)
        .fetch_optional(&self.pool)
        .await?;
        let object_type = object_type.unwrap_or_else(|| "TABLE".to_string());
        let ddl = if object_type == "VIEW" {
            sqlx::query_scalar(
                r#"
                select pg_get_viewdef(format('%I.%I', $1, $2)::regclass, true)
                "#,
            )
            .bind(&schema)
            .bind(table)
            .fetch_one(&self.pool)
            .await?
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
        collect_rows(&self.pool, sql, max_rows, limits).await
    }

    async fn explain_query(&self, sql: &str, limits: &Limits) -> Result<ExplainResultData> {
        let normalized = readonly_sql::normalize_read_only_query(sql)?;
        let rows = sqlx::query(&format!("EXPLAIN {normalized}"))
            .fetch_all(&self.pool)
            .await?;
        let mut lines = vec![];
        for row in rows.into_iter().take(limits.explain_max_lines) {
            let line: String = row.try_get::<String, _>(0).unwrap_or_default();
            lines.push(line);
        }
        Ok(ExplainResultData {
            format: "text".to_string(),
            plan_lines: lines,
            notice: None,
        })
    }
}

fn quote_ident(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn redact_postgres_dsn(raw: &str) -> String {
    if let Ok(mut url) = url::Url::parse(raw) {
        if url.password().is_some() {
            let _ = url.set_password(Some("REDACTED"));
        }
        return url.to_string();
    }
    raw.split_whitespace()
        .map(|part| {
            if part.to_lowercase().starts_with("password=") {
                "password=REDACTED".to_string()
            } else {
                part.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}
