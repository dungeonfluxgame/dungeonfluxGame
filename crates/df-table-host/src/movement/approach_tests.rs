use df_content::world_bible::Thing;
use serde_json::json;
use tokio::io::{AsyncBufReadExt, BufReader};

use super::approach::{
    AheadWord, after_away, after_run, after_step, allowed_after, arrives, parse_options,
    shown_after, within_reach,
};
use super::design::{check_approach, parse_design};
use super::types::{
    ApproachAsk, Direction, Distance, MoveIntent, MoveKind, Pace, PartyMember, SceneDesign,
    TargetDistance,
};

fn ask(target: &str, distance: Distance, step: u32) -> ApproachAsk {
    ApproachAsk {
        target: target.to_owned(),
        toward: "the old watchtower".to_owned(),
        distance,
        step,
        pace: Pace::Walk,
        away: false,
        pose: None,
    }
}

fn step_design(target: &str, distance: Option<Distance>) -> SceneDesign {
    SceneDesign {
        place: "a rise on the road".to_owned(),
        mood: "windy".to_owned(),
        feature: "rise".to_owned(),
        elements: vec!["a rutted road".to_owned()],
        things: vec![Thing {
            id: target.to_owned(),
            name: "watchtower".to_owned(),
            look: "grey stone, a broken crown".to_owned(),
            picture: None,
        }],
        shown_items: vec![],
        residents: vec![],
        placements: vec![],
        left_behind: vec![],
        stages: vec![],
        town_look: None,
        distances: distance
            .map(|distance| {
                vec![TargetDistance {
                    id: target.to_owned(),
                    distance,
                }]
            })
            .into_iter()
            .flatten()
            .collect(),
    }
}

#[test]
fn every_closed_key_is_its_serde_name() {
    for distance in Distance::ALL {
        assert_eq!(json!(distance), json!(distance.key()));
        assert_eq!(Distance::from_key(distance.key()), Some(distance));
    }
    for direction in Direction::ALL {
        assert_eq!(json!(direction), json!(direction.key()));
        assert_eq!(Direction::from_key(direction.key()), Some(direction));
    }
    assert_eq!(Distance::from_key("far"), None);
}

#[test]
fn a_near_target_is_reached_in_a_step_or_two_and_ends_in_its_close_up() {
    assert!(within_reach(Distance::WithinReach));
    assert!(!within_reach(Distance::AcrossTheSquare));
    assert_eq!(
        after_step(Distance::AcrossTheSquare),
        &[Distance::WithinReach]
    );
    // Across the square: one plate, then the close-up.
    let mut distance = Distance::AcrossTheSquare;
    let mut plates = 0;
    while !within_reach(distance) {
        distance = after_step(distance)[after_step(distance).len() - 1];
        plates += 1;
    }
    assert_eq!(plates, 1);
}

#[test]
fn a_far_target_takes_as_many_steps_as_the_view_keeps_it_far_and_never_goes_farther() {
    // A step down the road or on the horizon may keep it as far, or bring it one band closer.
    assert_eq!(
        after_step(Distance::DownTheRoad),
        &[Distance::DownTheRoad, Distance::AcrossTheSquare]
    );
    assert_eq!(
        after_step(Distance::OnTheHorizon),
        &[Distance::OnTheHorizon, Distance::DownTheRoad]
    );
    for before in Distance::ALL {
        for after in after_step(before) {
            assert!(*after <= before, "{before:?} to {after:?} goes farther");
        }
    }
    // Walking on and on toward the horizon: every step that keeps it there is a step that stands.
    let horizon = ask("tower", Distance::OnTheHorizon, 1);
    for step in 1..=6 {
        let stays = ApproachAsk {
            step,
            ..horizon.clone()
        };
        assert_eq!(
            check_approach(&step_design("tower", Some(Distance::OnTheHorizon)), &stays),
            Ok(Distance::OnTheHorizon)
        );
    }
    // Down the road to across the square to within reach: three plates, then the close-up.
    let mut distance = Distance::DownTheRoad;
    let path = [Distance::AcrossTheSquare, Distance::WithinReach];
    for (step, next) in (1..).zip(path) {
        let judged = check_approach(
            &step_design("tower", Some(next)),
            &ask("tower", distance, step),
        );
        assert_eq!(judged, Ok(next));
        distance = next;
    }
    assert!(within_reach(distance));
}

#[test]
fn a_step_that_loses_the_target_skips_a_band_or_goes_farther_is_refused() {
    let lost = check_approach(
        &step_design("well", Some(Distance::WithinReach)),
        &ask("tower", Distance::AcrossTheSquare, 1),
    );
    assert_eq!(lost.unwrap_err().key(), "missing_approach_target");
    let skipped = check_approach(
        &step_design("tower", Some(Distance::WithinReach)),
        &ask("tower", Distance::DownTheRoad, 1),
    );
    assert_eq!(skipped.unwrap_err().key(), "invalid_approach_distance");
    let farther = check_approach(
        &step_design("tower", Some(Distance::DownTheRoad)),
        &ask("tower", Distance::AcrossTheSquare, 2),
    );
    assert_eq!(farther.unwrap_err().key(), "invalid_approach_distance");
    let unsaid = check_approach(
        &step_design("tower", None),
        &ask("tower", Distance::AcrossTheSquare, 1),
    );
    assert_eq!(unsaid.unwrap_err().key(), "invalid_approach_distance");
}

#[test]
fn a_design_gives_distances_only_to_its_own_things_and_residents_once() {
    let intent = MoveIntent {
        kind: MoveKind::Walk,
        who: vec!["hero".to_owned()],
        to: "the tower".to_owned(),
        line: "I walk toward the tower".to_owned(),
    };
    let party = [PartyMember {
        id: "hero".to_owned(),
        display_name: "Hero".to_owned(),
    }];
    let answer = |distances: serde_json::Value| {
        json!({
            "place": "a rise", "mood": "windy", "feature": "rise", "elements": ["a road"],
            "things": [{"id": "tower", "name": "tower", "look": "grey stone"}],
            "shown_items": [], "residents": [],
            "placements": [{"character": "hero", "depth": "middle", "side": "centre",
                            "doing": "walks"}],
            "distances": distances
        })
    };
    let design = parse_design(
        &answer(json!([{"id": "tower", "distance": "on_the_horizon"}])),
        &intent,
        &party,
    )
    .expect("a distance of a thing stands");
    assert_eq!(
        design.distances,
        vec![TargetDistance {
            id: "tower".to_owned(),
            distance: Distance::OnTheHorizon
        }]
    );
    for refused in [
        json!([{"id": "castle", "distance": "on_the_horizon"}]),
        json!([
            {"id": "tower", "distance": "on_the_horizon"},
            {"id": "tower", "distance": "down_the_road"}
        ]),
    ] {
        let error = parse_design(&answer(refused), &intent, &party).unwrap_err();
        assert_eq!(error.key(), "invalid_distance");
    }
    let unknown_band = parse_design(
        &answer(json!([{"id": "tower", "distance": "far"}])),
        &intent,
        &party,
    )
    .unwrap_err();
    assert_eq!(unknown_band.key(), "malformed_design");
}

#[tokio::test]
async fn a_step_made_ahead_is_shown_only_when_it_is_taken() {
    let lines = |text: &'static str| BufReader::new(text.as_bytes()).lines();
    // Not made ahead: shown at once, nothing read.
    assert!(shown_after(false, &mut lines("@drop\n")).await);
    assert!(shown_after(true, &mut lines("@take\n")).await);
    // The player did something else: dropped, or the runner closed the input.
    assert!(!shown_after(true, &mut lines("@drop\n")).await);
    assert!(!shown_after(true, &mut lines("")).await);
    // Other lines are passed over until the word comes.
    assert!(shown_after(true, &mut lines("hello\n@take\n")).await);
    assert!(!shown_after(true, &mut lines("take\n")).await);
    assert_eq!(AheadWord::of_line(" @take "), Some(AheadWord::Take));
    assert_eq!(AheadWord::of_line("@TAKE"), None);
}

#[test]
fn the_options_of_a_step_are_read_and_checked() {
    let words = |text: &[&str]| {
        text.iter()
            .map(|word| (*word).to_owned())
            .collect::<Vec<_>>()
    };
    let options = parse_options(&words(&[
        "target=tower",
        "toward=the old watchtower",
        "distance=down_the_road",
        "step=2",
        "from=explore-01",
        "plate=explore-02",
        "ahead=true",
    ]))
    .expect("the options read");
    assert_eq!(
        options.ask,
        ApproachAsk {
            step: 2,
            ..ask("tower", Distance::DownTheRoad, 2)
        }
    );
    assert_eq!(
        (options.from.as_str(), options.plate.as_str()),
        ("explore-01", "explore-02")
    );
    assert!(options.ahead);
    for bad in [
        [
            "target=Tower",
            "toward=x",
            "distance=down_the_road",
            "step=1",
            "from=a",
            "plate=b",
        ],
        [
            "target=tower",
            "toward=x",
            "distance=far",
            "step=1",
            "from=a",
            "plate=b",
        ],
        [
            "target=tower",
            "toward=x",
            "distance=down_the_road",
            "step=one",
            "from=a",
            "plate=b",
        ],
        [
            "target=tower",
            "toward=x",
            "distance=down_the_road",
            "step=1",
            "from=a.png",
            "plate=b",
        ],
    ] {
        assert!(parse_options(&words(&bad)).is_err(), "{bad:?}");
    }
}

#[test]
fn a_turn_is_step_zero_and_may_leave_what_it_faces_at_any_distance() {
    // A look behind is a turn to face it from where the hero stands, through the same path.
    let turn = ApproachAsk {
        target: "behind".to_owned(),
        toward: "the road behind".to_owned(),
        distance: Distance::WithinReach,
        step: 0,
        pace: Pace::Walk,
        away: false,
        pose: None,
    };
    assert_eq!(allowed_after(&turn), &Distance::ALL);
    for distance in Distance::ALL {
        assert_eq!(
            check_approach(&step_design("behind", Some(distance)), &turn),
            Ok(distance)
        );
    }
    assert_eq!(
        check_approach(&step_design("well", Some(Distance::WithinReach)), &turn)
            .unwrap_err()
            .key(),
        "missing_approach_target"
    );
    let words = [
        "target=behind",
        "toward=the road behind",
        "distance=on_the_horizon",
        "step=0",
        "from=a",
        "plate=b",
    ]
    .map(str::to_owned);
    assert_eq!(parse_options(&words).expect("a turn reads").ask.step, 0);
}

#[test]
fn the_step_that_reaches_a_target_is_a_plate_of_it_up_close() {
    // Within reach, the next step goes right up to it: a step like any (made ahead like any), its
    // design the target up close and still within reach.
    assert!(within_reach(Distance::WithinReach));
    assert_eq!(after_step(Distance::WithinReach), &[Distance::WithinReach]);
    let up_close = ask("tower", Distance::WithinReach, 3);
    assert_eq!(
        check_approach(
            &step_design("tower", Some(Distance::WithinReach)),
            &up_close
        ),
        Ok(Distance::WithinReach)
    );
    assert_eq!(
        check_approach(
            &step_design("tower", Some(Distance::AcrossTheSquare)),
            &up_close
        )
        .unwrap_err()
        .key(),
        "invalid_approach_distance"
    );
}

#[test]
fn a_run_covers_one_band_more_than_a_walk() {
    // "I run down the road": the step may bring the target one band nearer than a walk would.
    for before in Distance::ALL {
        for after in after_run(before) {
            assert!(*after <= before, "{before:?} to {after:?} goes farther");
        }
    }
    let run = ApproachAsk {
        pace: Pace::Run,
        ..ask("tower", Distance::OnTheHorizon, 1)
    };
    assert_eq!(allowed_after(&run), &[Distance::AcrossTheSquare]);
    assert_eq!(
        allowed_after(&ask("tower", Distance::OnTheHorizon, 1)),
        &[Distance::OnTheHorizon, Distance::DownTheRoad]
    );
    assert_eq!(
        check_approach(&step_design("tower", Some(Distance::AcrossTheSquare)), &run),
        Ok(Distance::AcrossTheSquare)
    );
    let words: Vec<String> = [
        "target=tower",
        "toward=the old watchtower",
        "distance=on_the_horizon",
        "step=1",
        "from=roam-01",
        "plate=roam-02",
        "pace=run",
    ]
    .map(str::to_owned)
    .to_vec();
    assert_eq!(
        parse_options(&words).expect("a run reads").ask.pace,
        Pace::Run
    );
    let mut sprint = words.clone();
    sprint[6] = "pace=sprint".to_owned();
    assert!(parse_options(&sprint).is_err(), "pace is closed");
}

#[test]
fn a_walk_along_a_way_is_never_arrived_at() {
    // Within reach of a thing, a step arrives; a way (walking along the wall to the left) is
    // open-ended, however near the judge put it, and a turn (step 0) never arrives either.
    assert!(arrives(&ask("tower", Distance::WithinReach, 2)));
    assert!(!arrives(&ask("tower", Distance::AcrossTheSquare, 2)));
    assert!(!arrives(&ask("tower", Distance::WithinReach, 0)));
    for way in ["left", "right", "ahead", "behind"] {
        assert!(!arrives(&ask(way, Distance::WithinReach, 1)), "{way}");
    }
}

#[test]
fn backing_away_grows_the_distance_and_never_arrives() {
    // "I walk backwards away from the gate, keeping my eyes on it": the gate grows farther.
    for before in Distance::ALL {
        for after in after_away(before) {
            assert!(*after >= before, "{before:?} to {after:?} comes nearer");
        }
    }
    let back = ApproachAsk {
        away: true,
        ..ask("gate", Distance::WithinReach, 1)
    };
    assert_eq!(
        allowed_after(&back),
        &[Distance::WithinReach, Distance::AcrossTheSquare]
    );
    assert!(!arrives(&back), "backing away never goes up to it");
    assert_eq!(
        check_approach(&step_design("gate", Some(Distance::AcrossTheSquare)), &back),
        Ok(Distance::AcrossTheSquare)
    );
    assert!(
        check_approach(&step_design("gate", Some(Distance::WithinReach)), &back).is_ok(),
        "a step back may keep it as near"
    );
}

#[test]
fn a_pose_is_read_and_told_to_the_design() {
    // "I crouch and strike a pose": the same view, the hero in that stance (step 0).
    let words: Vec<String> = [
        "target=ahead",
        "toward=the view as it is",
        "distance=within_reach",
        "step=0",
        "from=roam-01",
        "plate=roam-01-pose",
        "pose=crouched low by the crates",
    ]
    .map(str::to_owned)
    .to_vec();
    let ask = parse_options(&words).expect("a pose reads").ask;
    assert_eq!(ask.pose.as_deref(), Some("crouched low by the crates"));
    let facts = super::design::with_approach(serde_json::json!({}), &ask);
    assert_eq!(facts["approach"]["pose"], "crouched low by the crates");
}
