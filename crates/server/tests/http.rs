mod common;

use common::*;
use kokoro_server::StubPipeline;
use kokoro_server::pipeline::{Chunk, DEFAULT_MAX_TOKENS, Lang, Pipeline, PipelineError};
use kokoro_server::protocol::worker_side::{self, FrontFrame};
use kokoro_server::protocol::{FailCode, Hello, Priority, Speed};
use kokoro_server::stitch::{Plan, max_render_ids};
use serde_json::json;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

async fn with_scripted(adjust: impl FnOnce(&mut kokoro_server::Config)) -> (Server, ScriptedWorker) {
    let server = start(adjust, Arc::new(StubPipeline)).await;
    let mut worker = ScriptedWorker::listen(&server.socket, worker_side::hello(&hello()));
    assert_eq!(worker.next().await, Event::Connected);
    server.worker.connected().await;
    (server, worker)
}

#[tokio::test]
async fn health_and_voices_follow_the_worker() {
    let server = start(|_| {}, Arc::new(StubPipeline)).await;
    assert_eq!(request(server.addr, get("/health", None)).await.status(), 503);
    assert_eq!(request(server.addr, get("/v1/audio/voices", Some(KEY))).await.status(), 503);

    let _worker = ScriptedWorker::listen(&server.socket, worker_side::hello(&hello()));
    server.worker.connected().await;
    let health = request(server.addr, get("/health", None)).await;
    assert_eq!(health.status(), 200);
    assert_eq!(json_body(health).await, json!({ "status": "healthy" }));
    let voices = request(server.addr, get("/v1/audio/voices", Some(KEY))).await;
    // Only the voices whose language the server speaks: the worker also has `xf_nobody`.
    assert!(VOICES.contains(&"xf_nobody"));
    let spoken: Vec<_> = VOICES.iter().filter(|v| !v.starts_with('x')).collect();
    assert!(spoken.contains(&&"zf_xiaobei") && spoken.contains(&&"jf_alpha") && spoken.contains(&&"ef_dora"));
    assert_eq!(json_body(voices).await, json!({ "voices": spoken }));
    assert_eq!(request(server.addr, get("/v1/audio/voices", None)).await.status(), 401);
    assert_eq!(request(server.addr, get("/v1/audio/voices", Some("nope"))).await.status(), 401);
    assert_eq!(request(server.addr, get("/v1/audio/voices", Some("other-key"))).await.status(), 200);
    assert_eq!(request(server.addr, get("/docs", Some(KEY))).await.status(), 404);
    assert_eq!(request(server.addr, get("/v1/audio/voices/", Some(KEY))).await.status(), 404);
}

#[tokio::test]
async fn other_methods_are_refused_before_the_key_is_checked() {
    let server = start(|_| {}, Arc::new(StubPipeline)).await;
    let with_method = |method: &str, path: &str| {
        let mut req = get(path, None);
        *req.method_mut() = method.parse().unwrap();
        req
    };
    for (method, path, allow) in [
        ("POST", "/health", "GET,HEAD"),
        ("DELETE", "/v1/audio/voices", "GET,HEAD"),
        ("GET", "/v1/audio/speech", "POST"),
        ("PUT", "/v1/audio/speech", "POST"),
    ] {
        let response = request(server.addr, with_method(method, path)).await;
        assert_eq!(response.status(), 405, "{method} {path}");
        assert_eq!(response.headers()["allow"], allow, "{method} {path}");
    }
    let head = request(server.addr, with_method("HEAD", "/v1/audio/voices")).await;
    assert_eq!(head.status(), 401);
}

/// The server answers 401 to a request whose body never arrives, so it can't have read it.
#[tokio::test]
async fn auth_is_checked_before_the_body_is_read() {
    let (server, _worker) = with_scripted(|_| {}).await;
    for auth in
        ["", "Authorization: Bearer wrong\r\n", "Authorization: Basic dGVzdC1rZXk=\r\n", "Authorization: Bearer \r\n"]
    {
        let mut stream = TcpStream::connect(server.addr).await.unwrap();
        let head = format!(
            "POST /v1/audio/speech HTTP/1.1\r\nHost: test\r\n{auth}Content-Type: application/json\r\nContent-Length: 100000\r\n\r\n"
        );
        stream.write_all(head.as_bytes()).await.unwrap();
        let mut response = vec![0; 12];
        tokio::time::timeout(Duration::from_secs(5), stream.read_exact(&mut response)).await.unwrap().unwrap();
        assert_eq!(&response, b"HTTP/1.1 401", "{auth:?}");
    }
}

#[tokio::test]
async fn refuses_bad_requests_before_text_processing() {
    let pipeline = Arc::new(CountingPipeline::default());
    let server = start(|c| c.max_input_chars = 50, pipeline.clone()).await;
    let _worker = ScriptedWorker::listen(&server.socket, worker_side::hello(&hello()));
    server.worker.connected().await;

    let too_large = speech_request(Some(KEY), pcm(&"x".repeat(50 * 12 + 16 * 1024)));
    assert_eq!(request(server.addr, too_large).await.status(), 413);

    let cases = [
        pcm(&"é".repeat(51)),
        json!({ "input": "Hi.", "voice": "af_heart" }),
        json!({ "input": "Hi.", "response_format": "mp3" }),
        json!({ "input": "Hi.", "response_format": "pcm", "stream": false }),
        json!({ "input": "Hi.", "response_format": "pcm", "speed": 4.01 }),
        json!({ "input": "Hi.", "response_format": "pcm", "speed": 0.24 }),
        json!({ "input": "Hi.", "response_format": "pcm", "speed": "fast" }),
        json!({ "input": "Hi.", "response_format": "pcm", "voice": "x".repeat(65) }),
        json!({ "response_format": "pcm" }),
        // Fields Kokoro-FastAPI or OpenAI take and this server doesn't, even at their defaults.
        json!({ "input": "Hi.", "response_format": "pcm", "ssml": false }),
        json!({ "input": "Hi.", "response_format": "pcm", "volume_multiplier": 1 }),
        json!({ "input": "Hi.", "response_format": "pcm", "lang_code": "a" }),
        json!({ "input": "Hi.", "response_format": "pcm", "normalization_options": {} }),
        json!({ "input": "Hi.", "response_format": "pcm", "allow_voice_tags": false }),
        json!({ "input": "Hi.", "response_format": "pcm", "instructions": "Speak calmly." }),
        json!({ "input": "Hi.", "response_format": "pcm", "stream_format": "audio" }),
        json!({ "input": "Hi.", "response_format": "pcm", "voice_aliases": {} }),
        json!({ "input": "Hi.", "response_format": "pcm", "return_download_link": true }),
    ];
    for body in cases {
        let response = speak(server.addr, body.clone()).await;
        assert_eq!(response.status(), 400, "{body}");
        assert_eq!(json_body(response).await["detail"]["error"], "validation_error");
    }
    let not_json = speech_request(Some(KEY), json!(null));
    assert_eq!(request(server.addr, not_json).await.status(), 400);
    assert_eq!(pipeline.calls.load(Ordering::SeqCst), 0);

    let after_parsing = [
        json!({ "input": "Hi.", "response_format": "pcm", "voice": "nobody" }),
        json!({ "input": "Hi.", "response_format": "pcm", "voice": "af_heart+af_sky" }),
        json!({ "input": "Hi.", "response_format": "pcm", "voice": "af_bella(2)+af_sky" }),
        json!({ "input": "Hi.", "response_format": "pcm", "voice": "af_heart-af_sky" }),
        json!({ "input": "Hi.", "response_format": "pcm", "voice": "alloy" }),
        json!({ "input": "Hi.", "response_format": "pcm", "voice": "xf_nobody" }),
        json!({ "input": "   ", "response_format": "pcm" }),
        json!({ "input": format!("a {}", "[pause:60s] ".repeat(6)), "response_format": "pcm" }),
    ];
    for body in after_parsing {
        let response = speak(server.addr, body.clone()).await;
        assert_eq!(response.status(), 400, "{body}");
        assert_eq!(json_body(response).await["detail"]["error"], "validation_error");
    }
    assert_eq!(pipeline.calls.load(Ordering::SeqCst), 0);
}

/// What the two clients send: Lion Reader's streamed request and clawed-abode's with a speed.
#[tokio::test]
async fn the_clients_requests_are_served() {
    let (server, mut worker) = with_scripted(|_| {}).await;
    let lion_reader =
        json!({ "model": "tts-1", "voice": "af_heart", "input": "Hi.", "response_format": "pcm", "stream": true });
    let clawed_abode =
        json!({ "model": "kokoro", "input": "Hi.", "voice": "am_adam", "speed": 1.25, "response_format": "pcm" });
    for (body, voice, speed) in [(lion_reader, 0, 100), (clawed_abode, 3, 125)] {
        let client = tokio::spawn(speak(server.addr, body));
        let (slot, _, got_speed, got_voice, ids) = worker.synthesize().await;
        assert_eq!((got_voice, got_speed), (voice, speed));
        worker.send(respond(slot, &ids));
        let response = client.await.unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(samples(&body_bytes(response).await.unwrap()), expected_audio("Hi."));
    }
}

/// Speed travels in hundredths, the nearest to what was asked.
#[tokio::test]
async fn speed_is_rounded_to_hundredths() {
    let (server, mut worker) = with_scripted(|_| {}).await;
    for (asked, sent) in [(0.25, 25), (1.234, 123), (1.235, 124), (4.0, 400)] {
        let body = json!({ "input": "Hi.", "response_format": "pcm", "speed": asked });
        let client = tokio::spawn(speak(server.addr, body));
        let (slot, _, speed, _, ids) = worker.synthesize().await;
        assert_eq!(speed, sent, "{asked}");
        worker.send(respond(slot, &ids));
        assert_eq!(client.await.unwrap().status(), 200);
    }
}

#[tokio::test]
async fn streams_in_order_when_the_worker_answers_out_of_order() {
    let (server, mut worker) = with_scripted(|c| c.lookahead = 3).await;
    let text = "One fish. Two fish. Red fish. Blue fish. Old fish.";
    let addr = server.addr;
    let client = tokio::spawn(async move {
        let response = speak(addr, pcm(text)).await;
        assert_eq!(response.status(), 200);
        assert_eq!(response.headers()["content-type"], "audio/pcm");
        body_bytes(response).await.unwrap()
    });
    let mut first_batch = Vec::new();
    for _ in 0..3 {
        first_batch.push(worker.synthesize().await);
    }
    assert_eq!(first_batch.iter().map(|r| r.1.waiting).collect::<Vec<_>>(), [true, false, false]);
    assert_eq!(worker.quiet(Duration::from_millis(200)).await, None, "more than the lookahead in flight");
    for (id, _, _, _, ids) in first_batch.iter().rev() {
        worker.send(respond(*id, ids));
    }
    for _ in 0..2 {
        let (id, _, _, _, ids) = worker.synthesize().await;
        worker.send(respond(id, &ids));
    }
    let audio = client.await.unwrap();
    assert_eq!(samples(&audio), expected_audio(text));
}

#[tokio::test]
async fn first_chunk_streams_before_the_rest_is_rendered() {
    let (server, mut worker) = with_scripted(|_| {}).await;
    let text = "First sentence here. Second one.";
    let chunks = stub_chunks(text);
    let response_task = tokio::spawn(speak(server.addr, pcm(text)));
    let (id0, _, _, _, ids0) = worker.synthesize().await;
    let (id1, _, _, _, ids1) = worker.synthesize().await;
    assert_eq!(ids0, chunks[0].ids);
    worker.send(respond(id0, &ids0));
    let response = response_task.await.unwrap();
    assert_eq!(response.status(), 200);
    let mut body = response.into_body();
    let first_len = expected_chunk(&ids0).len() * 2;
    let first = read_at_least(&mut body, first_len).await;
    assert_eq!(samples(&first), expected_chunk(&ids0));
    worker.send(respond(id1, &ids1));
    let rest = read_at_least(&mut body, expected_chunk(&ids1).len() * 2).await;
    assert_eq!(samples(&rest), expected_chunk(&ids1));
}

#[tokio::test]
async fn pauses_become_silence_and_options_reach_the_worker() {
    let (server, mut worker) = with_scripted(|_| {}).await;
    let body = json!({
        "input": "Hello there. [pause:0.5s] Bye now.",
        "voice": "af_bella",
        "response_format": "pcm",
        "speed": 1.5,
    });
    let client = tokio::spawn(speak(server.addr, body));
    let mut expected = Vec::new();
    for _ in 0..2 {
        let (slot, _, speed, voice, ids) = worker.synthesize().await;
        assert_eq!((speed, voice), (150, 1));
        worker.send(respond(slot, &ids));
        let (_, pcm) = render(&ids);
        // At 1.5x, the trailing pad is shorter than the chunk, so it still reaches the end.
        expected.push(pcm[24..pcm.len() - 24].to_vec());
    }
    let response = client.await.unwrap();
    assert_eq!(response.status(), 200);
    let audio = samples(&body_bytes(response).await.unwrap());
    let mut want = expected[0].clone();
    want.extend(std::iter::repeat_n(0, 12000));
    want.extend(&expected[1]);
    assert_eq!(audio, want);
}

/// Voice and rate tags aren't supported, so they're read as text, in the request's one voice.
#[tokio::test]
async fn voice_and_rate_tags_are_text() {
    let (server, mut worker) = with_scripted(|_| {}).await;
    let text = "Hello there. [voice:bf_emma] Cheerio. [rate:2] Quickly.";
    let client = tokio::spawn(speak(server.addr, pcm(text)));
    let chunks = stub_chunks(text);
    for chunk in &chunks {
        let (slot, _, speed, voice, ids) = worker.synthesize().await;
        assert_eq!((speed, voice, &ids), (100, 0, &chunk.ids));
        worker.send(respond(slot, &ids));
    }
    assert_eq!(worker.style_lens, chunks.iter().map(|c| c.ids.len() as u16).collect::<Vec<_>>());
    let response = client.await.unwrap();
    assert_eq!(samples(&body_bytes(response).await.unwrap()), expected_audio(text));
}

/// The stub pipeline, recording the language of each request.
#[derive(Default)]
struct LangRecorder(Mutex<Vec<Lang>>);

impl Pipeline for LangRecorder {
    fn chunks<'a>(
        &'a self,
        lang: Lang,
        text: &'a str,
        max_phonemes: usize,
    ) -> Box<dyn Iterator<Item = Result<Chunk, PipelineError>> + 'a> {
        self.0.lock().unwrap().push(lang);
        StubPipeline.chunks(lang, text, max_phonemes)
    }
}

#[tokio::test]
async fn the_voice_sets_the_language() {
    let pipeline = Arc::new(LangRecorder::default());
    let server = start(|_| {}, pipeline.clone()).await;
    let mut worker = ScriptedWorker::listen(&server.socket, worker_side::hello(&hello()));
    server.worker.connected().await;
    for voice in ["bf_emma", "am_adam"] {
        let body = json!({ "input": "Hi.", "voice": voice, "response_format": "pcm" });
        let client = tokio::spawn(speak(server.addr, body));
        let (slot, _, _, _, ids) = worker.synthesize().await;
        worker.send(respond(slot, &ids));
        assert_eq!(client.await.unwrap().status(), 200);
    }
    assert_eq!(*pipeline.0.lock().unwrap(), [Lang::BritishEnglish, Lang::AmericanEnglish]);
}

#[tokio::test]
async fn disconnecting_cancels_queued_chunks() {
    let (server, mut worker) = with_scripted(|c| c.lookahead = 3).await;
    let text = "Alpha one. Bravo two. Charlie three. Delta four. Echo five. Foxtrot six.";
    let response_task = tokio::spawn(speak(server.addr, pcm(text)));
    let (id0, _, _, _, ids0) = worker.synthesize().await;
    let (id1, ..) = worker.synthesize().await;
    let (id2, ..) = worker.synthesize().await;
    worker.send(respond(id0, &ids0));
    let response = response_task.await.unwrap();
    let mut body = response.into_body();
    read_at_least(&mut body, 2).await;
    let (id3, ..) = worker.synthesize().await;
    drop(body);

    let mut cancelled = Vec::new();
    while cancelled.len() < 3 {
        match worker.next().await {
            Event::Frame(FrontFrame::Cancel { slot }) => cancelled.push(slot),
            other => panic!("expected only cancellations, got {other:?}"),
        }
    }
    let mut queued = [id1, id2, id3];
    queued.sort();
    cancelled.sort();
    assert_eq!(cancelled, queued);
    assert_eq!(id3, id0, "the first chunk's slot was free again");
    for slot in &cancelled {
        worker.send(worker_side::failed(*slot, FailCode::Cancelled));
    }
    assert_eq!(worker.quiet(Duration::from_millis(300)).await, None);

    // The connection survives, and serves the next request.
    let client = tokio::spawn(speak(server.addr, pcm("Hi.")));
    let (id, _, _, _, ids) = worker.synthesize().await;
    worker.send(respond(id, &ids));
    let response = client.await.unwrap();
    assert_eq!(samples(&body_bytes(response).await.unwrap()), expected_audio("Hi."));
}

#[tokio::test]
async fn a_long_request_does_not_starve_a_short_one() {
    let server = start(|c| c.lookahead = 3, Arc::new(StubPipeline)).await;
    let worker = AutoWorker::listen(&server.socket, Duration::from_millis(15));
    server.worker.connected().await;
    let long_text = (0..40).map(|i| format!("Long sentence number {i}.")).collect::<Vec<_>>().join(" ");
    let addr = server.addr;
    let long = tokio::spawn(async move { body_bytes(speak(addr, pcm(&long_text)).await).await.unwrap() });
    while worker.log.lock().unwrap().len() < 5 {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let short = body_bytes(speak(server.addr, pcm("Zzz.")).await).await.unwrap();
    assert_eq!(samples(&short), expected_audio("Zzz."));
    let log = worker.log.lock().unwrap().clone();
    let z = 43 + 25;
    let short_chunk = log.iter().find(|r| r.first_id == z).unwrap();
    assert_eq!(short_chunk.priority, Priority { level: 128, waiting: true });
    assert!(short_chunk.waited <= 1, "the short request waited for {} long chunks", short_chunk.waited);
    long.await.unwrap();
    assert!(worker.most_queued.load(Ordering::SeqCst) <= 2 * 3, "more than the lookahead queued");
}

#[tokio::test]
async fn worker_restart_fails_streams_in_flight_and_reconnects() {
    let (server, mut worker) = with_scripted(|_| {}).await;
    let text = "Before the crash. After the crash.";
    let response_task = tokio::spawn(speak(server.addr, pcm(text)));
    let (id0, _, _, _, ids0) = worker.synthesize().await;
    worker.synthesize().await;
    worker.send(respond(id0, &ids0));
    let response = response_task.await.unwrap();
    assert_eq!(response.status(), 200);
    worker.hang_up();
    assert!(body_bytes(response).await.is_err(), "a truncated stream must not end cleanly");

    assert_eq!(worker.next().await, Event::Connected);
    server.worker.connected().await;

    let client = tokio::spawn(speak(server.addr, pcm("Hi.")));
    let (id, _, _, _, ids) = worker.synthesize().await;
    worker.send(respond(id, &ids));
    let response = client.await.unwrap();
    assert_eq!(samples(&body_bytes(response).await.unwrap()), expected_audio("Hi."));
}

#[tokio::test]
async fn requests_while_the_worker_is_down_get_503() {
    let server = start(|_| {}, Arc::new(StubPipeline)).await;
    let response = speak(server.addr, pcm("Hi.")).await;
    assert_eq!(response.status(), 503);
}

/// Each bad frame closes the connection, fails the request waiting on it, and the server
/// reconnects.
#[tokio::test]
async fn malformed_worker_frames_close_the_connection() {
    let (server, mut worker) = with_scripted(|_| {}).await;
    type Bad = fn(u16, &[u8]) -> Vec<u8>;
    /// A Result with `durations` and the audio they imply.
    fn with_durations(slot: u16, durations: &[u16]) -> Vec<u8> {
        let units: usize = durations.iter().map(|&d| usize::from(d)).sum();
        worker_side::result(slot, durations, &vec![1000; 600 * units])
    }
    let bad: [(&str, Bad); 14] = [
        ("audio too short", |slot, ids| {
            let (d, pcm) = render(ids);
            worker_side::result(slot, &d, &pcm[1..])
        }),
        ("audio too long", |slot, ids| {
            let (d, mut pcm) = render(ids);
            pcm.push(0);
            worker_side::result(slot, &d, &pcm)
        }),
        ("too few durations", |slot, ids| with_durations(slot, &vec![1; ids.len() + 1])),
        ("too many durations", |slot, ids| with_durations(slot, &vec![1; ids.len() + 3])),
        ("duration over ceil(50 / speed)", |slot, ids| with_durations(slot, &vec![51; ids.len() + 2])),
        ("zero duration", |slot, ids| {
            let mut d = vec![1; ids.len() + 2];
            d[1] = 0;
            with_durations(slot, &d)
        }),
        ("result for a free slot", |slot, ids| respond(slot + 1, ids)),
        ("result for a slot past the table", |_, ids| respond(5000, ids)),
        ("failure for a free slot", |slot, _| worker_side::failed(slot + 1, FailCode::Internal)),
        ("cancelled but not cancelled", |slot, _| worker_side::failed(slot, FailCode::Cancelled)),
        ("second hello", |_, _| worker_side::hello(&hello())),
        ("unknown type", |_, _| vec![1, 0, 0, 0, 0x7f]),
        ("bad failure code", |id, _| {
            let mut f = worker_side::failed(id, FailCode::Internal);
            *f.last_mut().unwrap() = 9;
            f
        }),
        ("huge length", |_, _| vec![0xff, 0xff, 0xff, 0x7f, 0x03]),
    ];
    for (what, frame) in bad {
        let client = tokio::spawn(speak(server.addr, pcm("Hi.")));
        let (slot, _, _, _, ids) = worker.synthesize().await;
        worker.send(frame(slot, &ids));
        assert_eq!(worker.next().await, Event::Closed, "{what}");
        let response = client.await.unwrap();
        assert_eq!(response.status(), 503, "{what}");
        assert_eq!(worker.next().await, Event::Connected, "{what}");
        server.worker.connected().await;
    }

    let client = tokio::spawn(speak(server.addr, pcm("Hi.")));
    let (id, ..) = worker.synthesize().await;
    worker.send(worker_side::failed(id, FailCode::Internal));
    assert_eq!(client.await.unwrap().status(), 500);
}

#[tokio::test]
async fn a_bad_hello_is_refused() {
    let server = start(|_| {}, Arc::new(StubPipeline)).await;
    let mut bad = hello();
    bad.voices.push("Not Valid".into());
    let mut worker = ScriptedWorker::listen(&server.socket, worker_side::hello(&bad));
    assert_eq!(worker.next().await, Event::Connected);
    assert_eq!(worker.next().await, Event::Closed);
    assert!(server.worker.current().is_none());
}

#[tokio::test]
async fn long_clauses_are_stitched_from_overlapping_pieces() {
    let (server, mut worker) = with_scripted(|_| {}).await;
    let text = (0..120).map(|i| format!("word{}", char::from(b'a' + (i % 26) as u8))).collect::<Vec<_>>().join(" ");
    let chunk = stub_chunks(&text).remove(0);
    assert!(chunk.phonemes.len() > 450);
    let client = tokio::spawn(speak(server.addr, pcm(&text)));
    let mut pieces = Vec::new();
    while pieces.len() < 2 {
        let (id, _, _, _, ids) = worker.synthesize().await;
        assert!(ids.len() <= 510);
        worker.send(respond(id, &ids));
        pieces.push(ids);
    }
    let response = client.await.unwrap();
    assert_eq!(response.status(), 200);
    let audio = body_bytes(response).await.unwrap();
    assert!(!audio.is_empty());
    assert!(pieces[0].len() < chunk.ids.len() && pieces[1].ends_with(&chunk.ids[chunk.ids.len() - 20..]));
    assert_eq!(worker.style_lens, [chunk.ids.len().min(510) as u16; 2]);
}

/// Below 450 phonemes, a chunk is still stitched when its audio could exceed the worker's chunk
/// cap: a 30 s cap holds 270 ids at speed 1 and 45 at speed 0.25.
#[tokio::test]
async fn chunks_that_could_exceed_the_cap_are_stitched() {
    let text = (0..50).map(|i| format!("word{}", char::from(b'a' + (i % 26) as u8))).collect::<Vec<_>>().join(" ");
    let chunk = stub_chunks(&text).remove(0);
    assert!(chunk.phonemes.chars().count() <= 450 && chunk.ids.len() > 270, "{}", chunk.ids.len());
    for (cap, speed, most) in [(1_440_000, 100, None), (720_000, 100, Some(270)), (720_000, 25, Some(45))] {
        let speed = Speed::from_hundredths(speed).unwrap();
        let expected = match most {
            None => vec![chunk.ids.clone()],
            Some(most) => {
                assert_eq!(max_render_ids(cap, speed), most);
                let plan = Plan::new(chunk.ids.clone(), most);
                (0..plan.pieces()).map(|i| plan.context_ids(i).to_vec()).collect()
            }
        };
        let server = start(|_| {}, Arc::new(StubPipeline)).await;
        let mut worker =
            ScriptedWorker::listen(&server.socket, worker_side::hello(&Hello { max_chunk_samples: cap, ..hello() }));
        server.worker.connected().await;
        let mut body = pcm(&text);
        body["speed"] = speed.as_f64().into();
        let client = tokio::spawn(speak(server.addr, body));
        let mut pieces = Vec::new();
        while pieces.len() < expected.len() {
            let (id, _, _, _, ids) = worker.synthesize().await;
            worker.send(respond(id, &ids));
            pieces.push(ids);
        }
        let response = client.await.unwrap();
        assert_eq!(response.status(), 200);
        assert!(!body_bytes(response).await.unwrap().is_empty());
        assert_eq!(pieces, expected);
        if let Some(most) = most {
            assert!(pieces.len() > 1 && pieces.iter().all(|p| p.len() <= most), "{} pieces", pieces.len());
            assert!(worker.style_lens.iter().all(|&s| usize::from(s) == chunk.ids.len()));
        }
    }
}

/// Below the cap-derived limit's ids, a long sentence is cut at its commas instead of stitched,
/// and the cut ends join at the model's own timing: the first chunk ends right after its comma's
/// frames, the next starts at its first loud sample, so neither keeps Kokoro-FastAPI's padding.
#[tokio::test]
async fn sentences_over_the_cap_limit_split_at_clauses_and_join_at_the_comma() {
    let clause = |c: usize| (0..10).map(move |i| format!("word{}", char::from(b'a' + ((c * 10 + i) % 26) as u8)));
    let text = (0..6).map(|c| clause(c).collect::<Vec<_>>().join(" ")).collect::<Vec<_>>().join(", ") + ".";
    let whole = stub_chunks(&text);
    assert_eq!(whole.len(), 1, "one chunk at the reference's limit");
    let (cap, speed) = (720_000, Speed::NORMAL);
    let limit = max_render_ids(cap, speed);
    assert!(limit == 270 && whole[0].ids.len() > limit);
    let chunks: Vec<_> = StubPipeline.chunks(Lang::AmericanEnglish, &text, limit).map(Result::unwrap).collect();
    assert_eq!(chunks.len(), 2);
    assert!(chunks.iter().all(|c| c.ids.len() <= limit));
    assert!(chunks[0].text.ends_with(',') && chunks[0].split.after && !chunks[0].split.before);
    assert!(chunks[1].split.before && !chunks[1].split.after);

    let server = start(|_| {}, Arc::new(StubPipeline)).await;
    let mut worker =
        ScriptedWorker::listen(&server.socket, worker_side::hello(&Hello { max_chunk_samples: cap, ..hello() }));
    server.worker.connected().await;
    let client = tokio::spawn(speak(server.addr, pcm(&text)));
    // Each id one frame of loud samples, after 3 silent frames for the leading boundary id and
    // before one for the trailing one.
    let render = |ids: &[u8]| -> (Vec<u16>, Vec<i16>) {
        let mut durations = vec![3];
        durations.extend(std::iter::repeat_n(1, ids.len() + 1));
        let mut pcm = vec![0; 1800];
        pcm.extend(std::iter::repeat_n(5000, 600 * ids.len()));
        pcm.extend([0; 600]);
        (durations, pcm)
    };
    let mut sent = Vec::new();
    for _ in 0..2 {
        let (id, _, _, _, ids) = worker.synthesize().await;
        let (durations, pcm) = render(&ids);
        worker.send(worker_side::result(id, &durations, &pcm));
        sent.push(ids);
    }
    let response = client.await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(sent, [chunks[0].ids.clone(), chunks[1].ids.clone()]);
    assert_eq!(worker.style_lens, [chunks[0].ids.len() as u16, chunks[1].ids.len() as u16]);
    // The first keeps 50 ms before its speech and ends after its comma, the last id; the second
    // starts at its speech and keeps the padding after it, cut to the 24 samples trimmed at the end.
    let (_, first) = render(&chunks[0].ids);
    let (_, second) = render(&chunks[1].ids);
    let expected = [&first[600..first.len() - 600], &second[1800..second.len() - 24]].concat();
    assert_eq!(samples(&body_bytes(response).await.unwrap()), expected);
}

#[tokio::test]
async fn pipeline_failures_fail_the_response() {
    for panic in [false, true] {
        let server = start(|_| {}, Arc::new(BrokenPipeline { fail_at: 4, panic })).await;
        let mut worker = ScriptedWorker::listen(&server.socket, worker_side::hello(&hello()));
        server.worker.connected().await;
        let client = tokio::spawn(speak(server.addr, pcm("One part. Two part. Three part. Four part. Five part.")));
        tokio::spawn(async move {
            loop {
                let (id, _, _, _, ids) = worker.synthesize().await;
                worker.send(respond(id, &ids));
            }
        });
        let response = client.await.unwrap();
        assert_eq!(response.status(), 200);
        assert!(body_bytes(response).await.is_err(), "panic: {panic}");

        let server = start(|_| {}, Arc::new(BrokenPipeline { fail_at: 0, panic })).await;
        let _worker = ScriptedWorker::listen(&server.socket, worker_side::hello(&hello()));
        server.worker.connected().await;
        assert_eq!(speak(server.addr, pcm("First part.")).await.status(), 500, "panic: {panic}");
    }
}

#[tokio::test]
async fn speaks_chinese_with_a_chinese_voice() {
    let pipelines = Arc::new(kokoro_server::kokoro::KokoroPipelines::load().unwrap());
    let server = start(|_| {}, pipelines.clone()).await;
    let mut worker = ScriptedWorker::listen(&server.socket, worker_side::hello(&hello()));
    server.worker.connected().await;
    let text = "银行行长在长城上。他说：“你好，世界！”";
    let mut body = pcm(text);
    body["voice"] = json!("zf_xiaobei");
    let client = tokio::spawn(speak(server.addr, body));
    let chunks: Vec<_> = pipelines.chunks(Lang::Chinese, text, DEFAULT_MAX_TOKENS).map(Result::unwrap).collect();
    assert_eq!(chunks.len(), 2);
    let voice = VOICES.iter().position(|&v| v == "zf_xiaobei").unwrap() as u16;
    let mut expected = Vec::new();
    for chunk in &chunks {
        let (id, _, _, v, ids) = worker.synthesize().await;
        assert_eq!((v, &ids), (voice, &chunk.ids));
        worker.send(respond(id, &ids));
        expected.extend(expected_chunk(&ids));
    }
    let response = client.await.unwrap();
    assert_eq!(samples(&body_bytes(response).await.unwrap()), expected);
}

#[tokio::test]
async fn speaks_spanish_with_a_spanish_voice() {
    let pipelines = Arc::new(kokoro_server::kokoro::KokoroPipelines::load().unwrap());
    let server = start(|_| {}, pipelines.clone()).await;
    let mut worker = ScriptedWorker::listen(&server.socket, worker_side::hello(&hello()));
    server.worker.connected().await;
    let text = "¿Dónde está la biblioteca? Está en Москва.";
    let mut body = pcm(text);
    body["voice"] = json!("ef_dora");
    let client = tokio::spawn(speak(server.addr, body));
    let chunks: Vec<_> = pipelines.chunks(Lang::Spanish, text, DEFAULT_MAX_TOKENS).map(Result::unwrap).collect();
    assert_eq!(chunks.len(), 2);
    assert_ne!(
        chunks,
        pipelines.chunks(Lang::AmericanEnglish, text, DEFAULT_MAX_TOKENS).map(Result::unwrap).collect::<Vec<_>>()
    );
    let voice = VOICES.iter().position(|&v| v == "ef_dora").unwrap() as u16;
    let mut expected = Vec::new();
    for chunk in &chunks {
        let (id, _, _, v, ids) = worker.synthesize().await;
        assert_eq!((v, &ids), (voice, &chunk.ids));
        worker.send(respond(id, &ids));
        expected.extend(expected_chunk(&ids));
    }
    let response = client.await.unwrap();
    assert_eq!(samples(&body_bytes(response).await.unwrap()), expected);
}

#[tokio::test]
async fn speaks_english_with_the_real_pipeline() {
    let pipelines = Arc::new(kokoro_server::kokoro::KokoroPipelines::load().unwrap());
    let server = start(|_| {}, pipelines.clone()).await;
    let mut worker = ScriptedWorker::listen(&server.socket, worker_side::hello(&hello()));
    server.worker.connected().await;
    let text = "The quick brown fox jumps over the lazy dog. It costs $5. 東京へ行く";
    let client = tokio::spawn(speak(server.addr, pcm(text)));
    let chunks: Vec<_> =
        pipelines.chunks(Lang::AmericanEnglish, text, DEFAULT_MAX_TOKENS).map(Result::unwrap).collect();

    assert_eq!(chunks.len(), 3);
    // Read by the Japanese G2P when UniDic is installed, else as "Japanese text".
    let japanese =
        if kokoro_pipeline::japanese_available() { ("toːkʲoː e ikɯ", 0) } else { ("ʤˌæpənˈiz tˈɛkst", 1) };
    assert_eq!((chunks[2].phonemes.as_str(), chunks[2].unread), japanese);
    let mut expected = Vec::new();
    for chunk in &chunks {
        let (id, _, _, _, ids) = worker.synthesize().await;
        assert_eq!(ids, chunk.ids);
        worker.send(respond(id, &ids));
        expected.extend(expected_chunk(&ids));
    }
    let response = client.await.unwrap();
    assert_eq!(samples(&body_bytes(response).await.unwrap()), expected);
}
