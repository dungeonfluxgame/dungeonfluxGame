//! Golden cases of every kind of roll, each with the SRD 5.2.1 clause its expected numbers come
//! from and the recorded faces it was resolved with. The warrior is the gauntlet's Champion
//! Fighter, level 3: Strength +3, Dexterity +2, Constitution +2, Wisdom +1, Proficiency Bonus +2.
use super::*;

const STRENGTH: i8 = 3;
const DEXTERITY: i8 = 2;
const CONSTITUTION: i8 = 2;
const WISDOM: i8 = 1;
const PROFICIENT: Proficiency = Proficiency::Proficient { bonus: 2 };

struct Golden {
    name: &'static str,
    source: &'static str,
    request: RollRequest,
    faces: &'static [u16],
    expected: Expected,
}

struct Expected {
    total: i16,
    outcome: RollOutcome,
    natural: Natural,
    /// The faces of the dice that counted, in roll order.
    kept: &'static [u16],
    modifiers: &'static [(ModifierLabel, i16)],
    target: Option<(TargetLabel, i16)>,
    /// The face Heroic Inspiration replaced.
    replaced: Option<u16>,
}

fn dc(value: i16) -> Option<Target> {
    Some(Target {
        label: TargetLabel::DifficultyClass,
        value,
    })
}

fn armor_class(value: i16) -> Option<Target> {
    Some(Target {
        label: TargetLabel::ArmorClass,
        value,
    })
}

fn d20(
    test: D20Test,
    mode: RollMode,
    ability_modifier: i8,
    proficiency: Proficiency,
    target: Option<Target>,
) -> RollRequest {
    RollRequest::D20(D20Request {
        test,
        mode,
        ability_modifier,
        proficiency,
        target,
        heroic_inspiration: None,
    })
}

fn with_inspiration(request: RollRequest, replaces_die: u8) -> RollRequest {
    match request {
        RollRequest::D20(request) => RollRequest::D20(D20Request {
            heroic_inspiration: Some(HeroicInspiration { replaces_die }),
            ..request
        }),
        RollRequest::Dice(request) => RollRequest::Dice(DiceRequest {
            heroic_inspiration: Some(HeroicInspiration { replaces_die }),
            ..request
        }),
        passive @ RollRequest::PassivePerception(_) => passive,
    }
}

fn melee(critical_from: u8) -> D20Test {
    D20Test::Attack {
        ability: Ability::Strength,
        kind: AttackKind::Melee,
        critical_from,
    }
}

fn ranged() -> D20Test {
    D20Test::Attack {
        ability: Ability::Dexterity,
        kind: AttackKind::Ranged,
        critical_from: NATURAL_TWENTY,
    }
}

const NATURAL_TWENTY: u8 = 20;
const IMPROVED_CRITICAL: u8 = 19;

fn dice(
    role: DieRole,
    count: u8,
    die: DieSize,
    modifier: i16,
    label: ModifierLabel,
) -> DiceRequest {
    DiceRequest {
        role,
        expression: DiceExpression {
            count,
            die,
            modifier,
        },
        modifier_label: label,
        critical: false,
        heroic_inspiration: None,
    }
}

fn golden_cases() -> Vec<Golden> {
    let normal = RollMode::Normal;
    let advantage = RollMode::Advantage;
    let disadvantage = RollMode::Disadvantage;
    let athletics = D20Test::SkillCheck(Skill::Athletics);
    vec![
        Golden {
            name: "a strength check that equals the DC succeeds",
            source: "page 6: a total that equals or exceeds the target number succeeds",
            request: d20(
                D20Test::AbilityCheck(Ability::Strength),
                normal,
                STRENGTH,
                Proficiency::None,
                dc(15),
            ),
            faces: &[12],
            expected: Expected {
                total: 15,
                outcome: RollOutcome::Success,
                natural: Natural::None,
                kept: &[12],
                modifiers: &[(ModifierLabel::Strength, 3)],
                target: Some((TargetLabel::DifficultyClass, 15)),
                replaced: None,
            },
        },
        Golden {
            name: "a strength check one short of the DC fails",
            source: "page 6: otherwise it fails",
            request: d20(
                D20Test::AbilityCheck(Ability::Strength),
                normal,
                STRENGTH,
                Proficiency::None,
                dc(15),
            ),
            faces: &[11],
            expected: Expected {
                total: 14,
                outcome: RollOutcome::Failure,
                natural: Natural::None,
                kept: &[11],
                modifiers: &[(ModifierLabel::Strength, 3)],
                target: Some((TargetLabel::DifficultyClass, 15)),
                replaced: None,
            },
        },
        Golden {
            name: "a natural 20 does not rescue an ability check short of the DC",
            source: concat!(
                "page 6 (no natural-roll rule for ability ",
                "checks; page 7 gives one to attack rolls)"
            ),
            request: d20(
                D20Test::AbilityCheck(Ability::Intelligence),
                normal,
                -1,
                Proficiency::None,
                dc(25),
            ),
            faces: &[20],
            expected: Expected {
                total: 19,
                outcome: RollOutcome::Failure,
                natural: Natural::None,
                kept: &[20],
                modifiers: &[(ModifierLabel::Intelligence, -1)],
                target: Some((TargetLabel::DifficultyClass, 25)),
                replaced: None,
            },
        },
        Golden {
            name: "a natural 1 does not fail an ability check the modifiers carry to the DC",
            source: "page 6 (no natural-roll rule for ability checks)",
            request: d20(
                D20Test::AbilityCheck(Ability::Strength),
                normal,
                10,
                Proficiency::Proficient { bonus: 6 },
                dc(15),
            ),
            faces: &[1],
            expected: Expected {
                total: 17,
                outcome: RollOutcome::Success,
                natural: Natural::None,
                kept: &[1],
                modifiers: &[
                    (ModifierLabel::Strength, 10),
                    (ModifierLabel::Proficiency, 6),
                ],
                target: Some((TargetLabel::DifficultyClass, 15)),
                replaced: None,
            },
        },
        Golden {
            name: "athletics with proficiency adds the strength modifier and the bonus",
            source: concat!(
                "pages 8 and 9: proficiency in a ",
                "skill adds the Proficiency Bonus; Athletics is Strength"
            ),
            request: d20(athletics, normal, STRENGTH, PROFICIENT, dc(15)),
            faces: &[10],
            expected: Expected {
                total: 15,
                outcome: RollOutcome::Success,
                natural: Natural::None,
                kept: &[10],
                modifiers: &[
                    (ModifierLabel::Strength, 3),
                    (ModifierLabel::Proficiency, 2),
                ],
                target: Some((TargetLabel::DifficultyClass, 15)),
                replaced: None,
            },
        },
        Golden {
            name: "a skill without proficiency adds the ability modifier alone",
            source: "page 8: without proficiency a creature does not add its Proficiency Bonus",
            request: d20(
                D20Test::SkillCheck(Skill::Stealth),
                normal,
                DEXTERITY,
                Proficiency::None,
                dc(15),
            ),
            faces: &[12],
            expected: Expected {
                total: 14,
                outcome: RollOutcome::Failure,
                natural: Natural::None,
                kept: &[12],
                modifiers: &[(ModifierLabel::Dexterity, 2)],
                target: Some((TargetLabel::DifficultyClass, 15)),
                replaced: None,
            },
        },
        Golden {
            name: "expertise doubles the Proficiency Bonus for the skill",
            source: "page 182: Expertise doubles the Proficiency Bonus for that check",
            request: d20(
                D20Test::SkillCheck(Skill::Stealth),
                normal,
                DEXTERITY,
                Proficiency::Expertise { bonus: 2 },
                dc(15),
            ),
            faces: &[9],
            expected: Expected {
                total: 15,
                outcome: RollOutcome::Success,
                natural: Natural::None,
                kept: &[9],
                modifiers: &[(ModifierLabel::Dexterity, 2), (ModifierLabel::Expertise, 4)],
                target: Some((TargetLabel::DifficultyClass, 15)),
                replaced: None,
            },
        },
        Golden {
            name: "advantage keeps the higher of two d20s and sets the lower aside",
            source: "page 8: roll two d20s and use the higher with Advantage",
            request: d20(athletics, advantage, STRENGTH, PROFICIENT, dc(20)),
            faces: &[4, 17],
            expected: Expected {
                total: 22,
                outcome: RollOutcome::Success,
                natural: Natural::None,
                kept: &[17],
                modifiers: &[
                    (ModifierLabel::Strength, 3),
                    (ModifierLabel::Proficiency, 2),
                ],
                target: Some((TargetLabel::DifficultyClass, 20)),
                replaced: None,
            },
        },
        Golden {
            name: "disadvantage keeps the lower of two d20s",
            source: "page 8: use the lower roll with Disadvantage (a roll of 18 and 3 uses the 3)",
            request: d20(athletics, disadvantage, STRENGTH, PROFICIENT, dc(10)),
            faces: &[18, 3],
            expected: Expected {
                total: 8,
                outcome: RollOutcome::Failure,
                natural: Natural::None,
                kept: &[3],
                modifiers: &[
                    (ModifierLabel::Strength, 3),
                    (ModifierLabel::Proficiency, 2),
                ],
                target: Some((TargetLabel::DifficultyClass, 10)),
                replaced: None,
            },
        },
        Golden {
            name: "advantage with equal dice keeps the first",
            source: "page 8: either die gives the same number",
            request: d20(athletics, advantage, STRENGTH, PROFICIENT, dc(15)),
            faces: &[10, 10],
            expected: Expected {
                total: 15,
                outcome: RollOutcome::Success,
                natural: Natural::None,
                kept: &[10],
                modifiers: &[
                    (ModifierLabel::Strength, 3),
                    (ModifierLabel::Proficiency, 2),
                ],
                target: Some((TargetLabel::DifficultyClass, 15)),
                replaced: None,
            },
        },
        Golden {
            name: "passive Perception is 10 plus the modifiers",
            source: concat!(
                "page 22 and 186: Wisdom 15 and proficiency ",
                "in Perception is a Passive Perception of 14"
            ),
            request: RollRequest::PassivePerception(PassivePerceptionRequest {
                ability_modifier: 2,
                proficiency: PROFICIENT,
                mode: normal,
            }),
            faces: &[],
            expected: Expected {
                total: 14,
                outcome: RollOutcome::Score,
                natural: Natural::None,
                kept: &[],
                modifiers: &[
                    (ModifierLabel::Base, 10),
                    (ModifierLabel::Wisdom, 2),
                    (ModifierLabel::Proficiency, 2),
                ],
                target: None,
                replaced: None,
            },
        },
        Golden {
            name: "passive Perception with Advantage is 5 higher",
            source: "page 186: with Advantage on Perception checks the score becomes 19",
            request: RollRequest::PassivePerception(PassivePerceptionRequest {
                ability_modifier: 2,
                proficiency: PROFICIENT,
                mode: advantage,
            }),
            faces: &[],
            expected: Expected {
                total: 19,
                outcome: RollOutcome::Score,
                natural: Natural::None,
                kept: &[],
                modifiers: &[
                    (ModifierLabel::Base, 10),
                    (ModifierLabel::Wisdom, 2),
                    (ModifierLabel::Proficiency, 2),
                    (ModifierLabel::Advantage, 5),
                ],
                target: None,
                replaced: None,
            },
        },
        Golden {
            name: "passive Perception with Disadvantage is 5 lower",
            source: "page 186: with Disadvantage the score decreases by 5",
            request: RollRequest::PassivePerception(PassivePerceptionRequest {
                ability_modifier: WISDOM,
                proficiency: Proficiency::None,
                mode: disadvantage,
            }),
            faces: &[],
            expected: Expected {
                total: 6,
                outcome: RollOutcome::Score,
                natural: Natural::None,
                kept: &[],
                modifiers: &[
                    (ModifierLabel::Base, 10),
                    (ModifierLabel::Wisdom, 1),
                    (ModifierLabel::Disadvantage, -5),
                ],
                target: None,
                replaced: None,
            },
        },
        Golden {
            name: "a Strength save with proficiency adds the Proficiency Bonus",
            source: "page 7: add the Proficiency Bonus to a save the creature is proficient in",
            request: d20(
                D20Test::SavingThrow(Ability::Strength),
                normal,
                STRENGTH,
                PROFICIENT,
                dc(13),
            ),
            faces: &[8],
            expected: Expected {
                total: 13,
                outcome: RollOutcome::Success,
                natural: Natural::None,
                kept: &[8],
                modifiers: &[
                    (ModifierLabel::Strength, 3),
                    (ModifierLabel::Proficiency, 2),
                ],
                target: Some((TargetLabel::DifficultyClass, 13)),
                replaced: None,
            },
        },
        Golden {
            name: "a Dexterity save without proficiency adds the modifier alone",
            source: "page 7: only a save the creature is proficient in adds the Proficiency Bonus",
            request: d20(
                D20Test::SavingThrow(Ability::Dexterity),
                normal,
                DEXTERITY,
                Proficiency::None,
                dc(13),
            ),
            faces: &[10],
            expected: Expected {
                total: 12,
                outcome: RollOutcome::Failure,
                natural: Natural::None,
                kept: &[10],
                modifiers: &[(ModifierLabel::Dexterity, 2)],
                target: Some((TargetLabel::DifficultyClass, 13)),
                replaced: None,
            },
        },
        Golden {
            name: "a natural 20 does not pass a saving throw short of the DC",
            source: "page 7 gives natural 1 and 20 an effect on attack rolls only",
            request: d20(
                D20Test::SavingThrow(Ability::Constitution),
                normal,
                CONSTITUTION,
                PROFICIENT,
                dc(30),
            ),
            faces: &[20],
            expected: Expected {
                total: 24,
                outcome: RollOutcome::Failure,
                natural: Natural::None,
                kept: &[20],
                modifiers: &[
                    (ModifierLabel::Constitution, 2),
                    (ModifierLabel::Proficiency, 2),
                ],
                target: Some((TargetLabel::DifficultyClass, 30)),
                replaced: None,
            },
        },
        Golden {
            name: "a melee attack that equals the armor class hits",
            source: "page 7: an attack roll hits if the roll equals or exceeds the Armor Class",
            request: d20(
                melee(NATURAL_TWENTY),
                normal,
                STRENGTH,
                PROFICIENT,
                armor_class(16),
            ),
            faces: &[11],
            expected: Expected {
                total: 16,
                outcome: RollOutcome::Hit,
                natural: Natural::None,
                kept: &[11],
                modifiers: &[
                    (ModifierLabel::Strength, 3),
                    (ModifierLabel::Proficiency, 2),
                ],
                target: Some((TargetLabel::ArmorClass, 16)),
                replaced: None,
            },
        },
        Golden {
            name: "a melee attack one short of the armor class misses",
            source: "page 7",
            request: d20(
                melee(NATURAL_TWENTY),
                normal,
                STRENGTH,
                PROFICIENT,
                armor_class(16),
            ),
            faces: &[10],
            expected: Expected {
                total: 15,
                outcome: RollOutcome::Miss,
                natural: Natural::None,
                kept: &[10],
                modifiers: &[
                    (ModifierLabel::Strength, 3),
                    (ModifierLabel::Proficiency, 2),
                ],
                target: Some((TargetLabel::ArmorClass, 16)),
                replaced: None,
            },
        },
        Golden {
            name: "a ranged attack adds the Dexterity modifier",
            source: "page 7, Attack Roll Abilities: Dexterity for a ranged attack with a weapon",
            request: d20(ranged(), normal, DEXTERITY, PROFICIENT, armor_class(14)),
            faces: &[10],
            expected: Expected {
                total: 14,
                outcome: RollOutcome::Hit,
                natural: Natural::None,
                kept: &[10],
                modifiers: &[
                    (ModifierLabel::Dexterity, 2),
                    (ModifierLabel::Proficiency, 2),
                ],
                target: Some((TargetLabel::ArmorClass, 14)),
                replaced: None,
            },
        },
        Golden {
            name: "a natural 20 hits whatever the armor class and is a Critical Hit",
            source: concat!(
                "page 7 and 179: a natural 20 hits ",
                "regardless of modifiers or AC, a Critical Hit"
            ),
            request: d20(
                melee(NATURAL_TWENTY),
                normal,
                STRENGTH,
                PROFICIENT,
                armor_class(30),
            ),
            faces: &[20],
            expected: Expected {
                total: 25,
                outcome: RollOutcome::CriticalHit,
                natural: Natural::Twenty,
                kept: &[20],
                modifiers: &[
                    (ModifierLabel::Strength, 3),
                    (ModifierLabel::Proficiency, 2),
                ],
                target: Some((TargetLabel::ArmorClass, 30)),
                replaced: None,
            },
        },
        Golden {
            name: "a natural 1 misses even when the total beats the armor class",
            source: "page 7: a natural 1 misses regardless of modifiers or AC",
            request: d20(
                melee(NATURAL_TWENTY),
                normal,
                10,
                Proficiency::Proficient { bonus: 6 },
                armor_class(5),
            ),
            faces: &[1],
            expected: Expected {
                total: 17,
                outcome: RollOutcome::Miss,
                natural: Natural::One,
                kept: &[1],
                modifiers: &[
                    (ModifierLabel::Strength, 10),
                    (ModifierLabel::Proficiency, 6),
                ],
                target: Some((TargetLabel::ArmorClass, 5)),
                replaced: None,
            },
        },
        Golden {
            name: "a 19 is only a hit for a fighter without Improved Critical",
            source: concat!(
                "pages 7 and 49: a Critical Hit is ",
                "a natural 20 unless a feature widens the range"
            ),
            request: d20(
                melee(NATURAL_TWENTY),
                normal,
                STRENGTH,
                PROFICIENT,
                armor_class(30),
            ),
            faces: &[19],
            expected: Expected {
                total: 24,
                outcome: RollOutcome::Miss,
                natural: Natural::None,
                kept: &[19],
                modifiers: &[
                    (ModifierLabel::Strength, 3),
                    (ModifierLabel::Proficiency, 2),
                ],
                target: Some((TargetLabel::ArmorClass, 30)),
                replaced: None,
            },
        },
        Golden {
            name: "a 19 is a Critical Hit for the Champion",
            source: "page 49: Improved Critical scores a Critical Hit on a roll of 19 or 20",
            request: d20(
                melee(IMPROVED_CRITICAL),
                normal,
                STRENGTH,
                PROFICIENT,
                armor_class(30),
            ),
            faces: &[19],
            expected: Expected {
                total: 24,
                outcome: RollOutcome::CriticalHit,
                natural: Natural::None,
                kept: &[19],
                modifiers: &[
                    (ModifierLabel::Strength, 3),
                    (ModifierLabel::Proficiency, 2),
                ],
                target: Some((TargetLabel::ArmorClass, 30)),
                replaced: None,
            },
        },
        Golden {
            name: "an 18 is not a Critical Hit for the Champion before Superior Critical",
            source: "page 49: Superior Critical widens the range to 18 to 20 only at level 15",
            request: d20(
                melee(IMPROVED_CRITICAL),
                normal,
                STRENGTH,
                PROFICIENT,
                armor_class(30),
            ),
            faces: &[18],
            expected: Expected {
                total: 23,
                outcome: RollOutcome::Miss,
                natural: Natural::None,
                kept: &[18],
                modifiers: &[
                    (ModifierLabel::Strength, 3),
                    (ModifierLabel::Proficiency, 2),
                ],
                target: Some((TargetLabel::ArmorClass, 30)),
                replaced: None,
            },
        },
        Golden {
            name: "an attack with advantage keeps the higher d20",
            source: "page 8 applied to an attack roll (page 7)",
            request: d20(
                melee(NATURAL_TWENTY),
                advantage,
                STRENGTH,
                PROFICIENT,
                armor_class(15),
            ),
            faces: &[3, 15],
            expected: Expected {
                total: 20,
                outcome: RollOutcome::Hit,
                natural: Natural::None,
                kept: &[15],
                modifiers: &[
                    (ModifierLabel::Strength, 3),
                    (ModifierLabel::Proficiency, 2),
                ],
                target: Some((TargetLabel::ArmorClass, 15)),
                replaced: None,
            },
        },
        Golden {
            name: "an attack with disadvantage keeps the lower d20 and a natural 1 then misses",
            source: "pages 7 and 8",
            request: d20(
                melee(NATURAL_TWENTY),
                disadvantage,
                STRENGTH,
                PROFICIENT,
                armor_class(5),
            ),
            faces: &[18, 1],
            expected: Expected {
                total: 6,
                outcome: RollOutcome::Miss,
                natural: Natural::One,
                kept: &[1],
                modifiers: &[
                    (ModifierLabel::Strength, 3),
                    (ModifierLabel::Proficiency, 2),
                ],
                target: Some((TargetLabel::ArmorClass, 5)),
                replaced: None,
            },
        },
        Golden {
            name: "initiative with advantage is a Dexterity check with no target",
            source: concat!(
                "pages 13 and 49: Initiative is a ",
                "Dexterity check; the Champion has Advantage on it"
            ),
            request: d20(
                D20Test::Initiative,
                advantage,
                DEXTERITY,
                Proficiency::None,
                None,
            ),
            faces: &[5, 12],
            expected: Expected {
                total: 14,
                outcome: RollOutcome::Rolled,
                natural: Natural::None,
                kept: &[12],
                modifiers: &[(ModifierLabel::Dexterity, 2)],
                target: None,
                replaced: None,
            },
        },
        Golden {
            name: "a Death Saving Throw of 10 succeeds",
            source: "page 17: if the roll is 10 or higher, you succeed",
            request: d20(D20Test::DeathSave, normal, 0, Proficiency::None, None),
            faces: &[10],
            expected: Expected {
                total: 10,
                outcome: RollOutcome::DeathSaveSuccess,
                natural: Natural::None,
                kept: &[10],
                modifiers: &[],
                target: Some((TargetLabel::DifficultyClass, 10)),
                replaced: None,
            },
        },
        Golden {
            name: "a Death Saving Throw of 9 fails",
            source: "page 17: otherwise, you fail",
            request: d20(D20Test::DeathSave, normal, 0, Proficiency::None, None),
            faces: &[9],
            expected: Expected {
                total: 9,
                outcome: RollOutcome::DeathSaveFailure,
                natural: Natural::None,
                kept: &[9],
                modifiers: &[],
                target: Some((TargetLabel::DifficultyClass, 10)),
                replaced: None,
            },
        },
        Golden {
            name: "a natural 1 on a Death Saving Throw is two failures",
            source: "page 18: when you roll a 1 on the d20 you suffer two failures",
            request: d20(D20Test::DeathSave, normal, 0, Proficiency::None, None),
            faces: &[1],
            expected: Expected {
                total: 1,
                outcome: RollOutcome::DeathSaveTwoFailures,
                natural: Natural::One,
                kept: &[1],
                modifiers: &[],
                target: Some((TargetLabel::DifficultyClass, 10)),
                replaced: None,
            },
        },
        Golden {
            name: "a natural 20 on a Death Saving Throw regains a Hit Point",
            source: "page 18: if you roll a 20 on the d20, you regain 1 Hit Point",
            request: d20(D20Test::DeathSave, normal, 0, Proficiency::None, None),
            faces: &[20],
            expected: Expected {
                total: 20,
                outcome: RollOutcome::DeathSaveRevived,
                natural: Natural::Twenty,
                kept: &[20],
                modifiers: &[],
                target: Some((TargetLabel::DifficultyClass, 10)),
                replaced: None,
            },
        },
        Golden {
            name: "a Death Saving Throw with Advantage keeps the higher d20",
            source: "pages 8 and 49: Survivor gives Advantage on Death Saving Throws",
            request: d20(D20Test::DeathSave, advantage, 0, Proficiency::None, None),
            faces: &[2, 12],
            expected: Expected {
                total: 12,
                outcome: RollOutcome::DeathSaveSuccess,
                natural: Natural::None,
                kept: &[12],
                modifiers: &[],
                target: Some((TargetLabel::DifficultyClass, 10)),
                replaced: None,
            },
        },
        Golden {
            name: "an Athletics check to end a grapple meets the escape DC",
            source: concat!(
                "pages 182 and 190: an Athletics or Acrobatics ",
                "check against 8 plus Strength modifier and Proficiency Bonus"
            ),
            request: d20(
                athletics,
                normal,
                STRENGTH,
                PROFICIENT,
                Some(Target {
                    label: TargetLabel::EscapeDc,
                    value: unarmed_strike_dc(3, 2),
                }),
            ),
            faces: &[8],
            expected: Expected {
                total: 13,
                outcome: RollOutcome::Success,
                natural: Natural::None,
                kept: &[8],
                modifiers: &[
                    (ModifierLabel::Strength, 3),
                    (ModifierLabel::Proficiency, 2),
                ],
                target: Some((TargetLabel::EscapeDc, 13)),
                replaced: None,
            },
        },
        Golden {
            name: "finding a hidden creature is a Perception check against its Stealth total",
            source: concat!(
                "page 183: the hider's Stealth total is the ",
                "DC for a Wisdom (Perception) check to find it"
            ),
            request: d20(
                D20Test::SkillCheck(Skill::Perception),
                normal,
                WISDOM,
                Proficiency::None,
                Some(Target {
                    label: TargetLabel::StealthTotal,
                    value: 17,
                }),
            ),
            faces: &[15],
            expected: Expected {
                total: 16,
                outcome: RollOutcome::Failure,
                natural: Natural::None,
                kept: &[15],
                modifiers: &[(ModifierLabel::Wisdom, 1)],
                target: Some((TargetLabel::StealthTotal, 17)),
                replaced: None,
            },
        },
        Golden {
            name: "Heroic Inspiration replaces the d20 that was rolled",
            source: "pages 8 and 183: expend it to reroll any die and use the new roll",
            request: with_inspiration(d20(athletics, normal, STRENGTH, PROFICIENT, dc(15)), 0),
            faces: &[3, 17],
            expected: Expected {
                total: 22,
                outcome: RollOutcome::Success,
                natural: Natural::None,
                kept: &[17],
                modifiers: &[
                    (ModifierLabel::Strength, 3),
                    (ModifierLabel::Proficiency, 2),
                ],
                target: Some((TargetLabel::DifficultyClass, 15)),
                replaced: Some(3),
            },
        },
        Golden {
            name: "Heroic Inspiration rerolls one die of an Advantage roll, not both",
            source: concat!(
                "page 8: with Advantage you reroll only one die, ",
                "the one you choose (a 3 and an 18)"
            ),
            request: with_inspiration(d20(athletics, advantage, STRENGTH, PROFICIENT, dc(15)), 0),
            faces: &[3, 18, 12],
            expected: Expected {
                total: 23,
                outcome: RollOutcome::Success,
                natural: Natural::None,
                kept: &[18],
                modifiers: &[
                    (ModifierLabel::Strength, 3),
                    (ModifierLabel::Proficiency, 2),
                ],
                target: Some((TargetLabel::DifficultyClass, 15)),
                replaced: Some(3),
            },
        },
        Golden {
            name: concat!(
                "Heroic Inspiration on the kept die of a ",
                "Disadvantage roll falls back to the other die"
            ),
            source: concat!(
                "page 8: you must use the new roll, ",
                "and a Disadvantage roll still uses the lower die"
            ),
            request: with_inspiration(
                d20(athletics, disadvantage, STRENGTH, PROFICIENT, dc(15)),
                1,
            ),
            faces: &[14, 6, 20],
            expected: Expected {
                total: 19,
                outcome: RollOutcome::Success,
                natural: Natural::None,
                kept: &[14],
                modifiers: &[
                    (ModifierLabel::Strength, 3),
                    (ModifierLabel::Proficiency, 2),
                ],
                target: Some((TargetLabel::DifficultyClass, 15)),
                replaced: Some(6),
            },
        },
        Golden {
            name: "Heroic Inspiration can replace a damage die",
            source: "pages 8 and 183: reroll any die immediately after rolling it",
            request: with_inspiration(
                RollRequest::Dice(DiceRequest::damage(
                    DiceExpression {
                        count: 2,
                        die: DieSize::D6,
                        modifier: 3,
                    },
                    Ability::Strength,
                    false,
                )),
                1,
            ),
            faces: &[2, 1, 6],
            expected: Expected {
                total: 11,
                outcome: RollOutcome::Rolled,
                natural: Natural::None,
                kept: &[2, 6],
                modifiers: &[(ModifierLabel::Strength, 3)],
                target: None,
                replaced: Some(1),
            },
        },
        Golden {
            name: "a longsword hit rolls 1d8 plus the Strength modifier",
            source: concat!(
                "page 22: add the same ability modifier ",
                "to damage rolls; damage dice from Equipment"
            ),
            request: RollRequest::Dice(DiceRequest::damage(
                DiceExpression {
                    count: 1,
                    die: DieSize::D8,
                    modifier: 3,
                },
                Ability::Strength,
                false,
            )),
            faces: &[5],
            expected: Expected {
                total: 8,
                outcome: RollOutcome::Rolled,
                natural: Natural::None,
                kept: &[5],
                modifiers: &[(ModifierLabel::Strength, 3)],
                target: None,
                replaced: None,
            },
        },
        Golden {
            name: "a Critical Hit rolls the damage dice twice and adds the modifier once",
            source: concat!(
                "page 16: roll the damage dice twice, ",
                "add them together, and add any relevant modifiers"
            ),
            request: RollRequest::Dice(DiceRequest::damage(
                DiceExpression {
                    count: 1,
                    die: DieSize::D8,
                    modifier: 3,
                },
                Ability::Strength,
                true,
            )),
            faces: &[5, 7],
            expected: Expected {
                total: 15,
                outcome: RollOutcome::Rolled,
                natural: Natural::None,
                kept: &[5, 7],
                modifiers: &[(ModifierLabel::Strength, 3)],
                target: None,
                replaced: None,
            },
        },
        Golden {
            name: "a dagger Critical Hit rolls 2d4",
            source: concat!(
                "page 16: a Critical Hit with a Dagger ",
                "rolls 2d4 for the damage rather than 1d4"
            ),
            request: RollRequest::Dice(DiceRequest::damage(
                DiceExpression {
                    count: 1,
                    die: DieSize::D4,
                    modifier: 0,
                },
                Ability::Dexterity,
                true,
            )),
            faces: &[4, 4],
            expected: Expected {
                total: 8,
                outcome: RollOutcome::Rolled,
                natural: Natural::None,
                kept: &[4, 4],
                modifiers: &[],
                target: None,
                replaced: None,
            },
        },
        Golden {
            name: "a greataxe rolls 1d12",
            source: "page 16 and Equipment: a die of 12 sides rolled for damage",
            request: RollRequest::Dice(dice(
                DieRole::Damage,
                1,
                DieSize::D12,
                0,
                ModifierLabel::Strength,
            )),
            faces: &[12],
            expected: Expected {
                total: 12,
                outcome: RollOutcome::Rolled,
                natural: Natural::None,
                kept: &[12],
                modifiers: &[],
                target: None,
                replaced: None,
            },
        },
        Golden {
            name: "healing rolls its dice and adds the modifier",
            source: "page 17: hit points restored by healing; a potion rolls 2d4 plus 2",
            request: RollRequest::Dice(dice(
                DieRole::Healing,
                2,
                DieSize::D4,
                2,
                ModifierLabel::Flat,
            )),
            faces: &[3, 4],
            expected: Expected {
                total: 9,
                outcome: RollOutcome::Rolled,
                natural: Natural::None,
                kept: &[3, 4],
                modifiers: &[(ModifierLabel::Flat, 2)],
                target: None,
                replaced: None,
            },
        },
        Golden {
            name: "a Fighter's hit die is a d10 plus the Constitution modifier",
            source: concat!(
                "pages 22 and 183: Hit Point Dice are ",
                "spent during a Short Rest to regain Hit Points"
            ),
            request: RollRequest::Dice(dice(
                DieRole::HitDie,
                1,
                DieSize::D10,
                2,
                ModifierLabel::Constitution,
            )),
            faces: &[7],
            expected: Expected {
                total: 9,
                outcome: RollOutcome::Rolled,
                natural: Natural::None,
                kept: &[7],
                modifiers: &[(ModifierLabel::Constitution, 2)],
                target: None,
                replaced: None,
            },
        },
        Golden {
            name: "a bonus die is one die added by a feature",
            source: "page 106: bonus dice come from individual features such as Bless",
            request: RollRequest::Dice(dice(
                DieRole::Bonus,
                1,
                DieSize::D6,
                0,
                ModifierLabel::Flat,
            )),
            faces: &[6],
            expected: Expected {
                total: 6,
                outcome: RollOutcome::Rolled,
                natural: Natural::None,
                kept: &[6],
                modifiers: &[],
                target: None,
                replaced: None,
            },
        },
        Golden {
            name: "a d20 table roll is a single d20",
            source: "page 16 and tables: a table is rolled with the die it names",
            request: RollRequest::Dice(dice(
                DieRole::Table,
                1,
                DieSize::D20,
                0,
                ModifierLabel::Flat,
            )),
            faces: &[20],
            expected: Expected {
                total: 20,
                outcome: RollOutcome::Rolled,
                natural: Natural::None,
                kept: &[20],
                modifiers: &[],
                target: None,
                replaced: None,
            },
        },
        Golden {
            name: "a percentile roll of two zeros is 100",
            source: "page 27: the last row of the 1d100 Trinkets table is 00",
            request: RollRequest::Dice(dice(
                DieRole::Table,
                1,
                DieSize::D100,
                0,
                ModifierLabel::Flat,
            )),
            faces: &[10, 10],
            expected: Expected {
                total: 100,
                outcome: RollOutcome::Rolled,
                natural: Natural::None,
                kept: &[10, 10],
                modifiers: &[],
                target: None,
                replaced: None,
            },
        },
        Golden {
            name: "a percentile roll of tens 00 and ones 1 is 1",
            source: "page 26: the first row of the 1d100 Trinkets table is 01",
            request: RollRequest::Dice(dice(
                DieRole::Table,
                1,
                DieSize::D100,
                0,
                ModifierLabel::Flat,
            )),
            faces: &[10, 1],
            expected: Expected {
                total: 1,
                outcome: RollOutcome::Rolled,
                natural: Natural::None,
                kept: &[10, 1],
                modifiers: &[],
                target: None,
                replaced: None,
            },
        },
        Golden {
            name: "a percentile roll reads tens then ones",
            source: "page 27: row 99 of the Trinkets table, tens 90 and ones 9",
            request: RollRequest::Dice(dice(
                DieRole::Table,
                1,
                DieSize::D100,
                0,
                ModifierLabel::Flat,
            )),
            faces: &[9, 9],
            expected: Expected {
                total: 99,
                outcome: RollOutcome::Rolled,
                natural: Natural::None,
                kept: &[9, 9],
                modifiers: &[],
                target: None,
                replaced: None,
            },
        },
        Golden {
            name: "a percentile roll of tens 00 and ones 0 plus a modifier counts as 100",
            source: "page 27: 00 is 100",
            request: RollRequest::Dice(dice(
                DieRole::Table,
                1,
                DieSize::D100,
                5,
                ModifierLabel::Flat,
            )),
            faces: &[10, 10],
            expected: Expected {
                total: 105,
                outcome: RollOutcome::Rolled,
                natural: Natural::None,
                kept: &[10, 10],
                modifiers: &[(ModifierLabel::Flat, 5)],
                target: None,
                replaced: None,
            },
        },
    ]
}

#[test]
fn every_golden_case_resolves_to_its_recorded_numbers() {
    for case in golden_cases() {
        let result = case
            .request
            .resolve(case.faces)
            .unwrap_or_else(|error| panic!("{}: {error} ({})", case.name, case.source));
        let kept: Vec<u16> = result
            .dice
            .iter()
            .filter(|die| die.kept)
            .map(|die| die.face)
            .collect();
        let modifiers: Vec<(ModifierLabel, i16)> = result
            .modifiers
            .iter()
            .map(|modifier| (modifier.label, modifier.value))
            .collect();
        let target = result.target.map(|target| (target.label, target.value));
        let name = case.name;
        assert_eq!(result.total, case.expected.total, "{name}: total");
        assert_eq!(result.outcome, case.expected.outcome, "{name}: outcome");
        assert_eq!(result.natural, case.expected.natural, "{name}: natural");
        assert_eq!(kept, case.expected.kept, "{name}: kept faces");
        assert_eq!(modifiers, case.expected.modifiers, "{name}: modifiers");
        assert_eq!(target, case.expected.target, "{name}: target");
        assert_eq!(
            result.inspiration_reroll.map(|die| die.face),
            case.expected.replaced,
            "{name}: replaced face"
        );
    }
}

#[test]
fn every_golden_case_draws_exactly_the_faces_it_was_recorded_with() {
    for case in golden_cases() {
        let plan = case.request.draw_plan().expect("a golden request plans");
        assert_eq!(plan.len(), case.faces.len(), "{}: draw count", case.name);
    }
}

#[test]
fn every_golden_case_cites_a_page_of_the_pinned_source() {
    for case in golden_cases() {
        assert!(
            case.source.contains("page"),
            "{}: {}",
            case.name,
            case.source
        );
        let result = case
            .request
            .resolve(case.faces)
            .expect("a golden request resolves");
        assert!(
            result.subject.source().starts_with("SRD-5.2.1:page-"),
            "{}: {}",
            case.name,
            result.subject.source()
        );
    }
}

#[test]
fn the_golden_table_covers_every_outcome_and_every_die_size() {
    let results: Vec<RollResult> = golden_cases()
        .iter()
        .map(|case| case.request.resolve(case.faces).expect("resolves"))
        .collect();
    for outcome in RollOutcome::ALL {
        assert!(
            results.iter().any(|result| result.outcome == *outcome),
            "no golden case has the outcome {}",
            outcome.key()
        );
    }
    let sides: Vec<u16> = results
        .iter()
        .flat_map(|result| result.dice.iter().map(|die| die.sides))
        .collect();
    for size in [4, 6, 8, 10, 12, 20] {
        assert!(sides.contains(&size), "no golden case rolls a d{size}");
    }
}
