use super::*;
use crate::backends::analysis::testing;

fn answer() -> Value {
    json!({
        "frame":"single_photograph",
        "medium":"photograph",
        "text":"none",
        "dress":"period",
        "setting":"period",
        "hero_count":2,
        "weapon_custody":"matches_custody",
        "visible_features":[

        ]
    })
}
fn feature() -> Value {
    json!({
        "name":"fruit stand",
        "look":"Apples under canvas.",
        "region":[
            100,
            200,
            400,
            600
        ],
        "recorded_id":"stall-1"
    })
}

#[test]
fn hero_count_and_custody_keep_legacy_priority_before_other_failures() {
    let spec = testing::spec(InspectionKind::ScenePlate, true);
    let mut value = answer();
    value["hero_count"] = json!(3);
    value["text"] = json!("visible_text");
    value.as_object_mut().unwrap().remove("weapon_custody");
    assert_eq!(
        interpret(value, &spec).unwrap().verdict,
        InspectionVerdict::Failed(PhotoCheckCause::HeroCount {
            expected: 2,
            seen: 3
        })
    );
    let mut value = answer();
    value["weapon_custody"] = json!("unexpected_drawn_weapon");
    value["frame"] = json!("grid_or_collage");
    assert_eq!(
        interpret(value, &spec).unwrap().verdict,
        InspectionVerdict::Failed(PhotoCheckCause::UnexpectedDrawnWeapon)
    );
    for count in [Value::Null, json!(-1), json!("2"), json!(2.5)] {
        let mut value = answer();
        value["hero_count"] = count;
        assert_eq!(interpret(value, &spec), Err(InspectionError::InvalidAnswer));
    }
}

#[test]
fn closed_quality_priority_and_room_character_requirements_are_preserved() {
    let spec = testing::spec(InspectionKind::ScenePlate, true);
    for (field, label, failure) in [
        ("frame", "grid_or_collage", PhotoCheckCause::GridOrCollage),
        (
            "frame",
            "cutout_composite",
            PhotoCheckCause::CutoutComposite,
        ),
        ("medium", "cgi_or_render", PhotoCheckCause::NotPhotograph),
        (
            "medium",
            "painting_or_illustration",
            PhotoCheckCause::NotPhotograph,
        ),
        ("setting", "modern", PhotoCheckCause::ModernSetting),
        ("text", "visible_text", PhotoCheckCause::VisibleText),
        ("dress", "modern_clothing", PhotoCheckCause::ModernClothing),
    ] {
        let mut value = answer();
        value[field] = json!(label);
        assert_eq!(
            interpret(value, &spec).unwrap().verdict,
            InspectionVerdict::Failed(failure)
        );
    }
    let mut basic = answer();
    for field in ["hero_count", "weapon_custody", "visible_features"] {
        basic.as_object_mut().unwrap().remove(field);
    }
    let room = testing::spec(InspectionKind::CanonicalRoom, false);
    let mut value = basic.clone();
    value["people_present"] = json!("some");
    assert_eq!(
        interpret(value, &room).unwrap().verdict,
        InspectionVerdict::Failed(PhotoCheckCause::PeopleInEmptyRoom)
    );
    assert!(interpret(basic.clone(), &room).is_err());
    let character = testing::spec(InspectionKind::CanonicalCharacter, false);
    let mut value = basic.clone();
    value["people_count"] = json!("not_one");
    assert_eq!(
        interpret(value, &character).unwrap().verdict,
        InspectionVerdict::Failed(PhotoCheckCause::CharacterCountNotOne)
    );
    basic["people_count"] = json!("exactly_one");
    basic["dress"] = json!("no_people");
    assert_eq!(
        interpret(basic, &character).unwrap().verdict,
        InspectionVerdict::Failed(PhotoCheckCause::CharacterHasNoPeople)
    );
}

#[test]
fn only_passed_bounded_features_with_existing_public_ids_keep_frozen_binding() {
    let mut spec = testing::spec(InspectionKind::ScenePlate, true);
    spec.hand_custody = crate::PrivateImageText::new(
        r#"{"place":"market","source_hash":"frozen-source","public_records":[{"id":"stall-1"}]}"#,
    );
    let mut value = answer();
    value["visible_features"] = json!([feature()]);
    let reading = interpret(value.clone(), &spec).unwrap();
    assert_eq!(reading.visible_features.len(), 1);
    assert_eq!(
        reading.source_binding.as_ref().unwrap().source_hash,
        "frozen-source"
    );
    assert_eq!(
        InspectionAssessment::from_value(&reading.to_value(), &spec).unwrap(),
        reading
    );
    let mut changed = reading.to_value();
    changed["source_binding"]["source_hash"] = json!("foreign-source");
    assert!(InspectionAssessment::from_value(&changed, &spec).is_err());
    let mut changed = value.clone();
    changed["visible_features"][0]["region"] = json!([400, 200, 100, 600]);
    assert!(interpret(changed.clone(), &spec).is_err());
    changed["hero_count"] = json!(3);
    let failed = interpret(changed, &spec).unwrap();
    assert!(failed.visible_features.is_empty());
    assert!(failed.source_binding.is_none());
    let mut changed = value.clone();
    changed["visible_features"][0]["recorded_id"] = json!("unlisted");
    assert!(interpret(changed, &spec).is_err());
    value["visible_features"][0]
        .as_object_mut()
        .unwrap()
        .remove("recorded_id");
    assert!(interpret(value, &spec).is_err());
}

#[test]
fn canonical_payload_rejects_forged_verdict_shape_revision_features_and_failure_basis() {
    let spec = testing::spec(InspectionKind::ScenePlate, true);
    let reading = interpret(answer(), &spec).unwrap();
    for field in ["schema_revision", "contract_revision"] {
        let mut changed = reading.to_value();
        changed[field] = json!(99);
        assert!(InspectionAssessment::from_value(&changed, &spec).is_err());
    }
    let mut changed = reading.to_value();
    changed["extra"] = json!(true);
    assert!(InspectionAssessment::from_value(&changed, &spec).is_err());
    let mut changed = reading.to_value();
    changed["failure"] = json!({
        "reason":"visible_text",
        "expected":null,
        "seen":null
    });
    assert!(InspectionAssessment::from_value(&changed, &spec).is_err());
    let mut failed = answer();
    failed["hero_count"] = json!(3);
    let mut changed = interpret(failed, &spec).unwrap().to_value();
    changed["failure"]["expected"] = json!(4);
    assert!(InspectionAssessment::from_value(&changed, &spec).is_err());
    let mut changed =
        InspectionAssessment::unjudged(&spec, UnjudgedReason::InvalidResponse).to_value();
    changed["visible_features"] = json!([feature()]);
    assert!(InspectionAssessment::from_value(&changed, &spec).is_err());
}

#[test]
fn unjudged_is_explicit_and_roundtrips_without_claiming_pass_or_features() {
    let spec = testing::spec(InspectionKind::ScenePlate, true);
    for reason in [
        UnjudgedReason::Preparation,
        UnjudgedReason::Transport,
        UnjudgedReason::Provider,
        UnjudgedReason::InvalidResponse,
        UnjudgedReason::Deadline,
        UnjudgedReason::Disabled,
        UnjudgedReason::Cancelled,
    ] {
        let reading = InspectionAssessment::unjudged(&spec, reason);
        assert_ne!(reading.verdict, InspectionVerdict::Pass);
        assert!(reading.visible_features.is_empty());
        assert_eq!(
            InspectionAssessment::from_value(&reading.to_value(), &spec).unwrap(),
            reading
        );
    }
    assert_eq!(
        PhotoCheckCause::VisibleText.correction(),
        "The previous result showed lettering; leave every sign, board, cloth and wall surface plain and blank."
    );
    assert!(
        PhotoCheckCause::UnexpectedDrawnWeapon
            .correction()
            .contains("Reference poses never authorize a drawn weapon.")
    );
}
