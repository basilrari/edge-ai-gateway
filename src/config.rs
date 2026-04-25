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
