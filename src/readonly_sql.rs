use anyhow::{Result, bail};

pub fn normalize_read_only_query(sql: &str) -> Result<String> {
    let trimmed = sql.trim();
    if trimmed.is_empty() {
        bail!("query is empty");
    }
    if trimmed.contains(';') {
        bail!("multiple statements are not allowed");
    }
    let upper = trimmed.to_uppercase();
    if upper.contains(" FOR UPDATE") {
        bail!("FOR UPDATE is not allowed");
    }
    if upper.starts_with("SELECT ") || upper == "SELECT" || upper.starts_with("WITH ") {
        Ok(trimmed.to_string())
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
}
