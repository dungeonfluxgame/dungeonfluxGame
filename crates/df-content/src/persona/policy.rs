//! A persona's social policy: the secrets it keeps, the stances (behaviors) it can take and the
//! triggers that move it between them.
//!
//! This is bounded policy data for the interaction engine and for a model's prompt
//! (`planning/interaction-engine.md`): it grants no mechanical authority, and a model only words
//! what the engine accepts. A trigger is never a keyword match on a player's words. Its condition
//! is a plain statement a model judges under a schema, or it names a committed event the engine
//! witnessed. A secret's reveal conditions are judged the same way; a trust number alone never
//! authorises a disclosure.
//!
//! All of it is the character's own: none of it is in a listener's brief.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::{
    BehaviorUse, MAXIMUM_ENTRY_CHARS, MAXIMUM_LIST_ENTRIES, PersonaError, PersonaField, check_line,
    check_list, is_label,
};

/// How much it would cost the character if a secret came out. A closed set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sensitivity {
    /// Embarrassing at worst.
    Low,
    /// Costly, but survivable.
    #[default]
    Medium,
    /// Ruinous or dangerous if it came out.
    High,
    /// The character would do almost anything to keep it hidden.
    Critical,
}

impl Sensitivity {
    /// Every level, mildest first.
    pub const ALL: [Self; 4] = [Self::Low, Self::Medium, Self::High, Self::Critical];

    /// The serde name.
    pub fn key(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Critical => "critical",
        }
    }

    /// The level with serde name `key`.
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|level| level.key() == key)
    }
}

/// Something the character knows and keeps back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonaSecret {
    /// Names the secret within the persona: letters, digits, `-` or `_`.
    pub id: String,
    /// What the secret is.
    pub text: String,
    #[serde(default)]
    pub sensitivity: Sensitivity,
    /// What must hold before the character may let it out, one plain condition per entry, such as
    /// "the speaker has proved they are not guild". Judged by a model; empty means never freely.
    #[serde(default)]
    pub reveal_conditions: Vec<String>,
    /// The characters who already know it, by persona id.
    #[serde(default)]
    pub known_by: Vec<String>,
}

/// A stance the character can take, such as `guarded` or `haggling`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonaBehavior {
    /// Names the behavior within the persona: letters, digits, `-` or `_`.
    pub id: String,
    /// How they act and what they will and will not do in this stance.
    pub description: String,
    /// How they talk in this stance, when it differs from their usual style.
    #[serde(default)]
    pub speech_style: String,
}

/// Something that moves the character into a behavior.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonaTrigger {
    /// Names the trigger within the persona: letters, digits, `-` or `_`.
    pub id: String,
    /// When it fires, as a plain statement a model judges against what just happened, such as
    /// "the speaker mentions the guild or the tax collector". Never matched against words.
    pub condition: String,
    /// A committed event kind the engine can witness that also fires it, by key. It is resolved
    /// when the persona is bound to an NPC; until then it is only a name.
    #[serde(default)]
    pub event: Option<String>,
    /// The behavior it moves the character into: a behavior id of this persona.
    pub behavior: String,
}

fn check_item_id(field: PersonaField, id: &str) -> Result<(), PersonaError> {
    if is_label(id) {
        Ok(())
    } else {
        Err(PersonaError::InvalidItemId {
            field,
            id: id.chars().take(super::MAXIMUM_ID_CHARS + 1).collect(),
        })
    }
}

fn check_count(field: PersonaField, entries: usize) -> Result<(), PersonaError> {
    if entries > MAXIMUM_LIST_ENTRIES {
        return Err(PersonaError::TooMany {
            field,
            entries,
            maximum: MAXIMUM_LIST_ENTRIES,
        });
    }
    Ok(())
}

fn check_unique<'a>(
    field: PersonaField,
    seen: &mut HashSet<&'a str>,
    id: &'a str,
) -> Result<(), PersonaError> {
    if seen.insert(id) {
        Ok(())
    } else {
        Err(PersonaError::DuplicateItemId {
            field,
            id: id.to_owned(),
        })
    }
}

fn check_secrets(secrets: &[PersonaSecret]) -> Result<(), PersonaError> {
    check_count(PersonaField::Secrets, secrets.len())?;
    let mut seen = HashSet::new();
    for secret in secrets {
        check_item_id(PersonaField::Secrets, &secret.id)?;
        check_unique(PersonaField::Secrets, &mut seen, &secret.id)?;
        check_line(
            PersonaField::Secrets,
            &secret.text,
            MAXIMUM_ENTRY_CHARS,
            true,
        )?;
        check_list(PersonaField::Secrets, &secret.reveal_conditions)?;
        check_count(PersonaField::Secrets, secret.known_by.len())?;
        for who in &secret.known_by {
            check_item_id(PersonaField::Secrets, who)?;
        }
    }
    Ok(())
}

/// Checks the behaviors and answers their ids, for the references that follow.
fn check_behaviors(behaviors: &[PersonaBehavior]) -> Result<HashSet<&str>, PersonaError> {
    check_count(PersonaField::Behaviors, behaviors.len())?;
    let mut known = HashSet::new();
    for behavior in behaviors {
        check_item_id(PersonaField::Behaviors, &behavior.id)?;
        check_unique(PersonaField::Behaviors, &mut known, &behavior.id)?;
        check_line(
            PersonaField::Behaviors,
            &behavior.description,
            MAXIMUM_ENTRY_CHARS,
            true,
        )?;
        check_line(
            PersonaField::Behaviors,
            &behavior.speech_style,
            MAXIMUM_ENTRY_CHARS,
            false,
        )?;
    }
    Ok(known)
}

fn check_triggers(
    triggers: &[PersonaTrigger],
    known_behaviors: &HashSet<&str>,
) -> Result<(), PersonaError> {
    check_count(PersonaField::Triggers, triggers.len())?;
    let mut seen = HashSet::new();
    for trigger in triggers {
        check_item_id(PersonaField::Triggers, &trigger.id)?;
        check_unique(PersonaField::Triggers, &mut seen, &trigger.id)?;
        check_line(
            PersonaField::Triggers,
            &trigger.condition,
            MAXIMUM_ENTRY_CHARS,
            true,
        )?;
        if let Some(event) = &trigger.event {
            check_item_id(PersonaField::Triggers, event)?;
        }
        if !known_behaviors.contains(trigger.behavior.as_str()) {
            return Err(PersonaError::UnknownBehavior {
                behavior: trigger.behavior.clone(),
                used_by: BehaviorUse::Trigger {
                    id: trigger.id.clone(),
                },
            });
        }
    }
    Ok(())
}

/// Checks the whole policy: every item valid, ids unique within their list, and every behavior a
/// trigger or the default names present.
pub(super) fn check(
    secrets: &[PersonaSecret],
    behaviors: &[PersonaBehavior],
    default_behavior: Option<&str>,
    triggers: &[PersonaTrigger],
) -> Result<(), PersonaError> {
    check_secrets(secrets)?;
    let known_behaviors = check_behaviors(behaviors)?;
    if let Some(default) = default_behavior
        && !known_behaviors.contains(default)
    {
        return Err(PersonaError::UnknownBehavior {
            behavior: default.to_owned(),
            used_by: BehaviorUse::Default,
        });
    }
    check_triggers(triggers, &known_behaviors)
}
