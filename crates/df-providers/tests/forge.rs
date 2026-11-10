#![cfg(not(target_arch = "wasm32"))]
//! The Forge client against a loopback stand-in for the studio's documented routes.

use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

use df_providers::{
    ForgeCause, ForgeClient, ForgeConfig, ForgeEngine, ForgeJobKind, ForgeMedia, ForgePriority,
};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::Message;

const PROJECT: &str = "41fbb3c9-ac91-4897-b890-7a056b4ae371";
const JOB: &str = "11111111-2222-4333-8444-555555555555";

/// One request the stand-in received.
#[derive(Clone, Debug)]
struct Seen {
    method: String,
    target: String,
    origin: Option<String>,
    kind: Option<String>,
    body: Vec<u8>,
}

/// What the stand-in answers: status, content type and body.
type Answer = (u16, &'static str, Vec<u8>);

/// A stand-in that answers each request with `script` and records it.
struct Studio {
    address: SocketAddr,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl Studio {
    fn start(script: impl Fn(&Seen, usize) -> Answer + Send + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let record = Arc::clone(&seen);
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let Some(request) = read_request(&mut stream) else {
                    continue;
                };
                let count = {
                    let mut seen = record.lock().unwrap();
                    seen.push(request.clone());
                    seen.len()
                };
                let (status, content_type, body) = script(&request, count);
                let head = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\
                     Connection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&body);
            }
        });
        Self { address, seen }
    }

    fn client(&self) -> ForgeClient {
        ForgeClient::new(ForgeConfig::new(&format!("http://{}", self.address), PROJECT).unwrap())
            .unwrap()
    }

    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

fn read_request(stream: &mut TcpStream) -> Option<Seen> {
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    let mut data = Vec::new();
    let end = loop {
        let mut block = [0; 4096];
        let count = stream.read(&mut block).ok()?;
        if count == 0 {
            return None;
        }
        data.extend_from_slice(&block[..count]);
        if let Some(end) = data.windows(4).position(|window| window == b"\r\n\r\n") {
            break end + 4;
        }
    };
    let head = String::from_utf8(data[..end].to_vec()).ok()?;
    let header = |name: &str| {
        head.lines().find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.eq_ignore_ascii_case(name)
                .then(|| value.trim().to_owned())
        })
    };
    let length: usize = header("content-length").map_or(0, |value| value.parse().unwrap());
    while data.len() - end < length {
        let mut block = [0; 4096];
        let count = stream.read(&mut block).ok()?;
        data.extend_from_slice(&block[..count]);
    }
    let mut first = head.lines().next()?.split(' ');
    Some(Seen {
        method: first.next()?.to_owned(),
        target: first.next()?.to_owned(),
        origin: header("origin"),
        kind: header("x-forge-request"),
        body: data[end..end + length].to_vec(),
    })
}

fn json_answer(status: u16, value: Value) -> Answer {
    (
        status,
        "application/json; charset=utf-8",
        value.to_string().into_bytes(),
    )
}

fn services(engine: &str, status: &str) -> Answer {
    json_answer(
        200,
        json!({"services": [
            {"id": "video", "status": "reachable", "modelLoaded": null},
            {"id": engine, "status": status, "modelLoaded": status == "reachable"},
        ]}),
    )
}

fn job(status: &str, output: Option<&str>, details: Value) -> Answer {
    json_answer(
        200,
        json!({"job": {"id": JOB, "status": status, "output": output, "details": details}}),
    )
}

fn soon(seconds: u64) -> Instant {
    Instant::now() + Duration::from_secs(seconds)
}

#[tokio::test(flavor = "current_thread")]
async fn a_voice_starts_its_one_engine_queues_once_polls_and_reads_the_wav() {
    let studio = Studio::start(
        |seen, _| match (seen.method.as_str(), seen.target.as_str()) {
            ("GET", "/api/runtime/services") => services("tts", "reachable"),
            ("POST", "/api/runtime/engines/tts/start") => {
                json_answer(200, json!({"id": "tts", "status": "starting"}))
            }
            ("POST", "/api/runtime/jobs") => job("queued", None, Value::Null),
            ("GET", target) if target.starts_with("/api/runtime/jobs/") => {
                job("completed", Some("voices/forge-1.wav"), Value::Null)
            }
            ("GET", target) if target.contains("/asset?path=voices%2Fforge-1.wav") => {
                (200, "audio/wav", b"RIFF....WAVEfmt ".to_vec())
            }
            _ => json_answer(404, json!({"error": "not found"})),
        },
    );
    let client = studio.client();
    client.prepare(ForgeEngine::Tts, soon(10)).await.unwrap();
    let request = json!({"text": "Hold.", "speaker": "Aiden", "language": "English"});
    let render = client
        .render(ForgeJobKind::Voices, request, soon(10))
        .await
        .unwrap();
    assert_eq!(render.output, "voices/forge-1.wav");
    let wav = client
        .asset(&render.output, ForgeMedia::Wav, soon(10))
        .await
        .unwrap();
    assert!(wav.starts_with(b"RIFF"));

    let seen = studio.seen();
    // The engine already ran, so nothing started it.
    assert!(!seen.iter().any(|seen| seen.target.contains("/start")));
    let queued: Vec<&Seen> = seen
        .iter()
        .filter(|seen| seen.method == "POST" && seen.target == "/api/runtime/jobs")
        .collect();
    assert_eq!(queued.len(), 1);
    let body: Value = serde_json::from_slice(&queued[0].body).unwrap();
    assert_eq!(body["type"], "voices");
    assert_eq!(body["projectId"], PROJECT);
    assert_eq!(body["request"]["speaker"], "Aiden");
    assert_eq!(queued[0].kind.as_deref(), Some("runtime"));
    let origin = format!("http://{}", studio.address);
    assert_eq!(queued[0].origin.as_deref(), Some(origin.as_str()));
}

#[tokio::test(flavor = "current_thread")]
async fn an_engine_that_does_not_run_is_started_alone_and_waited_for() {
    let started = Arc::new(Mutex::new(false));
    let flag = Arc::clone(&started);
    let studio = Studio::start(move |seen, _| match seen.target.as_str() {
        "/api/runtime/services" => {
            let running = *flag.lock().unwrap();
            services(
                "segmentation",
                if running { "reachable" } else { "unavailable" },
            )
        }
        "/api/runtime/engines/segmentation/start" => {
            *flag.lock().unwrap() = true;
            json_answer(200, json!({"id": "segmentation", "status": "starting"}))
        }
        _ => json_answer(404, json!({"error": "not found"})),
    });
    studio
        .client()
        .prepare(ForgeEngine::Segmentation, soon(10))
        .await
        .unwrap();
    let starts: Vec<String> = studio
        .seen()
        .into_iter()
        .filter(|seen| seen.method == "POST")
        .map(|seen| seen.target)
        .collect();
    assert_eq!(starts, vec!["/api/runtime/engines/segmentation/start"]);
}

#[tokio::test(flavor = "current_thread")]
async fn an_interrupted_render_is_an_error_and_is_never_resubmitted() {
    let studio = Studio::start(|seen, _| match seen.method.as_str() {
        "POST" => job("queued", None, Value::Null),
        _ => job("interrupted", None, Value::Null),
    });
    let failed = studio
        .client()
        .render(ForgeJobKind::Voices, json!({"text": "x"}), soon(10))
        .await
        .unwrap_err();
    assert_eq!(failed.cause, ForgeCause::JobInterrupted);
    let posts = studio
        .seen()
        .iter()
        .filter(|seen| seen.method == "POST")
        .count();
    assert_eq!(posts, 1);
}

#[tokio::test(flavor = "current_thread")]
async fn a_render_still_waiting_at_its_deadline_is_withdrawn() {
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&cancelled);
    let studio = Studio::start(move |seen, _| {
        if seen.target.ends_with("/cancel") {
            flag.store(true, Ordering::SeqCst);
            job("cancelled", None, Value::Null)
        } else if flag.load(Ordering::SeqCst) {
            job("cancelled", None, Value::Null)
        } else {
            job("waiting_engine", None, Value::Null)
        }
    });
    let until = Instant::now() + Duration::from_millis(900);
    let failed = studio
        .client()
        .render(ForgeJobKind::Images, json!({"prompt": "x"}), until)
        .await
        .unwrap_err();
    assert_eq!(failed.cause, ForgeCause::Deadline);
    let cancels: Vec<String> = studio
        .seen()
        .into_iter()
        .filter(|seen| seen.target.ends_with("/cancel"))
        .map(|seen| seen.target)
        .collect();
    assert_eq!(cancels, vec![format!("/api/runtime/jobs/{JOB}/cancel")]);
}

#[tokio::test(flavor = "current_thread")]
async fn a_segmentation_answers_its_matches_and_a_staged_picture_its_id() {
    let studio = Studio::start(|seen, _| match seen.target.as_str() {
        target if target.ends_with("/references") => {
            assert_eq!(seen.kind.as_deref(), Some("project"));
            json_answer(
                201,
                json!({"reference": {"id": "ref-1.png", "mime": "image/png"}}),
            )
        }
        "/api/runtime/jobs" => job("queued", None, Value::Null),
        _ => job(
            "completed",
            Some("masks/forge-1.png"),
            json!({"result": {"found": 3, "used": 1, "scores": [0.92, 0.9, 0.81]}}),
        ),
    });
    let client = studio.client();
    let staged = client
        .stage_reference(b"\x89PNG\r\n\x1a\nrest", soon(10))
        .await
        .unwrap();
    assert_eq!(staged.as_str(), "ref-1.png");
    let render = client
        .render(
            ForgeJobKind::Segmentation,
            json!({"prompt": "person"}),
            soon(10),
        )
        .await
        .unwrap();
    let matches = render.matches.unwrap();
    assert_eq!((matches.found, matches.used), (3, 1));
    assert_eq!(matches.scores.len(), 3);
}

#[tokio::test(flavor = "current_thread")]
async fn a_failed_render_and_a_refused_request_keep_their_causes() {
    let studio = Studio::start(|seen, _| match seen.method.as_str() {
        "POST" if posted_text_is_empty(seen) => json_answer(
            400,
            json!({"error": "Voice text must contain 1–1000 characters.", "supervisor": {}}),
        ),
        "POST" => job("queued", None, Value::Null),
        _ => job("failed", None, Value::Null),
    });
    let client = studio.client();
    let refused = client
        .render(ForgeJobKind::Voices, json!({"text": ""}), soon(10))
        .await
        .unwrap_err();
    assert_eq!(refused.cause, ForgeCause::Rejected);
    assert_eq!(
        refused.reason.as_deref(),
        Some("Voice text must contain 1–1000 characters.")
    );
    assert_eq!(
        refused.to_string(),
        "forge refused the request: Voice text must contain 1–1000 characters."
    );
    let failed = client
        .render(ForgeJobKind::Voices, json!({"text": "x"}), soon(10))
        .await
        .unwrap_err();
    assert_eq!(failed.cause, ForgeCause::JobFailed);
}

#[tokio::test(flavor = "current_thread")]
async fn a_recording_is_transcribed_through_the_realtime_protocol() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let send = |value: Value| Message::Text(value.to_string().into());
        socket
            .send(send(json!({"type": "session.created"})))
            .await
            .unwrap();
        let update = socket.next().await.unwrap().unwrap().into_text().unwrap();
        let update: Value = serde_json::from_str(&update).unwrap();
        assert_eq!(update["type"], "session.update");
        assert_eq!(update["session"]["sample_rate"], 16_000);
        socket
            .send(send(json!({"type": "session.updated"})))
            .await
            .unwrap();
        let mut audio = 0;
        loop {
            match socket.next().await.unwrap().unwrap() {
                Message::Binary(frame) => audio += frame.len(),
                Message::Text(text) => {
                    let event: Value = serde_json::from_str(&text).unwrap();
                    assert_eq!(event["type"], "input_audio_buffer.commit");
                    break;
                }
                other => panic!("unexpected {other:?}"),
            }
        }
        socket
            .send(send(json!({
                "type": "conversation.item.input_audio_transcription.delta",
                "delta": "Hold your",
            })))
            .await
            .unwrap();
        socket
            .send(send(json!({
                "type": "conversation.item.input_audio_transcription.completed",
                "transcript": " Hold your tongue, stranger. ",
            })))
            .await
            .unwrap();
        audio
    });
    let client =
        ForgeClient::new(ForgeConfig::new(&format!("http://{address}"), PROJECT).unwrap()).unwrap();
    let pcm = vec![0_u8; 40_000];
    let heard = client.transcribe(&pcm, 16_000, soon(10)).await.unwrap();
    assert_eq!(heard, "Hold your tongue, stranger.");
    assert_eq!(server.await.unwrap(), 40_000);
}

#[tokio::test(flavor = "current_thread")]
async fn an_unreachable_studio_is_a_transport_failure() {
    // A port nothing listens on: bind one and drop it.
    let address = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let client =
        ForgeClient::new(ForgeConfig::new(&format!("http://{address}"), PROJECT).unwrap()).unwrap();
    let failed = client.prepare(ForgeEngine::Asr, soon(5)).await.unwrap_err();
    assert!(
        matches!(failed.cause, ForgeCause::Transport(_)),
        "{failed:?}"
    );
}

/// `GET /api/runtime/jobs` as the studio answers it: open jobs first, in queue order.
fn listing(jobs: &[(&str, &str)]) -> Answer {
    let jobs: Vec<Value> = jobs
        .iter()
        .map(|(id, status)| json!({"id": id, "status": status, "type": "images"}))
        .collect();
    json_answer(200, json!({ "jobs": jobs }))
}

fn cancels(studio: &Studio) -> usize {
    studio
        .seen()
        .iter()
        .filter(|seen| seen.target.ends_with("/cancel"))
        .count()
}

const OTHER: &str = "88888888-2222-4333-8444-555555555555";

#[tokio::test(flavor = "current_thread")]
async fn a_blocked_queue_is_refused_before_anything_is_submitted() {
    for (status, cause) in [
        ("waiting_memory", ForgeCause::QueueWaitingMemory),
        ("interrupted", ForgeCause::QueueInterrupted),
        ("needs_attention", ForgeCause::QueueNeedsAttention),
    ] {
        let studio = Studio::start(move |seen, _| match seen.method.as_str() {
            "GET" if seen.target == "/api/runtime/jobs" => listing(&[
                ("99999999-2222-4333-8444-555555555555", "completed"),
                (OTHER, status),
            ]),
            _ => job("queued", None, Value::Null),
        });
        let started = Instant::now();
        let refused = studio
            .client()
            .render(ForgeJobKind::Images, json!({"prompt": "x"}), soon(900))
            .await
            .unwrap_err();
        assert_eq!(refused.cause, cause, "{status}");
        assert!(started.elapsed() < Duration::from_secs(5), "{status}");
        let posts = studio
            .seen()
            .iter()
            .filter(|seen| seen.method == "POST")
            .count();
        assert_eq!(
            posts, 0,
            "{status}: nothing is queued behind a blocked queue"
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn a_queue_that_blocks_while_the_render_waits_ends_the_wait_and_withdraws_it() {
    let cancelled = Arc::new(AtomicBool::new(false));
    let lists = Arc::new(AtomicUsize::new(0));
    let (flag, reads) = (Arc::clone(&cancelled), Arc::clone(&lists));
    let studio = Studio::start(move |seen, _| {
        match (seen.method.as_str(), seen.target.as_str()) {
            ("GET", "/api/runtime/jobs") => {
                // The first read, before the submit, finds a clear queue; the later ones do not.
                if reads.fetch_add(1, Ordering::SeqCst) == 0 {
                    listing(&[])
                } else {
                    listing(&[(OTHER, "interrupted"), (JOB, "queued")])
                }
            }
            ("POST", target) if target.ends_with("/cancel") => {
                flag.store(true, Ordering::SeqCst);
                job("cancelled", None, Value::Null)
            }
            ("POST", _) => job("queued", None, Value::Null),
            _ if flag.load(Ordering::SeqCst) => job("cancelled", None, Value::Null),
            _ => job("queued", None, Value::Null),
        }
    });
    let started = Instant::now();
    let refused = studio
        .client()
        .render(ForgeJobKind::Images, json!({"prompt": "x"}), soon(900))
        .await
        .unwrap_err();
    assert_eq!(refused.cause, ForgeCause::QueueInterrupted);
    assert!(started.elapsed() < Duration::from_secs(10));
    assert_eq!(cancels(&studio), 1, "the render it gave up on is withdrawn");
}

#[tokio::test(flavor = "current_thread")]
async fn a_blocked_job_behind_a_render_does_not_hold_it_back() {
    // Before the submit every listed job is ahead; once queued, only the jobs before it count.
    let lists = Arc::new(AtomicUsize::new(0));
    let reads = Arc::clone(&lists);
    let studio = Studio::start(
        move |seen, _| match (seen.method.as_str(), seen.target.as_str()) {
            ("GET", "/api/runtime/jobs") => {
                if reads.fetch_add(1, Ordering::SeqCst) == 0 {
                    listing(&[])
                } else {
                    listing(&[(JOB, "queued"), (OTHER, "interrupted")])
                }
            }
            ("POST", _) => job("queued", None, Value::Null),
            _ => job("completed", Some("images/forge-1.png"), Value::Null),
        },
    );
    let render = studio
        .client()
        .render(ForgeJobKind::Images, json!({"prompt": "x"}), soon(10))
        .await
        .expect("renders");
    assert_eq!(render.output, "images/forge-1.png");
    assert_eq!(cancels(&studio), 0);
}

#[tokio::test(flavor = "current_thread")]
async fn a_failed_poll_still_withdraws_a_render_last_seen_waiting() {
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&cancelled);
    let studio = Studio::start(
        move |seen, _| match (seen.method.as_str(), seen.target.as_str()) {
            ("POST", target) if target.ends_with("/cancel") => {
                flag.store(true, Ordering::SeqCst);
                job("cancelled", None, Value::Null)
            }
            ("POST", _) => job("queued", None, Value::Null),
            _ if flag.load(Ordering::SeqCst) => job("cancelled", None, Value::Null),
            ("GET", target) if target.starts_with("/api/runtime/jobs/") => {
                json_answer(500, json!({"error": "studio busy"}))
            }
            _ => json_answer(404, json!({"error": "not found"})),
        },
    );
    let failed = studio
        .client()
        .render(ForgeJobKind::Images, json!({"prompt": "x"}), soon(10))
        .await
        .unwrap_err();
    assert_eq!(failed.cause, ForgeCause::Failed);
    assert_eq!(cancels(&studio), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn a_withdrawal_tries_again_while_forge_checks_the_job() {
    // Forge refuses to cancel a job it is checking (`preparing`) and answers 400; the second try
    // lands.
    let attempts = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&attempts);
    let studio = Studio::start(
        move |seen, _| match (seen.method.as_str(), seen.target.as_str()) {
            ("POST", target) if target.ends_with("/cancel") => {
                if counted.fetch_add(1, Ordering::SeqCst) == 0 {
                    json_answer(
                        400,
                        json!({"error": "Only queued jobs can be cancelled here."}),
                    )
                } else {
                    job("cancelled", None, Value::Null)
                }
            }
            ("POST", _) => job("queued", None, Value::Null),
            _ if counted.load(Ordering::SeqCst) >= 2 => job("cancelled", None, Value::Null),
            _ => job("preparing", None, Value::Null),
        },
    );
    let until = Instant::now() + Duration::from_millis(900);
    let failed = studio
        .client()
        .render(ForgeJobKind::Images, json!({"prompt": "x"}), until)
        .await
        .unwrap_err();
    assert_eq!(failed.cause, ForgeCause::Deadline);
    assert_eq!(cancels(&studio), 2);
}

#[tokio::test(flavor = "current_thread")]
async fn a_job_that_started_meanwhile_is_left_to_finish_after_one_refused_cancel() {
    let refused = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&refused);
    let studio = Studio::start(
        move |seen, _| match (seen.method.as_str(), seen.target.as_str()) {
            ("POST", target) if target.ends_with("/cancel") => {
                flag.store(true, Ordering::SeqCst);
                json_answer(
                    400,
                    json!({"error": "Only queued jobs can be cancelled here."}),
                )
            }
            ("POST", _) => job("queued", None, Value::Null),
            _ if refused.load(Ordering::SeqCst) => job("running", None, Value::Null),
            _ => job("waiting_engine", None, Value::Null),
        },
    );
    let until = Instant::now() + Duration::from_millis(900);
    let failed = studio
        .client()
        .render(ForgeJobKind::Images, json!({"prompt": "x"}), until)
        .await
        .unwrap_err();
    assert_eq!(failed.cause, ForgeCause::Deadline);
    assert_eq!(cancels(&studio), 1);
}

/// Whether the job posted asks for an empty text.
fn posted_text_is_empty(seen: &Seen) -> bool {
    serde_json::from_slice::<Value>(&seen.body).is_ok_and(|body| body["request"]["text"] == "")
}

// The `FORGE` audit lines of this binary's tests, kept by a sink installed once (the sink is
// process-wide, so a test finds its own line by the id of the job it submitted).
static AUDIT: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn keep_audit(_event: &'static str, line: String) {
    AUDIT.lock().unwrap().push(line);
}

fn prioritised_lines(job: &str) -> Vec<String> {
    let prefix = format!("FORGE prioritised job={} ", &job[..8]);
    AUDIT
        .lock()
        .unwrap()
        .iter()
        .filter(|line| line.starts_with(&prefix))
        // The audit path appends the correlation (` op=.. seat=.. ms=..`) to every line.
        .map(|line| line.split(" op=").next().unwrap_or(line).to_owned())
        .collect()
}

const URGENT: &str = "aaaaaaaa-2222-4333-8444-555555555555";
const ORDINARY: &str = "bbbbbbbb-2222-4333-8444-555555555555";
const RUNNING: &str = "cccccccc-2222-4333-8444-555555555555";
const REFUSED: &str = "dddddddd-2222-4333-8444-555555555555";
const MISSING: &str = "eeeeeeee-2222-4333-8444-555555555555";

fn job_of(id: &str, status: &str, output: Option<&str>) -> Answer {
    json_answer(
        200,
        json!({"job": {"id": id, "status": status, "output": output, "details": Value::Null}}),
    )
}

/// A studio that queues every job as `id` in `first` status, answers the move with `moved`, and
/// reports the job completed once polled.
fn studio_moving(id: &'static str, first: &'static str, moved: Answer) -> Studio {
    let (status, content_type, body) = moved;
    Studio::start(
        move |seen, _| match (seen.method.as_str(), seen.target.as_str()) {
            ("GET", "/api/runtime/jobs") => listing(&[]),
            ("POST", target) if target.ends_with("/move") => (status, content_type, body.clone()),
            ("POST", _) => job_of(id, first, None),
            _ => job_of(id, "completed", Some("images/x.png")),
        },
    )
}

fn moves(studio: &Studio) -> Vec<Seen> {
    studio
        .seen()
        .into_iter()
        .filter(|seen| seen.target.ends_with("/move"))
        .collect()
}

fn queued_moves() -> Answer {
    json_answer(
        200,
        json!({"moved": true, "position": 1, "count": 3, "order": [], "ranks": {},
               "clamped": false, "carried": [], "renumbered": false, "message": "", "job": {}}),
    )
}

#[tokio::test(flavor = "current_thread")]
async fn an_emergency_render_is_put_first_in_the_line_once_and_only_its_own_job() {
    df_observe::install_audit_sink(keep_audit);
    let studio = studio_moving(URGENT, "queued", queued_moves());
    let render = studio
        .client()
        .render_with(
            ForgeJobKind::Images,
            json!({"prompt": "x"}),
            soon(10),
            ForgePriority::Emergency,
        )
        .await
        .expect("renders");
    assert_eq!(render.output, "images/x.png");
    let seen = studio.seen();
    let moved = moves(&studio);
    assert_eq!(moved.len(), 1, "one move, for the one job it submitted");
    assert_eq!(moved[0].target, format!("/api/runtime/jobs/{URGENT}/move"));
    assert_eq!(moved[0].body, br#"{"to":"top"}"#);
    assert_eq!(moved[0].kind.as_deref(), Some("runtime"));
    assert_eq!(
        moved[0].origin.as_deref(),
        Some(format!("http://{}", studio.address).as_str())
    );
    let submit = seen
        .iter()
        .position(|seen| seen.method == "POST" && seen.target == "/api/runtime/jobs")
        .expect("the submit");
    let move_at = seen
        .iter()
        .position(|seen| seen.target.ends_with("/move"))
        .expect("the move");
    assert!(submit < move_at, "it moves the job it has queued");
    assert_eq!(
        prioritised_lines(URGENT),
        [concat!(
            "FORGE prioritised job=aaaaaaaa outcome=moved status=queued answer=200 ",
            "position=1 waiting=3"
        )]
    );
}

#[tokio::test(flavor = "current_thread")]
async fn a_normal_render_asks_for_no_move() {
    df_observe::install_audit_sink(keep_audit);
    let studio = studio_moving(ORDINARY, "queued", queued_moves());
    studio
        .client()
        .render(ForgeJobKind::Images, json!({"prompt": "x"}), soon(10))
        .await
        .expect("renders");
    assert!(moves(&studio).is_empty());
    assert!(prioritised_lines(ORDINARY).is_empty());
}

#[tokio::test(flavor = "current_thread")]
async fn a_studio_without_the_move_route_leaves_the_render_in_its_place_and_says_unavailable() {
    df_observe::install_audit_sink(keep_audit);
    let studio = studio_moving(
        MISSING,
        "queued",
        json_answer(404, json!({"error": "Not found"})),
    );
    let render = studio
        .client()
        .render_with(
            ForgeJobKind::Images,
            json!({"prompt": "x"}),
            soon(10),
            ForgePriority::Emergency,
        )
        .await
        .expect("it still renders, in its place in the line");
    assert_eq!(render.output, "images/x.png");
    assert_eq!(moves(&studio).len(), 1);
    assert_eq!(
        prioritised_lines(MISSING),
        [concat!(
            "FORGE prioritised job=eeeeeeee outcome=unavailable status=queued answer=404 ",
            "position=- waiting=-"
        )]
    );
}

#[tokio::test(flavor = "current_thread")]
async fn a_move_the_studio_understood_and_refused_is_refused_and_the_render_goes_on() {
    df_observe::install_audit_sink(keep_audit);
    let studio = studio_moving(
        REFUSED,
        "queued",
        json_answer(
            409,
            json!({"error": "A render that already has an engine job cannot be moved.",
                   "code": "not_movable"}),
        ),
    );
    studio
        .client()
        .render_with(
            ForgeJobKind::Images,
            json!({"prompt": "x"}),
            soon(10),
            ForgePriority::Emergency,
        )
        .await
        .expect("renders");
    assert_eq!(
        prioritised_lines(REFUSED),
        [concat!(
            "FORGE prioritised job=dddddddd outcome=refused status=queued answer=409 ",
            "position=- waiting=-"
        )]
    );
}

#[tokio::test(flavor = "current_thread")]
async fn a_job_forge_already_runs_is_never_asked_to_move() {
    df_observe::install_audit_sink(keep_audit);
    let studio = studio_moving(RUNNING, "running", queued_moves());
    studio
        .client()
        .render_with(
            ForgeJobKind::Images,
            json!({"prompt": "x"}),
            soon(10),
            ForgePriority::Emergency,
        )
        .await
        .expect("renders");
    assert!(moves(&studio).is_empty(), "a running job is not touched");
    assert_eq!(
        prioritised_lines(RUNNING),
        [concat!(
            "FORGE prioritised job=cccccccc outcome=refused status=running answer=none ",
            "position=- waiting=-"
        )]
    );
}
