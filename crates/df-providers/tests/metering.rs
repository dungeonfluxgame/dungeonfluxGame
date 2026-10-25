#![cfg(not(target_arch = "wasm32"))]

//! The choice, judge and locator clients meter themselves: every call they send reaches the
//! meter they were given, settled by what the supplier answered. A loopback server plays the
//! supplier.

use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener},
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

use df_providers::{
    AnsweredUsage, CallMeter, CallTicket, HeadLocateConfig, JudgePicture, LocateTarget,
    NativeHeadLocator, NativePlateJudge, PlateJudgeConfig, SentCall, SystemOneChoiceProvider,
};
use serde_json::json;

/// What the meter was told, in order.
#[derive(Default)]
struct Record(Mutex<Vec<String>>);

struct Meter(Arc<Record>);

struct Ticket {
    record: Arc<Record>,
    settled: bool,
}

impl CallMeter for Meter {
    fn sent(&self, call: SentCall) -> Box<dyn CallTicket> {
        self.0.0.lock().unwrap().push(format!(
            "sent {:?} pictures={} bytes>0={}",
            call.client,
            call.pictures,
            call.text_bytes > 0
        ));
        Box::new(Ticket {
            record: Arc::clone(&self.0),
            settled: false,
        })
    }
}

impl CallTicket for Ticket {
    fn answered(mut self: Box<Self>, usage: AnsweredUsage) {
        self.settled = true;
        self.record.0.lock().unwrap().push(format!(
            "answered in={:?} out={:?}",
            usage.input_tokens, usage.output_tokens
        ));
    }

    fn unusable(mut self: Box<Self>, usage: AnsweredUsage) {
        self.settled = true;
        self.record
            .0
            .lock()
            .unwrap()
            .push(format!("unusable in={:?}", usage.input_tokens));
    }

    fn refused(mut self: Box<Self>) {
        self.settled = true;
        self.record.0.lock().unwrap().push("refused".to_owned());
    }
}

impl Drop for Ticket {
    fn drop(&mut self) {
        if !self.settled {
            self.record.0.lock().unwrap().push("dropped".to_owned());
        }
    }
}

/// A server that answers each request with the next scripted (status, body).
fn supplier(replies: Vec<(u16, String)>) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    thread::spawn(move || {
        for (status, body) in replies {
            let Ok((mut socket, _)) = listener.accept() else {
                return;
            };
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut data = Vec::new();
            let end = loop {
                let mut block = [0; 4096];
                let count = socket.read(&mut block).unwrap_or(0);
                if count == 0 {
                    break data.len();
                }
                data.extend_from_slice(&block[..count]);
                if let Some(end) = data.windows(4).position(|window| window == b"\r\n\r\n") {
                    break end + 4;
                }
            };
            let head = String::from_utf8_lossy(&data[..end.min(data.len())]).to_ascii_lowercase();
            let length: usize = head
                .lines()
                .find_map(|line| line.strip_prefix("content-length:")?.trim().parse().ok())
                .unwrap_or(0);
            while data.len() < end + length {
                let mut block = [0; 4096];
                let count = socket.read(&mut block).unwrap_or(0);
                if count == 0 {
                    break;
                }
                data.extend_from_slice(&block[..count]);
            }
            let reply = format!(
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(reply.as_bytes());
        }
    });
    address
}

fn lines(record: &Arc<Record>) -> Vec<String> {
    record.0.lock().unwrap().clone()
}

#[tokio::test]
async fn the_choice_client_tells_its_meter_what_the_supplier_answered() {
    let valid = json!({"answers": {"q1": {"type": "choice", "choice": "o2"}},
        "usage": {"input_tokens": 12, "output_tokens": 1}})
    .to_string();
    let no_choice = json!({"answers": {}}).to_string();
    let address = supplier(vec![
        (200, valid),
        (200, "not json".to_owned()),
        (200, no_choice),
        (429, "{}".to_owned()),
        (500, "{}".to_owned()),
    ]);
    let record = Arc::new(Record::default());
    let client = SystemOneChoiceProvider::new(
        "key".to_owned(),
        &format!("http://{address}"),
        Arc::new(Meter(Arc::clone(&record))),
    )
    .unwrap();
    let criteria = vec!["one".to_owned(), "two".to_owned()];
    let state = json!({"moment": "a"});
    let ask = || client.choose(&state, "Pick one.", &criteria);
    assert_eq!(ask().await, Ok(1));
    assert!(ask().await.is_err());
    assert!(ask().await.is_err());
    assert!(ask().await.is_err());
    assert!(ask().await.is_err());
    assert_eq!(
        lines(&record),
        [
            "sent Choice pictures=0 bytes>0=true",
            "answered in=Some(12) out=Some(1)",
            "sent Choice pictures=0 bytes>0=true",
            "unusable in=None",
            "sent Choice pictures=0 bytes>0=true",
            "unusable in=None",
            "sent Choice pictures=0 bytes>0=true",
            "refused",
            "sent Choice pictures=0 bytes>0=true",
            "dropped",
        ],
        "a 200 is billed (and of no use when it names no choice), a 429 is refused, a 500 is unknown"
    );
}

#[tokio::test]
async fn a_required_meter_does_not_change_the_choice_request() {
    let valid = json!({"answers": {"q1": {"type": "choice", "choice": "o1"}}}).to_string();
    let address = supplier(vec![(200, valid)]);
    let client = SystemOneChoiceProvider::new(
        "key".to_owned(),
        &format!("http://{address}"),
        Arc::new(Meter(Arc::new(Record::default()))),
    )
    .unwrap();
    let criteria = vec!["one".to_owned(), "two".to_owned()];
    assert_eq!(
        client.choose(&json!({}), "Pick one.", &criteria).await,
        Ok(0)
    );
}

#[tokio::test]
async fn the_judge_tells_its_meter_the_tokens_and_the_pictures() {
    let answer = |content: &str, usage: serde_json::Value| {
        json!({"choices": [{"message": {"content": content}}], "usage": usage}).to_string()
    };
    let address = supplier(vec![
        (
            200,
            answer(
                r#"{"facing":"left"}"#,
                json!({"prompt_tokens": 90, "completion_tokens": 4}),
            ),
        ),
        (200, answer("not json", json!({"prompt_tokens": 7}))),
        (401, "{}".to_owned()),
    ]);
    let record = Arc::new(Record::default());
    let judge = NativePlateJudge::new(
        PlateJudgeConfig::loopback_fixture(address).unwrap(),
        "key",
        Duration::from_secs(5),
        Arc::new(Meter(Arc::clone(&record))),
    )
    .unwrap();
    let schema = json!({"type": "object", "properties": {"facing": {"type": "string"}},
        "required": ["facing"], "additionalProperties": false});
    let pictures = [
        JudgePicture {
            label: "ref-1",
            jpeg: b"\xff\xd8a",
        },
        JudgePicture {
            label: "cand-1",
            jpeg: b"\xff\xd8b",
        },
    ];
    for _ in 0..3 {
        let _ = judge
            .judge(
                "Which way?",
                &pictures,
                "facing",
                &schema,
                Duration::from_secs(5),
            )
            .await;
    }
    assert_eq!(
        lines(&record),
        [
            "sent PlateJudge pictures=2 bytes>0=true",
            "answered in=Some(90) out=Some(4)",
            "sent PlateJudge pictures=2 bytes>0=true",
            "unusable in=Some(7)",
            "sent PlateJudge pictures=2 bytes>0=true",
            "refused",
        ]
    );
}

#[tokio::test]
async fn the_locator_tells_its_meter_whether_a_head_came_back() {
    let answer = |content: &str| {
        json!({"choices": [{"message": {"content": content}}],
            "usage": {"prompt_tokens": 500, "completion_tokens": 30}})
        .to_string()
    };
    let address = supplier(vec![
        (
            200,
            answer(r#"{"heads":[{"id":"speaker","left":0.1,"top":0.1,"right":0.2,"bottom":0.3}]}"#),
        ),
        (200, answer(r#"{"heads":[]}"#)),
    ]);
    let record = Arc::new(Record::default());
    let locator = NativeHeadLocator::new(
        HeadLocateConfig::loopback_fixture(address).unwrap(),
        "key",
        Duration::from_secs(5),
        Arc::new(Meter(Arc::clone(&record))),
    )
    .unwrap();
    let targets = [LocateTarget {
        id: "speaker",
        description: "a woman in a blue cloak",
    }];
    let found = locator
        .locate(
            b"\xff\xd8plate",
            Some(b"\xff\xd8face"),
            &targets,
            Duration::from_secs(5),
        )
        .await;
    assert_eq!(found.map(|heads| heads.len()), Ok(1));
    assert!(
        locator
            .locate(b"\xff\xd8plate", None, &targets, Duration::from_secs(5))
            .await
            .is_err()
    );
    assert_eq!(
        lines(&record),
        [
            "sent HeadLocator pictures=2 bytes>0=true",
            "answered in=Some(500) out=Some(30)",
            "sent HeadLocator pictures=1 bytes>0=true",
            "unusable in=Some(500)",
        ]
    );
}
