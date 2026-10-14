#[path = "support/rules_audit.rs"]
mod rules_audit;

use df_combat::tactical::propose_latest;
use df_model::checkpoint::{ActualDraw, ResolutionId, RuleReference, WindowId};
use df_rules::tactical::{
    Action, BattleMap, Cell, CombatantSpec, DrawBinding, RuleError, Weapon, begin, resolve,
    validate_archer,
};
use df_types::{OperationId, RevisionLabel};
use rules_audit::{FightAudit, Grid, judge_attack};

fn source() -> RuleReference {
    RuleReference {
        catalog: RevisionLabel::new(Some("fixture-srd-5.2.1")).unwrap(),
        source: RevisionLabel::new(Some("pdf-8974902d")).unwrap(),
        entry: RevisionLabel::new(Some("tactical")).unwrap(),
        clause: RevisionLabel::new(Some("rul-17-goblin-shield")).unwrap(),
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

fn record(source: &RuleReference, ordinal: u32, sides: u32, value: u32) -> ActualDraw {
    ActualDraw {
        operation: OperationId::from_bytes(&[1; 16]).unwrap(),
        resolution: ResolutionId::from_bytes(&[2; 16]).unwrap(),
        window: WindowId::from_bytes(&[3; 16]).unwrap(),
        ordinal,
        sides,
        value,
        source: source.clone(),
    }
}

fn archer(id: u32, cell: Cell) -> CombatantSpec {
    let choices = [
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
    ];
    validate_archer("Brynn", &choices)
        .unwrap()
        .combatant(id, cell)
}

#[test]
fn latest_goblin_policy_uses_shortbow_with_its_shield_and_ac_15() {
    // SRD 5.2.1 p. 290: Goblin Warrior AC 15, Shield in Gear, Shortbow Action.
    let json: &[u8] = br#"{"schema":2,"grid":{"cols":8,"rows":3,"blocked":[]},"layouts":[{"id":"flat","obstacles":[],"difficult":[],"spawns":{"heroes":[[7,1]],"enemies":[[2,1]]}}]}"#;
    let grid = Grid::from_scene(json, "flat", false);
    let map = BattleMap::from_json(json, "flat").unwrap();
    let specs = vec![
        archer(1, Cell(7, 1)),
        CombatantSpec::creature(10, "goblin-warrior", Cell(2, 1), 20).unwrap(),
    ];
    let source = source();
    let initiative = [record(&source, 0, 20, 1), record(&source, 1, 20, 20)];
    let battle = begin(map, specs.clone(), binding(&source), &initiative)
        .unwrap()
        .battle;
    assert_eq!(battle.active_actor(), Some(10));
    assert!(battle.actor(10).unwrap().shield_equipped());
    assert_eq!(battle.actor(10).unwrap().spec().armor_class, 15);

    let action = propose_latest(&battle, 10).unwrap().unwrap();
    let Action::Attack { target, weapon, .. } = &action else {
        panic!("the Goblin Warrior should attack with its listed Shortbow");
    };
    assert_eq!(weapon.weapon, Weapon::Shortbow);
    assert!(weapon.ranged);
    assert_eq!(judge_attack(&battle, &grid, 10, *target, weapon), Ok(()));

    let metadata: Vec<_> = specs
        .iter()
        .map(|spec| (spec.id, spec.kind, spec.abilities[1]))
        .collect();
    let mut auditor = FightAudit::begin(&battle, &grid, &metadata, &initiative);
    let mut draws = Vec::new();
    let resolution = loop {
        match resolve(&battle, 10, &action, binding(&source), &draws) {
            Err(RuleError::MissingDraw { sides }) => {
                let ordinal = draws.len() as u32;
                let value = if sides == 20 { 13 } else { 1 };
                draws.push(record(&source, ordinal, sides, value));
            }
            Ok(resolution) => break resolution,
            Err(error) => panic!("the listed Shortbow attack should resolve: {error:?}"),
        }
    };
    auditor.accepted(&battle, 10, &action, &resolution);
    assert!(
        auditor.audit.violations.is_empty(),
        "{:?}",
        auditor.audit.violations
    );
    assert!(resolution.attacks[0].hit);
    assert!(resolution.battle.actor(10).unwrap().shield_equipped());
    assert_eq!(resolution.battle.actor(10).unwrap().spec().armor_class, 15);
}
