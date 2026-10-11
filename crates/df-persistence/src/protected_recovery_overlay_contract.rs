//! Finite DESIGN contract for a protected irreversible restore overlay.
//! These private fixtures supply hypothetical qualified evidence. They do not authenticate a
//! head, hold keys, append objects, issue epochs, reconcile a provider, or serve a restored PG.
//! A PostgreSQL-only audit row, old backup, telemetry spool, or gameplay replay cannot replace
//! the independently protected head. Durable object choice, CAS, key custody and actual restore
//! remain G05/X02 gates; this test model cannot qualify their cost or recovery guarantee.

use df_model::checkpoint::LostGameRange;
use df_types::{Currency, Money, OperationId, RecoveryEpoch, SessionRevision};

const MAX_ENTRIES: usize = 16;
const MAX_ID_BYTES: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Suppression {
    Deletion,
    Revocation,
    Rights,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SendKnowledge {
    VerifiedUnsent,
    Unknown,
    MayHaveBeenSent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum IrreversibleKind<'a> {
    Suppress {
        identity: &'a str,
        kind: Suppression,
    },
    PossibleSend {
        intent: OperationId,
        maximum_liability: Money,
        knowledge: SendKnowledge,
    },
    PaymentObserved {
        identity: &'a str,
        amount: Money,
    },
    RetireNamespace {
        epoch: RecoveryEpoch,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct JournalEntry<'a> {
    sequence: u64,
    kind: IrreversibleKind<'a>,
}

/// Only a finite test input for a hypothetically selected, qualified protected backing.
/// Numeric equality here is necessary but cannot prove authenticity, durability or completeness.
struct QualifiedFixture<'a> {
    epoch: RecoveryEpoch,
    published_head: u64,
    off_host_watermark: u64,
    journal_head: u64,
    entries: &'a [JournalEntry<'a>],
}

enum HeadObservation<'a> {
    MissingJournal,
    MissingHead,
    MissingKey,
    Unqualified,
    Incomplete,
    QualifiedFixture(QualifiedFixture<'a>),
}

#[derive(Clone, Copy)]
struct OlderSnapshot {
    covered_head: u64,
    revision: SessionRevision,
    latest_acknowledged_revision: SessionRevision,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Closed {
    MissingJournal,
    MissingHead,
    MissingKey,
    Unqualified,
    Incomplete,
    HeadConflict,
    InvalidRange,
    InvalidEntry,
    ConflictingIdentity,
    Capacity,
    LiabilityCurrency,
    LiabilityOverflow,
    EpochExhausted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OverlayAction<'a> {
    Suppress {
        identity: &'a str,
        kind: Suppression,
    },
    HoldSend {
        intent: OperationId,
        maximum: Money,
    },
    ReleaseVerifiedUnsent {
        intent: OperationId,
    },
    ReconcilePayment {
        identity: &'a str,
        amount: Money,
    },
    RetireNamespace {
        epoch: RecoveryEpoch,
    },
}

#[derive(Debug, Eq, PartialEq)]
struct OverlayPlan<'a> {
    /// A candidate only; real issuance needs an authenticated protected-head CAS.
    candidate_epoch: RecoveryEpoch,
    lost_game_range: Option<LostGameRange>,
    actions: Vec<OverlayAction<'a>>,
    held_liability: Option<Money>,
}

fn entry_identity_is_bounded(kind: IrreversibleKind<'_>) -> bool {
    let identity = match kind {
        IrreversibleKind::Suppress { identity, .. }
        | IrreversibleKind::PaymentObserved { identity, .. } => Some(identity),
        IrreversibleKind::PossibleSend { .. } | IrreversibleKind::RetireNamespace { .. } => None,
    };
    identity.is_none_or(|value| !value.is_empty() && value.len() <= MAX_ID_BYTES)
}

fn same_identity(left: IrreversibleKind<'_>, right: IrreversibleKind<'_>) -> bool {
    match (left, right) {
        (
            IrreversibleKind::Suppress { identity: left, .. },
            IrreversibleKind::Suppress {
                identity: right, ..
            },
        )
        | (
            IrreversibleKind::PaymentObserved { identity: left, .. },
            IrreversibleKind::PaymentObserved {
                identity: right, ..
            },
        ) => left == right,
        (
            IrreversibleKind::PossibleSend { intent: left, .. },
            IrreversibleKind::PossibleSend { intent: right, .. },
        ) => left == right,
        (
            IrreversibleKind::RetireNamespace { epoch: left },
            IrreversibleKind::RetireNamespace { epoch: right },
        ) => left == right,
        _ => false,
    }
}

fn push_action<'a>(
    actions: &mut Vec<OverlayAction<'a>>,
    action: OverlayAction<'a>,
) -> Result<(), Closed> {
    if actions.len() >= MAX_ENTRIES {
        return Err(Closed::Capacity);
    }
    actions.try_reserve(1).map_err(|_| Closed::Capacity)?;
    actions.push(action);
    Ok(())
}

/// Absence from the older snapshot is never used as evidence that an irreversible event did not
/// occur. All post-backup entries become suppressions, holds or reconciliation, never new sends.
fn assess_restore<'a>(
    snapshot: OlderSnapshot,
    observation: HeadObservation<'a>,
) -> Result<OverlayPlan<'a>, Closed> {
    let fixture = match observation {
        HeadObservation::MissingJournal => return Err(Closed::MissingJournal),
        HeadObservation::MissingHead => return Err(Closed::MissingHead),
        HeadObservation::MissingKey => return Err(Closed::MissingKey),
        HeadObservation::Unqualified => return Err(Closed::Unqualified),
        HeadObservation::Incomplete => return Err(Closed::Incomplete),
        HeadObservation::QualifiedFixture(fixture) => fixture,
    };
    if fixture.published_head != fixture.off_host_watermark
        || fixture.published_head != fixture.journal_head
    {
        return Err(Closed::HeadConflict);
    }
    if fixture.entries.len() > MAX_ENTRIES {
        return Err(Closed::Capacity);
    }
    if fixture.published_head
        != u64::try_from(fixture.entries.len()).map_err(|_| Closed::Capacity)?
    {
        return Err(Closed::Incomplete);
    }
    if snapshot.covered_head > fixture.published_head
        || snapshot.revision.epoch() != fixture.epoch
        || snapshot.latest_acknowledged_revision < snapshot.revision
        || snapshot.latest_acknowledged_revision.epoch() != snapshot.revision.epoch()
    {
        return Err(Closed::InvalidRange);
    }
    let next_epoch = fixture
        .epoch
        .get()
        .checked_add(1)
        .ok_or(Closed::EpochExhausted)?;
    let candidate_epoch = RecoveryEpoch::new(next_epoch).map_err(|_| Closed::EpochExhausted)?;
    let lost_game_range = if snapshot.latest_acknowledged_revision > snapshot.revision {
        Some(LostGameRange {
            from: snapshot
                .revision
                .next_sequence()
                .map_err(|_| Closed::InvalidRange)?,
            through: snapshot.latest_acknowledged_revision,
        })
    } else {
        None
    };

    let mut seen: Vec<IrreversibleKind<'a>> = Vec::new();
    seen.try_reserve(fixture.entries.len())
        .map_err(|_| Closed::Capacity)?;
    for (index, entry) in fixture.entries.iter().enumerate() {
        let expected = u64::try_from(index + 1).map_err(|_| Closed::Capacity)?;
        if entry.sequence != expected || !entry_identity_is_bounded(entry.kind) {
            return Err(Closed::InvalidEntry);
        }
        if let Some(prior) = seen
            .iter()
            .copied()
            .find(|prior| same_identity(*prior, entry.kind))
        {
            if prior != entry.kind {
                return Err(Closed::ConflictingIdentity);
            }
            continue;
        }
        seen.push(entry.kind);
    }

    let mut actions = Vec::new();
    let mut held_liability: Option<Money> = None;
    // Replay every suppression before any private serving or derived-index rebuild. Do not
    // assume even a pre-backup suppression survived in the older PostgreSQL image.
    for entry in fixture.entries {
        if let IrreversibleKind::Suppress { identity, kind } = entry.kind
            && !actions.iter().any(|action| {
                matches!(action, OverlayAction::Suppress { identity: prior, .. } if *prior == identity)
            })
        {
            push_action(&mut actions, OverlayAction::Suppress { identity, kind })?;
        }
    }
    for (index, entry) in fixture.entries.iter().enumerate() {
        if fixture.entries[..index]
            .iter()
            .any(|prior| same_identity(prior.kind, entry.kind))
        {
            continue;
        }
        let action = match entry.kind {
            IrreversibleKind::Suppress { .. } => continue,
            IrreversibleKind::PossibleSend {
                intent,
                maximum_liability,
                knowledge,
            } => match knowledge {
                SendKnowledge::VerifiedUnsent => OverlayAction::ReleaseVerifiedUnsent { intent },
                SendKnowledge::Unknown | SendKnowledge::MayHaveBeenSent => {
                    if maximum_liability.micros() == 0 {
                        return Err(Closed::InvalidEntry);
                    }
                    held_liability = Some(match held_liability {
                        Some(total) => total.checked_add(maximum_liability).map_err(|error| {
                            if error == df_types::MoneyError::CurrencyMismatch {
                                Closed::LiabilityCurrency
                            } else {
                                Closed::LiabilityOverflow
                            }
                        })?,
                        None => maximum_liability,
                    });
                    OverlayAction::HoldSend {
                        intent,
                        maximum: maximum_liability,
                    }
                }
            },
            IrreversibleKind::PaymentObserved { identity, amount } => {
                OverlayAction::ReconcilePayment { identity, amount }
            }
            IrreversibleKind::RetireNamespace { epoch } => {
                if epoch >= candidate_epoch {
                    return Err(Closed::InvalidEntry);
                }
                OverlayAction::RetireNamespace { epoch }
            }
        };
        if !actions.contains(&action) {
            push_action(&mut actions, action)?;
        }
    }
    // Disaster recovery retires the old backup epoch even if its retired-namespace row was lost.
    let retired = OverlayAction::RetireNamespace {
        epoch: snapshot.revision.epoch(),
    };
    if !actions.contains(&retired) {
        push_action(&mut actions, retired)?;
    }
    Ok(OverlayPlan {
        candidate_epoch,
        lost_game_range,
        actions,
        held_liability,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RetryDisposition {
    CurrentNamespaceCandidate,
    RetainedLookupOnly,
    ExpiredOrIndeterminate,
}

fn retry_disposition(
    plan: &OverlayPlan<'_>,
    supplied_epoch: RecoveryEpoch,
    retained_exact_receipt: bool,
) -> RetryDisposition {
    if supplied_epoch == plan.candidate_epoch {
        RetryDisposition::CurrentNamespaceCandidate
    } else if supplied_epoch < plan.candidate_epoch && retained_exact_receipt {
        RetryDisposition::RetainedLookupOnly
    } else {
        RetryDisposition::ExpiredOrIndeterminate
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn epoch(value: u64) -> RecoveryEpoch {
        RecoveryEpoch::new(value).unwrap()
    }

    fn revision(epoch_value: u64, sequence: u64) -> SessionRevision {
        SessionRevision::new(epoch(epoch_value), sequence)
    }

    fn operation(byte: u8) -> OperationId {
        OperationId::from_bytes(&[byte; 16]).unwrap()
    }

    fn money(micros: u128) -> Money {
        Money::new(Currency::parse("USD").unwrap(), micros)
    }

    fn snapshot() -> OlderSnapshot {
        OlderSnapshot {
            covered_head: 0,
            revision: revision(7, 10),
            latest_acknowledged_revision: revision(7, 12),
        }
    }

    fn qualified<'a>(entries: &'a [JournalEntry<'a>]) -> HeadObservation<'a> {
        let head = u64::try_from(entries.len()).unwrap();
        HeadObservation::QualifiedFixture(QualifiedFixture {
            epoch: epoch(7),
            published_head: head,
            off_host_watermark: head,
            journal_head: head,
            entries,
        })
    }

    fn send<'a>(sequence: u64, intent: OperationId, knowledge: SendKnowledge) -> JournalEntry<'a> {
        JournalEntry {
            sequence,
            kind: IrreversibleKind::PossibleSend {
                intent,
                maximum_liability: money(900),
                knowledge,
            },
        }
    }

    fn label(result: &Result<OverlayPlan<'_>, Closed>) -> String {
        match result {
            Ok(plan) => {
                let kinds: Vec<&str> = plan
                    .actions
                    .iter()
                    .map(|action| match action {
                        OverlayAction::Suppress { .. } => "suppress",
                        OverlayAction::HoldSend { .. } => "hold_send",
                        OverlayAction::ReleaseVerifiedUnsent { .. } => "verified_unsent",
                        OverlayAction::ReconcilePayment { .. } => "reconcile_payment",
                        OverlayAction::RetireNamespace { .. } => "retire_namespace",
                    })
                    .collect();
                format!("overlay:{}", kinds.join(","))
            }
            Err(reason) => format!("closed:{reason:?}"),
        }
    }

    #[test]
    fn protected_recovery_overlay_contract() {
        let old = snapshot();
        let entries = [
            JournalEntry {
                sequence: 1,
                kind: IrreversibleKind::Suppress {
                    identity: "subject-A",
                    kind: Suppression::Deletion,
                },
            },
            send(2, operation(1), SendKnowledge::Unknown),
            JournalEntry {
                sequence: 3,
                kind: IrreversibleKind::PaymentObserved {
                    identity: "payment-A",
                    amount: money(500),
                },
            },
            JournalEntry {
                sequence: 4,
                kind: IrreversibleKind::RetireNamespace { epoch: epoch(7) },
            },
        ];
        let plan = assess_restore(old, qualified(&entries)).unwrap();
        assert_eq!(plan.candidate_epoch, epoch(8));
        assert_eq!(
            plan.lost_game_range,
            Some(LostGameRange {
                from: revision(7, 11),
                through: revision(7, 12),
            })
        );
        assert_eq!(plan.held_liability, Some(money(900)));
        assert_eq!(
            plan.actions,
            vec![
                OverlayAction::Suppress {
                    identity: "subject-A",
                    kind: Suppression::Deletion,
                },
                OverlayAction::HoldSend {
                    intent: operation(1),
                    maximum: money(900),
                },
                OverlayAction::ReconcilePayment {
                    identity: "payment-A",
                    amount: money(500),
                },
                OverlayAction::RetireNamespace { epoch: epoch(7) },
            ]
        );
        assert_eq!(
            retry_disposition(&plan, epoch(7), true),
            RetryDisposition::RetainedLookupOnly
        );
        assert_eq!(
            retry_disposition(&plan, epoch(7), false),
            RetryDisposition::ExpiredOrIndeterminate
        );
        assert_eq!(
            retry_disposition(&plan, epoch(8), false),
            RetryDisposition::CurrentNamespaceCandidate
        );
        assert_eq!(
            retry_disposition(&plan, epoch(9), true),
            RetryDisposition::ExpiredOrIndeterminate
        );

        let mut witness = vec![("rollback_absence", label(&Ok(plan)))];
        for (name, observation, expected) in [
            (
                "missing_journal",
                HeadObservation::MissingJournal,
                Closed::MissingJournal,
            ),
            (
                "missing_head",
                HeadObservation::MissingHead,
                Closed::MissingHead,
            ),
            (
                "missing_key",
                HeadObservation::MissingKey,
                Closed::MissingKey,
            ),
            (
                "unqualified",
                HeadObservation::Unqualified,
                Closed::Unqualified,
            ),
            (
                "incomplete",
                HeadObservation::Incomplete,
                Closed::Incomplete,
            ),
        ] {
            let outcome = assess_restore(old, observation);
            assert_eq!(outcome, Err(expected));
            witness.push((name, label(&outcome)));
        }
        let head_conflict = HeadObservation::QualifiedFixture(QualifiedFixture {
            epoch: epoch(7),
            published_head: 4,
            off_host_watermark: 3,
            journal_head: 4,
            entries: &entries,
        });
        let outcome = assess_restore(old, head_conflict);
        assert_eq!(outcome, Err(Closed::HeadConflict));
        witness.push(("regressed_watermark", label(&outcome)));
        let ahead = HeadObservation::QualifiedFixture(QualifiedFixture {
            epoch: epoch(7),
            published_head: 4,
            off_host_watermark: 5,
            journal_head: 4,
            entries: &entries,
        });
        let outcome = assess_restore(old, ahead);
        assert_eq!(outcome, Err(Closed::HeadConflict));
        witness.push(("ahead_watermark", label(&outcome)));
        let tail = HeadObservation::QualifiedFixture(QualifiedFixture {
            epoch: epoch(7),
            published_head: 3,
            off_host_watermark: 3,
            journal_head: 4,
            entries: &entries,
        });
        let outcome = assess_restore(old, tail);
        assert_eq!(outcome, Err(Closed::HeadConflict));
        witness.push(("unpublished_tail", label(&outcome)));
        let regressed_journal = HeadObservation::QualifiedFixture(QualifiedFixture {
            epoch: epoch(7),
            published_head: 4,
            off_host_watermark: 4,
            journal_head: 3,
            entries: &entries,
        });
        let outcome = assess_restore(old, regressed_journal);
        assert_eq!(outcome, Err(Closed::HeadConflict));
        witness.push(("regressed_journal", label(&outcome)));
        let missing_entry = HeadObservation::QualifiedFixture(QualifiedFixture {
            epoch: epoch(7),
            published_head: 4,
            off_host_watermark: 4,
            journal_head: 4,
            entries: &entries[..3],
        });
        let outcome = assess_restore(old, missing_entry);
        assert_eq!(outcome, Err(Closed::Incomplete));
        witness.push(("missing_latest_entry", label(&outcome)));
        let unordered = [entries[0], entries[2], entries[1], entries[3]];
        let outcome = assess_restore(old, qualified(&unordered));
        assert_eq!(outcome, Err(Closed::InvalidEntry));
        witness.push(("unordered_entries", label(&outcome)));
        let extra_unsent = [send(1, operation(2), SendKnowledge::Unknown)];
        let outcome = assess_restore(old, qualified(&extra_unsent));
        assert_eq!(outcome.as_ref().unwrap().held_liability, Some(money(900)));
        witness.push(("extra_unsent_entry", label(&outcome)));
        let verified_unsent = [send(1, operation(2), SendKnowledge::VerifiedUnsent)];
        let outcome = assess_restore(old, qualified(&verified_unsent));
        assert_eq!(outcome.as_ref().unwrap().held_liability, None);
        witness.push(("verified_unsent", label(&outcome)));
        let maybe_sent = [send(1, operation(2), SendKnowledge::MayHaveBeenSent)];
        let outcome = assess_restore(old, qualified(&maybe_sent));
        assert_eq!(outcome.as_ref().unwrap().held_liability, Some(money(900)));
        witness.push(("possibly_sent", label(&outcome)));
        let payment_twice = [
            entries[2],
            JournalEntry {
                sequence: 2,
                ..entries[2]
            },
        ];
        let payment_twice = [
            JournalEntry {
                sequence: 1,
                ..payment_twice[0]
            },
            payment_twice[1],
        ];
        let outcome = assess_restore(old, qualified(&payment_twice));
        assert_eq!(
            outcome
                .as_ref()
                .unwrap()
                .actions
                .iter()
                .filter(|action| matches!(action, OverlayAction::ReconcilePayment { .. }))
                .count(),
            1
        );
        witness.push(("duplicate_payment", label(&outcome)));
        let conflicting_payment = [
            payment_twice[0],
            JournalEntry {
                sequence: 2,
                kind: IrreversibleKind::PaymentObserved {
                    identity: "payment-A",
                    amount: money(501),
                },
            },
        ];
        let outcome = assess_restore(old, qualified(&conflicting_payment));
        assert_eq!(outcome, Err(Closed::ConflictingIdentity));
        witness.push(("conflicting_payment", label(&outcome)));
        let conflicting_send = [
            send(1, operation(1), SendKnowledge::Unknown),
            send(2, operation(1), SendKnowledge::VerifiedUnsent),
        ];
        let outcome = assess_restore(old, qualified(&conflicting_send));
        assert_eq!(outcome, Err(Closed::ConflictingIdentity));
        witness.push(("conflicting_send", label(&outcome)));
        let currency_conflict = [
            send(1, operation(1), SendKnowledge::Unknown),
            JournalEntry {
                sequence: 2,
                kind: IrreversibleKind::PossibleSend {
                    intent: operation(2),
                    maximum_liability: Money::new(Currency::parse("EUR").unwrap(), 2),
                    knowledge: SendKnowledge::Unknown,
                },
            },
        ];
        let outcome = assess_restore(old, qualified(&currency_conflict));
        assert_eq!(outcome, Err(Closed::LiabilityCurrency));
        witness.push(("liability_currency", label(&outcome)));
        let overflow = [
            JournalEntry {
                sequence: 1,
                kind: IrreversibleKind::PossibleSend {
                    intent: operation(1),
                    maximum_liability: money(u128::MAX),
                    knowledge: SendKnowledge::Unknown,
                },
            },
            send(2, operation(2), SendKnowledge::Unknown),
        ];
        let outcome = assess_restore(old, qualified(&overflow));
        assert_eq!(outcome, Err(Closed::LiabilityOverflow));
        witness.push(("liability_overflow", label(&outcome)));
        let old_epoch = HeadObservation::QualifiedFixture(QualifiedFixture {
            epoch: epoch(6),
            published_head: 0,
            off_host_watermark: 0,
            journal_head: 0,
            entries: &[],
        });
        let outcome = assess_restore(old, old_epoch);
        assert_eq!(outcome, Err(Closed::InvalidRange));
        witness.push(("epoch_regression", label(&outcome)));
        let older_snapshot_epoch = OlderSnapshot {
            revision: revision(6, 10),
            latest_acknowledged_revision: revision(6, 12),
            ..old
        };
        let outcome = assess_restore(older_snapshot_epoch, qualified(&[]));
        assert_eq!(outcome, Err(Closed::InvalidRange));
        witness.push(("older_snapshot_epoch", label(&outcome)));
        let exhausted = HeadObservation::QualifiedFixture(QualifiedFixture {
            epoch: epoch(u64::MAX),
            published_head: 0,
            off_host_watermark: 0,
            journal_head: 0,
            entries: &[],
        });
        let maximum_epoch_snapshot = OlderSnapshot {
            revision: revision(u64::MAX, 10),
            latest_acknowledged_revision: revision(u64::MAX, 12),
            ..old
        };
        let outcome = assess_restore(maximum_epoch_snapshot, exhausted);
        assert_eq!(outcome, Err(Closed::EpochExhausted));
        witness.push(("epoch_exhausted", label(&outcome)));
        let too_many = vec![
            JournalEntry {
                sequence: 1,
                kind: IrreversibleKind::RetireNamespace { epoch: epoch(7) },
            };
            MAX_ENTRIES + 1
        ];
        let outcome = assess_restore(old, qualified(&too_many));
        assert_eq!(outcome, Err(Closed::Capacity));
        witness.push(("entry_capacity", label(&outcome)));
        let oversize = [JournalEntry {
            sequence: 1,
            kind: IrreversibleKind::Suppress {
                identity: "this-identity-is-longer-than-32-bytes",
                kind: Suppression::Revocation,
            },
        }];
        let outcome = assess_restore(old, qualified(&oversize));
        assert_eq!(outcome, Err(Closed::InvalidEntry));
        witness.push(("oversize_identity", label(&outcome)));
        let rights = [JournalEntry {
            sequence: 1,
            kind: IrreversibleKind::Suppress {
                identity: "rights-A",
                kind: Suppression::Rights,
            },
        }];
        let outcome = assess_restore(old, qualified(&rights));
        assert!(matches!(
            outcome.as_ref().unwrap().actions.first(),
            Some(OverlayAction::Suppress {
                kind: Suppression::Rights,
                ..
            })
        ));
        witness.push(("rights_suppression", label(&outcome)));
        let revoked = [JournalEntry {
            sequence: 1,
            kind: IrreversibleKind::Suppress {
                identity: "grant-A",
                kind: Suppression::Revocation,
            },
        }];
        let outcome = assess_restore(old, qualified(&revoked));
        assert!(matches!(
            outcome.as_ref().unwrap().actions.first(),
            Some(OverlayAction::Suppress {
                kind: Suppression::Revocation,
                ..
            })
        ));
        witness.push(("revocation_suppression", label(&outcome)));

        let actual = format!(
            "[\n{}\n]\n",
            witness
                .iter()
                .map(|(name, decision)| format!(
                    "  {{\"case\":\"{name}\",\"result\":\"{decision}\"}}"
                ))
                .collect::<Vec<_>>()
                .join(",\n")
        );
        assert_eq!(
            actual,
            include_str!("../tests/protected_recovery_overlay_contract.json")
        );
    }

    #[test]
    fn rpo_recovery_epoch_loss_disclosure_contract() {
        let old = snapshot();
        let restored = assess_restore(old, qualified(&[])).unwrap();
        assert_eq!(
            restored.lost_game_range,
            Some(LostGameRange {
                from: revision(7, 11),
                through: revision(7, 12),
            })
        );
        assert_eq!(restored.candidate_epoch, epoch(8));

        fn disclosed(plan: &OverlayPlan<'_>) -> String {
            match plan.lost_game_range {
                Some(range) => format!(
                    "loss:{}:{}..{}:{};candidate:{}:0",
                    range.from.epoch().get(),
                    range.from.sequence(),
                    range.through.epoch().get(),
                    range.through.sequence(),
                    plan.candidate_epoch.get()
                ),
                None => format!(
                    "no_acknowledged_loss;candidate:{}:0",
                    plan.candidate_epoch.get()
                ),
            }
        }

        let mut witness = vec![
            ("OriginalAcceptance1", "game loss disclosed".to_owned()),
            (
                "OriginalAcceptance2",
                "The named outcome has actual source/build-bound evidence; unsupported, pending, failed and unperformed checks remain explicit.".to_owned(),
            ),
            (
                "decision",
                "qualified restore uses a strictly newer recovery epoch, exact lost game-revision range, retired old namespaces and lookup-only retained receipts".to_owned(),
            ),
            (
                "rejected_alternatives",
                "sequence-only ordering, old-Pg absence as unsent proof, guessed no-loss and retrying Unknown are rejected".to_owned(),
            ),
            (
                "unresolved",
                "protected backing authentication, atomic epoch CAS, mounted restore projection and measured RPO300seconds/RTO3600seconds remain unperformed; fixtures issue no epoch or public snapshot".to_owned(),
            ),
            ("exact_lost_acknowledged_range", disclosed(&restored)),
        ];

        let complete = OlderSnapshot {
            latest_acknowledged_revision: old.revision,
            ..old
        };
        let no_loss = assess_restore(complete, qualified(&[])).unwrap();
        assert_eq!(no_loss.lost_game_range, None);
        let ordinary_next = old.revision.next_sequence().unwrap();
        assert_eq!(ordinary_next, revision(7, 11));
        assert_eq!(ordinary_next.epoch(), old.revision.epoch());
        witness.push(("no_acknowledged_game_loss", disclosed(&no_loss)));

        let maximum_ack = OlderSnapshot {
            latest_acknowledged_revision: revision(7, u64::MAX),
            ..old
        };
        let maximum_loss = assess_restore(maximum_ack, qualified(&[])).unwrap();
        assert_eq!(
            maximum_loss.lost_game_range,
            Some(LostGameRange {
                from: revision(7, 11),
                through: revision(7, u64::MAX),
            })
        );
        let candidate_revision = SessionRevision::new(maximum_loss.candidate_epoch, 0);
        assert!(candidate_revision > maximum_ack.latest_acknowledged_revision);
        witness.push((
            "new_epoch_orders_above_maximum_old_sequence",
            disclosed(&maximum_loss),
        ));

        for acknowledged in [revision(7, 9), revision(6, 12), revision(8, 12)] {
            let inconsistent = OlderSnapshot {
                latest_acknowledged_revision: acknowledged,
                ..old
            };
            assert_eq!(
                assess_restore(inconsistent, qualified(&[])),
                Err(Closed::InvalidRange)
            );
        }
        witness.push((
            "reversed_or_cross_epoch_acknowledgement_refuses",
            "closed:InvalidRange".to_owned(),
        ));

        for (observation, expected) in [
            (HeadObservation::MissingJournal, Closed::MissingJournal),
            (HeadObservation::MissingHead, Closed::MissingHead),
            (HeadObservation::MissingKey, Closed::MissingKey),
            (HeadObservation::Unqualified, Closed::Unqualified),
            (HeadObservation::Incomplete, Closed::Incomplete),
        ] {
            assert_eq!(assess_restore(old, observation), Err(expected));
        }
        witness.push((
            "missing_or_unverified_latest_head_closes",
            "closed:MissingJournal,MissingHead,MissingKey,Unqualified,Incomplete".to_owned(),
        ));

        assert!(
            restored
                .actions
                .contains(&OverlayAction::RetireNamespace { epoch: epoch(7) })
        );
        assert_eq!(
            retry_disposition(&restored, epoch(7), true),
            RetryDisposition::RetainedLookupOnly
        );
        assert_eq!(
            retry_disposition(&restored, epoch(7), false),
            RetryDisposition::ExpiredOrIndeterminate
        );
        assert_eq!(
            retry_disposition(&restored, epoch(8), false),
            RetryDisposition::CurrentNamespaceCandidate
        );
        witness.push((
            "old_revision_namespaces_are_lookup_only",
            "retired:7;retained:lookup_only;missing:expired_or_indeterminate;new:candidate_only"
                .to_owned(),
        ));

        for retained_receipt in [false, true] {
            assert_eq!(
                retry_disposition(&restored, epoch(9), retained_receipt),
                RetryDisposition::ExpiredOrIndeterminate
            );
        }
        witness.push((
            "future_namespace_never_authorized_by_retained_receipt",
            "expired_or_indeterminate".to_owned(),
        ));

        let protected_entries = [
            JournalEntry {
                sequence: 1,
                kind: IrreversibleKind::Suppress {
                    identity: "subject-A",
                    kind: Suppression::Deletion,
                },
            },
            send(2, operation(1), SendKnowledge::Unknown),
        ];
        let protected = assess_restore(old, qualified(&protected_entries)).unwrap();
        assert_eq!(protected.lost_game_range, restored.lost_game_range);
        assert_eq!(protected.held_liability, Some(money(900)));
        assert_eq!(
            protected.actions,
            vec![
                OverlayAction::Suppress {
                    identity: "subject-A",
                    kind: Suppression::Deletion,
                },
                OverlayAction::HoldSend {
                    intent: operation(1),
                    maximum: money(900),
                },
                OverlayAction::RetireNamespace { epoch: epoch(7) },
            ]
        );
        witness.push((
            "unknown_send_and_suppression_survive_game_rollback",
            format!(
                "{};suppression_first;unknown_held:900",
                disclosed(&protected)
            ),
        ));

        let maximum_epoch = OlderSnapshot {
            revision: revision(u64::MAX, 0),
            latest_acknowledged_revision: revision(u64::MAX, 0),
            ..old
        };
        let exhausted = HeadObservation::QualifiedFixture(QualifiedFixture {
            epoch: epoch(u64::MAX),
            published_head: 0,
            off_host_watermark: 0,
            journal_head: 0,
            entries: &[],
        });
        assert_eq!(
            assess_restore(maximum_epoch, exhausted),
            Err(Closed::EpochExhausted)
        );
        let complete_maximum_sequence = OlderSnapshot {
            revision: revision(7, u64::MAX),
            latest_acknowledged_revision: revision(7, u64::MAX),
            ..old
        };
        assert_eq!(
            complete_maximum_sequence.revision.next_sequence(),
            Err(df_types::RevisionError::SequenceOverflow)
        );
        let next_epoch = assess_restore(complete_maximum_sequence, qualified(&[])).unwrap();
        assert_eq!(next_epoch.candidate_epoch, epoch(8));
        assert_eq!(next_epoch.lost_game_range, None);
        witness.push((
            "exhausted_epoch_and_sequence_boundary_refuse",
            "epoch:EpochExhausted;sequence:SequenceOverflow;complete:max_sequence_no_loss"
                .to_owned(),
        ));

        for (published_head, off_host_watermark, journal_head) in [(1, 0, 1), (0, 1, 0), (1, 1, 0)]
        {
            let conflicting = HeadObservation::QualifiedFixture(QualifiedFixture {
                epoch: epoch(7),
                published_head,
                off_host_watermark,
                journal_head,
                entries: &[],
            });
            assert_eq!(assess_restore(old, conflicting), Err(Closed::HeadConflict));
        }
        witness.push((
            "regressed_or_forked_head_never_reopens",
            "closed:HeadConflict;epoch_CAS:unperformed".to_owned(),
        ));

        let actual = format!(
            "[\n{}\n]\n",
            witness
                .iter()
                .map(|(name, result)| format!("  {{\"case\":\"{name}\",\"result\":\"{result}\"}}"))
                .collect::<Vec<_>>()
                .join(",\n")
        );
        assert_eq!(
            actual,
            include_str!("../tests/recovery_epoch_loss_disclosure_contract.json")
        );
    }

    #[test]
    fn recovery_epoch_view_contract() {
        let old = snapshot();
        let candidate = assess_restore(old, qualified(&[])).unwrap();
        assert_eq!(candidate.candidate_epoch, epoch(8));
        assert_eq!(
            candidate.lost_game_range,
            Some(LostGameRange {
                from: revision(7, 11),
                through: revision(7, 12),
            })
        );

        let complete = OlderSnapshot {
            latest_acknowledged_revision: old.revision,
            ..old
        };
        assert_eq!(
            assess_restore(complete, qualified(&[]))
                .unwrap()
                .lost_game_range,
            None
        );
        // Even an exact acknowledged boundary cannot bless missing history.
        assert_eq!(
            assess_restore(complete, HeadObservation::MissingHead),
            Err(Closed::MissingHead)
        );

        let maximum_old = OlderSnapshot {
            latest_acknowledged_revision: revision(7, u64::MAX),
            ..old
        };
        let maximum_candidate = assess_restore(maximum_old, qualified(&[])).unwrap();
        assert!(
            SessionRevision::new(maximum_candidate.candidate_epoch, 0)
                > maximum_old.latest_acknowledged_revision
        );
        assert_eq!(old.revision.next_sequence().unwrap(), revision(7, 11));

        for (observation, refusal) in [
            (HeadObservation::MissingJournal, Closed::MissingJournal),
            (HeadObservation::MissingHead, Closed::MissingHead),
            (HeadObservation::MissingKey, Closed::MissingKey),
            (HeadObservation::Unqualified, Closed::Unqualified),
            (HeadObservation::Incomplete, Closed::Incomplete),
        ] {
            assert_eq!(assess_restore(old, observation), Err(refusal));
        }
        for (published_head, off_host_watermark, journal_head) in [(1, 0, 1), (0, 1, 0)] {
            assert_eq!(
                assess_restore(
                    old,
                    HeadObservation::QualifiedFixture(QualifiedFixture {
                        epoch: epoch(7),
                        published_head,
                        off_host_watermark,
                        journal_head,
                        entries: &[],
                    }),
                ),
                Err(Closed::HeadConflict)
            );
        }
        for acknowledged in [revision(7, 9), revision(6, 12), revision(8, 12)] {
            assert_eq!(
                assess_restore(
                    OlderSnapshot {
                        latest_acknowledged_revision: acknowledged,
                        ..old
                    },
                    qualified(&[]),
                ),
                Err(Closed::InvalidRange)
            );
        }

        let intent = operation(1);
        let entries = [
            send(1, intent, SendKnowledge::Unknown),
            JournalEntry {
                sequence: 2,
                kind: IrreversibleKind::Suppress {
                    identity: "removed-record",
                    kind: Suppression::Deletion,
                },
            },
        ];
        let protected = assess_restore(old, qualified(&entries)).unwrap();
        assert_eq!(protected.lost_game_range, candidate.lost_game_range);
        assert_eq!(protected.held_liability, Some(money(900)));
        assert_eq!(
            protected.actions,
            vec![
                OverlayAction::Suppress {
                    identity: "removed-record",
                    kind: Suppression::Deletion,
                },
                OverlayAction::HoldSend {
                    intent,
                    maximum: money(900),
                },
                OverlayAction::RetireNamespace { epoch: epoch(7) },
            ]
        );
        assert_eq!(
            retry_disposition(&protected, epoch(7), true),
            RetryDisposition::RetainedLookupOnly
        );
        assert_eq!(
            retry_disposition(&protected, epoch(7), false),
            RetryDisposition::ExpiredOrIndeterminate
        );
        for retained in [false, true] {
            assert_eq!(
                retry_disposition(&protected, epoch(9), retained),
                RetryDisposition::ExpiredOrIndeterminate
            );
        }

        let exhausted = OlderSnapshot {
            covered_head: 0,
            revision: revision(u64::MAX, 0),
            latest_acknowledged_revision: revision(u64::MAX, 0),
        };
        assert_eq!(
            assess_restore(
                exhausted,
                HeadObservation::QualifiedFixture(QualifiedFixture {
                    epoch: epoch(u64::MAX),
                    published_head: 0,
                    off_host_watermark: 0,
                    journal_head: 0,
                    entries: &[],
                }),
            ),
            Err(Closed::EpochExhausted)
        );

        // Revision counts contain no elapsed backup/restore measurement. The witness
        // states the proposed targets and missing drill instead of deriving seconds.
        let witness = [
            ("OriginalAcceptance1", "game RPO honest"),
            (
                "OriginalAcceptance2",
                "The named outcome has actual source/build-bound evidence; unsupported, pending, failed and unperformed checks remain explicit.",
            ),
            (
                "decision",
                "disclose exact lost acknowledged game revisions under qualified fixture; recovery epoch remains a candidate",
            ),
            (
                "rejected_alternatives",
                "revision count as elapsed RPO, guessed no-loss, old backup as unsent proof, reused old command namespace and fixture as epoch issuance",
            ),
            ("exact_loss_view", "loss:7:11..7:12;candidate:8:0"),
            (
                "qualified_no_acknowledged_loss",
                "no_loss:7:10;missing_head:closed",
            ),
            (
                "composite_order_and_ordinary_restart",
                "8:0>7:max;ordinary_next:7:11",
            ),
            (
                "missing_conflicting_or_invalid_history",
                "closed:MissingJournal,MissingHead,MissingKey,Unqualified,Incomplete,HeadConflict,InvalidRange",
            ),
            (
                "suppression_and_unknown_survive_game_rollback",
                "suppression_first;hold:900;loss:7:11..7:12;retire:7;no_resend_or_refund",
            ),
            (
                "old_and_future_namespace",
                "old_retained:lookup_only;old_missing:expired_or_indeterminate;future:expired_or_indeterminate",
            ),
            ("exhausted_epoch", "closed:EpochExhausted"),
            (
                "elapsed_RPO_and_RTO",
                "proposed_RPO300seconds_RTO3600seconds;unmeasured;revision_range_is_not_elapsed_time",
            ),
            (
                "unresolved",
                "authenticated_latest_head,protected_backing,key_custody,epoch_CAS,mounted_restored_view,backup_PITR_and_restore_drill:unperformed;no_issued_epoch_or_served_PG",
            ),
        ];
        let actual = format!(
            "[\n{}\n]\n",
            witness
                .iter()
                .map(|(name, result)| format!("  {{\"case\":\"{name}\",\"result\":\"{result}\"}}"))
                .collect::<Vec<_>>()
                .join(",\n")
        );
        assert_eq!(
            actual,
            include_str!("../tests/recovery_epoch_view_contract.json")
        );
    }
}
