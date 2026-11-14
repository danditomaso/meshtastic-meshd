//! HTTP API: device list as JSON and Server-Sent Events.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::http::{HeaderName, HeaderValue, Method};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::get;
use axum::{Json, Router};
use futures::Stream;
use serde::Serialize;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::WatchStream;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::trace::TraceLayer;

use crate::registry::{Device, Registry};

pub fn router(registry: Arc<Registry>, allowed_origins: &[String]) -> Router {
    Router::new()
        .route("/api/devices", get(list_devices))
        .route("/api/devices/stream", get(stream_devices))
        .route("/healthz", get(healthz))
        .layer(cors(allowed_origins))
        .layer(TraceLayer::new_for_http())
        .with_state(registry)
}

/// Lets clients confirm they're talking to meshd.
#[derive(Serialize)]
pub struct Health {
    service: &'static str,
    api: u32,
    version: &'static str,
}

/// Bump on breaking API changes.
pub const API_VERSION: u32 = 1;

async fn healthz() -> Json<Health> {
    Json(Health {
        service: "meshd",
        api: API_VERSION,
        version: env!("CARGO_PKG_VERSION"),
    })
}

async fn list_devices(State(registry): State<Arc<Registry>>) -> Json<Vec<Device>> {
    Json(registry.devices())
}

/// Sends the full device list on connect and after each change.
async fn stream_devices(
    State(registry): State<Arc<Registry>>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let stream = WatchStream::new(registry.subscribe()).map(move |_| {
        let event = Event::default()
            .event("devices")
            .json_data(registry.devices())
            .unwrap_or_else(|_| Event::default().event("error"));
        Ok(event)
    });
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

/// Allows the web client's origins, including Chrome's private network preflight.
fn cors(allowed_origins: &[String]) -> CorsLayer {
    let origins: Vec<HeaderValue> = allowed_origins
        .iter()
        .filter_map(|o| HeaderValue::from_str(o).ok())
        .collect();
    CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_methods([Method::GET])
        .allow_headers([HeaderName::from_static("authorization")])
        .allow_private_network(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn healthz_identifies_meshd() {
        let Json(health) = healthz().await;
        let json = serde_json::to_value(health).unwrap();
        assert_eq!(json["service"], "meshd");
        assert_eq!(json["api"], API_VERSION);
        assert_eq!(json["version"], env!("CARGO_PKG_VERSION"));
    }
}
