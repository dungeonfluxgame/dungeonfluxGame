//! Drift tests for the crate's closed keys: each `key()` is the snake_case of its variant name
//! unless it is a published spelling listed here, keys are unique, and `from_key` reads every
//! key back. These keys are stored in rows and written to audit lines, so a drift is a defect.
//! Listed sets are walked through `ALL`, so a new variant is covered without editing this file.

use std::fmt::Debug;

use crate::backends::api::ApiExchange;
use crate::cost::UsageEvidence;
use crate::execution::{
    CheckpointAlpha, DecisionKind, DispatchSettlement, DispatchState, FailureCause,
    LocalInspectionReason, QuotaDeferralReason, StageKind, StageState, SubmitDeclineContract,
};
use crate::identity_question::IdentifyError;
use crate::oneshot_segment::SegmentCause;
use crate::runtime::inspection_policy::{PhotoCheckCause, UnjudgedReason};
use crate::runtime::{NoDeliveryReason, PreflightCause};
use crate::{
    AccountingMode, BackendId, CreatorCategory, CreatorRoute, InterestState, JobState,
    OutputFormat, Purpose, SegmentationPrompt, TransformField,
};

/// The snake_case spelling of the variant name that starts `debug`.
fn snake_case(debug: &str) -> String {
    let name = debug
        .split(|character: char| !character.is_ascii_alphanumeric())
        .next()
        .unwrap_or_default();
    let mut spelled = String::new();
    for (index, character) in name.char_indices() {
        if character.is_ascii_uppercase() {
            if index > 0 {
                spelled.push('_');
            }
            spelled.push(character.to_ascii_lowercase());
        } else {
            spelled.push(character);
        }
    }
    spelled
}

/// Asserts every key is its variant's snake_case name, or the listed published spelling, and
/// that no two variants share a key.
pub(crate) fn assert_snake_keys<T: Copy + Debug + PartialEq>(
    values: &[T],
    key: impl Fn(T) -> &'static str,
    published: &[(T, &str)],
) {
    let mut seen = Vec::new();
    for value in values {
        let expected = match published.iter().find(|(listed, _)| listed == value) {
            Some((_, spelling)) => (*spelling).to_owned(),
            None => snake_case(&format!("{value:?}")),
        };
        assert_eq!(key(*value), expected, "{value:?}");
        assert!(!seen.contains(&key(*value)), "duplicate key for {value:?}");
        seen.push(key(*value));
    }
}

/// Asserts `from_key` reads every key back and refuses an unknown one.
fn assert_round_trip<T: Copy + Debug + PartialEq>(
    values: &[T],
    key: impl Fn(T) -> &'static str,
    from_key: impl Fn(&str) -> Option<T>,
) {
    for value in values {
        assert_eq!(from_key(key(*value)), Some(*value), "{value:?}");
    }
    assert_eq!(from_key("not_a_key"), None);
}

/// Asserts the whole closed key contract over `all` (a listed set's `ALL`): snake_case or the
/// listed published spelling, unique, and read back by `from_key`.
pub(crate) fn assert_keys<T: Copy + Debug + PartialEq>(
    all: &[T],
    key: fn(T) -> &'static str,
    from_key: fn(&str) -> Option<T>,
    published: &[(T, &str)],
) {
    assert!(!all.is_empty());
    assert_snake_keys(all, key, published);
    assert_round_trip(all, key, from_key);
}

#[test]
fn snake_case_spells_variant_names() {
    assert_eq!(snake_case("RecipeFrozen"), "recipe_frozen");
    assert_eq!(
        snake_case("HeroCount { expected: 1, seen: 2 }"),
        "hero_count"
    );
    assert_eq!(snake_case("Png"), "png");
}

#[test]
fn configuration_and_output_keys_keep_their_published_spellings() {
    assert_keys(
        BackendId::ALL,
        BackendId::key,
        BackendId::from_key,
        &[
            (BackendId::OpenAi, "openai"),
            (BackendId::CodexCli, "codex-cli"),
        ],
    );
    assert_keys(
        AccountingMode::ALL,
        AccountingMode::key,
        AccountingMode::from_key,
        &[(AccountingMode::RecordOnly, "record-only")],
    );
    assert_keys(
        OutputFormat::ALL,
        OutputFormat::key,
        OutputFormat::from_key,
        &[(OutputFormat::WebP, "webp")],
    );
    assert_keys(
        SubmitDeclineContract::ALL,
        SubmitDeclineContract::key,
        SubmitDeclineContract::from_key,
        &[
            (
                SubmitDeclineContract::OpenAiImages429V1,
                "openai-images-429-v1",
            ),
            (SubmitDeclineContract::FalQueue429V1, "fal-queue-429-v1"),
        ],
    );
}

#[test]
fn request_and_transform_keys_are_snake_case_and_read_back() {
    assert_keys(Purpose::ALL, Purpose::key, Purpose::from_key, &[]);
    assert_eq!(Purpose::ALL.len(), Purpose::COUNT);
    for (index, purpose) in Purpose::ALL.iter().enumerate() {
        assert_eq!(purpose.index(), index, "{purpose:?}");
    }
    assert_keys(
        TransformField::ALL,
        TransformField::key,
        TransformField::from_key,
        &[],
    );
    assert_keys(
        SegmentationPrompt::ALL,
        SegmentationPrompt::key,
        SegmentationPrompt::from_key,
        &[],
    );
    assert_keys(
        UsageEvidence::ALL,
        UsageEvidence::key,
        UsageEvidence::from_key,
        &[],
    );
}

#[test]
fn stored_execution_keys_are_snake_case_and_read_back() {
    assert_keys(StageKind::ALL, StageKind::key, StageKind::from_key, &[]);
    assert_keys(
        DecisionKind::ALL,
        DecisionKind::key,
        DecisionKind::from_key,
        &[],
    );
    assert_keys(StageState::ALL, StageState::key, StageState::from_key, &[]);
    assert_keys(
        DispatchState::ALL,
        DispatchState::key,
        DispatchState::from_key,
        &[],
    );
    assert_keys(
        DispatchSettlement::ALL,
        DispatchSettlement::key,
        DispatchSettlement::from_key,
        &[],
    );
    assert_keys(
        FailureCause::ALL,
        FailureCause::key,
        FailureCause::from_key,
        &[],
    );
    assert_keys(
        CheckpointAlpha::ALL,
        CheckpointAlpha::key,
        CheckpointAlpha::from_key,
        &[],
    );
    assert_keys(
        QuotaDeferralReason::ALL,
        QuotaDeferralReason::key,
        QuotaDeferralReason::from_key,
        &[],
    );
    assert_keys(
        LocalInspectionReason::ALL,
        LocalInspectionReason::key,
        LocalInspectionReason::from_key,
        &[],
    );
}

#[test]
fn stored_journal_and_delivery_keys_are_snake_case_and_read_back() {
    assert_keys(JobState::ALL, JobState::key, JobState::from_key, &[]);
    assert_keys(
        InterestState::ALL,
        InterestState::key,
        InterestState::from_key,
        &[],
    );
    assert_keys(
        NoDeliveryReason::ALL,
        NoDeliveryReason::key,
        NoDeliveryReason::from_key,
        &[],
    );
    assert_keys(
        PreflightCause::ALL,
        PreflightCause::key,
        PreflightCause::from_key,
        &[],
    );
    for cause in PreflightCause::ALL {
        assert_eq!(
            NoDeliveryReason::from_key(cause.key()).map(NoDeliveryReason::key),
            Some(cause.key())
        );
    }
}

#[test]
fn analysis_and_one_shot_keys_are_snake_case_and_read_back() {
    assert_keys(
        CreatorCategory::ALL,
        CreatorCategory::key,
        CreatorCategory::from_key,
        &[],
    );
    assert_keys(
        CreatorRoute::ALL,
        CreatorRoute::key,
        CreatorRoute::from_key,
        &[],
    );
    assert_keys(
        IdentifyError::ALL,
        IdentifyError::key,
        IdentifyError::from_key,
        &[],
    );
    assert_keys(
        SegmentCause::ALL,
        SegmentCause::key,
        SegmentCause::from_key,
        &[],
    );
}

/// Every variant of the crate-private key sets, which are not listed: an `ALL` that nothing
/// outside tests reads would be dead code. The wildcard-free `match`es stop compiling when a
/// variant is added, so these lists cannot silently miss it.
fn crate_private_variants() -> (Vec<ApiExchange>, Vec<UnjudgedReason>, Vec<PhotoCheckCause>) {
    use PhotoCheckCause::*;
    use UnjudgedReason::*;
    let exchanges = vec![
        ApiExchange::Submit,
        ApiExchange::Poll,
        ApiExchange::Result,
        ApiExchange::Delivery,
    ];
    let reasons = vec![
        Preparation,
        Transport,
        Provider,
        InvalidResponse,
        Deadline,
        Disabled,
        Cancelled,
    ];
    let causes = vec![
        GridOrCollage,
        CutoutComposite,
        NotPhotograph,
        ModernSetting,
        VisibleText,
        ModernClothing,
        PeopleInEmptyRoom,
        CharacterCountNotOne,
        CharacterHasNoPeople,
        HeroCount {
            expected: 1,
            seen: 2,
        },
        UnexpectedDrawnWeapon,
    ];
    for exchange in &exchanges {
        match exchange {
            ApiExchange::Submit
            | ApiExchange::Poll
            | ApiExchange::Result
            | ApiExchange::Delivery => {}
        }
    }
    for reason in &reasons {
        match reason {
            Preparation | Transport | Provider | InvalidResponse | Deadline | Disabled
            | Cancelled => {}
        }
    }
    for cause in &causes {
        match cause {
            GridOrCollage
            | CutoutComposite
            | NotPhotograph
            | ModernSetting
            | VisibleText
            | ModernClothing
            | PeopleInEmptyRoom
            | CharacterCountNotOne
            | CharacterHasNoPeople
            | HeroCount { .. }
            | UnexpectedDrawnWeapon => {}
        }
    }
    (exchanges, reasons, causes)
}

#[test]
fn crate_private_keys_are_snake_case() {
    let (exchanges, reasons, causes) = crate_private_variants();
    assert_snake_keys(&exchanges, ApiExchange::key, &[]);
    assert_snake_keys(&reasons, UnjudgedReason::key, &[]);
    assert_snake_keys(&causes, PhotoCheckCause::key, &[]);
}
