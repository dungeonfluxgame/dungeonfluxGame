use df_commerce::{
    Access, AllowanceGrantId, AllowancePaymentObservation, AllowanceRefusal, AllowanceRenewal,
    AllowanceState, AllowanceTerms, ObservationAuthority, PaidInvoice, PaidPeriod, PaymentOutcome,
    ScheduledDowngrade, SubscriptionObservation, SubscriptionRefusal, SubscriptionState,
    effective_allowance_terms, propose_allowance_downgrade, propose_allowance_renewal,
};
use df_types::{Currency, Money, MoneyError, PaidInvoiceId, PriceVersion, SubscriptionId};

fn money(micro_dollars: u128) -> Money {
    Money::new(Currency::parse("USD").unwrap(), micro_dollars)
}

fn terms(version: u8, amount: u128) -> AllowanceTerms {
    AllowanceTerms {
        price_version: PriceVersion::from_bytes(&[version; 16]).unwrap(),
        supplier_allowance: money(amount),
    }
}

fn pending() -> AllowanceState {
    AllowanceState {
        subscription: SubscriptionId::from_bytes(&[1; 16]).unwrap(),
        revision: 5,
        subscription_state: SubscriptionState {
            currency: money(0).currency(),
            revision: 7,
            object_revision: 4,
            access: Access::PendingInitial,
            paid_through: 0,
            grace_until: None,
        },
        current_terms: terms(3, 80),
        current_grant: None,
        scheduled_downgrade: None,
    }
}

fn payment(
    state: AllowanceState,
    invoice: u8,
    period: PaidPeriod,
    selected: AllowanceTerms,
) -> (AllowanceRenewal, AllowancePaymentObservation) {
    let grant = AllowanceGrantId {
        subscription: state.subscription,
        invoice: PaidInvoiceId::from_bytes(&[invoice; 16]).unwrap(),
        period,
        price_version: selected.price_version,
    };
    (
        AllowanceRenewal {
            expected_revision: state.revision,
            grant,
        },
        AllowancePaymentObservation {
            grant,
            subscription: SubscriptionObservation {
                authority: ObservationAuthority::VerifiedLatest,
                expected_revision: state.subscription_state.revision,
                object_revision: state.subscription_state.object_revision + 1,
                outcome: PaymentOutcome::Settled(PaidInvoice {
                    period_start: period.start,
                    paid_through: period.end,
                    selected_allowance: selected.supplier_allowance,
                    allowance_already_recorded: false,
                }),
            },
        },
    )
}

fn active() -> AllowanceState {
    let state = pending();
    let (request, observation) = payment(
        state,
        2,
        PaidPeriod {
            start: 90,
            end: 200,
        },
        state.current_terms,
    );
    propose_allowance_renewal(&state, request, observation, 100)
        .unwrap()
        .next
}

fn next_payment(state: AllowanceState) -> (AllowanceRenewal, AllowancePaymentObservation) {
    payment(
        state,
        4,
        PaidPeriod {
            start: 200,
            end: 400,
        },
        state.current_terms,
    )
}

#[test]
fn initial_and_renewed_paid_period_propose_one_exact_versioned_grant() {
    let initial = pending();
    let (request, observation) = payment(
        initial,
        2,
        PaidPeriod {
            start: 90,
            end: 200,
        },
        initial.current_terms,
    );
    let first = propose_allowance_renewal(&initial, request, observation, 100).unwrap();
    assert_eq!(initial, pending());
    assert_eq!(first.before_revision, 5);
    assert_eq!(first.next.revision, 6);
    assert_eq!(first.next.subscription_state.revision, 8);
    assert_eq!(first.grant.unwrap().id, request.grant);
    assert_eq!(first.grant.unwrap().supplier_allowance, money(80));
    let state = first.next;
    let (request, observation) = next_payment(state);
    let next = propose_allowance_renewal(&state, request, observation, 250).unwrap();
    assert_eq!(next.next.current_grant, Some(request.grant));
    assert_ne!(next.next.current_grant, state.current_grant);
    assert_eq!(next.grant.unwrap().supplier_allowance, money(80));
    assert_eq!(state, first.next);
}

#[test]
fn duplicate_newer_refresh_different_invoice_and_late_period_never_reset_again() {
    let state = active();
    let (request, observation) = next_payment(state);
    let accepted = propose_allowance_renewal(&state, request, observation, 250).unwrap();
    assert_eq!(
        propose_allowance_renewal(&accepted.next, request, observation, 250),
        Err(AllowanceRefusal::StaleRevision)
    );
    for invoice in [4, 9] {
        let (again, refresh) = payment(
            accepted.next,
            invoice,
            request.grant.period,
            accepted.next.current_terms,
        );
        assert_eq!(
            propose_allowance_renewal(&accepted.next, again, refresh, 250),
            Err(AllowanceRefusal::Subscription(
                SubscriptionRefusal::InvalidPeriod
            ))
        );
    }
    let (old, latest) = payment(
        accepted.next,
        2,
        PaidPeriod {
            start: 90,
            end: 200,
        },
        state.current_terms,
    );
    assert_eq!(
        propose_allowance_renewal(&accepted.next, old, latest, 250),
        Err(AllowanceRefusal::Subscription(
            SubscriptionRefusal::InvalidPeriod
        ))
    );
    assert_eq!(accepted.next.current_grant, Some(request.grant));
}

#[test]
fn recorded_invoice_refuses_even_with_an_old_snapshot_and_current_lookup() {
    let state = active();
    let (request, mut observation) = next_payment(state);
    let PaymentOutcome::Settled(mut invoice) = observation.subscription.outcome else {
        panic!("fixture")
    };
    invoice.allowance_already_recorded = true;
    observation.subscription.outcome = PaymentOutcome::Settled(invoice);
    assert_eq!(
        propose_allowance_renewal(&state, request, observation, 250),
        Err(AllowanceRefusal::Subscription(
            SubscriptionRefusal::DuplicateAllowance
        ))
    );
    assert_eq!(state, active());
}

#[test]
fn subscription_invoice_period_and_price_basis_must_match_exactly() {
    let state = active();
    let (request, observation) = next_payment(state);
    let foreign = AllowanceRenewal {
        grant: AllowanceGrantId {
            subscription: SubscriptionId::from_bytes(&[9; 16]).unwrap(),
            ..request.grant
        },
        ..request
    };
    assert_eq!(
        propose_allowance_renewal(&state, foreign, observation, 250),
        Err(AllowanceRefusal::WrongSubscription)
    );
    let wrong_lookup = AllowancePaymentObservation {
        grant: AllowanceGrantId {
            invoice: PaidInvoiceId::from_bytes(&[9; 16]).unwrap(),
            ..observation.grant
        },
        ..observation
    };
    assert_eq!(
        propose_allowance_renewal(&state, request, wrong_lookup, 250),
        Err(AllowanceRefusal::WrongInvoiceObservation)
    );
    let wrong_period = AllowanceRenewal {
        grant: AllowanceGrantId {
            period: PaidPeriod {
                start: 201,
                end: 400,
            },
            ..request.grant
        },
        ..request
    };
    assert_eq!(
        propose_allowance_renewal(
            &state,
            wrong_period,
            AllowancePaymentObservation {
                grant: wrong_period.grant,
                ..observation
            },
            250
        ),
        Err(AllowanceRefusal::WrongPeriodBasis)
    );
    let wrong_price = AllowanceRenewal {
        grant: AllowanceGrantId {
            price_version: terms(8, 80).price_version,
            ..request.grant
        },
        ..request
    };
    assert_eq!(
        propose_allowance_renewal(
            &state,
            wrong_price,
            AllowancePaymentObservation {
                grant: wrong_price.grant,
                ..observation
            },
            250
        ),
        Err(AllowanceRefusal::WrongPriceVersion)
    );
}

#[test]
fn all_local_and_object_revision_fences_refuse_without_candidate_effects() {
    let state = active();
    let (request, observation) = next_payment(state);
    assert_eq!(
        propose_allowance_renewal(
            &state,
            AllowanceRenewal {
                expected_revision: state.revision - 1,
                ..request
            },
            observation,
            250
        ),
        Err(AllowanceRefusal::StaleRevision)
    );
    for (subscription, refusal) in [
        (
            SubscriptionObservation {
                expected_revision: state.subscription_state.revision - 1,
                ..observation.subscription
            },
            SubscriptionRefusal::StaleRevision,
        ),
        (
            SubscriptionObservation {
                object_revision: state.subscription_state.object_revision,
                ..observation.subscription
            },
            SubscriptionRefusal::StaleObject,
        ),
        (
            SubscriptionObservation {
                object_revision: state.subscription_state.object_revision - 1,
                ..observation.subscription
            },
            SubscriptionRefusal::StaleObject,
        ),
    ] {
        assert_eq!(
            propose_allowance_renewal(
                &state,
                request,
                AllowancePaymentObservation {
                    subscription,
                    ..observation
                },
                250
            ),
            Err(AllowanceRefusal::Subscription(refusal))
        );
    }
    assert_eq!(state, active());
}

#[test]
fn unverified_webhook_and_unknown_payment_never_grant_or_reset() {
    let state = active();
    let (request, observation) = next_payment(state);
    for (subscription, refusal) in [
        (
            SubscriptionObservation {
                authority: ObservationAuthority::Unverified,
                ..observation.subscription
            },
            SubscriptionRefusal::Unverified,
        ),
        (
            SubscriptionObservation {
                authority: ObservationAuthority::WebhookOnly,
                ..observation.subscription
            },
            SubscriptionRefusal::NeedsAuthoritativeRefresh,
        ),
        (
            SubscriptionObservation {
                outcome: PaymentOutcome::Unknown,
                ..observation.subscription
            },
            SubscriptionRefusal::UnknownPayment,
        ),
    ] {
        assert_eq!(
            propose_allowance_renewal(
                &state,
                request,
                AllowancePaymentObservation {
                    subscription,
                    ..observation
                },
                250
            ),
            Err(AllowanceRefusal::Subscription(refusal))
        );
    }
    assert_eq!(state, active());
}

#[test]
fn renewal_failure_and_clock_progression_preserve_the_existing_grant() {
    let state = active();
    let (request, mut observation) = next_payment(state);
    observation.subscription.outcome = PaymentOutcome::RenewalFailed;
    let grace = propose_allowance_renewal(&state, request, observation, 200).unwrap();
    assert_eq!(grace.grant, None);
    assert_eq!(grace.next.current_grant, state.current_grant);
    assert_eq!(grace.next.subscription_state.access, Access::Grace);
    for now in [199, 200, 200 + 72 * 60 * 60, u64::MAX] {
        assert_eq!(
            effective_allowance_terms(&grace.next, now),
            Ok(state.current_terms)
        );
        assert_eq!(grace.next.current_grant, state.current_grant);
    }
    let (retry, mut latest) = next_payment(grace.next);
    latest.subscription.outcome = PaymentOutcome::RenewalFailed;
    let expired =
        propose_allowance_renewal(&grace.next, retry, latest, 200 + 72 * 60 * 60).unwrap();
    assert_eq!(expired.grant, None);
    assert_eq!(expired.next.current_grant, state.current_grant);
    assert_eq!(expired.next.subscription_state.access, Access::ReadOnly);
    assert_eq!(
        expired.next.subscription_state.grace_until,
        grace.next.subscription_state.grace_until
    );
}

#[test]
fn disclosed_downgrade_selects_boundary_terms_and_paid_renewal_only_once() {
    let state = active();
    let lower = terms(5, 40);
    let scheduled = propose_allowance_downgrade(&state, state.revision, lower, 150).unwrap();
    assert_eq!(scheduled.grant, None);
    assert_eq!(scheduled.next.current_grant, state.current_grant);
    assert_eq!(scheduled.next.subscription_state, state.subscription_state);
    assert_eq!(
        effective_allowance_terms(&scheduled.next, 199),
        Ok(state.current_terms)
    );
    assert_eq!(effective_allowance_terms(&scheduled.next, 200), Ok(lower));
    let (request, observation) = payment(
        scheduled.next,
        6,
        PaidPeriod {
            start: 200,
            end: 400,
        },
        lower,
    );
    let renewed = propose_allowance_renewal(&scheduled.next, request, observation, 200).unwrap();
    assert_eq!(renewed.grant.unwrap().supplier_allowance, money(40));
    assert_eq!(renewed.next.current_terms, lower);
    assert_eq!(renewed.next.scheduled_downgrade, None);
    assert_eq!(scheduled.next.current_terms, state.current_terms);
    assert_eq!(
        propose_allowance_renewal(&renewed.next, request, observation, 200),
        Err(AllowanceRefusal::StaleRevision)
    );
}

#[test]
fn unpaid_boundary_changes_effective_terms_without_releasing_or_granting() {
    let state = active();
    let lower = terms(5, 40);
    let scheduled = propose_allowance_downgrade(&state, state.revision, lower, 150)
        .unwrap()
        .next;
    let (request, mut observation) = payment(
        scheduled,
        6,
        PaidPeriod {
            start: 200,
            end: 400,
        },
        lower,
    );
    observation.subscription.outcome = PaymentOutcome::Unknown;
    assert_eq!(
        propose_allowance_renewal(&scheduled, request, observation, 200),
        Err(AllowanceRefusal::Subscription(
            SubscriptionRefusal::UnknownPayment
        ))
    );
    observation.subscription.outcome = PaymentOutcome::RenewalFailed;
    let grace = propose_allowance_renewal(&scheduled, request, observation, 200).unwrap();
    assert_eq!(grace.grant, None);
    assert_eq!(grace.next.current_grant, state.current_grant);
    assert_eq!(effective_allowance_terms(&grace.next, 200), Ok(lower));
    let (old_price, old_invoice) = next_payment(grace.next);
    assert_eq!(
        propose_allowance_renewal(&grace.next, old_price, old_invoice, 250),
        Err(AllowanceRefusal::WrongPriceVersion)
    );
}

#[test]
fn invoice_allowance_must_equal_selected_disclosed_terms() {
    let state = active();
    for amount in [0, 79, 81, u128::MAX] {
        let (request, observation) = payment(
            state,
            4,
            PaidPeriod {
                start: 200,
                end: 400,
            },
            terms(3, amount),
        );
        assert_eq!(
            propose_allowance_renewal(&state, request, observation, 250),
            Err(AllowanceRefusal::TermsMismatch)
        );
    }
    assert_eq!(state, active());
}

#[test]
fn invalid_unstarted_overlapping_and_expired_periods_refuse_without_effects() {
    let state = active();
    for period in [
        PaidPeriod {
            start: 400,
            end: 400,
        },
        PaidPeriod {
            start: 251,
            end: 400,
        },
        PaidPeriod {
            start: 199,
            end: 400,
        },
        PaidPeriod {
            start: 200,
            end: 250,
        },
        PaidPeriod {
            start: 400,
            end: 200,
        },
    ] {
        let (request, observation) = payment(state, 4, period, state.current_terms);
        assert_eq!(
            propose_allowance_renewal(&state, request, observation, 250),
            Err(AllowanceRefusal::Subscription(
                SubscriptionRefusal::InvalidPeriod
            ))
        );
    }
    assert_eq!(state, active());
}

#[test]
fn currency_and_revision_overflow_refuse_and_exact_amount_boundaries_remain_valid() {
    for amount in [0, u128::MAX] {
        let state = AllowanceState {
            current_terms: terms(3, amount),
            ..pending()
        };
        let (request, observation) = payment(
            state,
            2,
            PaidPeriod {
                start: 90,
                end: 200,
            },
            state.current_terms,
        );
        assert_eq!(
            propose_allowance_renewal(&state, request, observation, 100)
                .unwrap()
                .grant
                .unwrap()
                .supplier_allowance,
            money(amount)
        );
    }
    let exhausted = AllowanceState {
        revision: u64::MAX,
        ..pending()
    };
    let (request, observation) = payment(
        exhausted,
        2,
        PaidPeriod {
            start: 90,
            end: 200,
        },
        exhausted.current_terms,
    );
    assert_eq!(
        propose_allowance_renewal(&exhausted, request, observation, 100),
        Err(AllowanceRefusal::RevisionOverflow)
    );
    let state = AllowanceState {
        subscription_state: SubscriptionState {
            revision: u64::MAX,
            ..pending().subscription_state
        },
        ..pending()
    };
    let (request, observation) = payment(
        state,
        2,
        PaidPeriod {
            start: 90,
            end: 200,
        },
        state.current_terms,
    );
    assert_eq!(
        propose_allowance_renewal(&state, request, observation, 100),
        Err(AllowanceRefusal::Subscription(
            SubscriptionRefusal::RevisionOverflow
        ))
    );
    let state = active();
    let foreign = AllowanceTerms {
        supplier_allowance: Money::new(Currency::parse("EUR").unwrap(), 40),
        ..terms(5, 40)
    };
    assert_eq!(
        propose_allowance_downgrade(&state, state.revision, foreign, 150),
        Err(AllowanceRefusal::Money(MoneyError::CurrencyMismatch))
    );
    let (request, observation) = payment(
        state,
        4,
        PaidPeriod {
            start: 200,
            end: 400,
        },
        foreign,
    );
    assert_eq!(
        propose_allowance_renewal(&state, request, observation, 250),
        Err(AllowanceRefusal::Subscription(SubscriptionRefusal::Money(
            MoneyError::CurrencyMismatch
        )))
    );
}

#[test]
fn downgrade_refuses_stale_duplicate_increasing_unchanged_and_elapsed_requests() {
    let state = active();
    let lower = terms(5, 40);
    assert_eq!(
        propose_allowance_downgrade(&state, state.revision - 1, lower, 150),
        Err(AllowanceRefusal::StaleRevision)
    );
    for invalid in [terms(5, 81), terms(3, 40)] {
        assert_eq!(
            propose_allowance_downgrade(&state, state.revision, invalid, 150),
            Err(AllowanceRefusal::InvalidDowngrade)
        );
    }
    let scheduled = propose_allowance_downgrade(&state, state.revision, lower, 150).unwrap();
    assert_eq!(
        propose_allowance_downgrade(&scheduled.next, scheduled.next.revision, lower, 150),
        Err(AllowanceRefusal::AlreadyScheduled)
    );
    assert_eq!(
        propose_allowance_downgrade(&state, state.revision, lower, 200),
        Err(AllowanceRefusal::WrongState)
    );
    let exhausted = AllowanceState {
        revision: u64::MAX,
        ..state
    };
    assert_eq!(
        propose_allowance_downgrade(&exhausted, exhausted.revision, lower, 150),
        Err(AllowanceRefusal::RevisionOverflow)
    );
    assert_eq!(state, active());
}

#[test]
fn forged_current_grants_and_downgrade_boundaries_cannot_authorize_terms() {
    let state = active();
    let current = state.current_grant.unwrap();
    for invalid in [
        AllowanceState {
            current_grant: None,
            ..state
        },
        AllowanceState {
            current_grant: Some(AllowanceGrantId {
                subscription: SubscriptionId::from_bytes(&[9; 16]).unwrap(),
                ..current
            }),
            ..state
        },
        AllowanceState {
            scheduled_downgrade: Some(ScheduledDowngrade {
                effective_from: 199,
                terms: terms(5, 40),
            }),
            ..state
        },
        AllowanceState {
            subscription_state: SubscriptionState {
                access: Access::PendingInitial,
                ..state.subscription_state
            },
            ..state
        },
    ] {
        assert_eq!(
            effective_allowance_terms(&invalid, 200),
            Err(AllowanceRefusal::InvalidState)
        );
        let (request, observation) = next_payment(invalid);
        assert_eq!(
            propose_allowance_renewal(&invalid, request, observation, 250),
            Err(AllowanceRefusal::InvalidState)
        );
    }
}

#[test]
fn initial_failure_and_restricted_state_do_not_propose_a_new_grant() {
    let state = pending();
    let (request, mut observation) = payment(
        state,
        2,
        PaidPeriod {
            start: 90,
            end: 200,
        },
        state.current_terms,
    );
    observation.subscription.outcome = PaymentOutcome::InitialFailed;
    let failed = propose_allowance_renewal(&state, request, observation, 100).unwrap();
    assert_eq!(failed.grant, None);
    assert_eq!(failed.next.current_grant, None);
    assert_eq!(failed.next.subscription_state.access, Access::NoAccess);
    let restricted = AllowanceState {
        subscription_state: SubscriptionState {
            access: Access::Restricted,
            ..active().subscription_state
        },
        ..active()
    };
    let (request, observation) = next_payment(restricted);
    assert_eq!(
        propose_allowance_renewal(&restricted, request, observation, 250),
        Err(AllowanceRefusal::Subscription(
            SubscriptionRefusal::WrongState
        ))
    );
}
