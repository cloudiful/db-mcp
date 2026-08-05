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
    err.chain().map(ToString::to_string).collect()
}

pub fn format_error_chain(err: &anyhow::Error) -> String {
    format!("{err:#}")
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
