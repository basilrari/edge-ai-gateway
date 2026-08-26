//! Environment-driven URLs for LLM (OpenAI-compatible) and drone HTTP service.

/// Base URL for the OpenAI-compatible server (no trailing path required).
/// Example: `http://127.0.0.1:8080` or `http://127.0.0.1:8080/v1`.
pub fn llm_base_url() -> String {
    std::env::var("LLM_BASE_URL").unwrap_or_else(|_| "http://127.0.0.1:8080".to_string())
}

/// Full URL for `POST .../v1/chat/completions`.
pub fn llm_chat_completions_url() -> String {
    let base = llm_base_url();
    let base = base.trim_end_matches('/');
    if base.ends_with("/v1/chat/completions") {
        base.to_string()
    } else if base.ends_with("/chat/completions") {
        base.to_string()
    } else if base.ends_with("/v1") {
        format!("{base}/chat/completions")
    } else {
        format!("{base}/v1/chat/completions")
    }
}

/// OpenAI `model` field for chat completions (must match llama-server `--alias`).
pub fn llm_chat_model() -> String {
    std::env::var("LLM_CHAT_MODEL").unwrap_or_else(|_| "qwen".to_string())
}

/// Base URL for `drone-http` (see `drone-server` binary). Default loopback on Jetson.
pub fn drone_server_base_url() -> String {
    std::env::var("DRONE_SERVER_URL").unwrap_or_else(|_| "http://127.0.0.1:3001".to_string())
}

pub fn drone_apply_tool_url() -> String {
    let base = drone_server_base_url().trim_end_matches('/').to_string();
    format!("{base}/v1/apply-tool")
}

pub fn drone_health_url() -> String {
    let base = drone_server_base_url().trim_end_matches('/').to_string();
    format!("{base}/health")
}

/// Latest GLOBAL_POSITION_INT from drone-http (for map / UI).
pub fn drone_position_url() -> String {
    let base = drone_server_base_url().trim_end_matches('/').to_string();
    format!("{base}/v1/position")
}

pub fn drone_telemetry_url() -> String {
    let base = drone_server_base_url().trim_end_matches('/').to_string();
    format!("{base}/v1/telemetry")
}

pub fn drone_mission_url() -> String {
    let base = drone_server_base_url().trim_end_matches('/').to_string();
    format!("{base}/v1/mission")
}

pub fn drone_mission_upload_url() -> String {
    let base = drone_server_base_url().trim_end_matches('/').to_string();
    format!("{base}/v1/mission/upload")
}

pub fn drone_mission_clear_url() -> String {
    let base = drone_server_base_url().trim_end_matches('/').to_string();
    format!("{base}/v1/mission/clear")
}

pub fn drone_logs_url() -> String {
    let base = drone_server_base_url().trim_end_matches('/').to_string();
    format!("{base}/v1/logs")
}

/// WebSocket logs stream on drone-http (gateway relays at `/drone/logs/ws`).
pub fn drone_logs_ws_url() -> String {
    let base = drone_server_base_url();
    let base = base.trim_end_matches('/');
    let ws_base = base
        .replacen("https://", "wss://", 1)
        .replacen("http://", "ws://", 1);
    format!("{ws_base}/v1/ws/logs")
}

pub fn drone_mavlink_logs_url() -> String {
    let base = drone_server_base_url().trim_end_matches('/').to_string();
    format!("{base}/v1/logs/mavlink")
}

pub fn drone_logs_clear_url() -> String {
    let base = drone_server_base_url().trim_end_matches('/').to_string();
    format!("{base}/v1/logs/clear")
}

/// WebSocket telemetry stream on drone-http (gateway relays at `/drone/ws`).
pub fn drone_telemetry_ws_url() -> String {
    let base = drone_server_base_url();
    let base = base.trim_end_matches('/');
    let ws_base = base
        .replacen("https://", "wss://", 1)
        .replacen("http://", "ws://", 1);
    format!("{ws_base}/v1/ws/telemetry")
}

/// Local edge-ai-MCP SSE server (not exposed publicly except via authenticated gateway proxy).
pub fn mcp_sse_base_url() -> String {
    std::env::var("MCP_SSE_URL").unwrap_or_else(|_| "http://127.0.0.1:8765".to_string())
}

/// Bearer / X-API-Key for `GET/POST /mcp/*`. When unset, public MCP routes return 503.
pub fn mcp_api_key() -> Option<String> {
    std::env::var("MCP_API_KEY").ok().filter(|s| !s.is_empty())
}

/// Model server base URL (Drone_LLM FastAPI). Default loopback :8000.
pub fn model_server_base_url() -> String {
    std::env::var("MODEL_SERVER_URL").unwrap_or_else(|_| "http://127.0.0.1:8000".to_string())
}

pub fn model_apply_tool_url() -> String {
    let base = model_server_base_url().trim_end_matches('/').to_string();
    format!("{base}/tool")
}

/// Camera / WebRTC signaling on Drone_LLM (defaults to model server URL).
pub fn camera_server_base_url() -> String {
    std::env::var("CAMERA_SERVER_URL")
        .or_else(|_| std::env::var("MODEL_SERVER_URL"))
        .unwrap_or_else(|_| "http://127.0.0.1:8000".to_string())
}

/// WebRTC signaling: browser POSTs SDP offer; camera server returns answer.
pub fn camera_webrtc_offer_url() -> String {
    format!(
        "{}/camera/webrtc/offer",
        camera_server_base_url().trim_end_matches('/')
    )
}

/// Optional client dispatch time (ms since epoch); correlation only.
pub fn client_dispatch_ms_from_headers(headers: &axum::http::HeaderMap) -> Option<u64> {
    headers
        .get("x-client-dispatch-ms")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.parse().ok())
}

/// When true, infer/apply waits for FC `COMMAND_ACK` on each drone step (via drone-http).
pub fn drone_wait_for_ack_default() -> bool {
    std::env::var("DRONE_WAIT_FOR_ACK")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

pub fn drone_ack_timeout_ms_default() -> u64 {
    std::env::var("DRONE_ACK_TIMEOUT_MS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3000)
}

/// Allowed drone base URL for eval E2E (comma-separated prefixes). Default: loopback only.
pub fn eval_sitl_drone_url_prefixes() -> Vec<String> {
    std::env::var("EVAL_SITL_DRONE_URL_PREFIXES")
        .unwrap_or_else(|_| "http://127.0.0.1:3001,http://localhost:3001".to_string())
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

pub fn eval_sitl_safety_token() -> Option<String> {
    std::env::var("EVAL_SITL_TOKEN").ok().filter(|s| !s.is_empty())
}

pub fn eval_sitl_drone_url_allowed() -> bool {
    let base = drone_server_base_url();
    eval_sitl_drone_url_prefixes()
        .iter()
        .any(|p| base.starts_with(p))
}

/// MJPEG stream on model-server (gateway relays at `/camera/stream`).
pub fn camera_stream_url() -> String {
    let base = model_server_base_url().trim_end_matches('/').to_string();
    std::env::var("CAMERA_STREAM_PATH").map_or_else(
        |_| format!("{base}/video/stream"),
        |path| {
            if path.starts_with("http://") || path.starts_with("https://") {
                path
            } else {
                format!("{base}/{}", path.trim_start_matches('/'))
            }
        },
    )
}
