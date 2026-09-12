use serde_json::Value;

use crate::config::OutputFormat;

use super::exit::CliError;

/// Encode a successful business object for stdout. JSON is the default and the
/// machine contract; text is a human-oriented projection of the same object.
pub fn render_success(format: OutputFormat, value: &Value) -> Result<String, CliError> {
    match format {
        OutputFormat::Json => {
            serde_json::to_string_pretty(value).map_err(|err| CliError::internal(err.to_string()))
        }
        OutputFormat::Text => Ok(render_text(value)),
    }
}

/// Stable error payload written to stderr: `{error, kind, command}`.
pub fn error_json(command: &str, err: &CliError) -> String {
    serde_json::json!({
        "error": err.message,
        "kind": err.kind.as_str(),
        "command": command,
    })
    .to_string()
}

/// Best-effort removal of credentials from an error message before it reaches
/// stderr. The driver error chain may otherwise echo a DSN or password.
pub fn redact_secrets(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(pos) = rest.find(|c: char| c.is_whitespace()) {
        let (token, tail) = rest.split_at(pos);
        out.push_str(&redact_token(token));
        let ws_end = tail
            .find(|c: char| !c.is_whitespace())
            .unwrap_or(tail.len());
        out.push_str(&tail[..ws_end]);
        rest = &tail[ws_end..];
    }
    out.push_str(&redact_token(rest));
    out
}

fn redact_token(token: &str) -> String {
    let redacted = redact_url_userinfo(token);
    let Some(eq) = redacted.find('=') else {
        return redacted;
    };
    let key = &redacted[..eq];
    let bare = key.rsplit(['?', '&', ';']).next().unwrap_or(key);
    if is_secret_key(bare) {
        format!("{key}=REDACTED")
    } else {
        redacted
    }
}

fn redact_url_userinfo(token: &str) -> String {
    let Some(scheme_end) = token.find("://") else {
        return token.to_string();
    };
    let after_scheme = &token[scheme_end + 3..];
    let authority_end = after_scheme
        .find(['/', '?', '#'])
        .unwrap_or(after_scheme.len());
    let authority = &after_scheme[..authority_end];
    let Some(at) = authority.rfind('@') else {
        return token.to_string();
    };
    let userinfo = &authority[..at];
    let Some(colon) = userinfo.find(':') else {
        return token.to_string();
    };
    format!(
        "{}{}:REDACTED@{}",
        &token[..scheme_end + 3],
        &userinfo[..colon],
        &after_scheme[at + 1..],
    )
}

fn is_secret_key(key: &str) -> bool {
    let key = key.trim_matches(['"', '\'']).to_ascii_lowercase();
    matches!(
        key.as_str(),
        "password" | "pwd" | "passwd" | "secret" | "token" | "apikey" | "api_key" | "access_token"
    )
}

fn render_text(value: &Value) -> String {
    let mut out = String::new();
    render_value(value, 0, &mut out);
    if out.ends_with('\n') {
        out.pop();
    }
    out
}

fn render_value(value: &Value, indent: usize, out: &mut String) {
    let pad = "  ".repeat(indent);
    match value {
        Value::Object(map) => {
            if let (Some(Value::String(kind)), Some(data)) = (map.get("kind"), map.get("data")) {
                out.push_str(&format!("{pad}kind: {kind}\n"));
                render_value(data, indent, out);
                return;
            }
            for (key, val) in map {
                match val {
                    Value::Object(nested) if !nested.is_empty() => {
                        out.push_str(&format!("{pad}{key}:\n"));
                        render_value(val, indent + 1, out);
                    }
                    Value::Array(items) if !items.is_empty() => {
                        out.push_str(&format!("{pad}{key}:\n"));
                        render_nested(val, indent + 1, out);
                    }
                    _ => out.push_str(&format!("{pad}{key}: {}\n", scalar(val))),
                }
            }
        }
        _ => render_nested(value, indent, out),
    }
}

fn render_nested(value: &Value, indent: usize, out: &mut String) {
    let pad = "  ".repeat(indent);
    match value {
        Value::Array(items) => {
            for item in items {
                match item {
                    Value::Object(_) | Value::Array(_) => {
                        out.push_str(&format!("{pad}-\n"));
                        render_value(item, indent + 1, out);
                    }
                    _ => out.push_str(&format!("{pad}- {}\n", scalar(item))),
                }
            }
        }
        _ => render_value(value, indent, out),
    }
}

fn scalar(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn json_success_round_trips_the_business_object() {
        let value = json!({"kind": "single", "data": {"connection": "sqlite"}});
        let rendered = render_success(OutputFormat::Json, &value).expect("render json");
        let parsed: Value = serde_json::from_str(&rendered).expect("parse rendered json");
        assert_eq!(parsed, value);
    }

    #[test]
    fn text_success_renders_kind_and_records() {
        let value = json!({
            "kind": "tables",
            "data": {
                "connection": "sqlite",
                "tables": [{"schema": "main", "name": "users", "tableType": "table"}],
            },
        });
        let rendered = render_success(OutputFormat::Text, &value).expect("render text");
        assert!(rendered.contains("kind: tables"), "{rendered}");
        assert!(rendered.contains("connection: sqlite"), "{rendered}");
        assert!(rendered.contains("name: users"), "{rendered}");
    }

    #[test]
    fn error_json_carries_error_kind_and_command() {
        let err = CliError::invalid_args("bad input");
        let rendered = error_json("query", &err);
        let parsed: Value = serde_json::from_str(&rendered).expect("parse error json");
        assert_eq!(parsed["error"], "bad input");
        assert_eq!(parsed["kind"], "invalid_args");
        assert_eq!(parsed["command"], "query");
    }

    #[test]
    fn redacts_key_value_credentials() {
        assert_eq!(redact_secrets("password=hunter2"), "password=REDACTED");
        assert_eq!(redact_secrets("token=abc123 next"), "token=REDACTED next");
    }

    #[test]
    fn redacts_url_userinfo_credentials() {
        assert_eq!(
            redact_secrets("postgres://alice:s3cret@db.example.com:5432/app"),
            "postgres://alice:REDACTED@db.example.com:5432/app"
        );
        assert_eq!(
            redact_secrets("connect postgres://alice:s3cret@host/db?password=x"),
            "connect postgres://alice:REDACTED@host/db?password=REDACTED"
        );
    }

    #[test]
    fn leaves_non_secret_text_unchanged() {
        assert_eq!(redact_secrets("select 1"), "select 1");
        assert_eq!(redact_secrets("user=alice"), "user=alice");
    }
}
