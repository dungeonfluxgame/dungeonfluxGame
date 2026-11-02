use serde_json::json;

use crate::element::{Color, FieldValue};
use crate::field_type::{FieldType, ValueError};

fn text(maximum_length: usize) -> FieldType {
    FieldType::Text { maximum_length }
}

fn number() -> FieldType {
    FieldType::Number {
        minimum: 0.0,
        maximum: 10.0,
    }
}

fn choice() -> FieldType {
    FieldType::Choice {
        choices: vec!["hero".into(), "foe".into()],
    }
}

#[test]
fn text_is_limited_in_characters_not_bytes() {
    let field = text(3);
    assert_eq!(
        field.check(&FieldValue::Text("\u{e9}\u{e9}\u{e9}".into())),
        Ok(())
    );
    assert_eq!(
        field.check(&FieldValue::Text("abcd".into())),
        Err(ValueError::TextTooLong { maximum: 3 })
    );
}

#[test]
fn text_with_a_control_character_is_refused() {
    let field = text(20);
    assert_eq!(
        field.check(&FieldValue::Text("line\nbreak".into())),
        Err(ValueError::ControlCharacter)
    );
    assert_eq!(
        field.check(&FieldValue::Text("escape\u{1b}[31m".into())),
        Err(ValueError::ControlCharacter)
    );
}

#[test]
fn a_number_must_be_finite_and_inside_its_limits_edges_included() {
    let field = number();
    assert_eq!(field.check(&FieldValue::Number(0.0)), Ok(()));
    assert_eq!(field.check(&FieldValue::Number(10.0)), Ok(()));
    for bad in [-0.1, 10.1, f64::NAN, f64::INFINITY] {
        assert!(
            matches!(
                field.check(&FieldValue::Number(bad)),
                Err(ValueError::OutOfRange { .. })
            ),
            "{bad}"
        );
    }
}

#[test]
fn a_value_of_the_wrong_shape_is_refused_for_every_type() {
    for field in [
        text(5),
        number(),
        FieldType::Flag,
        FieldType::Color,
        choice(),
    ] {
        let wrong = match field {
            FieldType::Text { .. } | FieldType::Choice { .. } => FieldValue::Flag(true),
            FieldType::Number { .. } | FieldType::Flag | FieldType::Color => {
                FieldValue::Text("x".into())
            }
        };
        assert!(
            matches!(field.check(&wrong), Err(ValueError::WrongShape { .. })),
            "{field:?}"
        );
    }
}

#[test]
fn a_choice_must_be_one_of_the_listed_texts() {
    assert_eq!(choice().check(&FieldValue::Text("hero".into())), Ok(()));
    assert_eq!(
        choice().check(&FieldValue::Text("wizard".into())),
        Err(ValueError::NotAChoice)
    );
}

#[test]
fn json_defaults_are_read_by_the_field_type() {
    assert_eq!(number().from_json(&json!(5)), Ok(FieldValue::Number(5.0)));
    assert_eq!(
        FieldType::Flag.from_json(&json!(true)),
        Ok(FieldValue::Flag(true))
    );
    assert_eq!(
        FieldType::Color.from_json(&json!("#112233")),
        Ok(FieldValue::Color(Color::parse("#112233").expect("valid")))
    );
    assert!(matches!(
        number().from_json(&json!("5")),
        Err(ValueError::WrongShape { .. })
    ));
    assert_eq!(
        FieldType::Color.from_json(&json!("blue")),
        Err(ValueError::InvalidColor)
    );
    assert!(matches!(
        number().from_json(&json!(50)),
        Err(ValueError::OutOfRange { .. })
    ));
}
