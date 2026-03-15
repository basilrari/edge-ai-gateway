use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    extract::State,
    http::Method,
    routing::{get, post},
    Json, Router,
};
use reqwest::Client;
use tokio::sync::Mutex;
use tracing::{info, info_span, warn};
use tower_http::cors::{Any, CorsLayer};

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
        .layer(cors)
        .with_state(state)
}

async fn infer_handler(
    State(state): State<AppState>,
    Json(payload): Json<serde_json::Value>,
) -> Json<ApiResponse> {
    let cmd: GatewayCommand = match serde_json::from_value(payload.clone()) {
        Ok(cmd) => cmd,
        Err(e) => {
            warn!(
                action = "infer_parse_failed",
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
            });
        }
    };

    let span = info_span!("gateway_command", action = "process_command_http");
    let _guard = span.enter();

    let mut orchestrator = state.orchestrator.lock().await;
    info!(
        action = "http_infer_received",
        state = ?orchestrator.current_state,
        model = %orchestrator.effective_model_name(),
        reason = "processing /infer request"
    );

    let (
        latency_ms,
        _memory_mb,
        llm_latency_ms,
        _new_model,
        action_taken,
        llm_response,
        category,
        tool_name,
        pending_approval,
    ) = orchestrator.process_command(cmd, &state.client).await;

    let api = ApiResponse {
        state: format!("{}", orchestrator.current_state),
        model: orchestrator.current_model.clone(),
        override_active: orchestrator.override_until.is_some(),
        category,
        tool_name,
        pending_approval,
        llm_response,
        action_taken,
        latency_ms,
        llm_latency_ms,
    };

    Json(api)
}

async fn status_handler(State(state): State<AppState>) -> Json<serde_json::Value> {
    let span = info_span!("gateway_command", action = "status_http");
    let _guard = span.enter();

    let mut orchestrator = state.orchestrator.lock().await;

    let (latency_ms, memory_mb, llm_latency_ms, _new_model, _action_taken, _llm, _cat, _tool, _pending) =
        orchestrator.process_command(GatewayCommand::Status, &state.client).await;

    Json(serde_json::json!({
        "state": format!("{}", orchestrator.current_state),
        "model": orchestrator.effective_model_name(),
        "override_active": orchestrator.override_until.is_some(),
        "active_command": orchestrator.active_command_display(),
        "latency_ms": latency_ms,
        "llm_latency_ms": llm_latency_ms,
        "memory_estimate_mb": memory_mb,
    }))
}

pub async fn run_http_server(state: AppState) {
    let app = build_router(state);
    let addr: SocketAddr = "0.0.0.0:3000".parse().expect("invalid listen addr");

    info!(
        action = "http_server_start",
        addr = %addr,
        reason = "starting persistent HTTP server"
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
