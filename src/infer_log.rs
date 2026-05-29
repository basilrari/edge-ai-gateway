//! Ring buffer of recent LLM infer outputs for the Flight Logs dashboard.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_ENTRIES: usize = 100;

#[derive(Clone, Debug, serde::Serialize)]
pub struct LlmLogEntry {
    pub ts_ms: u64,
    pub prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub llm_tool_json: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action_taken: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub request_id: String,
}

#[derive(Clone, Default)]
pub struct InferLog(Arc<Mutex<VecDeque<LlmLogEntry>>>);

impl InferLog {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(VecDeque::new())))
    }

    pub fn push(
        &self,
        prompt: impl Into<String>,
        llm_tool_json: Option<String>,
        action_taken: Option<String>,
        model: Option<String>,
        request_id: impl Into<String>,
    ) {
        let ts_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let entry = LlmLogEntry {
            ts_ms,
            prompt: prompt.into(),
            llm_tool_json,
            action_taken,
            model,
            request_id: request_id.into(),
        };
        if let Ok(mut q) = self.0.lock() {
            q.push_back(entry);
            while q.len() > MAX_ENTRIES {
                q.pop_front();
            }
        }
    }

    pub fn snapshot(&self) -> Vec<LlmLogEntry> {
        self.0
            .lock()
            .map(|q| q.iter().cloned().collect())
            .unwrap_or_default()
    }
}
