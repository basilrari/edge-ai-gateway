//! ICE server config for browser WebRTC (TURN for remote internet viewing).

use serde_json::{json, Value};

fn split_turn_urls(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

fn collect_turn_urls() -> Vec<String> {
    let mut urls = Vec::new();
    if let Ok(multi) = std::env::var("WEBRTC_TURN_URLS") {
        urls.extend(split_turn_urls(&multi));
    }
    if let Ok(single) = std::env::var("WEBRTC_TURN_URL") {
        let s = single.trim();
        if !s.is_empty() && !urls.iter().any(|u| u == s) {
            urls.push(s.to_string());
        }
    }
  let host = std::env::var("WEBRTC_PUBLIC_HOST")
        .ok()
        .filter(|s| !s.trim().is_empty());
    if urls.is_empty() && host.is_some() {
        let h = host.unwrap();
        urls.push(format!("turn:{h}:3478?transport=udp"));
        urls.push(format!("turn:{h}:3478?transport=tcp"));
    }
    urls
}

/// JSON for `GET /camera/webrtc/ice` (browser RTCPeerConnection config).
pub fn webrtc_ice_json() -> Value {
    let mut ice_servers: Vec<Value> = Vec::new();

    let stun = std::env::var("WEBRTC_STUN_URL")
        .unwrap_or_else(|_| "stun:stun.l.google.com:19302".to_string());
    if !stun.trim().is_empty() {
        ice_servers.push(json!({ "urls": stun.trim() }));
    }

    let turn_urls = collect_turn_urls();
    let user = std::env::var("WEBRTC_TURN_USERNAME").unwrap_or_default();
    let cred = std::env::var("WEBRTC_TURN_PASSWORD").unwrap_or_default();

    for url in &turn_urls {
        if user.is_empty() {
            ice_servers.push(json!({ "urls": url }));
        } else {
            ice_servers.push(json!({
                "urls": url,
                "username": user,
                "credential": cred,
            }));
        }
    }

    let policy = std::env::var("WEBRTC_ICE_TRANSPORT_POLICY").unwrap_or_else(|_| {
        if turn_urls.is_empty() {
            "all".to_string()
        } else {
            "relay".to_string()
        }
    });

    json!({
        "iceServers": ice_servers,
        "iceTransportPolicy": policy,
        "turn_configured": !turn_urls.is_empty(),
    })
}
