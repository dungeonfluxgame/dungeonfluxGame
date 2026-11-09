//! The brand mark's defaults and the one surface the scene logo and the top bar's logo share.

use crate::kit::brand::BrandMark;
use crate::lobby::model::LogoStyle;

#[test]
fn an_empty_mark_is_the_inline_wordmark_without_a_place() {
    let mark: BrandMark = serde_json::from_str("{}").expect("an empty mark reads");
    assert_eq!(mark.style(), LogoStyle::Inline);
    assert_eq!(mark.wordmark(), "DungeonFlux");
    assert_eq!(mark.place(), None);
}

#[test]
fn a_blank_wordmark_falls_back_and_a_place_is_trimmed() {
    let mark: BrandMark =
        serde_json::from_str(r#"{"style":"stacked","wordmark":" ","place":" Valemere "}"#)
            .expect("a mark reads");
    assert_eq!(mark.style(), LogoStyle::Stacked);
    assert_eq!(mark.wordmark(), "DungeonFlux");
    assert_eq!(mark.place(), Some("Valemere"));
}

#[test]
fn a_scene_draws_the_top_bar_logo_or_its_brand_flag_and_nothing_otherwise() {
    assert_eq!(BrandMark::for_scene(false, None, "Quay"), None);
    let flagged = BrandMark::for_scene(true, None, "Quay").expect("the brand flag draws a mark");
    assert_eq!(flagged.style(), LogoStyle::Inline);
    assert_eq!(flagged.place(), Some("Quay"));
    let bar = BrandMark::for_scene(false, Some(LogoStyle::Stacked), "").expect("a logo draws");
    assert_eq!(bar.style(), LogoStyle::Stacked);
    assert_eq!(bar.place(), None);
}

#[test]
fn an_unknown_key_is_refused() {
    assert!(serde_json::from_str::<BrandMark>(r#"{"logo":"inline"}"#).is_err());
}
