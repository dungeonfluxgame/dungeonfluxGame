use df_table::event::FailureCause;
use df_table::plate::scheduler::JobEnd;
use df_table::plate::timing::SceneBuildKind;
use df_table::private_data::Narration;
use df_table::scene_plan::SceneStage;
use df_table::scene_plan::card::CardState;
use df_table::scene_plan::locate::{Anchor, UnlocatedReason};

use super::builder::{BuilderError, BuilderOutput};
use super::dev_world::dev_script;
use super::events::{SceneBuilderEvent, StageSource};
use super::fixtures::{Rig, plate_of, requested_stages};
use super::purposes::BuildPriority;
use crate::broker::BrokerEvent;
use crate::broker::providers::ScriptedCall;
use crate::broker::purpose::{Fallback, QueueClass};
use crate::broker::request::{Epoch, ProviderFault};

fn failing(calls: usize) -> Vec<ScriptedCall> {
    vec![ScriptedCall::fails_after(0, ProviderFault::Refused); calls]
}

fn ended_with(outputs: &[BuilderOutput]) -> Vec<(u64, JobEnd)> {
    outputs
        .iter()
        .filter_map(|output| match output {
            BuilderOutput::JobEnded(outcome) => Some((outcome.job.get(), outcome.end)),
            BuilderOutput::PlateReady(_) => None,
        })
        .collect()
}

#[test]
fn the_stages_run_in_order_each_as_its_own_request_and_each_after_the_one_before_was_stored() {
    let mut rig = Rig::new();
    let outputs = rig.run_one_job();
    assert!(
        plate_of(&outputs).is_some(),
        "the job finished with a plate"
    );
    assert_eq!(
        requested_stages(&rig.audit),
        vec!["describe", "background", "plate", "locate"]
    );
    let events = rig.audit.scene_events();
    for (index, event) in events.iter().enumerate() {
        let SceneBuilderEvent::StageRequested { stage, .. } = event else {
            continue;
        };
        let Some(needs) = stage.needs() else { continue };
        let stored_before = events[..index].iter().any(|earlier| {
            matches!(earlier, SceneBuilderEvent::StageStored { stage, .. } if *stage == needs)
        });
        assert!(
            stored_before,
            "the {} request was made before the {} output was stored",
            stage.key(),
            needs.key()
        );
    }
    let stored: Vec<&str> = events
        .iter()
        .filter_map(|event| match event {
            SceneBuilderEvent::StageStored { stage, .. } => Some(stage.key()),
            _ => None,
        })
        .collect();
    assert_eq!(stored, vec!["describe", "background", "plate", "locate"]);
}

#[test]
fn each_stage_asks_the_broker_for_its_own_purpose_in_its_own_class() {
    let mut rig = Rig::new();
    rig.run_one_job();
    let requested: Vec<(String, QueueClass)> = rig
        .audit
        .broker_events()
        .into_iter()
        .filter_map(|event| match event {
            BrokerEvent::Requested { purpose, class, .. } => Some((purpose.to_string(), class)),
            _ => None,
        })
        .collect();
    assert_eq!(requested[0], ("describe".to_owned(), QueueClass::Live));
    assert_eq!(requested[1], ("background".to_owned(), QueueClass::Live));
    assert_eq!(requested[2], ("plate".to_owned(), QueueClass::Plate));
    assert!(
        requested[3..]
            .iter()
            .all(|(purpose, class)| purpose == "locate" && *class == QueueClass::Plate)
    );
}

#[test]
fn locate_finds_each_target_by_the_policy_of_the_s2_result() {
    let mut rig = Rig::new();
    let outputs = rig.run_one_job();
    let plate = plate_of(&outputs).expect("a plate");
    let figures = &plate.locate.figures;
    assert_eq!(
        figures.len(),
        3,
        "both heroes and the person who lives here"
    );
    assert!(matches!(figures[1].anchor, Anchor::Located { .. }), "Pip");
    assert!(matches!(figures[2].anchor, Anchor::Located { .. }), "Mara");
    assert_eq!(figures[0].id.as_str(), "local");
    assert!(matches!(
        figures[0].anchor,
        Anchor::Unlocated {
            reason: UnlocatedReason::NoInstance,
            ..
        }
    ));
    let items = &plate.locate.items;
    let by_id = |id: &str| {
        items
            .iter()
            .find(|item| item.id.as_str() == id)
            .expect("an item")
    };
    assert_eq!(by_id("lantern").found_by_phrase, Some(1));
    assert_eq!(by_id("crate").found_by_phrase, Some(2));
    assert_eq!(by_id("crate").phrases_tried, 2);
    let urn = by_id("urn");
    assert_eq!(urn.phrases_tried, 3);
    assert!(
        matches!(urn.anchor, Anchor::OwnPicture { .. }),
        "the urn falls back to its picture"
    );
}

#[test]
fn locate_makes_one_request_for_the_people_and_one_for_each_phrase_it_tries() {
    let mut rig = Rig::new();
    rig.run_one_job();
    let locate_calls = rig
        .audit
        .requested_purposes()
        .iter()
        .filter(|purpose| purpose.as_str() == "locate")
        .count();
    assert_eq!(
        locate_calls,
        1 + 1 + 2 + 3,
        "people, lantern, crate twice, urn three times"
    );
    let rounds = rig
        .audit
        .count(|event| matches!(event, SceneBuilderEvent::LocateRound { .. }));
    assert_eq!(
        rounds, 2,
        "the second and third rounds ask only for the items that came back empty"
    );
}

#[test]
fn a_job_after_the_first_starts_at_the_plate_stage() {
    let mut rig = Rig::new();
    rig.run_one_job();
    let before = requested_stages(&rig.audit).len();
    rig.world
        .move_cast("pip", 36, 20)
        .expect("a visible change");
    let outputs = rig.run_one_job();
    assert!(plate_of(&outputs).is_some());
    let later: Vec<&str> = requested_stages(&rig.audit)[before..].to_vec();
    assert_eq!(later, vec!["plate", "locate"]);
}

#[test]
fn an_authored_description_is_stored_without_asking_for_it() {
    let mut rig = Rig::new();
    let authored = rig
        .brief
        .clone()
        .described(Narration::new("Lanterns sway over wet stone."));
    rig.world.enter_scene(rig.scene).expect("enters again");
    let generation = rig
        .world
        .snapshot()
        .scene(rig.scene)
        .expect("open")
        .generation();
    rig.builder
        .open_scene(
            authored,
            generation,
            SceneBuildKind::Standard,
            BuildPriority::Current,
            0,
        )
        .expect("opens");
    rig.generation = generation;
    rig.run_one_job();
    assert_eq!(
        requested_stages(&rig.audit),
        vec!["background", "plate", "locate"],
        "describe was never asked for"
    );
    assert!(rig.audit.scene_events().iter().any(|event| matches!(
        event,
        SceneBuilderEvent::StageStored {
            stage: SceneStage::Describe,
            source: StageSource::SceneRecord,
            ..
        }
    )));
}

#[test]
fn a_describe_that_fails_ends_the_job_with_the_place_card_and_asks_for_nothing_after_it() {
    let script = dev_script()
        .expect("the dev script")
        .with_calls(&Rig::purpose("describe"), failing(1));
    let mut rig = Rig::with(script, SceneBuildKind::Standard, BuildPriority::Current);
    let outputs = rig.run_one_job();
    assert_eq!(ended_with(&outputs), vec![(1, JobEnd::Failed)]);
    let BuilderOutput::JobEnded(outcome) = &outputs[0] else {
        panic!("the job ended");
    };
    let failure = outcome.failure.expect("a failure");
    assert_eq!(failure.stage, SceneStage::Describe);
    assert_eq!(failure.cause, FailureCause::Refused);
    assert_eq!(requested_stages(&rig.audit), vec!["describe"]);
    assert!(rig.audit.scene_events().iter().any(|event| matches!(
        event,
        SceneBuilderEvent::StageFailed {
            fallback: Fallback::PlaceCard,
            cause: FailureCause::Refused,
            ..
        }
    )));
}

#[test]
fn a_background_that_fails_leaves_the_establishing_card_and_starts_no_plate() {
    let script = dev_script()
        .expect("the dev script")
        .with_calls(&Rig::purpose("background"), failing(1));
    let mut rig = Rig::with(script, SceneBuildKind::Standard, BuildPriority::Current);
    let outputs = rig.run_one_job();
    assert_eq!(ended_with(&outputs), vec![(1, JobEnd::Failed)]);
    assert_eq!(requested_stages(&rig.audit), vec!["describe", "background"]);
    assert!(rig.audit.scene_events().iter().any(|event| matches!(
        event,
        SceneBuilderEvent::StageFailed {
            fallback: Fallback::EstablishingCard,
            ..
        }
    )));
}

#[test]
fn a_plate_that_fails_holds_the_last_plate_and_does_not_locate() {
    let script = dev_script()
        .expect("the dev script")
        .with_calls(&Rig::purpose("plate"), failing(1));
    let mut rig = Rig::with(script, SceneBuildKind::Standard, BuildPriority::Current);
    let outputs = rig.run_one_job();
    assert_eq!(ended_with(&outputs), vec![(1, JobEnd::Failed)]);
    assert_eq!(
        requested_stages(&rig.audit),
        vec!["describe", "background", "plate"]
    );
    assert!(rig.audit.scene_events().iter().any(|event| matches!(
        event,
        SceneBuilderEvent::StageFailed {
            fallback: Fallback::HoldLastPlate,
            ..
        }
    )));
}

#[test]
fn a_locate_that_fails_still_gives_a_plate_with_every_target_unlocated_never_dropped() {
    let script = dev_script()
        .expect("the dev script")
        .with_calls(&Rig::purpose("locate"), failing(10));
    let mut rig = Rig::with(script, SceneBuildKind::Standard, BuildPriority::Current);
    let outputs = rig.run_one_job();
    let plate = plate_of(&outputs).expect("a failed Locate does not stop the plate");
    assert_eq!(plate.locate.figures.len(), 3);
    assert_eq!(plate.locate.located_figures(), 0);
    assert!(plate.locate.figures.iter().all(|figure| matches!(
        figure.anchor,
        Anchor::Unlocated {
            reason: UnlocatedReason::LocateFailed {
                cause: FailureCause::Refused
            },
            ..
        }
    )));
    assert_eq!(plate.locate.items.len(), 3, "no item is dropped");
    assert_eq!(
        plate.locate.own_picture_items(),
        2,
        "the lantern and the urn have pictures"
    );
    assert_eq!(
        plate.locate.unlocated_items(),
        1,
        "the crate has none, so it has a fallback point"
    );
}

#[test]
fn a_result_for_a_scene_entered_again_is_rejected_and_nothing_is_stored() {
    let mut rig = Rig::new();
    rig.submit_new_job();
    rig.step();
    rig.world.enter_scene(rig.scene).expect("enters again");
    let outputs = rig.run_until(60_000, |outputs| !outputs.is_empty());
    assert_eq!(ended_with(&outputs), vec![(1, JobEnd::Stale)]);
    assert!(plate_of(&outputs).is_none());
    assert!(
        rig.audit.scene_events().iter().any(|event| matches!(
            event,
            SceneBuilderEvent::Table(table_event) if table_event.render_audit_line().contains("cause=generation_mismatch")
        )),
        "the rejection is recorded once as the pure crate's own event"
    );
    assert_eq!(
        rig.audit
            .count(|event| matches!(event, SceneBuilderEvent::StageStored { .. })),
        0,
        "nothing was stored for the stale scene"
    );
}

#[test]
fn a_stale_job_cancels_the_requests_it_has_out() {
    let mut rig = Rig::new();
    rig.submit_new_job();
    rig.step();
    assert!(
        rig.audit
            .broker_events()
            .iter()
            .any(|event| matches!(event, BrokerEvent::Requested { .. }))
    );
    rig.world.enter_scene(rig.scene).expect("enters again");
    rig.run_until(1_000, |outputs| !outputs.is_empty());
    assert!(
        rig.audit
            .broker_events()
            .iter()
            .any(|event| matches!(event, BrokerEvent::Cancelled { .. }))
    );
}

#[test]
fn a_broker_epoch_that_moved_on_supersedes_the_request_and_the_job_ends_stale() {
    let mut rig = Rig::new();
    rig.submit_new_job();
    rig.step();
    rig.hub
        .advance_epoch(Epoch::new(rig.generation.get() + 1))
        .expect("moves forward");
    let outputs = rig.run_until(60_000, |outputs| !outputs.is_empty());
    assert_eq!(ended_with(&outputs), vec![(1, JobEnd::Stale)]);
    assert_eq!(
        requested_stages(&rig.audit),
        vec!["describe"],
        "nothing is asked after the epoch passed"
    );
}

#[test]
fn a_paused_table_holds_the_build_and_a_resume_makes_what_was_in_flight_stale() {
    let mut rig = Rig::new();
    rig.submit_new_job();
    rig.step();
    rig.world.pause().expect("pauses");
    let held = rig.run_until(5_000, |outputs| !outputs.is_empty());
    assert!(held.is_empty(), "a paused table holds results back");
    assert_eq!(
        requested_stages(&rig.audit),
        vec!["describe"],
        "and starts nothing"
    );
    rig.world.resume().expect("resumes");
    let outputs = rig.run_until(5_000, |outputs| !outputs.is_empty());
    assert_eq!(ended_with(&outputs), vec![(1, JobEnd::Stale)]);
}

#[test]
fn a_slow_stage_does_not_block_play_the_card_falls_due_once_and_the_build_goes_on() {
    let script = dev_script()
        .expect("the dev script")
        .with_latency(&Rig::purpose("background"), 20_000);
    let mut rig = Rig::with(script, SceneBuildKind::Standard, BuildPriority::Current);
    rig.submit_new_job();
    rig.run_until(7_900, |_| false);
    let early = rig
        .builder
        .card(rig.scene, rig.now_ms(), &rig.world.snapshot(), false)
        .expect("a card");
    assert!(
        matches!(early.state, CardState::Building { .. }),
        "inside the 8 s budget"
    );
    assert_eq!(
        rig.audit
            .count(|event| matches!(event, SceneBuilderEvent::CardDue { .. })),
        0
    );
    rig.run_until(400, |_| false);
    let late = rig
        .builder
        .card(rig.scene, rig.now_ms(), &rig.world.snapshot(), false)
        .expect("a card");
    assert!(
        matches!(late.state, CardState::Showing { .. }),
        "play went on to the card"
    );
    assert_eq!(
        late.card.narrator_line.as_ref().map(Narration::expose),
        Some("A quiet place, much as it always is."),
        "the stored description is the first narrator line"
    );
    assert_eq!(
        rig.audit
            .count(|event| matches!(event, SceneBuilderEvent::CardDue { .. })),
        1
    );
    rig.run_until(2_000, |_| false);
    assert_eq!(
        rig.audit
            .count(|event| matches!(event, SceneBuilderEvent::CardDue { .. })),
        1,
        "reported once"
    );
    let outputs = rig.run_until(600_000, |outputs| !outputs.is_empty());
    assert!(
        plate_of(&outputs).is_some(),
        "the build went on and finished late"
    );
    let retired = rig
        .builder
        .card(rig.scene, rig.now_ms(), &rig.world.snapshot(), true)
        .expect("a card");
    assert_eq!(retired.state, CardState::Retired);
}

#[test]
fn the_first_scene_is_held_for_twenty_seconds_before_the_card() {
    let script = dev_script()
        .expect("the dev script")
        .with_latency(&Rig::purpose("background"), 30_000);
    let mut rig = Rig::with(script, SceneBuildKind::FirstScene, BuildPriority::Current);
    rig.submit_new_job();
    rig.run_until(19_900, |_| false);
    assert_eq!(
        rig.audit
            .count(|event| matches!(event, SceneBuilderEvent::CardDue { .. })),
        0
    );
    rig.run_until(400, |_| false);
    assert_eq!(
        rig.audit
            .count(|event| matches!(event, SceneBuilderEvent::CardDue { .. })),
        1
    );
}

#[test]
fn a_card_with_no_description_yet_has_no_narrator_line() {
    let script = dev_script()
        .expect("the dev script")
        .with_latency(&Rig::purpose("describe"), 30_000);
    let mut rig = Rig::with(script, SceneBuildKind::Standard, BuildPriority::Current);
    rig.submit_new_job();
    rig.run_until(9_000, |_| false);
    let card = rig
        .builder
        .card(rig.scene, rig.now_ms(), &rig.world.snapshot(), false)
        .expect("a card");
    assert!(matches!(card.state, CardState::Showing { .. }));
    assert_eq!(card.card.narrator_line, None);
    assert_eq!(card.card.place.as_str(), "Harbor market");
    let party: Vec<&str> = card
        .card
        .party
        .iter()
        .map(|chip| chip.name.as_str())
        .collect();
    assert_eq!(party, vec!["Mara", "Pip"]);
}

#[test]
fn a_lookahead_build_asks_for_the_lookahead_purposes_in_the_lowest_class() {
    let mut rig = Rig::with(
        dev_script().expect("the dev script"),
        SceneBuildKind::Standard,
        BuildPriority::Lookahead,
    );
    let outputs = rig.run_one_job();
    assert!(plate_of(&outputs).is_some());
    let purposes = rig.audit.requested_purposes();
    assert_eq!(
        &purposes[..3],
        [
            "lookahead_describe",
            "lookahead_background",
            "lookahead_plate"
        ]
    );
    assert!(
        purposes[3..]
            .iter()
            .all(|purpose| purpose == "lookahead_locate")
    );
    let classes: Vec<QueueClass> = rig
        .audit
        .broker_events()
        .into_iter()
        .filter_map(|event| match event {
            BrokerEvent::Requested { class, .. } => Some(class),
            _ => None,
        })
        .collect();
    assert!(classes.iter().all(|class| *class == QueueClass::Lookahead));
}

#[test]
fn a_scene_cannot_be_opened_under_a_generation_that_is_not_forward() {
    let mut rig = Rig::new();
    let result = rig.builder.open_scene(
        rig.brief.clone(),
        rig.generation,
        SceneBuildKind::Standard,
        BuildPriority::Current,
        0,
    );
    assert_eq!(
        result.err(),
        Some(BuilderError::GenerationNotForward {
            current: rig.generation,
            requested: rig.generation,
        })
    );
}

#[test]
fn a_job_for_an_unopened_scene_or_another_generation_is_refused() {
    let mut rig = Rig::new();
    let job = rig.take_job();
    rig.world.enter_scene(rig.scene).expect("enters again");
    let new_generation = rig
        .world
        .snapshot()
        .scene(rig.scene)
        .expect("open")
        .generation();
    rig.builder
        .open_scene(
            rig.brief.clone(),
            new_generation,
            SceneBuildKind::Standard,
            BuildPriority::Current,
            0,
        )
        .expect("opens");
    assert_eq!(
        rig.builder.submit(job).err(),
        Some(BuilderError::JobForOtherGeneration {
            scene: new_generation,
            job: rig.generation,
        })
    );
}

#[test]
fn opening_a_scene_again_retires_the_old_build_and_cancels_what_it_had_out() {
    let mut rig = Rig::new();
    rig.submit_new_job();
    rig.step();
    rig.world.enter_scene(rig.scene).expect("enters again");
    let new_generation = rig
        .world
        .snapshot()
        .scene(rig.scene)
        .expect("open")
        .generation();
    let ended = rig
        .builder
        .open_scene(
            rig.brief.clone(),
            new_generation,
            SceneBuildKind::Standard,
            BuildPriority::Current,
            rig.now_ms(),
        )
        .expect("opens");
    assert_eq!(ended_with(&ended), vec![(1, JobEnd::Stale)]);
    assert_eq!(rig.builder.jobs_in_flight(rig.scene), 0);
}

#[test]
fn the_broker_s_full_plate_queue_drops_the_oldest_job_and_the_job_ends_dropped() {
    let mut rig = Rig::new();
    rig.run_one_job();
    let mut ids = Vec::new();
    for step in 0..6 {
        rig.world
            .move_cast("pip", 31 + step, 20)
            .expect("a visible change");
        ids.push(rig.submit_new_job().get());
    }
    let now = rig.now_ms();
    let first = rig.builder.advance(now, &rig.world.snapshot());
    assert!(first.is_empty(), "nothing has been answered yet");
    let after = rig.builder.advance(now, &rig.world.snapshot());
    let dropped: Vec<u64> = ended_with(&after)
        .into_iter()
        .filter(|(_, end)| *end == JobEnd::Dropped)
        .map(|(job, _)| job)
        .collect();
    assert_eq!(
        dropped,
        vec![ids[0], ids[1]],
        "the two oldest of six, with room for four"
    );
}

#[test]
fn two_jobs_that_would_draw_the_same_picture_share_one_flight() {
    let mut rig = Rig::new();
    rig.run_one_job();
    rig.submit_new_job();
    rig.submit_new_job();
    rig.step();
    let plates = rig
        .audit
        .requested_purposes()
        .into_iter()
        .filter(|purpose| purpose == "plate")
        .count();
    assert_eq!(
        plates, 2,
        "the first job's plate and one shared by the other two"
    );
    assert!(
        rig.audit
            .broker_events()
            .iter()
            .any(|event| matches!(event, BrokerEvent::Joined { .. }))
    );
}

#[test]
fn the_audit_lines_hold_ids_keys_and_counts_never_words() {
    let mut rig = Rig::new();
    rig.run_one_job();
    for line in rig.audit.scene_lines() {
        assert!(
            line.starts_with("SCENE ") || line.starts_with("PLATE "),
            "{line}"
        );
        assert!(!line.contains("Harbor"), "no place name in a line: {line}");
        assert!(!line.contains("lantern"), "no phrase in a line: {line}");
        assert!(
            !line.contains("quiet place"),
            "no description in a line: {line}"
        );
    }
}
