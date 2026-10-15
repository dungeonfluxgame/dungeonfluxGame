use df_model::checkpoint::{ActualDraw, ResolutionId, RuleReference, WindowId};
use df_rules::tactical::{
    self, Action, AttackSpend, AttackUse, Battle, BattleMap, Cell, CombatantSpec, CreatureKind,
    DrawBinding, Side, Size, Weapon,
};
use df_types::{OperationId, RevisionLabel};

const MAP: &[u8] = br#"{"schema":2,"grid":{"cols":5,"rows":5,"blocked":[]},"layouts":[{"id":"swap-audit","obstacles":[],"difficult":[],"spawns":{"heroes":[[1,1]],"enemies":[[2,1]]}}]}"#;

fn source() -> RuleReference {
    RuleReference {
        catalog: RevisionLabel::new(Some("srd-5.2.1")).unwrap(),
        source: RevisionLabel::new(Some("pdf-8974902d")).unwrap(),
        entry: RevisionLabel::new(Some("PLAY-T01")).unwrap(),
        clause: RevisionLabel::new(Some("weapon-swap-audit")).unwrap(),
    }
}

fn binding(source: &RuleReference, id: u8) -> DrawBinding<'_> {
    DrawBinding {
        operation: OperationId::from_bytes(&[id; 16]).unwrap(),
        resolution: ResolutionId::from_bytes(&[id.wrapping_add(1); 16]).unwrap(),
        window: WindowId::from_bytes(&[id.wrapping_add(2); 16]).unwrap(),
        source,
        next_ordinal: 0,
    }
}

fn draws(
    source: &RuleReference,
    binding: &DrawBinding<'_>,
    faces: &[(u32, u32)],
) -> Vec<ActualDraw> {
    faces
        .iter()
        .enumerate()
        .map(|(ordinal, &(sides, value))| ActualDraw {
            operation: binding.operation,
            resolution: binding.resolution,
            window: binding.window,
            source: source.clone(),
            ordinal: ordinal as u32,
            sides,
            value,
        })
        .collect()
}

fn encounter(archer: bool) -> Battle {
    let map = BattleMap::from_json(MAP, "swap-audit").unwrap();
    let (abilities, armor_class, weapons, masteries, archery) = if archer {
        (
            [13, 17, 15, 10, 12, 8],
            15,
            vec![Weapon::Longbow, Weapon::Scimitar, Weapon::Shortsword],
            vec![Weapon::Longbow, Weapon::Scimitar, Weapon::Shortsword],
            true,
        )
    } else {
        (
            [17, 13, 15, 10, 12, 8],
            17,
            vec![
                Weapon::Greatsword,
                Weapon::Flail,
                Weapon::Javelin,
                Weapon::Spear,
                Weapon::Shortbow,
            ],
            vec![Weapon::Greatsword, Weapon::Flail, Weapon::Javelin],
            false,
        )
    };
    let hero = CombatantSpec {
        id: 1,
        side: Side::Heroes,
        cell: Cell(1, 1),
        size: Size::Medium,
        kind: CreatureKind::Hero,
        abilities,
        armor_class,
        hit_points: 13,
        speed_ft: 30,
        weapons,
        masteries,
        arrows: 20,
        bolts: 0,
        archery,
        savage_attacker: true,
        second_wind_uses: 2,
        species: Some(df_rules::local_journey::HeroSpecies::Orc),
        healers_kit_uses: 0,
        relentless_endurance_available: true,
        shield: false,
    };
    let zombie = CombatantSpec::creature(2, "zombie", Cell(2, 1), 0).unwrap();
    let source = source();
    let binding = binding(&source, 1);
    let initiative = draws(&source, &binding, &[(20, 20), (20, 1)]);
    tactical::begin(map, vec![hero, zombie], binding, &initiative)
        .unwrap()
        .battle
}

fn use_weapon(weapon: Weapon, ranged: bool, hands: u8) -> AttackUse {
    AttackUse {
        weapon,
        ranged,
        hands,
        finesse_dexterity: false,
        knockout: false,
        savage_attacker: false,
        use_mastery: false,
    }
}

fn step(
    battle: &Battle,
    actor: u32,
    action: &Action,
    id: u8,
    faces: &[(u32, u32)],
) -> tactical::Resolution {
    let source = source();
    let binding = binding(&source, id);
    let draws = draws(&source, &binding, faces);
    tactical::resolve(battle, actor, action, binding, &draws).unwrap()
}

// SRD 5.2.1 p177 allows weapon changes for attacks in the Attack action; p13 grants one free
// object interaction on the turn. The Nick attack stays within the Attack action, and the later
// opportunity attack uses a weapon already in hand.
#[test]
fn srd_p13_p177_attack_action_nick_and_opportunity_reaction_need_no_interaction() {
    let battle = encounter(true);
    let attack = Action::Attack {
        target: 2,
        weapon: use_weapon(Weapon::Shortsword, false, 1),
        spend: AttackSpend::Action,
    };
    let battle = step(&battle, 1, &attack, 2, &[(20, 15), (6, 3)]).battle;
    assert_eq!(
        battle.actor(1).unwrap().held_weapons(),
        &[Weapon::Shortsword]
    );
    assert!(!battle.actor(1).unwrap().budget().object_interaction_used);
    let mut nick = use_weapon(Weapon::Scimitar, false, 1);
    nick.use_mastery = true;
    let nick_attack = Action::Attack {
        target: 2,
        weapon: nick,
        spend: AttackSpend::Nick,
    };
    let battle = step(&battle, 1, &nick_attack, 3, &[(20, 15), (6, 3)]).battle;
    assert_eq!(
        battle.actor(1).unwrap().held_weapons(),
        &[Weapon::Shortsword, Weapon::Scimitar]
    );
    assert!(!battle.actor(1).unwrap().budget().object_interaction_used);
    assert!(!battle.actor(1).unwrap().budget().bonus_action_used);

    let battle = step(&battle, 1, &Action::EndTurn, 4, &[]).battle;
    let moved = step(&battle, 2, &Action::Move(vec![Cell(3, 1)]), 5, &[]).battle;
    assert_eq!(moved.reaction_offers(), &[1]);
    let opportunity = Action::Opportunity {
        attack: Some(use_weapon(Weapon::Scimitar, false, 1)),
    };
    let after_reaction = step(&moved, 1, &opportunity, 6, &[(20, 15), (6, 3)]).battle;
    assert_eq!(
        after_reaction.actor(1).unwrap().held_weapons(),
        &[Weapon::Shortsword, Weapon::Scimitar]
    );
    assert!(
        !after_reaction
            .actor(1)
            .unwrap()
            .budget()
            .object_interaction_used
    );
}

#[test]
fn srd_p15_p177_opportunity_attack_uses_only_a_held_weapon() {
    let battle = encounter(false);
    let battle = step(&battle, 1, &Action::EndTurn, 8, &[]).battle;
    let moved = step(&battle, 2, &Action::Move(vec![Cell(3, 1)]), 9, &[]).battle;
    let source = source();
    let binding = binding(&source, 10);
    let draws = draws(&source, &binding, &[]);
    assert_eq!(
        tactical::resolve(
            &moved,
            1,
            &Action::Opportunity {
                attack: Some(use_weapon(Weapon::Javelin, false, 1)),
            },
            binding,
            &draws,
        )
        .unwrap_err(),
        tactical::RuleError::InvalidWeapon
    );
    let attack = Action::Opportunity {
        attack: Some(use_weapon(Weapon::Greatsword, false, 2)),
    };
    let after_reaction = step(&moved, 1, &attack, 11, &[(20, 15), (6, 3), (6, 3)]).battle;
    assert_eq!(
        after_reaction.actor(1).unwrap().held_weapons(),
        &[Weapon::Greatsword]
    );
}

#[test]
fn srd_p177_a_shortbow_attack_replaces_the_actual_in_hand_weapon() {
    let battle = encounter(false);
    let attack = Action::Attack {
        target: 2,
        weapon: use_weapon(Weapon::Shortbow, true, 2),
        spend: AttackSpend::Action,
    };
    let result = step(&battle, 1, &attack, 7, &[(20, 15), (20, 15), (6, 3)]);
    assert_eq!(
        result.battle.actor(1).unwrap().held_weapons(),
        &[Weapon::Shortbow]
    );
}
