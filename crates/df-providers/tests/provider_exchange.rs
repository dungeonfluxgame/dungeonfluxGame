#![cfg(not(target_arch = "wasm32"))]
//! Every `NativeHttp` exchange writes one `PROVIDER exchange` audit line of closed keys and
//! numbers: never the URL path or query, the credential or a body.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use df_providers::{HttpRequest, NativeHttp, NativeHttpCause, ProviderLimits};

const CREDENTIAL: &str = "fixture-secret-credential";

fn captured() -> &'static Mutex<Vec<(&'static str, String)>> {
    static LINES: OnceLock<Mutex<Vec<(&'static str, String)>>> = OnceLock::new();
    LINES.get_or_init(Mutex::default)
}

fn capture(event: &'static str, line: String) {
    captured().lock().unwrap().push((event, line));
}

/// The captured `PROVIDER` lines that name `operation`.
fn lines_for(operation: &str) -> Vec<String> {
    let label = format!(" operation={operation} ");
    captured()
        .lock()
        .unwrap()
        .iter()
        .filter(|(event, line)| *event == "PROVIDER" && line.contains(&label))
        .map(|(_, line)| line.clone())
        .collect()
}

/// Reads one request through its body, so closing the socket never resets an unread request.
fn read_request(stream: &mut TcpStream) {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        let count = stream.read(&mut chunk).unwrap();
        assert_ne!(count, 0, "the client closed before its headers");
        bytes.extend_from_slice(&chunk[..count]);
        if let Some(at) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break at + 4;
        }
    };
    let head = String::from_utf8_lossy(&bytes[..header_end]).to_ascii_lowercase();
    let length: usize = head
        .lines()
        .find_map(|line| line.strip_prefix("content-length:"))
        .map_or(0, |value| value.trim().parse().unwrap());
    while bytes.len() < header_end + length {
        let count = stream.read(&mut chunk).unwrap();
        assert_ne!(count, 0, "the client closed before its body");
        bytes.extend_from_slice(&chunk[..count]);
    }
}

/// Serves one request with `status` and a small JSON body.
fn serve_once(status: &'static str) -> (SocketAddr, std::thread::JoinHandle<()>) {
    serve_with(move |stream| {
        write!(
            stream,
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nx-request-id: req-fixture-1\r\n\
             Content-Length: 2\r\nConnection: close\r\n\r\n{{}}"
        )
        .unwrap();
    })
}

/// Serves one media request with 200, `content_type` and `body`.
fn serve_media_once(
    content_type: &'static str,
    body: &'static [u8],
) -> (SocketAddr, std::thread::JoinHandle<()>) {
    serve_with(move |stream| {
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\
             Connection: close\r\n\r\n",
            body.len()
        )
        .unwrap();
        stream.write_all(body).unwrap();
    })
}

/// Accepts one connection within a bounded wait, reads its request and lets `answer` reply.
fn serve_with(
    answer: impl FnOnce(&mut TcpStream) + Send + 'static,
) -> (SocketAddr, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        && Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(error) => panic!("bounded fixture accept: {error}"),
            }
        };
        stream.set_nonblocking(false).unwrap();
        read_request(&mut stream);
        answer(&mut stream);
    });
    (address, server)
}

fn transport() -> NativeHttp {
    let limits = ProviderLimits::new(Duration::from_secs(5), 1024, 1024).build();
    NativeHttp::new(limits, true).unwrap()
}

#[tokio::test]
async fn a_rate_limited_exchange_writes_one_line_without_the_path_query_or_key() {
    df_observe::install_audit_sink(capture);
    let (address, server) = serve_once("429 Too Many Requests");
    let body = b"{\"input\":\"private prompt\"}".to_vec();
    let sent_bytes = format!(" request_bytes={} ", body.len());
    let request = HttpRequest::bearer_native(
        &format!("http://{address}/v1/responses?private=query"),
        CREDENTIAL,
        "application/json".to_owned(),
        body,
    )
    .with_operation("fixture_rate_limited");
    let response = transport()
        .execute(request, Duration::from_secs(5), std::future::pending())
        .await
        .unwrap_or_else(|failure| panic!("transport refused: {:?}", failure.cause));
    server.join().unwrap();
    assert_eq!(response.status, 429);
    let lines = lines_for("fixture_rate_limited");
    assert_eq!(lines.len(), 1, "{lines:?}");
    let line = &lines[0];
    for field in [
        "PROVIDER exchange provider=openai operation=fixture_rate_limited host=127.0.0.1 ",
        " status=429 cause=rate_limited send_state=known_sent ",
        &sent_bytes,
        " response_bytes=2 duration_ms=",
        " request_id=req-fixture-1",
    ] {
        assert!(line.contains(field), "{field:?} missing from {line}");
    }
    for private in ["?", CREDENTIAL, "/v1", "private", "{"] {
        assert!(!line.contains(private), "{private:?} leaked into {line}");
    }
}

#[tokio::test]
async fn an_exchange_refused_before_sending_is_unsent_with_its_cause_key() {
    df_observe::install_audit_sink(capture);
    let request = HttpRequest::bearer_native(
        "http://127.0.0.1:9/v1/responses",
        CREDENTIAL,
        "application/json".to_owned(),
        vec![b'x'; 2048],
    )
    .with_operation("fixture_too_large");
    let failure = transport()
        .execute(request, Duration::from_secs(5), std::future::pending())
        .await
        .err()
        .expect("a body over the bound is refused");
    assert_eq!(failure.cause, NativeHttpCause::RequestTooLarge);
    let lines = lines_for("fixture_too_large");
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(
        lines[0].contains(
            " status=- cause=request_too_large send_state=unsent request_bytes=2048 \
             response_bytes=0 "
        ),
        "{}",
        lines[0]
    );
}

#[tokio::test]
async fn an_unchecked_operation_label_is_written_as_a_dash() {
    df_observe::install_audit_sink(capture);
    let request = HttpRequest::fal_native(
        df_providers::HttpMethod::Get,
        "http://127.0.0.1:9/queue",
        CREDENTIAL,
        Vec::new(),
    )
    .with_operation("Not A Label");
    // A cancelled exchange never sends, so the label check is all this call exercises.
    let failure = transport()
        .execute(request, Duration::from_secs(5), std::future::ready(()))
        .await
        .err()
        .expect("a cancelled exchange gives no answer");
    assert_eq!(failure.cause, NativeHttpCause::Cancelled);
    let lines: Vec<String> = captured()
        .lock()
        .unwrap()
        .iter()
        .map(|(_, line)| line.clone())
        .filter(|line| line.contains(" provider=fal operation=- ") && line.contains("cancelled"))
        .collect();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(
        lines[0].contains(" cause=cancelled send_state=unsent "),
        "{}",
        lines[0]
    );
    assert!(!lines[0].contains("Not A Label"));
}

/// The captured `PROVIDER` download lines of `provider`.
fn download_lines(provider: &str) -> Vec<String> {
    let label = format!("PROVIDER exchange provider={provider} operation=download ");
    lines_for("download")
        .into_iter()
        .filter(|line| line.starts_with(&label))
        .collect()
}

#[tokio::test]
async fn a_bfl_delivery_download_writes_one_line_without_the_path_or_query() {
    df_observe::install_audit_sink(capture);
    let body: &'static [u8] = b"synthetic-png-bytes";
    let (address, server) = serve_media_once("image/png", body);
    let locator = format!("http://{address}/private/sample.png?sig=private");
    let (bytes, mime, status) = df_providers::fetch_bfl_delivery(
        &transport(),
        &locator,
        1024,
        Duration::from_secs(5),
        Some(address),
    )
    .await
    .unwrap_or_else(|failure| panic!("delivery refused: {}", failure.cause.key()));
    server.join().unwrap();
    assert_eq!(
        (bytes.as_slice(), mime.as_str(), status),
        (body, "image/png", Some(200))
    );
    let lines = download_lines("bfl");
    assert_eq!(lines.len(), 1, "{lines:?}");
    let expected = format!(
        " host=127.0.0.1 status=200 cause=ok send_state=known_sent request_bytes=0 \
         response_bytes={} duration_ms=",
        body.len()
    );
    assert!(lines[0].contains(&expected), "{}", lines[0]);
    for private in ["?", "/private", "sig", "sample"] {
        assert!(
            !lines[0].contains(private),
            "{private:?} leaked into {}",
            lines[0]
        );
    }
}

#[tokio::test]
async fn a_world_labs_download_writes_one_line_without_the_path_or_key() {
    df_observe::install_audit_sink(capture);
    let body: &'static [u8] = b"PLYDATA";
    let (address, server) = serve_media_once("application/octet-stream", body);
    let client = df_providers::NativeWorldLabs::new(
        df_providers::WorldLabsConfig::loopback_fixture(address).unwrap(),
        CREDENTIAL,
        Duration::from_secs(5),
    )
    .unwrap();
    let url = format!("http://{address}/private/world.ply");
    assert_eq!(client.download(&url, 1024).await.unwrap(), body);
    server.join().unwrap();
    let lines = download_lines("worldlabs");
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(
        lines[0].contains(
            " host=127.0.0.1 status=200 cause=ok send_state=known_sent request_bytes=0 \
             response_bytes=7 duration_ms="
        ),
        "{}",
        lines[0]
    );
    for private in ["/private", "world.ply", CREDENTIAL] {
        assert!(
            !lines[0].contains(private),
            "{private:?} leaked into {}",
            lines[0]
        );
    }
}

#[test]
fn every_cause_key_round_trips() {
    for cause in NativeHttpCause::ALL {
        assert_eq!(NativeHttpCause::from_key(cause.key()), Some(cause));
    }
    for state in df_providers::SendState::ALL {
        assert_eq!(df_providers::SendState::from_key(state.key()), Some(state));
    }
    for provider in df_providers::ProviderName::ALL {
        assert_eq!(
            df_providers::ProviderName::from_key(provider.key()),
            Some(provider)
        );
    }
}
