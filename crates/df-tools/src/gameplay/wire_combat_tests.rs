use super::*;
use crate::gameplay::{
    adventure::{RulesMode, record, shop},
    beat_script_runtime, beat_script_variety_tests as native, fight_foresight, journey, pack,
    place_creation::{self, tests as places},
    startup_configuration::ContentRating,
};
use df_persistence::local_demo_scope::LocalDemoRole;
use std::sync::Arc;

struct StreetArena;

impl fight_foresight::Jev for StreetArena {
    fn forecast<'a>(
        &'a self,
        _state: &'a serde_json::Value,
        _choices: &'a [String],
    ) -> fight_foresight::ChoiceFuture<'a> {
        Box::pin(async { Ok(2) })
    }
}

struct InstalledArena;

impl Drop for InstalledArena {
    fn drop(&mut self) {
        fight_foresight::TEST_DIRECTOR.with(|slot| *slot.borrow_mut() = None);
    }
}

fn with_spilled_fight(run: impl FnOnce(Checkpoint)) {
    let generated = shop::generate(42, RulesMode::Standard, ContentRating::default()).unwrap();
    let (loaded, _) = record::reload(&generated.documents(true).unwrap()).unwrap();
    pack::with_test_pack(loaded, || {
        let current = native::act(&native::prepared(), native::first(), 5, "begin-story");
        let authored = beat_script_runtime::scene(current.state()).unwrap();
        let setting = authored.setting.as_ref().unwrap();
        let model = setting.model.as_ref().unwrap();
        let mut request = places::request();
        request.time_of_day.clone_from(&model.time_of_day);
        request.weather.clone_from(&model.weather);
        request.origin_name.clone_from(&setting.name);
        let street = place_creation::build(
            &request,
            &places::place_slot().to_string(),
            &places::people_slot().to_string(),
            &places::things_slot().to_string(),
        )
        .unwrap();
        let director = Arc::new(fight_foresight::FightForesight::new(Box::new(StreetArena)));
        fight_foresight::TEST_DIRECTOR.with(|slot| *slot.borrow_mut() = Some(director));
        let _installed = InstalledArena;
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(fight_foresight::prepare_opening(&current));
        assert!(fight_foresight::retain_destination(
            &setting.place_id,
            street
        ));
        let request = rpc::SubmitActionRequest {
            session_id: Some(rpc::SessionId {
                value: Some(current.basis().session.as_bytes().to_vec()),
            }),
            run_id: Some(rpc::RunId {
                value: Some(current.basis().run.as_bytes().to_vec()),
            }),
            observed_revision: Some(revision(current.basis().revision)),
            operation_id: Some(rpc::OperationId {
                value: Some(vec![6; 16]),
            }),
            action_kind: rpc::GameplayActionKind::SceneAssault as i32,
            offer_id: journey::scene_fight::offer_id(&current),
            action_choices: vec![rpc::JourneyChoice {
                group_id: "scene-target".to_owned(),
                option_id: "shop-soldier".to_owned(),
            }],
            ..Default::default()
        };
        let input = journey_input(&request, native::first(), &current).unwrap();
        let mut initiative = [20, 19, 1, 1, 1, 1].into_iter();
        let fight = journey::stage_with_supplier(&current, &input, &mut |sides| {
            Ok(if sides == 20 {
                initiative.next().unwrap()
            } else {
                sides.min(3)
            })
        })
        .unwrap();
        assert_eq!(
            journey::scene_fight::world_place(fight.state())
                .unwrap()
                .as_deref(),
            Some("place-market-street")
        );
        run(fight);
    });
}

#[test]
fn combat_after_a_spill_projects_the_committed_arena_title() {
    with_spilled_fight(|fight| {
        // A later forecast cannot rename a fight whose opening has already committed.
        fight_foresight::TEST_DIRECTOR.with(|slot| *slot.borrow_mut() = None);
        let projected = project_journey(&fight, LocalDemoRole::Display, native::first()).unwrap();
        let Some(rpc::view_message::Audience::Display(display)) = projected.view.audience else {
            panic!("display projection");
        };
        assert!(display.journey.unwrap().combat.is_some());
        assert_eq!(display.scene.unwrap().title, "Market Street");
    });
}

#[test]
fn combat_after_a_spill_republishes_the_fighters_identity_for_late_depictions() {
    with_spilled_fight(|fight| {
        for _ in 0..2 {
            let projected =
                project_journey(&fight, LocalDemoRole::Display, native::first()).unwrap();
            let Some(rpc::view_message::Audience::Display(display)) = projected.view.audience
            else {
                panic!("display projection");
            };
            let scene = display.scene.unwrap();
            let combat = display.journey.unwrap().combat.unwrap();
            let jessa = combat
                .participants
                .iter()
                .find(|member| member.name == "Jessa")
                .unwrap();
            let actor = scene
                .cast
                .iter()
                .find(|actor| actor.id == "shop-soldier")
                .unwrap();
            assert_eq!(jessa.portrait, actor.portrait);
            assert!(actor.portrait.contains("shop-soldier"));
            assert!(actor.sprite.contains("shop-soldier"));
            assert_eq!(
                scene
                    .cast
                    .iter()
                    .filter(|actor| actor.id == "shop-soldier")
                    .count(),
                1
            );
            assert!(!scene.cast.iter().any(|actor| actor.id == "patron-merchant"));
        }
    });
}

#[test]
fn combat_accepts_a_ready_participant_depiction_in_the_next_same_revision_view() {
    with_spilled_fight(|fight| {
        let mut shown = project_journey(&fight, LocalDemoRole::Display, native::first())
            .unwrap()
            .view;
        let mut ready = project_journey(&fight, LocalDemoRole::Display, native::first())
            .unwrap()
            .view;
        let path = format!("/gameplay/imagery/{}", "a".repeat(64));
        let Some(rpc::view_message::Audience::Display(display)) = ready.audience.as_mut() else {
            panic!("display projection");
        };
        display
            .scene
            .as_mut()
            .unwrap()
            .cast
            .iter_mut()
            .find(|actor| actor.id == "shop-soldier")
            .unwrap()
            .live_sprite = path.clone();
        assert_eq!(shown.revision, ready.revision);
        assert!(crate::live_art::merge_art(&mut shown, &ready));
        let Some(rpc::view_message::Audience::Display(display)) = shown.audience else {
            panic!("display projection");
        };
        let actor = display
            .scene
            .unwrap()
            .cast
            .into_iter()
            .find(|actor| actor.id == "shop-soldier")
            .unwrap();
        assert_eq!(actor.live_sprite, path);
        assert_eq!(actor.name, "Jessa");
    });
}

#[test]
fn forced_battle_scene_projects_no_generated_world_even_when_ready() {
    let ready = || {
        Some(super::super::splat_world::ReadyWorld {
            ply_url: "/world.ply".to_owned(),
            collider_url: "/collider.glb".to_owned(),
            metric_scale_factor: Some(1.0),
            ground_plane_offset: Some(0.0),
        })
    };
    assert!(project_battle_world(true, None, ready()).is_some());
    for scene in ["scan-riverbed", "merchant-shop"] {
        let mut combat = rpc::CombatView::default();
        combat.battle_world = project_battle_world(true, Some(scene), ready());
        assert!(combat.battle_world.is_none(), "forced {scene}");
    }
    assert!(project_battle_world(false, None, ready()).is_none());
}

#[test]
fn live_display_executes_committed_hit_and_miss_before_battlefield_redraw() {
    use crate::dice_roll::{AttackDisplay, AttackDisplayAction, live_attack};
    use crate::stage_director::{Command, StageDirector, StageState, ViewDiff};
    use df_observe::presentation::{ComponentPhase, PresentationComponent};
    with_spilled_fight(|fight| {
        let member = native::first();
        let kind = rpc::GameplayActionKind::GreatswordAttack;
        let groups = journey::tactical::groups(&fight, member, kind).unwrap();
        let selected = groups.first().unwrap().options.first().unwrap();
        let offer_id = beat_script_runtime::view_offers(&fight, member)
            .unwrap()
            .into_iter()
            .find(|(offered, ..)| *offered == kind)
            .unwrap()
            .2;
        for (face, hit) in [(5, false), (19, true)] {
            let request = rpc::SubmitActionRequest {
                session_id: Some(rpc::SessionId {
                    value: Some(fight.basis().session.as_bytes().to_vec()),
                }),
                run_id: Some(rpc::RunId {
                    value: Some(fight.basis().run.as_bytes().to_vec()),
                }),
                observed_revision: Some(revision(fight.basis().revision)),
                operation_id: Some(rpc::OperationId {
                    value: Some(vec![77; 16]),
                }),
                action_kind: kind as i32,
                offer_id: offer_id.clone(),
                action_choices: vec![rpc::JourneyChoice {
                    group_id: groups[0].group_id.clone(),
                    option_id: selected.option_id.clone(),
                }],
                ..Default::default()
            };
            let input = journey_input(&request, member, &fight).unwrap();
            let committed = journey::stage_with_supplier(&fight, &input, &mut |sides| {
                Ok(if sides == 20 { face } else { sides.min(3) })
            })
            .unwrap();
            let recovered = native::persisted(&committed);
            let projected = project_journey(&recovered, LocalDemoRole::Display, member).unwrap();
            let Some(rpc::view_message::Audience::Display(display)) = projected.view.audience
            else {
                panic!("display projection");
            };
            let combat = display.journey.unwrap().combat.unwrap();
            let outcome = combat
                .outcomes
                .iter()
                .find(|outcome| outcome.actor_name == "Brynn")
                .unwrap();
            assert_eq!(outcome.hit, hit);
            let mut director = StageDirector::new();
            let commands = director.on_view(
                0,
                ViewDiff {
                    roll: Some(outcome.sequence),
                    impact: Some(outcome.sequence),
                    sequence: outcome.sequence,
                    ..ViewDiff::default()
                },
            );
            assert!(
                !commands
                    .iter()
                    .any(|command| matches!(command, Command::Render))
            );
            let sequence = commands
                .into_iter()
                .find_map(|command| match command {
                    Command::Enter {
                        state: StageState::Rolling,
                        detail: Some(sequence),
                        ..
                    } => Some(sequence),
                    _ => None,
                })
                .expect("the live director starts this committed roll before redraw");
            // The renderer still has the previous empty snapshot when this cue enters.
            assert!(live_attack(&rpc::CombatView::default(), sequence).is_none());
            let attack = AttackDisplay::new(live_attack(&combat, sequence).unwrap()).unwrap();
            assert_eq!(u32::from(attack.play.kept), outcome.attack_die);
            assert_eq!(attack.play.other, None, "one ordinary attack die");
            assert!(!attack.attack_card.explanation.contains(" | damage "));
            let steps = attack.steps();
            assert_eq!(steps[0], (0, AttackDisplayAction::OpenDie));
            assert_eq!(steps[1], (1100, AttackDisplayAction::SettleDie));
            assert_eq!(steps[2], (1500, AttackDisplayAction::OpenCard));
            let records: Vec<_> = steps
                .into_iter()
                .flat_map(|(at, action)| {
                    action
                        .records(!attack.damage_faces.is_empty())
                        .into_iter()
                        .map(move |(component, phase, reason)| (at, component, phase, reason))
                })
                .collect();
            assert!(records.contains(&(
                0,
                PresentationComponent::Die,
                ComponentPhase::Opened,
                "attack-rolled"
            )));
            assert!(records.contains(&(
                1100,
                PresentationComponent::Die,
                ComponentPhase::Readable,
                "attack-settled"
            )));
            assert!(records.contains(&(
                1500,
                PresentationComponent::RollCard,
                ComponentPhase::Opened,
                "face-dwell-complete"
            )));
            assert!(records.contains(&(
                1850,
                PresentationComponent::RollCard,
                ComponentPhase::Readable,
                "attack-breakdown"
            )));
            if hit {
                assert!(!attack.damage_faces.is_empty());
                assert!(attack.damage_card.explanation.contains(" | damage "));
                assert!(records.contains(&(
                    1500,
                    PresentationComponent::Dice,
                    ComponentPhase::Opened,
                    "damage-rolled"
                )));
                assert!(records.contains(&(
                    2300,
                    PresentationComponent::Dice,
                    ComponentPhase::Readable,
                    "damage-settled"
                )));
            } else {
                assert!(attack.damage_faces.is_empty());
                assert!(
                    !records
                        .iter()
                        .any(|(_, component, ..)| *component == PresentationComponent::Dice)
                );
            }
            let closing_ms = if hit { 4300 } else { 4000 };
            for component in [PresentationComponent::Die, PresentationComponent::RollCard] {
                assert!(
                    records
                        .iter()
                        .any(|(at, observed, phase, _)| *at == closing_ms
                            && *observed == component
                            && *phase == ComponentPhase::Closing)
                );
                assert!(
                    records
                        .iter()
                        .any(|(at, observed, phase, _)| *at == closing_ms + 320
                            && *observed == component
                            && *phase == ComponentPhase::Hidden)
                );
            }
        }
    });
}
