# Jetson LLM Gateway

HTTP API gateway for SAR (Search and Rescue) drone control. It accepts natural-language or structured commands, calls an LLM to decide **drone** and/or **model** tools (including **ordered multi-step** plans), and exposes status (including the currently active command).

## How this ties into the project

The Gateway is the **central router** in the [SAR drone architecture](../README.md). The **frontend** sends user prompts (and **ApplyTool** / **ApplyToolSequence** after the user accepts) to this server. The Gateway calls the **LLM** to get structured tool call(s); when the user accepts, it applies them in order. **Model** tools are forwarded to the **Model Server** ([Drone_LLM](../Drone_LLM/)) for flood segmentation, flood classification, human detection. **Drone** tools are forwarded to the **Drone Server** ([drone-server/](../drone-server/)) via `drone-http` (`POST /v1/apply-tool`). The Gateway does not talk MAVLink itself; it delegates drone actions to the Drone Server.

## Overview

- **Server**: Axum HTTP server on `http://127.0.0.1:3000` (loopback; set `GATEWAY_BIND` e.g. `0.0.0.0:3000` to override). CORS enabled for all origins.
- **Auth**: Mutating routes require `MCP_API_KEY` via `Authorization: Bearer` or `X-API-Key` (503 if unset, 401 if mismatch): `POST /infer`, `/drone/mission/upload`, `/drone/mission/clear`, `/drone/logs/clear`, `/logs/clear-all`, `/logs/llm/clear`. Same check as `/mcp/*`.
- **ACK**: drone steps wait for FC `COMMAND_ACK` unless `DRONE_WAIT_FOR_ACK` is explicitly `0` / `false`.
- **LLM**: Sends prompts to `http://localhost:8080/v1/chat/completions` (e.g. local LLM server). System prompt is **`SAR_SYSTEM_PROMPT`** in `src/llm.rs` (example-first JSON `tasks` router, max **5** steps). The LLM returns `{"tasks":[...]}`. Responses include **`pending_approval`**: when **`true`**, the client should show approval before **ApplyTool** / **ApplyToolSequence**; today Infer **auto-applies** and returns **`false`**. Multi-step plans include **`tools`**: an ordered array of steps.
- **States**: `IDLE`, `ACTIVE` (last applied tool), `OVERRIDE_ACTIVE` (manual model override for a timeout).

## Build & Run

```bash
cargo build
cargo run
```

Server listens on **127.0.0.1:3000** unless `GATEWAY_BIND` is set. `POST /infer` needs `MCP_API_KEY`.

## API Summary

| Method | Path    | Description |
|--------|--------|-------------|
| `GET`  | `/status` | Current state, model, override flag, **active command** (drone/model tool or `none`), and latency/memory. |
| `POST` | `/infer`  | Auth (`MCP_API_KEY`). Infer, ApplyTool, ApplyToolSequence, override, or clear override. |
| `GET`  | `/drone/position` | Proxies drone-http `GET /v1/position` (lat/lon from last `GLOBAL_POSITION_INT`) for the frontend map. |

### GET /status

No request body. Returns JSON with:

- **state**: `"IDLE"` \| `"ACTIVE"` \| `"OVERRIDE_ACTIVE"` \| `"SWITCHING"`
- **model**: Current model name (e.g. `"vision"`, `"text"`) or `"none"`
- **override_active**: `true` if a time-limited override is in effect
- **active_command**: Last selected command: `"drone: <tool_name>"`, `"model: <tool_name>"`, or `"none"` if none has been set
- **latency_ms**, **llm_latency_ms**, **memory_estimate_mb**

### POST /infer

Request body: JSON matching one of these shapes:

- **Infer** (run LLM; returns proposal with optional `pending_approval: true`):  
  `{"Infer": {"prompt": "user message"}}`
- **ApplyTool** (after user accepts a **single-step** proposal):  
  `{"ApplyTool": {"category": "model", "tool_name": "human_detect"}}`  
  Optional: `"params": { ... }` for tools that need structured args (e.g. `goto_location`).
- **ApplyToolSequence** (after user accepts a **multi-step** proposal):  
  `{"ApplyToolSequence": {"tools": [{"category":"drone","name":"goto_location","params":{"lat_deg":0,"lon_deg":0,"alt_m":10}},{"category":"model","name":"human_detect"}]}}`
- **Override** (force model for a period):  
  `{"Override": {"model": "vision", "timeout_sec": 60}}` — `timeout_sec` optional (default 60)
- **ClearOverride**:  
  `{"ClearOverride": null}`

Response: JSON with **state**, **model**, **override_active**, **category**, **tool_name**, **pending_approval**, optional **`tools`** (proposal steps), **llm_response**, **action_taken**, **latency_ms**, **llm_latency_ms**. See **AGENTS.md** for exact shapes.

## Tool Names (reference)

- **Model**: `human_detect`, `flood_seg`, `flood_class` (short names; see `llm.rs` system prompt).
- **Drone**: `arm`, `disarm`, `takeoff`, `goto_location`, `start_mission`, `mission_set_current`, `mission_interrupt`, `mission_resume`, `return_to_home`, `land_immediately`, `hover`, `set_mode_guided`, `set_mode_auto` (full list in `SAR_SYSTEM_PROMPT` in `src/llm.rs`; drone-http may expose additional tools not offered to the LLM).

For full request/response contracts and examples for frontend or agent use, see **AGENTS.md**.
