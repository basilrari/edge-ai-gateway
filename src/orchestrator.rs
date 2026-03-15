use crate::llm::{parse_tool_call, ChatMessage, ChatRequest, ChatResponse, SAR_SYSTEM_PROMPT};
use crate::types::{GatewayCommand, GatewayState, ToolCall};
use reqwest::Client;
use std::time::{Duration, Instant};
use tracing::{info, warn};

#[derive(Debug)]
pub struct Orchestrator {
    pub current_state: GatewayState,
    pub current_model: Option<String>,
    pub override_until: Option<Instant>,
    /// Last command category ("drone" or "model") from a tool call; None when idle/none.
    pub last_command_category: Option<String>,
    /// Last command name (e.g. "take_off", "activate_human_detection_yolo"); None when idle/none.
    pub last_command_name: Option<String>,
}

impl Orchestrator {
    pub fn new() -> Self {
        Self {
            current_state: GatewayState::IDLE,
            current_model: None,
            override_until: None,
            last_command_category: None,
            last_command_name: None,
        }
    }

    /// Human-readable active command for status: "drone: take_off", "model: vision", or "none".
    pub fn active_command_display(&self) -> String {
        match (&self.last_command_category, &self.last_command_name) {
            (Some(cat), Some(name)) => format!("{}: {}", cat, name),
            _ => "none".to_string(),
        }
    }

    pub fn effective_model_name(&self) -> &str {
        self.current_model.as_deref().unwrap_or("none")
    }

    fn handle_tool_call(
        &mut self,
        tool: ToolCall,
        override_active: bool,
    ) -> (Option<String>, String, Option<String>, Option<String>) {
        let category = Some(tool.category.clone());
        let tool_name = Some(tool.name.clone());

        let mut new_model = self.current_model.clone();
        let action_taken = if override_active && tool.category == "model" {
            let msg = format!("override_active_skip_model_change: {}", tool.name);
            info!(
                action = "override_active_skip_model_change",
                state = ?self.current_state,
                model = %self.effective_model_name(),
                category = %tool.category,
                tool_name = %tool.name,
                reason = "override active; ignoring model tool decision"
            );
            msg
        } else if tool.category == "model" {
            let msg = format!("Python worker will activate: {}", tool.name);
            info!(
                action = "model_tool_selected",
                state = ?self.current_state,
                tool_name = %tool.name,
                reason = "model tool selected; updating model to vision"
            );
            new_model = Some("vision".to_string());
            msg
        } else {
            let msg = format!("Drone command: {}", tool.name);
            info!(
                action = "drone_tool_selected",
                state = ?self.current_state,
                tool_name = %tool.name,
                reason = "drone tool selected; no model change"
            );
            msg
        };

        (new_model, action_taken, category, tool_name)
    }

    pub async fn process_command(
        &mut self,
        cmd: GatewayCommand,
        client: &Client,
    ) -> (
        u64,
        f64,
        u64,
        Option<String>,
        String,
        String,
        Option<String>,
        Option<String>,
        bool,
    ) {
        let start = Instant::now();
        let mut llm_latency_ms: u64 = 0;
        let mut new_state = self.current_state;
        let mut new_model: Option<String> = self.current_model.clone();
        let action_taken: String;
        let mut llm_response = String::new();
        let mut category: Option<String> = None;
        let mut tool_name: Option<String> = None;
        let mut pending_approval = false;

        match cmd {
            GatewayCommand::Infer { prompt } => {
                info!(
                    action = "infer_request",
                    state = ?self.current_state,
                    model = %self.effective_model_name(),
                    prompt_len = prompt.len(),
                    reason = "received infer command"
                );

                let override_active = matches!(self.current_state, GatewayState::OVERRIDE_ACTIVE)
                    && self
                        .override_until
                        .map(|t| t > Instant::now())
                        .unwrap_or(false);

                if override_active {
                    info!(
                        action = "infer_skip_llm_due_to_override",
                        state = ?self.current_state,
                        model = %self.effective_model_name(),
                        reason = "override active; ignoring LLM decision"
                    );
                    new_state = GatewayState::OVERRIDE_ACTIVE;
                    action_taken = "override_active_skip_llm".to_string();
                } else {
                    let request = ChatRequest {
                        model: "qwen".to_string(),
                        messages: vec![
                            ChatMessage {
                                role: "system".to_string(),
                                content: SAR_SYSTEM_PROMPT.to_string(),
                            },
                            ChatMessage {
                                role: "user".to_string(),
                                content: prompt.clone(),
                            },
                        ],
                        temperature: 0.0,
                    };

                    let llm_start = Instant::now();

                    let http_result = client
                        .post("http://localhost:8080/v1/chat/completions")
                        .json(&request)
                        .send()
                        .await;

                    llm_latency_ms = llm_start.elapsed().as_millis() as u64;

                    match http_result {
                        Ok(resp) => {
                            let status = resp.status();
                            let text = resp.text().await.unwrap_or_default();
                            llm_response = text.clone();

                            let parsed_chat: Result<ChatResponse, _> =
                                serde_json::from_str(&text);
                            match parsed_chat {
                                Ok(chat) => {
                                    let content = chat
                                        .choices
                                        .get(0)
                                        .map(|c| c.message.content.clone())
                                        .unwrap_or_default();

                                    match parse_tool_call(&content) {
                                        Ok(tool) => {
                                            if tool.category == "none" {
                                                action_taken = tool.name.clone();
                                                category = Some("none".to_string());
                                                tool_name = Some(tool.name.clone());
                                                new_state = GatewayState::IDLE;
                                                self.last_command_category = None;
                                                self.last_command_name = None;

                                                info!(
                                                    action = "tool_none",
                                                    state = ?new_state,
                                                    name = %tool.name,
                                                    llm_latency_ms,
                                                    http_status = %status,
                                                    reason = "LLM returned category none; no tool activated"
                                                );
                                            } else {
                                                // Proposal only: do not update orchestrator state; frontend will show Accept/Reject.
                                                let (maybe_model, act, cat, tool_n) =
                                                    self.handle_tool_call(tool, override_active);
                                                new_model = maybe_model;
                                                action_taken = act;
                                                category = cat.clone();
                                                tool_name = tool_n.clone();
                                                pending_approval = true;
                                                // Do NOT set self.last_command_* or self.current_model here;
                                                // only ApplyTool (after user accepts) will do that and send to Python.

                                                info!(
                                                    action = "tool_proposal",
                                                    category = ?category,
                                                    tool_name = ?tool_name,
                                                    llm_latency_ms,
                                                    http_status = %status,
                                                    reason = "ToolCall proposal returned to frontend for approval"
                                                );
                                            }
                                        }
                                        Err(e) => {
                                            warn!(
                                                action = "tool_parse_failed",
                                                state = ?self.current_state,
                                                llm_latency_ms,
                                                http_status = %status,
                                                error = %e,
                                                reason = "failed to parse ToolCall JSON; falling back to vision model"
                                            );
                                            self.current_model = Some("vision".to_string());
                                            new_model = self.current_model.clone();
                                            new_state = GatewayState::ACTIVE;
                                            action_taken =
                                                "tool_parse_failed_fallback_vision".to_string();
                                        }
                                    }
                                }
                                Err(e) => {
                                    warn!(
                                        action = "llm_parse_failed",
                                        state = ?self.current_state,
                                        llm_latency_ms,
                                        error = %e,
                                        reason = "failed to parse LLM envelope; falling back to text model"
                                    );
                                    self.current_model = Some("text".to_string());
                                    new_model = self.current_model.clone();
                                    new_state = GatewayState::ACTIVE;
                                    action_taken = "llm_parse_failed_fallback_text".to_string();
                                }
                            }
                        }
                        Err(e) => {
                            warn!(
                                action = "llm_http_failed",
                                state = ?self.current_state,
                                llm_latency_ms,
                                error = %e,
                                reason = "HTTP request to LLM failed; falling back to text model"
                            );
                            self.current_model = Some("text".to_string());
                            new_model = self.current_model.clone();
                            new_state = GatewayState::ACTIVE;
                            action_taken = "llm_http_failed_fallback_text".to_string();
                        }
                    }
                }
            }
            GatewayCommand::ApplyTool {
                category: ref cat,
                tool_name: ref name,
            } => {
                let tool = ToolCall {
                    category: cat.clone(),
                    name: name.clone(),
                };
                let override_active = matches!(self.current_state, GatewayState::OVERRIDE_ACTIVE)
                    && self
                        .override_until
                        .map(|t| t > Instant::now())
                        .unwrap_or(false);
                let (maybe_model, act, _c, _n) = self.handle_tool_call(tool, override_active);
                new_model = maybe_model;
                action_taken = act;
                category = Some(cat.clone());
                tool_name = Some(name.clone());
                self.last_command_category = Some(cat.clone());
                self.last_command_name = Some(name.clone());
                if new_model.is_some() {
                    self.current_model = new_model.clone();
                }
                new_state = GatewayState::ACTIVE;

                // Send to Python server when category is "model" (placeholder: POST to env URL or log).
                if cat == "model" {
                    info!(
                        action = "apply_tool_send_to_python",
                        tool_name = %name,
                        reason = "user accepted; sending to Python server"
                    );
                    // TODO: real gRPC or HTTP call to python-worker when available.
                    // e.g. let _ = client.post(python_url).json(&json!({"tool": name, ...})).send().await;
                }

                info!(
                    action = "apply_tool",
                    state = ?new_state,
                    category = %cat,
                    tool_name = %name,
                    reason = "tool applied after user acceptance"
                );
            }
            GatewayCommand::Override { model, timeout_sec } => {
                info!(
                    action = "override_request",
                    state = ?self.current_state,
                    model = %model,
                    reason = "received override command"
                );

                let timeout = timeout_sec.unwrap_or(60);
                let until = Instant::now() + Duration::from_secs(timeout);
                self.override_until = Some(until);
                self.current_model = Some(model.clone());
                new_model = self.current_model.clone();
                new_state = GatewayState::OVERRIDE_ACTIVE;
                action_taken = "override_set".to_string();

                info!(
                    action = "set_override",
                    state = ?new_state,
                    model = %model,
                    override_timeout_sec = timeout,
                    reason = "activate override model"
                );
            }
            GatewayCommand::ClearOverride => {
                info!(
                    action = "clear_override_request",
                    state = ?self.current_state,
                    model = %self.effective_model_name(),
                    reason = "received clear-override command"
                );

                self.override_until = None;
                self.current_model = None;
                self.last_command_category = None;
                self.last_command_name = None;
                new_model = None;
                new_state = GatewayState::IDLE;
                action_taken = "override_cleared".to_string();

                info!(
                    action = "clear_override",
                    state = ?new_state,
                    model = %self.effective_model_name(),
                    reason = "clear override and return to idle"
                );
            }
            GatewayCommand::Status => {
                info!(
                    action = "status_only",
                    state = ?self.current_state,
                    model = %self.effective_model_name(),
                    reason = "status command; no state change"
                );
                action_taken = "status_only".to_string();
            }
        }

        let latency_ms = start.elapsed().as_millis() as u64;
        let fake_memory_mb = 12.5;
        let previous_state = self.current_state;

        self.current_state = new_state;

        info!(
            action = "state_transition",
            previous_state = ?previous_state,
            state = ?self.current_state,
            model = %self.effective_model_name(),
            latency_ms,
            llm_latency_ms,
            memory_estimate_mb = fake_memory_mb,
            reason = "command processed"
        );

        (
            latency_ms,
            fake_memory_mb,
            llm_latency_ms,
            new_model,
            action_taken,
            llm_response,
            category,
            tool_name,
            pending_approval,
        )
    }
}
