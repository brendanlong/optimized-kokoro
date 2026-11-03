//! Routes: `POST /v1/audio/speech` and `GET /v1/audio/voices` behind a bearer key, and
//! `GET /health`. Nothing else.

use crate::config::Config;
use crate::pipeline::Pipeline;
use crate::request::{self, Refusal, SpeechRequest};
use crate::stream::{self, StreamError};
use crate::worker::{SynthError, Worker};
use axum::Router;
use axum::Extension;
use axum::body::{Body, Bytes};
use axum::extract::{Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use futures_util::{StreamExt, stream as futures_stream};
use serde_json::json;
use std::sync::Arc;
use subtle::{ConditionallySelectable, ConstantTimeEq};
use tokio::sync::Semaphore;

pub struct AppState {
    config: Config,
    worker: Worker,
    pipeline: Arc<dyn Pipeline>,
    slots: Arc<Semaphore>,
    /// Streams per API key, by the key's position in the config.
    key_slots: Vec<Arc<Semaphore>>,
}

/// Which configured key a request authenticated with.
#[derive(Debug, Clone, Copy)]
struct KeyIndex(usize);

pub fn router(config: Config, worker: Worker, pipeline: Arc<dyn Pipeline>) -> Router {
    let slots = Arc::new(Semaphore::new(config.max_concurrent_requests));
    let key_slots = config.api_keys.iter().map(|_| Arc::new(Semaphore::new(config.max_streams_per_key))).collect();
    let state = Arc::new(AppState { config, worker, pipeline, slots, key_slots });
    let api = Router::new()
        .route("/v1/audio/speech", post(speech))
        .route("/v1/audio/voices", get(voices))
        .route_layer(middleware::from_fn_with_state(state.clone(), require_key));
    Router::new().route("/health", get(health)).merge(api).with_state(state)
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

/// The index of the key equal to `token`, comparing against every key in constant time.
fn find_key(keys: &[String], token: &str) -> Option<usize> {
    let mut found = subtle::Choice::from(0);
    let mut index = 0u64;
    for (i, key) in keys.iter().enumerate() {
        let equal = key.as_bytes().ct_eq(token.as_bytes());
        index.conditional_assign(&(i as u64), equal);
        found |= equal;
    }
    bool::from(found).then_some(index as usize)
}

/// Runs before the handler's extractors, so an unauthenticated request's body is never read.
async fn require_key(State(state): State<Arc<AppState>>, mut request: Request, next: Next) -> Response {
    let token = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split_once(' '))
        .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
        .map(|(_, token)| token);
    let Some(index) = token.filter(|t| !t.is_empty()).and_then(|t| find_key(&state.config.api_keys, t)) else {
        let mut response =
            error(StatusCode::UNAUTHORIZED, "unauthorized", "authentication_error", "A valid API key is required");
        response.headers_mut().insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        return response;
    };
    request.extensions_mut().insert(KeyIndex(index));
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

fn is_length_limit(e: &(dyn std::error::Error + 'static)) -> bool {
    e.is::<http_body_util::LengthLimitError>() || e.source().is_some_and(is_length_limit)
}

/// Reads the body, refusing one over the size limit or slower than the body timeout.
async fn read_body(config: &Config, body: Body) -> Result<Bytes, Response> {
    let limit = config.body_limit();
    match tokio::time::timeout(config.request_body_timeout(), axum::body::to_bytes(body, limit)).await {
        Err(_) => Err(error(
            StatusCode::REQUEST_TIMEOUT,
            "timeout",
            "invalid_request_error",
            "The request body took too long to arrive",
        )),
        Ok(Err(e)) if is_length_limit(&e) => Err(error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "too_large",
            "invalid_request_error",
            format!("request body exceeds {limit} bytes"),
        )),
        Ok(Err(e)) => Err(invalid(format!("reading the request body: {e}"))),
        Ok(Ok(bytes)) => Ok(bytes),
    }
}

async fn speech(State(state): State<Arc<AppState>>, Extension(key): Extension<KeyIndex>, body: Body) -> Response {
    let body = match read_body(&state.config, body).await {
        Ok(body) => body,
        Err(response) => return response,
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
    let Ok(key_permit) = state.key_slots[key.0].clone().try_acquire_owned() else {
        return error(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "rate_limit_error",
            format!("This API key already has {} speech requests in progress", state.config.max_streams_per_key),
        );
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
    let permits = (permit, key_permit);
    let mut audio = stream::start(speech, connection, state.pipeline.clone(), state.config.lookahead, permits);

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
