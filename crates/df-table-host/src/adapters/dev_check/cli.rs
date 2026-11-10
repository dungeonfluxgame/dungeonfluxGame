//! The grammar of `rules check <skill> <seat> [dc=N]` and `rules roll <kind> ..`, and their calls
//! to the host.

use std::str::FromStr;

use df_rules::roll::{
    Ability, AttackKind, D20Request, D20Test, DiceExpression, DiceRequest, DieRole,
    HeroicInspiration, ModifierLabel, PassivePerceptionRequest, Proficiency, RollMode, RollRequest,
    Target, TargetLabel,
};

use crate::adapters::action_book::DifficultyClass;
use crate::adapters::hero_profile::Skill;
use crate::command::CommandError;
use crate::dev::DevError;
use crate::dev_client;
use crate::engine::routes::path;

use super::format::{format_rules_check, format_rules_roll};
use super::request::{RulesCheckReply, RulesCheckRequest, RulesRollReply};

pub const RULES_CHECK_USAGE: &str = "rules check <skill> <seat> [dc=N]";
pub const RULES_ROLL_USAGE: &str = concat!(
    "rules roll ability|skill|save <name> | attack melee|ranged | initiative | death_save | ",
    "passive_perception | dice damage|healing|hit_die|bonus|table <NdS+M> ",
    "[mod=N] [prof=N|expertise=N] [dc=N|ac=N|escape_dc=N|stealth_total=N] ",
    "[mode=normal|advantage|disadvantage] [crit=N] [ability=NAME] [reroll=N] [label=KEY] ",
    "[critical=true]",
);

/// Parses the words after `rules`.
pub fn parse(words: &[String]) -> Result<RulesCheckRequest, CommandError> {
    let usage = || CommandError::Usage(RULES_CHECK_USAGE);
    let [verb, skill, actor, rest @ ..] = words else {
        return Err(usage());
    };
    if verb != "check" {
        return Err(usage());
    }
    let skill = Skill::from_key(skill).ok_or_else(|| CommandError::BadChoice {
        key: "the skill".to_owned(),
        text: skill.clone(),
        allowed: Skill::ALL
            .iter()
            .map(|skill| skill.key())
            .collect::<Vec<_>>()
            .join(", "),
    })?;
    let actor = actor.parse::<u64>().map_err(|_| CommandError::BadCount {
        what: "the seat",
        text: actor.clone(),
    })?;
    let dc = match rest {
        [] => None,
        [pair] => {
            let text = pair.strip_prefix("dc=").ok_or_else(usage)?;
            let invalid = || CommandError::BadCount {
                what: "the difficulty class",
                text: text.to_owned(),
            };
            let value = text.parse::<u8>().map_err(|_| invalid())?;
            Some(DifficultyClass::new(value).map_err(|_| invalid())?)
        }
        _ => return Err(usage()),
    };
    Ok(RulesCheckRequest { skill, actor, dc })
}

/// The `key=value` words of a roll, taken one at a time by the kind that reads them.
struct RollOptions(Vec<(String, String)>);

impl RollOptions {
    fn parse(words: &[String]) -> Result<Self, CommandError> {
        let mut pairs: Vec<(String, String)> = Vec::with_capacity(words.len());
        for word in words {
            let (key, value) = word
                .split_once('=')
                .filter(|(key, _)| !key.is_empty())
                .ok_or_else(|| CommandError::BadPair(word.clone()))?;
            if pairs.iter().any(|(taken, _)| taken == key) {
                return Err(CommandError::DuplicateKey(key.to_owned()));
            }
            pairs.push((key.to_owned(), value.to_owned()));
        }
        Ok(Self(pairs))
    }

    fn take(&mut self, key: &str) -> Option<String> {
        let place = self.0.iter().position(|(taken, _)| taken == key)?;
        Some(self.0.remove(place).1)
    }

    fn take_number<T: FromStr>(&mut self, key: &'static str) -> Result<Option<T>, CommandError> {
        self.take(key)
            .map(|text| {
                text.parse::<T>()
                    .map_err(|_| CommandError::BadCount { what: key, text })
            })
            .transpose()
    }

    fn take_choice<T>(
        &mut self,
        key: &'static str,
        from_key: impl Fn(&str) -> Option<T>,
        allowed: &[&str],
    ) -> Result<Option<T>, CommandError> {
        self.take(key)
            .map(|text| {
                from_key(&text).ok_or_else(|| CommandError::BadChoice {
                    key: key.to_owned(),
                    text,
                    allowed: allowed.join(", "),
                })
            })
            .transpose()
    }

    fn take_mode(&mut self) -> Result<RollMode, CommandError> {
        let mode = self.take_choice(
            "mode",
            |text| match text {
                "normal" => Some(RollMode::Normal),
                "advantage" => Some(RollMode::Advantage),
                "disadvantage" => Some(RollMode::Disadvantage),
                _ => None,
            },
            &["normal", "advantage", "disadvantage"],
        )?;
        Ok(mode.unwrap_or(RollMode::Normal))
    }

    fn take_target(&mut self) -> Result<Option<Target>, CommandError> {
        let mut targets = Vec::new();
        for (key, label) in [
            ("dc", TargetLabel::DifficultyClass),
            ("ac", TargetLabel::ArmorClass),
            ("escape_dc", TargetLabel::EscapeDc),
            ("stealth_total", TargetLabel::StealthTotal),
        ] {
            if let Some(value) = self.take_number::<i16>(key)? {
                targets.push(Target { label, value });
            }
        }
        match targets.as_slice() {
            [] => Ok(None),
            [target] => Ok(Some(*target)),
            _ => Err(CommandError::Usage(RULES_ROLL_USAGE)),
        }
    }

    fn take_proficiency(&mut self) -> Result<Proficiency, CommandError> {
        let proficient = self.take_number::<u8>("prof")?;
        let expert = self.take_number::<u8>("expertise")?;
        match (proficient, expert) {
            (None, None) => Ok(Proficiency::None),
            (Some(bonus), None) => Ok(Proficiency::Proficient { bonus }),
            (None, Some(bonus)) => Ok(Proficiency::Expertise { bonus }),
            (Some(_), Some(_)) => Err(CommandError::Usage(RULES_ROLL_USAGE)),
        }
    }

    fn take_reroll(&mut self) -> Result<Option<HeroicInspiration>, CommandError> {
        Ok(self
            .take_number::<u8>("reroll")?
            .map(|replaces_die| HeroicInspiration { replaces_die }))
    }

    fn take_ability(&mut self) -> Result<Option<Ability>, CommandError> {
        let allowed: Vec<&str> = Ability::ALL.iter().map(|ability| ability.key()).collect();
        self.take_choice("ability", Ability::from_key, &allowed)
    }

    /// Refuses an option the kind does not read, so no word is silently ignored.
    fn finish(self) -> Result<(), CommandError> {
        match self.0.into_iter().next() {
            None => Ok(()),
            Some((key, _)) => Err(CommandError::BadChoice {
                key: "an option of this roll".to_owned(),
                text: key,
                allowed: RULES_ROLL_USAGE.to_owned(),
            }),
        }
    }
}

fn subject<T>(
    what: &'static str,
    word: Option<&String>,
    from_key: impl Fn(&str) -> Option<T>,
    allowed: &[&str],
) -> Result<T, CommandError> {
    let word = word.ok_or(CommandError::Usage(RULES_ROLL_USAGE))?;
    from_key(word).ok_or_else(|| CommandError::BadChoice {
        key: what.to_owned(),
        text: word.clone(),
        allowed: allowed.join(", "),
    })
}

fn d20_request(test: D20Test, mut options: RollOptions) -> Result<RollRequest, CommandError> {
    let request = D20Request {
        test,
        mode: options.take_mode()?,
        ability_modifier: options.take_number::<i8>("mod")?.unwrap_or(0),
        proficiency: options.take_proficiency()?,
        target: options.take_target()?,
        heroic_inspiration: options.take_reroll()?,
    };
    options.finish()?;
    Ok(RollRequest::D20(request))
}

fn attack_request(
    attack: Option<&String>,
    mut options: RollOptions,
) -> Result<RollRequest, CommandError> {
    let kind = subject(
        "the attack",
        attack,
        AttackKind::from_key,
        &["melee", "ranged"],
    )?;
    let ability = options.take_ability()?.unwrap_or(match kind {
        AttackKind::Melee => Ability::Strength,
        AttackKind::Ranged => Ability::Dexterity,
    });
    let critical_from = options.take_number::<u8>("crit")?.unwrap_or(20);
    d20_request(
        D20Test::Attack {
            ability,
            kind,
            critical_from,
        },
        options,
    )
}

fn dice_request(words: &[String], mut options: RollOptions) -> Result<RollRequest, CommandError> {
    let roles: Vec<&str> = DieRole::ALL.iter().map(|role| role.key()).collect();
    let role = subject("the dice role", words.first(), DieRole::from_key, &roles)?;
    let text = words.get(1).ok_or(CommandError::Usage(RULES_ROLL_USAGE))?;
    let expression = text
        .parse::<DiceExpression>()
        .map_err(|error| CommandError::BadChoice {
            key: "the dice expression".to_owned(),
            text: text.clone(),
            allowed: error.to_string(),
        })?;
    let labels: Vec<&str> = ModifierLabel::ALL.iter().map(|label| label.key()).collect();
    let request = DiceRequest {
        role,
        expression,
        modifier_label: options
            .take_choice("label", ModifierLabel::from_key, &labels)?
            .unwrap_or(ModifierLabel::Flat),
        critical: options
            .take_choice(
                "critical",
                |text| match text {
                    "true" => Some(true),
                    "false" => Some(false),
                    _ => None,
                },
                &["true", "false"],
            )?
            .unwrap_or(false),
        heroic_inspiration: options.take_reroll()?,
    };
    options.finish()?;
    Ok(RollRequest::Dice(request))
}

fn passive_request(options: &[String]) -> Result<RollRequest, CommandError> {
    let mut options = RollOptions::parse(options)?;
    let request = PassivePerceptionRequest {
        ability_modifier: options.take_number::<i8>("mod")?.unwrap_or(0),
        proficiency: options.take_proficiency()?,
        mode: options.take_mode()?,
    };
    options.finish()?;
    Ok(RollRequest::PassivePerception(request))
}

/// Parses the words after `rules`, which begin with `roll`, into a typed roll.
pub fn parse_roll(words: &[String]) -> Result<RollRequest, CommandError> {
    let usage = || CommandError::Usage(RULES_ROLL_USAGE);
    let [verb, kind, rest @ ..] = words else {
        return Err(usage());
    };
    if verb != "roll" {
        return Err(usage());
    }
    let abilities: Vec<&str> = Ability::ALL.iter().map(|ability| ability.key()).collect();
    let skills: Vec<&str> = Skill::ALL.iter().map(|skill| skill.key()).collect();
    let (name, tail) = rest
        .split_first()
        .map_or((None, rest), |(name, tail)| (Some(name), tail));
    match kind.as_str() {
        "ability" => {
            let ability = subject("the ability", name, Ability::from_key, &abilities)?;
            d20_request(D20Test::AbilityCheck(ability), RollOptions::parse(tail)?)
        }
        "save" => {
            let ability = subject("the ability", name, Ability::from_key, &abilities)?;
            d20_request(D20Test::SavingThrow(ability), RollOptions::parse(tail)?)
        }
        "skill" => {
            let skill = subject("the skill", name, Skill::from_key, &skills)?;
            d20_request(D20Test::SkillCheck(skill), RollOptions::parse(tail)?)
        }
        "attack" => attack_request(name, RollOptions::parse(tail)?),
        "initiative" => d20_request(D20Test::Initiative, RollOptions::parse(rest)?),
        "death_save" => d20_request(D20Test::DeathSave, RollOptions::parse(rest)?),
        "passive_perception" => passive_request(rest),
        "dice" => {
            let (named, options) = rest.split_at(rest.len().min(2));
            dice_request(named, RollOptions::parse(options)?)
        }
        other => Err(CommandError::BadChoice {
            key: "the kind of roll".to_owned(),
            text: other.to_owned(),
            allowed:
                "ability, skill, save, attack, initiative, death_save, passive_perception, dice"
                    .to_owned(),
        }),
    }
}

/// Sends `request` to the host on `port` and reads its typed reply, a refusal included.
pub async fn ask(port: u16, request: &RulesCheckRequest) -> Result<RulesCheckReply, DevError> {
    dev_client::post_json(port, path::RULES_CHECK, request).await
}

/// Runs one check against the host on `port` and returns the text to print.
pub async fn run(port: u16, request: &RulesCheckRequest) -> Result<String, DevError> {
    dev_client::reply_value(ask(port, request).await?).map(|value| format_rules_check(&value))
}

/// Sends a roll to the host on `port` and reads its typed reply, a refusal included.
pub async fn ask_roll(port: u16, request: &RollRequest) -> Result<RulesRollReply, DevError> {
    dev_client::post_json(port, path::RULES_ROLL, request).await
}

/// Runs one roll against the host on `port` and returns the text to print.
pub async fn run_roll(port: u16, request: &RollRequest) -> Result<String, DevError> {
    dev_client::reply_value(ask_roll(port, request).await?).map(|value| format_rules_roll(&value))
}
