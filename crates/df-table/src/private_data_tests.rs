use std::fmt::Display;
use std::marker::PhantomData;

use crate::element::ElementId;
use crate::event_field::EventField;
use crate::identity::OperationId;
use crate::private_data::{
    LengthClass, MEDIUM_TEXT_MAXIMUM_CHARS, Narration, PlayerText, SHORT_TEXT_MAXIMUM_CHARS, Secret,
};

/// Answers, at compile time, whether a concrete type implements a trait. The inherent constant
/// exists only when the bound holds; otherwise the trait's default `false` is found. This is the
/// stable-Rust way to assert that a type does NOT implement something.
macro_rules! implements {
    ($checked:ty: $bound:path) => {{
        struct Probe<T: ?Sized>(PhantomData<T>);
        #[allow(dead_code, reason = "the fallback is unused when the bound holds")]
        trait Otherwise {
            const IMPLEMENTS: bool = false;
        }
        impl<T: ?Sized> Otherwise for Probe<T> {}
        #[allow(
            dead_code,
            reason = "the inherent constant is unused when the bound fails"
        )]
        impl<T: ?Sized + $bound> Probe<T> {
            const IMPLEMENTS: bool = true;
        }
        <Probe<$checked>>::IMPLEMENTS
    }};
}

#[test]
fn the_probe_sees_a_trait_that_is_implemented() {
    assert!(implements!(ElementId: Display));
    assert!(implements!(ElementId: EventField));
    assert!(implements!(OperationId: EventField));
}

#[test]
fn player_text_narration_and_secrets_cannot_be_displayed() {
    assert!(!implements!(PlayerText: Display));
    assert!(!implements!(Narration: Display));
    assert!(!implements!(Secret<String>: Display));
    assert!(!implements!(Secret<u64>: Display));
}

#[test]
fn player_text_narration_and_secrets_cannot_be_event_fields() {
    assert!(!implements!(PlayerText: EventField));
    assert!(!implements!(Narration: EventField));
    assert!(!implements!(Secret<String>: EventField));
    assert!(!implements!(&'static str: EventField));
    assert!(!implements!(String: EventField));
}

#[test]
fn player_text_narration_and_secrets_cannot_be_serialized() {
    assert!(!implements!(PlayerText: serde::Serialize));
    assert!(!implements!(Narration: serde::Serialize));
    assert!(!implements!(Secret<String>: serde::Serialize));
}

#[test]
fn debug_of_a_private_value_prints_only_its_length_class() {
    let sentinel = "SENTINEL-PRIVATE-PLAYER-LINE";
    let text = PlayerText::new(sentinel);
    let narration = Narration::new(sentinel);
    let secret = Secret::new(sentinel.to_owned());
    assert_eq!(format!("{text:?}"), "PlayerText(length=short)");
    assert_eq!(format!("{narration:?}"), "Narration(length=short)");
    assert_eq!(format!("{secret:?}"), "Secret(length=short)");
    assert_eq!(format!("{text:#?}"), "PlayerText(length=short)");
    for printed in [
        format!("{text:?}"),
        format!("{narration:?}"),
        format!("{secret:?}"),
    ] {
        assert!(!printed.contains("SENTINEL"));
    }
}

#[test]
fn the_length_classes_change_at_forty_and_two_hundred_eighty_characters() {
    let class = |characters: usize| LengthClass::of_length(characters);
    assert_eq!(class(0), LengthClass::Empty);
    assert_eq!(class(1), LengthClass::Short);
    assert_eq!(class(SHORT_TEXT_MAXIMUM_CHARS), LengthClass::Short);
    assert_eq!(class(SHORT_TEXT_MAXIMUM_CHARS + 1), LengthClass::Medium);
    assert_eq!(class(MEDIUM_TEXT_MAXIMUM_CHARS), LengthClass::Medium);
    assert_eq!(class(MEDIUM_TEXT_MAXIMUM_CHARS + 1), LengthClass::Long);
}

#[test]
fn length_counts_characters_not_bytes() {
    let forty_accented = "\u{e9}".repeat(SHORT_TEXT_MAXIMUM_CHARS);
    assert!(forty_accented.len() > SHORT_TEXT_MAXIMUM_CHARS);
    assert_eq!(
        format!("{:?}", PlayerText::new(forty_accented)),
        "PlayerText(length=short)"
    );
    assert_eq!(
        format!("{:?}", PlayerText::new("")),
        "PlayerText(length=empty)"
    );
}

#[test]
fn a_private_value_is_read_only_through_expose() {
    assert_eq!(PlayerText::new("I search").expose(), "I search");
    assert_eq!(
        Narration::new("The door creaks.").expose(),
        "The door creaks."
    );
    assert_eq!(Secret::new(7_u64).expose(), &7);
}

#[test]
fn the_private_types_are_shareable_between_threads() {
    fn shareable<T: Send + Sync>() {}
    shareable::<PlayerText>();
    shareable::<Narration>();
    shareable::<Secret<String>>();
}
