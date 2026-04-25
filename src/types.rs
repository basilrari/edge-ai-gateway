use std::fmt;

#[allow(non_camel_case_types)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayState {
    IDLE,
    ACTIVE,
    OVERRIDE_ACTIVE,
    #[allow(dead_code)]
    SWITCHING,
}

impl fmt::Display for GatewayState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GatewayState::IDLE => write!(f, "IDLE"),
            GatewayState::ACTIVE => write!(f, "ACTIVE"),
            GatewayState::OVERRIDE_ACTIVE => write!(f, "OVERRIDE_ACTIVE"),
            GatewayState::SWITCHING => write!(f, "SWITCHING"),
        }
    }
}

#[derive(serde::Deserialize, Debug)]
pub struct ToolCall {
    pub category: String,
    pub name: String,
}

#[derive(Debug, serde::Deserialize)]
pub enum GatewayCommand {
    Infer { prompt: String },
    /// Apply a previously proposed tool (after user accepts on frontend). Updates state and sends to Python.
    ApplyTool {
        category: String,
        tool_name: String,
    },
    Override { model: String, timeout_sec: Option<u64> },
    ClearOverride,
    Status,
}

#[derive(serde::Serialize)]
pub struct ApiResponse {
    pub state: String,
    pub model: Option<String>,
    pub override_active: bool,
    pub category: Option<String>,
    pub tool_name: Option<String>,
    /// When true, this response is a proposal: frontend should show Accept/Reject; only ApplyTool sends to Python.
    pub pending_approval: bool,
    pub llm_response: String,
    pub action_taken: String,
    pub latency_ms: u64,
    pub llm_latency_ms: u64,
    /// Correlates gateway logs with `drone-http` (`x-request-id`) and browser devtools.
    #[serde(default)]
    pub request_id: String,
    /// Ordered pipeline stages for debugging (no secrets).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub debug_trace: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drone_http_status: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drone_http_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drone_error: Option<String>,
}

/// Result of [`crate::orchestrator::Orchestrator::process_command`].
#[derive(Debug)]
pub struct CommandOutcome {
    pub latency_ms: u64,
    pub memory_estimate_mb: f64,
    pub llm_latency_ms: u64,
    pub action_taken: String,
    pub llm_response: String,
    pub category: Option<String>,
    pub tool_name: Option<String>,
    pub pending_approval: bool,
    pub drone_http_status: Option<u16>,
    pub drone_http_ms: Option<u64>,
    pub drone_error: Option<String>,
    pub trace: Vec<String>,
}
