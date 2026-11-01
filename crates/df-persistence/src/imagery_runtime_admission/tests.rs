//! Boundary regressions for exact route values and raw/effective usage semantics.
use super::context::route_matches;
use super::settlement::{SelectedUsage, selected};
use df_commerce::draft_image::{PaidStageKind, StageRoute};
use df_finance::draft_image::{StageUsageEvidence, Usability};
use df_imagery::cost::{CostRoute, CostUnits, UsageEvidence, UsageRevision};
use df_imagery::execution::{DispatchId, ExecutionStageId};
use df_imagery::runtime::{RuntimeError, RuntimeUsage};
use df_imagery::{ImageJobId, ImageOwner};
use df_types::AccountId;

fn route() -> StageRoute {
    StageRoute {
        key: [1; 32],
        kind: PaidStageKind::Classification,
        provider: "openai".into(),
        model: "analysis".into(),
        route: "creator".into(),
        cause: "creator_portrait".into(),
        detail: "classification".into(),
        price_version: [2; 16],
        maximum_units: vec![("text_input".into(), 100), ("text_output".into(), 50)],
    }
}
fn cost() -> CostRoute {
    CostRoute {
        provider: "openai".into(),
        model: "analysis".into(),
        cause: "creator_portrait".into(),
        detail: "classification".into(),
        maxima: vec![
            CostUnits {
                category: "text_output".into(),
                quantity: 50,
            },
            CostUnits {
                category: "text_input".into(),
                quantity: 100,
            },
        ],
    }
}
fn usage(evidence: UsageEvidence, units: Vec<CostUnits>, usable: bool) -> RuntimeUsage {
    RuntimeUsage {
        owner: ImageOwner::Draft {
            account: AccountId::from_bytes(&[3; 16]).unwrap(),
        },
        job: ImageJobId::from_bytes(&[4; 16]).unwrap(),
        stage: ExecutionStageId::from_bytes(&[5; 16]).unwrap(),
        dispatch: DispatchId::from_bytes(&[6; 16]).unwrap(),
        revision: UsageRevision(2),
        route: cost(),
        evidence,
        units,
        usable,
        at_ms: 123,
        raw_output: None,
    }
}

#[test]
fn actual_route_values_are_exact_while_category_order_is_canonical() {
    assert!(route_matches(&route(), &cost()));
    for field in 0..6 {
        let mut actual = cost();
        match field {
            0 => actual.provider = "other".into(),
            1 => actual.model = "other".into(),
            2 => actual.cause = "other".into(),
            3 => actual.detail = "other".into(),
            4 => actual.maxima[0].quantity += 1,
            _ => actual.maxima.pop().map(|_| ()).unwrap(),
        }
        assert!(!route_matches(&route(), &actual));
    }
    let mut duplicate = cost();
    duplicate.maxima[0].category = "text_input".into();
    assert!(!route_matches(&route(), &duplicate));
}

#[test]
fn missing_known_quantities_use_pinned_maxima_without_rewriting_raw_evidence() {
    for evidence in [UsageEvidence::Reported, UsageEvidence::Estimated] {
        let raw = usage(evidence, Vec::new(), true);
        let SelectedUsage {
            evidence: selected_evidence,
            usability,
            units,
        } = selected(&raw, Some(true)).unwrap();
        assert_eq!(
            selected_evidence,
            match evidence {
                UsageEvidence::Reported => StageUsageEvidence::Reported,
                UsageEvidence::Estimated => StageUsageEvidence::Estimated,
                UsageEvidence::Unknown => unreachable!(),
            }
        );
        assert_eq!(usability, Usability::Usable);
        assert_eq!(
            units,
            vec![("text_input".into(), 100), ("text_output".into(), 50)]
        );
        assert!(raw.units.is_empty());
        assert_eq!(raw.evidence, evidence);
    }
}

#[test]
fn explicit_zero_is_retained_and_never_treated_as_absent_usage() {
    let raw = usage(
        UsageEvidence::Reported,
        vec![
            CostUnits {
                category: "text_output".into(),
                quantity: 0,
            },
            CostUnits {
                category: "text_input".into(),
                quantity: 0,
            },
        ],
        false,
    );
    let SelectedUsage {
        usability, units, ..
    } = selected(&raw, Some(false)).unwrap();
    assert_eq!(usability, Usability::NoUsableOutput);
    assert_eq!(
        units,
        vec![("text_input".into(), 0), ("text_output".into(), 0)]
    );
}

#[test]
fn unknown_billing_uses_maxima_and_preserves_semantic_usability() {
    let raw = usage(UsageEvidence::Unknown, cost().maxima, false);
    let SelectedUsage {
        evidence, units, ..
    } = selected(&raw, None).unwrap();
    assert_eq!(evidence, StageUsageEvidence::Unknown);
    assert_eq!(
        units,
        vec![("text_input".into(), 100), ("text_output".into(), 50)]
    );
    assert_eq!(
        selected(&raw, Some(false)).unwrap().evidence,
        StageUsageEvidence::Unknown
    );
    let usable = usage(UsageEvidence::Unknown, cost().maxima, true);
    for hint in [None, Some(true)] {
        let SelectedUsage {
            evidence,
            usability,
            ..
        } = selected(&usable, hint).unwrap();
        assert_eq!(evidence, StageUsageEvidence::Unknown);
        assert_eq!(usability, Usability::Usable);
    }
    let held = selected(&usable, Some(false)).unwrap();
    assert_eq!(held.evidence, StageUsageEvidence::Unknown);
    assert_eq!(held.usability, Usability::NoUsableOutput);
    assert!(usable.usable);
}

#[test]
fn duplicate_observations_and_missing_known_economic_claim_fail_before_finance() {
    let raw = usage(
        UsageEvidence::Reported,
        vec![
            CostUnits {
                category: "text_input".into(),
                quantity: 2,
            },
            CostUnits {
                category: "text_input".into(),
                quantity: 3,
            },
        ],
        false,
    );
    assert_eq!(
        selected(&raw, Some(false)),
        Err(RuntimeError::InvalidRecord)
    );
    let raw = usage(UsageEvidence::Estimated, Vec::new(), false);
    let claim = selected(&raw, Some(true)).unwrap();
    assert_eq!(claim.usability, Usability::Usable);
    assert!(!raw.usable);
    assert_eq!(selected(&raw, None), Err(RuntimeError::InvalidRecord));
}

#[test]
fn superseded_economic_claim_preserves_technically_valid_raw_evidence() {
    let raw = usage(UsageEvidence::Reported, cost().maxima, true);
    let SelectedUsage {
        evidence,
        usability,
        units,
    } = selected(&raw, Some(false)).unwrap();
    assert_eq!(evidence, StageUsageEvidence::Reported);
    assert_eq!(usability, Usability::NoUsableOutput);
    assert!(raw.usable);
    assert_eq!(raw.evidence, UsageEvidence::Reported);
    assert_eq!(
        units,
        vec![
            ("text_input".to_owned(), 100),
            ("text_output".to_owned(), 50)
        ]
    );
}
