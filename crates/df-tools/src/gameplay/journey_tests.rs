use super::*;
use crate::gameplay::{
    adventure::{RulesMode, record, shop},
    beat_script_runtime, beat_script_variety_tests as native, place, place_creation,
    startup_configuration::ContentRating,
};
use df_rules::RulesCommandHandler;

pub(in crate::gameplay) fn with_created_place(
    run: impl FnOnce(Checkpoint, Checkpoint, pack::LoadedPack),
) {
    let generated = shop::generate(42, RulesMode::Standard, ContentRating::default()).unwrap();
    let (loaded, _) = record::reload(&generated.documents(true).unwrap()).unwrap();
    pack::with_test_pack(loaded.clone(), || {
        let opening = native::act(&native::prepared(), native::first(), 5, "begin-story");
        let scene = beat_script_runtime::scene(opening.state()).unwrap();
        let setting = scene.setting.unwrap();
        let mut request = place_creation::tests::request();
        request.origin_id = setting.place_id.clone();
        request.origin_name = setting.name;
        let model = setting.model.unwrap();
        request.time_of_day = model.time_of_day;
        request.weather = model.weather;
        let created = place_creation::build(
            &request,
            &place_creation::tests::place_slot().to_string(),
            &place_creation::tests::people_slot().to_string(),
            &place_creation::tests::things_slot().to_string(),
        )
        .unwrap();
        let street = place::plan(
            &opening,
            "Brynn",
            OperationId::from_bytes(&[40; 16]).unwrap(),
            &setting.place_id,
            place::JOURNEY_EXIT,
            &created.place_id,
            Some(&created),
        )
        .unwrap_or_else(|error| match error {
            place::PlaceFailure::Refused(reason) => panic!("place refused: {reason:?}"),
            place::PlaceFailure::State(error) => panic!("place state: {error:?}"),
        })
        .candidate;
        run(opening, street, loaded);
    });
}

fn assault(current: &Checkpoint) -> GameInput {
    wire::journey_input(
        &rpc::SubmitActionRequest {
            session_id: Some(rpc::SessionId {
                value: Some(current.basis().session.as_bytes().to_vec()),
            }),
            run_id: Some(rpc::RunId {
                value: Some(current.basis().run.as_bytes().to_vec()),
            }),
            observed_revision: Some(wire::revision(current.basis().revision)),
            operation_id: Some(rpc::OperationId {
                value: Some(vec![41; 16]),
            }),
            offer_id: scene_fight::offer_id(current),
            action_kind: rpc::GameplayActionKind::SceneAssault as i32,
            action_choices: vec![rpc::JourneyChoice {
                group_id: scene_fight::TARGET_GROUP.into(),
                option_id: "market-street-wenna".into(),
            }],
            ..Default::default()
        },
        native::first(),
        current,
    )
    .unwrap()
}

pub(in crate::gameplay) fn fight(current: &Checkpoint) -> Checkpoint {
    let mut initiative = [20, 19, 1].into_iter();
    stage_with_supplier(current, &assault(current), &mut |sides| {
        Ok(if sides == 20 {
            initiative.next().unwrap_or(1)
        } else {
            1
        })
    })
    .unwrap()
}

fn continue_fight(current: &Checkpoint) -> Checkpoint {
    let battle = tactical::read(current.state()).unwrap();
    let actor = battle.active_actor().unwrap();
    let hero = current.state().characters[0].entity;
    assert_eq!(actor, tactical::actor_id(current.state(), hero).unwrap());
    let input = wire::journey_input(
        &rpc::SubmitActionRequest {
            session_id: Some(rpc::SessionId {
                value: Some(current.basis().session.as_bytes().to_vec()),
            }),
            run_id: Some(rpc::RunId {
                value: Some(current.basis().run.as_bytes().to_vec()),
            }),
            observed_revision: Some(wire::revision(current.basis().revision)),
            operation_id: Some(rpc::OperationId {
                value: Some(vec![42; 16]),
            }),
            offer_id: offer_id(current, rpc::GameplayActionKind::EndTurn),
            action_kind: rpc::GameplayActionKind::EndTurn as i32,
            ..Default::default()
        },
        native::first(),
        current,
    )
    .unwrap();
    let continued = stage_with_supplier(current, &input, &mut |_| Ok(1)).unwrap();
    assert_ne!(
        tactical::read(continued.state()).unwrap().active_actor(),
        Some(actor)
    );
    assert!(continued.basis().revision > current.basis().revision);
    continued
}

#[test]
fn c2_created_place_fight_stages_replays_and_continues_with_canonical_bounds() {
    with_created_place(|_, street, _| {
        let committed = fight(&street);
        assert_eq!(phase(&committed).unwrap(), rpc::JourneyPhase::Combat);
        let replayed = JourneyHandler {
            pins: committed.pins().clone(),
        }
        .stage(
            df_rules::RulesCommandInput {
                command: &assault(&street),
                supplied_draws: &committed.state().draws,
            },
            &street,
        )
        .unwrap();
        assert_eq!(replayed, committed);
        continue_fight(&committed);
    });
}

#[test]
fn c2_created_place_fight_native_codec_recovery_needs_no_supplied_inventory() {
    with_created_place(|_, street, _| {
        let committed = fight(&street);
        // Uses model::recovery(), exactly as native startup does; no hand-added constraints.
        let recovered = native::persisted(&committed);
        assert_eq!(recovered, committed);
        assert_eq!(continue_fight(&recovered), continue_fight(&committed));
    });
}

/// The authored rows, except that a party hero's hit-point maximum is the bound its recorded creation
/// choices state (SRD 5.2.1 pp. 47, 84, 87), not the party-wide authored row, which is the largest
/// figure any supported build could reach. A party entity's rows exist only with its character, so the
/// authored rows of party entities without one are absent from the admitted rows.
fn assert_sheet_bounds(
    state: &GameState,
    admitted: &[ResourceConstraint],
    authored: &[ResourceConstraint],
) {
    let authored = authored
        .iter()
        .filter(|row| {
            !ENTITIES.iter().any(|bytes| bytes == row.owner.as_bytes())
                || state
                    .characters
                    .iter()
                    .any(|character| character.entity == row.owner)
        })
        .collect::<Vec<_>>();
    // Created places append their own rows after the authored ones.
    assert!(admitted.len() >= authored.len());
    for (admitted, authored) in admitted.iter().zip(authored) {
        let party_hit_points = authored.resource.as_str() == "hit-points"
            && ENTITIES
                .iter()
                .any(|bytes| bytes == authored.owner.as_bytes())
            && state
                .characters
                .iter()
                .any(|character| character.entity == authored.owner);
        if party_hit_points {
            assert_eq!(
                admitted.maximum,
                i64::from(maximum_hit_points(state, authored.owner).unwrap())
            );
            assert_eq!(admitted.minimum, authored.minimum);
        } else {
            assert_eq!(admitted, authored);
        }
    }
}

#[test]
fn c2_created_inventory_preserves_authored_bounds_and_rejects_stored_maxima() {
    with_created_place(|opening, street, _| {
        let authored = resources().unwrap();
        let inventory = resources_for(opening.state()).unwrap();
        assert_sheet_bounds(opening.state(), &inventory, &authored);
        let inventory = resources_for(street.state()).unwrap();
        assert_sheet_bounds(street.state(), &inventory, &authored);
        let committed = fight(&street);
        let mut state = committed.state().clone();
        let target = scene_fight::person_entity("market-street-wenna").unwrap();
        let hp = state
            .resources
            .iter_mut()
            .find(|resource| resource.owner == target && resource.resource.as_str() == "hit-points")
            .unwrap();
        hp.maximum = 999;
        hp.value = 999;
        assert!(model::checkpoint(committed.basis(), state).is_err());
    });
}

#[test]
fn c2_orphan_party_rows_at_the_party_wide_hit_point_figure_are_refused() {
    with_created_place(|opening, _, _| {
        let second = MemberId::from_bytes(&MEMBERS[1]).unwrap();
        let orphan = opening
            .state()
            .characters
            .iter()
            .find(|character| character.owner == second)
            .unwrap()
            .entity;
        let mut orphaned = opening.state().clone();
        orphaned
            .characters
            .retain(|character| character.owner != second);
        let party_wide_hit_points = resources()
            .unwrap()
            .into_iter()
            .find(|constraint| {
                constraint.owner == orphan && constraint.resource.as_str() == "hit-points"
            })
            .unwrap()
            .maximum;
        let hit_points = orphaned
            .resources
            .iter_mut()
            .find(|resource| resource.owner == orphan && resource.resource.as_str() == "hit-points")
            .unwrap();
        hit_points.maximum = party_wide_hit_points;
        hit_points.value = party_wide_hit_points;
        assert!(model::checkpoint(opening.basis(), orphaned).is_err());
    });
}
