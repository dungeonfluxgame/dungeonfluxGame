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

fn encounter(hero_weapons: Vec<Weapon>) -> Battle {
    let map = BattleMap::from_json(MAP, "swap-audit").unwrap();
    let hero = CombatantSpec {
        id: 1,
        side: Side::Heroes,
        cell: Cell(1, 1),
        size: Size::Medium,
        kind: CreatureKind::Hero,
        abilities: [16, 14, 15, 10, 12, 8],
        armor_class: 17,
        hit_points: 13,
        speed_ft: 30,
        weapons: hero_weapons,
        masteries: Vec::new(),
        arrows: 20,
        bolts: 0,
        archery: false,
        savage_attacker: false,
        second_wind_uses: 2,
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

// SRD 5.2.1 p177 allows one weapon change during the Attack action; p13 grants one free object
// interaction on the turn. Here, LightBonus also equips another weapon and drops the original
// without using that interaction; a later opportunity attack changes the hand again off-turn.
#[test]
fn srd_p13_p177_attacks_and_opportunity_reaction_change_weapons_without_interaction() {
    let battle = encounter(vec![
        Weapon::Longbow,
        Weapon::Scimitar,
        Weapon::Shortsword,
        Weapon::Javelin,
    ]);
    let attack = Action::Attack {
        target: 2,
        weapon: use_weapon(Weapon::Scimitar, false, 1),
        spend: AttackSpend::Action,
    };
    let battle = step(&battle, 1, &attack, 2, &[(20, 15), (6, 3)]).battle;
    assert_eq!(
        battle.actor(1).unwrap().held_weapons(),
        &[Weapon::Longbow, Weapon::Scimitar]
    );
    assert!(!battle.actor(1).unwrap().budget().object_interaction_used);
    let light_attack = Action::Attack {
        target: 2,
        weapon: use_weapon(Weapon::Shortsword, false, 1),
        spend: AttackSpend::LightBonus,
    };
    let battle = step(&battle, 1, &light_attack, 3, &[(20, 15), (6, 3)]).battle;
    assert_eq!(
        battle.actor(1).unwrap().held_weapons(),
        &[Weapon::Scimitar, Weapon::Shortsword]
    );
    assert!(!battle.actor(1).unwrap().budget().object_interaction_used);

    let battle = step(&battle, 1, &Action::EndTurn, 4, &[]).battle;
    let moved = step(&battle, 2, &Action::Move(vec![Cell(3, 1)]), 5, &[]).battle;
    assert_eq!(moved.reaction_offers(), &[1]);
    let opportunity = Action::Opportunity {
        attack: Some(use_weapon(Weapon::Javelin, false, 1)),
    };
    let after_reaction = step(&moved, 1, &opportunity, 6, &[(20, 15), (6, 3)]).battle;
    assert_eq!(
        after_reaction.actor(1).unwrap().held_weapons(),
        &[Weapon::Shortsword, Weapon::Javelin]
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
fn srd_p177_a_shortbow_attack_replaces_the_actual_in_hand_weapon() {
    let battle = encounter(vec![Weapon::Greatsword, Weapon::Shortbow]);
    let attack = Action::Attack {
        target: 2,
        weapon: use_weapon(Weapon::Shortbow, true, 2),
        spend: AttackSpend::Action,
    };
    let result = step(&battle, 1, &attack, 7, &[(20, 15), (6, 3)]);
    assert_eq!(
        result.battle.actor(1).unwrap().held_weapons(),
        &[Weapon::Shortbow]
    );
}
