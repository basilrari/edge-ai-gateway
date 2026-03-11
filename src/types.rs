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
    pub llm_response: String,
    pub action_taken: String,
    pub latency_ms: u64,
    pub llm_latency_ms: u64,
}
