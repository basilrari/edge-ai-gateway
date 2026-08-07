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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drone_step_id_format() {
        assert_eq!(
            drone_step_id("abc-123", 2),
            "abc-123:drone:2"
        );
    }
}
