use super::fixtures::{facts, name};
use super::purpose::{
    Fallback, Price, PurposeError, PurposeFacts, PurposeSpec, PurposeTable, QueueClass,
    ResourceKind, RetryPolicy, SizeRule,
};

#[test]
fn every_built_in_purpose_has_a_deadline_a_class_and_a_fallback() {
    let table = PurposeTable::built_in().expect("the built-in table is valid");
    let mut count = 0;
    for spec in table.all() {
        count += 1;
        assert!(spec.deadline_ms() > 0, "{} has no deadline", spec.name());
        assert!(QueueClass::ALL.contains(&spec.class()));
        assert!(!spec.fallback().key().is_empty());
        let retry = spec.retry();
        assert!(
            retry.initial_backoff_ms > 0 && retry.initial_backoff_ms <= retry.maximum_backoff_ms
        );
    }
    assert_eq!(count, 12);
}

#[test]
fn interpret_has_the_short_deadline_that_triggers_its_buttons_fallback() {
    let table = PurposeTable::built_in().expect("valid");
    let interpret = table
        .get(&name("interpret"))
        .expect("interpret is a purpose");
    assert_eq!(
        interpret.deadline_ms(),
        super::limits::interpret_deadline_ms()
    );
    assert_eq!(interpret.fallback(), Fallback::ButtonsFromLegalChoices);
    let narrate = table.get(&name("narrate")).expect("narrate is a purpose");
    assert!(
        narrate.deadline_ms() > interpret.deadline_ms(),
        "only interpret is held to the short deadline"
    );
}

#[test]
fn the_plans_model_calls_and_pictures_are_all_purposes() {
    let table = PurposeTable::built_in().expect("valid");
    for purpose in [
        "interpret",
        "narrate",
        "describe",
        "dress",
        "staging_text",
        "plate",
        "background",
        "portrait",
        "item_view",
        "lookahead_describe",
        "lookahead_background",
    ] {
        assert!(table.get(&name(purpose)).is_some(), "{purpose} is missing");
    }
}

#[test]
fn blocking_purposes_are_live_and_look_ahead_work_never_blocks_anything() {
    let table = PurposeTable::built_in().expect("valid");
    let class_of = |purpose: &str| table.get(&name(purpose)).expect("known").class();
    assert_eq!(class_of("interpret"), QueueClass::Live);
    assert_eq!(class_of("narrate"), QueueClass::Live);
    assert_eq!(class_of("plate"), QueueClass::Plate);
    assert_eq!(class_of("lookahead_background"), QueueClass::Lookahead);
    assert_eq!(class_of("portrait"), QueueClass::Background);
    assert_eq!(
        table
            .get(&name("lookahead_background"))
            .expect("known")
            .fallback(),
        Fallback::Nothing
    );
}

#[test]
fn the_classes_are_ordered_live_plate_lookahead_background() {
    let ranks: Vec<usize> = QueueClass::ALL.iter().map(|class| class.rank()).collect();
    assert_eq!(ranks, [0, 1, 2, 3]);
    assert!(!QueueClass::Live.drops_oldest_when_full());
    assert!(QueueClass::Plate.drops_oldest_when_full());
}

#[test]
fn a_purpose_without_a_deadline_is_refused() {
    let broken = PurposeFacts {
        deadline_ms: 0,
        ..facts(ResourceKind::Text, QueueClass::Live, 1)
    };
    assert!(matches!(
        PurposeSpec::new("broken", broken),
        Err(PurposeError::ZeroDeadline { .. })
    ));
}

#[test]
fn a_size_rule_for_the_other_kind_or_for_nothing_is_refused() {
    let wrong_kind = PurposeFacts {
        size: SizeRule::ImagePixels {
            width: 4,
            height: 4,
        },
        ..facts(ResourceKind::Text, QueueClass::Live, 10)
    };
    let empty = PurposeFacts {
        size: SizeRule::TextCharacters { maximum: 0 },
        ..facts(ResourceKind::Text, QueueClass::Live, 10)
    };
    for broken in [wrong_kind, empty] {
        assert!(matches!(
            PurposeSpec::new("broken", broken),
            Err(PurposeError::SizeRuleMismatch { .. })
        ));
    }
}

#[test]
fn a_retry_backoff_of_zero_or_above_its_maximum_is_refused() {
    for retry in [
        RetryPolicy {
            maximum_retries: 1,
            initial_backoff_ms: 0,
            maximum_backoff_ms: 10,
        },
        RetryPolicy {
            maximum_retries: 1,
            initial_backoff_ms: 20,
            maximum_backoff_ms: 10,
        },
    ] {
        let broken = PurposeFacts {
            retry,
            ..facts(ResourceKind::Text, QueueClass::Live, 10)
        };
        assert!(matches!(
            PurposeSpec::new("broken", broken),
            Err(PurposeError::InvalidBackoff { .. })
        ));
    }
}

#[test]
fn a_purpose_listed_twice_is_refused() {
    let spec = || PurposeSpec::new("twice", facts(ResourceKind::Text, QueueClass::Live, 10));
    assert!(matches!(
        PurposeTable::new(vec![spec().expect("valid"), spec().expect("valid")]),
        Err(PurposeError::DuplicatePurpose { .. })
    ));
}

#[test]
fn the_backoff_doubles_from_the_initial_and_stops_at_the_maximum() {
    let retry = RetryPolicy {
        maximum_retries: 9,
        initial_backoff_ms: 500,
        maximum_backoff_ms: 8_000,
    };
    let waits: Vec<u64> = (0..=6).map(|attempt| retry.backoff_ms(attempt)).collect();
    assert_eq!(waits, [0, 500, 1_000, 2_000, 4_000, 8_000, 8_000]);
    assert_eq!(retry.backoff_ms(u8::MAX), 8_000);
}

#[test]
fn the_built_in_lanes_are_unpriced_until_a_lane_sets_a_price() {
    let table = PurposeTable::built_in().expect("valid");
    assert!(table.all().all(|spec| spec.price() == Price::Unpriced));
}

#[test]
fn every_closed_purpose_key_is_its_serde_name_and_reads_back() {
    fn check<T>(all: &[T], key: fn(T) -> &'static str, from_key: fn(&str) -> Option<T>)
    where
        T: Copy + PartialEq + std::fmt::Debug + serde::Serialize,
    {
        for value in all.iter().copied() {
            let written = serde_json::to_string(&value).unwrap();
            assert_eq!(written, format!("\"{}\"", key(value)));
            assert_eq!(from_key(key(value)), Some(value));
        }
        assert_eq!(from_key("not_a_key"), None);
    }

    check(ResourceKind::ALL, ResourceKind::key, ResourceKind::from_key);
    check(QueueClass::ALL, QueueClass::key, QueueClass::from_key);
    check(Fallback::ALL, Fallback::key, Fallback::from_key);
}
