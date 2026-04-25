//! Jetson LLM Gateway — SAR + Drone controller with HTTP API.
//! Main entrypoint: startup telemetry and persistent Axum server.

mod config;
mod types;
mod llm;
mod orchestrator;
mod server;

use clap::Parser;
use reqwest::Client;
use tokio::sync::Mutex;
use tracing::info;
use tracing_subscriber::{fmt::format::FmtSpan, prelude::*, EnvFilter};

use crate::orchestrator::Orchestrator;
use crate::server::AppState;
use crate::types::GatewayState;

/// Jetson LLM Gateway — CLI-only entrypoint, now primarily starting HTTP server.
#[derive(Parser, Debug)]
#[command(name = "gateway", about = "Jetson LLM Gateway", version)]
struct Args {
    /// Reserved for future quick CLI tests (currently unused).
    #[arg(long)]
    _input: Option<String>,
}

/// Best-effort current process RSS in MB (Linux /proc/self/status).
fn memory_estimate_mb() -> Option<f64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        if line.starts_with("VmRSS:") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            let kb: u64 = parts.get(1)?.parse().ok()?;
            return Some(kb as f64 / 1024.0);
        }
    }
    None
}

/// Rust version string from rustc if available.
fn rust_version() -> String {
    std::process::Command::new("rustc")
        .arg("--version")
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                String::from_utf8(o.stdout).ok()
            } else {
                None
            }
        })
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "Rust (version unknown)".to_string())
}

#[tokio::main]
async fn main() {
    let _args = Args::parse();

    // Tracing: pretty console + structured fields (env RUST_LOG controls level).
    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_target(true)
        .with_thread_ids(false)
        .with_span_events(FmtSpan::NEW | FmtSpan::CLOSE)
        .with_ansi(true);

    tracing_subscriber::registry()
        .with(EnvFilter::from_default_env().add_directive(tracing::Level::INFO.into()))
        .with(fmt_layer)
        .init();

    let _guard = tracing::info_span!("gateway_startup").entered();

    let mem_mb = memory_estimate_mb().unwrap_or(0.0);
    let rust_ver = rust_version();
    let ts = chrono_lite();

    // Startup banner
    eprintln!("╔══════════════════════════════════════════════════════════════╗");
    eprintln!("║  Jetson LLM Gateway — CLI (Step 6 — Tool Actions + Raw LLM Reply) ║");
    eprintln!("╠══════════════════════════════════════════════════════════════╣");
    eprintln!("║  Started: {:<50} ║", truncate(&ts, 50));
    eprintln!("║  Rust:    {:<48} ║", truncate(&rust_ver, 48));
    eprintln!("║  Memory:  ~{:.2} MB (initial estimate)                       ║", mem_mb);
    eprintln!("╚══════════════════════════════════════════════════════════════╝");

    info!(
        state = ?GatewayState::IDLE,
        action = "startup",
        latency_ms = 0u64,
        reason = "initial boot",
        memory_estimate_mb = mem_mb,
        rust_version = %rust_ver,
    );

    let llm_url = crate::config::llm_chat_completions_url();
    let drone_url = crate::config::drone_apply_tool_url();
    let drone_health = crate::config::drone_health_url();
    info!(
        action = "startup_config",
        llm_chat_completions_url = %llm_url,
        drone_apply_tool_url = %drone_url,
        drone_health_url = %drone_health,
        reason = "set LLM_BASE_URL / DRONE_SERVER_URL env vars to override defaults"
    );
    eprintln!("LLM (OpenAI-compatible): {}", llm_url);
    eprintln!("Drone HTTP apply-tool:  {}", drone_url);
    eprintln!("Drone HTTP health:      {}", drone_health);

    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(125))
        .build()
        .expect("reqwest client");
    let orchestrator = Orchestrator::new();
    let state = AppState {
        orchestrator: std::sync::Arc::new(Mutex::new(orchestrator)),
        client,
    };

    // Step 7 will make real gRPC call to python-worker from the /infer handler.
    crate::server::run_http_server(state).await;
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        format!("{:<width$}", s, width = max)
    } else {
        format!("{}…", &s[..max.saturating_sub(1)])
    }
}

/// Simple timestamp (no extra dep).
fn chrono_lite() -> String {
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = t.as_secs();
    let millis = t.subsec_millis();
    let (s, m, h, _d) = (
        secs % 60,
        (secs / 60) % 60,
        (secs / 3600) % 24,
        secs / 86400,
    );
    format!("{:02}:{:02}:{:02}.{:03} UTC (epoch+{})", h, m, s, millis, secs)
}
