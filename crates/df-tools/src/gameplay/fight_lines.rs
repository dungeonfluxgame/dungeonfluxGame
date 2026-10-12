//! Short battle cries and Dungeon Master fight narration for the shared display.
//!
//! The server picks these while it records an attack, or while it projects a finished
//! fight, from the authored sets in `content/fight-lines.json` (content data, not code).
//! Every line is restrained-safe and carries only public combatant names, so no audience
//! needs a separate set; the tables fail closed (no lines) when the content is invalid.
//! Picks are deterministic in the recorded fight data, so a replay of the same fight says
//! the same thing. Lines ride on the outcome (or the finished view) they belong to: the
//! client stages them with that beat, and a skipped beat's lines never reach the stage.
use df_protocol::common as rpc;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::OnceLock;

const CONTENT: &str = include_str!("../../../../content/fight-lines.json");
const SCHEMA: &str = "df-fight-lines/1";
/// The longest speakable line the tables may hold; the voice path caps far above this,
/// but a cry or call-out must fit inside one beat.
pub(crate) const MAXIMUM_LINE_CHARS: usize = 140;
/// Pools the server reads; every other pool in the file is reserved for later content.
const KNOWN_POOLS: &[&str] = &[
    "hero_attack",
    "hero_hit",
    "hero_crit",
    "hero_down",
    "enemy_attack",
    "enemy_hit",
    "enemy_crit",
    "enemy_down",
    "enemy_taunt",
    "narrator_opening",
    "narrator_first_blood",
    "narrator_bloodied",
    "narrator_hero_down",
    "narrator_kill",
    "narrator_last_enemy",
    "narrator_crit",
    "narrator_victory",
    "narrator_defeat",
];

struct Tables {
    pools: HashMap<String, Vec<String>>,
}

fn valid(text: &str) -> bool {
    if text.trim().is_empty() || text.chars().count() > MAXIMUM_LINE_CHARS {
        return false;
    }
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        rest = &rest[open + 1..];
        let Some(close) = rest.find('}') else {
            return false;
        };
        if !matches!(&rest[..close], "attacker" | "target") {
            return false;
        }
        rest = &rest[close + 1..];
    }
    !rest.contains('}')
}

fn load() -> Option<Tables> {
    let parsed: serde_json::Value = serde_json::from_str(CONTENT).ok()?;
    if parsed.get("schema")?.as_str()? != SCHEMA {
        return None;
    }
    let mut pools = HashMap::new();
    for name in KNOWN_POOLS {
        let entries = parsed.get("pools")?.get(*name)?.as_array()?;
        if entries.is_empty() {
            return None;
        }
        let mut lines = Vec::with_capacity(entries.len());
        for entry in entries {
            let text = entry.as_str()?;
            if !valid(text) {
                return None;
            }
            lines.push(text.to_owned());
        }
        pools.insert((*name).to_owned(), lines);
    }
    Some(Tables { pools })
}

fn tables() -> Option<&'static Tables> {
    static TABLES: OnceLock<Option<Tables>> = OnceLock::new();
    TABLES.get_or_init(load).as_ref()
}

/// One recorded attack, from either side, in server order.
#[derive(Clone, Debug)]
pub(crate) struct AttackFact {
    pub(crate) attacker: String,
    pub(crate) attacker_hero: bool,
    pub(crate) target: String,
    pub(crate) target_hero: bool,
    pub(crate) hit: bool,
    pub(crate) critical: bool,
    pub(crate) damage: u32,
    pub(crate) target_down: bool,
    pub(crate) target_hp: u32,
}

/// What the fight has already decided before the attack being recorded.
#[derive(Clone, Debug, Default)]
pub(crate) struct FightSoFar {
    /// Earlier attacks in server order: earlier ops first, then this op's earlier attacks.
    pub(crate) prior: Vec<AttackFact>,
    /// Every enemy in the fight, to tell a kill from the next-to-last one.
    pub(crate) enemy_names: Vec<String>,
}

/// The attack being recorded, with its speaker pool.
pub(crate) struct CurrentAttack {
    pub(crate) fact: AttackFact,
    /// The cry pool family: `hero-male`, `hero-female`, `small-creature`, `beast`,
    /// `undead` or `humanoid`. Heroes cry from the hero pools, enemies from the enemy
    /// pools; the family is carried for kind-specific content later.
    pub(crate) attacker_pool: &'static str,
    pub(crate) die: u32,
    pub(crate) total: i32,
    /// The target's maximum hit points now (maximums do not move mid-fight, so the
    /// current value judges every earlier hit on the same target too).
    pub(crate) target_max_hp: u32,
}

fn draw(material: &[u8], pool: &[String]) -> String {
    let digest = Sha256::digest(material);
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&digest[..8]);
    pool[usize::try_from(u64::from_le_bytes(bytes)).unwrap_or(0) % pool.len()].clone()
}

fn material(parts: &[&str]) -> Vec<u8> {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part.as_bytes());
        hasher.update(b"\0");
    }
    hasher.finalize().to_vec()
}

fn render(template: &str, attacker: &str, target: &str) -> String {
    template
        .replace("{attacker}", attacker)
        .replace("{target}", target)
}

fn pick(tables: &Tables, pool: &str, attacker: &str, target: &str, seed: &[u8]) -> Option<String> {
    tables
        .pools
        .get(pool)
        .map(|lines| render(&draw(seed, lines), attacker, target))
}

fn cry_role(hero: bool) -> rpc::TableRole {
    if hero {
        rpc::TableRole::Hero
    } else {
        rpc::TableRole::Npc
    }
}

fn cry(
    tables: &Tables,
    pool: &str,
    speaker: &str,
    hero: bool,
    attacker: &str,
    target: &str,
    seed: &[u8],
) -> Option<rpc::FightLine> {
    pick(tables, pool, attacker, target, seed).map(|text| rpc::FightLine {
        role: cry_role(hero) as i32,
        hero_index: 0,
        speaker: speaker.to_owned(),
        text,
    })
}

fn narration(
    tables: &Tables,
    pool: &str,
    attacker: &str,
    target: &str,
    seed: &[u8],
) -> Option<rpc::FightLine> {
    pick(tables, pool, attacker, target, seed).map(|text| rpc::FightLine {
        role: rpc::TableRole::Narrator as i32,
        hero_index: 0,
        speaker: String::new(),
        text,
    })
}

fn downed_enemies(prior: &[AttackFact]) -> Vec<&str> {
    prior
        .iter()
        .filter(|fact| !fact.target_hero && fact.target_down)
        .map(|fact| fact.target.as_str())
        .collect()
}

/// The lines for one recorded attack, in stage order: the attacker's cry, the victim's
/// cry, then at most one Dungeon Master line. Sparing by construction: hits and misses
/// cry only on a seeded gate, while falls, kills, first blood and the opening always do.
pub(crate) fn select(current: &CurrentAttack, so_far: &FightSoFar) -> Vec<rpc::FightLine> {
    let Some(tables) = tables() else {
        return Vec::new();
    };
    let fact = &current.fact;
    let die = current.die.to_string();
    let total = current.total.to_string();
    let prior = so_far.prior.len().to_string();
    let seed_base = [
        fact.attacker.as_str(),
        current.attacker_pool,
        fact.target.as_str(),
        die.as_str(),
        total.as_str(),
        prior.as_str(),
    ];
    let seed = |tag: &str| {
        let mut parts = seed_base.clone().to_vec();
        parts.push(tag);
        material(&parts)
    };
    // One hash per sparing gate; callers take it modulo the gate width.
    let gate = |tag: &str| {
        let digest = Sha256::digest(seed(tag));
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(&digest[..8]);
        u64::from_le_bytes(bytes)
    };
    let mut lines = Vec::new();
    if so_far.prior.is_empty() {
        lines.extend(narration(
            tables,
            "narrator_opening",
            &fact.attacker,
            &fact.target,
            &seed("opening"),
        ));
    }
    let attacker_cry = if fact.attacker_hero {
        if fact.critical {
            cry(
                tables,
                "hero_crit",
                &fact.attacker,
                true,
                &fact.attacker,
                &fact.target,
                &seed("crit"),
            )
        } else if fact.target_down && !fact.target_hero {
            cry(
                tables,
                "hero_hit",
                &fact.attacker,
                true,
                &fact.attacker,
                &fact.target,
                &seed("kill"),
            )
        } else if fact.hit && fact.damage > 0 && gate("hit") % 6 == 0 {
            cry(
                tables,
                "hero_hit",
                &fact.attacker,
                true,
                &fact.attacker,
                &fact.target,
                &seed("hit"),
            )
        } else if !fact.hit && gate("miss") % 8 == 0 {
            cry(
                tables,
                "hero_attack",
                &fact.attacker,
                true,
                &fact.attacker,
                &fact.target,
                &seed("miss"),
            )
        } else {
            None
        }
    } else if fact.critical {
        cry(
            tables,
            "enemy_crit",
            &fact.attacker,
            false,
            &fact.attacker,
            &fact.target,
            &seed("crit"),
        )
    } else if fact.target_down && fact.target_hero {
        cry(
            tables,
            "enemy_taunt",
            &fact.attacker,
            false,
            &fact.attacker,
            &fact.target,
            &seed("taunt"),
        )
    } else if fact.hit && fact.damage > 0 && gate("hit") % 6 == 0 {
        cry(
            tables,
            "enemy_hit",
            &fact.attacker,
            false,
            &fact.attacker,
            &fact.target,
            &seed("hit"),
        )
    } else if !fact.hit && gate("miss") % 8 == 0 {
        cry(
            tables,
            "enemy_attack",
            &fact.attacker,
            false,
            &fact.attacker,
            &fact.target,
            &seed("miss"),
        )
    } else {
        None
    };
    lines.extend(attacker_cry);
    if fact.target_down {
        lines.extend(cry(
            tables,
            if fact.target_hero {
                "hero_down"
            } else {
                "enemy_down"
            },
            &fact.target,
            fact.target_hero,
            &fact.attacker,
            &fact.target,
            &seed("down"),
        ));
    }
    let downed = downed_enemies(&so_far.prior);
    let survivors = so_far
        .enemy_names
        .iter()
        .filter(|name| {
            name.as_str() != fact.target && !downed.contains(&name.as_str())
                || (fact.target_hero && !downed.contains(&name.as_str()))
        })
        .count();
    let bloodied_before = current.target_max_hp > 0
        && so_far.prior.iter().any(|before| {
            before.target == fact.target && before.target_hp * 2 <= current.target_max_hp
        });
    let turning = if fact.target_hero && fact.target_down {
        narration(
            tables,
            "narrator_hero_down",
            &fact.attacker,
            &fact.target,
            &seed("hero-down"),
        )
    } else if !fact.target_hero && fact.target_down {
        if survivors == 1 {
            let survivor = so_far
                .enemy_names
                .iter()
                .find(|name| name.as_str() != fact.target && !downed.contains(&name.as_str()))
                .map_or(fact.target.clone(), Clone::clone);
            narration(
                tables,
                "narrator_last_enemy",
                &fact.attacker,
                &survivor,
                &seed("last-enemy"),
            )
        } else {
            narration(
                tables,
                "narrator_kill",
                &fact.attacker,
                &fact.target,
                &seed("kill"),
            )
        }
    } else if fact.damage > 0 && !so_far.prior.iter().any(|before| before.damage > 0) {
        narration(
            tables,
            "narrator_first_blood",
            &fact.attacker,
            &fact.target,
            &seed("first-blood"),
        )
    } else if fact.critical {
        narration(
            tables,
            "narrator_crit",
            &fact.attacker,
            &fact.target,
            &seed("narrator-crit"),
        )
    } else if fact.target_hero
        && !fact.target_down
        && current.target_max_hp > 0
        && fact.target_hp * 2 <= current.target_max_hp
        && !bloodied_before
    {
        narration(
            tables,
            "narrator_bloodied",
            &fact.attacker,
            &fact.target,
            &seed("bloodied"),
        )
    } else {
        None
    };
    lines.extend(turning);
    lines
}

/// The Dungeon Master closing line for a finished fight, picked deterministically from
/// the recorded history. One line only: the banner carries the moment.
pub(crate) fn closing(victory: bool, attacks: u32, kills: u32) -> Vec<rpc::FightLine> {
    let Some(tables) = tables() else {
        return Vec::new();
    };
    let seed = material(&[
        if victory { "victory" } else { "defeat" },
        &attacks.to_string(),
        &kills.to_string(),
    ]);
    narration(
        tables,
        if victory {
            "narrator_victory"
        } else {
            "narrator_defeat"
        },
        "",
        "",
        &seed,
    )
    .into_iter()
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fact(attacker: &str, hero: bool) -> AttackFact {
        AttackFact {
            attacker: attacker.to_owned(),
            attacker_hero: hero,
            target: if hero { "Bandit" } else { "Brynn" }.to_owned(),
            target_hero: !hero,
            hit: true,
            critical: false,
            damage: 5,
            target_down: false,
            target_hp: 10,
        }
    }

    fn current(attacker: &str, hero: bool) -> CurrentAttack {
        CurrentAttack {
            fact: fact(attacker, hero),
            attacker_pool: if hero { "hero-male" } else { "humanoid" },
            die: 14,
            total: 18,
            target_max_hp: 20,
        }
    }

    fn so_far(prior: Vec<AttackFact>) -> FightSoFar {
        FightSoFar {
            prior,
            enemy_names: vec!["Bandit".to_owned()],
        }
    }

    #[test]
    fn tables_load_with_every_known_pool() {
        let tables = tables().expect("authored fight lines load");
        for pool in KNOWN_POOLS {
            assert!(
                tables
                    .pools
                    .get(*pool)
                    .is_some_and(|lines| !lines.is_empty()),
                "pool {pool} is present and non-empty"
            );
        }
    }

    #[test]
    fn selection_is_deterministic_in_the_same_fight_data() {
        let so_far = so_far(vec![fact("Brynn", true)]);
        let first = select(&current("Sylvie", true), &so_far);
        let second = select(&current("Sylvie", true), &so_far);
        assert_eq!(first, second);
        let closing_first = closing(true, 12, 2);
        assert_eq!(closing_first, closing(true, 12, 2));
        assert_ne!(closing(true, 12, 2), closing(false, 12, 2));
    }

    #[test]
    fn lines_carry_no_unrendered_placeholders_and_fit_a_beat() {
        let opening = select(&current("Brynn", true), &so_far(Vec::new()));
        assert!(
            opening
                .iter()
                .any(|line| line.role == rpc::TableRole::Narrator as i32),
            "the first attack of a fight opens with narration"
        );
        for line in opening
            .iter()
            .chain(closing(true, 9, 1).iter())
            .chain(closing(false, 9, 1).iter())
        {
            assert!(
                !line.text.contains('{') && !line.text.contains('}'),
                "no placeholders survive: {}",
                line.text
            );
            assert!(
                line.text.chars().count() <= MAXIMUM_LINE_CHARS,
                "a cry fits its beat: {}",
                line.text
            );
        }
    }

    #[test]
    fn a_fall_is_always_cried_and_narrated() {
        let mut fallen = fact("Bandit", false);
        fallen.target_down = true;
        fallen.target_hp = 0;
        let attack = CurrentAttack {
            fact: fallen,
            attacker_pool: "humanoid",
            die: 17,
            total: 20,
            target_max_hp: 20,
        };
        let lines = select(&attack, &so_far(vec![fact("Brynn", true)]));
        assert!(
            lines
                .iter()
                .any(|line| line.speaker == "Brynn" && line.role == rpc::TableRole::Hero as i32),
            "the fallen hero cries out"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.role == rpc::TableRole::Narrator as i32),
            "the fall is narrated"
        );
    }

    #[test]
    fn cries_stay_sparing_across_many_plain_hits() {
        let history: Vec<AttackFact> = (0..6).map(|_| fact("Brynn", true)).collect();
        let cries: usize = (0..60)
            .map(|index| {
                let attack = CurrentAttack {
                    die: 10 + (index % 10) as u32,
                    ..current("Brynn", true)
                };
                select(&attack, &so_far(history.clone()))
                    .iter()
                    .filter(|line| line.role != rpc::TableRole::Narrator as i32)
                    .count()
            })
            .sum();
        assert!(
            cries <= 20,
            "plain hits cry sparingly, not every blow: {cries}/60"
        );
    }
}
