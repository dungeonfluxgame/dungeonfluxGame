//! The nameplate's name and line.

use crate::fixture::Caption;
use crate::kit::nameplate::Nameplate;

#[test]
fn a_caption_converts_and_a_blank_speaker_draws_no_name() {
    let plate = Nameplate::from(&Caption {
        speaker: Some(" ".to_owned()),
        text: "Good gear, fair prices.".to_owned(),
    });
    assert_eq!(plate.speaker(), None);
    assert_eq!(plate.text(), Some("Good gear, fair prices."));
}

#[test]
fn a_nameplate_without_words_is_not_drawn() {
    let plate: Nameplate =
        serde_json::from_str(r#"{"speaker":"Garrick","text":"  "}"#).expect("a plate reads");
    assert_eq!(plate.speaker(), Some("Garrick"));
    assert_eq!(plate.text(), None);
}
