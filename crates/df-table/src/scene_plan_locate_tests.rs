use crate::event::FailureCause;
use crate::plate_fixtures_tests::{element, instance, phrase, place};
use crate::scene_plan_brief::{LocatePhrase, MAXIMUM_PHRASES_PER_ITEM, SetPiece};
use crate::scene_plan_locate::{
    Anchor, FigureTarget, Instance, ItemTarget, LocateError, LocateOutcome, LocatePlan,
    LocatePrompt, Region, RegionError, UnlocatedReason,
};
use crate::table_state::MediaName;

fn figure(id: &str, stated: Option<(u32, u32)>) -> FigureTarget {
    FigureTarget {
        id: element(id),
        stated: stated.map(|(x, y)| place(x, y)),
    }
}

fn item(id: &str, stated: Option<(u32, u32)>, phrases: &[&str], with_picture: bool) -> ItemTarget {
    ItemTarget {
        id: element(id),
        stated: stated.map(|(x, y)| place(x, y)),
        phrases: phrases.iter().map(|text| phrase(text)).collect(),
        picture: with_picture.then(|| MediaName::new(&format!("{id}.png")).expect("a media name")),
    }
}

fn item_prompt(round: &[LocatePrompt], id: &str) -> LocatePrompt {
    round
        .iter()
        .find(|prompt| matches!(prompt, LocatePrompt::Item { item, .. } if item.as_str() == id))
        .cloned()
        .expect("the round asks about the item")
}

fn answer(plan: &mut LocatePlan, prompt: &LocatePrompt, instances: Vec<Instance>) {
    plan.record(prompt, Ok(instances))
        .expect("the prompt was asked");
}

fn located_at(anchor: &Anchor) -> (u32, u32) {
    let Anchor::Located { region, .. } = anchor else {
        panic!("expected a located anchor, got {anchor:?}");
    };
    (
        region.centre().x_units().round() as u32,
        region.centre().y_units().round() as u32,
    )
}

#[test]
fn the_first_round_asks_for_the_people_once_and_for_each_items_first_phrase() {
    let mut plan = LocatePlan::new(
        vec![
            figure("pip", Some((30, 20))),
            figure("mara", Some((45, 20))),
        ],
        vec![
            item(
                "lantern",
                Some((80, 20)),
                &["paper lantern", "lantern"],
                false,
            ),
            item("crate", None, &["crate of fish"], false),
        ],
    )
    .expect("a plan");
    let round = plan.next_round();
    assert_eq!(round.len(), 3);
    assert_eq!(round[0], LocatePrompt::People);
    assert!(matches!(
        &round[1],
        LocatePrompt::Item { item, phrase, attempt: 1 }
            if item.as_str() == "lantern" && phrase.as_str() == "paper lantern"
    ));
    assert!(plan.next_round().is_empty(), "nothing is asked twice");
    assert!(plan.is_waiting());
}

#[test]
fn each_person_takes_the_instance_nearest_where_the_layout_says_they_stand() {
    let mut plan = LocatePlan::new(
        vec![
            figure("pip", Some((30, 20))),
            figure("mara", Some((60, 20))),
        ],
        Vec::new(),
    )
    .expect("a plan");
    let round = plan.next_round();
    // The instances are listed right to left, so order alone would swap the two people.
    answer(
        &mut plan,
        &round[0],
        vec![instance(58, 10, 4, 20, 900), instance(28, 10, 4, 20, 800)],
    );
    let outcome = plan.finish();
    assert_eq!(located_at(&outcome.figures[0].anchor), (30, 20));
    assert_eq!(located_at(&outcome.figures[1].anchor), (60, 20));
}

#[test]
fn one_instance_is_never_given_to_two_people() {
    let mut plan = LocatePlan::new(
        vec![
            figure("pip", Some((30, 20))),
            figure("mara", Some((31, 20))),
        ],
        Vec::new(),
    )
    .expect("a plan");
    let round = plan.next_round();
    answer(&mut plan, &round[0], vec![instance(28, 10, 4, 20, 900)]);
    let outcome = plan.finish();
    assert_eq!(outcome.located_figures(), 1);
    assert_eq!(outcome.unlocated_figures(), 1);
    assert!(
        matches!(outcome.figures[0].anchor, Anchor::Located { .. }),
        "the nearer person, Pip, takes it"
    );
    assert!(matches!(
        outcome.figures[1].anchor,
        Anchor::Unlocated {
            reason: UnlocatedReason::NoInstance,
            ..
        }
    ));
}

#[test]
fn a_person_the_picture_does_not_show_is_unlocated_at_the_stated_position_never_dropped() {
    let mut plan = LocatePlan::new(
        vec![
            figure("pip", Some((30, 20))),
            figure("mara", Some((45, 22))),
        ],
        Vec::new(),
    )
    .expect("a plan");
    let round = plan.next_round();
    answer(&mut plan, &round[0], Vec::new());
    let outcome = plan.finish();
    assert_eq!(outcome.figures.len(), 2, "every person has an anchor");
    let Anchor::Unlocated { fallback, reason } = &outcome.figures[1].anchor else {
        panic!("Mara is unlocated");
    };
    assert_eq!(*reason, UnlocatedReason::NoInstance);
    assert_eq!(
        *fallback,
        place(45, 22),
        "the explicit fallback is the stated position"
    );
}

#[test]
fn people_with_no_stated_position_are_spread_along_the_foreground_lane_when_lost() {
    let mut plan = LocatePlan::new(
        vec![
            figure("pip", None),
            figure("mara", None),
            figure("dain", None),
        ],
        Vec::new(),
    )
    .expect("a plan");
    let round = plan.next_round();
    answer(&mut plan, &round[0], Vec::new());
    let outcome = plan.finish();
    let fallbacks: Vec<f32> = outcome
        .figures
        .iter()
        .map(|figure| match &figure.anchor {
            Anchor::Unlocated { fallback, .. } => fallback.x_units(),
            other => panic!("expected unlocated, got {other:?}"),
        })
        .collect();
    assert_eq!(fallbacks, vec![25.0, 50.0, 75.0]);
}

#[test]
fn a_failed_people_request_leaves_everyone_unlocated_and_names_the_cause() {
    let mut plan =
        LocatePlan::new(vec![figure("pip", Some((30, 20)))], Vec::new()).expect("a plan");
    let round = plan.next_round();
    plan.record(&round[0], Err(FailureCause::TimedOut))
        .expect("asked");
    let outcome = plan.finish();
    assert_eq!(
        outcome.figures[0].anchor,
        Anchor::Unlocated {
            fallback: place(30, 20),
            reason: UnlocatedReason::LocateFailed {
                cause: FailureCause::TimedOut
            },
        }
    );
}

#[test]
fn an_item_found_by_its_first_phrase_takes_the_instance_nearest_its_stated_position() {
    let mut plan = LocatePlan::new(
        Vec::new(),
        vec![item(
            "lantern",
            Some((80, 20)),
            &["paper lantern", "lantern"],
            false,
        )],
    )
    .expect("a plan");
    let round = plan.next_round();
    answer(
        &mut plan,
        &round[0],
        vec![
            instance(10, 10, 4, 4, 990),
            instance(78, 18, 4, 4, 400),
            instance(50, 30, 4, 4, 950),
        ],
    );
    assert!(plan.next_round().is_empty());
    let outcome = plan.finish();
    assert_eq!(located_at(&outcome.items[0].anchor), (80, 20));
    assert_eq!(outcome.items[0].found_by_phrase, Some(1));
    assert_eq!(outcome.items[0].phrases_tried, 1);
}

#[test]
fn an_item_with_no_instance_is_asked_again_with_its_next_phrase() {
    let mut plan = LocatePlan::new(
        vec![figure("pip", Some((30, 20)))],
        vec![item(
            "boat",
            Some((60, 40)),
            &["rowboat", "boat", "small boat"],
            false,
        )],
    )
    .expect("a plan");
    let first = plan.next_round();
    answer(&mut plan, &first[0], vec![instance(28, 10, 4, 20, 900)]);
    answer(&mut plan, &item_prompt(&first, "boat"), Vec::new());
    let second = plan.next_round();
    assert_eq!(
        second.len(),
        1,
        "only the item that came back empty is asked again"
    );
    assert!(matches!(
        &second[0],
        LocatePrompt::Item { phrase, attempt: 2, .. } if phrase.as_str() == "boat"
    ));
    answer(&mut plan, &second[0], vec![instance(58, 38, 6, 4, 700)]);
    assert!(plan.is_settled());
    let outcome = plan.finish();
    assert_eq!(outcome.items[0].found_by_phrase, Some(2));
    assert_eq!(outcome.items[0].phrases_tried, 2);
    assert_eq!(located_at(&outcome.items[0].anchor), (61, 40));
}

#[test]
fn an_item_no_phrase_finds_falls_back_to_its_own_picture_after_three_tries() {
    let mut plan = LocatePlan::new(
        Vec::new(),
        vec![item(
            "urn",
            Some((20, 30)),
            &["clay urn", "urn", "pot"],
            true,
        )],
    )
    .expect("a plan");
    for attempt in 1..=3_u8 {
        let round = plan.next_round();
        assert_eq!(round.len(), 1);
        assert!(matches!(&round[0], LocatePrompt::Item { attempt: a, .. } if *a == attempt));
        answer(&mut plan, &round[0], Vec::new());
    }
    assert!(plan.next_round().is_empty(), "never a fourth phrase");
    assert!(plan.is_settled());
    let outcome = plan.finish();
    assert_eq!(outcome.items[0].phrases_tried, 3);
    assert_eq!(outcome.items[0].found_by_phrase, None);
    assert!(matches!(
        &outcome.items[0].anchor,
        Anchor::OwnPicture { at, picture } if *at == place(20, 30) && picture.as_str() == "urn.png"
    ));
    assert_eq!(outcome.own_picture_items(), 1);
}

#[test]
fn an_item_with_no_phrase_match_and_no_picture_is_unlocated_with_a_fallback_point() {
    let mut plan = LocatePlan::new(
        Vec::new(),
        vec![item("staff", None, &["wooden staff"], false)],
    )
    .expect("a plan");
    let round = plan.next_round();
    answer(&mut plan, &round[0], Vec::new());
    let outcome = plan.finish();
    assert!(matches!(
        &outcome.items[0].anchor,
        Anchor::Unlocated {
            reason: UnlocatedReason::NoPhraseMatched,
            ..
        }
    ));
    assert_eq!(outcome.unlocated_items(), 1);
}

#[test]
fn a_failed_call_ends_that_items_tries_and_falls_back_with_its_cause() {
    let mut plan = LocatePlan::new(
        Vec::new(),
        vec![item(
            "urn",
            Some((20, 30)),
            &["clay urn", "urn", "pot"],
            true,
        )],
    )
    .expect("a plan");
    let round = plan.next_round();
    plan.record(&round[0], Err(FailureCause::Unavailable))
        .expect("asked");
    assert!(
        plan.next_round().is_empty(),
        "a failure is not a miss: no second phrase"
    );
    let outcome = plan.finish();
    assert_eq!(outcome.items[0].phrases_tried, 1);
    assert!(matches!(
        &outcome.items[0].anchor,
        Anchor::OwnPicture { .. }
    ));
}

#[test]
fn an_answer_for_a_prompt_that_was_not_asked_is_refused() {
    let mut plan = LocatePlan::new(
        vec![figure("pip", None)],
        vec![item("urn", None, &["urn"], false)],
    )
    .expect("a plan");
    let stray = LocatePrompt::Item {
        item: element("urn"),
        phrase: phrase("urn"),
        attempt: 1,
    };
    assert_eq!(
        plan.record(&stray, Ok(Vec::new())),
        Err(LocateError::NotAsked)
    );
    assert_eq!(
        plan.record(&LocatePrompt::People, Ok(Vec::new())),
        Err(LocateError::NotAsked)
    );
}

#[test]
fn giving_up_marks_every_target_unlocated_with_the_reason_and_drops_none() {
    let figures = vec![figure("pip", Some((30, 20))), figure("mara", None)];
    let items = vec![item("lantern", Some((80, 20)), &["lantern"], false)];
    let outcome =
        LocateOutcome::unlocated(&figures, &items, UnlocatedReason::NotAsked).expect("an outcome");
    assert_eq!(outcome.figures.len(), 2);
    assert_eq!(outcome.items.len(), 1);
    assert_eq!(outcome.located_figures() + outcome.located_items(), 0);
}

#[test]
fn every_target_appears_exactly_once_whatever_the_answers() {
    let mut plan = LocatePlan::new(
        vec![
            figure("a", Some((10, 10))),
            figure("b", Some((20, 10))),
            figure("c", None),
        ],
        vec![
            item("one", Some((50, 20)), &["one", "uno"], true),
            item("two", None, &["two"], false),
        ],
    )
    .expect("a plan");
    let round = plan.next_round();
    answer(&mut plan, &round[0], vec![instance(9, 9, 2, 2, 500)]);
    answer(&mut plan, &item_prompt(&round, "one"), Vec::new());
    answer(
        &mut plan,
        &item_prompt(&round, "two"),
        vec![instance(1, 1, 2, 2, 300)],
    );
    let round = plan.next_round();
    answer(&mut plan, &round[0], Vec::new());
    let outcome = plan.finish();
    let figure_ids: Vec<&str> = outcome.figures.iter().map(|f| f.id.as_str()).collect();
    let item_ids: Vec<&str> = outcome.items.iter().map(|i| i.id.as_str()).collect();
    assert_eq!(figure_ids, vec!["a", "b", "c"]);
    assert_eq!(item_ids, vec!["one", "two"]);
}

#[test]
fn a_plan_refuses_an_item_with_no_phrase_or_with_more_than_three() {
    let none = LocatePlan::new(Vec::new(), vec![item("x", None, &[], false)]);
    assert!(matches!(none, Err(LocateError::NoPhrase { .. })));
    let four = LocatePlan::new(
        Vec::new(),
        vec![item("x", None, &["a", "b", "c", "d"], false)],
    );
    assert!(matches!(four, Err(LocateError::TooManyPhrases { .. })));
    let too_many: Vec<LocatePhrase> = (0..=MAXIMUM_PHRASES_PER_ITEM)
        .map(|index| phrase(&format!("thing {index}")))
        .collect();
    assert!(
        SetPiece::new(element("x"), crate::plate_fixtures_tests::display_name("X"))
            .locating_by(too_many)
            .is_err()
    );
}

#[test]
fn a_region_must_have_area_and_lie_on_the_canvas() {
    assert_eq!(Region::new(5, 5, 5, 9), Err(RegionError::Empty));
    assert_eq!(Region::new(0, 0, 100_001, 10), Err(RegionError::OffCanvas));
    assert_eq!(
        Instance::new(Region::new(0, 0, 10, 10).expect("a region"), 1_001),
        Err(RegionError::ScoreAboveOne)
    );
}

#[test]
fn the_head_point_is_the_middle_of_the_top_edge_of_the_region() {
    let region = Region::new(10_000, 20_000, 30_000, 50_000).expect("a region");
    assert_eq!(region.head(), place(20, 20));
    assert_eq!(region.centre(), place(20, 35));
}
