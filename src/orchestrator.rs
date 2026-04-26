use crate::config;
use crate::llm::{parse_tool_call, ChatMessage, ChatRequest, ChatResponse, SAR_SYSTEM_PROMPT};
use crate::types::{CommandOutcome, GatewayCommand, GatewayState, ToolCall};
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

    #[allow(unused_assignments)]
    pub async fn process_command(
        &mut self,
        cmd: GatewayCommand,
        client: &Client,
        request_id: &str,
    ) -> CommandOutcome {
        let start = Instant::now();
        let mut trace = vec![format!("gateway_request_id={request_id}")];
        let mut llm_latency_ms: u64 = 0;
        let mut new_state = self.current_state;
        // Overwritten on every path below before use in CommandOutcome.
        let mut action_taken = String::new();
        let mut llm_response = String::new();
        let mut category: Option<String> = None;
        let mut tool_name: Option<String> = None;
        let mut pending_approval = false;
        let mut drone_http_status: Option<u16> = None;
        let mut drone_http_ms: Option<u64> = None;
        let mut drone_error: Option<String> = None;
        let mut tool_params: Option<serde_json::Value> = None;

        match cmd {
            GatewayCommand::Infer { prompt } => {
                trace.push(format!("command=Infer prompt_len={}", prompt.len()));
                info!(
                    action = "infer_request",
                    request_id = %request_id,
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
                    trace.push("stage=infer_skipped_override_active".into());
                    info!(
                        action = "infer_skip_llm_due_to_override",
                        request_id = %request_id,
                        state = ?self.current_state,
                        model = %self.effective_model_name(),
                        reason = "override active; ignoring LLM decision"
                    );
                    new_state = GatewayState::OVERRIDE_ACTIVE;
                    action_taken = "override_active_skip_llm".to_string();
                    tool_params = None;
                } else {
                    let llm_url = config::llm_chat_completions_url();
                    trace.push(format!("stage=llm_http_post url={llm_url}"));

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
                        .post(&llm_url)
                        .header("x-request-id", request_id)
                        .json(&request)
                        .timeout(Duration::from_secs(120))
                        .send()
                        .await;

                    llm_latency_ms = llm_start.elapsed().as_millis() as u64;
                    trace.push(format!("stage=llm_http_done ms={llm_latency_ms}"));

                    match http_result {
                        Ok(resp) => {
                            let status = resp.status();
                            trace.push(format!(
                                "stage=llm_http_response http_status={}",
                                status.as_u16()
                            ));
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

                                    trace.push(format!(
                                        "stage=llm_content_len chars={}",
                                        content.len()
                                    ));

                                    match parse_tool_call(&content) {
                                        Ok(tool) => {
                                            if tool.category == "none" {
                                                tool_params = None;
                                                action_taken = tool.name.clone();
                                                category = Some("none".to_string());
                                                tool_name = Some(tool.name.clone());
                                                new_state = GatewayState::IDLE;
                                                self.last_command_category = None;
                                                self.last_command_name = None;
                                                trace.push("stage=tool_none".into());

                                                info!(
                                                    action = "tool_none",
                                                    request_id = %request_id,
                                                    state = ?new_state,
                                                    name = %tool.name,
                                                    llm_latency_ms,
                                                    http_status = %status,
                                                    reason = "LLM returned category none; no tool activated"
                                                );
                                            } else {
                                                tool_params = tool.params.clone();
                                                // Proposal only: do not update self.current_model or last_command_*.
                                                let (_, act, cat, tool_n) =
                                                    self.handle_tool_call(tool, override_active);
                                                action_taken = act;
                                                category = cat.clone();
                                                tool_name = tool_n.clone();
                                                pending_approval = true;
                                                trace.push(format!(
                                                    "stage=tool_proposal category={:?} tool={:?}",
                                                    category, tool_name
                                                ));

                                                info!(
                                                    action = "tool_proposal",
                                                    request_id = %request_id,
                                                    category = ?category,
                                                    tool_name = ?tool_name,
                                                    llm_latency_ms,
                                                    http_status = %status,
                                                    reason = "ToolCall proposal returned to frontend for approval"
                                                );
                                            }
                                        }
                                        Err(e) => {
                                            trace.push(format!("stage=tool_json_parse_failed err={e}"));
                                            warn!(
                                                action = "tool_parse_failed",
                                                request_id = %request_id,
                                                state = ?self.current_state,
                                                llm_latency_ms,
                                                http_status = %status,
                                                error = %e,
                                                reason = "failed to parse ToolCall JSON; falling back to vision model"
                                            );
                                            self.current_model = Some("vision".to_string());
                                            new_state = GatewayState::ACTIVE;
                                            action_taken =
                                                "tool_parse_failed_fallback_vision".to_string();
                                        }
                                    }
                                }
                                Err(e) => {
                                    trace.push(format!("stage=llm_envelope_parse_failed err={e}"));
                                    warn!(
                                        action = "llm_parse_failed",
                                        request_id = %request_id,
                                        state = ?self.current_state,
                                        llm_latency_ms,
                                        error = %e,
                                        reason = "failed to parse LLM envelope; falling back to text model"
                                    );
                                    self.current_model = Some("text".to_string());
                                    new_state = GatewayState::ACTIVE;
                                    action_taken = "llm_parse_failed_fallback_text".to_string();
                                }
                            }
                        }
                        Err(e) => {
                            trace.push(format!("stage=llm_http_transport_failed err={e}"));
                            warn!(
                                action = "llm_http_failed",
                                request_id = %request_id,
                                state = ?self.current_state,
                                llm_latency_ms,
                                error = %e,
                                reason = "HTTP request to LLM failed; falling back to text model"
                            );
                            self.current_model = Some("text".to_string());
                            new_state = GatewayState::ACTIVE;
                            action_taken = "llm_http_failed_fallback_text".to_string();
                        }
                    }
                }
            }
            GatewayCommand::ApplyTool {
                category: ref cat,
                tool_name: ref name,
                params: ref apply_params,
            } => {
                tool_params = apply_params.clone();
                trace.push(format!(
                    "command=ApplyTool category={cat} tool={name} has_params={}",
                    apply_params.is_some()
                ));
                let tool = ToolCall {
                    category: cat.clone(),
                    name: name.clone(),
                    params: apply_params.clone(),
                };
                let override_active = matches!(self.current_state, GatewayState::OVERRIDE_ACTIVE)
                    && self
                        .override_until
                        .map(|t| t > Instant::now())
                        .unwrap_or(false);
                let (maybe_model, act, _c, _n) = self.handle_tool_call(tool, override_active);
                if let Some(m) = maybe_model {
                    self.current_model = Some(m);
                }
                action_taken = act;
                category = Some(cat.clone());
                tool_name = Some(name.clone());
                self.last_command_category = Some(cat.clone());
                self.last_command_name = Some(name.clone());
                new_state = GatewayState::ACTIVE;

                if cat == "model" {
                    trace.push("stage=model_apply_placeholder".into());
                    info!(
                        action = "apply_tool_send_to_python",
                        request_id = %request_id,
                        tool_name = %name,
                        reason = "user accepted; model path (python-worker) not wired yet"
                    );
                }

                if cat == "drone" {
                    let url = config::drone_apply_tool_url();
                    trace.push(format!("stage=drone_http_begin url={url}"));
                    let t0 = Instant::now();
                    let params_json = match apply_params.as_ref() {
                        None | Some(serde_json::Value::Null) => serde_json::json!({}),
                        Some(v) if v.is_object() => v.clone(),
                        Some(_) => serde_json::json!({}),
                    };
                    let body = serde_json::json!({ "tool": name, "params": params_json });
                    let send_result = client
                        .post(&url)
                        .header("x-request-id", request_id)
                        .json(&body)
                        .timeout(Duration::from_secs(30))
                        .send()
                        .await;
                    let elapsed_ms = t0.elapsed().as_millis() as u64;
                    drone_http_ms = Some(elapsed_ms);

                    match send_result {
                        Ok(resp) => {
                            drone_http_status = Some(resp.status().as_u16());
                            let status_u = resp.status();
                            let body_text = resp.text().await.unwrap_or_default();
                            trace.push(format!(
                                "stage=drone_http_response status={} body_len={}",
                                status_u.as_u16(),
                                body_text.len()
                            ));

                            let parsed: Result<serde_json::Value, _> =
                                serde_json::from_str(&body_text);
                            match parsed {
                                Ok(v) => {
                                    let ok = v.get("ok").and_then(|x| x.as_bool()).unwrap_or(false);
                                    if ok {
                                        trace.push("stage=drone_mavlink_ok".into());
                                        action_taken =
                                            format!("drone_http_ok:{} ms={}", name, elapsed_ms);
                                    } else {
                                        let err = v
                                            .get("error")
                                            .and_then(|x| x.as_str())
                                            .unwrap_or("ok=false without error field")
                                            .to_string();
                                        drone_error = Some(format!(
                                            "drone_server_http={} error={}",
                                            status_u.as_u16(),
                                            err
                                        ));
                                        trace.push(format!("stage=drone_mavlink_rejected {err}"));
                                        action_taken = format!(
                                            "drone_http_rejected:{} http={}",
                                            name,
                                            status_u.as_u16()
                                        );
                                    }
                                }
                                Err(e) => {
                                    drone_error = Some(format!(
                                        "drone_server_bad_json http={} err={e} body_prefix={}",
                                        status_u.as_u16(),
                                        body_text.chars().take(200).collect::<String>()
                                    ));
                                    trace.push("stage=drone_response_not_json".into());
                                    action_taken =
                                        format!("drone_http_bad_json:{} ", name);
                                }
                            }
                        }
                        Err(e) => {
                            drone_error = Some(format!("drone_http_transport: {e}"));
                            trace.push(format!("stage=drone_http_transport_err {e}"));
                            action_taken = format!("drone_http_transport_failed:{name}");
                        }
                    }
                }

                info!(
                    action = "apply_tool",
                    request_id = %request_id,
                    state = ?new_state,
                    category = %cat,
                    tool_name = %name,
                    reason = "tool applied after user acceptance"
                );
            }
            GatewayCommand::Override { model, timeout_sec } => {
                trace.push("command=Override".into());
                info!(
                    action = "override_request",
                    request_id = %request_id,
                    state = ?self.current_state,
                    model = %model,
                    reason = "received override command"
                );

                let timeout = timeout_sec.unwrap_or(60);
                let until = Instant::now() + Duration::from_secs(timeout);
                self.override_until = Some(until);
                self.current_model = Some(model.clone());
                new_state = GatewayState::OVERRIDE_ACTIVE;
                action_taken = "override_set".to_string();

                info!(
                    action = "set_override",
                    request_id = %request_id,
                    state = ?new_state,
                    model = %model,
                    override_timeout_sec = timeout,
                    reason = "activate override model"
                );
            }
            GatewayCommand::ClearOverride => {
                trace.push("command=ClearOverride".into());
                info!(
                    action = "clear_override_request",
                    request_id = %request_id,
                    state = ?self.current_state,
                    model = %self.effective_model_name(),
                    reason = "received clear-override command"
                );

                self.override_until = None;
                self.current_model = None;
                self.last_command_category = None;
                self.last_command_name = None;
                new_state = GatewayState::IDLE;
                action_taken = "override_cleared".to_string();

                info!(
                    action = "clear_override",
                    request_id = %request_id,
                    state = ?new_state,
                    model = %self.effective_model_name(),
                    reason = "clear override and return to idle"
                );
            }
            GatewayCommand::Status => {
                trace.push("command=Status".into());
                info!(
                    action = "status_only",
                    request_id = %request_id,
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

        trace.push(format!(
            "stage=done state={:?} latency_ms={latency_ms}",
            self.current_state
        ));

        info!(
            action = "state_transition",
            request_id = %request_id,
            previous_state = ?previous_state,
            state = ?self.current_state,
            model = %self.effective_model_name(),
            latency_ms,
            llm_latency_ms,
            memory_estimate_mb = fake_memory_mb,
            reason = "command processed"
        );

        CommandOutcome {
            latency_ms,
            memory_estimate_mb: fake_memory_mb,
            llm_latency_ms,
            action_taken,
            llm_response,
            category,
            tool_name,
            pending_approval,
            drone_http_status,
            drone_http_ms,
            drone_error,
            trace,
            tool_params,
        }
    }
}
