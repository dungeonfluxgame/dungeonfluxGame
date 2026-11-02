use crate::element::{
    CANVAS_HEIGHT_UNITS, CANVAS_WIDTH_UNITS, Color, Degrees, DegreesError, ElementId, FieldValue,
    MAXIMUM_NAME_LENGTH, NameError, Percent, PercentError, Pivot, Position,
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

#[test]
fn a_rotation_is_zero_to_359_degrees_and_360_or_below_zero_is_refused_with_the_range() {
    assert_eq!(Degrees::new(0).map(Degrees::get), Ok(0));
    assert_eq!(Degrees::new(359).map(Degrees::get), Ok(359));
    assert_eq!(Degrees::new(360), Err(DegreesError(360)));
    assert_eq!(Degrees::new(-1), Err(DegreesError(-1)));
    assert_eq!(Degrees::new(i64::MAX), Err(DegreesError(i64::MAX)));
    let message = DegreesError(360).to_string();
    assert!(
        message.contains("0 to 359") && message.contains("360"),
        "{message}"
    );
}

#[test]
fn a_rotation_reads_and_writes_as_a_plain_number_and_refuses_one_out_of_range() {
    let turned = Degrees::new(90).expect("valid degrees");
    assert_eq!(serde_json::to_string(&turned).expect("serializes"), "90");
    assert_eq!(
        serde_json::from_str::<Degrees>("359").expect("parses"),
        Degrees::new(359).expect("valid degrees")
    );
    assert!(serde_json::from_str::<Degrees>("360").is_err());
    assert!(serde_json::from_str::<Degrees>("-1").is_err());
}

#[test]
fn a_share_is_zero_to_100_percent_and_the_pivot_defaults_to_the_centre() {
    assert_eq!(Percent::new(0).map(Percent::get), Ok(0));
    assert_eq!(Percent::new(100).map(Percent::get), Ok(100));
    assert_eq!(Percent::new(101), Err(PercentError(101)));
    assert_eq!(Percent::new(-1), Err(PercentError(-1)));
    assert!(PercentError(101).to_string().contains("0 to 100"));
    assert_eq!(
        (Pivot::default().x.get(), Pivot::default().y.get()),
        (50, 50)
    );
}
