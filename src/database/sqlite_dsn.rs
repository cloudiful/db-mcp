pub fn sqlite_dsn_is_read_only(dsn: &str) -> bool {
    let query = url::Url::parse(dsn)
        .ok()
        .and_then(|url| url.query().map(str::to_owned))
        .or_else(|| dsn.split_once('?').map(|(_, query)| query.to_string()));

    query.is_some_and(|query| {
        url::form_urlencoded::parse(query.as_bytes()).any(|(key, value)| {
            (key.eq_ignore_ascii_case("mode") && value.eq_ignore_ascii_case("ro"))
                || (key.eq_ignore_ascii_case("immutable") && value == "1")
        })
    })
}

#[cfg(test)]
mod tests {
    use super::sqlite_dsn_is_read_only;

    #[test]
    fn recognizes_sqlite_read_only_uri_modes() {
        assert!(sqlite_dsn_is_read_only("file:./app.db?mode=ro"));
        assert!(sqlite_dsn_is_read_only("sqlite://./app.db?mode=ro"));
        assert!(sqlite_dsn_is_read_only("file:./app.db?immutable=1"));
    }

    #[test]
    fn rejects_writable_or_missing_sqlite_uri_modes() {
        assert!(!sqlite_dsn_is_read_only("file:./app.db?mode=rwc"));
        assert!(!sqlite_dsn_is_read_only("sqlite::memory:"));
        assert!(!sqlite_dsn_is_read_only("./app.db"));
    }
}
