# Jetson LLM Gateway

HTTP API gateway for SAR (Search and Rescue) drone control. It accepts natural-language or structured commands, calls an LLM to decide a **drone** or **model** tool, and exposes status (including the currently active command).

## Overview

- **Server**: Axum HTTP server on `http://0.0.0.0:3000` (CORS enabled for all origins).
- **LLM**: Sends prompts to `http://localhost:8080/v1/chat/completions` (e.g. local LLM server). The LLM returns a single JSON tool call: `{"category": "drone"|"model", "name": "<tool_name>"}`. The gateway returns proposals with `pending_approval: true`; the frontend shows Accept/Reject and only **ApplyTool** (after user accepts) applies the tool and sends to Python.
- **States**: `IDLE`, `ACTIVE` (last applied tool), `OVERRIDE_ACTIVE` (manual model override for a timeout).

## Build & Run

```bash
cargo build
cargo run
```

Server listens on port **3000**.

## API Summary

| Method | Path    | Description |
|--------|--------|-------------|
| `GET`  | `/status` | Current state, model, override flag, **active command** (drone/model tool or `none`), and latency/memory. |
| `POST` | `/infer`  | Send a command (infer with prompt, override model, or clear override). Returns state, model, override, category/tool name, LLM response, action taken, and latencies. |

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
- **ApplyTool** (after user accepts on frontend; applies tool and sends to Python when category is `"model"`):  
  `{"ApplyTool": {"category": "model", "tool_name": "activate_human_detection_yolo"}}`
- **Override** (force model for a period):  
  `{"Override": {"model": "vision", "timeout_sec": 60}}` — `timeout_sec` optional (default 60)
- **ClearOverride**:  
  `{"ClearOverride": null}`

Response: JSON with **state**, **model**, **override_active**, **category**, **tool_name**, **pending_approval**, **llm_response**, **action_taken**, **latency_ms**, **llm_latency_ms**. See **AGENTS.md** for exact shapes.

## Tool Names (reference)

- **Model**: `activate_human_detection_yolo`, `activate_flood_segmentation`, `activate_human_behaviour_analysis`, `share_with_swarm`, `activate_flood_classification`
- **Drone**: `move_forward`, `hover`, `return_to_home`, `land_immediately`, `circle_search`

For full request/response contracts and examples for frontend or agent use, see **AGENTS.md**.
