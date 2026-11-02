use crate::element::{
    CANVAS_HEIGHT_UNITS, CANVAS_WIDTH_UNITS, Color, ElementId, FieldValue, MAXIMUM_NAME_LENGTH,
    NameError, Position,
};

#[test]
fn a_name_is_lowercase_letters_digits_underscore_and_hyphen_starting_with_a_letter() {
    assert!(ElementId::new("pip").is_ok());
    assert!(ElementId::new("goblin-2_b").is_ok());
    assert_eq!(ElementId::new(""), Err(NameError::Empty));
    assert_eq!(ElementId::new("2pip"), Err(NameError::MustStartWithLetter));
    assert_eq!(ElementId::new("Pip"), Err(NameError::MustStartWithLetter));
    assert_eq!(
        ElementId::new("pip!"),
        Err(NameError::InvalidCharacter('!'))
    );
    assert_eq!(ElementId::new("a b"), Err(NameError::InvalidCharacter(' ')));
}

#[test]
fn a_name_longer_than_the_maximum_is_refused() {
    assert!(ElementId::new(&"a".repeat(MAXIMUM_NAME_LENGTH)).is_ok());
    assert_eq!(
        ElementId::new(&"a".repeat(MAXIMUM_NAME_LENGTH + 1)),
        Err(NameError::TooLong)
    );
}

#[test]
fn a_name_with_non_ascii_letters_is_refused() {
    assert_eq!(
        ElementId::new("pip\u{e9}"),
        Err(NameError::InvalidCharacter('\u{e9}'))
    );
}

#[test]
fn a_color_is_hash_and_six_hexadecimal_digits() {
    assert_eq!(
        Color::parse("#ff8800").map(Color::to_hex),
        Some("#ff8800".to_owned())
    );
    assert_eq!(
        Color::parse("#FF8800").map(Color::to_hex),
        Some("#ff8800".to_owned())
    );
    assert!(Color::parse("ff8800").is_none());
    assert!(Color::parse("#ff880").is_none());
    assert!(Color::parse("#gg8800").is_none());
}

#[test]
fn the_canvas_includes_its_edges_and_nothing_beyond_or_not_a_number() {
    assert!(Position::new(0.0, 0.0).is_on_canvas());
    assert!(Position::new(CANVAS_WIDTH_UNITS, CANVAS_HEIGHT_UNITS).is_on_canvas());
    assert!(Position::CENTER.is_on_canvas());
    assert!(!Position::new(-0.1, 5.0).is_on_canvas());
    assert!(!Position::new(5.0, CANVAS_HEIGHT_UNITS + 0.1).is_on_canvas());
    assert!(!Position::new(f32::NAN, 5.0).is_on_canvas());
    assert!(!Position::new(5.0, f32::INFINITY).is_on_canvas());
}

#[test]
fn a_value_shows_as_text_for_a_label() {
    assert_eq!(FieldValue::Number(2.5).display_text(), "2.5");
    assert_eq!(FieldValue::Flag(true).display_text(), "true");
    assert_eq!(FieldValue::Text("Pip".into()).display_text(), "Pip");
    assert_eq!(
        FieldValue::Color(Color::parse("#00ff00").expect("valid color")).display_text(),
        "#00ff00"
    );
}
