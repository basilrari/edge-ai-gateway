//! Normalize drone tool `params` before forwarding to `drone-http` (and for UI display).

use crate::types::ToolCall;
use serde_json::{json, Map, Value};

/// Meters above home when `altitude_m` / `alt_m` is missing, zero, or non-positive.
pub const DEFAULT_ALTITUDE_ABOVE_HOME_M: f64 = 15.0;

fn altitude_above_home(value: Option<f64>) -> f64 {
    match value {
        Some(v) if v.is_finite() && v > 0.0 => v,
        _ => DEFAULT_ALTITUDE_ABOVE_HOME_M,
    }
}

fn set_alt_key(obj: &mut Map<String, Value>, key: &str, value: f64) {
    obj.insert(key.to_string(), json!(value));
}

/// Patch LLM drone params: `0` / omitted `alt_m` → 15 m; `takeoff` only patches explicit `altitude_m`.
pub fn normalize_drone_tool_params(
    tool_name: &str,
    params: Option<Value>,
) -> Option<Value> {
    let mut obj = match params {
        Some(Value::Object(m)) => m,
        Some(Value::Null) | None => Map::new(),
        Some(other) => return Some(other),
    };

    match tool_name {
        "takeoff" => {
            if obj.contains_key("altitude_m") {
                let v = obj.get("altitude_m").and_then(|x| x.as_f64());
                set_alt_key(&mut obj, "altitude_m", altitude_above_home(v));
            }
        }
        "goto_location" => {
            let v = obj.get("alt_m").and_then(|x| x.as_f64());
            set_alt_key(&mut obj, "alt_m", altitude_above_home(v));
        }
        "waypoint_inject" => {
            if obj.contains_key("lat_deg") && obj.contains_key("lon_deg") {
                let v = obj.get("alt_m").and_then(|x| x.as_f64());
                set_alt_key(&mut obj, "alt_m", altitude_above_home(v));
            }
        }
        _ => {}
    }

    if obj.is_empty() {
        None
    } else {
        Some(Value::Object(obj))
    }
}

pub fn normalize_drone_tasks(tasks: &mut [ToolCall]) {
    for t in tasks.iter_mut() {
        if t.category == "drone" {
            t.params = normalize_drone_tool_params(&t.name, t.params.take());
        }
    }
}

pub fn tasks_display_json(tasks: &[ToolCall]) -> String {
    json!({ "tasks": tasks }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ToolCall;

    #[test]
    fn takeoff_zero_becomes_fifteen() {
        let out = normalize_drone_tool_params(
            "takeoff",
            Some(json!({ "altitude_m": 0 })),
        )
        .unwrap();
        assert_eq!(out["altitude_m"], 15.0);
    }

    #[test]
    fn takeoff_omitted_unchanged() {
        assert!(normalize_drone_tool_params("takeoff", None).is_none());
    }

    #[test]
    fn goto_missing_alt_m_defaults() {
        let out = normalize_drone_tool_params(
            "goto_location",
            Some(json!({ "lat_deg": 1.0, "lon_deg": 2.0 })),
        )
        .unwrap();
        assert_eq!(out["alt_m"], 15.0);
    }

    #[test]
    fn normalizes_task_slice() {
        let mut tasks = vec![
            ToolCall {
                category: "drone".into(),
                name: "takeoff".into(),
                params: Some(json!({ "altitude_m": 0 })),
            },
            ToolCall {
                category: "drone".into(),
                name: "goto_location".into(),
                params: Some(json!({ "lat_deg": 23.0, "lon_deg": 120.0 })),
            },
        ];
        normalize_drone_tasks(&mut tasks);
        assert_eq!(tasks[0].params.as_ref().unwrap()["altitude_m"], 15.0);
        assert_eq!(tasks[1].params.as_ref().unwrap()["alt_m"], 15.0);
    }
}
