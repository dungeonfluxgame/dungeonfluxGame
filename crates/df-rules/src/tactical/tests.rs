use super::*;
use df_model::checkpoint::{ActualDraw, ResolutionId, RuleReference, WindowId};
use df_types::{OperationId, RevisionLabel};

fn source() -> RuleReference {
    RuleReference {
        catalog: RevisionLabel::new(Some("srd-5.2.1")).unwrap(),
        source: RevisionLabel::new(Some("pdf-8974902d")).unwrap(),
        entry: RevisionLabel::new(Some("PLAY-T01")).unwrap(),
        clause: RevisionLabel::new(Some("source-fixture")).unwrap(),
    }
}
fn binding(source: &RuleReference) -> DrawBinding<'_> {
    DrawBinding {
        operation: OperationId::from_bytes(&[1; 16]).unwrap(),
        resolution: ResolutionId::from_bytes(&[2; 16]).unwrap(),
        window: WindowId::from_bytes(&[3; 16]).unwrap(),
        source,
        next_ordinal: 0,
    }
}
fn records(source: &RuleReference, faces: &[(u32, u32)]) -> Vec<ActualDraw> {
    let bound = binding(source);
    faces
        .iter()
        .enumerate()
        .map(|(i, &(sides, value))| ActualDraw {
            operation: bound.operation,
            resolution: bound.resolution,
            window: bound.window,
            source: source.clone(),
            ordinal: i as u32,
            sides,
            value,
        })
        .collect()
}
fn step(battle: &Battle, actor: u32, mut action: Action, faces: &[(u32, u32)]) -> Resolution {
    // Successful source fixtures use only the actor's selected masteries. Refusal
    // fixtures call resolve directly, preserving their deliberately invalid inputs.
    let usage = match &mut action {
        Action::Attack { weapon, .. } => Some(weapon),
        Action::Opportunity { attack } => attack.as_mut(),
        _ => None,
    };
    if let Some(usage) = usage {
        usage.use_mastery &= battle
            .actor(actor)
            .unwrap()
            .spec()
            .masteries
            .contains(&usage.weapon);
    }
    let src = source();
    let dice = records(&src, faces);
    resolve(battle, actor, &action, binding(&src), &dice).unwrap()
}
fn rejects(battle: &Battle, actor: u32, action: Action, error: RuleError) {
    let src = source();
    let original = battle.clone();
    assert_eq!(
        resolve(battle, actor, &action, binding(&src), &[]),
        Err(error)
    );
    assert_eq!(*battle, original);
}
fn map(name: &str) -> BattleMap {
    let bytes: &[u8] = match name {
        "broken-yard" => include_bytes!("../../tests/data/tactical/broken-yard.json"),
        "wolf-trail" => include_bytes!("../../tests/data/tactical/wolf-trail.json"),
        _ => include_bytes!("../../tests/data/tactical/open-court.json"),
    };
    BattleMap::from_json(bytes, name).unwrap()
}
fn corner_map() -> BattleMap {
    BattleMap::from_json(
        br#"{"schema":2,"grid":{"cols":4,"rows":4,"blocked":[[1,2],[2,1]]},"layouts":[{"id":"corner-test","obstacles":[],"difficult":[],"spawns":{"heroes":[[1,1],[3,3]],"enemies":[[0,2]]}}]}"#,
        "corner-test",
    )
    .unwrap()
}
fn melee(id: u32, cell: Cell) -> CombatantSpec {
    CombatantSpec {
        id,
        side: Side::Heroes,
        cell,
        size: Size::Medium,
        kind: CreatureKind::Hero,
        abilities: [17, 13, 15, 10, 12, 8],
        armor_class: 17,
        hit_points: 13,
        speed_ft: 30,
        weapons: vec![
            Weapon::Greatsword,
            Weapon::Flail,
            Weapon::Javelin,
            Weapon::Spear,
            Weapon::Shortbow,
        ],
        masteries: vec![Weapon::Greatsword, Weapon::Flail, Weapon::Javelin],
        arrows: 20,
        bolts: 0,
        archery: false,
        savage_attacker: true,
        second_wind_uses: 2,
        shield: false,
    }
}
fn archer_choices() -> Vec<(&'static str, &'static str)> {
    vec![
        ("species", "dwarf"),
        ("class", "fighter-1"),
        ("background", "soldier"),
        ("array", "dexterous"),
        ("alignment", "neutral-good"),
        ("language-1", "dwarvish"),
        ("language-2", "goblin"),
        ("skill-1", "perception"),
        ("skill-2", "survival"),
        ("gaming-set", "dice"),
        ("equipment", "fighter-b-soldier-b"),
        ("style-masteries", "archery-longbow-scimitar-shortsword"),
        ("allied-ties", "join-order"),
    ]
}
fn archer(id: u32, cell: Cell) -> CombatantSpec {
    validate_archer("Brynn", &archer_choices())
        .unwrap()
        .combatant(id, cell)
}
fn battle(specs: Vec<CombatantSpec>) -> Battle {
    let src = source();
    let mut kinds = Vec::new();
    let mut faces = Vec::new();
    for spec in &specs {
        if spec.kind == CreatureKind::Hero || !kinds.contains(&spec.kind) {
            faces.push((
                20,
                if spec.kind == CreatureKind::Hero {
                    20
                } else {
                    1
                },
            ));
        }
        kinds.push(spec.kind);
    }
    begin(
        map("open-court"),
        specs,
        binding(&src),
        &records(&src, &faces),
    )
    .unwrap()
    .battle
}
fn usage(weapon: Weapon, ranged: bool) -> AttackUse {
    AttackUse {
        weapon,
        ranged,
        hands: if weapon.two_handed() { 2 } else { 1 },
        finesse_dexterity: weapon.finesse(),
        knockout: false,
        savage_attacker: false,
        use_mastery: true,
    }
}
#[test]
fn unselected_hero_and_monster_masteries_are_refused_without_draws() {
    let state = battle(vec![
        melee(1, Cell(1, 1)),
        CombatantSpec::creature(3, "wolf", Cell(2, 1), 0).unwrap(),
    ]);
    rejects(
        &state,
        1,
        strike(3, Weapon::Spear, false),
        RuleError::InvalidWeapon,
    );
    let mut state = state;
    state.active = 1;
    rejects(
        &state,
        3,
        strike(1, Weapon::Bite, false),
        RuleError::InvalidWeapon,
    );
}
fn strike(target: u32, weapon: Weapon, ranged: bool) -> Action {
    Action::Attack {
        target,
        weapon: usage(weapon, ranged),
        spend: AttackSpend::Action,
    }
}
fn next_turn(mut state: Battle, desired: u32) -> Battle {
    for _ in 0..64 {
        let actor = state.active_actor().unwrap();
        state = step(&state, actor, Action::EndTurn, &[]).battle;
        if state.active_actor() == Some(desired) {
            return state;
        }
    }
    panic!("desired turn missing");
}

#[test]
fn map_v2_validates_every_layout_and_explicitly_ignores_rendering_fields() {
    for name in ["open-court", "broken-yard", "wolf-trail"] {
        let loaded = map(name);
        assert_eq!(loaded.id(), name);
        assert_eq!(loaded.hero_spawns().len(), 3);
    }
    let mut data: serde_json::Value =
        serde_json::from_slice(include_bytes!("../../tests/data/tactical/broken-yard.json"))
            .unwrap();
    data["schema"] = serde_json::json!(1);
    assert_eq!(
        BattleMap::from_json(&serde_json::to_vec(&data).unwrap(), "broken-yard"),
        Err(RuleError::MapData)
    );
    data["schema"] = serde_json::json!(2);
    data["layouts"][0]["spawns"]["enemies"][0] = serde_json::json!([7, 3]);
    assert_eq!(
        BattleMap::from_json(&serde_json::to_vec(&data).unwrap(), "broken-yard"),
        Err(RuleError::Spawn)
    );
    data["layouts"][0]["spawns"]["enemies"][0] = serde_json::json!([128, 0]);
    assert_eq!(
        BattleMap::from_json(&serde_json::to_vec(&data).unwrap(), "broken-yard"),
        Err(RuleError::OutOfBounds)
    );
    data["layouts"][0]["spawns"]["enemies"][0] = serde_json::json!([14, 2]);
    data["layouts"][0]["obstacles"][1]["id"] = serde_json::json!("crate");
    assert_eq!(
        BattleMap::from_json(&serde_json::to_vec(&data).unwrap(), "broken-yard"),
        Err(RuleError::DuplicateId)
    );
    let isolated=br#"{"schema":2,"grid":{"cols":3,"rows":3,"blocked":[[1,0],[1,1],[1,2]]},"layouts":[{"id":"closed","obstacles":[],"difficult":[],"spawns":{"heroes":[[0,1]],"enemies":[[2,1]]}}]}"#;
    assert_eq!(
        BattleMap::from_json(isolated, "closed"),
        Err(RuleError::UnreachableSpawn)
    );
    assert_eq!(Cell(65535, 65535).distance(Cell(0, 0)), 327675);
}
#[test]
fn geometry_corners_cover_adjacent_shots_and_intervening_creatures() {
    let yard = map("broken-yard");
    assert_eq!(
        yard.cover_between(Cell(5, 3), Cell(9, 3), &[]),
        Ok(Cover::Half)
    );
    assert_eq!(
        yard.cover_between(Cell(5, 0), Cell(10, 2), &[]),
        Ok(Cover::ThreeQuarters)
    );
    assert_eq!(
        yard.cover_between(Cell(7, 1), Cell(9, 1), &[]),
        Ok(Cover::ThreeQuarters)
    );
    assert_eq!(
        yard.cover_between(Cell(8, 5), Cell(10, 5), &[]),
        Ok(Cover::Total)
    );
    // The ray touches a crate corner but never its interior.
    assert_eq!(
        yard.cover_between(Cell(6, 3), Cell(7, 2), &[]),
        Ok(Cover::None)
    );
    assert_eq!(
        yard.cover_between(Cell(5, 2), Cell(9, 2), &[Cell(7, 2)]),
        Ok(Cover::Half)
    );
    assert_eq!(
        yard.cover_between(Cell(5, 0), Cell(10, 2), &[Cell(7, 1)]),
        Ok(Cover::ThreeQuarters)
    );
    assert!(!yard.step_clear(Cell(6, 3), Cell(7, 2)));
    assert_eq!(Cover::Half.bonus(), 2);
    assert_eq!(Cover::ThreeQuarters.bonus(), 5);
}
#[test]
fn movement_diagonals_difficult_allies_hostiles_split_move_and_shortest_path() {
    let mut state = battle(vec![
        melee(1, Cell(4, 3)),
        melee(2, Cell(5, 3)),
        CombatantSpec::creature(3, "goblin-warrior", Cell(12, 3), 20).unwrap(),
    ]);
    state.map = map("broken-yard");
    assert_eq!(state.validate_path(1, &[Cell(5, 3), Cell(6, 3)]), Ok(15));
    rejects(
        &state,
        1,
        Action::Move(vec![Cell(5, 3)]),
        RuleError::Occupied,
    );
    let state = step(&state, 1, Action::Move(vec![Cell(5, 3), Cell(6, 3)]), &[]).battle;
    assert_eq!(state.actor(1).unwrap().budget().movement_spent_ft, 15);
    let state = step(&state, 1, Action::Dodge, &[]).battle;
    let state = step(&state, 1, Action::Move(vec![Cell(5, 2)]), &[]).battle;
    assert_eq!(state.actor(1).unwrap().budget().movement_spent_ft, 20);
    assert!(state.actor(1).unwrap().budget().action_used);
    assert_eq!(
        state.validate_path(1, &[Cell(9, 2)]),
        Err(RuleError::Blocked)
    );
    let path = state.shortest_path(1, Cell(8, 4), None).unwrap();
    assert!(path.iter().all(|cell| !state.map().is_blocked(*cell)));
    let reachable = state.reachable_cells(1).unwrap();
    for cell in reachable {
        let path = state
            .shortest_path(
                1,
                cell,
                Some(state.actor(1).unwrap().movement_remaining_ft()),
            )
            .unwrap();
        if !path.is_empty() {
            state.validate_path(1, &path).unwrap();
        }
    }
    let mut state = battle(vec![
        melee(1, Cell(1, 1)),
        CombatantSpec::creature(3, "wolf", Cell(2, 1), 0).unwrap(),
        CombatantSpec::creature(4, "wolf", Cell(10, 1), 0).unwrap(),
    ]);
    assert_eq!(
        state.validate_path(1, &[Cell(2, 1), Cell(3, 1)]),
        Err(RuleError::Occupied)
    );
    state.actor_mut(3).unwrap().health.unconscious = true;
    assert_eq!(state.validate_path(1, &[Cell(2, 1), Cell(3, 1)]), Ok(15));
    rejects(
        &state,
        1,
        Action::Move(vec![Cell(2, 1)]),
        RuleError::Occupied,
    );
}
#[test]
fn standing_crawling_dash_and_disengage_use_source_budgets() {
    let state = battle(vec![
        melee(1, Cell(1, 1)),
        CombatantSpec::creature(3, "wolf", Cell(10, 1), 0).unwrap(),
    ]);
    let state = step(&state, 1, Action::DropProne, &[]).battle;
    assert_eq!(state.validate_path(1, &[Cell(2, 2)]), Ok(10));
    let state = step(&state, 1, Action::Stand, &[]).battle;
    assert_eq!(state.actor(1).unwrap().movement_remaining_ft(), 15);
    let state = step(&state, 1, Action::Dash, &[]).battle;
    assert_eq!(state.actor(1).unwrap().movement_remaining_ft(), 45);
    rejects(&state, 1, Action::Dodge, RuleError::ActionSpent);
    let mut state = state;
    state.actor_mut(1).unwrap().spec.speed_ft = 0;
    state.actor_mut(1).unwrap().health.prone = true;
    rejects(&state, 1, Action::Stand, RuleError::InvalidPath);
}
#[test]
fn opportunity_window_is_optional_before_exit_and_reaction_refreshes_at_own_turn() {
    let state = battle(vec![
        melee(1, Cell(2, 2)),
        melee(2, Cell(1, 1)),
        CombatantSpec::creature(3, "wolf", Cell(3, 2), 0).unwrap(),
    ]);
    let pending = step(&state, 1, Action::Move(vec![Cell(1, 2), Cell(0, 2)]), &[]).battle;
    assert_eq!(pending.actor(1).unwrap().cell(), Cell(2, 2));
    assert_eq!(pending.reaction_offers(), &[3]);
    rejects(&pending, 1, Action::Dodge, RuleError::PendingReaction);
    rejects(
        &pending,
        2,
        Action::Opportunity { attack: None },
        RuleError::InvalidReaction,
    );
    let declined = step(&pending, 3, Action::Opportunity { attack: None }, &[]).battle;
    assert_eq!(declined.actor(1).unwrap().cell(), Cell(0, 2));
    assert!(!declined.actor(3).unwrap().budget().reaction_used);
    let reacted = step(
        &pending,
        3,
        Action::Opportunity {
            attack: Some(usage(Weapon::Bite, false)),
        },
        &[(20, 15), (6, 1), (20, 6)],
    );
    assert_eq!(reacted.attacks[0].distance_ft, 5);
    assert!(reacted.battle.actor(3).unwrap().budget().reaction_used);
    assert!(reacted.battle.actor(1).unwrap().health().prone);
    let state = next_turn(reacted.battle, 3);
    assert!(!state.actor(3).unwrap().budget().reaction_used);
    let state = step(&state, 3, Action::EndTurn, &[]).battle;
    assert_eq!(state.round(), 2);
    let state = battle(vec![
        melee(1, Cell(2, 2)),
        CombatantSpec::creature(3, "wolf", Cell(3, 2), 0).unwrap(),
    ]);
    let state = step(&state, 1, Action::Disengage, &[]).battle;
    let state = step(&state, 1, Action::Move(vec![Cell(1, 2)]), &[]).battle;
    assert!(state.reaction_offers().is_empty());
}
#[test]
fn opportunity_knockout_cancels_remaining_path_and_multiple_reactors_are_ordered() {
    let mut state = battle(vec![
        melee(1, Cell(2, 2)),
        CombatantSpec::creature(3, "wolf", Cell(3, 2), 0).unwrap(),
        CombatantSpec::creature(4, "wolf", Cell(3, 3), 0).unwrap(),
    ]);
    state.actor_mut(1).unwrap().health.hit_points = 1;
    let state = step(&state, 1, Action::Move(vec![Cell(1, 1)]), &[]).battle;
    assert_eq!(state.reaction_offers(), &[3, 4]);
    let mut bite = usage(Weapon::Bite, false);
    bite.knockout = true;
    let state = step(
        &state,
        3,
        Action::Opportunity { attack: Some(bite) },
        &[(20, 20), (20, 19), (6, 1), (6, 1), (20, 8)],
    )
    .battle;
    assert_eq!(state.actor(1).unwrap().cell(), Cell(2, 2));
    assert!(state.reaction_offers().is_empty());
    assert_eq!(state.winner(), Some(Side::Enemies));
}
#[test]
fn ranged_boundaries_ammunition_and_immutable_draw_rejections() {
    let state = battle(vec![
        archer(1, Cell(1, 1)),
        CombatantSpec::creature(3, "skeleton", Cell(31, 1), 20).unwrap(),
    ]);
    let result = step(
        &state,
        1,
        strike(3, Weapon::Longbow, true),
        &[(20, 10), (8, 4)],
    );
    assert_eq!(result.attacks[0].distance_ft, 150);
    assert_eq!(result.attacks[0].modifier, 7);
    assert_eq!(result.attacks[0].roll_mode, 0);
    assert_eq!(result.battle.actor(1).unwrap().arrows(), 19);
    let mut long = state.clone();
    long.actor_mut(3).unwrap().spec.cell = Cell(32, 1);
    let result = step(
        &long,
        1,
        strike(3, Weapon::Longbow, true),
        &[(20, 18), (20, 3)],
    );
    assert_eq!(result.attacks[0].roll_mode, -1);
    assert!(
        result.attacks[0]
            .disadvantage
            .contains(&AttackReason::LongRange)
    );
    long.actor_mut(3).unwrap().spec.cell = Cell(121, 1);
    step(
        &long,
        1,
        strike(3, Weapon::Longbow, true),
        &[(20, 1), (20, 2)],
    );
    long.actor_mut(3).unwrap().spec.cell = Cell(122, 1);
    rejects(
        &long,
        1,
        strike(3, Weapon::Longbow, true),
        RuleError::OutOfRange,
    );
    let mut empty = state.clone();
    empty.actor_mut(1).unwrap().spec.arrows = 0;
    rejects(
        &empty,
        1,
        strike(3, Weapon::Longbow, true),
        RuleError::AmmunitionMissing,
    );
    let mut illegal = usage(Weapon::Longbow, true);
    illegal.knockout = true;
    rejects(
        &state,
        1,
        Action::Attack {
            target: 3,
            weapon: illegal,
            spend: AttackSpend::Action,
        },
        RuleError::InvalidTarget,
    );
    let src = source();
    let mut dice = records(&src, &[(20, 10), (8, 4)]);
    dice[1].ordinal = 5;
    assert_eq!(
        resolve(
            &state,
            1,
            &strike(3, Weapon::Longbow, true),
            binding(&src),
            &dice
        ),
        Err(RuleError::StaleDraw)
    );
    assert_eq!(state.actor(1).unwrap().arrows(), 20);
    assert_eq!(
        resolve(
            &state,
            1,
            &strike(3, Weapon::Longbow, true),
            binding(&src),
            &records(&src, &[(20, 10), (8, 0)])
        ),
        Err(RuleError::InvalidDraw)
    );
}
#[test]
fn advantage_disadvantage_presence_cancels_and_close_hostile_must_see_and_be_capable() {
    let mut state = battle(vec![
        archer(1, Cell(1, 1)),
        CombatantSpec::creature(3, "skeleton", Cell(2, 1), 20).unwrap(),
        CombatantSpec::creature(4, "wolf", Cell(10, 1), 0).unwrap(),
    ]);
    state.actor_mut(1).unwrap().health.prone = true;
    state.actor_mut(3).unwrap().health.prone = true;
    let result = step(
        &state,
        1,
        strike(3, Weapon::Longbow, true),
        &[(20, 10), (8, 1)],
    );
    assert_eq!(result.attacks[0].roll_mode, 0);
    assert_eq!(result.attacks[0].faces.len(), 1);
    assert!(
        result.attacks[0]
            .disadvantage
            .contains(&AttackReason::HostileNearby)
    );
    state.actor_mut(1).unwrap().health.prone = false;
    state.actor_mut(3).unwrap().health.unconscious = true;
    let result = step(&state, 1, strike(4, Weapon::Longbow, true), &[(20, 1)]);
    assert_eq!(result.attacks[0].roll_mode, 0);
    let mut state = battle(vec![
        archer(1, Cell(8, 5)),
        CombatantSpec::creature(3, "skeleton", Cell(10, 5), 20).unwrap(),
    ]);
    state.map = map("broken-yard");
    rejects(
        &state,
        1,
        strike(3, Weapon::Longbow, true),
        RuleError::TotalCover,
    );
}
#[test]
fn cover_and_dodge_change_recorded_attack_and_dexterity_save_adjustments() {
    let mut state = battle(vec![
        archer(1, Cell(5, 3)),
        CombatantSpec::creature(3, "skeleton", Cell(9, 3), 20).unwrap(),
    ]);
    state.map = map("broken-yard");
    state.actor_mut(3).unwrap().budget.dodging = true;
    let result = step(
        &state,
        1,
        strike(3, Weapon::Longbow, true),
        &[(20, 7), (20, 9)],
    );
    assert_eq!(result.attacks[0].cover, Cover::Half);
    assert_eq!(result.attacks[0].armor_class, 16);
    assert_eq!(result.attacks[0].roll_mode, -1);
    assert_eq!(
        state.dexterity_save_adjustments(3, Cell(5, 3)),
        Ok((2, true, false))
    );
    state.actor_mut(3).unwrap().spec.speed_ft = 0;
    assert_eq!(
        state.dexterity_save_adjustments(3, Cell(5, 3)),
        Ok((2, false, false))
    );
}
#[test]
fn weapon_properties_hands_heavy_finesse_versatile_and_thrown_inventory() {
    let mut state = battle(vec![
        archer(1, Cell(1, 1)),
        CombatantSpec::creature(3, "skeleton", Cell(2, 1), 20).unwrap(),
    ]);
    let mut weapon = usage(Weapon::Longbow, true);
    weapon.hands = 1;
    rejects(
        &state,
        1,
        Action::Attack {
            target: 3,
            weapon,
            spend: AttackSpend::Action,
        },
        RuleError::InvalidHands,
    );
    state.actor_mut(1).unwrap().spec.abilities[1] = 12;
    let result = step(
        &state,
        1,
        strike(3, Weapon::Longbow, true),
        &[(20, 1), (20, 2)],
    );
    assert!(
        result.attacks[0]
            .disadvantage
            .contains(&AttackReason::Heavy)
    );
    let state = battle(vec![
        archer(1, Cell(1, 1)),
        CombatantSpec::creature(3, "skeleton", Cell(2, 1), 20).unwrap(),
    ]);
    let result = step(
        &state,
        1,
        strike(3, Weapon::Shortsword, false),
        &[(20, 10), (6, 2)],
    );
    assert_eq!(result.attacks[0].modifier, 5);
    assert_eq!(result.attacks[0].damage, 5);
    let state = battle(vec![
        melee(1, Cell(1, 1)),
        CombatantSpec::creature(3, "skeleton", Cell(2, 1), 20).unwrap(),
    ]);
    let mut weapon = usage(Weapon::Spear, false);
    weapon.hands = 2;
    assert_eq!(
        step(
            &state,
            1,
            Action::Attack {
                target: 3,
                weapon,
                spend: AttackSpend::Action
            },
            &[(20, 10), (8, 4)]
        )
        .attacks[0]
            .damage,
        7
    );
    let result = step(
        &state,
        1,
        strike(3, Weapon::Javelin, true),
        &[(20, 10), (20, 11), (6, 2)],
    );
    assert_eq!(
        result
            .battle
            .actor(1)
            .unwrap()
            .weapon_count(Weapon::Javelin),
        7
    );
    let mut guard = battle(vec![
        melee(1, Cell(1, 1)),
        CombatantSpec::creature(3, "guard", Cell(2, 1), 0).unwrap(),
    ]);
    guard.active = 1;
    let mut spear = usage(Weapon::Spear, false);
    spear.hands = 2;
    rejects(
        &guard,
        3,
        Action::Attack {
            target: 1,
            weapon: spear,
            spend: AttackSpend::Action,
        },
        RuleError::InvalidHands,
    );
    let guard = step(&guard, 3, Action::DoffShield, &[]).battle;
    assert_eq!(guard.actor(3).unwrap().spec().armor_class, 14);
}
#[test]
fn attack_summary_retains_recorded_damage_faces_and_modifier() {
    let state = battle(vec![
        archer(1, Cell(1, 1)),
        CombatantSpec::creature(3, "skeleton", Cell(2, 1), 20).unwrap(),
    ]);
    let result = step(
        &state,
        1,
        strike(3, Weapon::Shortsword, false),
        &[(20, 10), (6, 2)],
    );

    assert_eq!(result.attacks[0].damage_faces, [2]);
    assert_eq!(result.attacks[0].damage_modifier, 3);
    assert_eq!(
        result
            .draws
            .iter()
            .map(|draw| draw.value)
            .collect::<Vec<_>>(),
        [10, 2]
    );
}
#[test]
fn slow_does_not_stack_and_expires_at_source_next_turn() {
    let state = battle(vec![
        archer(1, Cell(1, 1)),
        melee(2, Cell(1, 7)),
        CombatantSpec::creature(3, "wolf", Cell(10, 1), 0).unwrap(),
    ]);
    let result = step(
        &state,
        1,
        strike(3, Weapon::Longbow, true),
        &[(20, 10), (8, 1)],
    );
    assert_eq!(result.battle.actor(3).unwrap().speed_ft(), 30);
    let mut state = result.battle;
    let effect = state.actor(3).unwrap().slow[0];
    state.actor_mut(3).unwrap().slow.push(effect);
    assert_eq!(state.actor(3).unwrap().speed_ft(), 30);
    let state = next_turn(state, 1);
    assert_eq!(state.actor(3).unwrap().speed_ft(), 40);
}
#[test]
fn vex_nick_light_and_savage_attacker_once_per_turn() {
    let state = battle(vec![
        archer(1, Cell(1, 1)),
        CombatantSpec::creature(3, "skeleton", Cell(2, 1), 20).unwrap(),
    ]);
    let result = step(
        &state,
        1,
        strike(3, Weapon::Shortsword, false),
        &[(20, 10), (6, 1)],
    );
    let result = step(
        &result.battle,
        1,
        Action::Attack {
            target: 3,
            weapon: usage(Weapon::Scimitar, false),
            spend: AttackSpend::Nick,
        },
        &[(20, 10), (20, 11), (6, 1)],
    );
    assert!(result.attacks[0].advantage.contains(&AttackReason::Vex));
    assert_eq!(result.attacks[0].damage, 1);
    assert!(!result.battle.actor(1).unwrap().budget().bonus_action_used);
    rejects(
        &result.battle,
        1,
        Action::Attack {
            target: 3,
            weapon: usage(Weapon::Scimitar, false),
            spend: AttackSpend::Nick,
        },
        RuleError::InvalidExtraAttack,
    );
    let state = battle(vec![
        archer(1, Cell(1, 1)),
        CombatantSpec::creature(3, "skeleton", Cell(5, 1), 20).unwrap(),
    ]);
    let mut weapon = usage(Weapon::Longbow, true);
    weapon.savage_attacker = true;
    let result = step(
        &state,
        1,
        Action::Attack {
            target: 3,
            weapon,
            spend: AttackSpend::Action,
        },
        &[(20, 10), (8, 1), (8, 6)],
    );
    assert_eq!(result.attacks[0].damage, 9);
    assert_eq!(result.draws.len(), 3);
}
#[test]
fn graze_sap_critical_and_second_wind_preserve_action_budget() {
    let state = battle(vec![
        melee(1, Cell(1, 1)),
        CombatantSpec::creature(3, "skeleton", Cell(2, 1), 20).unwrap(),
    ]);
    let result = step(&state, 1, strike(3, Weapon::Greatsword, false), &[(20, 1)]);
    assert!(result.attacks[0].graze);
    assert_eq!(result.attacks[0].damage, 3);
    let result = step(
        &state,
        1,
        strike(3, Weapon::Flail, false),
        &[(20, 10), (8, 1)],
    );
    assert_eq!(result.attacks[0].damage, 8); // skeleton vulnerability
    let state = next_turn(result.battle, 3);
    let result = step(
        &state,
        3,
        strike(1, Weapon::Shortsword, false),
        &[(20, 1), (20, 2)],
    );
    assert!(result.attacks[0].disadvantage.contains(&AttackReason::Sap));
    let mut state = next_turn(result.battle, 1);
    state.actor_mut(1).unwrap().health.hit_points = 2;
    let result = step(&state, 1, Action::SecondWind, &[(10, 4)]);
    assert_eq!(result.battle.actor(1).unwrap().health().hit_points, 7);
    assert!(!result.battle.actor(1).unwrap().budget().action_used);
    rejects(
        &result.battle,
        1,
        Action::SecondWind,
        RuleError::BonusActionSpent,
    );
}
#[test]
fn melee_knockout_ranged_death_massive_damage_and_timed_death_saves() {
    let mut state = battle(vec![
        archer(1, Cell(1, 1)),
        CombatantSpec::creature(3, "skeleton", Cell(2, 1), 20).unwrap(),
    ]);
    state.actor_mut(3).unwrap().health.hit_points = 1;
    let mut sword = usage(Weapon::Shortsword, false);
    sword.knockout = true;
    let result = step(
        &state,
        1,
        Action::Attack {
            target: 3,
            weapon: sword,
            spend: AttackSpend::Action,
        },
        &[(20, 10), (6, 1)],
    );
    let health = result.battle.actor(3).unwrap().health();
    assert_eq!(health.hit_points, 1);
    assert!(health.unconscious && health.prone && !health.dead && health.knocked_out);
    let result = step(
        &state,
        1,
        strike(3, Weapon::Longbow, true),
        &[(20, 10), (20, 11), (8, 1)],
    );
    assert!(result.battle.actor(3).unwrap().health().dead);
    assert_eq!(result.battle.actor(3).unwrap().health().hit_points, 0);
    let mut state = battle(vec![
        melee(1, Cell(1, 1)),
        melee(2, Cell(1, 7)),
        CombatantSpec::creature(3, "skeleton", Cell(5, 1), 20).unwrap(),
    ]);
    state.active = 2;
    state.actor_mut(1).unwrap().health.hit_points = 1;
    let result = step(
        &state,
        3,
        strike(1, Weapon::Shortbow, true),
        &[(20, 15), (6, 1)],
    );
    assert!(!result.battle.actor(1).unwrap().health().dead);
    assert_eq!(result.battle.actor(1).unwrap().health().hit_points, 0);
    let state = next_turn(result.battle, 1);
    assert!(state.needs_death_save());
    rejects(&state, 1, Action::EndTurn, RuleError::NeedDeathSave);
    let result = step(&state, 1, Action::DeathSave, &[(20, 1)]);
    assert_eq!(result.battle.actor(1).unwrap().health().death_failures, 2);
    rejects(&result.battle, 1, Action::DeathSave, RuleError::NotYourTurn);
    let state = next_turn(result.battle, 1);
    let state = step(&state, 1, Action::DeathSave, &[(20, 20)]).battle;
    assert_eq!(state.actor(1).unwrap().health().hit_points, 1);
    assert!(state.actor(1).unwrap().health().prone);
    let state = step(&state, 1, Action::PickUpWeapon(Weapon::Greatsword), &[]).battle;
    assert!(state.actor(1).unwrap().dropped_weapons().is_empty());
    let mut massive = battle(vec![
        melee(1, Cell(1, 1)),
        CombatantSpec::creature(3, "skeleton", Cell(2, 1), 20).unwrap(),
    ]);
    massive.active = 1;
    massive.actor_mut(1).unwrap().health.hit_points = 1;
    let massive = step(
        &massive,
        3,
        strike(1, Weapon::Shortsword, false),
        &[(20, 20), (6, 6), (6, 6)],
    )
    .battle;
    assert!(massive.actor(1).unwrap().health().dead);
}
#[test]
fn postfight_death_save_requires_turn_boundary_and_stable_resets_counters() {
    let mut state = battle(vec![
        melee(1, Cell(1, 1)),
        CombatantSpec::creature(3, "wolf", Cell(10, 1), 0).unwrap(),
    ]);
    state.actor_mut(1).unwrap().health.hit_points = 0;
    state.actor_mut(1).unwrap().health.unconscious = true;
    rejects(&state, 1, Action::DeathSave, RuleError::NotYourTurn);
    for _ in 0..3 {
        state = next_turn(state, 1);
        state = step(&state, 1, Action::DeathSave, &[(20, 10)]).battle;
    }
    let health = state.actor(1).unwrap().health();
    assert!(health.stable);
    assert_eq!((health.death_failures, health.death_successes), (0, 0));
    state = next_turn(state, 1);
    assert!(!state.needs_death_save());
    rejects(&state, 1, Action::DeathSave, RuleError::NotYourTurn);
}
#[test]
fn full_creature_catalog_preserves_actions_and_required_riders() {
    for source in crate::SRD_521_CREATURES {
        let spec = CombatantSpec::creature(3, source.id, Cell(2, 1), 20).unwrap();
        assert_eq!(spec.hit_points, u16::from(source.rules.hit_points));
        assert_eq!(spec.armor_class, u16::from(source.rules.armor_class));
        assert!(spec.speed_ft > 0);
    }
    let mut state = battle(vec![
        melee(1, Cell(1, 1)),
        CombatantSpec::creature(3, "wolf", Cell(2, 1), 0).unwrap(),
        CombatantSpec::creature(4, "wolf", Cell(2, 2), 0).unwrap(),
    ]);
    state.active = 1;
    let result = step(
        &state,
        3,
        strike(1, Weapon::Bite, false),
        &[(20, 15), (20, 16), (6, 1), (20, 6)],
    );
    assert!(
        result.attacks[0]
            .advantage
            .contains(&AttackReason::PackTactics)
    );
    let save = result.attacks[0].strength_save.unwrap();
    assert_eq!(
        (save.actor, save.dc, save.modifier, save.die),
        (1, 12, 5, 6)
    );
    assert!(!save.succeeded);
    assert!(result.battle.actor(1).unwrap().health().prone);
    let mut mastiff = battle(vec![
        melee(1, Cell(1, 1)),
        CombatantSpec::creature(3, "mastiff", Cell(2, 1), 0).unwrap(),
    ]);
    mastiff.active = 1;
    let result = step(
        &mastiff,
        3,
        strike(1, Weapon::Bite, false),
        &[(20, 15), (6, 1), (20, 8)],
    );
    let save = result.attacks[0].strength_save.unwrap();
    assert_eq!(
        (save.actor, save.dc, save.modifier, save.die),
        (1, 11, 5, 8)
    );
    assert!(save.succeeded);
    assert!(!result.battle.actor(1).unwrap().health().prone);
    let mut state = battle(vec![
        melee(1, Cell(1, 1)),
        CombatantSpec::creature(3, "goblin-warrior", Cell(2, 1), 20).unwrap(),
    ]);
    state.active = 1;
    state.actor_mut(1).unwrap().health.prone = true;
    let result = step(
        &state,
        3,
        strike(1, Weapon::Scimitar, false),
        &[(20, 15), (20, 16), (6, 1), (4, 3)],
    );
    assert_eq!(result.attacks[0].damage, 6);
    let result = step(&result.battle, 3, Action::NimbleDisengage, &[]);
    assert!(result.battle.actor(3).unwrap().budget().disengaged);
    let mut state = battle(vec![
        melee(1, Cell(1, 1)),
        CombatantSpec::creature(3, "scout", Cell(2, 1), 20).unwrap(),
    ]);
    state.active = 1;
    let result = step(
        &state,
        3,
        strike(1, Weapon::Shortsword, false),
        &[(20, 15), (6, 1)],
    );
    let result = step(
        &result.battle,
        3,
        Action::Attack {
            target: 1,
            weapon: usage(Weapon::Shortsword, false),
            spend: AttackSpend::Multiattack,
        },
        &[(20, 15), (6, 1)],
    );
    rejects(
        &result.battle,
        3,
        Action::Attack {
            target: 1,
            weapon: AttackUse {
                use_mastery: false,
                ..usage(Weapon::Shortsword, false)
            },
            spend: AttackSpend::Multiattack,
        },
        RuleError::InvalidExtraAttack,
    );
    let state = battle(vec![
        melee(1, Cell(1, 1)),
        CombatantSpec::creature(3, "zombie", Cell(2, 1), 0).unwrap(),
    ]);
    let mut state = state;
    state.actor_mut(3).unwrap().health.hit_points = 1;
    let result = step(
        &state,
        1,
        strike(3, Weapon::Greatsword, false),
        &[(20, 15), (6, 1), (6, 1), (20, 20)],
    );
    assert_eq!(result.battle.actor(3).unwrap().health().hit_points, 1);
    assert!(!result.battle.actor(3).unwrap().health().dead);
    let mut cultist = battle(vec![
        melee(1, Cell(1, 1)),
        CombatantSpec::creature(3, "cultist", Cell(2, 1), 0).unwrap(),
    ]);
    cultist.active = 1;
    let result = step(
        &cultist,
        3,
        strike(1, Weapon::Sickle, false),
        &[(20, 15), (4, 2)],
    );
    assert_eq!(
        (result.attacks[0].damage, result.attacks[0].necrotic_damage),
        (3, 1)
    );
}
#[test]
fn initiative_group_draws_ties_and_invalid_size_or_stats_are_explicit() {
    let src = source();
    let specs = vec![
        melee(1, Cell(1, 1)),
        melee(2, Cell(1, 2)),
        CombatantSpec::creature(3, "wolf", Cell(10, 1), 0).unwrap(),
        CombatantSpec::creature(4, "wolf", Cell(10, 2), 0).unwrap(),
    ];
    let result = begin(
        map("open-court"),
        specs.clone(),
        binding(&src),
        &records(&src, &[(20, 10), (20, 10), (20, 9)]),
    )
    .unwrap();
    assert_eq!(result.battle.initiative_order(), &[1, 2, 3, 4]);
    assert_eq!(result.draws.len(), 3);
    let mut specs = specs;
    specs[0].size = Size::Large;
    assert_eq!(
        begin(
            map("open-court"),
            specs,
            binding(&src),
            &records(&src, &[(20, 10), (20, 10), (20, 9)])
        ),
        Err(RuleError::UnsupportedSize)
    );
    let mut specs = vec![
        melee(1, Cell(1, 1)),
        CombatantSpec::creature(3, "wolf", Cell(10, 1), 0).unwrap(),
    ];
    specs[0].shield = true;
    specs[0].armor_class = 1;
    assert_eq!(
        begin(
            map("open-court"),
            specs,
            binding(&src),
            &records(&src, &[(20, 10), (20, 9)])
        ),
        Err(RuleError::InvalidCombatant)
    );
}
#[test]
fn archer_standard_array_background_creation_data_and_derived_sheet_match() {
    let built = validate_archer("Brynn", &archer_choices()).unwrap();
    assert_eq!(built.character().abilities, [13, 17, 15, 10, 12, 8]);
    assert_eq!(
        (
            built.character().armor_class,
            built.character().hit_points,
            built.character().proficiency_bonus,
            built.character().second_wind_uses
        ),
        (15, 13, 2, 2)
    );
    let data: serde_json::Value = serde_json::from_str(ARCHER_CREATION_JSON).unwrap();
    assert_eq!(
        data["hero"]["ability_arrays"][0]["scores"],
        serde_json::json!([13, 15, 14, 10, 12, 8])
    );
    assert_eq!(
        data["hero"]["background_bonus"],
        serde_json::json!({"dexterity":2,"constitution":1})
    );
    for group in data["creation"]["groups"].as_array().unwrap() {
        let key = group["id"].as_str().unwrap();
        let selected = archer_choices()
            .into_iter()
            .find(|(id, _)| *id == key)
            .unwrap()
            .1;
        assert!(
            group["options"]
                .as_array()
                .unwrap()
                .iter()
                .any(|option| option["id"] == selected)
        );
    }
    assert_eq!(data["departures"], serde_json::json!([]));
    assert_eq!(
        data["equipment"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["item"] == "arrow")
            .unwrap()["quantity"],
        20
    );
    let mut choices = archer_choices();
    choices[10].1 = "fighter-a-soldier-a";
    assert_eq!(
        validate_archer("Brynn", &choices),
        Err(RuleError::InvalidBuild)
    );
    choices = archer_choices();
    choices[6].1 = choices[5].1;
    assert_eq!(
        validate_archer("Brynn", &choices),
        Err(RuleError::InvalidBuild)
    );
}

#[test]
fn death_at_zero_critical_failures_stable_damage_and_recovery_are_source_correct() {
    let mut state = battle(vec![
        melee(1, Cell(1, 1)),
        melee(2, Cell(1, 7)),
        CombatantSpec::creature(3, "skeleton", Cell(2, 1), 20).unwrap(),
    ]);
    state.active = 2;
    state.actor_mut(1).unwrap().health.hit_points = 0;
    state.actor_mut(1).unwrap().health.unconscious = true;
    state.actor_mut(1).unwrap().health.prone = true;
    state.actor_mut(1).unwrap().health.stable = true;
    let result = step(
        &state,
        3,
        strike(1, Weapon::Shortsword, false),
        &[(20, 15), (20, 16), (6, 1), (6, 1)],
    );
    assert!(result.attacks[0].critical);
    let health = result.battle.actor(1).unwrap().health();
    assert!(!health.stable && !health.dead);
    assert_eq!(health.death_failures, 2);
    let mut state = result.battle;
    state.actor_mut(3).unwrap().budget.action_used = false;
    let state = step(
        &state,
        3,
        strike(1, Weapon::Shortsword, false),
        &[(20, 15), (20, 16), (6, 1), (6, 1)],
    )
    .battle;
    assert!(state.actor(1).unwrap().health().dead);
    let mut state = battle(vec![
        archer(1, Cell(1, 1)),
        CombatantSpec::creature(3, "skeleton", Cell(10, 1), 20).unwrap(),
    ]);
    state.actor_mut(3).unwrap().health.dead = true;
    state.actor_mut(3).unwrap().health.unconscious = true;
    state.actor_mut(1).unwrap().spec.arrows = 17;
    state.actor_mut(1).unwrap().ammunition_used = 3;
    rejects(
        &state,
        1,
        Action::RecoverAmmunition {
            elapsed_seconds: 59,
        },
        RuleError::RecoveryUnavailable,
    );
    let state = step(
        &state,
        1,
        Action::RecoverAmmunition {
            elapsed_seconds: 60,
        },
        &[],
    )
    .battle;
    assert_eq!(state.actor(1).unwrap().arrows(), 18);
    rejects(
        &state,
        1,
        Action::RecoverAmmunition {
            elapsed_seconds: 60,
        },
        RuleError::RecoveryUnavailable,
    );
}
#[test]
fn ranged_holder_opportunity_uses_unarmed_or_an_actually_held_melee_weapon() {
    let mut state = battle(vec![
        melee(1, Cell(10, 7)),
        archer(2, Cell(1, 1)),
        CombatantSpec::creature(3, "wolf", Cell(2, 1), 0).unwrap(),
    ]);
    state.active = 2;
    let state = step(&state, 3, Action::Move(vec![Cell(3, 1)]), &[]).battle;
    assert_eq!(state.reaction_offers(), &[2]);
    rejects(
        &state,
        2,
        Action::Opportunity {
            attack: Some(usage(Weapon::Shortsword, false)),
        },
        RuleError::InvalidWeapon,
    );
    let mut unarmed = usage(Weapon::UnarmedStrike, false);
    unarmed.hands = 0;
    let result = step(
        &state,
        2,
        Action::Opportunity {
            attack: Some(unarmed),
        },
        &[(20, 15)],
    );
    assert_eq!(result.attacks[0].damage, 2);
    assert_eq!(result.draws.len(), 1);
    assert!(result.battle.actor(2).unwrap().budget().reaction_used);
}
#[test]
fn vex_marks_multiple_targets_and_expires_after_next_turn_not_at_start() {
    let mut state = battle(vec![
        archer(1, Cell(1, 1)),
        CombatantSpec::creature(3, "skeleton", Cell(2, 1), 20).unwrap(),
        CombatantSpec::creature(4, "skeleton", Cell(2, 2), 20).unwrap(),
    ]);
    state = step(
        &state,
        1,
        strike(3, Weapon::Shortsword, false),
        &[(20, 10), (6, 1)],
    )
    .battle;
    state.actor_mut(1).unwrap().budget.action_used = false;
    state = step(
        &state,
        1,
        strike(4, Weapon::Shortsword, false),
        &[(20, 10), (6, 1)],
    )
    .battle;
    assert_eq!(state.actor(1).unwrap().vex.len(), 2);
    state = next_turn(state, 1);
    assert_eq!(state.actor(1).unwrap().vex.len(), 2);
    let state = step(&state, 1, Action::EndTurn, &[]).battle;
    assert!(state.actor(1).unwrap().vex.is_empty());
}

#[test]
fn tactical_snapshot_preserves_optional_opportunity_pending_path_and_continuation() {
    let before = battle(vec![
        melee(1, Cell(1, 1)),
        CombatantSpec::creature(3, "bandit", Cell(2, 1), 0).unwrap(),
    ]);
    let pending = step(&before, 1, Action::Move(vec![Cell(0, 1), Cell(0, 2)]), &[]).battle;
    assert_eq!(pending.reaction_offers(), &[3]);
    let saved = pending.snapshot().unwrap();
    let recovered = Battle::from_snapshot(map("open-court"), &saved).unwrap();
    assert_eq!(pending, recovered);
    assert_eq!(saved, recovered.snapshot().unwrap());
    let decline = Action::Opportunity { attack: None };
    assert_eq!(
        step(&pending, 3, decline.clone(), &[]),
        step(&recovered, 3, decline, &[])
    );
}

#[test]
fn tactical_snapshot_keeps_a_pending_move_paused_on_an_ally_cell() {
    let before = battle(vec![
        melee(1, Cell(2, 2)),
        melee(2, Cell(2, 4)),
        CombatantSpec::creature(3, "bandit", Cell(3, 2), 0).unwrap(),
    ]);
    let before = step(&before, 1, Action::EndTurn, &[]).battle;
    let pending = step(
        &before,
        2,
        Action::Move(vec![Cell(2, 3), Cell(2, 2), Cell(1, 1)]),
        &[],
    )
    .battle;
    assert_eq!(pending.reaction_offers(), &[3]);
    assert_eq!(pending.actor(2).unwrap().cell(), Cell(2, 2));
    let saved = pending.snapshot().unwrap();
    let recovered = Battle::from_snapshot(map("open-court"), &saved).unwrap();
    assert_eq!(pending, recovered);
}

#[test]
fn tactical_snapshot_rejects_two_combatants_at_rest_on_one_cell() {
    let before = battle(vec![
        melee(1, Cell(2, 2)),
        melee(2, Cell(2, 4)),
        CombatantSpec::creature(3, "bandit", Cell(3, 2), 0).unwrap(),
    ]);
    let mut snapshot: serde_json::Value =
        serde_json::from_slice(&before.snapshot().unwrap()).unwrap();
    snapshot["actors"][1]["spec"]["cell"] = serde_json::json!([2, 2]);
    assert_eq!(
        Battle::from_snapshot(map("open-court"), &serde_json::to_vec(&snapshot).unwrap()),
        Err(RuleError::SnapshotData)
    );
}

#[test]
fn tactical_snapshot_rejects_pending_diagonal_path_through_blocked_corners() {
    let src = source();
    let pending = begin(
        corner_map(),
        vec![
            melee(1, Cell(1, 1)),
            melee(2, Cell(3, 3)),
            CombatantSpec::creature(3, "bandit", Cell(0, 2), 0).unwrap(),
        ],
        binding(&src),
        &records(&src, &[(20, 20), (20, 1), (20, 1)]),
    )
    .unwrap()
    .battle;
    let pending = step(&pending, 1, Action::Move(vec![Cell(1, 0)]), &[]).battle;
    assert_eq!(pending.reaction_offers(), &[3]);

    let mut snapshot: serde_json::Value =
        serde_json::from_slice(&pending.snapshot().unwrap()).unwrap();
    snapshot["pending"]["path"][0] = serde_json::json!([2, 2]);
    assert_eq!(
        Battle::from_snapshot(corner_map(), &serde_json::to_vec(&snapshot).unwrap()),
        Err(RuleError::SnapshotData)
    );
}

#[test]
fn thrown_javelin_lands_at_target_cell_on_hit_and_can_be_picked_up() {
    let before = battle(vec![
        melee(1, Cell(1, 1)),
        CombatantSpec::creature(3, "goblin-warrior", Cell(3, 1), 0).unwrap(),
    ]);
    let mut thrown = step(
        &before,
        1,
        strike(3, Weapon::Javelin, true),
        &[(20, 10), (6, 6)],
    )
    .battle;
    assert_eq!(thrown.actor(1).unwrap().weapon_count(Weapon::Javelin), 7);
    assert_eq!(
        thrown.actor(1).unwrap().dropped_weapons(),
        &[(Weapon::Javelin, Cell(3, 1))]
    );
    thrown.actor_mut(3).unwrap().spec.cell = Cell(3, 2);

    let moved = step(&thrown, 1, Action::Move(vec![Cell(2, 1), Cell(3, 1)]), &[]).battle;
    let picked_up = step(&moved, 1, Action::PickUpWeapon(Weapon::Javelin), &[]).battle;
    assert_eq!(picked_up.actor(1).unwrap().weapon_count(Weapon::Javelin), 8);
    assert!(picked_up.actor(1).unwrap().dropped_weapons().is_empty());
    assert_eq!(
        Battle::from_snapshot(picked_up.map().clone(), &picked_up.snapshot().unwrap()).unwrap(),
        picked_up
    );
}

#[test]
fn thrown_javelin_lands_at_target_cell_on_miss_and_snapshot_keeps_repeated_drops() {
    let before = battle(vec![
        melee(1, Cell(1, 1)),
        CombatantSpec::creature(3, "goblin-warrior", Cell(3, 1), 0).unwrap(),
    ]);
    let thrown = step(&before, 1, strike(3, Weapon::Javelin, true), &[(20, 1)]).battle;
    assert_eq!(thrown.actor(1).unwrap().weapon_count(Weapon::Javelin), 7);
    assert_eq!(
        thrown.actor(1).unwrap().dropped_weapons(),
        &[(Weapon::Javelin, Cell(3, 1))]
    );

    let mut snapshot: serde_json::Value =
        serde_json::from_slice(&thrown.snapshot().unwrap()).unwrap();
    snapshot["actors"][0]["weapon_counts"]["Javelin"] = 6.into();
    snapshot["actors"][0]["dropped"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!(["Javelin", [3, 1]]));
    let restored = Battle::from_snapshot(
        thrown.map().clone(),
        &serde_json::to_vec(&snapshot).unwrap(),
    );
    assert!(restored.is_ok());
}

#[test]
fn tactical_snapshot_preserves_ammunition_spend_second_wind_and_doffed_shield() {
    let before = battle(vec![
        archer(1, Cell(1, 1)),
        CombatantSpec::creature(3, "goblin-warrior", Cell(9, 1), 20).unwrap(),
    ]);
    let fired = step(&before, 1, strike(3, Weapon::Longbow, true), &[(20, 1)]).battle;
    let healed = step(&fired, 1, Action::SecondWind, &[(10, 3)]).battle;
    let enemy_turn = step(&healed, 1, Action::EndTurn, &[]).battle;
    let doffed = step(&enemy_turn, 3, Action::DoffShield, &[]).battle;
    assert_eq!(doffed.actor(1).unwrap().arrows(), 19);
    assert_eq!(doffed.actor(1).unwrap().spec().second_wind_uses, 1);
    assert!(!doffed.actor(3).unwrap().shield_equipped());
    let saved = doffed.snapshot().unwrap();
    assert_eq!(
        Battle::from_snapshot(map("open-court"), &saved).unwrap(),
        doffed
    );
}

#[test]
fn tactical_snapshot_refuses_incompatible_source_map_order_health_and_overlapping_living_cells() {
    let before = battle(vec![
        melee(1, Cell(1, 1)),
        CombatantSpec::creature(3, "bandit", Cell(2, 1), 0).unwrap(),
    ]);
    let bytes = before.snapshot().unwrap();
    assert_eq!(
        Battle::from_snapshot(map("broken-yard"), &bytes),
        Err(RuleError::SnapshotData)
    );
    for field in ["version", "source", "order", "health", "cell"] {
        let mut json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        match field {
            "version" => json["version"] = 2.into(),
            "source" => json["source"] = "unadmitted".into(),
            "order" => json["order"] = serde_json::json!([1, 1]),
            "health" => json["actors"][0]["health"]["hit_points"] = 99.into(),
            "cell" => json["actors"][1]["spec"]["cell"] = serde_json::json!([1, 1]),
            _ => unreachable!(),
        }
        assert_eq!(
            Battle::from_snapshot(map("open-court"), &serde_json::to_vec(&json).unwrap()),
            Err(RuleError::SnapshotData),
            "{field}"
        );
    }
}

#[test]
fn tactical_snapshot_preserves_dropped_weapon_dying_death_save_and_natural_twenty_recovery() {
    let mut state = battle(vec![
        melee(1, Cell(1, 1)),
        melee(2, Cell(1, 7)),
        CombatantSpec::creature(3, "skeleton", Cell(5, 1), 20).unwrap(),
    ]);
    state.active = 2;
    state.actor_mut(1).unwrap().health.hit_points = 1;
    let state = step(
        &state,
        3,
        strike(1, Weapon::Shortbow, true),
        &[(20, 15), (6, 1)],
    )
    .battle;
    assert_eq!(state.actor(1).unwrap().health().hit_points, 0);
    assert!(!state.actor(1).unwrap().dropped_weapons().is_empty());
    let state = next_turn(state, 1);
    let restored = Battle::from_snapshot(state.map().clone(), &state.snapshot().unwrap()).unwrap();
    assert_eq!(state, restored);
    let recovered = step(&restored, 1, Action::DeathSave, &[(20, 20)]).battle;
    let held = step(&recovered, 1, Action::PickUpWeapon(Weapon::Greatsword), &[]).battle;
    assert!(
        held.actor(1)
            .unwrap()
            .held_weapons()
            .contains(&Weapon::Greatsword)
    );
    assert_eq!(
        held,
        Battle::from_snapshot(held.map().clone(), &held.snapshot().unwrap()).unwrap()
    );
    let mut dying = restored;
    dying.actor_mut(1).unwrap().health.death_failures = 2;
    let dead = step(&dying, 1, Action::DeathSave, &[(20, 1)]).battle;
    assert!(dead.actor(1).unwrap().health().dead);
    assert_eq!(dead.actor(1).unwrap().health().death_failures, 3);
    assert_eq!(
        dead,
        Battle::from_snapshot(dead.map().clone(), &dead.snapshot().unwrap()).unwrap()
    );
}

// ---- PLAY-R01 -------------------------------------------------------------------------------
fn dying_hero_state() -> Battle {
    let mut state = battle(vec![
        melee(1, Cell(1, 1)),
        melee(2, Cell(1, 7)),
        CombatantSpec::creature(3, "skeleton", Cell(10, 1), 20).unwrap(),
    ]);
    let health = &mut state.actor_mut(1).unwrap().health;
    health.hit_points = 0;
    health.unconscious = true;
    health.prone = true;
    state.active = state.order.iter().position(|id| *id == 1).unwrap();
    state.needs_death_save = true;
    state
}
#[test]
fn a_death_saving_throw_reports_the_roll_the_running_counts_and_the_result() {
    use DeathSaveResult as R;
    let state = dying_hero_state();
    let cases = [
        (14, R::Success, 1, 0),
        (9, R::Failure, 0, 1),
        (1, R::CriticalFailure, 0, 2),
        (20, R::Revived, 0, 0),
    ];
    for (die, result, successes, failures) in cases {
        let resolved = step(&state, 1, Action::DeathSave, &[(20, die)]);
        assert_eq!(
            resolved.death_saves,
            vec![DeathSaveSummary {
                actor: 1,
                die,
                successes,
                failures,
                result
            }]
        );
        assert!(!resolved.battle.needs_death_save());
    }
    let mut third_success = state.clone();
    third_success.actor_mut(1).unwrap().health.death_successes = 2;
    let resolved = step(&third_success, 1, Action::DeathSave, &[(20, 10)]);
    assert_eq!(resolved.death_saves[0].result, R::Stabilised);
    assert!(resolved.battle.actor(1).unwrap().health().stable);
    let mut third_failure = state;
    third_failure.actor_mut(1).unwrap().health.death_failures = 2;
    let resolved = step(&third_failure, 1, Action::DeathSave, &[(20, 4)]);
    assert_eq!(resolved.death_saves[0].result, R::Died);
    assert!(resolved.battle.actor(1).unwrap().health().dead);
}

#[test]
fn a_move_reports_the_squares_entered_and_their_cost_and_a_paused_move_reports_only_what_it_entered()
 {
    let state = battle(vec![
        melee(1, Cell(1, 1)),
        CombatantSpec::creature(3, "wolf", Cell(10, 1), 0).unwrap(),
    ]);
    let path = vec![Cell(2, 1), Cell(3, 1)];
    let moved = step(&state, 1, Action::Move(path.clone()), &[]);
    assert_eq!(moved.moves.len(), 1);
    let summary = &moved.moves[0];
    assert_eq!((summary.actor, summary.from), (1, Cell(1, 1)));
    assert_eq!(summary.path, path);
    assert_eq!(summary.cost_ft, 10);
    assert_eq!(summary.remaining_ft, 20);
    // Leaving a hostile creature's reach pauses before the first square: nothing entered yet.
    let adjacent = battle(vec![
        melee(1, Cell(1, 1)),
        CombatantSpec::creature(3, "wolf", Cell(2, 1), 0).unwrap(),
    ]);
    let paused = step(
        &adjacent,
        1,
        Action::Move(vec![Cell(0, 1), Cell(0, 2)]),
        &[],
    );
    assert!(paused.moves.is_empty());
    assert_eq!(paused.battle.reaction_offers(), &[3]);
}

#[test]
fn kobold_sunlight_sensitivity_cancels_pack_tactics_and_its_initiative_has_disadvantage() {
    let src = source();
    let sunlit = map("open-court").with_lighting(Lighting::Sunlight);
    let specs = vec![
        melee(1, Cell(3, 3)),
        CombatantSpec::creature(10, "kobold-warrior", Cell(4, 3), 0).unwrap(),
        CombatantSpec::creature(11, "kobold-warrior", Cell(3, 4), 0).unwrap(),
    ];
    // The Kobold group rolls twice and keeps the lower face (5): 5 + 2 beats the hero's 1 + 1.
    let started = begin(
        sunlit.clone(),
        specs.clone(),
        binding(&src),
        &records(&src, &[(20, 1), (20, 18), (20, 5)]),
    )
    .unwrap();
    assert_eq!(started.battle.initiative_order(), &[10, 11, 1]);
    assert_eq!(
        begin(
            sunlit,
            specs.clone(),
            binding(&src),
            &records(&src, &[(20, 1), (20, 18)])
        )
        .map(|_| ()),
        Err(RuleError::MissingDraw { sides: 20 })
    );
    let state = started.battle;
    let mut dagger = usage(Weapon::Dagger, false);
    dagger.use_mastery = false;
    let (advantage, disadvantage) = state.attack_reasons(10, 1, dagger).unwrap();
    assert!(advantage.contains(&AttackReason::PackTactics));
    assert!(disadvantage.contains(&AttackReason::SunlightSensitivity));
    let plain = step(
        &state,
        10,
        strike(1, Weapon::Dagger, false),
        &[(20, 15), (4, 2)],
    );
    assert_eq!(plain.attacks[0].roll_mode, 0);
    assert_eq!(plain.attacks[0].faces, vec![15]);
    // The same fight at night: Pack Tactics alone gives Advantage and two d20 faces.
    let night = begin(
        map("open-court"),
        specs,
        binding(&src),
        &records(&src, &[(20, 1), (20, 5)]),
    )
    .unwrap()
    .battle;
    let rolled = step(
        &night,
        10,
        strike(1, Weapon::Dagger, false),
        &[(20, 3), (20, 15), (4, 2)],
    );
    assert_eq!(rolled.attacks[0].roll_mode, 1);
    assert_eq!(rolled.attacks[0].selected, 15);
}

#[test]
fn a_kobold_throws_its_dagger_at_normal_range_and_carries_three() {
    let state = battle(vec![
        melee(1, Cell(1, 1)),
        CombatantSpec::creature(3, "kobold-warrior", Cell(5, 1), 0).unwrap(),
    ]);
    let state = next_turn(state, 3);
    let thrown = step(
        &state,
        3,
        strike(1, Weapon::Dagger, true),
        &[(20, 14), (4, 2)],
    );
    let attack = &thrown.attacks[0];
    assert!(attack.ranged);
    assert_eq!(attack.distance_ft, 20);
    assert!(attack.disadvantage.is_empty());
    assert_eq!(
        thrown.battle.actor(3).unwrap().weapon_count(Weapon::Dagger),
        2
    );
    // Beyond long range (60 ft) a thrown dagger cannot be used at all.
    let far = battle(vec![
        melee(1, Cell(0, 0)),
        CombatantSpec::creature(3, "kobold-warrior", Cell(15, 0), 0).unwrap(),
    ]);
    let far = next_turn(far, 3);
    let mut throw = usage(Weapon::Dagger, true);
    throw.use_mastery = false;
    rejects(
        &far,
        3,
        Action::Attack {
            target: 1,
            weapon: throw,
            spend: AttackSpend::Action,
        },
        RuleError::OutOfRange,
    );
}
