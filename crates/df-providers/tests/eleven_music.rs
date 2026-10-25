#![cfg(not(target_arch = "wasm32"))]

use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener},
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

use df_providers::{
    ElevenMusicConfig, ElevenMusicError, MUSIC_MAXIMUM_PROMPT_CHARACTERS,
    MUSIC_MINIMUM_LENGTH_MILLIS, NativeElevenMusicProvider,
};

struct Captured {
    request_line: String,
    credential: Option<String>,
    body: serde_json::Value,
}

/// One-shot loopback stand-in for the ElevenLabs endpoints.
struct Fake {
    address: SocketAddr,
    captured: Arc<Mutex<Vec<Captured>>>,
}

impl Fake {
    fn start(status: u16, content_type: &'static str, body: &'static [u8]) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let captured = Arc::new(Mutex::new(Vec::new()));
        let output = captured.clone();
        thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut data = Vec::new();
            let end = loop {
                let mut block = [0; 1024];
                let count = socket.read(&mut block).unwrap();
                assert!(count > 0);
                data.extend_from_slice(&block[..count]);
                if let Some(end) = data.windows(4).position(|window| window == b"\r\n\r\n") {
                    break end + 4;
                }
            };
            let head = String::from_utf8(data[..end].to_vec()).unwrap();
            let length = head
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|value| value.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            while data.len() - end < length {
                let mut block = [0; 1024];
                let count = socket.read(&mut block).unwrap();
                assert!(count > 0);
                data.extend_from_slice(&block[..count]);
            }
            output.lock().unwrap().push(Captured {
                request_line: head.lines().next().unwrap().to_owned(),
                credential: head.lines().find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("xi-api-key:")
                        .map(|value| value.trim().to_owned())
                }),
                body: serde_json::from_slice(&data[end..end + length]).unwrap(),
            });
            let reply = format!(
                "HTTP/1.1 {status} X\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            socket.write_all(reply.as_bytes()).unwrap();
            socket.write_all(body).unwrap();
        });
        Self { address, captured }
    }

    fn provider(&self) -> NativeElevenMusicProvider {
        NativeElevenMusicProvider::new(
            ElevenMusicConfig::loopback_fixture(self.address).unwrap(),
            "test-key",
            Duration::from_secs(5),
        )
        .unwrap()
    }
}

#[tokio::test(flavor = "current_thread")]
async fn composing_sends_an_instrumental_music_request_and_returns_the_audio() {
    let fake = Fake::start(200, "audio/mpeg", b"ID3-fake-audio");
    let audio = fake
        .provider()
        .compose("Instrumental score.", 48_000, Duration::from_secs(5))
        .await
        .unwrap();
    assert_eq!(audio.bytes, b"ID3-fake-audio");
    let captured = fake.captured.lock().unwrap();
    assert!(
        captured[0]
            .request_line
            .starts_with("POST /v1/music?output_format=")
    );
    assert_eq!(captured[0].credential.as_deref(), Some("test-key"));
    assert_eq!(captured[0].body["prompt"], "Instrumental score.");
    assert_eq!(captured[0].body["music_length_ms"], 48_000);
    assert_eq!(captured[0].body["force_instrumental"], true);
}

#[tokio::test(flavor = "current_thread")]
async fn an_ambience_request_asks_for_a_loop() {
    let fake = Fake::start(200, "audio/mpeg", b"ID3-fake-ambience");
    fake.provider()
        .ambience("A quiet shop.", 24, Duration::from_secs(5))
        .await
        .unwrap();
    let captured = fake.captured.lock().unwrap();
    assert!(
        captured[0]
            .request_line
            .starts_with("POST /v1/sound-generation?output_format=")
    );
    assert_eq!(captured[0].body["loop"], true);
    assert_eq!(captured[0].body["duration_seconds"], 24);
    assert_eq!(captured[0].body["text"], "A quiet shop.");
}

#[tokio::test(flavor = "current_thread")]
async fn a_rejected_credential_and_other_failures_are_typed_without_the_supplier_text() {
    let denied = Fake::start(401, "application/json", br#"{"detail":"secret detail"}"#);
    assert_eq!(
        denied
            .provider()
            .compose("x", 10_000, Duration::from_secs(5))
            .await
            .unwrap_err(),
        ElevenMusicError::Unauthorized
    );
    let busy = Fake::start(429, "application/json", b"{}");
    assert_eq!(
        busy.provider()
            .compose("x", 10_000, Duration::from_secs(5))
            .await
            .unwrap_err(),
        ElevenMusicError::Provider(429)
    );
}

#[tokio::test(flavor = "current_thread")]
async fn requests_outside_the_documented_bounds_send_nothing() {
    let fake = Fake::start(200, "audio/mpeg", b"unused");
    let provider = fake.provider();
    let long = "x".repeat(MUSIC_MAXIMUM_PROMPT_CHARACTERS + 1);
    for (prompt, length) in [
        ("   ", 10_000),
        ("ok", MUSIC_MINIMUM_LENGTH_MILLIS - 1),
        (long.as_str(), 10_000),
    ] {
        assert_eq!(
            provider
                .compose(prompt, length, Duration::from_secs(1))
                .await
                .unwrap_err(),
            ElevenMusicError::InvalidRequest
        );
    }
    assert_eq!(
        provider
            .ambience("ok", 31, Duration::from_secs(1))
            .await
            .unwrap_err(),
        ElevenMusicError::InvalidRequest
    );
    assert!(fake.captured.lock().unwrap().is_empty());
}

#[test]
fn a_bad_credential_or_a_remote_address_is_not_accepted() {
    for credential in ["", "has space"] {
        assert!(
            NativeElevenMusicProvider::new(
                ElevenMusicConfig::live(),
                credential,
                Duration::from_secs(5)
            )
            .is_err()
        );
    }
    assert!(ElevenMusicConfig::loopback_fixture("8.8.8.8:443".parse().unwrap()).is_err());
}
