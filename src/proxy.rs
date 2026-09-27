//! Forward `/v1/*` to the engine after the runtime scaffold has checked the
//! bearer token. Request bodies are buffered (the engine's own cap is 96 MB,
//! which covers image prompts). Responses stream, so token SSE is not held
//! until generation finishes.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use axum::Router;
use futures_util::TryStreamExt;
use http_body_util::BodyExt;
use sen_runtime_sdk::api::{codes, ErrorBody};
use sen_runtime_sdk::server::Readiness;

use crate::engine::EngineHandle;

const MAX_BODY: usize = 96 * 1024 * 1024;

pub struct ProxyState {
    client: reqwest::Client,
    engine: EngineHandle,
    readiness: Readiness,
}

pub fn router(engine: EngineHandle, readiness: Readiness) -> Router {
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(5))
        .build()
        .expect("reqwest client");
    let state = Arc::new(ProxyState { client, engine, readiness });
    Router::new()
        .route("/v1", any(forward))
        .route("/v1/*path", any(forward))
        .layer(axum::extract::DefaultBodyLimit::max(MAX_BODY))
        .with_state(state)
}

async fn forward(State(state): State<Arc<ProxyState>>, req: Request) -> Response {
    if !state.readiness.is_ready() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(ErrorBody::with_code("model is still loading", codes::LOADING)),
        )
            .into_response();
    }
    let path = req.uri().path_and_query().map(|p| p.as_str()).unwrap_or("/");
    let url = format!("{}{path}", state.engine.base);
    let method = match reqwest::Method::from_bytes(req.method().as_str().as_bytes()) {
        Ok(m) => m,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                axum::Json(ErrorBody::new("unsupported method")),
            )
                .into_response();
        }
    };
    let headers = forward_headers(req.headers());
    let (parts, body) = req.into_parts();
    let _ = parts;
    let collected = match body.collect().await {
        Ok(c) => c.to_bytes(),
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                axum::Json(ErrorBody::new(format!("could not read the request body: {e}"))),
            )
                .into_response();
        }
    };
    if collected.len() > MAX_BODY {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            axum::Json(ErrorBody::new("request body exceeds 96 MB")),
        )
            .into_response();
    }

    let upstream = match state
        .client
        .request(method, &url)
        .headers(headers)
        .body(collected)
        .send()
        .await
    {
        Ok(resp) => resp,
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                axum::Json(ErrorBody::new(format!("TurboFieldfareServer: {e}"))),
            )
                .into_response();
        }
    };

    let status = StatusCode::from_u16(upstream.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let mut builder = Response::builder().status(status);
    if let Some(headers) = builder.headers_mut() {
        copy_response_headers(upstream.headers(), headers);
    }
    let stream = upstream.bytes_stream().map_err(|e| std::io::Error::other(e));
    builder.body(Body::from_stream(stream)).unwrap_or_else(|_| {
        (StatusCode::BAD_GATEWAY, axum::Json(ErrorBody::new("could not stream the engine response"))).into_response()
    })
}

fn forward_headers(src: &HeaderMap) -> reqwest::header::HeaderMap {
    let mut out = reqwest::header::HeaderMap::new();
    for (name, value) in src.iter() {
        if matches!(
            name.as_str(),
            "host" | "authorization" | "connection" | "content-length" | "transfer-encoding"
        ) {
            continue;
        }
        let Ok(n) = reqwest::header::HeaderName::from_bytes(name.as_str().as_bytes()) else {
            continue;
        };
        let Ok(v) = reqwest::header::HeaderValue::from_bytes(value.as_bytes()) else {
            continue;
        };
        out.append(n, v);
    }
    out
}

fn copy_response_headers(src: &reqwest::header::HeaderMap, dst: &mut HeaderMap) {
    for (name, value) in src.iter() {
        if matches!(name.as_str(), "connection" | "transfer-encoding" | "content-length") {
            continue;
        }
        let Ok(n) = header::HeaderName::from_bytes(name.as_str().as_bytes()) else {
            continue;
        };
        let Ok(v) = header::HeaderValue::from_bytes(value.as_bytes()) else {
            continue;
        };
        dst.append(n, v);
    }
}
