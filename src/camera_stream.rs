//! Proxy MJPEG camera stream from the model server to the frontend.

use std::sync::OnceLock;

use axum::body::Body;
use axum::http::{header, StatusCode};
use axum::response::Response;
use futures_util::StreamExt;
use reqwest::Client;

fn stream_client() -> &'static Client {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        Client::builder()
            .build()
            .expect("camera stream reqwest client")
    })
}

pub async fn proxy_camera_stream(upstream_url: &str) -> Response {
    let send = stream_client().get(upstream_url).send().await;

    match send {
        Ok(resp) => {
            if !resp.status().is_success() {
                return Response::builder()
                    .status(StatusCode::BAD_GATEWAY)
                    .body(Body::from("camera_stream_upstream_error"))
                    .unwrap_or_else(|_| Response::new(Body::empty()));
            }

            let content_type = resp
                .headers()
                .get(header::CONTENT_TYPE)
                .cloned()
                .unwrap_or_else(|| {
                    header::HeaderValue::from_static(
                        "multipart/x-mixed-replace; boundary=frame",
                    )
                });

            let stream = resp.bytes_stream().map(|chunk| {
                chunk.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
            });

            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, content_type)
                .header(header::CACHE_CONTROL, "no-cache, no-store, must-revalidate")
                .header(header::PRAGMA, "no-cache")
                .body(Body::from_stream(stream))
                .unwrap_or_else(|_| Response::new(Body::empty()))
        }
        Err(e) => Response::builder()
            .status(StatusCode::BAD_GATEWAY)
            .body(Body::from(format!("camera_stream_proxy_failed: {e}")))
            .unwrap_or_else(|_| Response::new(Body::empty())),
    }
}
