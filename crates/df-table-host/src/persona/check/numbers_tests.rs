//! What the dungeon master says of a call and of a result: the exact words for each kind of roll.

use df_rules::roll::{
    Ability, AttackKind, D20Request, D20Test, DiceExpression, DiceRequest, DieRole, DieSize,
    HeroicInspiration, ModifierLabel, PassivePerceptionRequest, Proficiency, RollMode, RollRequest,
    Skill, Target, TargetLabel,
};

use super::numbers::{number_words, spoken_call, spoken_result, test_name};

const PROFICIENT: Proficiency = Proficiency::Proficient { bonus: 2 };

fn difficulty(value: i16) -> Option<Target> {
    Some(Target {
        label: TargetLabel::DifficultyClass,
        value,
    })
}

fn target(label: TargetLabel, value: i16) -> Option<Target> {
    Some(Target { label, value })
}

fn test(
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

fn athletics(mode: RollMode) -> RollRequest {
    test(
        D20Test::SkillCheck(Skill::Athletics),
        mode,
        3,
        PROFICIENT,
        difficulty(15),
    )
}

fn attack(critical_from: u8) -> RollRequest {
    test(
        D20Test::Attack {
            ability: Ability::Strength,
            kind: AttackKind::Melee,
            critical_from,
        },
        RollMode::Normal,
        3,
        PROFICIENT,
        target(TargetLabel::ArmorClass, 16),
    )
}

fn said(request: &RollRequest, faces: &[u16]) -> String {
    let result = request.resolve(faces).expect("the faces fit the roll");
    let line = spoken_result(request, &result);
    assert!(
        !line.chars().any(|letter| letter.is_ascii_digit()),
        "a spoken line writes its numbers as words: {line}"
    );
    line
}

fn called(request: &RollRequest) -> String {
    let line = spoken_call(request);
    assert!(
        !line.chars().any(|letter| letter.is_ascii_digit()),
        "a spoken call writes its numbers as words: {line}"
    );
    line
}

#[test]
fn numbers_are_said_as_english_words() {
    for (value, words) in [
        (0, "zero"),
        (7, "seven"),
        (13, "thirteen"),
        (20, "twenty"),
        (21, "twenty-one"),
        (42, "forty-two"),
        (99, "ninety-nine"),
        (100, "one hundred"),
        (105, "one hundred five"),
        (342, "three hundred forty-two"),
        (1_000, "one thousand"),
        (1_234, "one thousand two hundred thirty-four"),
        (-3, "minus three"),
    ] {
        assert_eq!(number_words(value), words, "{value}");
    }
}

#[test]
fn an_ability_check_reads_the_die_each_source_the_total_against_the_dc_and_the_verdict() {
    assert_eq!(
        said(&athletics(RollMode::Normal), &[14]),
        "Fourteen on the die, plus three for Strength, plus two for proficiency. \
         Nineteen against fifteen. A success."
    );
    assert_eq!(
        said(&athletics(RollMode::Normal), &[9]),
        "Nine on the die, plus three for Strength, plus two for proficiency. \
         Fourteen against fifteen. A failure."
    );
}

#[test]
fn advantage_says_both_dice_and_which_one_counts() {
    assert_eq!(
        said(&athletics(RollMode::Advantage), &[17, 5]),
        "Seventeen and five with advantage, so seventeen counts, plus three for Strength, plus \
         two for proficiency. Twenty-two against fifteen. A success."
    );
}

#[test]
fn disadvantage_says_both_dice_and_that_the_lower_one_counts() {
    assert_eq!(
        said(&athletics(RollMode::Disadvantage), &[17, 5]),
        "Seventeen and five with disadvantage, so five counts, plus three for Strength, plus two \
         for proficiency. Ten against fifteen. A failure."
    );
}

#[test]
fn a_saving_throw_names_its_ability_and_the_proficiency_it_has() {
    let save = test(
        D20Test::SavingThrow(Ability::Constitution),
        RollMode::Normal,
        2,
        PROFICIENT,
        difficulty(15),
    );
    assert_eq!(
        said(&save, &[9]),
        "Nine on the die, plus two for Constitution, plus two for proficiency. \
         Thirteen against fifteen. A failure."
    );
}

#[test]
fn an_attack_is_set_against_the_armor_class_and_a_crit_range_hit_is_a_critical_hit() {
    assert_eq!(
        said(&attack(19), &[19]),
        "Nineteen on the die, plus three for Strength, plus two for proficiency. \
         Twenty-four against armor class sixteen. A critical hit."
    );
    assert_eq!(
        said(&attack(19), &[12]),
        "Twelve on the die, plus three for Strength, plus two for proficiency. \
         Seventeen against armor class sixteen. A hit."
    );
}

#[test]
fn a_natural_one_and_a_natural_twenty_on_an_attack_are_marked() {
    assert_eq!(
        said(&attack(19), &[1]),
        "One on the die, plus three for Strength, plus two for proficiency. \
         Six against armor class sixteen. A natural one, which always misses."
    );
    assert_eq!(
        said(&attack(19), &[20]),
        "Twenty on the die, plus three for Strength, plus two for proficiency. \
         Twenty-five against armor class sixteen. A natural twenty, a critical hit."
    );
}

#[test]
fn a_natural_twenty_on_an_ability_check_is_not_marked() {
    assert_eq!(
        said(&athletics(RollMode::Normal), &[20]),
        "Twenty on the die, plus three for Strength, plus two for proficiency. \
         Twenty-five against fifteen. A success."
    );
}

#[test]
fn heroic_inspiration_says_which_die_it_replaced_and_with_what() {
    let rerolled = RollRequest::D20(D20Request {
        test: D20Test::SkillCheck(Skill::Athletics),
        mode: RollMode::Normal,
        ability_modifier: 3,
        proficiency: PROFICIENT,
        target: difficulty(15),
        heroic_inspiration: Some(HeroicInspiration { replaces_die: 0 }),
    });
    assert_eq!(
        said(&rerolled, &[6, 14]),
        "Heroic Inspiration rerolled a six into a fourteen. Fourteen on the die, plus three for \
         Strength, plus two for proficiency. Nineteen against fifteen. A success."
    );
}

#[test]
fn a_zero_modifier_is_left_out_and_a_negative_one_is_minus() {
    let arcana = test(
        D20Test::SkillCheck(Skill::Arcana),
        RollMode::Normal,
        0,
        Proficiency::None,
        difficulty(15),
    );
    assert_eq!(said(&arcana, &[11]), "Eleven against fifteen. A failure.");
    let persuasion = test(
        D20Test::SkillCheck(Skill::Persuasion),
        RollMode::Normal,
        -1,
        Proficiency::None,
        difficulty(15),
    );
    assert_eq!(
        said(&persuasion, &[10]),
        "Ten on the die, minus one for Charisma. Nine against fifteen. A failure."
    );
}

#[test]
fn initiative_reads_the_total_as_the_initiative() {
    let initiative = test(
        D20Test::Initiative,
        RollMode::Advantage,
        1,
        PROFICIENT,
        None,
    );
    assert_eq!(
        said(&initiative, &[12, 3]),
        "Twelve and three with advantage, so twelve counts, plus one for Dexterity, plus two for \
         proficiency. Your initiative is fifteen."
    );
}

#[test]
fn a_death_saving_throw_marks_its_naturals_and_reads_against_ten() {
    let save = test(
        D20Test::DeathSave,
        RollMode::Normal,
        0,
        Proficiency::None,
        None,
    );
    assert_eq!(
        said(&save, &[20]),
        "Twenty against ten. A natural twenty, and you regain one hit point."
    );
    assert_eq!(
        said(&save, &[1]),
        "One against ten. A natural one, which counts as two failures."
    );
    assert_eq!(said(&save, &[12]), "Twelve against ten. A success.");
    assert_eq!(said(&save, &[6]), "Six against ten. A failure.");
}

#[test]
fn a_passive_score_reads_its_base_and_each_source() {
    let passive = RollRequest::PassivePerception(PassivePerceptionRequest {
        ability_modifier: 1,
        proficiency: PROFICIENT,
        mode: RollMode::Normal,
    });
    assert_eq!(
        said(&passive, &[]),
        "Ten to start, plus one for Wisdom, plus two for proficiency. \
         Your passive Perception is thirteen."
    );
    let advantaged = RollRequest::PassivePerception(PassivePerceptionRequest {
        ability_modifier: 1,
        proficiency: PROFICIENT,
        mode: RollMode::Advantage,
    });
    assert_eq!(
        said(&advantaged, &[]),
        "Ten to start, plus one for Wisdom, plus two for proficiency, plus five for advantage. \
         Your passive Perception is eighteen."
    );
}

#[test]
fn the_two_contests_read_against_the_escape_difficulty_and_the_hiding_score() {
    let escape = test(
        D20Test::SkillCheck(Skill::Athletics),
        RollMode::Normal,
        3,
        PROFICIENT,
        target(TargetLabel::EscapeDc, 15),
    );
    assert_eq!(
        said(&escape, &[14]),
        "Fourteen on the die, plus three for Strength, plus two for proficiency. \
         Nineteen against an escape difficulty of fifteen. A success."
    );
    let spot = test(
        D20Test::SkillCheck(Skill::Perception),
        RollMode::Normal,
        1,
        PROFICIENT,
        target(TargetLabel::StealthTotal, 17),
    );
    assert_eq!(
        said(&spot, &[10]),
        "Ten on the die, plus one for Wisdom, plus two for proficiency. \
         Thirteen against a hiding score of seventeen. A failure."
    );
}

fn damage(count: u8, die: DieSize, modifier: i16, critical: bool) -> RollRequest {
    RollRequest::Dice(DiceRequest::damage(
        DiceExpression {
            count,
            die,
            modifier,
        },
        Ability::Strength,
        critical,
    ))
}

#[test]
fn damage_dice_read_each_die_the_modifier_and_the_total() {
    assert_eq!(
        said(&damage(1, DieSize::D8, 3, false), &[6]),
        "Six on the eight-sided die, plus three for Strength. Nine damage."
    );
    assert_eq!(
        said(&damage(2, DieSize::D6, 3, false), &[4, 5]),
        "Four and five on the six-sided dice, plus three for Strength. Twelve damage."
    );
    assert_eq!(
        said(&damage(1, DieSize::D8, 3, true), &[6, 2]),
        "Six and two on the eight-sided dice, plus three for Strength. Eleven damage."
    );
    assert_eq!(
        said(&damage(3, DieSize::D4, 0, false), &[1, 2, 4]),
        "One, two and four on the four-sided dice. Seven damage."
    );
}

#[test]
fn healing_reads_the_die_the_flat_bonus_and_the_hit_points_restored() {
    let second_wind = RollRequest::Dice(DiceRequest {
        role: DieRole::Healing,
        expression: DiceExpression {
            count: 1,
            die: DieSize::D10,
            modifier: 3,
        },
        modifier_label: ModifierLabel::Flat,
        critical: false,
        heroic_inspiration: None,
    });
    assert_eq!(
        said(&second_wind, &[7]),
        "Seven on the ten-sided die, plus three. Ten hit points restored."
    );
    let one_point = RollRequest::Dice(DiceRequest {
        role: DieRole::Healing,
        expression: DiceExpression {
            count: 1,
            die: DieSize::D4,
            modifier: 0,
        },
        modifier_label: ModifierLabel::Flat,
        critical: false,
        heroic_inspiration: None,
    });
    assert_eq!(said(&one_point, &[1]), "One hit point restored.");
}

#[test]
fn a_healing_modifier_the_roll_labels_is_named_by_its_source() {
    let hit_die = RollRequest::Dice(DiceRequest {
        role: DieRole::Healing,
        expression: DiceExpression {
            count: 1,
            die: DieSize::D10,
            modifier: 2,
        },
        modifier_label: ModifierLabel::Constitution,
        critical: false,
        heroic_inspiration: None,
    });
    assert_eq!(
        said(&hit_die, &[7]),
        "Seven on the ten-sided die, plus two for Constitution. Nine hit points restored."
    );
}

fn percentile() -> RollRequest {
    RollRequest::Dice(DiceRequest {
        role: DieRole::Table,
        expression: DiceExpression {
            count: 1,
            die: DieSize::D100,
            modifier: 0,
        },
        modifier_label: ModifierLabel::Flat,
        critical: false,
        heroic_inspiration: None,
    })
}

#[test]
fn a_percentile_table_roll_reads_the_tens_die_the_ones_die_and_the_value() {
    assert_eq!(
        said(&percentile(), &[6, 2]),
        "The tens die shows sixty and the ones die shows two. Sixty-two on the table."
    );
    assert_eq!(
        said(&percentile(), &[10, 10]),
        "The tens die shows zero and the ones die shows zero, which reads as one hundred. \
         One hundred on the table."
    );
}

#[test]
fn a_call_names_the_test_the_difficulty_and_tells_the_player_to_roll() {
    assert_eq!(
        called(&athletics(RollMode::Normal)),
        "Athletics, Strength. Difficulty fifteen. Roll it."
    );
    assert_eq!(
        called(&athletics(RollMode::Advantage)),
        "Athletics, Strength. Difficulty fifteen. You have advantage, so roll two dice and keep \
         the higher."
    );
    assert_eq!(
        called(&athletics(RollMode::Disadvantage)),
        "Athletics, Strength. Difficulty fifteen. You have disadvantage, so roll two dice and \
         keep the lower."
    );
}

#[test]
fn a_call_words_each_other_kind_of_test() {
    let save = test(
        D20Test::SavingThrow(Ability::Constitution),
        RollMode::Normal,
        2,
        PROFICIENT,
        difficulty(15),
    );
    assert_eq!(
        called(&save),
        "Constitution saving throw. Difficulty fifteen. Roll it."
    );
    let raw = test(
        D20Test::AbilityCheck(Ability::Strength),
        RollMode::Normal,
        3,
        Proficiency::None,
        difficulty(20),
    );
    assert_eq!(called(&raw), "Strength check. Difficulty twenty. Roll it.");
    assert_eq!(
        called(&attack(19)),
        "Melee attack roll. Armor class sixteen. Roll it."
    );
    assert_eq!(
        called(&test(
            D20Test::Initiative,
            RollMode::Normal,
            1,
            PROFICIENT,
            None
        )),
        "Initiative. Roll it."
    );
    assert_eq!(
        called(&test(
            D20Test::DeathSave,
            RollMode::Normal,
            0,
            Proficiency::None,
            None
        )),
        "Death saving throw. Difficulty ten. Roll it."
    );
    let escape = test(
        D20Test::SkillCheck(Skill::Athletics),
        RollMode::Normal,
        3,
        PROFICIENT,
        target(TargetLabel::EscapeDc, 15),
    );
    assert_eq!(
        called(&escape),
        "Athletics, Strength, to break free. Difficulty fifteen. Roll it."
    );
    let spot = test(
        D20Test::SkillCheck(Skill::Perception),
        RollMode::Normal,
        1,
        PROFICIENT,
        target(TargetLabel::StealthTotal, 17),
    );
    assert_eq!(
        called(&spot),
        "Perception, Wisdom, to spot what hides. Difficulty seventeen. Roll it."
    );
    let passive = RollRequest::PassivePerception(PassivePerceptionRequest {
        ability_modifier: 1,
        proficiency: PROFICIENT,
        mode: RollMode::Normal,
    });
    assert_eq!(
        called(&passive),
        "Passive Perception. Nothing is rolled, your score is read."
    );
}

#[test]
fn a_call_for_dice_says_how_many_of_what_and_the_modifier() {
    assert_eq!(
        called(&damage(1, DieSize::D8, 3, false)),
        "Damage. Roll one eight-sided die and add three."
    );
    assert_eq!(
        called(&damage(1, DieSize::D8, 3, true)),
        "Damage. Roll two eight-sided dice and add three."
    );
    assert_eq!(
        called(&damage(2, DieSize::D6, -1, false)),
        "Damage. Roll two six-sided dice and take away one."
    );
    assert_eq!(
        called(&percentile()),
        "Table roll. Roll percentile dice, two ten-sided dice, tens first."
    );
}

#[test]
fn the_flourish_judges_name_the_test_the_way_the_call_does() {
    assert_eq!(
        test_name(&athletics(RollMode::Normal)),
        "Athletics, Strength"
    );
    assert_eq!(test_name(&percentile()), "Table roll");
}
