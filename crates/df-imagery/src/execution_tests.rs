//! Unit tests for the neutral execution contracts: id parsing, command
//! validation, pure claim and capacity decisions, and the stable book call id.

use super::*;

fn owner_run() -> ImageOwner {
    use df_types::{CampaignId, RunId};
    ImageOwner::Run {
        campaign: CampaignId::from_bytes(&[0x53; 16]).expect("campaign"),
        run: RunId::from_bytes(&[0x51; 16]).expect("run"),
    }
}

fn stage_id(byte: u8) -> ExecutionStageId {
    ExecutionStageId::from_bytes(&[byte; 16]).expect("stage")
}

fn dispatch_id(byte: u8) -> DispatchId {
    DispatchId::from_bytes(&[byte; 16]).expect("dispatch")
}

fn claim_token(byte: u8) -> ClaimToken {
    ClaimToken::from_bytes(&[byte; 16]).expect("claim")
}

fn job_id(byte: u8) -> ImageJobId {
    ImageJobId::from_bytes(&[byte; 16]).expect("job")
}

fn enqueue_command() -> EnqueueCommand {
    EnqueueCommand::new(
        crate::execution::StageScope {
            owner: owner_run(),
            job: job_id(0x11),
            stage: stage_id(0x21),
        },
        crate::execution::StageBasis {
            kind: StageKind::Generation,
            input_revision: 1,
            input_digest: [0x31; 32],
            config_revision: 7,
            config_digest: [0x32; 32],
            parent_dispatch: None,
        },
        crate::execution::ExecutionIdentity {
            asset: [0x41; 16],
            attempt: [0x42; 16],
        },
        CapacityScope::new("live/generation".to_owned()).expect("scope"),
        false,
        1_700_000_000_000,
    )
    .expect("enqueue")
}

#[test]
fn stage_and_dispatch_ids_refuse_zero_and_wrong_lengths() {
    assert_eq!(
        ExecutionStageId::from_bytes(&[0; 16]),
        Err(ExecutionSetupError::ZeroId)
    );
    assert!(ExecutionStageId::from_bytes(&[1; 15]).is_err());
    assert!(DispatchId::from_bytes(&[0; 16]).is_err());
    assert!(ClaimToken::from_bytes(&[0; 16]).is_err());
    assert_eq!(
        ExecutionStageId::from_hex(&"11".repeat(16)),
        Ok(ExecutionStageId::from_bytes(&[0x11; 16]).expect("stage"))
    );
    assert!(ExecutionStageId::from_hex("zz".repeat(16).as_str()).is_err());
}

#[test]
fn kind_state_settlement_and_cause_labels_round_trip() {
    for kind in [
        StageKind::Classification,
        StageKind::Generation,
        StageKind::Edit,
        StageKind::Extraction,
        StageKind::Inspection,
        StageKind::Correction,
        StageKind::LocalTransform,
    ] {
        assert_eq!(StageKind::parse_label(kind.label()), Some(kind));
    }
    assert_eq!(StageKind::parse_label("backdrop"), None);
    for state in [
        StageState::Planned,
        StageState::Admitted,
        StageState::Released,
    ] {
        assert_eq!(StageState::parse_label(state.label()), Some(state));
    }
    for state in [
        DispatchState::Prepared,
        DispatchState::Sent,
        DispatchState::Succeeded,
        DispatchState::Failed,
        DispatchState::Unknown,
        DispatchState::Released,
    ] {
        assert_eq!(DispatchState::parse_label(state.label()), Some(state));
    }
    for settlement in [
        DispatchSettlement::Held,
        DispatchSettlement::Settled,
        DispatchSettlement::UnusableSettled,
        DispatchSettlement::UnknownOutstanding,
        DispatchSettlement::Released,
    ] {
        assert_eq!(
            DispatchSettlement::parse_label(settlement.label()),
            Some(settlement)
        );
    }
    for cause in [
        FailureCause::Refused,
        FailureCause::InvalidOutput,
        FailureCause::TransportLost,
    ] {
        assert_eq!(FailureCause::parse_label(cause.label()), Some(cause));
    }
}

#[test]
fn finance_tuple_needs_all_three_ids_and_mode_splits_local_from_hosted() {
    assert!(FinanceTuple::new([0x61; 16], [0x62; 16], [0x63; 16]).is_ok());
    assert_eq!(
        FinanceTuple::new([0; 16], [0x62; 16], [0x63; 16]),
        Err(ExecutionSetupError::ZeroId)
    );
    assert_eq!(
        FinanceTuple::new([0x61; 16], [0x62; 16], [0; 16]),
        Err(ExecutionSetupError::ZeroId)
    );
    let hosted =
        DispatchMode::Hosted(FinanceTuple::new([0x61; 16], [0x62; 16], [0x63; 16]).expect("tuple"));
    assert!(!hosted.is_local());
    assert!(hosted.finance().is_some());
    assert!(DispatchMode::Local.is_local());
    assert_eq!(DispatchMode::Local.finance(), None);
}

#[test]
fn enqueue_command_refuses_zero_revisions_zero_identity_and_bad_time() {
    let valid = enqueue_command();
    assert_eq!(valid.input_revision, 1);
    let zero_input = EnqueueCommand::new(
        crate::execution::StageScope {
            owner: valid.owner.clone(),
            job: valid.job,
            stage: valid.stage,
        },
        crate::execution::StageBasis {
            kind: valid.kind,
            input_revision: 0,
            input_digest: valid.input_digest,
            config_revision: valid.config_revision,
            config_digest: valid.config_digest,
            parent_dispatch: None,
        },
        crate::execution::ExecutionIdentity {
            asset: valid.planned_publication.expect("media plan").asset,
            attempt: valid.planned_publication.expect("media plan").attempt,
        },
        valid.capacity.clone(),
        valid.local_accounting,
        valid.now_ms,
    );
    assert_eq!(zero_input, Err(ExecutionSetupError::BadRevision));
    let zero_asset = EnqueueCommand::new(
        crate::execution::StageScope {
            owner: valid.owner.clone(),
            job: valid.job,
            stage: valid.stage,
        },
        crate::execution::StageBasis {
            kind: valid.kind,
            input_revision: valid.input_revision,
            input_digest: valid.input_digest,
            config_revision: valid.config_revision,
            config_digest: valid.config_digest,
            parent_dispatch: None,
        },
        crate::execution::ExecutionIdentity {
            asset: [0; 16],
            attempt: valid.planned_publication.expect("media plan").attempt,
        },
        valid.capacity.clone(),
        valid.local_accounting,
        valid.now_ms,
    );
    assert_eq!(zero_asset, Err(ExecutionSetupError::ZeroId));
    let bad_time = EnqueueCommand::new(
        crate::execution::StageScope {
            owner: valid.owner.clone(),
            job: valid.job,
            stage: valid.stage,
        },
        crate::execution::StageBasis {
            kind: valid.kind,
            input_revision: valid.input_revision,
            input_digest: valid.input_digest,
            config_revision: valid.config_revision,
            config_digest: valid.config_digest,
            parent_dispatch: None,
        },
        crate::execution::ExecutionIdentity {
            asset: valid.planned_publication.expect("media plan").asset,
            attempt: valid.planned_publication.expect("media plan").attempt,
        },
        valid.capacity.clone(),
        valid.local_accounting,
        0,
    );
    assert_eq!(bad_time, Err(ExecutionSetupError::BadTime));
}

#[test]
fn claim_command_needs_a_positive_lease_and_a_real_revision() {
    let owner = owner_run();
    assert_eq!(
        ClaimCommand::new(
            owner.clone(),
            job_id(0x11),
            stage_id(0x21),
            7,
            claim_token(0x71),
            0,
            1_700_000_000_000,
        ),
        Err(ExecutionSetupError::BadLease)
    );
    assert_eq!(
        ClaimCommand::new(
            owner.clone(),
            job_id(0x11),
            stage_id(0x21),
            7,
            claim_token(0x71),
            -5_000,
            1_700_000_000_000,
        ),
        Err(ExecutionSetupError::BadLease)
    );
    assert_eq!(
        ClaimCommand::new(
            owner.clone(),
            job_id(0x11),
            stage_id(0x21),
            0,
            claim_token(0x71),
            60_000,
            1_700_000_000_000,
        ),
        Err(ExecutionSetupError::BadRevision)
    );
    let short = ClaimCommand::new(
        owner.clone(),
        job_id(0x11),
        stage_id(0x21),
        7,
        claim_token(0x71),
        1,
        1_700_000_000_000,
    )
    .expect("positive lease fits no fixed ceiling");
    assert_eq!(short.lease_ms, 1);
    assert!(
        ClaimCommand::new(
            owner,
            job_id(0x11),
            stage_id(0x21),
            7,
            claim_token(0x71),
            60_000,
            1_700_000_000_000,
        )
        .is_ok()
    );
}

#[test]
fn lease_expiry_is_strict_and_claim_ownership_needs_token_and_generation() {
    assert!(lease_is_live(1_000, 999));
    assert!(!lease_is_live(1_000, 1_000));
    assert!(!lease_is_live(1_000, 1_001));
    assert!(claim_owns(&[0x71; 16], 3, &[0x71; 16], 3));
    assert!(!claim_owns(&[0x71; 16], 2, &[0x71; 16], 3));
    assert!(!claim_owns(&[0x72; 16], 3, &[0x71; 16], 3));
}

#[test]
fn dispatch_sequences_start_at_zero_and_never_repeat() {
    assert_eq!(next_sequence(None), 0);
    assert_eq!(next_sequence(Some(0)), 1);
    assert_eq!(next_sequence(Some(4)), 5);
    assert_eq!(next_sequence(Some(i32::MAX)), i32::MAX);
}

#[test]
fn capacity_admits_only_a_free_slot_under_a_positive_bound() {
    assert!(capacity_admits(0, 4));
    assert!(capacity_admits(3, 4));
    assert!(!capacity_admits(4, 4));
    assert!(!capacity_admits(5, 4));
    assert!(!capacity_admits(0, 0));
    assert!(!capacity_admits(-1, 4));
}

#[test]
fn only_the_current_dispatch_completes_and_terminal_states_release_capacity() {
    assert!(sender_may_complete(&[0x81; 16], &[0x81; 16]));
    assert!(!sender_may_complete(&[0x81; 16], &[0x82; 16]));
    assert!(completion_releases_capacity(DispatchState::Succeeded));
    assert!(completion_releases_capacity(DispatchState::Failed));
    assert!(completion_releases_capacity(DispatchState::Released));
    assert!(!completion_releases_capacity(DispatchState::Unknown));
    assert!(!completion_releases_capacity(DispatchState::Sent));
    assert!(!completion_releases_capacity(DispatchState::Prepared));
}

#[test]
fn checkpoint_alpha_labels_round_trip() {
    assert_eq!(
        CheckpointAlpha::parse_label(CheckpointAlpha::Opaque.label()),
        Some(CheckpointAlpha::Opaque)
    );
    assert_eq!(
        CheckpointAlpha::parse_label(CheckpointAlpha::Transparent.label()),
        Some(CheckpointAlpha::Transparent)
    );
    assert_eq!(CheckpointAlpha::parse_label("glossy"), None);
}

#[test]
fn book_call_id_is_stable_per_dispatch_and_checkpoint_binds_it() {
    let first = book_call_id_for(&dispatch_id(0x81));
    assert_eq!(first, book_call_id_for(&dispatch_id(0x81)));
    assert_ne!(first, book_call_id_for(&dispatch_id(0x82)));
    assert!(first.starts_with("img-"));
    let checkpoint = OutputCheckpoint::new(
        &dispatch_id(0x81),
        crate::execution::OutputObject {
            storage_key: "staged/0x81/variant".to_owned(),
            object_version: "etag-3".to_owned(),
            byte_len: 48_128,
            sha256: [0x91; 32],
        },
        crate::execution::OutputImage {
            mime: "image/png".to_owned(),
            width_px: Some(1024),
            height_px: Some(1536),
            alpha: CheckpointAlpha::Opaque,
        },
        [0x92; 32],
        2,
    )
    .expect("checkpoint");
    assert_eq!(checkpoint.book_call_id, first);
    assert_eq!(checkpoint.usage_revision, 2);
    assert_eq!(checkpoint.object_version, "etag-3");
    assert_eq!(checkpoint.byte_len, 48_128);
}

#[test]
fn checkpoint_refuses_empty_keys_zero_bytes_zero_dimensions_and_oversized_mime() {
    let dispatch = dispatch_id(0x81);
    assert_eq!(
        OutputCheckpoint::new(
            &dispatch,
            crate::execution::OutputObject {
                storage_key: String::new(),
                object_version: "etag-1".to_owned(),
                byte_len: 16,
                sha256: [0x91; 32]
            },
            crate::execution::OutputImage {
                mime: "image/png".to_owned(),
                width_px: None,
                height_px: None,
                alpha: CheckpointAlpha::Opaque
            },
            [0x92; 32],
            0
        ),
        Err(ExecutionSetupError::Empty)
    );
    assert_eq!(
        OutputCheckpoint::new(
            &dispatch,
            crate::execution::OutputObject {
                storage_key: "staged/key".to_owned(),
                object_version: "etag-1".to_owned(),
                byte_len: 0,
                sha256: [0x91; 32]
            },
            crate::execution::OutputImage {
                mime: "image/png".to_owned(),
                width_px: None,
                height_px: None,
                alpha: CheckpointAlpha::Opaque
            },
            [0x92; 32],
            0
        ),
        Err(ExecutionSetupError::ZeroBytes)
    );
    assert_eq!(
        OutputCheckpoint::new(
            &dispatch,
            crate::execution::OutputObject {
                storage_key: "staged/key".to_owned(),
                object_version: "etag-1".to_owned(),
                byte_len: 16,
                sha256: [0x91; 32]
            },
            crate::execution::OutputImage {
                mime: "image/png".to_owned(),
                width_px: Some(0),
                height_px: Some(100),
                alpha: CheckpointAlpha::Opaque
            },
            [0x92; 32],
            0
        ),
        Err(ExecutionSetupError::ZeroDimension)
    );
    assert_eq!(
        OutputCheckpoint::new(
            &dispatch,
            crate::execution::OutputObject {
                storage_key: "staged/key".to_owned(),
                object_version: "etag-1".to_owned(),
                byte_len: 16,
                sha256: [0x91; 32]
            },
            crate::execution::OutputImage {
                mime: "x".repeat(MAX_MIME_BYTES + 1),
                width_px: None,
                height_px: None,
                alpha: CheckpointAlpha::Opaque
            },
            [0x92; 32],
            0
        ),
        Err(ExecutionSetupError::TooLong {
            max_bytes: MAX_MIME_BYTES
        })
    );
}

#[test]
fn supplier_checkpoint_needs_version_origin_model_bounded_bytes_and_time() {
    let owner = owner_run();
    let valid = SupplierCheckpoint::new(
        crate::execution::ExecutionScope {
            owner: owner.clone(),
            job: job_id(0x11),
            stage: stage_id(0x21),
            dispatch: dispatch_id(0x81),
        },
        crate::execution::SupplierResume {
            checkpoint_version: 2,
            origin: "fal".to_owned(),
            model: "flux-ultra".to_owned(),
            checkpoint_bytes: vec![0x77; 64],
        },
        1_700_000_000_000,
    )
    .expect("supplier checkpoint");
    assert_eq!(valid.checkpoint_version, 2);
    assert_eq!(
        SupplierCheckpoint::new(
            crate::execution::ExecutionScope {
                owner: owner.clone(),
                job: job_id(0x11),
                stage: stage_id(0x21),
                dispatch: dispatch_id(0x81)
            },
            crate::execution::SupplierResume {
                checkpoint_version: 0,
                origin: "fal".to_owned(),
                model: "flux-ultra".to_owned(),
                checkpoint_bytes: vec![0x77; 64]
            },
            1_700_000_000_000
        ),
        Err(ExecutionSetupError::BadRevision)
    );
    assert_eq!(
        SupplierCheckpoint::new(
            crate::execution::ExecutionScope {
                owner: owner.clone(),
                job: job_id(0x11),
                stage: stage_id(0x21),
                dispatch: dispatch_id(0x81)
            },
            crate::execution::SupplierResume {
                checkpoint_version: 1,
                origin: String::new(),
                model: "flux-ultra".to_owned(),
                checkpoint_bytes: vec![0x77; 64]
            },
            1_700_000_000_000
        ),
        Err(ExecutionSetupError::Empty)
    );
    assert_eq!(
        SupplierCheckpoint::new(
            crate::execution::ExecutionScope {
                owner: owner.clone(),
                job: job_id(0x11),
                stage: stage_id(0x21),
                dispatch: dispatch_id(0x81)
            },
            crate::execution::SupplierResume {
                checkpoint_version: 1,
                origin: "fal".to_owned(),
                model: "flux-ultra".to_owned(),
                checkpoint_bytes: Vec::new()
            },
            1_700_000_000_000
        ),
        Err(ExecutionSetupError::Empty)
    );
    assert_eq!(
        SupplierCheckpoint::new(
            crate::execution::ExecutionScope {
                owner,
                job: job_id(0x11),
                stage: stage_id(0x21),
                dispatch: dispatch_id(0x81)
            },
            crate::execution::SupplierResume {
                checkpoint_version: 1,
                origin: "fal".to_owned(),
                model: "flux-ultra".to_owned(),
                checkpoint_bytes: vec![0x77; MAX_SUPPLIER_CHECKPOINT_BYTES + 1]
            },
            1_700_000_000_000
        ),
        Err(ExecutionSetupError::TooLong {
            max_bytes: MAX_SUPPLIER_CHECKPOINT_BYTES
        })
    );
}

#[test]
fn capacity_bounds_need_positive_limits_and_pending_admits_checks_both() {
    assert!(CapacityBounds::new(4, 16, 1 << 20).is_ok());
    assert_eq!(
        CapacityBounds::new(0, 16, 1 << 20),
        Err(ExecutionSetupError::BadRevision)
    );
    assert_eq!(
        CapacityBounds::new(4, 0, 1 << 20),
        Err(ExecutionSetupError::BadRevision)
    );
    assert_eq!(
        CapacityBounds::new(4, 16, 0),
        Err(ExecutionSetupError::BadRevision)
    );
    assert!(pending_admits(0, 0, 16, 1 << 20, 128));
    assert!(pending_admits(15, 0, 16, 1 << 20, 128));
    assert!(!pending_admits(16, 0, 16, 1 << 20, 128));
    assert!(!pending_admits(0, (1 << 20) - 64, 16, 1 << 20, 128));
    assert!(!pending_admits(0, 0, 0, 1 << 20, 128));
    assert!(!pending_admits(-1, 0, 16, 1 << 20, 128));
}

#[test]
fn supplier_report_needs_a_bounded_nonempty_correlation_id() {
    let owner = owner_run();
    assert!(
        SupplierReport::new(
            owner.clone(),
            job_id(0x11),
            stage_id(0x21),
            dispatch_id(0x81),
            "supplier-9f".to_owned(),
            1_700_000_000_000,
        )
        .is_ok()
    );
    assert_eq!(
        SupplierReport::new(
            owner.clone(),
            job_id(0x11),
            stage_id(0x21),
            dispatch_id(0x81),
            String::new(),
            1_700_000_000_000,
        ),
        Err(ExecutionSetupError::Empty)
    );
    assert_eq!(
        SupplierReport::new(
            owner,
            job_id(0x11),
            stage_id(0x21),
            dispatch_id(0x81),
            "s".repeat(MAX_SUPPLIER_JOB_ID_BYTES + 1),
            1_700_000_000_000,
        ),
        Err(ExecutionSetupError::TooLong {
            max_bytes: MAX_SUPPLIER_JOB_ID_BYTES
        })
    );
}

#[test]
fn execution_identity_needs_both_planned_ids() {
    assert!(ExecutionIdentity::new([0x41; 16], [0x42; 16]).is_ok());
    assert_eq!(
        ExecutionIdentity::new([0; 16], [0x42; 16]),
        Err(ExecutionSetupError::ZeroId)
    );
    assert_eq!(
        ExecutionIdentity::new([0x41; 16], [0; 16]),
        Err(ExecutionSetupError::ZeroId)
    );
}

#[test]
fn capacity_scope_refuses_empty_and_oversized_names() {
    assert_eq!(
        CapacityScope::new(String::new()),
        Err(ExecutionSetupError::Empty)
    );
    assert_eq!(
        CapacityScope::new("s".repeat(MAX_CAPACITY_SCOPE_BYTES + 1)),
        Err(ExecutionSetupError::TooLong {
            max_bytes: MAX_CAPACITY_SCOPE_BYTES
        })
    );
    assert_eq!(
        CapacityScope::new("live/generation".to_owned())
            .expect("scope")
            .as_str(),
        "live/generation"
    );
}

#[test]
fn opaque_object_versions_round_trip_the_existing_store_bound() {
    let checkpoint = |version: String| {
        OutputCheckpoint::new(
            &dispatch_id(0x81),
            crate::execution::OutputObject {
                storage_key: "staged/key".to_owned(),
                object_version: version,
                byte_len: 16,
                sha256: [0x91; 32],
            },
            crate::execution::OutputImage {
                mime: "image/png".to_owned(),
                width_px: None,
                height_px: None,
                alpha: CheckpointAlpha::Opaque,
            },
            [0x92; 32],
            0,
        )
    };
    let version = "x".repeat(MAXIMUM_OBJECT_VERSION_BYTES);
    assert_eq!(
        checkpoint(version.clone())
            .expect("store version")
            .object_version,
        version
    );
    assert_eq!(checkpoint(String::new()), Err(ExecutionSetupError::Empty));
    assert_eq!(
        checkpoint("x".repeat(MAXIMUM_OBJECT_VERSION_BYTES + 1)),
        Err(ExecutionSetupError::TooLong {
            max_bytes: MAXIMUM_OBJECT_VERSION_BYTES
        })
    );
}

fn decision_test_observation(
    payload: Vec<u8>,
    maximum_payload_bytes: usize,
) -> Result<DecisionObservation, ExecutionSetupError> {
    DecisionObservation::new(
        DecisionScope {
            owner: owner_run(),
            job: job_id(0x11),
            stage: stage_id(0x21),
            dispatch: dispatch_id(0x81),
        },
        DecisionKind::Classification,
        1,
        DecisionBasis {
            schema_revision: 2,
            validator_revision: 3,
            stage: StageBasis {
                kind: StageKind::Classification,
                input_revision: 1,
                input_digest: [0xab; 32],
                config_revision: 7,
                config_digest: [0xcd; 32],
                parent_dispatch: None,
            },
        },
        payload,
        DecisionReceipt {
            usage_revision: 1,
            observed_at_ms: 1_700_000_000_004,
        },
        maximum_payload_bytes,
    )
}

#[test]
fn decision_debug_redacts_private_payload_and_source_digests() {
    let marker = "PRIVATE_CLASSIFIER_SOURCE_NEVER_LOG";
    let observation =
        decision_test_observation(marker.as_bytes().to_vec(), 1024).expect("observation");
    assert_eq!(observation.payload(), marker.as_bytes());
    let printed = format!(
        "{observation:?} {basis:?} {scope:?}",
        basis = observation.basis,
        scope = observation.scope
    );
    assert!(!printed.contains(marker));
    assert!(!printed.contains("171, 171"));
    assert!(!printed.contains("205, 205"));
}

#[test]
fn decision_bounds_are_injected_and_can_exceed_the_documented_default() {
    let default = usize::try_from(DEFAULT_DECISION_PAYLOAD_BYTES).expect("default bytes");
    let payload = vec![0x61; default + 1];
    assert_eq!(
        decision_test_observation(payload.clone(), default),
        Err(ExecutionSetupError::TooLong { max_bytes: default })
    );
    assert_eq!(
        decision_test_observation(payload, default * 2)
            .expect("configured larger payload")
            .payload()
            .len(),
        default + 1
    );
    assert_eq!(
        decision_test_observation(Vec::new(), default),
        Err(ExecutionSetupError::Empty)
    );
    assert_eq!(
        decision_test_observation(vec![1], 0),
        Err(ExecutionSetupError::ZeroBytes)
    );
}

#[test]
fn decision_policy_preserves_capacity_constructor_default_and_rejects_invalid_bounds() {
    let default = CapacityBounds::new(1, 2, 4096).expect("capacity");
    assert_eq!(
        default.max_decision_payload_bytes,
        DEFAULT_DECISION_PAYLOAD_BYTES
    );
    assert_eq!(
        default
            .with_decision_payload_bytes(DEFAULT_DECISION_PAYLOAD_BYTES * 2)
            .expect("host policy")
            .max_decision_payload_bytes,
        DEFAULT_DECISION_PAYLOAD_BYTES * 2
    );
    assert_eq!(
        default.with_decision_payload_bytes(0),
        Err(ExecutionSetupError::ZeroBytes)
    );
    assert_eq!(
        default.with_decision_payload_bytes(MAXIMUM_SQL_DECISION_PAYLOAD_BYTES + 1),
        Err(ExecutionSetupError::ZeroBytes)
    );
}

#[test]
fn candidate_proof_preserves_edit_kind_and_rejects_final_or_non_image_stages() {
    use df_assets::{ImageAlpha, ImageIntent, ImageProofLimits};
    let bytes = [
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 6,
        0, 0, 0, 31, 21, 196, 137, 0, 0, 0, 13, 73, 68, 65, 84, 120, 156, 99, 248, 207, 192, 240,
        31, 0, 5, 0, 1, 255, 137, 153, 61, 29, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
    ];
    let scope = ExecutionScope {
        owner: owner_run(),
        job: job_id(0x11),
        stage: stage_id(0x21),
        dispatch: dispatch_id(0x81),
    };
    let mut basis = StageBasis {
        kind: StageKind::Edit,
        input_revision: 1,
        input_digest: [0xab; 32],
        config_revision: 7,
        config_digest: [0xcd; 32],
        parent_dispatch: None,
    };
    let receipt = ObservationReceipt {
        usage_revision: 1,
        observed_at_ms: 1_700_000_000_004,
    };
    let intent = ImageIntent::new("image/png", 1, 1, ImageAlpha::Opaque).expect("actual intent");
    let proof = CandidateObservation::inspect(
        scope.clone(),
        basis.clone(),
        1,
        receipt,
        &bytes,
        ImageProofLimits::default(),
        intent,
    )
    .expect("actual edit bytes");
    assert_eq!(proof.as_receipt().basis.kind, StageKind::Edit);
    assert_eq!(proof.as_receipt().descriptor.alpha, CheckpointAlpha::Opaque);
    assert!(!format!("{proof:?}").contains("image/png"));
    for kind in [
        StageKind::Classification,
        StageKind::Inspection,
        StageKind::LocalTransform,
    ] {
        basis.kind = kind;
        assert_eq!(
            CandidateObservation::inspect(
                scope.clone(),
                basis.clone(),
                1,
                receipt,
                &bytes,
                ImageProofLimits::default(),
                intent
            ),
            Err(CandidateError::WrongStage)
        );
    }
}
