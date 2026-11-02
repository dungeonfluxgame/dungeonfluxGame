use df_table::canvas::{Canvas, CanvasEffect, CanvasError};
use df_table::element::{ElementId, FieldName, FieldValue, Position};
use df_table::event::{EffectKind, RejectionCause};
use df_table::field_type::{FieldType, ValueError};
use df_table::testkit::{CHARACTER_KIND, registry_from};
use serde_json::json;

use crate::request::{NameKind, RequestError, parse_text_value};
use crate::wire::EffectRequest;

fn canvas() -> Canvas {
    Canvas::new(registry_from(&[CHARACTER_KIND]))
}

fn request(value: serde_json::Value) -> EffectRequest {
    serde_json::from_value(value).expect("the request parses")
}

#[test]
fn a_command_line_spawn_reads_each_value_by_its_field_type() {
    let effect = request(json!({
        "op": "spawn", "id": "pip", "kind": "character", "values": "text",
        "x": 20.0, "y": 30.0,
        "fields": {"name": "Pip", "hit_points": "7"}
    }))
    .into_effect(&canvas())
    .expect("converts");
    let CanvasEffect::Spawn {
        position, fields, ..
    } = effect
    else {
        panic!("expected a spawn");
    };
    assert_eq!(position, Position::new(20.0, 30.0));
    assert_eq!(
        fields.get(&FieldName::new("hit_points").expect("valid")),
        Some(&FieldValue::Number(7.0))
    );
}

#[test]
fn a_spawn_with_no_position_lands_in_the_middle_of_the_canvas() {
    let effect = request(json!({
        "op": "spawn", "id": "pip", "kind": "character", "fields": {"name": "Pip"}
    }))
    .into_effect(&canvas())
    .expect("converts");
    let CanvasEffect::Spawn { position, .. } = effect else {
        panic!("expected a spawn");
    };
    assert_eq!(position, Position::CENTER);
}

#[test]
fn json_values_must_already_have_the_right_shape_but_text_values_are_parsed() {
    let wrong = request(json!({
        "op": "spawn", "id": "pip", "kind": "character", "fields": {"name": "Pip", "hit_points": "7"}
    }))
    .into_effect(&canvas());
    assert!(matches!(wrong, Err(RequestError::Value { .. })));
    let right = request(json!({
        "op": "spawn", "id": "pip", "kind": "character", "fields": {"name": "Pip", "hit_points": 7}
    }))
    .into_effect(&canvas());
    assert!(right.is_ok());
}

#[test]
fn text_mode_refuses_a_value_that_is_not_text() {
    let result = request(json!({
        "op": "spawn", "id": "pip", "kind": "character", "values": "text",
        "fields": {"name": "Pip", "hit_points": 7}
    }))
    .into_effect(&canvas());
    assert!(matches!(
        result,
        Err(RequestError::Value {
            error: ValueError::WrongShape { .. },
            ..
        })
    ));
}

#[test]
fn a_badly_formed_name_is_refused_by_kind_of_name_before_the_canvas_sees_it() {
    let id = request(json!({"op": "despawn", "id": "Not Valid"})).into_effect(&canvas());
    assert!(matches!(
        id,
        Err(RequestError::Name {
            name_kind: NameKind::ElementId,
            ..
        })
    ));
    let kind =
        request(json!({"op": "spawn", "id": "a", "kind": "Bad Kind"})).into_effect(&canvas());
    assert!(matches!(
        kind,
        Err(RequestError::Name {
            name_kind: NameKind::Kind,
            ..
        })
    ));
    let field = request(
        json!({"op": "spawn", "id": "a", "kind": "character", "fields": {"Bad Field": "x"}}),
    )
    .into_effect(&canvas());
    assert!(matches!(
        field,
        Err(RequestError::Name {
            name_kind: NameKind::Field,
            ..
        })
    ));
}

#[test]
fn setting_a_field_of_an_element_that_does_not_exist_is_an_unknown_element() {
    let result = request(json!({
        "op": "set", "id": "ghost", "field": "name", "value": "x", "values": "text"
    }))
    .into_effect(&canvas());
    assert!(matches!(
        result,
        Err(RequestError::Canvas(CanvasError::UnknownElement(_)))
    ));
}

#[test]
fn a_conversion_failure_says_which_cause_it_is() {
    let name = RequestError::Name {
        name_kind: NameKind::ElementId,
        error: df_table::element::NameError::Empty,
    };
    assert_eq!(name.cause(), RejectionCause::InvalidName);
    let value = RequestError::Value {
        field: "hp".into(),
        error: ValueError::NotAChoice,
    };
    assert_eq!(value.cause(), RejectionCause::InvalidValue);
    assert_eq!(
        RequestError::Canvas(CanvasError::OutOfBounds).cause(),
        RejectionCause::OutOfBounds
    );
}

#[test]
fn a_request_knows_its_effect_and_its_element_when_the_id_is_valid() {
    let spawn = request(json!({"op": "spawn", "id": "pip", "kind": "character"}));
    assert_eq!(spawn.effect_kind(), EffectKind::Spawn);
    assert_eq!(spawn.element(), Some(ElementId::new("pip").expect("valid")));
    let bad = request(json!({"op": "despawn", "id": "Bad Id"}));
    assert_eq!(bad.element(), None);
    assert_eq!(request(json!({"op": "clear"})).element(), None);
}

#[test]
fn text_is_read_by_the_field_type_and_checked() {
    let number = FieldType::Number {
        minimum: 0.0,
        maximum: 10.0,
    };
    assert_eq!(parse_text_value(&number, "7"), Ok(FieldValue::Number(7.0)));
    assert!(matches!(
        parse_text_value(&number, "seven"),
        Err(ValueError::WrongShape { .. })
    ));
    assert!(matches!(
        parse_text_value(&number, "50"),
        Err(ValueError::OutOfRange { .. })
    ));
    assert!(matches!(
        parse_text_value(&number, "nan"),
        Err(ValueError::OutOfRange { .. })
    ));
    assert_eq!(
        parse_text_value(&FieldType::Flag, "true"),
        Ok(FieldValue::Flag(true))
    );
    assert!(parse_text_value(&FieldType::Flag, "yes").is_err());
    assert!(parse_text_value(&FieldType::Color, "#112233").is_ok());
    assert_eq!(
        parse_text_value(&FieldType::Color, "blue"),
        Err(ValueError::InvalidColor)
    );
    let choice = FieldType::Choice {
        choices: vec!["hero".into()],
    };
    assert_eq!(
        parse_text_value(&choice, "wizard"),
        Err(ValueError::NotAChoice)
    );
}
