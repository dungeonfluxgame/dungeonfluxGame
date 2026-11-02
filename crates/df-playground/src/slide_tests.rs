//! A card that slides across the shared screen.

use crate::fixture::{Fixture, ImageRef, Slide};
use crate::scene::{ImageSlot, MAXIMUM_SLIDE_MS, SceneChange, SceneError, SceneOverlay};

const FIXTURE: &str = r#"{
    "name": "Base",
    "tv": {"place": "Here", "status": {"kind": "ok"}, "elements": []},
    "phone": {"hero": "Pip", "status": {"kind": "ok"}}
}"#;

fn base() -> Fixture {
    Fixture::parse(FIXTURE).expect("the fixture reads")
}

fn slide(from: u8, to: u8, ms: u32) -> Slide {
    Slide {
        from_percent: from,
        to_percent: to,
        duration_ms: ms,
        serial: 0,
    }
}

fn show_card(
    overlay: &mut SceneOverlay,
    slot: ImageSlot,
    moving: Option<Slide>,
) -> Result<(), SceneError> {
    overlay.apply(
        SceneChange::Image {
            slot,
            media: "finn-alder.webp".into(),
            caption: None,
            rotation_degrees: 180,
            slide: moving,
        },
        &base(),
    )
}

fn drawn_card(overlay: &SceneOverlay) -> ImageRef {
    let mut fixture = base();
    overlay.apply_to(&mut fixture);
    fixture.tv.card.expect("has a card")
}

#[test]
fn a_card_slides_from_one_share_of_the_screen_to_another_and_keeps_its_turn() {
    let mut overlay = SceneOverlay::default();
    show_card(&mut overlay, ImageSlot::TvCard, Some(slide(15, 85, 5_000))).expect("applies");
    let card = drawn_card(&overlay);
    let moving = card.slide.expect("slides");
    assert_eq!(
        (moving.from_percent, moving.to_percent, moving.duration_ms),
        (15, 85, 5_000)
    );
    assert_eq!(card.rotation_degrees, 180);
}

#[test]
fn each_slide_gets_the_next_serial_so_a_page_can_tell_a_new_slide_from_one_under_way() {
    let mut overlay = SceneOverlay::default();
    show_card(&mut overlay, ImageSlot::TvCard, Some(slide(15, 85, 5_000))).expect("applies");
    let first = drawn_card(&overlay).slide.expect("slides").serial;
    show_card(&mut overlay, ImageSlot::TvCard, Some(slide(15, 85, 5_000))).expect("applies again");
    let second = drawn_card(&overlay).slide.expect("slides").serial;
    assert_eq!((first, second), (1, 2));
    assert_eq!(overlay.slide_count, 2);
}

#[test]
fn a_serial_a_caller_supplies_is_replaced() {
    let mut overlay = SceneOverlay::default();
    let mut claimed = slide(0, 100, 1_000);
    claimed.serial = 999;
    show_card(&mut overlay, ImageSlot::TvCard, Some(claimed)).expect("applies");
    assert_eq!(drawn_card(&overlay).slide.expect("slides").serial, 1);
}

#[test]
fn a_slide_may_run_either_way_and_may_be_as_long_as_the_limit() {
    let mut overlay = SceneOverlay::default();
    assert!(show_card(&mut overlay, ImageSlot::TvCard, Some(slide(100, 0, 1))).is_ok());
    assert!(
        show_card(
            &mut overlay,
            ImageSlot::TvCard,
            Some(slide(0, 100, MAXIMUM_SLIDE_MS))
        )
        .is_ok()
    );
}

#[test]
fn a_slide_off_the_screen_or_of_no_length_or_too_long_is_refused_and_changes_nothing() {
    for bad in [
        slide(101, 50, 1_000),
        slide(50, 101, 1_000),
        slide(10, 90, 0),
        slide(10, 90, MAXIMUM_SLIDE_MS + 1),
    ] {
        let mut overlay = SceneOverlay::default();
        assert_eq!(
            show_card(&mut overlay, ImageSlot::TvCard, Some(bad)),
            Err(SceneError::BadSlide),
            "{bad:?}"
        );
        assert!(overlay.is_empty() && overlay.slide_count == 0);
    }
    assert!(SceneError::BadSlide.to_string().contains("1 to 60000"));
}

#[test]
fn only_the_card_on_the_shared_screen_can_slide() {
    for slot in [ImageSlot::TvBackdrop, ImageSlot::PhoneImage] {
        let mut overlay = SceneOverlay::default();
        assert_eq!(
            show_card(&mut overlay, slot, Some(slide(10, 90, 1_000))),
            Err(SceneError::SlideNeedsTheCard)
        );
        assert!(overlay.is_empty());
    }
    let mut overlay = SceneOverlay::default();
    assert!(
        show_card(&mut overlay, ImageSlot::PhoneImage, None).is_ok(),
        "no slide is always fine"
    );
}

#[test]
fn the_slide_count_survives_a_reset_and_does_not_make_the_overlay_look_changed() {
    let mut overlay = SceneOverlay::default();
    show_card(&mut overlay, ImageSlot::TvCard, Some(slide(10, 90, 1_000))).expect("applies");
    overlay
        .apply(SceneChange::Reset {}, &base())
        .expect("resets");
    assert!(overlay.is_empty());
    assert_eq!(overlay.slide_count, 1);
    show_card(&mut overlay, ImageSlot::TvCard, Some(slide(10, 90, 1_000))).expect("applies");
    assert_eq!(drawn_card(&overlay).slide.expect("slides").serial, 2);
    assert_eq!(overlay.cleared().slide_count, 2);
}

#[test]
fn a_scene_file_may_give_a_card_a_slide_and_an_older_file_without_one_still_reads() {
    let with = r#"{"media": "a.png", "slide": {"from_percent": 10, "to_percent": 90, "duration_ms": 3000}}"#;
    let card: ImageRef = serde_json::from_str(with).expect("reads");
    assert_eq!(card.slide, Some(slide(10, 90, 3_000)));
    let without: ImageRef = serde_json::from_str(r#"{"media": "a.png"}"#).expect("reads");
    assert_eq!(without.slide, None);
    assert!(
        serde_json::from_str::<ImageRef>(r#"{"media": "a.png", "slide": {"from_percent": 1}}"#)
            .is_err()
    );
}
