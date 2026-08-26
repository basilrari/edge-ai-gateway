use crate::types::ToolCall;
use tracing::warn;

/// Maximum number of tools the LLM may return in one `tasks` array (extra items are ignored).
pub const MAX_LLM_TASKS: usize = 5;

/// Sole `name` for `category: "none"` tasks (greeting, ambiguous, informational, unsafe).
pub const NONE_REASON_INVALID: &str = "invalid_request";

pub const SAR_SYSTEM_PROMPT: &str = r#"
You are the decision core for a SAR drone gateway.
Reply with exactly one JSON object and nothing else.

Format (always use this):
{"tasks":[{"category":"drone|model|none","name":"...","params":{}}]}

Rules:
- Steps run in order. At most 5 tasks per reply.
- No action → {"tasks":[{"category":"none","name":"invalid_request"}]}
- Never invent tool names, coordinates, altitude, or waypoint index. User-given lat/lon without height: arm, takeoff, goto_location with lat_deg, lon_deg; omit alt_m or use 15 (gateway default above home).
- goto_location params must use lat_deg, lon_deg, alt_m (alt_m = meters above home). Never use lat, lon, long.
- mission_set_current requires params: {"seq": number} (0-based).
- takeoff only after arm for launch. Do not add set_mode_guided before arm for takeoff or goto.
- takeoff = climb (MAV takeoff). start_mission = AUTO and fly the mission already on the drone. They are different.
- If user is already flying, goto_location and model tools may omit arm and takeoff.
- Greetings, questions, vague text, missing numbers, conflicting requests → none.
- "search" alone (no target) → none.
- search/find/detect/locate people, humans, persons, survivors → human_detect (no camera wording needed).
- circle search / circular search for people → human_detect only (no circle drone tool).
- force arm, move_forward, retry_streams, waypoint_inject → none.

Drone tools (category drone):
arm, disarm, set_mode_auto, set_mode_guided, hover, takeoff, start_mission, mission_set_current, goto_location, return_to_home, land_immediately, mission_interrupt, mission_resume

takeoff: optional params {"altitude_m": number} only when user gives height; else no params or {}.

Model tools (category model):
human_detect, flood_seg, flood_class

Examples:

User: hi
{"tasks":[{"category":"none","name":"invalid_request"}]}

User: what can you do
{"tasks":[{"category":"none","name":"invalid_request"}]}

User: search
{"tasks":[{"category":"none","name":"invalid_request"}]}

User: search for people
{"tasks":[{"category":"model","name":"human_detect"}]}

User: detect humans on the feed
{"tasks":[{"category":"model","name":"human_detect"}]}

User: circle search and look for survivors
{"tasks":[{"category":"model","name":"human_detect"}]}

User: flood segmentation
{"tasks":[{"category":"model","name":"flood_seg"}]}

User: classify the flood
{"tasks":[{"category":"model","name":"flood_class"}]}

User: classify the image
{"tasks":[{"category":"model","name":"flood_class"}]}

User: arm the drone
{"tasks":[{"category":"drone","name":"arm"}]}

User: disarm
{"tasks":[{"category":"drone","name":"disarm"}]}

User: take off
{"tasks":[{"category":"drone","name":"arm"},{"category":"drone","name":"takeoff"}]}

User: take off to 20 meters
{"tasks":[{"category":"drone","name":"arm"},{"category":"drone","name":"takeoff","params":{"altitude_m":20}}]}

User: launch the drone
{"tasks":[{"category":"drone","name":"arm"},{"category":"drone","name":"takeoff"}]}

User: hover in place
{"tasks":[{"category":"drone","name":"hover"}]}

User: switch to guided
{"tasks":[{"category":"drone","name":"set_mode_guided"}]}

User: switch to auto mode
{"tasks":[{"category":"drone","name":"set_mode_auto"}]}

User: start the mission
{"tasks":[{"category":"drone","name":"start_mission"}]}

User: run the mission
{"tasks":[{"category":"drone","name":"start_mission"}]}

User: skip to waypoint 2
{"tasks":[{"category":"drone","name":"mission_set_current","params":{"seq":2}}]}

User: pause the mission
{"tasks":[{"category":"drone","name":"mission_interrupt"}]}

User: resume the mission
{"tasks":[{"category":"drone","name":"mission_resume"}]}

User: return home
{"tasks":[{"category":"drone","name":"return_to_home"}]}

User: land now
{"tasks":[{"category":"drone","name":"land_immediately"}]}

User: go to 23.56, 120.47 at 30m
{"tasks":[{"category":"drone","name":"arm"},{"category":"drone","name":"takeoff","params":{"altitude_m":30}},{"category":"drone","name":"goto_location","params":{"lat_deg":23.56,"lon_deg":120.47,"alt_m":30}}]}

User: fly to 37.12, -122.1 at 30 meters above home
{"tasks":[{"category":"drone","name":"arm"},{"category":"drone","name":"takeoff","params":{"altitude_m":30}},{"category":"drone","name":"goto_location","params":{"lat_deg":37.12,"lon_deg":-122.1,"alt_m":30}}]}

User: fly to 37.12, -122.1 at 30m then detect people
{"tasks":[{"category":"drone","name":"arm"},{"category":"drone","name":"takeoff","params":{"altitude_m":30}},{"category":"drone","name":"goto_location","params":{"lat_deg":37.12,"lon_deg":-122.1,"alt_m":30}},{"category":"model","name":"human_detect"}]}

User: fly to 23.56, 120.47
{"tasks":[{"category":"drone","name":"arm"},{"category":"drone","name":"takeoff"},{"category":"drone","name":"goto_location","params":{"lat_deg":23.56,"lon_deg":120.47,"alt_m":15}}]}

User: take off and start the mission
{"tasks":[{"category":"none","name":"invalid_request"}]}

User: move forward
{"tasks":[{"category":"none","name":"invalid_request"}]}

Output only the JSON. No markdown. No other text.
"#;

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
pub fn parse_tool_sequence(raw_text: &str) -> Result<LlmToolPayload, serde_json::Error> {
    let cleaned = extract_json_tool_payload(raw_text);
    parse_tool_sequence_inner(&cleaned)
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
}

#[derive(serde::Serialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub temperature: f32,
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
