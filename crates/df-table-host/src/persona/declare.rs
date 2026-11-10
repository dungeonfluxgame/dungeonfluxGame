//! A character declares the physical action they are doing right now (`persona declare`,
//! `POST /dev/persona/declare`): in the first person and the present tense, naming what the hands
//! are doing and with what ("I set the board on my easel and lay in the door with ochre").
//!
//! The declaration is spoken like any line (voice, bubble, the speaker's name), and it is the
//! subject of an emergency plate: a picture of the scene showing that action in progress, which
//! the frame watch makes ahead of everything else and shows at once (`frame_watch`, `plate`).
//!
//! The words are either the caller's own (`text=`, spoken as written; no model reads them, so
//! they change no inventory and no feeling) or the character's: the per-line model call is told
//! `now.declare` and answers `declared_action` against a closed schema: what the hands do for a
//! camera (`doing`, no name), the `pose` and the `items_in_use`. Code checks the answer against
//! the facts the call was given: an item the character does not hold is refused, and the line is
//! worded again once with the reason; a second miss is a refusal (`LineError::Declaration`), never
//! a picture of something they do not carry. Which action the words declare is the model's to
//! judge; code never reads them.
//!
//! The command prints the line as `persona say` does, then `  declared: ..` and one `watch: ` line:
//! the input (`@declare <id> | {..}`) the market runner hands the frame watch that is running
//! (`WatchInput::Declare`), because the watch owns the screen and its queue.

use df_content::persona::Persona;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::cli::reply_value;
use super::format::format_spoken;
use super::generate::ShownScene;
use super::line::{Occasion, say_line};
use super::options::take_options;
use super::request::{LineError, PersonaError, SaidLine, Spoken};
use super::state::load_state;
use crate::command::{CommandError, bad_choice};
use crate::dev::DevError;
use crate::dev_client::{get_json, post_json};
use crate::state::AppState;
use crate::text::TextRoute;
use crate::world::InventoryItem;

/// The address of the route (POST a [`DeclareRequest`]). A dev tool; it asks the text service
/// unless the request brings its own words.
pub(super) const PERSONA_DECLARE: &str = "/dev/persona/declare";

/// `persona declare`: a character declares the action they are doing right now.
pub const PERSONA_DECLARE_USAGE: &str = concat!(
    "persona declare <character> [model=<name>] [speak=true|false] [recent=<text>]... ",
    "[from=<id>] [text=<the declaration, spoken as written>] [doing=<what the picture shows, ",
    "with no name>] [uses=<item id>,<item id>...] [<situation...>]"
);

/// What the line is asked to do: a declaration. Data for the model, like any intent.
pub(super) const DECLARE_INTENT: &str = concat!(
    "Declare aloud the one concrete physical action you are doing right now: in the first ",
    "person and the present tense, saying what your hands are doing and with what, as it ",
    "happens (never a plan, a wish or something already done)."
);

/// Added to the line instruction when the call is a declaration.
pub(super) const DECLARE_INSTRUCTION: &str = concat!(
    "now.declare is true: the line is a declaration of the physical action the character is ",
    "doing at this very moment, in the first person and the present tense, naming what their ",
    "hands are doing and what they do it with, concretely. declared_action reports that same ",
    "action for a camera: doing is the action in the plain present tense of a third person, ",
    "with no name, saying what the hands do and with what; pose is how their body stands, ",
    "sits or leans while they do it, or null; items_in_use are the ids, from now.inventory, of ",
    "the things their hands use or hold in it. An item is used only when now.inventory holds ",
    "at least one of it: when the situation leaves them without what the action needs, they ",
    "declare what they can do with what they hold, or that they set off to get it, and the ",
    "facts decide, not the wish. declared_action is the action of the line, nothing else."
);

/// What a declaration does for the camera, as the call's answer reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeclaredAction {
    /// The action in the plain present tense of a third person, with no name: what the picture
    /// shows the hands doing and with what.
    pub doing: String,
    /// How the body stands, sits or leans while they do it.
    #[serde(default)]
    pub pose: Option<String>,
    /// The ids of the things in the character's inventory that the hands use or hold in it.
    #[serde(default)]
    pub items_in_use: Vec<String>,
}

/// The body of `POST /dev/persona/declare`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeclareRequest {
    pub character: String,
    /// What is happening, as the character meets it; the declaration is worded from it. Data for
    /// the model; not needed with `text`.
    #[serde(default)]
    pub situation: String,
    /// The declaration itself, spoken as written. No model reads it.
    #[serde(default)]
    pub text: Option<String>,
    /// For `text`: what the picture shows, in the plain present tense of a third person with no
    /// name; the text itself when not given.
    #[serde(default)]
    pub doing: Option<String>,
    /// For `text`: the ids of the things the hands use.
    #[serde(default)]
    pub uses: Vec<String>,
    /// The model to ask instead of the persona default.
    #[serde(default)]
    pub model: Option<String>,
    /// Voice the line and play it with its bubble.
    #[serde(default)]
    pub speak: bool,
    /// The last lines of the conversation, oldest first.
    #[serde(default)]
    pub recent: Vec<String>,
    /// Who the line is said to, when known.
    #[serde(default)]
    pub asked_by: Option<String>,
}

// Words are data for the model and never printed: the Debug of a request shows their lengths.
impl std::fmt::Debug for DeclareRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DeclareRequest")
            .field("character", &self.character)
            .field("situation_characters", &self.situation.chars().count())
            .field("has_text", &self.text.is_some())
            .field("uses", &self.uses)
            .field("model", &self.model)
            .field("speak", &self.speak)
            .field("recent_lines", &self.recent.len())
            .finish()
    }
}

/// The ids of `uses` that `inventory` does not hold at least one of, each once, in order.
pub(super) fn unheld(uses: &[String], inventory: &[InventoryItem]) -> Vec<String> {
    let mut missing: Vec<String> = Vec::new();
    for id in uses {
        let held = inventory
            .iter()
            .any(|item| item.id == *id && item.count > 0);
        if !held && !missing.contains(id) {
            missing.push(id.clone());
        }
    }
    missing
}

/// Why `action` does not match the facts, or none when it does: it says what the hands do, and
/// every item it uses is one the character holds now.
pub(super) fn check_declared(
    action: &DeclaredAction,
    inventory: &[InventoryItem],
) -> Result<(), String> {
    if action.doing.trim().is_empty() {
        return Err("declared_action.doing is empty: say what the hands are doing".to_owned());
    }
    let missing = unheld(&action.items_in_use, inventory);
    if missing.is_empty() {
        return Ok(());
    }
    let held: Vec<&str> = inventory
        .iter()
        .filter(|item| item.count > 0)
        .map(|item| item.id.as_str())
        .collect();
    Err(format!(
        "declared_action uses {}, which the character does not hold now; they hold: {}",
        missing.join(", "),
        if held.is_empty() {
            "nothing".to_owned()
        } else {
            held.join(", ")
        }
    ))
}

/// Why the call's answer is not a usable declaration, or none: only a declaring call is checked.
pub(super) fn declared_miss(
    declare: bool,
    declared: Option<&DeclaredAction>,
    inventory: &[InventoryItem],
) -> Option<String> {
    if !declare {
        return None;
    }
    match declared {
        None => Some(
            "declared_action is null: report the action the line declares, for the camera"
                .to_owned(),
        ),
        Some(action) => check_declared(action, inventory).err(),
    }
}

/// `schema`, a line's answer schema, with the closed `declared_action` of a declaration added and
/// required.
pub(super) fn add_declared_to(schema: &mut Value) {
    schema["properties"]["declared_action"] = json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["doing", "pose", "items_in_use"],
        "properties": {
            "doing": {
                "type": "string",
                "description": concat!(
                    "What the hands do and with what, plain present tense, third person, ",
                    "no name."
                ),
            },
            "pose": {
                "type": ["string", "null"],
                "description": "How the body stands, sits or leans while they do it, or null."
            },
            "items_in_use": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Ids from now.inventory of what the hands use or hold in it."
            },
        },
    });
    if let Some(required) = schema["required"].as_array_mut() {
        required.push(Value::from("declared_action"));
    }
}

/// The ids of `uses` without a repeat, in order.
pub(super) fn distinct(uses: Vec<String>) -> Vec<String> {
    let mut seen: Vec<String> = Vec::with_capacity(uses.len());
    for id in uses {
        if !seen.contains(&id) {
            seen.push(id);
        }
    }
    seen
}

/// The declaration of `request` as a line of `own`: its own words (`text`, no model), or the
/// per-line call asked to declare, with its answer checked against what the character holds.
pub(super) async fn declared_line(
    app: &AppState,
    route: &TextRoute,
    scene: &ShownScene,
    own: &Persona,
    request: &DeclareRequest,
) -> Result<SaidLine, PersonaError> {
    let character = own.id().as_str();
    let inventory = crate::world::inventory_of(app, character);
    if let Some(text) = &request.text {
        let line = text.trim();
        if line.is_empty() {
            return Err(PersonaError::EmptyLine);
        }
        let action = DeclaredAction {
            doing: request.doing.as_deref().map_or(line, str::trim).to_owned(),
            pose: None,
            items_in_use: distinct(request.uses.clone()),
        };
        check_declared(&action, &inventory).map_err(refused)?;
        app.record_line(format!(
            "PERSONA declaration id={character} verbatim=true items={}",
            action.items_in_use.len()
        ));
        return Ok(spoken_as_written(app, character, line, action));
    }
    if request.situation.trim().is_empty() {
        return Err(PersonaError::EmptyLine);
    }
    let occasion = Occasion {
        situation: &request.situation,
        question: None,
        guidance: None,
        not_established: &[],
        recent: &request.recent,
        asked_by: request.asked_by.as_deref(),
        intent: Some(DECLARE_INTENT),
        strike: false,
        declare: true,
    };
    let said = say_line(app, route, scene, own, occasion).await?;
    app.record_line(format!(
        "PERSONA declaration id={character} verbatim=false items={} retried={}",
        said.declared
            .as_ref()
            .map_or(0, |action| action.items_in_use.len()),
        said.repeat_retried
    ));
    Ok(said)
}

/// The refusal of a declaration that does not match the facts.
pub(super) fn refused(detail: String) -> PersonaError {
    PersonaError::AnswerRefused {
        replayed: false,
        cause: LineError::Declaration { detail }.into(),
    }
}

/// A line that is `line` as written, declaring `action`: nothing was judged, so it moves no
/// feeling and no inventory.
fn spoken_as_written(
    app: &AppState,
    character: &str,
    line: &str,
    action: DeclaredAction,
) -> SaidLine {
    SaidLine {
        line: line.to_owned(),
        behavior: None,
        fired_triggers: Vec::new(),
        fired_emotional_triggers: Vec::new(),
        state_triggered: Vec::new(),
        behavior_override: None,
        revealed_secrets: Vec::new(),
        quotes: Vec::new(),
        leak_suspect: false,
        gaps: Vec::new(),
        appraisal: df_content::persona::Appraisal::default(),
        dropped: Vec::new(),
        tone: None,
        repeats_recent: false,
        repeat_retried: false,
        retry_reason: None,
        names_learned: Vec::new(),
        inventory: Vec::new(),
        declared: Some(action),
        state: load_state(app, character),
        model_ms: 0,
        total_ms: 0,
    }
}

/// What the frame watch is handed for a declaration: the words spoken and the action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WatchDeclared {
    pub line: String,
    pub action: DeclaredAction,
}

/// The prefix of the watch input that declares an action: `@declare <speaker id> | <json>`.
pub(super) const DECLARE_PREFIX: &str = "@declare ";

/// The frame watch input for `speaker` declaring `action` in `line`.
pub fn watch_input(speaker: &str, line: &str, action: &DeclaredAction) -> String {
    let body = WatchDeclared {
        line: line.to_owned(),
        action: action.clone(),
    };
    // Plain fields: serialising cannot fail, and the JSON is one line.
    let json = serde_json::to_string(&body).unwrap_or_default();
    format!("{DECLARE_PREFIX}{speaker} | {json}")
}

/// A `persona declare` command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclareCommand {
    pub request: DeclareRequest,
}

/// The value of the option `key` among `given`, refusing a key given twice.
fn single<'a>(given: &[(&'a str, &'a str)], key: &str) -> Result<Option<&'a str>, CommandError> {
    let mut values = given.iter().filter(|(seen, _)| *seen == key);
    let first = values.next().map(|(_, value)| *value);
    if values.next().is_some() {
        return Err(CommandError::DuplicateKey(key.to_owned()));
    }
    Ok(first)
}

/// Reads the words after `persona declare`.
pub(super) fn parse_declare(words: &[String]) -> Result<DeclareCommand, CommandError> {
    let usage = || CommandError::Usage(PERSONA_DECLARE_USAGE);
    let [character, rest @ ..] = words else {
        return Err(usage());
    };
    let character = super::cli::id_word(character)?.to_string();
    let (given, situation) = take_options(
        rest,
        &["model", "speak", "recent", "from", "text", "doing", "uses"],
    );
    let text = single(&given, "text")?.map(str::to_owned);
    let situation = situation.join(" ");
    if text.is_none() && situation.trim().is_empty() {
        return Err(usage());
    }
    let uses = match single(&given, "uses")? {
        Some(list) => {
            let mut ids = Vec::new();
            for id in list.split(',') {
                let id = id.trim();
                if id.is_empty() {
                    return Err(bad_choice("uses", list, "item ids joined by ','"));
                }
                ids.push(id.to_owned());
            }
            ids
        }
        None => Vec::new(),
    };
    Ok(DeclareCommand {
        request: DeclareRequest {
            character,
            situation,
            text,
            doing: single(&given, "doing")?.map(str::to_owned),
            uses,
            model: single(&given, "model")?.map(str::to_owned),
            speak: single(&given, "speak")?
                .map(|value| super::cli::flag("speak", value))
                .transpose()?
                .unwrap_or(false),
            recent: given
                .iter()
                .filter(|(key, _)| *key == "recent")
                .map(|(_, value)| (*value).to_owned())
                .collect(),
            asked_by: single(&given, "from")?.map(str::to_owned),
        },
    })
}

/// The inventory of `owner`, from the host.
pub(super) async fn held_by(port: u16, owner: &str) -> Result<Vec<InventoryItem>, DevError> {
    use crate::world::inventory::request::{InventoryReply, InventoryView};
    let address = format!(
        "{}?owner={owner}",
        crate::world::inventory::routes::path::INVENTORY_OWNER
    );
    let reply: InventoryReply<InventoryView> = get_json(port, &address).await?;
    Ok(crate::dev_client::reply_value(reply)?.inventory.items)
}

/// Runs `persona declare` against the host on `port`: the line as `persona say` prints it, what is
/// declared, and the watch input that hands the declaration to a frame watch.
pub(super) async fn run(port: u16, command: DeclareCommand) -> Result<String, DevError> {
    let spoken: Spoken = reply_value(post_json(port, PERSONA_DECLARE, &command.request).await?)?;
    let sheets =
        super::plate::sheets_for_gains(port, &spoken.character, &spoken.said.inventory).await;
    let mut lines = vec![format_spoken(&spoken)];
    lines.extend(sheets.iter().map(|line| format!("  {line}")));
    if let Some(action) = &spoken.said.declared {
        lines.push(format!(
            "  declared: doing={} | pose={} | uses={}",
            action.doing.trim(),
            action.pose.as_deref().unwrap_or("-"),
            if action.items_in_use.is_empty() {
                "-".to_owned()
            } else {
                action.items_in_use.join(",")
            }
        ));
        lines.push(format!(
            "watch: {}",
            watch_input(&spoken.character, &spoken.said.line, action)
        ));
    }
    Ok(lines.join("\n"))
}

/// What a declared plate must not show, in plain neutral words, beside the house look's own: an
/// edit of a picture that is told about a person can paint them again.
pub(super) const NO_DUPLICATES: [&str; 3] = ["the same person twice", "clones", "extra people"];

/// The scene a declared action shows, for the picture: the person by their look and never their
/// name (a name in a picture's text comes out as writing on a sign), what their hands do, how they
/// stand and `held`, the names of the things in use, and how many people the scene's cast has: the
/// person described is one of them and appears once, nobody is added or copied, and whoever else
/// the picture already shows stays as it was. The rest of the picture is as it was.
pub(super) fn declared_scene(
    look: &str,
    action: &DeclaredAction,
    held: &[String],
    cast: usize,
) -> String {
    let doing = action.doing.trim().trim_end_matches('.');
    let pose = action
        .pose
        .as_deref()
        .map(|pose| pose.trim().trim_end_matches('.'))
        .filter(|pose| !pose.is_empty())
        .map_or_else(String::new, |pose| format!(", {pose}"));
    let carrying = if held.is_empty() {
        String::new()
    } else {
        format!(", with their {} in view", held.join(" and "))
    };
    let cast = if cast <= 1 {
        "The cast in view is one person, the one described, who appears once and only once."
            .to_owned()
    } else {
        format!(
            concat!(
                "The cast in view is {} people; the one described is one of them and appears ",
                "once and only once."
            ),
            cast
        )
    };
    format!(
        concat!(
            "A person ({}) {}{}{}. {} Nobody is added and no one is copied: anyone else ",
            "already in the picture, such as small distant figures, stays as small, as far ",
            "away and as unlike this person as before. Everything else in the scene is as it ",
            "was."
        ),
        look, doing, pose, carrying, cast
    )
}
