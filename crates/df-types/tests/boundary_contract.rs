#[path = "../contracts/boundary.rs"]
mod boundary;

use boundary::{
    AssetId, ClientId, Generation, GenerationError, RulesetComponent, RulesetId, UtteranceId,
};
use df_types::{
    BuildIdentity, BuildRevision, ClientBindingId, Currency, IdentityError, LiabilityRate,
    LocaleTag, MemberId, Money, MoneyError, OperationId, RecoveryEpoch, RevisionError,
    RevisionLabelError, RunId, SessionId, SessionRevision, TextIdentityError, Usage, UsageUnit,
};

const BYTES: [u8; 16] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
const TEXT: &str = "0102030405060708090a0b0c0d0e0f10";

#[test]
fn all_canonical_and_candidate_supplied_identity_kinds_preserve_bytes_and_refuse_invalid_input() {
    macro_rules! check {
        ($kind:ty) => {
            assert_eq!(<$kind>::from_bytes(&BYTES).unwrap().as_bytes(), &BYTES);
            assert_eq!(<$kind>::from_hex(TEXT).unwrap().as_bytes(), &BYTES);
            assert_eq!(
                <$kind>::from_bytes(&[1; 15]),
                Err(IdentityError::InvalidLength { actual: 15 })
            );
            assert_eq!(
                <$kind>::from_bytes(&[1; 17]),
                Err(IdentityError::InvalidLength { actual: 17 })
            );
            assert_eq!(<$kind>::from_bytes(&[0; 16]), Err(IdentityError::Zero));
            assert_eq!(
                <$kind>::from_hex(""),
                Err(TextIdentityError::InvalidLength { actual: 0 })
            );
            assert_eq!(
                <$kind>::from_hex(&"0".repeat(33)),
                Err(TextIdentityError::InvalidLength { actual: 33 })
            );
            assert_eq!(
                <$kind>::from_hex("00000000000000000000000000000000"),
                Err(TextIdentityError::Zero)
            );
            assert_eq!(
                <$kind>::from_hex("0102030405060708090A0B0C0D0E0F10"),
                Err(TextIdentityError::Malformed)
            );
            assert_eq!(
                <$kind>::from_hex("0102030405060708090a0b0c0d0e0f1 "),
                Err(TextIdentityError::Malformed)
            );
            assert_eq!(
                <$kind>::from_hex(&format!("{}é", "0".repeat(30))),
                Err(TextIdentityError::Malformed)
            );
        };
    }
    check!(SessionId);
    check!(MemberId);
    check!(ClientBindingId);
    check!(RunId);
    check!(OperationId);
    check!(ClientId);
    check!(UtteranceId);
    check!(AssetId);
}

#[test]
fn candidate_ruleset_identity_keeps_mechanics_catalog_and_source_independent() {
    let rules =
        RulesetId::new(Some("standard-2024"), Some("catalog-v1"), Some("source-v1")).unwrap();
    assert_eq!(rules.mechanics().as_str(), "standard-2024");
    assert_eq!(rules.catalog().as_str(), "catalog-v1");
    assert_eq!(rules.source_manifest().as_str(), "source-v1");
    assert_ne!(
        rules,
        RulesetId::new(Some("standard-2024"), Some("catalog-v2"), Some("source-v1")).unwrap()
    );
    assert_ne!(
        rules,
        RulesetId::new(Some("standard-2024"), Some("catalog-v1"), Some("source-v2")).unwrap()
    );
    assert_ne!(
        rules,
        RulesetId::new(Some("custom-v1"), Some("catalog-v1"), Some("source-v1")).unwrap()
    );
    let maximum = "r".repeat(128);
    assert_eq!(
        RulesetId::new(Some(&maximum), Some(&maximum), Some(&maximum))
            .unwrap()
            .mechanics()
            .as_str()
            .len(),
        128
    );
}

#[test]
fn candidate_ruleset_refusals_attribute_each_component_without_retaining_text() {
    let overlong = "s".repeat(129);
    let cases = [
        (None, RevisionLabelError::Missing),
        (Some(""), RevisionLabelError::Empty),
        (
            Some("private secret"),
            RevisionLabelError::InvalidCharacter { byte_index: 7 },
        ),
        (
            Some("é"),
            RevisionLabelError::InvalidCharacter { byte_index: 0 },
        ),
        (
            Some(overlong.as_str()),
            RevisionLabelError::TooLong { actual: 129 },
        ),
    ];
    for (invalid, expected) in cases {
        for component in [
            RulesetComponent::Mechanics,
            RulesetComponent::Catalog,
            RulesetComponent::SourceManifest,
        ] {
            let (mechanics, catalog, source) = match component {
                RulesetComponent::Mechanics => (invalid, Some("catalog"), Some("source")),
                RulesetComponent::Catalog => (Some("mechanics"), invalid, Some("source")),
                RulesetComponent::SourceManifest => (Some("mechanics"), Some("catalog"), invalid),
            };
            let error = RulesetId::new(mechanics, catalog, source).unwrap_err();
            assert_eq!(error.component, component);
            assert_eq!(error.cause, expected);
            assert!(!format!("{error:?}").contains("private secret"));
            assert!(!format!("{error:?}").contains(&overlong));
        }
    }
    let first = RulesetId::new(None, None, None).unwrap_err();
    assert_eq!(first.component, RulesetComponent::Mechanics);
    assert_eq!(first.cause, RevisionLabelError::Missing);
}

#[test]
fn candidate_work_generation_and_canonical_recovery_revision_never_wrap_or_mint_an_epoch() {
    assert_eq!(Generation::new(0), Err(GenerationError::Zero));
    for value in [1, 2, u32::MAX as u64, u64::MAX - 1, u64::MAX] {
        let generation = Generation::new(value).unwrap();
        assert_eq!(generation.get(), value);
        if value == u64::MAX {
            assert_eq!(generation.next(), Err(GenerationError::Exhausted));
        } else {
            assert_eq!(generation.next().unwrap().get(), value + 1);
        }
    }
    assert_eq!(RecoveryEpoch::new(0), Err(RevisionError::ZeroEpoch));
    let old = SessionRevision::new(RecoveryEpoch::new(7).unwrap(), u64::MAX);
    let restored = SessionRevision::new(RecoveryEpoch::new(8).unwrap(), 0);
    assert!(restored > old);
    assert_eq!(old.next_sequence(), Err(RevisionError::SequenceOverflow));
    assert_eq!(restored.next_sequence().unwrap().epoch(), restored.epoch());
    assert_eq!(restored.next_sequence().unwrap().sequence(), 1);
}

#[test]
fn exact_units_locale_and_all_five_build_components_reuse_the_existing_value_contracts() {
    let usd = Currency::parse("USD").unwrap();
    let eur = Currency::parse("EUR").unwrap();
    assert_eq!(
        Money::new(usd, 3).checked_add(Money::new(eur, 4)),
        Err(MoneyError::CurrencyMismatch)
    );
    assert_eq!(
        Money::new(usd, u128::MAX).checked_add(Money::new(usd, 1)),
        Err(MoneyError::Overflow)
    );
    assert_eq!(
        Money::new(usd, 0).checked_sub(Money::new(usd, 1)),
        Err(MoneyError::Underflow)
    );
    let units = [
        UsageUnit::Token,
        UsageUnit::Character,
        UsageUnit::Byte,
        UsageUnit::AudioMillisecond,
        UsageUnit::VideoMillisecond,
        UsageUnit::Image,
    ];
    for (index, unit) in units.iter().enumerate() {
        let usage = Usage::new(3, *unit);
        assert_eq!(
            usage.checked_add(Usage::new(4, *unit)).unwrap().quantity(),
            7
        );
        assert_eq!(
            Usage::new(u128::MAX, *unit).checked_add(Usage::new(1, *unit)),
            Err(MoneyError::Overflow)
        );
        for (other_index, other) in units.iter().enumerate() {
            if index != other_index {
                assert_eq!(
                    usage.checked_add(Usage::new(4, *other)),
                    Err(MoneyError::UnitMismatch)
                );
            }
        }
    }
    let rate = LiabilityRate::new(usd, UsageUnit::Token, 1, 3).unwrap();
    assert_eq!(
        rate.liability(Usage::new(1, UsageUnit::Token))
            .unwrap()
            .micros(),
        1
    );
    assert_eq!(
        rate.liability(Usage::new(0, UsageUnit::Token))
            .unwrap()
            .micros(),
        0
    );
    assert_eq!(LocaleTag::parse("EN-us").unwrap().as_str(), "en-us");
    assert!(LocaleTag::parse("").is_err());
    let build = BuildIdentity::new(
        Some("source"),
        Some("native"),
        Some("wasm"),
        Some("config"),
        Some("content"),
    )
    .unwrap();
    for (component, label) in [
        (BuildRevision::Source, "source"),
        (BuildRevision::Native, "native"),
        (BuildRevision::Wasm, "wasm"),
        (BuildRevision::Configuration, "config"),
        (BuildRevision::Content, "content"),
    ] {
        assert_eq!(build.revision(component).as_str(), label);
    }
    assert!(
        BuildIdentity::new(
            Some("source"),
            Some("native"),
            None,
            Some("config"),
            Some("content")
        )
        .is_err()
    );
}

#[test]
fn candidate_identity_and_generation_layouts_add_no_heap_or_extra_primitive_storage() {
    assert_eq!(
        std::mem::size_of::<ClientId>(),
        std::mem::size_of::<OperationId>()
    );
    assert_eq!(
        std::mem::size_of::<UtteranceId>(),
        std::mem::size_of::<OperationId>()
    );
    assert_eq!(
        std::mem::size_of::<AssetId>(),
        std::mem::size_of::<OperationId>()
    );
    assert_eq!(
        std::mem::size_of::<Generation>(),
        std::mem::size_of::<u64>()
    );
    eprintln!(
        "boundary layout bytes: client={} utterance={} asset={} canonical-operation={} \
         work-generation={}",
        std::mem::size_of::<ClientId>(),
        std::mem::size_of::<UtteranceId>(),
        std::mem::size_of::<AssetId>(),
        std::mem::size_of::<OperationId>(),
        std::mem::size_of::<Generation>()
    );
}
