//! The `persona` verbs of the dev command line: their grammar, the request each sends, and the
//! text of each answer.
//!
//! Every verb that changes a stored persona (`set`, `add`, `drop`, `secret`, `behavior`,
//! `trigger`, `voice`, `expertise`, `edit`) builds `PersonaEdit` values and sends them to the one
//! edit route; the host
//! applies them all together or not at all. `write`, `generate`, `say`, `ask`, `coverage`,
//! `backfill` and `backfills` have routes of their own, and so has `voice <id> generate`. The
//! questions to the model share the
//! options of `options` (`model=`, the guidance keys). A mistake in a file given to `write` or
//! `edit` is found here before anything is sent, as the same typed `PersonaError` a route
//! answers, in a `DevError::Rejected`.

use std::path::{Path, PathBuf};
use std::time::Instant;

use df_content::persona::backfill::{FILLABLE, Gap, NarrativeGuidance};
use df_content::persona::{
    MAXIMUM_ID_CHARS, Persona, PersonaEdit, PersonaField, PersonaId, PublicPersona,
};
use serde::Deserialize;

use super::converse::{ConverseRequest, DEFAULT_TURNS, MAXIMUM_TURNS, converse};
use super::edit_verbs::{
    parse_behavior, parse_expertise, parse_secret, parse_trigger, parse_voice,
};
use super::format::{
    format_answered, format_backfilled, format_backfills, format_covered, format_generation,
    format_list, format_owner, format_public, format_spoken, format_voice_generated,
};
use super::options::{Asking, GUIDANCE_KEYS, GuidanceArgs, asking_from, take_options};
use super::request::{
    Answered, AskRequest, Audience, BackfillRecord, BackfillRequest, Backfilled, CastOutcome,
    CastRow, CoverageRequest, Covered, EditRequest, Edited, GenerateCastRequest, GenerateRequest,
    Generation, PersonaError, PersonaRemoved, PersonaReply, PersonaStored, PersonaSummary,
    RemoveRequest, SayRequest, Spoken, VoiceRequest,
};
use super::routes::path;
use super::storage::{RESERVED_IDS, persona_id};
use crate::command::{CommandError, bad_choice};
use crate::dev::{DevError, ReplyError, read_text_file};
use crate::dev_client::{self, get_json, post_json};
use crate::dev_http::parse_body;
use crate::imagegen::MAXIMUM_NAME_CHARS;
use crate::imagegen::model::plain_name;

/// Every `persona` verb, for the unknown-verb refusal and the help.
pub const PERSONA_USAGE: &str = concat!(
    "persona [list] | persona show <id> [owner] | persona write <id> <file> | ",
    "persona set <id> <field> <text...> | persona add <id> <list-field> <text...> | ",
    "persona drop <id> <list-field> <index> | persona secret <id> add|drop ... | ",
    "persona behavior <id> add|drop ... | persona trigger <id> add|drop ... | ",
    "persona voice <id> set|add|drop|sample|pack|generate ... | ",
    "persona expertise <id> add|drop ... | ",
    "persona edit <id> <file.json> | persona generate <character|all> [overwrite=true|false] ",
    "[model=<name>] <guidance options> [hint <text...>] | ",
    "persona say <character> [model=<name>] [speak=true|false] [recent=<text>]... ",
    "[from=<id>] <situation...> | ",
    "persona converse [turns=N] [opener=<character>] [speak=true|false] [pace=true|false] ",
    "[model=<name>] [<topic...>] | ",
    "persona ask <character> [model=<name>] [backfill=true|false] [coverage=true|false] ",
    "[speak=true|false] [situation=<text>] [recent=<text>]... [from=<id>] ",
    "<guidance options> <question...> | ",
    "persona coverage <character> [model=<name>] ",
    "<guidance options> <question...> | persona backfill <character> gap=<field>:<topic>... ",
    "[model=<name>] <guidance options> | persona backfills <id> | persona remove <id>",
);
/// The guidance options, as every command that takes them says them.
/// The guidance options, as every command that takes them says them.
pub const GUIDANCE_OPTIONS: &str = concat!(
    "guidance=<file.json> arc=<text> phase=<text> beat=<text> hook=<text>... thread=<text>... ",
    "establish=<text>... avoid=<text>... tone=<text> latitude=strict|guided|open note=<text>",
);
/// `persona show`: one persona, public or owner view.
pub const PERSONA_SHOW_USAGE: &str = "persona show <id> [owner]";
/// `persona write`: a persona from a JSON file, replacing the same id.
pub const PERSONA_WRITE_USAGE: &str = "persona write <id> <file>";
/// `persona set`: replace one text field.
pub const PERSONA_SET_USAGE: &str = "persona set <id> <field> <text...>";
/// `persona add`: add an entry to a list field.
pub const PERSONA_ADD_USAGE: &str = "persona add <id> <list-field> <text...>";
/// `persona drop`: remove an entry of a list field by index.
pub const PERSONA_DROP_USAGE: &str = "persona drop <id> <list-field> <index>";
/// `persona secret`: add (or replace) or drop a secret.
pub const PERSONA_SECRET_USAGE: &str = concat!(
    "persona secret <id> add <secret-id> [sensitivity=low|medium|high|critical] ",
    "[reveal=<condition>]... [known_by=<character>]... <text...> | ",
    "persona secret <id> drop <secret-id>",
);
/// `persona behavior`: add (or replace) or drop a behavior.
pub const PERSONA_BEHAVIOR_USAGE: &str = concat!(
    "persona behavior <id> add <behavior-id> [default=true] [style=<text>] <description...> | ",
    "persona behavior <id> drop <behavior-id>",
);
/// `persona trigger`: add (or replace) or drop a trigger.
pub const PERSONA_TRIGGER_USAGE: &str = concat!(
    "persona trigger <id> add <trigger-id> behavior=<behavior-id> [event=<key>] ",
    "<condition...> | persona trigger <id> drop <trigger-id>",
);
/// `persona voice`: change the voice, its packs, or have the text service write them.
pub const PERSONA_VOICE_USAGE: &str = concat!(
    "persona voice <id> set rhythm|address|deflection <text...> | ",
    "persona voice <id> add favored|never|tics|idioms|samples <text...> | ",
    "persona voice <id> drop favored|never|tics|idioms|samples <index> | ",
    "persona voice <id> sample add <text...> | persona voice <id> sample drop <index> | ",
    "persona voice <id> pack add|drop <pack-id> | persona voice <id> generate [model=<name>]",
);
/// `persona expertise`: add or drop what a character knows from their trade or life.
pub const PERSONA_EXPERTISE_USAGE: &str =
    "persona expertise <id> add <text...> | persona expertise <id> drop <index>";
/// `persona edit`: apply the edits of a JSON file.
pub const PERSONA_EDIT_USAGE: &str = "persona edit <id> <file.json>";
/// `persona generate`: have the text service write personas.
pub const PERSONA_GENERATE_USAGE: &str = concat!(
    "persona generate <character|all> [overwrite=true|false] [model=<name>] <guidance options> ",
    "[hint <text...>]",
);
/// `persona say`: a line from a character, cued on the scene.
pub const PERSONA_SAY_USAGE: &str = concat!(
    "persona say <character> [model=<name>] [speak=true|false] [recent=<text>]... [from=<id>] ",
    "<situation...>",
);
/// `persona converse`: the characters of the shown scene talk to each other.
pub const PERSONA_CONVERSE_USAGE: &str = concat!(
    "persona converse [turns=N] [opener=<character>] [speak=true|false] [pace=true|false] ",
    "[plate=<name>] [model=<name>] [<topic...>]",
);
/// `persona ask`: coverage, backfill of gaps, then the line.
pub const PERSONA_ASK_USAGE: &str = concat!(
    "persona ask <character> [model=<name>] [backfill=true|false] [coverage=true|false] ",
    "[speak=true|false] [situation=<text>] [recent=<text>]... [from=<id>] ",
    "<guidance options> <question...>",
);
/// `persona coverage`: whether the persona holds enough to answer.
pub const PERSONA_COVERAGE_USAGE: &str =
    "persona coverage <character> [model=<name>] <guidance options> <question...>";
/// `persona backfill`: fill named gaps by adding detail.
pub const PERSONA_BACKFILL_USAGE: &str =
    "persona backfill <character> gap=<field>:<topic>... [model=<name>] <guidance options>";
/// `persona backfills`: the record of what was added.
pub const PERSONA_BACKFILLS_USAGE: &str = "persona backfills <id>";
/// `persona remove`: delete a persona; its backfill record stays.
pub const PERSONA_REMOVE_USAGE: &str = "persona remove <id>";

/// The fields `persona set` replaces: the ones that are one text.
pub const TEXT_FIELDS: [PersonaField; 5] = [
    PersonaField::DisplayName,
    PersonaField::Job,
    PersonaField::SpeechStyle,
    PersonaField::Backstory,
    PersonaField::PrivateBackstory,
];
/// The fields `persona add` and `persona drop` change: the ones that are a list of entries.
pub const LIST_FIELDS: [PersonaField; 6] = [
    PersonaField::Personality,
    PersonaField::Motivations,
    PersonaField::Goals,
    PersonaField::Needs,
    PersonaField::Fears,
    PersonaField::Boundaries,
];

/// Whose personas a `persona generate` writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GenerateTarget {
    /// Every named character of the shown scene.
    All,
    Character(PersonaId),
}

/// A persona command of the dev CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PersonaCommand {
    /// List the stored personas.
    List,
    /// Show one persona to `audience`.
    Show { id: PersonaId, audience: Audience },
    /// Store the persona in the JSON file at `path`, which must be the persona `id`.
    Write { id: PersonaId, path: PathBuf },
    /// Apply edits to a stored persona.
    Edit {
        id: PersonaId,
        edits: Vec<PersonaEdit>,
    },
    /// Apply the edits in the JSON file at `path` (`{"edits": [...]}`) to a stored persona.
    EditFile { id: PersonaId, path: PathBuf },
    /// Delete a stored persona.
    Remove { id: PersonaId },
    /// Have the text service write personas.
    Generate {
        target: GenerateTarget,
        overwrite: bool,
        hint: Option<String>,
        asking: Asking,
    },
    /// Have the text service word a line from a character, and cue it.
    Say {
        character: PersonaId,
        situation: String,
        model: Option<String>,
        speak: bool,
        recent: Vec<String>,
        asked_by: Option<String>,
    },
    /// Have the text service write only the voice, expertise and packs of a stored persona.
    VoiceGenerate {
        id: PersonaId,
        model: Option<String>,
    },
    /// Have the characters of the shown scene talk to each other.
    Converse(ConverseRequest),
    /// Put a question to a character: coverage, backfill of gaps, then the line.
    Ask {
        character: PersonaId,
        question: String,
        backfill: bool,
        coverage: bool,
        speak: bool,
        situation: Option<String>,
        recent: Vec<String>,
        asked_by: Option<String>,
        asking: Asking,
    },
    /// Ask whether a persona holds enough to answer a question.
    Coverage {
        character: PersonaId,
        question: String,
        asking: Asking,
    },
    /// Fill gaps of a persona by adding detail.
    Backfill {
        character: PersonaId,
        gaps: Vec<Gap>,
        asking: Asking,
    },
    /// List the backfills committed to a persona.
    Backfills { id: PersonaId },
}

/// The file `persona edit` reads.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EditFile {
    edits: Vec<PersonaEdit>,
}

/// `word` as a persona id, or the refusal naming the rule.
pub(super) fn id_word(word: &str) -> Result<PersonaId, CommandError> {
    persona_id(word).map_err(|_| {
        bad_choice(
            "id",
            word,
            format!(
                "1 to {MAXIMUM_ID_CHARS} letters, digits, '-' or '_', and none of: {}",
                RESERVED_IDS.join(", ")
            ),
        )
    })
}

/// `word` as one of `allowed`, or the refusal listing them.
fn field_word(word: &str, allowed: &[PersonaField]) -> Result<PersonaField, CommandError> {
    PersonaField::from_key(word)
        .filter(|field| allowed.contains(field))
        .ok_or_else(|| {
            let keys: Vec<&str> = allowed.iter().map(|field| field.key()).collect();
            bad_choice("field", word, keys.join(", "))
        })
}

/// A command that edits `id` with `edits`.
pub(super) fn edit(id: &str, edits: Vec<PersonaEdit>) -> Result<PersonaCommand, CommandError> {
    Ok(PersonaCommand::Edit {
        id: id_word(id)?,
        edits,
    })
}

/// The text of `words` as one string, or `usage` when there is none.
pub(super) fn free_text(words: &[String], usage: &'static str) -> Result<String, CommandError> {
    if words.is_empty() {
        Err(CommandError::Usage(usage))
    } else {
        Ok(words.join(" "))
    }
}

pub(super) fn flag(key: &'static str, value: &str) -> Result<bool, CommandError> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(bad_choice(key, value, "true or false")),
    }
}

fn parse_generate(words: &[String]) -> Result<PersonaCommand, CommandError> {
    let usage = || CommandError::Usage(PERSONA_GENERATE_USAGE);
    let [target, rest @ ..] = words else {
        return Err(usage());
    };
    let target = if target == "all" {
        GenerateTarget::All
    } else {
        GenerateTarget::Character(id_word(target)?)
    };
    let keys = [&["overwrite", "model"][..], &GUIDANCE_KEYS].concat();
    let (given, rest) = take_options(rest, &keys);
    let mut overwrite = false;
    for (_, value) in given.iter().filter(|(key, _)| *key == "overwrite") {
        overwrite = flag("overwrite", value)?;
    }
    let hint = match rest {
        [] => None,
        [word, text @ ..] if word == "hint" && !text.is_empty() => Some(text.join(" ")),
        _ => return Err(usage()),
    };
    Ok(PersonaCommand::Generate {
        target,
        overwrite,
        hint,
        asking: asking_from(&given)?,
    })
}

/// The `recent=` lines and the `from=` speaker among `given`: the talk a line is said in.
fn talk_from(given: &[(&str, &str)]) -> Result<(Vec<String>, Option<String>), CommandError> {
    let recent = given
        .iter()
        .filter(|(key, _)| *key == "recent")
        .map(|(_, value)| (*value).to_owned())
        .collect();
    let mut asked_by = None;
    for (_, value) in given.iter().filter(|(key, _)| *key == "from") {
        if asked_by.replace((*value).to_owned()).is_some() {
            return Err(CommandError::DuplicateKey("from".to_owned()));
        }
    }
    Ok((recent, asked_by))
}

fn parse_say(words: &[String]) -> Result<PersonaCommand, CommandError> {
    let [character, rest @ ..] = words else {
        return Err(CommandError::Usage(PERSONA_SAY_USAGE));
    };
    let (given, situation) = take_options(rest, &["model", "speak", "recent", "from"]);
    let (recent, asked_by) = talk_from(&given)?;
    let mut speak = false;
    for (_, value) in given.iter().filter(|(key, _)| *key == "speak") {
        speak = flag("speak", value)?;
    }
    Ok(PersonaCommand::Say {
        character: id_word(character)?,
        situation: free_text(situation, PERSONA_SAY_USAGE)?,
        model: asking_from(&given)?.model,
        speak,
        recent,
        asked_by,
    })
}

fn parse_converse(words: &[String]) -> Result<PersonaCommand, CommandError> {
    let (given, rest) = take_options(
        words,
        &["turns", "opener", "speak", "pace", "plate", "model"],
    );
    let mut turns = DEFAULT_TURNS;
    let (mut opener, mut plate, mut speak, mut pace) = (None, None, false, true);
    for (key, value) in &given {
        match *key {
            "turns" => {
                turns = value
                    .parse()
                    .ok()
                    .filter(|n| (1..=MAXIMUM_TURNS).contains(n))
                    .ok_or_else(|| {
                        bad_choice(
                            key,
                            value,
                            format!("a whole number from 1 to {MAXIMUM_TURNS}"),
                        )
                    })?;
            }
            "opener" => opener = Some(id_word(value)?.to_string()),
            "speak" => speak = flag("speak", value)?,
            "pace" => pace = flag("pace", value)?,
            "plate" => {
                if !plain_name(value) {
                    return Err(bad_choice(
                        key,
                        value,
                        format!(
                            "1 to {MAXIMUM_NAME_CHARS} letters, digits, '-' or '_', with no                              extension"
                        ),
                    ));
                }
                plate = Some((*value).to_owned());
            }
            _ => {}
        }
    }
    Ok(PersonaCommand::Converse(ConverseRequest {
        turns,
        opener,
        topic: (!rest.is_empty()).then(|| rest.join(" ")),
        model: asking_from(&given)?.model,
        speak,
        pace,
        plate,
    }))
}

fn parse_ask(words: &[String]) -> Result<PersonaCommand, CommandError> {
    let [character, rest @ ..] = words else {
        return Err(CommandError::Usage(PERSONA_ASK_USAGE));
    };
    let keys = [
        &[
            "model",
            "backfill",
            "coverage",
            "speak",
            "situation",
            "recent",
            "from",
        ][..],
        &GUIDANCE_KEYS,
    ]
    .concat();
    let (given, question) = take_options(rest, &keys);
    let (recent, asked_by) = talk_from(&given)?;
    let (mut backfill, mut coverage, mut speak) = (true, true, false);
    let mut situation = None;
    for &(key, value) in &given {
        match key {
            "backfill" => backfill = flag("backfill", value)?,
            "coverage" => coverage = flag("coverage", value)?,
            "speak" => speak = flag("speak", value)?,
            "situation" => situation = Some(value.to_owned()),
            _ => {}
        }
    }
    Ok(PersonaCommand::Ask {
        character: id_word(character)?,
        question: free_text(question, PERSONA_ASK_USAGE)?,
        backfill,
        coverage,
        speak,
        situation,
        recent,
        asked_by,
        asking: asking_from(&given)?,
    })
}

fn parse_coverage(words: &[String]) -> Result<PersonaCommand, CommandError> {
    let [character, rest @ ..] = words else {
        return Err(CommandError::Usage(PERSONA_COVERAGE_USAGE));
    };
    let keys = [&["model"][..], &GUIDANCE_KEYS].concat();
    let (given, question) = take_options(rest, &keys);
    Ok(PersonaCommand::Coverage {
        character: id_word(character)?,
        question: free_text(question, PERSONA_COVERAGE_USAGE)?,
        asking: asking_from(&given)?,
    })
}

/// `field:topic` as a gap, or the refusal listing the fields a backfill can fill.
fn gap_word(word: &str) -> Result<Gap, CommandError> {
    let fillable = || {
        let keys: Vec<&str> = FILLABLE.iter().map(|field| field.key()).collect();
        keys.join(", ")
    };
    let (field, topic) = word
        .split_once(':')
        .filter(|(_, topic)| !topic.trim().is_empty())
        .ok_or_else(|| bad_choice("gap", word, "<field>:<topic>"))?;
    let field = PersonaField::from_key(field)
        .filter(|field| FILLABLE.contains(field))
        .ok_or_else(|| bad_choice("gap field", field, fillable()))?;
    Ok(Gap {
        field,
        topic: topic.to_owned(),
    })
}

fn parse_backfill(words: &[String]) -> Result<PersonaCommand, CommandError> {
    let [character, rest @ ..] = words else {
        return Err(CommandError::Usage(PERSONA_BACKFILL_USAGE));
    };
    let keys = [&["gap", "model"][..], &GUIDANCE_KEYS].concat();
    let (given, rest) = take_options(rest, &keys);
    if !rest.is_empty() {
        return Err(CommandError::Usage(PERSONA_BACKFILL_USAGE));
    }
    let gaps = given
        .iter()
        .filter(|(key, _)| *key == "gap")
        .map(|(_, value)| gap_word(value))
        .collect::<Result<Vec<_>, _>>()?;
    if gaps.is_empty() {
        return Err(CommandError::Usage(PERSONA_BACKFILL_USAGE));
    }
    Ok(PersonaCommand::Backfill {
        character: id_word(character)?,
        gaps,
        asking: asking_from(&given)?,
    })
}

/// Reads the words after `persona`.
pub fn parse(words: &[String]) -> Result<PersonaCommand, CommandError> {
    let Some((verb, rest)) = words.split_first() else {
        return Ok(PersonaCommand::List);
    };
    match (verb.as_str(), rest) {
        ("list", []) => Ok(PersonaCommand::List),
        ("show", [id]) => Ok(PersonaCommand::Show {
            id: id_word(id)?,
            audience: Audience::Public,
        }),
        ("show", [id, owner]) if owner == "owner" => Ok(PersonaCommand::Show {
            id: id_word(id)?,
            audience: Audience::Owner,
        }),
        ("show", _) => Err(CommandError::Usage(PERSONA_SHOW_USAGE)),
        ("write", [id, file]) => Ok(PersonaCommand::Write {
            id: id_word(id)?,
            path: PathBuf::from(file),
        }),
        ("write", _) => Err(CommandError::Usage(PERSONA_WRITE_USAGE)),
        ("set", [id, field, text @ ..]) if !text.is_empty() => edit(
            id,
            vec![PersonaEdit::SetText {
                field: field_word(field, &TEXT_FIELDS)?,
                text: text.join(" "),
            }],
        ),
        ("set", _) => Err(CommandError::Usage(PERSONA_SET_USAGE)),
        ("add", [id, field, text @ ..]) if !text.is_empty() => edit(
            id,
            vec![PersonaEdit::AddEntry {
                field: field_word(field, &LIST_FIELDS)?,
                text: text.join(" "),
            }],
        ),
        ("add", _) => Err(CommandError::Usage(PERSONA_ADD_USAGE)),
        ("drop", [id, field, index]) => edit(
            id,
            vec![PersonaEdit::RemoveEntry {
                field: field_word(field, &LIST_FIELDS)?,
                index: index
                    .parse()
                    .map_err(|_| bad_choice("index", index, "a whole number from 0"))?,
            }],
        ),
        ("drop", _) => Err(CommandError::Usage(PERSONA_DROP_USAGE)),
        ("secret", rest) => parse_secret(rest),
        ("behavior", rest) => parse_behavior(rest),
        ("trigger", rest) => parse_trigger(rest),
        ("voice", rest) => parse_voice(rest),
        ("expertise", rest) => parse_expertise(rest),
        ("edit", [id, file]) => Ok(PersonaCommand::EditFile {
            id: id_word(id)?,
            path: PathBuf::from(file),
        }),
        ("edit", _) => Err(CommandError::Usage(PERSONA_EDIT_USAGE)),
        ("generate", rest) => parse_generate(rest),
        ("say", rest) => parse_say(rest),
        ("ask", rest) => parse_ask(rest),
        ("coverage", rest) => parse_coverage(rest),
        ("backfill", rest) => parse_backfill(rest),
        ("backfills", [id]) => Ok(PersonaCommand::Backfills { id: id_word(id)? }),
        ("backfills", _) => Err(CommandError::Usage(PERSONA_BACKFILLS_USAGE)),
        ("converse", words) => parse_converse(words),
        ("remove", [id]) => Ok(PersonaCommand::Remove { id: id_word(id)? }),
        ("remove", _) => Err(CommandError::Usage(PERSONA_REMOVE_USAGE)),
        _ => Err(CommandError::Usage(PERSONA_USAGE)),
    }
}

/// A refusal found by the command line itself, as the failure a route's refusal would be.
fn refused(error: PersonaError) -> DevError {
    DevError::Rejected {
        message: error.to_string(),
        error: ReplyError::Persona(error),
    }
}

/// The value of a persona reply, or its refusal (the typed `PersonaError`) as the command's
/// failure.
pub(super) fn reply_value<T>(reply: PersonaReply<T>) -> Result<T, DevError> {
    dev_client::reply_value(reply)
}

/// The address of one persona's view for `audience`.
fn show_path(id: &PersonaId, audience: Audience) -> String {
    format!(
        "{}?audience={}",
        path::PERSONA_ONE.replace("{id}", id.as_str()),
        audience.key()
    )
}

/// The persona in the JSON `text`, which must be the persona `id`.
fn persona_of_file(id: &PersonaId, text: &str) -> Result<Persona, PersonaError> {
    let persona: Persona = parse_body(text.as_bytes())?;
    if persona.id() == id {
        Ok(persona)
    } else {
        Err(PersonaError::IdMismatch {
            expected: id.to_string(),
            found: persona.id().to_string(),
        })
    }
}

async fn write_file(port: u16, id: &PersonaId, file: &Path) -> Result<String, DevError> {
    let text = read_text_file(file).await?;
    let persona = persona_of_file(id, &text).map_err(refused)?;
    let stored: PersonaStored = reply_value(post_json(port, path::PERSONA, &persona).await?)?;
    Ok(format!(
        "ok {} {}: {}, {}",
        if stored.replaced {
            "replaced"
        } else {
            "created"
        },
        stored.persona.id,
        stored.persona.display_name,
        stored.persona.job
    ))
}

/// Sends `edits` to the edit route and tells what came of them.
async fn send_edits(
    port: u16,
    id: &PersonaId,
    edits: Vec<PersonaEdit>,
) -> Result<String, DevError> {
    let count = edits.len();
    let request = EditRequest {
        id: id.to_string(),
        edits,
    };
    let edited: Edited = reply_value(post_json(port, path::PERSONA_EDIT, &request).await?)?;
    Ok(format!(
        "ok edited {id} ({count} edit{}, {})",
        if count == 1 { "" } else { "s" },
        if edited.changed {
            "changed"
        } else {
            "unchanged"
        }
    ))
}

async fn edit_file(port: u16, id: &PersonaId, file: &Path) -> Result<String, DevError> {
    let text = read_text_file(file).await?;
    let EditFile { edits } = parse_body(text.as_bytes())
        .map_err(PersonaError::from)
        .map_err(refused)?;
    send_edits(port, id, edits).await
}

/// The guidance `args` make, reading the file they name, if any.
async fn guidance_of(args: GuidanceArgs) -> Result<Option<NarrativeGuidance>, DevError> {
    let file = match &args.file {
        Some(path) => {
            let text = read_text_file(path).await?;
            let guidance = parse_body(text.as_bytes())
                .map_err(PersonaError::from)
                .map_err(refused)?;
            Some(guidance)
        }
        None => None,
    };
    Ok(args.merge(file))
}

async fn generate(
    port: u16,
    target: GenerateTarget,
    overwrite: bool,
    hint: Option<String>,
    asking: Asking,
) -> Result<String, DevError> {
    let model = asking.model;
    let guidance = guidance_of(asking.guidance).await?;
    match target {
        GenerateTarget::Character(character) => {
            let request = GenerateRequest {
                character: character.to_string(),
                hint,
                guidance,
                model,
                overwrite,
            };
            let done: Generation =
                reply_value(post_json(port, path::PERSONA_GENERATE, &request).await?)?;
            let outcome = CastOutcome::from(Ok::<_, PersonaError>(done));
            Ok(format_generation(character.as_str(), &outcome))
        }
        GenerateTarget::All => {
            let request = GenerateCastRequest {
                hint,
                guidance,
                model,
                overwrite,
            };
            let rows: Vec<CastRow> =
                reply_value(post_json(port, path::PERSONA_GENERATE_CAST, &request).await?)?;
            if rows.is_empty() {
                return Ok("no named characters in the shown scene".to_owned());
            }
            let lines: Vec<String> = rows
                .iter()
                .map(|row| format_generation(&row.character, &row.outcome))
                .collect();
            Ok(lines.join("\n"))
        }
    }
}

/// What `persona ask` takes besides the model and the guidance.
struct AskExtras {
    backfill: bool,
    coverage: bool,
    speak: bool,
    situation: Option<String>,
    recent: Vec<String>,
    asked_by: Option<String>,
}

async fn ask_character(
    port: u16,
    character: PersonaId,
    question: String,
    extras: AskExtras,
    asking: Asking,
) -> Result<String, DevError> {
    let request = AskRequest {
        character: character.to_string(),
        question,
        guidance: guidance_of(asking.guidance).await?,
        model: asking.model,
        backfill: extras.backfill,
        coverage: extras.coverage,
        speak: extras.speak,
        situation: extras.situation,
        recent: extras.recent,
        asked_by: extras.asked_by,
    };
    let started = Instant::now();
    let answered: Answered = reply_value(post_json(port, path::PERSONA_ASK, &request).await?)?;
    Ok(format!(
        "{}\n  total: {} ms",
        format_answered(&answered),
        started.elapsed().as_millis()
    ))
}

async fn cover(
    port: u16,
    character: PersonaId,
    question: String,
    asking: Asking,
) -> Result<String, DevError> {
    let request = CoverageRequest {
        character: character.to_string(),
        question,
        guidance: guidance_of(asking.guidance).await?,
        model: asking.model,
    };
    let covered: Covered = reply_value(post_json(port, path::PERSONA_COVERAGE, &request).await?)?;
    Ok(format_covered(&covered))
}

async fn backfill_character(
    port: u16,
    character: PersonaId,
    gaps: Vec<Gap>,
    asking: Asking,
) -> Result<String, DevError> {
    let request = BackfillRequest {
        character: character.to_string(),
        gaps,
        guidance: guidance_of(asking.guidance).await?,
        model: asking.model,
    };
    let done: Backfilled = reply_value(post_json(port, path::PERSONA_BACKFILL, &request).await?)?;
    Ok(format_backfilled(&character, &done))
}

async fn show_backfills(port: u16, id: PersonaId) -> Result<String, DevError> {
    let address = format!("{}?id={id}", path::PERSONA_BACKFILLS);
    let records: Vec<BackfillRecord> = reply_value(get_json(port, &address).await?)?;
    Ok(format_backfills(&id, &records))
}

async fn say(
    port: u16,
    character: PersonaId,
    situation: String,
    model: Option<String>,
    speak: bool,
    talk: (Vec<String>, Option<String>),
) -> Result<String, DevError> {
    let request = SayRequest {
        character: character.to_string(),
        situation,
        model,
        speak,
        recent: talk.0,
        asked_by: talk.1,
    };
    let spoken: Spoken = reply_value(post_json(port, path::PERSONA_SAY, &request).await?)?;
    Ok(format_spoken(&spoken))
}

async fn generate_voice(
    port: u16,
    id: PersonaId,
    model: Option<String>,
) -> Result<String, DevError> {
    let request = VoiceRequest {
        id: id.to_string(),
        model,
    };
    let edited: Edited = reply_value(post_json(port, path::PERSONA_VOICE, &request).await?)?;
    Ok(format_voice_generated(&edited.persona))
}

async fn show(port: u16, id: PersonaId, audience: Audience) -> Result<String, DevError> {
    let address = show_path(&id, audience);
    Ok(match audience {
        Audience::Public => {
            let shown: PublicPersona = reply_value(get_json(port, &address).await?)?;
            format_public(&shown)
        }
        Audience::Owner => {
            let shown: Persona = reply_value(get_json(port, &address).await?)?;
            format_owner(&shown)
        }
    })
}

async fn remove(port: u16, id: PersonaId) -> Result<String, DevError> {
    let request = RemoveRequest { id: id.to_string() };
    let removed: PersonaRemoved =
        reply_value(post_json(port, path::PERSONA_REMOVE, &request).await?)?;
    Ok(format!("ok removed {}", removed.id))
}

/// Runs one persona command against the host on `port`.
pub async fn run(port: u16, command: PersonaCommand) -> Result<String, DevError> {
    match command {
        PersonaCommand::List => {
            let list: Vec<PersonaSummary> = reply_value(get_json(port, path::PERSONA).await?)?;
            Ok(format_list(&list))
        }
        PersonaCommand::Show { id, audience } => show(port, id, audience).await,
        PersonaCommand::Write { id, path: file } => write_file(port, &id, &file).await,
        PersonaCommand::Edit { id, edits } => send_edits(port, &id, edits).await,
        PersonaCommand::EditFile { id, path: file } => edit_file(port, &id, &file).await,
        PersonaCommand::Remove { id } => remove(port, id).await,
        PersonaCommand::Generate {
            target,
            overwrite,
            hint,
            asking,
        } => generate(port, target, overwrite, hint, asking).await,
        PersonaCommand::Ask {
            character,
            question,
            backfill,
            coverage,
            speak,
            situation,
            recent,
            asked_by,
            asking,
        } => {
            let extras = AskExtras {
                backfill,
                coverage,
                speak,
                situation,
                recent,
                asked_by,
            };
            ask_character(port, character, question, extras, asking).await
        }
        PersonaCommand::Coverage {
            character,
            question,
            asking,
        } => cover(port, character, question, asking).await,
        PersonaCommand::Backfill {
            character,
            gaps,
            asking,
        } => backfill_character(port, character, gaps, asking).await,
        PersonaCommand::Backfills { id } => show_backfills(port, id).await,
        PersonaCommand::Converse(request) => converse(port, request).await,
        PersonaCommand::Say {
            character,
            situation,
            model,
            speak,
            recent,
            asked_by,
        } => say(port, character, situation, model, speak, (recent, asked_by)).await,
        PersonaCommand::VoiceGenerate { id, model } => generate_voice(port, id, model).await,
    }
}
