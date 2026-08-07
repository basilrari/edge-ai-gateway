use std::fmt;

#[allow(non_camel_case_types)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayState {
    IDLE,
    ACTIVE,
    OVERRIDE_ACTIVE,
}

impl fmt::Display for GatewayState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GatewayState::IDLE => write!(f, "IDLE"),
            GatewayState::ACTIVE => write!(f, "ACTIVE"),
            GatewayState::OVERRIDE_ACTIVE => write!(f, "OVERRIDE_ACTIVE"),
        }
    }
}

#[derive(serde::Serialize, Debug, Clone, Default)]
pub struct PipelineTiming {
    pub gateway_received_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gateway_response_ms: Option<u64>,
    pub queue_wait_ms: u64,
    pub handler_total_ms: u64,
    pub llm_http_ms: u64,
    pub llm_parse_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub apply_total_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_to_final_ack_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_dispatch_ms: Option<u64>,
}

#[derive(serde::Serialize, Debug, Clone)]
pub struct DroneStepTiming {
    pub step_index: usize,
    pub tool: String,
    pub step_id: String,
    pub drone_http_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dispatch_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ack_wait_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ack_result: Option<String>,
    pub http_status: u16,
    pub ok: bool,
}

#[derive(serde::Serialize, Debug, Clone)]
pub struct ModelStepTiming {
    pub step_index: usize,
    pub tool: String,
    pub placeholder: bool,
}

/// Per-request options for orchestrator (infer / eval E2E).
#[derive(Debug, Clone, Copy)]
pub struct ProcessOptions {
    pub wait_for_drone_ack: bool,
    pub ack_timeout_ms: u64,
}

impl Default for ProcessOptions {
    fn default() -> Self {
        Self {
            wait_for_drone_ack: crate::config::drone_wait_for_ack_default(),
            ack_timeout_ms: crate::config::drone_ack_timeout_ms_default(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct HandlerTimingInput {
    pub gateway_received_ms: u64,
    pub queue_wait_ms: u64,
    pub client_dispatch_ms: Option<u64>,
}

#[derive(serde::Deserialize, serde::Serialize, Debug, Clone)]
pub struct ToolCall {
    pub category: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

#[derive(Debug, serde::Deserialize)]
pub enum GatewayCommand {
    Infer { prompt: String },
    /// Apply a previously proposed tool (after user accepts on frontend). Updates state and sends to Python.
    ApplyTool {
        category: String,
        tool_name: String,
        /// Forwarded to `drone-http` for MAVLink tools that need extra fields (`seq`, `lat_deg`, …).
        #[serde(default)]
        params: Option<serde_json::Value>,
    },
    /// Apply an ordered list of tools after one operator approval (multi-step LLM proposal).
    ApplyToolSequence {
        tools: Vec<ToolCall>,
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
    /// When true, this response is a proposal: frontend should show Accept/Reject; apply with **ApplyTool** (one step) or **ApplyToolSequence** (multi-step).
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
    /// When `pending_approval` is true, optional structured args for the next `ApplyTool` (from LLM).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_params: Option<serde_json::Value>,
    /// Ordered tasks from the LLM (executed immediately after infer).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolCall>>,
    /// Parsed tool JSON from the LLM assistant message (`{"tasks":[...]}` only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm_tool_json: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pipeline: Option<PipelineTiming>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub drone_steps: Vec<DroneStepTiming>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub model_steps: Vec<ModelStepTiming>,
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
    pub tool_params: Option<serde_json::Value>,
    pub tools: Option<Vec<ToolCall>>,
    pub llm_tool_json: Option<String>,
    pub pipeline: Option<PipelineTiming>,
    pub drone_steps: Vec<DroneStepTiming>,
    pub model_steps: Vec<ModelStepTiming>,
    pub llm_http_ms: u64,
    pub llm_parse_ms: u64,
    pub apply_total_ms: Option<u64>,
}
