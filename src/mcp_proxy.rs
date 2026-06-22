//! Reverse-proxy MCP SSE (edge-ai-mcp) for external clients via the public gateway URL.

use axum::{
    body::Body,
    extract::{Request, State},
    http::{header, StatusCode},
    response::Response,
};
use futures_util::StreamExt;
use tracing::warn;

use crate::config;
use crate::server::AppState;

pub async fn mcp_proxy_handler(
    State(state): State<AppState>,
    req: Request,
) -> Result<Response, StatusCode> {
    let (parts, body) = req.into_parts();
    let path = parts.uri.path();
    let suffix = path.strip_prefix("/mcp").unwrap_or(path);
    let mut target = format!(
        "{}{}",
        config::mcp_sse_base_url().trim_end_matches('/'),
        suffix
    );
    if let Some(q) = parts.uri.query() {
        target = format!("{target}?{q}");
    }

    let body_bytes = axum::body::to_bytes(body, 16 * 1024 * 1024)
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?;

    let mut rb = state
        .client
        .request(parts.method, &target)
        .body(body_bytes);

    for (name, value) in parts.headers.iter() {
        if name == header::HOST {
            continue;
        }
        rb = rb.header(name, value);
    }

    let upstream = rb.send().await.map_err(|e| {
        warn!(error = %e, url = %target, "mcp proxy upstream failed");
        StatusCode::BAD_GATEWAY
    })?;

    let status = upstream.status();
    let mut builder = Response::builder().status(status);
    for (name, value) in upstream.headers() {
        builder = builder.header(name, value);
    }

    let stream = upstream.bytes_stream().map(|chunk| {
        chunk.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
    });
    builder
        .body(Body::from_stream(stream))
        .map_err(|_| StatusCode::BAD_GATEWAY)
}
