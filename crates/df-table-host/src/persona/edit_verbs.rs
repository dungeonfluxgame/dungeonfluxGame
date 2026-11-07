//! The grammar of the persona verbs that change the policy a persona keeps: `secret`, `behavior`
//! and `trigger`. Each builds the `PersonaEdit` values of the one edit route.

use df_content::persona::{
    PersonaBehavior, PersonaEdit, PersonaSecret, PersonaTrigger, Sensitivity,
};

use super::cli::{
    PERSONA_BEHAVIOR_USAGE, PERSONA_SECRET_USAGE, PERSONA_TRIGGER_USAGE, PersonaCommand, edit,
    flag, free_text,
};
use super::options::take_options;
use crate::command::{CommandError, bad_choice};

pub(super) fn parse_secret(words: &[String]) -> Result<PersonaCommand, CommandError> {
    match words {
        [id, verb, secret, rest @ ..] if verb == "add" => {
            let (given, text) = take_options(rest, &["sensitivity", "reveal", "known_by"]);
            let mut put = PersonaSecret {
                id: secret.clone(),
                text: free_text(text, PERSONA_SECRET_USAGE)?,
                sensitivity: Sensitivity::default(),
                reveal_conditions: Vec::new(),
                known_by: Vec::new(),
            };
            for (key, value) in given {
                match key {
                    "sensitivity" => {
                        put.sensitivity = Sensitivity::from_key(value).ok_or_else(|| {
                            let keys: Vec<&str> =
                                Sensitivity::ALL.iter().map(|level| level.key()).collect();
                            bad_choice("sensitivity", value, keys.join(", "))
                        })?;
                    }
                    "reveal" => put.reveal_conditions.push(value.to_owned()),
                    _ => put.known_by.push(value.to_owned()),
                }
            }
            edit(id, vec![PersonaEdit::PutSecret { secret: put }])
        }
        [id, verb, secret] if verb == "drop" => {
            edit(id, vec![PersonaEdit::RemoveSecret { id: secret.clone() }])
        }
        _ => Err(CommandError::Usage(PERSONA_SECRET_USAGE)),
    }
}

pub(super) fn parse_behavior(words: &[String]) -> Result<PersonaCommand, CommandError> {
    match words {
        [id, verb, behavior, rest @ ..] if verb == "add" => {
            let (given, text) = take_options(rest, &["default", "style"]);
            let mut put = PersonaBehavior {
                id: behavior.clone(),
                description: free_text(text, PERSONA_BEHAVIOR_USAGE)?,
                speech_style: String::new(),
            };
            let mut make_default = false;
            for (key, value) in given {
                match key {
                    "default" => make_default = flag("default", value)?,
                    _ => put.speech_style = value.to_owned(),
                }
            }
            let mut edits = vec![PersonaEdit::PutBehavior { behavior: put }];
            if make_default {
                edits.push(PersonaEdit::SetDefaultBehavior {
                    id: Some(behavior.clone()),
                });
            }
            edit(id, edits)
        }
        [id, verb, behavior] if verb == "drop" => edit(
            id,
            vec![PersonaEdit::RemoveBehavior {
                id: behavior.clone(),
            }],
        ),
        _ => Err(CommandError::Usage(PERSONA_BEHAVIOR_USAGE)),
    }
}

pub(super) fn parse_trigger(words: &[String]) -> Result<PersonaCommand, CommandError> {
    match words {
        [id, verb, trigger, rest @ ..] if verb == "add" => {
            let (given, text) = take_options(rest, &["behavior", "event"]);
            let behavior = given
                .iter()
                .find(|(key, _)| *key == "behavior")
                .map(|(_, value)| (*value).to_owned())
                .ok_or(CommandError::Usage(PERSONA_TRIGGER_USAGE))?;
            let event = given
                .iter()
                .find(|(key, _)| *key == "event")
                .map(|(_, value)| (*value).to_owned());
            let put = PersonaTrigger {
                id: trigger.clone(),
                condition: free_text(text, PERSONA_TRIGGER_USAGE)?,
                event,
                behavior,
            };
            edit(id, vec![PersonaEdit::PutTrigger { trigger: put }])
        }
        [id, verb, trigger] if verb == "drop" => edit(
            id,
            vec![PersonaEdit::RemoveTrigger {
                id: trigger.clone(),
            }],
        ),
        _ => Err(CommandError::Usage(PERSONA_TRIGGER_USAGE)),
    }
}
