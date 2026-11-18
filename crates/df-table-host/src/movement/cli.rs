//! The `move` verbs of the dev command line for a scene transition: `move say` runs the whole path
//! from a line to the brief, and `move interpret`, `move design` and `move brief` stop after one
//! stage. Every stage runs on its fake, so no provider is called, except the plate step: `plate=NAME`
//! makes the new backdrop through `deliver_plate`, and the designed place, with the things in sight
//! there, becomes the world's active place (`POST /dev/world/arrive`, its id the plate's name), so
//! a frame judge can name one of them for a close look. `move act` has a character say a line
//! through `persona say` and runs that line as a move: Jev is told the line was said toward going
//! somewhere (`declared: move`), and a first answer that finds no destination in it is read once
//! more with its own reason, as a `retry` record in the trace. `walk=N` asks the design for up to N
//! sights on the way, which are made with the place and shown before it (`walk`), `town=` is the
//! move's role in its town, `hold=` how long each sight is up and `impulse=` the tempo impulse
//! each posts.
//!
//! The party is the shown scene's characters that have a stored persona, read from the host's cast
//! route exactly as `persona converse` reads them. Each stage either advances the transition with a
//! record or fails it with its typed key, and the old scene stays up.

use std::fmt;
use std::time::{Duration, Instant};

use super::brief::{BriefCharacter, LookSheets, build_brief, scene_words};
use super::interpret::retry_after;
use super::routes::{
    BackRequest, DesignReply, DesignRequest, ExpectedResident, InterpretReply, InterpretRequest,
    path as move_path,
};
use super::transition::{Deadlines, Transition, format_trace};
use super::types::{
    Clarification, Declared, Interpreted, MoveIntent, MoveKind, PartyMember, PlateBrief, Resident,
    SceneDesign, TownRole, TransitionStage, Unclear,
};
use super::walk::{Impulse, MINIMUM_HOLD, Through, WalkOptions, walk_through};
use crate::cast::request::{CastList, CastReply};
use crate::cast::routes::path as cast_path;
use crate::command::{CommandError, bad_choice};
use crate::dev::{DevError, ReplyError};
use crate::dev_client::{self, get_json, post_json};
use crate::dev_http::DevReply;
use crate::imagegen::MAXIMUM_NAME_CHARS;
use crate::imagegen::model::plain_name;
use crate::locate::request::CharacterRequest;
use crate::persona::options::take_options;
use crate::persona::request::{
    GenerateRequest, Generation, PersonaError, PersonaReply, PersonaSummary, SaidLine, SayRequest,
    Spoken,
};
use crate::persona::routes::path as persona_path;
use crate::persona::{
    Painting, deliver_plate, image_limits, inventory_sheets, items_that_fit, sheets_for_gains,
};
use crate::shown_scene::ShownSceneError;
use crate::world::InventoryItem;
use crate::world::ShopStock;
use crate::world::inventory::request::{InventoryReply, InventoryView};
use crate::world::inventory::routes::path as inventory_path;
use crate::world::request::{ArriveRequest, PlaceSet, WorldReply};
use crate::world::routes::path as world_path;
use df_content::world_bible::{Place, Thing};

/// Every `move` stage verb, for the unknown-verb refusal and the help.
pub const MOVEMENT_USAGE: &str = concat!(
    "move say [options] <line...> | move act <character> [options] <situation...> | ",
    "move interpret <line...> | move design <line...> | move brief <line...>; options: ",
    "plate=NAME walk=<sights> town=<seeds|within|leaves> hold=<seconds> impulse=<kind>:<strength>",
    "; move act also takes text=<the line, spoken as written> (a player's own words) and ",
    "party=<id>,<id> (the heroes who move with the speaker, named in the picture or not) and ",
    "resident=<id>|<who> (someone the destination has, met there with that id), repeated, and ",
    "to=<destination words> (the caller read the line and said it: the move goes there, the line ",
    "neither read again nor said) | ",
    "move approach <character> target=<id> toward=<words> distance=<band> step=<n> ",
    "from=<plate> plate=NAME [ahead=true] [pace=walk|run|sneak] (one step toward something on ",
    "screen; a run covers one band more; ahead=true ",
    "makes it and shows it only on @take from stdin)",
);

/// A `move` command of the dev CLI: the line, and the last stage to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MoveCommand {
    /// Run every stage, make the plate when one is named, and print each record.
    Say { line: String, options: MoveOptions },
    /// Have `character` say a line about `situation` through `persona say`, then run that line as
    /// a move with them as the speaker; the plate is made when one is named.
    Act {
        character: String,
        situation: String,
        options: MoveOptions,
    },
    /// One step of `character` toward something on screen (`approach`).
    Approach {
        character: String,
        options: super::approach::ApproachOptions,
    },
    /// Read the line, then stop.
    Interpret(String),
    /// Read the line and design the scene, then stop.
    Design(String),
    /// Read the line, design the scene and make the brief, then stop.
    Brief(String),
}

/// The options before a move's text: the plate to make (`plate=NAME`) and what the move asks of
/// its walk (`walk=`, `town=`, `hold=`, `impulse=`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MoveOptions {
    pub plate: Option<String>,
    pub walk: WalkOptions,
    /// `move act`'s line itself, spoken as written instead of worded from the situation.
    pub text: Option<String>,
    /// The party's heroes (`party=`, persona ids): they move with the speaker and are drawn in the
    /// move's plate whether or not the shown picture's scan named them, and a hero of them whom it
    /// did not name still makes the move (`unanchored`).
    pub party: Vec<String>,
    /// Who the destination has (`resident=<id>|<who>`, repeated): each is one of the design's
    /// residents with that id.
    pub residents: Vec<ExpectedResident>,
    /// The destination as the caller already read the line (`to=<words>`): the one reading of
    /// where the line goes, so the move does not read it again, and the caller said the line, so
    /// the move says nothing.
    pub to: Option<String>,
}

/// Why a movement could not be carried through. `report` is everything printed before the failure
/// and the transition's records, ending in the failed record; `message` is the failed record's
/// words (the typed cause of a refused answer, never the model's text).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MoveError {
    Failed {
        report: String,
        key: String,
        message: String,
    },
}

impl fmt::Display for MoveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Failed {
                report,
                key,
                message,
            } => {
                let message = message.lines().collect::<Vec<_>>().join(" ");
                write!(
                    formatter,
                    "{report}\nthe transition failed: {key}: {message}"
                )
            }
        }
    }
}

impl std::error::Error for MoveError {}

// TODO(DEV-12): parse grammar (each verb, empty line usage, unknown verb), walk on fakes for say,
// interpret, design and brief, a not-a-move reading, and a failed stage's report.
/// Reads the words after `move` when they are a stage verb. The caller sends the other `move`
/// forms (an element's id and position) to the canvas.
pub fn parse(words: &[String]) -> Result<MoveCommand, CommandError> {
    let Some((verb, rest)) = words.split_first() else {
        return Err(CommandError::Usage(MOVEMENT_USAGE));
    };
    if rest.is_empty() {
        return Err(CommandError::Usage(MOVEMENT_USAGE));
    }
    let line = rest.join(" ");
    match verb.as_str() {
        "say" => {
            let (options, line) = options_and_text(rest)?;
            if line.is_empty() {
                return Err(CommandError::Usage(MOVEMENT_USAGE));
            }
            Ok(MoveCommand::Say { line, options })
        }
        "act" => {
            let [character, rest @ ..] = rest else {
                return Err(CommandError::Usage(MOVEMENT_USAGE));
            };
            let (options, situation) = options_and_text(rest)?;
            Ok(MoveCommand::Act {
                character: character.clone(),
                situation,
                options,
            })
        }
        "approach" => {
            let [character, rest @ ..] = rest else {
                return Err(CommandError::Usage(MOVEMENT_USAGE));
            };
            Ok(MoveCommand::Approach {
                character: character.clone(),
                options: super::approach::parse_options(rest)?,
            })
        }
        "interpret" => Ok(MoveCommand::Interpret(line)),
        "design" => Ok(MoveCommand::Design(line)),
        "brief" => Ok(MoveCommand::Brief(line)),
        _ => Err(CommandError::Usage(MOVEMENT_USAGE)),
    }
}

/// The options before a move's text, and the text. The `plate=NAME` is checked as `persona
/// converse plate=` checks it. `walk=<N>` asks for at most N sights on the way, `town=<seeds|
/// within|leaves>` is the move's role in its town, `hold=<seconds>` (at least [`MINIMUM_HOLD`])
/// how long each sight stays up, and `impulse=<kind>:<permille>` the tempo impulse each posts; the
/// last of a key given twice counts.
fn options_and_text(words: &[String]) -> Result<(MoveOptions, String), CommandError> {
    let (given, text) = take_options(
        words,
        &[
            "plate", "walk", "town", "hold", "impulse", "text", "party", "resident", "to",
        ],
    );
    let mut options = MoveOptions::default();
    for &(key, value) in &given {
        match key {
            "text" => options.text = Some(value.to_owned()),
            "to" if !value.trim().is_empty() => options.to = Some(value.trim().to_owned()),
            "to" => return Err(bad_choice(key, value, "the destination in words")),
            "resident" => {
                let expected = || bad_choice(key, value, "<persona id>|<who they are>");
                let (id, who) = value.split_once('|').ok_or_else(expected)?;
                let id = id.trim();
                if !df_content::world_bible::is_valid_id(id) || who.trim().is_empty() {
                    return Err(expected());
                }
                options.residents.push(ExpectedResident {
                    id: id.to_owned(),
                    who: who.trim().to_owned(),
                });
            }
            "party" => {
                options.party = value
                    .split(',')
                    .map(str::trim)
                    .map(|id| {
                        if df_content::world_bible::is_valid_id(id) {
                            Ok(id.to_owned())
                        } else {
                            Err(bad_choice(key, value, "persona ids, comma-separated"))
                        }
                    })
                    .collect::<Result<_, _>>()?;
            }
            "plate" if plain_name(value) => options.plate = Some(value.to_owned()),
            "plate" => {
                return Err(bad_choice(
                    key,
                    value,
                    format!(
                        "1 to {MAXIMUM_NAME_CHARS} letters, digits, '-' or '_', with no extension"
                    ),
                ));
            }
            "walk" => {
                options.walk.stages = value
                    .parse()
                    .map_err(|_| bad_choice(key, value, "a whole number of sights".to_owned()))?;
            }
            "town" => {
                options.walk.town =
                    Some(TownRole::from_key(value).ok_or_else(|| {
                        bad_choice(key, value, "seeds, within or leaves".to_owned())
                    })?);
            }
            "hold" => options.walk.hold = Some(hold_of(key, value)?),
            _ => options.walk.impulse = Some(impulse_of(key, value)?),
        }
    }
    if text.is_empty() && options.text.is_none() {
        return Err(CommandError::Usage(MOVEMENT_USAGE));
    }
    Ok((options, text.join(" ")))
}

/// `hold=`: a number of seconds, at least the least a sight stays up.
fn hold_of(key: &str, value: &str) -> Result<Duration, CommandError> {
    let seconds: f64 = value
        .parse()
        .map_err(|_| bad_choice(key, value, "a number of seconds".to_owned()))?;
    let minimum = MINIMUM_HOLD.as_secs_f64();
    if !seconds.is_finite() || seconds < minimum {
        return Err(bad_choice(
            key,
            value,
            format!("at least {minimum} seconds, so a sight can be taken in"),
        ));
    }
    Ok(Duration::from_secs_f64(seconds))
}

/// `impulse=<kind>:<permille>`: a tempo kind and a strength from 0 to 1000.
fn impulse_of(key: &str, value: &str) -> Result<Impulse, CommandError> {
    let expected = || bad_choice(key, value, "<kind>:<strength 0 to 1000>".to_owned());
    let (kind, permille) = value.split_once(':').ok_or_else(expected)?;
    let permille: u16 = permille.parse().map_err(|_| expected())?;
    if kind.is_empty() || permille > 1000 {
        return Err(expected());
    }
    Ok(Impulse {
        kind: kind.to_owned(),
        permille,
    })
}

/// Runs one `move` command against the host on `port` and gives its report.
pub async fn run(port: u16, command: MoveCommand) -> Result<String, DevError> {
    let cast_reply: CastReply<CastList> = get_json(port, cast_path::CAST).await?;
    let cast: CastList = dev_client::reply_value(cast_reply)?;
    let mut present = present_personas(&cast);
    let party = match &command {
        MoveCommand::Say { options, .. } | MoveCommand::Act { options, .. } => {
            options.party.clone()
        }
        _ => Vec::new(),
    };
    if !party.is_empty() {
        present = party_of(port, &present, &party).await?;
    }
    let outcome = match command {
        MoveCommand::Say { line, options } => {
            walk(port, &line, None, &present, None, &options).await
        }
        MoveCommand::Act {
            character,
            situation,
            options,
        } => {
            // A move the caller read and said goes where it read, with nothing said here.
            let (line, inventory) = if let Some(to) = &options.to {
                (
                    options.text.clone().unwrap_or_else(|| to.clone()),
                    Vec::new(),
                )
            } else {
                match speak(port, &character, situation.clone(), options.text.clone()).await {
                    Ok(said) => {
                        // What the line used up or gained of what the speaker holds, as the host
                        // took it.
                        let inventory: Vec<String> = said
                            .inventory
                            .iter()
                            .map(|change| {
                                format!(
                                    "inventory: {} {} -> {} ({})",
                                    change.item, change.before, change.after, change.why
                                )
                            })
                            .chain(sheets_for_gains(port, &character, &said.inventory).await)
                            .collect();
                        (said.line, inventory)
                    }
                    Err(error) => match unanchored(&error, &character, &options) {
                        Some((line, report)) => (line, vec![report]),
                        None => return Err(error),
                    },
                }
            };
            let spoken = Said {
                speaker: &character,
                situation: &situation,
            };
            walk(port, &line, None, &present, Some(spoken), &options)
                .await
                .map(|report| {
                    [report]
                        .into_iter()
                        .chain(inventory)
                        .collect::<Vec<_>>()
                        .join("\n")
                })
        }
        MoveCommand::Approach { character, options } => {
            super::approach::run(port, &character, &options, &cast).await
        }
        MoveCommand::Interpret(line) => {
            walk(
                port,
                &line,
                Some(TransitionStage::Interpreting),
                &present,
                None,
                &MoveOptions::default(),
            )
            .await
        }
        MoveCommand::Design(line) => {
            walk(
                port,
                &line,
                Some(TransitionStage::Designing),
                &present,
                None,
                &MoveOptions::default(),
            )
            .await
        }
        MoveCommand::Brief(line) => {
            walk(
                port,
                &line,
                Some(TransitionStage::Briefing),
                &present,
                None,
                &MoveOptions::default(),
            )
            .await
        }
    };
    outcome.map_err(DevError::Move)
}

/// The line `character` says about `situation`, worded by `persona say` and not voiced.
/// `character`'s line toward going somewhere: worded from `situation`, or `text` as written.
async fn speak(
    port: u16,
    character: &str,
    situation: String,
    text: Option<String>,
) -> Result<SaidLine, DevError> {
    let request = SayRequest {
        character: character.to_owned(),
        situation,
        model: None,
        speak: false,
        recent: Vec::new(),
        asked_by: None,
        intent: None,
        strike: false,
        text,
        expect: None,
    };
    let reply: PersonaReply<Spoken> = post_json(port, persona_path::PERSONA_SAY, &request).await?;
    let spoken = dev_client::reply_value(reply)?;
    Ok(spoken.said)
}

/// The characters a move with a `party=` carries: exactly the party's heroes, in its order, each
/// as the shown scene has them (`present`) or else by their stored persona (`stored`, the persona
/// list), so a hero the shown picture's scan did not name still moves and is drawn. Whoever else
/// the scene shows (the last place's keeper) stays behind: they are not drawn at the destination,
/// not one to talk to there and not a trader there. A party id with no stored persona fails the
/// move (`unknown_party_member`).
/// `party` as `present` (the shown scene's personas) has them, each one it lacks read from the
/// stored personas ([`with_party`]); the stored ones are read only when one is lacking.
pub(super) async fn party_of(
    port: u16,
    present: &[PersonaSummary],
    party: &[String],
) -> Result<Vec<PersonaSummary>, DevError> {
    let stored = if party
        .iter()
        .any(|id| !present.iter().any(|summary| &summary.id == id))
    {
        let listed: PersonaReply<Vec<PersonaSummary>> =
            get_json(port, persona_path::PERSONA).await?;
        dev_client::reply_value(listed)?
    } else {
        Vec::new()
    };
    with_party(present, party, &stored).map_err(DevError::Move)
}

pub(super) fn with_party(
    present: &[PersonaSummary],
    party: &[String],
    stored: &[PersonaSummary],
) -> Result<Vec<PersonaSummary>, MoveError> {
    party
        .iter()
        .map(|id| {
            present
                .iter()
                .chain(stored)
                .find(|summary| &summary.id == id)
                .cloned()
                .ok_or_else(|| MoveError::Failed {
                    report: String::new(),
                    key: "unknown_party_member".to_owned(),
                    message: format!("the party's {id} has no stored persona"),
                })
        })
        .collect()
}

/// The line of a party hero whose words the shown picture cannot anchor: `persona say` refused
/// because the picture's scan named no region for them (`unknown_character` for this speaker), the
/// speaker is of the move's `party=` and the line was given as written (`text=`). The move goes
/// on with that line, unanchored: no bubble on the plate, and the report's `unanchored:` line
/// tells the runner to show it as a caption. Any other refusal is the move's failure.
pub(super) fn unanchored(
    error: &DevError,
    speaker: &str,
    options: &MoveOptions,
) -> Option<(String, String)> {
    let DevError::Rejected {
        error:
            ReplyError::Persona(PersonaError::Shown {
                cause:
                    ShownSceneError::UnknownCharacter {
                        character, picture, ..
                    },
            }),
        ..
    } = error
    else {
        return None;
    };
    let text = options.text.as_ref()?;
    if character != speaker || !options.party.iter().any(|id| id == speaker) {
        return None;
    }
    Some((
        text.clone(),
        format!(
            "unanchored: {speaker} is in the party but not named in {picture}; the line goes on \
             as written"
        ),
    ))
}

/// The shown scene's characters that have a stored persona, in the scene's order.
pub(super) fn present_personas(cast: &CastList) -> Vec<PersonaSummary> {
    cast.members
        .iter()
        .filter_map(|member| {
            cast.personas
                .iter()
                .find(|summary| summary.id == member.name)
                .cloned()
        })
        .collect()
}

/// A line a character said toward going somewhere (`move act`): who said it and the situation
/// they said it in. Jev reads such a line as declared a move.
pub(super) struct Said<'a> {
    pub(super) speaker: &'a str,
    pub(super) situation: &'a str,
}

/// One transition being walked through its stages, with the text printed so far.
struct Walk {
    transition: Transition,
    clock: Instant,
    report: Vec<String>,
}

impl Walk {
    fn start(line: &str) -> Self {
        let clock = Instant::now();
        let transition = Transition::start(1, line, 0, Deadlines::default());
        Self {
            transition,
            clock,
            report: Vec::new(),
        }
    }

    fn now_ms(&self) -> u64 {
        u64::try_from(self.clock.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// Moves to the next stage with a record of `detail`.
    fn advance(&mut self, detail: &str) -> Result<(), MoveError> {
        let now = self.now_ms();
        self.transition
            .advance(now, detail)
            .map(|_| ())
            .map_err(|error| self.failed(error.key(), &error.to_string()))
    }

    /// Moves to the next stage with a record of `detail`, as of `at_ms` on the walk's clock (a
    /// stage that was passed while others were shown).
    fn advance_at(&mut self, at_ms: u64, detail: &str) -> Result<(), MoveError> {
        self.transition
            .advance(at_ms, detail)
            .map(|_| ())
            .map_err(|error| self.failed(error.key(), &error.to_string()))
    }

    /// Fails the transition and returns the report that ends in its failed record.
    fn failed(&mut self, key: &str, message: &str) -> MoveError {
        let now = self.now_ms();
        self.transition.fail(now, key, message);
        self.report.push(format_trace(self.transition.records()));
        MoveError::Failed {
            report: self.report.join("\n"),
            key: key.to_owned(),
            message: message.to_owned(),
        }
    }

    /// Asks Jev for one reading of the request; a refusal or a route that cannot be reached fails
    /// the transition.
    async fn read(
        &mut self,
        port: u16,
        request: &InterpretRequest,
    ) -> Result<Interpreted, MoveError> {
        match post_json::<_, InterpretReply>(port, move_path::MOVE_INTERPRET, request).await {
            Ok(DevReply::Done { value }) => Ok(value),
            Ok(DevReply::Rejected { error, message }) => Err(self.failed(error.key(), &message)),
            Err(error) => Err(self.failed("route_unreachable", &error.to_string())),
        }
    }

    /// Records that the line is read a second time, with the reason of the first reading: the
    /// record's time is the end of the first attempt.
    fn retry(&mut self, refused: &Clarification) {
        let now = self.now_ms();
        self.transition.note(
            now,
            &format!("retry reason={}", clarification_text(refused)),
        );
    }

    /// Ends a line that is not a movement: the transition fails with its key and the answer is
    /// printed, not refused.
    fn unmoved(mut self, key: &str, message: &str) -> String {
        let now = self.now_ms();
        self.transition.fail(now, key, message);
        self.report.push(format!("not moved: {message}"));
        self.finished()
    }

    /// Ends a walk that stopped on a stage: the trace of the records so far.
    fn finished(mut self) -> String {
        self.report.push(format_trace(self.transition.records()));
        self.report.join("\n")
    }
}

/// What a clarification leaves unclear, in a sentence, with the model's reason when it gave one.
fn clarification_text(clarification: &Clarification) -> String {
    let what = match clarification.unclear {
        Unclear::Destination => "the movement names no destination",
        Unclear::Other => "the line cannot be read as one movement",
    };
    match clarification.reason.as_deref() {
        Some(reason) => {
            let reason: Vec<&str> = reason.split_whitespace().collect();
            format!("{what}: {}", reason.join(" "))
        }
        None => what.to_owned(),
    }
}

/// The stages in order, stopping after `stop`; `None` runs the whole path (`move say`, `move act`).
/// `said` is the character who said `line` toward going somewhere in `move act`: the line is
/// printed first, and Jev is told who said it and that it was meant as a move. `options.plate`
/// makes the plate and puts it on the scene, after the sights on the way when the design has any.
pub(super) async fn walk(
    port: u16,
    line: &str,
    stop: Option<TransitionStage>,
    present: &[PersonaSummary],
    said: Option<Said<'_>>,
    options: &MoveOptions,
) -> Result<String, MoveError> {
    let party: Vec<PartyMember> = present
        .iter()
        .map(|summary| PartyMember {
            id: summary.id.clone(),
            display_name: summary.display_name.clone(),
        })
        .collect();
    let mut walk = Walk::start(line);
    if let Some(said) = &said
        && options.to.is_none()
    {
        walk.report.push(format!("{} says: {line}", said.speaker));
    }
    let context = said.as_ref().map(|said| said.situation);
    let declared = if said.is_some() {
        Declared::Move
    } else {
        Declared::Unknown
    };

    let reading = match (&options.to, &said) {
        // The caller's reading of where the line goes stands: the speaker (or the party) walks
        // there, and the line is not read a second time.
        (Some(to), Some(said)) => Interpreted::Move {
            intent: MoveIntent {
                kind: MoveKind::Walk,
                who: if options.party.is_empty() {
                    vec![said.speaker.to_owned()]
                } else {
                    options.party.clone()
                },
                to: to.clone(),
                line: line.to_owned(),
            },
            back_to: None,
        },
        _ => {
            let mut request = InterpretRequest {
                line: line.to_owned(),
                speaker: said.as_ref().map(|said| said.speaker.to_owned()),
                context: context.map(str::to_owned),
                declared,
                refused_before: None,
                party: party.clone(),
                model: None,
            };
            let mut reading = walk.read(port, &request).await?;
            // One more reading, never a loop: Jev found no destination in a line said toward
            // going somewhere, so it is shown its own reason and asked again.
            if let Some(refused) = retry_after(&reading, declared).cloned() {
                walk.retry(&refused);
                request.refused_before = Some(refused);
                reading = walk.read(port, &request).await?;
            }
            reading
        }
    };
    let (intent, back_to) = match reading {
        Interpreted::Move { intent, back_to } => (intent, back_to),
        Interpreted::NotAMove => {
            return Ok(walk.unmoved("not_a_move", "the line is not a movement"));
        }
        Interpreted::NeedsClarification(clarification) => {
            return Ok(walk.unmoved("needs_clarification", &clarification_text(&clarification)));
        }
    };
    walk.report.push(reading_text(&intent));
    // A move back to a place the party has been to names it by id, for the runner to branch on.
    if let Some(place) = &back_to {
        walk.report.push(format!("back_to={place}"));
    }
    walk.advance(&format!(
        "kind={} who={} to={} back_to={}",
        intent.kind.key(),
        intent.who.join(","),
        intent.to,
        back_to.as_deref().unwrap_or("-")
    ))?;
    if stop == Some(TransitionStage::Interpreting) {
        return Ok(walk.finished());
    }
    // A move back to a place the party has been to goes back to it as it was left: no design and
    // no plate are spent on it.
    if let Some(place) = back_to {
        let request = BackRequest { place };
        let kept: crate::world::KeptPlace =
            match post_json::<_, WorldReply<crate::world::KeptPlace>>(
                port,
                move_path::MOVE_BACK,
                &request,
            )
            .await
            {
                Ok(DevReply::Done { value }) => value,
                Ok(DevReply::Rejected { error, message }) => {
                    return Err(walk.failed(error.key(), &message));
                }
                Err(error) => return Err(walk.failed("route_unreachable", &error.to_string())),
            };
        walk.report
            .push(format!("back at place={} ({})", kept.id, kept.name));
        return Ok(walk.finished());
    }

    let design_request = DesignRequest {
        intent: intent.clone(),
        party: party.clone(),
        context: context.map(str::to_owned),
        model: None,
        walk: options.walk.ask(options.plate.as_deref()),
        residents: options.residents.clone(),
        approach: None,
    };
    let design =
        match post_json::<_, DesignReply>(port, move_path::MOVE_DESIGN, &design_request).await {
            Ok(DevReply::Done { value }) => value,
            Ok(DevReply::Rejected { error, message }) => {
                return Err(walk.failed(error.key(), &message));
            }
            Err(error) => return Err(walk.failed("route_unreachable", &error.to_string())),
        };
    walk.report.push(design_text(&design));
    walk.advance(&format!(
        "place={} things={} placements={} residents={} left_behind={} stages={}",
        design.place,
        design.things.len(),
        design.placements.len(),
        design.residents.len(),
        design.left_behind.len(),
        design.stages.len()
    ))?;
    if stop == Some(TransitionStage::Designing) {
        return Ok(walk.finished());
    }

    let Briefed {
        characters,
        shown,
        brief,
        picture,
    } = match brief_design(port, &design, present).await {
        Ok(briefed) => briefed,
        Err((key, message)) => return Err(walk.failed(&key, &message)),
    };
    walk.report.push(brief_text(&brief));
    if let Some(words) = picture {
        walk.report.push(format!("picture: {words}"));
    }
    walk.advance(&format!(
        "prompt_chars={} references={}",
        brief.prompt.chars().count(),
        brief.references.len()
    ))?;
    if stop == Some(TransitionStage::Briefing) {
        return Ok(walk.finished());
    }
    let Some(name) = options.plate.as_deref() else {
        walk.report
            .push("Generating, locating and placing were not run: give plate=NAME.".to_owned());
        return Ok(walk.finished());
    };
    let (placed, sheets) = match plate_inputs(port, &design, &characters, shown).await {
        Ok((placed, sheets, lines)) => {
            walk.report.extend(lines);
            (placed, sheets)
        }
        Err((key, message)) => return Err(walk.failed(&key, &message)),
    };
    let painting = Painting {
        prompt: &brief.prompt,
        avoid: &brief.avoid,
        sheets: &sheets,
    };
    if design.stages.is_empty() {
        match deliver_plate(port, name, &painting, placed, None).await {
            Ok(summary) => walk.report.push(summary),
            Err((step, error)) => return Err(walk.failed(step.key(), &error.to_string())),
        }
        // deliver_plate runs the three stages in one call, so their records are written after it.
        walk.advance(&format!("plate={name} generated and stored"))?;
        walk.advance("characters located and named in the picture")?;
        walk.advance("backdrop swapped behind the scene")?;
    } else {
        let movers: Vec<BriefCharacter> = characters
            .iter()
            .filter(|character| {
                design
                    .placements
                    .iter()
                    .any(|placement| placement.character == character.id)
                    && !design
                        .residents
                        .iter()
                        .any(|resident| resident.id == character.id)
            })
            .cloned()
            .collect();
        let began_ms = walk.now_ms();
        let through = Through {
            port,
            plate: name,
            design: &design,
            movers: &movers,
            placed: &placed,
            arrival: painting,
            owner: said.as_ref().map(|said| said.speaker),
            options: &options.walk,
        };
        let walked = match walk_through(through).await {
            Ok(walked) => walked,
            Err(failure) => return Err(walk.failed(&failure.key, &failure.message)),
        };
        walk.report.extend(walked.lines);
        // The place was made, scanned and named while the sights were shown; it went up after the
        // last of them.
        let made_ms = began_ms + walked.made_ms.unwrap_or_default();
        let arrived_ms = began_ms + walked.arrived_ms.unwrap_or_default();
        walk.advance_at(made_ms, &format!("plate={name} generated and stored"))?;
        walk.advance_at(made_ms, "characters located and named in the picture")?;
        walk.advance_at(arrived_ms, "backdrop swapped behind the scene")?;
    }
    walk.report.push(arrive(port, name, &design).await);
    // A resident has a persona only once the plate has named them in the scene, made to fit the
    // person as painted.
    let painted = match &said {
        Some(said) if !design.residents.is_empty() => {
            painted_looks(port, said.speaker, name, &mut walk.report).await
        }
        _ => Vec::new(),
    };
    for resident in &design.residents {
        let line = resident_persona(port, resident, &design.place, &painted).await;
        walk.report.push(line);
    }
    Ok(walk.finished())
}

/// What a design is painted from: everyone in its picture as the brief knows them, the items the
/// picture shows with their owners, the brief, and what the plate shows in the prompt's words
/// without the style (a frame watch's `scene=`).
pub(super) struct Briefed {
    pub(super) characters: Vec<BriefCharacter>,
    pub(super) shown: Vec<(String, InventoryItem)>,
    pub(super) brief: PlateBrief,
    pub(super) picture: Option<String>,
}

/// The brief of `design` with `present` the party in it: what each mover carries that the picture
/// shows, read from what they hold now, and each seated hero drawn as their look studio's design
/// shows them, never by their job alone. A failure is its key and words.
pub(super) async fn brief_design(
    port: u16,
    design: &SceneDesign,
    present: &[PersonaSummary],
) -> Result<Briefed, (String, String)> {
    let shown = shown_items(port, design)
        .await
        .map_err(|error| ("inventory_unreadable".to_owned(), error.to_string()))?;
    let characters: Vec<BriefCharacter> = present
        .iter()
        .map(|summary| brief_character(summary, &shown))
        .chain(design.residents.iter().map(resident_character))
        .collect();
    let ids = characters
        .iter()
        .map(|character| character.id.clone())
        .collect();
    let looks = crate::persona::hero_looks_from(port, ids)
        .await
        .map_err(|error| ("looks_unreadable".to_owned(), error.to_string()))?;
    let looks = LookSheets::from_looks(looks)
        .map_err(|error| (error.key().to_owned(), error.to_string()))?;
    let brief = build_brief(design, &characters, &looks)
        .map_err(|error| (error.key().to_owned(), error.to_string()))?;
    let picture = scene_words(design, &characters, &looks).ok();
    Ok(Briefed {
        characters,
        shown,
        brief,
        picture,
    })
}

/// Who the plate of `design` names (each placement, as the scan is asked to name them) and the
/// sheets of the `shown` items its room for pictures takes after the people's, in the design's
/// order, with the report's lines of what was left out or asked for. A failure is its key and
/// words.
pub(super) async fn plate_inputs(
    port: u16,
    design: &SceneDesign,
    characters: &[BriefCharacter],
    shown: Vec<(String, InventoryItem)>,
) -> Result<(Vec<CharacterRequest>, Vec<Thing>, Vec<String>), (String, String)> {
    let placed: Vec<CharacterRequest> = design
        .placements
        .iter()
        .map(|placement| {
            let character = characters
                .iter()
                .find(|character| character.id == placement.character);
            CharacterRequest {
                name: placement.character.clone(),
                display_name: character.map(|character| character.display_name.clone()),
                description: character.map(|character| character.job.clone()),
                portrait: None,
            }
        })
        .collect();
    let mut lines = Vec::new();
    let limits = image_limits(port)
        .await
        .map_err(|error| ("image_limits".to_owned(), error.to_string()))?;
    let listed = shown.len();
    let shown = items_that_fit(&limits, placed.len(), false, shown);
    if shown.len() < listed {
        lines.push(format!(
            "items left out for want of room: {} of {listed} fit beside {} people",
            shown.len(),
            placed.len()
        ));
    }
    let made = inventory_sheets(port, &shown)
        .await
        .map_err(|(step, error)| (step.key().to_owned(), error.to_string()))?;
    for name in &made.requested {
        lines.push(format!(
            "sheet {name}: none yet, asked for (told in words this time)"
        ));
    }
    for refused in &made.not_requested {
        lines.push(format!("sheet {refused}: not asked for"));
    }
    Ok((placed, made.sheets, lines))
}

/// Each item `design` shows, with its owner, in the design's order, read from what they hold now.
/// An item its owner does not hold any more is left out.
async fn shown_items(
    port: u16,
    design: &SceneDesign,
) -> Result<Vec<(String, InventoryItem)>, DevError> {
    let mut inventories: Vec<InventoryView> = Vec::new();
    let mut shown = Vec::new();
    for item in &design.shown_items {
        if !inventories
            .iter()
            .any(|view| view.inventory.owner == item.owner)
        {
            let address = format!("{}?owner={}", inventory_path::INVENTORY_OWNER, item.owner);
            let view: InventoryView = dev_client::reply_value(
                get_json::<InventoryReply<InventoryView>>(port, &address).await?,
            )?;
            inventories.push(view);
        }
        let found = inventories
            .iter()
            .filter(|view| view.inventory.owner == item.owner)
            .flat_map(|view| view.inventory.held())
            .find(|held| held.id == item.item);
        if let Some(found) = found {
            shown.push((item.owner.clone(), found.clone()));
        }
    }
    Ok(shown)
}

/// Makes the place `design` describes, painted as the plate `plate`, the world's active place, and
/// the line that says so. A place that could not be kept is reported, not fatal: the plate is up.
pub(super) async fn arrive(port: u16, plate: &str, design: &SceneDesign) -> String {
    let request = ArriveRequest {
        place: Place {
            id: plate.to_ascii_lowercase(),
            name: design.place.clone(),
            description: format!(
                "Mood: {}. In sight: {}.",
                design.mood,
                design.elements.join("; ")
            ),
            stalls: Vec::new(),
            things: design.things.clone(),
        },
        feature: Some(design.feature.clone()),
        shops: design
            .residents
            .iter()
            .filter(|resident| !resident.wares.is_empty() || resident.coin.is_some())
            .map(|resident| ShopStock {
                owner: resident.id.clone(),
                wares: resident.wares.clone(),
                coin: resident.coin.clone(),
            })
            .collect(),
    };
    let set: Result<PlaceSet, DevError> = match post_json::<_, WorldReply<PlaceSet>>(
        port,
        world_path::WORLD_ARRIVE,
        &request,
    )
    .await
    {
        Ok(reply) => dev_client::reply_value(reply),
        Err(error) => Err(error),
    };
    match set {
        Ok(_) => format!(
            "place {}: the active place, {} thing(s) worth a close look",
            request.place.id,
            request.place.things.len()
        ),
        Err(error) => format!(
            "place {}: not made the active place ({error}); nothing there can be looked at \
             closely",
            request.place.id
        ),
    }
}

/// The file of every name a resident has been given, one a line, kept across runs and hosts (each
/// host's persona store is its own). Unset, only the stored personas' names are known.
pub const NAMES_GIVEN_VARIABLE: &str = "DF_NAMES_GIVEN";

/// The names already given: the stored personas' and the file's ([`NAMES_GIVEN_VARIABLE`]), each
/// once; what cannot be read adds none.
async fn used_names(port: u16) -> Vec<String> {
    let listed: Result<PersonaReply<Vec<PersonaSummary>>, DevError> =
        get_json(port, persona_path::PERSONA).await;
    let given = match crate::environment::path(NAMES_GIVEN_VARIABLE) {
        Some(file) => tokio::fs::read_to_string(file).await.unwrap_or_default(),
        None => String::new(),
    };
    let mut names: Vec<String> = listed
        .ok()
        .and_then(|reply| dev_client::reply_value(reply).ok())
        .unwrap_or_default()
        .into_iter()
        .map(|summary| summary.display_name)
        .chain(given.lines().map(str::to_owned))
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
        .collect();
    names.sort();
    names.dedup();
    names
}

/// Adds `name` to the names given ([`NAMES_GIVEN_VARIABLE`]); the line that says what came of it,
/// none when the file is not set.
async fn keep_name_given(name: &str) -> Option<String> {
    use tokio::io::AsyncWriteExt;
    let file = crate::environment::path(NAMES_GIVEN_VARIABLE)?;
    let name = name.trim().replace(['\n', '\r'], " ");
    let written = async {
        if let Some(parent) = file.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let mut names = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&file)
            .await?;
        names.write_all(format!("{name}\n").as_bytes()).await
    }
    .await;
    Some(match written {
        Ok(()) => format!("PERSONA name_given name={name}"),
        Err(error) => format!("PERSONA name_unkept name={name}: {error}"),
    })
}

/// The things of the active place as the world keeps them (ids, looks with their states, close-up
/// pictures); none when it cannot be read or no place is active.
pub(super) async fn things_here(port: u16) -> Option<Vec<df_content::world_bible::Thing>> {
    let shown: Result<WorldReply<crate::world::request::WorldView>, DevError> =
        get_json(port, world_path::WORLD).await;
    let view = dev_client::reply_value(shown.ok()?).ok()?;
    serde_json::from_value(view.facts.get("in_sight")?.clone()).ok()
}

/// The persona of `resident`, made through the persona generate route as `persona generate <id>`
/// does (a stored persona is kept), and the line that says so. A persona that could not be made is
/// reported, not fatal.
pub(super) async fn resident_persona(
    port: u16,
    resident: &Resident,
    place: &str,
    painted: &[(String, String)],
) -> String {
    // The person as the plate shows them wins over the design's words: their name, voice and
    // manner are made to fit the figure painted (host 46: a woman at the counter was "Aldric").
    let as_painted = painted
        .iter()
        .find(|(id, _)| *id == resident.id)
        .map(|(_, look)| look.as_str());
    let hint = match as_painted {
        Some(look) => format!(
            "{}, met at {place}. As painted: {look}. Their name, voice and manner fit the person \
             as painted; where these words do not fit the picture, the picture wins: {}, {}.",
            resident.job, resident.display_name, resident.look
        ),
        None => format!(
            "{}, {}, met at {place}. Look: {}.",
            resident.display_name, resident.job, resident.look
        ),
    };
    // Every seller was "Mara" (host 56): the names the stored people already have are never
    // given again.
    let used = used_names(port).await;
    let hint = if used.is_empty() {
        hint
    } else {
        format!(
            "{hint} Their name is one of their own, never one of these already used: {}.",
            used.join(", ")
        )
    };
    let matched = format!(
        "PERSONA matched_look id={} from={} names_taken={}",
        resident.id,
        if as_painted.is_some() {
            "plate"
        } else {
            "design"
        },
        used.len()
    );
    let request = GenerateRequest {
        character: resident.id.clone(),
        hint: Some(hint),
        guidance: None,
        model: None,
        overwrite: false,
        unplaced: false,
        names_taken: used,
        as_painted: as_painted.map(str::to_owned),
    };
    let made: Result<Generation, DevError> = match post_json::<_, PersonaReply<Generation>>(
        port,
        persona_path::PERSONA_GENERATE,
        &request,
    )
    .await
    {
        Ok(reply) => dev_client::reply_value(reply),
        Err(error) => Err(error),
    };
    let made_line = match made {
        Ok(Generation::Generated { persona, .. }) => {
            let made = format!(
                concat!(
                    "resident {}: persona made; `persona ask {0} \"...\"` and `persona converse` ",
                    "work with them now"
                ),
                resident.id
            );
            match keep_name_given(&persona.display_name).await {
                Some(kept) => format!("{made}\n{kept}"),
                None => made,
            }
        }
        Ok(Generation::KeptExisting { .. }) => format!(
            concat!(
                "resident {}: kept the stored persona; `persona ask {0} \"...\"` and ",
                "`persona converse` work with them"
            ),
            resident.id
        ),
        Err(error) => format!(
            concat!(
                "resident {}: no persona could be made ({}); run `persona generate {0}` ",
                "once the scene names them"
            ),
            resident.id, error
        ),
    };
    format!("{matched}\n{made_line}")
}

/// How each person the plate `plate` names looks as painted, by their id: what `viewer`'s own eyes
/// find on it (one vision read, `persona perceive mode=study`), each person's sight matched to the
/// region the scan named for them. Empty when either cannot be had; `report` gets one `PERSONA
/// painted plate= people=` line, or `unread=` and why.
pub(super) async fn painted_looks(
    port: u16,
    viewer: &str,
    plate: &str,
    report: &mut Vec<String>,
) -> Vec<(String, String)> {
    use crate::locate::request::{LocateReply, StoredLayout};
    use crate::persona::perceive::request::{Mode, PerceiveRequest, Perceived, Subject};
    let picture = format!("{plate}.png");
    let request = PerceiveRequest {
        who: viewer.to_owned(),
        mode: Mode::Study,
        picture: Some(picture.clone()),
        focus: None,
        recent: Vec::new(),
        scene: None,
        vision: true,
        model: None,
    };
    let perceived: Result<PersonaReply<Perceived>, DevError> = post_json(
        port,
        crate::persona::perceive::routes::path::PERSONA_PERCEIVE,
        &request,
    )
    .await;
    let perceived = match perceived.map(dev_client::reply_value) {
        Ok(Ok(perceived)) => perceived,
        Ok(Err(error)) | Err(error) => {
            report.push(format!(
                "PERSONA painted plate={plate} unread=perceive: {error}"
            ));
            return Vec::new();
        }
    };
    let shown: Result<LocateReply<StoredLayout>, DevError> = get_json(
        port,
        &format!(
            "{}?picture={picture}",
            crate::locate::routes::path::LOCATE_SHOW
        ),
    )
    .await;
    let shown = match shown.map(dev_client::reply_value) {
        Ok(Ok(shown)) => shown,
        Ok(Err(error)) | Err(error) => {
            report.push(format!(
                "PERSONA painted plate={plate} unread=layout: {error}"
            ));
            return Vec::new();
        }
    };
    let looks: Vec<(String, String)> = perceived
        .sees
        .iter()
        .filter(|seen| seen.subject == Subject::Person)
        .filter_map(|seen| {
            let region = seen.region.as_deref()?;
            let label = shown
                .layout
                .regions()
                .iter()
                .find(|named| named.id().as_str() == region)?
                .label()?;
            Some((label.to_owned(), seen.what.clone()))
        })
        .collect();
    report.push(format!(
        "PERSONA painted plate={plate} people={}",
        looks
            .iter()
            .map(|(id, _)| id.as_str())
            .collect::<Vec<_>>()
            .join(",")
    ));
    looks
}

/// A party member as the brief knows a character, with the names of the `shown` items they hold.
fn brief_character(summary: &PersonaSummary, shown: &[(String, InventoryItem)]) -> BriefCharacter {
    BriefCharacter {
        id: summary.id.clone(),
        display_name: summary.display_name.clone(),
        job: summary.job.clone(),
        portrait: None,
        carrying: shown
            .iter()
            .filter(|(owner, _)| *owner == summary.id)
            .map(|(_, item)| item.name.clone())
            .collect(),
    }
}

/// A resident as the brief knows a character: the job text carries the job and the visible look.
fn resident_character(resident: &Resident) -> BriefCharacter {
    BriefCharacter {
        id: resident.id.clone(),
        display_name: resident.display_name.clone(),
        job: format!("{}, {}", resident.job, resident.look),
        portrait: None,
        carrying: Vec::new(),
    }
}

fn reading_text(intent: &MoveIntent) -> String {
    format!(
        "reading: move kind={} who={} to={}",
        intent.kind.key(),
        intent.who.join(", "),
        intent.to
    )
}

pub(super) fn design_text(design: &SceneDesign) -> String {
    let mut lines = vec![
        format!("design: place={} mood={}", design.place, design.mood),
        format!("elements: {}", design.elements.join(", ")),
    ];
    if let Some(look) = &design.town_look {
        lines.push(format!("town: {look}"));
    }
    for (index, stage) in design.stages.iter().enumerate() {
        lines.push(format!(
            "stage {}/{} {} ({}): {}",
            index + 1,
            design.stages.len(),
            stage.id,
            stage.pose.key(),
            stage.sight
        ));
    }
    for thing in &design.things {
        lines.push(format!(
            "thing {} ({}): {}",
            thing.id, thing.name, thing.look
        ));
    }
    if !design.shown_items.is_empty() {
        let shown: Vec<String> = design
            .shown_items
            .iter()
            .map(|shown| format!("{}:{}", shown.owner, shown.item))
            .collect();
        lines.push(format!("items in sight: {}", shown.join(", ")));
    }
    for resident in &design.residents {
        lines.push(format!(
            "resident {} ({}): {}, {}",
            resident.id, resident.display_name, resident.job, resident.look
        ));
    }
    for given in &design.distances {
        lines.push(format!("distance {} {}", given.id, given.distance.key()));
    }
    for placement in &design.placements {
        lines.push(format!(
            "  {}: {} {}, {}",
            placement.character,
            placement.depth.key(),
            placement.side.key(),
            placement.doing
        ));
    }
    let left = if design.left_behind.is_empty() {
        "none".to_owned()
    } else {
        design.left_behind.join(", ")
    };
    lines.push(format!("left behind: {left}"));
    lines.join("\n")
}

fn brief_text(brief: &PlateBrief) -> String {
    let mut lines = vec![format!("prompt: {}", brief.prompt)];
    for card in &brief.references {
        let portrait = card.portrait.as_deref().unwrap_or("none");
        lines.push(format!(
            "reference {}: {} (portrait: {portrait})",
            card.character, card.look
        ));
    }
    lines.join("\n")
}
