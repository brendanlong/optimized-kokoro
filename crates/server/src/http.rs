//! Routes: `POST /v1/audio/speech` and `GET /v1/audio/voices` behind a bearer key, and
//! `GET /health`. Nothing else.

use crate::config::{ApiKey, Config};
use crate::load::{Load, Shed};
use crate::pipeline::{Pipeline, voice_lang};
use crate::request::{self, SpeechRequest};
use crate::stream::{self, StreamError};
use crate::worker::{SynthError, Worker};
use bytes::Bytes;
use futures_util::{StreamExt, TryStreamExt, stream as futures_stream};
use http_body_util::combinators::UnsyncBoxBody;
use http_body_util::{BodyExt, Full, Limited, StreamBody};
use hyper::body::Frame;
use hyper::header::{self, HeaderValue};
use hyper::{Method, Request, Response, StatusCode};
use serde_json::json;
use std::sync::Arc;
use subtle::{ConditionallySelectable, ConstantTimeEq};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Every response's body: a whole JSON document, or streamed PCM.
pub type Body = UnsyncBoxBody<Bytes, BoxError>;

struct AppState {
    config: Config,
    worker: Worker,
    pipeline: Arc<dyn Pipeline>,
    slots: Arc<Semaphore>,
    /// Streams per API key, by the key's position in the config.
    key_slots: Vec<Arc<Semaphore>>,
    load: Arc<Load>,
}

/// How long a request waits for the permit of a less urgent request it displaced, which frees it
/// as soon as that request's text processing notices (milliseconds, normally).
const DISPLACE_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

/// The HTTP application: cheap to clone, one per connection.
#[derive(Clone)]
pub struct App(Arc<AppState>);

/// Which configured key a request authenticated with.
#[derive(Debug, Clone, Copy)]
struct KeyIndex(usize);

pub fn router(config: Config, worker: Worker, pipeline: Arc<dyn Pipeline>) -> App {
    let slots = Arc::new(Semaphore::new(config.max_concurrent_requests));
    let key_slots = (0..config.api_keys.len()).map(|i| Arc::new(Semaphore::new(config.max_streams(i)))).collect();
    let load = Load::new(config.latency_budget());
    App(Arc::new(AppState { config, worker, pipeline, slots, key_slots, load }))
}

impl App {
    /// Routes one request. Unknown paths get 404 and known paths with another method 405, both
    /// before the key is checked; the key is checked before anything reads the body.
    pub async fn handle<B>(&self, request: Request<B>) -> Response<Body>
    where
        B: hyper::body::Body<Data = Bytes>,
        B::Error: Into<BoxError>,
    {
        let state = &self.0;
        // HEAD is answered as GET; hyper leaves out the body.
        let get = matches!(*request.method(), Method::GET | Method::HEAD);
        match request.uri().path() {
            "/health" if get => health(state),
            "/health" => not_allowed("GET,HEAD"),
            "/v1/audio/voices" if get => match authenticate(state, &request) {
                Some(_) => voices(state),
                None => unauthorized(),
            },
            "/v1/audio/voices" => not_allowed("GET,HEAD"),
            "/v1/audio/speech" if request.method() == Method::POST => match authenticate(state, &request) {
                Some(key) => speech(state, key, request.into_body()).await,
                None => unauthorized(),
            },
            "/v1/audio/speech" => not_allowed("POST"),
            _ => empty(StatusCode::NOT_FOUND),
        }
    }
}

fn empty(status: StatusCode) -> Response<Body> {
    let mut response = Response::new(Full::new(Bytes::new()).map_err(|e| match e {}).boxed_unsync());
    *response.status_mut() = status;
    response
}

fn not_allowed(allow: &'static str) -> Response<Body> {
    let mut response = empty(StatusCode::METHOD_NOT_ALLOWED);
    response.headers_mut().insert(header::ALLOW, HeaderValue::from_static(allow));
    response
}

fn json_response(status: StatusCode, body: serde_json::Value) -> Response<Body> {
    let bytes = serde_json::to_vec(&body).expect("a JSON value serializes");
    let mut response = Response::new(Full::new(Bytes::from(bytes)).map_err(|e| match e {}).boxed_unsync());
    *response.status_mut() = status;
    response.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
    response
}

fn error(status: StatusCode, code: &str, kind: &str, message: impl Into<String>) -> Response<Body> {
    json_response(status, json!({ "detail": { "error": code, "message": message.into(), "type": kind } }))
}

fn invalid(message: impl Into<String>) -> Response<Body> {
    error(StatusCode::BAD_REQUEST, "validation_error", "invalid_request_error", message)
}

fn unavailable(message: &str) -> Response<Body> {
    error(StatusCode::SERVICE_UNAVAILABLE, "unavailable", "server_error", message)
}

/// The server is too busy to serve this request soon: 503 with `Retry-After: 1`, before any
/// audio. Clients should send this request to another provider rather than retry it here.
fn overloaded(message: impl Into<String>) -> Response<Body> {
    let mut response = error(StatusCode::SERVICE_UNAVAILABLE, "overloaded", "server_error", message);
    response.headers_mut().insert(header::RETRY_AFTER, HeaderValue::from_static("1"));
    response
}

/// The index of the key equal to `token`, comparing against every key in constant time.
fn find_key(keys: &[ApiKey], token: &str) -> Option<usize> {
    let mut found = subtle::Choice::from(0);
    let mut index = 0u64;
    for (i, key) in keys.iter().enumerate() {
        let equal = key.key.as_bytes().ct_eq(token.as_bytes());
        index.conditional_assign(&(i as u64), equal);
        found |= equal;
    }
    bool::from(found).then_some(index as usize)
}

/// The key a request's bearer token matches. Checked before the handler reads the body, so the app
/// never reads an unauthenticated request's body. hyper may already have buffered some of it while
/// parsing the headers (bounded by its read buffer, see `serve::MAX_BUF`), and it doesn't send
/// `100 Continue` until the body is read.
fn authenticate<B>(state: &AppState, request: &Request<B>) -> Option<KeyIndex> {
    let token = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split_once(' '))
        .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
        .map(|(_, token)| token);
    token.filter(|t| !t.is_empty()).and_then(|t| find_key(&state.config.api_keys, t)).map(KeyIndex)
}

fn unauthorized() -> Response<Body> {
    let mut response =
        error(StatusCode::UNAUTHORIZED, "unauthorized", "authentication_error", "A valid API key is required");
    response.headers_mut().insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
    response
}

fn health(state: &AppState) -> Response<Body> {
    match state.worker.current() {
        Some(_) => json_response(StatusCode::OK, json!({ "status": "healthy" })),
        None => json_response(StatusCode::SERVICE_UNAVAILABLE, json!({ "status": "worker unavailable" })),
    }
}

/// The worker's voices whose language this server speaks (the others would only get a 400).
fn voices(state: &AppState) -> Response<Body> {
    match state.worker.current() {
        Some(connection) => {
            let voices: Vec<&str> =
                connection.voices().iter().map(String::as_str).filter(|v| voice_lang(v).is_some()).collect();
            json_response(StatusCode::OK, json!({ "voices": voices }))
        }
        None => unavailable("The model worker is not connected"),
    }
}

fn stream_failure(e: &StreamError) -> Response<Body> {
    match e {
        StreamError::Synth(SynthError::ConnectionLost) => unavailable("Lost the connection to the model worker"),
        StreamError::TooMuchSpeech { .. } => invalid(e.to_string()),
        StreamError::Shed(_) => overloaded(e.to_string()),
        _ => error(StatusCode::INTERNAL_SERVER_ERROR, "processing_error", "server_error", e.to_string()),
    }
}

fn is_length_limit(e: &(dyn std::error::Error + 'static)) -> bool {
    e.is::<http_body_util::LengthLimitError>() || e.source().is_some_and(is_length_limit)
}

/// Reads the body, refusing one over the size limit or slower than the body timeout.
async fn read_body<B>(config: &Config, body: B) -> Result<Bytes, Response<Body>>
where
    B: hyper::body::Body<Data = Bytes>,
    B::Error: Into<BoxError>,
{
    let limit = config.body_limit();
    let read = async { Limited::new(body, limit).collect().await.map(|collected| collected.to_bytes()) };
    match tokio::time::timeout(config.request_body_timeout(), read).await {
        Err(_) => Err(error(
            StatusCode::REQUEST_TIMEOUT,
            "timeout",
            "invalid_request_error",
            "The request body took too long to arrive",
        )),
        Ok(Err(e)) if is_length_limit(&*e) => Err(error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "too_large",
            "invalid_request_error",
            format!("request body exceeds {limit} bytes"),
        )),
        Ok(Err(e)) => Err(invalid(format!("reading the request body: {e}"))),
        Ok(Ok(bytes)) => Ok(bytes),
    }
}

/// A permit from `slots`, or, when there is none, the one a less urgent queued request (from API
/// key `key`, if given) frees when it is shed for this one. `None` if there's neither.
async fn permit_for(
    state: &AppState,
    slots: &Arc<Semaphore>,
    level: u8,
    key: Option<usize>,
) -> Option<OwnedSemaphorePermit> {
    if let Ok(permit) = slots.clone().try_acquire_owned() {
        return Some(permit);
    }
    if !state.load.displace(level, key) {
        return None;
    }
    tokio::time::timeout(DISPLACE_WAIT, slots.clone().acquire_owned()).await.ok()?.ok()
}

async fn speech<B>(state: &Arc<AppState>, key: KeyIndex, body: B) -> Response<Body>
where
    B: hyper::body::Body<Data = Bytes>,
    B::Error: Into<BoxError>,
{
    let body = match read_body(&state.config, body).await {
        Ok(body) => body,
        Err(response) => return response,
    };
    let request: SpeechRequest = match serde_json::from_slice(&body) {
        Ok(request) => request,
        Err(e) => return invalid(e.to_string()),
    };
    if let Err(message) = request::check_fields(&request, &state.config) {
        return invalid(message);
    }
    // A priority over the key's maximum is lowered to it, not refused, so lowering a key's cap
    // can't make every request from it fail. Every response from here on says the level used.
    let level = match request.priority {
        Some(p) => p.min(state.config.max_priority(key.0)),
        None => state.config.default_priority(key.0),
    };
    let mut response = speak_at(state, key, request, level).await;
    response.headers_mut().insert(header::HeaderName::from_static("x-priority"), HeaderValue::from(u16::from(level)));
    response
}

async fn speak_at(state: &Arc<AppState>, key: KeyIndex, request: SpeechRequest, level: u8) -> Response<Body> {
    let Some(connection) = state.worker.current() else {
        return unavailable("The model worker is not connected");
    };
    state.load.set_max_batch(connection.max_batch());
    // Refused at once if over budget, before it can displace anything (`admit` decides for sure).
    if state.load.would_refuse(level) {
        return overloaded(StreamError::Shed(Shed { estimate: state.load.estimate(level) }).to_string());
    }
    // At either limit, a request may take the permits of a less urgent one that hasn't sent audio
    // yet (of the same key, for the key's limit), which is shed for it.
    let Some(key_permit) = permit_for(state, &state.key_slots[key.0], level, Some(key.0)).await else {
        return error(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "rate_limit_error",
            format!("This API key already has {} speech requests in progress", state.config.max_streams(key.0)),
        );
    };
    let Some(permit) = permit_for(state, &state.slots, level, None).await else {
        return overloaded("Too many requests in progress");
    };
    let admission = match state.load.admit(level, key.0) {
        Ok(admission) => Arc::new(admission),
        Err(shed) => return overloaded(StreamError::Shed(shed).to_string()),
    };

    // The text checks scan the whole input, so they run on the blocking pool. The permits go with
    // that work, so they stay taken until it ends even if the client leaves meanwhile, and then on
    // to the response body.
    let permits = (permit, key_permit);
    let planned = {
        let state = state.clone();
        let connection = connection.clone();
        tokio::task::spawn_blocking(move || (request::plan(request, &state.config, &connection), permits))
            .await
            .expect("planning doesn't panic")
    };
    let (speech, permits) = match planned {
        (Ok(speech), permits) => (speech, permits),
        (Err(message), _) => return invalid(message),
    };
    // Shed while its text was checked: don't send the worker anything.
    if admission.is_shed() {
        return overloaded(StreamError::Shed(Shed { estimate: admission.shed_estimate() }).to_string());
    }
    let mut audio = stream::start(
        speech,
        connection,
        state.pipeline.clone(),
        stream::Limits { lookahead: state.config.lookahead, max_ids: state.config.max_request_ids },
        admission,
        permits,
    );

    // Headers wait for the first audio, so a request that fails before any is made gets a status.
    let first = match audio.next().await {
        None => Bytes::new(),
        Some(Err(e)) => return stream_failure(&e),
        Some(Ok(first)) => first,
    };
    let rest = futures_stream::unfold(audio, |mut audio| async move { audio.next().await.map(|item| (item, audio)) });
    let frames = futures_stream::once(async { Ok(first) }).chain(rest).map_ok(Frame::data).map_err(BoxError::from);
    let mut response = Response::new(StreamBody::new(frames).boxed_unsync());
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("audio/pcm"));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    headers.insert(header::HeaderName::from_static("x-accel-buffering"), HeaderValue::from_static("no"));
    response
}
