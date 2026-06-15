use anyhow::{Context, Result};

use super::oracle::OracleConnection;
use super::oracle_query::{value_to_i64, value_to_string};
use super::*;

pub(super) async fn list_schemas(
    conn: &OracleConnection,
    filter: Option<&str>,
    limit: usize,
) -> Result<Vec<SchemaInfo>> {
    let filter = filter.unwrap_or("");
    let limit = conn.limits.normalize_list_limit(limit);
    let sql = format!(
        "select username from all_users where lower(username) like lower({}) order by username fetch first {} rows only",
        quote_literal(&format!("%{filter}%")),
        limit
    );
    Ok(conn
        .query(&sql)
        .await?
        .rows
        .iter()
        .filter_map(|row| row.get(0).and_then(value_to_string))
        .map(|name| SchemaInfo { name })
        .collect())
}

pub(super) async fn list_tables(
    conn: &OracleConnection,
    schema: Option<&str>,
    filter: Option<&str>,
    limit: usize,
) -> Result<Vec<TableInfo>> {
    let schema = schema.map(str::to_string).unwrap_or(conn.schema().await);
    let filter = filter.unwrap_or("");
    let limit = conn.limits.normalize_list_limit(limit);
    let sql = format!(
        "select owner, object_name, object_type from (
            select owner, table_name object_name, 'TABLE' object_type from all_tables
            union all
            select owner, view_name object_name, 'VIEW' object_type from all_views
        )
        where upper(owner) = upper({})
          and lower(object_name) like lower({})
        order by object_name
        fetch first {} rows only",
        quote_literal(&schema),
        quote_literal(&format!("%{filter}%")),
        limit
    );
    Ok(conn
        .query(&sql)
        .await?
        .rows
        .iter()
        .map(|row| TableInfo {
            schema: row
                .get(0)
                .and_then(value_to_string)
                .unwrap_or_else(|| schema.clone()),
            name: row.get(1).and_then(value_to_string).unwrap_or_default(),
            table_type: row.get(2).and_then(value_to_string).unwrap_or_default(),
        })
        .collect())
}

pub(super) async fn search_columns(
    conn: &OracleConnection,
    schema: Option<&str>,
    filter: Option<&str>,
    limit: usize,
) -> Result<Vec<ColumnMatchInfo>> {
    let schema = schema.map(str::to_string).unwrap_or(conn.schema().await);
    let filter = filter.unwrap_or("");
    let limit = conn.limits.normalize_list_limit(limit);
    let sql = format!(
        "select owner, table_name, column_name, data_type, column_id
         from all_tab_columns
         where upper(owner) = upper({})
           and (lower(column_name) like lower({}) or lower(table_name) like lower({}))
         order by table_name, column_id
         fetch first {} rows only",
        quote_literal(&schema),
        quote_literal(&format!("%{filter}%")),
        quote_literal(&format!("%{filter}%")),
        limit
    );
    Ok(conn
        .query(&sql)
        .await?
        .rows
        .iter()
        .map(|row| ColumnMatchInfo {
            schema: row
                .get(0)
                .and_then(value_to_string)
                .unwrap_or_else(|| schema.clone()),
            table: row.get(1).and_then(value_to_string).unwrap_or_default(),
            column: row.get(2).and_then(value_to_string).unwrap_or_default(),
            data_type: row.get(3).and_then(value_to_string).unwrap_or_default(),
            position: row.get(4).and_then(value_to_i64).unwrap_or_default(),
        })
        .collect())
}

pub(super) async fn list_constraints(
    conn: &OracleConnection,
    schema: Option<&str>,
    table: Option<&str>,
    limit: usize,
) -> Result<Vec<ConstraintInfo>> {
    let schema = schema.map(str::to_string).unwrap_or(conn.schema().await);
    let table = table.context("table is required")?;
    let limit = conn.limits.normalize_list_limit(limit);
    let sql = format!(
        "select c.constraint_name, c.owner, c.table_name, c.constraint_type,
            listagg(cc.column_name, ',') within group (order by cc.position) columns,
            c.r_owner, rc.table_name referenced_table,
            listagg(rcc.column_name, ',') within group (order by rcc.position) referenced_columns,
            c.search_condition_vc
         from all_constraints c
         left join all_cons_columns cc on cc.owner = c.owner and cc.constraint_name = c.constraint_name
         left join all_constraints rc on rc.owner = c.r_owner and rc.constraint_name = c.r_constraint_name
         left join all_cons_columns rcc on rcc.owner = rc.owner and rcc.constraint_name = rc.constraint_name and rcc.position = cc.position
         where upper(c.owner) = upper({}) and upper(c.table_name) = upper({})
         group by c.constraint_name, c.owner, c.table_name, c.constraint_type, c.r_owner, rc.table_name, c.search_condition_vc
         order by c.constraint_name
         fetch first {} rows only",
        quote_literal(&schema),
        quote_literal(table),
        limit
    );
    Ok(conn
        .query(&sql)
        .await?
        .rows
        .iter()
        .map(|row| {
            let kind = row.get(3).and_then(value_to_string).unwrap_or_default();
            let columns = row.get(4).and_then(value_to_string).unwrap_or_default();
            let referenced_columns = row.get(7).and_then(value_to_string).unwrap_or_default();
            ConstraintInfo {
                name: row.get(0).and_then(value_to_string),
                schema: row
                    .get(1)
                    .and_then(value_to_string)
                    .unwrap_or_else(|| schema.clone()),
                table: row
                    .get(2)
                    .and_then(value_to_string)
                    .unwrap_or_else(|| table.to_string()),
                constraint_type: constraint_type(&kind).to_string(),
                columns: split_csv(&columns),
                referenced_schema: row.get(5).and_then(value_to_string),
                referenced_table: row.get(6).and_then(value_to_string),
                referenced_columns: split_csv(&referenced_columns),
                check_expression: row.get(8).and_then(value_to_string),
                definition: None,
            }
        })
        .collect())
}

pub(super) async fn list_indexes(
    conn: &OracleConnection,
    schema: Option<&str>,
    table: Option<&str>,
    limit: usize,
) -> Result<Vec<IndexInfo>> {
    let schema = schema.map(str::to_string).unwrap_or(conn.schema().await);
    let table = table.context("table is required")?;
    let limit = conn.limits.normalize_list_limit(limit);
    let sql = format!(
        "select i.owner, i.table_name, i.index_name, i.index_type, i.uniqueness,
            case when c.constraint_type = 'P' then 'YES' else 'NO' end primary_flag,
            listagg(ic.column_name, ',') within group (order by ic.column_position) columns
         from all_indexes i
         left join all_ind_columns ic on ic.index_owner = i.owner and ic.index_name = i.index_name
         left join all_constraints c on c.owner = i.owner and c.index_name = i.index_name and c.constraint_type = 'P'
         where upper(i.owner) = upper({}) and upper(i.table_name) = upper({})
         group by i.owner, i.table_name, i.index_name, i.index_type, i.uniqueness, c.constraint_type
         order by i.index_name
         fetch first {} rows only",
        quote_literal(&schema),
        quote_literal(table),
        limit
    );
    Ok(conn
        .query(&sql)
        .await?
        .rows
        .iter()
        .map(|row| {
            let uniqueness = row.get(4).and_then(value_to_string).unwrap_or_default();
            let primary = row.get(5).and_then(value_to_string).unwrap_or_default();
            let columns = row.get(6).and_then(value_to_string).unwrap_or_default();
            IndexInfo {
                schema: row
                    .get(0)
                    .and_then(value_to_string)
                    .unwrap_or_else(|| schema.clone()),
                table: row
                    .get(1)
                    .and_then(value_to_string)
                    .unwrap_or_else(|| table.to_string()),
                name: row.get(2).and_then(value_to_string).unwrap_or_default(),
                index_type: row.get(3).and_then(value_to_string),
                unique: uniqueness == "UNIQUE",
                primary: primary == "YES",
                partial: None,
                columns: split_csv(&columns),
                definition: None,
            }
        })
        .collect())
}

pub(super) async fn get_table_comments(
    conn: &OracleConnection,
    schema: Option<&str>,
    table: &str,
) -> Result<TableCommentInfo> {
    let schema = schema.map(str::to_string).unwrap_or(conn.schema().await);
    let table_comment = conn
        .query_optional_string(&format!(
            "select comments from all_tab_comments where upper(owner) = upper({}) and upper(table_name) = upper({})",
            quote_literal(&schema),
            quote_literal(table)
        ))
        .await
        .ok()
        .flatten();
    let sql = format!(
        "select column_name, comments from all_col_comments
         where upper(owner) = upper({}) and upper(table_name) = upper({})
         order by column_name",
        quote_literal(&schema),
        quote_literal(table)
    );
    let columns = conn
        .query(&sql)
        .await?
        .rows
        .iter()
        .map(|row| ColumnCommentInfo {
            name: row.get(0).and_then(value_to_string).unwrap_or_default(),
            comment: row.get(1).and_then(value_to_string),
        })
        .collect();
    Ok(TableCommentInfo {
        schema,
        table: table.to_string(),
        table_comment,
        columns,
        notice: None,
    })
}

pub(super) async fn describe_table(
    conn: &OracleConnection,
    schema: Option<&str>,
    table: &str,
) -> Result<Vec<ColumnInfo>> {
    let schema = schema.map(str::to_string).unwrap_or(conn.schema().await);
    let sql = format!(
        "select column_name, data_type, nullable, column_id, char_length, data_precision, data_scale
         from all_tab_columns
         where upper(owner) = upper({}) and upper(table_name) = upper({})
         order by column_id",
        quote_literal(&schema),
        quote_literal(table)
    );
    Ok(conn
        .query(&sql)
        .await?
        .rows
        .iter()
        .map(|row| ColumnInfo {
            name: row.get(0).and_then(value_to_string).unwrap_or_default(),
            data_type: row.get(1).and_then(value_to_string).unwrap_or_default(),
            nullable: row
                .get(2)
                .and_then(value_to_string)
                .is_some_and(|v| v == "Y"),
            position: row.get(3).and_then(value_to_i64).unwrap_or_default(),
            length: row.get(4).and_then(value_to_i64),
            precision: row.get(5).and_then(value_to_i64),
            scale: row.get(6).and_then(value_to_i64),
        })
        .collect())
}

pub(super) async fn get_table_ddl(
    conn: &OracleConnection,
    schema: Option<&str>,
    table: &str,
) -> Result<TableDDLInfo> {
    let schema = schema.map(str::to_string).unwrap_or(conn.schema().await);
    let object_type = conn
        .query_optional_string(&format!(
            "select object_type from all_objects where upper(owner) = upper({}) and upper(object_name) = upper({}) and object_type in ('TABLE', 'VIEW') fetch first 1 row only",
            quote_literal(&schema),
            quote_literal(table)
        ))
        .await?
        .unwrap_or_else(|| "TABLE".to_string());
    let ddl = conn
        .query_optional_string(&format!(
            "select dbms_metadata.get_ddl({}, {}, {}) from dual",
            quote_literal(&object_type),
            quote_literal(table),
            quote_literal(&schema)
        ))
        .await;
    match ddl {
        Ok(Some(ddl)) => Ok(TableDDLInfo {
            schema,
            table: table.to_string(),
            object_type,
            ddl,
            notice: None,
        }),
        _ => {
            let columns = describe_table(conn, Some(&schema), table).await?;
            Ok(TableDDLInfo {
                schema: schema.clone(),
                table: table.to_string(),
                object_type,
                ddl: fallback_create_table(&schema, table, &columns),
                notice: Some(
                    "DBMS_METADATA.GET_DDL unavailable; returned normalized fallback DDL"
                        .to_string(),
                ),
            })
        }
    }
}

pub(super) fn quote_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn quote_ident(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn constraint_type(value: &str) -> &str {
    match value {
        "P" => "PRIMARY KEY",
        "U" => "UNIQUE",
        "R" => "FOREIGN KEY",
        "C" => "CHECK",
        other => other,
    }
}

fn split_csv(value: &str) -> Vec<String> {
    value
        .split(',')
        .filter_map(|part| {
            let value = part.trim();
            (!value.is_empty()).then(|| value.to_string())
        })
        .collect()
}

fn fallback_create_table(schema: &str, table: &str, columns: &[ColumnInfo]) -> String {
    let mut ddl = format!(
        "CREATE TABLE {}.{} (\n",
        quote_ident(schema),
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
}
