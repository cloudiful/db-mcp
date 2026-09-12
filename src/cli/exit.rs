use std::fmt;

use anyhow::Error;

use super::format::redact_secrets;

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
}

#[derive(Debug)]
pub struct CliError {
    pub kind: CliErrorKind,
    pub message: String,
}

impl CliError {
    pub fn new(kind: CliErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn invalid_args(message: impl Into<String>) -> Self {
        Self::new(CliErrorKind::InvalidArgs, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(CliErrorKind::Internal, message)
    }

    /// Classify a database-operation error chain and redact credentials.
    pub fn from_anyhow(err: Error) -> Self {
        let kind = classify(&err);
        Self::with_kind(kind, err)
    }

    /// Build an error from an anyhow chain with a caller-chosen kind, redacting
    /// credentials from the rendered message.
    pub fn with_kind(kind: CliErrorKind, err: Error) -> Self {
        Self::new(kind, redact_secrets(&format!("{err:#}")))
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

/// Classify a runtime error chain by the first matching signal. Invalid
/// arguments are normally constructed at CLI validation points; this covers
/// the connection-resolution, timeout, and backend signals that only surface
/// inside an anyhow chain.
fn classify(err: &Error) -> CliErrorKind {
    let text = format!("{err:#}").to_lowercase();
    if text.contains("not found") || text.contains("connection is required") {
        CliErrorKind::MissingConnection
    } else if text.contains("timeout") || text.contains("timed out") {
        CliErrorKind::Timeout
    } else if text.contains("invalid parameter") || text.contains("invalid argument") {
        CliErrorKind::InvalidArgs
    } else {
        CliErrorKind::Backend
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

        let timeout = CliError::from_anyhow(anyhow::anyhow!("query timeout"));
        assert_eq!(timeout.kind, CliErrorKind::Timeout);

        let backend = CliError::from_anyhow(anyhow::anyhow!("syntax error near SELECT"));
        assert_eq!(backend.kind, CliErrorKind::Backend);
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
        assert_eq!(cli.exit_code(), 2);
        assert!(!cli.message.contains("abc123"));
    }
}
