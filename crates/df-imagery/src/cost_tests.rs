use super::*;

fn dispatch(byte: u8) -> [u8; 16] {
    [byte; 16]
}

fn route() -> CostRoute {
    CostRoute {
        provider: "openai".to_owned(),
        model: "gpt-image-2.5-flare".to_owned(),
        cause: "scene_plate".to_owned(),
        detail: "test".to_owned(),
        maxima: vec![CostUnits {
            category: "image_output".to_owned(),
            quantity: 1,
        }],
    }
}

#[test]
fn a_dispatch_id_maps_to_one_stable_call_id() {
    let first = DispatchId::new(dispatch(7));
    assert_eq!(first.call_id(), pinned_call_id(&dispatch(7)));
    assert_eq!(pinned_call_id(&dispatch(7)), pinned_call_id(&dispatch(7)));
    assert_ne!(pinned_call_id(&dispatch(7)), pinned_call_id(&dispatch(8)));
}

#[test]
fn a_pinned_call_id_is_prefixed_lowercase_hex_of_the_dispatch_bytes() {
    let id = pinned_call_id(&[
        0x00, 0x0f, 0x10, 0xff, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89, 0x9a, 0xbc, 0xde,
        0xf0,
    ]);
    assert_eq!(id, "img-000f10ffabcdef01234567899abcdef0");
    assert!(id.starts_with(PINNED_CALL_PREFIX));
    assert_eq!(DispatchId::new(dispatch(9)).as_bytes(), &dispatch(9));
}

#[test]
fn usage_evidence_labels_are_distinct_words() {
    assert_eq!(UsageEvidence::Reported.key(), "reported");
    assert_eq!(UsageEvidence::Estimated.key(), "estimated");
    assert_eq!(UsageEvidence::Unknown.key(), "unknown");
}

#[test]
fn usage_revisions_order_like_the_journal_numbers_them() {
    assert!(UsageRevision(0) < UsageRevision(1));
    assert_eq!(UsageRevision(2), UsageRevision(2));
    assert!(UsageRevision(3) > UsageRevision(2));
}

struct RecordingProjection {
    seen: std::sync::Mutex<Vec<String>>,
}

impl ImageCostProjection for RecordingProjection {
    fn start_pinned<'a>(
        &'a self,
        call_id: &'a str,
        route: &'a CostRoute,
    ) -> CostFuture<'a, Result<PinnedStart, CostError>> {
        Box::pin(async move {
            self.seen.lock().expect("recording").push(format!(
                "start {call_id} {}/{}",
                route.provider, route.model
            ));
            Ok(PinnedStart::Opened)
        })
    }

    fn correct<'a>(
        &'a self,
        call_id: &'a str,
        report: &'a UsageReport<'a>,
    ) -> CostFuture<'a, Result<PinnedCorrection, CostError>> {
        Box::pin(async move {
            self.seen.lock().expect("recording").push(format!(
                "correct {call_id} rev={} {} usable={}",
                report.revision.0,
                report.evidence.key(),
                report.usable
            ));
            Ok(PinnedCorrection::Applied)
        })
    }

    fn record_unsent<'a>(
        &'a self,
        call_id: &'a str,
    ) -> CostFuture<'a, Result<PinnedUnsent, CostError>> {
        Box::pin(async move {
            self.seen
                .lock()
                .expect("recording")
                .push(format!("unsent {call_id}"));
            Ok(PinnedUnsent::Recorded)
        })
    }
    fn is_durable<'a>(
        &'a self,
        _call_id: &'a str,
        _revision: Option<UsageRevision>,
    ) -> CostFuture<'a, Result<bool, CostError>> {
        Box::pin(async { Ok(false) })
    }
}

#[tokio::test]
async fn the_projection_contract_is_implementable_and_object_safe() {
    let projection = RecordingProjection {
        seen: std::sync::Mutex::new(Vec::new()),
    };
    let wide: &dyn ImageCostProjection = &projection;
    let call_id = wide.call_id_for(&dispatch(3));
    assert_eq!(
        wide.start_pinned(&call_id, &route()).await,
        Ok(PinnedStart::Opened)
    );
    let report = UsageReport {
        revision: UsageRevision(0),
        evidence: UsageEvidence::Unknown,
        units: &[],
        usable: true,
        at_ms: 1_000,
    };
    assert_eq!(
        wide.correct(&call_id, &report).await,
        Ok(PinnedCorrection::Applied)
    );
    assert_eq!(
        wide.record_unsent(&call_id).await,
        Ok(PinnedUnsent::Recorded)
    );
    let seen = projection.seen.lock().expect("recording");
    assert_eq!(seen.len(), 3);
    assert!(seen[1].contains("rev=0 unknown usable=true"), "{seen:?}");
}

#[test]
fn cost_errors_carry_revisions_but_no_payloads() {
    let stale = CostError::StaleRevision {
        stored: UsageRevision(2),
        got: UsageRevision(1),
    };
    assert_eq!(
        stale,
        CostError::StaleRevision {
            stored: UsageRevision(2),
            got: UsageRevision(1)
        }
    );
    assert_ne!(
        stale,
        CostError::RevisionConflict {
            revision: UsageRevision(1)
        }
    );
    let debug = format!("{stale:?}");
    assert!(debug.contains('2') && debug.contains('1'), "{debug}");
}

#[tokio::test]
async fn a_port_without_an_actual_sealer_never_claims_a_missing_or_durable_receipt() {
    let projection = std::sync::Arc::new(RecordingProjection {
        seen: std::sync::Mutex::new(Vec::new()),
    });
    let call = pinned_call_id(&dispatch(44));
    assert_eq!(
        projection.seal_unsent(&call).await,
        Err(CostError::UnsentSealUnavailable)
    );
    assert!(projection.seen.lock().unwrap().is_empty());
}

#[test]
fn durable_unsent_proof_requires_actual_durability_and_the_exact_expected_dispatch() {
    let expected = dispatch(45);
    let id = pinned_call_id(&expected);
    assert_eq!(
        DurableUnsentReceipt::from_seal(
            PinnedUnsentSeal {
                call_id: id.clone(),
                durability: UnsentDurability::Pending
            },
            &expected
        ),
        Err(CostError::ProjectionFull)
    );
    assert_eq!(
        DurableUnsentReceipt::from_seal(
            PinnedUnsentSeal {
                call_id: pinned_call_id(&dispatch(46)),
                durability: UnsentDurability::Durable
            },
            &expected
        ),
        Err(CostError::UnknownCall)
    );
    let proof = DurableUnsentReceipt::from_seal(
        PinnedUnsentSeal {
            call_id: id.clone(),
            durability: UnsentDurability::Durable,
        },
        &expected,
    )
    .unwrap();
    assert_eq!(proof.call_id(), id);
    assert_eq!(proof.durability(), UnsentDurability::Durable);
}
