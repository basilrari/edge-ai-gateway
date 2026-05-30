//! Ring buffer of recent LLM infer outputs for the Flight Logs dashboard.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_ENTRIES: usize = 100;
/// Infer log entries older than this are dropped from memory (24 hours).
const LOG_RETENTION_MS: u64 = 24 * 60 * 60 * 1000;

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
            prune_expired(&mut q);
            while q.len() > MAX_ENTRIES {
                q.pop_front();
            }
        }
    }

    pub fn snapshot(&self) -> Vec<LlmLogEntry> {
        self.0
            .lock()
            .map(|mut q| {
                prune_expired(&mut q);
                q.iter().cloned().collect()
            })
            .unwrap_or_default()
    }

    pub fn clear(&self) {
        if let Ok(mut q) = self.0.lock() {
            q.clear();
        }
    }
}

fn retention_cutoff_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
        .saturating_sub(LOG_RETENTION_MS)
}

fn prune_expired(q: &mut VecDeque<LlmLogEntry>) {
    let cutoff = retention_cutoff_ms();
    while q.front().is_some_and(|e| e.ts_ms < cutoff) {
        q.pop_front();
    }
}
