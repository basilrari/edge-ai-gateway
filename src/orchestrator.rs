use crate::config;
use crate::drone_params::{normalize_drone_tasks, normalize_drone_tool_params, tasks_display_json};
use crate::llm::{validate_llm_tasks, LlmToolPayload};
use crate::llm_decision::run_llm_tool_decision;
use crate::model_apply::model_apply_via_http;
use crate::timing::{drone_step_id, finalize_pipeline_layers, unix_ms_now};
use crate::types::{
    CommandOutcome, DroneStepTiming, GatewayCommand, GatewayState, HandlerTimingInput,
    ModelStepTiming, PipelineTiming, ProcessOptions, ToolCall,
};
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
    /// Last command name (e.g. "takeoff", "human_detect"); None when idle/none.
    pub last_command_name: Option<String>,
}

struct ApplyOutcome {
    action_taken: String,
    category: Option<String>,
    tool_name: Option<String>,
    state: GatewayState,
    drone_http_status: Option<u16>,
    drone_http_ms: Option<u64>,
    drone_error: Option<String>,
    drone_steps: Vec<DroneStepTiming>,
    model_steps: Vec<ModelStepTiming>,
    apply_total_ms: u64,
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

    fn override_is_live(&self) -> bool {
        matches!(self.current_state, GatewayState::OVERRIDE_ACTIVE)
            && self
                .override_until
                .map(|t| t > Instant::now())
                .unwrap_or(false)
    }

    fn handle_tool_call(
        &mut self,
        tool: &ToolCall,
        override_active: bool,
    ) -> (Option<String>, String) {
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
            let msg = format!("Model server: {}", tool.name);
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

        (new_model, action_taken)
    }

    /// Run an ordered list of validated drone/model tasks (HTTP apply).
    /// `command` is the `/infer` variant that called in (`infer`, `apply_tool`, `apply_tool_sequence`).
    async fn apply_tasks(
        &mut self,
        tools: &[ToolCall],
        client: &Client,
        request_id: &str,
        options: ProcessOptions,
        command: &'static str,
        trace: &mut Vec<String>,
    ) -> ApplyOutcome {
        let override_active = self.override_is_live();
        let apply_start = Instant::now();
        let new_state = GatewayState::ACTIVE;
        let mut last_success: Option<ToolCall> = None;
        let mut stopped_at: Option<usize> = None;
        let mut drone_http_status: Option<u16> = None;
        let mut drone_http_ms: Option<u64> = None;
        let mut drone_error: Option<String> = None;
        let mut drone_steps = Vec::new();
        let mut model_steps = Vec::new();

        for (idx, tool) in tools.iter().enumerate() {
            if tool.category == "drone" {
                let step_id = drone_step_id(request_id, idx);
                let r = drone_apply_via_http(
                    client,
                    request_id,
                    &step_id,
                    &tool.name,
                    &tool.params,
                    options,
                    trace,
                )
                .await;
                drone_http_status = Some(r.http_status);
                drone_http_ms = Some(r.elapsed_ms);
                drone_steps.push(DroneStepTiming {
                    step_index: idx,
                    tool: tool.name.clone(),
                    step_id: step_id.clone(),
                    drone_http_ms: r.elapsed_ms,
                    dispatch_ms: r.dispatch_ms,
                    ack_wait_ms: r.ack_wait_ms,
                    completion_status: r.completion_status.clone(),
                    ack_result: r.ack_result.clone(),
                    http_status: r.http_status,
                    ok: r.mavlink_ok,
                });
                if r.mavlink_ok {
                    last_success = Some(tool.clone());
                    trace.push(format!(
                        "stage=sequence_step_ok idx={} tool={}",
                        idx, tool.name
                    ));
                } else {
                    let err = r
                        .error_detail
                        .unwrap_or_else(|| "drone step failed without error detail".to_string());
                    drone_error = Some(err);
                    stopped_at = Some(idx);
                    trace.push(format!("stage=sequence_stopped idx={idx}"));
                    break;
                }
            } else if tool.category == "model" {
                let (maybe_model, act) = self.handle_tool_call(tool, override_active);
                if let Some(m) = maybe_model {
                    self.current_model = Some(m);
                }
                if override_active {
                    trace.push(format!(
                        "stage=sequence_model_step idx={} action={} skipped=override",
                        idx, act
                    ));
                    model_steps.push(ModelStepTiming {
                        step_index: idx,
                        tool: tool.name.clone(),
                        placeholder: true,
                        elapsed_ms: 0,
                        inference_ms: None,
                        http_status: 0,
                        ok: true,
                    });
                    last_success = Some(tool.clone());
                } else {
                    let r = model_apply_via_http(client, request_id, &tool.name, trace).await;
                    model_steps.push(ModelStepTiming {
                        step_index: idx,
                        tool: tool.name.clone(),
                        placeholder: false,
                        elapsed_ms: r.elapsed_ms,
                        inference_ms: r.inference_ms,
                        http_status: r.http_status,
                        ok: r.ok,
                    });
                    if r.ok {
                        last_success = Some(tool.clone());
                        trace.push(format!(
                            "stage=sequence_step_ok idx={} tool={} ms={}",
                            idx, tool.name, r.elapsed_ms
                        ));
                        info!(
                            action = "apply_sequence_model_step",
                            request_id = %request_id,
                            step = idx,
                            tool_name = %tool.name,
                            elapsed_ms = r.elapsed_ms,
                            inference_ms = ?r.inference_ms,
                            skipped = r.skipped,
                            reason = "model server /tool"
                        );
                    } else {
                        let err = r
                            .error_detail
                            .unwrap_or_else(|| "model step failed".to_string());
                        drone_error =
                            Some(format!("model_server_http={} error={err}", r.http_status));
                        stopped_at = Some(idx);
                        trace.push(format!("stage=sequence_stopped idx={idx} model"));
                        break;
                    }
                }
            } else {
                trace.push(format!(
                    "stage=sequence_skip_unknown_category idx={} cat={}",
                    idx, tool.category
                ));
            }
        }

        if let Some(ref ok_tool) = last_success {
            self.last_command_category = Some(ok_tool.category.clone());
            self.last_command_name = Some(ok_tool.name.clone());
        }

        let category = last_success.as_ref().map(|t| t.category.clone());
        let tool_name = last_success.as_ref().map(|t| t.name.clone());

        let action_taken = if let Some(idx) = stopped_at {
            format!("sequence_stopped_at_step_{idx}_tool_{}", tools[idx].name)
        } else if tools.len() == 1 {
            let t = &tools[0];
            if t.category == "drone" {
                format!("drone_http_ok:{}", t.name)
            } else {
                format!("model_http_ok:{}", t.name)
            }
        } else {
            format!("sequence_ok:{}_steps", tools.len())
        };

        if let Some(idx) = stopped_at {
            let stopped_category = tools[idx].category.as_str();
            info!(
                action = "apply_tasks_partial",
                command,
                request_id = %request_id,
                stopped_at = idx,
                stopped_category,
                reason = "sequence stopped on first failed step"
            );
        } else {
            info!(
                action = "apply_tasks_complete",
                command,
                request_id = %request_id,
                steps = tools.len(),
                reason = "all tasks applied"
            );
        }

        let apply_total_ms = apply_start.elapsed().as_millis() as u64;
        trace.push(format!("stage={command}_apply_done ms={apply_total_ms}"));

        ApplyOutcome {
            action_taken,
            category,
            tool_name,
            state: new_state,
            drone_http_status,
            drone_http_ms,
            drone_error,
            drone_steps,
            model_steps,
            apply_total_ms,
        }
    }

    pub async fn process_command(
        &mut self,
        cmd: GatewayCommand,
        client: &Client,
        request_id: &str,
        handler_timing: Option<HandlerTimingInput>,
        options: ProcessOptions,
    ) -> CommandOutcome {
        let start = Instant::now();
        let mut trace = vec![format!("gateway_request_id={request_id}")];
        let mut llm_latency_ms: u64 = 0;
        let mut llm_http_ms: u64 = 0;
        let mut llm_parse_ms: u64 = 0;
        let mut apply_total_ms: Option<u64> = None;
        let mut drone_steps: Vec<DroneStepTiming> = Vec::new();
        let mut model_steps: Vec<ModelStepTiming> = Vec::new();
        let mut new_state = self.current_state;
        let mut action_taken = String::new();
        let mut llm_response = String::new();
        let mut category: Option<String> = None;
        let mut tool_name: Option<String> = None;
        let mut drone_http_status: Option<u16> = None;
        let mut drone_http_ms: Option<u64> = None;
        let mut drone_error: Option<String> = None;
        let mut tool_params: Option<serde_json::Value> = None;
        let mut tools_proposal: Option<Vec<ToolCall>> = None;
        let mut llm_tool_json: Option<String> = None;
        let mut applied: Option<ApplyOutcome> = None;

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

                let override_active = self.override_is_live();

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
                    let dec = run_llm_tool_decision(client, &prompt, request_id, &mut trace).await;
                    llm_latency_ms = dec.llm_latency_ms;
                    llm_http_ms = dec.llm_http_ms;
                    llm_parse_ms = dec.llm_parse_ms;
                    llm_response = dec.llm_envelope_raw.clone();

                    if dec.transport_error.is_some() {
                        let e = dec.transport_error.as_deref().unwrap_or("");
                        warn!(
                            action = "llm_http_failed",
                            request_id = %request_id,
                            state = ?self.current_state,
                            llm_latency_ms,
                            error = %e,
                            reason = "HTTP request to LLM failed"
                        );
                        action_taken = "llm_http_failed".to_string();
                    } else if dec.chat_parse_error.is_some() {
                        let e = dec.chat_parse_error.as_deref().unwrap_or("");
                        warn!(
                            action = "llm_parse_failed",
                            request_id = %request_id,
                            state = ?self.current_state,
                            llm_latency_ms,
                            error = %e,
                            reason = "failed to parse LLM envelope"
                        );
                        action_taken = "llm_parse_failed".to_string();
                    } else if let Some(tool_res) = dec.tool_payload {
                        let status = dec.http_status.unwrap_or(0);
                        match tool_res {
                            Ok(LlmToolPayload::NoneReason(reason)) => {
                                tool_params = None;
                                tools_proposal = None;
                                action_taken = reason.clone();
                                category = Some("none".to_string());
                                tool_name = Some(reason);
                                new_state = GatewayState::IDLE;
                                self.last_command_category = None;
                                self.last_command_name = None;
                                trace.push("stage=tool_none".into());

                                info!(
                                    action = "tool_none",
                                    request_id = %request_id,
                                    state = ?new_state,
                                    llm_latency_ms,
                                    http_status = %status,
                                    reason = "LLM returned category none; no tool activated"
                                );
                            }
                            Ok(LlmToolPayload::Tasks(mut tasks)) => {
                                normalize_drone_tasks(&mut tasks);
                                trace.push("stage=drone_params_normalized".into());
                                tool_params = tasks.first().and_then(|t| t.params.clone());
                                tools_proposal = Some(tasks.clone());
                                llm_tool_json = Some(tasks_display_json(&tasks));
                                trace.push(format!("stage=infer_auto_apply steps={}", tasks.len()));
                                let a = self
                                    .apply_tasks(
                                        &tasks, client, request_id, options, "infer", &mut trace,
                                    )
                                    .await;
                                info!(
                                    action = "infer_auto_apply",
                                    request_id = %request_id,
                                    steps = tasks.len(),
                                    category = ?a.category,
                                    tool_name = ?a.tool_name,
                                    llm_latency_ms,
                                    http_status = %status,
                                    reason = "LLM tasks applied immediately after infer"
                                );
                                applied = Some(a);
                            }
                            Err(e) => {
                                let preview: String =
                                    dec.assistant_content.chars().take(240).collect();
                                trace.push(format!("stage=tool_json_parse_failed err={e}"));
                                trace.push(format!("stage=llm_content_preview={preview:?}"));
                                llm_tool_json = Some(dec.assistant_content.clone());
                                warn!(
                                    action = "tool_parse_failed",
                                    request_id = %request_id,
                                    llm_latency_ms,
                                    http_status = %status,
                                    error = %e,
                                    reason = "failed to parse ToolCall JSON"
                                );
                                category = Some("none".to_string());
                                tool_name = Some("tool_parse_failed".into());
                                action_taken = format!("tool_parse_failed: {e}");
                            }
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
                match validate_llm_tasks(vec![tool]) {
                    LlmToolPayload::NoneReason(reason) => {
                        action_taken = format!("tool_rejected:{reason}");
                        drone_error = Some(format!("tool_not_allowed:{name}"));
                        category = Some(cat.clone());
                        tool_name = Some(name.clone());
                        trace.push(format!(
                            "stage=apply_tool_rejected name={name} reason={reason}"
                        ));
                    }
                    LlmToolPayload::Tasks(tasks) => {
                        applied = Some(
                            self.apply_tasks(
                                &tasks,
                                client,
                                request_id,
                                options,
                                "apply_tool",
                                &mut trace,
                            )
                            .await,
                        );
                    }
                }
            }
            GatewayCommand::ApplyToolSequence { tools } => {
                trace.push(format!("command=ApplyToolSequence steps={}", tools.len()));

                if tools.is_empty() {
                    action_taken = "apply_sequence_empty".to_string();
                    trace.push("stage=apply_sequence_empty".into());
                    warn!(
                        action = "apply_tool_sequence_empty",
                        request_id = %request_id,
                        reason = "ApplyToolSequence received empty tools"
                    );
                } else {
                    match validate_llm_tasks(tools) {
                        LlmToolPayload::NoneReason(reason) => {
                            action_taken = format!("tool_rejected:{reason}");
                            drone_error = Some(format!("tool_not_allowed:{reason}"));
                            trace.push(format!("stage=apply_sequence_rejected reason={reason}"));
                        }
                        LlmToolPayload::Tasks(tools) => {
                            applied = Some(
                                self.apply_tasks(
                                    &tools,
                                    client,
                                    request_id,
                                    options,
                                    "apply_tool_sequence",
                                    &mut trace,
                                )
                                .await,
                            );
                        }
                    }
                }
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

        if let Some(a) = applied {
            action_taken = a.action_taken;
            category = a.category;
            tool_name = a.tool_name;
            new_state = a.state;
            drone_http_status = a.drone_http_status;
            drone_http_ms = a.drone_http_ms;
            drone_error = a.drone_error;
            drone_steps = a.drone_steps;
            model_steps = a.model_steps;
            apply_total_ms = Some(a.apply_total_ms);
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

        let handler_total_ms = latency_ms;
        let pipeline = handler_timing.map(|h| {
            let prompt_to_final_ack_ms = if options.wait_for_drone_ack && !drone_steps.is_empty() {
                Some(latency_ms)
            } else {
                None
            };
            let mut p = PipelineTiming {
                gateway_received_ms: h.gateway_received_ms,
                gateway_response_ms: Some(unix_ms_now()),
                queue_wait_ms: h.queue_wait_ms,
                handler_total_ms,
                llm_http_ms,
                llm_parse_ms,
                apply_total_ms,
                prompt_to_final_ack_ms,
                client_dispatch_ms: h.client_dispatch_ms,
                llm_ms: 0,
                drone_server_ms: 0,
                drone_ack_wait_ms: 0,
                model_server_ms: 0,
                gateway_ms: 0,
            };
            finalize_pipeline_layers(&mut p, &drone_steps, &model_steps);
            p
        });

        // Structured latency line for log aggregation (see benchmark/run_latency_batch.py).
        let llm_ms = llm_http_ms.saturating_add(llm_parse_ms);
        let tool_ms = apply_total_ms.unwrap_or(0);
        let drone_server_ms = pipeline.as_ref().map(|p| p.drone_server_ms).unwrap_or(0);
        let model_server_ms = pipeline.as_ref().map(|p| p.model_server_ms).unwrap_or(0);
        let gateway_ms = pipeline.as_ref().map(|p| p.gateway_ms).unwrap_or(0);
        info!(
            action = "query_latency",
            request_id = %request_id,
            llm_ms,
            tool_ms,
            drone_server_ms,
            model_server_ms,
            gateway_ms,
            queue_wait_ms = pipeline.as_ref().map(|p| p.queue_wait_ms).unwrap_or(0),
            total_ms = handler_total_ms,
            drone_steps = drone_steps.len(),
            model_steps = model_steps.len(),
            success = drone_error.is_none(),
            action_taken = %action_taken,
        );

        CommandOutcome {
            latency_ms,
            memory_estimate_mb: fake_memory_mb,
            llm_latency_ms,
            action_taken,
            llm_response,
            category,
            tool_name,
            pending_approval: false,
            drone_http_status,
            drone_http_ms,
            drone_error,
            trace,
            tool_params,
            tools: tools_proposal,
            llm_tool_json,
            pipeline,
            drone_steps,
            model_steps,
        }
    }
}

struct DroneApplyResult {
    http_status: u16,
    elapsed_ms: u64,
    mavlink_ok: bool,
    error_detail: Option<String>,
    dispatch_ms: Option<u64>,
    ack_wait_ms: Option<u64>,
    completion_status: Option<String>,
    ack_result: Option<String>,
}

async fn drone_apply_via_http(
    client: &Client,
    request_id: &str,
    step_id: &str,
    name: &str,
    apply_params: &Option<serde_json::Value>,
    options: ProcessOptions,
    trace: &mut Vec<String>,
) -> DroneApplyResult {
    let url = config::drone_apply_tool_url();
    trace.push(format!("stage=drone_http_begin tool={name} step_id={step_id} url={url}"));
    let t0 = Instant::now();
    let normalized = normalize_drone_tool_params(name, apply_params.clone());
    let params_json = match normalized.as_ref() {
        None | Some(serde_json::Value::Null) => serde_json::json!({}),
        Some(v) if v.is_object() => v.clone(),
        Some(_) => serde_json::json!({}),
    };
    let mut body = serde_json::json!({
        "tool": name,
        "params": params_json,
        "step_id": step_id,
        "wait_for": if options.wait_for_drone_ack { "ack" } else { "none" },
    });
    if options.wait_for_drone_ack {
        body["ack_timeout_ms"] = serde_json::json!(options.ack_timeout_ms);
    }
    let mut req = client.post(&url).header("x-request-id", request_id);
    if let Some(key) = config::mcp_api_key() {
        req = req
            .header("authorization", format!("Bearer {key}"))
            .header("x-api-key", key);
    }
    let send_result = req
        .json(&body)
        .timeout(Duration::from_secs(
            if options.wait_for_drone_ack {
                30 + options.ack_timeout_ms / 1000
            } else {
                30
            },
        ))
        .send()
        .await;
    let elapsed_ms = t0.elapsed().as_millis() as u64;

    match send_result {
        Ok(resp) => {
            let status_u = resp.status();
            let status_code = status_u.as_u16();
            let body_text = resp.text().await.unwrap_or_default();
            trace.push(format!(
                "stage=drone_http_response tool={name} status={} body_len={}",
                status_code,
                body_text.len()
            ));

            let parsed: Result<serde_json::Value, _> = serde_json::from_str(&body_text);
            match parsed {
                Ok(v) => {
                    let ok = v.get("ok").and_then(|x| x.as_bool()).unwrap_or(false);
                    let dispatch_ms = v.get("dispatch_ms").and_then(|x| x.as_u64());
                    let ack_wait_ms = v.get("ack_wait_ms").and_then(|x| x.as_u64());
                    let completion_status = v
                        .get("completion_status")
                        .and_then(|x| x.as_str())
                        .map(str::to_string);
                    let ack_result = v
                        .get("ack_result")
                        .and_then(|x| x.as_str())
                        .map(str::to_string);
                    if ok {
                        DroneApplyResult {
                            http_status: status_code,
                            elapsed_ms,
                            mavlink_ok: true,
                            error_detail: None,
                            dispatch_ms,
                            ack_wait_ms,
                            completion_status,
                            ack_result,
                        }
                    } else {
                        let err = v
                            .get("error")
                            .and_then(|x| x.as_str())
                            .unwrap_or("ok=false without error field")
                            .to_string();
                        DroneApplyResult {
                            http_status: status_code,
                            elapsed_ms,
                            mavlink_ok: false,
                            error_detail: Some(format!(
                                "drone_server_http={} error={}",
                                status_code, err
                            )),
                            dispatch_ms,
                            ack_wait_ms,
                            completion_status,
                            ack_result,
                        }
                    }
                }
                Err(e) => DroneApplyResult {
                    http_status: status_code,
                    elapsed_ms,
                    mavlink_ok: false,
                    error_detail: Some(format!(
                        "drone_server_bad_json http={} err={e} body_prefix={}",
                        status_code,
                        body_text.chars().take(200).collect::<String>()
                    )),
                    dispatch_ms: None,
                    ack_wait_ms: None,
                    completion_status: None,
                    ack_result: None,
                },
            }
        }
        Err(e) => DroneApplyResult {
            http_status: 0,
            elapsed_ms,
            mavlink_ok: false,
            error_detail: Some(format!("drone_http_transport: {e}")),
            dispatch_ms: None,
            ack_wait_ms: None,
            completion_status: None,
            ack_result: None,
        },
    }
}

#[cfg(test)]
mod apply_tool_reject_tests {
    use super::*;
    use crate::types::{GatewayCommand, GatewayState, ProcessOptions, ToolCall};
    use std::time::{Duration, Instant};

    fn no_http_wait() -> ProcessOptions {
        ProcessOptions {
            wait_for_drone_ack: false,
            ack_timeout_ms: 50,
        }
    }

    fn override_orch() -> Orchestrator {
        let mut orch = Orchestrator::new();
        orch.current_state = GatewayState::OVERRIDE_ACTIVE;
        orch.override_until = Some(Instant::now() + Duration::from_secs(60));
        orch
    }

    fn fast_client() -> Client {
        Client::builder()
            .timeout(Duration::from_millis(200))
            .build()
            .expect("client")
    }

    #[tokio::test]
    async fn apply_tool_force_arm_rejected_before_dispatch() {
        let mut orch = override_orch();
        let outcome = orch
            .process_command(
                GatewayCommand::ApplyTool {
                    category: "drone".into(),
                    tool_name: "force_arm".into(),
                    params: None,
                },
                &fast_client(),
                "test-force-arm",
                None,
                no_http_wait(),
            )
            .await;
        assert_eq!(outcome.action_taken, "tool_rejected:invalid_request");
        assert!(outcome.drone_steps.is_empty());
        assert!(outcome.model_steps.is_empty());
        assert!(outcome.drone_http_status.is_none());
        assert_eq!(orch.current_state, GatewayState::OVERRIDE_ACTIVE);
        assert!(
            outcome.latency_ms < 1000,
            "reject must not wait on HTTP: {} ms",
            outcome.latency_ms
        );
    }

    #[tokio::test]
    async fn apply_sequence_unknown_name_rejected_before_dispatch() {
        let mut orch = override_orch();
        let outcome = orch
            .process_command(
                GatewayCommand::ApplyToolSequence {
                    tools: vec![ToolCall {
                        category: "drone".into(),
                        name: "do_a_barrel_roll".into(),
                        params: None,
                    }],
                },
                &fast_client(),
                "test-seq-unknown",
                None,
                no_http_wait(),
            )
            .await;
        assert_eq!(outcome.action_taken, "tool_rejected:invalid_request");
        assert!(outcome.drone_steps.is_empty());
        assert_eq!(orch.current_state, GatewayState::OVERRIDE_ACTIVE);
        assert!(
            outcome.latency_ms < 1000,
            "reject must not wait on HTTP: {} ms",
            outcome.latency_ms
        );
    }
}
