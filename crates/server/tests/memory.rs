//! Planning a request must not use memory out of proportion to its body. One test, since it
//! measures the whole process's allocations.

mod common;

use common::*;
use kokoro_server::protocol::worker_side;
use kokoro_server::{StubPipeline, request};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Meter;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Meter {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            let live = LIVE.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
            PEAK.fetch_max(live, Ordering::Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe { System.dealloc(p, layout) }
    }
}

#[global_allocator]
static ALLOC: Meter = Meter;

#[tokio::test]
async fn voice_expressions_are_bounded_and_resolved_once() {
    let server = start(|_| {}, Arc::new(StubPipeline)).await;
    let _worker = ScriptedWorker::listen(&server.socket, worker_side::hello(&hello()));
    let connection = server.worker.connected().await;
    let config = config(&server.socket);

    // A 200 KB voice expression fits the body limit, but not the voice limit.
    let huge = serde_json::json!({
        "input": "a[rate:2]b[rate:1]".repeat(1000),
        "voice": format!("af_heart({}1)", "0".repeat(200_000)),
        "response_format": "pcm",
        "allow_voice_tags": true,
    });
    assert!(huge.to_string().len() < config.body_limit());
    let huge: request::SpeechRequest = serde_json::from_value(huge).unwrap();
    assert!(request::check_fields(&huge, &config).is_err());
    let tag = serde_json::json!({
        "input": format!("[voice:af_heart({}1)] hi", "0".repeat(1000)),
        "response_format": "pcm",
        "allow_voice_tags": true,
    });
    let tag: request::SpeechRequest = serde_json::from_value(tag).unwrap();
    request::check_fields(&tag, &config).unwrap();
    assert!(request::plan(tag, &config, &connection, &StubPipeline).is_err());

    // The longest allowed voice, over 2,000 segments, is resolved once and shared.
    let voice = format!("af_heart({}1)", "0".repeat(480));
    let body = serde_json::json!({
        "input": "a[rate:2]b[rate:1]".repeat(1100),
        "voice": voice,
        "response_format": "pcm",
        "allow_voice_tags": true,
    });
    let req: request::SpeechRequest = serde_json::from_value(body).unwrap();
    request::check_fields(&req, &config).unwrap();
    let baseline = LIVE.load(Ordering::Relaxed);
    PEAK.store(baseline, Ordering::Relaxed);
    let speech = request::plan(req, &config, &connection, &StubPipeline).unwrap();
    let extra = PEAK.load(Ordering::Relaxed).saturating_sub(baseline);
    assert_eq!(speech.segments.len(), 2200);
    assert!(speech.segments.iter().all(|s| Arc::ptr_eq(&s.voices, &speech.segments[0].voices)));
    assert!(extra < 600_000, "planning took {extra} bytes for a {} byte input", 18 * 1100);
}
