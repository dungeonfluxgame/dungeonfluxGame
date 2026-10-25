use df_commerce::{
    SpendConsent, SpendCounter, SpendOperationObservation, SpendOperationStatus, SpendProposal,
    SpendRefusal, SpendRequest, SpendScope, SpendSnapshot, propose_spend,
};
use df_types::{Currency, LiabilityRate, Money, MoneyError, OperationId, Usage, UsageUnit};

const SCOPES: [SpendScope; 6] = [
    SpendScope::PlatformDay,
    SpendScope::PlatformMonth,
    SpendScope::SupplierAccount,
    SpendScope::TenantPayer,
    SpendScope::Campaign,
    SpendScope::Job,
];

fn money(micro_dollars: u128) -> Money {
    Money::new(Currency::parse("USD").unwrap(), micro_dollars)
}

fn snapshot() -> SpendSnapshot {
    SpendSnapshot {
        currency: money(0).currency(),
        revision: 9,
        counters: [SpendCounter {
            used: money(20),
            limit: money(100),
        }; 6],
    }
}

fn consent() -> SpendConsent {
    SpendConsent {
        revision: 7,
        maximum: money(80),
    }
}

fn request() -> SpendRequest {
    SpendRequest {
        operation: OperationId::from_bytes(&[1; 16]).unwrap(),
        expected_revision: 9,
        expected_consent_revision: 7,
        maximum_supplier_liability: money(80),
    }
}

fn unseen() -> SpendOperationObservation {
    SpendOperationObservation {
        operation: request().operation,
        status: SpendOperationStatus::Unseen,
    }
}

fn refusal_without_effects(
    state: SpendSnapshot,
    grant: SpendConsent,
    admission: SpendRequest,
    observation: SpendOperationObservation,
    expected: SpendRefusal,
) {
    let before = state;
    let before_grant = grant;
    let before_admission = admission;
    let before_observation = observation;
    assert_eq!(
        propose_spend(&state, grant, admission, observation),
        Err(expected)
    );
    assert_eq!(state, before);
    assert_eq!(grant, before_grant);
    assert_eq!(admission, before_admission);
    assert_eq!(observation, before_observation);
}

#[test]
fn exact_boundary_proposal_updates_every_hierarchy_without_mutation() {
    let state = snapshot();
    let before = state;
    let proposal = propose_spend(&state, consent(), request(), unseen()).unwrap();
    assert_eq!(proposal.operation, request().operation);
    assert_eq!(proposal.before_revision, 9);
    assert_eq!(proposal.next.revision, 10);
    assert_eq!(proposal.consent_revision, 7);
    assert_eq!(proposal.consented_maximum, money(80));
    assert_eq!(proposal.maximum_supplier_liability, money(80));
    assert!(
        proposal
            .next
            .counters
            .iter()
            .all(|counter| { counter.used == money(100) && counter.limit == money(100) })
    );
    assert_eq!(state, before);
}

#[test]
fn each_day_month_supplier_payer_campaign_and_job_cap_independently_refuses() {
    for (index, scope) in SCOPES.into_iter().enumerate() {
        let mut state = snapshot();
        state.counters[index].limit = money(99);
        refusal_without_effects(
            state,
            consent(),
            request(),
            unseen(),
            SpendRefusal::BudgetExceeded(scope),
        );
    }
}

#[test]
fn existing_unknown_exposure_counts_and_second_campaign_cannot_collectively_overspend() {
    let mut state = snapshot();
    // The 91 includes held Unknown liability; cancellation/expiry cannot remove it.
    state.counters[0].used = money(91);
    let admission = SpendRequest {
        maximum_supplier_liability: money(10),
        ..request()
    };
    refusal_without_effects(
        state,
        consent(),
        admission,
        unseen(),
        SpendRefusal::BudgetExceeded(SpendScope::PlatformDay),
    );
    let first = propose_spend(&snapshot(), consent(), request(), unseen()).unwrap();
    let second = SpendRequest {
        operation: OperationId::from_bytes(&[2; 16]).unwrap(),
        expected_revision: first.next.revision,
        maximum_supplier_liability: money(1),
        ..request()
    };
    refusal_without_effects(
        first.next,
        consent(),
        second,
        SpendOperationObservation {
            operation: second.operation,
            status: SpendOperationStatus::Unseen,
        },
        SpendRefusal::BudgetExceeded(SpendScope::PlatformDay),
    );
}

#[test]
fn stale_duplicate_unknown_and_wrong_operation_never_propose_new_liability() {
    for (admission, observation, expected) in [
        (
            SpendRequest {
                expected_revision: 8,
                ..request()
            },
            unseen(),
            SpendRefusal::StaleRevision,
        ),
        (
            SpendRequest {
                expected_consent_revision: 6,
                ..request()
            },
            unseen(),
            SpendRefusal::StaleConsent,
        ),
        (
            request(),
            SpendOperationObservation {
                operation: OperationId::from_bytes(&[2; 16]).unwrap(),
                ..unseen()
            },
            SpendRefusal::WrongOperationObservation,
        ),
        (
            request(),
            SpendOperationObservation {
                status: SpendOperationStatus::Recorded,
                ..unseen()
            },
            SpendRefusal::DuplicateOperation,
        ),
        (
            request(),
            SpendOperationObservation {
                status: SpendOperationStatus::Unknown,
                ..unseen()
            },
            SpendRefusal::UnknownOperation,
        ),
    ] {
        refusal_without_effects(snapshot(), consent(), admission, observation, expected);
    }
}

#[test]
fn consent_and_currency_mismatches_refuse_before_overflow() {
    let foreign = Money::new(Currency::parse("EUR").unwrap(), u128::MAX);
    refusal_without_effects(
        snapshot(),
        SpendConsent {
            maximum: money(79),
            ..consent()
        },
        request(),
        unseen(),
        SpendRefusal::CustomerConsentExceeded,
    );
    refusal_without_effects(
        snapshot(),
        consent(),
        SpendRequest {
            maximum_supplier_liability: foreign,
            ..request()
        },
        unseen(),
        SpendRefusal::Money(MoneyError::CurrencyMismatch),
    );
    refusal_without_effects(
        snapshot(),
        SpendConsent {
            maximum: foreign,
            ..consent()
        },
        request(),
        unseen(),
        SpendRefusal::Money(MoneyError::CurrencyMismatch),
    );
    for index in 0..6 {
        for foreign_used in [true, false] {
            let mut state = snapshot();
            state.counters[0].used = money(u128::MAX);
            if foreign_used {
                state.counters[index].used = foreign;
            } else {
                state.counters[index].limit = foreign;
            }
            refusal_without_effects(
                state,
                consent(),
                request(),
                unseen(),
                SpendRefusal::Money(MoneyError::CurrencyMismatch),
            );
        }
    }
}

#[test]
fn arithmetic_and_revision_overflow_return_no_partial_hierarchy() {
    for index in 0..6 {
        let mut state = snapshot();
        state.counters[index] = SpendCounter {
            used: money(u128::MAX),
            limit: money(u128::MAX),
        };
        refusal_without_effects(
            state,
            consent(),
            request(),
            unseen(),
            SpendRefusal::Money(MoneyError::Overflow),
        );
    }
    refusal_without_effects(
        SpendSnapshot {
            revision: u64::MAX,
            ..snapshot()
        },
        consent(),
        SpendRequest {
            expected_revision: u64::MAX,
            ..request()
        },
        unseen(),
        SpendRefusal::RevisionOverflow,
    );
}

#[test]
fn rounded_up_shared_rate_and_zero_liability_preserve_exact_units() {
    let maximum = LiabilityRate::new(money(0).currency(), UsageUnit::Token, 2, 3)
        .unwrap()
        .liability(Usage::new(5, UsageUnit::Token))
        .unwrap();
    let proposal: SpendProposal = propose_spend(
        &snapshot(),
        consent(),
        SpendRequest {
            maximum_supplier_liability: maximum,
            ..request()
        },
        unseen(),
    )
    .unwrap();
    assert_eq!(proposal.maximum_supplier_liability, money(4));
    assert!(
        proposal
            .next
            .counters
            .iter()
            .all(|counter| counter.used == money(24))
    );
    let state = snapshot();
    let zero = propose_spend(
        &state,
        consent(),
        SpendRequest {
            maximum_supplier_liability: money(0),
            ..request()
        },
        unseen(),
    )
    .unwrap();
    assert_eq!(zero.next.counters, state.counters);
    assert_eq!(zero.maximum_supplier_liability, money(0));
}
