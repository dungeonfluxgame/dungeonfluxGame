//! The quests of the world state: each authored in the scenario (an id, a title, its giver, the
//! reward and the advance in gold, the site and the ids of its clues), offered on a board and
//! taken by the party. The record lives in `world/state.json` beside the active place
//! (`store`), written whole or not at all.
//!
//! Taking a quest (`accept`) marks it accepted by the heroes, pays each hero the advance through
//! the inventory's own coin gain (`gain_coins`, gold pieces into the hero's purse, a new gold item
//! when it holds none; never an edit of the file),
//! keeps the quest in each hero's notebook (a `note` memory, the phone's Notes) and puts its title
//! on the shared screen as the story's mark (`SceneChange::Story`). `progress` moves the mark's
//! line to the stop the party reached.
//!
//! Which quest the party's words take is not decided here: Jev picks among the offered ids
//! (`persona quest pick`, `persona::line_judges::judge_quest_pick`), and the runner then accepts
//! the id it named.
//!
//! A Start takes every quest off the board (`clear_quests`): a fresh journey's quests start
//! unoffered.
//!
//! Audit lines: `QUEST completed id= by= reward_gp= each_gp= remainder_to=`, `QUEST offered id=
//! reward_gp= advance_gp= clues=`, `QUEST accepted id= by=`, `QUEST cleared count=`,
//! `QUEST paid id= hero= gp= item= count=`, `QUEST pay_refused id= hero= reason=`, `QUEST noted
//! id= hero= memory=`, `QUEST shown id= at=` (or `QUEST shown_refused status=`) and `WORLD refused
//! verb=quest_* error=<key>`.

use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::response::Response;
use axum::routing::{get, post};
use df_content::world_bible::is_valid_id;
use df_playground::overlay::{Progress, StoryMark};
use df_playground::pages::wire::KeptKind;
use df_playground::scene::SceneChange;
use df_rules::coins::Denomination;
use serde::{Deserialize, Serialize};

use super::memory::{MemoryDraft, keep_next};
use super::request::WorldError;
use super::routes::path;
use super::store::{read_quests, world_directory, write_quests};
use crate::blocking::run_blocking;
use crate::dev_http::{parse_body, reply};
use crate::state::AppState;

/// Who keeps a quest's note in a hero's notebook, as the memory records it.
const NOTED_BY: &str = "host";

/// One quest as the scenario authors it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Quest {
    pub id: String,
    pub title: String,
    /// Who offers it, as the table knows them.
    pub giver: String,
    /// The gold paid when it is done.
    pub reward_gp: u64,
    /// The gold paid to each hero when it is taken, part of the reward.
    pub advance_gp: u64,
    /// Where it leads, in words.
    pub site: String,
    /// The ids of the clues that lead to it (the story's evidence).
    #[serde(default)]
    pub clues: Vec<String>,
}

/// Where a quest stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuestStanding {
    /// On the board, not taken.
    Offered,
    /// Taken by the party.
    Accepted,
    /// Done: the reward was paid to the heroes who took it.
    Completed,
}

impl QuestStanding {
    pub fn key(self) -> &'static str {
        match self {
            Self::Offered => "offered",
            Self::Accepted => "accepted",
            Self::Completed => "completed",
        }
    }
}

/// A quest of the world state and where it stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestRecord {
    pub quest: Quest,
    pub standing: QuestStanding,
    /// The heroes who took it, in seat order; none while it is offered.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accepted_by: Vec<String>,
}

/// `POST path::WORLD_QUEST_OFFER`: the quests a board offers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestOffer {
    pub quests: Vec<Quest>,
}

/// `POST path::WORLD_QUEST_ACCEPT`: the quest the party takes and its heroes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestAccept {
    pub quest: String,
    pub heroes: Vec<String>,
}

/// `POST path::WORLD_QUEST_PROGRESS`: the stop the party reached, out of the quest's stops.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestProgress {
    pub index: u16,
    pub total: u16,
    pub place: String,
}

/// `POST path::WORLD_QUEST_COMPLETE`: the quest whose work is done (the tomb is open).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestComplete {
    pub quest: String,
}

/// What the quest routes answer: the quests of the world state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestsView {
    pub quests: Vec<QuestRecord>,
}

/// What one hero was paid in gold of a quest (its advance, or their share of its reward), or why
/// not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum GoldPaid {
    Paid {
        hero: String,
        item: String,
        count: u64,
    },
    Refused {
        hero: String,
        reason: String,
    },
}

/// What `POST path::WORLD_QUEST_ACCEPT` answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestAccepted {
    pub quest: Quest,
    /// The heroes who had taken it before: nothing is paid or noted for them again.
    #[serde(default)]
    pub already: Vec<String>,
    pub paid: Vec<GoldPaid>,
    /// The note kept in each hero's notebook, `(hero, memory id)`.
    pub noted: Vec<(String, String)>,
}

/// What `POST path::WORLD_QUEST_COMPLETE` answers: the reward paid to each hero who took the quest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestCompleted {
    pub quest: Quest,
    pub paid: Vec<GoldPaid>,
    /// The note of the finished quest kept in each hero's notebook, `(hero, memory id)`.
    pub noted: Vec<(String, String)>,
}

fn invalid(reason: impl Into<String>) -> WorldError {
    WorldError::InvalidQuest {
        reason: reason.into(),
    }
}

/// Checks a quest as authored: ids in the grammar, words that say something, an advance no
/// larger than the reward, and its clue ids each once.
pub fn check_quest(quest: &Quest) -> Result<(), WorldError> {
    if !is_valid_id(&quest.id) {
        return Err(invalid(format!("'{}' is not an id", quest.id)));
    }
    for (field, text) in [
        ("title", &quest.title),
        ("giver", &quest.giver),
        ("site", &quest.site),
    ] {
        if text.trim().is_empty() {
            return Err(invalid(format!("{} has no {field}", quest.id)));
        }
    }
    if quest.reward_gp == 0 || quest.advance_gp > quest.reward_gp {
        return Err(invalid(format!(
            "{} pays {} gp in advance of a {} gp reward",
            quest.id, quest.advance_gp, quest.reward_gp
        )));
    }
    for (at, clue) in quest.clues.iter().enumerate() {
        if !is_valid_id(clue) || quest.clues[..at].contains(clue) {
            return Err(invalid(format!(
                "{}'s clue '{clue}' is not a new id",
                quest.id
            )));
        }
    }
    Ok(())
}

/// The quests of the world state, oldest first; none when it cannot be read (one `WORLD
/// quests_unavailable error=<key>` line).
pub fn quests_of(app: &AppState) -> Vec<QuestRecord> {
    match read_quests(&world_directory(app)) {
        Ok(quests) => quests,
        Err(error) => {
            app.record_line(format!("WORLD quests_unavailable error={}", error.key()));
            Vec::new()
        }
    }
}

/// Takes every quest off the world state, so a Start's quests start unoffered (a fresh journey,
/// `planning/town-to-tomb.md` part C). One `QUEST cleared count=` line (the quests there were), or
/// `QUEST clear_refused error=`.
pub(crate) async fn clear_quests(app: &AppState) {
    let directory = world_directory(app);
    let cleared = run_blocking(
        move || {
            write_quests(&directory, |records| {
                let count = records.len();
                records.clear();
                Ok(count)
            })
        },
        WorldError::Stopped,
    )
    .await;
    match cleared {
        Ok((_, count)) => app.record_line(format!("QUEST cleared count={count}")),
        Err(error) => app.record_line(format!("QUEST clear_refused error={}", error.key())),
    }
}

/// `offered` put on the board: each checked and none twice. Offered quests not offered again leave
/// the board; accepted ones stay as they are, and one offered again stays accepted (it is the
/// heroes' already, never offered anew).
pub(super) fn offer(records: &mut Vec<QuestRecord>, offered: Vec<Quest>) -> Result<(), WorldError> {
    if offered.is_empty() {
        return Err(invalid("a board offers at least one quest"));
    }
    for (at, quest) in offered.iter().enumerate() {
        check_quest(quest)?;
        if offered[..at].iter().any(|seen| seen.id == quest.id) {
            return Err(invalid(format!("'{}' is offered twice", quest.id)));
        }
    }
    records.retain(|record| record.standing != QuestStanding::Offered);
    let fresh: Vec<QuestRecord> = offered
        .into_iter()
        .filter(|quest| !records.iter().any(|record| record.quest.id == quest.id))
        .map(|quest| QuestRecord {
            quest,
            standing: QuestStanding::Offered,
            accepted_by: Vec::new(),
        })
        .collect();
    records.extend(fresh);
    Ok(())
}

/// The quest `id` taken by `heroes`, once for each hero: answers the quest and the heroes who take
/// it now. A hero who took it before takes it again as theirs already, so no advance is paid twice
/// (a later day's board offers the same quest).
pub(super) fn accept(
    records: &mut [QuestRecord],
    id: &str,
    heroes: &[String],
) -> Result<(Quest, Vec<String>), WorldError> {
    if heroes.is_empty() {
        return Err(invalid("a quest is taken by at least one hero"));
    }
    if let Some((at, hero)) = heroes
        .iter()
        .enumerate()
        .find(|(at, hero)| !is_valid_id(hero) || heroes[..*at].contains(hero))
    {
        return Err(invalid(format!(
            "hero {} ('{hero}') is not a new id",
            at + 1
        )));
    }
    let record = records
        .iter_mut()
        .find(|record| record.quest.id == id)
        .ok_or_else(|| WorldError::UnknownQuest { id: id.to_owned() })?;
    let newly: Vec<String> = heroes
        .iter()
        .filter(|hero| !record.accepted_by.contains(hero))
        .cloned()
        .collect();
    record.standing = QuestStanding::Accepted;
    record.accepted_by.extend(newly.iter().cloned());
    Ok((record.quest.clone(), newly))
}

/// Marks the accepted quest `id` completed: answers it and the heroes who took it, in seat order.
/// A quest not taken, or already done, is refused so the reward is never paid twice.
pub(super) fn complete(
    records: &mut [QuestRecord],
    id: &str,
) -> Result<(Quest, Vec<String>), WorldError> {
    let record = records
        .iter_mut()
        .find(|record| record.quest.id == id)
        .ok_or_else(|| WorldError::UnknownQuest { id: id.to_owned() })?;
    if record.standing != QuestStanding::Accepted {
        return Err(invalid(format!(
            "{id} is {}, not a quest in hand",
            record.standing.key()
        )));
    }
    record.standing = QuestStanding::Completed;
    Ok((record.quest.clone(), record.accepted_by.clone()))
}

/// The hero's share of `reward_gp` among `heroes`: an even split in whole gold pieces. The few
/// gold pieces left over (the table's own simple rule; the SRD sets none) go to the first hero.
pub(super) fn share_of(reward_gp: u64, heroes: usize, position: usize) -> u64 {
    let count = heroes as u64;
    reward_gp / count + if position == 0 { reward_gp % count } else { 0 }
}

/// Pays `quest`'s advance to `hero` in gold pieces through the inventory's coin gain
/// (`gain_coins`: into the hero's gold, or a new gold item when the purse holds none; its own
/// `INVENTORY gain_coins` line, whose why names the giver and the quest). The advance is the
/// giver's reward paid out, not coins taken from anyone's bag: the giver is the quest's words, no
/// inventory owner, so no other bag changes.
async fn pay(app: &AppState, quest: &Quest, hero: &str) -> GoldPaid {
    let why = format!("a reward advance from {} on {}", quest.giver, quest.title);
    pay_gp(app, quest, hero, quest.advance_gp, &why).await
}

/// Pays `gp` gold pieces of `quest` to `hero` for `why`: one `QUEST paid` line, or `QUEST
/// pay_refused`.
async fn pay_gp(app: &AppState, quest: &Quest, hero: &str, gp: u64, why: &str) -> GoldPaid {
    match super::inventory::gain_coins(app, hero, Denomination::Gp, gp, why).await {
        Ok(applied) => {
            let item = applied
                .first()
                .map_or_else(String::new, |applied| applied.item.clone());
            app.record_line(format!(
                "QUEST paid id={} hero={hero} gp={gp} item={item} count={gp}",
                quest.id
            ));
            GoldPaid::Paid {
                hero: hero.to_owned(),
                item,
                count: gp,
            }
        }
        Err(error) => {
            let reason = error.key().to_owned();
            app.record_line(format!(
                "QUEST pay_refused id={} hero={hero} reason={reason}",
                quest.id
            ));
            GoldPaid::Refused {
                hero: hero.to_owned(),
                reason,
            }
        }
    }
}

/// Keeps `quest` in `hero`'s notebook as a note: taken, or `done` (beside the one made when it was
/// taken); answers the memory's id. One `QUEST noted id= hero= memory=` line (`done=yes` for a
/// finished one), or `QUEST note_refused`.
async fn note(app: &AppState, quest: &Quest, hero: &str, done: bool) -> Option<String> {
    let kind = KeptKind::Note.key();
    let (title, description) = if done {
        (
            format!("{} (complete)", quest.title),
            format!(
                "Done: {}. The way to {} is open and the reward from {} was paid.",
                quest.title, quest.site, quest.giver
            ),
        )
    } else {
        (
            quest.title.clone(),
            format!(
                "Taken from {}: {}. The reward is {} gp; {} gp each was paid in advance. It leads \
                 to {}.",
                quest.giver, quest.title, quest.reward_gp, quest.advance_gp, quest.site
            ),
        )
    };
    let owner = hero.to_owned();
    let place = Some(quest.id.clone());
    let kept = keep_next(app, hero, kind, move |id| MemoryDraft {
        id,
        owner,
        kind: kind.to_owned(),
        title,
        description,
        place,
        pictures: Vec::new(),
        links: Vec::new(),
        tags: Vec::new(),
        created_by: Some(NOTED_BY.to_owned()),
    })
    .await;
    match kept {
        Ok(put) => {
            app.record_line(format!(
                "QUEST noted id={} hero={hero} memory={}{}",
                quest.id,
                put.memory.id,
                if done { " done=yes" } else { "" }
            ));
            Some(put.memory.id)
        }
        Err(error) => {
            app.record_line(format!(
                "QUEST note_refused id={} hero={hero} error={}",
                quest.id,
                error.key()
            ));
            None
        }
    }
}

/// Puts the quest's mark on the shared screen: its title and, once the party is on its way, the
/// stop reached.
async fn show(app: &AppState, quest: &Quest, progress: Option<Progress>) {
    let at = progress.as_ref().map_or(0, |progress| progress.index);
    let mark = StoryMark {
        title: quest.title.clone(),
        progress,
    };
    if let Err(error) = mark.check() {
        app.record_line(format!(
            "QUEST shown_refused id={} reason={error:?}",
            quest.id
        ));
        return;
    }
    match crate::playground::apply_scene_changes(app, vec![SceneChange::Story { mark }]).await {
        Ok(_) => app.record_line(format!("QUEST shown id={} at={at}", quest.id)),
        Err(error) => app.record_line(format!(
            "QUEST shown_refused id={} status={}",
            quest.id,
            error.status.as_u16()
        )),
    }
}

/// Answers `result`, recording a refusal first as `WORLD refused verb=.. error=<key>`.
fn answer<T: Serialize>(app: &AppState, verb: &str, result: Result<T, WorldError>) -> Response {
    if let Err(error) = &result {
        app.record_line(format!("WORLD refused verb={verb} error={}", error.key()));
    }
    reply::<T, WorldError>(result)
}

async fn get_quests(State(app): State<Arc<AppState>>) -> Response {
    let directory = world_directory(&app);
    let result = run_blocking(
        move || read_quests(&directory).map(|quests| QuestsView { quests }),
        WorldError::Stopped,
    )
    .await;
    answer(&app, "quests", result)
}

async fn post_offer(State(app): State<Arc<AppState>>, bytes: Bytes) -> Response {
    let result = async {
        let request: QuestOffer = parse_body(&bytes)?;
        let directory = world_directory(&app);
        let offered = request.quests.clone();
        let (quests, ()) = run_blocking(
            move || write_quests(&directory, |records| offer(records, request.quests)),
            WorldError::Stopped,
        )
        .await?;
        for quest in &offered {
            app.record_line(format!(
                "QUEST offered id={} reward_gp={} advance_gp={} clues={}",
                quest.id,
                quest.reward_gp,
                quest.advance_gp,
                quest.clues.join(",")
            ));
        }
        Ok::<_, WorldError>(QuestsView { quests })
    }
    .await;
    answer(&app, "quest_offer", result)
}

async fn post_accept(State(app): State<Arc<AppState>>, bytes: Bytes) -> Response {
    let result = async {
        let request: QuestAccept = parse_body(&bytes)?;
        let directory = world_directory(&app);
        let heroes = request.heroes.clone();
        let (_, (quest, newly)) = run_blocking(
            move || {
                write_quests(&directory, |records| {
                    accept(records, &request.quest, &request.heroes)
                })
            },
            WorldError::Stopped,
        )
        .await?;
        let already: Vec<String> = heroes
            .iter()
            .filter(|hero| !newly.contains(hero))
            .cloned()
            .collect();
        if !already.is_empty() {
            app.record_line(format!(
                "QUEST already_yours id={} by={}",
                quest.id,
                already.join(",")
            ));
        }
        if !newly.is_empty() {
            app.record_line(format!(
                "QUEST accepted id={} by={} reward_gp={} advance_gp={}",
                quest.id,
                newly.join(","),
                quest.reward_gp,
                quest.advance_gp
            ));
        }
        let mut paid = Vec::new();
        let mut noted = Vec::new();
        for hero in &newly {
            if quest.advance_gp > 0 {
                paid.push(pay(&app, &quest, hero).await);
            }
            if let Some(memory) = note(&app, &quest, hero, false).await {
                noted.push((hero.clone(), memory));
            }
        }
        show(&app, &quest, None).await;
        Ok::<_, WorldError>(QuestAccepted {
            quest,
            already,
            paid,
            noted,
        })
    }
    .await;
    answer(&app, "quest_accept", result)
}

async fn post_complete(State(app): State<Arc<AppState>>, bytes: Bytes) -> Response {
    let result = async {
        let request: QuestComplete = parse_body(&bytes)?;
        let directory = world_directory(&app);
        let (_, (quest, heroes)) = run_blocking(
            move || write_quests(&directory, |records| complete(records, &request.quest)),
            WorldError::Stopped,
        )
        .await?;
        let shares: Vec<u64> = (0..heroes.len())
            .map(|position| share_of(quest.reward_gp, heroes.len(), position))
            .collect();
        // Each hero's share past the first, who also takes what is left over.
        app.record_line(format!(
            "QUEST completed id={} by={} reward_gp={} each_gp={} remainder_to={}",
            quest.id,
            heroes.join(","),
            quest.reward_gp,
            shares.last().copied().unwrap_or(0),
            heroes.first().map_or("-", String::as_str)
        ));
        let why = format!("the reward from {} for {}", quest.giver, quest.title);
        let mut paid = Vec::new();
        let mut noted = Vec::new();
        for (hero, gp) in heroes.iter().zip(shares) {
            paid.push(pay_gp(&app, &quest, hero, gp, &why).await);
            if let Some(memory) = note(&app, &quest, hero, true).await {
                noted.push((hero.clone(), memory));
            }
        }
        let progress = Progress {
            index: 1,
            total: 1,
            title: Some("Complete".to_owned()),
        };
        show(&app, &quest, Some(progress)).await;
        Ok::<_, WorldError>(QuestCompleted { quest, paid, noted })
    }
    .await;
    answer(&app, "quest_complete", result)
}

async fn post_progress(State(app): State<Arc<AppState>>, bytes: Bytes) -> Response {
    let result = async {
        let request: QuestProgress = parse_body(&bytes)?;
        if request.index == 0 || request.index > request.total || request.place.trim().is_empty() {
            return Err(invalid(format!(
                "stop {} of {} at '{}' is not a stop of the way",
                request.index, request.total, request.place
            )));
        }
        let directory = world_directory(&app);
        let quests = run_blocking(move || read_quests(&directory), WorldError::Stopped).await?;
        let quest = quests
            .iter()
            .rev()
            .find(|record| record.standing == QuestStanding::Accepted)
            .map(|record| record.quest.clone())
            .ok_or(WorldError::NoQuestTaken)?;
        let progress = Progress {
            index: request.index,
            total: request.total,
            title: Some(request.place.trim().to_owned()),
        };
        show(&app, &quest, Some(progress)).await;
        Ok::<_, WorldError>(QuestsView { quests })
    }
    .await;
    answer(&app, "quest_progress", result)
}

/// The quest routes.
pub(super) fn router_with(app: Arc<AppState>) -> Router {
    Router::new()
        .route(path::WORLD_QUESTS, get(get_quests))
        .route(path::WORLD_QUEST_OFFER, post(post_offer))
        .route(path::WORLD_QUEST_ACCEPT, post(post_accept))
        .route(path::WORLD_QUEST_PROGRESS, post(post_progress))
        .route(path::WORLD_QUEST_COMPLETE, post(post_complete))
        .with_state(app)
}
