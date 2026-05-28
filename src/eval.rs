//! Benchmark-only HTTP endpoint: run LLM + tool JSON parse **without** applying drone tools.
//! Enabled with `cargo build --features eval`.

use axum::{
    extract::State,
    http::HeaderMap,
    routing::post,
    Json, Router,
};
use std::time::Instant;

use crate::drone_params::{normalize_drone_tasks, tasks_display_json};
use crate::llm::{normalize_none_reason, LlmToolPayload};
use crate::llm_decision::run_llm_tool_decision;
use crate::server::{pick_request_id, AppState};
use crate::types::ToolCall;
use serde::{Deserialize, Serialize};
use tracing::{info, info_span};

#[derive(Debug, Deserialize)]
pub struct EvalRequest {
    pub prompt: String,
}

#[derive(Debug, Serialize)]
pub struct EvalResponse {
    pub request_id: String,
    pub llm_latency_ms: u64,
    pub e2e_parse_ms: u64,
    pub json_valid: bool,
    pub parse_error: Option<String>,
    pub http_status: Option<u16>,
    pub transport_error: Option<String>,
    pub chat_parse_error: Option<String>,
    /// Assistant `message.content` (may include markdown fences).
    pub raw_llm_content: String,
    /// Serialized `{"tasks":[...]}` from parsed tasks **before** altitude normalization.
    pub llm_tool_json_raw: Option<String>,
    /// After `normalize_drone_tasks` (same as production infer display).
    pub llm_tool_json: Option<String>,
    pub tasks_raw: Option<Vec<ToolCall>>,
    pub tasks: Option<Vec<ToolCall>>,
    pub none_reason: Option<String>,
    pub action_taken: String,
    pub debug_trace: Vec<String>,
}

pub fn eval_router() -> Router<AppState> {
    Router::new().route("/eval", post(eval_handler))
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
    })
}
