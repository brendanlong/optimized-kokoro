//! Routes: `POST /v1/audio/speech` and `GET /v1/audio/voices` behind a bearer key, and
//! `GET /health`. Nothing else.

use crate::config::Config;
use crate::pipeline::Pipeline;
use crate::request::{self, Refusal, SpeechRequest};
use crate::stream::{self, StreamError};
use crate::worker::{SynthError, Worker};
use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::rejection::BytesRejection;
use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use futures_util::{StreamExt, stream as futures_stream};
use serde_json::json;
use std::sync::Arc;
use subtle::ConstantTimeEq;
use tokio::sync::Semaphore;

pub struct AppState {
    config: Config,
    worker: Worker,
    pipeline: Arc<dyn Pipeline>,
    slots: Arc<Semaphore>,
}

pub fn router(config: Config, worker: Worker, pipeline: Arc<dyn Pipeline>) -> Router {
    let body_limit = config.body_limit();
    let slots = Arc::new(Semaphore::new(config.max_concurrent_requests));
    let state = Arc::new(AppState { config, worker, pipeline, slots });
    let api = Router::new()
        .route("/v1/audio/speech", post(speech))
        .route("/v1/audio/voices", get(voices))
        .route_layer(middleware::from_fn_with_state(state.clone(), require_key));
    Router::new()
        .route("/health", get(health))
        .merge(api)
        .layer(DefaultBodyLimit::max(body_limit))
        .with_state(state)
}

fn error(status: StatusCode, code: &str, kind: &str, message: impl Into<String>) -> Response {
    let body = json!({ "detail": { "error": code, "message": message.into(), "type": kind } });
    (status, axum::Json(body)).into_response()
}

fn invalid(message: impl Into<String>) -> Response {
    error(StatusCode::BAD_REQUEST, "validation_error", "invalid_request_error", message)
}

fn unavailable(message: &str) -> Response {
    error(StatusCode::SERVICE_UNAVAILABLE, "unavailable", "server_error", message)
}

fn key_matches(keys: &[String], token: &str) -> bool {
    let mut matched = subtle::Choice::from(0);
    for key in keys {
        matched |= key.as_bytes().ct_eq(token.as_bytes());
    }
    matched.into()
}

/// Runs before the handler's extractors, so an unauthenticated request's body is never read.
async fn require_key(State(state): State<Arc<AppState>>, request: Request, next: Next) -> Response {
    let token = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split_once(' '))
        .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
        .map(|(_, token)| token);
    if !token.is_some_and(|t| !t.is_empty() && key_matches(&state.config.api_keys, t)) {
        let mut response =
            error(StatusCode::UNAUTHORIZED, "unauthorized", "authentication_error", "A valid API key is required");
        response.headers_mut().insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        return response;
    }
    next.run(request).await
}

async fn health(State(state): State<Arc<AppState>>) -> Response {
    match state.worker.current() {
        Some(_) => axum::Json(json!({ "status": "healthy" })).into_response(),
        None => (StatusCode::SERVICE_UNAVAILABLE, axum::Json(json!({ "status": "worker unavailable" }))).into_response(),
    }
}

async fn voices(State(state): State<Arc<AppState>>) -> Response {
    match state.worker.current() {
        Some(connection) => axum::Json(json!({ "voices": connection.voices() })).into_response(),
        None => unavailable("The model worker is not connected"),
    }
}

fn stream_failure(e: &StreamError) -> Response {
    match e {
        StreamError::Synth(SynthError::ConnectionLost) => unavailable("Lost the connection to the model worker"),
        _ => error(StatusCode::INTERNAL_SERVER_ERROR, "processing_error", "server_error", e.to_string()),
    }
}

async fn speech(State(state): State<Arc<AppState>>, body: Result<Bytes, BytesRejection>) -> Response {
    let body = match body {
        Ok(body) => body,
        Err(rejection) if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE => {
            return error(
                StatusCode::PAYLOAD_TOO_LARGE,
                "too_large",
                "invalid_request_error",
                format!("request body exceeds {} bytes", state.config.body_limit()),
            );
        }
        Err(rejection) => return invalid(rejection.body_text()),
    };
    let request: SpeechRequest = match serde_json::from_slice(&body) {
        Ok(request) => request,
        Err(e) => return invalid(e.to_string()),
    };
    if let Err(refusal) = request::check_fields(&request, &state.config) {
        return refused(refusal);
    }
    let Some(connection) = state.worker.current() else {
        return unavailable("The model worker is not connected");
    };
    let Ok(permit) = state.slots.clone().try_acquire_owned() else {
        return unavailable("Too many requests in progress");
    };

    let planned = {
        let state = state.clone();
        let connection = connection.clone();
        tokio::task::spawn_blocking(move || {
            request::plan(request, &state.config, &connection, state.pipeline.as_ref())
        })
        .await
        .expect("planning doesn't panic")
    };
    let speech = match planned {
        Ok(speech) => speech,
        Err(refusal) => return refused(refusal),
    };
    let mut audio = stream::start(speech, connection, state.pipeline.clone(), state.config.lookahead, permit);

    // Headers wait for the first audio, so a request that fails before any is made gets a status.
    let first = match audio.next().await {
        None => Bytes::new(),
        Some(Err(e)) => return stream_failure(&e),
        Some(Ok(first)) => first,
    };
    let rest = futures_stream::unfold(audio, |mut audio| async move { audio.next().await.map(|item| (item, audio)) });
    let body = Body::from_stream(futures_stream::once(async { Ok(first) }).chain(rest));
    (
        [
            (header::CONTENT_TYPE, "audio/pcm"),
            (header::CACHE_CONTROL, "no-cache"),
            (header::HeaderName::from_static("x-accel-buffering"), "no"),
        ],
        body,
    )
        .into_response()
}

fn refused(refusal: Refusal) -> Response {
    match refusal {
        Refusal::Invalid(message) => invalid(message),
        Refusal::Forbidden(message) => error(StatusCode::FORBIDDEN, "permission_denied", "permission_error", message),
    }
}
