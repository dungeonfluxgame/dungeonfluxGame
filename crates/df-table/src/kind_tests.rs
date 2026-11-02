use std::collections::BTreeMap;

use crate::element::{Appearance, Element, ElementId, FieldName, FieldValue, KindName, Position};
use crate::field_type::ValueError;
use crate::kind::KindRegistry;
use crate::kind_file::{
    ChoiceProblem, FieldProblem, KindError, KindFile, KindProblem, UnusedOption,
};
use crate::limits::{MAXIMUM_CHOICES, MAXIMUM_FIELDS_PER_KIND, MAXIMUM_TEXT_LENGTH};
use crate::testkit::{CHARACTER_KIND, registry_from};

fn file(json: &str) -> KindFile {
    serde_json::from_str(json).expect("the test kind file parses")
}

fn errors_of(json: &str) -> Vec<KindError> {
    KindRegistry::from_files(&[file(json)]).expect_err("the kind is invalid")
}

fn field_problem(json: &str) -> FieldProblem {
    let KindProblem::Field { problem, .. } = errors_of(json).remove(0).problem else {
        panic!("expected a field problem");
    };
    problem
}

fn field_json(field: &str) -> String {
    format!(r#"{{"name": "gate", "shape": "rect", "width": 4, "height": 4, "fields": [{field}]}}"#)
}

#[test]
fn a_valid_kind_file_becomes_a_definition_with_typed_fields() {
    let registry = registry_from(&[CHARACTER_KIND]);
    let definition = registry.iter().next().expect("one kind");
    assert_eq!(definition.name.as_str(), "character");
    assert_eq!(definition.fields.len(), 5);
    assert_eq!(
        definition.label_field.as_ref().map(FieldName::as_str),
        Some("name")
    );
    assert_eq!(
        definition.color_field.as_ref().map(FieldName::as_str),
        Some("color")
    );
}

#[test]
fn every_problem_across_several_files_is_reported_together() {
    let bad_size = file(r#"{"name": "wall", "shape": "rect", "width": 0, "height": 5}"#);
    let bad_name = file(r#"{"name": "Bad Name", "shape": "rect", "width": 5, "height": 5}"#);
    let bad_label = file(
        r#"{"name": "lamp", "shape": "circle", "width": 3, "height": 3, "label_field": "title"}"#,
    );
    let errors = KindRegistry::from_files(&[bad_size, bad_name, bad_label]).expect_err("invalid");
    assert_eq!(errors.len(), 3);
    assert!(
        errors
            .iter()
            .any(|error| error.problem == KindProblem::BadSize)
    );
    assert!(
        errors
            .iter()
            .any(|error| matches!(error.problem, KindProblem::InvalidName(_)))
    );
    assert!(
        errors
            .iter()
            .any(|error| error.problem == KindProblem::LabelFieldMissing("title".into()))
    );
}

#[test]
fn a_height_taller_than_the_canvas_or_a_negative_or_nan_size_is_refused() {
    for size in [
        r#""width": 4, "height": 60"#,
        r#""width": -1, "height": 4"#,
        r#""width": 101, "height": 4"#,
    ] {
        let json = format!(r#"{{"name": "wall", "shape": "rect", {size}}}"#);
        assert_eq!(errors_of(&json)[0].problem, KindProblem::BadSize, "{size}");
    }
}

#[test]
fn a_kind_with_too_many_fields_is_refused() {
    let fields: Vec<String> = (0..=MAXIMUM_FIELDS_PER_KIND)
        .map(|index| format!(r#"{{"name": "f{index}", "type": "flag"}}"#))
        .collect();
    let json = field_json(&fields.join(","));
    assert!(
        errors_of(&json)
            .iter()
            .any(|error| error.problem == KindProblem::TooManyFields)
    );
}

#[test]
fn a_field_declared_twice_is_reported_once() {
    let json = field_json(r#"{"name": "a", "type": "flag"}, {"name": "a", "type": "flag"}"#);
    assert_eq!(field_problem(&json), FieldProblem::Duplicate);
}

#[test]
fn a_default_that_does_not_fit_its_field_is_refused() {
    let json = field_json(
        r#"{"name": "health", "type": "number", "minimum": 0, "maximum": 10, "default": 50}"#,
    );
    assert!(matches!(
        field_problem(&json),
        FieldProblem::BadDefault(ValueError::OutOfRange { .. })
    ));
}

#[test]
fn text_and_number_limits_must_make_sense() {
    assert_eq!(
        field_problem(&field_json(
            r#"{"name": "a", "type": "text", "maximum_length": 0}"#
        )),
        FieldProblem::TextLimit
    );
    assert_eq!(
        field_problem(&field_json(&format!(
            r#"{{"name": "a", "type": "text", "maximum_length": {}}}"#,
            MAXIMUM_TEXT_LENGTH + 1
        ))),
        FieldProblem::TextLimit
    );
    assert!(matches!(
        field_problem(&field_json(
            r#"{"name": "a", "type": "number", "minimum": 5, "maximum": 1}"#
        )),
        FieldProblem::NumberLimits { .. }
    ));
}

#[test]
fn a_lone_minimum_above_the_default_maximum_names_both_limits() {
    let problem = field_problem(&field_json(
        r#"{"name": "a", "type": "number", "minimum": 2000000}"#,
    ));
    assert_eq!(
        problem,
        FieldProblem::NumberLimits {
            minimum: 2_000_000.0,
            maximum: 1_000_000.0
        }
    );
    assert!(problem.to_string().contains("1000000"));
}

#[test]
fn an_option_that_does_not_belong_to_the_field_type_is_refused_for_every_type() {
    let cases = [
        (
            r#"{"name": "a", "type": "text", "minimum": 1}"#,
            UnusedOption::NumberLimits,
        ),
        (
            r#"{"name": "a", "type": "text", "choices": ["x"]}"#,
            UnusedOption::Choices,
        ),
        (
            r#"{"name": "a", "type": "number", "maximum_length": 3}"#,
            UnusedOption::MaximumLength,
        ),
        (
            r#"{"name": "a", "type": "flag", "choices": ["x"]}"#,
            UnusedOption::Choices,
        ),
        (
            r##"{"name": "a", "type": "color", "maximum": 3}"##,
            UnusedOption::NumberLimits,
        ),
        (
            r#"{"name": "a", "type": "choice", "choices": ["x"], "maximum_length": 3}"#,
            UnusedOption::MaximumLength,
        ),
    ];
    for (field, option) in cases {
        assert_eq!(
            field_problem(&field_json(field)),
            FieldProblem::UnusedOption(option),
            "{field}"
        );
    }
}

#[test]
fn the_choices_of_a_choice_field_are_checked() {
    let choices = |list: &str| {
        field_json(&format!(
            r#"{{"name": "a", "type": "choice", "choices": {list}}}"#
        ))
    };
    assert_eq!(
        field_problem(&field_json(r#"{"name": "a", "type": "choice"}"#)),
        FieldProblem::Choices(ChoiceProblem::NoneListed)
    );
    assert_eq!(
        field_problem(&choices("[]")),
        FieldProblem::Choices(ChoiceProblem::NoneListed)
    );
    let many: Vec<String> = (0..=MAXIMUM_CHOICES)
        .map(|index| format!("\"c{index}\""))
        .collect();
    assert!(matches!(
        field_problem(&choices(&format!("[{}]", many.join(",")))),
        FieldProblem::Choices(ChoiceProblem::TooMany { .. })
    ));
    assert!(matches!(
        field_problem(&choices(r#"["a", "a"]"#)),
        FieldProblem::Choices(ChoiceProblem::Duplicate { .. })
    ));
    assert!(matches!(
        field_problem(&choices(r#"[""]"#)),
        FieldProblem::Choices(ChoiceProblem::EmptyOrTooLong { .. })
    ));
}

#[test]
fn a_choice_is_measured_in_characters_not_bytes() {
    let long_in_bytes = "\u{43f}".repeat(30);
    let json = field_json(&format!(
        r#"{{"name": "a", "type": "choice", "choices": ["{long_in_bytes}"]}}"#
    ));
    assert!(KindRegistry::from_files(&[file(&json)]).is_ok());
}

#[test]
fn a_color_field_must_have_type_color_and_exist() {
    let not_a_color = file(
        r#"{"name": "gate", "shape": "rect", "width": 4, "height": 4, "color_field": "label",
            "fields": [{"name": "label", "type": "text"}]}"#,
    );
    let missing = file(
        r#"{"name": "door", "shape": "rect", "width": 4, "height": 4, "color_field": "tint"}"#,
    );
    let errors = KindRegistry::from_files(&[not_a_color, missing]).expect_err("invalid");
    assert!(
        errors
            .iter()
            .any(|error| error.problem == KindProblem::ColorFieldNotAColor("label".into()))
    );
    assert!(
        errors
            .iter()
            .any(|error| error.problem == KindProblem::ColorFieldMissing("tint".into()))
    );
}

#[test]
fn a_label_that_points_at_a_failed_field_is_not_blamed_a_second_time() {
    let json = r#"{"name": "gate", "shape": "rect", "width": 4, "height": 4, "label_field": "name",
        "fields": [{"name": "name", "type": "text", "maximum_length": 0}]}"#;
    let errors = errors_of(json);
    assert_eq!(errors.len(), 1);
    assert!(matches!(errors[0].problem, KindProblem::Field { .. }));
}

#[test]
fn a_kind_defined_twice_is_refused() {
    let errors = KindRegistry::from_files(&[file(CHARACTER_KIND), file(CHARACTER_KIND)])
        .expect_err("invalid");
    assert_eq!(errors[0].problem, KindProblem::Duplicate);
}

#[test]
fn a_misspelt_key_in_a_kind_file_is_refused_when_it_is_read() {
    let result: Result<KindFile, _> = serde_json::from_str(
        r#"{"name": "gate", "shape": "rect", "width": 4, "height": 4, "heigth": 9}"#,
    );
    assert!(result.is_err());
}

#[test]
fn a_long_name_from_a_file_is_shortened_when_it_is_echoed_in_an_error() {
    let json = format!(
        r#"{{"name": "{}", "shape": "rect", "width": 4, "height": 4}}"#,
        "a".repeat(500)
    );
    let error = &errors_of(&json)[0];
    assert!(error.kind.len() < 60, "{}", error.kind);
    assert!(error.to_string().len() < 200);
}

/// A character element named "pip", with one text field set when `value` is given.
fn element_with(field: &str, value: Option<&str>) -> Element {
    let mut fields = BTreeMap::new();
    if let Some(value) = value {
        fields.insert(
            FieldName::new(field).expect("valid"),
            FieldValue::Text(value.to_owned()),
        );
    }
    Element {
        id: ElementId::new("pip").expect("valid"),
        kind: KindName::new("character").expect("valid"),
        position: Position::CENTER,
        fields,
        motion: None,
        appearance: Appearance::default(),
    }
}

#[test]
fn the_label_is_the_label_field_and_falls_back_to_the_id_when_it_is_missing_or_empty() {
    let registry = registry_from(&[CHARACTER_KIND]);
    let definition = registry.iter().next().expect("one kind");
    assert_eq!(
        definition.label(&element_with("name", Some("Pip the Brave"))),
        "Pip the Brave"
    );
    assert_eq!(definition.label(&element_with("name", Some(""))), "pip");
    assert_eq!(definition.label(&element_with("name", None)), "pip");
}

#[test]
fn a_label_field_with_an_empty_default_is_labelled_by_the_id_not_left_blank() {
    let registry = registry_from(&[
        r#"{"name": "character", "shape": "circle", "width": 2, "height": 2, "label_field": "note",
            "fields": [{"name": "note", "type": "text", "default": ""}]}"#,
    ]);
    let definition = registry.iter().next().expect("one kind");
    assert_eq!(definition.label(&element_with("note", Some(""))), "pip");
}

#[test]
fn the_color_is_the_color_field_when_there_is_one() {
    let registry = registry_from(&[CHARACTER_KIND]);
    let definition = registry.iter().next().expect("one kind");
    let mut pip = element_with("name", Some("Pip"));
    assert_eq!(definition.color(&pip), None);
    pip.fields.insert(
        FieldName::new("color").expect("valid"),
        FieldValue::Color(crate::element::Color::parse("#ff0000").expect("valid")),
    );
    assert_eq!(
        definition.color(&pip).map(crate::element::Color::to_hex),
        Some("#ff0000".to_owned())
    );
}
