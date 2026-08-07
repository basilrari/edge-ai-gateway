//! Shared LLM call + tool JSON parse (used by `/infer` and optional `/eval`).

use crate::config;
use crate::llm::{
    parse_tool_sequence, ChatMessage, ChatRequest, ChatResponse, LlmToolPayload, SAR_SYSTEM_PROMPT,
};
use reqwest::Client;
use std::time::{Duration, Instant};

/// Outcome of one LLM round-trip and tool JSON parse (no drone apply).
#[derive(Debug)]
pub struct LlmToolDecisionOutcome {
    /// Full LLM HTTP round-trip including response body read.
    pub llm_latency_ms: u64,
    pub llm_http_ms: u64,
    pub llm_parse_ms: u64,
    /// Set when `reqwest` failed before a response body was read.
    pub transport_error: Option<String>,
    pub http_status: Option<u16>,
    /// Raw HTTP body from the chat completions endpoint (OpenAI-style envelope).
    pub llm_envelope_raw: String,
    /// Assistant `message.content` when the envelope parsed successfully.
    pub assistant_content: String,
    /// Set when the HTTP body is not a valid `ChatResponse` JSON.
    pub chat_parse_error: Option<String>,
    /// Tool parse result; `None` only when transport failed or chat envelope did not parse.
    pub tool_payload: Option<Result<LlmToolPayload, serde_json::Error>>,
}

/// POST to LLM, parse chat envelope, then `parse_tool_sequence` on assistant content.
/// Pushes the same `stage=*` trace lines as the historical `/infer` path.
pub async fn run_llm_tool_decision(
    client: &Client,
    prompt: &str,
    request_id: &str,
    trace: &mut Vec<String>,
) -> LlmToolDecisionOutcome {
    let llm_url = config::llm_chat_completions_url();
    trace.push(format!("stage=llm_http_post url={llm_url}"));

    let request = ChatRequest {
        model: config::llm_chat_model(),
        messages: vec![
            ChatMessage {
                role: "system".to_string(),
                content: SAR_SYSTEM_PROMPT.to_string(),
            },
            ChatMessage {
                role: "user".to_string(),
                content: prompt.to_string(),
            },
        ],
        temperature: 0.0,
    };

    let llm_start = Instant::now();
    let http_result = client
        .post(&llm_url)
        .header("x-request-id", request_id)
        .json(&request)
        .timeout(Duration::from_secs(120))
        .send()
        .await;

    let mut transport_error = None;
    let mut http_status = None;
    let mut llm_envelope_raw = String::new();
    let mut assistant_content = String::new();
    let mut chat_parse_error = None;
    let mut tool_payload = None;
    let mut llm_http_ms = 0u64;
    let mut llm_parse_ms = 0u64;

    match http_result {
        Err(e) => {
            trace.push(format!("stage=llm_http_transport_failed err={e}"));
            transport_error = Some(e.to_string());
        }
        Ok(resp) => {
            let status = resp.status();
            http_status = Some(status.as_u16());
            trace.push(format!(
                "stage=llm_http_response http_status={}",
                status.as_u16()
            ));
            let text = resp.text().await.unwrap_or_default();
            llm_http_ms = llm_start.elapsed().as_millis() as u64;
            trace.push(format!("stage=llm_body_read_done ms={llm_http_ms}"));
            llm_envelope_raw = text.clone();

            let parse_start = Instant::now();
            let parsed_chat: Result<ChatResponse, _> = serde_json::from_str(&text);
            match parsed_chat {
                Ok(chat) => {
                    assistant_content = chat
                        .choices
                        .get(0)
                        .map(|c| c.message.content.clone())
                        .unwrap_or_default();

                    trace.push(format!(
                        "stage=llm_content_len chars={}",
                        assistant_content.len()
                    ));

                    tool_payload = Some(parse_tool_sequence(&assistant_content));
                }
                Err(e) => {
                    trace.push(format!("stage=llm_envelope_parse_failed err={e}"));
                    chat_parse_error = Some(e.to_string());
                }
            }
            llm_parse_ms = parse_start.elapsed().as_millis() as u64;
            trace.push(format!("stage=llm_tool_parse_done ms={llm_parse_ms}"));
        }
    }

    let llm_latency_ms = llm_start.elapsed().as_millis() as u64;
    trace.push(format!("stage=llm_http_done ms={llm_latency_ms}"));

    LlmToolDecisionOutcome {
        llm_latency_ms,
        llm_http_ms,
        llm_parse_ms,
        transport_error,
        http_status,
        llm_envelope_raw,
        assistant_content,
        chat_parse_error,
        tool_payload,
    }
}
