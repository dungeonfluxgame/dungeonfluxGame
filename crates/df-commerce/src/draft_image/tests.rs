use super::*;
use crate::finance_ids::{PriceVersionId, QuoteId};
use crate::ledger::CurrencyTag;
use crate::price_schedule::{PriceVersion, TariffLine};

fn account(byte: u8) -> AccountId {
    AccountId::from_bytes(&[byte; 16]).unwrap()
}

fn scope() -> Scope {
    Scope {
        account: account(1),
        draft: DraftId([2; 16]),
        draft_revision: 1,
        job: JobId([3; 16]),
        source_digest: [4; 32],
        config_digest: [5; 32],
        provider: "fixture".into(),
        model: "fixture-model".into(),
        route: "portrait".into(),
        price_version: [6; 16],
        maximum_units: vec![("image".into(), 2)],
    }
}

fn binding() -> Binding {
    Binding {
        scope: scope(),
        stage: StageId([7; 16]),
        asset: AssetId([8; 16]),
        publication: PublicationId([9; 16]),
        operation: [10; 16],
        attempt: [11; 16],
        effect: [12; 16],
        payload_digest: [13; 32],
    }
}

fn version() -> PriceVersion {
    PriceVersion {
        id: PriceVersionId::from_array([6; 16]),
        label: "fixture".into(),
        currency: CurrencyTag::USD,
        platform_rate_micro_dollars_per_hour: 0,
        markup_numerator: 3,
        markup_denominator: 2,
        proposed_pilot: false,
        tariff: vec![TariffLine {
            provider: "fixture".into(),
            model: "fixture-model".into(),
            category: "image".into(),
            context_tier_max_tokens: 0,
            nano_usd: 1_001,
            per: 1,
        }],
    }
}

fn consent() -> Consent {
    Consent {
        id: ConsentId([14; 16]),
        scope: scope(),
        cap_micro_dollars: 10,
        expires_at_ms: 1_000,
        revision: 1,
        revoked: false,
    }
}

fn quote() -> Quote {
    Quote::new(
        QuoteId::from_array([15; 16]),
        &consent(),
        binding(),
        &version(),
        100,
        900,
    )
    .unwrap()
}

fn exposure() -> Exposure {
    Exposure {
        charged_micro_dollars: 0,
        held_micro_dollars: 0,
        available_micro_dollars: 10,
    }
}

#[test]
fn source_authenticated_account_must_match_before_pricing_or_admission() {
    assert_eq!(scope().validate(account(2)), Err(Refusal::WrongAccount));
    assert_eq!(
        quote().validate_admission(account(2), &consent(), &binding(), exposure(), 200),
        Err(Refusal::WrongAccount)
    );
}

#[test]
fn maximum_uses_existing_price_version_and_rounds_both_liabilities_up() {
    assert_eq!(
        price_maximum(&scope(), &version()).unwrap(),
        Maximums {
            retail_micro_dollars: 4,
            supplier_micro_dollars: 3,
        }
    );
    let mut changed = version();
    changed.id = PriceVersionId::from_array([99; 16]);
    assert_eq!(
        price_maximum(&scope(), &changed),
        Err(Refusal::PriceVersionMismatch)
    );
    changed = version();
    changed.tariff.clear();
    assert!(matches!(
        price_maximum(&scope(), &changed),
        Err(Refusal::Unpriced(_))
    ));
}

#[test]
fn duplicate_or_noncanonical_maximum_categories_are_refused() {
    let mut changed = scope();
    changed.maximum_units.push(("image".into(), 1));
    assert_eq!(changed.validate(account(1)), Err(Refusal::InvalidUnits));
    changed.maximum_units = vec![("z".into(), 1), ("a".into(), 1)];
    assert_eq!(changed.validate(account(1)), Err(Refusal::InvalidUnits));
}

#[test]
fn binding_fingerprint_covers_every_dispatch_identity_and_payload() {
    let original = binding();
    let mut changes = vec![];
    let mut changed = original.clone();
    changed.stage.0[0] += 1;
    changes.push(changed);
    let mut changed = original.clone();
    changed.asset.0[0] += 1;
    changes.push(changed);
    let mut changed = original.clone();
    changed.publication.0[0] += 1;
    changes.push(changed);
    let mut changed = original.clone();
    changed.operation[0] += 1;
    changes.push(changed);
    let mut changed = original.clone();
    changed.attempt[0] += 1;
    changes.push(changed);
    let mut changed = original.clone();
    changed.effect[0] += 1;
    changes.push(changed);
    let mut changed = original.clone();
    changed.payload_digest[0] += 1;
    changes.push(changed);
    for changed in changes {
        assert_ne!(original.digest(), changed.digest());
    }
    assert_eq!(original.digest(), original.clone().digest());
}

#[test]
fn consent_scope_fingerprint_covers_owner_source_config_model_price_units_and_revision() {
    let original = scope();
    let mut changes = vec![];
    let mut changed = original.clone();
    changed.account = account(2);
    changes.push(changed);
    let mut changed = original.clone();
    changed.draft.0[0] += 1;
    changes.push(changed);
    let mut changed = original.clone();
    changed.draft_revision += 1;
    changes.push(changed);
    let mut changed = original.clone();
    changed.job.0[0] += 1;
    changes.push(changed);
    let mut changed = original.clone();
    changed.source_digest[0] += 1;
    changes.push(changed);
    let mut changed = original.clone();
    changed.config_digest[0] += 1;
    changes.push(changed);
    let mut changed = original.clone();
    changed.provider.push('x');
    changes.push(changed);
    let mut changed = original.clone();
    changed.model.push('x');
    changes.push(changed);
    let mut changed = original.clone();
    changed.route.push('x');
    changes.push(changed);
    let mut changed = original.clone();
    changed.price_version[0] += 1;
    changes.push(changed);
    let mut changed = original.clone();
    changed.maximum_units[0].1 += 1;
    changes.push(changed);
    for changed in changes {
        assert_ne!(original.digest(), changed.digest());
    }
}

#[test]
fn expired_or_revoked_consent_cannot_admit_a_still_pinned_quote() {
    assert_eq!(
        quote().validate_admission(account(1), &consent(), &binding(), exposure(), 1_000),
        Err(Refusal::ConsentExpired)
    );
    let mut revoked = consent();
    revoked.revoked = true;
    assert_eq!(
        quote().validate_admission(account(1), &revoked, &binding(), exposure(), 200),
        Err(Refusal::ConsentRevoked)
    );
}

#[test]
fn stale_revision_and_changed_payload_are_distinct_refusals() {
    let mut stale = consent();
    stale.revision += 1;
    assert_eq!(
        quote().validate_admission(account(1), &stale, &binding(), exposure(), 200),
        Err(Refusal::ConsentRevisionMismatch)
    );
    let mut changed = binding();
    changed.payload_digest[0] += 1;
    assert_eq!(
        quote().validate_admission(account(1), &consent(), &changed, exposure(), 200),
        Err(Refusal::QuoteMismatch)
    );
}

#[test]
fn aggregate_cap_counts_charged_money_and_full_unknown_reserves() {
    let used = Exposure {
        charged_micro_dollars: 3,
        held_micro_dollars: 4,
        available_micro_dollars: 100,
    };
    assert_eq!(
        quote().validate_admission(account(1), &consent(), &binding(), used, 200),
        Err(Refusal::ConsentCapExceeded)
    );
    let fits = Exposure {
        charged_micro_dollars: 2,
        ..used
    };
    assert_eq!(
        quote()
            .validate_admission(account(1), &consent(), &binding(), fits, 200)
            .unwrap(),
        quote().maximums()
    );
}

#[test]
fn insufficient_account_balance_cannot_be_replaced_by_consent_capacity() {
    let poor = Exposure {
        available_micro_dollars: 3,
        ..exposure()
    };
    assert_eq!(
        quote().validate_admission(account(1), &consent(), &binding(), poor, 200),
        Err(Refusal::InsufficientFunds)
    );
}

#[test]
fn quote_cannot_outlive_consent_and_expires_at_its_exact_deadline() {
    assert_eq!(
        Quote::new(
            QuoteId::from_array([15; 16]),
            &consent(),
            binding(),
            &version(),
            100,
            1_001
        ),
        Err(Refusal::QuoteExpired)
    );
    assert_eq!(
        quote().validate_admission(account(1), &consent(), &binding(), exposure(), 900),
        Err(Refusal::QuoteExpired)
    );
}

#[test]
fn aggregate_arithmetic_overflow_refuses_without_wrapping_the_cap() {
    let enormous = Exposure {
        charged_micro_dollars: u64::MAX,
        ..exposure()
    };
    assert_eq!(
        quote().validate_admission(account(1), &consent(), &binding(), enormous, 200),
        Err(Refusal::ConsentCapExceeded)
    );
}

#[test]
fn closed_serialized_scope_rejects_unknown_fields_and_zero_account() {
    let encoded = serde_json::to_value(scope()).unwrap();
    assert_eq!(
        serde_json::from_value::<Scope>(encoded.clone()).unwrap(),
        scope()
    );
    let mut unknown = encoded.clone();
    unknown["secret"] = serde_json::json!(true);
    assert!(serde_json::from_value::<Scope>(unknown).is_err());
    let mut wrong = encoded;
    wrong["account"] = serde_json::json!(vec![0; 16]);
    assert!(serde_json::from_value::<Scope>(wrong).is_err());
}

#[test]
fn changing_cap_or_expiry_without_a_new_consent_revision_cannot_reuse_the_quote() {
    let mut changed = consent();
    changed.cap_micro_dollars += 1;
    assert_eq!(
        quote().validate_admission(account(1), &changed, &binding(), exposure(), 200),
        Err(Refusal::ConsentRevisionMismatch)
    );
    changed = consent();
    changed.expires_at_ms += 1;
    assert_eq!(
        quote().validate_admission(account(1), &changed, &binding(), exposure(), 200),
        Err(Refusal::ConsentRevisionMismatch)
    );
}
