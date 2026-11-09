#![cfg(not(target_arch = "wasm32"))]
//! The Forge client against a loopback stand-in for the studio's documented routes.

use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

use df_providers::{ForgeCause, ForgeClient, ForgeConfig, ForgeEngine, ForgeJobKind, ForgeMedia};
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
    let studio = Studio::start(|seen, _| {
        if seen.target.ends_with("/cancel") {
            json_answer(200, json!({"cancelled": true}))
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
    let studio = Studio::start(|seen, count| match (seen.method.as_str(), count) {
        ("POST", 1) => json_answer(400, json!({"error": "Voice text must contain 1–1000"})),
        ("POST", _) => job("queued", None, Value::Null),
        _ => job("failed", None, Value::Null),
    });
    let client = studio.client();
    let refused = client
        .render(ForgeJobKind::Voices, json!({"text": ""}), soon(10))
        .await
        .unwrap_err();
    assert_eq!(refused.cause, ForgeCause::Rejected);
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
