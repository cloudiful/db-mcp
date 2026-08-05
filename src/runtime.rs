use anyhow::{Context, Result};
use axum::{Json, Router, middleware, routing::get};
use std::env;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use tower_http::cors::CorsLayer;
use tracing::{info, warn};

use crate::runtime::mcp_transport as transport;
use crate::{config, database, mcp};

pub mod mcp_transport;

pub fn init_tracing() {
    let default_filter = if debug_requested() { "debug" } else { "info" };
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(default_filter));
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
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
    transport::serve_stdio(mcp::DbMcpService::new(state)).await
}

async fn run_http(state: Arc<mcp::AppState>) -> Result<()> {
    let ct = CancellationToken::new();
    let addr = state.cfg.server.http_addr.clone();
    let app = build_router(state);
    let mut http_task = tokio::spawn(transport::serve_http(app, addr, ct.child_token()));
    tokio::select! {
        res = &mut http_task => res.context("join http task")??,
        _ = tokio::signal::ctrl_c() => {
            warn!("shutdown requested");
            ct.cancel();
            let _ = http_task.await;
        }
    }
    Ok(())
}

async fn run_both(state: Arc<mcp::AppState>) -> Result<()> {
    let ct = CancellationToken::new();
    let addr = state.cfg.server.http_addr.clone();
    let app = build_router(state.clone());
    let mut http_task = tokio::spawn(transport::serve_http(app, addr, ct.child_token()));
    let mut stdio_task = tokio::spawn(transport::serve_stdio(mcp::DbMcpService::new(state)));

    tokio::select! {
        res = &mut http_task => res.context("join http task")??,
        res = &mut stdio_task => {
            ct.cancel();
            let _ = http_task.await;
            res.context("join stdio task")??;
        }
        _ = tokio::signal::ctrl_c() => {
            warn!("shutdown requested");
            ct.cancel();
            let _ = http_task.await;
            let _ = stdio_task.await;
        }
    }
    Ok(())
}

fn build_router(state: Arc<mcp::AppState>) -> Router {
    let mcp_router = transport::mcp_router(
        mcp_service_factory(state.clone()),
        &state.cfg.server.http_path,
    );
    let mcp_router = if state.cfg.server.http_auth.mode == crate::config::HttpAuthMode::Bearer {
        mcp_router.route_layer(middleware::from_fn_with_state(
            state.clone(),
            mcp::bearer_auth,
        ))
    } else {
        mcp_router
    };

    Router::new()
        .route("/healthz", get(healthz))
        .route(
            "/readyz",
            get({
                let state = state.clone();
                move || readyz(state.clone())
            }),
        )
        .merge(mcp_router)
        .layer(CorsLayer::permissive())
}

fn mcp_service_factory(
    state: Arc<mcp::AppState>,
) -> impl Fn() -> Result<mcp::DbMcpService, std::io::Error> + Send + Sync + 'static {
    move || Ok(mcp::DbMcpService::new(state.clone()))
}

async fn healthz() -> &'static str {
    "ok"
}

async fn readyz(
    state: Arc<mcp::AppState>,
) -> Result<
    (axum::http::StatusCode, Json<database::HealthSnapshot>),
    (axum::http::StatusCode, String),
> {
    match state.manager.health(None, true).await {
        Ok(snapshot) => {
            let code = if snapshot.status == "down" {
                axum::http::StatusCode::SERVICE_UNAVAILABLE
            } else {
                axum::http::StatusCode::OK
            };
            Ok((code, Json(snapshot)))
        }
        Err(err) => Err((
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            err.to_string(),
        )),
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
            database::connection_target_summary(conn)
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
    if matches!(
        cfg.server.transport,
        config::TransportMode::Http | config::TransportMode::Both
    ) {
        let (host, port) = split_listen_addr(&cfg.server.http_addr);
        info!(
            "http MCP listening host={} port={} path={} healthz=/healthz readyz=/readyz",
            host, port, cfg.server.http_path
        );
        if !matches!(host.as_str(), "127.0.0.1" | "::1" | "localhost")
            && cfg.server.http_auth.mode == config::HttpAuthMode::None
        {
            warn!(
                "http MCP exposed on {host}:{port} without authentication; any client that can reach it (including arbitrary web pages via permissive CORS) can invoke database tools"
            );
        }
    }
}

fn split_listen_addr(addr: &str) -> (String, String) {
    let addr = config::normalize_listen_addr(addr);
    match addr.rsplit_once(':') {
        Some((host, port)) => (host.to_string(), port.to_string()),
        None => (addr, String::new()),
    }
}

fn debug_requested() -> bool {
    env::var("DBMCP_DEBUG")
        .map(|v| matches!(v.to_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
}
