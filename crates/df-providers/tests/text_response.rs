#![cfg(not(target_arch = "wasm32"))]

use std::{
    future,
    io::{Read, Write},
    net::{SocketAddr, TcpListener},
    thread,
    time::Duration,
};

use df_model::checkpoint::{Basis, ContentDigest, ExecutionMode, JobId};
use df_provider_api::{
    RequestBasis, RequestBinding, RequestIdentity, RequestLimits, RequestModality,
    RequestOwnerState, RequestUsage, TextEventKind, TextRequest, TextSemantics,
};
use df_providers::{
    CandidateRouteId, NativeHttpLimits, NativeHttpRefusal, NativeTextProvider,
    ProviderFailureClass, TextOutputSchema, TextProviderConfig, TextResponseContract,
    TextResponseError, TextSchemaError, TextSchemaLimits, registered_text_provider,
};
use df_types::{
    LocaleTag, OperationId, RecoveryEpoch, RevisionLabel, RunId, SessionId, SessionRevision, Usage,
    UsageUnit,
};
use serde_json::{Value, json};

type Binding = RequestBinding<RequestBasis<TextSemantics<String, TextOutputSchema>>>;

fn schema() -> TextOutputSchema {
    TextOutputSchema::new("dialogue", br#"{"type":"object","properties":{"line":{"type":"string"}},"required":["line"],"additionalProperties":false}"#,
        TextSchemaLimits { maximum_schema_bytes: 4096, maximum_depth: 32, maximum_nodes: 2048 }).unwrap()
}

fn limits() -> NativeHttpLimits {
    NativeHttpLimits {
        connect_timeout: Duration::from_secs(2),
        read_timeout: Duration::from_secs(2),
        total_timeout: Duration::from_secs(3),
        maximum_request_bytes: 16_384,
        maximum_response_bytes: 16_384,
        maximum_header_bytes: 4096,
        maximum_chunks: 1024,
    }
}

fn provider(address: SocketAddr) -> NativeTextProvider {
    registered_text_provider(
        CandidateRouteId::OpenAiResponsesText,
        TextProviderConfig::new("caller-model-snapshot", schema(), 50).unwrap(),
        "credential-private-sentinel",
        limits(),
    )
    .unwrap()
    .for_loopback_fixture(address)
    .unwrap()
}

fn binding(mode: ExecutionMode) -> Binding {
    RequestBinding {
        identity: RequestIdentity {
            basis: Basis {
                session: SessionId::from_bytes(&[1; 16]).unwrap(),
                run: RunId::from_bytes(&[2; 16]).unwrap(),
                revision: SessionRevision::new(RecoveryEpoch::new(1).unwrap(), 2),
            },
            job: JobId::from_bytes(&[3; 16]).unwrap(),
            operation: OperationId::from_bytes(&[4; 16]).unwrap(),
            generation: 7,
        },
        semantic_basis: RequestBasis {
            modality: RequestModality::Text,
            source: ContentDigest([5; 32]),
            rights_revision: RevisionLabel::new(Some("rights")).unwrap(),
            output_format_revision: RevisionLabel::new(Some("format")).unwrap(),
            semantic: TextSemantics {
                locale: LocaleTag::parse("en-US").unwrap(),
                context: "private-context".to_owned(),
                output_schema: schema(),
            },
        },
        mode,
        deadline: Duration::from_secs(5),
    }
}

fn owner(
    current: &Binding,
) -> RequestOwnerState<'_, RequestBasis<TextSemantics<String, TextOutputSchema>>> {
    RequestOwnerState {
        current: Some(current),
        elapsed: Duration::ZERO,
        cancelled: false,
    }
}

fn request(current: &Binding) -> TextRequest<String, TextOutputSchema> {
    TextRequest::new(
        binding(current.mode),
        b"prompt-private-sentinel",
        RequestUsage::new(1, Duration::ZERO, Usage::new(1, UsageUnit::Token)),
        RequestLimits::new(
            4096,
            100,
            Duration::from_secs(5),
            Usage::new(100, UsageUnit::Token),
        )
        .unwrap(),
        owner(current),
    )
    .unwrap()
}

fn completed(sent: &Value) -> Value {
    json!({"object":"response","id":"resp_fixture","status":"completed","model":sent["model"],"metadata":sent["metadata"],"text":sent["text"],"error":null,"incomplete_details":null,
        "output":[{"type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":"{\"line\":\"output-private-sentinel\"}","annotations":[]}]}],
        "usage":{"input_tokens":12,"output_tokens":8,"total_tokens":20,"input_tokens_details":{"cached_tokens":3,"cache_write_tokens":2},"output_tokens_details":{"reasoning_tokens":1}}})
}

struct Captured {
    head: String,
    body: Value,
    extra_connection: bool,
}

fn fixture(
    transform: impl FnOnce(&Value) -> (u16, Vec<u8>) + Send + 'static,
) -> (SocketAddr, thread::JoinHandle<Captured>) {
    fixture_wire(move |sent, socket| {
        let (status, reply) = transform(sent);
        write!(socket,"HTTP/1.1 {status} fixture\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nx-request-id: http_fixture\r\nConnection: close\r\n\r\n").unwrap();
        for chunk in reply.chunks(17) {
            if write!(socket, "{:x}\r\n", chunk.len())
                .and_then(|()| socket.write_all(chunk))
                .and_then(|()| socket.write_all(b"\r\n"))
                .is_err()
            {
                break;
            }
        }
        let _ = socket.write_all(b"0\r\n\r\n");
    })
}

fn fixture_wire(
    transform: impl FnOnce(&Value, &mut std::net::TcpStream) + Send + 'static,
) -> (SocketAddr, thread::JoinHandle<Captured>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let task = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut bytes = Vec::new();
        let end = loop {
            let mut buffer = [0u8; 1024];
            let count = socket.read(&mut buffer).unwrap();
            assert!(count != 0 && bytes.len() + count <= 32_768);
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
            assert!(count != 0 && bytes.len() + count <= 32_768);
            bytes.extend_from_slice(&buffer[..count]);
        }
        let body: Value = serde_json::from_slice(&bytes[end..end + length]).unwrap();
        transform(&body, &mut socket);
        drop(socket);
        listener.set_nonblocking(true).unwrap();
        let extra_connection = listener.accept().is_ok();
        Captured {
            head,
            body,
            extra_connection,
        }
    });
    (address, task)
}

#[tokio::test(flavor = "current_thread")]
async fn registered_factory_checks_actual_chunked_eof_schema_usage_and_current_owner_before_events()
{
    let (address, server) = fixture(|sent| (200, serde_json::to_vec(&completed(sent)).unwrap()));
    let current = binding(ExecutionMode::Live);
    let request = request(&current);
    let provider = provider(address);
    let response = provider
        .submit(&request, owner(&current), future::pending())
        .await
        .unwrap();
    assert_eq!(response.provider_request_id().as_str(), "resp_fixture");
    assert_eq!(response.transport_request_id(), Some("http_fixture"));
    assert_eq!(response.supplier_usage().total_tokens, 20);
    assert_eq!(response.supplier_usage().cached_input_tokens, 3);
    assert_eq!(response.supplier_usage().cache_write_tokens, Some(2));
    assert_eq!(response.supplier_usage().reasoning_output_tokens, 1);
    let observation = response.observation();
    assert_eq!(observation.identity(), current.identity);
    let events = response.complete(owner(&current)).unwrap();
    assert_eq!(events[0].usage, Usage::new(20, UsageUnit::Token));
    match &events[0].kind {
        TextEventKind::Candidate(output) => {
            assert_eq!(output.json(), &json!({"line":"output-private-sentinel"}))
        }
        _ => panic!("candidate first"),
    }
    assert!(
        matches!(events[1].kind,TextEventKind::Completed { identity } if identity == current.identity)
    );
    let captured = tokio::task::spawn_blocking(move || server.join().unwrap())
        .await
        .unwrap();
    assert!(captured.head.starts_with("POST /v1/responses HTTP/1.1\r\n"));
    assert!(
        captured
            .head
            .to_ascii_lowercase()
            .contains("authorization: bearer credential-private-sentinel")
    );
    assert_eq!(captured.body["input"], "prompt-private-sentinel");
    assert_eq!(captured.body["model"], "caller-model-snapshot");
    assert_eq!(captured.body["stream"], false);
    assert_eq!(captured.body["store"], false);
    assert_eq!(captured.body["text"]["format"]["strict"], true);
    assert_eq!(captured.body["text"]["format"]["type"], "json_schema");
    assert_eq!(captured.body["max_output_tokens"], 50);
    assert!(!captured.extra_connection);
    for value in [format!("{provider:?}"), format!("{observation:?}")] {
        for secret in [
            "credential-private-sentinel",
            "prompt-private-sentinel",
            "output-private-sentinel",
        ] {
            assert!(!value.contains(secret));
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn refusal_incomplete_tools_foreign_metadata_model_schema_and_invalid_output_never_complete()
{
    for case in 0..9 {
        let (address, server) = fixture(move |sent| {
            let mut reply = completed(sent);
            match case {
                0 => {
                    reply["output"][0]["content"][0] =
                        json!({"type":"refusal","refusal":"private-refusal"})
                }
                1 => reply["status"] = json!("incomplete"),
                2 => reply["error"] = json!({"code":"server_error","message":"private-error"}),
                3 => reply["output"][0]["type"] = json!("function_call"),
                4 => reply["metadata"]["df_operation"] = json!("foreign"),
                5 => reply["model"] = json!("foreign-model"),
                6 => reply["text"]["format"]["name"] = json!("foreign-schema"),
                7 => reply["output"][0]["content"][0]["text"] = json!("{\"line\":7}"),
                8 => reply["id"] = json!("foreign-id"),
                _ => unreachable!(),
            }
            (200, serde_json::to_vec(&reply).unwrap())
        });
        let current = binding(ExecutionMode::Live);
        let request = request(&current);
        let error = provider(address)
            .submit(&request, owner(&current), future::pending())
            .await
            .unwrap_err();
        assert_eq!(error.failure_class(), ProviderFailureClass::Contract);
        assert!(
            matches!(error.reason(),TextResponseError::Contract { usage:Some(usage),.. } if usage.total_tokens == 20)
        );
        assert!(!format!("{error:?}").contains("private-error"));
        assert!(
            !tokio::task::spawn_blocking(move || server.join().unwrap())
                .await
                .unwrap()
                .extra_connection
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn live_api_echo_of_a_null_format_description_is_accepted_but_other_extra_keys_are_not() {
    for (extra, accepted) in [("description", true), ("verbosity", false)] {
        let (address, server) = fixture(move |sent| {
            let mut reply = completed(sent);
            reply["text"]["format"][extra] = Value::Null;
            (200, serde_json::to_vec(&reply).unwrap())
        });
        let current = binding(ExecutionMode::Live);
        let request = request(&current);
        let outcome = provider(address)
            .submit(&request, owner(&current), future::pending())
            .await;
        assert_eq!(outcome.is_ok(), accepted, "{extra}");
        tokio::task::spawn_blocking(move || server.join().unwrap())
            .await
            .unwrap();
    }
}

#[tokio::test(flavor = "current_thread")]
async fn all_invalid_usage_forms_reject_and_valid_overrun_retains_exact_actual_counters() {
    for case in 0..10 {
        let (address, server) = fixture(move |sent| {
            let mut reply = completed(sent);
            match case {
                0 => {
                    reply.as_object_mut().unwrap().remove("usage");
                }
                1 => reply["usage"] = Value::Null,
                2 => reply["usage"]["input_tokens"] = json!(-1),
                3 => reply["usage"]["input_tokens"] = json!(12.5),
                4 => reply["usage"]["total_tokens"] = json!(21),
                5 => reply["usage"]["input_tokens_details"]["cached_tokens"] = json!(13),
                6 => reply["usage"]["output_tokens_details"]["reasoning_tokens"] = json!(9),
                7 => reply["usage"]["extra_billable_tokens"] = json!(1),
                8 => {
                    reply["usage"]["input_tokens"] =
                        json!("340282366920938463463374607431768211456")
                }
                9 => {
                    reply["usage"]["input_tokens"] = json!(200);
                    reply["usage"]["total_tokens"] = json!(208);
                }
                _ => unreachable!(),
            }
            (200, serde_json::to_vec(&reply).unwrap())
        });
        let current = binding(ExecutionMode::Live);
        let request = request(&current);
        let error = provider(address)
            .submit(&request, owner(&current), future::pending())
            .await
            .unwrap_err();
        if case == 9 {
            assert!(
                matches!(error.reason(),TextResponseError::UsageExceeded(usage) if usage.total_tokens == 208 && usage.input_tokens == 200)
            );
        } else {
            assert!(matches!(
                error.reason(),
                TextResponseError::Contract {
                    reason: TextResponseContract::Usage,
                    usage: None
                }
            ));
        }
        assert!(
            !tokio::task::spawn_blocking(move || server.join().unwrap())
                .await
                .unwrap()
                .extra_connection
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn malformed_trailing_duplicate_keys_and_schema_mismatch_do_not_publish_raw_output() {
    for case in 0..5 {
        let (address, server) = fixture(move |sent| {
            let mut reply = completed(sent);
            let bytes = match case {
                0 => b"{".to_vec(),
                1 => {
                    let mut bytes = serde_json::to_vec(&reply).unwrap();
                    bytes.extend_from_slice(b"{} ");
                    bytes
                }
                2 => br#"{"status":"completed","status":"failed"}"#.to_vec(),
                3 => {
                    reply["output"][0]["content"][0]["text"] =
                        json!("{\"line\":\"one\",\"line\":\"two\"}");
                    serde_json::to_vec(&reply).unwrap()
                }
                4 => {
                    reply["output"][0]["content"][0]["text"] =
                        json!("{\"line\":\"one\",\"secret\":\"two\"}");
                    serde_json::to_vec(&reply).unwrap()
                }
                _ => unreachable!(),
            };
            (200, bytes)
        });
        let current = binding(ExecutionMode::Live);
        let request = request(&current);
        assert!(matches!(
            provider(address)
                .submit(&request, owner(&current), future::pending())
                .await
                .unwrap_err()
                .reason(),
            TextResponseError::Contract { .. }
        ));
        assert!(
            !tokio::task::spawn_blocking(move || server.join().unwrap())
                .await
                .unwrap()
                .extra_connection
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn final_consumption_rechecks_generation_source_rights_schema_mode_basis_and_deadline() {
    for case in 0..8 {
        let (address, server) =
            fixture(|sent| (200, serde_json::to_vec(&completed(sent)).unwrap()));
        let current = binding(ExecutionMode::Live);
        let request = request(&current);
        let response = provider(address)
            .submit(&request, owner(&current), future::pending())
            .await
            .unwrap();
        let mut replacement = binding(ExecutionMode::Live);
        match case {
            0 => replacement.identity.generation += 1,
            1 => replacement.semantic_basis.source = ContentDigest([7; 32]),
            2 => {
                replacement.semantic_basis.rights_revision =
                    RevisionLabel::new(Some("changed")).unwrap()
            }
            3 => replacement.semantic_basis.semantic.output_schema = TextOutputSchema::new(
                "changed",
                br#"{"type":"object","properties":{},"required":[],"additionalProperties":false}"#,
                TextSchemaLimits {
                    maximum_schema_bytes: 4096,
                    maximum_depth: 32,
                    maximum_nodes: 2048,
                },
            )
            .unwrap(),
            4 => replacement.mode = ExecutionMode::Replay,
            5 => replacement.identity.basis.run = RunId::from_bytes(&[8; 16]).unwrap(),
            _ => {}
        }
        let mut final_owner = owner(&replacement);
        if case == 6 {
            final_owner.cancelled = true;
        }
        if case == 7 {
            final_owner.elapsed = replacement.deadline;
        }
        let refusal = response.complete(final_owner).err().unwrap();
        assert!(matches!(refusal.reason(), TextResponseError::Owner(_)));
        assert_eq!(refusal.supplier_usage().unwrap().total_tokens, 20);
        assert_eq!(
            refusal.provider_request_id().map(|id| id.as_str()),
            Some("resp_fixture")
        );
        assert_eq!(
            refusal.transport_request_id().map(|id| id.as_str()),
            Some("http_fixture")
        );
        assert!(
            !tokio::task::spawn_blocking(move || server.join().unwrap())
                .await
                .unwrap()
                .extra_connection
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn prepared_replay_wrong_schema_and_cancelled_owner_do_not_egress() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let provider = provider(listener.local_addr().unwrap());
    for mode in [ExecutionMode::PreparedOnly, ExecutionMode::Replay] {
        let current = binding(mode);
        let request = request(&current);
        assert!(matches!(
            provider
                .submit(&request, owner(&current), future::pending())
                .await,
            Err(TextResponseError::ModeDoesNotDispatch)
        ));
    }
    let current = binding(ExecutionMode::Live);
    let request = request(&current);
    let mut cancelled = owner(&current);
    cancelled.cancelled = true;
    assert!(matches!(
        provider
            .submit(&request, cancelled, future::pending())
            .await,
        Err(TextResponseError::Owner(
            df_provider_api::RequestError::Cancelled
        ))
    ));
    assert!(matches!(
        provider
            .submit(&request, owner(&current), future::ready(()))
            .await
            .unwrap_err()
            .reason(),
        TextResponseError::Transport(NativeHttpRefusal::Cancelled)
    ));
    let wrong_schema = TextOutputSchema::new(
        "foreign",
        br#"{"type":"object","properties":{},"required":[],"additionalProperties":false}"#,
        TextSchemaLimits {
            maximum_schema_bytes: 4096,
            maximum_depth: 32,
            maximum_nodes: 2048,
        },
    )
    .unwrap();
    let wrong_provider = registered_text_provider(
        CandidateRouteId::OpenAiResponsesText,
        TextProviderConfig::new("caller-model-snapshot", wrong_schema, 50).unwrap(),
        "owned-key",
        limits(),
    )
    .unwrap()
    .for_loopback_fixture(listener.local_addr().unwrap())
    .unwrap();
    assert!(matches!(
        wrong_provider
            .submit(&request, owner(&current), future::pending())
            .await,
        Err(TextResponseError::SchemaConfigurationMismatch)
    ));
    let invalid_input = TextRequest::new(
        binding(ExecutionMode::Live),
        &[0xff],
        RequestUsage::new(1, Duration::ZERO, Usage::new(1, UsageUnit::Token)),
        RequestLimits::new(
            4096,
            100,
            Duration::from_secs(5),
            Usage::new(100, UsageUnit::Token),
        )
        .unwrap(),
        owner(&current),
    )
    .unwrap();
    assert!(matches!(
        provider
            .submit(&invalid_input, owner(&current), future::pending())
            .await,
        Err(TextResponseError::InvalidInput)
    ));
    let mut bounded = limits();
    bounded.maximum_request_bytes = 64;
    let bounded_provider = registered_text_provider(
        CandidateRouteId::OpenAiResponsesText,
        TextProviderConfig::new("caller-model-snapshot", schema(), 50).unwrap(),
        "owned-key",
        bounded,
    )
    .unwrap()
    .for_loopback_fixture(listener.local_addr().unwrap())
    .unwrap();
    assert!(matches!(
        bounded_provider
            .submit(&request, owner(&current), future::pending())
            .await,
        Err(TextResponseError::Transport(
            NativeHttpRefusal::RequestTooLarge
        ))
    ));
    listener.set_nonblocking(true).unwrap();
    assert!(listener.accept().is_err());
}

#[test]
fn unsupported_schema_keywords_duplicate_keys_depth_and_names_are_rejected_before_dispatch() {
    let limits = TextSchemaLimits {
        maximum_schema_bytes: 4096,
        maximum_depth: 8,
        maximum_nodes: 128,
    };
    for json in [
        br#"{"type":"object","properties":{},"required":[],"additionalProperties":true}"#.as_slice(),
        br#"{"type":"object","properties":{},"required":[],"additionalProperties":false,"allOf":[]}"#,
        br#"{"type":"object","type":"object","properties":{},"required":[],"additionalProperties":false}"#,
        br#"{"type":"object","properties":{"a":{"type":"string"}},"required":[],"additionalProperties":false}"#,
    ] { assert!(TextOutputSchema::new("valid",json,limits).is_err()); }
    assert_eq!(
        TextOutputSchema::new("bad name", b"{}", limits).unwrap_err(),
        TextSchemaError::InvalidName
    );
    assert!(TextOutputSchema::new("bounded",br#"{"type":"object","properties":{"a":{"type":"array","items":{"type":"array","items":{"type":"array","items":{"type":"string"}}}}},"required":["a"],"additionalProperties":false}"#,TextSchemaLimits { maximum_depth:5, ..limits }).is_err());
}

#[tokio::test(flavor = "current_thread")]
async fn valid_json_waits_for_actual_http_terminal_chunk_before_completion() {
    let (ready, ready_rx) = tokio::sync::oneshot::channel();
    let (release, release_rx) = std::sync::mpsc::channel();
    let (address, server) = fixture_wire(move |sent, socket| {
        let bytes = serde_json::to_vec(&completed(sent)).unwrap();
        write!(socket,"HTTP/1.1 200 fixture\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nx-request-id: held_eof\r\nConnection: close\r\n\r\n{:x}\r\n",bytes.len()).unwrap();
        socket.write_all(&bytes).unwrap();
        socket.write_all(b"\r\n").unwrap();
        ready.send(()).unwrap();
        release_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        socket.write_all(b"0\r\n\r\n").unwrap();
    });
    let current = binding(ExecutionMode::Live);
    let request = request(&current);
    let provider = provider(address);
    let submit = provider.submit(&request, owner(&current), future::pending());
    tokio::pin!(submit);
    tokio::select! {
        result = &mut submit => panic!("completed without HTTP EOF: {result:?}"),
        ready = ready_rx => ready.unwrap(),
    }
    tokio::select! {
        biased;
        result = &mut submit => panic!("completed without HTTP EOF: {result:?}"),
        () = tokio::task::yield_now() => {},
    }
    release.send(()).unwrap();
    let response = submit.await.unwrap();
    assert_eq!(response.transport_request_id(), Some("held_eof"));
    assert!(matches!(
        response.complete(owner(&current)).unwrap()[1].kind,
        TextEventKind::Completed { .. }
    ));
    assert!(
        !tokio::task::spawn_blocking(move || server.join().unwrap())
            .await
            .unwrap()
            .extra_connection
    );
}

#[tokio::test(flavor = "current_thread")]
async fn explicit_owner_cancellation_during_body_drops_owned_io_without_retry() {
    let (ready, ready_rx) = tokio::sync::oneshot::channel();
    let (release, release_rx) = std::sync::mpsc::channel();
    let (address, server) = fixture_wire(move |_, socket| {
        socket.write_all(b"HTTP/1.1 200 fixture\r\nContent-Type: application/json\r\nContent-Length: 1024\r\nx-request-id: cancel_body\r\nConnection: close\r\n\r\n{").unwrap();
        ready.send(()).unwrap();
        release_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let mut byte = [0; 1];
        match socket.read(&mut byte) {
            Ok(0) => {}
            Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => {}
            result => panic!("owned socket did not close: {result:?}"),
        }
    });
    let current = binding(ExecutionMode::Live);
    let request = request(&current);
    let error = provider(address)
        .submit(&request, owner(&current), async move {
            ready_rx.await.unwrap();
        })
        .await
        .unwrap_err();
    assert_eq!(error.failure_class(), ProviderFailureClass::Cancelled);
    release.send(()).unwrap();
    assert!(
        !tokio::task::spawn_blocking(move || server.join().unwrap())
            .await
            .unwrap()
            .extra_connection
    );
}

#[tokio::test(flavor = "current_thread")]
async fn body_deadline_retains_captured_header_id_and_never_resends() {
    let (release, release_rx) = std::sync::mpsc::channel();
    let (address, server) = fixture_wire(move |_, socket| {
        socket.write_all(b"HTTP/1.1 200 fixture\r\nContent-Type: application/json\r\nContent-Length: 1024\r\nx-request-id: deadline_body\r\nConnection: close\r\n\r\n{").unwrap();
        release_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let mut byte = [0; 1];
        match socket.read(&mut byte) {
            Ok(0) => {}
            Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => {}
            result => panic!("owned socket did not close: {result:?}"),
        }
    });
    let current = binding(ExecutionMode::Live);
    let request = request(&current);
    let mut transport = limits();
    transport.read_timeout = Duration::from_millis(150);
    let provider = registered_text_provider(
        CandidateRouteId::OpenAiResponsesText,
        TextProviderConfig::new("caller-model-snapshot", schema(), 50).unwrap(),
        "owned-key",
        transport,
    )
    .unwrap()
    .for_loopback_fixture(address)
    .unwrap();
    let error = provider
        .submit(&request, owner(&current), future::pending())
        .await
        .unwrap_err();
    assert_eq!(error.failure_class(), ProviderFailureClass::Deadline);
    assert_eq!(
        error.transport_request_id().map(|id| id.as_str()),
        Some("deadline_body")
    );
    release.send(()).unwrap();
    assert!(
        !tokio::task::spawn_blocking(move || server.join().unwrap())
            .await
            .unwrap()
            .extra_connection
    );
}

#[tokio::test(flavor = "current_thread")]
async fn redirects_do_not_follow_foreign_endpoint_or_create_a_second_submission() {
    let foreign = TcpListener::bind("127.0.0.1:0").unwrap();
    let target = foreign.local_addr().unwrap();
    let (address, server) = fixture_wire(move |_, socket| {
        write!(socket,"HTTP/1.1 307 redirect\r\nLocation: http://{target}/stolen\r\nContent-Type: application/json\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
    });
    let current = binding(ExecutionMode::Live);
    let request = request(&current);
    let error = provider(address)
        .submit(&request, owner(&current), future::pending())
        .await
        .unwrap_err();
    assert!(matches!(
        error.reason(),
        TextResponseError::Transport(NativeHttpRefusal::Redirect)
    ));
    foreign.set_nonblocking(true).unwrap();
    assert!(foreign.accept().is_err());
    assert!(
        !tokio::task::spawn_blocking(move || server.join().unwrap())
            .await
            .unwrap()
            .extra_connection
    );
}

#[tokio::test(flavor = "current_thread")]
async fn header_content_length_type_truncation_body_and_chunk_bounds_are_enforced() {
    for case in 0..7 {
        let (address, server) = fixture_wire(move |sent, socket| {
            let reply = serde_json::to_vec(&completed(sent)).unwrap();
            let wire = match case {
                0 => "HTTP/1.1 200 fixture\r\nContent-Type: application/json\r\nContent-Length: 999999\r\nConnection: close\r\n\r\n".as_bytes().to_vec(),
                1 => format!("HTTP/1.1 200 fixture\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",reply.len()).into_bytes(),
                2 => format!("HTTP/1.1 200 fixture\r\nContent-Type: application/json\r\nX-Large: {}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n","x".repeat(5000)).into_bytes(),
                3 => b"HTTP/1.1 200 fixture\r\nContent-Type: application/json\r\nContent-Length: 1000\r\nx-request-id: truncated_body\r\nConnection: close\r\n\r\n{".to_vec(),
                4 => { let mut wire = b"HTTP/1.1 200 fixture\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n5000\r\n".to_vec(); wire.extend_from_slice(&vec![b'x';20480]); wire.extend_from_slice(b"\r\n0\r\n\r\n"); wire }
                5 => { let mut wire = b"HTTP/1.1 200 fixture\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n".to_vec(); for byte in reply { wire.extend_from_slice(&[b'1',b'\r',b'\n',byte,b'\r',b'\n']); } wire.extend_from_slice(b"0\r\n\r\n"); wire }
                6 => b"HTTP/1.1 200 fixture\r\nContent-Type: application/json\r\nContent-Type: text/plain\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
                _ => unreachable!(),
            };
            let _ = socket.write_all(&wire);
        });
        let current = binding(ExecutionMode::Live);
        let request = request(&current);
        let mut transport = limits();
        if case == 5 {
            transport.maximum_chunks = 1;
        }
        let provider = registered_text_provider(
            CandidateRouteId::OpenAiResponsesText,
            TextProviderConfig::new("caller-model-snapshot", schema(), 50).unwrap(),
            "owned-key",
            transport,
        )
        .unwrap()
        .for_loopback_fixture(address)
        .unwrap();
        let error = provider
            .submit(&request, owner(&current), future::pending())
            .await
            .unwrap_err();
        let expected = match case {
            0 | 4 => NativeHttpRefusal::ResponseTooLarge,
            1 | 6 => NativeHttpRefusal::ContentType,
            2 => NativeHttpRefusal::HeadersTooLarge,
            3 => NativeHttpRefusal::Unknown,
            5 => NativeHttpRefusal::TooManyChunks,
            _ => unreachable!(),
        };
        assert!(
            matches!(error.reason(),TextResponseError::Transport(reason) if *reason == expected),
            "case {case}: {error:?}"
        );
        if case == 3 {
            assert_eq!(
                error.transport_request_id().map(|id| id.as_str()),
                Some("truncated_body")
            );
        }
        assert!(
            !tokio::task::spawn_blocking(move || server.join().unwrap())
                .await
                .unwrap()
                .extra_connection
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn connection_failure_and_supplier_error_classes_are_typed_without_payload_leaks() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let current = binding(ExecutionMode::Live);
    let request = request(&current);
    let error = provider(address)
        .submit(&request, owner(&current), future::pending())
        .await
        .unwrap_err();
    assert_eq!(error.failure_class(), ProviderFailureClass::Unknown);
    for (status, code, class) in [
        (401, "invalid_api_key", ProviderFailureClass::Denied),
        (429, "rate_limit_exceeded", ProviderFailureClass::Capacity),
        (429, "insufficient_quota", ProviderFailureClass::Denied),
        (429, "unknown", ProviderFailureClass::Unknown),
        (503, "server_error", ProviderFailureClass::Unavailable),
    ] {
        let (address, server) = fixture(move |_| {
            (
                status,
                serde_json::to_vec(
                    &json!({"error":{"code":code,"message":"credential-private-sentinel"}}),
                )
                .unwrap(),
            )
        });
        let error = provider(address)
            .submit(&request, owner(&current), future::pending())
            .await
            .unwrap_err();
        assert_eq!(error.failure_class(), class);
        assert!(!format!("{error:?}").contains("credential-private-sentinel"));
        assert_eq!(
            error.transport_request_id().map(|id| id.as_str()),
            Some("http_fixture")
        );
        assert!(
            !tokio::task::spawn_blocking(move || server.join().unwrap())
                .await
                .unwrap()
                .extra_connection
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn complete_text_reconciliation_preserves_ambiguous_billing_and_refuses_stale_identity() {
    use df_provider_api::{
        BudgetMutation, CheckedRequest, ProviderBillingClass, ProviderLiabilityDisposition,
        ProviderNextAction, ProviderResultClass, ProviderRetryPolicy,
    };
    use df_providers::{
        ProviderAttemptIdentity, ProviderRequestId, ReconciliationInput, reconcile_provider_attempt,
    };
    let (address, server) = fixture(|sent| (200, serde_json::to_vec(&completed(sent)).unwrap()));
    let current = binding(ExecutionMode::Live);
    let text_request = request(&current);
    let response = provider(address)
        .submit(&text_request, owner(&current), future::pending())
        .await
        .unwrap();
    let observation = response.observation();
    let settlement: BudgetMutation<(), (), ()> = BudgetMutation::Unknown;
    let policy = ProviderRetryPolicy {
        attempts_remaining: 2,
        retry: true,
        fallback: true,
    };
    for case in 0..5 {
        let attempt = ProviderAttemptIdentity::new(
            current.identity.operation,
            CandidateRouteId::OpenAiResponsesText,
            Some(
                ProviderRequestId::new(if case == 4 {
                    "resp_foreign"
                } else {
                    "resp_fixture"
                })
                .unwrap(),
            ),
        );
        let mut replacement = binding(ExecutionMode::Live);
        if case == 1 {
            replacement.identity.generation += 1;
        }
        if case == 2 {
            replacement.identity.basis.run = RunId::from_bytes(&[9; 16]).unwrap();
        }
        if case == 3 {
            replacement.identity.job = JobId::from_bytes(&[9; 16]).unwrap();
        }
        let mut candidate = binding(ExecutionMode::Live);
        candidate.identity = replacement.identity;
        let checked = CheckedRequest::new(
            candidate,
            b"checked",
            RequestUsage::new(1, Duration::ZERO, Usage::new(1, UsageUnit::Token)),
            RequestLimits::new(32, 100, Duration::ZERO, Usage::new(100, UsageUnit::Token)).unwrap(),
            owner(&replacement),
        )
        .unwrap();
        let outcome = reconcile_provider_attempt(
            &checked,
            &attempt,
            owner(&replacement),
            ReconciliationInput::OpenAiResponse(&observation),
            ProviderBillingClass::MissingOrAmbiguous,
            &settlement,
            policy,
        )
        .unwrap();
        assert_eq!(
            outcome.result,
            if case == 0 {
                ProviderResultClass::Complete
            } else {
                ProviderResultClass::Incomplete
            }
        );
        assert_eq!(
            outcome.liability,
            ProviderLiabilityDisposition::RetainWorstCase
        );
        assert_eq!(outcome.next, ProviderNextAction::ReconcileSameOperation);
    }
    assert!(
        !tokio::task::spawn_blocking(move || server.join().unwrap())
            .await
            .unwrap()
            .extra_connection
    );
}
