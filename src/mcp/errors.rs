//! Stable, machine-readable error contract shared by the MCP tools and the CLI.
//!
//! Public responses carry a curated [`ErrorPayload`] instead of the raw
//! `anyhow` chain: a stable `code`/`kind`, a redacted caller-facing `message`,
//! and, when the error chain recorded it, the timed-out `operation`, the
//! `cancellation` state, and whether the statement `resultUnknown`. The full
//! cause chain is emitted only through [`log_failure`].

use anyhow::Error;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::database::{
    Cancellation, Operation, cancellation_report, redact_secrets, timeout_operation,
};

/// Upper bound for a public error message so a driver cannot flood an MCP or
/// CLI response with internal detail.
const MAX_MESSAGE_CHARS: usize = 512;

/// Stable machine-readable error codes. `code` is the precise contract a
/// client should branch on; `kind` groups it into the coarse category that maps
/// to the CLI exit-code contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    InvalidParams,
    MissingConnection,
    OperationTimeout,
    BackendError,
    InternalError,
}

impl ErrorCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidParams => "invalid_params",
            Self::MissingConnection => "missing_connection",
            Self::OperationTimeout => "operation_timeout",
            Self::BackendError => "backend_error",
            Self::InternalError => "internal_error",
        }
    }

    /// Coarse category shared with the CLI error kinds and exit codes.
    pub const fn kind(self) -> &'static str {
        match self {
            Self::InvalidParams => "invalid_args",
            Self::MissingConnection => "missing_connection",
            Self::OperationTimeout => "timeout",
            Self::BackendError => "backend",
            Self::InternalError => "internal",
        }
    }
}

/// The public error document. Optional detail fields are omitted entirely when
/// the error chain did not carry them, so clients can distinguish "absent" from
/// a default value.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ErrorPayload {
    pub code: String,
    pub kind: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancellation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_unknown: Option<bool>,
}

/// The typed classification plus its serializable payload.
#[derive(Debug, Clone)]
pub struct ClassifiedError {
    pub code: ErrorCode,
    pub payload: ErrorPayload,
}

/// Classify an error chain without string-matching timeouts. Timeout and
/// cancellation state come from the typed `OperationTimeout` /
/// `CancellationReport` markers attached by the database layer.
pub fn classify(error: &Error, connection: Option<&str>) -> ClassifiedError {
    let operation_timeout = timeout_operation(error);
    let report = cancellation_report(error);
    let is_timeout = operation_timeout.is_some() || report.is_some();
    let operation = operation_timeout.or_else(|| report.map(|report| report.operation()));
    let cancellation = report.map(|report| report.cancellation());
    let result_unknown = report.map(|report| report.result_unknown());

    let code = if is_timeout {
        ErrorCode::OperationTimeout
    } else if is_missing_connection(error) {
        ErrorCode::MissingConnection
    } else {
        ErrorCode::BackendError
    };

    let message = if is_timeout {
        timeout_message(operation, cancellation, result_unknown)
    } else {
        safe_message(error)
    };

    ClassifiedError {
        code,
        payload: ErrorPayload {
            code: code.as_str().to_string(),
            kind: code.kind().to_string(),
            message,
            operation: operation.map(|operation| operation.as_str().to_string()),
            connection: connection.map(str::to_owned),
            timeout: is_timeout.then_some(true),
            cancellation: cancellation.map(|cancellation| cancellation.as_str().to_string()),
            result_unknown,
        },
    }
}

/// Build the JSON document for an MCP tool-level error and write the full,
/// redacted cause chain to the controlled log.
pub fn error_json_value(error: &Error, connection: Option<&str>) -> Value {
    log_failure(error, connection);
    let payload = classify(error, connection).payload;
    serde_json::to_value(payload).unwrap_or_else(|_| {
        serde_json::json!({
            "code": ErrorCode::InternalError.as_str(),
            "kind": ErrorCode::InternalError.kind(),
            "message": "failed to serialize the error payload",
        })
    })
}

/// Emit the full cause chain at error level. This is the only place the raw
/// chain is surfaced; credentials are still redacted.
pub fn log_failure(error: &Error, connection: Option<&str>) {
    tracing::error!(
        connection = connection.unwrap_or_default(),
        error_chain = %redact_secrets(&format!("{error:#}")),
        "database operation failed"
    );
}

fn is_missing_connection(error: &Error) -> bool {
    let text = error.to_string().to_lowercase();
    text.contains("not found") || text.contains("connection is required")
}

/// A deadline timeout message built only from typed state, never from the
/// driver chain.
fn timeout_message(
    operation: Option<Operation>,
    cancellation: Option<Cancellation>,
    result_unknown: Option<bool>,
) -> String {
    let mut message = match operation {
        Some(operation) => format!("{} operation timeout", operation.as_str()),
        None => "operation timeout".to_string(),
    };
    match cancellation {
        Some(Cancellation::Confirmed) => {
            message.push_str("; the database confirmed the statement was cancelled")
        }
        Some(Cancellation::Unconfirmed) => message
            .push_str("; cancellation could not be confirmed and the connection was discarded"),
        None => {}
    }
    if result_unknown == Some(true) {
        message.push_str("; the result may be unknown, do not blindly retry");
    }
    message
}

/// Caller-facing message for non-timeout errors: the root cause (not the full
/// chain), redacted and length-bounded. Falls back to the outermost context
/// when the chain has no root cause distinct from the wrapped error.
fn safe_message(error: &Error) -> String {
    let root = error.root_cause().to_string();
    let text = if root.is_empty() {
        error.to_string()
    } else {
        root
    };
    bounded(&redact_secrets(&text))
}

fn bounded(message: &str) -> String {
    let collapsed = message.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= MAX_MESSAGE_CHARS {
        return collapsed;
    }
    let truncated: String = collapsed.chars().take(MAX_MESSAGE_CHARS).collect();
    format!("{truncated}...")
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use anyhow::anyhow;

    use super::*;
    use crate::database::{OperationTimeout, attach_cancellation};

    fn timed_out(cancellation: Cancellation, result_unknown: bool) -> Error {
        let timeout = Error::new(OperationTimeout::new(
            Operation::Execute,
            Duration::from_millis(50),
        ));
        attach_cancellation(timeout, Operation::Execute, cancellation, result_unknown)
    }

    #[test]
    fn confirmed_timeout_serializes_stable_code_and_cancellation() {
        let classified = classify(&timed_out(Cancellation::Confirmed, false), Some("pg"));
        assert_eq!(classified.code, ErrorCode::OperationTimeout);

        let value = serde_json::to_value(&classified.payload).expect("serialize payload");
        assert_eq!(value["code"], "operation_timeout");
        assert_eq!(value["kind"], "timeout");
        assert_eq!(value["operation"], "execute");
        assert_eq!(value["connection"], "pg");
        assert_eq!(value["timeout"], true);
        assert_eq!(value["cancellation"], "confirmed");
        assert_eq!(value["resultUnknown"], false);
        let message = value["message"].as_str().expect("message");
        assert!(message.contains("timeout"), "{message}");
        assert!(message.contains("confirmed"), "{message}");
    }

    #[test]
    fn unconfirmed_timeout_marks_the_result_unknown() {
        let classified = classify(&timed_out(Cancellation::Unconfirmed, true), None);
        let value = serde_json::to_value(&classified.payload).expect("serialize payload");
        assert_eq!(value["cancellation"], "unconfirmed");
        assert_eq!(value["resultUnknown"], true);
        let message = value["message"].as_str().expect("message");
        assert!(message.contains("do not blindly retry"), "{message}");
    }

    #[test]
    fn plain_backend_error_redacts_credentials() {
        let error = anyhow!("connect failed dsn=postgres://alice:s3cret@db.example/app");
        let classified = classify(&error, Some("pg"));
        assert_eq!(classified.code, ErrorCode::BackendError);

        let value = serde_json::to_value(&classified.payload).expect("serialize payload");
        assert_eq!(value["code"], "backend_error");
        assert_eq!(value["kind"], "backend");
        assert_eq!(value["connection"], "pg");
        assert!(value.get("timeout").is_none());
        assert!(value.get("operation").is_none());
        let message = value["message"].as_str().expect("message");
        assert!(!message.contains("s3cret"), "{message}");
        assert!(message.contains("REDACTED"), "{message}");
    }

    #[test]
    fn missing_connection_has_a_stable_code_and_message() {
        let error = anyhow!("connection \"ghost\" not found");
        let classified = classify(&error, Some("ghost"));
        assert_eq!(classified.code, ErrorCode::MissingConnection);
        assert_eq!(classified.payload.code, "missing_connection");
        assert_eq!(classified.payload.kind, "missing_connection");
        assert!(classified.payload.message.contains("ghost"));
    }

    #[test]
    fn error_json_value_is_an_object_with_the_stable_fields() {
        let value = error_json_value(&timed_out(Cancellation::Confirmed, false), Some("pg"));
        assert_eq!(value["code"], "operation_timeout");
        assert_eq!(value["kind"], "timeout");
        assert!(value["message"].is_string());
    }
}
