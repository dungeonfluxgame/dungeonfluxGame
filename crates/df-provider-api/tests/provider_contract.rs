#[path = "budget.rs"]
mod budget_authority_contract;
#[path = "execution_mode_contract.rs"]
mod execution_mode_and_owner_contract;
#[path = "modality.rs"]
mod modality_basis_contract;
#[path = "outcome.rs"]
mod outcome_and_commerce_contract;
#[path = "stream.rs"]
mod owned_stream_cleanup_contract;

use std::time::Duration;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use df_ai::admission::{
    CompleteRecord, RecordAdmissionError, RecordEvent, RecordIdentity, RecordInputError,
    RecordLimits, RecordingPublisher, admit_complete_record,
};
use df_commerce::{
    SpendConsent, SpendCounter, SpendOperationObservation, SpendOperationStatus, SpendRefusal,
    SpendRequest, SpendSnapshot, propose_spend,
};
use df_model::checkpoint::{
    AssetKind, AssetReference, AssetRequestKey, AudienceScope, Basis, ContentDigest, ExecutionMode,
    JobId, RecordId,
};
use df_provider_api::{
    BudgetAdmission, BudgetMutation, BudgetStore, ImageEventKind, ImageSemantics,
    ModalityRequestError, ProviderCloseReason, ProviderEvent, ProviderFailureClass,
    ProviderProgress, ProviderRequestId, ProviderStreamLease, ProviderStreamOwner,
    RecordingArtifact, RecordingError, RecordingManifest, RecordingPage, RecordingPageError,
    RecordingStore, RequestBasis, RequestBinding, RequestIdentity, RequestLimits, RequestModality,
    RequestOwnerState, RequestUsage, SoundSemantics, SttSemantics, TextSemantics, TtsSemantics,
    VideoSemantics,
};
use df_types::{
    Currency, LocaleTag, Money, OperationId, RecoveryEpoch, RevisionLabel, RunId, SessionId,
    SessionRevision, Usage, UsageUnit,
};

fn label(value: &str) -> RevisionLabel {
    RevisionLabel::new(Some(value)).expect("fixture label")
}

fn identity() -> RequestIdentity {
    RequestIdentity {
        basis: Basis {
            session: SessionId::from_bytes(&[1; 16]).expect("session"),
            run: RunId::from_bytes(&[2; 16]).expect("run"),
            revision: SessionRevision::new(RecoveryEpoch::new(3).expect("epoch"), 4),
        },
        job: JobId::from_bytes(&[5; 16]).expect("job"),
        operation: OperationId::from_bytes(&[6; 16]).expect("operation"),
        generation: 7,
    }
}

fn binding<S>(modality: RequestModality, semantic: S) -> RequestBinding<RequestBasis<S>> {
    RequestBinding {
        identity: identity(),
        semantic_basis: RequestBasis {
            modality,
            source: ContentDigest([8; 32]),
            rights_revision: label("rights-r1"),
            output_format_revision: label("format-r1"),
            semantic,
        },
        mode: ExecutionMode::PreparedOnly,
        deadline: Duration::from_secs(2),
    }
}

fn same_binding<S: Clone>(
    original: &RequestBinding<RequestBasis<S>>,
) -> RequestBinding<RequestBasis<S>> {
    RequestBinding {
        identity: original.identity,
        semantic_basis: original.semantic_basis.clone(),
        mode: original.mode,
        deadline: original.deadline,
    }
}

fn limits(unit: UsageUnit) -> RequestLimits {
    RequestLimits::new(128, 20, Duration::from_secs(1), Usage::new(10, unit))
        .expect("positive limits")
}

fn usage(unit: UsageUnit) -> RequestUsage {
    RequestUsage::new(1, Duration::from_millis(10), Usage::new(1, unit))
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FixtureReferenceFrame {
    sequence: u32,
}

fn assert_envelope<S: Eq>(
    actual: &df_provider_api::CheckedModalityRequest<S>,
    expected: &RequestBinding<RequestBasis<S>>,
    payload: &[u8],
    work: RequestUsage,
) {
    assert_eq!(actual.binding().identity, expected.identity);
    assert_eq!(actual.binding().mode, expected.mode);
    assert_eq!(actual.binding().deadline, expected.deadline);
    assert_eq!(actual.binding().semantic_basis, expected.semantic_basis);
    assert_eq!(actual.payload(), payload);
    assert_eq!(actual.usage(), work);
}

#[test]
fn six_request_families_keep_typed_semantics_inside_the_existing_checked_envelope() {
    let locale = LocaleTag::parse("en-US").expect("locale");

    let text = binding(
        RequestModality::Text,
        TextSemantics {
            locale: locale.clone(),
            context: String::from("bounded caller context"),
            output_schema: String::from("schema-v1"),
        },
    );
    let text_request = df_provider_api::TextRequest::new(
        same_binding(&text),
        b"private-prompt-secret",
        usage(UsageUnit::Token),
        limits(UsageUnit::Token),
        RequestOwnerState {
            current: Some(&text),
            elapsed: Duration::ZERO,
            cancelled: false,
        },
    )
    .expect("text request");
    assert_envelope(
        text_request.checked(),
        &text,
        b"private-prompt-secret",
        usage(UsageUnit::Token),
    );
    assert_eq!(
        text_request
            .checked()
            .binding()
            .semantic_basis
            .semantic
            .output_schema,
        "schema-v1"
    );
    let debug = format!("{:?}", text_request.checked());
    assert!(!debug.contains("private-prompt-secret"));
    assert!(!debug.contains("bounded caller context"));

    let stt = binding(
        RequestModality::Stt,
        SttSemantics {
            locale: locale.clone(),
            audio_format: String::from("pcm-s16le"),
            transcript_profile: String::from("final-only"),
            keyterms: vec![String::from("Marek")],
        },
    );
    let _stt_request = df_provider_api::SttRequest::new(
        same_binding(&stt),
        b"audio",
        usage(UsageUnit::AudioMillisecond),
        limits(UsageUnit::AudioMillisecond),
        RequestOwnerState {
            current: Some(&stt),
            elapsed: Duration::ZERO,
            cancelled: false,
        },
    )
    .expect("stt request");
    assert_envelope(
        _stt_request.checked(),
        &stt,
        b"audio",
        usage(UsageUnit::AudioMillisecond),
    );
    assert_eq!(
        _stt_request
            .checked()
            .binding()
            .semantic_basis
            .semantic
            .locale
            .as_str(),
        "en-us"
    );
    assert_eq!(
        _stt_request
            .checked()
            .binding()
            .semantic_basis
            .semantic
            .keyterms,
        ["Marek"]
    );

    let tts = binding(
        RequestModality::Tts,
        TtsSemantics {
            locale,
            text: String::from("validated text"),
            voice: String::from("voice-r2"),
            output_format: String::from("encoded"),
            pcm_format: String::from("s16le"),
            timebase: String::from("48000hz"),
        },
    );
    let _tts_request = df_provider_api::TtsRequest::new(
        same_binding(&tts),
        b"text",
        usage(UsageUnit::Byte),
        limits(UsageUnit::Byte),
        RequestOwnerState {
            current: Some(&tts),
            elapsed: Duration::ZERO,
            cancelled: false,
        },
    )
    .expect("tts request");
    assert_envelope(
        _tts_request.checked(),
        &tts,
        b"text",
        usage(UsageUnit::Byte),
    );
    assert_eq!(
        _tts_request
            .checked()
            .binding()
            .semantic_basis
            .semantic
            .timebase,
        "48000hz"
    );

    let image = binding(
        RequestModality::Image,
        ImageSemantics {
            locale: None,
            prompt: String::from("bounded prompt"),
            output_profile: String::from("image-profile"),
            references: vec![AssetReference {
                key: label("reference-key-v2"),
                digest: ContentDigest([19; 32]),
                byte_length: 42,
                kind: AssetKind::Image,
            }],
        },
    );
    let _image_request = df_provider_api::ImageRequest::new(
        same_binding(&image),
        b"image",
        usage(UsageUnit::Image),
        limits(UsageUnit::Image),
        RequestOwnerState {
            current: Some(&image),
            elapsed: Duration::ZERO,
            cancelled: false,
        },
    )
    .expect("image request");
    assert_envelope(
        _image_request.checked(),
        &image,
        b"image",
        usage(UsageUnit::Image),
    );
    assert_eq!(
        _image_request
            .checked()
            .binding()
            .semantic_basis
            .semantic
            .references[0]
            .digest,
        ContentDigest([19; 32])
    );

    let video = binding(
        RequestModality::Video,
        VideoSemantics {
            locale: None,
            prompt: String::from("bounded prompt"),
            output_profile: String::from("video-profile"),
            reference_frames: vec![
                FixtureReferenceFrame { sequence: 4 },
                FixtureReferenceFrame { sequence: 9 },
            ],
        },
    );
    let video_request = df_provider_api::VideoRequest::new(
        same_binding(&video),
        b"video",
        usage(UsageUnit::VideoMillisecond),
        limits(UsageUnit::VideoMillisecond),
        RequestOwnerState {
            current: Some(&video),
            elapsed: Duration::ZERO,
            cancelled: false,
        },
    )
    .expect("video request");
    assert_envelope(
        video_request.checked(),
        &video,
        b"video",
        usage(UsageUnit::VideoMillisecond),
    );
    assert_eq!(
        video_request
            .checked()
            .binding()
            .semantic_basis
            .semantic
            .reference_frames,
        vec![
            FixtureReferenceFrame { sequence: 4 },
            FixtureReferenceFrame { sequence: 9 },
        ]
    );

    let sound = binding(
        RequestModality::Sound,
        SoundSemantics {
            locale: None,
            intent: String::from("ambience"),
            output_profile: String::from("loop-profile"),
            loop_metadata: String::from("loop-4-bars"),
        },
    );
    let _sound_request = df_provider_api::SoundRequest::new(
        same_binding(&sound),
        b"sound",
        usage(UsageUnit::Byte),
        limits(UsageUnit::Byte),
        RequestOwnerState {
            current: Some(&sound),
            elapsed: Duration::ZERO,
            cancelled: false,
        },
    )
    .expect("sound request");
    assert_envelope(
        _sound_request.checked(),
        &sound,
        b"sound",
        usage(UsageUnit::Byte),
    );

    let wrong = binding(
        RequestModality::Video,
        ImageSemantics {
            locale: None,
            prompt: String::new(),
            output_profile: String::new(),
            references: Vec::new(),
        },
    );
    let error = df_provider_api::ImageRequest::new(
        same_binding(&wrong),
        b"x",
        usage(UsageUnit::Image),
        limits(UsageUnit::Image),
        RequestOwnerState {
            current: Some(&wrong),
            elapsed: Duration::ZERO,
            cancelled: false,
        },
    );
    let error = match error {
        Err(error) => error,
        Ok(_) => panic!("typed family must reject another modality"),
    };
    assert_eq!(error, ModalityRequestError::WrongModality);
}

#[test]
fn events_carry_provider_id_usage_and_request_completion_identity() {
    let identity = identity();
    fn observed<Kind>(
        event: ProviderEvent<String, Kind>,
        expected_id: &str,
        expected_usage: Usage,
    ) -> Kind {
        assert_eq!(event.provider_request_id.0, expected_id);
        assert!(!format!("{:?}", event.provider_request_id).contains(expected_id));
        assert_eq!(event.usage, expected_usage);
        event.kind
    }

    let image: df_provider_api::ImageEvent<String, String> = ProviderEvent {
        provider_request_id: ProviderRequestId(String::from("adapter-owned-id")),
        usage: Usage::new(3, UsageUnit::Image),
        kind: ImageEventKind::Completed {
            identity,
            output: String::from("candidate"),
        },
    };
    match observed(image, "adapter-owned-id", Usage::new(3, UsageUnit::Image)) {
        ImageEventKind::Completed {
            identity: actual,
            output,
        } => {
            assert_eq!(actual, identity);
            assert_eq!(output, "candidate");
        }
        _ => panic!("expected terminal completion"),
    }
    let failure: df_provider_api::ImageEvent<String, String> = ProviderEvent {
        provider_request_id: ProviderRequestId(String::from("image-failure")),
        usage: Usage::new(4, UsageUnit::Image),
        kind: ImageEventKind::Failed(ProviderFailureClass::Unknown),
    };
    assert!(matches!(
        observed(failure, "image-failure", Usage::new(4, UsageUnit::Image)),
        ImageEventKind::Failed(ProviderFailureClass::Unknown)
    ));

    let text: df_provider_api::TextEvent<String, String> = ProviderEvent {
        provider_request_id: ProviderRequestId(String::from("text-id")),
        usage: Usage::new(2, UsageUnit::Token),
        kind: df_provider_api::TextEventKind::Completed { identity },
    };
    match observed(text, "text-id", Usage::new(2, UsageUnit::Token)) {
        df_provider_api::TextEventKind::Completed { identity: actual } => {
            assert_eq!(actual, identity);
        }
        _ => panic!("expected text completion"),
    }
    let text_candidate: df_provider_api::TextEvent<String, String> = ProviderEvent {
        provider_request_id: ProviderRequestId(String::from("text-candidate")),
        usage: Usage::new(1, UsageUnit::Token),
        kind: df_provider_api::TextEventKind::Candidate(String::from("candidate")),
    };
    match observed(
        text_candidate,
        "text-candidate",
        Usage::new(1, UsageUnit::Token),
    ) {
        df_provider_api::TextEventKind::Candidate(candidate) => assert_eq!(candidate, "candidate"),
        _ => panic!("expected text candidate"),
    }
    let text_failure: df_provider_api::TextEvent<String, String> = ProviderEvent {
        provider_request_id: ProviderRequestId(String::from("text-failure")),
        usage: Usage::new(3, UsageUnit::Token),
        kind: df_provider_api::TextEventKind::Failed(ProviderFailureClass::Capacity),
    };
    assert!(matches!(
        observed(
            text_failure,
            "text-failure",
            Usage::new(3, UsageUnit::Token)
        ),
        df_provider_api::TextEventKind::Failed(ProviderFailureClass::Capacity)
    ));

    let stt: df_provider_api::SttEvent<String, String> = ProviderEvent {
        provider_request_id: ProviderRequestId(String::from("stt-id")),
        usage: Usage::new(1, UsageUnit::AudioMillisecond),
        kind: df_provider_api::SttEventKind::Completed { identity },
    };
    match observed(stt, "stt-id", Usage::new(1, UsageUnit::AudioMillisecond)) {
        df_provider_api::SttEventKind::Completed { identity: actual } => {
            assert_eq!(actual, identity);
        }
        _ => panic!("expected STT completion"),
    }
    let partial: df_provider_api::SttEvent<String, String> = ProviderEvent {
        provider_request_id: ProviderRequestId(String::from("stt-partial")),
        usage: Usage::new(2, UsageUnit::AudioMillisecond),
        kind: df_provider_api::SttEventKind::Partial(String::from("draft")),
    };
    match observed(
        partial,
        "stt-partial",
        Usage::new(2, UsageUnit::AudioMillisecond),
    ) {
        df_provider_api::SttEventKind::Partial(draft) => assert_eq!(draft, "draft"),
        _ => panic!("expected partial transcript"),
    }
    let final_transcript: df_provider_api::SttEvent<String, String> = ProviderEvent {
        provider_request_id: ProviderRequestId(String::from("stt-final")),
        usage: Usage::new(3, UsageUnit::AudioMillisecond),
        kind: df_provider_api::SttEventKind::Final(String::from("final")),
    };
    match observed(
        final_transcript,
        "stt-final",
        Usage::new(3, UsageUnit::AudioMillisecond),
    ) {
        df_provider_api::SttEventKind::Final(transcript) => assert_eq!(transcript, "final"),
        _ => panic!("expected final transcript"),
    }
    let stt_failure: df_provider_api::SttEvent<String, String> = ProviderEvent {
        provider_request_id: ProviderRequestId(String::from("stt-failure")),
        usage: Usage::new(4, UsageUnit::AudioMillisecond),
        kind: df_provider_api::SttEventKind::Failed(ProviderFailureClass::Unavailable),
    };
    assert!(matches!(
        observed(
            stt_failure,
            "stt-failure",
            Usage::new(4, UsageUnit::AudioMillisecond)
        ),
        df_provider_api::SttEventKind::Failed(ProviderFailureClass::Unavailable)
    ));

    let tts: df_provider_api::TtsEvent<String, Vec<u8>> = ProviderEvent {
        provider_request_id: ProviderRequestId(String::from("tts-id")),
        usage: Usage::new(5, UsageUnit::Byte),
        kind: df_provider_api::TtsEventKind::Completed { identity },
    };
    match observed(tts, "tts-id", Usage::new(5, UsageUnit::Byte)) {
        df_provider_api::TtsEventKind::Completed { identity: actual } => {
            assert_eq!(actual, identity);
        }
        _ => panic!("expected TTS completion"),
    }
    let tts_chunk: df_provider_api::TtsEvent<String, Vec<u8>> = ProviderEvent {
        provider_request_id: ProviderRequestId(String::from("tts-chunk")),
        usage: Usage::new(6, UsageUnit::Byte),
        kind: df_provider_api::TtsEventKind::Chunk {
            sequence: 2,
            chunk: vec![3_u8],
        },
    };
    match observed(tts_chunk, "tts-chunk", Usage::new(6, UsageUnit::Byte)) {
        df_provider_api::TtsEventKind::Chunk { sequence, chunk } => {
            assert_eq!(sequence, 2);
            assert_eq!(chunk, vec![3_u8]);
        }
        _ => panic!("expected ordered TTS chunk"),
    }
    let tts_failure: df_provider_api::TtsEvent<String, Vec<u8>> = ProviderEvent {
        provider_request_id: ProviderRequestId(String::from("tts-failure")),
        usage: Usage::new(7, UsageUnit::Byte),
        kind: df_provider_api::TtsEventKind::Failed(ProviderFailureClass::Deadline),
    };
    assert!(matches!(
        observed(tts_failure, "tts-failure", Usage::new(7, UsageUnit::Byte)),
        df_provider_api::TtsEventKind::Failed(ProviderFailureClass::Deadline)
    ));

    let video: df_provider_api::VideoEvent<String, String, String, String> = ProviderEvent {
        provider_request_id: ProviderRequestId(String::from("video-id")),
        usage: Usage::new(8, UsageUnit::VideoMillisecond),
        kind: df_provider_api::VideoEventKind::Completed {
            identity,
            output: String::from("video-candidate"),
        },
    };
    match observed(
        video,
        "video-id",
        Usage::new(8, UsageUnit::VideoMillisecond),
    ) {
        df_provider_api::VideoEventKind::Completed {
            identity: actual,
            output,
        } => {
            assert_eq!(actual, identity);
            assert_eq!(output, "video-candidate");
        }
        _ => panic!("expected video completion"),
    }
    let video_accepted: df_provider_api::VideoEvent<String, String, String, String> =
        ProviderEvent {
            provider_request_id: ProviderRequestId(String::from("video-accepted")),
            usage: Usage::new(9, UsageUnit::VideoMillisecond),
            kind: df_provider_api::VideoEventKind::Accepted {
                handle: String::from("opaque-handle"),
            },
        };
    match observed(
        video_accepted,
        "video-accepted",
        Usage::new(9, UsageUnit::VideoMillisecond),
    ) {
        df_provider_api::VideoEventKind::Accepted { handle } => {
            assert_eq!(handle, "opaque-handle");
        }
        _ => panic!("expected video acceptance"),
    }
    let video_status: df_provider_api::VideoEvent<String, String, String, String> = ProviderEvent {
        provider_request_id: ProviderRequestId(String::from("video-status")),
        usage: Usage::new(10, UsageUnit::VideoMillisecond),
        kind: df_provider_api::VideoEventKind::Status(String::from("processing")),
    };
    match observed(
        video_status,
        "video-status",
        Usage::new(10, UsageUnit::VideoMillisecond),
    ) {
        df_provider_api::VideoEventKind::Status(status) => assert_eq!(status, "processing"),
        _ => panic!("expected video status"),
    }
    let video_failure: df_provider_api::VideoEvent<String, String, String, String> =
        ProviderEvent {
            provider_request_id: ProviderRequestId(String::from("video-failure")),
            usage: Usage::new(11, UsageUnit::VideoMillisecond),
            kind: df_provider_api::VideoEventKind::Failed(ProviderFailureClass::Contract),
        };
    assert!(matches!(
        observed(
            video_failure,
            "video-failure",
            Usage::new(11, UsageUnit::VideoMillisecond)
        ),
        df_provider_api::VideoEventKind::Failed(ProviderFailureClass::Contract)
    ));

    let sound: df_provider_api::SoundEvent<String, String> = ProviderEvent {
        provider_request_id: ProviderRequestId(String::from("sound-id")),
        usage: Usage::new(12, UsageUnit::AudioMillisecond),
        kind: df_provider_api::SoundEventKind::Completed {
            identity,
            output: String::from("sound-candidate"),
        },
    };
    match observed(
        sound,
        "sound-id",
        Usage::new(12, UsageUnit::AudioMillisecond),
    ) {
        df_provider_api::SoundEventKind::Completed {
            identity: actual,
            output,
        } => {
            assert_eq!(actual, identity);
            assert_eq!(output, "sound-candidate");
        }
        _ => panic!("expected sound completion"),
    }
    let sound_candidate: df_provider_api::SoundEvent<String, String> = ProviderEvent {
        provider_request_id: ProviderRequestId(String::from("sound-candidate")),
        usage: Usage::new(13, UsageUnit::AudioMillisecond),
        kind: df_provider_api::SoundEventKind::Candidate(String::from("candidate")),
    };
    match observed(
        sound_candidate,
        "sound-candidate",
        Usage::new(13, UsageUnit::AudioMillisecond),
    ) {
        df_provider_api::SoundEventKind::Candidate(output) => assert_eq!(output, "candidate"),
        _ => panic!("expected sound candidate"),
    }
    let sound_failure: df_provider_api::SoundEvent<String, String> = ProviderEvent {
        provider_request_id: ProviderRequestId(String::from("sound-failure")),
        usage: Usage::new(14, UsageUnit::AudioMillisecond),
        kind: df_provider_api::SoundEventKind::Failed(ProviderFailureClass::Cancelled),
    };
    assert!(matches!(
        observed(
            sound_failure,
            "sound-failure",
            Usage::new(14, UsageUnit::AudioMillisecond)
        ),
        df_provider_api::SoundEventKind::Failed(ProviderFailureClass::Cancelled)
    ));
    assert_eq!(
        ProviderProgress::new(100)
            .expect("bounded progress")
            .percent(),
        100
    );
    assert!(df_provider_api::ProviderProgress::new(101).is_err());
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum QuoteRefusal {
    StaleQuote,
    Commerce(SpendRefusal),
    DispatchUnavailable,
}

/// Finite test consumer: native quote qualification is represented by one
/// supplied current revision; only df-commerce computes spend changes.
struct QuoteFixture {
    current_quote: u64,
    snapshot: SpendSnapshot,
    reserved: Option<OperationId>,
    dispatch_calls: usize,
}

impl BudgetStore for QuoteFixture {
    type Scope = u64;
    type Grant = SpendConsent;
    type Quote = u64;
    type Reservation = OperationId;
    type DispatchClaim = ();
    type DispatchPermit = ();
    type Settlement = ();
    type SettlementReceipt = ();
    type LedgerView = SpendSnapshot;
    type Refusal = QuoteRefusal;
    type Failure = ();

    fn inspect(&mut self, _scope: &u64) -> Result<SpendSnapshot, ()> {
        Ok(self.snapshot)
    }

    fn reserve(
        &mut self,
        admission: BudgetAdmission<'_, u64, SpendConsent, u64>,
    ) -> BudgetMutation<OperationId, QuoteRefusal, ()> {
        if *admission.quote != self.current_quote {
            return BudgetMutation::Refused(QuoteRefusal::StaleQuote);
        }
        let proposed = match propose_spend(
            &self.snapshot,
            *admission.grant,
            SpendRequest {
                operation: admission.operation,
                expected_revision: self.snapshot.revision,
                expected_consent_revision: admission.grant.revision,
                maximum_supplier_liability: admission.maximum_supplier_liability,
            },
            SpendOperationObservation {
                operation: admission.operation,
                status: SpendOperationStatus::Unseen,
            },
        ) {
            Ok(proposed) => proposed,
            Err(refusal) => return BudgetMutation::Refused(QuoteRefusal::Commerce(refusal)),
        };
        self.snapshot = proposed.next;
        self.reserved = Some(admission.operation);
        BudgetMutation::Committed {
            result: admission.operation,
            replayed: false,
        }
    }

    fn claim_dispatch(
        &mut self,
        _reservation: &OperationId,
        _claim: &(),
    ) -> BudgetMutation<(), QuoteRefusal, ()> {
        self.dispatch_calls += 1;
        BudgetMutation::Refused(QuoteRefusal::DispatchUnavailable)
    }

    fn settle(
        &mut self,
        _reservation: &OperationId,
        _settlement: &(),
    ) -> BudgetMutation<(), QuoteRefusal, ()> {
        BudgetMutation::Refused(QuoteRefusal::DispatchUnavailable)
    }

    fn reconcile(&mut self, _reservation: &OperationId) -> BudgetMutation<(), QuoteRefusal, ()> {
        BudgetMutation::Refused(QuoteRefusal::DispatchUnavailable)
    }
}

fn quoted_admission<'a>(
    scope: &'a u64,
    grant: &'a SpendConsent,
    quote: &'a u64,
    original: RequestIdentity,
    maximum: Money,
) -> BudgetAdmission<'a, u64, SpendConsent, u64> {
    BudgetAdmission {
        scope,
        grant,
        quote,
        job: original.job,
        operation: original.operation,
        mode: ExecutionMode::Live,
        maximum_usage: Usage::new(1, UsageUnit::Token),
        maximum_supplier_liability: maximum,
    }
}

#[test]
fn stale_quote_refuses_the_original_operation_without_reserving_or_dispatching() {
    let original = identity();
    let currency = Currency::parse("USD").expect("currency");
    let maximum = Money::new(currency, 80);
    let grant = SpendConsent {
        revision: 7,
        maximum,
    };
    let initial = SpendSnapshot {
        currency,
        revision: 9,
        counters: [SpendCounter {
            used: Money::new(currency, 0),
            limit: Money::new(currency, 100),
        }; 6],
    };
    let current_quote = 12;
    let stale_quote = 11;
    let scope = 1;
    let mut store = QuoteFixture {
        current_quote,
        snapshot: initial,
        reserved: None,
        dispatch_calls: 0,
    };

    assert!(matches!(
        store.reserve(quoted_admission(
            &scope,
            &grant,
            &stale_quote,
            original,
            maximum
        )),
        BudgetMutation::Refused(QuoteRefusal::StaleQuote)
    ));
    assert_eq!(store.inspect(&scope), Ok(initial));
    assert_eq!(store.reserved, None);
    assert_eq!(store.dispatch_calls, 0);

    match store.reserve(quoted_admission(
        &scope,
        &grant,
        &current_quote,
        original,
        maximum,
    )) {
        BudgetMutation::Committed {
            result,
            replayed: false,
        } => assert_eq!(result, original.operation),
        _ => panic!("current quote must admit the unchanged original operation"),
    }
    assert_eq!(store.reserved, Some(original.operation));
    assert_eq!(store.dispatch_calls, 0);
    assert!(
        store
            .inspect(&scope)
            .expect("current quote exposure")
            .counters
            .iter()
            .all(|row| row.used == maximum)
    );
}

struct FakeStreamOwner {
    closes: Rc<Cell<usize>>,
    reasons: Rc<RefCell<Vec<ProviderCloseReason>>>,
    fail: bool,
}

impl ProviderStreamOwner for FakeStreamOwner {
    type CloseError = &'static str;

    fn close(&mut self, reason: ProviderCloseReason) -> Result<(), Self::CloseError> {
        self.closes.set(self.closes.get() + 1);
        self.reasons.borrow_mut().push(reason);
        if self.fail {
            Err("cleanup failed")
        } else {
            Ok(())
        }
    }
}

#[test]
fn provider_stream_lease_closes_once_and_surfaces_explicit_and_drop_failures() {
    let closes = Rc::new(Cell::new(0));
    let reasons = Rc::new(RefCell::new(Vec::new()));
    let observed = Rc::new(RefCell::new(None));
    let mut lease = ProviderStreamLease::new(
        FakeStreamOwner {
            closes: closes.clone(),
            reasons: reasons.clone(),
            fail: true,
        },
        {
            let observed = observed.clone();
            move |result| *observed.borrow_mut() = Some(result)
        },
    );
    assert_eq!(
        lease.close(ProviderCloseReason::Cancelled),
        Some(Err("cleanup failed"))
    );
    assert_eq!(lease.close(ProviderCloseReason::Deadline), None);
    drop(lease);
    assert_eq!(closes.get(), 1);
    assert_eq!(&*reasons.borrow(), &[ProviderCloseReason::Cancelled]);
    assert!(observed.borrow().is_none());

    let closes = Rc::new(Cell::new(0));
    let reasons = Rc::new(RefCell::new(Vec::new()));
    let observed = Rc::new(RefCell::new(None));
    let lease = ProviderStreamLease::new(
        FakeStreamOwner {
            closes: closes.clone(),
            reasons: reasons.clone(),
            fail: true,
        },
        {
            let observed = observed.clone();
            move |result| *observed.borrow_mut() = Some(result)
        },
    );
    drop(lease);
    assert_eq!(closes.get(), 1);
    assert_eq!(&*reasons.borrow(), &[ProviderCloseReason::OwnerDropped]);
    assert_eq!(*observed.borrow(), Some(Err("cleanup failed")));
}

fn asset_key() -> AssetRequestKey {
    AssetRequestKey {
        schema: 3,
        source: ContentDigest([8; 32]),
        moment: RecordId::from_bytes(&[9; 16]).expect("moment id"),
        identity: label("identity-v2"),
        style: label("style-v3"),
        voice: Some(label("voice-v4")),
        provider: label("provider-profile-v5"),
        model: label("model-profile-v6"),
        format: label("format-v7"),
        references: Vec::new(),
        audience: AudienceScope::Shared,
        parameters: label("parameters-v8"),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StoreFault {
    Denied,
    Unavailable,
    Unknown,
}

struct Store {
    fault: Option<StoreFault>,
    observed: Option<StorageObservation>,
    page: Option<RecordingPage>,
}

struct StorageObservation {
    key: (AssetRequestKey, LocaleTag),
    contract: String,
    identity: RequestIdentity,
    mode: ExecutionMode,
    usage: Usage,
    bytes: Vec<u8>,
    digest: [u8; 32],
}

impl Store {
    fn new() -> Self {
        Self {
            fault: None,
            observed: None,
            page: None,
        }
    }

    fn error(&self) -> Option<RecordingError<StoreFault, StoreFault, StoreFault>> {
        self.fault.map(|fault| match fault {
            StoreFault::Denied => RecordingError::Denied(fault),
            StoreFault::Unavailable => RecordingError::Unavailable(fault),
            StoreFault::Unknown => RecordingError::Unknown(fault),
        })
    }
}

impl RecordingStore<(AssetRequestKey, LocaleTag), String> for Store {
    type Artifact = usize;
    type Entry = (AssetRequestKey, LocaleTag);
    type Denied = StoreFault;
    type Unavailable = StoreFault;
    type Unknown = StoreFault;

    fn lookup(
        &mut self,
        _manifest: &RecordingManifest<(AssetRequestKey, LocaleTag), String>,
    ) -> Result<Option<Self::Entry>, RecordingError<StoreFault, StoreFault, StoreFault>> {
        if let Some(error) = self.error() {
            return Err(error);
        }
        Ok(None)
    }
    fn publish_complete(
        &mut self,
        manifest: RecordingManifest<(AssetRequestKey, LocaleTag), String>,
        bytes: Vec<u8>,
        sha256: [u8; 32],
    ) -> Result<RecordingArtifact<usize>, RecordingError<StoreFault, StoreFault, StoreFault>> {
        if let Some(error) = self.error() {
            return Err(error);
        }
        self.observed = Some(StorageObservation {
            key: manifest.key,
            contract: manifest.contract,
            identity: manifest.identity,
            mode: manifest.mode,
            usage: manifest.usage,
            bytes: bytes.clone(),
            digest: sha256,
        });
        Ok(RecordingArtifact {
            artifact: bytes.len(),
            usage: manifest.usage,
        })
    }
    fn list(
        &mut self,
        page: RecordingPage,
    ) -> Result<Vec<Self::Entry>, RecordingError<StoreFault, StoreFault, StoreFault>> {
        assert!(page.limit() > 0);
        self.page = Some(page);
        if let Some(error) = self.error() {
            return Err(error);
        }
        Ok(Vec::new())
    }
}

struct Publisher {
    store: Store,
    manifest: Option<RecordingManifest<(AssetRequestKey, LocaleTag), String>>,
}

impl RecordingPublisher for Publisher {
    type Key = (AssetRequestKey, LocaleTag);
    type Basis = String;
    type Artifact = usize;
    type Error = RecordingError<StoreFault, StoreFault, StoreFault>;

    fn publish_complete(
        &mut self,
        record: CompleteRecord<Self::Key, Self::Basis>,
    ) -> Result<Self::Artifact, Self::Error> {
        let (identity, bytes, sha256) = record.into_parts();
        let manifest = self.manifest.take().expect("one terminal publication");
        assert_eq!(identity.operation, manifest.identity.operation);
        assert_eq!(identity.key, manifest.key);
        assert_eq!(identity.basis, manifest.contract);
        let artifact = self.store.publish_complete(manifest, bytes, sha256)?;
        Ok(artifact.artifact)
    }
}

#[test]
fn recording_publication_uses_actual_complete_record_hook_and_preserves_full_manifest() {
    let identity = identity();
    let locale = LocaleTag::parse("en-US").expect("locale");
    let recording_binding = RequestBinding {
        identity,
        semantic_basis: asset_key(),
        mode: ExecutionMode::PreparedOnly,
        deadline: Duration::from_secs(2),
    }
    .with_recording_locale(locale);
    let key = recording_binding.semantic_basis.clone();
    let manifest = RecordingManifest {
        key: key.clone(),
        contract: String::from("contract-v9"),
        identity: recording_binding.identity,
        mode: recording_binding.mode,
        usage: Usage::new(4, UsageUnit::Byte),
    };
    let debug = format!("{manifest:?}");
    assert!(!debug.contains("provider-profile-v5"));
    assert!(!debug.contains("contract-v9"));
    let store = Store::new();
    let publisher = Publisher {
        store,
        manifest: Some(manifest),
    };
    // The verifier checks final declaration, digest, length, and iterator EOF before
    // the store adapter receives a CompleteRecord.
    let bytes = b"complete output".to_vec();
    let digest = [
        0x84, 0x68, 0xc5, 0x03, 0xac, 0x8d, 0xae, 0x01, 0x1a, 0xc2, 0x6d, 0x98, 0x73, 0xb7, 0x19,
        0x8d, 0x2a, 0x64, 0xb8, 0xf1, 0xa3, 0xb3, 0x8c, 0x0d, 0x5d, 0x85, 0x06, 0x48, 0x27, 0x10,
        0xbf, 0x03,
    ];
    let expected = RecordIdentity {
        key: key.clone(),
        basis: String::from("contract-v9"),
        operation: identity.operation,
    };
    let events = vec![
        Ok(RecordEvent::Chunk(bytes.clone())),
        Ok(RecordEvent::Complete {
            identity: RecordIdentity {
                key: key.clone(),
                basis: String::from("contract-v9"),
                operation: identity.operation,
            },
            byte_length: bytes.len() as u64,
            sha256: digest,
        }),
    ];
    // The storage adapter's manifest is owned and consumed on publication.
    let mut publisher = publisher;
    let admitted = admit_complete_record(
        &mut publisher,
        expected,
        RecordLimits::new(128, 64, 4).expect("limits"),
        events,
    );
    assert_eq!(admitted, Ok(bytes.len()));
    let observed = publisher.store.observed.expect("published complete record");
    assert_eq!(observed.key, key);
    assert_eq!(observed.contract, "contract-v9");
    assert_eq!(observed.identity, identity);
    assert_eq!(observed.mode, ExecutionMode::PreparedOnly);
    assert_eq!(observed.usage, Usage::new(4, UsageUnit::Byte));
    assert_eq!(observed.bytes, bytes);
    assert_eq!(observed.digest, digest);
}

fn test_manifest(
    key: (AssetRequestKey, LocaleTag),
) -> RecordingManifest<(AssetRequestKey, LocaleTag), String> {
    RecordingManifest {
        key,
        contract: String::from("contract-v9"),
        identity: identity(),
        mode: ExecutionMode::PreparedOnly,
        usage: Usage::new(4, UsageUnit::Byte),
    }
}

fn test_publisher(key: (AssetRequestKey, LocaleTag)) -> Publisher {
    Publisher {
        store: Store::new(),
        manifest: Some(test_manifest(key)),
    }
}

fn record_identity(
    key: (AssetRequestKey, LocaleTag),
) -> RecordIdentity<(AssetRequestKey, LocaleTag), String> {
    RecordIdentity {
        key,
        basis: String::from("contract-v9"),
        operation: identity().operation,
    }
}

fn complete_digest() -> [u8; 32] {
    [
        0x84, 0x68, 0xc5, 0x03, 0xac, 0x8d, 0xae, 0x01, 0x1a, 0xc2, 0x6d, 0x98, 0x73, 0xb7, 0x19,
        0x8d, 0x2a, 0x64, 0xb8, 0xf1, 0xa3, 0xb3, 0x8c, 0x0d, 0x5d, 0x85, 0x06, 0x48, 0x27, 0x10,
        0xbf, 0x03,
    ]
}

#[test]
fn complete_record_refuses_nonterminal_or_unverified_streams_before_storage() {
    let key = (asset_key(), LocaleTag::parse("en-US").expect("locale"));
    let bytes = b"complete output".to_vec();
    let digest = complete_digest();

    let mut publisher = test_publisher(key.clone());
    assert!(matches!(
        admit_complete_record(
            &mut publisher,
            record_identity(key.clone()),
            RecordLimits::new(128, 64, 4).expect("limits"),
            vec![Ok(RecordEvent::Chunk(bytes.clone()))],
        ),
        Err(RecordAdmissionError::MissingCompletion)
    ));
    assert!(publisher.store.observed.is_none());

    let mut publisher = test_publisher(key.clone());
    let trailing = vec![
        Ok(RecordEvent::Chunk(bytes.clone())),
        Ok(RecordEvent::Complete {
            identity: record_identity(key.clone()),
            byte_length: bytes.len() as u64,
            sha256: digest,
        }),
        Ok(RecordEvent::Chunk(Vec::new())),
    ];
    assert!(matches!(
        admit_complete_record(
            &mut publisher,
            record_identity(key.clone()),
            RecordLimits::new(128, 64, 4).expect("limits"),
            trailing
        ),
        Err(RecordAdmissionError::TrailingEvent)
    ));
    assert!(publisher.store.observed.is_none());

    let mut publisher = test_publisher(key.clone());
    let cancelled = vec![
        Ok(RecordEvent::Chunk(bytes.clone())),
        Err(RecordInputError::Cancelled),
    ];
    assert!(matches!(
        admit_complete_record(
            &mut publisher,
            record_identity(key.clone()),
            RecordLimits::new(128, 64, 4).expect("limits"),
            cancelled
        ),
        Err(RecordAdmissionError::Input(RecordInputError::Cancelled))
    ));
    assert!(publisher.store.observed.is_none());

    let mut publisher = test_publisher(key.clone());
    let bad_length = vec![
        Ok(RecordEvent::Chunk(bytes.clone())),
        Ok(RecordEvent::Complete {
            identity: record_identity(key.clone()),
            byte_length: bytes.len() as u64 + 1,
            sha256: digest,
        }),
    ];
    assert!(matches!(
        admit_complete_record(
            &mut publisher,
            record_identity(key.clone()),
            RecordLimits::new(128, 64, 4).expect("limits"),
            bad_length
        ),
        Err(RecordAdmissionError::LengthMismatch)
    ));

    let mut publisher = test_publisher(key.clone());
    let bad_digest = vec![
        Ok(RecordEvent::Chunk(bytes.clone())),
        Ok(RecordEvent::Complete {
            identity: record_identity(key.clone()),
            byte_length: bytes.len() as u64,
            sha256: [0; 32],
        }),
    ];
    assert!(matches!(
        admit_complete_record(
            &mut publisher,
            record_identity(key.clone()),
            RecordLimits::new(128, 64, 4).expect("limits"),
            bad_digest
        ),
        Err(RecordAdmissionError::DigestMismatch)
    ));

    let mut publisher = test_publisher(key.clone());
    assert!(matches!(
        admit_complete_record(
            &mut publisher,
            record_identity(key),
            RecordLimits::new(128, 64, 4).expect("limits"),
            vec![Ok(RecordEvent::Chunk(vec![0; 129]))]
        ),
        Err(RecordAdmissionError::LimitExceeded(_))
    ));
    assert!(publisher.store.observed.is_none());
}

#[test]
fn recording_store_errors_and_pages_are_typed_and_bounded() {
    assert_eq!(
        RecordingPage::new(None, 0),
        Err(RecordingPageError::InvalidLimit)
    );
    assert_eq!(
        RecordingPage::new(None, RecordingPage::MAX_LIMIT + 1),
        Err(RecordingPageError::InvalidLimit)
    );
    let page = RecordingPage::new(Some(17), 8).expect("bounded page");
    assert_eq!(page.after(), Some(17));
    let manifest = test_manifest((asset_key(), LocaleTag::parse("en-US").expect("locale")));

    for fault in [
        StoreFault::Denied,
        StoreFault::Unavailable,
        StoreFault::Unknown,
    ] {
        let mut store = Store {
            fault: Some(fault),
            observed: None,
            page: None,
        };
        let lookup = store.lookup(&manifest);
        let list = store.list(page);
        let publish = store.publish_complete(
            test_manifest((asset_key(), LocaleTag::parse("en-US").expect("locale"))),
            b"candidate".to_vec(),
            complete_digest(),
        );
        match (fault, lookup, list) {
            (
                StoreFault::Denied,
                Err(RecordingError::Denied(StoreFault::Denied)),
                Err(RecordingError::Denied(StoreFault::Denied)),
            ) => {}
            (
                StoreFault::Unavailable,
                Err(RecordingError::Unavailable(StoreFault::Unavailable)),
                Err(RecordingError::Unavailable(StoreFault::Unavailable)),
            ) => {}
            (
                StoreFault::Unknown,
                Err(RecordingError::Unknown(StoreFault::Unknown)),
                Err(RecordingError::Unknown(StoreFault::Unknown)),
            ) => {}
            _ => panic!("typed recording fault was changed"),
        }
        match (fault, publish) {
            (StoreFault::Denied, Err(RecordingError::Denied(StoreFault::Denied))) => {}
            (
                StoreFault::Unavailable,
                Err(RecordingError::Unavailable(StoreFault::Unavailable)),
            ) => {}
            (StoreFault::Unknown, Err(RecordingError::Unknown(StoreFault::Unknown))) => {}
            _ => panic!("publication fault was changed"),
        }
        assert_eq!(store.page, Some(page));
        assert!(store.observed.is_none());
    }
}
