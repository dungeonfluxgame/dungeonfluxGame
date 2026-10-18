//! Rules coverage matrix (PLAY-R01 M3): every mechanic family of planning/rules-coverage.md
//! against implementation, reachability in a live game, native test, browser check and TV or
//! phone display. The table is data checked by tests, so a claim cannot rot: every `Yes` or
//! `Partial` implementation claim names evidence that must still exist in the named source file,
//! every browser claim names a script that must still exist, and every family of the planning
//! document must still be present.
//!
//! Levels are deliberately blunt. `Yes` means the supported subset works end to end for what the
//! family needs at level 1; `Partial` means a named subset; `No` means nothing exists;
//! `Unverified` means a check or screen exists but this task did not run it.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Level {
    Yes,
    Partial,
    No,
    Unverified,
}
impl Level {
    pub fn word(self) -> &'static str {
        match self {
            Self::Yes => "yes",
            Self::Partial => "partial",
            Self::No => "NO",
            Self::Unverified => "unverified",
        }
    }
}
/// A source file whose text must contain a needle, relative to the repository root.
pub type Evidence = (&'static str, &'static str);

#[derive(Clone, Copy, Debug)]
pub struct Row {
    /// The planning/rules-coverage.md family this row answers (R01..R17), or `-` for a split.
    pub planning: &'static str,
    pub family: &'static str,
    pub rules: Level,
    pub live: Level,
    pub native_test: Level,
    pub browser: Level,
    pub display: Level,
    pub note: &'static str,
    /// Evidence for the implementation and native-test claims.
    pub evidence: &'static [Evidence],
    /// Browser scripts (under development/browser-checks) this row relies on.
    pub browser_scripts: &'static [&'static str],
}

const RESOLVE: &str = "crates/df-rules/src/tactical/resolve.rs";
const STATE: &str = "crates/df-rules/src/tactical/state.rs";
const WEAPONS: &str = "crates/df-rules/src/tactical/weapons.rs";
const MAP: &str = "crates/df-rules/src/tactical/map.rs";
const INVARIANTS: &str = "crates/df-combat/tests/rules_invariants.rs";
const AUDIT: &str = "crates/df-combat/tests/support/rules_audit.rs";
const CHECK: &str = "crates/df-rules/src/ability_check.rs";
const INFLUENCE: &str = "crates/df-rules/src/influence.rs";
const LOCAL: &str = "crates/df-rules/src/local_journey.rs";
const CREATURES: &str = "crates/df-rules/src/srd_creatures.rs";

use Level::{No, Partial, Unverified, Yes};

pub const ROWS: &[Row] = &[
    Row {
        planning: "R04",
        family: "Abilities and ability checks",
        rules: Partial,
        live: Partial,
        native_test: Yes,
        browser: Unverified,
        display: Unverified,
        note: "one normal d20 check against a DC with the proficiency bonus; no Advantage or Disadvantage on checks, no passive checks, no group or contested checks",
        evidence: &[
            (CHECK, "pub fn resolve"),
            (LOCAL, "pub const fn ability_modifier"),
        ],
        browser_scripts: &["investigate-mystery.mjs"],
    },
    Row {
        planning: "R01",
        family: "Skills",
        rules: Partial,
        live: Partial,
        native_test: Partial,
        browser: Unverified,
        display: Unverified,
        note: "creation picks Fighter and Soldier skills and a check can add proficiency; no skill list is checked against a roll, Medicine is not on the Fighter list",
        evidence: &[(LOCAL, "pub const FIGHTER_SKILLS")],
        browser_scripts: &["play-t03-create.mjs"],
    },
    Row {
        planning: "R04",
        family: "Saving throws",
        rules: Partial,
        live: No,
        native_test: Partial,
        browser: No,
        display: No,
        note: "only the Zombie's Undead Fortitude Constitution save and the Dexterity cover/Dodge adjustments exist; there is no general saving-throw resolver or proficiency in saves",
        evidence: &[
            (RESOLVE, "pub fn dexterity_save_adjustments"),
            (RESOLVE, "Zombie"),
        ],
        browser_scripts: &[],
    },
    Row {
        planning: "R04",
        family: "Advantage and Disadvantage",
        rules: Yes,
        live: Yes,
        native_test: Yes,
        browser: Unverified,
        display: Partial,
        note: "attack rolls: any source of each cancels, never stacks; the reasons are carried on every attack outcome and on each legal target. Ability checks and saves have none",
        evidence: &[(RESOLVE, "roll_mode"), (AUDIT, "A-ROLL-MODE")],
        browser_scripts: &["play-t03-fight.mjs"],
    },
    Row {
        planning: "R01",
        family: "Proficiency",
        rules: Partial,
        live: Yes,
        native_test: Yes,
        browser: No,
        display: Partial,
        note: "Fighter attack proficiency +2 at levels 1-4 and +3 at level 5; native sheet saves scale, while general skill-check callers still need the persisted level; no expertise",
        evidence: &[(STATE, "hero_attack_modifiers")],
        browser_scripts: &[],
    },
    Row {
        planning: "R09",
        family: "Conditions",
        rules: Partial,
        live: Partial,
        native_test: Partial,
        browser: No,
        display: Partial,
        note: "Prone, Unconscious, Incapacitated (as Unconscious or dead), Dodging, Slowed and Sapped exist; Blinded, Charmed, Deafened, Exhaustion, Frightened, Grappled, Invisible, Paralyzed, Petrified, Poisoned, Restrained and Stunned do not",
        evidence: &[(STATE, "\"prone\""), (STATE, "\"slowed\"")],
        browser_scripts: &[],
    },
    Row {
        planning: "R05",
        family: "Actions in combat",
        rules: Partial,
        live: Partial,
        native_test: Yes,
        browser: Unverified,
        display: Unverified,
        note: "Attack, Dash, Disengage, Dodge, Move, Stand, Utilize (doff shield, pick up a weapon) exist; Help, Hide, Influence, Magic, Ready, Search and Study do not. The owner's order-of-combat checks (initiative, turns, one action) hold over generated fights",
        evidence: &[
            (RESOLVE, "Action::Dash"),
            (INVARIANTS, "generated_fights_with_planner_enemies"),
        ],
        browser_scripts: &["play-t03-fight.mjs"],
    },
    Row {
        planning: "R05",
        family: "Bonus actions and reactions",
        rules: Partial,
        live: Partial,
        native_test: Yes,
        browser: Unverified,
        display: Unverified,
        note: "Second Wind, the Light bonus attack, Nimble Escape and the Opportunity Attack exist; the Ready action and other reaction triggers do not; enemies decline their Opportunity Attacks by policy",
        evidence: &[
            (RESOLVE, "Action::Opportunity"),
            (RESOLVE, "NimbleDisengage"),
        ],
        browser_scripts: &["play-t03-fight.mjs"],
    },
    Row {
        planning: "R05",
        family: "Initiative, turns and rounds",
        rules: Partial,
        live: Yes,
        native_test: Yes,
        browser: Unverified,
        display: Partial,
        note: "one d20 + Dexterity per participant, Champion Initiative Advantage from level 3, one shared roll per identical monster group, fixed order, ties by the table's agreed order; no surprise, Delay or Ready",
        evidence: &[(STATE, "begin_faces"), (AUDIT, "I-INIT-ORDER")],
        browser_scripts: &["stage-fight.mjs"],
    },
    Row {
        planning: "R06",
        family: "Movement and position",
        rules: Yes,
        live: Yes,
        native_test: Yes,
        browser: Unverified,
        display: Partial,
        note: "square-by-square Speed budget, diagonals 5 ft, Difficult Terrain and crawling, Dash, standing, passing through allies and the Incapacitated, corners, Opportunity Attacks; Large/Huge source monsters use full 2x2/3x3 spaces, swept occupancy and footprint reach; Huge wire projection awaits its enum; flying, climbing and swimming are not supported",
        evidence: &[
            (STATE, "fn step_cost"),
            (MAP, "fn step_clear"),
            (AUDIT, "M-COST"),
        ],
        browser_scripts: &["play-t03-fight.mjs"],
    },
    Row {
        planning: "R06",
        family: "Cover",
        rules: Yes,
        live: Yes,
        native_test: Yes,
        browser: Unverified,
        display: Unverified,
        note: "Half, Three-Quarters and Total from the grid with the disclosed centre-line convention (the one custom rule); no light or obscurement",
        evidence: &[(MAP, "pub fn cover_between"), (AUDIT, "A-COVER")],
        browser_scripts: &["stage-tactical.mjs"],
    },
    Row {
        planning: "R07",
        family: "Attack rolls",
        rules: Yes,
        live: Yes,
        native_test: Yes,
        browser: Unverified,
        display: Unverified,
        note: "melee within reach, ranged within range with long-range Disadvantage and the close-enemy Disadvantage, thrown weapons, ammunition, natural 1 and 20, cover; no grappling, shoving or area attacks",
        evidence: &[(RESOLVE, "fn admit_attack"), (AUDIT, "A-ACCEPTED-ILLEGAL")],
        browser_scripts: &["stage-probe-attack.mjs"],
    },
    Row {
        planning: "R08",
        family: "Damage types, Resistance and Vulnerability",
        rules: Partial,
        live: Partial,
        native_test: Partial,
        browser: No,
        display: No,
        note: "Slashing, Piercing, Bludgeoning and a separate Necrotic damage exist and the Skeleton's Bludgeoning Vulnerability doubles; there is no general Resistance or Immunity resolver and the damage type is not yet on the wire",
        evidence: &[(WEAPONS, "pub enum DamageType"), (RESOLVE, "Skeleton")],
        browser_scripts: &[],
    },
    Row {
        planning: "R07",
        family: "Critical hits",
        rules: Yes,
        live: Yes,
        native_test: Yes,
        browser: Unverified,
        display: Unverified,
        note: "natural 20 and any hit on an Unconscious creature within 5 ft double the dice, never the modifier",
        evidence: &[(RESOLVE, "let critical"), (AUDIT, "A-CRITICAL")],
        browser_scripts: &["stage-fight.mjs"],
    },
    Row {
        planning: "R08",
        family: "Death saves and dying",
        rules: Yes,
        live: Partial,
        native_test: Yes,
        browser: No,
        display: Partial,
        note: "0 hit points, Unconscious, Death Saving Throws at the start of the turn, nat 1 and 20, damage at 0, massive damage, Stable; one save per turn is now in the log and in CombatView.death_saves; the timed post-fight rounds and the Stable hero's 1d4-hour recovery are accepted by the server; the Help action to stabilise (DC 10 Wisdom (Medicine), SRD p18) is not offered yet; the display of the saves is PLAY-V14's",
        evidence: &[(RESOLVE, "fn death_save"), (AUDIT, "T-DEATH-SAVE-SKIPPED")],
        browser_scripts: &[],
    },
    Row {
        planning: "R08",
        family: "Healing",
        rules: Partial,
        live: Partial,
        native_test: Yes,
        browser: Unverified,
        display: Partial,
        note: "Second Wind (self only) and the knockout awakening exist; no potions, no Help action to stabilise a dying ally, no Hit Point Dice, no temporary Hit Points",
        evidence: &[(RESOLVE, "Action::SecondWind")],
        browser_scripts: &["play-t03-fight.mjs"],
    },
    Row {
        planning: "R13",
        family: "Resting",
        rules: Partial,
        live: Partial,
        native_test: Yes,
        browser: No,
        display: Partial,
        note: "a one-hour Short Rest that recharges one Second Wind use (cap 2 through level 3, 3 from level 4), recharges Action Surge and ends the knockout; a Stable hero regains 1 hit point after 1d4 hours (Action::StableRecovery); no tactical Hit Dice or general Long Rest admission (the pure level-5 Fighter rest handler restores HP, Second Wind and Action Surge), no Exhaustion; a rest cannot start while any hero is at 0 hit points, so the table first waits out the dying hero's fate",
        evidence: &[(RESOLVE, "Action::ShortRest"), (RESOLVE, "StableRecovery")],
        browser_scripts: &[],
    },
    Row {
        planning: "R12",
        family: "Equipment, weapon properties and masteries",
        rules: Partial,
        live: Yes,
        native_test: Yes,
        browser: Unverified,
        display: Partial,
        note: "thirteen weapons with Heavy, Finesse, Light, Two-Handed, Versatile (Spear), Thrown range and Ammunition; masteries Graze, Sap, Slow, Vex and Nick; the Shield; no Cleave, Push or Topple, no armour system (Armor Class is fixed by the build), no magic items",
        evidence: &[(WEAPONS, "pub enum Mastery"), (WEAPONS, "pub fn heavy")],
        browser_scripts: &["play-t03-fight.mjs"],
    },
    Row {
        planning: "R10",
        family: "Spellcasting",
        rules: No,
        live: No,
        native_test: No,
        browser: No,
        display: No,
        note: "nothing exists: no spell data, slots, components, concentration or spell attacks",
        evidence: &[],
        browser_scripts: &[],
    },
    Row {
        planning: "R02",
        family: "Classes and levels",
        rules: Partial,
        live: Partial,
        native_test: Yes,
        browser: Unverified,
        display: Partial,
        note: "fixed Fighter levels 1-5 with source-pinned HP, proficiency, Second Wind, Action Surge, Champion, level-4 ASI and Extra Attack; level-5 test creation is native, not browser-verified; Tactical Mind and Champion Athletics have pure mechanics but await ability-check admission, and Champion critical movement is currently on-turn only; no general advancement, other class or multiclass",
        evidence: &[(LOCAL, "pub fn validate_character")],
        browser_scripts: &["play-t03-create.mjs"],
    },
    Row {
        planning: "R03",
        family: "Backgrounds and origin feats",
        rules: Partial,
        live: Partial,
        native_test: Yes,
        browser: Unverified,
        display: Partial,
        note: "Soldier with the Savage Attacker origin feat (once per turn, in the checker); no other background or origin feat",
        evidence: &[(STATE, "savage_attacker"), (AUDIT, "F-SAVAGE-ATTACKER")],
        browser_scripts: &["play-t03-create.mjs"],
    },
    Row {
        planning: "R03",
        family: "Species traits",
        rules: Partial,
        live: Partial,
        native_test: Partial,
        browser: Unverified,
        display: Partial,
        note: "Dwarf (Speed 30, Dwarven Toughness in the Hit Points); Human and Elf are refused explicitly because their traits would be lost; no Darkvision or senses rules",
        evidence: &[(LOCAL, "dwarf")],
        browser_scripts: &["play-t03-create.mjs"],
    },
    Row {
        planning: "R15",
        family: "Monster stat blocks and traits",
        rules: Partial,
        live: Yes,
        native_test: Yes,
        browser: Unverified,
        display: Partial,
        note: "seventeen SRD creatures including CR 2-5 Ogre, Owlbear, Wight, Hill Giant and Troll; source actions and all excluded features are classified in SRD_521_CREATURE_FEATURES and disclosed in the card rules note; Troll regeneration is active, limbs/Charge, Wight Life Drain, Hill Giant Trash Lob and vertical climbing are excluded explicitly",
        evidence: &[
            (CREATURES, "SRD_521_CREATURE_FEATURES"),
            (STATE, "pub fn creature"),
        ],
        browser_scripts: &["stage-fight.mjs"],
    },
    Row {
        planning: "R14",
        family: "Exploration",
        rules: No,
        live: Partial,
        native_test: Partial,
        browser: Unverified,
        display: Partial,
        note: "authored story beats with one ability check; no travel, light, hazards, traps, falling, searching or stealth rules in df-rules",
        evidence: &[],
        browser_scripts: &["investigate-mystery.mjs"],
    },
    Row {
        planning: "R14",
        family: "Social interaction",
        rules: Yes,
        live: Partial,
        native_test: Yes,
        browser: Unverified,
        display: Partial,
        note: "Influence action with willing/hesitant/unwilling, DC 15 or Intelligence, attitude Advantage/Disadvantage, the 24-hour retry wait, an approach-to-skill table and table-ready explanations in df-rules; word quality as an Advantage/Disadvantage source is the disclosed house rule influence.word-quality-advantage; nothing calls it yet, so NPC replies still come from the story layer",
        evidence: &[
            (INFLUENCE, "pub fn resolve"),
            (INFLUENCE, "WORD_QUALITY_RULE_ID"),
            (INFLUENCE, "APPROACH_SKILLS"),
        ],
        browser_scripts: &["converse.mjs", "talk.mjs"],
    },
];

/// What must be built, in the shortest order that lets a table exercise every row above end
/// to end, each with the development scenario that exercises it. `DF_GAMEPLAY_TEST_FIGHT` is
/// the vehicle for the combat rows; the others are proposals of the same kind.
pub const BUILD_ORDER: &[(&str, &str)] = &[
    (
        "Typed rules facts a scenario can set: a fight, a hero's level and abilities, conditions, resources, lighting",
        "DF_GAMEPLAY_TEST_FIGHT=<creature>:<count>[,<creature>:<count>] with hero hit points, conditions and scene light selectable; used by every combat row",
    ),
    (
        "A general saving-throw resolver (ability, proficiency, Advantage, DC, Half on success) and a general condition set with source, duration and end triggers",
        "DF_GAMEPLAY_TEST_CONDITIONS=<condition>: a fight that starts with each condition on a hero or enemy and a trap that forces a save",
    ),
    (
        "Damage types, Resistance, Vulnerability, Immunity and temporary Hit Points in one resolver in SRD order, and the damage type on the wire",
        "DF_GAMEPLAY_TEST_FIGHT with a Skeleton (Bludgeoning), a resistant test creature and a fire source",
    ),
    (
        "Ability checks with Advantage and Disadvantage, passive checks, skills with their abilities, Help, Search, Hide, Influence",
        "DF_GAMEPLAY_TEST_CHECKS=<ability|skill>: one authored room per ability and skill with a visible DC ladder",
    ),
    (
        "Healing and recovery: Hit Point Dice, Short and Long Rest, stable recovery after 1d4 hours, Help to stabilise, potions, Exhaustion",
        "DF_GAMEPLAY_TEST_REST=<short|long|stable>: a party that starts hurt, spent and one hero stable at 0",
    ),
    (
        "Spellcasting: spell data, slots, components, attack and save spells, concentration, ritual casting, one caster class",
        "DF_GAMEPLAY_TEST_SPELLS=<spell>: a fight and an exploration room per implemented spell",
    ),
    (
        "Classes, levels, backgrounds, origin feats and species beyond level 1 Fighter, Soldier and Dwarf",
        "DF_GAMEPLAY_TEST_PARTY=<class:level,...>: a party built from the catalog for the fight and check scenarios",
    ),
    (
        "Exploration and social rules: travel, light and senses, hazards, traps, falling, Influence with attitude changes",
        "DF_GAMEPLAY_TEST_EXPLORE=<hazard|social>: an authored room per hazard and per attitude",
    ),
];

/// The matrix as a markdown report.
pub fn report() -> String {
    let mut out = String::new();
    out.push_str("# Rules coverage matrix (generated by df-rules coverage::report)\n\n");
    out.push_str("yes = works end to end for what level 1 needs; partial = a named subset; NO = nothing exists; unverified = a check or screen exists that this task did not run.\n\n");
    out.push_str("| Family | Planning | Implemented in df-rules | Reachable live | Native test | Browser check | TV and phone | Notes |\n|---|---|---|---|---|---|---|---|\n");
    for row in ROWS {
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} |\n",
            row.family,
            row.planning,
            row.rules.word(),
            row.live.word(),
            row.native_test.word(),
            row.browser.word(),
            row.display.word(),
            row.note
        ));
    }
    out.push_str("\n## What must be built, in order, with a development scenario for each\n\n");
    for (index, (build, scenario)) in BUILD_ORDER.iter().enumerate() {
        out.push_str(&format!(
            "{}. {build}\n   Scenario: {scenario}\n",
            index + 1
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn read(relative: &str) -> String {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        std::fs::read_to_string(root.join(relative))
            .unwrap_or_else(|error| panic!("{relative}: {error}"))
    }

    #[test]
    fn every_claimed_row_names_evidence_that_still_exists() {
        for row in ROWS {
            if matches!(row.rules, Yes | Partial) {
                assert!(
                    !row.evidence.is_empty(),
                    "{} claims rules without evidence",
                    row.family
                );
            }
            for (file, needle) in row.evidence {
                assert!(
                    read(file).contains(needle),
                    "{}: `{needle}` missing from {file}",
                    row.family
                );
            }
            for script in row.browser_scripts {
                assert!(
                    Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("../../development/browser-checks")
                        .join(script)
                        .exists(),
                    "{}: browser script {script} is gone",
                    row.family
                );
            }
            // A browser or display level above `No` needs a script or the live view to point at.
            if row.browser != No {
                assert!(
                    !row.browser_scripts.is_empty(),
                    "{} claims a browser check without a script",
                    row.family
                );
            }
            // Nothing is `Yes` for the browser or the screens: this task ran no browser or screen.
            assert_ne!(row.browser, Yes, "{}", row.family);
            assert_ne!(row.display, Yes, "{}", row.family);
        }
    }

    #[test]
    fn every_planning_family_is_in_the_matrix() {
        let planning = read("planning/rules-coverage.md");
        for id in [
            "R01", "R02", "R03", "R04", "R05", "R06", "R07", "R08", "R09", "R10", "R12", "R13",
            "R14", "R15",
        ] {
            assert!(
                planning.contains(&format!("| {id} |")),
                "{id} left the planning document"
            );
            assert!(
                ROWS.iter().any(|row| row.planning == id),
                "{id} has no matrix row"
            );
        }
    }

    #[test]
    fn the_report_lists_every_family_and_the_build_order() {
        let text = report();
        for row in ROWS {
            assert!(text.contains(row.family));
        }
        assert!(text.contains("DF_GAMEPLAY_TEST_FIGHT"));
        if let Ok(path) = std::env::var("DF_R01_COVERAGE_REPORT") {
            std::fs::write(path, text).unwrap();
        }
    }
}
