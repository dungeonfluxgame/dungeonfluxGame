//! The frame judge's settings and the checks on its answer.

use serde_json::json;

use super::converse::Speaker;
use super::frame::{FrameConfig, FrameError, parse_frame, read_frames};

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
            "shot": "close-up"
        })
    };
    let parsed = parse_frame(
        &answer(json!("tavern_server"), "expression", 30),
        &speakers(),
        &config,
    )
    .unwrap();
    assert_eq!((parsed.changed_percent, parsed.changes.len()), (30, 1));
    assert_eq!(parsed.next_picture, "Lena smiles at Kael across the bar.");
    assert_eq!(parsed.shot.id, "close-up");
    let mut wrong_shot = answer(json!(null), "light", 5);
    wrong_shot["shot"] = json!("selfie");
    assert_eq!(
        parse_frame(&wrong_shot, &speakers(), &config).err(),
        Some(FrameError::UnknownShot {
            shot: "selfie".to_owned()
        })
    );
    assert!(parse_frame(&answer(json!(null), "light", 5), &speakers(), &config).is_ok());
    assert_eq!(
        parse_frame(
            &answer(json!("rina"), "expression", 30),
            &speakers(),
            &config
        ),
        Err(FrameError::UnknownCharacter {
            id: "rina".to_owned()
        })
    );
    assert_eq!(
        parse_frame(&answer(json!(null), "smell", 30), &speakers(), &config),
        Err(FrameError::UnknownKind {
            kind: "smell".to_owned()
        })
    );
    assert!(matches!(
        parse_frame(&answer(json!(null), "light", 101), &speakers(), &config),
        Err(FrameError::MalformedAnswer { .. })
    ));
}
