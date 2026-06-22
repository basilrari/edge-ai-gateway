use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::ws::WebSocketUpgrade,
    extract::State,
    http::{HeaderMap, Method, StatusCode},
    routing::{any, get, post},
    Json, Router,
};
use reqwest::Client;
use tokio::sync::Mutex;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing::{info, info_span, warn};
use uuid::Uuid;

use crate::config;
use crate::infer_log::InferLog;
use crate::orchestrator::Orchestrator;
use crate::types::{ApiResponse, GatewayCommand};

#[derive(Clone)]
pub struct AppState {
    pub orchestrator: Arc<Mutex<Orchestrator>>,
    pub client: Client,
    pub infer_log: InferLog,
}

pub fn build_router(state: AppState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers(Any);

    let app = Router::new()
        .route("/infer", post(infer_handler))
        .route("/status", get(status_handler))
        .route("/drone/position", get(drone_position_handler))
        .route("/drone/telemetry", get(drone_telemetry_handler))
        .route("/drone/mission", get(drone_mission_handler))
        .route("/drone/mission/upload", post(drone_mission_upload_handler))
        .route("/drone/mission/clear", post(drone_mission_clear_handler))
        .route("/drone/logs", get(drone_logs_handler))
        .route("/drone/logs/clear", post(drone_logs_clear_handler))
        .route("/drone/logs/mavlink", get(drone_mavlink_logs_handler))
        .route("/drone/logs/ws", get(drone_logs_ws_handler))
        .route("/logs/llm", get(llm_logs_handler))
        .route("/logs/llm/clear", post(llm_logs_clear_handler))
        .route("/logs/clear-all", post(logs_clear_all_handler))
        .route("/camera/stream", get(camera_stream_handler))
        .route("/drone/ws", get(drone_ws_handler))
        .route("/mcp/sse", any(crate::mcp_proxy::mcp_proxy_handler))
        .route("/mcp/messages", any(crate::mcp_proxy::mcp_proxy_handler))
        .route("/mcp/messages/", any(crate::mcp_proxy::mcp_proxy_handler));

    #[cfg(feature = "eval")]
    let app = app.merge(crate::eval::eval_router());

    app.layer(TraceLayer::new_for_http())
        .layer(cors)
        .with_state(state)
}

pub(crate) fn pick_request_id(headers: &HeaderMap) -> String {
    headers
        .get("x-request-id")
        .and_then(|h| h.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| Uuid::new_v4().to_string())
}

async fn infer_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<serde_json::Value>,
) -> Json<ApiResponse> {
    let request_id = pick_request_id(&headers);
    let span = info_span!("http_infer", request_id = %request_id);
    let _guard = span.enter();

    let cmd: GatewayCommand = match serde_json::from_value(payload.clone()) {
        Ok(cmd) => cmd,
        Err(e) => {
            warn!(
                action = "infer_parse_failed",
                request_id = %request_id,
                error = %e,
                reason = "failed to parse GatewayCommand from /infer payload"
            );
            return Json(ApiResponse {
                state: "ERROR".to_string(),
                model: None,
                override_active: false,
                category: None,
                tool_name: None,
                pending_approval: false,
                llm_response: format!("invalid payload: {e}"),
                action_taken: "parse_failed".to_string(),
                latency_ms: 0,
                llm_latency_ms: 0,
                request_id,
                debug_trace: vec!["stage=parse_infer_payload_failed".into()],
                drone_http_status: None,
                drone_http_ms: None,
                drone_error: None,
                tool_params: None,
                tools: None,
                llm_tool_json: None,
            });
        }
    };

    let prompt_text = match &cmd {
        GatewayCommand::Infer { prompt } => Some(prompt.clone()),
        _ => None,
    };

    let mut orchestrator = state.orchestrator.lock().await;
    info!(
        action = "http_infer_received",
        request_id = %request_id,
        state = ?orchestrator.current_state,
        model = %orchestrator.effective_model_name(),
        reason = "processing /infer request"
    );

    let outcome = orchestrator
        .process_command(cmd, &state.client, &request_id)
        .await;

    let model = orchestrator.current_model.clone();
    let override_active = orchestrator.override_until.is_some();
    let state_str = format!("{}", orchestrator.current_state);
    drop(orchestrator);

    if let Some(prompt) = prompt_text {
        state.infer_log.push(
            prompt,
            outcome.llm_tool_json.clone(),
            Some(outcome.action_taken.clone()),
            model.clone(),
            request_id.clone(),
        );
    }

    let api = ApiResponse {
        state: state_str,
        model,
        override_active,
        category: outcome.category.clone(),
        tool_name: outcome.tool_name.clone(),
        pending_approval: outcome.pending_approval,
        llm_response: outcome.llm_response,
        action_taken: outcome.action_taken,
        latency_ms: outcome.latency_ms,
        llm_latency_ms: outcome.llm_latency_ms,
        request_id,
        debug_trace: outcome.trace,
        drone_http_status: outcome.drone_http_status,
        drone_http_ms: outcome.drone_http_ms,
        drone_error: outcome.drone_error,
        tool_params: outcome.tool_params,
        tools: outcome.tools,
        llm_tool_json: outcome.llm_tool_json,
    };

    Json(api)
}

async fn status_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Json<serde_json::Value> {
    let request_id = pick_request_id(&headers);
    let span = info_span!("http_status", request_id = %request_id);
    let _guard = span.enter();

    let mut orchestrator = state.orchestrator.lock().await;

    let outcome = orchestrator
        .process_command(GatewayCommand::Status, &state.client, &request_id)
        .await;

    Json(serde_json::json!({
        "state": format!("{}", orchestrator.current_state),
        "model": orchestrator.effective_model_name(),
        "override_active": orchestrator.override_until.is_some(),
        "active_command": orchestrator.active_command_display(),
        "latency_ms": outcome.latency_ms,
        "llm_latency_ms": outcome.llm_latency_ms,
        "memory_estimate_mb": outcome.memory_estimate_mb,
        "request_id": request_id,
        "debug_trace": outcome.trace,
    }))
}

async fn drone_ws_handler(ws: WebSocketUpgrade) -> impl axum::response::IntoResponse {
    let drone_url = config::drone_telemetry_ws_url();
    ws.on_upgrade(move |socket| crate::drone_ws::relay_telemetry_ws(socket, drone_url))
}

async fn proxy_drone_get(
    state: &AppState,
    request_id: &str,
    url: &str,
    fail_label: &str,
) -> (StatusCode, Json<serde_json::Value>) {
    let send = state
        .client
        .get(url)
        .header("x-request-id", request_id)
        .timeout(Duration::from_secs(5))
        .send()
        .await;

    match send {
        Ok(resp) => {
            let code = resp.status().as_u16();
            let status = StatusCode::from_u16(code).unwrap_or(StatusCode::BAD_GATEWAY);
            let text = resp.text().await.unwrap_or_default();
            let body: serde_json::Value = serde_json::from_str(&text).unwrap_or(serde_json::json!({
                "ok": false,
                "error": "drone_server_non_json_body",
                "body_prefix": text.chars().take(120).collect::<String>()
            }));
            (status, Json(body))
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({
                "ok": false,
                "error": format!("{fail_label}: {e}")
            })),
        ),
    }
}

/// Proxies `GET` drone-http `/v1/position` for the frontend map (CORS stays on the gateway).
async fn drone_position_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<serde_json::Value>) {
    let request_id = pick_request_id(&headers);
    proxy_drone_get(
        &state,
        &request_id,
        &config::drone_position_url(),
        "drone_position_proxy_failed",
    )
    .await
}

async fn drone_telemetry_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<serde_json::Value>) {
    let request_id = pick_request_id(&headers);
    proxy_drone_get(
        &state,
        &request_id,
        &config::drone_telemetry_url(),
        "drone_telemetry_proxy_failed",
    )
    .await
}

async fn drone_mission_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<serde_json::Value>) {
    let request_id = pick_request_id(&headers);
    proxy_drone_get(
        &state,
        &request_id,
        &config::drone_mission_url(),
        "drone_mission_proxy_failed",
    )
    .await
}

async fn drone_mission_upload_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let request_id = pick_request_id(&headers);
    let url = config::drone_mission_upload_url();
    let send = state
        .client
        .post(&url)
        .header("x-request-id", &request_id)
        .json(&body)
        .timeout(Duration::from_secs(45))
        .send()
        .await;

    match send {
        Ok(resp) => {
            let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            let text = resp.text().await.unwrap_or_default();
            let json: serde_json::Value = serde_json::from_str(&text).unwrap_or(serde_json::json!({
                "ok": false,
                "error": "drone_server_non_json_body"
            }));
            (status, Json(json))
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({
                "ok": false,
                "error": format!("drone_mission_upload_proxy_failed: {e}")
            })),
        ),
    }
}

async fn drone_mission_clear_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<serde_json::Value>) {
    let request_id = pick_request_id(&headers);
    let url = config::drone_mission_clear_url();
    let send = state
        .client
        .post(&url)
        .header("x-request-id", &request_id)
        .timeout(Duration::from_secs(10))
        .send()
        .await;

    match send {
        Ok(resp) => {
            let status =
                StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            let text = resp.text().await.unwrap_or_default();
            let json: serde_json::Value = serde_json::from_str(&text).unwrap_or(serde_json::json!({
                "ok": false,
                "error": "drone_server_non_json_body"
            }));
            (status, Json(json))
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({
                "ok": false,
                "error": format!("drone_mission_clear_proxy_failed: {e}")
            })),
        ),
    }
}

async fn drone_logs_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<serde_json::Value>) {
    let request_id = pick_request_id(&headers);
    proxy_drone_get(
        &state,
        &request_id,
        &config::drone_logs_url(),
        "drone_logs_proxy_failed",
    )
    .await
}

async fn drone_mavlink_logs_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<serde_json::Value>) {
    let request_id = pick_request_id(&headers);
    proxy_drone_get(
        &state,
        &request_id,
        &config::drone_mavlink_logs_url(),
        "drone_mavlink_logs_proxy_failed",
    )
    .await
}

async fn llm_logs_handler(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "entries": state.infer_log.snapshot() }))
}

async fn llm_logs_clear_handler(State(state): State<AppState>) -> Json<serde_json::Value> {
    state.infer_log.clear();
    Json(serde_json::json!({ "ok": true }))
}

async fn drone_logs_clear_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let request_id = pick_request_id(&headers);
    let url = config::drone_logs_clear_url();
    let send = state
        .client
        .post(&url)
        .header("x-request-id", &request_id)
        .json(&body)
        .timeout(Duration::from_secs(10))
        .send()
        .await;

    match send {
        Ok(resp) => {
            let status =
                StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            let text = resp.text().await.unwrap_or_default();
            let json: serde_json::Value = serde_json::from_str(&text).unwrap_or(serde_json::json!({
                "ok": false,
                "error": "drone_server_non_json_body"
            }));
            (status, Json(json))
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({
                "ok": false,
                "error": format!("drone_logs_clear_proxy_failed: {e}")
            })),
        ),
    }
}

async fn logs_clear_all_handler(State(state): State<AppState>) -> Json<serde_json::Value> {
    state.infer_log.clear();
    let url = config::drone_logs_clear_url();
    let drone_ok = state
        .client
        .post(&url)
        .json(&serde_json::json!({ "target": "all" }))
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false);
    Json(serde_json::json!({ "ok": true, "drone_ok": drone_ok }))
}

async fn camera_stream_handler() -> axum::response::Response {
    let url = config::camera_stream_url();
    crate::camera_stream::proxy_camera_stream(&url).await
}

async fn drone_logs_ws_handler(ws: WebSocketUpgrade) -> impl axum::response::IntoResponse {
    let drone_url = config::drone_logs_ws_url();
    ws.on_upgrade(move |socket| crate::logs_ws::relay_logs_ws(socket, drone_url))
}

pub async fn run_http_server(state: AppState) {
    let app = build_router(state);
    let addr: SocketAddr = "0.0.0.0:3000".parse().expect("invalid listen addr");

    info!(
        action = "http_server_start",
        addr = %addr,
        reason = "starting persistent HTTP server (RUST_LOG for filters; x-request-id forwarded to LLM and drone-http)"
    );

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("failed to bind address");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("server error");
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to listen for Ctrl-C");
    info!(action = "http_server_shutdown", reason = "Ctrl-C received");
}
