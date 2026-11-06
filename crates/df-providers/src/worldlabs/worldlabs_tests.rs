use super::*;
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Instant,
};

const KEY: &str = "fixture-secret-key-123";

#[derive(Clone)]
struct Reply {
    status: u16,
    content_type: &'static str,
    body: Vec<u8>,
    location: Option<&'static str>,
}

fn json_reply(status: u16, value: Value) -> Reply {
    Reply {
        status,
        content_type: "application/json",
        body: serde_json::to_vec(&value).unwrap(),
        location: None,
    }
}

struct Seen {
    method: String,
    path: String,
    key: Option<String>,
    body: Vec<u8>,
}

struct Fixture {
    address: SocketAddr,
    seen: Arc<Mutex<Vec<Seen>>>,
    stop: Arc<AtomicBool>,
    join: Option<thread::JoinHandle<()>>,
}

impl Fixture {
    fn new(replies: Vec<Reply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let record = seen.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let join = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(15);
            let mut index = 0usize;
            while !stopped.load(Ordering::Acquire) && Instant::now() < deadline {
                let (mut socket, _) = match listener.accept() {
                    Ok(pair) => pair,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(error) => panic!("fixture accept: {error}"),
                };
                socket.set_nonblocking(false).unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut data = Vec::new();
                let end = loop {
                    let mut block = [0; 4096];
                    let count = socket.read(&mut block).unwrap();
                    assert!(count > 0);
                    data.extend_from_slice(&block[..count]);
                    if let Some(end) = data.windows(4).position(|w| w == b"\r\n\r\n") {
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
                    let mut block = [0; 4096];
                    let count = socket.read(&mut block).unwrap();
                    assert!(count > 0);
                    data.extend_from_slice(&block[..count]);
                }
                let mut first = head.lines().next().unwrap().split_whitespace();
                let (method, path) = (
                    first.next().unwrap().to_owned(),
                    first.next().unwrap().to_owned(),
                );
                let key = head.lines().find_map(|line| {
                    let lower = line.to_ascii_lowercase();
                    lower
                        .strip_prefix("wlt-api-key:")
                        .map(|value| value.trim().to_owned())
                });
                record.lock().unwrap().push(Seen {
                    method,
                    path,
                    key,
                    body: data[end..].to_vec(),
                });
                let reply = replies[index.min(replies.len() - 1)].clone();
                index += 1;
                let location = reply
                    .location
                    .map(|value| format!("Location: {value}\r\n"))
                    .unwrap_or_default();
                let header = format!(
                    concat!(
                        "HTTP/1.1 {} fixture\r\nContent-Type: {}\r\nContent-Length: {}\r\n",
                        "Connection: close\r\n{}\r\n"
                    ),
                    reply.status,
                    reply.content_type,
                    reply.body.len(),
                    location
                );
                let _ = socket.write_all(header.as_bytes());
                let _ = socket.write_all(&reply.body);
            }
        });
        Self {
            address,
            seen,
            stop,
            join: Some(join),
        }
    }

    fn client(&self) -> NativeWorldLabs {
        NativeWorldLabs::new(
            WorldLabsConfig::loopback_fixture(self.address).unwrap(),
            KEY,
            Duration::from_secs(5),
        )
        .unwrap()
    }

    fn finish(&mut self) -> Vec<Seen> {
        self.stop.store(true, Ordering::Release);
        if let Some(join) = self.join.take() {
            join.join().unwrap();
        }
        std::mem::take(&mut *self.seen.lock().unwrap())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

fn request() -> WorldRequest<'static> {
    WorldRequest {
        display_name: "cloth shop",
        image_jpeg: b"\xff\xd8\xff\xe0jpeg-bytes",
        text_prompt: "An empty period cloth shop, photographic.",
        model: MarbleModel::Marble11,
    }
}

fn done_operation() -> Value {
    json!({
        "operation_id": "op-1",
        "done": true,
        "error": null,
        "response": {
            "world_id": "world-1",
            "assets": {
                "mesh": {"collider_mesh_url": "https://cdn.example/world-1/c.glb"},
                "splats": {"semantics_metadata": {"metric_scale_factor": 1.5014322, "ground_plane_offset": 1.2088797}},
                "thumbnail_url": "https://cdn.example/world-1/t.webp",
                "caption": "A cloth shop."
            }
        },
        "cost": {"total_credits": 1580, "line_items": []}
    })
}

#[test]
fn model_and_resolution_strings_are_the_api_names() {
    assert_eq!(MarbleModel::Draft.as_str(), "marble-1.0-draft");
    assert_eq!(MarbleModel::Marble11.as_str(), "marble-1.1");
    assert_eq!(PlyResolution::K100.as_str(), "100k");
    assert_eq!(PlyResolution::K500.as_str(), "500k");
    assert_eq!(PlyResolution::FullRes.as_str(), "full_res");
    assert_eq!(
        WorldLabsError::InsufficientCredits.label(),
        "insufficient_credits"
    );
    assert_eq!(WorldLabsError::Http(500).label(), "http_status");
}

#[test]
fn config_and_credential_validation() {
    assert!(WorldLabsConfig::loopback_fixture("8.8.8.8:443".parse().unwrap()).is_err());
    assert!(WorldLabsConfig::loopback_fixture("127.0.0.1:0".parse().unwrap()).is_err());
    assert!(NativeWorldLabs::new(WorldLabsConfig::live(), "", Duration::from_secs(5)).is_err());
    assert!(
        NativeWorldLabs::new(WorldLabsConfig::live(), "has space", Duration::from_secs(5)).is_err()
    );
    assert!(NativeWorldLabs::new(WorldLabsConfig::live(), KEY, Duration::ZERO).is_err());
}

#[test]
fn the_key_never_appears_in_debug_output() {
    let mut fixture = Fixture::new(vec![json_reply(
        200,
        json!({"operation_id": "op-1", "done": false}),
    )]);
    let client = fixture.client();
    let req = request();
    let debug = format!("{client:?} {req:?}");
    assert!(!debug.contains(KEY), "{debug}");
    fixture.finish();
}

#[test]
fn parse_poll_reads_proof_scale_fields_and_draft_null() {
    match parse_poll(&done_operation()).unwrap() {
        WorldPoll::Done(result) => {
            assert_eq!(result.metric_scale_factor, Some(1.5014322));
            assert_eq!(result.ground_plane_offset, Some(1.2088797));
            assert_eq!(result.credits, Some(1580));
            assert_eq!(
                result.collider_url.as_deref(),
                Some("https://cdn.example/world-1/c.glb")
            );
        }
        other => panic!("{other:?}"),
    }
    let draft = json!({
        "done": true, "error": null,
        "response": {"world_id": "w-draft", "assets": {"splats": {"semantics_metadata": null}, "caption": "x"}},
        "cost": {"total_credits": 230}
    });
    match parse_poll(&draft).unwrap() {
        WorldPoll::Done(result) => {
            assert_eq!(result.metric_scale_factor, None);
            assert_eq!(result.ground_plane_offset, None);
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn submit_sends_image_world_prompt_and_key_header() {
    let mut fixture = Fixture::new(vec![json_reply(
        200,
        json!({"operation_id": "op-abc", "done": false, "error": null}),
    )]);
    let id = fixture.client().submit(&request()).await.unwrap();
    assert_eq!(id.as_str(), "op-abc");
    let seen = fixture.finish();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "POST");
    assert_eq!(seen[0].path, "/marble/v1/worlds:generate");
    assert_eq!(seen[0].key.as_deref(), Some(KEY));
    let sent: Value = serde_json::from_slice(&seen[0].body).unwrap();
    assert_eq!(sent["model"], "marble-1.1");
    assert_eq!(sent["display_name"], "cloth shop");
    assert_eq!(sent["permission"]["public"], false);
    assert_eq!(sent["world_prompt"]["type"], "image");
    assert_eq!(sent["world_prompt"]["is_pano"], false);
    assert_eq!(
        sent["world_prompt"]["image_prompt"]["source"],
        "data_base64"
    );
    assert_eq!(sent["world_prompt"]["image_prompt"]["extension"], "jpg");
    assert!(
        sent["world_prompt"]["image_prompt"]["data_base64"]
            .as_str()
            .unwrap()
            .len()
            > 8
    );
}

#[tokio::test]
async fn poll_pending_then_done_parses_world() {
    let mut fixture = Fixture::new(vec![
        json_reply(
            200,
            json!({"operation_id": "op-1", "done": false, "error": null}),
        ),
        json_reply(200, done_operation()),
    ]);
    let client = fixture.client();
    let op = OperationId::new("op-1").unwrap();
    assert_eq!(client.poll(&op).await.unwrap(), WorldPoll::Pending);
    match client.poll(&op).await.unwrap() {
        WorldPoll::Done(result) => {
            assert_eq!(result.world_id, "world-1");
            assert_eq!(result.caption, "A cloth shop.");
            assert_eq!(
                result.thumbnail_url.as_deref(),
                Some("https://cdn.example/world-1/t.webp")
            );
            assert_eq!(result.metric_scale_factor, Some(1.5014322));
            assert_eq!(result.ground_plane_offset, Some(1.2088797));
            assert_eq!(result.credits, Some(1580));
        }
        other => panic!("{other:?}"),
    }
    let seen = fixture.finish();
    assert!(
        seen.iter()
            .all(|call| call.method == "GET" && call.path == "/marble/v1/operations/op-1")
    );
}

#[tokio::test]
async fn poll_failed_operation_returns_message() {
    let mut fixture = Fixture::new(vec![json_reply(
        200,
        json!({"operation_id": "op-1", "done": true, "error": {"code": 400, "message": "generation failed"}}),
    )]);
    let op = OperationId::new("op-1").unwrap();
    assert_eq!(
        fixture.client().poll(&op).await.unwrap(),
        WorldPoll::Failed("generation failed".to_owned())
    );
    fixture.finish();
}

#[tokio::test]
async fn export_ply_returns_signed_url_and_sends_ply_request() {
    let mut fixture = Fixture::new(vec![json_reply(
        200,
        json!({"operation_id": "exp-1", "done": true, "error": null, "response": {"asset_type": "splats", "format": "ply", "resolution": "500k", "url": "https://cdn.example/world-1/x.ply?sig=1"}}),
    )]);
    let url = fixture
        .client()
        .export_ply("world-1", PlyResolution::K500)
        .await
        .unwrap();
    assert_eq!(url, "https://cdn.example/world-1/x.ply?sig=1");
    let seen = fixture.finish();
    assert_eq!(seen[0].path, "/marble/v1/worlds/world-1:export");
    let sent: Value = serde_json::from_slice(&seen[0].body).unwrap();
    assert_eq!(
        sent,
        json!({"asset_type": "splats", "format": "ply", "resolution": "500k"})
    );
}

#[tokio::test]
async fn download_is_bounded_and_sends_no_key() {
    let mut fixture = Fixture::new(vec![
        Reply {
            status: 200,
            content_type: "application/octet-stream",
            body: b"PLYDATA".to_vec(),
            location: None,
        },
        Reply {
            status: 200,
            content_type: "application/octet-stream",
            body: b"PLYDATA".to_vec(),
            location: None,
        },
    ]);
    let client = fixture.client();
    let url = format!("http://{}/asset.ply", fixture.address);
    assert_eq!(client.download(&url, 1024).await.unwrap(), b"PLYDATA");
    assert_eq!(
        client.download(&url, 4).await.unwrap_err(),
        WorldLabsError::Transport
    );
    let seen = fixture.finish();
    assert!(
        seen.iter().all(|call| call.key.is_none()),
        "download must not send the key"
    );
}

#[test]
fn returned_download_urls_are_restricted_to_documented_https_hosts_and_default_port() {
    let client =
        NativeWorldLabs::new(WorldLabsConfig::live(), KEY, Duration::from_secs(5)).unwrap();
    for allowed in [
        "https://cdn.marble.worldlabs.ai/world/asset.ply?sig=secret",
        "https://wlt-ai-cdn.art/example_exports/world.ply",
        "https://wlt-ai-cdn.art:443/example_exports/world.ply",
    ] {
        assert_eq!(client.resolve_download_url(allowed).unwrap(), allowed);
    }
    for refused in [
        "http://wlt-ai-cdn.art/world.ply",
        "https://attacker.example/world.ply",
        "https://wlt-ai-cdn.art:8443/world.ply",
        "https://user@wlt-ai-cdn.art/world.ply",
        "https://127.0.0.1/world.ply",
        "https://[::1]/world.ply",
    ] {
        assert_eq!(
            client.resolve_download_url(refused).unwrap_err(),
            WorldLabsError::InvalidRequest,
            "{refused}"
        );
    }
}

#[test]
fn destination_filter_refuses_private_link_local_and_reserved_addresses() {
    for refused in [
        "0.0.0.0",
        "10.0.0.1",
        "100.64.0.1",
        "127.0.0.1",
        "169.254.169.254",
        "172.16.0.1",
        "192.168.0.1",
        "198.18.0.1",
        "203.0.113.1",
        "224.0.0.1",
        "::",
        "::1",
        "fc00::1",
        "fe80::1",
        "2001:db8::1",
    ] {
        let address = refused.parse().unwrap();
        assert!(!is_public(address), "{refused}");
    }
    for allowed in ["8.8.8.8", "2606:4700:4700::1111"] {
        let address = allowed.parse().unwrap();
        assert!(is_public(address), "{allowed}");
    }
}

/// The download pin used its own address filter, which admitted 6to4 (`2002::/16`, which embeds
/// an IPv4 address such as 10.0.0.1) and the `3fff::/20` documentation block. It now shares the
/// resolver filter of every other pin, which refuses both.
#[test]
fn download_pins_refuse_six_to_four_and_documentation_addresses() {
    for refused in ["2002:0a00:0001::1", "3fff::1"] {
        let address = refused.parse().unwrap();
        assert!(!is_public(address), "{refused}");
    }
}

#[test]
fn a_fractional_cost_is_rounded_up_and_an_impossible_one_records_nothing() {
    assert_eq!(spent_credits(&json!(1580)), Some(1580));
    assert_eq!(spent_credits(&json!(12.25)), Some(13));
    assert_eq!(spent_credits(&json!(-5.0)), None);
    assert_eq!(spent_credits(&json!(1e30)), None);
    assert_eq!(spent_credits(&json!("12")), None);
}

#[tokio::test]
async fn returned_download_redirect_is_refused() {
    let mut redirect = json_reply(302, json!({}));
    redirect.location = Some("https://attacker.example/x");
    let mut fixture = Fixture::new(vec![redirect]);
    let client = fixture.client();
    let url = format!("http://{}/asset.ply", fixture.address);
    assert_eq!(
        client.download(&url, 1024).await.unwrap_err(),
        WorldLabsError::Transport
    );
    assert_eq!(fixture.finish().len(), 1);
}

#[tokio::test]
async fn status_mapping_402_429_401() {
    for (status, expected) in [
        (402, WorldLabsError::InsufficientCredits),
        (429, WorldLabsError::RateLimited),
        (401, WorldLabsError::Unauthorized),
    ] {
        let mut fixture = Fixture::new(vec![json_reply(status, json!({"detail": "x"}))]);
        assert_eq!(
            fixture.client().submit(&request()).await.unwrap_err(),
            expected,
            "status {status}"
        );
        fixture.finish();
    }
    // 400 with a content-policy detail maps to ContentPolicy.
    let mut fixture = Fixture::new(vec![json_reply(
        400,
        json!({"detail": "Input violates content policy"}),
    )]);
    assert_eq!(
        fixture.client().submit(&request()).await.unwrap_err(),
        WorldLabsError::ContentPolicy
    );
    fixture.finish();
}

#[tokio::test]
async fn invalid_submit_sends_nothing() {
    let mut fixture = Fixture::new(vec![json_reply(200, json!({"operation_id": "op-1"}))]);
    let bad = WorldRequest {
        image_jpeg: b"",
        ..request()
    };
    assert_eq!(
        fixture.client().submit(&bad).await.unwrap_err(),
        WorldLabsError::InvalidRequest
    );
    assert!(fixture.finish().is_empty());
}
