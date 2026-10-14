use df_combat::{
    candidates::CandidateContext,
    ranking::{KnowledgeObserver, RankingObservation, TacticalKnowledge, UtilityPolicy},
    tactical::{TacticContext, choose, propose_latest as propose},
};
use df_model::checkpoint::{
    ActualDraw, Basis, CheckpointPins, ContentDigest, ContentPins, ContentReference, EntityId,
    LogicalTime, ResolutionId, RuleReference, RulesMode, RulesPins, WindowId,
};
use df_rules::tactical::{
    Action, AttackReason, AttackSpend, AttackUse, Battle, BattleMap, Cell, CombatantSpec, Cover,
    DrawBinding, RuleError, Side, Weapon, begin, resolve, validate_archer,
};
use df_types::{
    BuildIdentity, OperationId, RecoveryEpoch, RevisionLabel, RunId, SessionId, SessionRevision,
};
use std::collections::BTreeMap;

fn label(text: &str) -> RevisionLabel {
    RevisionLabel::new(Some(text)).unwrap()
}
fn source() -> RuleReference {
    RuleReference {
        catalog: label("fixture-srd-5.2.1"),
        source: label("pdf-8974902d"),
        entry: label("tactical"),
        clause: label("source-linked-simulation"),
    }
}
fn pins() -> CheckpointPins {
    CheckpointPins {
        rules: RulesPins {
            mode: RulesMode::DisclosedCustom,
            ruleset: label("fixture-tactical-centre-line"),
            catalog: label("fixture-srd-5.2.1"),
            catalog_digest: ContentDigest([1; 32]),
            source_manifest: label("retained-srd-manifest"),
            source_manifest_digest: ContentDigest([2; 32]),
            handler: label("PLAY-T01"),
            handler_digest: ContentDigest([3; 32]),
        },
        content: ContentPins {
            content: label("synthetic-fixtures"),
            content_digest: ContentDigest([4; 32]),
            package: label("tactical-fixtures"),
            package_digest: ContentDigest([5; 32]),
        },
        build: BuildIdentity::new(
            Some("63484e9-working-tree"),
            Some("tactical-native"),
            Some("unaffected-wasm"),
            Some("synthetic"),
            Some("fixtures-v2"),
        )
        .unwrap(),
    }
}
fn basis() -> Basis {
    Basis {
        session: SessionId::from_bytes(&[1; 16]).unwrap(),
        run: RunId::from_bytes(&[2; 16]).unwrap(),
        revision: SessionRevision::new(RecoveryEpoch::new(1).unwrap(), 0),
    }
}
fn choices(archer: bool) -> Vec<(&'static str, &'static str)> {
    vec![
        ("species", "dwarf"),
        ("class", "fighter-1"),
        ("background", "soldier"),
        ("array", if archer { "dexterous" } else { "stalwart" }),
        ("alignment", "neutral-good"),
        ("language-1", "dwarvish"),
        ("language-2", "goblin"),
        ("skill-1", "perception"),
        ("skill-2", "survival"),
        ("gaming-set", "dice"),
        (
            "equipment",
            if archer {
                "fighter-b-soldier-b"
            } else {
                "fighter-a-soldier-a"
            },
        ),
        (
            "style-masteries",
            if archer {
                "archery-longbow-scimitar-shortsword"
            } else {
                "defense-greatsword-flail-javelin"
            },
        ),
        ("allied-ties", "join-order"),
    ]
}
fn hero(id: u32, cell: Cell) -> CombatantSpec {
    if id == 3 {
        validate_archer("Brynn", &choices(true))
            .unwrap()
            .combatant(id, cell)
    } else {
        let built = df_rules::local_journey::validate_character(
            &df_rules::local_journey::CreationRules::srd_521(),
            "Brakka",
            &choices(false),
        )
        .unwrap();
        CombatantSpec::melee_hero(&built, id, cell).unwrap()
    }
}
// Test boundary only: same SplitMix64 stream as adventure/rng.rs, recorded before Rules.
struct Dice {
    state: u64,
    ordinal: u32,
}
impl Dice {
    fn record(&mut self, sides: u32, source: &RuleReference, ordinal: u32) -> ActualDraw {
        self.state = self.state.wrapping_add(0x9e3779b97f4a7c15);
        let mut value = self.state;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
        value ^= value >> 31;
        ActualDraw {
            operation: OperationId::from_bytes(&[1; 16]).unwrap(),
            resolution: ResolutionId::from_bytes(&[2; 16]).unwrap(),
            window: WindowId::from_bytes(&[3; 16]).unwrap(),
            ordinal,
            sides,
            value: (value % u64::from(sides)) as u32 + 1,
            source: source.clone(),
        }
    }
    fn binding<'a>(&self, source: &'a RuleReference) -> DrawBinding<'a> {
        DrawBinding {
            operation: OperationId::from_bytes(&[1; 16]).unwrap(),
            resolution: ResolutionId::from_bytes(&[2; 16]).unwrap(),
            window: WindowId::from_bytes(&[3; 16]).unwrap(),
            source,
            next_ordinal: self.ordinal,
        }
    }
    fn step(
        &mut self,
        state: &Battle,
        actor: u32,
        action: &Action,
    ) -> df_rules::tactical::Resolution {
        let source = source();
        let mut supplied = Vec::new();
        let before = state.clone();
        loop {
            match resolve(state, actor, action, self.binding(&source), &supplied) {
                Err(RuleError::MissingDraw { sides }) => {
                    let ordinal = self.ordinal + supplied.len() as u32;
                    supplied.push(self.record(sides, &source, ordinal));
                }
                Ok(result) => {
                    assert_eq!(result.draws, supplied);
                    assert_eq!(*state, before);
                    self.ordinal += supplied.len() as u32;
                    return result;
                }
                Err(error) => panic!(
                    "illegal proposal actor={actor} round={} action={action:?}: {error:?}",
                    state.round()
                ),
            }
        }
    }
}
fn load(layout: &str) -> BattleMap {
    let bytes: &[u8] = match layout {
        "broken-yard" => include_bytes!("../../df-rules/tests/data/tactical/broken-yard.json"),
        "wolf-trail" => include_bytes!("../../df-rules/tests/data/tactical/wolf-trail.json"),
        _ => include_bytes!("../../df-rules/tests/data/tactical/open-court.json"),
    };
    BattleMap::from_json(bytes, layout).unwrap()
}
fn setup(layout: &str, enemies: &[&str], dice: &mut Dice) -> Battle {
    let map = load(layout);
    let mut specs: Vec<_> = map
        .hero_spawns()
        .iter()
        .enumerate()
        .map(|(i, &cell)| hero(i as u32 + 1, cell))
        .collect();
    for (index, &kind) in enemies.iter().enumerate() {
        let cell = if layout == "open-court" && kind == "skeleton" {
            map.enemy_spawns()[1]
        } else {
            map.enemy_spawns()[index]
        };
        specs.push(CombatantSpec::creature(index as u32 + 10, kind, cell, 20).unwrap());
    }
    let source = source();
    let mut records = Vec::new();
    loop {
        match begin(map.clone(), specs.clone(), dice.binding(&source), &records) {
            Err(RuleError::MissingDraw { sides }) => {
                let ordinal = dice.ordinal + records.len() as u32;
                records.push(dice.record(sides, &source, ordinal));
            }
            Ok(result) => {
                assert_eq!(result.draws, records);
                dice.ordinal += records.len() as u32;
                return result.battle;
            }
            Err(error) => panic!("begin:{error:?}"),
        }
    }
}
fn admitted_choice(state: &Battle, actor: u32, dice: &Dice) -> Action {
    let pins = pins();
    let basis = basis();
    let source = source();
    let policy_ref = ContentReference {
        package: pins.content.package.clone(),
        entry: label("nearest-reachable-cover"),
    };
    let observer = KnowledgeObserver::Entity(EntityId::from_bytes(&[actor as u8; 16]).unwrap());
    let observation = RankingObservation {
        basis: &basis,
        pins: &pins,
        observer,
        logical_time: LogicalTime {
            ticks: u64::from(state.round()) * 6,
            ticks_per_second: 1,
        },
        cause: OperationId::from_bytes(&[1; 16]).unwrap(),
        policy: &policy_ref,
    };
    let knowledge = TacticalKnowledge {
        basis: &basis,
        pins: &pins,
        observer,
        grants: &[],
        beliefs: &[],
        npcs: &[],
    };
    let policy = UtilityPolicy {
        basis: &basis,
        pins: &pins,
        definition: &policy_ref,
        criteria: &[],
    };
    let choice = choose(
        state,
        actor,
        TacticContext {
            candidates: CandidateContext {
                basis: &basis,
                pins: &pins,
            },
            observation: &observation,
            knowledge: &knowledge,
            policy: &policy,
            draws: dice.binding(&source),
        },
    )
    .unwrap();
    choice.unwrap_or_else(|| {
        let combatant = state.actor(actor).unwrap();
        let proposed = propose(state, actor).unwrap();
        let legality = proposed
            .as_ref()
            .map(|action| resolve(state, actor, action, dice.binding(&source), &[]));
        panic!(
            "no legal tactic: proposed={proposed:?} legality={legality:?} actor={actor} side={:?} kind={:?} cell={:?} hp={} active={:?} finished={} reactions={:?} budget={:?}",
            combatant.side(),
            combatant.spec().kind,
            combatant.cell(),
            combatant.health().hit_points,
            state.active_actor(),
            state.finished(),
            state.reaction_offers(),
            combatant.budget()
        )
    })
}
fn simulate(layout: &str, enemies: &[&str], seed: u64) -> (u32, String) {
    let mut dice = Dice {
        state: seed,
        ordinal: 0,
    };
    let mut state = setup(layout, enemies, &mut dice);
    let mut log: BTreeMap<u32, Vec<String>> = BTreeMap::new();
    let mut spent: BTreeMap<u32, u16> = BTreeMap::new();
    let mut saw_cover = false;
    let mut saw_range = false;
    let mut saw_long_range = false;
    let initial_ammo: BTreeMap<_, _> = state
        .combatants()
        .iter()
        .map(|a| (a.id(), (a.arrows(), a.bolts())))
        .collect();
    let mut attacks = 0;
    for _ in 0..1000 {
        if state.finished() {
            break;
        }
        assert!(state.round() <= 30, "fight stalled seed={seed}");
        let actor = state
            .reaction_offers()
            .first()
            .copied()
            .or_else(|| state.active_actor())
            .unwrap();
        let action = admitted_choice(&state, actor, &dice);
        let before = state.actor(actor).unwrap().budget();
        let round = state.round();
        let result = dice.step(&state, actor, &action);
        if let Action::Move(path) = &action {
            state.validate_path(actor, path).unwrap();
        }
        for attack in &result.attacks {
            attacks += 1;
            assert_ne!(attack.cover, Cover::Total);
            assert!(
                attack.distance_ft
                    <= if matches!(
                        action,
                        Action::Attack {
                            weapon: AttackUse { ranged: true, .. },
                            ..
                        }
                    ) {
                        u32::from(attack.weapon.range().unwrap().1)
                    } else {
                        5
                    }
            );
            saw_cover |= attack.cover != Cover::None;
            saw_range |= attack.distance_ft > 5;
            saw_long_range |= attack.disadvantage.contains(&AttackReason::LongRange);
            assert_eq!(
                attack.roll_mode,
                match (attack.advantage.is_empty(), attack.disadvantage.is_empty()) {
                    (false, true) => 1,
                    (true, false) => -1,
                    _ => 0,
                }
            );
            if attack.weapon.ammunition() {
                *spent.entry(actor).or_default() += 1;
            }
            log.entry(round).or_default().push(format!(
                "{}>{} {} {}ft {:?} d{:?}{} dmg{}{}",
                actor,
                attack.target,
                attack.weapon.id(),
                attack.distance_ft,
                attack.cover,
                attack.faces,
                if attack.hit { "hit" } else { "miss" },
                attack.damage,
                if attack.killed { "dead" } else { "" }
            ));
        }
        if matches!(
            action,
            Action::Dash
                | Action::Dodge
                | Action::Disengage
                | Action::DoffShield
                | Action::Attack {
                    spend: AttackSpend::Action,
                    ..
                }
        ) {
            assert!(!before.action_used);
            assert!(result.battle.actor(actor).unwrap().budget().action_used);
        }
        for combatant in result.battle.combatants() {
            assert!(combatant.budget().movement_spent_ft <= combatant.spec().speed_ft * 2);
        }
        state = result.battle;
    }
    assert!(state.finished(), "simulation did not terminate");
    assert!(attacks > 0);
    assert!(saw_range);
    if enemies == ["skeleton"] && layout == "open-court" {
        assert!(
            saw_long_range,
            "distant skeleton must exercise recorded long-range disadvantage"
        );
    }
    if layout == "broken-yard" {
        assert!(
            saw_cover,
            "cover layout must affect an actual recorded attack"
        );
    }
    for actor in state.combatants() {
        let used = spent.get(&actor.id()).copied().unwrap_or(0);
        assert_eq!(
            actor.arrows() + actor.bolts() + used,
            initial_ammo[&actor.id()].0 + initial_ammo[&actor.id()].1
        );
        if actor.id() == 3 {
            assert_eq!(actor.arrows(), 20 - used);
            assert_eq!(actor.ammunition_used(), used);
        }
    }
    let archer_used = state.actor(3).unwrap().ammunition_used();
    let recovered = dice.step(
        &state,
        3,
        &Action::RecoverAmmunition {
            elapsed_seconds: 60,
        },
    );
    assert_eq!(
        recovered.battle.actor(3).unwrap().arrows(),
        20 - archer_used + archer_used / 2
    );
    assert_eq!(state.winner(), Some(Side::Heroes));
    let rounds = state.round();
    let mut text = format!(
        "{layout} seed={seed} rounds={rounds} arrows={}/20\n",
        state.actor(3).unwrap().arrows()
    );
    for round in 1..=rounds {
        text.push_str(&format!(
            "R{round}: {}\n",
            log.get(&round)
                .map_or("positioning/end turns".to_owned(), |events| events
                    .join("; "))
        ));
    }
    (rounds, text)
}
#[test]
fn tactical_simulator_four_complete_source_fights_with_recorded_dice() {
    for (layout, enemies, seed) in [
        ("open-court", vec!["goblin-warrior"], 31),
        ("broken-yard", vec!["goblin-warrior"; 3], 26),
        // SRD 5.2.1 p. 364: a Wolf Bite has no save roll after its attack/damage dice.
        ("wolf-trail", vec!["wolf"; 2], 23),
        ("open-court", vec!["skeleton"], 0),
    ] {
        let (rounds, log) = simulate(layout, &enemies, seed);
        println!("{log}");
        assert!(
            rounds >= 3,
            "expected several rounds without inflating HP: {log}"
        );
    }
}
#[test]
fn tactical_dash_approaches_after_action_and_every_proposal_makes_progress() {
    let mut dice = Dice {
        state: 1,
        ordinal: 0,
    };
    let mut state = setup("open-court", &["wolf"], &mut dice);
    while state.active_actor() != Some(1) {
        let actor = state.active_actor().unwrap();
        state = dice.step(&state, actor, &Action::EndTurn).battle;
    }
    let origin = state.actor(1).unwrap().cell();
    assert_eq!(propose(&state, 1), Ok(Some(Action::Dash)));
    state = dice.step(&state, 1, &Action::Dash).battle;
    let action = propose(&state, 1).unwrap().unwrap();
    assert!(matches!(action, Action::Move(_)));
    state = dice.step(&state, 1, &action).battle;
    assert_ne!(state.actor(1).unwrap().cell(), origin);
    assert!(state.actor(1).unwrap().budget().action_used);
}
#[test]
fn tactical_skeleton_uses_actual_ranged_stat_block_and_cover_policy_is_legal() {
    let mut dice = Dice {
        state: 7,
        ordinal: 0,
    };
    let mut state = setup("broken-yard", &["skeleton"], &mut dice);
    while state.active_actor() != Some(10) {
        let actor = state.active_actor().unwrap();
        state = dice.step(&state, actor, &Action::EndTurn).battle;
    }
    let mut seen = false;
    for _ in 0..12 {
        let action = propose(&state, 10).unwrap().unwrap();
        if let Action::Attack { weapon, .. } = action {
            assert_eq!(weapon.weapon, Weapon::Shortbow);
            assert!(weapon.ranged);
            let result = dice.step(&state, 10, &action);
            assert_eq!(result.attacks[0].modifier, 5);
            seen = true;
            break;
        }
        state = dice.step(&state, 10, &action).battle;
    }
    assert!(seen);
}
#[test]
fn tactical_policy_prefers_reachable_target_and_handles_optional_reactions() {
    let mut dice = Dice {
        state: 5,
        ordinal: 0,
    };
    let map = load("open-court");
    let specs = vec![
        hero(1, Cell(1, 1)),
        hero(2, Cell(5, 4)),
        hero(3, Cell(1, 7)),
        CombatantSpec::creature(10, "wolf", Cell(6, 4), 0).unwrap(),
    ];
    let src = source();
    let faces = [1, 1, 1, 20];
    let records: Vec<_> = faces
        .into_iter()
        .enumerate()
        .map(|(i, value)| {
            let mut record = dice.record(20, &src, i as u32);
            record.value = value;
            record
        })
        .collect();
    let state = begin(map, specs, dice.binding(&src), &records)
        .unwrap()
        .battle;
    dice.ordinal = 4;
    let action = propose(&state, 10).unwrap().unwrap();
    assert!(matches!(action, Action::Attack { target: 2, .. }));
    assert_eq!(Some(action.clone()), propose(&state, 10).unwrap());
    let pending = dice
        .step(&state, 10, &Action::Move(vec![Cell(7, 4)]))
        .battle;
    assert_eq!(pending.reaction_offers(), &[2]);
    let decline = propose(&pending, 2).unwrap().unwrap();
    assert!(matches!(
        decline,
        Action::Opportunity {
            attack: Some(AttackUse {
                weapon: Weapon::Greatsword,
                ..
            })
        }
    ));
    let moved = dice.step(&pending, 2, &decline).battle;
    assert_eq!(moved.actor(10).unwrap().cell(), Cell(7, 4));
    assert!(moved.actor(2).unwrap().budget().reaction_used);
}

#[test]
fn tactical_monster_unarmed_attacks_do_not_inherit_named_weapon_actions() {
    for (kind, modifier, damage) in [
        ("wolf", 4, 3),
        ("mastiff", 3, 2),
        ("cultist", 2, 1),
        ("goblin-warrior", 1, 0),
        ("giant-rat", 0, 0),
        ("giant-weasel", 2, 1),
    ] {
        let mut dice = Dice {
            state: 1,
            ordinal: 0,
        };
        let src = source();
        let specs = vec![
            hero(1, Cell(1, 1)),
            CombatantSpec::creature(10, kind, Cell(2, 1), 0).unwrap(),
        ];
        let mut records = vec![dice.record(20, &src, 0), dice.record(20, &src, 1)];
        records[0].value = 20;
        records[1].value = 1;
        let mut state = begin(load("open-court"), specs, dice.binding(&src), &records)
            .unwrap()
            .battle;
        dice.ordinal = 2;
        if kind == "goblin-warrior" {
            state = dice.step(&state, 1, &Action::DropProne).battle;
        }
        state = dice.step(&state, 1, &Action::EndTurn).battle;
        let usage = AttackUse {
            weapon: Weapon::UnarmedStrike,
            ranged: false,
            hands: 0,
            finesse_dexterity: false,
            knockout: false,
            savage_attacker: false,
            use_mastery: false,
        };
        let action = Action::Attack {
            target: 1,
            weapon: usage,
            spend: AttackSpend::Action,
        };
        let count = if kind == "goblin-warrior" { 2 } else { 1 };
        let records: Vec<_> = (0..count)
            .map(|index| {
                let mut record = dice.record(20, &src, dice.ordinal + index);
                record.value = 19;
                record
            })
            .collect();
        let result = resolve(&state, 10, &action, dice.binding(&src), &records).unwrap();
        assert_eq!(result.attacks[0].modifier, modifier, "{kind}");
        assert_eq!(result.attacks[0].damage, damage, "{kind}");
        assert_eq!(result.attacks[0].necrotic_damage, 0, "{kind}");
        assert_eq!(result.draws.len(), count as usize);
        if matches!(kind, "wolf" | "mastiff") {
            assert!(!result.battle.actor(1).unwrap().health().prone);
        }
    }
}
#[test]
fn tactical_scout_multiattack_admits_only_its_named_actions() {
    let mut dice = Dice {
        state: 1,
        ordinal: 0,
    };
    let src = source();
    let specs = vec![
        hero(1, Cell(1, 1)),
        CombatantSpec::creature(10, "scout", Cell(2, 1), 20).unwrap(),
    ];
    let mut records = vec![dice.record(20, &src, 0), dice.record(20, &src, 1)];
    records[0].value = 1;
    records[1].value = 20;
    let state = begin(load("open-court"), specs, dice.binding(&src), &records)
        .unwrap()
        .battle;
    dice.ordinal = 2;
    let unarmed = AttackUse {
        weapon: Weapon::UnarmedStrike,
        ranged: false,
        hands: 0,
        finesse_dexterity: false,
        knockout: false,
        savage_attacker: false,
        use_mastery: false,
    };
    let action = Action::Attack {
        target: 1,
        weapon: unarmed,
        spend: AttackSpend::Action,
    };
    let result = dice.step(&state, 10, &action);
    assert!(!result.battle.actor(10).unwrap().extra_attacks_available());
    let mut sword = unarmed;
    sword.weapon = Weapon::Shortsword;
    sword.hands = 1;
    sword.finesse_dexterity = true;
    let first = dice.step(
        &state,
        10,
        &Action::Attack {
            target: 1,
            weapon: sword,
            spend: AttackSpend::Action,
        },
    );
    assert!(first.battle.actor(10).unwrap().extra_attacks_available());
    let before = first.battle.clone();
    assert_eq!(
        resolve(
            &first.battle,
            10,
            &Action::Attack {
                target: 1,
                weapon: unarmed,
                spend: AttackSpend::Multiattack
            },
            dice.binding(&src),
            &[]
        ),
        Err(RuleError::InvalidExtraAttack)
    );
    assert_eq!(first.battle, before);
}
#[test]
fn tactical_approach_uses_rules_attackable_endpoints_instead_of_total_cover_target() {
    let map=BattleMap::from_json(br#"{"schema":2,"grid":{"cols":10,"rows":5,"blocked":[]},"layouts":[{"id":"curtain","obstacles":[{"id":"curtain","kind":"curtain","cells":[[2,1]],"blocks_movement":false,"cover":"total","height_m":3}],"difficult":[],"spawns":{"heroes":[[1,1]],"enemies":[[2,1],[6,1]]}}]}"#,"curtain").unwrap();
    let mut dice = Dice {
        state: 1,
        ordinal: 0,
    };
    let src = source();
    let specs = vec![
        hero(1, Cell(1, 1)),
        CombatantSpec::creature(10, "wolf", Cell(2, 1), 0).unwrap(),
        CombatantSpec::creature(11, "wolf", Cell(6, 1), 0).unwrap(),
    ];
    let mut records = vec![dice.record(20, &src, 0), dice.record(20, &src, 1)];
    records[0].value = 20;
    records[1].value = 1;
    let state = begin(map, specs, dice.binding(&src), &records)
        .unwrap()
        .battle;
    let usage = AttackUse {
        weapon: Weapon::Greatsword,
        ranged: false,
        hands: 2,
        finesse_dexterity: false,
        knockout: false,
        savage_attacker: false,
        use_mastery: true,
    };
    assert_eq!(
        state.attack_from(1, Cell(1, 1), 10, usage),
        Err(RuleError::TotalCover)
    );
    assert_eq!(state.attack_from(1, Cell(5, 2), 11, usage), Ok(()));
    let action = propose(&state, 1).unwrap().unwrap();
    let Action::Move(path) = &action else {
        panic!("must approach legal target: {action:?}")
    };
    state.validate_path(1, path).unwrap();
    assert_eq!(
        state.attack_from(1, *path.last().unwrap(), 11, usage),
        Ok(())
    );
}

#[test]
fn tactical_optional_masteries_can_be_declined_without_changing_source_damage() {
    let mut dice = Dice {
        state: 1,
        ordinal: 0,
    };
    let src = source();
    let specs = vec![
        hero(1, Cell(1, 1)),
        CombatantSpec::creature(10, "wolf", Cell(2, 1), 0).unwrap(),
    ];
    let mut records = vec![dice.record(20, &src, 0), dice.record(20, &src, 1)];
    records[0].value = 20;
    records[1].value = 1;
    let state = begin(load("open-court"), specs, dice.binding(&src), &records)
        .unwrap()
        .battle;
    dice.ordinal = 2;
    let mut weapon = AttackUse {
        weapon: Weapon::Greatsword,
        ranged: false,
        hands: 2,
        finesse_dexterity: false,
        knockout: false,
        savage_attacker: false,
        use_mastery: false,
    };
    let mut miss = dice.record(20, &src, 2);
    miss.value = 1;
    let declined = resolve(
        &state,
        1,
        &Action::Attack {
            target: 10,
            weapon,
            spend: AttackSpend::Action,
        },
        dice.binding(&src),
        &[miss.clone()],
    )
    .unwrap();
    assert_eq!(declined.attacks[0].damage, 0);
    assert!(!declined.attacks[0].graze);
    assert_eq!(declined.battle.actor(10).unwrap().health().hit_points, 11);
    weapon.use_mastery = true;
    let selected = resolve(
        &state,
        1,
        &Action::Attack {
            target: 10,
            weapon,
            spend: AttackSpend::Action,
        },
        dice.binding(&src),
        &[miss],
    )
    .unwrap();
    assert_eq!(selected.attacks[0].damage, 3);
    assert!(selected.attacks[0].graze);
    let specs = vec![
        hero(3, Cell(1, 1)),
        CombatantSpec::creature(10, "wolf", Cell(10, 1), 0).unwrap(),
    ];
    let state = begin(
        load("open-court"),
        specs,
        dice.binding(&src),
        &records
            .iter()
            .cloned()
            .map(|mut record| {
                record.ordinal += 2;
                record
            })
            .collect::<Vec<_>>(),
    )
    .unwrap()
    .battle;
    let mut weapon = AttackUse {
        weapon: Weapon::Longbow,
        ranged: true,
        hands: 2,
        finesse_dexterity: false,
        knockout: false,
        savage_attacker: false,
        use_mastery: false,
    };
    let mut faces = vec![dice.record(20, &src, 2), dice.record(8, &src, 3)];
    faces[0].value = 15;
    faces[1].value = 1;
    let declined = resolve(
        &state,
        3,
        &Action::Attack {
            target: 10,
            weapon,
            spend: AttackSpend::Action,
        },
        dice.binding(&src),
        &faces,
    )
    .unwrap();
    assert_eq!(declined.battle.actor(10).unwrap().speed_ft(), 40);
    assert_eq!(declined.attacks[0].damage, 4);
    weapon.use_mastery = true;
    let selected = resolve(
        &state,
        3,
        &Action::Attack {
            target: 10,
            weapon,
            spend: AttackSpend::Action,
        },
        dice.binding(&src),
        &faces,
    )
    .unwrap();
    assert_eq!(selected.battle.actor(10).unwrap().speed_ft(), 30);
    assert_eq!(selected.attacks[0].damage, 4);
}

#[test]
fn tactical_adjacent_ranged_creature_with_safe_retreat_keeps_distance_and_fires() {
    let src = source();
    let mut dice = Dice {
        state: 1,
        ordinal: 0,
    };
    let specs = vec![
        hero(1, Cell(1, 1)),
        CombatantSpec::creature(11, "wolf", Cell(1, 2), 0).unwrap(),
        CombatantSpec::creature(10, "skeleton", Cell(4, 1), 20).unwrap(),
    ];
    let mut records: Vec<_> = (0..3).map(|i| dice.record(20, &src, i)).collect();
    records[0].value = 20;
    records[1].value = 15;
    records[2].value = 1;
    let mut state = begin(load("open-court"), specs, dice.binding(&src), &records)
        .unwrap()
        .battle;
    dice.ordinal = 3;
    state = dice.step(&state, 1, &Action::EndTurn).battle;
    state = dice
        .step(&state, 11, &Action::Move(vec![Cell(2, 3)]))
        .battle;
    assert_eq!(state.reaction_offers(), &[1]);
    let weapon = AttackUse {
        weapon: Weapon::Greatsword,
        ranged: false,
        hands: 2,
        finesse_dexterity: false,
        knockout: false,
        savage_attacker: false,
        use_mastery: true,
    };
    let mut miss = dice.record(20, &src, dice.ordinal);
    miss.value = 1;
    state = resolve(
        &state,
        1,
        &Action::Opportunity {
            attack: Some(weapon),
        },
        dice.binding(&src),
        &[miss],
    )
    .unwrap()
    .battle;
    dice.ordinal += 1;
    state = dice.step(&state, 11, &Action::EndTurn).battle;
    state = dice
        .step(&state, 10, &Action::Move(vec![Cell(3, 1), Cell(2, 1)]))
        .battle;
    assert!(state.actor(1).unwrap().budget().reaction_used);
    assert_eq!(state.actor(10).unwrap().movement_remaining_ft(), 20);
    let action = propose(&state, 10).unwrap().unwrap();
    let Action::Move(path) = &action else {
        panic!("ranged creature must retreat: {action:?}")
    };
    state.validate_path(10, path).unwrap();
    assert!(
        path.last()
            .unwrap()
            .distance(state.actor(1).unwrap().cell())
            > 5
    );
    state = dice.step(&state, 10, &action).battle;
    assert!(state.reaction_offers().is_empty());
    let action = propose(&state, 10).unwrap().unwrap();
    assert!(matches!(
        action,
        Action::Attack {
            weapon: AttackUse {
                weapon: Weapon::Shortbow,
                ranged: true,
                ..
            },
            ..
        }
    ));
    let result = dice.step(&state, 10, &action);
    assert_eq!(result.attacks[0].modifier, 5);
}

#[test]
fn tactical_ammunition_collection_requires_a_living_capable_actor_after_combat() {
    let src = source();
    let mut dice = Dice {
        state: 1,
        ordinal: 0,
    };
    let specs = vec![
        hero(3, Cell(1, 1)),
        CombatantSpec::creature(10, "wolf", Cell(2, 1), 0).unwrap(),
        hero(1, Cell(2, 2)),
    ];
    let mut records: Vec<_> = (0..3).map(|i| dice.record(20, &src, i)).collect();
    records[0].value = 20;
    records[1].value = 20;
    records[2].value = 1;
    let mut state = begin(load("open-court"), specs, dice.binding(&src), &records)
        .unwrap()
        .battle;
    dice.ordinal = 3;
    let mut fixed_step = |state: &Battle, actor, action: &Action, faces: &[(u32, u32)]| {
        let records: Vec<_> = faces
            .iter()
            .enumerate()
            .map(|(i, &(sides, value))| {
                let mut record = dice.record(sides, &src, dice.ordinal + i as u32);
                record.value = value;
                record
            })
            .collect();
        let result = resolve(state, actor, action, dice.binding(&src), &records).unwrap();
        dice.ordinal += records.len() as u32;
        result
    };
    let bow = AttackUse {
        weapon: Weapon::Longbow,
        ranged: true,
        hands: 2,
        finesse_dexterity: false,
        knockout: false,
        savage_attacker: false,
        use_mastery: true,
    };
    state = fixed_step(
        &state,
        3,
        &Action::Attack {
            target: 10,
            weapon: bow,
            spend: AttackSpend::Action,
        },
        &[(20, 1), (20, 2)],
    )
    .battle;
    assert_eq!(state.actor(3).unwrap().arrows(), 19);
    state = fixed_step(&state, 3, &Action::EndTurn, &[]).battle;
    let bite = AttackUse {
        weapon: Weapon::Bite,
        ranged: false,
        hands: 1,
        finesse_dexterity: false,
        knockout: false,
        savage_attacker: false,
        use_mastery: false,
    };
    let bite_result = fixed_step(
        &state,
        10,
        &Action::Attack {
            target: 3,
            weapon: bite,
            spend: AttackSpend::Action,
        },
        // SRD 5.2.1 p. 364: Bite is an attack roll plus damage dice, with Prone on a hit.
        &[(20, 20), (6, 6), (6, 6)],
    );
    assert!(bite_result.battle.actor(3).unwrap().health().prone);
    state = bite_result.battle;
    state = fixed_step(&state, 10, &Action::EndTurn, &[]).battle;
    let sword = AttackUse {
        weapon: Weapon::Greatsword,
        ranged: false,
        hands: 2,
        finesse_dexterity: false,
        knockout: false,
        savage_attacker: false,
        use_mastery: true,
    };
    state = fixed_step(
        &state,
        1,
        &Action::Attack {
            target: 10,
            weapon: sword,
            spend: AttackSpend::Action,
        },
        &[(20, 20), (6, 6), (6, 6), (6, 6), (6, 6)],
    )
    .battle;
    assert!(state.finished());
    assert!(state.actor(3).unwrap().health().unconscious);
    let before = state.clone();
    let supplied = Vec::new();
    let original = supplied.clone();
    assert_eq!(
        resolve(
            &state,
            3,
            &Action::RecoverAmmunition {
                elapsed_seconds: 60
            },
            dice.binding(&src),
            &supplied
        ),
        Err(RuleError::Incapacitated)
    );
    assert_eq!(state, before);
    assert_eq!(supplied, original);
    let alive = resolve(
        &state,
        1,
        &Action::RecoverAmmunition {
            elapsed_seconds: 60,
        },
        dice.binding(&src),
        &[],
    )
    .unwrap();
    assert_eq!(alive.battle.actor(1).unwrap().arrows(), 20);
}

#[test]
fn tactical_natural_attacks_need_no_hands_and_are_never_dropped_items() {
    let src = source();
    let mut dice = Dice {
        state: 1,
        ordinal: 0,
    };
    let specs = vec![
        hero(1, Cell(1, 1)),
        CombatantSpec::creature(10, "wolf", Cell(2, 1), 0).unwrap(),
    ];
    let mut records = vec![dice.record(20, &src, 0), dice.record(20, &src, 1)];
    records[0].value = 1;
    records[1].value = 20;
    let state = begin(load("open-court"), specs, dice.binding(&src), &records)
        .unwrap()
        .battle;
    dice.ordinal = 2;
    let bite = AttackUse {
        weapon: Weapon::Bite,
        ranged: false,
        hands: 0,
        finesse_dexterity: false,
        knockout: false,
        savage_attacker: false,
        use_mastery: false,
    };
    // SRD 5.2.1 p. 364: no d20 is drawn for the Bite's Prone rider.
    let mut rolls = vec![dice.record(20, &src, 2), dice.record(6, &src, 3)];
    rolls[0].value = 15;
    rolls[1].value = 1;
    let bite_result = resolve(
        &state,
        10,
        &Action::Attack {
            target: 1,
            weapon: bite,
            spend: AttackSpend::Action,
        },
        dice.binding(&src),
        &rolls,
    )
    .unwrap();
    assert_eq!(
        bite_result
            .draws
            .iter()
            .map(|draw| draw.sides)
            .collect::<Vec<_>>(),
        vec![20, 6]
    );
    let state = bite_result.battle;
    assert!(state.actor(1).unwrap().health().prone);
    dice.ordinal = 4;
    let state = dice.step(&state, 10, &Action::EndTurn).battle;
    let sword = AttackUse {
        weapon: Weapon::Greatsword,
        ranged: false,
        hands: 2,
        finesse_dexterity: false,
        knockout: true,
        savage_attacker: false,
        use_mastery: true,
    };
    let mut rolls = vec![
        dice.record(20, &src, 4),
        dice.record(20, &src, 5),
        dice.record(6, &src, 6),
        dice.record(6, &src, 7),
    ];
    rolls[0].value = 15;
    rolls[1].value = 16;
    rolls[2].value = 6;
    rolls[3].value = 6;
    // Wolf Bite makes the hero Prone; this deliberately selects the lower of two attack dice.
    let state = resolve(
        &state,
        1,
        &Action::Attack {
            target: 10,
            weapon: sword,
            spend: AttackSpend::Action,
        },
        dice.binding(&src),
        &rolls,
    )
    .unwrap()
    .battle;
    assert!(state.actor(10).unwrap().health().knocked_out);
    assert!(state.actor(10).unwrap().dropped_weapons().is_empty());
}
