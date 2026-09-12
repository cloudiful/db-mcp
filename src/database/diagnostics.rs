use anyhow::Result;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

use super::ConnectionInfo;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProbeCheck {
    pub phase: String,
    pub status: String,
    pub target: String,
    pub elapsed: String,
    pub error: Option<String>,
    pub error_chain: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionDiagnostic {
    pub name: String,
    pub driver: String,
    pub default_schema: String,
    pub status: String,
    pub available: bool,
    pub failure_phase: Option<String>,
    pub last_error: Option<String>,
    pub last_error_chain: Vec<String>,
    pub checks: Vec<ProbeCheck>,
}

pub fn error_chain(err: &anyhow::Error) -> Vec<String> {
    err.chain()
        .map(|cause| redact_secrets(&cause.to_string()))
        .collect()
}

pub fn format_error_chain(err: &anyhow::Error) -> String {
    redact_secrets(&format!("{err:#}"))
}

/// Best-effort removal of credentials from text before it reaches a public MCP
/// or CLI response. Driver error chains may otherwise echo a DSN or password.
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

pub fn probe_check_result(
    phase: impl Into<String>,
    target: impl Into<String>,
    elapsed: Duration,
    result: Result<()>,
) -> ProbeCheck {
    match result {
        Ok(()) => ProbeCheck {
            phase: phase.into(),
            status: "up".to_string(),
            target: target.into(),
            elapsed: format!("{elapsed:?}"),
            error: None,
            error_chain: vec![],
        },
        Err(err) => ProbeCheck {
            phase: phase.into(),
            status: "down".to_string(),
            target: target.into(),
            elapsed: format!("{elapsed:?}"),
            error: Some(format_error_chain(&err)),
            error_chain: error_chain(&err),
        },
    }
}

pub async fn timed_probe<F, Fut>(
    phase: impl Into<String>,
    target: impl Into<String>,
    f: F,
) -> ProbeCheck
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<()>>,
{
    let started = Instant::now();
    let result = f().await;
    probe_check_result(phase, target, started.elapsed(), result)
}

pub fn diagnostic_from_checks(
    info: ConnectionInfo,
    checks: Vec<ProbeCheck>,
) -> ConnectionDiagnostic {
    let failed = checks.iter().find(|check| check.status != "up");
    ConnectionDiagnostic {
        name: info.name,
        driver: info.driver,
        default_schema: info.default_schema,
        status: if failed.is_some() { "down" } else { "up" }.to_string(),
        available: failed.is_none(),
        failure_phase: failed.map(|check| check.phase.clone()),
        last_error: failed.and_then(|check| check.error.clone()),
        last_error_chain: failed
            .map(|check| check.error_chain.clone())
            .unwrap_or_default(),
        checks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn error_chain_and_rendering_are_redacted() {
        let err =
            anyhow::anyhow!("connect postgres://alice:s3cret@host/app").context("password=hunter2");
        let chain = error_chain(&err);
        assert!(
            chain.iter().all(|line| !line.contains("s3cret")),
            "unredacted chain: {chain:?}"
        );
        let rendered = format_error_chain(&err);
        assert!(!rendered.contains("hunter2"), "unredacted: {rendered}");
        assert!(rendered.contains("REDACTED"), "unredacted: {rendered}");
    }
}
