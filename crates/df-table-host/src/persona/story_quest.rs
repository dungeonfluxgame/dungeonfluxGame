//! The quest's judgements at the table: which offered quest the party's words take up, and how a
//! line at the sealed door stands against its riddle. Both are Jev's: one call each against a
//! closed schema (`line_judges::judge_quest_pick`, `line_judges::judge_riddle`), the answer
//! checked in code against the offered quest ids, or the riddle's answer id and clue ids. Nothing
//! here reads the words.
//!
//! A riddle is written in the scenario (`@riddle`): its id, the door's words, the id of its one
//! answer and what that answer is, and the ids of the clues that lead to it. The clues are the
//! stored story's evidence, gathered at the stops by the clue route (`story_clues`), so each id is
//! checked against the story and the judge is told what each clue shows and whether the party has
//! found it. The hints of a stuck party stay the clue route's (`stalled`) and the narrator's: this
//! route never words one.
//!
//! Routes: `POST /dev/persona/quest/pick` ([`QuestPickRequest`]), `POST /dev/persona/riddle/ask`
//! and `POST /dev/persona/riddle/judge` ([`RiddleRequest`]). Audit lines: `PERSONA quest_judged
//! ..` (the judge's), `RIDDLE asked id= clues= found=`, `RIDDLE judged id= speaker= verdict=
//! clues_used= ms=`, `RIDDLE solved id= by=` and `PERSONA refused verb=story error=<key>`.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use axum::response::Response;
use df_content::world_bible::is_valid_id;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::judge_route::judge_route;
use super::line_judges::{RiddleAsk, RiddleJudged, RiddleVerdict, judge_quest_pick, judge_riddle};
use super::request::PersonaError;
use super::story::{Story, StoryRouteError, StoryRoutes, malformed, refused, stored_story};
use crate::blocking::{run_blocking, run_owned};
use crate::dev::DevError;
use crate::dev_client::{post_json, reply_value};
use crate::dev_http::{DevReply, parse_body, reply};
use crate::world::Quest;

/// Which offered quest a line takes up: POST a [`QuestPickRequest`].
pub(super) const PERSONA_QUEST_PICK: &str = "/dev/persona/quest/pick";
/// The riddle put to the party: POST a [`RiddleRequest`] with no line.
pub(super) const PERSONA_RIDDLE_ASK: &str = "/dev/persona/riddle/ask";
/// A line at the door judged against the riddle: POST a [`RiddleRequest`] with its line.
pub(super) const PERSONA_RIDDLE_JUDGE: &str = "/dev/persona/riddle/judge";

/// The body of `POST /dev/persona/quest/pick`: who said the line, the line and the talk before it.
/// The quests are the world state's offered ones, never the request's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestPickRequest {
    pub speaker: String,
    pub line: String,
    #[serde(default)]
    pub recent: Vec<String>,
    #[serde(default)]
    pub model: Option<String>,
}

/// What the quest route answers: the offered quest the line takes up, or none, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestPicked {
    pub quest: Option<String>,
    pub because: String,
}

/// A riddle as the scenario writes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Riddle {
    pub id: String,
    /// The riddle as the door puts it.
    pub door: String,
    /// The id of its one answer.
    pub answer_id: String,
    /// What the answer is, in words, for the judge.
    pub answer: String,
    /// The ids of the clues that lead to it: pieces of the stored story's evidence.
    pub clues: Vec<String>,
}

/// The body of the riddle routes: the riddle, and for a judgement who said the line, the line and
/// the talk before it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RiddleRequest {
    pub riddle: Riddle,
    #[serde(default)]
    pub speaker: Option<String>,
    #[serde(default)]
    pub line: Option<String>,
    #[serde(default)]
    pub recent: Vec<String>,
    #[serde(default)]
    pub model: Option<String>,
}

/// What the riddle routes answer: the riddle's clues the party has found, and for a judgement
/// the verdict (none when the judge gave no answer that stood).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RiddleReply {
    pub found: Vec<String>,
    #[serde(default)]
    pub judged: Option<RiddleJudged>,
}

/// The answer of the quest route.
pub type QuestPickReply = DevReply<QuestPicked, StoryRouteError>;
/// The answer of the riddle routes.
pub type RiddleRouteReply = DevReply<RiddleReply, StoryRouteError>;

fn bad(reason: String) -> StoryRouteError {
    StoryRouteError::BadRequest {
        cause: malformed(reason),
    }
}

/// A persona judge's failure as the story route's: the text service's own causes kept, anything
/// else a request that could not be used.
fn from_persona(error: PersonaError) -> StoryRouteError {
    match error {
        PersonaError::TextService { cause, reason } => {
            StoryRouteError::TextService { cause, reason }
        }
        PersonaError::TextServiceMissing { missing } => {
            StoryRouteError::TextServiceMissing { missing }
        }
        PersonaError::InvalidOperationKey => StoryRouteError::InvalidOperationKey,
        other => bad(other.to_string()),
    }
}

/// Checks `riddle` against `story`: ids in the grammar, words that say something, and clues that
/// are the story's evidence, each once. Answers the clues as the judge is told them.
fn riddle_clues(riddle: &Riddle, story: &Story) -> Result<Vec<Value>, StoryRouteError> {
    if !is_valid_id(&riddle.id) || !is_valid_id(&riddle.answer_id) {
        return Err(bad(format!(
            "the riddle '{}' or its answer '{}' is not an id",
            riddle.id, riddle.answer_id
        )));
    }
    if riddle.door.trim().is_empty() || riddle.answer.trim().is_empty() || riddle.clues.is_empty() {
        return Err(bad(format!(
            "the riddle '{}' needs the door's words, its answer and its clues",
            riddle.id
        )));
    }
    riddle
        .clues
        .iter()
        .enumerate()
        .map(|(at, id)| {
            let piece = story
                .evidence
                .iter()
                .find(|piece| &piece.id == id)
                .filter(|_| !riddle.clues[..at].contains(id))
                .ok_or_else(|| {
                    bad(format!(
                        "the riddle's clue '{id}' is not a piece of the story's evidence, once"
                    ))
                })?;
            Ok(json!({
                "id": piece.id,
                "what": piece.what,
                "up_close": piece.up_close,
                "meaning": piece.flaw,
                "found": story.findings.is_seen(&piece.id),
            }))
        })
        .collect()
}

/// The ids of the riddle's clues the party has found.
fn found_of(clues: &[Value]) -> Vec<String> {
    clues
        .iter()
        .filter(|clue| clue["found"].as_bool() == Some(true))
        .filter_map(|clue| clue["id"].as_str().map(str::to_owned))
        .collect()
}

// TODO(DEV-12): route tests on a fake text service: no quest offered, a pick, a riddle clue that
// is no evidence, the found clues, and the RIDDLE and PERSONA audit lines.
async fn pick(
    routes: &StoryRoutes,
    request: &QuestPickRequest,
) -> Result<QuestPicked, StoryRouteError> {
    if request.line.trim().is_empty() {
        return Err(bad("the line is empty".to_owned()));
    }
    let app = Arc::clone(&routes.app);
    let offered: Vec<Quest> = run_blocking(
        move || {
            // The board's quests, those the party took on an earlier day among them: taking one
            // again is theirs already (the world pays no advance twice).
            Ok(crate::world::quests_of(&app)
                .into_iter()
                .map(|record| record.quest)
                .collect())
        },
        StoryRouteError::Stopped,
    )
    .await?;
    if offered.is_empty() {
        return Err(bad(
            "no quest is on the board (world quest offer)".to_owned()
        ));
    }
    let route = judge_route(&routes.text, request.model.as_deref())?;
    let pick = judge_quest_pick(
        &routes.app,
        &route,
        (&request.speaker, &request.line, &request.recent),
        &offered,
    )
    .await
    .map_err(from_persona)?;
    Ok(QuestPicked {
        quest: pick.quest,
        because: pick.because,
    })
}

async fn riddle(
    routes: &StoryRoutes,
    request: &RiddleRequest,
    judging: bool,
) -> Result<RiddleReply, StoryRouteError> {
    let story = stored_story(routes).await?;
    let clues = riddle_clues(&request.riddle, &story)?;
    let found = found_of(&clues);
    let riddle = &request.riddle;
    if !judging {
        routes.app.record_line(format!(
            "RIDDLE asked id={} clues={} found={}",
            riddle.id,
            riddle.clues.join(","),
            found.join(",")
        ));
        return Ok(RiddleReply {
            found,
            judged: None,
        });
    }
    let (Some(speaker), Some(line)) = (request.speaker.as_deref(), request.line.as_deref()) else {
        return Err(bad("a judgement needs the speaker and the line".to_owned()));
    };
    if line.trim().is_empty() {
        return Err(bad("the line is empty".to_owned()));
    }
    let route = judge_route(&routes.text, request.model.as_deref())?;
    let ask = RiddleAsk {
        riddle: &riddle.id,
        door: &riddle.door,
        answer_id: &riddle.answer_id,
        answer: &riddle.answer,
        clues,
        speaker,
        line,
        recent: &request.recent,
    };
    let (judged, ms) = judge_riddle(&routes.app, &route, &ask)
        .await
        .map_err(from_persona)?;
    routes.app.record_line(format!(
        "RIDDLE judged id={} speaker={speaker} verdict={} clues_used={} ms={ms}",
        riddle.id,
        judged
            .as_ref()
            .map_or("unjudged", |judged| judged.verdict.key()),
        judged
            .as_ref()
            .map(|judged| judged.clue_ids_used.join(","))
            .unwrap_or_default()
    ));
    if judged
        .as_ref()
        .is_some_and(|judged| judged.verdict == RiddleVerdict::Solved)
    {
        routes
            .app
            .record_line(format!("RIDDLE solved id={} by={speaker}", riddle.id));
    }
    Ok(RiddleReply { found, judged })
}

pub(super) async fn post_quest_pick(
    State(routes): State<Arc<StoryRoutes>>,
    bytes: Bytes,
) -> Response {
    let result = match parse_body::<QuestPickRequest>(&bytes) {
        Ok(request) => {
            let owned = Arc::clone(&routes);
            run_owned(
                async move { pick(&owned, &request).await },
                StoryRouteError::Stopped,
            )
            .await
        }
        Err(error) => Err(StoryRouteError::InvalidBody(error)),
    };
    refused(&routes.app, &result);
    reply(result)
}

async fn post_riddle(routes: Arc<StoryRoutes>, bytes: Bytes, judging: bool) -> Response {
    let result = match parse_body::<RiddleRequest>(&bytes) {
        Ok(request) => {
            let owned = Arc::clone(&routes);
            run_owned(
                async move { riddle(&owned, &request, judging).await },
                StoryRouteError::Stopped,
            )
            .await
        }
        Err(error) => Err(StoryRouteError::InvalidBody(error)),
    };
    refused(&routes.app, &result);
    reply(result)
}

pub(super) async fn post_riddle_ask(
    State(routes): State<Arc<StoryRoutes>>,
    bytes: Bytes,
) -> Response {
    post_riddle(routes, bytes, false).await
}

pub(super) async fn post_riddle_judge(
    State(routes): State<Arc<StoryRoutes>>,
    bytes: Bytes,
) -> Response {
    post_riddle(routes, bytes, true).await
}

// --- The dev CLI's side: `persona quest pick` and `persona riddle ask|judge`.

/// `persona quest pick <speaker> [recent=..].. <line>`: `quest=<id|none>` and `because=<why>`.
pub(super) async fn quest_pick(port: u16, request: QuestPickRequest) -> Result<String, DevError> {
    let reply: QuestPickReply = post_json(port, PERSONA_QUEST_PICK, &request).await?;
    let picked = reply_value(reply)?;
    Ok(format!(
        "quest={}\nbecause={}",
        picked.quest.as_deref().unwrap_or("none"),
        picked.because
    ))
}

/// `persona riddle ask|judge ..`: `found=<ids>`, then for a judgement `verdict=<solved|close|
/// wrong|unjudged>`, `clues_used=<ids>` and `because=<why>`.
pub(super) async fn riddle_line(
    port: u16,
    request: RiddleRequest,
    judging: bool,
) -> Result<String, DevError> {
    let path = if judging {
        PERSONA_RIDDLE_JUDGE
    } else {
        PERSONA_RIDDLE_ASK
    };
    let reply: RiddleRouteReply = post_json(port, path, &request).await?;
    let answered = reply_value(reply)?;
    let mut lines = vec![format!("found={}", answered.found.join(","))];
    if judging {
        match &answered.judged {
            Some(judged) => {
                lines.push(format!("verdict={}", judged.verdict.key()));
                lines.push(format!("clues_used={}", judged.clue_ids_used.join(",")));
                lines.push(format!("because={}", judged.because));
            }
            None => lines.push("verdict=unjudged".to_owned()),
        }
    }
    Ok(lines.join("\n"))
}
