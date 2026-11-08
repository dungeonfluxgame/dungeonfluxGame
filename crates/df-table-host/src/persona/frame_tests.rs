//! The frame judge's settings and the checks on its answer.

use serde_json::json;

use super::converse::Speaker;
use super::frame::{FrameConfig, FrameError, parse_frame, read_frames};
use crate::world::PlaceItem;
use df_content::world_bible::Item;

fn speakers() -> Vec<Speaker> {
    vec![Speaker {
        id: "tavern_server".to_owned(),
        display_name: "Lena".to_owned(),
        job: "Server".to_owned(),
    }]
}

#[test]
fn the_shipped_frame_file_is_the_built_in_settings() {
    let shipped: FrameConfig = serde_json::from_str(include_str!(
        "../../../../assets/playground/persona/frames.json"
    ))
    .unwrap();
    assert_eq!(shipped, FrameConfig::default());
    let missing = std::path::Path::new("no/such/frames.json");
    assert_eq!(read_frames(missing).unwrap(), FrameConfig::default());
}

#[test]
fn an_answer_is_kept_only_with_listed_characters_and_kinds_and_a_percent_to_100() {
    let config = FrameConfig::default();
    let answer = |character: serde_json::Value, kind: &str, percent: u64| {
        json!({
            "changed_percent": percent,
            "changes": [{"character": character, "kind": kind, "now": "smiling at Kael"}],
            "next_picture": "Lena smiles at Kael across the bar.",
            "shot": "close-up",
            "inspected_item": null,
            "new_look": false
        })
    };
    let parsed = parse_frame(
        &answer(json!("tavern_server"), "expression", 30),
        &speakers(),
        &[],
        &config,
    )
    .unwrap();
    assert_eq!((parsed.changed_percent, parsed.changes.len()), (30, 1));
    assert_eq!(parsed.next_picture, "Lena smiles at Kael across the bar.");
    assert_eq!(parsed.shot.id, "close-up");
    let mut wrong_shot = answer(json!(null), "light", 5);
    wrong_shot["shot"] = json!("selfie");
    assert_eq!(
        parse_frame(&wrong_shot, &speakers(), &[], &config).err(),
        Some(FrameError::UnknownShot {
            shot: "selfie".to_owned()
        })
    );
    assert!(parse_frame(&answer(json!(null), "light", 5), &speakers(), &[], &config).is_ok());
    assert_eq!(
        parse_frame(
            &answer(json!("rina"), "expression", 30),
            &speakers(),
            &[],
            &config
        ),
        Err(FrameError::UnknownCharacter {
            id: "rina".to_owned()
        })
    );
    assert_eq!(
        parse_frame(&answer(json!(null), "smell", 30), &speakers(), &[], &config),
        Err(FrameError::UnknownKind {
            kind: "smell".to_owned()
        })
    );
    assert!(matches!(
        parse_frame(
            &answer(json!(null), "light", 101),
            &speakers(),
            &[],
            &config
        ),
        Err(FrameError::MalformedAnswer { .. })
    ));
}

#[test]
fn an_inspected_item_is_one_offered_here_or_null() {
    let config = FrameConfig::default();
    let items = [PlaceItem {
        item: Item {
            id: "frostline_longsword".to_owned(),
            name: "Frostline longsword".to_owned(),
            unit: "each".to_owned(),
            base_value_cp: 45_000,
            tags: vec!["frost-runes".to_owned()],
        },
        keeper: "weapon_seller".to_owned(),
    }];
    let answer = |inspected: serde_json::Value| {
        json!({
            "changed_percent": 10,
            "changes": [],
            "next_picture": "The Frostline fills the frame, frost runes along the fuller.",
            "shot": "wide",
            "inspected_item": inspected,
            "new_look": true
        })
    };
    let parsed = parse_frame(
        &answer(json!("frostline_longsword")),
        &speakers(),
        &items,
        &config,
    )
    .unwrap();
    assert_eq!(
        parsed.inspected.map(|offered| offered.item.id).as_deref(),
        Some("frostline_longsword")
    );
    let nobody = parse_frame(&answer(json!(null)), &speakers(), &items, &config).unwrap();
    assert_eq!(nobody.inspected, None);
    assert_eq!(
        parse_frame(&answer(json!("teapot")), &speakers(), &items, &config),
        Err(FrameError::UnknownItem {
            id: "teapot".to_owned()
        })
    );
    assert_eq!(config.inspect_shot.as_deref(), Some("insert"));
    let mut chose_insert = answer(json!(null));
    chose_insert["shot"] = json!("insert");
    assert_eq!(
        parse_frame(&chose_insert, &speakers(), &items, &config),
        Err(FrameError::UnknownShot {
            shot: "insert".to_owned()
        }),
        "the insert is code's for an inspection, never the judge's choice"
    );
}
