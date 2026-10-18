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
