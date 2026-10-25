//! SRD 5.2.1 Giant Ancestry (Goliath, pp. 85-86): a Stone or Storm hero who takes damage with uses
//! left has a pending species reaction. Ordinary actions are refused until `resolve_species`
//! clears it. These tests pin that contract, which the table's reaction loop must answer.
#[path = "support/dice.rs"]
mod dice;

use df_combat::tactical::{propose, propose_latest, propose_legacy};
use df_model::checkpoint::RuleReference;
use df_rules::local_journey::CreationRules;
use df_rules::tactical::{
    Action, Battle, BattleMap, Cell, CombatantSpec, PendingReaction, RuleError, SpeciesAction,
    SpeciesReaction, begin,
};
use dice::{Rng, binding, record, resolve_dice, resolve_species_dice, source};

const MAP: &[u8] = br#"{"schema":2,"grid":{"cols":12,"rows":3,"blocked":[]},"layouts":[{"id":"flat","obstacles":[],"difficult":[],"spawns":{"heroes":[[0,1]],"enemies":[[1,1]]}}]}"#;

const PROVOKED_MAP: &[u8] = br#"{"schema":2,"grid":{"cols":12,"rows":3,"blocked":[]},"layouts":[{"id":"flat","obstacles":[],"difficult":[],"spawns":{"heroes":[[3,1]],"enemies":[[2,0],[2,2]]}}]}"#;

/// Opening initiative: one face per hero, then one shared face for the identical wolves.
fn opening(source: &RuleReference, faces: &[u32]) -> Vec<df_model::checkpoint::ActualDraw> {
    let mut rng = Rng(61);
    faces
        .iter()
        .enumerate()
        .map(|(ordinal, face)| {
            let mut draw = record(&mut rng, 20, ordinal as u32, source);
            draw.value = *face;
            draw
        })
        .collect()
}

fn goliath(ancestry: &'static str, cell: Cell) -> CombatantSpec {
    let choices = [
        ("species", "goliath"),
        ("giant-ancestry", ancestry),
        ("class", "fighter-1"),
        ("background", "soldier"),
        ("array", "stalwart"),
        ("alignment", "neutral-good"),
        ("language-1", "dwarvish"),
        ("language-2", "elvish"),
        ("skill-1", "perception"),
        ("skill-2", "survival"),
        ("gaming-set", "dice"),
        ("equipment", "fighter-a-soldier-a"),
        ("style-masteries", "defense-greatsword-flail-javelin"),
        ("allied-ties", "join-order"),
    ];
    let built =
        df_rules::local_journey::validate_character(&CreationRules::srd_521(), "Gora", &choices)
            .unwrap();
    CombatantSpec::melee_hero(&built, 1, cell).unwrap()
}

/// A Goliath Fighter with the given Giant Ancestry, at (0,1), and a wolf at (1,1) that acts first.
fn goliath_fight(ancestry: &'static str) -> Battle {
    let map = BattleMap::from_json(MAP, "flat").unwrap();
    let specs = vec![
        goliath(ancestry, Cell(0, 1)),
        CombatantSpec::creature(10, "wolf", Cell(1, 1), 0).unwrap(),
    ];
    let source = source();
    let initiative = opening(&source, &[1, 20]);
    begin(map, specs, binding(&source), &initiative)
        .unwrap()
        .battle
}

/// The first recorded wolf attack that leaves the Goliath with a pending species reaction.
fn wolf_hits_goliath(battle: &Battle) -> Battle {
    let action = propose_latest(battle, 10).unwrap().unwrap();
    for seed in 0..400 {
        let mut rng = Rng(seed);
        if let Ok(result) = resolve_dice(battle, 10, &action, &mut rng)
            && result.battle.species_reaction_offer().is_some()
        {
            return result.battle;
        }
    }
    panic!("no seed in 0..400 produced a species reaction for the Goliath");
}

/// A Stone Goliath at (3,1) with two wolves beside it, one on each side. The Goliath steps away
/// and provokes both wolves' opportunity attacks. The moved state keeps both reactors waiting.
fn provoked_goliath_fight() -> Battle {
    let map = BattleMap::from_json(PROVOKED_MAP, "flat").unwrap();
    let specs = vec![
        goliath("stone", Cell(3, 1)),
        CombatantSpec::creature(10, "wolf", Cell(2, 0), 0).unwrap(),
        CombatantSpec::creature(11, "wolf", Cell(2, 2), 0).unwrap(),
    ];
    let source = source();
    let initiative = opening(&source, &[20, 1]);
    let opened = begin(map, specs, binding(&source), &initiative)
        .unwrap()
        .battle;
    let mut rng = Rng(3);
    let moved = resolve_dice(
        &opened,
        1,
        &Action::Move(vec![Cell(4, 1), Cell(5, 1)]),
        &mut rng,
    )
    .unwrap()
    .battle;
    assert_eq!(moved.reaction_offers(), &[10, 11]);
    moved
}

/// The first opportunity attack that hits the Goliath and leaves its species reaction pending.
fn first_reactor_hit(pending: &Battle) -> Battle {
    let action = propose_latest(pending, 10).unwrap().unwrap();
    for seed in 0..400 {
        let mut rng = Rng(seed);
        if let Ok(result) = resolve_dice(pending, 10, &action, &mut rng)
            && result.battle.species_reaction_offer().is_some()
        {
            return result.battle;
        }
    }
    panic!("no seed in 0..400 let the first reactor hit the Goliath");
}

/// SRD 5.2.1 p. 85: a Stone Goliath's reaction is offered when damage is about to be applied. While
/// it is pending, the wolf cannot end its turn, and only `resolve_species` clears the offer.
#[test]
fn a_stone_goliath_hit_with_uses_left_blocks_ordinary_actions_until_resolved() {
    let battle = goliath_fight("stone");
    let pending = wolf_hits_goliath(&battle);
    assert_eq!(
        pending.species_reaction_offer(),
        Some((1, SpeciesReaction::StoneEndurance))
    );
    let mut rng = Rng(7);
    assert!(
        matches!(
            resolve_dice(&pending, 10, &Action::EndTurn, &mut rng),
            Err(RuleError::PendingReaction)
        ),
        "an ordinary action must wait for the species reaction"
    );
    let cleared = resolve_species_dice(
        &pending,
        1,
        &SpeciesAction::Reaction { use_trait: false },
        &mut rng,
    )
    .unwrap()
    .battle;
    assert_eq!(cleared.species_reaction_offer(), None);
    assert!(resolve_dice(&cleared, 10, &Action::EndTurn, &mut rng).is_ok());
}

/// SRD 5.2.1 p. 85: the same hit and the same dice, answered two ways. Using Stone's Endurance
/// spends the reaction and one use, and never leaves the Goliath with less HP than declining.
#[test]
fn stone_endurance_reduces_the_triggering_damage_and_spends_one_use() {
    let pending = wolf_hits_goliath(&goliath_fight("stone"));
    let declined = resolve_species_dice(
        &pending,
        1,
        &SpeciesAction::Reaction { use_trait: false },
        &mut Rng(9),
    )
    .unwrap()
    .battle;
    let used = resolve_species_dice(
        &pending,
        1,
        &SpeciesAction::Reaction { use_trait: true },
        &mut Rng(9),
    )
    .unwrap()
    .battle;
    let (declined_hero, used_hero) = (declined.actor(1).unwrap(), used.actor(1).unwrap());
    assert!(!declined_hero.budget().reaction_used);
    assert!(used_hero.budget().reaction_used);
    assert_eq!(declined_hero.species_uses_remaining(), 2);
    assert_eq!(used_hero.species_uses_remaining(), 1);
    // The fixture's stalwart array gives CON 15 (modifier +2 or more), so the reduction is at least 3
    // and the strict inequality holds for any triggering hit of at least 1 damage.
    assert!(
        used_hero.health().hit_points > declined_hero.health().hit_points,
        "using the trait left {} HP, declining left {}",
        used_hero.health().hit_points,
        declined_hero.health().hit_points
    );
    assert_eq!(used.species_reaction_offer(), None);
}

/// Only the reacting hero may answer its own offer; the wolf cannot clear it.
#[test]
fn only_the_offered_hero_may_clear_the_species_reaction() {
    let pending = wolf_hits_goliath(&goliath_fight("stone"));
    let mut rng = Rng(11);
    assert!(
        matches!(
            resolve_species_dice(
                &pending,
                10,
                &SpeciesAction::Reaction { use_trait: false },
                &mut rng
            ),
            Err(RuleError::InvalidReaction)
        ),
        "a creature must not answer a hero's species reaction"
    );
}

/// While the Goliath's species reaction is pending, the wolf's policy must not propose an action:
/// the engine refuses every ordinary action with `PendingReaction`, so the policy should say so too.
#[test]
fn the_policy_refuses_to_propose_while_a_species_reaction_is_pending() {
    let pending = wolf_hits_goliath(&goliath_fight("stone"));
    let proposed = propose_latest(&pending, 10);
    assert!(
        matches!(proposed, Err(RuleError::PendingReaction)),
        "the policy proposed {proposed:?} while a species reaction was pending"
    );
}

/// The first reactor's hit leaves the Goliath's reaction pending. The second opportunity reactor
/// is still waiting in order, but no policy may offer it an opportunity attack until the species
/// reaction is cleared, because the engine refuses every ordinary action.
#[test]
fn the_second_opportunity_reactor_waits_while_a_species_reaction_is_pending() {
    let hit = first_reactor_hit(&provoked_goliath_fight());
    assert!(hit.reaction_offers().contains(&11));
    let latest = propose_latest(&hit, 11);
    assert!(
        matches!(latest, Err(RuleError::PendingReaction)),
        "propose_latest offered the second reactor {latest:?} while the Goliath's reaction was pending"
    );
    let legacy = propose_legacy(&hit, 11);
    assert!(
        matches!(legacy, Err(RuleError::PendingReaction)),
        "propose_legacy offered the second reactor {legacy:?} while the Goliath's reaction was pending"
    );
    let base = propose(&hit, 11);
    assert!(
        matches!(base, Err(RuleError::PendingReaction)),
        "propose offered the second reactor {base:?} while the Goliath's reaction was pending"
    );
}

/// The single caller-facing answer to "who must act before anything else?" names the Goliath while
/// its species reaction is pending, and nobody once `resolve_species` declines it.
#[test]
fn a_pending_species_reaction_is_reported_as_the_pending_reaction() {
    let pending = wolf_hits_goliath(&goliath_fight("stone"));
    assert_eq!(
        pending.pending_reaction(),
        Some(PendingReaction::Species {
            hero: 1,
            kind: SpeciesReaction::StoneEndurance
        })
    );
    let mut rng = Rng(13);
    let cleared = resolve_species_dice(
        &pending,
        1,
        &SpeciesAction::Reaction { use_trait: false },
        &mut rng,
    )
    .unwrap()
    .battle;
    assert_eq!(cleared.pending_reaction(), None);
}

/// SRD 5.2.1 p. 86: Storm's Thunder is offered after the damage is applied, with the same blocking
/// and clearing contract as Stone's Endurance.
#[test]
fn a_storm_goliath_reaction_is_offered_after_damage_and_clears_the_same_way() {
    let battle = goliath_fight("storm");
    let pending = wolf_hits_goliath(&battle);
    assert_eq!(
        pending.species_reaction_offer(),
        Some((1, SpeciesReaction::StormThunder))
    );
    assert_eq!(
        pending.pending_reaction(),
        Some(PendingReaction::Species {
            hero: 1,
            kind: SpeciesReaction::StormThunder
        })
    );
    let mut rng = Rng(15);
    let cleared = resolve_species_dice(
        &pending,
        1,
        &SpeciesAction::Reaction { use_trait: false },
        &mut rng,
    )
    .unwrap()
    .battle;
    assert_eq!(cleared.species_reaction_offer(), None);
    assert!(resolve_dice(&cleared, 10, &Action::EndTurn, &mut rng).is_ok());
}
