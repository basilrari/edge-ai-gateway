//! Relay browser WebSocket ↔ drone-http combined logs stream.

use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use tracing::warn;

pub async fn relay_logs_ws(mut client: WebSocket, drone_ws_url: String) {
    let drone_connect = match tokio_tungstenite::connect_async(&drone_ws_url).await {
        Ok(pair) => pair,
        Err(e) => {
            warn!(error = %e, url = %drone_ws_url, "drone logs ws connect failed");
            let _ = client
                .send(Message::Text(
                    format!(r#"{{"type":"error","detail":"drone_logs_ws_connect_failed: {e}"}}"#)
                        .into(),
                ))
                .await;
            let _ = client.close().await;
            return;
        }
    };

    let (mut drone_sink, mut drone_stream) = drone_connect.0.split();
    let (mut client_sink, mut client_stream) = client.split();

    let client_to_drone = async {
        while let Some(msg) = client_stream.next().await {
            match msg {
                Ok(Message::Close(_)) => break,
                Ok(Message::Ping(p)) => {
                    if drone_sink
                        .send(tokio_tungstenite::tungstenite::Message::Ping(p))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Ok(Message::Pong(p)) => {
                    if drone_sink
                        .send(tokio_tungstenite::tungstenite::Message::Pong(p))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Ok(other) => {
                    let wire = match other {
                        Message::Text(t) => tokio_tungstenite::tungstenite::Message::Text(t),
                        Message::Binary(b) => tokio_tungstenite::tungstenite::Message::Binary(b),
                        _ => continue,
                    };
                    if drone_sink.send(wire).await.is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    };

    let drone_to_client = async {
        while let Some(msg) = drone_stream.next().await {
            match msg {
                Ok(tokio_tungstenite::tungstenite::Message::Text(t)) => {
                    if client_sink.send(Message::Text(t)).await.is_err() {
                        break;
                    }
                }
                Ok(tokio_tungstenite::tungstenite::Message::Binary(b)) => {
                    if client_sink.send(Message::Binary(b)).await.is_err() {
                        break;
                    }
                }
                Ok(tokio_tungstenite::tungstenite::Message::Close(_)) => break,
                Ok(tokio_tungstenite::tungstenite::Message::Ping(p)) => {
                    if client_sink.send(Message::Ping(p)).await.is_err() {
                        break;
                    }
                }
                Ok(tokio_tungstenite::tungstenite::Message::Pong(p)) => {
                    if client_sink.send(Message::Pong(p)).await.is_err() {
                        break;
                    }
                }
                Err(_) => break,
                _ => {}
            }
        }
    };

    tokio::select! {
        _ = client_to_drone => {}
        _ = drone_to_client => {}
    }
}
