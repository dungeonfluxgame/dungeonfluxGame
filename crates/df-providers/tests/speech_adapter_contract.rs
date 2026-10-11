#![cfg(not(target_arch = "wasm32"))]

use std::{
    future,
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use df_model::checkpoint::{Basis, ContentDigest, ExecutionMode, JobId};
use df_provider_api::{
    BudgetMutation, CheckedRequest, ProviderBillingClass, ProviderFailureClass,
    ProviderLiabilityDisposition, ProviderNextAction, ProviderResultClass, ProviderRetryPolicy,
    RequestBinding, RequestIdentity, RequestLimits, RequestModality, RequestOwnerState,
    RequestUsage, SttEventKind, SttSemantics, TtsEventKind, TtsSemantics,
};
use df_providers::{
    CandidateRouteId, ElevenSpeechBasis, ElevenSpeechRequest, ElevenSttBasis, ElevenSttRequest,
    NativeHttpLimits, NativeHttpRefusal, NativeSpeechProvider, ProviderAttemptIdentity,
    ProviderRequestId, SpeechAdapterReason, SpeechInputFormat, SpeechOutputFormat,
    SpeechTranscriptProfile, TranscriptWordKind, candidate_for, candidate_routes,
    reconcile_speech_attempt, reconcile_stt_attempt, registered_speech_provider,
};
use df_types::{
    LocaleTag, OperationId, RecoveryEpoch, RevisionLabel, RunId, SessionId, SessionRevision, Usage,
    UsageUnit,
};
use serde_json::{Value, json};

const SENTINEL: &str = "private-payload-sentinel";
const HEADERS: &str = "request-id: supplier_private\r\nx-request-id: transport_private\r\nx-trace-id: trace_private\r\ncharacter-cost: 7.5\r\n";
const AUDIO: &[u8] = b"opaque-encoded-private-sentinel";

fn limits() -> NativeHttpLimits {
    NativeHttpLimits {
        connect_timeout: Duration::from_secs(1),
        read_timeout: Duration::from_secs(2),
        total_timeout: Duration::from_secs(3),
        maximum_request_bytes: 16_384,
        maximum_response_bytes: 16_384,
        maximum_header_bytes: 4096,
        maximum_chunks: 4096,
    }
}

fn identity() -> RequestIdentity {
    RequestIdentity {
        basis: Basis {
            session: SessionId::from_bytes(&[1; 16]).unwrap(),
            run: RunId::from_bytes(&[2; 16]).unwrap(),
            revision: SessionRevision::new(RecoveryEpoch::new(1).unwrap(), 2),
        },
        job: JobId::from_bytes(&[3; 16]).unwrap(),
        operation: OperationId::from_bytes(&[4; 16]).unwrap(),
        generation: 7,
    }
}

fn stt_binding() -> RequestBinding<ElevenSttBasis> {
    RequestBinding {
        identity: identity(),
        semantic_basis: ElevenSttBasis {
            modality: RequestModality::Stt,
            source: ContentDigest([5; 32]),
            rights_revision: RevisionLabel::new(Some("rights")).unwrap(),
            output_format_revision: RevisionLabel::new(Some("format")).unwrap(),
            semantic: SttSemantics {
                locale: LocaleTag::parse("en-US").unwrap(),
                audio_format: SpeechInputFormat::PcmS16Le16Mono,
                transcript_profile: SpeechTranscriptProfile::FinalWords,
            },
        },
        mode: ExecutionMode::Live,
        deadline: Duration::from_secs(5),
    }
}

fn speech_binding() -> RequestBinding<ElevenSpeechBasis> {
    RequestBinding {
        identity: identity(),
        semantic_basis: ElevenSpeechBasis {
            modality: RequestModality::Tts,
            source: ContentDigest([5; 32]),
            rights_revision: RevisionLabel::new(Some("rights")).unwrap(),
            output_format_revision: RevisionLabel::new(Some("format")).unwrap(),
            semantic: TtsSemantics {
                locale: LocaleTag::parse("en-GB").unwrap(),
                text: format!("{SENTINEL} \"\\\n雪"),
                voice: "voice/private ?雪".to_owned(),
                output_format: SpeechOutputFormat::Mp3At22050Hz32Kbps,
                pcm_format: None,
                timebase: None,
            },
        },
        mode: ExecutionMode::Live,
        deadline: Duration::from_secs(5),
    }
}

fn copy_binding<S: Clone>(b: &RequestBinding<S>) -> RequestBinding<S> {
    RequestBinding {
        identity: b.identity,
        semantic_basis: b.semantic_basis.clone(),
        mode: b.mode,
        deadline: b.deadline,
    }
}

fn owner<S>(b: &RequestBinding<S>) -> RequestOwnerState<'_, S> {
    RequestOwnerState {
        current: Some(b),
        elapsed: Duration::ZERO,
        cancelled: false,
    }
}

fn stt_request(b: &RequestBinding<ElevenSttBasis>, payload: &[u8]) -> ElevenSttRequest {
    let d = Duration::from_nanos((payload.len() as u64 / 2) * 62_500);
    ElevenSttRequest::new(
        copy_binding(b),
        payload,
        RequestUsage::new(
            0,
            d,
            Usage::new(
                d.as_nanos().div_ceil(1_000_000),
                UsageUnit::AudioMillisecond,
            ),
        ),
        RequestLimits::new(
            2_000_000,
            0,
            Duration::from_secs(60),
            Usage::new(60_000, UsageUnit::AudioMillisecond),
        )
        .unwrap(),
        owner(b),
    )
    .unwrap()
}

fn speech_request(b: &RequestBinding<ElevenSpeechBasis>) -> ElevenSpeechRequest {
    let t = &b.semantic_basis.semantic.text;
    ElevenSpeechRequest::new(
        copy_binding(b),
        t.as_bytes(),
        RequestUsage::new(
            0,
            Duration::ZERO,
            Usage::new(t.chars().count() as u128, UsageUnit::Character),
        ),
        RequestLimits::new(
            32_768,
            0,
            Duration::ZERO,
            Usage::new(10_000, UsageUnit::Character),
        )
        .unwrap(),
        owner(b),
    )
    .unwrap()
}

fn provider(
    route: CandidateRouteId,
    address: SocketAddr,
    cap: NativeHttpLimits,
) -> NativeSpeechProvider {
    registered_speech_provider(route, "credential-private-sentinel", cap)
        .unwrap()
        .for_loopback_fixture(address)
        .unwrap()
}

struct Seen {
    target: String,
    headers: String,
    body: Vec<u8>,
    extra_connection: bool,
}

fn read_request(socket: &mut TcpStream) -> Seen {
    socket
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        let mut b = [0];
        socket.read_exact(&mut b).unwrap();
        head.push(b[0]);
        assert!(head.len() <= 16_384);
    }
    let headers = String::from_utf8(head).unwrap();
    let length: usize = headers
        .lines()
        .find_map(|l| {
            l.split_once(':')
                .filter(|(n, _)| n.eq_ignore_ascii_case("content-length"))
                .map(|(_, v)| v.trim().parse().unwrap())
        })
        .unwrap();
    assert!(length <= 2_100_000);
    let mut body = vec![0; length];
    socket.read_exact(&mut body).unwrap();
    let target = headers
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .to_owned();
    Seen {
        target,
        headers,
        body,
        extra_connection: false,
    }
}

fn wire(status: u16, mime: &str, headers: &str, body: &[u8]) -> Vec<u8> {
    let mut v = format!("HTTP/1.1 {status} fixture\r\nContent-Type: {mime}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n",body.len()).into_bytes();
    v.extend_from_slice(body);
    v
}

fn chunked(mime: &str, headers: &str, body: &[u8]) -> Vec<u8> {
    let mut v = format!("HTTP/1.1 200 fixture\r\nContent-Type: {mime}\r\n{headers}Transfer-Encoding: chunked\r\nConnection: close\r\n\r\n").into_bytes();
    for part in body.chunks(7) {
        v.extend_from_slice(format!("{:x}\r\n", part.len()).as_bytes());
        v.extend_from_slice(part);
        v.extend_from_slice(b"\r\n");
    }
    v.extend_from_slice(b"0\r\n\r\n");
    v
}

fn fixture(
    reply: impl FnOnce(&Seen) -> Vec<u8> + Send + 'static,
) -> (SocketAddr, thread::JoinHandle<Seen>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let task = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut seen = read_request(&mut socket);
        socket.write_all(&reply(&seen)).ok();
        drop(socket);
        listener.set_nonblocking(true).unwrap();
        let end = Instant::now() + Duration::from_millis(25);
        while Instant::now() < end {
            if listener.accept().is_ok() {
                seen.extra_connection = true;
                break;
            }
            thread::sleep(Duration::from_millis(1));
        }
        seen
    });
    (address, task)
}

async fn joined(task: thread::JoinHandle<Seen>) -> Seen {
    tokio::task::spawn_blocking(move || task.join().unwrap())
        .await
        .unwrap()
}

fn transcript() -> Value {
    json!({"language_code":"eng","language_probability":0.98,"text":format!("{SENTINEL} exact  whitespace"),"words":[
        {"text":"raw","type":"word","logprob":-0.124,"start":0.0,"end":0.09},
        {"text":"  ","type":"spacing","logprob":0.0}
    ],"transcription_id":"transcription_private","audio_duration_secs":0.1})
}

#[tokio::test]
async fn scribe_registered_route_sends_exact_multipart_and_preserves_verbatim_final_profile() {
    let (address, task) = fixture(|seen| {
        assert_eq!(seen.target, "/v1/speech-to-text");
        chunked(
            "application/json",
            HEADERS,
            &serde_json::to_vec(&transcript()).unwrap(),
        )
    });
    let b = stt_binding();
    let pcm = vec![0; 3200];
    let request = stt_request(&b, &pcm);
    let prepared = provider(CandidateRouteId::ElevenScribeV2Stt, address, limits())
        .transcribe(&request, owner(&b), future::pending())
        .await
        .unwrap();
    let observed = prepared.observation();
    assert_eq!(observed.identity(), b.identity);
    assert_eq!(
        observed.workload(),
        Usage::new(100, UsageUnit::AudioMillisecond)
    );
    assert_eq!(
        observed.metadata().request_id().unwrap().as_str(),
        "supplier_private"
    );
    assert_eq!(
        observed.metadata().transport_request_id().unwrap().as_str(),
        "transport_private"
    );
    assert_eq!(observed.metadata().character_cost(), Some("7.5"));
    assert_eq!(
        observed.transcription_id().unwrap().as_str(),
        "transcription_private"
    );
    assert_eq!(observed.audio_duration_seconds(), Some(0.1));
    let [final_event, completed] = prepared.complete(owner(&b)).unwrap();
    assert_eq!(final_event.usage, completed.usage);
    let SttEventKind::Final(out) = final_event.kind else {
        panic!("no final transcript")
    };
    assert_eq!(out.locale(), &b.semantic_basis.semantic.locale);
    assert_eq!(out.detected_language(), "eng");
    assert_eq!(out.text(), format!("{SENTINEL} exact  whitespace"));
    assert_eq!(out.words()[1].kind, TranscriptWordKind::Spacing);
    assert_eq!(out.words()[1].start_seconds, None);
    assert!(matches!(completed.kind,SttEventKind::Completed { identity } if identity==b.identity));
    let seen = joined(task).await;
    assert!(!seen.extra_connection);
    assert!(
        seen.headers
            .to_ascii_lowercase()
            .contains("content-type: multipart/form-data; boundary=df-scribe-v2-")
    );
    let body = String::from_utf8_lossy(&seen.body);
    for (name, value) in [
        ("model_id", "scribe_v2"),
        ("language_code", "en"),
        ("file_format", "pcm_s16le_16"),
        ("timestamps_granularity", "word"),
        ("webhook", "false"),
        ("use_multi_channel", "false"),
        ("diarize", "false"),
        ("tag_audio_events", "false"),
        ("no_verbatim", "false"),
        ("use_speaker_library", "false"),
        ("detect_speaker_roles", "false"),
    ] {
        assert!(body.contains(&format!("name=\"{name}\"\r\n\r\n{value}\r\n")));
    }
    assert!(seen.body.windows(pcm.len()).any(|w| w == pcm));
    for absent in [
        "source_url",
        "transcript_edit",
        "keyterms",
        "webhook_metadata",
    ] {
        assert!(!body.contains(absent));
    }
}

#[tokio::test]
async fn flash_preserves_caller_text_voice_encoded_profile_and_terminal_events() {
    let (address, task) = fixture(|seen| {
        assert!(
            seen.target
                .contains("voice%2Fprivate%20%3F%E9%9B%AA/stream?output_format=mp3_22050_32")
        );
        chunked("audio/mpeg", HEADERS, AUDIO)
    });
    let b = speech_binding();
    let request = speech_request(&b);
    let prepared = provider(CandidateRouteId::ElevenFlashV25Tts, address, limits())
        .speak(&request, owner(&b), future::pending())
        .await
        .unwrap();
    let observation = prepared.observation();
    assert_eq!(
        observation.workload().quantity(),
        b.semantic_basis.semantic.text.chars().count() as u128
    );
    assert_eq!(observation.metadata().character_cost(), Some("7.5"));
    let [chunk, completed] = prepared.complete(owner(&b)).unwrap();
    assert_eq!(chunk.usage, completed.usage);
    let TtsEventKind::Chunk {
        sequence,
        chunk: audio,
    } = chunk.kind
    else {
        panic!("no encoded chunk")
    };
    assert_eq!(sequence, 0);
    assert_eq!(audio.bytes(), AUDIO);
    assert_eq!(audio.locale(), &b.semantic_basis.semantic.locale);
    assert_eq!(audio.voice(), b.semantic_basis.semantic.voice);
    assert_eq!(
        audio.requested_format(),
        SpeechOutputFormat::Mp3At22050Hz32Kbps
    );
    assert!(matches!(completed.kind,TtsEventKind::Completed { identity } if identity==b.identity));
    let seen = joined(task).await;
    assert!(!seen.extra_connection);
    let sent: Value = serde_json::from_slice(&seen.body).unwrap();
    assert_eq!(
        sent,
        json!({"text":b.semantic_basis.semantic.text,"model_id":"eleven_flash_v2_5","language_code":"en"})
    );
}

#[tokio::test]
async fn unsupported_profiles_modes_cancelled_and_stale_owners_never_egress() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let stt = provider(CandidateRouteId::ElevenScribeV2Stt, address, limits());
    let speech = provider(CandidateRouteId::ElevenFlashV25Tts, address, limits());
    for case in 0..8 {
        let mut b = stt_binding();
        match case {
            0 => b.semantic_basis.semantic.locale = LocaleTag::parse("fr-FR").unwrap(),
            1 => b.semantic_basis.semantic.audio_format = SpeechInputFormat::Other,
            2 => b.semantic_basis.semantic.transcript_profile = SpeechTranscriptProfile::Characters,
            3 => b.mode = ExecutionMode::PreparedOnly,
            4 => b.mode = ExecutionMode::Replay,
            _ => {}
        }
        let request = stt_request(&b, &vec![0; 3200]);
        let mut replacement = copy_binding(&b);
        if case == 6 {
            replacement.identity.generation += 1;
        }
        let mut state = owner(&replacement);
        if case == 5 {
            state.cancelled = true;
        }
        if case == 7 {
            state.current = None;
        }
        assert!(
            stt.transcribe(&request, state, future::pending())
                .await
                .is_err()
        );
    }
    for case in 0..7 {
        let mut b = speech_binding();
        match case {
            0 => b.semantic_basis.semantic.locale = LocaleTag::parse("ja-JP").unwrap(),
            1 => b.semantic_basis.semantic.output_format = SpeechOutputFormat::PcmAt16000Hz,
            2 => b.semantic_basis.semantic.pcm_format = Some(()),
            3 => b.semantic_basis.semantic.timebase = Some(()),
            4 => b.semantic_basis.semantic.voice = " ".to_owned(),
            5 => b.semantic_basis.semantic.voice = "x".repeat(129),
            _ => b.semantic_basis.semantic.text = "x".repeat(4097),
        }
        let request = speech_request(&b);
        assert!(
            speech
                .speak(&request, owner(&b), future::pending())
                .await
                .is_err()
        );
    }
    let b = stt_binding();
    let request = stt_request(&b, &vec![0; 3200]);
    assert!(
        speech
            .transcribe(&request, owner(&b), future::pending())
            .await
            .is_err()
    );
    assert_eq!(
        stt.transcribe(&request, owner(&b), future::ready(()))
            .await
            .unwrap_err()
            .reason(),
        SpeechAdapterReason::Transport(NativeHttpRefusal::Cancelled)
    );
    let b = speech_binding();
    let request = speech_request(&b);
    assert_eq!(
        speech
            .speak(&request, owner(&b), future::ready(()))
            .await
            .unwrap_err()
            .reason(),
        SpeechAdapterReason::Transport(NativeHttpRefusal::Cancelled)
    );
    assert!(listener.accept().is_err());
}

#[tokio::test]
async fn complete_rechecks_every_current_binding_field_before_any_event() {
    for case in 0..14 {
        let (address, task) = fixture(|_| wire(200, "audio/mpeg", HEADERS, AUDIO));
        let b = speech_binding();
        let request = speech_request(&b);
        let prepared = provider(CandidateRouteId::ElevenFlashV25Tts, address, limits())
            .speak(&request, owner(&b), future::pending())
            .await
            .unwrap();
        let mut replacement = copy_binding(&b);
        match case {
            0 => replacement.identity.generation += 1,
            1 => replacement.identity.job = JobId::from_bytes(&[9; 16]).unwrap(),
            2 => replacement.identity.operation = OperationId::from_bytes(&[9; 16]).unwrap(),
            3 => replacement.identity.basis.run = RunId::from_bytes(&[9; 16]).unwrap(),
            4 => replacement.semantic_basis.source = ContentDigest([9; 32]),
            5 => {
                replacement.semantic_basis.rights_revision =
                    RevisionLabel::new(Some("foreign")).unwrap()
            }
            6 => {
                replacement.semantic_basis.output_format_revision =
                    RevisionLabel::new(Some("foreign")).unwrap()
            }
            7 => replacement.semantic_basis.semantic.locale = LocaleTag::parse("en-US").unwrap(),
            8 => replacement.semantic_basis.semantic.voice = "foreign".to_owned(),
            9 => replacement.semantic_basis.semantic.text = "foreign".to_owned(),
            10 => {
                replacement.semantic_basis.semantic.output_format = SpeechOutputFormat::PcmAt16000Hz
            }
            11 => replacement.mode = ExecutionMode::Replay,
            _ => {}
        }
        let mut state = owner(&replacement);
        if case == 12 {
            state.cancelled = true;
        }
        if case == 13 {
            state.elapsed = b.deadline;
        }
        let error = match prepared.complete(state) {
            Err(error) => error,
            Ok(_) => panic!("stale completion emitted an event"),
        };
        assert!(matches!(error.reason(), SpeechAdapterReason::Owner(_)));
        assert_eq!(
            error.metadata().request_id().unwrap().as_str(),
            "supplier_private"
        );
        assert_eq!(error.metadata().character_cost(), Some("7.5"));
        assert!(!joined(task).await.extra_connection);
    }
    for case in 0..3 {
        let (address, task) = fixture(|_| {
            wire(
                200,
                "application/json",
                HEADERS,
                &serde_json::to_vec(&transcript()).unwrap(),
            )
        });
        let b = stt_binding();
        let request = stt_request(&b, &vec![0; 3200]);
        let prepared = provider(CandidateRouteId::ElevenScribeV2Stt, address, limits())
            .transcribe(&request, owner(&b), future::pending())
            .await
            .unwrap();
        let mut replacement = copy_binding(&b);
        match case {
            0 => replacement.semantic_basis.semantic.locale = LocaleTag::parse("en-GB").unwrap(),
            1 => {
                replacement.semantic_basis.semantic.transcript_profile =
                    SpeechTranscriptProfile::Characters
            }
            _ => replacement.semantic_basis.source = ContentDigest([9; 32]),
        }
        let error = match prepared.complete(owner(&replacement)) {
            Err(error) => error,
            Ok(_) => panic!("stale STT completion emitted an event"),
        };
        assert!(matches!(error.reason(), SpeechAdapterReason::Owner(_)));
        assert_eq!(
            error.transcription_id().unwrap().as_str(),
            "transcription_private"
        );
        assert!(!joined(task).await.extra_connection);
    }
}

#[tokio::test]
async fn absent_metadata_silent_audio_and_detected_language_remain_distinct() {
    for case in 0..2 {
        let (address, task) = fixture(move |_| {
            let mut j = transcript();
            j.as_object_mut().unwrap().remove("transcription_id");
            j.as_object_mut().unwrap().remove("audio_duration_secs");
            if case == 0 {
                j["text"] = json!("");
                j["words"] = json!([]);
            } else {
                j["language_code"] = json!("fr");
            }
            wire(
                200,
                "application/json",
                "",
                &serde_json::to_vec(&j).unwrap(),
            )
        });
        let b = stt_binding();
        let request = stt_request(&b, &vec![0; 3200]);
        let p = provider(CandidateRouteId::ElevenScribeV2Stt, address, limits())
            .transcribe(&request, owner(&b), future::pending())
            .await
            .unwrap();
        let o = p.observation();
        assert!(o.metadata().request_id().is_none());
        assert!(o.transcription_id().is_none());
        assert!(o.audio_duration_seconds().is_none());
        let [event, _] = p.complete(owner(&b)).unwrap();
        assert!(event.provider_request_id.0.is_none());
        let SttEventKind::Final(out) = event.kind else {
            panic!("final")
        };
        assert_eq!(out.locale(), &b.semantic_basis.semantic.locale);
        if case == 0 {
            assert!(out.text().is_empty());
            assert!(out.words().is_empty());
        } else {
            assert_eq!(out.detected_language(), "fr");
        }
        assert!(!joined(task).await.extra_connection);
    }
}

#[tokio::test]
async fn malformed_duplicate_and_unselected_transcript_shapes_never_emit_final() {
    for case in 0..17 {
        let (address, task) = fixture(move |_| {
            let mut j = transcript();
            match case {
                0 => {
                    j.as_object_mut().unwrap().remove("words");
                }
                1 => j["language_probability"] = json!(1.1),
                2 => j["words"][0]["logprob"] = json!(0.1),
                3 => j["words"][0]["start"] = json!(-0.1),
                4 => j["words"][0]["end"] = json!(0.11),
                5 => j["words"][0]["type"] = json!("audio_event"),
                6 => j["channel_index"] = json!(0),
                7 => j["edited_transcript"] = json!({"text":"private"}),
                8 => j["entities"] = json!([{}]),
                9 => j["words"][0]["characters"] = json!([{}]),
                10 => j["words"][0]["speaker_id"] = json!("speaker_1"),
                11 => j["words"][0]["end"] = json!(-1),
                12 => j["unexpected"] = json!([]),
                13 => j["text"] = json!("x".repeat(65_537)),
                14 => {
                    j["words"] = json!(vec![json!({"text":"","type":"spacing","logprob":0}); 4097])
                }
                _ => {}
            }
            let mut body = serde_json::to_vec(&j).unwrap();
            if case == 15 {
                body.extend_from_slice(b" trailing");
            }
            if case == 16 {
                body.splice(1..1, b"\"language_code\":\"en\",".iter().copied());
            }
            wire(200, "application/json", HEADERS, &body)
        });
        let b = stt_binding();
        let request = stt_request(&b, &vec![0; 3200]);
        let mut cap = limits();
        cap.maximum_response_bytes = 1_048_576;
        let error = provider(CandidateRouteId::ElevenScribeV2Stt, address, cap)
            .transcribe(&request, owner(&b), future::pending())
            .await
            .unwrap_err();
        assert!(matches!(
            error.reason(),
            SpeechAdapterReason::Json | SpeechAdapterReason::Transcript
        ));
        assert_eq!(
            error.metadata().request_id().unwrap().as_str(),
            "supplier_private"
        );
        assert!(!joined(task).await.extra_connection);
    }
}

#[tokio::test]
async fn pcm_minimum_fractional_millisecond_ceiling_and_exact_maximum_are_checked() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let p = provider(CandidateRouteId::ElevenScribeV2Stt, address, limits());
    let b = stt_binding();
    for n in [3198, 3199, 3201, 1_920_001] {
        let request = stt_request(&b, &vec![0; n]);
        assert!(
            p.transcribe(&request, owner(&b), future::pending())
                .await
                .is_err()
        );
    }
    assert!(listener.accept().is_err());
    for n in [3202, 1_920_000] {
        let (address, task) = fixture(|_| {
            wire(
                200,
                "application/json",
                "",
                br#"{"language_code":"en","language_probability":1,"text":"","words":[]}"#,
            )
        });
        let mut cap = limits();
        cap.maximum_request_bytes = 2_100_000;
        let request = stt_request(&b, &vec![0; n]);
        let prepared = provider(CandidateRouteId::ElevenScribeV2Stt, address, cap)
            .transcribe(&request, owner(&b), future::pending())
            .await
            .unwrap();
        assert_eq!(
            prepared.observation().workload().quantity(),
            if n == 3202 { 101 } else { 60_000 }
        );
        assert!(!joined(task).await.extra_connection);
    }
}

#[tokio::test]
async fn canonical_workload_text_and_duration_mismatch_refuse_without_send() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let b = stt_binding();
    let request = ElevenSttRequest::new(
        copy_binding(&b),
        &vec![0; 3200],
        RequestUsage::new(
            0,
            Duration::from_millis(99),
            Usage::new(99, UsageUnit::AudioMillisecond),
        ),
        RequestLimits::new(
            4096,
            0,
            Duration::from_secs(60),
            Usage::new(60_000, UsageUnit::AudioMillisecond),
        )
        .unwrap(),
        owner(&b),
    )
    .unwrap();
    assert_eq!(
        provider(CandidateRouteId::ElevenScribeV2Stt, address, limits())
            .transcribe(&request, owner(&b), future::pending())
            .await
            .unwrap_err()
            .reason(),
        SpeechAdapterReason::InvalidInput
    );
    let b = speech_binding();
    let request = ElevenSpeechRequest::new(
        copy_binding(&b),
        b"foreign text",
        RequestUsage::new(0, Duration::ZERO, Usage::new(1, UsageUnit::Character)),
        RequestLimits::new(
            4096,
            0,
            Duration::ZERO,
            Usage::new(100, UsageUnit::Character),
        )
        .unwrap(),
        owner(&b),
    )
    .unwrap();
    assert_eq!(
        provider(CandidateRouteId::ElevenFlashV25Tts, address, limits())
            .speak(&request, owner(&b), future::pending())
            .await
            .unwrap_err()
            .reason(),
        SpeechAdapterReason::InvalidInput
    );
    assert!(listener.accept().is_err());
}

#[tokio::test]
async fn multipart_boundary_collision_scan_and_complete_body_overhead_are_bounded() {
    let b = stt_binding();
    let mut pcm = vec![0; 3200];
    for n in 0..8 {
        let s = format!("df-scribe-v2-{}-{n}", "04".repeat(16));
        let start = n * 64;
        pcm[start..start + s.len()].copy_from_slice(s.as_bytes());
    }
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let request = stt_request(&b, &pcm);
    let p = provider(CandidateRouteId::ElevenScribeV2Stt, address, limits());
    assert_eq!(
        p.transcribe(&request, owner(&b), future::pending())
            .await
            .unwrap_err()
            .reason(),
        SpeechAdapterReason::InvalidInput
    );
    let request = stt_request(&b, &vec![0; 3200]);
    let mut cap = limits();
    cap.maximum_request_bytes = 3200;
    assert_eq!(
        provider(CandidateRouteId::ElevenScribeV2Stt, address, cap)
            .transcribe(&request, owner(&b), future::pending())
            .await
            .unwrap_err()
            .reason(),
        SpeechAdapterReason::Transport(NativeHttpRefusal::RequestTooLarge)
    );
    assert!(listener.accept().is_err());
}

#[tokio::test]
async fn eof_is_required_and_truncated_or_empty_audio_never_completes() {
    for case in 0..3 {
        let (address, task) = fixture(move |_| {
            match case {
            0=>b"HTTP/1.1 200 fixture\r\nContent-Type: audio/mpeg\r\nrequest-id: supplier_private\r\nContent-Length: 500\r\nConnection: close\r\n\r\npartial".to_vec(),
            1=>b"HTTP/1.1 200 fixture\r\nContent-Type: audio/mpeg\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n3\r\nabc\r\n".to_vec(),
            _=>wire(200,"audio/mpeg",HEADERS,b"") }
        });
        let b = speech_binding();
        let request = speech_request(&b);
        let error = provider(CandidateRouteId::ElevenFlashV25Tts, address, limits())
            .speak(&request, owner(&b), future::pending())
            .await
            .unwrap_err();
        if case == 2 {
            assert_eq!(error.reason(), SpeechAdapterReason::EmptyAudio);
        } else {
            assert!(matches!(error.reason(), SpeechAdapterReason::Transport(_)));
        }
        assert!(!joined(task).await.extra_connection);
    }
}

#[tokio::test]
async fn actual_terminal_chunk_holds_output_until_eof() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (release_tx, release_rx) = mpsc::channel();
    let (sent_tx, sent_rx) = mpsc::channel();
    let task = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let seen = read_request(&mut socket);
        let mut partial = chunked("audio/mpeg", HEADERS, AUDIO);
        partial.truncate(partial.len() - 5);
        socket.write_all(&partial).unwrap();
        sent_tx.send(()).unwrap();
        release_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        socket.write_all(b"0\r\n\r\n").unwrap();
        seen
    });
    let b = speech_binding();
    let request = speech_request(&b);
    let p = provider(CandidateRouteId::ElevenFlashV25Tts, address, limits());
    let submission = p.speak(&request, owner(&b), future::pending());
    tokio::pin!(submission);
    assert!(
        tokio::time::timeout(Duration::from_millis(40), &mut submission)
            .await
            .is_err()
    );
    sent_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    release_tx.send(()).unwrap();
    let prepared = submission.await.unwrap();
    assert!(matches!(
        &prepared.complete(owner(&b)).unwrap()[1].kind,
        TtsEventKind::Completed { .. }
    ));
    joined(task).await;
    // The second body frame is withheld while the first is consumed. One frame
    // is the exact admitted ceiling; the released second frame exceeds it.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (release_tx, release_rx) = mpsc::channel();
    let (sent_tx, sent_rx) = mpsc::channel();
    let task = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let seen = read_request(&mut socket);
        socket.write_all(b"HTTP/1.1 200 fixture\r\nContent-Type: audio/mpeg\r\nrequest-id: chunk_supplier\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n1\r\na\r\n").unwrap();
        sent_tx.send(()).unwrap();
        release_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        socket.write_all(b"1\r\nb\r\n0\r\n\r\n").unwrap();
        seen
    });
    let mut cap = limits();
    cap.maximum_chunks = 1;
    let p = provider(CandidateRouteId::ElevenFlashV25Tts, address, cap);
    let submission = p.speak(&request, owner(&b), future::pending());
    tokio::pin!(submission);
    assert!(
        tokio::time::timeout(Duration::from_millis(40), &mut submission)
            .await
            .is_err()
    );
    sent_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    release_tx.send(()).unwrap();
    let error = submission.await.unwrap_err();
    assert_eq!(
        error.reason(),
        SpeechAdapterReason::Transport(NativeHttpRefusal::TooManyChunks)
    );
    assert_eq!(
        error.metadata().request_id().unwrap().as_str(),
        "chunk_supplier"
    );
    joined(task).await;
}

#[tokio::test]
async fn response_type_selected_headers_and_exact_body_bounds_fail_closed() {
    for case in 0..10 {
        let (address, task) = fixture(move |_| match case {
            0 => wire(200, "application/json", HEADERS, AUDIO),
            1 => wire(
                200,
                "audio/mpeg",
                "request-id: first\r\nrequest-id: second\r\n",
                AUDIO,
            ),
            2 => wire(
                200,
                "audio/mpeg",
                &format!("request-id: {}\r\n", "x".repeat(257)),
                AUDIO,
            ),
            3 => wire(200, "audio/mpeg", "Content-Type: audio/mpeg\r\n", AUDIO),
            4 => wire(200, "audio/mpeg", HEADERS, &vec![b'x'; 16_385]),
            5 => wire(
                200,
                "audio/mpeg",
                &format!("X-Large: {}\r\n", "x".repeat(4096)),
                AUDIO,
            ),
            6 => wire(200, "audio/mpeg", HEADERS, &vec![b'x'; 16_384]),
            7 => format!(
                "HTTP/1.1 200 fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                AUDIO.len()
            )
            .into_bytes()
            .into_iter()
            .chain(AUDIO.iter().copied())
            .collect(),
            _ => wire(200, "audio/mpeg", HEADERS, AUDIO),
        });
        let b = speech_binding();
        let request = speech_request(&b);
        let mut cap = limits();
        if case >= 8 {
            cap.maximum_header_bytes = [
                ("content-type", "audio/mpeg".to_owned()),
                ("content-length", AUDIO.len().to_string()),
                ("connection", "close".to_owned()),
                ("request-id", "supplier_private".to_owned()),
                ("x-request-id", "transport_private".to_owned()),
                ("x-trace-id", "trace_private".to_owned()),
                ("character-cost", "7.5".to_owned()),
            ]
            .iter()
            .map(|(name, value)| name.len() + value.len())
            .sum::<usize>()
                - usize::from(case == 9);
        }
        let result = provider(CandidateRouteId::ElevenFlashV25Tts, address, cap)
            .speak(&request, owner(&b), future::pending())
            .await;
        if case == 6 || case == 8 {
            assert!(result.is_ok());
        } else {
            assert!(matches!(
                result.unwrap_err().reason(),
                SpeechAdapterReason::Transport(_)
            ));
        }
        assert!(!joined(task).await.extra_connection);
    }
}

#[tokio::test]
async fn cancellation_deadline_and_future_drop_close_owned_socket_once_without_resend() {
    for case in 0..3 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (sent_tx, sent_rx) = mpsc::channel();
        let task = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut seen = read_request(&mut socket);
            socket.write_all(b"HTTP/1.1 200 fixture\r\nContent-Type: audio/mpeg\r\nrequest-id: cancelled_supplier\r\nx-request-id: cancelled_transport\r\ncharacter-cost: 12.5\r\nContent-Length: 4096\r\nConnection: close\r\n\r\npart").unwrap();
            sent_tx.send(()).unwrap();
            let mut b = [0];
            match socket.read(&mut b) {
                Ok(0) => {}
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::ConnectionReset
                            | std::io::ErrorKind::BrokenPipe
                            | std::io::ErrorKind::ConnectionAborted
                    ) => {}
                result => panic!("owned socket was not closed: {result:?}"),
            };
            listener.set_nonblocking(true).unwrap();
            seen.extra_connection = listener.accept().is_ok();
            seen
        });
        let mut b = speech_binding();
        if case == 1 {
            b.deadline = Duration::from_millis(60);
        }
        let request = speech_request(&b);
        let p = provider(CandidateRouteId::ElevenFlashV25Tts, address, limits());
        if case == 0 {
            let cancellation = async {
                tokio::time::sleep(Duration::from_millis(40)).await;
            };
            let error = p
                .speak(&request, owner(&b), cancellation)
                .await
                .unwrap_err();
            assert_eq!(
                error.reason(),
                SpeechAdapterReason::Transport(NativeHttpRefusal::Cancelled)
            );
            assert_eq!(
                error.metadata().request_id().unwrap().as_str(),
                "cancelled_supplier"
            );
            assert_eq!(error.metadata().character_cost(), Some("12.5"));
        } else if case == 1 {
            let error = p
                .speak(&request, owner(&b), future::pending())
                .await
                .unwrap_err();
            assert_eq!(
                error.reason(),
                SpeechAdapterReason::Transport(NativeHttpRefusal::Deadline)
            );
            assert_eq!(
                error.metadata().request_id().unwrap().as_str(),
                "cancelled_supplier"
            );
        } else {
            let mut submission = Box::pin(p.speak(&request, owner(&b), future::pending()));
            assert!(
                tokio::time::timeout(Duration::from_millis(40), &mut submission)
                    .await
                    .is_err()
            );
            sent_rx.recv_timeout(Duration::from_secs(1)).unwrap();
            drop(submission);
        }
        assert!(!joined(task).await.extra_connection);
    }
}

#[tokio::test]
async fn supplier_failures_preserve_safe_identity_without_private_payload_or_retry() {
    for (status, kind, code, class) in [
        (
            400,
            "validation_error",
            "invalid_parameters",
            ProviderFailureClass::InvalidRequest,
        ),
        (
            422,
            "validation_error",
            "invalid_parameters",
            ProviderFailureClass::InvalidRequest,
        ),
        (
            401,
            "authentication_error",
            "invalid_api_key",
            ProviderFailureClass::Denied,
        ),
        (
            402,
            "payment_required",
            "insufficient_credits",
            ProviderFailureClass::Denied,
        ),
        (
            403,
            "authorization_error",
            "voice_access_denied",
            ProviderFailureClass::Denied,
        ),
        (
            429,
            "rate_limit_error",
            "concurrent_limit_exceeded",
            ProviderFailureClass::Capacity,
        ),
        (
            503,
            "service_unavailable",
            "service_unavailable",
            ProviderFailureClass::Unavailable,
        ),
        (
            500,
            "internal_error",
            "internal_error",
            ProviderFailureClass::Unknown,
        ),
        (
            404,
            "not_found",
            "voice_not_found",
            ProviderFailureClass::Unknown,
        ),
        (409, "conflict", "conflict", ProviderFailureClass::Unknown),
    ] {
        let (address, task) = fixture(move |_| {
            wire(status,"application/json",HEADERS,&serde_json::to_vec(&json!({"detail":{"type":kind,"code":code,"message":SENTINEL,"param":SENTINEL,"request_id":"error_private"}})).unwrap())
        });
        let b = speech_binding();
        let request = speech_request(&b);
        let error = provider(CandidateRouteId::ElevenFlashV25Tts, address, limits())
            .speak(&request, owner(&b), future::pending())
            .await
            .unwrap_err();
        assert_eq!(error.failure_class(), class);
        assert_eq!(error.error_request_id().unwrap().as_str(), "error_private");
        let debug = format!("{error:?} {:?}", error.metadata());
        for raw in [SENTINEL, "supplier_private", "error_private", "7.5"] {
            assert!(!debug.contains(raw));
        }
        assert!(!joined(task).await.extra_connection);
    }
}

#[tokio::test]
async fn native_reconciliation_fences_full_source_binding_and_retains_unknown_liability() {
    let (address, task) = fixture(|_| {
        wire(
            200,
            "application/json",
            HEADERS,
            &serde_json::to_vec(&transcript()).unwrap(),
        )
    });
    let b = stt_binding();
    let request = stt_request(&b, &vec![0; 3200]);
    let p = provider(CandidateRouteId::ElevenScribeV2Stt, address, limits())
        .transcribe(&request, owner(&b), future::pending())
        .await
        .unwrap();
    let observation = p.observation();
    let settlement: BudgetMutation<(), (), ()> = BudgetMutation::Unknown;
    let policy = ProviderRetryPolicy {
        attempts_remaining: 2,
        retry: true,
        fallback: true,
    };
    for case in 0..10 {
        let mut current = copy_binding(&b);
        match case {
            1 => current.identity.generation += 1,
            2 => current.semantic_basis.source = ContentDigest([9; 32]),
            3 => {
                current.semantic_basis.rights_revision =
                    RevisionLabel::new(Some("foreign")).unwrap()
            }
            4 => current.semantic_basis.semantic.locale = LocaleTag::parse("en-GB").unwrap(),
            _ => {}
        }
        let checked = CheckedRequest::new(
            copy_binding(&current),
            if case == 7 {
                b"checked"
            } else {
                request.checked().payload()
            },
            if case == 8 {
                RequestUsage::new(
                    0,
                    Duration::from_millis(100),
                    Usage::new(1, UsageUnit::AudioMillisecond),
                )
            } else {
                request.checked().usage()
            },
            if case == 9 {
                RequestLimits::new(
                    2_000_001,
                    0,
                    Duration::from_secs(60),
                    Usage::new(60_000, UsageUnit::AudioMillisecond),
                )
                .unwrap()
            } else {
                request.checked().limits()
            },
            owner(&current),
        )
        .unwrap();
        let attempt = ProviderAttemptIdentity::new(
            b.identity.operation,
            if case == 5 {
                CandidateRouteId::ElevenFlashV25Tts
            } else {
                CandidateRouteId::ElevenScribeV2Stt
            },
            Some(
                ProviderRequestId::new(if case == 6 {
                    "foreign_id"
                } else {
                    "supplier_private"
                })
                .unwrap(),
            ),
        );
        let decision = reconcile_stt_attempt(
            &checked,
            &attempt,
            owner(&current),
            &observation,
            ProviderBillingClass::MissingOrAmbiguous,
            &settlement,
            policy,
        )
        .unwrap();
        assert_eq!(
            decision.result,
            if case == 0 {
                ProviderResultClass::Complete
            } else {
                ProviderResultClass::Incomplete
            }
        );
        assert_eq!(
            decision.liability,
            ProviderLiabilityDisposition::RetainWorstCase
        );
        assert_eq!(decision.next, ProviderNextAction::ReconcileSameOperation);
    }
    let foreign_attempt = ProviderAttemptIdentity::new(
        OperationId::from_bytes(&[9; 16]).unwrap(),
        CandidateRouteId::ElevenScribeV2Stt,
        Some(ProviderRequestId::new("supplier_private").unwrap()),
    );
    assert!(
        reconcile_stt_attempt(
            &CheckedRequest::new(
                copy_binding(&b),
                request.checked().payload(),
                request.checked().usage(),
                request.checked().limits(),
                owner(&b)
            )
            .unwrap(),
            &foreign_attempt,
            owner(&b),
            &observation,
            ProviderBillingClass::MissingOrAmbiguous,
            &settlement,
            policy
        )
        .is_err()
    );
    assert!(!joined(task).await.extra_connection);
    let (address, task) = fixture(|_| wire(200, "audio/mpeg", HEADERS, AUDIO));
    let b = speech_binding();
    let request = speech_request(&b);
    let p = provider(CandidateRouteId::ElevenFlashV25Tts, address, limits())
        .speak(&request, owner(&b), future::pending())
        .await
        .unwrap();
    let observation = p.observation();
    let checked = CheckedRequest::new(
        copy_binding(&b),
        request.checked().payload(),
        request.checked().usage(),
        request.checked().limits(),
        owner(&b),
    )
    .unwrap();
    let attempt = ProviderAttemptIdentity::new(
        b.identity.operation,
        CandidateRouteId::ElevenFlashV25Tts,
        Some(ProviderRequestId::new("supplier_private").unwrap()),
    );
    assert_eq!(
        reconcile_speech_attempt(
            &checked,
            &attempt,
            owner(&b),
            &observation,
            ProviderBillingClass::MissingOrAmbiguous,
            &settlement,
            policy
        )
        .unwrap()
        .result,
        ProviderResultClass::Complete
    );
    assert!(!joined(task).await.extra_connection);
}

#[tokio::test]
async fn factory_diagnostics_and_historical_candidates_keep_scope_and_privacy() {
    assert_eq!(candidate_routes().len(), 2);
    assert!(candidate_for(df_providers::Capability::Stt).is_err());
    assert!(
        registered_speech_provider(
            CandidateRouteId::OpenAiResponsesText,
            "credential",
            limits()
        )
        .is_err()
    );
    assert!(
        registered_speech_provider(
            CandidateRouteId::ElevenScribeV2Stt,
            "bad\ncredential",
            limits()
        )
        .is_err()
    );
    let (address, task) = fixture(|_| {
        wire(
            200,
            "application/json",
            HEADERS,
            &serde_json::to_vec(&transcript()).unwrap(),
        )
    });
    let b = stt_binding();
    let request = stt_request(&b, &vec![0; 3200]);
    let adapter = provider(CandidateRouteId::ElevenScribeV2Stt, address, limits());
    let p = adapter
        .transcribe(&request, owner(&b), future::pending())
        .await
        .unwrap();
    let observation = p.observation();
    let before = format!("{adapter:?} {p:?} {observation:?}");
    let [event, _] = p.complete(owner(&b)).unwrap();
    let SttEventKind::Final(out) = event.kind else {
        panic!("final")
    };
    let after = format!("{out:?} {:?}", out.words());
    for raw in [
        SENTINEL,
        "credential-private-sentinel",
        "supplier_private",
        "transcription_private",
        "trace_private",
        "raw",
    ] {
        assert!(!before.contains(raw));
        assert!(!after.contains(raw));
    }
    assert!(!joined(task).await.extra_connection);
}
