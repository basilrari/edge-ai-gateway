use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    extract::State,
    http::{HeaderMap, Method},
    routing::{get, post},
    Json, Router,
};
use reqwest::Client;
use tokio::sync::Mutex;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing::{info, info_span, warn};
use uuid::Uuid;

use crate::orchestrator::Orchestrator;
use crate::types::{ApiResponse, GatewayCommand};

#[derive(Clone)]
pub struct AppState {
    pub orchestrator: Arc<Mutex<Orchestrator>>,
    pub client: Client,
}

pub fn build_router(state: AppState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers(Any);

    Router::new()
        .route("/infer", post(infer_handler))
        .route("/status", get(status_handler))
        .layer(TraceLayer::new_for_http())
        .layer(cors)
        .with_state(state)
}

fn pick_request_id(headers: &HeaderMap) -> String {
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
            });
        }
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

    let api = ApiResponse {
        state: format!("{}", orchestrator.current_state),
        model: orchestrator.current_model.clone(),
        override_active: orchestrator.override_until.is_some(),
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
