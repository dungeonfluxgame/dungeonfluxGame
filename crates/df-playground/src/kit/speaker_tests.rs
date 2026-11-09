//! The speaker card's line, tone and conversion from the old card.

use crate::kit::speaker::SpeakerPanel;
use crate::lobby::model::BubbleTone;
use crate::speaker_card::{Portrait, PortraitCrop, SpeakerCard};

#[test]
fn a_card_reads_with_the_rose_tone_and_no_line_is_thinking() {
    let card: SpeakerPanel =
        serde_json::from_str(r#"{"name":" Stranger ","line":" "}"#).expect("a card reads");
    assert_eq!(card.name(), "Stranger");
    assert_eq!(card.line(), None);
    assert_eq!(card.rim_tone(), BubbleTone::Npc);
    assert!(!card.twin);
}

#[test]
fn the_thought_tone_takes_the_default_rim() {
    let card: SpeakerPanel =
        serde_json::from_str(r#"{"name":"Kael","tone":"thought"}"#).expect("a card reads");
    assert_eq!(card.rim_tone(), BubbleTone::Npc);
}

#[test]
fn the_old_card_converts_with_its_face_and_twin() {
    let old = SpeakerCard {
        character: "garrick".to_owned(),
        name: "Garrick".to_owned(),
        line: Some("Welcome.".to_owned()),
        portrait: Some(Portrait {
            media: "garrick.webp".to_owned(),
            crop: PortraitCrop {
                left_permille: 0,
                top_permille: 0,
                width_permille: 1000,
                height_permille: 1000,
            },
        }),
        twin: true,
    };
    let card = SpeakerPanel::from(&old);
    assert_eq!(card.name(), "Garrick");
    assert_eq!(card.portrait.as_deref(), Some("garrick.webp"));
    assert_eq!(card.crop.map(|crop| crop.width_permille), Some(1000));
    assert!(card.twin);
    assert_eq!(card.line(), Some("Welcome."));
}
