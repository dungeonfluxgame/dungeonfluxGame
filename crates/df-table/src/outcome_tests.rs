use std::collections::BTreeSet;

use crate::limits::MAXIMUM_ALTERNATIVES;
use crate::outcome::{
    Decline, DeclineCode, DeclineError, Outcome, OutcomeKind, PendingKind, PublicClass,
};

/// The plan's table (C.6, RES40), written out a second time with no wildcard: a code added to
/// `DeclineCode` stops this test compiling until the owner's table says where it goes.
fn planned_public_class(code: DeclineCode) -> PublicClass {
    match code {
        DeclineCode::NotYourTurn => PublicClass::WrongTurn,
        DeclineCode::WindowClosed => PublicClass::StaleOffer,
        DeclineCode::StockChanged => PublicClass::StaleOffer,
        DeclineCode::IntentInvalid => PublicClass::InvalidSelection,
        DeclineCode::ResourceSpent => PublicClass::ResourceMissing,
        DeclineCode::EconomySpent => PublicClass::ResourceMissing,
        DeclineCode::TimeLocked => PublicClass::Paused,
        DeclineCode::PendingOpen => PublicClass::Paused,
        DeclineCode::PvpOff => PublicClass::CapabilityUnavailable,
        DeclineCode::ConsentDeclined => PublicClass::CapabilityUnavailable,
        DeclineCode::TableBusy => PublicClass::ResourceExhausted,
        DeclineCode::RateLimited => PublicClass::ResourceExhausted,
        DeclineCode::NotViable => PublicClass::InternalReasonKey,
        DeclineCode::NotSupportedYet => PublicClass::InternalReasonKey,
        DeclineCode::NotInCombat => PublicClass::InternalReasonKey,
        DeclineCode::NoCombatProfile => PublicClass::InternalReasonKey,
        DeclineCode::MissingRequirement => PublicClass::InternalReasonKey,
        DeclineCode::NotKnowable => PublicClass::InternalReasonKey,
        DeclineCode::SameUrging => PublicClass::InternalReasonKey,
        DeclineCode::CastingRestricted => PublicClass::InternalReasonKey,
    }
}

#[test]
fn every_decline_code_maps_to_the_public_class_the_plan_names() {
    for code in DeclineCode::ALL {
        assert_eq!(
            code.public_class(),
            planned_public_class(*code),
            "{}",
            code.key()
        );
    }
}

#[test]
fn the_list_holds_the_twenty_codes_of_section_c6() {
    assert_eq!(DeclineCode::ALL.len(), 20);
    let distinct: BTreeSet<_> = DeclineCode::ALL.iter().map(|code| code.key()).collect();
    assert_eq!(distinct.len(), 20);
}

#[test]
fn the_wire_classes_cover_the_rejection_codes_the_plan_maps() {
    let classes: BTreeSet<_> = DeclineCode::ALL
        .iter()
        .map(|code| format!("{:?}", code.public_class()))
        .collect();
    for expected in [
        "WrongTurn",
        "StaleOffer",
        "InvalidSelection",
        "ResourceMissing",
        "Paused",
        "CapabilityUnavailable",
        "ResourceExhausted",
        "InternalReasonKey",
    ] {
        assert!(classes.contains(expected), "no code maps to {expected}");
    }
}

#[test]
fn a_code_key_is_its_serialized_name_and_parses_back_to_the_code() {
    for code in DeclineCode::ALL {
        let json = serde_json::to_string(code).expect("a code serializes");
        assert_eq!(json, format!("\"{}\"", code.key()));
        let parsed: DeclineCode = serde_json::from_str(&json).expect("a code parses");
        assert_eq!(parsed, *code);
    }
}

#[test]
fn an_unknown_decline_key_does_not_parse() {
    assert!(serde_json::from_str::<DeclineCode>("\"not_a_code\"").is_err());
}

#[test]
fn every_reason_text_is_a_plain_sentence_with_no_code_in_it() {
    for code in DeclineCode::ALL {
        let reason = code.default_reason();
        assert!(!reason.trim().is_empty(), "{}", code.key());
        assert!(
            reason.ends_with('.'),
            "{} reason ends a sentence",
            code.key()
        );
        assert!(!reason.contains('_'), "{} reason shows a code", code.key());
        assert!(!reason.contains(code.key()), "{}", code.key());
    }
}

#[test]
fn the_reason_of_not_supported_yet_is_the_fixed_phrase_of_section_k5() {
    assert_eq!(
        DeclineCode::NotSupportedYet.default_reason(),
        "I can't run that yet."
    );
}

#[test]
fn a_decline_cannot_be_built_without_a_reason() {
    assert_eq!(
        Decline::new(DeclineCode::NotViable, ""),
        Err(DeclineError::EmptyReason)
    );
    assert_eq!(
        Decline::new(DeclineCode::NotViable, "  \t\n"),
        Err(DeclineError::EmptyReason)
    );
}

#[test]
fn a_decline_carries_its_code_reason_missing_piece_and_alternatives() {
    let decline = Decline::new(DeclineCode::MissingRequirement, "You need a torch.")
        .and_then(|decline| decline.with_missing_piece("a torch"))
        .and_then(|decline| decline.with_alternative("Feel along the wall"))
        .expect("a well-formed decline builds");
    assert_eq!(decline.code(), DeclineCode::MissingRequirement);
    assert_eq!(decline.reason(), "You need a torch.");
    assert_eq!(decline.missing_piece(), Some("a torch"));
    assert_eq!(decline.alternatives(), ["Feel along the wall".to_owned()]);
}

#[test]
fn the_default_reason_decline_uses_the_text_of_the_code() {
    let decline = Decline::with_default_reason(DeclineCode::NotYourTurn);
    assert_eq!(decline.reason(), "It isn't your turn.");
    assert_eq!(decline.missing_piece(), None);
    assert!(decline.alternatives().is_empty());
}

#[test]
fn blank_missing_pieces_and_alternatives_are_refused() {
    let decline = Decline::with_default_reason(DeclineCode::NotViable);
    assert_eq!(
        decline.clone().with_missing_piece(" ").unwrap_err(),
        DeclineError::EmptyMissingPiece
    );
    assert_eq!(
        decline.with_alternative("").unwrap_err(),
        DeclineError::EmptyAlternative
    );
}

#[test]
fn a_decline_holds_alternatives_up_to_its_named_bound_and_no_more() {
    let mut decline = Decline::with_default_reason(DeclineCode::NotViable);
    for number in 0..MAXIMUM_ALTERNATIVES {
        decline = decline
            .with_alternative(&format!("option {number}"))
            .expect("within the cap");
    }
    assert_eq!(
        decline.with_alternative("one too many").unwrap_err(),
        DeclineError::TooManyAlternatives
    );
}

#[test]
fn an_outcome_names_its_kind_and_the_four_kinds_have_distinct_keys() {
    let decline = Decline::with_default_reason(DeclineCode::NotViable);
    assert_eq!(Outcome::Applied.kind(), OutcomeKind::Applied);
    assert_eq!(
        Outcome::NeedsClarification.kind(),
        OutcomeKind::NeedsClarification
    );
    assert_eq!(Outcome::Declined(decline).kind(), OutcomeKind::Declined);
    assert_eq!(
        Outcome::Pending(PendingKind::Roll).kind(),
        OutcomeKind::Pending
    );
    let keys: BTreeSet<_> = OutcomeKind::ALL.iter().map(|kind| kind.key()).collect();
    assert_eq!(keys.len(), 4);
}

#[test]
fn a_pending_is_one_of_choice_roll_reaction_or_ruling() {
    let keys: Vec<_> = PendingKind::ALL.iter().map(|kind| kind.key()).collect();
    assert_eq!(keys, ["choice", "roll", "reaction", "ruling"]);
    for kind in PendingKind::ALL {
        let json = serde_json::to_string(kind).expect("a kind serializes");
        assert_eq!(json, format!("\"{}\"", kind.key()));
    }
}

#[test]
fn debug_of_a_decline_never_prints_its_texts() {
    let decline = Decline::new(DeclineCode::MissingRequirement, "SENTINEL-REASON about Zul")
        .and_then(|decline| decline.with_missing_piece("SENTINEL-RUBY-OF-ZUL"))
        .and_then(|decline| decline.with_alternative("SENTINEL-ALTERNATIVE"))
        .expect("a well-formed decline builds");
    let printed = format!("{decline:?}");
    assert!(!printed.contains("SENTINEL"), "{printed}");
    assert!(!printed.contains("Zul"), "{printed}");
    assert!(printed.contains("MissingRequirement"), "{printed}");
    assert!(printed.contains("Short"), "{printed}");
    let outcome = format!("{:?}", Outcome::Declined(decline));
    assert!(!outcome.contains("SENTINEL"), "{outcome}");
}
