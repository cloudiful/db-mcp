use anyhow::{Context, Result};
use base64::Engine;
use oracle_rs::Value as OracleValue;
use serde_json::{Map, Value};
use std::time::Instant;

use super::oracle::OracleConnection;
use super::*;

pub(super) async fn execute_query(
    conn: &OracleConnection,
    sql: &str,
    max_rows: usize,
    limits: &Limits,
) -> Result<QueryResult> {
    let normalized = crate::readonly_sql::normalize_read_only_query(sql)?;
    let applied_max_rows = limits.normalize_max_rows(max_rows);
    let limited = limited_sql(&normalized, applied_max_rows);
    let started = Instant::now();
    let result = conn.query(&limited).await?;
    result_to_query_result(result, applied_max_rows, limits, started)
}

pub(super) async fn explain_query(
    db: &OracleConnection,
    sql: &str,
    limits: &Limits,
) -> Result<ExplainResultData> {
    let normalized = crate::readonly_sql::normalize_read_only_query(sql)?;
    let explain_sql = format!("explain plan for {normalized}");
    let conn = db.connect().await?;
    let binds: Vec<OracleValue> = vec![];
    tokio::time::timeout(db.limits.query_timeout, conn.query(&explain_sql, &binds))
        .await
        .context("oracle explain timeout")?
        .context("oracle explain failed")?;
    let plan = tokio::time::timeout(
        db.limits.query_timeout,
        conn.query("select plan_table_output from table(dbms_xplan.display())", &binds),
    )
    .await
    .context("oracle dbms_xplan timeout")?
    .context("oracle dbms_xplan failed")?;
    let _ = conn.close().await;
    Ok(ExplainResultData {
        format: "text".to_string(),
        plan_lines: plan
            .rows
            .iter()
            .take(limits.explain_max_lines)
            .filter_map(|row| row.get(0).and_then(value_to_string))
            .collect(),
        notice: None,
    })
}

pub(super) fn value_to_string(value: &OracleValue) -> Option<String> {
    match value {
        OracleValue::Null => None,
        OracleValue::String(v) => Some(v.clone()),
        OracleValue::Integer(v) => Some(v.to_string()),
        OracleValue::Float(v) => Some(v.to_string()),
        OracleValue::Number(v) => Some(v.as_str().to_string()),
        OracleValue::Date(v) => Some(format!("{v:?}")),
        OracleValue::Timestamp(v) => Some(format!("{v:?}")),
        OracleValue::RowId(v) => Some(format!("{v:?}")),
        OracleValue::Boolean(v) => Some(v.to_string()),
        OracleValue::Bytes(v) => Some(base64::engine::general_purpose::STANDARD.encode(v)),
        OracleValue::Json(v) => Some(v.to_string()),
        other => Some(other.to_string()),
    }
}

pub(super) fn value_to_i64(value: &OracleValue) -> Option<i64> {
    match value {
        OracleValue::Integer(v) => Some(*v),
        OracleValue::Float(v) => Some(*v as i64),
        OracleValue::Number(v) => v.to_i64().ok(),
        OracleValue::String(v) => v.parse().ok(),
        _ => None,
    }
}

fn limited_sql(sql: &str, max_rows: usize) -> String {
    let lower = sql.to_lowercase();
    if lower.contains(" fetch first ") || lower.contains(" rownum ") {
        sql.to_string()
    } else {
        format!("select * from ({sql}) where rownum <= {max_rows}")
    }
}

fn value_to_json(value: &OracleValue) -> Value {
    match value {
        OracleValue::Null => Value::Null,
        OracleValue::String(v) => Value::String(v.clone()),
        OracleValue::Bytes(v) => {
            Value::String(base64::engine::general_purpose::STANDARD.encode(v))
        }
        OracleValue::Integer(v) => Value::from(*v),
        OracleValue::Float(v) => Value::from(*v),
        OracleValue::Number(v) => v
            .to_i64()
            .map(Value::from)
            .or_else(|_| v.to_f64().map(Value::from))
            .unwrap_or_else(|_| Value::String(v.as_str().to_string())),
        OracleValue::Boolean(v) => Value::Bool(*v),
        OracleValue::Json(v) => v.clone(),
        other => Value::String(other.to_string()),
    }
}

fn result_to_query_result(
    result: oracle_rs::QueryResult,
    applied_max_rows: usize,
    limits: &Limits,
    started: Instant,
) -> Result<QueryResult> {
    let columns = result
        .columns
        .iter()
        .take(limits.max_columns)
        .map(|col| QueryColumn {
            name: col.name.clone(),
            data_type: format!("{:?}", col.oracle_type),
        })
        .collect::<Vec<_>>();
    let mut rows = vec![];
    let mut truncated = result.has_more_rows || result.rows.len() > applied_max_rows;
    let mut reasons = if truncated {
        vec!["row limit reached".to_string()]
    } else {
        vec![]
    };
    let mut total_chars = 0usize;
    for row in result.rows.iter().take(applied_max_rows) {
        let mut map = Map::new();
        for (idx, col) in result.columns.iter().enumerate().take(limits.max_columns) {
            let raw = row.get(idx).map(value_to_json).unwrap_or(Value::Null);
            let (value, was_truncated) = serialize_value(&raw, limits.max_cell_chars);
            if was_truncated {
                truncated = true;
                reasons.push(format!("cell {} truncated", col.name));
            }
            total_chars += serde_json::to_string(&value)?.chars().count();
            if total_chars > limits.max_total_chars {
                truncated = true;
                reasons.push("total character limit reached".to_string());
                break;
            }
            map.insert(col.name.clone(), value);
        }
        rows.push(map);
        if total_chars > limits.max_total_chars {
            break;
        }
    }
    Ok(QueryResult {
        columns,
        row_count: rows.len(),
        rows,
        applied_max_rows,
        truncated,
        truncation_reasons: reasons,
        notice: if truncated {
            Some("result truncated to protect MCP context".to_string())
        } else {
            None
        },
        execution_time: format!("{:?}", started.elapsed()),
    })
}
