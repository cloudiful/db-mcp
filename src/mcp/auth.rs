use std::sync::Arc;

use axum::extract::State;

use super::AppState;

pub async fn bearer_auth(
    State(state): State<Arc<AppState>>,
    request: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> axum::response::Response {
    if state.cfg.server.http_auth.mode == crate::config::HttpAuthMode::None {
        return next.run(request).await;
    }
    let expected = format!("Bearer {}", state.cfg.server.http_auth.token);
    match request
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    {
        Some(value) if constant_time_eq(value.trim().as_bytes(), expected.as_bytes()) => {
            next.run(request).await
        }
        _ => {
            let mut response =
                axum::response::Response::new(axum::body::Body::from("unauthorized"));
            *response.status_mut() = axum::http::StatusCode::UNAUTHORIZED;
            response.headers_mut().insert(
                axum::http::header::WWW_AUTHENTICATE,
                axum::http::HeaderValue::from_static("Bearer"),
            );
            response
        }
    }
}

/// Content comparison that does not short-circuit on the first differing byte,
/// avoiding token timing side channels; length differences fold into the diff.
pub(super) fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    let max_len = a.len().max(b.len());
    let mut diff = a.len() ^ b.len();
    for i in 0..max_len {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        diff |= (x ^ y) as usize;
    }
    diff == 0
}
