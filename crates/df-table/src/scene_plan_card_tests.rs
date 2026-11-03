use crate::plate_fixtures_tests::{SCENE, brief, display_name, market, picture, scene};
use crate::plate_timing::{PlateTiming, SceneBuildKind, TimingSetting};
use crate::private_data::Narration;
use crate::scene_plan::StageOutputs;
use crate::scene_plan_brief::SceneBrief;
use crate::scene_plan_card::{CardState, CardTag, CardWatch, EstablishingCard};

#[test]
fn the_card_shows_the_place_the_party_the_music_and_the_picture_on_its_way_tag() {
    let world = market();
    let card = EstablishingCard::compose(&brief(), &StageOutputs::default(), &world.snapshot());
    assert_eq!(card.place.as_str(), "Harbor market");
    let party: Vec<&str> = card.party.iter().map(|chip| chip.name.as_str()).collect();
    assert_eq!(party, vec!["Mara", "Pip"]);
    assert_eq!(
        card.place_music.as_ref().map(|music| music.as_str()),
        Some("harbor.ogg")
    );
    assert_eq!(card.tag, CardTag::PictureOnItsWay);
    assert_eq!(card.tag.key(), "picture_on_its_way");
}

#[test]
fn the_stored_description_is_the_first_narrator_line() {
    let world = market();
    let stored = StageOutputs {
        description: Some(Narration::new("Lanterns sway over wet stone.")),
        ..StageOutputs::default()
    };
    let card = EstablishingCard::compose(&brief(), &stored, &world.snapshot());
    assert_eq!(
        card.narrator_line.as_ref().map(Narration::expose),
        Some("Lanterns sway over wet stone.")
    );
}

#[test]
fn an_authored_description_on_the_brief_is_voiced_before_the_describe_stage_stores_one() {
    let world = market();
    let authored = SceneBrief::new(scene(SCENE), display_name("Harbor market"))
        .described(Narration::new("The authored line."));
    let card = EstablishingCard::compose(&authored, &StageOutputs::default(), &world.snapshot());
    assert_eq!(
        card.narrator_line.as_ref().map(Narration::expose),
        Some("The authored line.")
    );
}

#[test]
fn with_no_description_stored_the_card_has_no_narrator_line_and_invents_none() {
    let world = market();
    let card = EstablishingCard::compose(&brief(), &StageOutputs::default(), &world.snapshot());
    assert_eq!(card.narrator_line, None);
}

#[test]
fn a_background_that_arrives_first_is_carried_on_the_card() {
    let world = market();
    let stored = StageOutputs {
        background: Some(picture(7)),
        ..StageOutputs::default()
    };
    let card = EstablishingCard::compose(&brief(), &stored, &world.snapshot());
    assert_eq!(card.background, Some(picture(7)));
}

#[test]
fn the_card_is_due_exactly_scene_build_budget_s_after_the_build_started() {
    let watch = CardWatch::new(4_000, SceneBuildKind::Standard, PlateTiming::defaults());
    assert_eq!(watch.due_at_ms(), 12_000, "4 s start plus the 8 s budget");
    assert_eq!(
        watch.state(11_999, false),
        CardState::Building { due_in_ms: 1 }
    );
    assert_eq!(
        watch.state(12_000, false),
        CardState::Showing { late_by_ms: 0 }
    );
    assert_eq!(
        watch.state(15_500, false),
        CardState::Showing { late_by_ms: 3_500 }
    );
}

#[test]
fn a_plate_on_screen_retires_the_card_whenever_it_arrives() {
    let watch = CardWatch::new(0, SceneBuildKind::Standard, PlateTiming::defaults());
    assert_eq!(watch.state(3_000, true), CardState::Retired);
    assert_eq!(watch.state(60_000, true), CardState::Retired);
}

#[test]
fn the_first_scene_may_be_held_up_to_twenty_seconds() {
    let watch = CardWatch::new(0, SceneBuildKind::FirstScene, PlateTiming::defaults());
    assert_eq!(watch.due_at_ms(), 20_000);
}

#[test]
fn a_changed_budget_moves_the_due_time_and_the_first_scene_hold_is_never_below_it() {
    let timing = PlateTiming::defaults()
        .with(TimingSetting::SceneBuildBudgetS, 30)
        .expect("in range");
    assert_eq!(
        CardWatch::new(0, SceneBuildKind::Standard, timing).due_at_ms(),
        30_000
    );
    assert_eq!(
        CardWatch::new(0, SceneBuildKind::FirstScene, timing).due_at_ms(),
        30_000,
        "the first-scene hold of 20 s is below the budget, so the budget stands"
    );
}
