use serde_json::{Map, Value};

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

/// Stable error payload written to stderr. Keeps the historical `error`, `kind`,
/// and `command` fields and adds the shared machine-readable `code` plus the
/// optional timeout/cancellation detail when the error chain carries it.
pub fn error_json(command: &str, err: &CliError) -> String {
    let mut payload = Map::new();
    payload.insert("error".to_string(), Value::String(err.message.clone()));
    payload.insert(
        "kind".to_string(),
        Value::String(err.kind.as_str().to_string()),
    );
    payload.insert(
        "code".to_string(),
        Value::String(err.code.as_str().to_string()),
    );
    payload.insert("command".to_string(), Value::String(command.to_string()));
    if let Some(operation) = &err.operation {
        payload.insert("operation".to_string(), Value::String(operation.clone()));
    }
    if let Some(timeout) = err.timeout {
        payload.insert("timeout".to_string(), Value::Bool(timeout));
    }
    if let Some(cancellation) = &err.cancellation {
        payload.insert(
            "cancellation".to_string(),
            Value::String(cancellation.clone()),
        );
    }
    if let Some(result_unknown) = err.result_unknown {
        payload.insert("resultUnknown".to_string(), Value::Bool(result_unknown));
    }
    Value::Object(payload).to_string()
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
    fn error_json_carries_error_kind_code_and_command() {
        let err = CliError::invalid_args("bad input");
        let rendered = error_json("query", &err);
        let parsed: Value = serde_json::from_str(&rendered).expect("parse error json");
        assert_eq!(parsed["error"], "bad input");
        assert_eq!(parsed["kind"], "invalid_args");
        assert_eq!(parsed["code"], "invalid_params");
        assert_eq!(parsed["command"], "query");
        assert!(parsed.get("operation").is_none());
        assert!(parsed.get("timeout").is_none());
    }
}
