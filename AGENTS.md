# Gateway API — For Agents & Frontend

This file defines the **exact API contract** (inputs and outputs) and **how the server works** so other agents building the frontend can integrate without reading the Rust code.

---

## Base URL

- Server: `http://0.0.0.0:3000` (or `http://localhost:3000` from the same host).
- CORS: All origins, `GET` / `POST` / `OPTIONS`, all headers allowed.

---

## 1. GET /status

**Purpose**: Read current gateway state, active model, override flag, and **the current drone or model command** (or `none`).

**Request**

- Method: `GET`
- Path: `/status`
- Body: none

**Response** (JSON)

| Field               | Type    | Description |
|---------------------|--------|-------------|
| `state`             | string | One of: `"IDLE"`, `"ACTIVE"`, `"OVERRIDE_ACTIVE"`, `"SWITCHING"`. |
| `model`             | string | Current model name (e.g. `"vision"`, `"text"`) or `"none"`. |
| `override_active`   | bool   | `true` if a time-limited override is active. |
| `active_command`    | string | **Current active command.** Either `"drone: <tool_name>"`, `"model: <tool_name>"`, or `"none"` if no command has been set (e.g. idle or just started). |
| `latency_ms`        | number | Last operation latency (ms). |
| `llm_latency_ms`    | number | Last LLM call latency (ms). |
| `memory_estimate_mb`| number | Approximate process memory (MB). |

**Example**

```json
{
  "state": "ACTIVE",
  "model": "vision",
  "override_active": false,
  "active_command": "model: activate_human_detection_yolo",
  "latency_ms": 12,
  "llm_latency_ms": 8,
  "memory_estimate_mb": 12.5
}
```

When nothing has been run yet or after clear override:

```json
{
  "state": "IDLE",
  "model": "none",
  "override_active": false,
  "active_command": "none",
  "latency_ms": 0,
  "llm_latency_ms": 0,
  "memory_estimate_mb": 12.5
}
```

---

## 2. POST /infer

**Purpose**: Send a command: either **infer** (user prompt → LLM → tool applied), **override** (force model for a period), or **clear override**.

**Request**

- Method: `POST`
- Path: `/infer`
- Headers: `Content-Type: application/json`
- Body: **One** of the following JSON objects (externally tagged enum).

**Infer** (run LLM and apply chosen tool):

```json
{
  "Infer": {
    "prompt": "detect people in the flooded area"
  }
}
```

**Override** (force model; optional timeout in seconds, default 60):

```json
{
  "Override": {
    "model": "vision",
    "timeout_sec": 60
  }
}
```

**ClearOverride**:

```json
{
  "ClearOverride": null
}
```

**Response** (JSON)

| Field             | Type    | Description |
|-------------------|--------|-------------|
| `state`           | string | `"IDLE"` \| `"ACTIVE"` \| `"OVERRIDE_ACTIVE"` \| `"SWITCHING"`. |
| `model`           | string \| null | Current model or `null`. |
| `override_active` | bool   | Whether an override is active. |
| `category`        | string \| null | Tool category: `"drone"` or `"model"` (only set when Infer ran and a tool was parsed). |
| `tool_name`       | string \| null | Tool name (e.g. `"move_forward"`, `"activate_human_detection_yolo"`). |
| `llm_response`    | string | Raw LLM response body (or error message). |
| `action_taken`    | string | Short description of what was done (e.g. `"Drone command: move_forward"`, `"override_set"`). |
| `latency_ms`       | number | Total request latency (ms). |
| `llm_latency_ms`   | number | LLM call latency (ms). |

**Example (Infer success)**

```json
{
  "state": "ACTIVE",
  "model": "vision",
  "override_active": false,
  "category": "model",
  "tool_name": "activate_human_detection_yolo",
  "llm_response": "...",
  "action_taken": "Python worker will activate: activate_human_detection_yolo",
  "latency_ms": 120,
  "llm_latency_ms": 95
}
```

**Example (parse error)**

```json
{
  "state": "ERROR",
  "model": null,
  "override_active": false,
  "category": null,
  "tool_name": null,
  "llm_response": "invalid payload: ...",
  "action_taken": "parse_failed",
  "latency_ms": 0,
  "llm_latency_ms": 0
}
```

---

## How the server works (short)

1. **IDLE**: No model set, no active command; `active_command` is `"none"`.
2. **Infer (prompt)**:
   - Gateway sends the prompt (with system prompt) to the LLM at `http://localhost:8080/v1/chat/completions`.
   - LLM is expected to return exactly one JSON tool: `{"category": "drone"|"model", "name": "<tool_name>"}`.
   - Gateway updates state to **ACTIVE**, stores the last **category** and **name** (so **GET /status** can return `active_command` as `"drone: ..."` or `"model: ..."`).
   - Model tools set internal model to `"vision"`; drone tools do not change model.
3. **Override**: Sets state to **OVERRIDE_ACTIVE** and forces the given **model** for **timeout_sec**; does not change `active_command` (that stays the last inferred tool until cleared).
4. **ClearOverride**: Resets to **IDLE**, clears model and **active_command** (so next **GET /status** returns `active_command: "none"`).
5. **GET /status** never changes state; it returns current state, model, override flag, and **active_command** (last drone or model command, or `"none"`).

---

## Tool names reference

- **model**: `activate_human_detection_yolo`, `activate_flood_segmentation`, `activate_human_behaviour_analysis`, `share_with_swarm`, `activate_flood_classification`
- **drone**: `move_forward`, `hover`, `return_to_home`, `land_immediately`, `circle_search`

Use **GET /status** and the **active_command** field to show the current drone or model command in the UI; use **POST /infer** to send user prompts or override/clear-override.
