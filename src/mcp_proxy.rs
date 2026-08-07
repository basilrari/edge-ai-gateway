//! Authenticated reverse-proxy to edge-ai-MCP SSE (Hermes / external MCP clients).

use axum::{
    body::Body,
    extract::{Request, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};
use futures_util::StreamExt;
use tracing::warn;

use crate::config;
use crate::server::AppState;

fn extract_bearer(headers: &axum::http::HeaderMap) -> Option<String> {
    if let Some(v) = headers.get(header::AUTHORIZATION) {
        if let Ok(s) = v.to_str() {
            if let Some(token) = s.strip_prefix("Bearer ") {
                return Some(token.trim().to_string());
            }
        }
    }
    headers
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string())
}

fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        axum::Json(serde_json::json!({
            "ok": false,
            "error": "unauthorized",
            "hint": "Send Authorization: Bearer <MCP_API_KEY> or X-API-Key header"
        })),
    )
        .into_response()
}

fn mcp_disabled() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        axum::Json(serde_json::json!({
            "ok": false,
            "error": "mcp_public_disabled",
            "hint": "Set MCP_API_KEY in sar-stack.env and restart sar-stack"
        })),
    )
        .into_response()
}

pub async fn mcp_proxy_handler(
    State(state): State<AppState>,
    req: Request,
) -> Result<Response, StatusCode> {
    let expected = match config::mcp_api_key() {
        Some(k) if !k.is_empty() => k,
        _ => return Ok(mcp_disabled()),
    };

    let (parts, body) = req.into_parts();
    if extract_bearer(&parts.headers).as_deref() != Some(expected.as_str()) {
        return Ok(unauthorized());
    }

    let path = parts.uri.path();
    let base = config::mcp_sse_base_url();
    let base = base.trim_end_matches('/');
    let suffix = path.strip_prefix("/mcp").unwrap_or(path);
    let mut target = if suffix.is_empty() {
        format!("{base}/mcp")
    } else {
        format!("{base}{suffix}")
    };
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
        if name == header::HOST || name == header::AUTHORIZATION {
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
