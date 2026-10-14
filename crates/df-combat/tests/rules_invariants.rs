//! PLAY-R01: the independent rules auditor run as a property test over generated fights.
//!
//! Every creature in the compiled table, one to four enemies, every authored battle layout
//! (the scan layouts included) and the three df-rules fixture layouts, heroes at random legal
//! cells. Enemy turns come from the server's own planner (`df_combat::tactical::propose`, the
//! function `choose` and the journey's `enemy_turns` call); hero turns are random mixtures of
//! legal and illegal inputs. Every accepted step is audited against the SRD by
//! `support/rules_audit.rs`, and every rejection of a path or attack the auditor considers
//! legal is a violation too.
#[path = "support/rules_audit.rs"]
mod rules_audit;

use df_combat::tactical::propose;
use df_model::checkpoint::{ActualDraw, ResolutionId, RuleReference, WindowId};
use df_rules::tactical::{
    Action, AttackSpend, AttackUse, Battle, BattleMap, Cell, CombatantSpec, Cover, CreatureKind,
    DrawBinding, Lighting, Resolution, RuleError, Side, Weapon, begin, resolve, validate_archer,
};
use df_types::{OperationId, RevisionLabel};
use rules_audit::{Audit, FightAudit, Grid, judge_attack, judge_path};
use std::collections::BTreeMap;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut value = self.0;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^ (value >> 31)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }
}

fn label(text: &str) -> RevisionLabel {
    RevisionLabel::new(Some(text)).unwrap()
}
fn source() -> RuleReference {
    RuleReference {
        catalog: label("fixture-srd-5.2.1"),
        source: label("pdf-8974902d"),
        entry: label("tactical"),
        clause: label("play-r01-audit"),
    }
}
fn ids() -> (OperationId, ResolutionId, WindowId) {
    (
        OperationId::from_bytes(&[1; 16]).unwrap(),
        ResolutionId::from_bytes(&[2; 16]).unwrap(),
        WindowId::from_bytes(&[3; 16]).unwrap(),
    )
}
fn record(rng: &mut Rng, sides: u32, ordinal: u32, source: &RuleReference) -> ActualDraw {
    let (operation, resolution, window) = ids();
    ActualDraw {
        operation,
        resolution,
        window,
        ordinal,
        sides,
        value: (rng.next() % u64::from(sides)) as u32 + 1,
        source: source.clone(),
    }
}
fn binding(source: &RuleReference) -> DrawBinding<'_> {
    let (operation, resolution, window) = ids();
    DrawBinding {
        operation,
        resolution,
        window,
        source,
        next_ordinal: 0,
    }
}
/// Resolve with freshly recorded dice, retrying `MissingDraw` exactly like the journey does.
fn resolve_dice(
    battle: &Battle,
    actor: u32,
    action: &Action,
    rng: &mut Rng,
) -> Result<Resolution, RuleError> {
    let source = source();
    let mut supplied = Vec::new();
    loop {
        match resolve(battle, actor, action, binding(&source), &supplied) {
            Err(RuleError::MissingDraw { sides }) => {
                let ordinal = supplied.len() as u32;
                supplied.push(record(rng, sides, ordinal, &source));
            }
            other => return other,
        }
    }
}
fn resolve_faces(battle: &Battle, actor: u32, action: &Action, faces: &[u32]) -> Resolution {
    let source = source();
    let mut rng = Rng(31);
    let mut supplied = Vec::new();
    loop {
        match resolve(battle, actor, action, binding(&source), &supplied) {
            Err(RuleError::MissingDraw { sides }) => {
                let ordinal = supplied.len() as u32;
                let value = faces
                    .get(ordinal as usize)
                    .copied()
                    .expect("the fixture supplies every requested die");
                assert!(value > 0 && value <= sides);
                supplied.push(ActualDraw {
                    value,
                    ..record(&mut rng, sides, ordinal, &source)
                });
            }
            Ok(result) => {
                assert_eq!(supplied.len(), faces.len());
                return result;
            }
            Err(error) => panic!("the fixture action must resolve: {error:?}"),
        }
    }
}

const SCENES: &[(&str, &[u8])] = &[
    (
        "harbor-journey",
        include_bytes!("../../../content/battle-scenes/harbor-journey.json"),
    ),
    (
        "lamplit-market",
        include_bytes!("../../../content/battle-scenes/lamplit-market.json"),
    ),
    (
        "orrery-library",
        include_bytes!("../../../content/battle-scenes/orrery-library.json"),
    ),
    (
        "pinewood-trail",
        include_bytes!("../../../content/battle-scenes/pinewood-trail.json"),
    ),
    (
        "reedmoor-boardwalk",
        include_bytes!("../../../content/battle-scenes/reedmoor-boardwalk.json"),
    ),
    (
        "rookhill-gate",
        include_bytes!("../../../content/battle-scenes/rookhill-gate.json"),
    ),
    (
        "salt-quay",
        include_bytes!("../../../content/battle-scenes/salt-quay.json"),
    ),
    (
        "scan-riverbed",
        include_bytes!("../../../content/battle-scenes/scan-riverbed.json"),
    ),
    (
        "scan-summit",
        include_bytes!("../../../content/battle-scenes/scan-summit.json"),
    ),
    (
        "scan-valley-bridges",
        include_bytes!("../../../content/battle-scenes/scan-valley-bridges.json"),
    ),
    (
        "scan-village",
        include_bytes!("../../../content/battle-scenes/scan-village.json"),
    ),
    (
        "thistledown-farm",
        include_bytes!("../../../content/battle-scenes/thistledown-farm.json"),
    ),
    (
        "windward-pass",
        include_bytes!("../../../content/battle-scenes/windward-pass.json"),
    ),
    (
        "fixture-open-court",
        include_bytes!("../../df-rules/tests/data/tactical/open-court.json"),
    ),
    (
        "fixture-broken-yard",
        include_bytes!("../../df-rules/tests/data/tactical/broken-yard.json"),
    ),
    (
        "fixture-wolf-trail",
        include_bytes!("../../df-rules/tests/data/tactical/wolf-trail.json"),
    ),
];
const CREATURES: [&str; 12] = [
    "bandit",
    "cultist",
    "guard",
    "goblin-warrior",
    "kobold-warrior",
    "skeleton",
    "zombie",
    "wolf",
    "giant-rat",
    "giant-weasel",
    "mastiff",
    "scout",
];

/// The lighting fact per scene, mirroring the typed table the journey stages fights from: the
/// photographic scan scenes are daylight, every painted scene is night or interior.
fn is_sunlit(scene: &str) -> bool {
    scene.starts_with("scan-")
}
fn layouts() -> Vec<(String, &'static [u8], String)> {
    let mut all = Vec::new();
    for (name, bytes) in SCENES {
        let data: df_rules::tactical::BattleSceneData = serde_json::from_slice(bytes).unwrap();
        for layout in &data.layouts {
            all.push(((*name).to_owned(), *bytes, layout.id.clone()));
        }
    }
    all
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
fn hero(id: u32, cell: Cell, archer: bool) -> CombatantSpec {
    if archer {
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

struct Fight<'g> {
    battle: Battle,
    grid: &'g Grid,
    specs: Vec<(u32, CreatureKind, u8)>,
    begin_draws: Vec<ActualDraw>,
}
fn free_cells(grid: &Grid, taken: &[Cell]) -> Vec<Cell> {
    let mut cells = Vec::new();
    for col in 0..grid.cols {
        for row in 0..grid.rows {
            let cell = Cell(col, row);
            if !grid.blocked(cell) && !taken.contains(&cell) {
                cells.push(cell);
            }
        }
    }
    cells
}
fn make_fight<'g>(
    bytes: &[u8],
    layout: &str,
    grid: &'g Grid,
    enemies: &[&str],
    heroes: usize,
    rng: &mut Rng,
) -> Fight<'g> {
    let map = BattleMap::from_json(bytes, layout)
        .unwrap()
        .with_lighting(if grid.sunlight {
            Lighting::Sunlight
        } else {
            Lighting::NightOrInterior
        });
    let mut taken: Vec<Cell> = Vec::new();
    let mut specs = Vec::new();
    for index in 0..heroes {
        let free = free_cells(grid, &taken);
        let cell = free[rng.below(free.len())];
        taken.push(cell);
        specs.push(hero(index as u32 + 1, cell, rng.chance(50)));
    }
    let mut spawns = grid.enemy_spawns.clone();
    for (index, kind) in enemies.iter().enumerate() {
        let cell = if rng.chance(70) && !spawns.is_empty() {
            let pick = rng.below(spawns.len());
            let cell = spawns.remove(pick);
            if taken.contains(&cell) {
                let free = free_cells(grid, &taken);
                free[rng.below(free.len())]
            } else {
                cell
            }
        } else {
            let free = free_cells(grid, &taken);
            free[rng.below(free.len())]
        };
        taken.push(cell);
        specs.push(CombatantSpec::creature(index as u32 + 10, kind, cell, 20).unwrap());
    }
    let source = source();
    let mut records = Vec::new();
    let resolution = loop {
        match begin(map.clone(), specs.clone(), binding(&source), &records) {
            Err(RuleError::MissingDraw { sides }) => {
                let ordinal = records.len() as u32;
                records.push(record(rng, sides, ordinal, &source));
            }
            Ok(done) => break done,
            Err(error) => panic!("begin failed: {error:?}"),
        }
    };
    Fight {
        battle: resolution.battle,
        grid,
        specs: specs
            .iter()
            .map(|s| (s.id, s.kind, s.abilities[1]))
            .collect(),
        begin_draws: records,
    }
}

#[derive(Default)]
struct Totals {
    fights: u64,
    enemy_turns: u64,
    hero_inputs: u64,
    hero_rejections: u64,
    counts: BTreeMap<&'static str, u64>,
    violations: BTreeMap<&'static str, (u64, Vec<String>)>,
}
impl Totals {
    fn absorb(&mut self, audit: Audit, context: &str) {
        for (name, count) in audit.counts {
            *self.counts.entry(name).or_default() += count;
        }
        for violation in audit.violations {
            let entry = self.violations.entry(violation.id).or_default();
            entry.0 += 1;
            if entry.1.len() < 3 {
                entry.1.push(format!("{context}: {}", violation.detail));
            }
        }
    }
    fn violate(&mut self, id: &'static str, detail: String) {
        let entry = self.violations.entry(id).or_default();
        entry.0 += 1;
        if entry.1.len() < 3 {
            entry.1.push(detail);
        }
    }
}

fn weapon_use(weapon: Weapon, ranged: bool, rng: &mut Rng, masteries: &[Weapon]) -> AttackUse {
    AttackUse {
        weapon,
        ranged,
        hands: if weapon.two_handed() || (weapon == Weapon::Spear && !ranged && rng.chance(50)) {
            2
        } else {
            1
        },
        finesse_dexterity: weapon.finesse() && rng.chance(50),
        knockout: false,
        savage_attacker: rng.chance(35),
        use_mastery: masteries.contains(&weapon) && rng.chance(60),
    }
}

/// One hero input drawn at random, deliberately including illegal ones.
fn hero_input(battle: &Battle, grid: &Grid, actor: u32, rng: &mut Rng) -> Action {
    let me = battle.actor(actor).unwrap();
    match rng.below(20) {
        0..=5 => {
            if rng.chance(25) {
                // Illegal geometry: teleport, wall walk, or straight through a capable enemy.
                let mut path = Vec::new();
                let mut at = me.cell();
                for _ in 0..1 + rng.below(4) {
                    let jump = if rng.chance(50) {
                        1
                    } else {
                        2 + rng.below(4) as i32
                    };
                    let col = (i32::from(at.0) + rng.below(3) as i32 * jump - jump)
                        .clamp(0, i32::from(grid.cols) - 1);
                    let row = (i32::from(at.1) + rng.below(3) as i32 * jump - jump)
                        .clamp(0, i32::from(grid.rows) - 1);
                    at = Cell(col as u16, row as u16);
                    path.push(at);
                }
                Action::Move(path)
            } else {
                let reachable = battle.reachable_cells(actor).unwrap_or_default();
                let goals: Vec<Cell> = reachable.into_iter().filter(|c| *c != me.cell()).collect();
                if goals.is_empty() {
                    return Action::EndTurn;
                }
                let goal = goals[rng.below(goals.len())];
                match battle.shortest_path(actor, goal, Some(me.movement_remaining_ft())) {
                    Ok(path) if !path.is_empty() => Action::Move(path),
                    _ => Action::EndTurn,
                }
            }
        }
        6..=14 => {
            let others: Vec<u32> = battle.combatants().iter().map(|c| c.id()).collect();
            let near: Vec<u32> = battle
                .combatants()
                .iter()
                .filter(|c| c.side() == Side::Enemies && !c.health().dead)
                .map(|c| c.id())
                .collect();
            let target = if rng.chance(85) && !near.is_empty() {
                near[rng.below(near.len())]
            } else {
                others[rng.below(others.len())]
            };
            let mut weapons: Vec<Weapon> = me.spec().weapons.clone();
            weapons.push(Weapon::UnarmedStrike);
            let weapon = weapons[rng.below(weapons.len())];
            let ranged = if weapon.ranged_weapon() {
                rng.chance(95)
            } else {
                weapon.range().is_some() && rng.chance(50)
            };
            let usage = weapon_use(weapon, ranged, rng, &me.spec().masteries);
            let spend = match rng.below(10) {
                0 => AttackSpend::LightBonus,
                1 => AttackSpend::Nick,
                _ => AttackSpend::Action,
            };
            Action::Attack {
                target,
                weapon: usage,
                spend,
            }
        }
        15 => Action::Dash,
        16 => Action::Disengage,
        17 => [
            Action::Dodge,
            Action::Stand,
            Action::DropProne,
            Action::SecondWind,
            Action::DoffShield,
        ][rng.below(5)]
        .clone(),
        _ => Action::EndTurn,
    }
}

fn reaction_choice(battle: &Battle, actor: u32, rng: &mut Rng, enemy_takes: bool) -> Action {
    let me = battle.actor(actor).unwrap();
    let takes = if me.side() == Side::Heroes {
        rng.chance(75)
    } else {
        enemy_takes && rng.chance(75)
    };
    if !takes {
        return Action::Opportunity { attack: None };
    }
    let mut options: Vec<Weapon> = me
        .held_weapons()
        .iter()
        .copied()
        .filter(|w| !w.ranged_weapon())
        .collect();
    if me.side() == Side::Enemies {
        options.extend(
            me.spec()
                .weapons
                .iter()
                .copied()
                .filter(|w| !w.ranged_weapon()),
        );
    }
    options.push(Weapon::UnarmedStrike);
    let weapon = options[rng.below(options.len())];
    let mut usage = weapon_use(weapon, false, rng, &me.spec().masteries);
    if matches!(weapon, Weapon::Bite | Weapon::Slam | Weapon::UnarmedStrike) {
        usage.hands = 0;
    }
    Action::Opportunity {
        attack: Some(usage),
    }
}

/// One accepted step as the auditor saw it, kept so tests can replay it with a tampered record.
#[derive(Clone)]
struct Step {
    before: Battle,
    actor: u32,
    action: Action,
    result: Resolution,
}

fn run_fight(
    fight: Fight<'_>,
    rng: &mut Rng,
    enemy_reactions: bool,
    totals: &mut Totals,
    context: &str,
    mut record: Option<&mut Vec<Step>>,
) {
    let grid = fight.grid;
    let mut battle = fight.battle;
    let mut checker = FightAudit::begin(&battle, grid, &fight.specs, &fight.begin_draws);
    let mut attempts = 0_u32;
    let mut last_active = battle.active_actor();
    let mut hero_turns_without_attack = 0;
    let _ = &mut hero_turns_without_attack;
    for _ in 0..900 {
        if battle.finished() || battle.round() > 25 {
            break;
        }
        let reactor = battle.reaction_offers().first().copied();
        let actor = reactor.or_else(|| battle.active_actor()).unwrap();
        if battle.active_actor() != last_active {
            last_active = battle.active_actor();
            attempts = 0;
        }
        let me = battle.actor(actor).unwrap();
        let (action, planned) = if reactor.is_some() {
            (reaction_choice(&battle, actor, rng, enemy_reactions), false)
        } else if battle.needs_death_save() {
            (Action::DeathSave, false)
        } else if me.health().incapacitated() {
            (Action::EndTurn, false)
        } else if me.side() == Side::Enemies {
            match propose(&battle, actor) {
                Ok(Some(action)) => (action, true),
                other => {
                    totals.violate(
                        "P-NO-PROPOSAL",
                        format!("{context}: planner returned {other:?} for {actor}"),
                    );
                    break;
                }
            }
        } else {
            attempts += 1;
            totals.hero_inputs += 1;
            if attempts > 8 {
                (Action::EndTurn, false)
            } else {
                (hero_input(&battle, grid, actor, rng), false)
            }
        };
        if planned {
            totals.enemy_turns += 0;
        }
        let trace = std::env::var("DF_R01_TRACE")
            .is_ok_and(|t| context.starts_with(&format!("fight {t} ")));
        if trace {
            println!(
                "STEP actor {actor} {action:?}\n   state {}",
                rules_audit::dump(&battle)
            );
        }
        match resolve_dice(&battle, actor, &action, rng) {
            Ok(result) => {
                if trace {
                    println!(
                        "   -> ok attacks {:?} offers {:?}",
                        result
                            .attacks
                            .iter()
                            .map(|a| (a.hit, a.damage))
                            .collect::<Vec<_>>(),
                        result.battle.reaction_offers()
                    );
                }
                checker.accepted(&battle, actor, &action, &result);
                if let Some(steps) = record.as_deref_mut() {
                    steps.push(Step {
                        before: battle.clone(),
                        actor,
                        action: action.clone(),
                        result: result.clone(),
                    });
                }
                if planned && matches!(action, Action::EndTurn) {
                    totals.enemy_turns += 1;
                }
                battle = result.battle;
            }
            Err(error) => {
                if planned {
                    totals.violate(
                        "P-ILLEGAL-PROPOSAL",
                        format!("{context}: planner proposed {action:?} for {actor}: {error:?}"),
                    );
                    break;
                }
                totals.hero_rejections += 1;
                reject_check(
                    &battle,
                    grid,
                    actor,
                    &action,
                    &error,
                    &mut checker.audit,
                    reactor.is_some(),
                );
            }
        }
    }
    aftermath(&mut battle, &mut checker, rng, totals, context);
    checker.finish(&battle);
    totals.fights += 1;
    totals.absorb(checker.audit, context);
}

fn dying_heroes(battle: &Battle) -> usize {
    battle
        .combatants()
        .iter()
        .filter(|c| {
            c.side() == Side::Heroes
                && c.health().hit_points == 0
                && !c.health().dead
                && !c.health().stable
        })
        .count()
}
/// After a won fight time keeps passing in six-second rounds, in the same initiative order,
/// until no hero is dying (SRD p17-18): each round holds exactly one Death Saving Throw per
/// dying hero, and the fate always resolves (stable, dead or back on their feet) within the
/// at most five saves three successes against three failures allow.
fn aftermath(
    battle: &mut Battle,
    checker: &mut FightAudit<'_>,
    rng: &mut Rng,
    totals: &mut Totals,
    context: &str,
) {
    if !battle.finished() {
        return;
    }
    if dying_heroes(battle) > 0 {
        resolve_dying(battle, checker, rng, totals, context);
    }
    after_fate(battle, checker, rng, totals, context);
}
fn resolve_dying(
    battle: &mut Battle,
    checker: &mut FightAudit<'_>,
    rng: &mut Rng,
    totals: &mut Totals,
    context: &str,
) {
    checker.audit.tally("aftermaths-with-a-dying-hero");
    let start_round = battle.round();
    for _ in 0..200 {
        if dying_heroes(battle) == 0 {
            break;
        }
        let Some(actor) = battle.active_actor() else {
            break;
        };
        let action = if battle.needs_death_save() {
            Action::DeathSave
        } else {
            Action::EndTurn
        };
        match resolve_dice(battle, actor, &action, rng) {
            Ok(result) => {
                checker.accepted(battle, actor, &action, &result);
                *battle = result.battle;
            }
            Err(error) => {
                totals.violate(
                    "P-AFTERMATH-REFUSED",
                    format!("{context}: {action:?} for {actor}: {error:?}"),
                );
                return;
            }
        }
    }
    if dying_heroes(battle) != 0 {
        totals.violate(
            "P-AFTERMATH-NEVER-ENDS",
            format!("{context}: a hero is still dying"),
        );
        return;
    }
    let rounds = battle.round() - start_round;
    checker.audit.expect(rounds <= 5, "P-AFTERMATH-LONG", || {
        format!("{rounds} rounds of death saves")
    });
    checker.audit.tally("aftermaths-resolved");
    for c in battle
        .combatants()
        .iter()
        .filter(|c| c.side() == Side::Heroes && c.health().hit_points == 0)
    {
        checker.audit.tally(if c.health().dead {
            "aftermath-hero-died"
        } else {
            "aftermath-hero-stabilised"
        });
    }
}
fn after_fate(
    battle: &mut Battle,
    checker: &mut FightAudit<'_>,
    rng: &mut Rng,
    totals: &mut Totals,
    context: &str,
) {
    // A Stable hero at 0 regains 1 Hit Point after 1d4 hours: the fight can always move on.
    let stable: Vec<u32> = battle
        .combatants()
        .iter()
        .filter(|c| c.side() == Side::Heroes && c.health().stable && c.health().hit_points == 0)
        .map(|c| c.id())
        .collect();
    for id in stable {
        match resolve_dice(battle, id, &Action::StableRecovery, rng) {
            Ok(result) => {
                checker.accepted(battle, id, &Action::StableRecovery, &result);
                *battle = result.battle;
            }
            Err(error) => {
                totals.violate("P-STABLE-RECOVERY-REFUSED", format!("{context}: {error:?}"))
            }
        }
    }
    // Then the party rests an hour: every hero on their feet recharges one Second Wind use.
    let rested: Vec<u32> = battle
        .combatants()
        .iter()
        .filter(|c| c.side() == Side::Heroes && c.health().hit_points > 0 && !c.health().dead)
        .map(|c| c.id())
        .collect();
    for id in rested {
        let action = Action::ShortRest {
            elapsed_seconds: 3600,
        };
        match resolve_dice(battle, id, &action, rng) {
            Ok(result) => {
                checker.accepted(battle, id, &action, &result);
                *battle = result.battle;
            }
            Err(error) => totals.violate("P-SHORT-REST-REFUSED", format!("{context}: {error:?}")),
        }
    }
}

/// A rejection is only a violation when the independent oracle says the input was legal.
fn reject_check(
    battle: &Battle,
    grid: &Grid,
    actor: u32,
    action: &Action,
    error: &RuleError,
    audit: &mut Audit,
    pending_window: bool,
) {
    audit.tally("rejections");
    if pending_window
        || battle.needs_death_save()
        || battle.active_actor() != Some(actor)
        || battle.finished()
    {
        return;
    }
    let Ok(me) = battle.actor(actor) else { return };
    if !rules_audit::capable(me) {
        return;
    }
    match action {
        Action::Move(path) => {
            if let Ok(cost) = judge_path(battle, grid, actor, path) {
                audit.fail(
                    "M-REJECTED-LEGAL-PATH",
                    format!("path {path:?} ({} ft) rejected with {error:?}", cost.total),
                );
            } else {
                audit.tally("illegal-paths-refused");
            }
        }
        Action::Attack { target, weapon, .. } => {
            if judge_attack(battle, grid, actor, *target, weapon).is_ok() {
                if matches!(
                    error,
                    RuleError::OutOfRange
                        | RuleError::TotalCover
                        | RuleError::InvalidTarget
                        | RuleError::Incapacitated
                ) {
                    audit.fail(
                        "A-REJECTED-LEGAL-ATTACK",
                        format!("{weapon:?} on {target} rejected with {error:?}"),
                    );
                }
            } else {
                audit.tally("illegal-attacks-refused");
            }
        }
        _ => {}
    }
}

fn fight_count() -> usize {
    std::env::var("DF_R01_FIGHTS")
        .ok()
        .and_then(|v| v.parse().ok())
        // An unoptimised build is an order of magnitude slower; `cargo test --release` runs the
        // thousands the task asks for, and DF_R01_FIGHTS sets any count.
        .unwrap_or(if cfg!(debug_assertions) { 220 } else { 4000 })
}

fn run_property(count: usize, mixed: bool) -> Totals {
    let layouts = layouts();
    let mut totals = Totals::default();
    let mut grids: BTreeMap<(usize, usize), Grid> = BTreeMap::new();
    for (index, (_, bytes, layout)) in layouts.iter().enumerate() {
        grids.insert(
            (index, 0),
            Grid::from_scene(bytes, layout, is_sunlit(&layouts[index].0)),
        );
    }
    let mut rng = Rng(0x00D0_0D1E);
    for fight in 0..count {
        let index = fight % layouts.len();
        let (scene, bytes, layout) = &layouts[index];
        let grid = &grids[&(index, 0)];
        let kind_slot = (fight / layouts.len()) % CREATURES.len();
        let enemy_count = 1 + (fight / (layouts.len() * CREATURES.len())) % 4;
        let enemies: Vec<&str> = (0..enemy_count)
            .map(|_| {
                if mixed {
                    CREATURES[rng.below(CREATURES.len())]
                } else {
                    CREATURES[kind_slot % CREATURES.len()]
                }
            })
            .collect();
        let heroes = 1 + rng.below(3);
        let context = format!("fight {fight} {scene}/{layout} {enemies:?} heroes {heroes}");
        let fight = make_fight(bytes, layout, grid, &enemies, heroes, &mut rng);
        let reacting = fight_is_reacting(&mut rng);
        run_fight(fight, &mut rng, reacting, &mut totals, &context, None);
    }
    totals
}
fn fight_is_reacting(rng: &mut Rng) -> bool {
    rng.chance(50)
}

fn print_report(totals: &Totals) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "fights {} | hero inputs {} (rejected {}) | audited enemy end-of-turn {}\n",
        totals.fights, totals.hero_inputs, totals.hero_rejections, totals.enemy_turns
    ));
    for (name, count) in &totals.counts {
        out.push_str(&format!("  {name}: {count}\n"));
    }
    if totals.violations.is_empty() {
        out.push_str("VIOLATIONS: none\n");
    }
    for (id, (count, examples)) in &totals.violations {
        out.push_str(&format!("VIOLATION {id} x{count}\n"));
        for example in examples {
            out.push_str(&format!("    {example}\n"));
        }
    }
    out
}

#[test]
fn generated_fights_with_planner_enemies_and_random_hero_inputs_obey_the_srd() {
    let totals = run_property(fight_count(), false);
    let report = print_report(&totals);
    println!("{report}");
    if let Ok(path) = std::env::var("DF_R01_REPORT") {
        std::fs::write(path, &report).unwrap();
    }
    assert!(totals.violations.is_empty(), "{report}");
    for must in [
        "moves-completed",
        "opportunity-windows",
        "opportunity-attacks",
        "enemy-opportunity-attacks",
        "stat-block-attacks",
        "ranged-attacks",
        "melee-attacks",
        "death-saves",
        "critical-hits",
        "advantage-rolls",
        "disadvantage-rolls",
        "dashes",
        "stand-ups",
        "second-winds",
        "savage-attacker-hits",
        "mastery-attacks",
        "illegal-paths-refused",
        "illegal-attacks-refused",
    ] {
        assert!(
            totals.counts.get(must).copied().unwrap_or(0) > 0,
            "property never exercised {must}\n{report}"
        );
    }
}

#[test]
fn mixed_creature_fights_obey_the_srd() {
    let totals = run_property(fight_count() / 2, true);
    let report = print_report(&totals);
    assert!(totals.violations.is_empty(), "{report}");
}

/// A recorded fight kept whole so a test can replay it through a fresh auditor with one defect
/// injected, proving the auditor notices that defect rather than passing everything.
#[derive(Clone)]
struct Scripted {
    grid: Grid,
    specs: Vec<(u32, CreatureKind, u8)>,
    begin_draws: Vec<ActualDraw>,
    initial: Battle,
    steps: Vec<Step>,
}
fn scripted(index: usize) -> Scripted {
    let layouts = layouts();
    let (scene, bytes, layout) = &layouts[index % layouts.len()];
    let grid = Grid::from_scene(bytes, layout, is_sunlit(scene));
    let mut rng = Rng(0x5EED_0000 + index as u64);
    let enemies: Vec<&str> = (0..1 + rng.below(3))
        .map(|_| CREATURES[rng.below(CREATURES.len())])
        .collect();
    let heroes = 1 + rng.below(2);
    let fight = make_fight(bytes, layout, &grid, &enemies, heroes, &mut rng);
    let (specs, begin_draws, initial) = (
        fight.specs.clone(),
        fight.begin_draws.clone(),
        fight.battle.clone(),
    );
    let mut steps = Vec::new();
    let mut totals = Totals::default();
    run_fight(
        fight,
        &mut rng,
        true,
        &mut totals,
        "scripted",
        Some(&mut steps),
    );
    assert!(
        totals.violations.is_empty(),
        "scripted fight itself must be clean"
    );
    Scripted {
        grid,
        specs,
        begin_draws,
        initial,
        steps,
    }
}
/// Replays `steps` through a fresh auditor; `tamper` may edit a step or ask to skip it.
fn replay(s: &Scripted, tamper: &mut dyn FnMut(usize, &mut Step) -> bool) -> Audit {
    let mut checker = FightAudit::begin(&s.initial, &s.grid, &s.specs, &s.begin_draws);
    for (index, step) in s.steps.iter().enumerate() {
        let mut step = step.clone();
        if tamper(index, &mut step) {
            continue;
        }
        checker.accepted(&step.before, step.actor, &step.action, &step.result);
    }
    checker.audit
}
fn flagged(audit: &Audit, id: &str) -> bool {
    audit.violations.iter().any(|v| v.id == id)
}
/// Finds a recorded fight with a step matching `want`, searching deterministic fights.
fn find_step(want: impl Fn(&Step) -> bool) -> (Scripted, usize) {
    for index in 0..600 {
        let s = scripted(index);
        if let Some(position) = s.steps.iter().position(&want) {
            return (s, position);
        }
    }
    panic!("no recorded fight contains the wanted step");
}
fn first_attack(step: &Step) -> bool {
    matches!(step.action, Action::Attack { .. }) && step.result.attacks.len() == 1
}

#[test]
fn the_auditor_flags_a_teleporting_move() {
    let (s, at) = find_step(|st| matches!(&st.action, Action::Move(p) if p.len() >= 2));
    let cols = s.grid.cols;
    let audit = replay(&s, &mut |i, st| {
        if i == at
            && let Action::Move(path) = &mut st.action
        {
            let last = *path.last().unwrap();
            *path = vec![Cell(last.0.saturating_add(4).min(cols - 1), last.1)];
        }
        false
    });
    assert!(
        flagged(&audit, "M-ACCEPTED-ILLEGAL-PATH")
            || flagged(&audit, "M-COST")
            || flagged(&audit, "M-POSITION"),
        "teleport not flagged: {:?}",
        audit.violations
    );
}

#[test]
fn the_auditor_flags_each_edited_attack_record_and_passes_the_unedited_one() {
    let (s, at) = find_step(|st| first_attack(st) && st.result.attacks[0].hit);
    type Edit = fn(&mut Step);
    let edits: [(&str, Edit); 7] = [
        ("A-DISTANCE", |st| st.result.attacks[0].distance_ft += 5),
        ("A-HIT", |st| st.result.attacks[0].hit = false),
        ("A-MODIFIER", |st| st.result.attacks[0].modifier += 1),
        ("A-DAMAGE", |st| st.result.attacks[0].damage += 1),
        ("A-ROLL-MODE", |st| {
            st.result.attacks[0].roll_mode = -1 - st.result.attacks[0].roll_mode.abs()
        }),
        ("A-COVER", |st| {
            st.result.attacks[0].cover = if st.result.attacks[0].cover == Cover::None {
                Cover::Half
            } else {
                Cover::None
            }
        }),
        ("A-ARMOR-CLASS", |st| st.result.attacks[0].armor_class += 1),
    ];
    for (id, edit) in edits {
        let audit = replay(&s, &mut |i, st| {
            if i == at {
                edit(st);
            }
            false
        });
        assert!(
            flagged(&audit, id),
            "{id} not flagged: {:?}",
            audit.violations
        );
    }
    assert!(replay(&s, &mut |_, _| false).violations.is_empty());
}

#[test]
fn the_auditor_flags_a_melee_attack_from_beyond_reach() {
    let (s, at) = find_step(|st| {
        first_attack(st) && st.result.attacks[0].ranged && st.result.attacks[0].distance_ft > 5
    });
    let audit = replay(&s, &mut |i, st| {
        if i == at
            && let Action::Attack { weapon, .. } = &mut st.action
        {
            weapon.ranged = false;
        }
        false
    });
    assert!(
        flagged(&audit, "A-ACCEPTED-ILLEGAL"),
        "melee past reach not flagged: {:?}",
        audit.violations
    );
}

#[test]
fn the_auditor_flags_a_second_action_in_one_turn() {
    let (s, at) = find_step(|st| {
        matches!(
            st.action,
            Action::Attack {
                spend: AttackSpend::Action,
                ..
            }
        ) && st.result.attacks.len() == 1
    });
    let mut twice = s.clone();
    let again = Step {
        before: twice.steps[at].result.battle.clone(),
        ..twice.steps[at].clone()
    };
    twice.steps.insert(at + 1, again);
    let audit = replay(&twice, &mut |_, _| false);
    assert!(
        flagged(&audit, "T-SECOND-ACTION"),
        "second action not flagged: {:?}",
        audit.violations
    );
}

#[test]
fn the_auditor_flags_acting_out_of_turn() {
    let (s, at) = find_step(|st| first_attack(st) && st.actor >= 10);
    let other = s
        .specs
        .iter()
        .map(|spec| spec.0)
        .find(|id| *id != s.steps[at].actor && *id < 10)
        .unwrap();
    let audit = replay(&s, &mut |i, st| {
        if i == at {
            st.actor = other;
        }
        false
    });
    assert!(
        flagged(&audit, "T-OUT-OF-TURN"),
        "out of turn not flagged: {:?}",
        audit.violations
    );
}

#[test]
fn the_auditor_flags_a_dying_heros_turn_without_its_death_saving_throw() {
    let (s, _) = find_step(|st| matches!(st.action, Action::DeathSave));
    let audit = replay(&s, &mut |_, st| matches!(st.action, Action::DeathSave));
    assert!(
        flagged(&audit, "T-DEATH-SAVE-SKIPPED"),
        "skipped death save not flagged: {:?}",
        audit.violations
    );
}

#[test]
fn the_auditor_flags_an_initiative_order_that_disagrees_with_the_rolls() {
    let (mut s, _) = find_step(|_| true);
    s.specs.reverse();
    let audit = replay(&s, &mut |_, _| true);
    assert!(
        flagged(&audit, "I-INIT-ORDER") || flagged(&audit, "I-INIT-DRAWS"),
        "initiative: {:?}",
        audit.violations
    );
}

/// The live fight's Kobold Warrior 3 "struck Brakka" and ended four squares away beside Sylvie.
/// That is a legal turn: move next to Brakka, stab him, and spend the rest of the 30 feet of
/// Speed walking toward the only hero still standing (SRD 5.2.1 p14: movement may be split around
/// an action). This reproduces the shape on the scan-village layout, audits every step, and checks
/// the feet add up to no more than the Speed.
#[test]
fn a_kobold_can_stab_one_hero_then_walk_toward_the_other_within_thirty_feet() {
    let layouts = layouts();
    let index = layouts
        .iter()
        .position(|(scene, _, layout)| scene == "scan-village" && layout == "north-south")
        .unwrap();
    let (scene, bytes, layout) = &layouts[index];
    let grid = Grid::from_scene(bytes, layout, is_sunlit(scene));
    // Brakka, the Kobold beside him, and Sylvie four or five squares away, all on free squares.
    let free = free_cells(&grid, &[]);
    let (brakka, kobold, sylvie) =
        free.iter()
            .find_map(|&a| {
                let b = free
                    .iter()
                    .copied()
                    .find(|b| rules_audit::squares(a, *b) == 1)?;
                let c = free.iter().copied().find(|c| {
                    rules_audit::squares(a, *c) == 5 && rules_audit::squares(b, *c) >= 4
                })?;
                Some((a, b, c))
            })
            .expect("a free triple exists");
    let mut found = false;
    for seed in 0..400_u64 {
        let mut rng = Rng(seed);
        let map = BattleMap::from_json(bytes, layout)
            .unwrap()
            .with_lighting(Lighting::Sunlight);
        let specs = vec![
            hero(1, brakka, false),
            hero(2, sylvie, false),
            CombatantSpec::creature(10, "kobold-warrior", kobold, 0).unwrap(),
        ];
        let source = source();
        // Heroes roll 1, the Kobold group rolls twice (sunlight) and keeps a high enough face.
        let faces = [1, 1, 20, 20];
        let records: Vec<ActualDraw> = faces
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let mut d = record(&mut rng, 20, i as u32, &source);
                d.value = *v;
                d
            })
            .collect();
        let started = begin(map, specs.clone(), binding(&source), &records).unwrap();
        let mut battle = started.battle.with_hero_resources(1, 1, 20, 2).unwrap();
        assert_eq!(battle.active_actor(), Some(10));
        let metas: Vec<_> = specs
            .iter()
            .map(|s| (s.id, s.kind, s.abilities[1]))
            .collect();
        let mut checker = FightAudit::begin(&battle, &grid, &metas, &records);
        let (mut feet_moved, mut attacked, mut moved_after_attack) = (0_u32, false, false);
        for _ in 0..8 {
            let Ok(Some(action)) = propose(&battle, 10) else {
                break;
            };
            let result = resolve_dice(&battle, 10, &action, &mut rng).unwrap();
            checker.accepted(&battle, 10, &action, &result);
            for step in &result.moves {
                feet_moved += u32::from(step.cost_ft);
                moved_after_attack |= attacked;
            }
            for attack in &result.attacks {
                attacked = true;
                assert!(!attack.ranged && attack.distance_ft <= 5);
                if !attack.hit {
                    break;
                }
            }
            battle = result.battle;
            if matches!(action, Action::EndTurn) {
                break;
            }
        }
        assert!(
            checker.audit.violations.is_empty(),
            "{:?}",
            checker.audit.violations
        );
        assert!(feet_moved <= 30, "{feet_moved} ft moved");
        let brakka_down = battle.actor(1).unwrap().health().hit_points == 0;
        if attacked && brakka_down && moved_after_attack {
            let kobold_after = battle.actor(10).unwrap().cell();
            assert!(
                rules_audit::squares(kobold_after, brakka) > 1,
                "the Kobold walked away from Brakka"
            );
            found = true;
            break;
        }
    }
    assert!(found, "no seed produced stab then walk");
}

/// SRD 5.2.1 p302: the Kobold Warrior's Dagger is a melee or ranged attack (range 20/60 ft). With no
/// melee square reachable this turn it walks into range and throws instead of only walking.
#[test]
fn a_kobold_that_cannot_reach_melee_this_turn_walks_into_range_and_throws_its_dagger() {
    let json: &[u8] = br#"{"schema":2,"grid":{"cols":14,"rows":3,"blocked":[]},"layouts":[{"id":"flat","obstacles":[],"difficult":[],"spawns":{"heroes":[[0,1]],"enemies":[[13,1]]}}]}"#;
    let grid = Grid::from_scene(json, "flat", false);
    let map = BattleMap::from_json(json, "flat").unwrap();
    let specs = vec![
        hero(1, Cell(1, 1), false),
        CombatantSpec::creature(10, "kobold-warrior", Cell(10, 1), 0).unwrap(),
    ];
    let source = source();
    let mut rng = Rng(7);
    let records: Vec<ActualDraw> = [1, 20]
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let mut d = record(&mut rng, 20, i as u32, &source);
            d.value = *v;
            d
        })
        .collect();
    let mut battle = begin(map, specs.clone(), binding(&source), &records)
        .unwrap()
        .battle;
    let metas: Vec<_> = specs
        .iter()
        .map(|s| (s.id, s.kind, s.abilities[1]))
        .collect();
    let mut checker = FightAudit::begin(&battle, &grid, &metas, &records);
    let mut thrown = None;
    let mut dashed = false;
    for _ in 0..6 {
        let Ok(Some(action)) = propose(&battle, 10) else {
            break;
        };
        dashed |= matches!(action, Action::Dash);
        let result = resolve_dice(&battle, 10, &action, &mut rng).unwrap();
        checker.accepted(&battle, 10, &action, &result);
        if let Some(attack) = result.attacks.first() {
            thrown = Some((attack.ranged, attack.distance_ft, attack.weapon));
        }
        battle = result.battle;
        if matches!(action, Action::EndTurn) {
            break;
        }
    }
    assert!(
        checker.audit.violations.is_empty(),
        "{:?}",
        checker.audit.violations
    );
    assert!(!dashed, "it should throw, not Dash");
    assert_eq!(thrown, Some((true, 20, Weapon::Dagger)));
    assert_eq!(battle.actor(10).unwrap().weapon_count(Weapon::Dagger), 2);
}

/// SRD 5.2.1 pp. 18, 190: damage at 0 HP causes a Death Saving Throw failure, doubled on a Critical Hit.
#[test]
fn wolf_attacks_a_dying_hero_when_no_standing_hero_can_be_attacked_this_turn() {
    let json: &[u8] = br#"{"schema":2,"grid":{"cols":16,"rows":3,"blocked":[]},"layouts":[{"id":"flat","obstacles":[],"difficult":[],"spawns":{"heroes":[[0,1],[10,2]],"enemies":[[9,2],[10,1]]}}]}"#;
    let map = BattleMap::from_json(json, "flat").unwrap();
    let specs = vec![
        hero(1, Cell(0, 1), false),
        hero(2, Cell(10, 2), false),
        CombatantSpec::creature(10, "bandit", Cell(9, 2), 0).unwrap(),
        CombatantSpec::creature(11, "wolf", Cell(10, 1), 0).unwrap(),
    ];
    let source = source();
    let mut rng = Rng(37);
    let mut initiative = Vec::new();
    for (ordinal, face) in [18, 17, 20, 19].into_iter().enumerate() {
        let mut draw = record(&mut rng, 20, ordinal as u32, &source);
        draw.value = face;
        initiative.push(draw);
    }
    let battle = begin(map, specs, binding(&source), &initiative)
        .unwrap()
        .battle;
    assert_eq!(battle.active_actor(), Some(10));

    let bandit_attack = Action::Attack {
        target: 2,
        weapon: AttackUse {
            weapon: Weapon::Scimitar,
            ranged: false,
            hands: 1,
            finesse_dexterity: false,
            knockout: false,
            savage_attacker: false,
            use_mastery: false,
        },
        spend: AttackSpend::Action,
    };
    let downed = resolve_faces(&battle, 10, &bandit_attack, &[20, 6, 6]).battle;
    assert_eq!(downed.actor(2).unwrap().health().hit_points, 0);
    assert!(downed.actor(2).unwrap().health().unconscious);
    let wolf_turn = resolve_dice(&downed, 10, &Action::EndTurn, &mut rng)
        .unwrap()
        .battle;
    assert_eq!(wolf_turn.active_actor(), Some(11));

    let action = df_combat::tactical::propose_latest(&wolf_turn, 11)
        .unwrap()
        .unwrap();
    let Action::Attack { target, weapon, .. } = &action else {
        panic!("the wolf should attack the nearby dying hero");
    };
    assert_eq!(*target, 2);
    assert_eq!(weapon.weapon, Weapon::Bite);
    let attacked = resolve_faces(&wolf_turn, 11, &action, &[17, 1, 1, 1]);
    assert!(attacked.attacks[0].hit);
    assert!(attacked.attacks[0].critical);
    assert_eq!(attacked.battle.actor(2).unwrap().health().death_failures, 2);
}

/// SRD 5.2.1 p. 18 permits damage at 0 HP; humanoid targeting remains the table's policy choice.
#[test]
fn bandit_approaches_a_standing_hero_instead_of_attacking_a_dying_hero() {
    let json: &[u8] = br#"{"schema":2,"grid":{"cols":16,"rows":3,"blocked":[]},"layouts":[{"id":"flat","obstacles":[],"difficult":[],"spawns":{"heroes":[[0,1],[10,2]],"enemies":[[9,2],[10,1]]}}]}"#;
    let map = BattleMap::from_json(json, "flat").unwrap();
    let specs = vec![
        hero(1, Cell(0, 1), false),
        hero(2, Cell(10, 2), false),
        CombatantSpec::creature(10, "bandit", Cell(9, 2), 0).unwrap(),
        CombatantSpec::creature(11, "bandit", Cell(10, 1), 0).unwrap(),
    ];
    let source = source();
    let mut rng = Rng(41);
    let mut initiative = Vec::new();
    for (ordinal, face) in [18, 17, 20].into_iter().enumerate() {
        let mut draw = record(&mut rng, 20, ordinal as u32, &source);
        draw.value = face;
        initiative.push(draw);
    }
    let battle = begin(map, specs, binding(&source), &initiative)
        .unwrap()
        .battle;
    let bandit_attack = Action::Attack {
        target: 2,
        weapon: AttackUse {
            weapon: Weapon::Scimitar,
            ranged: false,
            hands: 1,
            finesse_dexterity: false,
            knockout: false,
            savage_attacker: false,
            use_mastery: false,
        },
        spend: AttackSpend::Action,
    };
    let downed = resolve_faces(&battle, 10, &bandit_attack, &[20, 6, 6]).battle;
    let bandit_turn = resolve_dice(&downed, 10, &Action::EndTurn, &mut rng)
        .unwrap()
        .battle;
    assert_eq!(bandit_turn.active_actor(), Some(11));
    assert_eq!(bandit_turn.actor(2).unwrap().health().hit_points, 0);
    assert!(bandit_turn.actor(2).unwrap().health().unconscious);

    assert_eq!(
        df_combat::tactical::propose_latest(&bandit_turn, 11).unwrap(),
        Some(Action::Dash)
    );
    let dashed = resolve_dice(&bandit_turn, 11, &Action::Dash, &mut rng)
        .unwrap()
        .battle;
    let approach = df_combat::tactical::propose_latest(&dashed, 11)
        .unwrap()
        .unwrap();
    assert!(matches!(approach, Action::Move(_)));
    let moved = resolve_dice(&dashed, 11, &approach, &mut rng)
        .unwrap()
        .battle;
    assert_eq!(moved.actor(11).unwrap().cell().distance(Cell(0, 1)), 5);
}

/// SRD 5.2.1 p. 14: a visible creature leaving reach provokes an attack made as a Reaction.
#[test]
fn enemy_policy_takes_a_natural_weapon_opportunity_attack() {
    let json: &[u8] = br#"{"schema":2,"grid":{"cols":4,"rows":3,"blocked":[]},"layouts":[{"id":"flat","obstacles":[],"difficult":[],"spawns":{"heroes":[[1,1]],"enemies":[[2,1]]}}]}"#;
    let map = BattleMap::from_json(json, "flat").unwrap();
    let specs = vec![
        hero(1, Cell(1, 1), false),
        CombatantSpec::creature(10, "giant-weasel", Cell(2, 1), 0).unwrap(),
    ];
    let source = source();
    let mut rng = Rng(11);
    let mut initiative = [
        record(&mut rng, 20, 0, &source),
        record(&mut rng, 20, 1, &source),
    ];
    initiative[0].value = 20;
    initiative[1].value = 1;
    let battle = begin(map, specs, binding(&source), &initiative)
        .unwrap()
        .battle;
    assert_eq!(battle.active_actor(), Some(1));

    let moving = resolve_dice(&battle, 1, &Action::Move(vec![Cell(0, 1)]), &mut rng)
        .unwrap()
        .battle;
    assert_eq!(moving.reaction_offers(), &[10]);
    let Action::Opportunity {
        attack: Some(attack),
    } = propose(&moving, 10).unwrap().unwrap()
    else {
        panic!("the enemy must take its available opportunity attack");
    };
    assert_eq!(attack.weapon, Weapon::Bite);
    assert!(!attack.ranged);
    assert_eq!(
        df_combat::tactical::propose_legacy(&moving, 10).unwrap(),
        Some(Action::Opportunity { attack: None })
    );
}

/// SRD 5.2.1 pp. 9, 14: a spent Reaction cannot be used for a second mover before the enemy's turn.
#[test]
fn enemy_policy_spends_only_one_opportunity_reaction_before_its_turn() {
    let json: &[u8] = br#"{"schema":2,"grid":{"cols":5,"rows":3,"blocked":[]},"layouts":[{"id":"flat","obstacles":[],"difficult":[],"spawns":{"heroes":[[2,1],[3,2]],"enemies":[[3,1]]}}]}"#;
    let map = BattleMap::from_json(json, "flat").unwrap();
    let specs = vec![
        hero(1, Cell(2, 1), false),
        hero(2, Cell(3, 2), false),
        CombatantSpec::creature(10, "wolf", Cell(3, 1), 0).unwrap(),
    ];
    let source = source();
    let mut rng = Rng(13);
    let mut initiative = [
        record(&mut rng, 20, 0, &source),
        record(&mut rng, 20, 1, &source),
        record(&mut rng, 20, 2, &source),
    ];
    initiative[0].value = 20;
    initiative[1].value = 19;
    initiative[2].value = 1;
    let battle = begin(map, specs, binding(&source), &initiative)
        .unwrap()
        .battle;
    assert_eq!(battle.active_actor(), Some(1));

    let pending = resolve_dice(&battle, 1, &Action::Move(vec![Cell(1, 1)]), &mut rng)
        .unwrap()
        .battle;
    let reaction = propose(&pending, 10).unwrap().unwrap();
    let after_reaction = resolve_dice(&pending, 10, &reaction, &mut rng)
        .unwrap()
        .battle;
    assert!(after_reaction.actor(10).unwrap().budget().reaction_used);
    let next_hero = resolve_dice(&after_reaction, 1, &Action::EndTurn, &mut rng)
        .unwrap()
        .battle;
    assert_eq!(next_hero.active_actor(), Some(2));

    let second_move = resolve_dice(&next_hero, 2, &Action::Move(vec![Cell(4, 2)]), &mut rng)
        .unwrap()
        .battle;
    assert!(second_move.reaction_offers().is_empty());
    assert_eq!(second_move.actor(2).unwrap().cell(), Cell(4, 2));
}

/// SRD 5.2.1 pp. 14, 183: an Incapacitated creature cannot take a Reaction.
#[test]
fn incapacitated_enemy_does_not_offer_an_opportunity_attack() {
    let json: &[u8] = br#"{"schema":2,"grid":{"cols":6,"rows":3,"blocked":[]},"layouts":[{"id":"flat","obstacles":[],"difficult":[],"spawns":{"heroes":[[1,1]],"enemies":[[2,1],[5,1]]}}]}"#;
    let map = BattleMap::from_json(json, "flat").unwrap();
    let specs = vec![
        hero(1, Cell(1, 1), false),
        CombatantSpec::creature(10, "wolf", Cell(2, 1), 0).unwrap(),
        CombatantSpec::creature(11, "wolf", Cell(5, 1), 0).unwrap(),
    ];
    let source = source();
    let mut rng = Rng(17);
    let mut initiative = Vec::new();
    let battle = loop {
        match begin(map.clone(), specs.clone(), binding(&source), &initiative) {
            Err(RuleError::MissingDraw { sides }) => {
                let ordinal = initiative.len() as u32;
                let mut draw = record(&mut rng, sides, ordinal, &source);
                draw.value = [20, 10, 1].get(ordinal as usize).copied().unwrap_or(1);
                initiative.push(draw);
            }
            Ok(result) => break result.battle,
            Err(error) => panic!("the battle must begin with its supplied dice: {error:?}"),
        }
    };
    let knockout_attack = AttackUse {
        weapon: Weapon::Greatsword,
        ranged: false,
        hands: 2,
        finesse_dexterity: false,
        knockout: true,
        savage_attacker: false,
        use_mastery: false,
    };
    let action = Action::Attack {
        target: 10,
        weapon: knockout_attack,
        spend: AttackSpend::Action,
    };
    let mut attack_draws = Vec::new();
    let attacked = loop {
        match resolve(&battle, 1, &action, binding(&source), &attack_draws) {
            Err(RuleError::MissingDraw { sides }) => {
                let ordinal = attack_draws.len() as u32;
                let mut draw = record(&mut rng, sides, ordinal, &source);
                draw.value = if sides == 20 { 20 } else { 6 };
                attack_draws.push(draw);
            }
            Ok(result) => break result.battle,
            Err(error) => panic!("the attack must resolve with its supplied dice: {error:?}"),
        }
    };
    assert!(attacked.actor(10).unwrap().health().unconscious);

    let moved = resolve_dice(&attacked, 1, &Action::Move(vec![Cell(0, 1)]), &mut rng)
        .unwrap()
        .battle;
    assert!(moved.reaction_offers().is_empty());
    assert_eq!(moved.actor(1).unwrap().cell(), Cell(0, 1));
}
