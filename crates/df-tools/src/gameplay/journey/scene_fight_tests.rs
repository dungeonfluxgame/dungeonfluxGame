//! Fight-start narration wording: the lines that tell how a fight began, in the order the
//! table must hear them (the aggressor's blow first, then who joins, then who flees).

use super::*;
use super::{Person, Plan, announce};
use crate::gameplay::{fight_foresight, place, place_creation, table_log};
use std::sync::Arc;

fn person(id: &str, name: &str) -> Person {
    Person {
        id: id.to_owned(),
        name: name.to_owned(),
    }
}

#[test]
fn the_fight_start_is_told_before_anyone_acts_in_join_flee_order() {
    let plan = Plan {
        target: person("shop-adventurer", "Corin"),
        joiners: vec![person("shop-soldier", "Jessa")],
        fleers: vec![person("merchant", "Marek")],
        out: vec![person("shop-keeper", "Tovin")],
    };
    let (lines, audit) = announce("Brakka", &plan);
    assert_eq!(
        lines,
        [
            "Brakka attacks Corin.",
            "Jessa joins the fight against Brakka.",
            "Marek flees, shouting for the watch.",
            "Tovin steps back and stays out of it.",
        ]
    );
    // The offer's button caption is never a narration line.
    assert!(lines.iter().all(|line| !line.contains("Start a fight")));
    assert!(audit.contains("aggressor=\"Brakka\""));
}

struct ArenaJev(usize);

impl fight_foresight::Jev for ArenaJev {
    fn forecast<'a>(
        &'a self,
        state: &'a serde_json::Value,
        choices: &'a [String],
    ) -> fight_foresight::ChoiceFuture<'a> {
        assert_eq!(state["fight_starting"], true);
        assert!(choices[self.0].contains("large"));
        Box::pin(async { Ok(self.0) })
    }
}

struct InstalledArena;

impl Drop for InstalledArena {
    fn drop(&mut self) {
        fight_foresight::TEST_DIRECTOR.with(|slot| *slot.borrow_mut() = None);
    }
}

fn install_arena(choice: usize) -> InstalledArena {
    fight_foresight::TEST_DIRECTOR.with(|slot| {
        *slot.borrow_mut() = Some(Arc::new(fight_foresight::FightForesight::new(Box::new(
            ArenaJev(choice),
        ))));
    });
    InstalledArena
}

fn street_record(current: &Checkpoint) -> place::PlaceRecord {
    let scene = crate::gameplay::beat_script_runtime::scene(current.state()).unwrap();
    let setting = scene.setting.as_ref().unwrap();
    let mut request = place_creation::tests::request();
    request.origin_id.clone_from(&setting.place_id);
    request.origin_name.clone_from(&setting.name);
    request
        .time_of_day
        .clone_from(&setting.model.as_ref().unwrap().time_of_day);
    request
        .weather
        .clone_from(&setting.model.as_ref().unwrap().weather);
    request.named = setting
        .model
        .as_ref()
        .unwrap()
        .ways_out
        .first()
        .map(|exit| exit.beyond_place_id.clone())
        .filter(|id| id != place::UNNAMED);
    place_creation::build(
        &request,
        &place_creation::tests::place_slot().to_string(),
        &place_creation::tests::people_slot().to_string(),
        &place_creation::tests::things_slot().to_string(),
    )
    .unwrap()
}

#[test]
fn the_opening_moves_every_fighter_through_the_validated_exit_and_survives_a_full_round_and_recovery()
 {
    tests::with_created_shop(|| {
        let _installed = install_arena(2);
        let story = tests::opening();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(fight_foresight::prepare_opening(&story));
        let destination = street_record(&story);
        let target = destination.place_id.clone();
        assert!(fight_foresight::retain_destination(
            "place-mareks-cloth-shop",
            destination
        ));
        assert!(!fight_foresight::approved_large_arena(
            "place-mareks-cloth-shop"
        ));
        assert!(fight_foresight::approved_large_arena(&target));
        let fight = tests::start(&story, 6, "shop-adventurer", None);
        assert_eq!(
            fight.basis().revision,
            story.basis().revision.next_sequence().unwrap()
        );
        assert_eq!(
            fight.state().decisions.len(),
            story.state().decisions.len() + 1
        );
        assert_eq!(
            world_place(fight.state()).unwrap().as_deref(),
            Some(target.as_str())
        );
        let committed = place::entries(fight.state()).unwrap();
        assert_eq!(committed.len(), 1);
        assert_eq!(committed[0].0, fight.basis().revision);
        assert_eq!(committed[0].1.change.to, target);
        assert_eq!(committed[0].1.change.exit, "street-door");
        assert_eq!(
            arena_narration(fight.state()).unwrap().as_deref(),
            Some("The fight spills out into Market Street.")
        );
        let arrived = crate::gameplay::beat_script_runtime::scene(fight.state()).unwrap();
        assert_eq!(arrived.setting.unwrap().place_id, target);
        assert!(encounters_admissible(&fight).unwrap());
        let battle = tactical::read(fight.state()).unwrap();
        assert_eq!(battle.combatants().len(), 4);
        for who in &fight.state().encounters[0].participants {
            let world = fight
                .state()
                .entities
                .iter()
                .find(|world| world.id == *who)
                .unwrap();
            assert_eq!(world.location, Some(room_entity().unwrap()));
            assert!(world.position.is_some());
        }
        crate::gameplay::beat_script_variety_tests::persisted(&fight);
        let first = tests::act(&fight, 7, rpc::GameplayActionKind::EndTurn, None);
        let second = tests::act(&first, 8, rpc::GameplayActionKind::EndTurn, None);
        assert!(tactical::read(second.state()).unwrap().round() > battle.round());
        assert_eq!(
            world_place(second.state()).unwrap().as_deref(),
            Some(target.as_str())
        );
        crate::gameplay::beat_script_variety_tests::persisted(&second);
    });
}

#[test]
fn a_stale_arena_exit_keeps_the_fight_at_the_source_and_flat() {
    tests::with_created_shop(|| {
        let _installed = install_arena(2);
        let story = tests::opening();
        let director = fight_foresight::TEST_DIRECTOR.with(|slot| slot.borrow().clone().unwrap());
        let stale = table_log::LinePlace {
            id: "place-mareks-cloth-shop".to_owned(),
            facts: vec!["A small shop".to_owned()],
            exits: vec![table_log::LineExit {
                id: "old-door".to_owned(),
                place_id: "old-street".to_owned(),
                description: "A broad street".to_owned(),
            }],
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        assert_eq!(
            runtime
                .block_on(director.opening(&stale, |_| false))
                .as_deref(),
            Some("old-street")
        );
        let fight = tests::start(&story, 6, "shop-adventurer", None);
        assert!(place::entries(fight.state()).unwrap().is_empty());
        assert_eq!(world_place(fight.state()).unwrap(), None);
        assert_eq!(arena_narration(fight.state()).unwrap(), None);
        assert_eq!(
            crate::gameplay::beat_script_runtime::scene(fight.state())
                .unwrap()
                .setting
                .unwrap()
                .place_id,
            "place-mareks-cloth-shop"
        );
        crate::gameplay::beat_script_variety_tests::persisted(&fight);
    });
}

#[test]
fn a_small_room_without_a_large_arena_opens_flat_without_a_move() {
    tests::with_created_shop(|| {
        let _installed = install_arena(3);
        let story = tests::opening();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        assert_eq!(
            runtime.block_on(fight_foresight::prepare_opening(&story)),
            None
        );
        assert!(!fight_foresight::approved_large_arena(
            "place-mareks-cloth-shop"
        ));
        let fight = tests::start(&story, 6, "shop-adventurer", None);
        assert_eq!(world_place(fight.state()).unwrap(), None);
        assert_eq!(arena_narration(fight.state()).unwrap(), None);
        assert!(place::entries(fight.state()).unwrap().is_empty());
        crate::gameplay::beat_script_variety_tests::persisted(&fight);
    });
}

#[test]
fn the_single_opening_journal_exposes_the_ordinary_receipt_and_normal_place_entry() {
    use prost::Message;
    tests::with_created_shop(|| {
        let _installed = install_arena(2);
        let story = tests::opening();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(fight_foresight::prepare_opening(&story));
        let destination = street_record(&story);
        let target = destination.place_id.clone();
        assert!(fight_foresight::retain_destination(
            "place-mareks-cloth-shop",
            destination.clone()
        ));
        let request = tests::assault_request(&story, 6, "shop-adventurer", None);
        let GameInput::Game(command) = wire::journey_input(
            &request,
            crate::gameplay::beat_script_variety_tests::first(),
            &story,
        )
        .unwrap() else {
            panic!("the assault is a game command");
        };
        let staged = stage_arena_move(&story, &command, Some(&target), Some(&destination)).unwrap();
        let entry = staged.movement.unwrap();
        assert_eq!(entry.change.to, target);
        assert_eq!(entry.change.created.as_ref(), Some(&destination));
        assert!(
            place::entries(story.state()).unwrap().is_empty(),
            "planning never changes the committed source"
        );
        let accepted = rpc::AcceptedAction {
            phase: rpc::JourneyPhase::Combat as i32,
            ..Default::default()
        };
        let semantic = format!(
            "{OPENING_PREFIX}{}",
            serde_json::to_string(&ArenaOpening {
                accepted: hex(&accepted.encode_to_vec()),
                arena: staged.arena,
                movement: Some(entry.clone()),
            })
            .unwrap()
        );
        let decision = AcceptedDecision {
            operation: command.operation,
            revision: story.basis().revision.next_sequence().unwrap(),
            facts: Vec::new(),
            draws: Vec::new(),
            effects: Vec::new(),
            source_policy: model::label(THREAD_POLICY).unwrap(),
            semantic_output: Some(semantic),
        };
        assert_eq!(opening_receipt(&decision).unwrap(), Some(accepted));
        assert_eq!(opening_place_move(&decision).unwrap(), Some(entry));
        assert!(
            opening_receipt(story.state().decisions.last().unwrap())
                .unwrap()
                .is_none()
        );
    });
}

#[test]
fn s1_jessa_outcome_projects_fled_dead_and_standing_people_and_body_gear_replays() {
    use crate::gameplay::{
        adventure::scene_model::{CarriedPossession, SceneChangeKind},
        freeform_action,
        social::fight_witness,
    };
    tests::with_created_shop(|| {
        let story = tests::opening();
        let fight = tests::start(
            &story,
            6,
            "shop-soldier",
            Some("out:shop-adventurer/flee:merchant"),
        );
        let over = tests::play_out(fight.clone(), 7);
        let recovered = crate::gameplay::beat_script_variety_tests::persisted(&over);
        assert_eq!(over, recovered);
        let scene = crate::gameplay::beat_script_runtime::scene(story.state()).unwrap();
        let base = scene
            .setting
            .as_ref()
            .unwrap()
            .model
            .as_ref()
            .unwrap()
            .clone();
        let mut carried = fight_witness::stat_block_possessions("guard").unwrap();
        carried.extend(
            ["plain clothes", "scuffed boots"].map(|name| CarriedPossession {
                name: name.into(),
                source: "Jessa compares warm cloaks, wearing plain clothes and scuffed boots."
                    .into(),
            }),
        );
        let possessions = std::collections::BTreeMap::from([("shop-soldier".into(), carried)]);
        let changes = outcome_changes(fight.state(), over.state(), &base, &possessions).unwrap();
        assert_eq!(
            changes,
            outcome_changes(fight.state(), recovered.state(), &base, &possessions).unwrap()
        );
        assert!(
            changes
                .iter()
                .any(|change| change.entity_id == "shop-soldier"
                    && matches!(
                        change.kind,
                        SceneChangeKind::FightEnded {
                            outcome:
                                crate::gameplay::adventure::scene_model::SceneFightOutcome::Dead,
                            ..
                        }
                    ))
        );
        let mut model = base.clone();
        for change in &changes {
            // JSON is the same durable scene journal representation recovery reads.
            let change = serde_json::from_slice(&serde_json::to_vec(change).unwrap()).unwrap();
            model.apply_change(&change).unwrap();
        }
        model.validate_state_structure().unwrap();
        let projected = model.for_scene(scene.key);
        assert!(!projected.people.iter().any(|person| person.name == "Marek"));
        let jessa = projected
            .people
            .iter()
            .find(|person| person.name == "Jessa")
            .unwrap();
        assert_eq!(
            jessa.description,
            "Jessa lies dead beside the cloak rail on the right."
        );
        assert_eq!(jessa.position, "beside the cloak rail on the right");
        let corin = projected
            .people
            .iter()
            .find(|person| person.name == "Corin")
            .unwrap();
        assert!(corin.description.contains("after the fight"));
        assert!(!corin.description.contains("examines"));
        assert!(
            !projected
                .picture_state()
                .unwrap()
                .contains("compares warm cloaks")
        );
        let hero = name(over.state(), over.state().characters[0].entity).unwrap();
        let custody =
            freeform_action::search_body(over.state(), &model, "shop-soldier", &hero).unwrap();
        assert_eq!(custody.len(), 5);
        assert_eq!(
            over.state().draws,
            recovered.state().draws,
            "searching requests no die"
        );
        for change in &custody {
            model.apply_change(change).unwrap();
        }
        let held = model
            .things
            .iter()
            .filter(|thing| {
                thing.holder_id.as_deref() == Some(&hero)
                    && thing.owner_id.as_deref() == Some("shop-soldier")
            })
            .map(|thing| thing.name.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            held,
            [
                "chain shirt",
                "shield",
                "spear",
                "plain clothes",
                "scuffed boots"
            ]
            .into_iter()
            .collect()
        );
        assert!(
            freeform_action::search_body(over.state(), &model, "shop-soldier", &hero)
                .unwrap()
                .is_empty()
        );
        assert!(
            freeform_action::search_body(over.state(), &model, "shop-adventurer", &hero).is_err()
        );
        let mut replayed = base;
        for change in changes.iter().chain(&custody) {
            replayed.apply_change(change).unwrap();
        }
        assert_eq!(replayed, model);
        for change in &changes {
            replayed.apply_change(change).unwrap();
        }
        assert_eq!(
            replayed, model,
            "outcome recovery never restores looted gear"
        );
        let observation = witness_observation(fight.state(), over.state())
            .unwrap()
            .unwrap();
        assert_eq!(observation.witnesses, ["shop-adventurer"]);
        assert_eq!(observation.deaths.len(), 1);
        assert_eq!(observation.deaths[0].person_name, "Jessa");
        assert!(observation.cause.contains("killed Jessa"));
        assert_eq!(
            observation,
            witness_observation(fight.state(), recovered.state())
                .unwrap()
                .unwrap()
        );
        assert!(
            witness_observation(over.state(), over.state())
                .unwrap()
                .is_none()
        );
    });
}

#[test]
fn s1_unconscious_and_standing_outcomes_replace_the_previous_activity_without_changing_position() {
    use crate::gameplay::adventure::scene_model::{
        SceneChange, SceneChangeKind, SceneFightOutcome,
    };
    tests::with_created_shop(|| {
        let story = tests::opening();
        let scene = crate::gameplay::beat_script_runtime::scene(story.state()).unwrap();
        let mut model = scene.setting.unwrap().model.unwrap();
        let previous = model
            .people
            .iter()
            .find(|person| person.id == "shop-soldier")
            .unwrap()
            .position
            .clone();
        for outcome in [SceneFightOutcome::Unconscious, SceneFightOutcome::Stood] {
            model
                .apply_change(&SceneChange {
                    entity_id: "shop-soldier".into(),
                    description: "Fight ended.".into(),
                    kind: SceneChangeKind::FightEnded {
                        outcome,
                        possessions: Vec::new(),
                    },
                })
                .unwrap();
            let person = model
                .people
                .iter()
                .find(|person| person.id == "shop-soldier")
                .unwrap();
            assert_eq!(person.position, previous);
            assert!(!person.description.contains("compares"));
            assert!(
                person
                    .description
                    .contains(if outcome == SceneFightOutcome::Unconscious {
                        "lies unconscious"
                    } else {
                        "stands"
                    })
            );
        }
    });
}

#[test]
fn s1_body_search_preserves_existing_carried_instances_and_does_not_duplicate_named_gear() {
    use crate::gameplay::{
        adventure::scene_model::{
            CarriedPossession, SceneChange, SceneChangeKind, SceneFightOutcome,
        },
        freeform_action,
    };
    tests::with_created_shop(|| {
        let story = tests::opening();
        let fight = tests::start(
            &story,
            6,
            "shop-soldier",
            Some("out:shop-adventurer/flee:merchant"),
        );
        let over = tests::play_out(fight, 7);
        let scene = crate::gameplay::beat_script_runtime::scene(story.state()).unwrap();
        let mut model = scene.setting.unwrap().model.unwrap();
        let existing_id = model.things[0].id.clone();
        let thing = &mut model.things[0];
        thing.name = "shield".into();
        thing.owner_id = Some("shop-soldier".into());
        thing.holder_id = Some("shop-soldier".into());
        thing.portable = true;
        thing.in_use = true;
        thing.in_scene = false;
        model
            .apply_change(&SceneChange {
                entity_id: "shop-soldier".into(),
                description: "Jessa lies dead.".into(),
                kind: SceneChangeKind::FightEnded {
                    outcome: SceneFightOutcome::Dead,
                    possessions: vec![
                        CarriedPossession {
                            name: "shield".into(),
                            source: "srd-5.2.1:p296:guard:shield".into(),
                        },
                        CarriedPossession {
                            name: "shield".into(),
                            source: "Wearing a shield.".into(),
                        },
                    ],
                },
            })
            .unwrap();
        assert_eq!(
            model
                .things
                .iter()
                .filter(|thing| thing.name == "shield")
                .count(),
            1
        );
        let hero = name(over.state(), over.state().characters[0].entity).unwrap();
        let custody =
            freeform_action::search_body(over.state(), &model, "shop-soldier", &hero).unwrap();
        assert_eq!(custody.len(), 1);
        assert_eq!(custody[0].entity_id, existing_id);
        model.apply_change(&custody[0]).unwrap();
        let found = model
            .things
            .iter()
            .find(|thing| thing.id == existing_id)
            .unwrap();
        assert_eq!(found.holder_id.as_deref(), Some(hero.as_str()));
        assert_eq!(found.owner_id.as_deref(), Some("shop-soldier"));
    });
}

#[test]
fn s1_fallen_hero_outcome_replaces_standing_description_without_duplicating_hero_inventory() {
    use crate::gameplay::adventure::scene_model::{SceneChangeKind, SceneFightOutcome};
    tests::with_created_shop(|| {
        let story = tests::opening();
        let fight = tests::start(
            &story,
            6,
            "shop-soldier",
            Some("out:shop-adventurer/flee:merchant"),
        );
        let over = tests::play_out(fight.clone(), 7);
        let scene = crate::gameplay::beat_script_runtime::scene(story.state()).unwrap();
        let base = scene.setting.unwrap().model.unwrap();
        let actor = over.state().characters[0].entity;
        let hero_name = name(over.state(), actor).unwrap();
        let possessions = std::collections::BTreeMap::from([("shop-soldier".into(), Vec::new())]);
        for dead in [false, true] {
            let mut state = over.state().clone();
            set(&mut state, actor, "hit-points", 0).unwrap();
            set(&mut state, actor, "unconscious", 1).unwrap();
            if state
                .resources
                .iter()
                .any(|resource| resource.owner == actor && resource.resource.as_str() == "dead")
            {
                set(&mut state, actor, "dead", u32::from(dead)).unwrap();
            } else {
                add_resource(&mut state, actor, "dead", i64::from(dead)).unwrap();
            }
            let changes = outcome_changes(fight.state(), &state, &base, &possessions).unwrap();
            let change = changes
                .iter()
                .find(|change| change.entity_id == hero_name)
                .unwrap();
            let SceneChangeKind::FightEnded {
                outcome,
                possessions,
            } = &change.kind
            else {
                panic!("hero outcome")
            };
            assert_eq!(
                *outcome,
                if dead {
                    SceneFightOutcome::Dead
                } else {
                    SceneFightOutcome::Unconscious
                }
            );
            assert!(possessions.is_empty());
            let mut projected = base.clone();
            projected.apply_change(change).unwrap();
            let hero = projected
                .people
                .iter()
                .find(|person| person.id == hero_name)
                .unwrap();
            assert!(hero.description.contains(if dead {
                "lies dead"
            } else {
                "lies unconscious"
            }));
            assert!(!hero.description.contains("Standing"));
            assert_eq!(projected.things, base.things);
        }
    });
}
