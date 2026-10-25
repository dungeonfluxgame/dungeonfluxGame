#[path = "support/candidate_fixture.rs"]
mod fixture;
use df_intent::targets::*;
use df_model::affordance::*;
use df_model::checkpoint::*;
use fixture::*;

fn bounds() -> TargetResolutionLimits {
    TargetResolutionLimits {
        input_records: 128,
        candidates: 16,
        output_bytes: 16384,
    }
}
fn current() -> Checkpoint {
    let mut supplied = state();
    supplied.entities.push(WorldEntity {
        id: entity(5),
        definition: content(),
        location: None,
        position: Some(Position { x: 0, y: 0, z: 0 }),
        identity_revision: label("fixture-entity-1"),
    });
    checkpoint(supplied).unwrap()
}
fn set<'a>(current: &'a Checkpoint, ids: &[u8]) -> AffordanceSet<'a> {
    AffordanceSet {
        basis: current.basis(),
        pins: current.pins(),
        actor: entity(4),
        action: content(),
        matches: ids
            .iter()
            .map(|id| Affordance {
                actor: entity(4),
                action: content(),
                source: rule(),
                target: Some(
                    current
                        .state()
                        .entities
                        .iter()
                        .find(|target| target.id == entity(*id))
                        .unwrap()
                        .clone(),
                ),
            })
            .collect(),
    }
}
fn resolve<'a>(
    current: &Checkpoint,
    set: &'a AffordanceSet<'_>,
    selection: TargetSelection,
    limits: TargetResolutionLimits,
) -> Result<TargetResolution<'a>, TargetResolutionError> {
    resolve_target(
        current,
        set,
        selection,
        ReferenceInventory {
            rules: &[rule()],
            content: &[content()],
            resources: &[],
            assets: &[],
        },
        limits,
    )
}

#[test]
fn exact_typed_target_and_unique_binding_resolve_without_state_change() {
    let current = current();
    let before = current.clone();
    let set = set(&current, &[5]);
    for selection in [
        TargetSelection::Unspecified,
        TargetSelection::Explicit(entity(5)),
    ] {
        assert_eq!(
            resolve(&current, &set, selection, bounds()),
            Ok(TargetResolution::Selected(&set.matches[0]))
        );
    }
    assert_eq!(current, before);
}

#[test]
fn ambiguous_same_definition_targets_require_clarification_in_stable_order() {
    let current = current();
    for order in [&[4, 5][..], &[5, 4][..]] {
        let set = set(&current, order);
        assert_eq!(
            resolve(&current, &set, TargetSelection::Unspecified, bounds()),
            Ok(TargetResolution::NeedsClarification(vec![
                entity(4),
                entity(5)
            ]))
        );
        let chosen = resolve(
            &current,
            &set,
            TargetSelection::Explicit(entity(5)),
            bounds(),
        )
        .unwrap();
        let TargetResolution::Selected(chosen) = chosen else {
            panic!("explicit confirmation");
        };
        assert_eq!(chosen.target.as_ref().unwrap().id, entity(5));
    }
}

#[test]
fn unsupported_targets_and_missing_mechanics_have_distinct_outcomes() {
    let current = current();
    let set = set(&current, &[5]);
    for id in [4, 99] {
        assert_eq!(
            resolve(
                &current,
                &set,
                TargetSelection::Explicit(entity(id)),
                bounds()
            ),
            Ok(TargetResolution::UnsupportedTarget)
        );
    }
    let empty = AffordanceSet {
        matches: vec![],
        ..set
    };
    assert_eq!(
        resolve(&current, &empty, TargetSelection::Unspecified, bounds()),
        Ok(TargetResolution::NeedsRuling)
    );
}

#[test]
fn changed_source_owner_basis_and_complete_pins_refuse_old_lookup() {
    let current = current();
    for mutation in 0..3 {
        let mut set = set(&current, &[5]);
        let mut other_pins = current.pins().clone();
        match mutation {
            0 => set.basis.revision = set.basis.revision.next_sequence().unwrap(),
            1 => {
                other_pins.rules.handler_digest.0[0] ^= 1;
                set.pins = &other_pins;
            }
            _ => {
                other_pins.build = df_types::BuildIdentity::new(
                    Some("changed"),
                    Some("fixture-native-1"),
                    Some("fixture-wasm-1"),
                    Some("fixture-config-1"),
                    Some("fixture-content-1"),
                )
                .unwrap();
                set.pins = &other_pins;
            }
        }
        assert!(matches!(
            resolve(&current, &set, TargetSelection::Unspecified, bounds()),
            Err(TargetResolutionError::Snapshot(_))
        ));
    }
}

#[test]
fn foreign_source_actor_action_and_changed_target_records_are_not_admitted() {
    let current = current();
    for mutation in 0..5 {
        let mut set = set(&current, &[5]);
        let record = &mut set.matches[0];
        match mutation {
            0 => record.source.clause = label("foreign-clause"),
            1 => record.actor = entity(5),
            2 => record.action.entry = label("fake-action"),
            3 => record.target.as_mut().unwrap().identity_revision = label("changed-target"),
            _ => record.target.as_mut().unwrap().position = Some(Position { x: 2, y: 0, z: 0 }),
        }
        assert_eq!(
            resolve(&current, &set, TargetSelection::Unspecified, bounds()),
            Err(TargetResolutionError::InvalidAffordance)
        );
    }
}

#[test]
fn duplicate_target_bindings_never_turn_collection_order_into_authority() {
    let current = current();
    let set = set(&current, &[5, 5]);
    assert_eq!(
        resolve(&current, &set, TargetSelection::Unspecified, bounds()),
        Err(TargetResolutionError::AmbiguousSource)
    );
}

#[test]
fn exact_owned_bytes_and_candidate_scan_bounds_remain_explicit() {
    let current = current();
    let set = set(&current, &[4, 5]);
    let retained_bytes = set.retained_bytes().unwrap();
    assert!(
        resolve(
            &current,
            &set,
            TargetSelection::Unspecified,
            TargetResolutionLimits {
                output_bytes: retained_bytes,
                ..bounds()
            }
        )
        .is_ok()
    );
    for limits in [
        TargetResolutionLimits {
            output_bytes: retained_bytes - 1,
            ..bounds()
        },
        TargetResolutionLimits {
            candidates: 1,
            ..bounds()
        },
        TargetResolutionLimits {
            input_records: 1,
            ..bounds()
        },
    ] {
        assert_eq!(
            resolve(&current, &set, TargetSelection::Unspecified, limits),
            Err(TargetResolutionError::Capacity)
        );
    }
}

#[test]
fn lookup_cannot_complete_or_modify_existing_pending_resolution() {
    let mut supplied = state();
    supplied.facts.push(fact(7, 0));
    supplied.pending.push(pending());
    let current = checkpoint(supplied).unwrap();
    let before = current.clone();
    let set = set(&current, &[4]);
    assert!(matches!(
        resolve(&current, &set, TargetSelection::Unspecified, bounds()),
        Ok(TargetResolution::Selected(_))
    ));
    assert_eq!(current, before);
}
