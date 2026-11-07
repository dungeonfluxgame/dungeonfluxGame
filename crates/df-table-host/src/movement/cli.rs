//! The `move` verbs of the dev command line for a scene transition: `move say` runs the whole path
//! from a line to the brief, and `move interpret`, `move design` and `move brief` stop after one
//! stage. Every stage runs on its fake, so no provider is called, except the plate step: `plate=NAME`
//! makes the new backdrop through `deliver_plate`. `move act` has a character say a line through
//! `persona say` and runs that line as a move.
//!
//! The party is the shown scene's characters that have a stored persona, read from the host's cast
//! route exactly as `persona converse` reads them. Each stage either advances the transition with a
//! record or fails it with its typed key, and the old scene stays up.

use std::fmt;
use std::time::Instant;

use super::brief::{BriefCharacter, LookSheets, build_brief};
use super::design::{DesignContext, DesignTable, SceneDesigner, TableDesigner};
use super::interpret::{LabelTable, LineReader, ScriptedReader};
use super::transition::{Deadlines, Transition, format_trace};
use super::types::{
    Depth, Interpreted, MoveIntent, PartyMember, PlateBrief, SceneDesign, Side, TransitionStage,
};
use crate::cast::request::{CastList, CastReply};
use crate::cast::routes::path as cast_path;
use crate::command::{CommandError, bad_choice};
use crate::dev::DevError;
use crate::dev_client::{self, get_json, post_json};
use crate::imagegen::MAXIMUM_NAME_CHARS;
use crate::imagegen::model::plain_name;
use crate::locate::request::CharacterRequest;
use crate::persona::options::take_options;
use crate::persona::request::{PersonaReply, PersonaSummary, SayRequest, Spoken};
use crate::persona::routes::path as persona_path;
use crate::persona::deliver_plate;

/// Every `move` stage verb, for the unknown-verb refusal and the help.
pub const MOVEMENT_USAGE: &str = concat!(
    "move say [plate=NAME] <line...> | move act <character> [plate=NAME] <situation...> | ",
    "move interpret <line...> | move design <line...> | move brief <line...>",
);

/// A `move` command of the dev CLI: the line, and the last stage to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MoveCommand {
    /// Run every stage, make the plate when one is named, and print each record.
    Say { line: String, plate: Option<String> },
    /// Have `character` say a line about `situation` through `persona say`, then run that line as
    /// a move with them as the speaker; the plate is made when one is named.
    Act {
        character: String,
        situation: String,
        plate: Option<String>,
    },
    /// Read the line, then stop.
    Interpret(String),
    /// Read the line and design the scene, then stop.
    Design(String),
    /// Read the line, design the scene and make the brief, then stop.
    Brief(String),
}

/// Why a movement could not be carried through. `report` is everything printed before the failure
/// and the transition's records, ending in the failed record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MoveError {
    Failed { report: String, key: String },
}

impl fmt::Display for MoveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Failed { report, key } => {
                write!(formatter, "{report}\nthe transition failed: {key}")
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
            let (plate, line) = plate_and_text(rest)?;
            Ok(MoveCommand::Say { line, plate })
        }
        "act" => {
            let [character, rest @ ..] = rest else {
                return Err(CommandError::Usage(MOVEMENT_USAGE));
            };
            let (plate, situation) = plate_and_text(rest)?;
            Ok(MoveCommand::Act {
                character: character.clone(),
                situation,
                plate,
            })
        }
        "interpret" => Ok(MoveCommand::Interpret(line)),
        "design" => Ok(MoveCommand::Design(line)),
        "brief" => Ok(MoveCommand::Brief(line)),
        _ => Err(CommandError::Usage(MOVEMENT_USAGE)),
    }
}

/// The `plate=NAME` option before a move's text, and the text. The name is checked as
/// `persona converse plate=` checks it.
fn plate_and_text(words: &[String]) -> Result<(Option<String>, String), CommandError> {
    let (given, text) = take_options(words, &["plate"]);
    if text.is_empty() {
        return Err(CommandError::Usage(MOVEMENT_USAGE));
    }
    let plate = match given.last() {
        None => None,
        Some(&(key, value)) => {
            if !plain_name(value) {
                return Err(bad_choice(
                    key,
                    value,
                    format!(
                        "1 to {MAXIMUM_NAME_CHARS} letters, digits, '-' or '_', with no extension"
                    ),
                ));
            }
            Some(value.to_owned())
        }
    };
    Ok((plate, text.join(" ")))
}

/// Runs one `move` command against the host on `port` and gives its report.
pub async fn run(port: u16, command: MoveCommand) -> Result<String, DevError> {
    let cast_reply: CastReply<CastList> = get_json(port, cast_path::CAST).await?;
    let cast: CastList = dev_client::reply_value(cast_reply)?;
    let present = present_personas(&cast);
    let outcome = match command {
        MoveCommand::Say { line, plate } => {
            walk(port, &line, None, &present, None, plate.as_deref()).await
        }
        MoveCommand::Act {
            character,
            situation,
            plate,
        } => {
            let line = speak(port, &character, situation).await?;
            walk(port, &line, None, &present, Some(character.as_str()), plate.as_deref()).await
        }
        MoveCommand::Interpret(line) => {
            walk(port, &line, Some(TransitionStage::Interpreting), &present, None, None).await
        }
        MoveCommand::Design(line) => {
            walk(port, &line, Some(TransitionStage::Designing), &present, None, None).await
        }
        MoveCommand::Brief(line) => {
            walk(port, &line, Some(TransitionStage::Briefing), &present, None, None).await
        }
    };
    outcome.map_err(DevError::Move)
}

/// The line `character` says about `situation`, worded by `persona say` and not voiced.
async fn speak(port: u16, character: &str, situation: String) -> Result<String, DevError> {
    let request = SayRequest {
        character: character.to_owned(),
        situation,
        model: None,
        speak: false,
    };
    let reply: PersonaReply<Spoken> =
        post_json(port, persona_path::PERSONA_SAY, &request).await?;
    let spoken = dev_client::reply_value(reply)?;
    Ok(spoken.said.line)
}

/// Whether `line` speaks in the first person: "I", "me", "we", "us", "my" or "I'll" as a word.
fn speaks_first_person(line: &str) -> bool {
    line.to_lowercase()
        .split(|character: char| !(character.is_alphanumeric() || character == '\''))
        .any(|word| matches!(word, "i" | "me" | "we" | "us" | "my" | "i'll"))
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

    /// Fails the transition and returns the report that ends in its failed record.
    fn failed(&mut self, key: &str, message: &str) -> MoveError {
        let now = self.now_ms();
        self.transition.fail(now, key, message);
        self.report.push(format_trace(self.transition.records()));
        MoveError::Failed {
            report: self.report.join("\n"),
            key: key.to_owned(),
        }
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

/// The stages in order, stopping after `stop`; `None` runs the whole path (`move say`, `move act`).
/// `speaker` is the character who said `line` in `move act`: the line is printed first, and a
/// first-person line adds them to the movers. `plate` makes the plate and puts it on the scene.
async fn walk(
    port: u16,
    line: &str,
    stop: Option<TransitionStage>,
    present: &[PersonaSummary],
    speaker: Option<&str>,
    plate: Option<&str>,
) -> Result<String, MoveError> {
    let party: Vec<PartyMember> = present
        .iter()
        .map(|summary| PartyMember {
            id: summary.id.clone(),
            display_name: summary.display_name.clone(),
        })
        .collect();
    let mut walk = Walk::start(line);
    if let Some(speaker) = speaker {
        walk.report.push(format!("{speaker} says: {line}"));
    }

    let mut intent = match ScriptedReader::new(LabelTable::builtin()).read(line, &party) {
        Ok(Interpreted::Move { intent }) => intent,
        Ok(Interpreted::NotAMove) => {
            return Ok(walk.unmoved("not_a_move", "the line is not a movement"));
        }
        Ok(Interpreted::NeedsClarification) => {
            return Ok(walk.unmoved("needs_clarification", "the movement names no destination"));
        }
        Err(error) => return Err(walk.failed(error.key(), &error.to_string())),
    };
    if let Some(speaker) = speaker {
        let speaks_in_party = party.iter().any(|member| member.id == speaker);
        if speaks_in_party
            && speaks_first_person(line)
            && !intent.who.iter().any(|id| id == speaker)
        {
            intent.who.push(speaker.to_owned());
        }
    }
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

    let design = match TableDesigner::new(DesignTable::builtin()).design(
        &intent,
        &party,
        &DesignContext::default(),
    ) {
        Ok(design) => design,
        Err(error) => return Err(walk.failed(error.key(), &error.to_string())),
    };
    walk.report.push(design_text(&design));
    walk.advance(&format!(
        "place={} placements={} left_behind={}",
        design.place,
        design.placements.len(),
        design.left_behind.len()
    ))?;
    if stop == Some(TransitionStage::Designing) {
        return Ok(walk.finished());
    }

    let characters: Vec<BriefCharacter> = present.iter().map(brief_character).collect();
    let brief = match build_brief(&design, &characters, &LookSheets::empty()) {
        Ok(brief) => brief,
        Err(error) => return Err(walk.failed(error.key(), &error.to_string())),
    };
    walk.report.push(brief_text(&brief));
    walk.advance(&format!(
        "prompt_chars={} references={}",
        brief.prompt.chars().count(),
        brief.references.len()
    ))?;
    if stop == Some(TransitionStage::Briefing) {
        return Ok(walk.finished());
    }
    let Some(name) = plate else {
        walk.report
            .push("Generating, locating and placing were not run: give plate=NAME.".to_owned());
        return Ok(walk.finished());
    };
    walk.advance(&format!("plate={name}"))?;
    let placed: Vec<CharacterRequest> = design
        .placements
        .iter()
        .map(|placement| CharacterRequest {
            name: placement.character.clone(),
            description: present
                .iter()
                .find(|summary| summary.id == placement.character)
                .map(|summary| summary.job.clone()),
            portrait: None,
        })
        .collect();
    match deliver_plate(port, name, &brief.prompt, placed).await {
        Ok(summary) => walk.report.push(summary),
        Err((step, error)) => return Err(walk.failed(step.key(), &error.to_string())),
    }
    // deliver_plate runs the three stages in one call, so their records are written after it.
    walk.advance("located and named in the picture")?;
    walk.advance("backdrop swapped behind the scene")?;
    walk.advance("the new plate is on the scene")?;
    Ok(walk.finished())
}

fn brief_character(summary: &PersonaSummary) -> BriefCharacter {
    BriefCharacter {
        id: summary.id.clone(),
        display_name: summary.display_name.clone(),
        job: summary.job.clone(),
        portrait: None,
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
