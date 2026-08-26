//! Wall-clock helpers and trace step IDs for pipeline metrics.

/// Unix epoch milliseconds (for correlation only; do not subtract across hosts).
pub fn unix_ms_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Child trace ID for one drone step in a multi-step infer sequence.
pub fn drone_step_id(parent_request_id: &str, step_index: usize) -> String {
    format!("{parent_request_id}:drone:{step_index}")
}

/// Fill denormalized per-layer fields on [`crate::types::PipelineTiming`].
pub fn finalize_pipeline_layers(
    pipeline: &mut crate::types::PipelineTiming,
    drone_steps: &[crate::types::DroneStepTiming],
    model_steps: &[crate::types::ModelStepTiming],
) {
    pipeline.llm_ms = pipeline.llm_http_ms.saturating_add(pipeline.llm_parse_ms);
    pipeline.drone_server_ms = drone_steps.iter().map(|s| s.drone_http_ms).sum();
    pipeline.drone_ack_wait_ms = drone_steps
        .iter()
        .filter_map(|s| s.ack_wait_ms)
        .sum();
    pipeline.model_server_ms = model_steps.iter().map(|s| s.elapsed_ms).sum();
    let accounted = pipeline
        .queue_wait_ms
        .saturating_add(pipeline.llm_ms)
        .saturating_add(pipeline.drone_server_ms)
        .saturating_add(pipeline.model_server_ms);
    pipeline.gateway_ms = pipeline.handler_total_ms.saturating_sub(accounted);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finalize_layers_sums() {
        use crate::types::{DroneStepTiming, ModelStepTiming, PipelineTiming};
        let mut p = PipelineTiming {
            queue_wait_ms: 2,
            handler_total_ms: 1000,
            llm_http_ms: 800,
            llm_parse_ms: 10,
            ..Default::default()
        };
        let drone = vec![DroneStepTiming {
            step_index: 0,
            tool: "arm".into(),
            step_id: "x".into(),
            drone_http_ms: 150,
            dispatch_ms: Some(5),
            ack_wait_ms: Some(140),
            completion_status: None,
            ack_result: None,
            http_status: 200,
            ok: true,
        }];
        let model = vec![ModelStepTiming {
            step_index: 1,
            tool: "human_detect".into(),
            placeholder: false,
            elapsed_ms: 1,
            inference_ms: None,
            http_status: 200,
            ok: true,
        }];
        finalize_pipeline_layers(&mut p, &drone, &model);
        assert_eq!(p.llm_ms, 810);
        assert_eq!(p.drone_server_ms, 150);
        assert_eq!(p.drone_ack_wait_ms, 140);
        assert_eq!(p.model_server_ms, 1);
        assert_eq!(p.gateway_ms, 37);
    }
}
