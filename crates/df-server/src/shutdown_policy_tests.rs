use crate::shutdown_policy::{
    AdmissionDecision, AdmissionHold, AdmissionMode, AdmissionPolicy, AdmissionRefusal,
    DrainPolicy, DrainRefusal, DrainStatus, DrainStep, MAX_RETAINED_POSSIBLE_SENDS, RetainRefusal,
};
use df_session::effects::{
    DispatchKnowledge, DispatchLookup, DispatchRecord, EffectRepositoryError,
};

type Record = DispatchRecord<&'static str, &'static str, &'static str>;
type Lookup = DispatchLookup<&'static str, &'static str, &'static str>;

fn identity(knowledge: DispatchKnowledge) -> Record {
    Record {
        attempt_id: "attempt-7",
        reservation_id: "reservation-3",
        provider_key: "provider-key-9",
        knowledge,
    }
}

fn held(
    decision: AdmissionDecision<&'static str, &'static str, &'static str>,
) -> AdmissionHold<&'static str, &'static str, &'static str> {
    let AdmissionDecision::Held(hold) = decision else {
        panic!("expected a held admission")
    };
    hold
}

#[test]
fn serving_new_admission_checks_latest_journal_and_retains_attempt_on_every_gap() {
    let admission = AdmissionPolicy::default();
    assert_eq!(
        admission.admit_new_work(
            Ok(Lookup::Recorded(identity(DispatchKnowledge::Completed))),
            None
        ),
        AdmissionDecision::Admitted
    );
    assert_eq!(
        admission.admit_new_work(
            Ok(Lookup::Recorded(identity(
                DispatchKnowledge::VerifiedUnsentCanceled
            ))),
            None
        ),
        AdmissionDecision::Admitted
    );

    for (latest, reason) in [
        (Ok(Lookup::Missing), AdmissionRefusal::JournalMissing),
        (Ok(Lookup::Pending), AdmissionRefusal::JournalPending),
        (
            Err(EffectRepositoryError::Unavailable),
            AdmissionRefusal::JournalUnavailable,
        ),
        (
            Err(EffectRepositoryError::UnresolvedClaim),
            AdmissionRefusal::PossibleSendUnresolved,
        ),
    ] {
        let hold =
            held(admission.admit_new_work(latest, Some(identity(DispatchKnowledge::Unknown))));
        assert_eq!(hold.reason, reason);
        assert_eq!(
            hold.attempted_record,
            Some(identity(DispatchKnowledge::Unknown))
        );
        assert_eq!(hold.latest_record, None);
    }
    assert_eq!(
        held(admission.admit_new_work(Err(EffectRepositoryError::UnresolvedClaim), None)).reason,
        AdmissionRefusal::UnresolvedClaimWithoutIdentity
    );
}

#[test]
fn possible_send_and_conflicting_terminal_identity_remain_discoverable() {
    let admission = AdmissionPolicy::default();
    for knowledge in [DispatchKnowledge::Dispatching, DispatchKnowledge::Unknown] {
        let record = identity(knowledge);
        let hold = held(admission.admit_recovery(Ok(Lookup::Recorded(record.clone())), None));
        assert_eq!(hold.reason, AdmissionRefusal::PossibleSendUnresolved);
        assert_eq!(hold.latest_record, Some(record));
    }

    let attempted = Record {
        attempt_id: "different-attempt",
        reservation_id: "reservation-3",
        provider_key: "different-key",
        knowledge: DispatchKnowledge::Unknown,
    };
    let hold = held(admission.admit_new_work(
        Ok(Lookup::Recorded(identity(DispatchKnowledge::Completed))),
        Some(attempted.clone()),
    ));
    assert_eq!(hold.reason, AdmissionRefusal::ConflictingRecordedIdentity);
    assert_eq!(
        hold.latest_record,
        Some(identity(DispatchKnowledge::Completed))
    );
    assert_eq!(hold.attempted_record, Some(attempted));
}

#[test]
fn incident_and_shutdown_are_monotone_and_preserve_held_attempts() {
    let mut admission = AdmissionPolicy::default();
    admission.enter_incident();
    assert_eq!(admission.mode(), AdmissionMode::Incident);
    let hold = held(admission.admit_new_work(
        Ok(Lookup::Recorded(identity(DispatchKnowledge::Unknown))),
        None,
    ));
    assert_eq!(hold.reason, AdmissionRefusal::Incident);
    assert_eq!(
        hold.latest_record,
        Some(identity(DispatchKnowledge::Unknown))
    );

    admission.begin_shutdown();
    admission.enter_incident();
    assert_eq!(admission.mode(), AdmissionMode::ShuttingDown);
    let hold = held(admission.admit_recovery(
        Err(EffectRepositoryError::Unavailable),
        Some(identity(DispatchKnowledge::Unknown)),
    ));
    assert_eq!(hold.reason, AdmissionRefusal::ShuttingDown);
    assert_eq!(
        hold.attempted_record,
        Some(identity(DispatchKnowledge::Unknown))
    );
}

#[test]
fn repository_close_and_telemetry_timeout_keep_owned_attempts_visible() {
    let mut drain: DrainPolicy<&str, &str, &str> = DrainPolicy::default();
    drain
        .retain_possible_send(identity(DispatchKnowledge::Unknown))
        .unwrap();
    assert_eq!(
        drain.status(),
        DrainStatus::Incomplete {
            next: DrainStep::StopProducers
        }
    );
    assert_eq!(
        drain.observe(DrainStep::DrainAcceptedInputs, true),
        Err(DrainRefusal::WrongStep {
            expected: DrainStep::StopProducers,
            received: DrainStep::DrainAcceptedInputs,
        })
    );
    drain.observe(DrainStep::StopProducers, true).unwrap();
    drain.observe(DrainStep::DrainAcceptedInputs, true).unwrap();
    assert_eq!(
        drain.observe(DrainStep::CloseRepositoryOnActorThread, false),
        Err(DrainRefusal::StepFailed(
            DrainStep::CloseRepositoryOnActorThread
        ))
    );
    assert_eq!(
        drain.status(),
        DrainStatus::Incomplete {
            next: DrainStep::CloseRepositoryOnActorThread
        }
    );
    assert_eq!(
        drain.possible_sends(),
        &[identity(DispatchKnowledge::Unknown)]
    );
    drain
        .observe(DrainStep::CloseRepositoryOnActorThread, true)
        .unwrap();
    drain
        .observe(DrainStep::JoinActorWhileRuntimeAlive, true)
        .unwrap();
    assert_eq!(
        drain.observe(DrainStep::ShutdownTelemetry, false),
        Err(DrainRefusal::StepFailed(DrainStep::ShutdownTelemetry))
    );
    assert_eq!(
        drain.status(),
        DrainStatus::Incomplete {
            next: DrainStep::ShutdownTelemetry
        }
    );
    assert_eq!(
        drain.possible_sends(),
        &[identity(DispatchKnowledge::Unknown)]
    );
    assert_eq!(
        drain.observe(DrainStep::ShutdownTelemetry, true),
        Ok(DrainStatus::HeldPossibleSends { count: 1 })
    );
    assert_eq!(
        drain.possible_sends(),
        &[identity(DispatchKnowledge::Unknown)]
    );
}

#[test]
fn bounded_drain_returns_rejected_record_to_its_owner() {
    let mut drain: DrainPolicy<u16, u16, u16> = DrainPolicy::default();
    for index in 0..MAX_RETAINED_POSSIBLE_SENDS {
        drain
            .retain_possible_send(DispatchRecord {
                attempt_id: index as u16,
                reservation_id: index as u16,
                provider_key: index as u16,
                knowledge: DispatchKnowledge::Unknown,
            })
            .unwrap();
    }
    let overflow = DispatchRecord {
        attempt_id: MAX_RETAINED_POSSIBLE_SENDS as u16,
        reservation_id: 0,
        provider_key: 0,
        knowledge: DispatchKnowledge::Dispatching,
    };
    assert_eq!(
        drain.retain_possible_send(overflow.clone()),
        Err(RetainRefusal::Capacity(overflow))
    );
    assert_eq!(drain.possible_sends().len(), MAX_RETAINED_POSSIBLE_SENDS);
}
