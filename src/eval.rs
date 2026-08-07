//! Benchmark-only HTTP endpoints: LLM decision (`/eval`) and SITL E2E (`/eval/e2e`).
//! Enabled with `cargo build --features eval`.

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::post,
    Json, Router,
};
use std::time::Instant;

use crate::config;
use crate::drone_params::{normalize_drone_tasks, tasks_display_json};
use crate::llm::{normalize_none_reason, LlmToolPayload};
use crate::llm_decision::run_llm_tool_decision;
use crate::server::{pick_request_id, AppState};
use crate::timing::unix_ms_now;
use crate::types::{
    GatewayCommand, HandlerTimingInput, PipelineTiming, ProcessOptions, ToolCall,
};
use serde::{Deserialize, Serialize};
use tracing::{info, info_span, warn};

#[derive(Debug, Deserialize)]
pub struct EvalRequest {
    pub prompt: String,
}

#[derive(Debug, Deserialize)]
pub struct EvalE2eRequest {
    pub prompt: String,
    pub target: String,
    pub wait_for: String,
    #[serde(default)]
    pub ack_timeout_ms: Option<u64>,
    pub safety_token: String,
}

#[derive(Debug, Serialize)]
pub struct EvalResponse {
    pub request_id: String,
    pub llm_latency_ms: u64,
    pub llm_http_ms: u64,
    pub llm_parse_ms: u64,
    pub e2e_parse_ms: u64,
    pub json_valid: bool,
    pub parse_error: Option<String>,
    pub http_status: Option<u16>,
    pub transport_error: Option<String>,
    pub chat_parse_error: Option<String>,
    pub raw_llm_content: String,
    pub llm_tool_json_raw: Option<String>,
    pub llm_tool_json: Option<String>,
    pub tasks_raw: Option<Vec<ToolCall>>,
    pub tasks: Option<Vec<ToolCall>>,
    pub none_reason: Option<String>,
    pub action_taken: String,
    pub debug_trace: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pipeline: Option<PipelineTiming>,
}

#[derive(Debug, Serialize)]
pub struct EvalE2eResponse {
    pub request_id: String,
    pub action_taken: String,
    pub latency_ms: u64,
    pub llm_latency_ms: u64,
    pub pipeline: Option<PipelineTiming>,
    pub drone_steps: Vec<crate::types::DroneStepTiming>,
    pub model_steps: Vec<crate::types::ModelStepTiming>,
    pub drone_error: Option<String>,
    pub tools: Option<Vec<ToolCall>>,
    pub llm_tool_json: Option<String>,
    pub debug_trace: Vec<String>,
}

pub fn eval_router() -> Router<AppState> {
    Router::new()
        .route("/eval", post(eval_handler))
        .route("/eval/e2e", post(eval_e2e_handler))
}

pub async fn eval_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<EvalRequest>,
) -> Json<EvalResponse> {
    let request_id = pick_request_id(&headers);
    let span = info_span!("http_eval", request_id = %request_id);
    let _g = span.enter();

    let t0 = Instant::now();
    let mut trace = vec![format!("gateway_request_id={request_id}")];
    trace.push(format!(
        "command=Eval prompt_len={}",
        body.prompt.len()
    ));

    let dec = run_llm_tool_decision(&state.client, &body.prompt, &request_id, &mut trace).await;
    let e2e_parse_ms = t0.elapsed().as_millis() as u64;

    let raw_llm_content = dec.assistant_content.clone();

    let mut json_valid = false;
    let mut parse_error: Option<String> = None;
    let mut llm_tool_json_raw: Option<String> = None;
    let mut llm_tool_json: Option<String> = None;
    let mut tasks_raw: Option<Vec<ToolCall>> = None;
    let mut tasks: Option<Vec<ToolCall>> = None;
    let mut none_reason: Option<String> = None;

    let action_taken = if dec.transport_error.is_some() {
        parse_error = dec.transport_error.clone();
        "eval_llm_transport_failed".to_string()
    } else if dec.chat_parse_error.is_some() {
        parse_error = dec.chat_parse_error.clone();
        "eval_llm_envelope_parse_failed".to_string()
    } else if let Some(tool_res) = dec.tool_payload {
        match tool_res {
            Ok(LlmToolPayload::NoneReason(reason)) => {
                let reason = normalize_none_reason(&reason);
                json_valid = true;
                none_reason = Some(reason.clone());
                let canon = serde_json::json!({ "tasks": [{ "category": "none", "name": reason }] })
                    .to_string();
                llm_tool_json_raw = Some(canon.clone());
                llm_tool_json = Some(canon);
                "eval_ok_none".to_string()
            }
            Ok(LlmToolPayload::Tasks(mut t)) => {
                if t.is_empty() {
                    json_valid = false;
                    parse_error = Some("empty_tasks_after_parse".into());
                    "eval_tool_parse_failed".to_string()
                } else {
                    json_valid = true;
                    llm_tool_json_raw = Some(tasks_display_json(&t));
                    tasks_raw = Some(t.clone());
                    normalize_drone_tasks(&mut t);
                    llm_tool_json = Some(tasks_display_json(&t));
                    tasks = Some(t);
                    "eval_ok_tasks".to_string()
                }
            }
            Err(e) => {
                parse_error = Some(e.to_string());
                "eval_tool_parse_failed".to_string()
            }
        }
    } else {
        "eval_internal_no_tool_payload".to_string()
    };

    trace.push(format!(
        "stage=eval_done e2e_parse_ms={e2e_parse_ms} json_valid={json_valid}"
    ));

    info!(
        action = "http_eval_done",
        request_id = %request_id,
        llm_latency_ms = dec.llm_latency_ms,
        e2e_parse_ms,
        json_valid,
        action_taken = %action_taken,
        reason = "eval path does not apply drone tools"
    );

    Json(EvalResponse {
        request_id,
        llm_latency_ms: dec.llm_latency_ms,
        llm_http_ms: dec.llm_http_ms,
        llm_parse_ms: dec.llm_parse_ms,
        e2e_parse_ms,
        json_valid,
        parse_error,
        http_status: dec.http_status,
        transport_error: dec.transport_error,
        chat_parse_error: dec.chat_parse_error,
        raw_llm_content,
        llm_tool_json_raw,
        llm_tool_json,
        tasks_raw,
        tasks,
        none_reason,
        action_taken,
        debug_trace: trace,
        pipeline: None,
    })
}

pub async fn eval_e2e_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<EvalE2eRequest>,
) -> (StatusCode, Json<EvalE2eResponse>) {
    let request_id = pick_request_id(&headers);
    let span = info_span!("http_eval_e2e", request_id = %request_id);
    let _g = span.enter();

    if body.target != "sitl" || body.wait_for != "ack" {
        return (
            StatusCode::BAD_REQUEST,
            Json(EvalE2eResponse {
                request_id,
                action_taken: "eval_e2e_invalid_request".into(),
                latency_ms: 0,
                llm_latency_ms: 0,
                pipeline: None,
                drone_steps: vec![],
                model_steps: vec![],
                drone_error: Some("target must be sitl and wait_for must be ack".into()),
                tools: None,
                llm_tool_json: None,
                debug_trace: vec![],
            }),
        );
    }

    let expected_token = config::eval_sitl_safety_token();
    if expected_token.as_deref() != Some(body.safety_token.as_str()) {
        warn!(request_id = %request_id, "eval_e2e rejected: safety token mismatch");
        return (
            StatusCode::FORBIDDEN,
            Json(EvalE2eResponse {
                request_id,
                action_taken: "eval_e2e_forbidden".into(),
                latency_ms: 0,
                llm_latency_ms: 0,
                pipeline: None,
                drone_steps: vec![],
                model_steps: vec![],
                drone_error: Some("EVAL_SITL_TOKEN mismatch or unset on gateway".into()),
                tools: None,
                llm_tool_json: None,
                debug_trace: vec![],
            }),
        );
    }

    if !config::eval_sitl_drone_url_allowed() {
        return (
            StatusCode::FORBIDDEN,
            Json(EvalE2eResponse {
                request_id,
                action_taken: "eval_e2e_drone_url_not_allowed".into(),
                latency_ms: 0,
                llm_latency_ms: 0,
                pipeline: None,
                drone_steps: vec![],
                model_steps: vec![],
                drone_error: Some("DRONE_SERVER_URL not in EVAL_SITL_DRONE_URL_PREFIXES".into()),
                tools: None,
                llm_tool_json: None,
                debug_trace: vec![],
            }),
        );
    }

    let gateway_received_ms = unix_ms_now();
    let lock_start = Instant::now();
    let mut orchestrator = state.orchestrator.lock().await;
    let queue_wait_ms = lock_start.elapsed().as_millis() as u64;

    let options = ProcessOptions {
        wait_for_drone_ack: true,
        ack_timeout_ms: body
            .ack_timeout_ms
            .unwrap_or_else(config::drone_ack_timeout_ms_default),
    };

    let outcome = orchestrator
        .process_command(
            GatewayCommand::Infer {
                prompt: body.prompt.clone(),
            },
            &state.client,
            &request_id,
            Some(HandlerTimingInput {
                gateway_received_ms,
                queue_wait_ms,
                client_dispatch_ms: config::client_dispatch_ms_from_headers(&headers),
            }),
            options,
        )
        .await;

    drop(orchestrator);

    info!(
        action = "http_eval_e2e_done",
        request_id = %request_id,
        latency_ms = outcome.latency_ms,
        reason = "eval e2e ran production infer path with ACK wait"
    );

    (
        StatusCode::OK,
        Json(EvalE2eResponse {
            request_id,
            action_taken: outcome.action_taken,
            latency_ms: outcome.latency_ms,
            llm_latency_ms: outcome.llm_latency_ms,
            pipeline: outcome.pipeline,
            drone_steps: outcome.drone_steps,
            model_steps: outcome.model_steps,
            drone_error: outcome.drone_error,
            tools: outcome.tools,
            llm_tool_json: outcome.llm_tool_json,
            debug_trace: outcome.trace,
        }),
    )
}
