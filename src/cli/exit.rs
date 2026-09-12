use std::fmt;

use anyhow::Error;

use crate::mcp::errors::{self, ErrorCode, ErrorPayload};

/// Exit-code contract for the progressive CLI.
///
/// `0` success, `2` invalid arguments, `3` missing connection, `4` timeout,
/// `5` backend error, `1` any other internal error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliErrorKind {
    InvalidArgs,
    MissingConnection,
    Timeout,
    Backend,
    Internal,
}

impl CliErrorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidArgs => "invalid_args",
            Self::MissingConnection => "missing_connection",
            Self::Timeout => "timeout",
            Self::Backend => "backend",
            Self::Internal => "internal",
        }
    }

    pub fn exit_code(self) -> u8 {
        match self {
            Self::InvalidArgs => 2,
            Self::MissingConnection => 3,
            Self::Timeout => 4,
            Self::Backend => 5,
            Self::Internal => 1,
        }
    }

    /// The shared stable code for this coarse kind.
    pub const fn code(self) -> ErrorCode {
        match self {
            Self::InvalidArgs => ErrorCode::InvalidParams,
            Self::MissingConnection => ErrorCode::MissingConnection,
            Self::Timeout => ErrorCode::OperationTimeout,
            Self::Backend => ErrorCode::BackendError,
            Self::Internal => ErrorCode::InternalError,
        }
    }

    pub const fn from_code(code: ErrorCode) -> Self {
        match code {
            ErrorCode::InvalidParams => Self::InvalidArgs,
            ErrorCode::MissingConnection => Self::MissingConnection,
            ErrorCode::OperationTimeout => Self::Timeout,
            ErrorCode::BackendError => Self::Backend,
            ErrorCode::InternalError => Self::Internal,
        }
    }
}

#[derive(Debug)]
pub struct CliError {
    pub kind: CliErrorKind,
    pub message: String,
    pub code: ErrorCode,
    pub operation: Option<String>,
    pub timeout: Option<bool>,
    pub cancellation: Option<String>,
    pub result_unknown: Option<bool>,
}

impl CliError {
    pub fn new(kind: CliErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            code: kind.code(),
            message: message.into(),
            operation: None,
            timeout: None,
            cancellation: None,
            result_unknown: None,
        }
    }

    pub fn invalid_args(message: impl Into<String>) -> Self {
        Self::new(CliErrorKind::InvalidArgs, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(CliErrorKind::Internal, message)
    }

    /// Classify a database-operation error chain and redact credentials. The
    /// typed `OperationTimeout`/`CancellationReport` markers select the timeout
    /// kind; no string matching is used for timeout detection.
    pub fn from_anyhow(err: Error) -> Self {
        let classified = errors::classify(&err, None);
        errors::log_failure(&err, None);
        Self::from_classified(CliErrorKind::from_code(classified.code), classified.payload)
    }

    /// Build an error from an anyhow chain with a caller-chosen kind, redacting
    /// credentials and keeping any typed timeout detail from the chain.
    pub fn with_kind(kind: CliErrorKind, err: Error) -> Self {
        let classified = errors::classify(&err, None);
        errors::log_failure(&err, None);
        Self::from_classified(kind, classified.payload)
    }

    fn from_classified(kind: CliErrorKind, payload: ErrorPayload) -> Self {
        Self {
            kind,
            code: kind.code(),
            message: payload.message,
            operation: payload.operation,
            timeout: payload.timeout,
            cancellation: payload.cancellation,
            result_unknown: payload.result_unknown,
        }
    }

    pub fn exit_code(&self) -> u8 {
        self.kind.exit_code()
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::database::{Cancellation, Operation, OperationTimeout, attach_cancellation};

    #[test]
    fn exit_codes_follow_the_cli_contract() {
        assert_eq!(CliErrorKind::InvalidArgs.exit_code(), 2);
        assert_eq!(CliErrorKind::MissingConnection.exit_code(), 3);
        assert_eq!(CliErrorKind::Timeout.exit_code(), 4);
        assert_eq!(CliErrorKind::Backend.exit_code(), 5);
        assert_eq!(CliErrorKind::Internal.exit_code(), 1);
    }

    #[test]
    fn kinds_render_stable_names() {
        assert_eq!(CliErrorKind::InvalidArgs.as_str(), "invalid_args");
        assert_eq!(
            CliErrorKind::MissingConnection.as_str(),
            "missing_connection"
        );
        assert_eq!(CliErrorKind::Timeout.as_str(), "timeout");
        assert_eq!(CliErrorKind::Backend.as_str(), "backend");
        assert_eq!(CliErrorKind::Internal.as_str(), "internal");
    }

    #[test]
    fn classifies_missing_connection_and_timeout_and_backend() {
        let missing = CliError::from_anyhow(anyhow::anyhow!("connection \"bogus\" not found"));
        assert_eq!(missing.kind, CliErrorKind::MissingConnection);
        assert_eq!(missing.code, ErrorCode::MissingConnection);

        let timeout = CliError::from_anyhow(Error::new(OperationTimeout::new(
            Operation::Query,
            Duration::from_millis(5),
        )));
        assert_eq!(timeout.kind, CliErrorKind::Timeout);
        assert_eq!(timeout.code, ErrorCode::OperationTimeout);
        assert_eq!(timeout.operation.as_deref(), Some("query"));
        assert_eq!(timeout.timeout, Some(true));

        let backend = CliError::from_anyhow(anyhow::anyhow!("syntax error near SELECT"));
        assert_eq!(backend.kind, CliErrorKind::Backend);
        assert_eq!(backend.code, ErrorCode::BackendError);
    }

    #[test]
    fn typed_cancellation_is_carried_into_the_cli_error() {
        let err = attach_cancellation(
            Error::new(OperationTimeout::new(
                Operation::Fetch,
                Duration::from_millis(5),
            )),
            Operation::Fetch,
            Cancellation::Unconfirmed,
            true,
        );
        let cli = CliError::from_anyhow(err);
        assert_eq!(cli.kind, CliErrorKind::Timeout);
        assert_eq!(cli.cancellation.as_deref(), Some("unconfirmed"));
        assert_eq!(cli.result_unknown, Some(true));
        assert!(
            cli.message.contains("could not be confirmed"),
            "unexpected message: {}",
            cli.message
        );
    }

    #[test]
    fn from_anyhow_redacts_credentials_in_the_chain() {
        let err = anyhow::anyhow!("connect failed dsn=postgres://alice:s3cret@db.example/app")
            .context("password=hunter2");
        let cli = CliError::from_anyhow(err);
        assert!(!cli.message.contains("s3cret"), "{}", cli.message);
        assert!(!cli.message.contains("hunter2"), "{}", cli.message);
        assert!(cli.message.contains("REDACTED"), "{}", cli.message);
    }

    #[test]
    fn with_kind_preserves_explicit_kind_and_redacts() {
        let cli = CliError::with_kind(
            CliErrorKind::InvalidArgs,
            anyhow::anyhow!("token=abc123 is not valid"),
        );
        assert_eq!(cli.kind, CliErrorKind::InvalidArgs);
        assert_eq!(cli.code, ErrorCode::InvalidParams);
        assert_eq!(cli.exit_code(), 2);
        assert!(!cli.message.contains("abc123"));
    }
}
