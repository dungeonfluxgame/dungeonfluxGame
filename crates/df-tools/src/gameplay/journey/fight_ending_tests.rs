//! A fight chapter ends like every other: a resolution, an epilogue, then an end card that
//! lets the table start another adventure. These tests play fights to their end through submitted actions only.
use super::tactical::FightOutcome;
use super::tests::{inn_command, input};
use super::*;
use crate::gameplay::{beat_script, beat_script_runtime as runtime};
use prost::Message;
use rpc::GameplayActionKind as K;

fn first() -> MemberId {
    MemberId::from_bytes(&MEMBERS[0]).unwrap()
}

fn live(run: impl FnOnce()) {
    let mut tuning: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../../../content/harbor-journey/tuning.json"
    ))
    .unwrap();
    tuning["tactical"] =
        serde_json::json!({"enemies":[{"srd_id":"bandit","count":2,"ammunition":20}]});
    let loaded = pack::load_from_bytes(
        include_bytes!("../../../../../content/harbor-journey/pack.json"),
        &serde_json::to_vec(&tuning).unwrap(),
    )
    .unwrap();
    pack::with_test_pack(loaded, run);
}

fn persisted(current: &Checkpoint) -> Checkpoint {
    let codec = df_persistence::NativeCodecLimits {
        maximum_document_bytes: 2 * 1024 * 1024,
        maximum_allocated_bytes: 2 * 1024 * 1024,
        maximum_collection_items: 4096,
        maximum_text_bytes: 4096,
    };
    let bytes =
        df_persistence::local_demo_scope::encode_owned_demo_checkpoint(current, codec).unwrap();
    let recovered = df_persistence::local_demo_scope::decode_owned_demo_checkpoint(
        &bytes,
        current.basis(),
        &model::pins().unwrap(),
        &model::recovery().unwrap(),
        codec,
    )
    .unwrap();
    assert_eq!(&recovered, current);
    phase(&recovered).unwrap();
    recovered
}

fn request(
    current: &Checkpoint,
    operation: u8,
    kind: K,
    offer_id: String,
) -> rpc::SubmitActionRequest {
    rpc::SubmitActionRequest {
        session_id: Some(rpc::SessionId {
            value: Some(current.basis().session.as_bytes().to_vec()),
        }),
        run_id: Some(rpc::RunId {
            value: Some(current.basis().run.as_bytes().to_vec()),
        }),
        observed_revision: Some(wire::revision(current.basis().revision)),
        operation_id: Some(rpc::OperationId {
            value: Some(vec![operation; 16]),
        }),
        offer_id,
        action_kind: kind as i32,
        ..Default::default()
    }
}

/// What the player's phone lists: kind, label and opaque id.
fn listed(current: &Checkpoint) -> Vec<(K, String, String)> {
    runtime::view_offers(current, first()).unwrap()
}

fn ending_step(current: &Checkpoint, operation: u8) -> Result<Checkpoint, RepositoryError> {
    let (kind, _, id) = listed(current)
        .into_iter()
        .find(|(kind, _, _)| *kind == K::ResolveErrand)
        .expect("an ending step is offered");
    let request = request(current, operation, kind, id.clone());
    assert_eq!(
        wire::action_offer_rejection(current, first(), kind, &id, &[]).unwrap(),
        None,
        "the server accepts what the phone lists"
    );
    let request = rpc::SubmitActionRequest::decode(request.encode_to_vec().as_slice()).unwrap();
    let input = wire::journey_input(&request, first(), current).unwrap();
    stage_with_supplier(current, &input, &mut |_| {
        panic!("an ending step rolls no dice")
    })
}

fn beat_id(current: &Checkpoint) -> String {
    beat_script::with_script(|script| script.current(current.state()).unwrap().id.clone())
}

fn kinds(current: &Checkpoint) -> Vec<K> {
    listed(current)
        .into_iter()
        .map(|(kind, _, _)| kind)
        .collect()
}

fn heroes_at_hit_points(current: &Checkpoint, hit_points: u32) -> Checkpoint {
    let mut state = current.state().clone();
    let actors = state.encounters[0]
        .tactical
        .as_ref()
        .expect("tactical fight")
        .actors
        .iter()
        .filter_map(|(actor, entity)| {
            state
                .characters
                .iter()
                .any(|character| character.entity == *entity)
                .then_some(*actor)
        })
        .collect::<Vec<_>>();
    for character in &state.characters {
        let resource = state
            .resources
            .iter_mut()
            .find(|resource| {
                resource.owner == character.entity && resource.resource.as_str() == "hit-points"
            })
            .expect("hero hit-point resource");
        resource.value = i64::from(hit_points);
        if let Some(unconscious) = state.resources.iter_mut().find(|resource| {
            resource.owner == character.entity && resource.resource.as_str() == "unconscious"
        }) {
            unconscious.value = 0;
        }
        for (resource_name, resource_value) in [("hit-point-dice", 1), ("short-rest-choice", 0)] {
            if let Some(resource) = state.resources.iter_mut().find(|resource| {
                resource.owner == character.entity && resource.resource.as_str() == resource_name
            }) {
                resource.value = resource_value;
            }
        }
    }
    let snapshot = state.encounters[0]
        .tactical
        .as_mut()
        .expect("tactical fight");
    let mut battle: serde_json::Value = serde_json::from_slice(&snapshot.state).unwrap();
    for actor in actors {
        let combatant = battle["actors"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|combatant| combatant["spec"]["id"].as_u64() == Some(u64::from(actor)))
            .expect("hero combatant");
        combatant["health"]["hit_points"] = serde_json::json!(hit_points);
        combatant["health"]["unconscious"] = serde_json::json!(false);
        combatant["health"]["knocked_out"] = serde_json::json!(false);
        combatant["health"]["dead"] = serde_json::json!(false);
        combatant["health"]["stable"] = serde_json::json!(false);
        combatant["health"]["death_successes"] = serde_json::json!(0);
        combatant["health"]["death_failures"] = serde_json::json!(0);
    }
    snapshot.state = serde_json::to_vec(&battle).unwrap();
    model::checkpoint(current.basis(), state).unwrap()
}

fn assert_player_offers_are_accepted(current: &Checkpoint, member: MemberId) {
    let view = wire::journey_view(
        current,
        df_persistence::local_demo_scope::LocalDemoRole::Player,
        member,
    )
    .unwrap();
    let Some(rpc::view_message::Audience::Player(player)) = view.audience else {
        panic!("player view");
    };
    for (index, offer) in player.offers.iter().enumerate() {
        let kind = K::try_from(offer.action_kind).expect("listed action kind");
        let choices = offer
            .input_groups
            .iter()
            .map(|group| {
                let selected = group
                    .options
                    .iter()
                    .position(|option| {
                        matches!(
                            group.group_id.as_str(),
                            "weapon-mastery" | "savage-attacker"
                        ) && option.option_id == "no"
                    })
                    .or_else(|| (!group.options.is_empty()).then_some(0))
                    .expect("listed choice group has options");
                rpc::JourneyChoice {
                    group_id: group.group_id.clone(),
                    option_id: group.options[selected].option_id.clone(),
                }
            })
            .collect::<Vec<_>>();
        let mut request = rpc::SubmitActionRequest {
            offer_id: offer.offer_id.clone(),
            action_kind: kind as i32,
            destination: offer
                .destinations
                .first()
                .map_or(rpc::HarborDestination::Unspecified as i32, |destination| {
                    destination.destination
                }),
            action_choices: choices,
            ..request(
                current,
                u8::try_from(index + 100).expect("offer count"),
                kind,
                String::new(),
            )
        };
        if kind == K::CreateCharacter {
            request.character = Some(rpc::CharacterSelection {
                name: "Newhero".to_owned(),
                choices: crate::gameplay::beat_script_tests::build("Newhero")
                    .into_iter()
                    .filter(|(group, _)| group.as_str() != "name")
                    .chain([(
                        model::label("portrait").expect("portrait group"),
                        model::label("portrait-hero-1").expect("accepted quickstart portrait"),
                    )])
                    .map(|(group, option)| rpc::JourneyChoice {
                        group_id: group.as_str().to_owned(),
                        option_id: option.as_str().to_owned(),
                    })
                    .collect(),
                ..Default::default()
            });
        }
        assert_eq!(
            wire::action_offer_rejection(
                current,
                member,
                kind,
                &request.offer_id,
                &request.action_choices,
            )
            .unwrap(),
            None,
            "{} ({kind:?}) is listed in {:?}",
            offer.label,
            phase(current).unwrap()
        );
        let input = wire::journey_input(&request, member, current).unwrap();
        let staged = stage_with_supplier(current, &input, &mut |sides| {
            Ok(if sides == 20 { 20 } else { 1 })
        });
        assert!(
            staged.is_ok(),
            "{} ({kind:?}) was listed in {:?} but refused: {:?} step={}",
            offer.label,
            phase(current).unwrap(),
            staged.err(),
            crate::gameplay::refusal_detail::step_or_unknown()
        );
    }
}

fn refuses(current: &Checkpoint, member: MemberId, operation: u8, entry: &str) {
    let before = current.clone();
    assert_eq!(
        stage_with_supplier(
            current,
            &input(current, member, operation, entry, vec![]),
            &mut |_| panic!("a refused step draws nothing")
        ),
        Err(RepositoryError::InvalidCandidate),
        "{entry}"
    );
    let _ = crate::gameplay::refusal_detail::take();
    assert_eq!(current, &before);
}

fn assert_end_card(current: &Checkpoint) {
    assert_eq!(beat_id(current), "the-end");
    assert_eq!(phase(current).unwrap(), rpc::JourneyPhase::Complete);
    assert!(runtime::ended(current.state()).unwrap());
    assert_eq!(kinds(current), vec![K::StartNewAdventure]);
    for entry in [
        "short-rest",
        "resolve-errand",
        "end-turn",
        "choose-harbor-scene",
        // Ended is entered from nowhere: no new fight, no new conversation.
        "defend-courier",
        "ask-courier",
        "escort-courier",
    ] {
        refuses(current, first(), 90, entry);
    }
}

#[test]
fn a_won_fight_resolves_once_closes_and_ends_on_a_card_that_offers_a_new_adventure() {
    live(|| {
        let victory = tactical::tests::current_victory("escort-courier");
        assert!(combat_victory(victory.state()).unwrap());
        assert_eq!(
            runtime::fight_outcome(victory.state()).unwrap(),
            FightOutcome::Won
        );
        // The step is not offered before the inn, and a forged submission is refused.
        assert!(!kinds(&victory).contains(&K::ResolveErrand));
        refuses(&victory, first(), 20, "resolve-errand");
        let inn = stage_with_supplier(&victory, &inn_command(&victory, first(), 21), &mut |_| {
            panic!("inn dice")
        })
        .unwrap();
        assert_eq!(beat_id(&inn), "harbor-inn");
        assert!(kinds(&inn).contains(&K::ResolveErrand));
        // Rest remains a choice even when the played fight left everyone untouched.
        assert!(kinds(&inn).contains(&K::ShortRest));
        let stale = listed(&inn)
            .into_iter()
            .find(|(kind, _, _)| *kind == K::ResolveErrand)
            .unwrap()
            .2;
        let epilogue = ending_step(&inn, 22).unwrap();
        assert_eq!(beat_id(&epilogue), "epilogue");
        assert_eq!(phase(&epilogue).unwrap(), rpc::JourneyPhase::Complete);
        // Offered once: the epilogue offers only its own close, and the old id is stale.
        assert_eq!(kinds(&epilogue), vec![K::ResolveErrand]);
        assert!(!runtime::matches_id(&epilogue, K::ResolveErrand, &stale).unwrap());
        assert!(!runtime::ended(epilogue.state()).unwrap());
        let epilogue = persisted(&epilogue);
        let end = ending_step(&epilogue, 23).unwrap();
        assert_end_card(&end);
        assert_end_card(&persisted(&end));
        let outcome = crate::gameplay::adventure::campaign_completion::terminal_outcome(&end)
            .unwrap()
            .unwrap();
        assert_eq!(
            outcome.result,
            crate::gameplay::adventure::campaign::ChapterResult::Victory
        );
    });
}

#[test]
fn a_won_ending_pays_the_advertised_gold_once_to_each_hero_and_survives_reload() {
    live(|| {
        let active = pack::active().unwrap();
        let mut content = active.pack.clone();
        content.reward = Some(pack::ErrandReward {
            gold_per_hero: 14,
            favour: None,
        });
        let loaded = pack::load_from_bytes(
            &serde_json::to_vec(&content).unwrap(),
            &serde_json::to_vec(&active.tuning).unwrap(),
        )
        .unwrap();
        pack::with_test_pack(loaded, || {
            let victory = tactical::tests::current_victory("escort-courier");
            let before = victory
                .state()
                .characters
                .iter()
                .map(|character| {
                    (
                        character.entity,
                        living_scene::balance_cp(victory.state(), character.entity).unwrap(),
                    )
                })
                .collect::<Vec<_>>();
            let inn =
                stage_with_supplier(&victory, &inn_command(&victory, first(), 41), &mut |_| {
                    panic!("inn cannot roll")
                })
                .unwrap();
            let epilogue = ending_step(&inn, 42).unwrap();
            assert!(!rewards::paid(epilogue.state()));
            let ended = ending_step(&epilogue, 43).unwrap();
            assert!(rewards::paid(ended.state()));
            for (who, coin) in before {
                assert_eq!(
                    living_scene::balance_cp(ended.state(), who).unwrap(),
                    coin + 1400
                );
                assert!(
                    character_sheet(ended.state(), who)
                        .unwrap()
                        .facts
                        .iter()
                        .any(|fact| fact.label == "Coin")
                );
            }
            let restored = persisted(&ended);
            let mut same_state = restored.state().clone();
            assert_eq!(rewards::pay(&mut same_state).unwrap(), None);
            assert_eq!(&same_state, restored.state());
        });
    });
}

#[test]
fn a_short_rest_at_the_inn_still_comes_first_and_the_resolution_stays_offered() {
    live(|| {
        // A hero with a spent Second Wind use, so the inn rest regains it.
        let victory = tactical::tests::current_victory_with_spent_second_wind("ask-courier");
        let inn = stage_with_supplier(&victory, &inn_command(&victory, first(), 21), &mut |_| {
            panic!("inn dice")
        })
        .unwrap();
        let rested = stage_with_supplier(
            &inn,
            &input(&inn, first(), 22, "short-rest", vec![]),
            &mut |_| panic!("rest dice"),
        )
        .unwrap();
        assert_eq!(beat_id(&rested), "harbor-inn");
        let epilogue = ending_step(&rested, 23).unwrap();
        assert_eq!(beat_id(&epilogue), "epilogue");
        assert_end_card(&ending_step(&persisted(&epilogue), 24).unwrap());
    });
}

#[test]
fn three_defeated_giant_rats_show_75_xp_only_at_the_won_end_card_after_reload() {
    live(|| {
        let active = pack::active().unwrap();
        let mut tuning = serde_json::to_value(&active.tuning).unwrap();
        tuning["tactical"] =
            serde_json::json!({"enemies":[{"srd_id":"giant-rat","count":3,"ammunition":0}]});
        let loaded = pack::load_from_bytes(
            &serde_json::to_vec(&active.pack).unwrap(),
            &serde_json::to_vec(&tuning).unwrap(),
        )
        .unwrap();
        pack::with_test_pack(loaded, || {
            let victory = tactical::tests::current_victory("escort-courier");
            let inn =
                stage_with_supplier(&victory, &inn_command(&victory, first(), 41), &mut |_| {
                    panic!("inn cannot roll")
                })
                .unwrap();
            let epilogue = ending_step(&inn, 42).unwrap();
            let ended = ending_step(&epilogue, 43).unwrap();
            for role in [
                df_persistence::local_demo_scope::LocalDemoRole::Display,
                df_persistence::local_demo_scope::LocalDemoRole::Player,
            ] {
                let narration =
                    |current: &Checkpoint| match wire::journey_view(current, role, first())
                        .unwrap()
                        .audience
                        .unwrap()
                    {
                        rpc::view_message::Audience::Display(view) => view.narration,
                        rpc::view_message::Audience::Player(view) => view.narration,
                    };
                assert!(!narration(&epilogue).contains("75 XP"));
                let expected = narration(&ended);
                assert!(expected.contains("The party earns 75 XP."));
                assert_eq!(narration(&persisted(&ended)), expected);
            }
        });
    });
}

/// The machine's rest rule (OBS-48): one rest per aftermath while a hero
/// has at least 1 Hit Point, and a stored game that resumes
/// after the rest loads and plays on to the end card.
#[test]
fn a_rest_is_once_per_aftermath_and_a_resumed_rested_game_plays_on() {
    live(|| {
        let victory = tactical::tests::current_victory_with_spent_second_wind("escort-courier");
        assert!(session_machine::rest_accepted(&victory).unwrap());
        assert!(kinds(&victory).contains(&K::ShortRest));
        let rested = stage_with_supplier(
            &victory,
            &input(&victory, first(), 30, "short-rest", vec![]),
            &mut |_| panic!("rest dice"),
        )
        .unwrap();
        // Resumed from stored bytes, the rested aftermath offers no second
        // rest and refuses one, but the inn and the ending still play.
        let resumed = persisted(&rested);
        assert!(!session_machine::rest_accepted(&resumed).unwrap());
        assert!(!kinds(&resumed).contains(&K::ShortRest));
        refuses(&resumed, first(), 31, "short-rest");
        let inn = stage_with_supplier(&resumed, &inn_command(&resumed, first(), 32), &mut |_| {
            panic!("inn dice")
        })
        .unwrap();
        assert!(!kinds(&inn).contains(&K::ShortRest));
        assert_end_card(&ending_step(&ending_step(&inn, 33).unwrap(), 34).unwrap());
    });
}

/// RUL-26: an unhurt party can spend an hour resting without spending a die.
#[test]
fn a_healthy_party_can_choose_a_short_rest_without_spending_hit_point_dice() {
    live(|| {
        let victory = tactical::tests::current_victory("escort-courier");
        assert!(session_machine::rest_accepted(&victory).unwrap());
        assert!(kinds(&victory).contains(&K::ShortRest));
        let rested = stage_with_supplier(
            &victory,
            &input(&victory, first(), 30, "short-rest", vec![]),
            &mut |_| panic!("an unhurt party rolls no Hit Point Dice"),
        )
        .unwrap();
        assert!(rested.state().logical_time.ticks > victory.state().logical_time.ticks);
        for hero in &victory.state().characters {
            for resource in ["hit-points", "hit-point-dice"] {
                assert_eq!(
                    value(rested.state(), hero.entity, resource).unwrap(),
                    value(victory.state(), hero.entity, resource).unwrap(),
                );
            }
        }
        let resumed = persisted(&rested);
        assert!(!kinds(&resumed).contains(&K::ShortRest));
    });
}

fn lost_fight() -> Checkpoint {
    tactical::tests::current_defeat("escort-courier")
}

#[test]
fn a_total_party_defeat_waits_for_the_death_saves_then_ends_without_reviving_anyone() {
    let _process_state = tactical::development_tests::process_state_test_scope();
    live(|| {
        let mut current = lost_fight();
        assert!(!combat_victory(current.state()).unwrap());
        assert!(tactical::dying(current.state()).unwrap());
        // While heroes make death saves only the six-second advance is offered, and a forged
        // ending step is refused.
        assert_eq!(kinds(&current), vec![K::EndTurn]);
        refuses(&current, first(), 30, "resolve-errand");
        for operation in 31..60 {
            if !tactical::dying(current.state()).unwrap() {
                break;
            }
            let member = [first(), MemberId::from_bytes(&MEMBERS[1]).unwrap()]
                .into_iter()
                .find(|member| {
                    runtime::view_offers(&current, *member).is_ok_and(|offers| !offers.is_empty())
                })
                .expect("a living hero can pass time while another hero is dying");
            let available = runtime::view_offers(&current, member).unwrap();
            let offer_id = available[0].2.clone();
            let request = request(&current, operation, K::EndTurn, offer_id);
            let input = wire::journey_input(&request, member, &current).unwrap();
            let staged = stage_with_supplier(&current, &input, &mut |_| Ok(1));
            assert!(
                staged.is_ok(),
                "death-save offer at operation {operation} was refused: {:?} step={} member={member:?} offers={available:?} beat={} phase={:?} state={:?}",
                staged.as_ref().err(),
                crate::gameplay::refusal_detail::step_or_unknown(),
                beat_id(&current),
                phase(&current).unwrap(),
                session_machine::SessionState::of(&current).unwrap()
            );
            current = staged.unwrap();
        }
        assert!(!tactical::dying(current.state()).unwrap());
        assert_eq!(
            runtime::fight_outcome(current.state()).unwrap(),
            FightOutcome::LostAll
        );
        // The defeat is drawn as a story scene during its saves; when the last hero dies it
        // advances directly to the authored terminal beat.
        let story = |current: &Checkpoint| {
            let Some(rpc::view_message::Audience::Player(view)) = wire::journey_view(
                current,
                df_persistence::local_demo_scope::LocalDemoRole::Player,
                first(),
            )
            .unwrap()
            .audience
            else {
                panic!("player audience");
            };
            assert!(view.narration.contains("Two bandits overwhelmed the party"));
            assert!(view.narration.contains("fell."));
            assert!(
                view.narration
                    .starts_with("Two bandits overwhelmed the party. "),
                "{}",
                view.narration
            );
            (
                view.scene.unwrap().destination == rpc::HarborDestination::HarborInn as i32,
                view.journey.unwrap().combat.is_none(),
            )
        };
        assert_eq!(story(&lost_fight()), (false, false));
        // Nobody is revived, and with no living hero the defeat goes straight to the end card.
        assert!(!kinds(&current).contains(&K::ShortRest));
        assert_eq!(beat_id(&current), "the-end");
        assert_end_card(&current);
        let end = persisted(&current);
        assert_end_card(&end);
        assert!(
            tactical::read(end.state())
                .unwrap()
                .combatants()
                .iter()
                .filter(|actor| actor.side() == df_rules::tactical::Side::Heroes)
                .all(|actor| actor.health().dead)
        );
        let outcome = crate::gameplay::adventure::campaign_completion::terminal_outcome(&end)
            .unwrap()
            .unwrap();
        assert_eq!(
            outcome.result,
            crate::gameplay::adventure::campaign::ChapterResult::Defeat
        );
    });
}

#[test]
fn a_dead_hero_is_not_offered_an_aftermath_turn() {
    live(|| {
        let defeat = lost_fight();
        let mut state = defeat.state().clone();
        let dead = state.characters[0].entity;
        let dead_member = super::participants(&defeat)
            .into_iter()
            .find(|link| link.character == Some(dead))
            .unwrap()
            .member;
        let survivor_member = super::participants(&defeat)
            .into_iter()
            .find(|link| link.character != Some(dead))
            .unwrap()
            .member;
        let saved = state.encounters[0].tactical.as_mut().unwrap();
        let actor = saved.actors.iter().find(|(_, who)| *who == dead).unwrap().0;
        let mut battle: serde_json::Value = serde_json::from_slice(&saved.state).unwrap();
        let combatant = battle["actors"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|combatant| combatant["spec"]["id"].as_u64() == Some(u64::from(actor)))
            .unwrap();
        combatant["health"]["hit_points"] = serde_json::json!(0);
        combatant["health"]["unconscious"] = serde_json::json!(true);
        combatant["health"]["prone"] = serde_json::json!(true);
        combatant["health"]["dead"] = serde_json::json!(true);
        combatant["health"]["stable"] = serde_json::json!(false);
        combatant["health"]["death_failures"] = serde_json::json!(3);
        saved.state = serde_json::to_vec(&battle).unwrap();
        for resource in &mut state.resources {
            if resource.owner == dead {
                match resource.resource.as_str() {
                    "hit-points" => resource.value = 0,
                    "unconscious" | "prone" => resource.value = 1,
                    _ => {}
                }
            }
        }
        let current = model::checkpoint(defeat.basis(), state).unwrap();
        assert_eq!(
            runtime::view_offers(&current, dead_member).unwrap(),
            Vec::new()
        );
        assert_eq!(
            runtime::view_offers(&current, survivor_member)
                .unwrap()
                .iter()
                .map(|(kind, _, _)| *kind)
                .collect::<Vec<_>>(),
            vec![K::EndTurn]
        );
        refuses(&current, dead_member, 151, "end-turn");
    });
}

#[test]
fn a_defeat_with_survivors_may_rest_or_end_and_both_reach_the_card() {
    live(|| {
        let defeat = lost_fight();
        let request = request(&defeat, 120, K::EndTurn, listed(&defeat).remove(0).2);
        let game_input = wire::journey_input(&request, first(), &defeat).unwrap();
        let revived = stage_with_supplier(&defeat, &game_input, &mut |_| Ok(20)).unwrap();
        assert!(!tactical::dying(revived.state()).unwrap());
        assert_eq!(
            runtime::fight_outcome(revived.state()).unwrap(),
            FightOutcome::Lost
        );
        assert!(kinds(&revived).contains(&K::ShortRest));
        assert!(kinds(&revived).contains(&K::ResolveErrand));
        // The ending is open before and after the rest.
        let epilogue = ending_step(&revived, 121).unwrap();
        assert_end_card(&ending_step(&persisted(&epilogue), 122).unwrap());
        let rest_started = stage_with_supplier(
            &revived,
            &input_for(&revived, 123, "short-rest"),
            &mut |_| panic!("starting a rest does not roll"),
        )
        .unwrap();
        assert!(kinds(&rest_started).contains(&K::ShortRest));
        assert!(super::short_rest_started(rest_started.state()));
        assert!(!super::short_rest_completed(rest_started.state()));
        assert!(super::pending_hit_point_die_choices(rest_started.state()).unwrap());
        let _previous_refusal = crate::gameplay::refusal_detail::take();
        let rest_result = stage_with_supplier(
            &rest_started,
            &input(
                &rest_started,
                first(),
                124,
                "short-rest",
                vec![(
                    model::label("hit-point-die").unwrap(),
                    model::label("spend").unwrap(),
                )],
            ),
            &mut |sides| Ok(sides),
        );
        assert!(
            rest_result.is_ok(),
            "an eligible survivor's offered Hit Point Die choice was refused: {:?} step={}",
            rest_result.as_ref().err(),
            crate::gameplay::refusal_detail::step_or_unknown()
        );
        let rested = rest_result.unwrap();
        assert!(kinds(&rested).contains(&K::ResolveErrand));
        let epilogue = ending_step(&persisted(&rested), 125).unwrap();
        assert_end_card(&ending_step(&epilogue, 126).unwrap());
    });
}

fn input_for(current: &Checkpoint, operation: u8, entry: &str) -> GameInput {
    input(current, first(), operation, entry, vec![])
}

/// The sub-machines are entered and left only through the machine: a
/// conversation cannot be entered from a fight, and a fight cannot be
/// re-entered or fought on from its aftermath.
#[test]
fn a_conversation_cannot_be_entered_from_a_fight_and_a_fight_cannot_be_reentered() {
    live(|| {
        let combat = tactical::tests::current_combat("escort-courier");
        assert_eq!(
            session_machine::SessionState::of(&combat).unwrap(),
            session_machine::SessionState::Fight
        );
        refuses(&combat, first(), 40, "ask-courier");
        refuses(&combat, first(), 41, "escort-courier");
        refuses(&combat, first(), 42, "defend-courier");
        let victory = tactical::tests::current_victory("escort-courier");
        assert_eq!(
            session_machine::SessionState::of(&victory).unwrap(),
            session_machine::SessionState::Aftermath
        );
        refuses(&victory, first(), 43, "defend-courier");
        refuses(&victory, first(), 44, "greatsword-attack");
        refuses(&victory, first(), 45, "ask-courier");
    });
}

/// The offer/machine agreement, over the checkpoints a played game actually
/// reaches: every offered kind (handler, authored, tactical and view
/// projection alike) is an event the machine accepts in that checkpoint's
/// state, and every event the machine accepts is offered somewhere on the
/// path or carries a written internal reason in the table.
#[test]
fn every_listed_player_offer_is_accepted_when_taken_across_the_played_path() {
    let _process_state = tactical::development_tests::process_state_test_scope();
    use session_machine::{Event, SessionState};
    live(|| {
        let second = MemberId::from_bytes(&MEMBERS[1]).unwrap();
        let prepared = super::tests::prepared_story();
        let opening = super::tests::opening_story();
        let dialogue = stage_with_supplier(
            &opening,
            &input(&opening, first(), 6, "escort-courier", vec![]),
            &mut |_| panic!("dialogue dice"),
        )
        .unwrap();
        let combat = tactical::tests::current_combat("escort-courier");
        let victory = tactical::tests::current_victory_with_spent_second_wind("escort-courier");
        let hurt = heroes_at_hit_points(&victory, 9);
        let rest_started = stage_with_supplier(
            &hurt,
            &input(&hurt, first(), 20, "short-rest", vec![]),
            &mut |_| panic!("starting a rest does not roll"),
        )
        .unwrap();
        let rest_kept = stage_with_supplier(
            &rest_started,
            &input(
                &rest_started,
                first(),
                24,
                "short-rest",
                vec![(
                    model::label("hit-point-die").unwrap(),
                    model::label("keep").unwrap(),
                )],
            ),
            &mut |_| panic!("keeping a die does not roll"),
        )
        .unwrap();
        let rest_completed = stage_with_supplier(
            &rest_kept,
            &input(
                &rest_kept,
                second,
                25,
                "short-rest",
                vec![(
                    model::label("hit-point-die").unwrap(),
                    model::label("spend").unwrap(),
                )],
            ),
            &mut |_| Ok(10),
        )
        .unwrap();
        let defeat = tactical::tests::current_defeat("escort-courier");
        let survivor_defeat = heroes_at_hit_points(
            &stage_with_supplier(
                &defeat,
                &input(&defeat, first(), 26, "end-turn", vec![]),
                &mut |_| Ok(20),
            )
            .unwrap(),
            5,
        );
        let survivor_rest_started = stage_with_supplier(
            &survivor_defeat,
            &input(&survivor_defeat, first(), 27, "short-rest", vec![]),
            &mut |_| panic!("starting a rest does not roll"),
        )
        .unwrap();
        let inn = stage_with_supplier(&victory, &inn_command(&victory, first(), 21), &mut |_| {
            panic!("inn dice")
        })
        .unwrap();
        let epilogue = ending_step(&inn, 22).unwrap();
        let end = ending_step(&epilogue, 23).unwrap();
        let mut uncreated = initial().unwrap();
        for operation in [1, 2] {
            uncreated = stage_join(
                &uncreated,
                &input(
                    &uncreated,
                    bootstrap_member().unwrap(),
                    operation,
                    "join-room",
                    vec![],
                ),
            )
            .unwrap();
        }
        let path = [
            uncreated,
            prepared,
            opening,
            dialogue,
            combat,
            victory,
            defeat,
            inn,
            epilogue,
            end,
            hurt,
            rest_started,
            rest_kept,
            rest_completed,
            survivor_defeat,
            survivor_rest_started,
        ];
        let mut covered: Vec<(SessionState, Event)> = Vec::new();
        let mut states_seen: Vec<SessionState> = Vec::new();
        for current in &path {
            let state = SessionState::of(current).unwrap();
            if !states_seen.contains(&state) {
                states_seen.push(state);
            }
            for member in [first(), second] {
                assert_player_offers_are_accepted(current, member);
                let mut offered_kinds: Vec<K> = Vec::new();
                if let Ok(offers) = handler_offered(current, member) {
                    offered_kinds.extend(offers.into_iter().map(|(kind, _)| kind));
                }
                if let Ok(offers) = authored_offered(current, member) {
                    offered_kinds.extend(offers.into_iter().map(|(kind, _)| kind));
                }
                if let Ok(offers) = offered(current, member) {
                    offered_kinds.extend(offers.into_iter().map(|(kind, _)| kind));
                }
                if let Ok(offers) = runtime::view_offers(current, member) {
                    offered_kinds.extend(offers.into_iter().map(|(kind, _, _)| kind));
                }
                for kind in offered_kinds {
                    assert!(
                        session_machine::kind_accepted(state, kind),
                        "{kind:?} offered in {state:?} but the machine refuses it there"
                    );
                    for event in session_machine::events_for_kind(kind) {
                        covered.push((state, *event));
                    }
                }
            }
        }
        assert_eq!(states_seen.len(), 6, "the path visits every state");
        for state in states_seen {
            for event in session_machine::accepted_events(state) {
                assert!(
                    covered.contains(&(state, event))
                        || session_machine::internal_reason(state, event).is_some(),
                    "{event:?} accepted in {state:?} is never offered and has no internal reason"
                );
            }
        }
    });
}

#[test]
fn outcome_classes_follow_who_stands_and_who_is_dead() {
    // Victory with every hero standing, or with one down; defeat with a survivor, or none.
    let class = FightOutcome::classify;
    assert_eq!(class(true, false, true), FightOutcome::Won);
    assert_eq!(class(true, true, true), FightOutcome::WonCostly);
    assert_eq!(class(false, true, true), FightOutcome::Lost);
    assert_eq!(class(false, true, false), FightOutcome::LostAll);
    let slugs = FightOutcome::ALL.map(FightOutcome::slug);
    assert_eq!(slugs, ["won", "won-costly", "lost", "lost-all"]);
}
