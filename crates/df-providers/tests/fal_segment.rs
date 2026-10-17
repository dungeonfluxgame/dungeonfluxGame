#![cfg(not(target_arch = "wasm32"))]
//! The fal SAM adapter against a faithful loopback fake of fal's queue: one submit, status
//! reads, one result read, with fal's real reply shapes (measured 2026-10-17, see
//! `planning/sam-3-1-research.md`).

use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use df_providers::{
    FalSegmentConfig, FalSegmentError, NativeFalSegmenter, NativeHttpRefusal, SegmentModel,
    SegmentRequest,
};
use serde_json::{Value, json};

const KEY: &str = "fixture-private-key";
const ID: &str = "0f1e2d3c-sam";

#[derive(Clone)]
struct Reply {
    status: u16,
    body: Value,
}

struct Seen {
    method: String,
    path: String,
    authorization: Option<String>,
    body: Vec<u8>,
}

/// Answers `POST /fal-ai/sam-3-1/image-rle`, then each `GET …/status` with the next of
/// `statuses` (the last one repeats), then `GET …/requests/<id>` with `result`.
struct FalQueue {
    address: SocketAddr,
    seen: Arc<Mutex<Vec<Seen>>>,
    stop: Arc<AtomicBool>,
    join: Option<thread::JoinHandle<()>>,
}

impl FalQueue {
    fn new(submit: Reply, statuses: Vec<Reply>, result: Reply) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let record = seen.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let join = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut reads = 0usize;
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
                let authorization = head.lines().find_map(|line| {
                    line.strip_prefix("authorization: ")
                        .or_else(|| line.strip_prefix("Authorization: "))
                        .map(str::to_owned)
                });
                record.lock().unwrap().push(Seen {
                    method: method.clone(),
                    path: path.clone(),
                    authorization,
                    body: data[end..].to_vec(),
                });
                let reply = if method == "POST" {
                    submit.clone()
                } else if path.ends_with("/status") {
                    let reply = statuses[reads.min(statuses.len() - 1)].clone();
                    reads += 1;
                    reply
                } else {
                    result.clone()
                };
                let body = serde_json::to_vec(&reply.body).unwrap();
                let header = format!(
                    "HTTP/1.1 {} fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    reply.status,
                    body.len()
                );
                let _ = socket.write_all(header.as_bytes());
                let _ = socket.write_all(&body);
            }
        });
        Self {
            address,
            seen,
            stop,
            join: Some(join),
        }
    }

    fn finish(&mut self) -> Vec<Seen> {
        self.stop.store(true, Ordering::Release);
        if let Some(join) = self.join.take() {
            join.join().unwrap();
        }
        std::mem::take(&mut *self.seen.lock().unwrap())
    }
}

impl Drop for FalQueue {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

fn ok(body: Value) -> Reply {
    Reply { status: 200, body }
}

fn accepted() -> Reply {
    ok(json!({
        "request_id": ID,
        "status_url": format!("https://queue.fal.run/fal-ai/sam-3-1/requests/{ID}/status"),
        "response_url": format!("https://queue.fal.run/fal-ai/sam-3-1/requests/{ID}"),
        "cancel_url": format!("https://queue.fal.run/fal-ai/sam-3-1/requests/{ID}/cancel"),
    }))
}

fn status(name: &str) -> Reply {
    ok(json!({"status": name, "request_id": ID}))
}

/// Two people in a 40 by 20 picture, in score order, as fal returns them (1-based starts).
fn two_people() -> Reply {
    ok(json!({
        "rle": ["206 6 246 6 286 6", "31 4 51 4"],
        "scores": [0.96, 0.87],
        "boxes": [[0.2, 0.4, 0.15, 0.3], [0.8, 0.5, 0.1, 0.2]],
        "metadata": [
            {"index": 1, "score": 0.96, "box": [0.2, 0.4, 0.15, 0.3]},
            {"index": 0, "score": 0.87, "box": [0.8, 0.5, 0.1, 0.2]}
        ],
        "boundingbox_frames_zip": null,
    }))
}

fn segmenter(queue: &FalQueue, model: SegmentModel) -> NativeFalSegmenter {
    NativeFalSegmenter::new(
        FalSegmentConfig::loopback_fixture(model, queue.address, Duration::from_millis(5)).unwrap(),
        KEY,
        Duration::from_secs(3),
    )
    .unwrap()
}

fn request(prompt: &str) -> SegmentRequest<'_> {
    SegmentRequest {
        jpeg: b"\xff\xd8not-really-a-jpeg",
        width: 40,
        height: 20,
        prompt,
        max_masks: 8,
    }
}

#[tokio::test]
async fn a_call_submits_once_polls_to_completion_and_reads_the_masks() {
    let mut queue = FalQueue::new(
        accepted(),
        vec![
            status("IN_QUEUE"),
            status("IN_PROGRESS"),
            status("COMPLETED"),
        ],
        two_people(),
    );
    let answer = segmenter(&queue, SegmentModel::Sam31)
        .segment(&request("person"), Duration::from_secs(5))
        .await
        .unwrap();

    assert_eq!(answer.request_id, ID);
    assert_eq!(answer.polls, 3);
    assert_eq!(answer.model, SegmentModel::Sam31);
    assert_eq!(answer.instances.len(), 2);
    assert_eq!(
        (answer.instances[0].index, answer.instances[0].score),
        (1, 0.96)
    );
    assert_eq!(answer.instances[0].mask.pixel_count(), 18);
    assert_eq!(answer.instances[0].mask.bounds().unwrap().left, 5);

    let seen = queue.finish();
    let posts: Vec<_> = seen.iter().filter(|call| call.method == "POST").collect();
    assert_eq!(posts.len(), 1, "a call is submitted exactly once");
    assert_eq!(posts[0].path, "/fal-ai/sam-3-1/image-rle");
    assert_eq!(
        posts[0].authorization.as_deref(),
        Some("Key fixture-private-key")
    );
    let sent: Value = serde_json::from_slice(&posts[0].body).unwrap();
    assert_eq!(sent["prompt"], "person");
    assert_eq!(sent["max_masks"], 8);
    assert!(
        sent["image_url"]
            .as_str()
            .unwrap()
            .starts_with("data:image/jpeg;base64,")
    );
    let reads: Vec<_> = seen.iter().filter(|call| call.method == "GET").collect();
    assert_eq!(reads.len(), 4, "three status reads and one result read");
    assert_eq!(
        reads[3].path,
        format!("/fal-ai/sam-3-1/requests/{ID}"),
        "the result is read from the queue URL fal named"
    );
}

#[tokio::test]
async fn the_sam_3_model_goes_to_its_own_endpoint() {
    let mut queue = FalQueue::new(
        accepted(),
        vec![status("COMPLETED")],
        ok(json!({"rle": [], "scores": []})),
    );
    let answer = segmenter(&queue, SegmentModel::Sam3)
        .segment(&request("head"), Duration::from_secs(5))
        .await
        .unwrap();
    assert!(
        answer.instances.is_empty(),
        "no match is a successful answer"
    );
    let seen = queue.finish();
    assert_eq!(seen[0].path, "/fal-ai/sam-3/image-rle");
}

#[tokio::test]
async fn a_refused_submit_is_a_provider_failure_and_nothing_else_is_asked() {
    let mut queue = FalQueue::new(
        Reply {
            status: 422,
            body: json!({"detail": "refused"}),
        },
        vec![status("COMPLETED")],
        two_people(),
    );
    let failure = segmenter(&queue, SegmentModel::Sam31)
        .segment(&request("person"), Duration::from_secs(5))
        .await
        .unwrap_err();
    assert_eq!(failure, FalSegmentError::Provider(422));
    assert!(failure.was_sent());
    assert!(!failure.is_transient());
    assert_eq!(queue.finish().len(), 1);
}

#[tokio::test]
async fn a_bad_key_is_unauthorized() {
    let mut queue = FalQueue::new(
        Reply {
            status: 401,
            body: json!({"detail": "bad key"}),
        },
        vec![status("COMPLETED")],
        two_people(),
    );
    let failure = segmenter(&queue, SegmentModel::Sam31)
        .segment(&request("person"), Duration::from_secs(5))
        .await
        .unwrap_err();
    assert_eq!(failure, FalSegmentError::Unauthorized);
    queue.finish();
}

#[tokio::test]
async fn a_request_fal_reports_failed_ends_the_call() {
    let mut queue = FalQueue::new(
        accepted(),
        vec![status("IN_PROGRESS"), status("FAILED")],
        two_people(),
    );
    let failure = segmenter(&queue, SegmentModel::Sam31)
        .segment(&request("person"), Duration::from_secs(5))
        .await
        .unwrap_err();
    assert_eq!(failure, FalSegmentError::Failed);
    assert!(failure.was_sent());
    queue.finish();
}

#[tokio::test]
async fn queue_urls_that_name_another_host_or_request_are_refused_before_any_poll() {
    for hostile in [
        format!("https://evil.example/fal-ai/sam-3-1/requests/{ID}/status"),
        "https://queue.fal.run/fal-ai/sam-3-1/requests/another/status".to_owned(),
    ] {
        let mut queue = FalQueue::new(
            ok(json!({
                "request_id": ID,
                "status_url": hostile,
                "response_url": format!("https://queue.fal.run/fal-ai/sam-3-1/requests/{ID}"),
            })),
            vec![status("COMPLETED")],
            two_people(),
        );
        let failure = segmenter(&queue, SegmentModel::Sam31)
            .segment(&request("person"), Duration::from_secs(5))
            .await
            .unwrap_err();
        assert_eq!(failure, FalSegmentError::Queue);
        assert_eq!(queue.finish().len(), 1, "only the submit was sent");
    }
}

#[tokio::test]
async fn a_request_that_never_completes_ends_at_the_deadline() {
    let mut queue = FalQueue::new(accepted(), vec![status("IN_PROGRESS")], two_people());
    let failure = segmenter(&queue, SegmentModel::Sam31)
        .segment(&request("person"), Duration::from_millis(150))
        .await
        .unwrap_err();
    assert_eq!(
        failure,
        FalSegmentError::Transport(NativeHttpRefusal::Deadline)
    );
    assert!(failure.is_transient());
    queue.finish();
}

#[tokio::test]
async fn a_mask_that_does_not_fit_the_picture_is_a_bad_output() {
    let mut queue = FalQueue::new(
        accepted(),
        vec![status("COMPLETED")],
        ok(json!({"rle": ["790 20"], "scores": [0.9]})),
    );
    let failure = segmenter(&queue, SegmentModel::Sam31)
        .segment(&request("person"), Duration::from_secs(5))
        .await
        .unwrap_err();
    assert_eq!(failure, FalSegmentError::Output);
    queue.finish();
}

#[tokio::test]
async fn invalid_input_is_refused_without_contacting_fal() {
    let mut queue = FalQueue::new(accepted(), vec![status("COMPLETED")], two_people());
    let failure = segmenter(&queue, SegmentModel::Sam31)
        .segment(&request(""), Duration::from_secs(5))
        .await
        .unwrap_err();
    assert_eq!(failure, FalSegmentError::InvalidRequest);
    assert!(!failure.was_sent());
    assert!(queue.finish().is_empty());
}
