use anyhow::{Context, Result};
use axum::Router;
use rmcp::ServiceExt;
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::never::NeverSessionManager,
};
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use crate::mcp::DbMcpService;

/// Serve a single MCP service over stdio until the transport closes.
pub async fn serve_stdio(service: DbMcpService) -> Result<()> {
    let running = service.serve(rmcp::transport::stdio()).await?;
    let _ = running.waiting().await;
    Ok(())
}

/// Build the streamable HTTP MCP router mounted at `path`.
///
/// The transport runs in sessionless mode: every request is served
/// statelessly, so no session state is kept between calls.
pub fn mcp_router(
    service_factory: impl Fn() -> Result<DbMcpService, std::io::Error> + Send + Sync + 'static,
    path: &str,
) -> Router {
    let config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .disable_allowed_hosts()
        .with_sse_keep_alive(Some(std::time::Duration::from_secs(15)));
    let service = StreamableHttpService::new(
        service_factory,
        Arc::new(NeverSessionManager::default()),
        config,
    );
    Router::new().route_service(path, service)
}

/// Bind `addr` and serve `app` over HTTP, shutting down when `ct` is cancelled.
pub async fn serve_http(app: Router, addr: String, ct: CancellationToken) -> Result<()> {
    let listener = TcpListener::bind(&addr)
        .await
        .with_context(|| format!("invalid http listen address addr={addr}"))?;
    axum::serve(listener, app)
        .with_graceful_shutdown(async move { ct.cancelled().await })
        .await
        .context("http server error")?;
    Ok(())
}
