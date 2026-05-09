use anyhow::{Context, Result};
use axum::{Json, Router, routing::get, middleware};
use cloudiful_server::{ServerConfig as BaseServerConfig, axum::Server as HttpServer};
use std::sync::Arc;
use tracing::{info, warn};

use crate::{config, database, mcp};

pub fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
}

pub async fn run(cli: config::Cli) -> Result<()> {
    let cfg = config::Config::load(&cli)?;
    log_startup_plan(&cfg);
    let manager = database::Manager::new(&cfg).await?;
    log_startup(&cfg, &manager);
    let state = Arc::new(mcp::AppState {
        cfg: cfg.clone(),
        manager,
    });
    match cfg.server.transport {
        config::TransportMode::Stdio => run_stdio(state).await,
        config::TransportMode::Http => run_http(state).await,
        config::TransportMode::Both => run_both(state).await,
    }
}

async fn run_stdio(state: Arc<mcp::AppState>) -> Result<()> {
    let service = mcp::DbMcpService::new(state);
    let running = cloudiful_server::mcp::serve_stdio(service).await?;
    let _ = running.waiting().await;
    Ok(())
}

async fn run_http(state: Arc<mcp::AppState>) -> Result<()> {
    let cfg = http_config(state.clone())?;
    let app = build_router(state);
    HttpServer::new_with_state(cfg, app).start().await?;
    Ok(())
}

async fn run_both(state: Arc<mcp::AppState>) -> Result<()> {
    let cfg = http_config(state.clone())?;
    let app = build_router(state.clone());
    let http = HttpServer::new_with_state(cfg, app);
    let http_task = tokio::spawn(async move { http.start().await });
    let stdio_task = tokio::spawn(async move { run_stdio(state).await });

    tokio::select! {
        res = http_task => {
            res.context("join http task")??;
        }
        res = stdio_task => {
            res.context("join stdio task")??;
        }
        _ = tokio::signal::ctrl_c() => {
            warn!("shutdown requested");
        }
    }
    Ok(())
}

fn http_config(state: Arc<mcp::AppState>) -> Result<cloudiful_server::ValidatedServerConfig<Arc<mcp::AppState>>> {
    let http_addr = state.cfg.server.http_addr.clone();
    Ok(BaseServerConfig::new()
        .with_listen_addr(http_addr.clone())
        .with_app_data(state)
        .build()
        .with_context(|| format!("invalid http listen address addr={http_addr}"))?)
}

fn build_router(state: Arc<mcp::AppState>) -> Router<Arc<mcp::AppState>> {
    let mcp_service = mcp::mcp_service(state.clone());
    let mcp_path = state.cfg.server.http_path.clone();
    let mcp_router = Router::new().nest_service(&mcp_path, mcp_service);
    let mcp_router = if state.cfg.server.http_auth.mode == crate::config::HttpAuthMode::Bearer {
        mcp_router.route_layer(middleware::from_fn_with_state(state.clone(), mcp::bearer_auth))
    } else {
        mcp_router
    };

    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .merge(mcp_router)
}

async fn healthz() -> &'static str {
    "ok"
}

async fn readyz(
    State(state): State<Arc<mcp::AppState>>,
) -> Result<(axum::http::StatusCode, Json<database::HealthSnapshot>), (axum::http::StatusCode, String)> {
    match state.manager.health(None, true).await {
        Ok(snapshot) => {
            let code = if snapshot.status == "down" {
                axum::http::StatusCode::SERVICE_UNAVAILABLE
            } else {
                axum::http::StatusCode::OK
            };
            Ok((code, Json(snapshot)))
        }
        Err(err) => Err((axum::http::StatusCode::INTERNAL_SERVER_ERROR, err.to_string())),
    }
}

fn log_startup_plan(cfg: &config::Config) {
    info!(
        "db-mcp starting transport={:?} debug={}",
        cfg.server.transport, cfg.server.debug
    );
    for conn in &cfg.connections {
        info!(
            "database connecting name={} driver={} target={}",
            conn.name,
            conn.driver,
            connection_target_summary(conn)
        );
    }
}

fn log_startup(cfg: &config::Config, manager: &database::Manager) {
    for info in manager.connection_infos() {
        if info.available {
            info!(
                "database probe name={} driver={} default_schema={} status={}",
                info.name, info.driver, info.default_schema, info.status
            );
        } else {
            info!(
                "database probe name={} driver={} default_schema={} status={} last_error={:?}",
                info.name, info.driver, info.default_schema, info.status, info.last_error
            );
        }
    }
    if matches!(cfg.server.transport, config::TransportMode::Http | config::TransportMode::Both) {
        let (host, port) = split_listen_addr(&cfg.server.http_addr);
        info!(
            "http MCP listening host={} port={} path={} healthz=/healthz readyz=/readyz",
            host, port, cfg.server.http_path
        );
    }
}

fn split_listen_addr(addr: &str) -> (String, String) {
    if addr.is_empty() {
        return ("0.0.0.0".to_string(), "8080".to_string());
    }
    match addr.rsplit_once(':') {
        Some((host, port)) => (
            if host.is_empty() { "0.0.0.0".to_string() } else { host.to_string() },
            port.to_string(),
        ),
        None => (addr.to_string(), String::new()),
    }
}

fn connection_target_summary(conn: &config::ConnectionConfig) -> String {
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

fn redact_postgres_dsn(raw: &str) -> String {
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

use axum::extract::State;
