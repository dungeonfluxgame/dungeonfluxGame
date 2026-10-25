use df_commerce::{
    CreditAdjustment, CreditEvent, CreditObservation, CreditReceipt, CreditRefusal, CreditState,
    CreditTransition, ObservationAuthority, OriginalCharge, RefundObligation, RefundOutcome,
    RefundStatus, SpendConsent, SpendCounter, SpendOperationObservation, SpendOperationStatus,
    SpendRefusal, SpendRequest, SpendScope, SpendSnapshot, propose_credit_adjustment,
    propose_spend,
};
use df_types::{Currency, Money, MoneyError, OperationId};

fn currency() -> Currency {
    Currency::parse("USD").unwrap()
}

fn money(amount_micros: u128) -> Money {
    Money::new(currency(), amount_micros)
}

fn operation(byte: u8) -> OperationId {
    OperationId::from_bytes(&[byte; 16]).unwrap()
}

fn state() -> CreditState {
    CreditState {
        currency: currency(),
        revision: 7,
        topup_principal: money(100),
        deferred_credit: money(70),
        earned_credit: money(20),
        refunded_principal: money(10),
        refund_hold: money(15),
    }
}

fn observation(state: CreditState) -> CreditObservation {
    CreditObservation {
        authority: ObservationAuthority::VerifiedLatest,
        expected_revision: state.revision,
        prior_receipt: None,
        refund: None,
        reversible_charge: None,
    }
}

fn receipt(event: CreditEvent) -> CreditReceipt {
    CreditReceipt {
        operation: operation(1),
        event,
    }
}

fn refund() -> RefundObligation {
    RefundObligation {
        operation: operation(2),
        held: money(15),
        object_revision: 4,
        status: RefundStatus::Unknown,
    }
}

fn refund_event(outcome: RefundOutcome) -> CreditEvent {
    CreditEvent::ObserveRefund {
        refund_operation: operation(2),
        held: money(15),
        object_revision: 5,
        outcome,
    }
}

fn conserved(next: CreditState) {
    assert_eq!(
        next.topup_principal,
        next.deferred_credit
            .checked_add(next.earned_credit)
            .unwrap()
            .checked_add(next.refunded_principal)
            .unwrap()
    );
    assert!(next.refund_hold.micros() <= next.deferred_credit.micros());
}

#[test]
fn settled_topup_is_deferred_principal_and_never_free_income() {
    let before = state();
    let request = receipt(CreditEvent::SettledTopup { amount: money(1) });
    let proposal = propose_credit_adjustment(before, request, observation(before)).unwrap();
    assert_eq!(proposal.next.topup_principal, money(101));
    assert_eq!(proposal.next.deferred_credit, money(71));
    assert_eq!(proposal.next.earned_credit, before.earned_credit);
    assert_eq!(proposal.next.refunded_principal, before.refunded_principal);
    assert_eq!(proposal.next.refund_hold, before.refund_hold);
    assert_eq!(proposal.receipt, request);
    assert_eq!(
        proposal.adjustment,
        CreditAdjustment::TopupDeferred { amount: money(1) }
    );
    assert_eq!(proposal.before_revision, 7);
    assert_eq!(proposal.next.revision, 8);
    conserved(proposal.next);
    assert_eq!(before, state());
}

#[test]
fn consumption_only_recognizes_available_deferred_principal() {
    let before = state();
    let proposal = propose_credit_adjustment(
        before,
        receipt(CreditEvent::Consume { amount: money(55) }),
        observation(before),
    )
    .unwrap();
    assert_eq!(proposal.next.deferred_credit, money(15));
    assert_eq!(proposal.next.earned_credit, money(75));
    assert_eq!(proposal.next.refund_hold, money(15));
    conserved(proposal.next);
    assert_eq!(
        propose_credit_adjustment(
            before,
            receipt(CreditEvent::Consume { amount: money(56) }),
            observation(before)
        ),
        Err(CreditRefusal::InsufficientAvailableCredit)
    );
    assert_eq!(before, state());
}

#[test]
fn reversals_append_an_original_operation_bound_adjustment() {
    let before = state();
    let facts = CreditObservation {
        reversible_charge: Some(OriginalCharge {
            operation: operation(3),
            remaining_reversible: money(12),
        }),
        ..observation(before)
    };
    let event = CreditEvent::ReverseConsumption {
        original_operation: operation(3),
        amount: money(12),
    };
    let proposal = propose_credit_adjustment(before, receipt(event), facts).unwrap();
    assert_eq!(proposal.next.deferred_credit, money(82));
    assert_eq!(proposal.next.earned_credit, money(8));
    assert_eq!(proposal.next.refund_hold, money(15));
    assert_eq!(
        proposal.adjustment,
        CreditAdjustment::ConsumptionReversed {
            original_operation: operation(3),
            amount: money(12),
        }
    );
    conserved(proposal.next);
    assert_eq!(
        propose_credit_adjustment(before, receipt(event), observation(before)),
        Err(CreditRefusal::MissingOriginalCharge)
    );
    assert_eq!(
        propose_credit_adjustment(
            before,
            receipt(event),
            CreditObservation {
                reversible_charge: Some(OriginalCharge {
                    operation: operation(4),
                    remaining_reversible: money(12),
                }),
                ..facts
            }
        ),
        Err(CreditRefusal::WrongOriginalCharge)
    );
    assert_eq!(
        propose_credit_adjustment(
            before,
            receipt(event),
            CreditObservation {
                reversible_charge: Some(OriginalCharge {
                    remaining_reversible: money(11),
                    ..facts.reversible_charge.unwrap()
                }),
                ..facts
            }
        ),
        Err(CreditRefusal::ExceedsOriginalCharge)
    );
}

#[test]
fn refund_admission_holds_only_unused_credit_and_keeps_other_holds() {
    let before = state();
    let proposal = propose_credit_adjustment(
        before,
        receipt(CreditEvent::HoldRefund { amount: money(40) }),
        observation(before),
    )
    .unwrap();
    assert_eq!(proposal.next.refund_hold, money(55));
    assert_eq!(proposal.next.deferred_credit, before.deferred_credit);
    assert_eq!(proposal.next.earned_credit, before.earned_credit);
    assert_eq!(proposal.next.refunded_principal, before.refunded_principal);
    assert_eq!(
        proposal.adjustment,
        CreditAdjustment::RefundHeld(RefundObligation {
            operation: operation(1),
            held: money(40),
            object_revision: 0,
            status: RefundStatus::Pending,
        })
    );
    assert_eq!(
        propose_credit_adjustment(
            proposal.next,
            CreditReceipt {
                operation: operation(4),
                event: CreditEvent::HoldRefund { amount: money(16) }
            },
            observation(proposal.next)
        ),
        Err(CreditRefusal::InsufficientAvailableCredit)
    );
    assert_eq!(
        propose_credit_adjustment(
            before,
            receipt(CreditEvent::HoldRefund { amount: money(40) }),
            CreditObservation {
                refund: Some(refund()),
                ..observation(before)
            }
        ),
        Err(CreditRefusal::WrongRefund)
    );
    conserved(proposal.next);
}

#[test]
fn unknown_refund_retains_full_hold_and_stale_reconciliation_cannot_clear_it() {
    let before = state();
    let facts = CreditObservation {
        refund: Some(refund()),
        ..observation(before)
    };
    let proposal =
        propose_credit_adjustment(before, receipt(refund_event(RefundOutcome::Unknown)), facts)
            .unwrap();
    assert_eq!(
        proposal.next,
        CreditState {
            revision: 8,
            ..before
        }
    );
    let CreditAdjustment::RefundUnknown(outstanding) = proposal.adjustment else {
        panic!("fixture must preserve an outstanding obligation");
    };
    assert_eq!(outstanding.held, money(15));
    assert_eq!(outstanding.operation, operation(2));
    assert_eq!(outstanding.object_revision, 5);
    assert_eq!(outstanding.status, RefundStatus::Unknown);
    for object_revision in [0, 4, 5] {
        let input = CreditEvent::ObserveRefund {
            refund_operation: operation(2),
            held: money(15),
            object_revision,
            outcome: RefundOutcome::KnownFailure,
        };
        assert_eq!(
            propose_credit_adjustment(
                proposal.next,
                CreditReceipt {
                    operation: operation(4),
                    event: input
                },
                CreditObservation {
                    refund: Some(outstanding),
                    ..observation(proposal.next)
                }
            ),
            Err(CreditRefusal::StaleRefundObservation)
        );
    }
    assert_eq!(
        propose_credit_adjustment(
            proposal.next,
            CreditReceipt {
                operation: operation(5),
                event: CreditEvent::Consume { amount: money(56) }
            },
            observation(proposal.next)
        ),
        Err(CreditRefusal::InsufficientAvailableCredit)
    );
}

#[test]
fn same_key_known_refund_settles_once_without_revenue_or_other_hold_release() {
    let before = CreditState {
        refund_hold: money(25),
        ..state()
    };
    for (outcome, expected_deferred, expected_refunded) in [
        (RefundOutcome::KnownSuccess, 55, 25),
        (RefundOutcome::KnownFailure, 70, 10),
    ] {
        let input = receipt(refund_event(outcome));
        let proposal = propose_credit_adjustment(
            before,
            input,
            CreditObservation {
                refund: Some(refund()),
                ..observation(before)
            },
        )
        .unwrap();
        assert_eq!(proposal.next.refund_hold, money(10));
        assert_eq!(proposal.next.deferred_credit, money(expected_deferred));
        assert_eq!(proposal.next.refunded_principal, money(expected_refunded));
        assert_eq!(proposal.next.earned_credit, before.earned_credit);
        assert_eq!(proposal.next.topup_principal, before.topup_principal);
        conserved(proposal.next);
        assert_eq!(
            propose_credit_adjustment(
                proposal.next,
                input,
                CreditObservation {
                    prior_receipt: Some(input),
                    ..observation(proposal.next)
                }
            ),
            Err(CreditRefusal::AlreadyRecorded)
        );
        let later = CreditReceipt {
            operation: operation(4),
            ..input
        };
        assert_eq!(
            propose_credit_adjustment(proposal.next, later, observation(proposal.next)),
            Err(CreditRefusal::MissingRefund)
        );
        assert_eq!(proposal.next.refund_hold, money(10));
    }
}

#[test]
fn wrong_missing_and_overheld_refund_observations_never_settle() {
    let before = state();
    let input = receipt(refund_event(RefundOutcome::KnownSuccess));
    assert_eq!(
        propose_credit_adjustment(before, input, observation(before)),
        Err(CreditRefusal::MissingRefund)
    );
    for outstanding in [
        RefundObligation {
            operation: operation(3),
            ..refund()
        },
        RefundObligation {
            held: money(14),
            ..refund()
        },
    ] {
        assert_eq!(
            propose_credit_adjustment(
                before,
                input,
                CreditObservation {
                    refund: Some(outstanding),
                    ..observation(before)
                }
            ),
            Err(CreditRefusal::WrongRefund)
        );
    }
    let insufficient = CreditState {
        refund_hold: money(14),
        ..before
    };
    assert_eq!(
        propose_credit_adjustment(
            insufficient,
            input,
            CreditObservation {
                refund: Some(refund()),
                ..observation(insufficient)
            }
        ),
        Err(CreditRefusal::WrongRefund)
    );
    assert_eq!(before, state());
}

#[test]
fn old_receipts_conflicts_and_foreign_operation_lookups_cannot_credit_again() {
    let before = state();
    let input = receipt(CreditEvent::SettledTopup { amount: money(20) });
    for (prior, error) in [
        (input, CreditRefusal::AlreadyRecorded),
        (
            CreditReceipt {
                event: CreditEvent::SettledTopup { amount: money(21) },
                ..input
            },
            CreditRefusal::ConflictingOperation,
        ),
        (
            CreditReceipt {
                operation: operation(2),
                ..input
            },
            CreditRefusal::WrongReceipt,
        ),
    ] {
        assert_eq!(
            propose_credit_adjustment(
                before,
                input,
                CreditObservation {
                    prior_receipt: Some(prior),
                    ..observation(before)
                }
            ),
            Err(error)
        );
    }
    assert_eq!(before, state());
}

#[test]
fn observation_authority_and_expected_revision_are_required_for_any_credit() {
    let before = state();
    let input = receipt(CreditEvent::SettledTopup { amount: money(1) });
    for (authority, error) in [
        (ObservationAuthority::Unverified, CreditRefusal::Unverified),
        (
            ObservationAuthority::WebhookOnly,
            CreditRefusal::NeedsAuthoritativeRefresh,
        ),
    ] {
        assert_eq!(
            propose_credit_adjustment(
                before,
                input,
                CreditObservation {
                    authority,
                    ..observation(before)
                }
            ),
            Err(error)
        );
    }
    assert_eq!(
        propose_credit_adjustment(
            before,
            input,
            CreditObservation {
                expected_revision: 6,
                ..observation(before)
            }
        ),
        Err(CreditRefusal::StaleRevision)
    );
    assert_eq!(
        propose_credit_adjustment(
            before,
            receipt(CreditEvent::UnknownTopup),
            observation(before)
        ),
        Err(CreditRefusal::UnknownPayment)
    );
}

#[test]
fn state_conservation_and_currency_mismatches_fail_without_a_partial_candidate() {
    let before = state();
    let input = receipt(CreditEvent::SettledTopup { amount: money(1) });
    for bad in [
        CreditState {
            topup_principal: money(99),
            ..before
        },
        CreditState {
            refund_hold: money(71),
            ..before
        },
    ] {
        assert_eq!(
            propose_credit_adjustment(bad, input, observation(bad)),
            Err(CreditRefusal::InvalidLedger)
        );
    }
    let euro = Currency::parse("EUR").unwrap();
    let foreign = Money::new(euro, 1);
    for event in [
        CreditEvent::SettledTopup { amount: foreign },
        CreditEvent::Consume { amount: foreign },
        CreditEvent::HoldRefund { amount: foreign },
        CreditEvent::ReverseConsumption {
            original_operation: operation(3),
            amount: foreign,
        },
        CreditEvent::ObserveRefund {
            refund_operation: operation(2),
            held: foreign,
            object_revision: 5,
            outcome: RefundOutcome::KnownSuccess,
        },
    ] {
        assert_eq!(
            propose_credit_adjustment(before, receipt(event), observation(before)),
            Err(CreditRefusal::Money(MoneyError::CurrencyMismatch))
        );
    }
    for bad in [
        CreditState {
            currency: euro,
            ..before
        },
        CreditState {
            topup_principal: foreign,
            ..before
        },
        CreditState {
            deferred_credit: foreign,
            ..before
        },
        CreditState {
            earned_credit: foreign,
            ..before
        },
        CreditState {
            refunded_principal: foreign,
            ..before
        },
        CreditState {
            refund_hold: foreign,
            ..before
        },
    ] {
        assert_eq!(
            propose_credit_adjustment(bad, input, observation(bad)),
            Err(CreditRefusal::Money(MoneyError::CurrencyMismatch))
        );
    }
    assert_eq!(before, state());
}

#[test]
fn exact_arithmetic_overflow_underflow_and_zero_events_are_explicit() {
    let before = state();
    for event in [
        CreditEvent::SettledTopup { amount: money(0) },
        CreditEvent::Consume { amount: money(0) },
        CreditEvent::HoldRefund { amount: money(0) },
        CreditEvent::ReverseConsumption {
            original_operation: operation(3),
            amount: money(0),
        },
        CreditEvent::ObserveRefund {
            refund_operation: operation(2),
            held: money(0),
            object_revision: 5,
            outcome: RefundOutcome::KnownSuccess,
        },
    ] {
        assert_eq!(
            propose_credit_adjustment(before, receipt(event), observation(before)),
            Err(CreditRefusal::ZeroAmount)
        );
    }
    assert_eq!(
        propose_credit_adjustment(
            before,
            receipt(CreditEvent::SettledTopup {
                amount: money(u128::MAX)
            }),
            observation(before)
        ),
        Err(CreditRefusal::Money(MoneyError::Overflow))
    );
    assert_eq!(
        propose_credit_adjustment(
            before,
            receipt(CreditEvent::ReverseConsumption {
                original_operation: operation(3),
                amount: money(21)
            }),
            CreditObservation {
                reversible_charge: Some(OriginalCharge {
                    operation: operation(3),
                    remaining_reversible: money(21),
                }),
                ..observation(before)
            }
        ),
        Err(CreditRefusal::Money(MoneyError::Underflow))
    );
    let exhausted = CreditState {
        revision: u64::MAX,
        ..before
    };
    assert_eq!(
        propose_credit_adjustment(
            exhausted,
            receipt(CreditEvent::Consume { amount: money(1) }),
            observation(exhausted)
        ),
        Err(CreditRefusal::RevisionOverflow)
    );
    assert_eq!(exhausted.revision, u64::MAX);
}

#[test]
fn maximum_exact_principal_round_trips_through_consumption_and_reversal() {
    let empty = CreditState {
        currency: currency(),
        revision: 0,
        topup_principal: money(0),
        deferred_credit: money(0),
        earned_credit: money(0),
        refunded_principal: money(0),
        refund_hold: money(0),
    };
    let topup = propose_credit_adjustment(
        empty,
        receipt(CreditEvent::SettledTopup {
            amount: money(u128::MAX),
        }),
        observation(empty),
    )
    .unwrap();
    let consumption = CreditReceipt {
        operation: operation(3),
        event: CreditEvent::Consume {
            amount: money(u128::MAX),
        },
    };
    let earned =
        propose_credit_adjustment(topup.next, consumption, observation(topup.next)).unwrap();
    assert_eq!(earned.next.earned_credit, money(u128::MAX));
    assert_eq!(earned.next.deferred_credit, money(0));
    let reversed = propose_credit_adjustment(
        earned.next,
        CreditReceipt {
            operation: operation(4),
            event: CreditEvent::ReverseConsumption {
                original_operation: consumption.operation,
                amount: money(u128::MAX),
            },
        },
        CreditObservation {
            reversible_charge: Some(OriginalCharge {
                operation: consumption.operation,
                remaining_reversible: money(u128::MAX),
            }),
            ..observation(earned.next)
        },
    )
    .unwrap();
    assert_eq!(reversed.next.deferred_credit, money(u128::MAX));
    assert_eq!(reversed.next.earned_credit, money(0));
    conserved(reversed.next);
}

#[test]
fn foreign_original_charge_and_refund_fact_currency_cannot_be_laundered() {
    let before = state();
    let foreign = Money::new(Currency::parse("EUR").unwrap(), 15);
    assert_eq!(
        propose_credit_adjustment(
            before,
            receipt(CreditEvent::ReverseConsumption {
                original_operation: operation(3),
                amount: money(1)
            }),
            CreditObservation {
                reversible_charge: Some(OriginalCharge {
                    operation: operation(3),
                    remaining_reversible: foreign,
                }),
                ..observation(before)
            }
        ),
        Err(CreditRefusal::Money(MoneyError::CurrencyMismatch))
    );
    assert_eq!(
        propose_credit_adjustment(
            before,
            receipt(refund_event(RefundOutcome::KnownSuccess)),
            CreditObservation {
                refund: Some(RefundObligation {
                    held: foreign,
                    ..refund()
                }),
                ..observation(before)
            }
        ),
        Err(CreditRefusal::Money(MoneyError::CurrencyMismatch))
    );
}

#[test]
fn repeated_partial_reversal_uses_remaining_original_principal_and_receipt_lookup() {
    let before = state();
    let input = receipt(CreditEvent::ReverseConsumption {
        original_operation: operation(3),
        amount: money(8),
    });
    let first = propose_credit_adjustment(
        before,
        input,
        CreditObservation {
            reversible_charge: Some(OriginalCharge {
                operation: operation(3),
                remaining_reversible: money(10),
            }),
            ..observation(before)
        },
    )
    .unwrap();
    assert_eq!(
        propose_credit_adjustment(
            first.next,
            input,
            CreditObservation {
                prior_receipt: Some(input),
                ..observation(first.next)
            }
        ),
        Err(CreditRefusal::AlreadyRecorded)
    );
    let next_input = CreditReceipt {
        operation: operation(4),
        ..input
    };
    assert_eq!(
        propose_credit_adjustment(
            first.next,
            next_input,
            CreditObservation {
                reversible_charge: Some(OriginalCharge {
                    operation: operation(3),
                    remaining_reversible: money(2),
                }),
                ..observation(first.next)
            }
        ),
        Err(CreditRefusal::ExceedsOriginalCharge)
    );
}

#[test]
fn public_credit_journey_keeps_supplier_unknown_exposure_in_actual_spend_policy() {
    let supplier = SpendSnapshot {
        currency: currency(),
        revision: 9,
        counters: [SpendCounter {
            used: money(100),
            limit: money(100),
        }; 6],
    };
    let assert_supplier_held = |_: CreditTransition| {
        let request = SpendRequest {
            operation: operation(9),
            expected_revision: supplier.revision,
            expected_consent_revision: 1,
            maximum_supplier_liability: money(1),
        };
        let consent = SpendConsent {
            revision: 1,
            maximum: money(1),
        };
        assert_eq!(
            propose_spend(
                &supplier,
                consent,
                request,
                SpendOperationObservation {
                    operation: request.operation,
                    status: SpendOperationStatus::Unseen,
                }
            ),
            Err(SpendRefusal::BudgetExceeded(SpendScope::PlatformDay))
        );
        assert_eq!(
            propose_spend(
                &supplier,
                consent,
                request,
                SpendOperationObservation {
                    operation: request.operation,
                    status: SpendOperationStatus::Unknown,
                }
            ),
            Err(SpendRefusal::UnknownOperation)
        );
        assert_eq!(
            supplier.counters,
            [SpendCounter {
                used: money(100),
                limit: money(100)
            }; 6]
        );
    };
    let before = state();
    let topup = propose_credit_adjustment(
        before,
        receipt(CreditEvent::SettledTopup { amount: money(50) }),
        observation(before),
    )
    .unwrap();
    assert_supplier_held(topup);
    let reversed = propose_credit_adjustment(
        topup.next,
        CreditReceipt {
            operation: operation(4),
            event: CreditEvent::ReverseConsumption {
                original_operation: operation(3),
                amount: money(10),
            },
        },
        CreditObservation {
            reversible_charge: Some(OriginalCharge {
                operation: operation(3),
                remaining_reversible: money(10),
            }),
            ..observation(topup.next)
        },
    )
    .unwrap();
    assert_supplier_held(reversed);
    let held = propose_credit_adjustment(
        reversed.next,
        CreditReceipt {
            operation: operation(7),
            event: CreditEvent::HoldRefund { amount: money(40) },
        },
        observation(reversed.next),
    )
    .unwrap();
    assert_supplier_held(held);
    let CreditAdjustment::RefundHeld(outstanding) = held.adjustment else {
        panic!("fixture")
    };
    let event = CreditEvent::ObserveRefund {
        refund_operation: outstanding.operation,
        held: outstanding.held,
        object_revision: 1,
        outcome: RefundOutcome::Unknown,
    };
    let unknown = propose_credit_adjustment(
        held.next,
        CreditReceipt {
            operation: operation(5),
            event,
        },
        CreditObservation {
            refund: Some(outstanding),
            ..observation(held.next)
        },
    )
    .unwrap();
    assert_supplier_held(unknown);
    let CreditAdjustment::RefundUnknown(outstanding) = unknown.adjustment else {
        panic!("fixture")
    };
    for outcome in [RefundOutcome::KnownSuccess, RefundOutcome::KnownFailure] {
        let reconciled = propose_credit_adjustment(
            unknown.next,
            CreditReceipt {
                operation: operation(6),
                event: CreditEvent::ObserveRefund {
                    refund_operation: outstanding.operation,
                    held: outstanding.held,
                    object_revision: 2,
                    outcome,
                },
            },
            CreditObservation {
                refund: Some(outstanding),
                ..observation(unknown.next)
            },
        )
        .unwrap();
        assert_supplier_held(reconciled);
        conserved(reconciled.next);
    }
}

#[test]
fn small_exact_principal_space_never_recognizes_a_topup_or_releases_uncertain_refunds() {
    for deferred in 0..=12 {
        for earned in 0..=12 {
            for held in 0..=deferred {
                let before = CreditState {
                    currency: currency(),
                    revision: 1,
                    topup_principal: money(deferred + earned),
                    deferred_credit: money(deferred),
                    earned_credit: money(earned),
                    refunded_principal: money(0),
                    refund_hold: money(held),
                };
                let topup = propose_credit_adjustment(
                    before,
                    receipt(CreditEvent::SettledTopup { amount: money(1) }),
                    observation(before),
                )
                .unwrap();
                assert_eq!(topup.next.earned_credit, before.earned_credit);
                assert_eq!(topup.next.deferred_credit.micros(), deferred + 1);
                conserved(topup.next);
                if held > 0 {
                    let unknown = propose_credit_adjustment(
                        before,
                        receipt(CreditEvent::ObserveRefund {
                            refund_operation: operation(2),
                            held: money(held),
                            object_revision: 1,
                            outcome: RefundOutcome::Unknown,
                        }),
                        CreditObservation {
                            refund: Some(RefundObligation {
                                operation: operation(2),
                                held: money(held),
                                object_revision: 0,
                                status: RefundStatus::Pending,
                            }),
                            ..observation(before)
                        },
                    )
                    .unwrap();
                    assert_eq!(unknown.next.refund_hold, money(held));
                    assert_eq!(unknown.next.deferred_credit, money(deferred));
                    assert_eq!(unknown.next.earned_credit, money(earned));
                    conserved(unknown.next);
                }
            }
        }
    }
}
