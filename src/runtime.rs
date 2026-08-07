use anyhow::{Context, Result};
use axum::{Json, Router, middleware, routing::get};
use std::env;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use tower_http::cors::{Any, CorsLayer};
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

const HEALTH_REFRESH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);

/// Periodically refresh the cached connection health so `/readyz` and
/// `list_connections` (which serve cached state) do not stay frozen on the
/// startup snapshot. Ticks are skipped while a refresh is still in flight.
async fn background_refresh(manager: database::Manager, ct: CancellationToken) {
    let in_flight = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut interval = tokio::time::interval(HEALTH_REFRESH_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = ct.cancelled() => break,
            _ = interval.tick() => {
                if !in_flight.swap(true, std::sync::atomic::Ordering::SeqCst) {
                    manager.refresh_all().await;
                    in_flight.store(false, std::sync::atomic::Ordering::SeqCst);
                }
            }
        }
    }
}

async fn run_stdio(state: Arc<mcp::AppState>) -> Result<()> {
    transport::serve_stdio(mcp::DbMcpService::new(state)).await
}

async fn run_http(state: Arc<mcp::AppState>) -> Result<()> {
    let ct = CancellationToken::new();
    let addr = state.cfg.server.http_addr.clone();
    let app = build_router(state.clone());
    let refresh_task = tokio::spawn(background_refresh(state.manager.clone(), ct.child_token()));
    let mut http_task = tokio::spawn(transport::serve_http(app, addr, ct.child_token()));
    tokio::select! {
        res = &mut http_task => {
            ct.cancel();
            let _ = refresh_task.await;
            res.context("join http task")??;
        }
        _ = tokio::signal::ctrl_c() => {
            warn!("shutdown requested");
            ct.cancel();
            let _ = refresh_task.await;
            let _ = http_task.await;
        }
    }
    Ok(())
}

async fn run_both(state: Arc<mcp::AppState>) -> Result<()> {
    let ct = CancellationToken::new();
    let addr = state.cfg.server.http_addr.clone();
    let app = build_router(state.clone());
    let refresh_task = tokio::spawn(background_refresh(state.manager.clone(), ct.child_token()));
    let mut http_task = tokio::spawn(transport::serve_http(app, addr, ct.child_token()));
    let mut stdio_task = tokio::spawn(transport::serve_stdio(mcp::DbMcpService::new(state)));

    tokio::select! {
        res = &mut http_task => {
            ct.cancel();
            let _ = refresh_task.await;
            res.context("join http task")??;
        }
        res = &mut stdio_task => {
            ct.cancel();
            let _ = refresh_task.await;
            let _ = http_task.await;
            res.context("join stdio task")??;
        }
        _ = tokio::signal::ctrl_c() => {
            warn!("shutdown requested");
            ct.cancel();
            let _ = refresh_task.await;
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

    let mut router = Router::new()
        .route("/healthz", get(healthz))
        .route(
            "/readyz",
            get({
                let state = state.clone();
                move || readyz(state.clone())
            }),
        )
        .merge(mcp_router);
    if !state.cfg.server.http_cors_allowed_origins.is_empty() {
        let origins: Vec<axum::http::HeaderValue> = state
            .cfg
            .server
            .http_cors_allowed_origins
            .iter()
            .map(|origin| {
                axum::http::HeaderValue::from_str(origin)
                    .expect("validated http_cors_allowed_origins")
            })
            .collect();
        router = router.layer(
            CorsLayer::new()
                .allow_origin(origins)
                .allow_methods(Any)
                .allow_headers(Any),
        );
    }
    router
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
    // Serve the cached probe state instead of re-probing every connection on
    // every request; use diagnose_connection for on-demand deep checks.
    match state.manager.health(None, false).await {
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
        let (host, port) = config::split_host_port(&cfg.server.http_addr);
        info!(
            "http MCP listening host={} port={} path={} healthz=/healthz readyz=/readyz",
            host, port, cfg.server.http_path
        );
        if !config::is_loopback_host(&host)
            && cfg.server.http_auth.mode == config::HttpAuthMode::None
        {
            warn!(
                "http MCP exposed on {host}:{port} without authentication; any client that can reach it can invoke database tools"
            );
        }
    }
}

fn debug_requested() -> bool {
    env::var("DBMCP_DEBUG")
        .map(|v| matches!(v.to_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode, header::AUTHORIZATION};
    use tower::ServiceExt;

    async fn test_state(auth_token: Option<&str>) -> Arc<mcp::AppState> {
        let cfg = config::Config {
            server: config::ServerConfig {
                http_auth: config::HttpAuthConfig {
                    mode: if auth_token.is_some() {
                        config::HttpAuthMode::Bearer
                    } else {
                        config::HttpAuthMode::None
                    },
                    token: auth_token.unwrap_or("").to_string(),
                },
                ..config::ServerConfig::default()
            },
            limits: config::LimitsConfig::default(),
            connections: vec![config::ConnectionConfig {
                name: "sqlite".to_string(),
                driver: "sqlite".to_string(),
                sqlite: Some(config::SQLiteConfig {
                    dsn: "sqlite::memory:".to_string(),
                    read_only: true,
                }),
                postgres: None,
                oracle: None,
            }],
        };
        let manager = database::Manager::new(&cfg).await.expect("manager");
        Arc::new(mcp::AppState { cfg, manager })
    }

    #[tokio::test]
    async fn healthz_is_not_authenticated() {
        let app = build_router(test_state(Some("secret-token")).await);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/healthz")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("healthz response");
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn bearer_auth_rejects_unauthenticated_mcp_requests() {
        let app = build_router(test_state(Some("secret-token")).await);
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/mcp")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("mcp response");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert!(
            response.headers().contains_key("www-authenticate"),
            "401 must carry a WWW-Authenticate challenge"
        );
    }

    #[tokio::test]
    async fn bearer_auth_allows_valid_token() {
        let app = build_router(test_state(Some("secret-token")).await);
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/mcp")
                    .header(AUTHORIZATION, "Bearer secret-token")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("mcp response");
        assert_ne!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn without_auth_mode_mcp_requests_are_not_authenticated() {
        let app = build_router(test_state(None).await);
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/mcp")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("mcp response");
        assert_ne!(response.status(), StatusCode::UNAUTHORIZED);
    }
}
