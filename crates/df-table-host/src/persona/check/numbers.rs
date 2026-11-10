//! The numbers and the verdict of a check, spoken: composed in code from the roll request and its
//! resolved result, so what the dungeon master says always matches what the dice showed.
//!
//! The model only supplies the flourish around these slots (`lines`); it never writes a number.
//! Numerals are words (`forty-two`), so a voice reads them and a line never mixes digits and
//! words. A call names the test and its difficulty and tells the player to roll; a result names
//! each die, each modifier by its source, the total against the target and the verdict
//! (SRD 5.2.1 pages 6 to 9 for D20 Tests, 16 to 17 and 26 to 27 for damage, healing and tables).
//! It reads no wall clock and decides nothing: every fact is the result's.

use df_rules::roll::{
    Ability, AttackKind, D20Test, DieRole, ModifierLabel, RollMode, RollOutcome, RollRequest,
    RollResult, RollSubject, Skill, Target, TargetLabel,
};

/// The difficulty of a Death Saving Throw (SRD 5.2.1 page 17): `df-rules` keeps it private, so the
/// call line states it here.
const DEATH_SAVE_DIFFICULTY: i64 = 10;

const ONES: [&str; 20] = [
    "zero",
    "one",
    "two",
    "three",
    "four",
    "five",
    "six",
    "seven",
    "eight",
    "nine",
    "ten",
    "eleven",
    "twelve",
    "thirteen",
    "fourteen",
    "fifteen",
    "sixteen",
    "seventeen",
    "eighteen",
    "nineteen",
];

const TENS: [&str; 10] = [
    "", "", "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety",
];

/// The words of `index` in `table`; the callers keep it in range.
fn word(table: &[&'static str], index: u64) -> &'static str {
    usize::try_from(index)
        .ok()
        .and_then(|index| table.get(index))
        .copied()
        .unwrap_or("")
}

fn below_hundred(value: u64) -> String {
    if value < 20 {
        return word(&ONES, value).to_owned();
    }
    let tens = word(&TENS, value / 10);
    match value % 10 {
        0 => tens.to_owned(),
        ones => format!("{tens}-{}", word(&ONES, ones)),
    }
}

fn below_thousand(value: u64) -> String {
    let (hundreds, rest) = (value / 100, value % 100);
    let mut parts = Vec::new();
    if hundreds > 0 {
        parts.push(format!("{} hundred", word(&ONES, hundreds)));
    }
    if rest > 0 {
        parts.push(below_hundred(rest));
    }
    parts.join(" ")
}

/// `value` in English words: `forty-two`, `one hundred five`, `minus three`.
pub(super) fn number_words(value: i64) -> String {
    let magnitude = value.unsigned_abs();
    let mut parts = Vec::new();
    for (scale, name) in [
        (1_000_000_000_u64, "billion"),
        (1_000_000, "million"),
        (1_000, "thousand"),
    ] {
        let count = (magnitude / scale) % 1_000;
        if count > 0 {
            parts.push(format!("{} {name}", below_thousand(count)));
        }
    }
    match magnitude % 1_000 {
        0 if parts.is_empty() => parts.push(word(&ONES, 0).to_owned()),
        0 => {}
        rest => parts.push(below_thousand(rest)),
    }
    let words = parts.join(" ");
    if value < 0 {
        format!("minus {words}")
    } else {
        words
    }
}

/// `text` with its first letter in capitals.
pub(super) fn capitalized(text: &str) -> String {
    let mut letters = text.chars();
    letters
        .next()
        .map(|first| first.to_uppercase().chain(letters).collect())
        .unwrap_or_default()
}

pub(super) fn ability_word(ability: Ability) -> &'static str {
    match ability {
        Ability::Strength => "Strength",
        Ability::Dexterity => "Dexterity",
        Ability::Constitution => "Constitution",
        Ability::Intelligence => "Intelligence",
        Ability::Wisdom => "Wisdom",
        Ability::Charisma => "Charisma",
    }
}

pub(super) fn skill_word(skill: Skill) -> &'static str {
    match skill {
        Skill::Acrobatics => "Acrobatics",
        Skill::AnimalHandling => "Animal Handling",
        Skill::Arcana => "Arcana",
        Skill::Athletics => "Athletics",
        Skill::Deception => "Deception",
        Skill::History => "History",
        Skill::Insight => "Insight",
        Skill::Intimidation => "Intimidation",
        Skill::Investigation => "Investigation",
        Skill::Medicine => "Medicine",
        Skill::Nature => "Nature",
        Skill::Perception => "Perception",
        Skill::Performance => "Performance",
        Skill::Persuasion => "Persuasion",
        Skill::Religion => "Religion",
        Skill::SleightOfHand => "Sleight of Hand",
        Skill::Stealth => "Stealth",
        Skill::Survival => "Survival",
    }
}

/// The name of a modifier's source as the card writes it.
pub(super) fn modifier_word(label: ModifierLabel) -> &'static str {
    match label {
        ModifierLabel::Base => "Base",
        ModifierLabel::Strength => "Strength",
        ModifierLabel::Dexterity => "Dexterity",
        ModifierLabel::Constitution => "Constitution",
        ModifierLabel::Intelligence => "Intelligence",
        ModifierLabel::Wisdom => "Wisdom",
        ModifierLabel::Charisma => "Charisma",
        ModifierLabel::Proficiency => "Proficiency",
        ModifierLabel::Expertise => "Expertise",
        ModifierLabel::Advantage => "Advantage",
        ModifierLabel::Disadvantage => "Disadvantage",
        ModifierLabel::ClassLevel => "Class level",
        ModifierLabel::Flat => "Modifier",
    }
}

/// The name of a modifier's source as it is spoken: abilities keep their capitals, the rest are
/// plain words, and a flat number has no source to name.
fn spoken_source(label: ModifierLabel) -> Option<String> {
    match label {
        ModifierLabel::Flat => None,
        ModifierLabel::Base => Some("the base".to_owned()),
        ModifierLabel::Strength
        | ModifierLabel::Dexterity
        | ModifierLabel::Constitution
        | ModifierLabel::Intelligence
        | ModifierLabel::Wisdom
        | ModifierLabel::Charisma => Some(modifier_word(label).to_owned()),
        ModifierLabel::Proficiency
        | ModifierLabel::Expertise
        | ModifierLabel::Advantage
        | ModifierLabel::Disadvantage
        | ModifierLabel::ClassLevel => Some(modifier_word(label).to_lowercase()),
    }
}

/// The name of a target's kind on the card.
pub(super) fn target_word(target: Target) -> &'static str {
    match target.label {
        TargetLabel::DifficultyClass => "DC",
        TargetLabel::ArmorClass => "AC",
        TargetLabel::EscapeDc => "Escape DC",
        TargetLabel::StealthTotal => "Stealth",
    }
}

fn mode_word(mode: RollMode) -> &'static str {
    match mode {
        RollMode::Normal => "normal",
        RollMode::Advantage => "advantage",
        RollMode::Disadvantage => "disadvantage",
    }
}

fn sided(sides: u16) -> String {
    let name = match sides {
        4 => "four",
        6 => "six",
        8 => "eight",
        10 => "ten",
        12 => "twelve",
        20 => "twenty",
        other => return format!("{}-sided", number_words(i64::from(other))),
    };
    format!("{name}-sided")
}

/// `a`, `a and b` or `a, b and c`.
fn listed(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [only] => only.clone(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

/// `, plus three for Strength` for each modifier that is not zero; a negative one is `minus`.
fn modifiers_phrase(result: &RollResult) -> String {
    result
        .modifiers
        .iter()
        .filter(|modifier| modifier.value != 0)
        .map(|modifier| {
            let direction = if modifier.value < 0 { "minus" } else { "plus" };
            let amount = number_words(i64::from(modifier.value.unsigned_abs()));
            match spoken_source(modifier.label) {
                Some(source) => format!(", {direction} {amount} for {source}"),
                None => format!(", {direction} {amount}"),
            }
        })
        .collect()
}

/// The place among the dice of the one Heroic Inspiration replaced, if it was spent.
fn replaced_place(request: &RollRequest) -> Option<usize> {
    let inspiration = match request {
        RollRequest::D20(request) => request.heroic_inspiration,
        RollRequest::Dice(request) => request.heroic_inspiration,
        RollRequest::PassivePerception(_) => None,
    };
    inspiration.map(|spent| usize::from(spent.replaces_die))
}

fn reroll_sentence(request: &RollRequest, result: &RollResult) -> Option<String> {
    let was = result.inspiration_reroll?;
    let now = result.dice.get(replaced_place(request)?)?;
    Some(format!(
        "Heroic Inspiration rerolled a {} into a {}.",
        number_words(i64::from(was.face)),
        number_words(i64::from(now.face))
    ))
}

fn target_phrase(target: Target) -> String {
    let value = number_words(i64::from(target.value));
    match target.label {
        TargetLabel::DifficultyClass => value,
        TargetLabel::ArmorClass => format!("armor class {value}"),
        TargetLabel::EscapeDc => format!("an escape difficulty of {value}"),
        TargetLabel::StealthTotal => format!("a hiding score of {value}"),
    }
}

fn outcome_sentence(result: &RollResult) -> Option<&'static str> {
    use df_rules::roll::Natural;
    Some(match (result.outcome, result.natural) {
        (RollOutcome::Success | RollOutcome::DeathSaveSuccess, _) => "A success.",
        (RollOutcome::Failure | RollOutcome::DeathSaveFailure, _) => "A failure.",
        (RollOutcome::Hit, _) => "A hit.",
        (RollOutcome::Miss, Natural::One) => "A natural one, which always misses.",
        (RollOutcome::Miss, _) => "A miss.",
        (RollOutcome::CriticalHit, Natural::Twenty) => "A natural twenty, a critical hit.",
        (RollOutcome::CriticalHit, _) => "A critical hit.",
        (RollOutcome::DeathSaveTwoFailures, _) => "A natural one, which counts as two failures.",
        (RollOutcome::DeathSaveRevived, _) => "A natural twenty, and you regain one hit point.",
        (RollOutcome::Rolled | RollOutcome::Score, _) => return None,
    })
}

fn d20_result(request: &RollRequest, result: &RollResult) -> String {
    let mut sentences: Vec<String> = reroll_sentence(request, result).into_iter().collect();
    let faces: Vec<String> = result
        .dice
        .iter()
        .map(|die| number_words(i64::from(die.face)))
        .collect();
    let kept = result
        .dice
        .iter()
        .find(|die| die.kept)
        .map(|die| number_words(i64::from(die.face)))
        .unwrap_or_default();
    let on_the_dice = match result.mode {
        RollMode::Normal => format!("{kept} on the die"),
        mode => format!(
            "{} with {}, so {kept} counts",
            listed(&faces),
            mode_word(mode)
        ),
    };
    let modifiers = modifiers_phrase(result);
    // A single die that is the whole total would be said twice, as the die and as the total.
    if result.mode != RollMode::Normal || !modifiers.is_empty() {
        sentences.push(format!("{}{modifiers}.", capitalized(&on_the_dice)));
    }
    let total = number_words(i64::from(result.total));
    sentences.push(match result.target {
        Some(target) => format!("{} against {}.", capitalized(&total), target_phrase(target)),
        None => format!("Your initiative is {total}."),
    });
    sentences.extend(outcome_sentence(result).map(str::to_owned));
    sentences.join(" ")
}

fn passive_result(result: &RollResult) -> String {
    let base = result
        .modifiers
        .first()
        .map_or(0, |base| i64::from(base.value));
    let rest: String = result
        .modifiers
        .iter()
        .skip(1)
        .filter(|modifier| modifier.value != 0)
        .map(|modifier| {
            let direction = if modifier.value < 0 { "minus" } else { "plus" };
            let amount = number_words(i64::from(modifier.value.unsigned_abs()));
            let source = spoken_source(modifier.label).unwrap_or_default();
            format!(", {direction} {amount} for {source}")
        })
        .collect();
    format!(
        "{} to start{rest}. Your passive Perception is {}.",
        capitalized(&number_words(base)),
        number_words(i64::from(result.total))
    )
}

/// The dice of a damage, healing, hit die, bonus die or table roll as one sentence.
fn dice_result(request: &RollRequest, result: &RollResult, role: DieRole) -> String {
    let mut sentences: Vec<String> = reroll_sentence(request, result).into_iter().collect();
    let total = i64::from(result.total);
    let percentile = result.dice.iter().any(|die| {
        matches!(
            die.place,
            df_rules::roll::DiePlace::Tens | df_rules::roll::DiePlace::Ones
        )
    });
    if percentile {
        let counted: Vec<i64> = result
            .dice
            .iter()
            .map(|die| i64::from(die.counted()))
            .collect();
        match counted.as_slice() {
            [tens, ones] if total == 100 => sentences.push(format!(
                "The tens die shows {} and the ones die shows {}, which reads as one hundred.",
                number_words(*tens),
                number_words(*ones)
            )),
            [tens, ones] => sentences.push(format!(
                "The tens die shows {} and the ones die shows {}.",
                number_words(*tens),
                number_words(*ones)
            )),
            _ => {}
        }
    } else {
        let faces: Vec<String> = result
            .dice
            .iter()
            .map(|die| number_words(i64::from(die.face)))
            .collect();
        let sides = result.dice.first().map_or(0, |die| die.sides);
        let noun = if result.dice.len() == 1 {
            "die"
        } else {
            "dice"
        };
        let modifiers = modifiers_phrase(result);
        // A single die with no modifier is the whole total, which is said next.
        if result.dice.len() > 1 || !modifiers.is_empty() {
            sentences.push(format!(
                "{} on the {} {noun}{modifiers}.",
                capitalized(&listed(&faces)),
                sided(sides),
            ));
        }
    }
    let amount = capitalized(&number_words(total));
    sentences.push(match role {
        DieRole::Damage => format!("{amount} damage."),
        DieRole::Healing if total == 1 => format!("{amount} hit point restored."),
        DieRole::Healing => format!("{amount} hit points restored."),
        DieRole::HitDie if total == 1 => format!("{amount} hit point regained."),
        DieRole::HitDie => format!("{amount} hit points regained."),
        DieRole::Table => format!("{amount} on the table."),
        DieRole::Bonus | DieRole::Test => format!("{amount} on the bonus die."),
    });
    sentences.join(" ")
}

/// What the dungeon master says of a finished roll: each die, each modifier by its source, the
/// total against the target and the verdict.
pub(super) fn spoken_result(request: &RollRequest, result: &RollResult) -> String {
    match result.subject {
        RollSubject::PassivePerception => passive_result(result),
        RollSubject::Dice(role) => dice_result(request, result, role),
        RollSubject::AbilityCheck(_)
        | RollSubject::SkillCheck(_)
        | RollSubject::SavingThrow(_)
        | RollSubject::Attack(_)
        | RollSubject::Initiative
        | RollSubject::DeathSave => d20_result(request, result),
    }
}

/// The test of a D20 request in words: `Athletics, Strength`, `Strength saving throw`.
pub(super) fn test_words(test: D20Test, target: Option<Target>) -> String {
    let named = match test {
        D20Test::SkillCheck(skill) => {
            format!("{}, {}", skill_word(skill), ability_word(skill.ability()))
        }
        D20Test::AbilityCheck(ability) => format!("{} check", ability_word(ability)),
        D20Test::SavingThrow(ability) => format!("{} saving throw", ability_word(ability)),
        D20Test::Attack { kind, .. } => match kind {
            AttackKind::Melee => "Melee attack roll".to_owned(),
            AttackKind::Ranged => "Ranged attack roll".to_owned(),
        },
        D20Test::Initiative => "Initiative".to_owned(),
        D20Test::DeathSave => "Death saving throw".to_owned(),
    };
    match target.map(|target| target.label) {
        Some(TargetLabel::EscapeDc) => format!("{named}, to break free"),
        Some(TargetLabel::StealthTotal) => format!("{named}, to spot what hides"),
        _ => named,
    }
}

fn mode_instruction(mode: RollMode) -> &'static str {
    match mode {
        RollMode::Normal => "Roll it.",
        RollMode::Advantage => "You have advantage, so roll two dice and keep the higher.",
        RollMode::Disadvantage => "You have disadvantage, so roll two dice and keep the lower.",
    }
}

fn dice_call(request: &df_rules::roll::DiceRequest) -> String {
    let expression = request.expression;
    let count = u16::from(expression.count) * if request.critical { 2 } else { 1 };
    let name = match request.role {
        DieRole::Damage => "Damage",
        DieRole::Healing => "Healing",
        DieRole::HitDie => "Hit die",
        DieRole::Table => "Table roll",
        DieRole::Bonus | DieRole::Test => "Bonus die",
    };
    let dice = if expression.die == df_rules::roll::DieSize::D100 {
        "percentile dice, two ten-sided dice, tens first".to_owned()
    } else {
        let noun = if count == 1 { "die" } else { "dice" };
        format!(
            "{} {} {noun}",
            number_words(i64::from(count)),
            sided(expression.die.sides())
        )
    };
    let modifier = match expression.modifier {
        0 => String::new(),
        modifier if modifier < 0 => format!(
            " and take away {}",
            number_words(i64::from(modifier.unsigned_abs()))
        ),
        modifier => format!(" and add {}", number_words(i64::from(modifier))),
    };
    format!("{name}. Roll {dice}{modifier}.")
}

/// What the dungeon master says to call a roll: the test, its difficulty and the order to roll.
pub(super) fn spoken_call(request: &RollRequest) -> String {
    match request {
        RollRequest::D20(request) => {
            let mut sentences = vec![format!("{}.", test_words(request.test, request.target))];
            let difficulty = match (request.test, request.target) {
                (D20Test::DeathSave, _) => Some(format!(
                    "Difficulty {}.",
                    number_words(DEATH_SAVE_DIFFICULTY)
                )),
                (_, Some(target)) => Some(match target.label {
                    TargetLabel::ArmorClass => {
                        format!("Armor class {}.", number_words(i64::from(target.value)))
                    }
                    _ => format!("Difficulty {}.", number_words(i64::from(target.value))),
                }),
                (_, None) => None,
            };
            sentences.extend(difficulty);
            sentences.push(mode_instruction(request.mode).to_owned());
            sentences.join(" ")
        }
        RollRequest::PassivePerception(request) => {
            let mode = match request.mode {
                RollMode::Normal => "",
                RollMode::Advantage => " It counts as with advantage.",
                RollMode::Disadvantage => " It counts as with disadvantage.",
            };
            format!("Passive Perception. Nothing is rolled, your score is read.{mode}")
        }
        RollRequest::Dice(request) => dice_call(request),
    }
}

/// The test of `request` in words, as the call and the flourish judges name it.
pub(super) fn test_name(request: &RollRequest) -> String {
    match request {
        RollRequest::D20(request) => test_words(request.test, request.target),
        RollRequest::PassivePerception(_) => "Passive Perception".to_owned(),
        RollRequest::Dice(request) => match request.role {
            DieRole::Damage => "Damage",
            DieRole::Healing => "Healing",
            DieRole::HitDie => "Hit die",
            DieRole::Table => "Table roll",
            DieRole::Bonus | DieRole::Test => "Bonus die",
        }
        .to_owned(),
    }
}
