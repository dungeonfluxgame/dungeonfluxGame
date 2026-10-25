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
        species: Some(crate::local_journey::HeroSpecies::Dwarf),
        species_options: Default::default(),
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
        healers_kit_uses: 10,
        relentless_endurance_available: false,
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
fn battle_with_map(map: BattleMap, specs: Vec<CombatantSpec>) -> Battle {
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
        if spec
            .fighter_level()
            .is_some_and(|progression| progression.champion())
        {
            faces.push((20, 20));
        }
        kinds.push(spec.kind);
    }
    begin(map, specs, binding(&src), &records(&src, &faces))
        .unwrap()
        .battle
}
fn battle(specs: Vec<CombatantSpec>) -> Battle {
    battle_with_map(map("open-court"), specs)
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
fn srd_521_p14_dead_creature_space_uses_disclosed_passable_policy() {
    // SRD 5.2.1 p. 14 does not say whether a dead creature still occupies a space.
    let mut state = battle(vec![
        melee(1, Cell(1, 1)),
        CombatantSpec::creature(3, "wolf", Cell(2, 1), 0).unwrap(),
        CombatantSpec::creature(4, "wolf", Cell(10, 1), 0).unwrap(),
    ]);
    state.actor_mut(3).unwrap().health.dead = true;
    state.actor_mut(3).unwrap().health.unconscious = true;
    state.actor_mut(3).unwrap().health.hit_points = 0;
    assert_eq!(state.validate_path(1, &[Cell(2, 1)]), Ok(5));
    assert_eq!(
        state.shortest_path(1, Cell(2, 1), None),
        Ok(vec![Cell(2, 1)])
    );
}
#[test]
fn standing_crawling_dash_and_disengage_use_source_budgets() {
    // SRD 5.2.1 p. 186: crawling adds movement cost; standing costs half Speed.
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
fn srd_521_p15_opportunity_attack_is_not_offered_through_a_blocked_corner() {
    let map = BattleMap::from_json(
        br#"{"schema":2,"grid":{"cols":4,"rows":4,"blocked":[[2,1],[1,2]]},"layouts":[{"id":"blocked-corner","obstacles":[],"difficult":[],"spawns":{"heroes":[[1,1]],"enemies":[[2,2]]}}]}"#,
        "blocked-corner",
    )
    .unwrap();
    let mut state = battle_with_map(
        map,
        vec![
            melee(1, Cell(1, 1)),
            CombatantSpec::creature(3, "bandit", Cell(2, 2), 0).unwrap(),
        ],
    );
    assert_eq!(state.active_actor(), Some(1));
    state = step(&state, 1, Action::EndTurn, &[]).battle;
    assert_eq!(state.active_actor(), Some(3));

    let moved = step(&state, 3, Action::Move(vec![Cell(3, 2)]), &[]).battle;

    assert!(moved.reaction_offers().is_empty());
    assert_eq!(moved.actor(3).unwrap().cell(), Cell(3, 2));
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
        // SRD 5.2.1 p. 364: a Wolf Bite has no post-hit saving throw.
        &[(20, 15), (6, 1)],
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
        // SRD 5.2.1 p. 364: the two d20s are Pack Tactics' attack roll; no save follows.
        &[(20, 20), (20, 19), (6, 1), (6, 1)],
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
fn a_prone_creature_gives_no_cover_to_same_size_or_larger_targets_srd_p15() {
    let mut wolf_target = battle(vec![
        archer(1, Cell(1, 1)),
        melee(2, Cell(5, 1)),
        CombatantSpec::creature(3, "wolf", Cell(9, 1), 0).unwrap(),
    ]);
    wolf_target.actor_mut(2).unwrap().health.prone = true;
    wolf_target.actor_mut(2).unwrap().health.unconscious = true;
    assert_eq!(wolf_target.attack_cover(1, 3), Ok(Cover::None));

    let mut rat_target = battle(vec![
        archer(1, Cell(1, 1)),
        melee(2, Cell(5, 1)),
        CombatantSpec::creature(3, "giant-rat", Cell(9, 1), 0).unwrap(),
    ]);
    rat_target.actor_mut(2).unwrap().health.prone = true;
    rat_target.actor_mut(2).unwrap().health.unconscious = true;
    assert_eq!(rat_target.attack_cover(1, 3), Ok(Cover::None));

    let small_body = CombatantSpec::creature(2, "giant-rat", Cell(5, 1), 0).unwrap();
    let mut larger_target = battle(vec![
        archer(1, Cell(1, 1)),
        small_body,
        CombatantSpec::creature(3, "wolf", Cell(9, 1), 0).unwrap(),
    ]);
    larger_target.actor_mut(2).unwrap().health.prone = true;
    larger_target.actor_mut(2).unwrap().health.unconscious = true;
    assert_eq!(larger_target.attack_cover(1, 3), Ok(Cover::None));
}

#[test]
fn melee_reach_across_a_low_scan_barrier_remains_refused_srd_p15() {
    let mut state = battle(vec![
        melee(1, Cell(1, 1)),
        CombatantSpec::creature(3, "wolf", Cell(2, 1), 0).unwrap(),
    ]);
    state.map = BattleMap::from_json(
        br#"{"schema":2,"grid":{"cols":4,"rows":4,"blocked":[],"blocked_steps":[[[1,1],[2,1]]],"step_heights_dm":[4]},"layouts":[{"id":"scan","obstacles":[],"difficult":[],"spawns":{"heroes":[[0,0]],"enemies":[[3,3]]}}]}"#,
        "scan",
    )
    .unwrap();
    rejects(
        &state,
        1,
        strike(3, Weapon::Greatsword, false),
        RuleError::Blocked,
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
fn goblin_shortbow_resolves_with_its_shield_equipped() {
    let mut state = battle(vec![
        melee(1, Cell(7, 1)),
        CombatantSpec::creature(3, "goblin-warrior", Cell(2, 1), 20).unwrap(),
    ]);
    state.active = 1;

    let result = step(
        &state,
        3,
        strike(1, Weapon::Shortbow, true),
        &[(20, 13), (6, 1)],
    );

    assert_eq!(result.attacks[0].weapon, Weapon::Shortbow);
    assert!(result.attacks[0].hit);
    assert!(result.battle.actor(3).unwrap().shield_equipped());
    assert_eq!(result.battle.actor(3).unwrap().spec().armor_class, 15);
}
#[test]
fn hero_shield_still_prevents_two_handed_shortbow_attack() {
    let mut state = battle(vec![
        melee(1, Cell(1, 1)),
        CombatantSpec::creature(3, "skeleton", Cell(2, 1), 20).unwrap(),
    ]);
    let hero = state.actor_mut(1).unwrap();
    hero.shield_equipped = true;
    hero.spec.shield = true;
    hero.spec.armor_class = 19;

    rejects(
        &state,
        1,
        strike(3, Weapon::Shortbow, true),
        RuleError::InvalidHands,
    );
}
#[test]
fn thrown_javelin_slow_reduces_speed_until_attacker_next_turn() {
    // SRD 5.2.1 pp. 90-91: the Javelin's mastery is Slow.
    let state = battle(vec![
        melee(1, Cell(1, 1)),
        melee(2, Cell(1, 7)),
        CombatantSpec::creature(3, "wolf", Cell(6, 1), 0).unwrap(),
    ]);
    let result = step(
        &state,
        1,
        strike(3, Weapon::Javelin, true),
        &[(20, 10), (6, 1)],
    );

    assert_eq!(result.attacks[0].mastery, Some(Mastery::Slow));
    assert_eq!(result.battle.actor(3).unwrap().speed_ft(), 30);
    let next = next_turn(result.battle, 2);
    assert_eq!(next.actor(3).unwrap().speed_ft(), 30);
    let next = next_turn(next, 1);
    assert_eq!(next.actor(3).unwrap().speed_ft(), 40);
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
    // SRD 5.2.1 pp. 357, 364: both are Medium, AC 12, Speed 40 ft.; HP are 5/11.
    let wolf = CombatantSpec::creature(3, "wolf", Cell(2, 1), 0).unwrap();
    assert_eq!(
        (wolf.size, wolf.armor_class, wolf.hit_points, wolf.speed_ft),
        (Size::Medium, 12, 11, 40)
    );
    let mastiff = CombatantSpec::creature(3, "mastiff", Cell(2, 1), 0).unwrap();
    assert_eq!(
        (
            mastiff.size,
            mastiff.armor_class,
            mastiff.hit_points,
            mastiff.speed_ft
        ),
        (Size::Medium, 12, 5, 40)
    );
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
        &[(20, 15), (20, 16), (6, 1)],
    );
    assert!(
        result.attacks[0]
            .advantage
            .contains(&AttackReason::PackTactics)
    );
    assert_eq!(result.attacks[0].modifier, 4);
    assert_eq!(result.attacks[0].damage_faces, vec![1]);
    assert_eq!(result.attacks[0].damage, 3);
    assert_eq!(
        result
            .draws
            .iter()
            .map(|draw| draw.sides)
            .collect::<Vec<_>>(),
        vec![20, 20, 6]
    );
    assert!(result.battle.actor(1).unwrap().health().prone);
    let mut state = battle(vec![
        melee(1, Cell(1, 1)),
        CombatantSpec::creature(3, "mastiff", Cell(2, 1), 0).unwrap(),
    ]);
    state.active = 1;
    let result = step(
        &state,
        3,
        strike(1, Weapon::Bite, false),
        &[(20, 15), (6, 1)],
    );
    assert_eq!(result.attacks[0].modifier, 3);
    assert_eq!(result.attacks[0].damage_faces, vec![1]);
    assert_eq!(result.attacks[0].damage, 2);
    assert_eq!(
        result
            .draws
            .iter()
            .map(|draw| draw.sides)
            .collect::<Vec<_>>(),
        vec![20, 6]
    );
    assert!(result.battle.actor(1).unwrap().health().prone);
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
fn creature_senses_and_climb_speeds_match_srd_521_catalog() {
    // SRD 5.2.1 pp. 290, 302, 325, 343, 353, 355, 357, 364.
    for kind in [
        CreatureKind::Goblin,
        CreatureKind::Kobold,
        CreatureKind::Skeleton,
        CreatureKind::Zombie,
        CreatureKind::Wolf,
        CreatureKind::GiantRat,
        CreatureKind::GiantWeasel,
        CreatureKind::Mastiff,
    ] {
        assert_eq!(kind.darkvision_ft(), Some(60));
    }
    assert_eq!(melee(1, Cell(1, 1)).darkvision_ft(), Some(120));
    assert_eq!(CreatureKind::GiantRat.climb_speed_ft(), Some(30));
    assert_eq!(CreatureKind::GiantWeasel.climb_speed_ft(), Some(30));
    assert_eq!(CreatureKind::Wolf.climb_speed_ft(), None);
}

#[test]
fn a_wolf_bite_does_not_knock_a_large_target_prone() {
    // SRD 5.2.1 p. 364: the Prone rider applies only to Medium or smaller targets.
    let mut state = battle(vec![
        melee(1, Cell(1, 1)),
        CombatantSpec::creature(3, "wolf", Cell(2, 1), 0).unwrap(),
    ]);
    state.active = 1;
    // Large combatants are rejected by begin until the tactical grid supports 2x2 footprints.
    // This isolated mutation exercises the bite's size boundary after a valid battle is created.
    state.actor_mut(1).unwrap().spec.size = Size::Large;
    let result = step(
        &state,
        3,
        strike(1, Weapon::Bite, false),
        &[(20, 15), (6, 1)],
    );
    assert!(!result.battle.actor(1).unwrap().health().prone);
    assert_eq!(
        result
            .draws
            .iter()
            .map(|draw| draw.sides)
            .collect::<Vec<_>>(),
        vec![20, 6]
    );
}

#[test]
fn prone_attack_rolls_follow_the_five_foot_boundary() {
    // SRD 5.2.1 p. 186: Prone attackers have Disadvantage; attacks against them have
    // Advantage within 5 feet and Disadvantage beyond 5 feet.
    let mut adjacent = battle(vec![
        melee(1, Cell(1, 1)),
        CombatantSpec::creature(3, "bandit", Cell(2, 1), 0).unwrap(),
    ]);
    adjacent.actor_mut(3).unwrap().health.prone = true;
    let (advantage, disadvantage) = adjacent
        .attack_reasons(1, 3, usage(Weapon::Greatsword, false))
        .unwrap();
    assert!(advantage.contains(&AttackReason::ProneTarget));
    assert!(!disadvantage.contains(&AttackReason::ProneTarget));

    let mut distant = battle(vec![
        melee(1, Cell(1, 1)),
        CombatantSpec::creature(3, "bandit", Cell(4, 1), 0).unwrap(),
    ]);
    distant.actor_mut(3).unwrap().health.prone = true;
    let mut shortbow = usage(Weapon::Shortbow, true);
    shortbow.use_mastery = false;
    let (advantage, disadvantage) = distant.attack_reasons(1, 3, shortbow).unwrap();
    assert!(!advantage.contains(&AttackReason::ProneTarget));
    assert!(disadvantage.contains(&AttackReason::ProneTarget));

    let mut prone_attacker = battle(vec![
        melee(1, Cell(1, 1)),
        CombatantSpec::creature(3, "bandit", Cell(2, 1), 0).unwrap(),
    ]);
    prone_attacker.actor_mut(1).unwrap().health.prone = true;
    let (_, disadvantage) = prone_attacker
        .attack_reasons(1, 3, usage(Weapon::Greatsword, false))
        .unwrap();
    assert!(disadvantage.contains(&AttackReason::ProneAttacker));
}

#[test]
fn wolf_pack_tactics_requires_an_ally_without_incapacitation() {
    // SRD 5.2.1 p. 364: the nearby ally must not have the Incapacitated condition.
    let mut state = battle(vec![
        melee(1, Cell(1, 1)),
        CombatantSpec::creature(3, "wolf", Cell(2, 1), 0).unwrap(),
        CombatantSpec::creature(4, "wolf", Cell(2, 2), 0).unwrap(),
    ]);
    state.active = 1;
    state.actor_mut(4).unwrap().health.unconscious = true;
    let result = step(
        &state,
        3,
        strike(1, Weapon::Bite, false),
        &[(20, 15), (6, 1)],
    );
    assert!(
        !result.attacks[0]
            .advantage
            .contains(&AttackReason::PackTactics)
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
fn srd_521_p13_initiative_tie_uses_disclosed_input_order_even_with_enemy_first() {
    // SRD 5.2.1 p. 13 leaves ties to the DM; this server discloses input order.
    let src = source();
    let specs = vec![
        CombatantSpec::creature(3, "wolf", Cell(10, 1), 0).unwrap(),
        melee(1, Cell(1, 1)),
    ];
    // The Wolf's +2 and Hero's +1 make these equal Initiative totals.
    let result = begin(
        map("open-court"),
        specs,
        binding(&src),
        &records(&src, &[(20, 9), (20, 10)]),
    )
    .unwrap();
    assert_eq!(result.battle.initiative_order(), &[3, 1]);
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
            CombatantSpec::creature(3, "bandit", Cell(0, 0), 0).unwrap(),
        ],
        binding(&src),
        &records(&src, &[(20, 20), (20, 1), (20, 1)]),
    )
    .unwrap()
    .battle;
    let pending = step(&pending, 1, Action::Move(vec![Cell(0, 1), Cell(0, 2)]), &[]).battle;
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
            "version" => json["version"] = 1.into(),
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
fn help_can_stabilize_with_medicine_or_consume_one_healers_kit_use() {
    let mut state = battle(vec![
        melee(1, Cell(1, 1)),
        melee(2, Cell(1, 2)),
        CombatantSpec::creature(3, "giant-weasel", Cell(7, 1), 0).unwrap(),
    ]);
    let target = state.actor_mut(2).unwrap();
    target.health.hit_points = 0;
    target.health.unconscious = true;
    target.health.prone = true;

    let medicine = step(
        &state,
        1,
        Action::Help {
            purpose: HelpPurpose::Stabilize { target: 2 },
        },
        &[(20, 9)],
    );
    assert_eq!(
        medicine.stabilizations,
        vec![StabilizationSummary {
            helper: 1,
            target: 2,
            outcome: StabilizationOutcome::Medicine {
                die: 9,
                modifier: 1,
                total: 10,
                success: true,
            },
        }]
    );
    let health = medicine.battle.actor(2).unwrap().health();
    assert_eq!(health.hit_points, 0);
    assert!(health.unconscious && health.stable);
    assert!(medicine.battle.actor(1).unwrap().budget().action_used);

    let mut failed = state.clone();
    failed.actor_mut(2).unwrap().health.death_failures = 1;
    let medicine = step(
        &failed,
        1,
        Action::Help {
            purpose: HelpPurpose::Stabilize { target: 2 },
        },
        &[(20, 8)],
    );
    assert!(!medicine.battle.actor(2).unwrap().health().stable);
    assert_eq!(medicine.battle.actor(2).unwrap().health().death_failures, 1);
    assert!(medicine.battle.actor(1).unwrap().budget().action_used);

    let kit = step(&state, 1, Action::UtilizeHealersKit { target: 2 }, &[]);
    assert_eq!(kit.draws, []);
    assert_eq!(
        kit.stabilizations[0].outcome,
        StabilizationOutcome::HealersKit { uses_remaining: 9 }
    );
    assert!(kit.battle.actor(2).unwrap().health().stable);
    assert_eq!(kit.battle.actor(1).unwrap().spec().healers_kit_uses, 9);
    let mut empty_kit = state;
    empty_kit.actor_mut(1).unwrap().spec.healers_kit_uses = 0;
    rejects(
        &empty_kit,
        1,
        Action::UtilizeHealersKit { target: 2 },
        RuleError::HealersKitMissing,
    );
}

#[test]
fn help_gives_one_ally_advantage_on_the_target_before_the_helpers_next_turn() {
    let state = battle(vec![
        melee(1, Cell(1, 1)),
        melee(2, Cell(1, 2)),
        CombatantSpec::creature(3, "giant-weasel", Cell(2, 1), 0).unwrap(),
    ]);
    let helped = step(
        &state,
        1,
        Action::Help {
            purpose: HelpPurpose::Attack { ally: 2, target: 3 },
        },
        &[],
    );
    let restored = Battle::from_snapshot(
        helped.battle.map().clone(),
        &helped.battle.snapshot().unwrap(),
    )
    .unwrap();
    let ally_turn = step(&restored, 1, Action::EndTurn, &[]).battle;
    let attack = step(
        &ally_turn,
        2,
        strike(3, Weapon::Greatsword, false),
        &[(20, 5), (20, 15), (6, 1), (6, 1)],
    );
    assert!(attack.attacks[0].advantage.contains(&AttackReason::Help));
    assert_eq!(attack.attacks[0].roll_mode, 1);
    assert_eq!(attack.attacks[0].selected, 15);

    let next_helper_turn = next_turn(attack.battle, 1);
    let (advantage, _) = next_helper_turn
        .attack_reasons(2, 3, usage(Weapon::Greatsword, false))
        .unwrap();
    assert!(!advantage.contains(&AttackReason::Help));
}

#[test]
fn a_won_encounter_reports_srd_challenge_rating_xp_and_its_total() {
    let mut state = battle(vec![
        melee(1, Cell(1, 1)),
        CombatantSpec::creature(3, "giant-weasel", Cell(7, 1), 0).unwrap(),
        CombatantSpec::creature(4, "giant-weasel", Cell(8, 1), 0).unwrap(),
        CombatantSpec::creature(5, "giant-weasel", Cell(9, 1), 0).unwrap(),
    ]);
    for id in [3, 4, 5] {
        let enemy = state.actor_mut(id).unwrap();
        enemy.health.hit_points = 0;
        enemy.health.unconscious = true;
        enemy.health.dead = true;
    }

    assert_eq!(state.winner(), Some(Side::Heroes));
    let award = state.experience_award().unwrap().unwrap();
    assert_eq!(award.total_xp, 75);
    assert_eq!(award.creatures.len(), 3);
    assert!(award.creatures.iter().all(|creature| {
        creature.challenge_rating == ChallengeRating::Eighth && creature.experience_points == 25
    }));

    let mut defeat = battle(vec![
        melee(1, Cell(1, 1)),
        CombatantSpec::creature(3, "giant-weasel", Cell(2, 1), 0).unwrap(),
    ]);
    let hero = defeat.actor_mut(1).unwrap();
    hero.health.hit_points = 0;
    hero.health.dead = true;
    hero.health.unconscious = true;
    assert_eq!(defeat.experience_award(), Ok(None));
}

#[test]
fn halfling_luck_rerolls_a_natural_one_on_an_attack_and_uses_the_new_face() {
    let mut halfling = melee(1, Cell(1, 1));
    halfling.species = Some(crate::local_journey::HeroSpecies::Halfling);
    halfling.size = Size::Small;
    halfling.hit_points = 12;
    let state = battle(vec![
        halfling,
        CombatantSpec::creature(3, "bandit", Cell(2, 1), 0).unwrap(),
    ]);

    let resolved = step(
        &state,
        1,
        strike(3, Weapon::Greatsword, false),
        &[(20, 1), (20, 15), (6, 2), (6, 3)],
    );

    assert_eq!(resolved.attacks[0].faces, vec![15]);
    assert_eq!(resolved.attacks[0].selected, 15);
    assert_eq!(
        resolved.rerolls,
        vec![D20Reroll {
            actor: 1,
            test: D20Test::Attack,
            original: 1,
            replacement: 15,
        }]
    );
}

#[test]
fn orc_relentless_endurance_prevents_nonlethal_damage_from_reducing_hp_to_zero() {
    let mut orc = melee(1, Cell(1, 1));
    orc.species = Some(crate::local_journey::HeroSpecies::Orc);
    orc.hit_points = 12;
    orc.relentless_endurance_available = true;
    let mut state = battle(vec![
        orc,
        CombatantSpec::creature(3, "giant-weasel", Cell(2, 1), 0).unwrap(),
    ]);
    state.actor_mut(1).unwrap().health.hit_points = 1;
    let state = next_turn(state, 3);

    let resolved = step(
        &state,
        3,
        strike(1, Weapon::Bite, false),
        &[(20, 15), (4, 4)],
    );

    let orc = resolved.battle.actor(1).unwrap();
    assert_eq!(orc.health().hit_points, 1);
    assert!(!orc.health().unconscious && !orc.health().dead);
    assert!(!orc.spec().relentless_endurance_available);
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
fn darkness_uses_observer_darkvision_for_unseen_attack_reasons() {
    let dark = map("open-court").with_lighting(Lighting::Darkness);
    let state = battle_with_map(
        dark,
        vec![
            melee(1, Cell(3, 3)),
            CombatantSpec::creature(10, "bandit", Cell(4, 3), 0).unwrap(),
        ],
    );

    // A bandit has no darkvision, so its attack at a visible distance is still
    // against an unseen target in Darkness (SRD 5.2.1 p16).
    let mut bandit_weapon = usage(Weapon::Scimitar, false);
    bandit_weapon.use_mastery = false;
    let (_, disadvantage) = state.attack_reasons(10, 1, bandit_weapon).unwrap();
    assert!(disadvantage.contains(&AttackReason::UnseenTarget));

    // The Dwarf hero's 120-foot darkvision makes the same distance visible.
    let (advantage, disadvantage) = state
        .attack_reasons(1, 10, usage(Weapon::Greatsword, false))
        .unwrap();
    assert!(advantage.contains(&AttackReason::UnseenAttacker));
    assert!(!disadvantage.contains(&AttackReason::UnseenTarget));

    let long_map = BattleMap::from_json(
        br#"{"schema":2,"grid":{"cols":26,"rows":1,"blocked":[]},"layouts":[{"id":"long-dark","obstacles":[],"difficult":[],"spawns":{"heroes":[[0,0]],"enemies":[[25,0]]}}]}"#,
        "long-dark",
    )
    .unwrap()
    .with_lighting(Lighting::Darkness);
    let long_state = battle_with_map(
        long_map,
        vec![
            archer(1, Cell(0, 0)),
            CombatantSpec::creature(10, "bandit", Cell(25, 0), 0).unwrap(),
        ],
    );
    let (_, disadvantage) = long_state
        .attack_reasons(1, 10, usage(Weapon::Longbow, true))
        .unwrap();
    assert!(disadvantage.contains(&AttackReason::UnseenTarget));
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

#[test]
fn a_scan_fence_refuses_movement_reach_and_attacks_without_spending_a_turn() {
    let mut data = serde_json::json!({"schema":2,"grid":{"cols":4,"rows":4,"blocked":[]},
        "layouts":[{"id":"fence","obstacles":[],"difficult":[],"spawns":{"heroes":[[1,1]],"enemies":[[1,3]]}}]});
    let mut barriers = Vec::new();
    for row in 0..4 {
        for next_row in (row - 1).max(0)..=(row + 1).min(3) {
            barriers.push([[1, row], [2, next_row]]);
        }
    }
    data["grid"]["blocked_steps"] = serde_json::json!(barriers);
    let map = BattleMap::from_json(&serde_json::to_vec(&data).unwrap(), "fence").unwrap();
    let src = source();
    let state = begin(
        map,
        vec![
            melee(1, Cell(1, 1)),
            CombatantSpec::creature(2, "wolf", Cell(2, 1), 0).unwrap(),
        ],
        binding(&src),
        &records(&src, &[(20, 20), (20, 1)]),
    )
    .unwrap()
    .battle;
    assert!(
        state
            .reachable_cells(1)
            .unwrap()
            .iter()
            .all(|cell| cell.0 < 2)
    );
    rejects(
        &state,
        1,
        Action::Move(vec![Cell(1, 1), Cell(2, 1)]),
        RuleError::Blocked,
    );
    rejects(
        &state,
        1,
        strike(2, Weapon::Greatsword, false),
        RuleError::TotalCover,
    );
}

#[test]
fn fighter_srd_521_pp23_47_49_fixed_progression_and_level_four_asi() {
    for (level, hp, proficiency, winds) in [
        (1, 13, 2, 2),
        (2, 22, 2, 2),
        (3, 31, 2, 2),
        (4, 40, 2, 3),
        (5, 49, 3, 3),
    ] {
        let profile = melee(1, Cell(1, 1)).at_fighter_level(level).unwrap();
        let progression = profile.fighter_level().unwrap();
        assert_eq!(profile.hit_points, hp);
        assert_eq!(progression.proficiency_bonus(), proficiency);
        assert_eq!(profile.second_wind_uses, winds);
        assert_eq!(profile.abilities[0], if level >= 4 { 19 } else { 17 });
        assert_eq!(profile.masteries.len(), if level >= 4 { 4 } else { 3 });
        assert_eq!(progression.athletics_advantage(), level >= 3);
        assert_eq!(
            profile
                .hero_attack_modifiers(Weapon::Greatsword, false, false)
                .unwrap()
                .0,
            (if level >= 4 { 4 } else { 3 }) + i16::from(proficiency)
        );
        let bow = archer(2, Cell(2, 2)).at_fighter_level(level).unwrap();
        assert_eq!(bow.abilities[1], if level >= 4 { 19 } else { 17 });
        assert_eq!(bow.armor_class, if level >= 4 { 16 } else { 15 });
        for species in [HeroSpecies::Halfling, HeroSpecies::Orc] {
            let mut base = melee(1, Cell(1, 1));
            base.species = Some(species);
            base.hit_points = 12;
            if species == HeroSpecies::Halfling {
                base.size = Size::Small;
            }
            let hero = base.at_fighter_level(level).unwrap();
            assert_eq!(hero.hit_points, 12 + 8 * u16::from(level - 1));
        }
    }
    assert!(melee(1, Cell(1, 1)).at_fighter_level(6).is_err());
}
#[test]
fn fighter_srd_521_p48_tactical_mind_spends_only_if_failed_check_becomes_success() {
    let progression = crate::local_journey::FighterProgression::new(2).unwrap();
    assert_eq!(progression.tactical_mind(8, 15, 6, 2), Ok((14, 2)));
    assert_eq!(progression.tactical_mind(8, 15, 7, 2), Ok((15, 1)));
    assert!(progression.tactical_mind(15, 15, 7, 2).is_err());
    assert!(progression.tactical_mind(8, 15, 7, 0).is_err());
    assert!(progression.tactical_mind(8, 15, 11, 2).is_err());
}
#[test]
fn fighter_srd_521_p48_extra_attack_targets_and_rolls_are_independent() {
    let state = battle(vec![
        melee(1, Cell(1, 1)).at_fighter_level(5).unwrap(),
        CombatantSpec::creature(3, "skeleton", Cell(2, 1), 0).unwrap(),
        CombatantSpec::creature(4, "skeleton", Cell(1, 2), 0).unwrap(),
    ]);
    let action = |target, spend| Action::Attack {
        target,
        weapon: usage(Weapon::Greatsword, false),
        spend,
    };
    let first = step(
        &state,
        1,
        action(3, AttackSpend::Action),
        &[(20, 12), (6, 1), (6, 2)],
    );
    assert_eq!(first.attacks[0].target, 3);
    assert!(first.battle.actor(1).unwrap().extra_attacks_available());
    let second = step(
        &first.battle,
        1,
        action(4, AttackSpend::Multiattack),
        &[(20, 2)],
    );
    assert_eq!(second.attacks[0].target, 4);
    assert_eq!(second.attacks[0].selected, 2);
    assert!(!second.attacks[0].hit);
    assert!(!second.battle.actor(1).unwrap().extra_attacks_available());
    rejects(
        &second.battle,
        1,
        action(4, AttackSpend::Multiattack),
        RuleError::InvalidExtraAttack,
    );
}
#[test]
fn fighter_srd_521_p48_action_surge_survives_pending_checkpoint_and_recharges_on_both_rests() {
    let mut state = battle(vec![
        melee(1, Cell(1, 1)).at_fighter_level(5).unwrap(),
        CombatantSpec::creature(3, "skeleton", Cell(2, 1), 0).unwrap(),
    ]);
    state = step(&state, 1, Action::Dodge, &[]).battle;
    state = step(&state, 1, Action::ActionSurge, &[]).battle;
    assert_eq!(state.actor(1).unwrap().additional_actions(), 1);
    rejects(
        &state,
        1,
        Action::ActionSurge,
        RuleError::RecoveryUnavailable,
    );
    let bytes = state.snapshot().unwrap();
    let restored = Battle::from_snapshot(state.map().clone(), &bytes).unwrap();
    assert_eq!(state, restored);
    assert_eq!(bytes, restored.snapshot().unwrap());
    let struck = step(
        &restored,
        1,
        Action::Attack {
            target: 3,
            weapon: usage(Weapon::Greatsword, false),
            spend: AttackSpend::Action,
        },
        &[(20, 19), (6, 4), (6, 4), (6, 4), (6, 4)],
    );
    assert!(struck.attacks[0].critical);
    assert_eq!(struck.battle.actor(1).unwrap().additional_actions(), 0);
    let rested = step(
        &struck.battle,
        1,
        Action::ShortRest {
            elapsed_seconds: 3600,
        },
        &[],
    )
    .battle;
    assert!(rested.actor(1).unwrap().action_surge_available());
    let mut spent = rested.clone();
    spent.actor_mut(1).unwrap().action_surge_spent = true;
    let rested = step(
        &spent,
        1,
        Action::LongRest {
            elapsed_seconds: 28800,
        },
        &[],
    )
    .battle;
    assert!(rested.actor(1).unwrap().action_surge_available());
}
#[test]
fn fighter_srd_521_p49_champion_nineteen_is_critical_and_grants_safe_half_speed_move() {
    let state = battle(vec![
        melee(1, Cell(1, 1)).at_fighter_level(3).unwrap(),
        CombatantSpec::creature(3, "skeleton", Cell(2, 1), 0).unwrap(),
        CombatantSpec::creature(4, "skeleton", Cell(5, 5), 0).unwrap(),
    ]);
    let struck = step(
        &state,
        1,
        Action::Attack {
            target: 3,
            weapon: usage(Weapon::Greatsword, false),
            spend: AttackSpend::Action,
        },
        &[(20, 19), (6, 1), (6, 1), (6, 1), (6, 1)],
    );
    assert!(struck.attacks[0].critical);
    assert_eq!(struck.attacks[0].damage_faces.len(), 4);
    assert_eq!(struck.battle.actor(1).unwrap().feature_movement_ft(), 15);
    let moved = step(&struck.battle, 1, Action::Move(vec![Cell(0, 1)]), &[]);
    assert!(moved.battle.reaction_offers().is_empty());
    assert_eq!(moved.battle.actor(1).unwrap().budget().movement_spent_ft, 0);
    assert_eq!(moved.battle.actor(1).unwrap().feature_movement_ft(), 10);
}
#[test]
fn fighter_srd_521_p48_second_wind_heals_level_and_tactical_shift_does_not_spend_speed() {
    let mut state = battle(vec![
        melee(1, Cell(1, 1)).at_fighter_level(5).unwrap(),
        CombatantSpec::creature(3, "skeleton", Cell(2, 1), 0).unwrap(),
    ]);
    state.actor_mut(1).unwrap().health.hit_points = 10;
    let wind = step(&state, 1, Action::SecondWind, &[(10, 4)]);
    assert_eq!(wind.battle.actor(1).unwrap().health().hit_points, 19);
    assert_eq!(wind.battle.actor(1).unwrap().spec().second_wind_uses, 2);
    let moved = step(&wind.battle, 1, Action::Move(vec![Cell(0, 1)]), &[]);
    assert!(moved.battle.reaction_offers().is_empty());
    assert_eq!(moved.battle.actor(1).unwrap().budget().movement_spent_ft, 0);
}
#[test]
fn fighter_srd_521_action_surge_dash_and_end_turn_consume_only_their_own_budgets() {
    let state = battle(vec![
        melee(1, Cell(1, 1)).at_fighter_level(5).unwrap(),
        CombatantSpec::creature(3, "skeleton", Cell(5, 5), 0).unwrap(),
    ]);
    let state = step(&state, 1, Action::Dash, &[]).battle;
    let state = step(&state, 1, Action::ActionSurge, &[]).battle;
    let state = step(&state, 1, Action::Dash, &[]).battle;
    assert_eq!(state.actor(1).unwrap().movement_remaining_ft(), 90);
    assert!(!state.actor(1).unwrap().budget().bonus_action_used);
    assert_eq!(
        state,
        Battle::from_snapshot(state.map().clone(), &state.snapshot().unwrap()).unwrap()
    );
    let state = step(&state, 1, Action::EndTurn, &[]).battle;
    assert_eq!(state.actor(1).unwrap().additional_actions(), 0);
}

#[test]
fn fighter_srd_521_level_five_content_pins_the_exact_compiled_builds() {
    for bytes in LEVEL_FIVE_BUILDS {
        let content: serde_json::Value = serde_json::from_str(bytes).unwrap();
        assert_eq!(
            content["source"]["mechanics_revision"],
            FIGHTER_SOURCE_REVISION
        );
        assert_eq!(
            content["source"]["sha256"],
            "8974902d109d6e63672d7c490bde9ccf052410503d9cfa768237154fbc5e3d87"
        );
        let pairs = content["starting_choices"].as_array().unwrap();
        let choices = pairs
            .iter()
            .map(|pair| (pair[0].as_str().unwrap(), pair[1].as_str().unwrap()))
            .collect::<Vec<_>>();
        let mut legal = if content["id"] == "dwarf-archer-level5" {
            validate_archer("Brynn", &choices)
                .unwrap()
                .character()
                .clone()
        } else {
            crate::local_journey::validate_character(
                &crate::local_journey::CreationRules::srd_521(),
                "Brynn",
                &choices,
            )
            .unwrap()
        };
        crate::local_journey::FighterProgression::new(5)
            .unwrap()
            .apply(&mut legal)
            .unwrap();
        assert_eq!(
            serde_json::json!(legal.abilities),
            content["hero"]["abilities"]
        );
        assert_eq!(
            serde_json::json!(legal.hit_points),
            content["hero"]["hit_points"]
        );
        assert_eq!(
            serde_json::json!(legal.armor_class),
            content["hero"]["armor_class"]
        );
        assert_eq!(
            serde_json::json!(legal.proficiency_bonus),
            content["hero"]["proficiency_bonus"]
        );
        assert_eq!(
            serde_json::json!(legal.second_wind_uses),
            content["hero"]["second_wind"]["uses_max"]
        );
    }
}
#[test]
fn fighter_srd_521_p48_two_attack_actions_from_surge_make_four_and_never_five_attacks() {
    let state = battle(vec![
        melee(1, Cell(1, 1)).at_fighter_level(5).unwrap(),
        CombatantSpec::creature(3, "skeleton", Cell(2, 1), 0).unwrap(),
        CombatantSpec::creature(4, "skeleton", Cell(1, 2), 0).unwrap(),
    ]);
    let mut state = step(&state, 1, Action::ActionSurge, &[]).battle;
    for (target, spend) in [
        (3, AttackSpend::Action),
        (3, AttackSpend::Multiattack),
        (4, AttackSpend::Action),
        (4, AttackSpend::Multiattack),
    ] {
        let hit = step(
            &state,
            1,
            Action::Attack {
                target,
                weapon: usage(Weapon::Greatsword, false),
                spend,
            },
            &[(20, 12), (6, 1), (6, 1)],
        );
        assert_eq!(hit.attacks.len(), 1);
        state = hit.battle;
    }
    rejects(
        &state,
        1,
        Action::Attack {
            target: 4,
            weapon: usage(Weapon::Greatsword, false),
            spend: AttackSpend::Action,
        },
        RuleError::ActionSpent,
    );
    assert!(!state.actor(1).unwrap().extra_attacks_available());
}
#[test]
fn fighter_srd_521_p49_champion_initiative_consumes_two_independent_d20s() {
    let src = source();
    let specs = vec![
        melee(1, Cell(1, 1)).at_fighter_level(3).unwrap(),
        CombatantSpec::creature(3, "skeleton", Cell(2, 1), 0).unwrap(),
    ];
    let result = begin(
        map("open-court"),
        specs,
        binding(&src),
        &records(&src, &[(20, 3), (20, 18), (20, 12)]),
    )
    .unwrap();
    assert_eq!(result.draws.len(), 3);
    assert_eq!(result.battle.active_actor(), Some(1));
}

#[test]
fn development_edits_preserve_sources_and_add_to_depleted_battles() {
    let initial = battle(vec![
        melee(1, Cell(1, 1)),
        CombatantSpec::creature(3, "bandit", Cell(5, 1), 20).unwrap(),
    ]);
    let original = initial.snapshot().unwrap();
    assert_eq!(
        initial.development_edit(1, &DevelopmentEdit::Cell(Cell(5, 1))),
        Err(RuleError::Occupied)
    );
    assert_eq!(initial.snapshot().unwrap(), original);
    let depleted = initial
        .development_edit(1, &DevelopmentEdit::HitPoints(3))
        .unwrap()
        .development_edit(
            1,
            &DevelopmentEdit::Resource {
                name: "second-wind".to_owned(),
                value: 0,
            },
        )
        .unwrap()
        .development_edit(
            1,
            &DevelopmentEdit::Resource {
                name: "ammo".to_owned(),
                value: 0,
            },
        )
        .unwrap();
    let added = depleted
        .development_add(
            CombatantSpec::creature(4, "wolf", Cell(3, 3), 0).unwrap(),
            13,
        )
        .unwrap();
    assert_eq!(added.actor(1).unwrap().health().hit_points, 3);
    assert_eq!(added.actor(1).unwrap().spec().second_wind_uses, 0);
    assert_eq!(added.actor(1).unwrap().arrows(), 0);
    assert_eq!(added.development_initiative(4), Some(13));
    let restored = added
        .development_edit(1, &DevelopmentEdit::Restore)
        .unwrap();
    assert_eq!(restored.actor(1).unwrap().health().hit_points, 13);
    let removed = restored
        .development_edit(4, &DevelopmentEdit::Remove)
        .unwrap();
    assert_eq!(removed.combatants().len(), 2);
    assert_eq!(
        Battle::from_snapshot(removed.map().clone(), &removed.snapshot().unwrap()).unwrap(),
        removed
    );
}
#[test]
fn development_legacy_initiative_metadata_is_optional_and_attaches_real_totals_without_rerolling() {
    let initial = battle(vec![
        melee(1, Cell(1, 1)),
        CombatantSpec::creature(3, "bandit", Cell(5, 1), 20).unwrap(),
    ]);
    let mut legacy: serde_json::Value =
        serde_json::from_slice(&initial.snapshot().unwrap()).unwrap();
    legacy.as_object_mut().unwrap().remove("initiatives");
    let decoded =
        Battle::from_snapshot(initial.map().clone(), &serde_json::to_vec(&legacy).unwrap())
            .unwrap();
    assert_eq!(decoded.development_initiative(1), None);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&decoded.snapshot().unwrap()).unwrap(),
        legacy
    );
    let order = decoded.initiative_order().to_vec();
    let active = decoded.active_actor();
    let migrated = decoded
        .with_development_initiatives(&[(1, 21), (3, 2)])
        .unwrap();
    assert_eq!(migrated.initiative_order(), order);
    assert_eq!(migrated.active_actor(), active);
    assert_eq!(migrated.development_initiative(1), Some(21));
    assert!(
        migrated
            .development_add(
                CombatantSpec::creature(4, "wolf", Cell(3, 3), 0).unwrap(),
                100
            )
            .is_err()
    );
}

#[test]
fn development_restore_clears_a_dying_turn_without_reopening_a_consumed_save() {
    let dying = dying_hero_state();
    let restored = dying
        .development_edit(1, &DevelopmentEdit::Restore)
        .unwrap();
    assert!(!restored.needs_death_save());
    assert!(restored.actor(1).unwrap().health().hit_points > 0);
    let saved = step(&dying, 1, Action::DeathSave, &[(20, 10)]).battle;
    assert!(!saved.needs_death_save());
    let edited = saved
        .development_edit(3, &DevelopmentEdit::HitPoints(5))
        .unwrap();
    assert!(!edited.needs_death_save());
    rejects(&edited, 1, Action::DeathSave, RuleError::NotYourTurn);
    let healed = dying
        .development_edit(1, &DevelopmentEdit::HitPoints(2))
        .unwrap();
    assert!(!healed.needs_death_save());
    let another_turn = saved.development_edit(1, &DevelopmentEdit::Turn).unwrap();
    assert!(another_turn.needs_death_save());
    assert!(another_turn.actor(1).unwrap().turn_count > saved.actor(1).unwrap().turn_count);
    let spent = step(&restored, 1, Action::Dodge, &[]).battle;
    assert!(spent.actor(1).unwrap().budget().action_used);
    let refreshed = spent.development_edit(1, &DevelopmentEdit::Turn).unwrap();
    assert!(!refreshed.actor(1).unwrap().budget().action_used);
    assert_eq!(refreshed.actor(1).unwrap().budget().movement_spent_ft, 0);
}

#[test]
fn development_depleted_crossbow_ammunition_can_be_refilled() {
    let initial = battle(vec![
        melee(1, Cell(1, 1)),
        CombatantSpec::creature(3, "bandit", Cell(5, 1), 20).unwrap(),
    ]);
    let empty = initial
        .development_edit(
            3,
            &DevelopmentEdit::Resource {
                name: "ammo".to_owned(),
                value: 0,
            },
        )
        .unwrap();
    assert_eq!(empty.actor(3).unwrap().spec().bolts, 0);
    let refilled = empty
        .development_edit(
            3,
            &DevelopmentEdit::Resource {
                name: "ammo".to_owned(),
                value: 20,
            },
        )
        .unwrap();
    assert_eq!(refilled.actor(3).unwrap().spec().bolts, 20);
    for values in [
        serde_json::json!({"1":21}),
        serde_json::json!({"1":99,"3":2}),
        serde_json::json!({"1":21,"3":2,"99":10}),
    ] {
        let mut malformed: serde_json::Value =
            serde_json::from_slice(&refilled.snapshot().unwrap()).unwrap();
        malformed["initiatives"] = values;
        assert!(
            Battle::from_snapshot(
                refilled.map().clone(),
                &serde_json::to_vec(&malformed).unwrap()
            )
            .is_err()
        );
    }
}

#[test]
fn development_conditions_refuse_conscious_zero_hp_and_dead_stable_combinations() {
    let dying = dying_hero_state();
    let before = dying.snapshot().unwrap();
    assert!(
        dying
            .development_edit(
                1,
                &DevelopmentEdit::Condition {
                    name: "unconscious".to_owned(),
                    present: false
                }
            )
            .is_err()
    );
    assert_eq!(dying.snapshot().unwrap(), before);
    let stable = dying
        .development_edit(
            1,
            &DevelopmentEdit::Condition {
                name: "stable".to_owned(),
                present: true,
            },
        )
        .unwrap();
    assert!(!stable.needs_death_save());
    assert!(
        stable
            .development_edit(
                1,
                &DevelopmentEdit::Condition {
                    name: "dead".to_owned(),
                    present: true
                }
            )
            .is_err()
    );
    let unstable = stable
        .development_edit(
            1,
            &DevelopmentEdit::Condition {
                name: "stable".to_owned(),
                present: false,
            },
        )
        .unwrap();
    assert!(unstable.needs_death_save());
    let dead = dying
        .development_edit(3, &DevelopmentEdit::HitPoints(0))
        .unwrap();
    let corpse = dead.snapshot().unwrap();
    for (name, present) in [("stable", true), ("dead", false), ("unconscious", false)] {
        assert!(
            dead.development_edit(
                3,
                &DevelopmentEdit::Condition {
                    name: name.to_owned(),
                    present
                }
            )
            .is_err()
        );
        assert_eq!(dead.snapshot().unwrap(), corpse);
    }
}

// SRD 5.2.1 pp.298,312-313,333,341; PDF SHA256 is pinned by the source manifest.
#[test]
fn r2_stronger_creatures_preserve_source_stats_spaces_and_published_initiative() {
    let manifest: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../development/rules-sources/manifest.json"
    ))
    .unwrap();
    assert_eq!(
        manifest["sha256"],
        "8974902d109d6e63672d7c490bde9ccf052410503d9cfa768237154fbc5e3d87"
    );
    for (
        id,
        page,
        hp,
        ac,
        abilities,
        bonus,
        modifier,
        dice,
        sides,
        speed,
        size,
        initiative,
        attacks,
    ) in [
        (
            "ogre",
            312,
            68,
            11,
            [19, 8, 16, 5, 7, 7],
            6,
            4,
            2,
            8,
            40,
            Size::Large,
            -1,
            1,
        ),
        (
            "owlbear",
            313,
            59,
            13,
            [20, 12, 17, 3, 12, 7],
            7,
            5,
            2,
            8,
            40,
            Size::Large,
            1,
            2,
        ),
        (
            "wight",
            341,
            82,
            14,
            [15, 14, 16, 10, 13, 15],
            4,
            2,
            1,
            8,
            30,
            Size::Medium,
            4,
            2,
        ),
        (
            "hill-giant",
            298,
            105,
            13,
            [21, 8, 19, 5, 9, 6],
            8,
            5,
            3,
            8,
            40,
            Size::Huge,
            2,
            2,
        ),
        (
            "troll",
            333,
            94,
            15,
            [18, 13, 20, 7, 9, 7],
            7,
            4,
            2,
            6,
            30,
            Size::Large,
            1,
            3,
        ),
    ] {
        let source = crate::SRD_521_CREATURES
            .iter()
            .find(|row| row.id == id)
            .unwrap();
        assert_eq!(
            (
                source.page,
                source.rules.hit_points,
                source.rules.armor_class
            ),
            (page, hp, ac)
        );
        assert_eq!(source.rules.abilities, abilities);
        assert_eq!(
            (
                source.rules.attack_bonus,
                source.rules.damage_modifier,
                source.rules.weapon.damage_dice,
                source.rules.weapon.damage_sides
            ),
            (bonus, modifier, dice, sides)
        );
        let spec = CombatantSpec::creature(3, id, Cell(5, 2), 20).unwrap();
        assert_eq!(
            (
                spec.speed_ft,
                spec.size,
                spec.initiative_modifier(),
                spec.multiattack_count()
            ),
            (speed, size, initiative, attacks)
        );
        let state = battle(vec![melee(1, Cell(1, 1)), spec]);
        assert_eq!(
            Battle::from_snapshot(state.map().clone(), &state.snapshot().unwrap()).unwrap(),
            state
        );
    }
}

#[test]
fn r2_large_and_huge_space_reject_overlap_edges_and_swept_walls() {
    let ogre = CombatantSpec::creature(3, "ogre", Cell(5, 2), 0).unwrap();
    assert_eq!(
        ogre.space_cells(ogre.cell),
        vec![Cell(5, 2), Cell(5, 3), Cell(6, 2), Cell(6, 3)]
    );
    let state = battle(vec![melee(1, Cell(4, 3)), ogre.clone()]);
    assert_eq!(
        state.actor(1).unwrap().distance_to(state.actor(3).unwrap()),
        5
    );
    assert_eq!(
        state.attack_from(1, Cell(6, 3), 3, usage(Weapon::Greatsword, false)),
        Err(RuleError::Occupied)
    );
    let mut invalid = ogre.clone();
    invalid.cell = Cell(127, 2);
    let src = source();
    assert_eq!(
        begin(
            map("open-court"),
            vec![melee(1, Cell(1, 1)), invalid],
            binding(&src),
            &records(&src, &[(20, 20), (20, 1)])
        ),
        Err(RuleError::Spawn)
    );
    let walled=BattleMap::from_json(br#"{"schema":2,"grid":{"cols":8,"rows":6,"blocked":[[4,3]]},"layouts":[{"id":"large-wall","obstacles":[],"difficult":[],"spawns":{"heroes":[[0,0]],"enemies":[[2,2]]}}]}"#,"large-wall").unwrap();
    let state = battle_with_map(
        walled,
        vec![
            melee(1, Cell(0, 0)),
            CombatantSpec::creature(3, "ogre", Cell(2, 2), 0).unwrap(),
        ],
    );
    assert_eq!(
        state.validate_path(3, &[Cell(3, 2)]),
        Err(RuleError::Blocked)
    );
    let giant = CombatantSpec::creature(3, "hill-giant", Cell(3, 2), 0).unwrap();
    assert_eq!(giant.space_cells(giant.cell).len(), 9);
}

#[test]
fn r2_stat_block_multiattacks_damage_reach_and_ogre_javelin_dice() {
    for (id, chosen, faces, expected, count, reach) in [
        (
            "ogre",
            Weapon::Greatclub,
            vec![(20, 15), (8, 4), (8, 5)],
            13,
            1,
            5,
        ),
        (
            "owlbear",
            Weapon::Rend,
            vec![(20, 15), (8, 4), (8, 5)],
            14,
            2,
            5,
        ),
        (
            "troll",
            Weapon::Rend,
            vec![(20, 15), (6, 3), (6, 4)],
            11,
            3,
            10,
        ),
        (
            "hill-giant",
            Weapon::TreeClub,
            vec![(20, 15), (8, 4), (8, 5), (8, 4)],
            18,
            2,
            10,
        ),
        (
            "wight",
            Weapon::NecroticSword,
            vec![(20, 15), (8, 4), (8, 5)],
            11,
            2,
            5,
        ),
    ] {
        let state = battle(vec![
            melee(1, Cell(1, 2)).at_fighter_level(5).unwrap(),
            CombatantSpec::creature(3, id, Cell(if reach == 10 { 3 } else { 2 }, 2), 0).unwrap(),
        ]);
        let mut state = next_turn(state, 3);
        for index in 0..count {
            let mut attack = usage(chosen, false);
            attack.use_mastery = false;
            let resolved = step(
                &state,
                3,
                Action::Attack {
                    target: 1,
                    weapon: attack,
                    spend: if index == 0 {
                        AttackSpend::Action
                    } else {
                        AttackSpend::Multiattack
                    },
                },
                &faces,
            );
            assert_eq!(
                resolved.attacks[0].damage + resolved.attacks[0].necrotic_damage,
                expected,
                "{id}"
            );
            assert_eq!(resolved.attacks[0].distance_ft, reach, "{id}");
            state = resolved.battle;
            // Get up between isolated damage samples without modifying source HP or dice.
            if index + 1 < count {
                state.actor_mut(1).unwrap().health.prone = false;
            }
        }
        assert_eq!(state.actor(3).unwrap().extra_attacks, 0);
        let mut attack = usage(chosen, false);
        attack.use_mastery = false;
        rejects(
            &state,
            3,
            Action::Attack {
                target: 1,
                weapon: attack,
                spend: AttackSpend::Multiattack,
            },
            RuleError::InvalidExtraAttack,
        );
    }
    let mut state = next_turn(
        battle(vec![
            melee(1, Cell(1, 2)),
            CombatantSpec::creature(3, "ogre", Cell(5, 2), 0).unwrap(),
        ]),
        3,
    );
    assert_eq!(state.actor(3).unwrap().weapon_count(Weapon::Javelin), 3);
    let resolved = step(
        &state,
        3,
        strike(1, Weapon::Javelin, true),
        &[(20, 15), (6, 3), (6, 4)],
    );
    assert_eq!(resolved.attacks[0].damage, 11);
    state = resolved.battle;
    assert_eq!(state.actor(3).unwrap().weapon_count(Weapon::Javelin), 2);
}

#[test]
fn r2_troll_regenerates_from_zero_and_is_not_prematurely_awarded_as_dead() {
    let mut state = battle(vec![
        melee(1, Cell(1, 2)),
        CombatantSpec::creature(3, "troll", Cell(2, 2), 0).unwrap(),
    ]);
    state.actor_mut(3).unwrap().health.hit_points = 1;
    let state = step(
        &state,
        1,
        strike(3, Weapon::Greatsword, false),
        &[(20, 15), (6, 3), (6, 3)],
    )
    .battle;
    assert_eq!(state.actor(3).unwrap().health.hit_points, 0);
    assert!(!state.actor(3).unwrap().health.dead);
    assert!(!state.finished());
    assert_eq!(state.winner(), None);
    assert_eq!(state.experience_award().unwrap(), None);
    let state = next_turn(state, 3);
    assert_eq!(state.actor(3).unwrap().health.hit_points, 15);
    assert!(!state.actor(3).unwrap().health.unconscious);
    assert!(
        state.actor(3).unwrap().held.is_empty(),
        "Rend is not discarded gear"
    );
}

#[test]
fn r2_javelin_stack_spends_on_hits_and_misses_and_zero_is_not_legal() {
    let mut state = battle(vec![
        melee(1, Cell(1, 1)),
        CombatantSpec::creature(3, "ogre", Cell(7, 1), 0).unwrap(),
    ]);
    assert_eq!(state.actor(1).unwrap().weapon_count(Weapon::Javelin), 8);
    for throw in 1..=8 {
        state = step(
            &state,
            1,
            strike(3, Weapon::Javelin, true),
            if throw % 2 == 1 {
                &[(20, 1)]
            } else {
                &[(20, 15), (6, 1)]
            },
        )
        .battle;
        assert_eq!(
            state.actor(1).unwrap().weapon_count(Weapon::Javelin),
            8 - throw
        );
        assert_eq!(state.actor(1).unwrap().dropped.len(), usize::from(throw));
        if throw < 8 {
            state = next_turn(state, 1);
        }
    }
    assert!(
        state
            .legal_targets(1, usage(Weapon::Javelin, true))
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        Battle::from_snapshot(state.map().clone(), &state.snapshot().unwrap()).unwrap(),
        state
    );
}

#[test]
fn r2_action_surge_leaves_second_wind_bonus_action_and_healing_independent() {
    let mut state = battle(vec![
        melee(1, Cell(1, 1)).at_fighter_level(5).unwrap(),
        CombatantSpec::creature(3, "ogre", Cell(6, 1), 0).unwrap(),
    ]);
    state.actor_mut(1).unwrap().health.hit_points = 39;
    let before = state.actor(1).unwrap().clone();
    let surged = step(&state, 1, Action::ActionSurge, &[]).battle;
    let after = surged.actor(1).unwrap();
    assert_eq!(
        after.spec().second_wind_uses,
        before.spec().second_wind_uses
    );
    assert!(!after.budget().bonus_action_used);
    assert_eq!(after.health(), before.health());
    assert_eq!(after.additional_actions(), 1);
    let healed = step(&surged, 1, Action::SecondWind, &[(10, 3)]).battle;
    let after = healed.actor(1).unwrap();
    assert_eq!(after.spec().second_wind_uses, 2);
    assert!(after.budget().bonus_action_used);
    assert_eq!(after.health().hit_points, 47);
    assert_eq!(after.additional_actions(), 1);
}

#[test]
fn r2_wight_critical_doubles_both_types_and_bow_is_not_a_thrown_object() {
    let state = battle(vec![
        melee(1, Cell(1, 2)).at_fighter_level(5).unwrap(),
        CombatantSpec::creature(3, "wight", Cell(8, 2), 0).unwrap(),
    ]);
    let state = next_turn(state, 3);
    let result = step(
        &state,
        3,
        strike(1, Weapon::NecroticBow, true),
        &[(20, 20), (8, 2), (8, 3), (8, 4), (8, 5)],
    );
    assert_eq!(
        (result.attacks[0].damage, result.attacks[0].necrotic_damage),
        (7, 9)
    );
    assert!(result.attacks[0].critical);
    assert_eq!(
        result
            .battle
            .actor(3)
            .unwrap()
            .weapon_count(Weapon::NecroticBow),
        1
    );
    assert!(result.battle.actor(3).unwrap().dropped.is_empty());
    assert_eq!(result.battle.actor(3).unwrap().arrows(), 0);
    assert_eq!(
        Battle::from_snapshot(
            result.battle.map().clone(),
            &result.battle.snapshot().unwrap()
        )
        .unwrap(),
        result.battle
    );
}

#[test]
fn r2_sunlit_wight_group_initiative_uses_one_disadvantaged_source_check() {
    let src = source();
    let specs = vec![
        melee(1, Cell(1, 1)),
        CombatantSpec::creature(3, "wight", Cell(5, 1), 0).unwrap(),
        CombatantSpec::creature(4, "wight", Cell(6, 1), 0).unwrap(),
    ];
    let result = begin(
        map("open-court").with_lighting(Lighting::Sunlight),
        specs,
        binding(&src),
        &records(&src, &[(20, 10), (20, 18), (20, 3)]),
    )
    .unwrap();
    assert_eq!(result.draws.len(), 3);
    assert_eq!(
        result.battle.initiative_policy(),
        "identical-monster-groups"
    );
    assert_eq!(result.battle.development_initiative(3), Some(7));
    assert_eq!(result.battle.development_initiative(4), Some(7));
}

fn species_hero(
    id: u32,
    species: &'static str,
    extra: &[(&'static str, &'static str)],
    cell: Cell,
) -> CombatantSpec {
    let mut choices = archer_choices();
    for (key, value) in &mut choices {
        *value = match *key {
            "species" => species,
            "array" => "stalwart",
            "equipment" => "fighter-a-soldier-a",
            "style-masteries" => "defense-greatsword-flail-javelin",
            _ => *value,
        };
    }
    choices.extend(extra.iter().copied());
    let character = crate::local_journey::validate_character(
        &crate::local_journey::CreationRules::srd_521(),
        "Brynn",
        &choices,
    )
    .unwrap();
    CombatantSpec::melee_hero(&character, id, cell).unwrap()
}
fn species_step(
    battle: &Battle,
    actor: u32,
    action: SpeciesAction,
    faces: &[(u32, u32)],
) -> Resolution {
    let source = source();
    resolve_species(
        battle,
        actor,
        &action,
        binding(&source),
        &records(&source, faces),
    )
    .unwrap()
}
#[test]
fn srd_521_all_nine_species_reach_tactical_validation_and_keep_their_derived_profiles() {
    for (species, extra, hp, speed, size) in [
        (
            "dragonborn",
            vec![("draconic-ancestry", "red")],
            12,
            30,
            Size::Medium,
        ),
        ("dwarf", vec![], 13, 30, Size::Medium),
        (
            "elf",
            vec![
                ("elven-lineage", "wood-elf"),
                ("keen-senses", "insight"),
                ("species-spellcasting", "wisdom"),
            ],
            12,
            35,
            Size::Medium,
        ),
        (
            "gnome",
            vec![
                ("gnomish-lineage", "forest-gnome"),
                ("species-spellcasting", "wisdom"),
            ],
            12,
            30,
            Size::Small,
        ),
        (
            "goliath",
            vec![("giant-ancestry", "fire")],
            12,
            35,
            Size::Medium,
        ),
        ("halfling", vec![], 12, 30, Size::Small),
        (
            "human",
            vec![
                ("species-size", "small"),
                ("human-skill", "arcana"),
                ("versatile", "tough"),
            ],
            14,
            30,
            Size::Small,
        ),
        ("orc", vec![], 12, 30, Size::Medium),
        (
            "tiefling",
            vec![
                ("fiendish-legacy", "infernal"),
                ("species-size", "medium"),
                ("species-spellcasting", "charisma"),
            ],
            12,
            30,
            Size::Medium,
        ),
    ] {
        let spec = species_hero(1, species, &extra, Cell(1, 1));
        assert_eq!(
            (spec.hit_points, spec.speed_ft, spec.size),
            (hp, speed, size),
            "{species}"
        );
        let state = battle(vec![
            spec.clone(),
            CombatantSpec::creature(3, "bandit", Cell(5, 1), 0).unwrap(),
        ]);
        assert_eq!(
            Battle::from_snapshot(map("open-court"), &state.snapshot().unwrap()).unwrap(),
            state
        );
        assert!(spec.at_fighter_level(5).is_ok(), "{species}");
    }
}
#[test]
fn a_level_five_tough_human_matches_the_shared_maximum_hit_points() {
    let human = species_hero(
        1,
        "human",
        &[
            ("species-size", "small"),
            ("human-skill", "arcana"),
            ("versatile", "tough"),
        ],
        Cell(1, 1),
    );
    let leveled = human.at_fighter_level(5).unwrap();
    // SRD 5.2.1 p. 87: Tough adds 2 per level on top of the Fighter's d10 and fixed 6.
    assert_eq!(
        u32::from(leveled.hit_points),
        crate::local_journey::maximum_hit_points(5, 10, leveled.abilities[2], 2)
    );
    assert_eq!(leveled.fighter_level().map(|level| level.level()), Some(5));
}
#[test]
fn srd_521_halfling_nimbleness_crosses_larger_enemy_but_cannot_stop_or_cross_equal_size() {
    let halfling = species_hero(1, "halfling", &[], Cell(1, 1));
    let state = battle(vec![
        halfling,
        CombatantSpec::creature(3, "bandit", Cell(2, 1), 0).unwrap(),
    ]);
    assert_eq!(state.validate_path(1, &[Cell(2, 1), Cell(3, 1)]), Ok(15));
    assert_eq!(
        state.validate_path(1, &[Cell(2, 1)]),
        Err(RuleError::Occupied)
    );
    let state = battle(vec![
        species_hero(1, "halfling", &[], Cell(1, 1)),
        CombatantSpec::creature(3, "goblin-warrior", Cell(2, 1), 0).unwrap(),
    ]);
    assert_eq!(
        state.validate_path(1, &[Cell(2, 1), Cell(3, 1)]),
        Err(RuleError::Occupied)
    );
}
#[test]
fn srd_521_dragonborn_breath_saves_include_allies_resistance_and_do_not_use_attack_roll_criticals()
{
    let dragon = species_hero(1, "dragonborn", &[("draconic-ancestry", "red")], Cell(1, 1));
    let ally = species_hero(
        2,
        "tiefling",
        &[
            ("fiendish-legacy", "infernal"),
            ("species-size", "small"),
            ("species-spellcasting", "wisdom"),
        ],
        Cell(2, 1),
    );
    let state = battle(vec![
        dragon,
        ally,
        CombatantSpec::creature(3, "bandit", Cell(3, 1), 0).unwrap(),
    ]);
    let result = species_step(
        &state,
        1,
        SpeciesAction::BreathWeapon {
            shape: BreathShape::Line,
            direction: BreathDirection { column: 1, row: 0 },
            spend: AttackSpend::Action,
        },
        &[(10, 9), (20, 1), (20, 20)],
    );
    assert_eq!(result.saving_throws.len(), 2);
    assert_eq!(
        (
            result.saving_throws[0].dc,
            result.saving_throws[0].modifier,
            result.saving_throws[0].damage
        ),
        (12, 1, 4)
    );
    assert!(!result.saving_throws[0].success);
    assert!(result.saving_throws[1].success);
    assert_eq!(result.saving_throws[1].damage, 4);
    assert_eq!(result.battle.actor(1).unwrap().species_uses_remaining(), 1);
    assert_eq!(result.battle.actor(2).unwrap().health().hit_points, 8);
    assert_eq!(result.battle.actor(3).unwrap().health().hit_points, 7);
    rejects(&result.battle, 1, Action::Dodge, RuleError::ActionSpent);
    assert_eq!(
        Battle::from_snapshot(map("open-court"), &result.battle.snapshot().unwrap()).unwrap(),
        result.battle
    );
}
#[test]
fn srd_521_dragonborn_breath_shapes_and_extra_attack_and_exhaustion_follow_source() {
    let dragon = species_hero(
        1,
        "dragonborn",
        &[("draconic-ancestry", "blue")],
        Cell(1, 2),
    )
    .at_fighter_level(5)
    .unwrap();
    let state = battle(vec![
        dragon,
        CombatantSpec::creature(3, "bandit", Cell(3, 3), 0).unwrap(),
        CombatantSpec::creature(4, "bandit", Cell(5, 2), 0).unwrap(),
    ]);
    let action = SpeciesAction::BreathWeapon {
        shape: BreathShape::Cone,
        direction: BreathDirection { column: 1, row: 0 },
        spend: AttackSpend::Action,
    };
    let result = species_step(&state, 1, action.clone(), &[(10, 2), (10, 3), (20, 1)]);
    assert_eq!(
        result
            .saving_throws
            .iter()
            .map(|save| save.target)
            .collect::<Vec<_>>(),
        [3]
    );
    assert_eq!(result.battle.actor(1).unwrap().extra_attacks, 1);
    let state = result.battle.with_hero_species_uses(1, 0).unwrap();
    let src = source();
    assert_eq!(
        resolve_species(&state, 1, &action, binding(&src), &[]),
        Err(RuleError::RecoveryUnavailable)
    );
}
#[test]
fn srd_521_goliath_fire_frost_and_hill_are_optional_spend_only_on_damage_and_keep_expiry() {
    for ancestry in ["fire", "frost", "hill"] {
        let state = battle(vec![
            species_hero(1, "goliath", &[("giant-ancestry", ancestry)], Cell(1, 1)),
            CombatantSpec::creature(3, "ogre", Cell(2, 1), 0).unwrap(),
        ]);
        let Action::Attack {
            target,
            weapon,
            spend,
        } = strike(3, Weapon::Greatsword, false)
        else {
            panic!("attack")
        };
        let action = SpeciesAction::AncestryAttack {
            target,
            weapon,
            spend,
        };
        let miss = species_step(&state, 1, action.clone(), &[(20, 2)]);
        assert_eq!(miss.battle.actor(1).unwrap().species_uses_remaining(), 2);
        let mut faces = vec![(20, 15), (6, 2), (6, 3)];
        if ancestry == "fire" {
            faces.push((10, 7));
        }
        if ancestry == "frost" {
            faces.push((6, 4));
        }
        let result = species_step(&state, 1, action, &faces);
        assert_eq!(result.battle.actor(1).unwrap().species_uses_remaining(), 1);
        let foe = result.battle.actor(3).unwrap();
        assert_eq!(foe.health().prone, ancestry == "hill");
        if ancestry == "frost" {
            assert_eq!(foe.speed_ft(), 30);
        }
        if ancestry == "fire" {
            assert_eq!(result.attacks[0].ancestry_damage, 7);
        }
        let restored =
            Battle::from_snapshot(map("open-court"), &result.battle.snapshot().unwrap()).unwrap();
        assert_eq!(restored, result.battle);
        let next = next_turn(result.battle, 1);
        assert_eq!(next.actor(3).unwrap().speed_ft(), 40);
    }
}
#[test]
fn srd_521_goliath_stone_and_storm_offer_owner_reactions_and_snapshot_pending_damage() {
    for ancestry in ["stone", "storm"] {
        let state = battle(vec![
            species_hero(1, "goliath", &[("giant-ancestry", ancestry)], Cell(1, 1)),
            CombatantSpec::creature(3, "bandit", Cell(2, 1), 0).unwrap(),
        ]);
        let state = next_turn(state, 3);
        let result = step(
            &state,
            3,
            strike(1, Weapon::Scimitar, false),
            &[(20, 18), (6, 5)],
        );
        assert_eq!(
            result.battle.species_reaction_offer().map(|offer| offer.0),
            Some(1)
        );
        let state =
            Battle::from_snapshot(map("open-court"), &result.battle.snapshot().unwrap()).unwrap();
        rejects(&state, 3, Action::EndTurn, RuleError::PendingReaction);
        let src = source();
        assert_eq!(
            resolve_species(
                &state,
                3,
                &SpeciesAction::Reaction { use_trait: true },
                binding(&src),
                &[]
            ),
            Err(RuleError::InvalidReaction)
        );
        let result = species_step(
            &state,
            1,
            SpeciesAction::Reaction { use_trait: true },
            &[if ancestry == "stone" { (12, 3) } else { (8, 4) }],
        );
        assert!(result.battle.species_reaction_offer().is_none());
        assert_eq!(result.battle.actor(1).unwrap().species_uses_remaining(), 1);
        assert!(result.battle.actor(1).unwrap().budget().reaction_used);
        assert_eq!(
            result.battle.actor(1).unwrap().health().hit_points,
            if ancestry == "stone" { 11 } else { 6 }
        );
        assert_eq!(
            result.battle.actor(3).unwrap().health().hit_points,
            if ancestry == "stone" { 11 } else { 7 }
        );
        let declined = species_step(&state, 1, SpeciesAction::Reaction { use_trait: false }, &[]);
        assert_eq!(
            declined.battle.actor(1).unwrap().species_uses_remaining(),
            2
        );
        assert!(!declined.battle.actor(1).unwrap().budget().reaction_used);
    }
}
#[test]
fn srd_521_species_uses_and_orc_endurance_survive_encounters_and_recharge_only_on_long_rest() {
    for (species, extra) in [
        ("orc", vec![]),
        ("dragonborn", vec![("draconic-ancestry", "black")]),
        ("goliath", vec![("giant-ancestry", "stone")]),
    ] {
        let mut state = battle(vec![
            species_hero(1, species, &extra, Cell(1, 1)),
            CombatantSpec::creature(3, "bandit", Cell(2, 1), 0).unwrap(),
        ]);
        state.actor_mut(3).unwrap().health.dead = true;
        state.actor_mut(3).unwrap().health.hit_points = 0;
        if species == "orc" {
            state = state.with_hero_feature_resources(1, 10, false).unwrap();
        } else {
            state = state.with_hero_species_uses(1, 0).unwrap();
        }
        let state = step(
            &state,
            1,
            Action::ShortRest {
                elapsed_seconds: 3600,
            },
            &[],
        )
        .battle;
        assert!(
            !state
                .actor(1)
                .unwrap()
                .spec()
                .relentless_endurance_available
        );
        assert_eq!(state.actor(1).unwrap().species_uses_remaining(), 0);
        let state = step(
            &state,
            1,
            Action::LongRest {
                elapsed_seconds: 28800,
            },
            &[],
        )
        .battle;
        if species == "orc" {
            assert!(
                state
                    .actor(1)
                    .unwrap()
                    .spec()
                    .relentless_endurance_available
            );
        } else {
            assert_eq!(state.actor(1).unwrap().species_uses_remaining(), 2);
        }
    }
}

#[test]
fn srd_521_goliath_critical_hit_doubles_added_attack_damage_dice() {
    let state = battle(vec![
        species_hero(1, "goliath", &[("giant-ancestry", "fire")], Cell(1, 1)),
        CombatantSpec::creature(3, "ogre", Cell(2, 1), 0).unwrap(),
    ]);
    let Action::Attack {
        target,
        weapon,
        spend,
    } = strike(3, Weapon::Greatsword, false)
    else {
        panic!("attack")
    };
    let result = species_step(
        &state,
        1,
        SpeciesAction::AncestryAttack {
            target,
            weapon,
            spend,
        },
        &[(20, 20), (6, 2), (6, 3), (6, 2), (6, 3), (10, 4), (10, 5)],
    );
    assert!(result.attacks[0].critical);
    assert_eq!(result.attacks[0].ancestry_damage, 9);
    assert_eq!(result.battle.actor(1).unwrap().species_uses_remaining(), 1);
}
#[test]
fn srd_521_wight_breath_save_uses_published_dexterity_save_not_initiative_modifier() {
    let state = battle(vec![
        species_hero(1, "dragonborn", &[("draconic-ancestry", "red")], Cell(1, 1)),
        CombatantSpec::creature(3, "wight", Cell(2, 1), 0).unwrap(),
    ]);
    let result = species_step(
        &state,
        1,
        SpeciesAction::BreathWeapon {
            shape: BreathShape::Line,
            direction: BreathDirection { column: 1, row: 0 },
            spend: AttackSpend::Action,
        },
        &[(10, 7), (20, 10)],
    );
    assert_eq!(result.saving_throws[0].modifier, 2);
    assert!(result.saving_throws[0].success);
    assert_eq!(result.saving_throws[0].damage, 3);
}
#[test]
fn srd_521_pending_stone_reaction_on_opportunity_attack_resumes_the_saved_move() {
    let state = battle(vec![
        species_hero(1, "goliath", &[("giant-ancestry", "stone")], Cell(1, 1)),
        CombatantSpec::creature(3, "bandit", Cell(2, 1), 0).unwrap(),
    ]);
    let moved = step(&state, 1, Action::Move(vec![Cell(0, 1)]), &[]);
    assert_eq!(moved.battle.reaction_offers(), [3]);
    let Action::Attack { weapon, .. } = strike(1, Weapon::Scimitar, false) else {
        panic!("attack")
    };
    let attacked = step(
        &moved.battle,
        3,
        Action::Opportunity {
            attack: Some(weapon),
        },
        &[(20, 18), (6, 4)],
    );
    assert!(attacked.battle.species_reaction_offer().is_some());
    let restored =
        Battle::from_snapshot(map("open-court"), &attacked.battle.snapshot().unwrap()).unwrap();
    let reacted = species_step(
        &restored,
        1,
        SpeciesAction::Reaction { use_trait: true },
        &[(12, 5)],
    );
    assert_eq!(reacted.battle.actor(1).unwrap().cell(), Cell(0, 1));
    assert_eq!(reacted.battle.actor(1).unwrap().health().hit_points, 12);
    assert!(reacted.battle.reaction_offers().is_empty());
}

#[test]
fn srd_521_dodge_retains_dexterity_save_advantage_when_breath_source_is_unseen() {
    let mut state = battle_with_map(
        map("open-court").with_lighting(Lighting::Darkness),
        vec![
            species_hero(1, "dragonborn", &[("draconic-ancestry", "red")], Cell(1, 1)),
            species_hero(
                2,
                "human",
                &[
                    ("species-size", "medium"),
                    ("human-skill", "arcana"),
                    ("versatile", "tough"),
                ],
                Cell(2, 1),
            ),
            CombatantSpec::creature(3, "bandit", Cell(9, 1), 0).unwrap(),
        ],
    );
    state.actor_mut(2).unwrap().budget.dodging = true;
    state.actor_mut(2).unwrap().budget.action_used = true;
    assert!(!state.can_see_actor(state.actor(2).unwrap(), state.actor(1).unwrap()));
    let result = species_step(
        &state,
        1,
        SpeciesAction::BreathWeapon {
            shape: BreathShape::Line,
            direction: BreathDirection { column: 1, row: 0 },
            spend: AttackSpend::Action,
        },
        &[(10, 9), (20, 2), (20, 18)],
    );
    assert_eq!(result.saving_throws[0].die, 18);
    assert!(result.saving_throws[0].success);
    assert_eq!(result.saving_throws[0].damage, 4);
}

#[test]
fn srd_521_breath_total_cover_uses_small_and_large_target_heights() {
    for (creature, height, expected) in [("goblin-warrior", 1.2, 0), ("ogre", 1.8, 1)] {
        let data = serde_json::json!({"schema":2,"grid":{"cols":10,"rows":5,"blocked":[]},
            "layouts":[{"id":"breath-height","obstacles":[{"id":"barrier","kind":"wall","cells":[[2,1]],"blocks_movement":false,"cover":"total","height_m":height}],
            "difficult":[],"spawns":{"heroes":[[1,1]],"enemies":[[3,1]]}}]});
        let map = BattleMap::from_json(data.to_string().as_bytes(), "breath-height").unwrap();
        let state = battle_with_map(
            map,
            vec![
                species_hero(1, "dragonborn", &[("draconic-ancestry", "red")], Cell(1, 1)),
                CombatantSpec::creature(3, creature, Cell(3, 1), 0).unwrap(),
            ],
        );
        let faces = if expected == 0 {
            vec![(10, 7)]
        } else {
            vec![(10, 7), (20, 10)]
        };
        let result = species_step(
            &state,
            1,
            SpeciesAction::BreathWeapon {
                shape: BreathShape::Line,
                direction: BreathDirection { column: 1, row: 0 },
                spend: AttackSpend::Action,
            },
            &faces,
        );
        assert_eq!(result.saving_throws.len(), expected, "{creature}");
        if expected == 1 {
            assert_eq!(result.saving_throws[0].modifier, 1);
        }
    }
}

#[test]
fn srd_521_species_acid_and_fire_inhibit_troll_regeneration_for_exactly_one_turn() {
    for colour in ["black", "red"] {
        let mut state = battle(vec![
            species_hero(
                1,
                "dragonborn",
                &[("draconic-ancestry", colour)],
                Cell(1, 1),
            ),
            CombatantSpec::creature(3, "troll", Cell(3, 1), 0).unwrap(),
        ]);
        state.actor_mut(3).unwrap().health.hit_points = 20;
        let result = species_step(
            &state,
            1,
            SpeciesAction::BreathWeapon {
                shape: BreathShape::Line,
                direction: BreathDirection { column: 1, row: 0 },
                spend: AttackSpend::Action,
            },
            &[(10, 7), (20, 1)],
        );
        let state =
            Battle::from_snapshot(map("open-court"), &result.battle.snapshot().unwrap()).unwrap();
        let state = next_turn(state, 3);
        assert_eq!(state.actor(3).unwrap().health().hit_points, 13, "{colour}");
        let state = next_turn(state, 1);
        let state = next_turn(state, 3);
        assert_eq!(state.actor(3).unwrap().health().hit_points, 28, "{colour}");
    }
}
#[test]
fn srd_521_troll_dies_at_zero_on_inhibited_turn_and_cold_does_not_inhibit() {
    for colour in ["red", "black", "silver"] {
        let mut state = battle(vec![
            species_hero(
                1,
                "dragonborn",
                &[("draconic-ancestry", colour)],
                Cell(1, 1),
            ),
            CombatantSpec::creature(3, "troll", Cell(3, 1), 0).unwrap(),
        ]);
        state.actor_mut(3).unwrap().health.hit_points = 1;
        let result = species_step(
            &state,
            1,
            SpeciesAction::BreathWeapon {
                shape: BreathShape::Line,
                direction: BreathDirection { column: 1, row: 0 },
                spend: AttackSpend::Action,
            },
            &[(10, 7), (20, 1)],
        );
        assert_eq!(result.battle.actor(3).unwrap().health().hit_points, 0);
        assert!(!result.battle.actor(3).unwrap().health().dead);
        let state = next_turn(result.battle, 3);
        assert_eq!(state.actor(3).unwrap().health().dead, colour != "silver");
        assert_eq!(
            state.actor(3).unwrap().health().hit_points,
            if colour == "silver" { 15 } else { 0 }
        );
    }
}
#[test]
fn srd_521_goliath_fire_inhibits_troll_regeneration_on_a_damaging_hit() {
    let mut state = battle(vec![
        species_hero(1, "goliath", &[("giant-ancestry", "fire")], Cell(1, 1)),
        CombatantSpec::creature(3, "troll", Cell(2, 1), 0).unwrap(),
    ]);
    state.actor_mut(3).unwrap().health.hit_points = 20;
    let Action::Attack {
        target,
        weapon,
        spend,
    } = strike(3, Weapon::Greatsword, false)
    else {
        panic!("attack")
    };
    let result = species_step(
        &state,
        1,
        SpeciesAction::AncestryAttack {
            target,
            weapon,
            spend,
        },
        &[(20, 15), (6, 2), (6, 3), (10, 4)],
    );
    let state = next_turn(result.battle, 3);
    assert_eq!(state.actor(3).unwrap().health().hit_points, 8);
}

#[test]
fn srd_521_goliath_ancestry_can_be_chosen_for_an_off_turn_opportunity_attack() {
    let state = battle(vec![
        species_hero(1, "goliath", &[("giant-ancestry", "fire")], Cell(1, 1)),
        CombatantSpec::creature(3, "bandit", Cell(2, 1), 0).unwrap(),
    ]);
    let state = next_turn(state, 3);
    let moved = step(&state, 3, Action::Move(vec![Cell(3, 1)]), &[]);
    assert_eq!(moved.battle.reaction_offers(), [1]);
    let Action::Attack { weapon, .. } = strike(3, Weapon::Greatsword, false) else {
        panic!("attack")
    };
    let result = species_step(
        &moved.battle,
        1,
        SpeciesAction::AncestryOpportunity { attack: weapon },
        &[(20, 15), (6, 1), (6, 1), (10, 2)],
    );
    assert_eq!(result.attacks[0].ancestry_damage, 2);
    assert_eq!(result.battle.actor(1).unwrap().species_uses_remaining(), 1);
    assert!(result.battle.actor(1).unwrap().budget().reaction_used);
    assert_eq!(result.battle.actor(3).unwrap().cell(), Cell(3, 1));
}

#[test]
fn srd_521_resumed_damage_uses_wide_zombie_save_arithmetic_at_the_snapshot_boundary() {
    let state = battle(vec![
        species_hero(1, "goliath", &[("giant-ancestry", "stone")], Cell(1, 1)),
        CombatantSpec::creature(3, "bandit", Cell(2, 1), 0).unwrap(),
        CombatantSpec::creature(4, "zombie", Cell(4, 1), 0).unwrap(),
    ]);
    let state = next_turn(state, 3);
    let hit = step(
        &state,
        3,
        strike(1, Weapon::Scimitar, false),
        &[(20, 18), (6, 4)],
    );
    let mut snapshot: serde_json::Value =
        serde_json::from_slice(&hit.battle.snapshot().unwrap()).unwrap();
    snapshot["species_damage"].as_array_mut().unwrap().push(serde_json::json!({"source":3,"target":4,"amount":u16::MAX,"critical":false,"knockout":false,"applied":false}));
    let restored =
        Battle::from_snapshot(map("open-court"), &serde_json::to_vec(&snapshot).unwrap()).unwrap();
    let result = species_step(
        &restored,
        1,
        SpeciesAction::Reaction { use_trait: false },
        &[(20, 20)],
    );
    assert!(result.battle.actor(4).unwrap().health().dead);
    assert_eq!(result.battle.actor(4).unwrap().health().hit_points, 0);
}

#[test]
fn pending_reaction_is_none_without_a_window_or_species_reaction() {
    let state = battle(vec![
        melee(1, Cell(1, 1)),
        CombatantSpec::creature(3, "bandit", Cell(2, 1), 0).unwrap(),
    ]);
    assert_eq!(state.pending_reaction(), None);
}

#[test]
fn pending_reaction_falls_back_to_the_first_opportunity_reactor() {
    let mut state = battle(vec![
        melee(1, Cell(1, 1)),
        CombatantSpec::creature(3, "bandit", Cell(2, 1), 0).unwrap(),
        CombatantSpec::creature(4, "bandit", Cell(2, 2), 0).unwrap(),
    ]);
    state.pending = Some(super::state::PendingMove {
        actor: 1,
        path: vec![Cell(1, 0)],
        next: 0,
        eligible: vec![4, 3],
    });
    assert_eq!(state.reaction_offers(), &[4, 3]);
    assert_eq!(
        state.pending_reaction(),
        Some(PendingReaction::Opportunity { reactor: 4 })
    );
}

#[test]
fn a_species_reaction_takes_precedence_over_an_opportunity_window() {
    let mut state = battle(vec![
        species_hero(1, "goliath", &[("giant-ancestry", "stone")], Cell(1, 1)),
        CombatantSpec::creature(3, "bandit", Cell(2, 1), 0).unwrap(),
    ]);
    state.pending = Some(super::state::PendingMove {
        actor: 1,
        path: vec![Cell(0, 1)],
        next: 0,
        eligible: vec![3],
    });
    state.species_damage.push(super::species::SpeciesDamage {
        source: 3,
        target: 1,
        amount: 5,
        critical: false,
        knockout: false,
        applied: false,
    });
    assert_eq!(state.reaction_offers(), &[3]);
    assert_eq!(
        state.pending_reaction(),
        Some(PendingReaction::Species {
            hero: 1,
            kind: SpeciesReaction::StoneEndurance
        })
    );
}
