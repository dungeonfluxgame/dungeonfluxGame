//! The grammar of the persona verbs that change the policy a persona keeps (`secret`, `behavior`
//! and `trigger`) and how it sounds (`voice`, `expertise`). Each builds the `PersonaEdit` values
//! of the one edit route.

use df_content::persona::{
    PersonaBehavior, PersonaEdit, PersonaField, PersonaSecret, PersonaTrigger, Sensitivity,
    VoiceField,
};

use super::cli::{
    PERSONA_BEHAVIOR_USAGE, PERSONA_EXPERTISE_USAGE, PERSONA_SECRET_USAGE, PERSONA_TRIGGER_USAGE,
    PERSONA_VOICE_USAGE, PersonaCommand, edit, flag, free_text, id_word,
};
use super::options::{asking_from, take_options};
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

/// `word` as a voice part that is one text (`text`) or a list of entries (otherwise).
fn voice_field(word: &str, text: bool) -> Result<VoiceField, CommandError> {
    VoiceField::from_key(word)
        .filter(|field| field.is_text() == text)
        .ok_or_else(|| {
            let keys: Vec<&str> = VoiceField::ALL
                .into_iter()
                .filter(|field| field.is_text() == text)
                .map(VoiceField::key)
                .collect();
            bad_choice("voice field", word, keys.join(", "))
        })
}

fn entry_index(word: &str) -> Result<usize, CommandError> {
    word.parse()
        .map_err(|_| bad_choice("index", word, "a whole number from 0"))
}

pub(super) fn parse_voice(words: &[String]) -> Result<PersonaCommand, CommandError> {
    let usage = || CommandError::Usage(PERSONA_VOICE_USAGE);
    match words {
        [id, verb, field, text @ ..] if verb == "set" && !text.is_empty() => edit(
            id,
            vec![PersonaEdit::SetVoiceText {
                field: voice_field(field, true)?,
                text: text.join(" "),
            }],
        ),
        [id, verb, field, text @ ..] if verb == "add" && !text.is_empty() => edit(
            id,
            vec![PersonaEdit::AddVoiceEntry {
                field: voice_field(field, false)?,
                text: text.join(" "),
            }],
        ),
        [id, verb, field, index] if verb == "drop" => edit(
            id,
            vec![PersonaEdit::RemoveVoiceEntry {
                field: voice_field(field, false)?,
                index: entry_index(index)?,
            }],
        ),
        [id, noun, verb, text @ ..] if noun == "sample" && verb == "add" && !text.is_empty() => {
            edit(
                id,
                vec![PersonaEdit::AddVoiceEntry {
                    field: VoiceField::Samples,
                    text: text.join(" "),
                }],
            )
        }
        [id, noun, verb, index] if noun == "sample" && verb == "drop" => edit(
            id,
            vec![PersonaEdit::RemoveVoiceEntry {
                field: VoiceField::Samples,
                index: entry_index(index)?,
            }],
        ),
        [id, noun, verb, pack] if noun == "pack" && verb == "add" => edit(
            id,
            vec![PersonaEdit::AddVoicePack { pack: pack.clone() }],
        ),
        [id, noun, verb, pack] if noun == "pack" && verb == "drop" => edit(
            id,
            vec![PersonaEdit::RemoveVoicePack { pack: pack.clone() }],
        ),
        [id, verb, rest @ ..] if verb == "generate" => {
            let (given, rest) = take_options(rest, &["model"]);
            if !rest.is_empty() {
                return Err(usage());
            }
            Ok(PersonaCommand::VoiceGenerate {
                id: id_word(id)?,
                model: asking_from(&given)?.model,
            })
        }
        _ => Err(usage()),
    }
}

pub(super) fn parse_expertise(words: &[String]) -> Result<PersonaCommand, CommandError> {
    match words {
        [id, verb, text @ ..] if verb == "add" && !text.is_empty() => edit(
            id,
            vec![PersonaEdit::AddEntry {
                field: PersonaField::Expertise,
                text: text.join(" "),
            }],
        ),
        [id, verb, index] if verb == "drop" => edit(
            id,
            vec![PersonaEdit::RemoveEntry {
                field: PersonaField::Expertise,
                index: entry_index(index)?,
            }],
        ),
        _ => Err(CommandError::Usage(PERSONA_EXPERTISE_USAGE)),
    }
}
