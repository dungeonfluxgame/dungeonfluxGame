//! The grammar of the persona verbs that change the policy a persona keeps (`secret`, `behavior`
//! and `trigger`) and how it sounds (`voice`, `expertise`). Each builds the `PersonaEdit` values
//! of the one edit route.

use df_content::persona::{
    EmotionDelta, EmotionalTrigger, PersonaBehavior, PersonaEdit, PersonaField, PersonaSecret,
    PersonaTrigger, Sensitivity, StateCondition, TemperamentField, VoiceField,
};

use super::cli::{
    PERSONA_BEHAVIOR_USAGE, PERSONA_EMOTION_USAGE, PERSONA_EXPERTISE_USAGE, PERSONA_SECRET_USAGE,
    PERSONA_STATE_USAGE, PERSONA_TEMPERAMENT_USAGE, PERSONA_TRIGGER_USAGE, PERSONA_VOICE_USAGE,
    PersonaCommand, edit, flag, free_text, id_word,
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
        [id, noun, verb, pack] if noun == "pack" && verb == "add" => {
            edit(id, vec![PersonaEdit::AddVoicePack { pack: pack.clone() }])
        }
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

fn level_word(word: &str) -> Result<u8, CommandError> {
    word.parse::<u8>()
        .ok()
        .filter(|level| *level <= 100)
        .ok_or_else(|| bad_choice("level", word, "a whole number from 0 to 100"))
}

fn delta_word(word: &str) -> Result<i8, CommandError> {
    word.parse::<i8>()
        .ok()
        .filter(|delta| (-100..=100).contains(delta))
        .ok_or_else(|| bad_choice("delta", word, "a whole number from -100 to 100"))
}

/// `value` as `<emotion>:<number>`, the number read by `number`.
fn emotion_pair<T>(
    key: &'static str,
    value: &str,
    number: fn(&str) -> Result<T, CommandError>,
) -> Result<(String, T), CommandError> {
    let (emotion, amount) = value
        .split_once(':')
        .ok_or_else(|| bad_choice(key, value, "<emotion>:<number>"))?;
    Ok((emotion.to_owned(), number(amount)?))
}

pub(super) fn parse_temperament(words: &[String]) -> Result<PersonaCommand, CommandError> {
    let usage = || CommandError::Usage(PERSONA_TEMPERAMENT_USAGE);
    match words {
        [id, verb, field, value] if verb == "set" => {
            let field = TemperamentField::from_key(field).ok_or_else(|| {
                let keys: Vec<&str> = TemperamentField::ALL.into_iter().map(|f| f.key()).collect();
                bad_choice("temperament field", field, keys.join(", "))
            })?;
            let value = value
                .parse::<u8>()
                .map_err(|_| bad_choice("value", value, "a whole number from 0 to 100"))?;
            edit(id, vec![PersonaEdit::SetTemperament { field, value }])
        }
        [id, verb, emotion, level] if verb == "baseline" => edit(
            id,
            vec![PersonaEdit::SetBaseline {
                emotion: emotion.clone(),
                level: level_word(level)?,
            }],
        ),
        [id, verb, rest @ ..] if verb == "generate" => {
            let (given, rest) = take_options(rest, &["model"]);
            if !rest.is_empty() {
                return Err(usage());
            }
            Ok(PersonaCommand::TemperamentGenerate {
                id: id_word(id)?,
                model: asking_from(&given)?.model,
            })
        }
        _ => Err(usage()),
    }
}

pub(super) fn parse_emotion(words: &[String]) -> Result<PersonaCommand, CommandError> {
    let usage = || CommandError::Usage(PERSONA_EMOTION_USAGE);
    match words {
        [id, noun, verb, trigger, rest @ ..] if noun == "trigger" && verb == "add" => {
            let (given, text) = take_options(rest, &["state", "set", "behavior", "event"]);
            let mut put = EmotionalTrigger {
                id: trigger.clone(),
                when_state: None,
                when_event: None,
                sets: Vec::new(),
                behavior: None,
            };
            let mut event: Vec<&str> = Vec::new();
            for (key, value) in given {
                match key {
                    "state" => {
                        let (emotion, at_least) = emotion_pair("state", value, level_word)?;
                        put.when_state = Some(StateCondition { emotion, at_least });
                    }
                    "set" => {
                        let (emotion, delta) = emotion_pair("set", value, delta_word)?;
                        put.sets.push(EmotionDelta { emotion, delta });
                    }
                    "behavior" => put.behavior = Some(value.to_owned()),
                    _ => event.push(value),
                }
            }
            event.extend(text.iter().map(String::as_str));
            if !event.is_empty() {
                put.when_event = Some(event.join(" "));
            }
            if put.when_state.is_none() && put.when_event.is_none() {
                return Err(usage());
            }
            edit(id, vec![PersonaEdit::PutEmotionalTrigger { trigger: put }])
        }
        [id, noun, verb, trigger] if noun == "trigger" && verb == "drop" => edit(
            id,
            vec![PersonaEdit::RemoveEmotionalTrigger {
                id: trigger.clone(),
            }],
        ),
        _ => Err(usage()),
    }
}

pub(super) fn parse_state(words: &[String]) -> Result<PersonaCommand, CommandError> {
    match words {
        [id] => Ok(PersonaCommand::State { id: id_word(id)? }),
        [id, verb] if verb == "reset" => Ok(PersonaCommand::StateReset { id: id_word(id)? }),
        [id, verb, emotion, level] if verb == "set" => Ok(PersonaCommand::StateSet {
            id: id_word(id)?,
            emotion: emotion.clone(),
            level: level_word(level)?,
        }),
        [id, verb, toward] if verb == "knows" => Ok(PersonaCommand::StateKnows {
            id: id_word(id)?,
            toward: toward.clone(),
            known: true,
        }),
        [id, verb, toward, known] if verb == "knows" => Ok(PersonaCommand::StateKnows {
            id: id_word(id)?,
            toward: toward.clone(),
            known: super::cli::flag("known", known)?,
        }),
        [id, verb, toward, rapport] if verb == "rapport" => Ok(PersonaCommand::StateRapport {
            id: id_word(id)?,
            toward: toward.clone(),
            rapport: rapport
                .parse::<i8>()
                .ok()
                .filter(|rapport| (-100..=100).contains(rapport))
                .ok_or(CommandError::Usage(PERSONA_STATE_USAGE))?,
        }),
        _ => Err(CommandError::Usage(PERSONA_STATE_USAGE)),
    }
}
