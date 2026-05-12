use anyhow::{Context, Result};
use sqlx::{AnyPool, Row};
use std::sync::{Arc, RwLock};

use super::*;

#[derive(Clone)]
pub struct SQLiteConnection {
    name: String,
    dsn: String,
    limits: Limits,
    pool: AnyPool,
    default_schema: Arc<RwLock<String>>,
}

impl SQLiteConnection {
    pub fn new(name: String, config: config::SQLiteConfig, limits: Limits) -> Result<Self> {
        let pool = AnyPool::connect_lazy(&config.dsn)?;
        Ok(Self {
            name,
            dsn: config.dsn,
            limits,
            pool,
            default_schema: Arc::new(RwLock::new("main".to_string())),
        })
    }

    async fn schema(&self) -> String {
        self.default_schema
            .read()
            .expect("default_schema poisoned")
            .clone()
    }
}

#[async_trait::async_trait]
impl DatabaseConnection for SQLiteConnection {
    fn info(&self) -> ConnectionInfo {
        ConnectionInfo {
            name: self.name.clone(),
            driver: "sqlite".to_string(),
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
        self.dsn.clone()
    }

    async fn ping(&self) -> Result<()> {
        tokio::time::timeout(
            self.limits.query_timeout,
            sqlx::query_scalar::<_, i64>("select 1").fetch_one(&self.pool),
        )
        .await
        .with_context(|| format!("sqlite ping timeout name={} dsn={}", self.name, self.dsn))?
        .with_context(|| format!("sqlite ping failed name={} dsn={}", self.name, self.dsn))?;
        Ok(())
    }

    async fn diagnose(&self, deep: bool) -> ConnectionDiagnostic {
        let target = self.target_summary();
        let mut checks = vec![];
        checks.push(
            timed_probe("config", target.clone(), || async {
                if self.dsn.trim().is_empty() {
                    anyhow::bail!("sqlite dsn is empty");
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
                    .with_context(|| format!("sqlite acquire timeout name={} dsn={}", self.name, self.dsn))?
                    .with_context(|| format!("sqlite acquire failed name={} dsn={}", self.name, self.dsn))?;
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
                    sqlx::query_scalar::<_, i64>("select 1").fetch_one(&self.pool),
                )
                .await
                .context("sqlite ping query timeout")?
                .context("sqlite ping query failed")?;
                Ok(())
            })
            .await,
        );
        if checks.last().is_some_and(|check| check.status != "up") || !deep {
            return diagnostic_from_checks(self.info(), checks);
        }

        checks.push(
            timed_probe("metadataSmokeTest", target, || async {
                sqlx::query("pragma database_list")
                    .fetch_all(&self.pool)
                    .await
                    .context("sqlite metadata smoke test")?;
                Ok(())
            })
            .await,
        );
        diagnostic_from_checks(self.info(), checks)
    }

    async fn list_schemas(&self, filter: Option<&str>, limit: usize) -> Result<Vec<SchemaInfo>> {
        let filter = filter.unwrap_or("");
        let limit = self.limits.normalize_list_limit(limit);
        let rows = sqlx::query("pragma database_list")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows
            .into_iter()
            .filter_map(|row| {
                let name: String = row.try_get("name").ok()?;
                if !name.to_lowercase().contains(&filter.to_lowercase()) {
                    return None;
                }
                Some(SchemaInfo { name })
            })
            .take(limit)
            .collect())
    }

    async fn list_tables(
        &self,
        schema: Option<&str>,
        filter: Option<&str>,
        limit: usize,
    ) -> Result<Vec<TableInfo>> {
        let schema = schema.map(str::to_string).unwrap_or(self.schema().await);
        let filter = filter.unwrap_or("").to_lowercase();
        let sql = format!(
            "select name, type from {}.sqlite_master where type in ('table','view') order by name",
            quote_ident(&schema)
        );
        let rows = sqlx::query(&sql).fetch_all(&self.pool).await?;
        Ok(rows
            .into_iter()
            .filter_map(|row| {
                let name: String = row.try_get("name").ok()?;
                if !name.to_lowercase().contains(&filter) {
                    return None;
                }
                let table_type: String = row.try_get("type").unwrap_or_default();
                Some(TableInfo {
                    schema: schema.clone(),
                    name,
                    table_type,
                })
            })
            .take(self.limits.normalize_list_limit(limit))
            .collect())
    }

    async fn search_columns(
        &self,
        schema: Option<&str>,
        filter: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ColumnMatchInfo>> {
        let schema = schema.map(str::to_string).unwrap_or(self.schema().await);
        let filter = filter.unwrap_or("").to_lowercase();
        let tables = self.list_tables(Some(&schema), None, usize::MAX).await?;
        let mut out = vec![];
        for table in tables {
            let pragma = format!(
                "pragma {}.table_info({})",
                quote_ident(&schema),
                quote_string(&table.name)
            );
            let rows = sqlx::query(&pragma).fetch_all(&self.pool).await?;
            for row in rows {
                let column: String = row.try_get("name").unwrap_or_default();
                if !column.to_lowercase().contains(&filter)
                    && !table.name.to_lowercase().contains(&filter)
                {
                    continue;
                }
                out.push(ColumnMatchInfo {
                    schema: schema.clone(),
                    table: table.name.clone(),
                    column,
                    data_type: row.try_get("type").unwrap_or_default(),
                    position: row.try_get::<i64, _>("cid").unwrap_or_default(),
                });
                if out.len() >= self.limits.normalize_list_limit(limit) {
                    return Ok(out);
                }
            }
        }
        Ok(out)
    }

    async fn list_constraints(
        &self,
        schema: Option<&str>,
        table: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ConstraintInfo>> {
        let schema = schema.map(str::to_string).unwrap_or(self.schema().await);
        let table = table.context("table is required")?;
        let pragma = format!(
            "pragma {}.foreign_key_list({})",
            quote_ident(&schema),
            quote_string(table)
        );
        let rows = sqlx::query(&pragma).fetch_all(&self.pool).await?;
        let mut out = vec![];
        for row in rows {
            out.push(ConstraintInfo {
                name: None,
                schema: schema.clone(),
                table: table.to_string(),
                constraint_type: "FOREIGN KEY".to_string(),
                columns: vec![row.try_get::<String, _>("from").unwrap_or_default()],
                referenced_schema: Some(schema.clone()),
                referenced_table: Some(row.try_get::<String, _>("table").unwrap_or_default()),
                referenced_columns: vec![row.try_get::<String, _>("to").unwrap_or_default()],
                check_expression: None,
                definition: None,
            });
            if out.len() >= self.limits.normalize_list_limit(limit) {
                break;
            }
        }
        Ok(out)
    }

    async fn list_indexes(
        &self,
        schema: Option<&str>,
        table: Option<&str>,
        limit: usize,
    ) -> Result<Vec<IndexInfo>> {
        let schema = schema.map(str::to_string).unwrap_or(self.schema().await);
        let table = table.context("table is required")?;
        let pragma = format!(
            "pragma {}.index_list({})",
            quote_ident(&schema),
            quote_string(table)
        );
        let rows = sqlx::query(&pragma).fetch_all(&self.pool).await?;
        let mut out = vec![];
        for row in rows {
            let name: String = row.try_get("name").unwrap_or_default();
            let cols = sqlx::query(&format!(
                "pragma {}.index_info({})",
                quote_ident(&schema),
                quote_string(&name)
            ))
            .fetch_all(&self.pool)
            .await?;
            let columns = cols
                .into_iter()
                .map(|r| r.try_get::<String, _>("name").unwrap_or_default())
                .collect();
            out.push(IndexInfo {
                schema: schema.clone(),
                table: table.to_string(),
                name,
                index_type: None,
                unique: row.try_get::<i64, _>("unique").unwrap_or_default() == 1,
                primary: row.try_get::<String, _>("origin").unwrap_or_default() == "pk",
                partial: row.try_get::<i64, _>("partial").ok().map(|v| v == 1),
                columns,
                definition: None,
            });
            if out.len() >= self.limits.normalize_list_limit(limit) {
                break;
            }
        }
        Ok(out)
    }

    async fn get_table_comments(
        &self,
        schema: Option<&str>,
        table: &str,
    ) -> Result<TableCommentInfo> {
        Ok(TableCommentInfo {
            schema: schema.map(str::to_string).unwrap_or(self.schema().await),
            table: table.to_string(),
            table_comment: None,
            columns: vec![],
            notice: Some("SQLite does not store comments".to_string()),
        })
    }

    async fn describe_table(&self, schema: Option<&str>, table: &str) -> Result<Vec<ColumnInfo>> {
        let schema = schema.map(str::to_string).unwrap_or(self.schema().await);
        let pragma = format!(
            "pragma {}.table_info({})",
            quote_ident(&schema),
            quote_string(table)
        );
        let rows = sqlx::query(&pragma).fetch_all(&self.pool).await?;
        Ok(rows
            .into_iter()
            .map(|row| ColumnInfo {
                name: row.try_get("name").unwrap_or_default(),
                data_type: row.try_get("type").unwrap_or_default(),
                nullable: row.try_get::<i64, _>("notnull").unwrap_or_default() == 0,
                position: row.try_get::<i64, _>("cid").unwrap_or_default(),
                length: None,
                precision: None,
                scale: None,
            })
            .collect())
    }

    async fn get_table_ddl(&self, schema: Option<&str>, table: &str) -> Result<TableDDLInfo> {
        let schema = schema.map(str::to_string).unwrap_or(self.schema().await);
        let sql = format!(
            "select sql from {}.sqlite_master where name = ?",
            quote_ident(&schema)
        );
        let ddl: Option<String> = sqlx::query_scalar(&sql)
            .bind(table)
            .fetch_optional(&self.pool)
            .await?;
        Ok(TableDDLInfo {
            schema,
            table: table.to_string(),
            object_type: "TABLE".to_string(),
            ddl: ddl.unwrap_or_default(),
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
        let normalized = crate::readonly_sql::normalize_read_only_query(sql)?;
        let rows = sqlx::query(&format!("EXPLAIN QUERY PLAN {normalized}"))
            .fetch_all(&self.pool)
            .await?;
        let mut lines = vec![];
        for row in rows.into_iter().take(limits.explain_max_lines) {
            let detail: String = row
                .try_get("detail")
                .unwrap_or_else(|_| row.try_get(3).unwrap_or_default());
            lines.push(detail);
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

fn quote_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}
