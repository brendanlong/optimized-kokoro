//! Request priorities, the latency estimate and load shedding, end to end with fake workers.

mod common;

use common::*;
use hyper::Response;
use hyper::body::Incoming;
use kokoro_server::StubPipeline;
use kokoro_server::config::ApiKey;
use kokoro_server::load::{PRIOR_CHUNK_IDS, PRIOR_SECS_PER_ID};
use kokoro_server::protocol::worker_side::{self, FrontFrame};
use kokoro_server::protocol::{FailCode, Priority};
use serde_json::Value;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::task::JoinHandle;

async fn with_scripted(adjust: impl FnOnce(&mut kokoro_server::Config)) -> (Server, ScriptedWorker) {
    let server = start(adjust, Arc::new(StubPipeline)).await;
    let mut worker = ScriptedWorker::listen(&server.socket, worker_side::hello(&hello()));
    assert_eq!(worker.next().await, Event::Connected);
    server.worker.connected().await;
    (server, worker)
}

fn at(priority: impl Into<Value>, text: &str) -> Value {
    let mut body = pcm(text);
    body["priority"] = priority.into();
    body
}

/// One sentence of exactly `PRIOR_CHUNK_IDS` ids with the stub pipeline (one id per character),
/// so each costs a typical chunk in the estimate before anything is learned.
fn typical_sentence(letter: char) -> String {
    let text = format!("{}.", letter.to_string().repeat(PRIOR_CHUNK_IDS as usize - 1));
    assert_eq!(stub_chunks(&text)[0].ids.len(), PRIOR_CHUNK_IDS as usize);
    text
}

/// A budget of `chunks` typical chunks at the prior cost per id, with a little room.
fn budget_ms(chunks: f64) -> u64 {
    (PRIOR_SECS_PER_ID * PRIOR_CHUNK_IDS * chunks * 1000.0) as u64 + 2
}

fn speak_task(server: &Server, body: Value) -> JoinHandle<Response<Incoming>> {
    tokio::spawn(speak(server.addr, body))
}

fn priority_used(response: &Response<Incoming>) -> u8 {
    response.headers()["x-priority"].to_str().unwrap().parse().unwrap()
}

/// Refused for load, at priority `level`.
async fn assert_shed(response: Response<Incoming>, level: u8) {
    let (status, retry_after) = (response.status(), response.headers().get("retry-after").cloned());
    let used = priority_used(&response);
    let body = json_body(response).await;
    assert_eq!(status, 503, "{body}");
    assert_eq!(body["detail"]["error"], "overloaded", "{body}");
    assert_eq!(retry_after.unwrap(), "1");
    assert_eq!(used, level, "the level it was shed at");
}

/// The next `n` Synthesize frames as `(slot, priority, ids)`, acknowledging any Cancel between them.
async fn synthesizes(worker: &mut ScriptedWorker, n: usize) -> Vec<(u16, Priority, Vec<u8>)> {
    let mut out = Vec::new();
    while out.len() < n {
        match worker.next().await {
            Event::Frame(FrontFrame::Synthesize { slot, priority, waiting, ids, .. }) => {
                out.push((slot, Priority { level: priority, waiting: waiting == 1 }, ids));
            }
            Event::Frame(FrontFrame::Cancel { slot }) => worker.send(worker_side::failed(slot, FailCode::Cancelled)),
            other => panic!("unexpected {other:?}"),
        }
    }
    out
}

/// Answers every Synthesize and Cancel from now on, in the background; the priorities rendered, in
/// order, go to the returned list.
fn answer_from_now_on(mut worker: ScriptedWorker) -> Arc<std::sync::Mutex<Vec<Priority>>> {
    let rendered = Arc::new(std::sync::Mutex::new(Vec::new()));
    let log = rendered.clone();
    tokio::spawn(async move {
        // Slots answered since their Synthesize: a Cancel for one crossed the reply, so is ignored.
        let mut answered = std::collections::HashSet::new();
        while let Some(event) = worker.frames.recv().await {
            match event {
                Event::Frame(FrontFrame::Synthesize { slot, priority, waiting, ids, .. }) => {
                    log.lock().unwrap().push(Priority { level: priority, waiting: waiting == 1 });
                    worker.send(respond(slot, &ids));
                    answered.insert(slot);
                }
                Event::Frame(FrontFrame::Cancel { slot }) => {
                    if !answered.contains(&slot) {
                        worker.send(worker_side::failed(slot, FailCode::Cancelled));
                    }
                }
                // The server hung up: the test is over, or it found a violation (which the test sees).
                Event::Closed => return,
                other => panic!("unexpected {other:?}"),
            }
        }
    });
    rendered
}

#[tokio::test]
async fn a_high_priority_request_overtakes_queued_low_ones() {
    let server = start(|c| c.max_estimated_latency_ms = 0, Arc::new(StubPipeline)).await;
    let worker = AutoWorker::listen(&server.socket, Duration::from_millis(20));
    server.worker.connected().await;
    let text = (0..6).map(|i| format!("Low sentence number {i}.")).collect::<Vec<_>>().join(" ");
    let lows: Vec<_> = (0..6).map(|_| speak_task(&server, at(10, &text))).collect();
    while worker.log.lock().unwrap().len() < 3 {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let started = Instant::now();
    let high = speak(server.addr, at(200, "Zzz.")).await;
    assert_eq!(high.status(), 200);
    assert_eq!(samples(&body_bytes(high).await.unwrap()), expected_audio("Zzz."));
    // Each low request has up to three chunks queued; the high one waits only for the chunk
    // already rendering.
    assert!(started.elapsed() < Duration::from_millis(200), "{:?}", started.elapsed());
    let log = worker.log.lock().unwrap().clone();
    let z = log.iter().find(|r| r.first_id == 43 + 25).unwrap();
    assert_eq!(z.priority, Priority { level: 200, waiting: true });
    assert!(z.waited <= 1, "the high-priority request waited for {} low chunks", z.waited);
    assert!(log.iter().filter(|r| r.priority.level == 10).count() >= 3);
    for low in lows {
        let response = low.await.unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(samples(&body_bytes(response).await.unwrap()), expected_audio(&text));
    }
}

#[tokio::test]
async fn new_low_priority_requests_over_budget_are_shed_fast() {
    let (server, mut worker) = with_scripted(|c| c.max_estimated_latency_ms = budget_ms(3.0)).await;
    // Three low requests fill the budget; the worker holds their chunks.
    let mut lows = Vec::new();
    let mut held = Vec::new();
    for letter in ['a', 'b', 'c'] {
        lows.push(speak_task(&server, at(10, &typical_sentence(letter))));
        let (slot, priority, _, _, ids) = worker.synthesize().await;
        assert_eq!(priority, Priority { level: 10, waiting: true });
        held.push((slot, ids));
    }
    // A fourth at the same level would wait past the budget, so it's refused before any work.
    let started = Instant::now();
    let refused = speak(server.addr, at(10, &typical_sentence('d'))).await;
    assert!(started.elapsed() < Duration::from_millis(100), "{:?}", started.elapsed());
    assert_shed(refused, 10).await;
    assert_eq!(worker.quiet(Duration::from_millis(100)).await, None, "the refused request reached the worker");

    // A high one is admitted (only its own chunk is ahead of it), and pushes the newest low one
    // out: its queued chunk is cancelled.
    let high = speak_task(&server, at(200, &typical_sentence('z')));
    let newest = lows.pop().unwrap();
    assert_shed(newest.await.unwrap(), 10).await;
    let mut cancelled = None;
    let mut high_chunk = None;
    while cancelled.is_none() || high_chunk.is_none() {
        match worker.next().await {
            Event::Frame(FrontFrame::Cancel { slot }) => cancelled = Some(slot),
            Event::Frame(FrontFrame::Synthesize { slot, priority, waiting, ids, .. }) => {
                assert_eq!((priority, waiting), (200, 1));
                high_chunk = Some((slot, ids));
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    assert_eq!(cancelled, Some(held.pop().unwrap().0), "the shed request's chunk is cancelled");
    worker.send(worker_side::failed(cancelled.unwrap(), FailCode::Cancelled));

    let (high_slot, high_ids) = high_chunk.unwrap();
    worker.send(respond(high_slot, &high_ids));
    let high = high.await.unwrap();
    assert_eq!(high.status(), 200);
    assert_eq!(samples(&body_bytes(high).await.unwrap()), expected_audio(&typical_sentence('z')));
    // The two older low requests kept their places.
    for (slot, ids) in held {
        worker.send(respond(slot, &ids));
    }
    for (low, letter) in lows.into_iter().zip(['a', 'b']) {
        let response = low.await.unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(samples(&body_bytes(response).await.unwrap()), expected_audio(&typical_sentence(letter)));
    }
}

#[tokio::test]
async fn a_stream_that_has_started_is_never_cut() {
    let (server, mut worker) = with_scripted(|c| {
        c.max_estimated_latency_ms = budget_ms(4.0);
        c.lookahead = 5;
    })
    .await;
    // It starts with a pause, so its first audio needs no render (and nothing is learned that
    // would change the estimates); pauses also keep the sentences in chunks of their own.
    let long =
        ["".to_owned(), typical_sentence('a'), typical_sentence('b'), typical_sentence('c')].join(" [pause:0.1s] ");
    let units = stub_chunks(&long);
    assert!(units[0].pause.is_some(), "{:?}", units[0]);
    let speech: Vec<_> = units.iter().filter(|c| c.pause.is_none()).map(|c| c.ids.len()).collect();
    assert_eq!(speech, [PRIOR_CHUNK_IDS as usize; 3]);
    let started = speak_task(&server, at(10, &long));
    // The pause is sent at once, so `a` (waiting), `b` and `c` (prefetch) all go to the worker.
    let mut chunks = Vec::new();
    for _ in 0..3 {
        chunks.push(worker.synthesize().await);
    }
    let response = started.await.unwrap();
    assert_eq!(response.status(), 200);
    let mut body = response.into_body();
    let mut audio = read_at_least(&mut body, 2 * 2400).await;

    // A queued low request, then high ones until the low level is over budget.
    let queued = speak_task(&server, at(10, &typical_sentence('q')));
    worker.synthesize().await;
    let highs: Vec<_> = ['x', 'y', 'z'].map(|l| speak_task(&server, at(200, &typical_sentence(l)))).into();
    assert_shed(queued.await.unwrap(), 10).await;

    // Everything else is served, the started stream in full.
    for (slot, _, _, _, ids) in chunks {
        worker.send(respond(slot, &ids));
    }
    let rendered = answer_from_now_on(worker);
    while let Some(frame) = tokio::time::timeout(Duration::from_secs(10), http_body_util::BodyExt::frame(&mut body))
        .await
        .expect("body stalled")
    {
        audio.extend_from_slice(&frame.unwrap().into_data().unwrap());
    }
    assert_eq!(samples(&audio), expected_audio(&long));
    for high in highs {
        assert_eq!(high.await.unwrap().status(), 200);
    }
    let rendered = rendered.lock().unwrap().clone();
    assert_eq!(rendered.iter().filter(|p| p.level == 200).count(), 3, "{rendered:?}");
}

#[tokio::test]
async fn high_priority_is_shed_only_when_high_priority_alone_is_over_budget() {
    let (server, mut worker) = with_scripted(|c| c.max_estimated_latency_ms = budget_ms(2.5)).await;
    let low = speak_task(&server, at(0, &typical_sentence('a')));
    synthesizes(&mut worker, 1).await;
    let highs: Vec<_> = ['b', 'c'].map(|l| speak_task(&server, at(255, &typical_sentence(l)))).into();
    let chunks = synthesizes(&mut worker, 2).await;
    assert!(chunks.iter().all(|(_, p, _)| p.level == 255));
    // The low request went first, then the high ones filled the budget at their own level.
    assert_shed(low.await.unwrap(), 0).await;
    let started = Instant::now();
    assert_shed(speak(server.addr, at(255, &typical_sentence('d'))).await, 255).await;
    assert!(started.elapsed() < Duration::from_millis(100));
    for (slot, _, ids) in chunks {
        worker.send(respond(slot, &ids));
    }
    answer_from_now_on(worker);
    for high in highs {
        assert_eq!(high.await.unwrap().status(), 200);
    }
}

#[tokio::test]
async fn a_priority_over_the_keys_maximum_runs_at_the_maximum() {
    let (server, mut worker) = with_scripted(|c| {
        c.api_keys = vec![ApiKey { key: KEY.into(), max_streams: None, max_priority: Some(50) }, "other-key".into()];
    })
    .await;
    for body in [at(256, "Hi."), at(-1, "Hi."), at(1.5, "Hi."), at("high", "Hi.")] {
        let response = speak(server.addr, body.clone()).await;
        assert_eq!(response.status(), 400, "{body}");
        let message = json_body(response).await["detail"]["message"].as_str().unwrap().to_owned();
        assert!(message.contains("priority must be an integer from 0 to 255"), "{message}");
    }
    // (key, priority asked for, priority used)
    let cases = [
        (KEY, Some(240), 50),
        (KEY, Some(51), 50),
        (KEY, Some(50), 50),
        (KEY, Some(0), 0),
        (KEY, None, 50),
        ("other-key", Some(255), 255),
        ("other-key", None, 128),
    ];
    for (key, priority, expected) in cases {
        let body = match priority {
            Some(p) => at(p, "Hi."),
            None => pcm("Hi."),
        };
        let client = tokio::spawn(request(server.addr, speech_request(Some(key), body)));
        let (slot, got, _, _, ids) = worker.synthesize().await;
        assert_eq!(got, Priority { level: expected, waiting: true }, "{key} {priority:?}");
        worker.send(respond(slot, &ids));
        let response = client.await.unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(priority_used(&response), expected, "{key} {priority:?}");
    }
}

#[tokio::test]
async fn at_the_request_limit_a_more_urgent_request_displaces_a_queued_one() {
    let (server, mut worker) = with_scripted(|c| {
        c.max_concurrent_requests = 1;
        c.max_estimated_latency_ms = 0;
    })
    .await;
    let low = speak_task(&server, at(10, "Low."));
    worker.synthesize().await;
    // Equal priority doesn't displace.
    assert_shed(speak(server.addr, at(10, "Same.")).await, 10).await;
    let high = speak_task(&server, at(20, "High. [pause:0.1s] More."));
    assert_shed(low.await.unwrap(), 10).await;
    let mut slot_ids = None;
    while slot_ids.is_none() {
        match worker.next().await {
            Event::Frame(FrontFrame::Cancel { slot }) => worker.send(worker_side::failed(slot, FailCode::Cancelled)),
            Event::Frame(FrontFrame::Synthesize { slot, priority, ids, .. }) => {
                assert_eq!(priority, 20);
                slot_ids = Some((slot, ids));
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    let (slot, ids) = slot_ids.unwrap();
    worker.send(respond(slot, &ids));
    let high = high.await.unwrap();
    assert_eq!(high.status(), 200);
    // A started stream holds its permit: an even more urgent request is refused, not let in.
    let mut body = high.into_body();
    read_at_least(&mut body, 2).await;
    assert_shed(speak(server.addr, at(255, "Top.")).await, 255).await;
    drop(body);
}

#[tokio::test]
async fn at_a_keys_stream_limit_only_that_keys_queued_requests_are_displaced() {
    let (server, mut worker) = with_scripted(|c| {
        c.api_keys = vec![ApiKey { key: KEY.into(), max_streams: Some(1), max_priority: None }, "other-key".into()];
        c.max_estimated_latency_ms = 0;
    })
    .await;
    let other = tokio::spawn(request(server.addr, speech_request(Some("other-key"), at(0, "Other."))));
    let low = speak_task(&server, at(10, "Low."));
    let first = synthesizes(&mut worker, 2).await;
    let high = speak_task(&server, at(20, "High."));
    // The key's own queued request is shed for it; the other key's, less urgent, is not.
    assert_shed(low.await.unwrap(), 10).await;
    let chunks = synthesizes(&mut worker, 1).await;
    assert_eq!(chunks[0].1, Priority { level: 20, waiting: true });
    for (slot, _, ids) in chunks {
        worker.send(respond(slot, &ids));
    }
    assert_eq!(high.await.unwrap().status(), 200);
    let (slot, _, ids) = first.into_iter().find(|(_, p, _)| p.level == 0).unwrap();
    worker.send(respond(slot, &ids));
    answer_from_now_on(worker);
    let other = other.await.unwrap();
    assert_eq!(other.status(), 200);
}

#[tokio::test]
async fn the_estimate_learns_the_workers_speed() {
    // A slow worker: 4 ms per id, ten times the prior, so a typical chunk takes 320 ms. At the
    // prior cost two queued requests would fit a 500 ms budget many times over; once a few
    // chunks have been rendered, they don't.
    let server = start(|c| c.max_estimated_latency_ms = 500, Arc::new(StubPipeline)).await;
    let _worker = AutoWorker::listen_with(&server.socket, |ids| Duration::from_millis(4 * ids.len() as u64));
    server.worker.connected().await;
    for letter in ['a', 'b', 'c', 'd', 'e', 'f'] {
        let response = speak(server.addr, at(10, &typical_sentence(letter))).await;
        assert_eq!(response.status(), 200);
        body_bytes(response).await.unwrap();
    }
    let first = speak_task(&server, at(10, &typical_sentence('g')));
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_shed(speak(server.addr, at(10, &typical_sentence('h'))).await, 10).await;
    assert_eq!(first.await.unwrap().status(), 200);
}
