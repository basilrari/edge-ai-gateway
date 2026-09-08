use crate::types::ToolCall;
use tracing::warn;

/// Maximum number of tools the LLM may return in one `tasks` array (extra items are ignored).
pub const MAX_LLM_TASKS: usize = 5;

/// Sole `name` for `category: "none"` tasks (greeting, ambiguous, informational, unsafe).
pub const NONE_REASON_INVALID: &str = "invalid_request";

pub const SAR_SYSTEM_PROMPT: &str = r#"
You convert a SAR operator command into one JSON object. Output only JSON.

Format:
{"tasks":[{"category":"drone|model|none","name":"...","params":{}}]}

Rules:
- At most 5 tasks, in order.
- Use only the tools listed below. Never invent tool names.
- If ANY requested action has no matching tool → {"tasks":[{"category":"none","name":"invalid_request"}]}
- Greeting, vague text, questions, unsafe commands, missing required coordinates, conflicting commands → none.
- "search" with no target → none.
- search/find/detect/locate people, humans, persons, survivors → human_detect.
- takeoff = climb. start_mission = switch to AUTO and fly the mission already on the drone. They can be used together: arm, takeoff, start_mission.
- For fly-to / takeoff / launch, always start with arm then takeoff.
- For hover, land, return home, pause, resume, do not add arm or takeoff.
- takeoff height: params {"altitude_m": number} only if the user gave a height.
- goto_location must use lat_deg, lon_deg, alt_m. If height is missing, use alt_m 15.
- Do not add set_mode_guided unless the user asks for guided.
- Do not add set_mode_auto unless the user asks for auto or start_mission already covers it.

Drone tools:
arm, disarm, set_mode_auto, set_mode_guided, hover, takeoff, start_mission, mission_set_current, goto_location, return_to_home, land_immediately, mission_interrupt, mission_resume

takeoff params: {"altitude_m": number} optional
goto_location params: {"lat_deg": number, "lon_deg": number, "alt_m": number}
mission_set_current params: {"seq": number}

Model tools:
human_detect, flood_seg, flood_class

Examples:
User: hi
{"tasks":[{"category":"none","name":"invalid_request"}]}

User: search
{"tasks":[{"category":"none","name":"invalid_request"}]}

User: circle search and look for survivors
{"tasks":[{"category":"none","name":"invalid_request"}]}

User: search for people
{"tasks":[{"category":"model","name":"human_detect"}]}

User: classify the flood
{"tasks":[{"category":"model","name":"flood_class"}]}

User: arm the drone
{"tasks":[{"category":"drone","name":"arm"}]}

User: take off to 20 meters
{"tasks":[{"category":"drone","name":"arm"},{"category":"drone","name":"takeoff","params":{"altitude_m":20}}]}

User: take off and start the mission
{"tasks":[{"category":"drone","name":"arm"},{"category":"drone","name":"takeoff"},{"category":"drone","name":"start_mission"}]}

User: start the mission
{"tasks":[{"category":"drone","name":"start_mission"}]}

User: return home
{"tasks":[{"category":"drone","name":"return_to_home"}]}

User: fly to 23.56, 120.47
{"tasks":[{"category":"drone","name":"arm"},{"category":"drone","name":"takeoff"},{"category":"drone","name":"goto_location","params":{"lat_deg":23.56,"lon_deg":120.47,"alt_m":15}}]}

User: fly to 23.56, 120.47 at 30m then detect people
{"tasks":[{"category":"drone","name":"arm"},{"category":"drone","name":"takeoff","params":{"altitude_m":30}},{"category":"drone","name":"goto_location","params":{"lat_deg":23.56,"lon_deg":120.47,"alt_m":30}},{"category":"model","name":"human_detect"}]}
"#;

/// LLM-visible drone tools. Extra drone-http names (`force_arm`, `circle_search`, …) are rejected here.
pub const ALLOWED_DRONE_TOOLS: &[&str] = &[
    "arm",
    "disarm",
    "set_mode_auto",
    "set_mode_guided",
    "hover",
    "takeoff",
    "start_mission",
    "mission_set_current",
    "goto_location",
    "return_to_home",
    "land_immediately",
    "mission_interrupt",
    "mission_resume",
];

pub const ALLOWED_MODEL_TOOLS: &[&str] = &["human_detect", "flood_seg", "flood_class"];

fn allowed_param_keys(category: &str, name: &str) -> Option<&'static [&'static str]> {
    match (category, name) {
        ("drone", "takeoff") => Some(&["altitude_m"]),
        ("drone", "goto_location") => Some(&["lat_deg", "lon_deg", "alt_m"]),
        ("drone", "mission_set_current") => Some(&["seq"]),
        ("drone", n) if ALLOWED_DRONE_TOOLS.contains(&n) => Some(&[]),
        ("model", n) if ALLOWED_MODEL_TOOLS.contains(&n) => Some(&[]),
        _ => None,
    }
}

fn strip_unknown_params(
    params: Option<serde_json::Value>,
    allowed: &[&str],
) -> Option<serde_json::Value> {
    match params {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::Object(mut m)) => {
            m.retain(|k, _| allowed.contains(&k.as_str()));
            if m.is_empty() {
                None
            } else {
                Some(serde_json::Value::Object(m))
            }
        }
        Some(_) => None,
    }
}

/// After JSON parse: reject unknown tools; strip unknown param keys. `goto_location` needs lat/lon.
pub fn validate_llm_tasks(tasks: Vec<ToolCall>) -> LlmToolPayload {
    let mut out = Vec::with_capacity(tasks.len());
    for t in tasks {
        let Some(keys) = allowed_param_keys(&t.category, &t.name) else {
            return LlmToolPayload::NoneReason(NONE_REASON_INVALID.into());
        };
        let params = strip_unknown_params(t.params, keys);
        if t.name == "goto_location" {
            let ok = params
                .as_ref()
                .and_then(|v| v.as_object())
                .map(|m| m.contains_key("lat_deg") && m.contains_key("lon_deg"))
                .unwrap_or(false);
            if !ok {
                return LlmToolPayload::NoneReason(NONE_REASON_INVALID.into());
            }
        }
        out.push(ToolCall {
            category: t.category,
            name: t.name,
            params,
        });
    }
    if out.is_empty() {
        LlmToolPayload::NoneReason(NONE_REASON_INVALID.into())
    } else {
        LlmToolPayload::Tasks(out)
    }
}

#[derive(Debug, Clone)]
pub enum LlmToolPayload {
    /// `tasks` entry with `category: none` — no tools to run.
    NoneReason(String),
    /// One or more drone/model steps in order (already capped).
    Tasks(Vec<ToolCall>),
}

#[derive(serde::Deserialize)]
struct TasksEnvelope {
    tasks: Vec<ToolCall>,
}

/// Map legacy none `name` values to the single contract reason.
pub fn normalize_none_reason(reason: &str) -> String {
    match reason {
        NONE_REASON_INVALID => NONE_REASON_INVALID.to_string(),
        "greeting_only" | "ambiguous_request" | "informational_request" | "unsafe_or_invalid" => {
            NONE_REASON_INVALID.to_string()
        }
        _ => reason.to_string(),
    }
}

/// Strip optional Markdown fences so models that wrap JSON in ` ```json ` blocks still parse.
pub fn extract_json_tool_payload(raw_text: &str) -> String {
    let s = raw_text.trim();
    if let Some(pos) = s.find("```") {
        let after_fence = &s[pos + 3..];
        let after_fence = after_fence
            .strip_prefix("json")
            .or_else(|| after_fence.strip_prefix("JSON"))
            .unwrap_or(after_fence)
            .trim_start();
        if let Some(end) = after_fence.find("```") {
            return after_fence[..end].trim().to_string();
        }
    }
    s.to_string()
}

fn parse_tool_sequence_inner(cleaned: &str) -> Result<LlmToolPayload, serde_json::Error> {
    let v: serde_json::Value = serde_json::from_str(cleaned)?;

    if v.get("tasks").is_some() {
        let envelope: TasksEnvelope = serde_json::from_value(v)?;
        if envelope.tasks.is_empty() {
            return Ok(LlmToolPayload::NoneReason(NONE_REASON_INVALID.into()));
        }
        let original_len = envelope.tasks.len();
        let out: Vec<ToolCall> = envelope
            .tasks
            .into_iter()
            .take(MAX_LLM_TASKS)
            .collect();
        if original_len > MAX_LLM_TASKS {
            warn!(
                action = "llm_tasks_truncated",
                original_len,
                kept = MAX_LLM_TASKS,
                reason = "LLM returned more than MAX_LLM_TASKS; extra steps dropped"
            );
        }
        for t in &out {
            if t.category == "none" {
                return Ok(LlmToolPayload::NoneReason(normalize_none_reason(&t.name)));
            }
            if t.category != "drone" && t.category != "model" {
                return Ok(LlmToolPayload::NoneReason(NONE_REASON_INVALID.into()));
            }
        }
        if out.is_empty() {
            return Ok(LlmToolPayload::NoneReason(NONE_REASON_INVALID.into()));
        }
        return Ok(LlmToolPayload::Tasks(out));
    }

    Err(serde::de::Error::custom(
        "expected top-level {\"tasks\":[...]}; legacy single-object form is not accepted",
    ))
}

/// Parse LLM JSON strictly: only `{"tasks":[...]}` (one or more steps, or a single none task).
/// Only strips optional Markdown fences; invalid JSON is an error.
/// After a successful parse, unknown tools / missing goto lat-lon become `invalid_request`.
pub fn parse_tool_sequence(raw_text: &str) -> Result<LlmToolPayload, serde_json::Error> {
    let cleaned = extract_json_tool_payload(raw_text);
    let parsed = parse_tool_sequence_inner(&cleaned)?;
    Ok(match parsed {
        LlmToolPayload::Tasks(tasks) => validate_llm_tasks(tasks),
        other => other,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_trailing_comma() {
        let raw = r#"{"tasks":[{"category":"drone","name":"arm"},]}"#;
        assert!(parse_tool_sequence(raw).is_err());
    }

    #[test]
    fn parses_valid_tasks() {
        let raw = r#"{"tasks":[{"category":"drone","name":"arm"}]}"#;
        match parse_tool_sequence(raw).unwrap() {
            LlmToolPayload::Tasks(t) => assert_eq!(t.len(), 1),
            _ => panic!("expected tasks"),
        }
    }

    #[test]
    fn parses_none_in_tasks() {
        let raw = r#"{"tasks":[{"category":"none","name":"invalid_request"}]}"#;
        match parse_tool_sequence(raw).unwrap() {
            LlmToolPayload::NoneReason(r) => assert_eq!(r, "invalid_request"),
            _ => panic!("expected none reason"),
        }
    }

    #[test]
    fn rejects_legacy_single_object() {
        let raw = r#"{"category":"none","name":"ambiguous_request"}"#;
        assert!(parse_tool_sequence(raw).is_err());
    }

    #[test]
    fn unknown_tool_name_is_invalid_request() {
        let raw = r#"{"tasks":[{"category":"drone","name":"do_a_barrel_roll"}]}"#;
        match parse_tool_sequence(raw).unwrap() {
            LlmToolPayload::NoneReason(r) => assert_eq!(r, "invalid_request"),
            _ => panic!("expected invalid_request"),
        }
    }

    #[test]
    fn force_arm_is_invalid_request() {
        let raw = r#"{"tasks":[{"category":"drone","name":"force_arm"}]}"#;
        match parse_tool_sequence(raw).unwrap() {
            LlmToolPayload::NoneReason(r) => assert_eq!(r, "invalid_request"),
            _ => panic!("expected invalid_request"),
        }
    }

    #[test]
    fn valid_human_detect_still_works() {
        let raw = r#"{"tasks":[{"category":"model","name":"human_detect"}]}"#;
        match parse_tool_sequence(raw).unwrap() {
            LlmToolPayload::Tasks(t) => {
                assert_eq!(t.len(), 1);
                assert_eq!(t[0].category, "model");
                assert_eq!(t[0].name, "human_detect");
            }
            _ => panic!("expected tasks"),
        }
    }

    #[test]
    fn goto_missing_alt_still_allowed() {
        let raw = r#"{"tasks":[{"category":"drone","name":"goto_location","params":{"lat_deg":23.56,"lon_deg":120.47}}]}"#;
        match parse_tool_sequence(raw).unwrap() {
            LlmToolPayload::Tasks(t) => {
                assert_eq!(t.len(), 1);
                let p = t[0].params.as_ref().unwrap();
                assert_eq!(p["lat_deg"], 23.56);
                assert_eq!(p["lon_deg"], 120.47);
                assert!(p.get("alt_m").is_none());
            }
            _ => panic!("expected tasks"),
        }
    }
}

#[derive(serde::Serialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub temperature: f32,
    pub max_tokens: u32,
}

#[derive(serde::Deserialize, Debug)]
pub struct ChatResponse {
    pub choices: Vec<Choice>,
}

#[derive(serde::Deserialize, Debug)]
pub struct Choice {
    pub message: Message,
}

#[derive(serde::Deserialize, Debug)]
pub struct Message {
    pub content: String,
}

#[derive(serde::Serialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}
