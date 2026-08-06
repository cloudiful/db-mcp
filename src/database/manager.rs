use anyhow::{Context, Result, bail};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
use tracing::{info, warn};

use crate::config;

use super::oracle::OracleConnection;
use super::unavailable::FailedConnection;
use super::{
    ConnectionDiagnostic, ConnectionHealth, ConnectionInfo, DatabaseConnection, HealthSnapshot,
    Limits, chrono_like_now, postgres, sqlite,
};

#[derive(Clone)]
struct Slot {
    conn: Arc<dyn DatabaseConnection>,
    health: Arc<Mutex<ConnectionHealth>>,
}

#[derive(Clone)]
pub struct Manager {
    slots: Arc<BTreeMap<String, Slot>>,
    limits: Limits,
}

impl Manager {
    pub async fn new(cfg: &config::Config) -> Result<Self> {
        let limits = Limits::from_config(cfg)?;
        let mut slots = BTreeMap::new();
        for conn_cfg in &cfg.connections {
            let target = connection_target_summary(conn_cfg);
            let conn = match open_connection(conn_cfg, &limits).await {
                Ok(conn) => {
                    info!(
                        "database connection initialized name={} driver={} target={}",
                        conn_cfg.name, conn_cfg.driver, target
                    );
                    conn
                }
                Err(err) => {
                    let error = format!("{:#}", err);
                    warn!(
                        "database connection unavailable name={} driver={} target={} error={}",
                        conn_cfg.name, conn_cfg.driver, target, error
                    );
                    Box::new(FailedConnection::new(
                        conn_cfg.name.clone(),
                        conn_cfg.driver.clone(),
                        default_schema_for_failed_connection(conn_cfg),
                        error,
                    ))
                }
            };
            let info = conn.info();
            let health = ConnectionHealth {
                name: info.name.clone(),
                driver: info.driver.clone(),
                default_schema: info.default_schema.clone(),
                status: "down".to_string(),
                available: false,
                last_checked_at: None,
                last_successful_at: None,
                last_error: None,
                failure_phase: None,
                last_error_chain: vec![],
                probe_checks: vec![],
            };
            slots.insert(
                info.name.clone(),
                Slot {
                    conn: Arc::from(conn),
                    health: Arc::new(Mutex::new(health)),
                },
            );
        }

        let manager = Self {
            slots: Arc::new(slots),
            limits,
        };
        manager.refresh_all().await;
        Ok(manager)
    }

    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    pub fn connection_infos(&self) -> Vec<ConnectionInfo> {
        self.slots
            .values()
            .map(|slot| slot.health.lock().expect("health poisoned").clone())
            .collect()
    }

    pub fn connection(&self, name: Option<&str>) -> Result<Arc<dyn DatabaseConnection>> {
        let Some(name) = name.map(str::trim).filter(|name| !name.is_empty()) else {
            bail!(
                "connection is required; call list_connections first and pass one of the returned names"
            );
        };
        self.slots
            .get(name)
            .map(|slot| slot.conn.clone())
            .with_context(|| format!("connection {name:?} not found"))
    }

    pub async fn health(&self, name: Option<&str>, refresh: bool) -> Result<HealthSnapshot> {
        if refresh {
            if let Some(name) = name {
                self.refresh_one(name).await?;
            } else {
                self.refresh_all().await;
            }
        }
        let mut connections = vec![];
        if let Some(name) = name {
            let slot = self
                .slots
                .get(name)
                .with_context(|| format!("connection {name:?} not found"))?;
            connections.push(slot.health.lock().expect("health poisoned").clone());
        } else {
            for slot in self.slots.values() {
                connections.push(slot.health.lock().expect("health poisoned").clone());
            }
        }
        connections.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(HealthSnapshot {
            status: overall_status(&connections),
            connections,
        })
    }

    pub async fn diagnose(
        &self,
        name: Option<&str>,
        deep: bool,
    ) -> Result<Vec<ConnectionDiagnostic>> {
        if let Some(name) = name {
            let slot = self
                .slots
                .get(name)
                .with_context(|| format!("connection {name:?} not found"))?;
            return Ok(vec![slot.conn.diagnose(deep).await]);
        }
        let mut tasks = vec![];
        for slot in self.slots.values() {
            let conn = slot.conn.clone();
            tasks.push(async move { conn.diagnose(deep).await });
        }
        Ok(futures_util::future::join_all(tasks).await)
    }

    async fn refresh_all(&self) {
        let mut tasks = vec![];
        for slot in self.slots.values() {
            tasks.push(Self::refresh_slot(slot.clone()));
        }
        futures_util::future::join_all(tasks).await;
    }

    async fn refresh_one(&self, name: &str) -> Result<()> {
        let slot = self
            .slots
            .get(name)
            .with_context(|| format!("connection {name:?} not found"))?
            .clone();
        Self::refresh_slot(slot).await;
        Ok(())
    }

    async fn refresh_slot(slot: Slot) {
        let now = chrono_like_now();
        let diagnostic = slot.conn.diagnose(false).await;
        let mut health = slot.health.lock().expect("health poisoned");
        health.last_checked_at = Some(now.clone());
        health.default_schema = diagnostic.default_schema;
        health.failure_phase = diagnostic.failure_phase;
        health.last_error_chain = diagnostic.last_error_chain;
        health.probe_checks = diagnostic.checks;
        match diagnostic.available {
            true => {
                health.available = true;
                health.status = "up".to_string();
                health.last_successful_at = Some(now);
                health.last_error = None;
            }
            false => {
                health.available = false;
                health.status = "down".to_string();
                health.last_error = diagnostic.last_error;
            }
        }
    }
}

fn overall_status(connections: &[ConnectionHealth]) -> String {
    if connections.iter().all(|c| c.available) {
        "up".to_string()
    } else if connections.iter().any(|c| c.available) {
        "degraded".to_string()
    } else {
        "down".to_string()
    }
}

fn default_schema_for_failed_connection(cfg: &config::ConnectionConfig) -> String {
    match cfg.driver.as_str() {
        "postgres" => cfg
            .postgres
            .as_ref()
            .and_then(|c| {
                let schema = c.schema.trim();
                (!schema.is_empty()).then(|| schema.to_string())
            })
            .unwrap_or_else(|| "public".to_string()),
        "sqlite" => "main".to_string(),
        "oracle" => cfg
            .oracle
            .as_ref()
            .map(|c| c.username.to_uppercase())
            .unwrap_or_default(),
        _ => String::new(),
    }
}

pub fn connection_target_summary(conn: &config::ConnectionConfig) -> String {
    match conn.driver.as_str() {
        "oracle" => conn
            .oracle
            .as_ref()
            .map(|c| c.jdbc_url.clone())
            .unwrap_or_else(|| "oracle:<missing>".to_string()),
        "postgres" => conn
            .postgres
            .as_ref()
            .map(|c| redact_postgres_dsn(&c.dsn))
            .unwrap_or_else(|| "postgres:<missing>".to_string()),
        "sqlite" => conn
            .sqlite
            .as_ref()
            .map(|c| c.dsn.clone())
            .unwrap_or_else(|| "sqlite:<missing>".to_string()),
        _ => "<unknown>".to_string(),
    }
}

pub fn redact_postgres_dsn(raw: &str) -> String {
    if let Ok(mut url) = url::Url::parse(raw) {
        if url.password().is_some() {
            let username = url.username().to_string();
            let _ = url.set_password(Some("REDACTED"));
            if username.is_empty() {
                let _ = url.set_username("");
            }
        }
        return url.to_string();
    }
    raw.split_whitespace()
        .map(|part| {
            if part.to_lowercase().starts_with("password=") {
                "password=REDACTED".to_string()
            } else {
                part.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

async fn open_connection(
    cfg: &config::ConnectionConfig,
    limits: &Limits,
) -> Result<Box<dyn DatabaseConnection>> {
    match cfg.driver.as_str() {
        "postgres" => Ok(Box::new(
            postgres::PostgresConnection::new(
                cfg.name.clone(),
                cfg.postgres
                    .as_ref()
                    .with_context(|| format!("postgres config missing name={}", cfg.name))?
                    .clone(),
                limits.clone(),
            )
            .with_context(|| {
                format!(
                    "postgres connection init failed name={} target={}",
                    cfg.name,
                    connection_target_summary(cfg)
                )
            })?,
        )),
        "sqlite" => Ok(Box::new(
            sqlite::SQLiteConnection::new(
                cfg.name.clone(),
                cfg.sqlite
                    .as_ref()
                    .with_context(|| format!("sqlite config missing name={}", cfg.name))?
                    .clone(),
                limits.clone(),
            )
            .with_context(|| {
                format!(
                    "sqlite connection init failed name={} target={}",
                    cfg.name,
                    connection_target_summary(cfg)
                )
            })?,
        )),
        "oracle" => Ok(Box::new(OracleConnection::new(
            cfg.name.clone(),
            cfg.oracle
                .as_ref()
                .with_context(|| format!("oracle config missing name={}", cfg.name))?
                .clone(),
            limits.clone(),
        )?)),
        other => bail!("unsupported driver {other}"),
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, sync::Arc};

    use super::{Manager, Slot};
    use crate::database::unavailable::FailedConnection;
    use crate::database::{ConnectionHealth, Limits};

    fn test_manager() -> Manager {
        let conn = Arc::new(FailedConnection::new(
            "postgres".to_string(),
            "postgres".to_string(),
            "public".to_string(),
            "offline".to_string(),
        ));
        let mut slots = BTreeMap::new();
        slots.insert(
            "postgres".to_string(),
            Slot {
                health: Arc::new(std::sync::Mutex::new(ConnectionHealth {
                    name: "postgres".to_string(),
                    driver: "postgres".to_string(),
                    default_schema: "public".to_string(),
                    status: "down".to_string(),
                    available: false,
                    last_checked_at: None,
                    last_successful_at: None,
                    last_error: Some("offline".to_string()),
                    failure_phase: Some("config".to_string()),
                    last_error_chain: vec!["offline".to_string()],
                    probe_checks: vec![],
                })),
                conn,
            },
        );

        Manager {
            slots: Arc::new(slots),
            limits: Limits {
                default_max_rows: 5,
                hard_max_rows: 20,
                max_columns: 40,
                max_cell_chars: 512,
                max_total_chars: 1000,
                explain_max_lines: 12,
                explain_max_total_chars: 4000,
                query_timeout: std::time::Duration::from_secs(60),
            },
        }
    }

    #[test]
    fn connection_requires_explicit_name() {
        let manager = test_manager();
        let result = manager.connection(None);
        assert!(result.is_err(), "missing name should fail");
        let err = result.err().expect("error should exist");
        let message = err.to_string();
        assert!(
            message.contains("connection is required") && message.contains("list_connections"),
            "unexpected error: {err:#}"
        );
    }

    #[test]
    fn connection_rejects_blank_name() {
        let manager = test_manager();
        let result = manager.connection(Some("   "));
        assert!(result.is_err(), "blank name should fail");
        let err = result.err().expect("error should exist");
        let message = err.to_string();
        assert!(
            message.contains("connection is required") && message.contains("list_connections"),
            "unexpected error: {err:#}"
        );
    }
}
