use anyhow::{Result, bail};

pub fn normalize_read_only_query(sql: &str) -> Result<String> {
    let trimmed = sql.trim();
    if trimmed.is_empty() {
        bail!("query is empty");
    }

    let normalized = trimmed.trim_end_matches(';').trim_end();
    if normalized.is_empty() {
        bail!("query is empty");
    }
    if normalized.contains(';') {
        bail!("multiple statements are not allowed");
    }
    let upper = normalized.to_uppercase();
    if upper.contains(" FOR UPDATE") {
        bail!("FOR UPDATE is not allowed");
    }
    if upper.starts_with("SELECT ") || upper == "SELECT" || upper.starts_with("WITH ") {
        Ok(normalized.to_string())
    } else {
        bail!("only SELECT and WITH queries are allowed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_write_sql() {
        assert!(normalize_read_only_query("UPDATE t SET x = 1").is_err());
    }

    #[test]
    fn allows_single_trailing_semicolon() {
        assert_eq!(
            normalize_read_only_query("select 1;").unwrap(),
            "select 1".to_string()
        );
        assert_eq!(
            normalize_read_only_query("select 1;;   ").unwrap(),
            "select 1".to_string()
        );
    }

    #[test]
    fn rejects_multiple_statements_with_inner_semicolon() {
        assert!(normalize_read_only_query("select 1; select 2").is_err());
    }
}
