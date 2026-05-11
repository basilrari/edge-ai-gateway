# Gateway API — For Agents & Frontend

This file defines the **exact API contract** (inputs and outputs) and **how the server works** so other agents building the frontend can integrate without reading the Rust code.

---

## Project context

- **Code** is the SAR (Search and Rescue) drone repo. See [Code/README.md](../README.md) for the full architecture.
- The **Gateway** sits between the **frontend** and the **LLM** / **Drone Server** / **Model Server**. It receives HTTP requests (prompts, ApplyTool, ApplyToolSequence, Override, ClearOverride), calls the LLM for inference, and routes accepted tools: **model** → Model Server (e.g. python-worker); **drone** → Drone Server ([drone-server/](../drone-server/)) via HTTP. The Gateway does not implement MAVLink or drone logic; it only routes commands and keeps state (IDLE, ACTIVE, OVERRIDE_ACTIVE).

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
  "active_command": "model: human_detect",
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

**Purpose**: Send a command: **Infer** (user prompt → LLM → proposed tool(s)), **ApplyTool** / **ApplyToolSequence** (after operator accepts), **override**, or **clear override**.

**Request**

- Method: `POST`
- Path: `/infer`
- Headers: `Content-Type: application/json`
- Body: **One** of the following JSON objects (externally tagged enum).

**Infer** (run LLM and return proposed tool; does **not** apply or send to Python until frontend sends **ApplyTool**):

```json
{
  "Infer": {
    "prompt": "detect people in the flooded area"
  }
}
```

**ApplyTool** (after user accepts a **single-step** proposal; applies one tool):

```json
{
  "ApplyTool": {
    "category": "model",
    "tool_name": "human_detect"
  }
}
```

Optional **`params`** (object) for tools that need structured arguments, e.g. `goto_location`.

**ApplyToolSequence** (after user accepts a **multi-step** proposal; runs steps **in order**; stops on first failed drone step):

```json
{
  "ApplyToolSequence": {
    "tools": [
      { "category": "drone", "name": "goto_location", "params": { "lat_deg": 37.12, "lon_deg": -122.1, "alt_m": 30 } },
      { "category": "model", "name": "human_detect" }
    ]
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
| `category`        | string \| null | First step category when Infer ran: `"drone"` or `"model"` (for display / single-step ApplyTool). |
| `tool_name`       | string \| null | First step tool name (e.g. `"goto_location"`, `"human_detect"`). |
| `pending_approval` | bool   | When `true`, this is a **proposal** only; frontend shows Accept/Reject. Apply with **ApplyTool** (one step) or **ApplyToolSequence** (when **`tools`** has 2+ entries). |
| `tools`           | array \| omitted | When `pending_approval` is true and the LLM proposed **multiple** steps, ordered `{ "category", "name", "params"? }` objects (max 5). Omitted for single-step proposals. |
| `tool_params`     | object \| omitted | Params for the **first** step when needed (e.g. `goto_location`); also sent with **ApplyTool** on Accept. |
| `llm_response`    | string | Raw LLM response body (or error message). |
| `action_taken`    | string | Short description of what was done (e.g. `"Drone command: move_forward"`, `"override_set"`). |
| `latency_ms`       | number | Total request latency (ms). |
| `llm_latency_ms`   | number | LLM call latency (ms). |

**Example (Infer proposal — pending approval)**

```json
{
  "state": "IDLE",
  "model": null,
  "override_active": false,
  "category": "model",
  "tool_name": "human_detect",
  "pending_approval": true,
  "llm_response": "...",
  "action_taken": "Python worker will activate: human_detect",
  "latency_ms": 120,
  "llm_latency_ms": 95
}
```

**Example (Infer — multi-step proposal)**

```json
{
  "state": "IDLE",
  "model": null,
  "override_active": false,
  "category": "drone",
  "tool_name": "goto_location",
  "pending_approval": true,
  "tools": [
    { "category": "drone", "name": "goto_location", "params": { "lat_deg": 37.12, "lon_deg": -122.1, "alt_m": 30 } },
    { "category": "model", "name": "human_detect" }
  ],
  "tool_params": { "lat_deg": 37.12, "lon_deg": -122.1, "alt_m": 30 },
  "llm_response": "...",
  "action_taken": "Sequence proposal (2 steps): drone:goto_location -> model:human_detect",
  "latency_ms": 120,
  "llm_latency_ms": 95
}
```

**Example (ApplyTool success — tool applied and sent to Python)**

```json
{
  "state": "ACTIVE",
  "model": "vision",
  "override_active": false,
  "category": "model",
  "tool_name": "human_detect",
  "pending_approval": false,
  "llm_response": "",
  "action_taken": "Python worker will activate: human_detect",
  "latency_ms": 12,
  "llm_latency_ms": 0
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
  "pending_approval": false,
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
   - Gateway sends the prompt (with system prompt in `llm.rs`) to the LLM at `http://localhost:8080/v1/chat/completions`.
   - LLM returns JSON: preferred **`{"tasks":[...]}`** (up to **5** steps, each `category` + `name` + optional `params`), or legacy **`{"category":"drone"|"model"|"none","name":"..."}`**.
   - If the result is one or more drone/model steps, the gateway returns **`pending_approval: true`**. For **2+** steps it also returns **`tools`**. It does **not** send to drone/model until the user accepts.
   - If the result is **`none`**, the gateway returns `pending_approval: false` and may set state to IDLE.
3. **ApplyTool** / **ApplyToolSequence**:
   - After the user accepts: **ApplyTool** for a single-step proposal; **ApplyToolSequence** with the full **`tools`** array for multi-step. Gateway runs drone steps via **Drone Server** HTTP in order; on first drone failure it stops and reports **`drone_error`**. Model steps update internal model / placeholder python path. **`active_command`** reflects the **last successful** step (or remains unchanged if the first step fails).
4. **Override**: Sets state to **OVERRIDE_ACTIVE** and forces the given **model** for **timeout_sec**; does not change `active_command` (that stays the last applied tool until cleared).
5. **ClearOverride**: Resets to **IDLE**, clears model and **active_command** (so next **GET /status** returns `active_command: "none"`).
6. **GET /status** never changes state; it returns current state, model, override flag, and **active_command** (last drone or model command, or `"none"`).

---

## Tool names reference

- **model**: `human_detect`, `flood_seg`, `flood_class`
- **drone**: see [src/llm.rs](src/llm.rs) system prompt and [drone-server/](../drone-server/) (e.g. `arm`, `takeoff`, `goto_location`, `circle_search`, `return_to_home`, …)

Use **GET /status** and the **active_command** field to show the current drone or model command in the UI; use **POST /infer** to send user prompts or override/clear-override.
