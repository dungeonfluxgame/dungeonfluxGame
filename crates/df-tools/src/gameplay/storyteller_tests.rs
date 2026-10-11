use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

use serde_json::{Value, json};

use super::*;

const CREDENTIAL: &str = "credential-private-sentinel";

/// What the fake endpoint answers to one connection.
pub(in crate::gameplay) enum Reply {
    /// A complete response whose assistant text is this JSON.
    Output(Value),
    /// A complete response whose assistant text is exactly this string.
    OutputText(String),
    /// A bare HTTP status with a JSON body.
    Status(u16, Value),
    /// Accept the connection and never answer.
    Hang,
}

pub(in crate::gameplay) struct FakeEndpoint {
    pub(in crate::gameplay) address: SocketAddr,
    connections: Arc<AtomicUsize>,
    bodies: Arc<Mutex<Vec<Value>>>,
    heads: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl FakeEndpoint {
    pub(in crate::gameplay) fn start(script: Vec<Reply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let connections = Arc::new(AtomicUsize::new(0));
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let heads = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let (connections, bodies, heads, stop) = (
                connections.clone(),
                bodies.clone(),
                heads.clone(),
                stop.clone(),
            );
            thread::spawn(move || {
                let mut script = script.into_iter();
                while !stop.load(Ordering::SeqCst) {
                    let Ok((mut socket, _)) = listener.accept() else {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    };
                    connections.fetch_add(1, Ordering::SeqCst);
                    socket.set_nonblocking(false).unwrap();
                    socket
                        .set_read_timeout(Some(Duration::from_secs(3)))
                        .unwrap();
                    let (head, body) = read_request(&mut socket);
                    bodies.lock().unwrap().push(body.clone());
                    heads.lock().unwrap().push(head);
                    match script.next() {
                        Some(Reply::Output(slots)) => {
                            respond(&mut socket, 200, &completed(&body, slots.to_string()))
                        }
                        Some(Reply::OutputText(text)) => {
                            respond(&mut socket, 200, &completed(&body, text))
                        }
                        Some(Reply::Status(status, body)) => respond(&mut socket, status, &body),
                        Some(Reply::Hang) | None => {
                            while !stop.load(Ordering::SeqCst) {
                                thread::sleep(Duration::from_millis(10));
                            }
                        }
                    }
                }
            })
        };
        Self {
            address,
            connections,
            bodies,
            heads,
            stop,
            thread: Some(thread),
        }
    }

    pub(in crate::gameplay) fn connections(&self) -> usize {
        self.connections.load(Ordering::SeqCst)
    }

    pub(in crate::gameplay) fn bodies(&self) -> Vec<Value> {
        self.bodies.lock().unwrap().clone()
    }
}

impl Drop for FakeEndpoint {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}

fn read_request(socket: &mut std::net::TcpStream) -> (String, Value) {
    let mut bytes = Vec::new();
    let end = loop {
        let mut buffer = [0u8; 1024];
        let count = socket.read(&mut buffer).unwrap();
        assert!(count != 0 && bytes.len() + count <= 262_144);
        bytes.extend_from_slice(&buffer[..count]);
        if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            break end + 4;
        }
    };
    let head = String::from_utf8(bytes[..end].to_vec()).unwrap();
    let length: usize = head
        .lines()
        .find_map(|line| {
            line.split_once(':')
                .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                .map(|(_, value)| value.trim().parse().unwrap())
        })
        .unwrap();
    while bytes.len() - end < length {
        let mut buffer = [0u8; 1024];
        let count = socket.read(&mut buffer).unwrap();
        assert!(count != 0);
        bytes.extend_from_slice(&buffer[..count]);
    }
    (
        head,
        serde_json::from_slice(&bytes[end..end + length]).unwrap(),
    )
}

fn respond(socket: &mut std::net::TcpStream, status: u16, body: &Value) {
    let bytes = serde_json::to_vec(body).unwrap();
    let _ = write!(
        socket,
        "HTTP/1.1 {status} fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nx-request-id: http_fixture\r\nConnection: close\r\n\r\n",
        bytes.len()
    );
    let _ = socket.write_all(&bytes);
}

/// The supplier's complete response to `sent`, with 100 input and 40 output tokens.
fn completed(sent: &Value, assistant_text: String) -> Value {
    json!({"object":"response","id":"resp_fixture","status":"completed","model":sent["model"],
        "metadata":sent["metadata"],"text":sent["text"],"error":null,"incomplete_details":null,
        "output":[{"type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":assistant_text,"annotations":[]}]}],
        "usage":{"input_tokens":100,"output_tokens":40,"total_tokens":140,
            "input_tokens_details":{"cached_tokens":0},"output_tokens_details":{"reasoning_tokens":0}}})
}

pub(in crate::gameplay) fn cache_directory(name: &str) -> PathBuf {
    let directory =
        std::env::temp_dir().join(format!("df-storyteller-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    directory
}

pub(in crate::gameplay) fn teller(
    address: SocketAddr,
    budget: u64,
    deadline: Duration,
    cache: &Path,
) -> Storyteller {
    Storyteller::new(StorytellerSettings {
        credential: CREDENTIAL.to_owned(),
        model: "gpt-test-model".to_owned(),
        token_budget: budget,
        cache_directory: cache.to_owned(),
        deadline,
    })
    .with_loopback(address)
}

fn slots() -> Vec<Slot> {
    vec![Slot::new("reply", 80)]
}

fn facts() -> Value {
    json!({"npc": "Courier", "scene": "A rain-soaked wharf", "forbidden": ["Harbor Inn"]})
}

fn expression<'a>(facts: &'a Value, slots: &'a [Slot]) -> Expression<'a> {
    Expression {
        purpose: "npc_reply",
        facts,
        slots,
        style: "a short spoken line",
    }
}

fn reply(text: &str) -> Reply {
    Reply::Output(json!({ "reply": text }))
}

fn sent_input(body: &Value) -> String {
    body["input"].as_str().unwrap().to_owned()
}

const LONG_TEXT: &str =
    "This reply keeps going well past the eighty characters that the slot allows for it, sadly.";

#[tokio::test(flavor = "current_thread")]
async fn a_valid_answer_is_returned_cached_and_charged_at_the_reported_usage() {
    let endpoint = FakeEndpoint::start(vec![reply("I cannot say, friend. Keep your voice low.")]);
    let cache = cache_directory("success");
    let storyteller = teller(endpoint.address, 20_000, Duration::from_secs(5), &cache);
    let (facts, slots) = (facts(), slots());

    let values = storyteller
        .express(&expression(&facts, &slots))
        .await
        .unwrap();

    assert_eq!(
        values["reply"],
        "I cannot say, friend. Keep your voice low."
    );
    let body = &endpoint.bodies()[0];
    assert_eq!(body["model"], "gpt-test-model");
    assert_eq!(body["store"], false);
    assert_eq!(body["text"]["format"]["strict"], true);
    assert_eq!(
        body["text"]["format"]["schema"]["properties"]["reply"],
        json!({"type": "string"})
    );
    assert!(sent_input(body).contains("FACTS:"));
    assert!(
        endpoint.heads.lock().unwrap()[0]
            .to_ascii_lowercase()
            .contains(&format!("authorization: bearer {CREDENTIAL}"))
    );
    assert_eq!(storyteller.remaining_tokens(), Some(20_000 - 140));
    std::fs::remove_dir_all(cache).unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn output_missing_the_requested_field_is_retried_then_refused_as_the_wrong_shape() {
    let endpoint = FakeEndpoint::start(vec![
        Reply::Output(json!({"other": "x"})),
        Reply::OutputText("not json at all".to_owned()),
    ]);
    let cache = cache_directory("shape");
    let storyteller = teller(endpoint.address, 20_000, Duration::from_secs(5), &cache);
    let (facts, slots) = (facts(), slots());

    let error = storyteller
        .express(&expression(&facts, &slots))
        .await
        .unwrap_err();

    assert_eq!(
        error,
        StorytellerError::Rejected(ValidationFailure::NotTheRequestedShape)
    );
    assert_eq!(endpoint.connections(), 2);
    let _ = std::fs::remove_dir_all(cache);
}

#[tokio::test(flavor = "current_thread")]
async fn an_over_length_answer_is_rejected_whole_not_truncated() {
    let endpoint = FakeEndpoint::start(vec![reply(LONG_TEXT), reply(LONG_TEXT)]);
    let cache = cache_directory("long");
    let storyteller = teller(endpoint.address, 20_000, Duration::from_secs(5), &cache);
    let (facts, slots) = (facts(), slots());

    let error = storyteller
        .express(&expression(&facts, &slots))
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        StorytellerError::Rejected(ValidationFailure::TooLong { maximum: 80, .. })
    ));
    assert!(!cache.exists(), "a rejected text is never cached");
}

#[tokio::test(flavor = "current_thread")]
async fn a_missing_must_mention_term_is_rejected() {
    let endpoint = FakeEndpoint::start(vec![reply("Keep going."), reply("Please hurry.")]);
    let cache = cache_directory("mention");
    let storyteller = teller(endpoint.address, 20_000, Duration::from_secs(5), &cache);
    let facts = facts();
    let slots = vec![Slot {
        name: "reply".to_owned(),
        max_chars: 80,
        must_mention: vec!["wharf".to_owned()],
    }];

    let error = storyteller
        .express(&expression(&facts, &slots))
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        StorytellerError::Rejected(ValidationFailure::MissingMention { .. })
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn a_forbidden_term_is_rejected_even_in_a_different_case() {
    let endpoint = FakeEndpoint::start(vec![
        reply("It is bound for the harbor INN."),
        reply("Take it to the Harbor Inn, quickly."),
    ]);
    let cache = cache_directory("forbidden");
    let storyteller = teller(endpoint.address, 20_000, Duration::from_secs(5), &cache);
    let (facts, slots) = (facts(), slots());

    let error = storyteller
        .express(&expression(&facts, &slots))
        .await
        .unwrap_err();

    assert_eq!(
        error,
        StorytellerError::Rejected(ValidationFailure::ForbiddenTerm {
            slot: "reply".to_owned()
        })
    );
    assert!(!cache.exists());
}

#[tokio::test(flavor = "current_thread")]
async fn a_rejected_first_answer_is_retried_once_with_the_complaint_and_can_succeed() {
    let endpoint = FakeEndpoint::start(vec![reply(LONG_TEXT), reply("Short and safe.")]);
    let cache = cache_directory("retry");
    let storyteller = teller(endpoint.address, 20_000, Duration::from_secs(5), &cache);
    let (facts, slots) = (facts(), slots());

    let values = storyteller
        .express(&expression(&facts, &slots))
        .await
        .unwrap();

    assert_eq!(values["reply"], "Short and safe.");
    let bodies = endpoint.bodies();
    assert_eq!(bodies.len(), 2);
    assert!(!sent_input(&bodies[0]).contains("previous answer was rejected"));
    let retry = sent_input(&bodies[1]);
    assert!(retry.contains("previous answer was rejected"));
    assert!(retry.contains("at most 80"));
    assert!(
        !retry.contains(LONG_TEXT),
        "the complaint never repeats the text"
    );
    assert_eq!(storyteller.remaining_tokens(), Some(20_000 - 280));
    std::fs::remove_dir_all(cache).unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn two_rejected_answers_give_a_typed_error_after_exactly_two_calls() {
    let endpoint = FakeEndpoint::start(vec![reply(LONG_TEXT), reply(LONG_TEXT), reply("late")]);
    let cache = cache_directory("twice");
    let storyteller = teller(endpoint.address, 20_000, Duration::from_secs(5), &cache);
    let (facts, slots) = (facts(), slots());

    let error = storyteller
        .express(&expression(&facts, &slots))
        .await
        .unwrap_err();

    assert!(matches!(error, StorytellerError::Rejected(_)));
    assert_eq!(endpoint.connections(), 2);
}

#[tokio::test(flavor = "current_thread")]
async fn an_exhausted_budget_makes_no_call() {
    let endpoint = FakeEndpoint::start(vec![reply("never asked")]);
    let cache = cache_directory("budget");
    let storyteller = teller(endpoint.address, 50, Duration::from_secs(5), &cache);
    let (facts, slots) = (facts(), slots());

    let error = storyteller
        .express(&expression(&facts, &slots))
        .await
        .unwrap_err();

    assert_eq!(error, StorytellerError::BudgetExhausted);
    assert_eq!(endpoint.connections(), 0);
    assert_eq!(storyteller.remaining_tokens(), Some(50));
}

#[tokio::test(flavor = "current_thread")]
async fn a_repeated_identical_request_is_served_from_the_cache_without_a_call() {
    let endpoint = FakeEndpoint::start(vec![reply("Once only, friend.")]);
    let cache = cache_directory("cache");
    let storyteller = teller(endpoint.address, 20_000, Duration::from_secs(5), &cache);
    let (facts, slots) = (facts(), slots());
    let first = storyteller
        .express(&expression(&facts, &slots))
        .await
        .unwrap();
    let after_first = storyteller.remaining_tokens();

    let second = storyteller
        .express(&expression(&facts, &slots))
        .await
        .unwrap();
    // A resumed game builds a new storyteller over the same cache directory.
    let resumed = teller(endpoint.address, 20_000, Duration::from_secs(5), &cache);
    let third = resumed.express(&expression(&facts, &slots)).await.unwrap();

    assert_eq!(first, second);
    assert_eq!(first, third);
    assert_eq!(endpoint.connections(), 1);
    assert_eq!(storyteller.remaining_tokens(), after_first);
    assert_eq!(resumed.remaining_tokens(), Some(20_000));
    std::fs::remove_dir_all(cache).unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn a_cached_text_that_no_longer_validates_is_not_served() {
    let endpoint = FakeEndpoint::start(vec![reply("Fresh and safe.")]);
    let cache = cache_directory("stale");
    let storyteller = teller(endpoint.address, 20_000, Duration::from_secs(5), &cache);
    let (facts, slots) = (facts(), slots());
    let key = cache_key("gpt-test-model", &expression(&facts, &slots));
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::write(
        cache.join(format!("{}.json", hex(&key))),
        br#"{"reply":"Go to the Harbor Inn."}"#,
    )
    .unwrap();

    let values = storyteller
        .express(&expression(&facts, &slots))
        .await
        .unwrap();

    assert_eq!(values["reply"], "Fresh and safe.");
    assert_eq!(endpoint.connections(), 1);
    std::fs::remove_dir_all(cache).unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn a_silent_endpoint_hits_the_deadline_and_the_sent_call_stays_charged() {
    let endpoint = FakeEndpoint::start(vec![Reply::Hang]);
    let cache = cache_directory("deadline");
    let storyteller = teller(endpoint.address, 20_000, Duration::from_millis(400), &cache);
    let (facts, slots) = (facts(), slots());

    let error = storyteller
        .express(&expression(&facts, &slots))
        .await
        .unwrap_err();

    assert_eq!(error, StorytellerError::DeadlineExceeded);
    assert_eq!(endpoint.connections(), 1);
    assert!(
        storyteller.remaining_tokens().unwrap() < 20_000,
        "unknown outcome is not refunded"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn a_supplier_refusal_is_typed_and_refunded() {
    let endpoint = FakeEndpoint::start(vec![Reply::Status(
        401,
        json!({"error": {"code": "invalid_api_key"}}),
    )]);
    let cache = cache_directory("denied");
    let storyteller = teller(endpoint.address, 20_000, Duration::from_secs(5), &cache);
    let (facts, slots) = (facts(), slots());

    let error = storyteller
        .express(&expression(&facts, &slots))
        .await
        .unwrap_err();

    assert_eq!(
        error,
        StorytellerError::Provider(ProviderFailureClass::Denied)
    );
    assert_eq!(storyteller.remaining_tokens(), Some(20_000));
}

#[tokio::test(flavor = "current_thread")]
async fn an_unknown_supplier_failure_is_typed_and_not_refunded() {
    let endpoint = FakeEndpoint::start(vec![Reply::Status(500, json!({"error": {}}))]);
    let cache = cache_directory("unknown");
    let storyteller = teller(endpoint.address, 20_000, Duration::from_secs(5), &cache);
    let (facts, slots) = (facts(), slots());

    let error = storyteller
        .express(&expression(&facts, &slots))
        .await
        .unwrap_err();

    assert!(matches!(error, StorytellerError::Provider(_)));
    assert!(storyteller.remaining_tokens().unwrap() < 20_000);
}

#[tokio::test(flavor = "current_thread")]
async fn an_off_storyteller_refuses_without_any_call() {
    let (facts, slots) = (facts(), slots());

    let error = Storyteller::disabled()
        .express(&expression(&facts, &slots))
        .await
        .unwrap_err();

    assert_eq!(error, StorytellerError::Off);
}

#[tokio::test(flavor = "current_thread")]
async fn the_adventure_wrapper_uses_the_adventure_purpose() {
    let endpoint = FakeEndpoint::start(vec![Reply::Output(
        json!({"hook": "Rain hammers the wharf."}),
    )]);
    let cache = cache_directory("adventure");
    let storyteller = teller(endpoint.address, 20_000, Duration::from_secs(5), &cache);
    let facts = json!({"who": "a courier", "where": "the wharf"});
    let slots = vec![Slot::new("hook", 120)];

    let values = storyteller.express_adventure(&facts, &slots).await.unwrap();

    assert_eq!(values["hook"], "Rain hammers the wharf.");
    assert!(sent_input(&endpoint.bodies()[0]).contains("PURPOSE: adventure"));
    std::fs::remove_dir_all(cache).unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn malformed_expressions_are_refused_before_any_call() {
    let endpoint = FakeEndpoint::start(vec![]);
    let cache = cache_directory("invalid");
    let storyteller = teller(endpoint.address, 20_000, Duration::from_secs(5), &cache);
    let (facts, good) = (facts(), slots());
    let bad_forbidden = json!({"forbidden": "Vell"});
    let cases: Vec<(&str, &Value, Vec<Slot>)> = vec![
        ("npc_reply", &facts, vec![]),
        ("npc_reply", &facts, vec![Slot::new("Reply", 10)]),
        ("npc_reply", &facts, vec![Slot::new("reply", 0)]),
        (
            "npc_reply",
            &facts,
            vec![Slot::new("a", 5), Slot::new("a", 5)],
        ),
        ("Bad Purpose", &facts, good.clone()),
        ("npc_reply", &bad_forbidden, good.clone()),
    ];
    for (purpose, facts, slots) in &cases {
        let request = Expression {
            purpose,
            facts,
            slots,
            style: "",
        };
        assert!(matches!(
            storyteller.express(&request).await,
            Err(StorytellerError::InvalidRequest(_))
        ));
    }
    assert_eq!(endpoint.connections(), 0);
}

fn check(facts: &Value, text: &str) -> Result<(), ValidationFailure> {
    let slots = slots();
    let request = expression(facts, &slots);
    validate(
        &request,
        &BTreeMap::from([("reply".to_owned(), text.to_owned())]),
    )
}

#[test]
fn dice_difficulty_classes_and_hit_points_need_the_exact_text_in_the_facts() {
    let plain = json!({"npc": "Courier"});
    for text in [
        "Roll a d20 for me.",
        "That is a DC 15 job.",
        "You have 7 hit points left.",
        "Try your luck with d6.",
    ] {
        assert!(
            matches!(
                check(&plain, text),
                Err(ValidationFailure::MechanicsLeak { .. })
            ),
            "{text}"
        );
    }
    assert_eq!(check(&plain, "A steady hand wins the day."), Ok(()));
    assert_eq!(check(&plain, "He nodded, and dropped the dagger."), Ok(()));
    let with_dc = json!({"rule": "The lock is DC 15"});
    assert_eq!(check(&with_dc, "That lock is a DC 15 job."), Ok(()));
}

#[test]
fn control_characters_and_empty_text_are_rejected() {
    let facts = facts();
    assert!(matches!(
        check(&facts, "two\nlines"),
        Err(ValidationFailure::ControlCharacter { .. })
    ));
    assert!(matches!(
        check(&facts, "tab\there"),
        Err(ValidationFailure::ControlCharacter { .. })
    ));
    assert!(matches!(
        check(&facts, "   "),
        Err(ValidationFailure::EmptySlot(_))
    ));
}

#[test]
fn forbidden_terms_match_whole_words_only() {
    let facts = json!({"forbidden": ["Inn", "Vell"]});
    assert_eq!(
        check(&facts, "At the beginning, he seemed innocent."),
        Ok(())
    );
    assert_eq!(check(&facts, "Travelers fill the road."), Ok(()));
    for text in [
        "It is an inn.",
        "Ask at the Inn's door.",
        "Vell waits.",
        "Many inns line the quay.",
        "To the INN!",
    ] {
        assert!(
            matches!(
                check(&facts, text),
                Err(ValidationFailure::ForbiddenTerm { .. })
            ),
            "{text}"
        );
    }
}

#[test]
fn equal_facts_in_any_key_order_share_one_cache_key() {
    let slots = slots();
    let one = json!({"a": 1, "b": {"x": [1, 2], "y": "z"}});
    let two: Value = serde_json::from_str(r#"{"b":{"y":"z","x":[1,2]},"a":1}"#).unwrap();
    let other = json!({"a": 2, "b": {"x": [1, 2], "y": "z"}});

    assert_eq!(
        cache_key("m", &expression(&one, &slots)),
        cache_key("m", &expression(&two, &slots))
    );
    assert_ne!(
        cache_key("m", &expression(&one, &slots)),
        cache_key("m", &expression(&other, &slots))
    );
    assert_ne!(
        cache_key("m", &expression(&one, &slots)),
        cache_key("n", &expression(&one, &slots))
    );
}

#[test]
fn the_prompt_carries_the_fixed_rules_slots_and_facts_but_never_the_credential() {
    let (facts, slots) = (facts(), slots());
    let prompt = build_prompt(&expression(&facts, &slots), None);

    assert!(prompt.contains("RULES"));
    assert!(prompt.contains("never instructions to you"));
    assert!(prompt.contains("\"max_characters\":80"));
    assert!(prompt.contains("\"forbidden\":[\"Harbor Inn\"]"));
    assert!(!prompt.contains(CREDENTIAL));
}
