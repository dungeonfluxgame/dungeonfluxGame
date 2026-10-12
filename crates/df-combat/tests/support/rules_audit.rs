//! PLAY-R01 independent rules auditor for recorded tactical turns.
//!
//! Everything here is recomputed from the SRD 5.2.1 text (page numbers in the comments) using
//! only the public read accessors of `df_rules::tactical` and the authored scene JSON. It never
//! calls the engine's own path, cost, cover, reach or legality helpers (`validate_path`,
//! `path_cost`, `shortest_path`, `cover_between`, `legal_targets`, `attack_from`, ...), so a bug
//! shared by the engine and its bookkeeping cannot hide from it.
#![allow(dead_code)]

use df_model::checkpoint::ActualDraw;
use df_rules::tactical::{
    Action, AttackReason, AttackSpend, AttackSummary, AttackUse, Battle, BattleSceneData, Cell,
    Combatant, Cover, CreatureKind, Mastery, Resolution, Side, Weapon,
};
use std::collections::{BTreeMap, BTreeSet};

/// One rules violation found by the auditor. `id` is a stable kind used in the report.
#[derive(Clone, Debug)]
pub struct Violation {
    pub id: &'static str,
    pub detail: String,
}

/// Violations plus coverage counters ("how many of each thing was actually checked").
#[derive(Default)]
pub struct Audit {
    pub violations: Vec<Violation>,
    pub counts: BTreeMap<&'static str, u64>,
}
impl Audit {
    pub fn fail(&mut self, id: &'static str, detail: String) {
        self.violations.push(Violation { id, detail });
    }
    pub fn tally(&mut self, name: &'static str) {
        *self.counts.entry(name).or_default() += 1;
    }
    pub fn expect(&mut self, ok: bool, id: &'static str, detail: impl FnOnce() -> String) {
        if !ok {
            self.fail(id, detail());
        }
    }
}

/// Compact picture of the battle for violation details.
pub fn dump(b: &Battle) -> String {
    b.combatants()
        .iter()
        .map(|c| {
            format!(
                "#{}:{:?}@{},{} hp{} {}{}{}{}",
                c.id(),
                c.side(),
                c.cell().0,
                c.cell().1,
                c.health().hit_points,
                if c.health().prone { "P" } else { "" },
                if c.health().unconscious { "U" } else { "" },
                if c.budget().reaction_used { "R" } else { "" },
                if c.budget().disengaged { "D" } else { "" },
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn squares(a: Cell, b: Cell) -> u32 {
    u32::from(a.0.abs_diff(b.0).max(a.1.abs_diff(b.1)))
}
pub fn feet(a: Cell, b: Cell) -> u32 {
    squares(a, b) * 5
}
pub fn modifier(score: u8) -> i16 {
    (i16::from(score) - 10).div_euclid(2)
}

/// The scene geometry read straight from the authored JSON, not from `BattleMap`.
#[derive(Clone)]
pub struct Grid {
    pub cols: u16,
    pub rows: u16,
    blocked: BTreeSet<Cell>,
    difficult: BTreeSet<Cell>,
    cover: BTreeMap<Cell, Cover>,
    pub hero_spawns: Vec<Cell>,
    pub enemy_spawns: Vec<Cell>,
    /// The scene's lighting fact (SRD 5.2.1 p302 Sunlight Sensitivity).
    pub sunlight: bool,
}
impl Grid {
    pub fn from_scene(bytes: &[u8], layout: &str, sunlight: bool) -> Grid {
        let data: BattleSceneData = serde_json::from_slice(bytes).expect("scene json");
        let chosen = data
            .layouts
            .iter()
            .find(|candidate| candidate.id == layout)
            .expect("layout present");
        let mut blocked: BTreeSet<Cell> = data.grid.blocked.iter().copied().collect();
        let mut cover = BTreeMap::new();
        for obstacle in &chosen.obstacles {
            for &cell in &obstacle.cells {
                if obstacle.blocks_movement {
                    blocked.insert(cell);
                }
                let entry = cover.entry(cell).or_insert(Cover::None);
                *entry = (*entry).max(obstacle.cover);
            }
        }
        Grid {
            cols: data.grid.cols,
            rows: data.grid.rows,
            blocked,
            difficult: chosen.difficult.iter().copied().collect(),
            cover,
            hero_spawns: chosen.spawns.heroes.clone(),
            enemy_spawns: chosen.spawns.enemies.clone(),
            sunlight,
        }
    }
    pub fn contains(&self, cell: Cell) -> bool {
        cell.0 < self.cols && cell.1 < self.rows
    }
    pub fn blocked(&self, cell: Cell) -> bool {
        !self.contains(cell) || self.blocked.contains(&cell)
    }
    pub fn difficult(&self, cell: Cell) -> bool {
        self.difficult.contains(&cell)
    }
    /// SRD p15 Cover with the project's disclosed centre-line convention: the segment between
    /// the two centres must enter the open interior of an obstacle square (or of another
    /// living creature's square, Half Cover). Separating-axis test, exact integer arithmetic.
    pub fn cover(&self, from: Cell, to: Cell, creatures: &[Cell]) -> Cover {
        let mut result = Cover::None;
        for (&cell, &grade) in &self.cover {
            if cell != from && segment_enters(from, to, cell) {
                result = result.max(grade);
            }
        }
        for &cell in creatures {
            if cell != from && cell != to && segment_enters(from, to, cell) {
                result = result.max(Cover::Half);
            }
        }
        result
    }
    pub fn in_sight(&self, from: Cell, to: Cell) -> bool {
        self.cover(from, to, &[]) != Cover::Total
    }
}
fn segment_enters(from: Cell, to: Cell, cell: Cell) -> bool {
    let p = [i64::from(from.0) * 2 + 1, i64::from(from.1) * 2 + 1];
    let q = [i64::from(to.0) * 2 + 1, i64::from(to.1) * 2 + 1];
    let low = [i64::from(cell.0) * 2, i64::from(cell.1) * 2];
    for axis in 0..2 {
        let (lo, hi) = (p[axis].min(q[axis]), p[axis].max(q[axis]));
        if !(hi > low[axis] && lo < low[axis] + 2) {
            return false;
        }
    }
    let normal = [-(q[1] - p[1]), q[0] - p[0]];
    if normal == [0, 0] {
        return true;
    }
    let along = normal[0] * p[0] + normal[1] * p[1];
    let corners = [
        [low[0], low[1]],
        [low[0] + 2, low[1]],
        [low[0], low[1] + 2],
        [low[0] + 2, low[1] + 2],
    ];
    let values = corners.map(|corner| normal[0] * corner[0] + normal[1] * corner[1]);
    let min = values.iter().copied().min().unwrap_or(0);
    let max = values.iter().copied().max().unwrap_or(0);
    min < along && along < max
}

// ---------------------------------------------------------------------------------------------
// Source tables, written from SRD 5.2.1 and not read from the engine.
// ---------------------------------------------------------------------------------------------

/// Weapons table (p91) normal/long range for thrown and ammunition weapons.
pub fn range_of(weapon: Weapon) -> Option<(u32, u32)> {
    match weapon {
        Weapon::Dagger | Weapon::Spear => Some((20, 60)),
        Weapon::Javelin => Some((30, 120)),
        Weapon::Shortbow | Weapon::LightCrossbow => Some((80, 320)),
        Weapon::Longbow => Some((150, 600)),
        _ => None,
    }
}
pub fn is_bow_or_crossbow(weapon: Weapon) -> bool {
    matches!(
        weapon,
        Weapon::Shortbow | Weapon::Longbow | Weapon::LightCrossbow
    )
}
pub fn is_light(weapon: Weapon) -> bool {
    matches!(
        weapon,
        Weapon::Dagger | Weapon::Scimitar | Weapon::Shortsword | Weapon::Sickle
    )
}
pub fn mastery_of(weapon: Weapon) -> Option<Mastery> {
    match weapon {
        Weapon::Greatsword => Some(Mastery::Graze),
        Weapon::Flail | Weapon::Spear => Some(Mastery::Sap),
        Weapon::Javelin | Weapon::Longbow | Weapon::LightCrossbow => Some(Mastery::Slow),
        Weapon::Shortbow | Weapon::Shortsword => Some(Mastery::Vex),
        Weapon::Scimitar | Weapon::Dagger | Weapon::Sickle => Some(Mastery::Nick),
        _ => None,
    }
}
pub fn bludgeoning(weapon: Weapon) -> bool {
    matches!(weapon, Weapon::Flail | Weapon::Slam | Weapon::UnarmedStrike)
}
/// (dice count, sides) of the damage a weapon action rolls.
pub fn dice_of(kind: CreatureKind, weapon: Weapon, two_handed_melee: bool) -> (usize, u32) {
    match weapon {
        Weapon::UnarmedStrike => (0, 6),
        Weapon::Greatsword => (2, 6),
        Weapon::Flail | Weapon::Longbow | Weapon::LightCrossbow | Weapon::Slam => (1, 8),
        Weapon::Spear if two_handed_melee => (1, 8),
        Weapon::Dagger | Weapon::Sickle => (1, 4),
        Weapon::Bite if matches!(kind, CreatureKind::GiantRat | CreatureKind::GiantWeasel) => {
            (1, 4)
        }
        _ => (1, 6),
    }
}
/// Stat-block attack bonus and flat damage modifier (SRD 5.2.1 monster entries).
pub fn creature_numbers(kind: CreatureKind) -> (i16, i16) {
    match kind {
        CreatureKind::Goblin | CreatureKind::Kobold | CreatureKind::Wolf | CreatureKind::Scout => {
            (4, 2)
        }
        CreatureKind::Skeleton | CreatureKind::GiantRat | CreatureKind::GiantWeasel => (5, 3),
        _ => (3, 1),
    }
}

// ---------------------------------------------------------------------------------------------
// Movement oracle (SRD p13-14 Playing on a Grid, Difficult Terrain, Moving around Other
// Creatures; Glossary Crawling, Prone, Dash, Speed).
// ---------------------------------------------------------------------------------------------

pub fn is_slowed(c: &Combatant) -> bool {
    c.conditions().contains(&"slowed")
}
/// Speed in feet after the Slow mastery's 10-foot reduction (never below 0).
pub fn current_speed(c: &Combatant) -> u16 {
    if c.health().unconscious || c.health().dead {
        return 0;
    }
    c.spec()
        .speed_ft
        .saturating_sub(if is_slowed(c) { 10 } else { 0 })
}
/// Movement still available this turn: Speed (doubled once by Dash) minus movement spent.
pub fn allowance(c: &Combatant) -> u16 {
    (current_speed(c) * if c.budget().dash { 2 } else { 1 })
        .saturating_sub(c.budget().movement_spent_ft)
}
pub fn capable(c: &Combatant) -> bool {
    !c.health().unconscious && !c.health().dead
}
fn living_cells(b: &Battle) -> Vec<Cell> {
    b.combatants()
        .iter()
        .filter(|c| !c.health().dead)
        .map(Combatant::cell)
        .collect()
}

/// Cost in feet of one step, or why it is illegal.
pub fn step_cost(
    b: &Battle,
    g: &Grid,
    mover: &Combatant,
    from: Cell,
    to: Cell,
) -> Result<u16, &'static str> {
    if squares(from, to) != 1 {
        return Err("step-not-adjacent");
    }
    if g.blocked(to) {
        return Err("step-into-blocked-square");
    }
    if from.0 != to.0
        && from.1 != to.1
        && (g.blocked(Cell(from.0, to.1)) || g.blocked(Cell(to.0, from.1)))
    {
        return Err("diagonal-across-corner");
    }
    let mut difficult = g.difficult(to);
    for other in b.combatants() {
        if other.id() == mover.id() || other.health().dead || other.cell() != to {
            continue;
        }
        if other.side() == mover.side() {
            continue;
        }
        if !capable(other) {
            difficult = true;
        } else {
            return Err("pass-through-capable-hostile");
        }
    }
    let extra = u16::from(difficult) + u16::from(mover.health().prone);
    Ok(5 * (1 + extra))
}

pub struct MoveCost {
    pub total: u16,
    pub per_step: Vec<u16>,
}
/// Judge a whole requested path against the state `b` (mover = the active actor).
pub fn judge_path(
    b: &Battle,
    g: &Grid,
    mover: u32,
    path: &[Cell],
) -> Result<MoveCost, &'static str> {
    let mover = b.actor(mover).map_err(|_| "unknown-mover")?;
    if !capable(mover) {
        return Err("mover-incapacitated");
    }
    if path.is_empty() {
        return Err("empty-path");
    }
    let mut from = mover.cell();
    let mut per_step = Vec::new();
    let mut total = 0_u16;
    for &to in path {
        let cost = step_cost(b, g, mover, from, to)?;
        per_step.push(cost);
        total = total.saturating_add(cost);
        from = to;
    }
    if b.combatants()
        .iter()
        .any(|o| o.id() != mover.id() && !o.health().dead && o.cell() == from)
    {
        return Err("end-in-occupied-square");
    }
    if total > allowance(mover) {
        return Err("exceeds-movement");
    }
    Ok(MoveCost { total, per_step })
}

#[derive(Debug, PartialEq, Eq)]
pub enum Stop {
    Done,
    /// Paused before leaving the square at this path index, offering these reactors in order.
    Offer(usize, Vec<u32>),
    /// Mover incapacitated, fight over, or out of movement after a mid-move Slow.
    Halt(usize),
}
#[derive(Debug)]
pub struct MoveSim {
    pub cell: Cell,
    pub spent: u16,
    pub stop: Stop,
}
/// SRD p15 Opportunity Attacks: a creature that can see the mover and is not incapacitated,
/// with its Reaction available, may attack right before the mover leaves its reach.
pub fn opportunity_reactors(
    b: &Battle,
    g: &Grid,
    mover: &Combatant,
    from: Cell,
    to: Cell,
    order: &[u32],
) -> Vec<u32> {
    if mover.budget().disengaged {
        return Vec::new();
    }
    order
        .iter()
        .filter_map(|id| b.actor(*id).ok())
        .filter(|o| {
            o.side() != mover.side()
                && capable(o)
                && !o.budget().reaction_used
                && feet(o.cell(), from) <= 5
                && feet(o.cell(), to) > 5
                && g.in_sight(o.cell(), from)
        })
        .map(Combatant::id)
        .collect()
}
/// Walk `path` from index `start`. `facts` supplies every static fact (speed, reactions,
/// positions of others); the mover's own square and movement spent are passed explicitly so
/// a continuation after a reaction can use the post-reaction facts.
/// Where a simulated walk starts: the mover, its square and movement already spent, the path
/// and the index to resume from, and whether the first departure is already approved.
pub struct MoveWalk<'a> {
    pub mover: u32,
    pub cell: Cell,
    pub spent: u16,
    pub path: &'a [Cell],
    pub start: usize,
    pub skip_first_trigger: bool,
}
pub fn simulate_move(facts: &Battle, g: &Grid, walk: MoveWalk<'_>, order: &[u32]) -> MoveSim {
    let MoveWalk {
        mover,
        cell,
        spent,
        path,
        start,
        skip_first_trigger,
    } = walk;
    let Ok(m) = facts.actor(mover) else {
        return MoveSim {
            cell,
            spent,
            stop: Stop::Halt(start),
        };
    };
    let allow = (current_speed(m) * if m.budget().dash { 2 } else { 1 }) as i32;
    let (mut cell, mut spent, mut skip) = (cell, spent, skip_first_trigger);
    for (index, &to) in path.iter().enumerate().skip(start) {
        if !capable(m) || facts.finished() {
            return MoveSim {
                cell,
                spent,
                stop: Stop::Halt(index),
            };
        }
        if !skip {
            let reactors = opportunity_reactors(facts, g, m, cell, to, order);
            if !reactors.is_empty() {
                return MoveSim {
                    cell,
                    spent,
                    stop: Stop::Offer(index, reactors),
                };
            }
        }
        skip = false;
        let Ok(cost) = step_cost(facts, g, m, cell, to) else {
            return MoveSim {
                cell,
                spent,
                stop: Stop::Halt(index),
            };
        };
        if i32::from(spent) + i32::from(cost) > allow {
            return MoveSim {
                cell,
                spent,
                stop: Stop::Halt(index),
            };
        }
        spent += cost;
        cell = to;
    }
    MoveSim {
        cell,
        spent,
        stop: Stop::Done,
    }
}

// ---------------------------------------------------------------------------------------------
// Attack oracle (SRD p15 Making an Attack, Cover, Ranged Attacks, Melee Attacks, Reach).
// ---------------------------------------------------------------------------------------------

/// Positional and ownership legality of an attack, before any budget is spent.
pub fn judge_attack(
    b: &Battle,
    g: &Grid,
    actor: u32,
    target: u32,
    usage: &AttackUse,
) -> Result<(), &'static str> {
    let a = b.actor(actor).map_err(|_| "unknown-actor")?;
    let t = b.actor(target).map_err(|_| "unknown-target")?;
    if !capable(a) {
        return Err("attacker-incapacitated");
    }
    if a.side() == t.side() || a.id() == t.id() || t.health().dead {
        return Err("bad-target");
    }
    if usage.weapon != Weapon::UnarmedStrike
        && (a.weapon_count(usage.weapon) == 0
            || a.dropped_weapons().iter().any(|(w, _)| *w == usage.weapon))
    {
        return Err("weapon-not-held");
    }
    let distance = feet(a.cell(), t.cell());
    if usage.ranged {
        let Some((_, long)) = range_of(usage.weapon) else {
            return Err("weapon-has-no-range");
        };
        if distance > long {
            return Err("beyond-long-range");
        }
    } else {
        if is_bow_or_crossbow(usage.weapon) {
            return Err("ranged-weapon-in-melee");
        }
        if distance > 5 {
            return Err("beyond-reach");
        }
    }
    if g.cover(a.cell(), t.cell(), &living_cells(b)) == Cover::Total {
        return Err("total-cover");
    }
    Ok(())
}

fn expected_reasons(
    b: &Battle,
    g: &Grid,
    a: &Combatant,
    t: &Combatant,
    usage: &AttackUse,
) -> (BTreeSet<&'static str>, BTreeSet<&'static str>) {
    let mut adv = BTreeSet::new();
    let mut dis = BTreeSet::new();
    let distance = feet(a.cell(), t.cell());
    if a.health().prone {
        dis.insert("ProneAttacker");
    }
    if t.health().prone {
        if distance <= 5 {
            adv.insert("ProneTarget");
        } else {
            dis.insert("ProneTarget");
        }
    }
    if t.health().unconscious {
        adv.insert("UnconsciousTarget");
    }
    if t.budget().dodging && current_speed(t) != 0 && capable(t) && g.in_sight(t.cell(), a.cell()) {
        dis.insert("Dodge");
    }
    if a.conditions().contains(&"sapped") {
        dis.insert("Sap");
    }
    if matches!(usage.weapon, Weapon::Greatsword | Weapon::Longbow) {
        let ability = if is_bow_or_crossbow(usage.weapon) {
            1
        } else {
            0
        };
        if a.spec().abilities[ability] < 13 {
            dis.insert("Heavy");
        }
    }
    if usage.ranged {
        if let Some((normal, _)) = range_of(usage.weapon)
            && distance > normal
        {
            dis.insert("LongRange");
        }
        if b.combatants().iter().any(|o| {
            o.side() != a.side()
                && capable(o)
                && feet(o.cell(), a.cell()) <= 5
                && g.in_sight(o.cell(), a.cell())
        }) {
            dis.insert("HostileNearby");
        }
    }
    if a.spec().kind == CreatureKind::Kobold && g.sunlight {
        dis.insert("SunlightSensitivity");
    }
    if matches!(
        a.spec().kind,
        CreatureKind::Kobold | CreatureKind::Wolf | CreatureKind::GiantRat
    ) && b.combatants().iter().any(|ally| {
        ally.id() != a.id()
            && ally.side() == a.side()
            && capable(ally)
            && feet(ally.cell(), t.cell()) <= 5
    }) {
        adv.insert("PackTactics");
    }
    (adv, dis)
}
fn reason_name(reason: AttackReason) -> &'static str {
    match reason {
        AttackReason::LongRange => "LongRange",
        AttackReason::HostileNearby => "HostileNearby",
        AttackReason::Heavy => "Heavy",
        AttackReason::ProneAttacker => "ProneAttacker",
        AttackReason::ProneTarget => "ProneTarget",
        AttackReason::UnconsciousTarget => "UnconsciousTarget",
        AttackReason::Dodge => "Dodge",
        AttackReason::PackTactics => "PackTactics",
        AttackReason::SunlightSensitivity => "SunlightSensitivity",
        AttackReason::Vex => "Vex",
        AttackReason::Sap => "Sap",
    }
}

// ---------------------------------------------------------------------------------------------
// The fight auditor.
// ---------------------------------------------------------------------------------------------

/// What the attack's own resolution fixed about its target's consequences.
struct TargetFacts<'a> {
    damage: i32,
    critical: bool,
    usage: &'a AttackUse,
    fortitude_roll: bool,
}

struct PendingMove {
    mover: u32,
    /// Reactors still to answer the open window, in offer order.
    offers: Vec<u32>,
    path: Vec<Cell>,
    index: usize,
    origin: Cell,
    spent: u16,
}

pub struct FightAudit<'g> {
    pub audit: Audit,
    grid: &'g Grid,
    order: Vec<u32>,
    expected_active: usize,
    expected_round: u32,
    pending: Option<PendingMove>,
    last_mover: Option<u32>,
    save_owed: Option<u32>,
    turn_actions: u8,
    turn_multi: u8,
    turn_light: u8,
    turn_bonus: u8,
    turn_saves: u8,
    turn_savage_hits: u8,
    turn_reactions: BTreeMap<u32, u8>,
    pub turns: u64,
}

impl<'g> FightAudit<'g> {
    /// Check Initiative (SRD p13): one d20 + Dexterity modifier per participant, one shared roll
    /// per group of identical creatures, order high to low, ties by the agreed fixed order.
    pub fn begin(
        initial: &Battle,
        grid: &'g Grid,
        specs_in_order: &[(u32, CreatureKind, u8)],
        begin_draws: &[ActualDraw],
    ) -> Self {
        let mut audit = Audit::default();
        let mut faces: Vec<u32> = Vec::new();
        let mut taken = 0;
        for (index, (_, kind, _)) in specs_in_order.iter().enumerate() {
            if *kind != CreatureKind::Hero
                && let Some(earlier) = specs_in_order[..index].iter().position(|s| s.1 == *kind)
            {
                faces.push(faces[earlier]);
                continue;
            }
            match begin_draws.get(taken) {
                Some(draw) => {
                    audit.expect(draw.sides == 20, "I-INIT-DIE", || {
                        format!("initiative die d{}", draw.sides)
                    });
                    let mut face = draw.value;
                    // Initiative is a Dexterity check; a Kobold in sunlight has Disadvantage on
                    // ability checks (SRD 5.2.1 p302), so its group rolls twice and keeps the lower.
                    if *kind == CreatureKind::Kobold && grid.sunlight {
                        taken += 1;
                        face = face.min(begin_draws.get(taken).map_or(face, |d| d.value));
                        audit.tally("sunlight-kobold-initiatives");
                    }
                    faces.push(face);
                }
                None => {
                    audit.fail("I-INIT-DRAWS", "fewer initiative rolls than groups".into());
                    faces.push(1);
                }
            }
            taken += 1;
        }
        audit.expect(taken == begin_draws.len(), "I-INIT-DRAWS", || {
            format!("{} rolls for {taken} groups", begin_draws.len())
        });
        let mut keyed: Vec<(i32, usize, u32)> = specs_in_order
            .iter()
            .enumerate()
            .map(|(i, (id, _, dex))| (faces[i] as i32 + i32::from(modifier(*dex)), i, *id))
            .collect();
        keyed.sort_by_key(|(total, index, _)| (std::cmp::Reverse(*total), *index));
        let expected: Vec<u32> = keyed.iter().map(|k| k.2).collect();
        audit.expect(
            initial.initiative_order() == expected.as_slice(),
            "I-INIT-ORDER",
            || {
                format!(
                    "engine {:?} vs recomputed {expected:?}",
                    initial.initiative_order()
                )
            },
        );
        audit.expect(initial.round() == 1, "T-ROUND", || {
            "fight does not start at round 1".into()
        });
        audit.expect(
            initial.active_actor() == expected.first().copied(),
            "T-ACTIVE",
            || "first actor is not the highest initiative".into(),
        );
        audit.tally("initiative-orders-checked");
        FightAudit {
            audit,
            grid,
            order: expected,
            expected_active: 0,
            expected_round: 1,
            pending: None,
            last_mover: None,
            save_owed: None,
            turn_actions: 0,
            turn_multi: 0,
            turn_light: 0,
            turn_bonus: 0,
            turn_saves: 0,
            turn_savage_hits: 0,
            turn_reactions: BTreeMap::new(),
            turns: 1,
        }
    }

    /// Audit one step the engine accepted.
    pub fn accepted(&mut self, before: &Battle, actor: u32, action: &Action, res: &Resolution) {
        let after = &res.battle;
        self.audit.tally("accepted-steps");
        let order = self.order.clone();
        self.audit.expect(
            after.initiative_order() == order.as_slice(),
            "T-ORDER",
            || "initiative order changed during the fight".into(),
        );
        // Who may act: only the active creature, or the first offered reactor on a reaction.
        match action {
            Action::Opportunity { .. } => {
                let offered = before.reaction_offers().first().copied();
                self.audit.expect(
                    offered == Some(actor) && self.pending.is_some(),
                    "T-OUT-OF-TURN",
                    || format!("reaction by {actor} not offered ({offered:?})"),
                );
            }
            Action::ShortRest { .. }
            | Action::RecoverAmmunition { .. }
            | Action::StableRecovery => {}
            _ => {
                self.audit.expect(
                    before.reaction_offers().is_empty()
                        && before.active_actor() == Some(actor)
                        && order.get(self.expected_active) == Some(&actor),
                    "T-OUT-OF-TURN",
                    || {
                        format!(
                            "{actor} acted but active is {:?}",
                            order.get(self.expected_active)
                        )
                    },
                );
            }
        }
        // A dying hero's turn begins with exactly one Death Saving Throw (SRD p17-18).
        if let Some(owed) = self.save_owed {
            if matches!(action, Action::DeathSave) {
                self.save_owed = None;
            } else {
                self.audit.fail(
                    "T-DEATH-SAVE-SKIPPED",
                    format!(
                        "{owed} began a turn at 0 HP with no Death Saving Throw before {action:?}"
                    ),
                );
                self.save_owed = None;
            }
        }
        if let Some(a) = before.actor(actor).ok()
            && !capable(a)
            && !matches!(
                action,
                Action::EndTurn
                    | Action::DeathSave
                    | Action::StableRecovery
                    | Action::ShortRest { .. }
                    | Action::RecoverAmmunition { .. }
            )
        {
            self.audit.fail(
                "T-INCAPACITATED-ACTS",
                format!("{actor} acted while incapacitated: {action:?}"),
            );
        }
        self.reaction_flags(before, after, actor, action);
        match action {
            Action::Move(path) => self.audit_move_start(before, actor, path, after),
            Action::Opportunity { attack } => self.audit_reaction(before, actor, attack, res),
            Action::Stand => self.audit_stand(before, actor, after),
            Action::DropProne => {
                let (b, a) = (before.actor(actor).ok(), after.actor(actor).ok());
                if let (Some(b), Some(a)) = (b, a) {
                    self.audit.expect(
                        current_speed(b) > 0 && a.health().prone,
                        "M-DROP-PRONE",
                        || "drop prone with Speed 0 or not applied".into(),
                    );
                    self.audit
                        .expect(a.budget() == b.budget(), "M-DROP-PRONE", || {
                            "dropping prone cost action or movement".into()
                        });
                }
            }
            Action::Dash | Action::Disengage | Action::Dodge | Action::DoffShield => {
                self.audit_action_flag(before, actor, action, after)
            }
            Action::NimbleDisengage => {
                self.take_bonus(before, actor);
            }
            Action::Attack {
                target,
                weapon,
                spend,
            } => self.audit_attack_step(before, actor, *target, weapon, *spend, res),
            Action::SecondWind => self.audit_second_wind(before, actor, res),
            Action::EndTurn => self.audit_end_turn(before, actor, after),
            Action::DeathSave => self.audit_death_save(before, actor, res),
            Action::StableRecovery => self.audit_stable_recovery(before, actor, res),
            Action::ShortRest { elapsed_seconds } => {
                self.audit_short_rest(before, actor, *elapsed_seconds, res)
            }
            Action::RecoverAmmunition { .. } | Action::PickUpWeapon(_) => {}
        }
        self.audit_untouched(before, after, actor, action);
    }

    /// The rest of the fight must be quiet: at fight end, no death save may be owed.
    pub fn finish(&mut self, last: &Battle) {
        if let Some(owed) = self.save_owed
            && !last.finished()
        {
            self.audit.fail(
                "T-DEATH-SAVE-SKIPPED",
                format!("{owed} owed a save at end of record"),
            );
        }
    }

    fn reaction_flags(&mut self, before: &Battle, after: &Battle, actor: u32, action: &Action) {
        for c in before.combatants() {
            let was = c.budget().reaction_used;
            let Ok(now) = after.actor(c.id()).map(|n| n.budget().reaction_used) else {
                continue;
            };
            let took = matches!(action, Action::Opportunity { attack: Some(_) }) && c.id() == actor;
            let refreshed = matches!(action, Action::EndTurn)
                && after.active_actor().is_some_and(|next| next == c.id());
            let ok = (was == now) || (!was && now && took) || (was && !now && refreshed);
            self.audit.expect(ok, "T-REACTION", || {
                format!("{}: reaction_used {was} -> {now} on {action:?}", c.id())
            });
            if !was && now {
                *self.turn_reactions.entry(c.id()).or_default() += 1;
                self.audit.tally("reactions-taken");
            }
        }
        for (id, count) in &self.turn_reactions {
            let over = *count > 1;
            if over {
                self.audit.violations.push(Violation {
                    id: "T-REACTION",
                    detail: format!("{id} took {count} reactions since its own turn started"),
                });
            }
        }
        self.turn_reactions.retain(|_, count| *count <= 1);
    }

    fn take_bonus(&mut self, before: &Battle, actor: u32) {
        self.turn_bonus += 1;
        let used = before
            .actor(actor)
            .map(|a| a.budget().bonus_action_used)
            .unwrap_or(true);
        self.audit.expect(
            !used && self.turn_bonus <= 1,
            "T-SECOND-BONUS-ACTION",
            || format!("{actor} took a second Bonus Action this turn"),
        );
    }
    fn take_action(&mut self, before: &Battle, actor: u32, what: &str) {
        self.turn_actions += 1;
        let used = before
            .actor(actor)
            .map(|a| a.budget().action_used)
            .unwrap_or(true);
        self.audit
            .expect(!used && self.turn_actions <= 1, "T-SECOND-ACTION", || {
                format!("{actor} took a second action this turn ({what})")
            });
    }

    fn audit_action_flag(&mut self, before: &Battle, actor: u32, action: &Action, after: &Battle) {
        self.take_action(before, actor, &format!("{action:?}"));
        let (Ok(b), Ok(a)) = (before.actor(actor), after.actor(actor)) else {
            return;
        };
        let flag = match action {
            Action::Dash => a.budget().dash && !b.budget().dash,
            Action::Disengage => a.budget().disengaged,
            Action::Dodge => a.budget().dodging,
            _ => true,
        };
        self.audit.expect(flag, "M-ACTION-EFFECT", || {
            format!("{action:?} did not set its effect")
        });
        if matches!(action, Action::Dash) {
            self.audit.expect(
                allowance(a) == allowance(b) + current_speed(b),
                "M-DASH",
                || {
                    format!(
                        "Dash must add Speed once: {} -> {}",
                        allowance(b),
                        allowance(a)
                    )
                },
            );
            self.audit.tally("dashes");
        }
    }

    fn audit_stand(&mut self, before: &Battle, actor: u32, after: &Battle) {
        let (Ok(b), Ok(a)) = (before.actor(actor), after.actor(actor)) else {
            return;
        };
        let cost = current_speed(b) / 2;
        self.audit.expect(
            b.health().prone && !a.health().prone && current_speed(b) > 0,
            "M-STAND",
            || "stand requires Prone and Speed above 0".into(),
        );
        self.audit.expect(
            a.budget().movement_spent_ft == b.budget().movement_spent_ft + cost
                && cost <= allowance(b),
            "M-STAND-COST",
            || {
                format!(
                    "standing costs half Speed ({cost} ft); spent {} -> {}",
                    b.budget().movement_spent_ft,
                    a.budget().movement_spent_ft
                )
            },
        );
        self.audit.tally("stand-ups");
    }

    fn audit_move_start(&mut self, before: &Battle, actor: u32, path: &[Cell], after: &Battle) {
        self.audit.tally("moves");
        self.last_mover = Some(actor);
        let Ok(b) = before.actor(actor) else { return };
        let cost = match judge_path(before, self.grid, actor, path) {
            Ok(cost) => cost,
            Err(reason) => {
                self.audit.fail(
                    "M-ACCEPTED-ILLEGAL-PATH",
                    format!("{actor} path {path:?} accepted but {reason}"),
                );
                return;
            }
        };
        let order = self.order.clone();
        let sim = simulate_move(
            before,
            self.grid,
            MoveWalk {
                mover: actor,
                cell: b.cell(),
                spent: b.budget().movement_spent_ft,
                path,
                start: 0,
                skip_first_trigger: false,
            },
            &order,
        );
        self.check_move_result(before, after, actor, path, &sim, cost.total);
        if let Stop::Offer(index, reactors) = &sim.stop {
            self.pending = Some(PendingMove {
                mover: actor,
                offers: reactors.clone(),
                path: path.to_vec(),
                index: *index,
                origin: sim.cell,
                spent: sim.spent,
            });
            self.audit.tally("opportunity-windows");
            self.audit.expect(
                after.reaction_offers() == reactors.as_slice(),
                "M-OA-OFFERS",
                || {
                    format!(
                        "offers {:?} vs recomputed {reactors:?}",
                        after.reaction_offers()
                    )
                },
            );
        } else {
            self.pending = None;
        }
    }

    fn check_move_result(
        &mut self,
        before: &Battle,
        after: &Battle,
        actor: u32,
        path: &[Cell],
        sim: &MoveSim,
        full_cost: u16,
    ) {
        let (Ok(b), Ok(a)) = (before.actor(actor), after.actor(actor)) else {
            return;
        };
        self.audit.expect(a.cell() == sim.cell, "M-POSITION", || {
            format!(
                "{actor} ends at {:?}; recomputed {:?} ({:?}) for path {path:?}; before: {}",
                a.cell(),
                sim.cell,
                sim.stop,
                dump(before)
            )
        });
        self.audit.expect(a.budget().movement_spent_ft == sim.spent, "M-COST", || {
            format!(
                "{actor} spent {} ft; recomputed {} ft for path {path:?} (full path {full_cost} ft)",
                a.budget().movement_spent_ft, sim.spent
            )
        });
        self.audit.expect(
            a.budget().movement_spent_ft <= current_speed(b) * if b.budget().dash { 2 } else { 1 }
                || a.budget().movement_spent_ft == b.budget().movement_spent_ft,
            "M-BUDGET",
            || "movement exceeds Speed".into(),
        );
        if matches!(sim.stop, Stop::Done) {
            self.audit.expect(
                sim.spent <= b.budget().movement_spent_ft + allowance(b),
                "M-BUDGET",
                || "completed path overruns the movement left".into(),
            );
            self.audit.tally("moves-completed");
        }
        // Every square on the way must be plausible movement, not teleportation.
        let moved = squares(b.cell(), a.cell());
        let max_squares = u32::from((sim.spent - b.budget().movement_spent_ft) / 5);
        self.audit.expect(moved <= max_squares, "M-TELEPORT", || {
            format!(
                "moved {moved} squares for {} ft",
                sim.spent - b.budget().movement_spent_ft
            )
        });
    }

    fn audit_reaction(
        &mut self,
        before: &Battle,
        actor: u32,
        attack: &Option<AttackUse>,
        res: &Resolution,
    ) {
        let after = &res.battle;
        let Some(pending) = self.pending.take() else {
            return;
        };
        let mover = pending.mover;
        if let Some(usage) = attack {
            self.audit.expect(!usage.ranged, "M-OA-MELEE-ONLY", || {
                "Opportunity Attack must be a melee attack (p15)".into()
            });
            self.audit
                .expect(res.attacks.len() == 1, "M-OA-ATTACKS", || {
                    "one reaction, one attack".into()
                });
            if let Some(summary) = res.attacks.first() {
                self.audit_attack_summary(
                    before,
                    summary,
                    usage,
                    AttackSpend::Action,
                    &res.draws,
                    after,
                );
                self.audit.expect(
                    summary.actor == actor && summary.target == mover,
                    "M-OA-TARGET",
                    || "OA actor/target".into(),
                );
                self.audit
                    .expect(summary.distance_ft <= 5, "M-OA-REACH", || {
                        format!("OA from {} ft", summary.distance_ft)
                    });
            }
            self.audit.tally("opportunity-attacks");
        } else {
            self.audit
                .expect(res.attacks.is_empty(), "M-OA-ATTACKS", || {
                    "declined reaction attacked".into()
                });
            self.audit.tally("opportunity-declines");
        }
        // Continue the interrupted path once every offered reactor has answered.
        let mut pending = pending;
        if pending.offers.first() == Some(&actor) {
            pending.offers.remove(0);
        }
        let Ok(a) = after.actor(mover) else { return };
        if !capable(a) || after.finished() {
            // SRD p15: the reaction occurs right before the mover leaves; if it stops the mover
            // the rest of the move, and every remaining offer, is void.
            self.audit.expect(
                after.reaction_offers().is_empty() && a.cell() == pending.origin,
                "M-OA-CANCEL",
                || "mover stopped but the window stayed open or it moved on".into(),
            );
            self.audit.tally("opportunity-cancelled-moves");
            return;
        }
        if pending.offers.is_empty() {
            let order = self.order.clone();
            let sim = simulate_move(
                after,
                self.grid,
                MoveWalk {
                    mover,
                    cell: pending.origin,
                    spent: pending.spent,
                    path: &pending.path,
                    start: pending.index,
                    skip_first_trigger: true,
                },
                &order,
            );
            self.check_move_result(before, after, mover, &pending.path, &sim, 0);
            let expected_offers = match &sim.stop {
                Stop::Offer(_, reactors) => reactors.clone(),
                _ => Vec::new(),
            };
            self.audit.expect(
                after.reaction_offers() == expected_offers.as_slice(),
                "M-OA-OFFERS",
                || {
                    format!(
                        "continuation offers {:?} vs recomputed {expected_offers:?}",
                        after.reaction_offers()
                    )
                },
            );
            if let Stop::Offer(index, reactors) = sim.stop {
                self.pending = Some(PendingMove {
                    mover,
                    offers: reactors,
                    path: pending.path,
                    index,
                    origin: sim.cell,
                    spent: sim.spent,
                });
            }
        } else {
            self.audit
                .expect(a.cell() == pending.origin, "M-OA-BEFORE-EXIT", || {
                    "mover left reach before every reactor answered".into()
                });
            self.audit.expect(
                after.reaction_offers() == pending.offers.as_slice(),
                "M-OA-OFFERS",
                || "remaining reactors differ".into(),
            );
            self.pending = Some(pending);
        }
    }

    fn audit_second_wind(&mut self, before: &Battle, actor: u32, res: &Resolution) {
        self.take_bonus(before, actor);
        let (Ok(b), Ok(a)) = (before.actor(actor), res.battle.actor(actor)) else {
            return;
        };
        let die = res.draws.first().map_or(0, |d| d.value);
        let sides = res.draws.first().map_or(0, |d| d.sides);
        self.audit.expect(
            b.spec().second_wind_uses > 0
                && a.spec().second_wind_uses + 1 == b.spec().second_wind_uses
                && sides == 10
                && u32::from(a.health().hit_points)
                    == (u32::from(b.health().hit_points) + die + 1)
                        .min(u32::from(b.health().maximum)),
            "F-SECOND-WIND",
            || {
                format!(
                    "Second Wind is 1d10 + level 1, one use, capped at maximum; got {:?} -> {:?}",
                    b.health().hit_points,
                    a.health().hit_points
                )
            },
        );
        self.audit.tally("second-winds");
    }

    /// SRD 5.2.1 p47-48 Second Wind: a Short Rest (an hour, started with at least 1 Hit Point)
    /// recharges one expended use, never above the two a level 1 Fighter has.
    fn audit_short_rest(&mut self, before: &Battle, actor: u32, seconds: u32, res: &Resolution) {
        let (Ok(b), Ok(a)) = (before.actor(actor), res.battle.actor(actor)) else {
            return;
        };
        self.audit.expect(
            seconds >= 3600 && b.health().hit_points > 0 && !b.health().dead,
            "F-SHORT-REST",
            || "a Short Rest lasts an hour and starts with at least 1 hit point".into(),
        );
        if b.spec().kind == CreatureKind::Hero {
            self.audit.expect(
                a.spec().second_wind_uses == (b.spec().second_wind_uses + 1).min(2),
                "F-SECOND-WIND-RECHARGE",
                || "a Short Rest recharges exactly one Second Wind use up to two".into(),
            );
            self.audit.tally("short-rests");
        }
    }

    /// SRD p18: only a Stable hero at 0 recovers, to exactly 1 Hit Point, after the d4 hours.
    fn audit_stable_recovery(&mut self, before: &Battle, actor: u32, res: &Resolution) {
        let (Ok(b), Ok(a)) = (before.actor(actor), res.battle.actor(actor)) else {
            return;
        };
        let hours = res.draws.first().map(|d| (d.sides, d.value));
        self.audit.expect(
            b.health().stable
                && b.health().hit_points == 0
                && a.health().hit_points == 1
                && !a.health().stable
                && !a.health().unconscious
                && a.health().death_failures == 0
                && a.health().death_successes == 0
                && hours.is_some_and(|(sides, value)| {
                    sides == 4 && res.recoveries.first().is_some_and(|r| r.hours == value)
                }),
            "F-STABLE-RECOVERY",
            || "a Stable creature regains 1 Hit Point after 1d4 hours, and only that".into(),
        );
        self.audit.tally("stable-recoveries");
    }

    fn audit_death_save(&mut self, before: &Battle, actor: u32, res: &Resolution) {
        self.turn_saves += 1;
        let (Ok(b), Ok(a)) = (before.actor(actor), res.battle.actor(actor)) else {
            return;
        };
        let (bh, ah) = (b.health(), a.health());
        self.audit.expect(
            bh.hit_points == 0
                && !bh.dead
                && !bh.stable
                && b.spec().kind == CreatureKind::Hero
                && self.turn_saves == 1,
            "T-DEATH-SAVE-EXTRA",
            || {
                "Death Saving Throw outside the start of a dying hero's turn, or more than one"
                    .into()
            },
        );
        let die = res.draws.first().map_or(0, |d| d.value);
        let (mut s, mut f) = (bh.death_successes, bh.death_failures);
        let mut hp = 0;
        match die {
            20 => {
                hp = 1;
                s = 0;
                f = 0;
            }
            1 => f = (f + 2).min(3),
            2..=9 => f = (f + 1).min(3),
            _ => s += 1,
        }
        let mut stable = false;
        if s == 3 {
            stable = true;
            s = 0;
            f = 0;
        }
        let dead = f >= 3;
        self.audit.expect(
            ah.hit_points == hp
                && ah.death_successes == s
                && ah.death_failures == f
                && ah.stable == stable
                && ah.dead == dead
                && (die != 20 || !ah.unconscious),
            "F-DEATH-SAVE-ARITHMETIC",
            || {
                format!(
                    "d20={die}: expected hp {hp} s{s} f{f} stable {stable} dead {dead}; got {ah:?}"
                )
            },
        );
        self.audit.tally("death-saves");
    }

    fn audit_end_turn(&mut self, before: &Battle, actor: u32, after: &Battle) {
        let count = self.order.len();
        let mut next = self.expected_active + 1;
        if next == count {
            next = 0;
            self.expected_round += 1;
        }
        self.expected_active = next;
        self.audit
            .expect(after.round() == self.expected_round, "T-ROUND", || {
                format!("round {} expected {}", after.round(), self.expected_round)
            });
        self.audit.expect(
            after.active_actor() == self.order.get(next).copied(),
            "T-ACTIVE",
            || {
                format!(
                    "next active {:?} expected {:?}",
                    after.active_actor(),
                    self.order.get(next)
                )
            },
        );
        self.audit.expect(
            before.reaction_offers().is_empty() && self.pending.is_none(),
            "T-END-WITH-PENDING",
            || "turn ended with a movement window open".into(),
        );
        if let Some(n) = after.active_actor().and_then(|id| after.actor(id).ok()) {
            let fresh = n.budget();
            self.audit.expect(
                !fresh.action_used
                    && !fresh.bonus_action_used
                    && !fresh.reaction_used
                    && fresh.movement_spent_ft == 0
                    && !fresh.dash
                    && !fresh.disengaged
                    && !fresh.dodging
                    && !fresh.object_interaction_used,
                "T-BUDGET-RESET",
                || format!("{} starts its turn with a spent budget {fresh:?}", n.id()),
            );
            let owes = n.health().hit_points == 0 && !n.health().dead && !n.health().stable;
            self.audit.expect(
                after.needs_death_save() == owes,
                "T-DEATH-SAVE-FLAG",
                || {
                    format!(
                        "needs_death_save {} but dying-hero rule says {owes}",
                        after.needs_death_save()
                    )
                },
            );
            if owes {
                self.save_owed = Some(n.id());
            }
        }
        self.turn_actions = 0;
        self.turn_multi = 0;
        self.turn_light = 0;
        self.turn_bonus = 0;
        self.turn_saves = 0;
        self.turn_savage_hits = 0;
        if let Some(next_id) = after.active_actor() {
            self.turn_reactions.remove(&next_id);
        }
        self.turns += 1;
        self.audit.tally("turns");
        let _ = actor;
    }

    fn audit_attack_step(
        &mut self,
        before: &Battle,
        actor: u32,
        target: u32,
        usage: &AttackUse,
        spend: AttackSpend,
        res: &Resolution,
    ) {
        match spend {
            AttackSpend::Action => self.take_action(before, actor, "Attack"),
            AttackSpend::Multiattack => {
                self.turn_multi += 1;
                self.audit.expect(
                    self.turn_multi <= 1
                        && before
                            .actor(actor)
                            .is_ok_and(|a| a.spec().kind == CreatureKind::Scout)
                        && self.turn_actions == 1,
                    "T-EXTRA-ATTACK",
                    || "Multiattack is the Scout's second attack of one Attack action".into(),
                );
            }
            AttackSpend::LightBonus => {
                self.turn_light += 1;
                self.take_bonus(before, actor);
                self.audit.expect(
                    self.turn_light <= 1 && is_light(usage.weapon),
                    "T-EXTRA-ATTACK",
                    || "Light bonus attack: once, Light weapon".into(),
                );
            }
            AttackSpend::Nick => {
                self.turn_light += 1;
                self.audit.expect(
                    self.turn_light <= 1
                        && usage.use_mastery
                        && usage.weapon != Weapon::UnarmedStrike
                        && mastery_of(usage.weapon) == Some(Mastery::Nick),
                    "T-EXTRA-ATTACK",
                    || "Nick extra attack: once, Nick weapon with the mastery chosen".into(),
                );
            }
        }
        self.audit
            .expect(res.attacks.len() == 1, "A-ONE-ATTACK", || {
                format!(
                    "{} attacks resolved for one attack action",
                    res.attacks.len()
                )
            });
        if let Some(summary) = res.attacks.first() {
            self.audit.expect(
                summary.actor == actor && summary.target == target,
                "A-IDENTITY",
                || "summary names other creatures".into(),
            );
            self.audit_attack_summary(before, summary, usage, spend, &res.draws, &res.battle);
        }
    }

    /// Recompute one attack from first principles and compare with the engine's summary.
    pub fn audit_attack_summary(
        &mut self,
        before: &Battle,
        s: &AttackSummary,
        usage: &AttackUse,
        spend: AttackSpend,
        draws: &[ActualDraw],
        after: &Battle,
    ) {
        self.audit.tally("attacks");
        let (Ok(a), Ok(t)) = (before.actor(s.actor), before.actor(s.target)) else {
            return;
        };
        let g = self.grid;
        if let Err(reason) = judge_attack(before, g, s.actor, s.target, usage) {
            self.audit.fail(
                "A-ACCEPTED-ILLEGAL",
                format!(
                    "{} -> {} {:?} accepted but {reason}",
                    s.actor, s.target, usage.weapon
                ),
            );
            return;
        }
        let distance = feet(a.cell(), t.cell());
        self.audit
            .expect(s.distance_ft == distance, "A-DISTANCE", || {
                format!("reported {} ft, grid says {distance} ft", s.distance_ft)
            });
        self.audit.expect(s.ranged == usage.ranged, "A-MODE", || {
            "melee/ranged flag".into()
        });
        if usage.ranged {
            self.audit.tally("ranged-attacks");
        } else {
            self.audit.tally("melee-attacks");
            self.audit.expect(distance <= 5, "A-REACH", || {
                format!("melee attack at {distance} ft")
            });
        }
        if distance > 5 && !usage.ranged {
            self.audit.fail("A-REACH", "melee past reach".into());
        }
        // Ammunition: one per shot, none when empty (p89).
        if usage.ranged && is_bow_or_crossbow(usage.weapon) {
            let have = if usage.weapon == Weapon::LightCrossbow {
                a.bolts()
            } else {
                a.arrows()
            };
            self.audit.expect(have > 0, "A-AMMUNITION", || {
                "ranged ammunition weapon fired with none".into()
            });
        }
        // Cover.
        let cover = g.cover(a.cell(), t.cell(), &living_cells(before));
        self.audit.expect(s.cover == cover, "A-COVER", || {
            format!("cover reported {:?}, grid says {cover:?}", s.cover)
        });
        let bonus = match cover {
            Cover::Half => 2,
            Cover::ThreeQuarters => 5,
            _ => 0,
        };
        self.audit.expect(
            s.armor_class == t.spec().armor_class + bonus,
            "A-ARMOR-CLASS",
            || {
                format!(
                    "AC {} vs {} + cover {bonus}",
                    s.armor_class,
                    t.spec().armor_class
                )
            },
        );
        // Attack bonus.
        let hero = a.spec().kind == CreatureKind::Hero;
        let dex_attack =
            (usage.ranged && is_bow_or_crossbow(usage.weapon)) || usage.finesse_dexterity;
        let ability = if dex_attack {
            modifier(a.spec().abilities[1])
        } else {
            modifier(a.spec().abilities[0])
        };
        let (bonus_expected, flat) = if hero {
            (
                ability
                    + 2
                    + if a.spec().archery && usage.ranged && is_bow_or_crossbow(usage.weapon) {
                        2
                    } else {
                        0
                    },
                ability,
            )
        } else if usage.weapon == Weapon::UnarmedStrike {
            (ability + 2, ability)
        } else {
            let (atk, dmg) = creature_numbers(a.spec().kind);
            (atk, dmg)
        };
        self.audit
            .expect(s.modifier == bonus_expected, "A-MODIFIER", || {
                format!("attack bonus {} vs {bonus_expected}", s.modifier)
            });
        // Advantage and Disadvantage never stack; any of each cancels (p7-8).
        let (adv, dis) = expected_reasons(before, g, a, t, usage);
        let reported_adv: BTreeSet<&str> = s.advantage.iter().map(|r| reason_name(*r)).collect();
        let reported_dis: BTreeSet<&str> = s.disadvantage.iter().map(|r| reason_name(*r)).collect();
        let private = ["Vex"];
        for want in &adv {
            self.audit
                .expect(reported_adv.contains(want), "A-ADVANTAGE", || {
                    format!("missing Advantage reason {want}")
                });
        }
        for want in &dis {
            self.audit
                .expect(reported_dis.contains(want), "A-DISADVANTAGE", || {
                    format!("missing Disadvantage reason {want}")
                });
        }
        for got in &reported_adv {
            self.audit.expect(
                adv.contains(got) || private.contains(got),
                "A-ADVANTAGE",
                || format!("unsupported Advantage reason {got}"),
            );
        }
        for got in &reported_dis {
            self.audit.expect(dis.contains(got), "A-DISADVANTAGE", || {
                format!("unsupported Disadvantage reason {got}")
            });
        }
        let mode: i8 = match (!reported_adv.is_empty(), !reported_dis.is_empty()) {
            (true, false) => 1,
            (false, true) => -1,
            _ => 0,
        };
        self.audit.expect(s.roll_mode == mode, "A-ROLL-MODE", || {
            format!("roll mode {} but reasons say {mode}", s.roll_mode)
        });
        let dice = if mode == 0 { 1 } else { 2 };
        self.audit.expect(
            s.faces.len() == dice && draws.len() >= dice,
            "A-ROLL-MODE",
            || format!("{} d20 for mode {mode}", s.faces.len()),
        );
        let faces: Vec<u32> = draws.iter().take(dice).map(|d| d.value).collect();
        self.audit.expect(faces == s.faces, "A-FACES", || {
            "recorded faces differ from the draws".into()
        });
        let chosen = match mode {
            1 => faces.iter().copied().max(),
            -1 => faces.iter().copied().min(),
            _ => faces.first().copied(),
        }
        .unwrap_or(0);
        self.audit.expect(s.selected == chosen, "A-SELECTED", || {
            format!("selected {} of {faces:?} in mode {mode}", s.selected)
        });
        if mode > 0 {
            self.audit.tally("advantage-rolls");
        }
        if mode < 0 {
            self.audit.tally("disadvantage-rolls");
        }
        // Hit and critical (p7 natural 1/20; p16 Critical Hits; unconscious within 5 ft).
        let total = chosen as i32 + i32::from(bonus_expected);
        let hit = chosen != 1 && (chosen == 20 || total >= i32::from(t.spec().armor_class + bonus));
        self.audit.expect(s.hit == hit, "A-HIT", || {
            format!(
                "d20 {chosen} + {bonus_expected} vs AC {}: hit {hit}",
                t.spec().armor_class + bonus
            )
        });
        let critical = hit && (chosen == 20 || (t.health().unconscious && distance <= 5));
        self.audit.expect(s.critical == critical, "A-CRITICAL", || {
            format!("critical expected {critical}")
        });
        if critical {
            self.audit.tally("critical-hits");
        }
        // Damage dice: critical doubles dice, not modifiers; Savage Attacker doubles the roll
        // once per turn; Goblin rider on Advantage.
        let multiplier = if critical { 2 } else { 1 };
        let two_hands_melee = hero && usage.hands == 2 && !usage.ranged;
        let (count, sides) = dice_of(a.spec().kind, usage.weapon, two_hands_melee);
        let damage_draws = &draws[dice.min(draws.len())..];
        let mut expected_draw_count = 0;
        let mut expected_damage: i32 = 0;
        if hit {
            let mut first = i32::from(usage.weapon == Weapon::UnarmedStrike);
            let mut taken = 0;
            for _ in 0..count * multiplier {
                first += damage_draws.get(taken).map_or(0, |d| d.value as i32);
                taken += 1;
            }
            if usage.savage_attacker {
                let mut second = i32::from(usage.weapon == Weapon::UnarmedStrike);
                for _ in 0..count * multiplier {
                    second += damage_draws.get(taken).map_or(0, |d| d.value as i32);
                    taken += 1;
                }
                first = first.max(second);
                self.turn_savage_hits += 1;
                self.audit
                    .expect(self.turn_savage_hits <= 1, "F-SAVAGE-ATTACKER", || {
                        "Savage Attacker used twice in one turn (p87)".into()
                    });
                self.audit
                    .expect(a.spec().savage_attacker, "F-SAVAGE-ATTACKER", || {
                        "Savage Attacker without the feat".into()
                    });
                self.audit.tally("savage-attacker-hits");
            }
            let mut modifier_total = if matches!(spend, AttackSpend::LightBonus | AttackSpend::Nick)
            {
                flat.min(0)
            } else {
                flat
            };
            if !hero && usage.weapon != Weapon::UnarmedStrike {
                modifier_total = flat;
            }
            expected_damage = first + i32::from(modifier_total);
            if a.spec().kind == CreatureKind::Goblin
                && mode > 0
                && matches!(usage.weapon, Weapon::Scimitar | Weapon::Shortbow)
            {
                for _ in 0..multiplier {
                    expected_damage += damage_draws.get(taken).map_or(0, |d| d.value as i32);
                    taken += 1;
                }
            }
            expected_damage = expected_damage.max(0);
            expected_draw_count = taken;
            let _ = sides;
        } else if s.graze {
            expected_damage = i32::from(flat.max(0));
        }
        if t.spec().kind == CreatureKind::Skeleton && bludgeoning(usage.weapon) {
            expected_damage *= 2;
        }
        self.audit.expect(i32::from(s.damage) == expected_damage, "A-DAMAGE", || {
            format!("damage {} but dice {damage_draws:?} give {expected_damage} (crit {critical}, hit {hit})", s.damage)
        });
        // Zombie Undead Fortitude spends one more d20 only when a non-critical blow would drop it.
        let extra = draws.len().saturating_sub(dice + expected_draw_count);
        self.audit.expect(extra <= 1, "A-DRAWS", || {
            format!("{extra} unexplained draws")
        });
        // A thrown weapon leaves the hand: one fewer carried (stat block Gear: Daggers (3)).
        if usage.ranged && !is_bow_or_crossbow(usage.weapon) {
            if let Ok(a_after) = after.actor(s.actor) {
                self.audit.expect(
                    a_after.weapon_count(usage.weapon) + 1 == a.weapon_count(usage.weapon),
                    "A-THROWN-COUNT",
                    || "a thrown weapon must leave the thrower's supply".into(),
                );
            }
            self.audit.tally(if a.spec().kind == CreatureKind::Hero {
                "thrown-attacks"
            } else {
                "enemy-thrown-attacks"
            });
        }
        // Consequences on the target (p17-18).
        self.audit_target_after(
            s,
            t,
            TargetFacts {
                damage: expected_damage,
                critical,
                usage,
                fortitude_roll: extra == 1,
            },
            after,
        );
        // Mastery: only the three chosen weapons (p48) and the effect they promise.
        if let Some(m) = s.mastery {
            self.audit.expect(
                usage.use_mastery
                    && a.spec().masteries.contains(&usage.weapon)
                    && mastery_of(usage.weapon) == Some(m),
                "F-MASTERY",
                || {
                    format!(
                        "mastery {m:?} on {:?} not chosen or not that weapon's property",
                        usage.weapon
                    )
                },
            );
            self.audit.tally("mastery-attacks");
            if let Ok(t_after) = after.actor(s.target) {
                match m {
                    Mastery::Sap => self.audit.expect(
                        !s.hit || t_after.conditions().contains(&"sapped"),
                        "F-MASTERY",
                        || "Sap hit did not sap".into(),
                    ),
                    Mastery::Slow => self.audit.expect(
                        !s.hit || s.damage == 0 || t_after.conditions().contains(&"slowed"),
                        "F-MASTERY",
                        || "Slow hit did not slow".into(),
                    ),
                    Mastery::Graze => self.audit.expect(
                        s.hit || s.damage == i32::from(flat.max(0)) as u16,
                        "F-MASTERY",
                        || "Graze miss damage is the ability modifier".into(),
                    ),
                    _ => {}
                }
            }
        } else {
            self.audit
                .expect(!(s.graze), "F-MASTERY", || "graze without mastery".into());
        }
    }

    fn audit_target_after(
        &mut self,
        s: &AttackSummary,
        t: &Combatant,
        facts: TargetFacts<'_>,
        after: &Battle,
    ) {
        let TargetFacts {
            damage,
            critical,
            usage,
            fortitude_roll,
        } = facts;
        let Ok(now) = after.actor(s.target) else {
            return;
        };
        let total = (damage + i32::from(s.necrotic_damage)) as u16;
        let before_health = t.health();
        let mut hp = before_health.hit_points;
        let (mut dead, mut unconscious, mut prone) = (
            before_health.dead,
            before_health.unconscious,
            before_health.prone,
        );
        let mut failures = before_health.death_failures;
        let mut stable = before_health.stable;
        if total > 0 {
            if hp > 0 && total < hp {
                hp -= total;
            } else if hp > 0 && usage.knockout {
                hp = 1;
                unconscious = true;
                prone = true;
            } else if hp > 0 && t.spec().kind == CreatureKind::Zombie && !critical && fortitude_roll
            {
                // The Constitution save outcome decides between 1 and 0 hit points; both are legal.
                if now.health().hit_points == 1 {
                    hp = 1;
                } else {
                    hp = 0;
                    unconscious = true;
                    prone = true;
                    dead = true;
                }
            } else {
                let remainder = if hp > 0 { total - hp } else { total };
                let was_zero = hp == 0;
                hp = 0;
                unconscious = true;
                prone = true;
                if t.spec().kind != CreatureKind::Hero || remainder >= t.health().maximum {
                    dead = true;
                } else if was_zero {
                    stable = false;
                    failures = (failures + if critical { 2 } else { 1 }).min(3);
                    if failures >= 3 {
                        dead = true;
                    }
                }
                if !was_zero && t.spec().kind == CreatureKind::Hero {
                    stable = false;
                }
            }
        }
        let h = now.health();
        let wolf_prone = s.hit
            && usage.weapon == Weapon::Bite
            && matches!(
                s.actor_kind_hint(),
                Some(CreatureKind::Wolf | CreatureKind::Mastiff)
            );
        let _ = wolf_prone;
        self.audit.expect(
            h.hit_points == hp && h.dead == dead && h.unconscious == unconscious && (h.prone || !prone) && h.death_failures == failures && h.stable == stable,
            "A-TARGET-STATE",
            || format!("after {damage} damage (crit {critical}): expected hp {hp} dead {dead} unconscious {unconscious} failures {failures} stable {stable}; got {h:?} (before {before_health:?})"),
        );
        if before_health.hit_points == 0 && total > 0 && t.spec().kind == CreatureKind::Hero {
            self.audit.tally("damage-at-zero");
        }
        if dead && t.spec().kind == CreatureKind::Hero {
            self.audit.tally("hero-deaths");
        }
    }

    /// Everything the step did not own must be unchanged: no pushes, no stray healing.
    fn audit_untouched(&mut self, before: &Battle, after: &Battle, actor: u32, action: &Action) {
        let mover = self.last_mover;
        for c in before.combatants() {
            let Ok(n) = after.actor(c.id()) else {
                self.audit.fail("T-ROSTER", "combatant vanished".into());
                continue;
            };
            let touched = c.id() == actor || Some(c.id()) == mover;
            if !touched
                && !matches!(
                    action,
                    Action::Attack { .. } | Action::Opportunity { .. } | Action::EndTurn
                )
            {
                self.audit.expect(
                    c.cell() == n.cell() && c.health() == n.health(),
                    "T-ROSTER",
                    || format!("{} changed during {actor}'s {action:?}", c.id()),
                );
            }
            self.audit
                .expect(!c.health().dead || n.health().dead, "T-ROSTER", || {
                    "a dead creature came back".into()
                });
            self.audit
                .expect(c.health().maximum == n.health().maximum, "T-ROSTER", || {
                    "maximum hit points changed".into()
                });
            if c.id() != actor && Some(c.id()) != mover {
                self.audit.expect(c.cell() == n.cell(), "T-ROSTER", || {
                    format!("{} moved without moving", c.id())
                });
            }
        }
    }
}

trait KindHint {
    fn actor_kind_hint(&self) -> Option<CreatureKind>;
}
impl KindHint for AttackSummary {
    fn actor_kind_hint(&self) -> Option<CreatureKind> {
        None
    }
}

/// Side of a creature id, for harnesses.
pub fn side_of(b: &Battle, id: u32) -> Option<Side> {
    b.actor(id).ok().map(Combatant::side)
}
