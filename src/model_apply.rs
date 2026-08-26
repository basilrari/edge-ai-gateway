//! Forward gateway model tools to Drone_LLM `POST /tool` (vision inference).

use crate::config;
use reqwest::Client;
use std::time::{Duration, Instant};

/// Map SAR router model tool names → Drone_LLM tool ids (see `core/gateway_tools.py`).
pub fn map_gateway_model_tool(name: &str) -> Option<&'static str> {
    match name {
        "human_detect" => Some("detect_human"),
        "flood_seg" | "flood_class" => Some("detect_flood"),
        _ => None,
    }
}

pub struct ModelApplyResult {
    pub http_status: u16,
    /// Gateway ↔ model-server HTTP wall time (includes synchronous inference in Drone_LLM).
    pub elapsed_ms: u64,
    pub ok: bool,
    pub error_detail: Option<String>,
    /// Model-reported inference latency when present in JSON body.
    pub inference_ms: Option<u64>,
    pub skipped: bool,
}

fn inference_ms_from_body(v: &serde_json::Value) -> Option<u64> {
    v.get("metrics")
        .and_then(|m| {
            m.get("total_inference_ms")
                .or_else(|| m.get("total_latency_ms"))
                .or_else(|| m.get("detection_ms"))
        })
        .and_then(|x| x.as_f64())
        .map(|f| f.round() as u64)
        .or_else(|| {
            v.get("system")
                .and_then(|s| s.get("latency_ms"))
                .and_then(|x| x.as_f64())
                .map(|f| f.round() as u64)
        })
}

pub async fn model_apply_via_http(
    client: &Client,
    request_id: &str,
    gateway_tool: &str,
    trace: &mut Vec<String>,
) -> ModelApplyResult {
    let Some(model_tool) = map_gateway_model_tool(gateway_tool) else {
        trace.push(format!("stage=model_http_unknown_tool name={gateway_tool}"));
        return ModelApplyResult {
            http_status: 0,
            elapsed_ms: 0,
            ok: false,
            error_detail: Some(format!("unknown model tool: {gateway_tool}")),
            inference_ms: None,
            skipped: false,
        };
    };

    let url = config::model_apply_tool_url();
    trace.push(format!(
        "stage=model_http_begin tool={gateway_tool} model_tool={model_tool} url={url}"
    ));

    let body = serde_json::json!({ "tool": model_tool });
    let t0 = Instant::now();
    let send_result = client
        .post(&url)
        .header("x-request-id", request_id)
        .json(&body)
        .timeout(Duration::from_secs(120))
        .send()
        .await;
    let elapsed_ms = t0.elapsed().as_millis() as u64;

    match send_result {
        Err(e) => {
            trace.push(format!("stage=model_http_transport_failed err={e}"));
            ModelApplyResult {
                http_status: 0,
                elapsed_ms,
                ok: false,
                error_detail: Some(format!("model_http_transport: {e}")),
                inference_ms: None,
                skipped: false,
            }
        }
        Ok(resp) => {
            let status_code = resp.status().as_u16();
            let body_text = resp.text().await.unwrap_or_default();
            trace.push(format!(
                "stage=model_http_response tool={gateway_tool} status={status_code} body_len={}",
                body_text.len()
            ));

            let parsed: Result<serde_json::Value, _> = serde_json::from_str(&body_text);
            match parsed {
                Ok(v) => {
                    let skipped = v.get("skipped").and_then(|x| x.as_bool()).unwrap_or(false);
                    let inference_ms = inference_ms_from_body(&v);
                    if let Some(inf) = inference_ms {
                        trace.push(format!("stage=model_inference_ms ms={inf}"));
                    }
                    if skipped {
                        trace.push("stage=model_http_skipped busy".into());
                        return ModelApplyResult {
                            http_status: status_code,
                            elapsed_ms,
                            ok: true,
                            error_detail: None,
                            inference_ms,
                            skipped: true,
                        };
                    }
                    if let Some(err) = v.get("error").and_then(|x| x.as_str()) {
                        return ModelApplyResult {
                            http_status: status_code,
                            elapsed_ms,
                            ok: false,
                            error_detail: Some(err.to_string()),
                            inference_ms,
                            skipped: false,
                        };
                    }
                    ModelApplyResult {
                        http_status: status_code,
                        elapsed_ms,
                        ok: status_code >= 200 && status_code < 300,
                        error_detail: None,
                        inference_ms,
                        skipped: false,
                    }
                }
                Err(e) => ModelApplyResult {
                    http_status: status_code,
                    elapsed_ms,
                    ok: false,
                    error_detail: Some(format!("model_http_bad_json: {e}")),
                    inference_ms: None,
                    skipped: false,
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_gateway_model_tools() {
        assert_eq!(map_gateway_model_tool("human_detect"), Some("detect_human"));
        assert_eq!(map_gateway_model_tool("flood_seg"), Some("detect_flood"));
        assert_eq!(map_gateway_model_tool("flood_class"), Some("detect_flood"));
        assert!(map_gateway_model_tool("takeoff").is_none());
    }
}
