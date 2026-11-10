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
    DesignReply, DesignRequest, InterpretReply, InterpretRequest, path as move_path,
};
use super::transition::{Deadlines, Transition, format_trace};
use super::types::{
    Clarification, Declared, Depth, Interpreted, MoveIntent, PartyMember, PlateBrief, Resident,
    SceneDesign, Side, TownRole, TransitionStage, Unclear,
};
use super::walk::{Impulse, MINIMUM_HOLD, Through, WalkOptions, walk_through};
use crate::cast::request::{CastList, CastReply};
use crate::cast::routes::path as cast_path;
use crate::command::{CommandError, bad_choice};
use crate::dev::DevError;
use crate::dev_client::{self, get_json, post_json};
use crate::dev_http::DevReply;
use crate::imagegen::MAXIMUM_NAME_CHARS;
use crate::imagegen::model::plain_name;
use crate::locate::request::CharacterRequest;
use crate::persona::options::take_options;
use crate::persona::request::{
    GenerateRequest, Generation, PersonaReply, PersonaSummary, SaidLine, SayRequest, Spoken,
};
use crate::persona::routes::path as persona_path;
use crate::persona::{
    Painting, deliver_plate, image_limits, inventory_sheets, items_that_fit, sheets_for_gains,
};
use crate::world::InventoryItem;
use crate::world::inventory::request::{InventoryReply, InventoryView};
use crate::world::inventory::routes::path as inventory_path;
use crate::world::request::{ArriveRequest, PlaceSet, WorldReply};
use crate::world::routes::path as world_path;
use df_content::world_bible::Place;

/// Every `move` stage verb, for the unknown-verb refusal and the help.
pub const MOVEMENT_USAGE: &str = concat!(
    "move say [options] <line...> | move act <character> [options] <situation...> | ",
    "move interpret <line...> | move design <line...> | move brief <line...>; options: ",
    "plate=NAME walk=<sights> town=<seeds|within|leaves> hold=<seconds> impulse=<kind>:<strength>",
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
    let (given, text) = take_options(words, &["plate", "walk", "town", "hold", "impulse"]);
    if text.is_empty() {
        return Err(CommandError::Usage(MOVEMENT_USAGE));
    }
    let mut options = MoveOptions::default();
    for &(key, value) in &given {
        match key {
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
    let present = present_personas(&cast);
    let outcome = match command {
        MoveCommand::Say { line, options } => {
            walk(port, &line, None, &present, None, &options).await
        }
        MoveCommand::Act {
            character,
            situation,
            options,
        } => {
            let said = speak(port, &character, situation.clone()).await?;
            // What the line used up or gained of what the speaker holds, as the host took it.
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
            let spoken = Said {
                speaker: &character,
                situation: &situation,
            };
            walk(port, &said.line, None, &present, Some(spoken), &options)
                .await
                .map(|report| {
                    [report]
                        .into_iter()
                        .chain(inventory)
                        .collect::<Vec<_>>()
                        .join("\n")
                })
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
async fn speak(port: u16, character: &str, situation: String) -> Result<SaidLine, DevError> {
    let request = SayRequest {
        character: character.to_owned(),
        situation,
        model: None,
        speak: false,
        recent: Vec::new(),
        asked_by: None,
        intent: None,
        strike: false,
    };
    let reply: PersonaReply<Spoken> = post_json(port, persona_path::PERSONA_SAY, &request).await?;
    let spoken = dev_client::reply_value(reply)?;
    Ok(spoken.said)
}

/// The shown scene's characters that have a stored persona, in the scene's order.
fn present_personas(cast: &CastList) -> Vec<PersonaSummary> {
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
    if let Some(said) = &said {
        walk.report.push(format!("{} says: {line}", said.speaker));
    }
    let context = said.as_ref().map(|said| said.situation);
    let declared = if said.is_some() {
        Declared::Move
    } else {
        Declared::Unknown
    };

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
    // One more reading, never a loop: Jev found no destination in a line said toward going
    // somewhere, so it is shown its own reason and asked again.
    if let Some(refused) = retry_after(&reading, declared).cloned() {
        walk.retry(&refused);
        request.refused_before = Some(refused);
        reading = walk.read(port, &request).await?;
    }
    let intent = match reading {
        Interpreted::Move { intent } => intent,
        Interpreted::NotAMove => {
            return Ok(walk.unmoved("not_a_move", "the line is not a movement"));
        }
        Interpreted::NeedsClarification(clarification) => {
            return Ok(walk.unmoved("needs_clarification", &clarification_text(&clarification)));
        }
    };
    walk.report.push(reading_text(&intent));
    walk.advance(&format!(
        "kind={} who={} to={}",
        intent.kind.key(),
        intent.who.join(","),
        intent.to
    ))?;
    if stop == Some(TransitionStage::Interpreting) {
        return Ok(walk.finished());
    }

    let design_request = DesignRequest {
        intent: intent.clone(),
        party: party.clone(),
        context: context.map(str::to_owned),
        model: None,
        walk: options.walk.ask(options.plate.as_deref()),
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

    // What each mover carries that the picture shows, in the design's order, read from what they
    // hold now: the brief says where it is, and the plate attaches its sheets after the people's.
    let shown = match shown_items(port, &design).await {
        Ok(shown) => shown,
        Err(error) => return Err(walk.failed("inventory_unreadable", &error.to_string())),
    };
    let characters: Vec<BriefCharacter> = present
        .iter()
        .map(|summary| brief_character(summary, &shown))
        .chain(design.residents.iter().map(resident_character))
        .collect();
    let brief = match build_brief(&design, &characters, &LookSheets::empty()) {
        Ok(brief) => brief,
        Err(error) => return Err(walk.failed(error.key(), &error.to_string())),
    };
    walk.report.push(brief_text(&brief));
    // What the plate shows, in the prompt's words without the style: a frame watch's `scene=`.
    if let Ok(words) = scene_words(&design, &characters, &LookSheets::empty()) {
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
    // The sheets of what the movers hold that the picture shows: the plate's room for pictures
    // takes them after the people's sheets, in the design's order, and the rest are left out.
    let shown = match image_limits(port).await {
        Ok(limits) => {
            let listed = shown.len();
            let fitting = items_that_fit(&limits, placed.len(), false, shown);
            if fitting.len() < listed {
                walk.report.push(format!(
                    "items left out for want of room: {} of {listed} fit beside {} people",
                    fitting.len(),
                    placed.len()
                ));
            }
            fitting
        }
        Err(error) => return Err(walk.failed("image_limits", &error.to_string())),
    };
    let sheets = match inventory_sheets(port, &shown).await {
        Ok(made) => {
            for name in &made.requested {
                walk.report.push(format!(
                    "sheet {name}: none yet, asked for (told in words this time)"
                ));
            }
            for refused in &made.not_requested {
                walk.report.push(format!("sheet {refused}: not asked for"));
            }
            made.sheets
        }
        Err((step, error)) => return Err(walk.failed(step.key(), &error.to_string())),
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
    // A resident has a persona only once the plate has named them in the scene.
    for resident in &design.residents {
        let line = resident_persona(port, resident, &design.place).await;
        walk.report.push(line);
    }
    Ok(walk.finished())
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
async fn arrive(port: u16, plate: &str, design: &SceneDesign) -> String {
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

/// The persona of `resident`, made through the persona generate route as `persona generate <id>`
/// does (a stored persona is kept), and the line that says so. A persona that could not be made is
/// reported, not fatal.
async fn resident_persona(port: u16, resident: &Resident, place: &str) -> String {
    let request = GenerateRequest {
        character: resident.id.clone(),
        hint: Some(format!(
            "{}, {}, met at {place}. Look: {}.",
            resident.display_name, resident.job, resident.look
        )),
        guidance: None,
        model: None,
        overwrite: false,
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
    match made {
        Ok(Generation::Generated { .. }) => format!(
            concat!(
                "resident {}: persona made; `persona ask {0} \"...\"` and `persona converse` ",
                "work with them now"
            ),
            resident.id
        ),
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
    }
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

fn design_text(design: &SceneDesign) -> String {
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
    for placement in &design.placements {
        lines.push(format!(
            "  {}: {} {}, {}",
            placement.character,
            depth_word(placement.depth),
            side_word(placement.side),
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

fn depth_word(depth: Depth) -> &'static str {
    match depth {
        Depth::Foreground => "foreground",
        Depth::Middle => "middle",
        Depth::Background => "background",
    }
}

fn side_word(side: Side) -> &'static str {
    match side {
        Side::Left => "left",
        Side::Centre => "centre",
        Side::Right => "right",
    }
}
