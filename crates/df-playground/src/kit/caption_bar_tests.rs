//! The caption bar's line.

use crate::kit::caption_bar::CaptionBar;
use crate::lobby::model::RoleplayScene;

#[test]
fn a_roleplay_caption_becomes_the_bar_and_a_blank_one_none() {
    let mut roleplay = RoleplayScene {
        caption: Some(" The party arrives. ".to_owned()),
        ..RoleplayScene::default()
    };
    let bar = CaptionBar::from_roleplay(&roleplay).expect("a worded caption draws a bar");
    assert_eq!(bar.text(), Some("The party arrives."));
    roleplay.caption = Some(String::new());
    assert_eq!(CaptionBar::from_roleplay(&roleplay), None);
}

#[test]
fn an_unknown_key_is_refused() {
    assert!(serde_json::from_str::<CaptionBar>(r#"{"text":"a","title":"b"}"#).is_err());
}
