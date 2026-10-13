//! A fight chapter ends like every other: a resolution, an epilogue, then an end card that
//! accepts nothing. These tests play fights to their end through submitted actions only.
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
    assert_eq!(current, &before);
}

fn assert_end_card(current: &Checkpoint) {
    assert_eq!(beat_id(current), "the-end");
    assert_eq!(phase(current).unwrap(), rpc::JourneyPhase::Complete);
    assert!(runtime::ended(current.state()).unwrap());
    assert!(
        listed(current).is_empty(),
        "the end card offers nothing: {:?}",
        listed(current)
    );
    for entry in [
        "short-rest",
        "resolve-errand",
        "end-turn",
        "choose-harbor-scene",
    ] {
        refuses(current, first(), 90, entry);
    }
}

#[test]
fn a_won_fight_resolves_once_closes_and_ends_on_a_card_that_accepts_nothing() {
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
        // The played fight left both heroes untouched, so nobody has
        // anything to regain and no rest is offered.
        assert!(!kinds(&inn).contains(&K::ShortRest));
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

/// The machine's rest rule (OBS-48): one rest per aftermath, only while a
/// living hero has something to regain, and a stored game that resumes
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

/// With nobody to regain anything, the rest is neither offered nor staged.
#[test]
fn no_rest_is_offered_when_no_living_hero_has_anything_to_regain() {
    live(|| {
        let victory = tactical::tests::current_victory("escort-courier");
        assert!(!session_machine::rest_accepted(&victory).unwrap());
        assert!(!kinds(&victory).contains(&K::ShortRest));
        refuses(&victory, first(), 30, "short-rest");
    });
}

fn lost_fight() -> Checkpoint {
    tactical::tests::current_defeat("escort-courier")
}

#[test]
fn a_total_party_defeat_waits_for_the_death_saves_then_ends_without_reviving_anyone() {
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
            let request = request(
                &current,
                operation,
                K::EndTurn,
                listed(&current).remove(0).2,
            );
            let input = wire::journey_input(&request, first(), &current).unwrap();
            current = stage_with_supplier(&current, &input, &mut |_| Ok(1)).unwrap();
        }
        assert!(!tactical::dying(current.state()).unwrap());
        assert_eq!(
            runtime::fight_outcome(current.state()).unwrap(),
            FightOutcome::LostAll
        );
        // The settled defeat is drawn as a story scene, not as the fight; while a hero is still
        // dying it keeps the fight's own screen.
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
                view.narration.starts_with("Two bandits overwhelmed the party. "),
                "{}",
                view.narration
            );
            (
                view.scene.unwrap().destination == rpc::HarborDestination::HarborInn as i32,
                view.journey.unwrap().combat.is_none(),
            )
        };
        assert_eq!(story(&current), (true, true));
        assert_eq!(story(&lost_fight()), (false, false));
        // Nobody is revived: there is no rest to take, only the ending.
        assert!(!kinds(&current).contains(&K::ShortRest));
        assert_eq!(kinds(&current), vec![K::ResolveErrand]);
        let epilogue = ending_step(&current, 61).unwrap();
        assert_eq!(beat_id(&epilogue), "epilogue");
        let end = ending_step(&persisted(&epilogue), 62).unwrap();
        assert_end_card(&end);
        assert_end_card(&persisted(&end));
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
fn a_defeat_with_survivors_may_rest_or_end_and_both_reach_the_card() {
    live(|| {
        let defeat = lost_fight();
        let request = request(&defeat, 70, K::EndTurn, listed(&defeat).remove(0).2);
        let input = wire::journey_input(&request, first(), &defeat).unwrap();
        let revived = stage_with_supplier(&defeat, &input, &mut |_| Ok(20)).unwrap();
        assert!(!tactical::dying(revived.state()).unwrap());
        assert_eq!(
            runtime::fight_outcome(revived.state()).unwrap(),
            FightOutcome::Lost
        );
        assert!(kinds(&revived).contains(&K::ShortRest));
        assert!(kinds(&revived).contains(&K::ResolveErrand));
        // The ending is open before and after the rest.
        let epilogue = ending_step(&revived, 71).unwrap();
        assert_end_card(&ending_step(&persisted(&epilogue), 72).unwrap());
        let rested = stage_with_supplier(
            &revived,
            &input_for(&revived, 73, "short-rest"),
            &mut |_| panic!("rest dice"),
        )
        .unwrap();
        assert!(kinds(&rested).contains(&K::ResolveErrand));
        let epilogue = ending_step(&persisted(&rested), 74).unwrap();
        assert_end_card(&ending_step(&epilogue, 75).unwrap());
    });
}

fn input_for(current: &Checkpoint, operation: u8, entry: &str) -> GameInput {
    input(current, first(), operation, entry, vec![])
}

/// The offer/machine agreement, over the checkpoints a played game actually
/// reaches: every offered kind (handler, authored, tactical and view
/// projection alike) is an event the machine accepts in that checkpoint's
/// state, and every event the machine accepts is offered somewhere on the
/// path or carries a written internal reason in the table.
#[test]
fn every_offer_on_the_played_path_is_an_event_the_machine_accepts() {
    use session_machine::{Event, SessionState};
    live(|| {
        let second = MemberId::from_bytes(&MEMBERS[1]).unwrap();
        let mut lobby = initial().unwrap();
        for op in [1, 2] {
            lobby = stage_join(
                &lobby,
                &input(&lobby, bootstrap_member().unwrap(), op, "join-room", vec![]),
            )
            .unwrap();
        }
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
        let defeat = tactical::tests::current_defeat("escort-courier");
        let inn = stage_with_supplier(&victory, &inn_command(&victory, first(), 21), &mut |_| {
            panic!("inn dice")
        })
        .unwrap();
        let epilogue = ending_step(&inn, 22).unwrap();
        let end = ending_step(&epilogue, 23).unwrap();
        let path = [
            lobby, prepared, opening, dialogue, combat, victory, defeat, inn, epilogue, end,
        ];
        let mut covered: Vec<(SessionState, Event)> = Vec::new();
        let mut states_seen: Vec<SessionState> = Vec::new();
        for current in &path {
            let state = SessionState::of(current).unwrap();
            if !states_seen.contains(&state) {
                states_seen.push(state);
            }
            for member in [first(), second] {
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
