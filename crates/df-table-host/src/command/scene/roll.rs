//! The `scene roll` verb: puts one roll on the shared screen as the roll card, from values given
//! on the command line, so the display can be judged without the rules engine.
//!
//! Every face, modifier, total, target and verdict is read as given and the page shows them as
//! they are; only the total may be left out, and then it is the kept dice and the modifiers added
//! up. Whether the roll is drawable (a plain id, dice with the faces they have, a total that is
//! the sum) is decided by the host when it arrives, as for every live change.

use df_playground::fixture::{
    DieRole, InspirationReroll, NaturalMark, RollDie, RollKind, RollMode, RollModifier,
    RollOutcome, RollTarget, RollView,
};
use df_playground::scene::SceneChange;

use crate::command::usage::{ROLL_CLEAR_USAGE, ROLL_USAGE};
use crate::command::{Command, CommandError, bad_choice, keyed, split_list};

const ROLL_KEYS: [&str; 10] = [
    "id", "mode", "dice", "mods", "target", "outcome", "natural", "reroll", "total", "hold",
];
const ROLL_KEYS_TEXT: &str = concat!(
    "id=NAME, mode=normal|advantage|disadvantage, dice=SIDES:FACE[:ROLE][:dropped],..., ",
    "mods=LABEL:N,..., target=LABEL:N, outcome=KEY, natural=one|twenty|none, ",
    "reroll=INDEX:WAS, total=N, hold=MS",
);
/// The id of a roll given none.
const DEFAULT_ID: &str = "dev-roll";
/// The word after a die's role that leaves it out of the sum.
const DROPPED: &str = "dropped";

/// The role a die has when its entry names none: the one its kind rolls for.
fn default_role(kind: RollKind) -> DieRole {
    match kind {
        RollKind::Damage => DieRole::Damage,
        RollKind::Healing => DieRole::Healing,
        RollKind::HitDie => DieRole::HitDie,
        RollKind::BonusDie => DieRole::Bonus,
        RollKind::TableRoll => DieRole::Table,
        _ => DieRole::Test,
    }
}

fn whole<T: std::str::FromStr>(key: &str, text: &str, allowed: &str) -> Result<T, CommandError> {
    text.parse::<T>()
        .map_err(|_| bad_choice(key, text, allowed))
}

fn die(kind: RollKind, entry: &str) -> Result<RollDie, CommandError> {
    const ALLOWED: &str = "SIDES:FACE with optional :ROLE (test, bonus, damage, healing, hit_die, \
                           table) and :dropped";
    let mut parts = entry.split(':');
    let (Some(sides), Some(face)) = (parts.next(), parts.next()) else {
        return Err(bad_choice("dice", entry, ALLOWED));
    };
    let mut die = RollDie {
        sides: whole("dice", sides, ALLOWED)?,
        face: whole("dice", face, ALLOWED)?,
        role: default_role(kind),
        kept: true,
    };
    for word in parts {
        match (word, DieRole::from_key(word)) {
            (DROPPED, _) => die.kept = false,
            (_, Some(role)) => die.role = role,
            _ => return Err(bad_choice("dice", entry, ALLOWED)),
        }
    }
    Ok(die)
}

/// `LABEL:N`, where the label may hold colons.
fn labelled_number(key: &str, text: &str) -> Result<(String, i32), CommandError> {
    const ALLOWED: &str = "LABEL:N, a label and a whole number";
    let (label, value) = text
        .rsplit_once(':')
        .filter(|(label, _)| !label.is_empty())
        .ok_or_else(|| bad_choice(key, text, ALLOWED))?;
    Ok((label.to_owned(), whole(key, value, ALLOWED)?))
}

fn reroll(text: &str) -> Result<InspirationReroll, CommandError> {
    const ALLOWED: &str = "INDEX:WAS, the die's place in dice (from 0) and the face it had";
    let (index, was) = text
        .split_once(':')
        .ok_or_else(|| bad_choice("reroll", text, ALLOWED))?;
    Ok(InspirationReroll {
        index: whole("reroll", index, ALLOWED)?,
        was: whole("reroll", was, ALLOWED)?,
    })
}

/// The kept dice and the modifiers added up.
fn sum(view: &RollView) -> i32 {
    let dice: i32 = view
        .dice
        .iter()
        .filter(|die| die.kept)
        .map(|die| i32::from(die.face))
        .sum();
    dice + view
        .modifiers
        .iter()
        .map(|modifier| modifier.value)
        .sum::<i32>()
}

fn roll(words: &[String]) -> Result<Command, CommandError> {
    let [kind, title, pairs @ ..] = words else {
        return Err(CommandError::Usage(ROLL_USAGE));
    };
    let kind = RollKind::from_key(kind).ok_or_else(|| {
        let keys: Vec<&str> = RollKind::ALL.iter().map(|kind| kind.key()).collect();
        bad_choice("kind", kind, keys.join(", "))
    })?;
    let mut view = RollView {
        id: DEFAULT_ID.to_owned(),
        title: title.clone(),
        kind,
        mode: RollMode::Normal,
        dice: Vec::new(),
        modifiers: Vec::new(),
        total: 0,
        target: None,
        outcome: None,
        natural: NaturalMark::Unmarked,
        inspiration_reroll: None,
    };
    let (mut total, mut min_hold_ms) = (None, 0_u64);
    for (key, value) in keyed(pairs, ROLL_KEYS_TEXT, &ROLL_KEYS)? {
        match key {
            "id" => value.clone_into(&mut view.id),
            "mode" => {
                view.mode = RollMode::from_key(value)
                    .ok_or_else(|| bad_choice(key, value, "normal, advantage or disadvantage"))?;
            }
            "dice" => {
                view.dice = split_list(value)
                    .iter()
                    .map(|entry| die(kind, entry))
                    .collect::<Result<_, _>>()?;
            }
            "mods" => {
                for entry in split_list(value) {
                    let (label, value) = labelled_number(key, &entry)?;
                    view.modifiers.push(RollModifier { label, value });
                }
            }
            "target" => {
                let (label, value) = labelled_number(key, value)?;
                view.target = Some(RollTarget { label, value });
            }
            "outcome" => {
                view.outcome = Some(RollOutcome::from_key(value).ok_or_else(|| {
                    let keys: Vec<&str> = RollOutcome::ALL.iter().map(|each| each.key()).collect();
                    bad_choice(key, value, keys.join(", "))
                })?);
            }
            "natural" => {
                view.natural = NaturalMark::from_key(value)
                    .ok_or_else(|| bad_choice(key, value, "one, twenty or none"))?;
            }
            "reroll" => view.inspiration_reroll = Some(reroll(value)?),
            "total" => total = Some(whole(key, value, "a whole number")?),
            _ => min_hold_ms = whole(key, value, "a whole number of milliseconds")?,
        }
    }
    view.total = total.unwrap_or_else(|| sum(&view));
    Ok(Command::Scene(SceneChange::Roll { view, min_hold_ms }))
}

/// Reads the words of `scene roll` or `scene roll-clear` into one live change.
pub(super) fn parse(verb: &str, rest: &[String]) -> Result<Command, CommandError> {
    if verb == "roll-clear" {
        return match rest {
            [] => Ok(Command::Scene(SceneChange::ClearRoll {})),
            _ => Err(CommandError::Usage(ROLL_CLEAR_USAGE)),
        };
    }
    roll(rest)
}

#[cfg(test)]
mod roll_tests;
