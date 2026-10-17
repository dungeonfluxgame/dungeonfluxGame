//! Moving the party between places: the service side.
//!
//! `leave_by_freeform` is the hero-line route: a freeform act Jev read as a move, once it has
//! succeeded, is put to Jev again as a closed choice among the place's open exits. `move_party_*`
//! are the one path every move takes (a hero line, `dev place go`): they resolve where the exit
//! leads, create the place when the run does not have it yet, and ask the session owner to commit
//! the move. The table is told about a move only from the committed result.

use std::time::Duration;

use df_model::checkpoint::Checkpoint;
use df_protocol::common as rpc;
use df_types::OperationId;
use serde_json::{Value, json};
use tokio::sync::oneshot;

use super::{
    Service,
    actor::{
        self,
        place_actor::{MoveOutcome, MoveProposal, Moved},
    },
    adventure::scene_model::{SceneModel, SceneWayOut},
    beat_script_runtime,
    dialogue::DialogueReply,
    freeform_action::{self, ActKind, Resolution, Verdict},
    journey,
    memory::{self, Audience, recall},
    pack,
    place::{self, DEV_EXIT, Destination, PlaceRefusal},
    place_creation::{self, CreationRequest},
    timing, wire,
};

/// How long the whole creation of a place may take: two storyteller attempts at most.
const CREATION_DEADLINE: Duration = Duration::from_secs(90);
/// Recent public table lines given to the storyteller.
const RECENT_LINES: usize = 10;
const RECENT_LINE_CHARACTERS: usize = 200;
const STATEMENT_LIMIT: usize = 8;

/// Why a move was not made.
pub(super) enum MoveError {
    Refused(PlaceRefusal),
    Status(tonic::Status),
}

impl From<PlaceRefusal> for MoveError {
    fn from(reason: PlaceRefusal) -> Self {
        Self::Refused(reason)
    }
}

impl From<tonic::Status> for MoveError {
    fn from(status: tonic::Status) -> Self {
        Self::Status(status)
    }
}

fn state_failure(error: df_session::submission::RepositoryError) -> MoveError {
    MoveError::Status(actor::unavailable(error))
}

fn audit_refusal(from: &str, exit: &str, reason: &str) {
    place::audit_refused(from, "none", exit, reason);
}

/// One exit as the CLI and the audit see it: where it leads, as far as the run knows.
fn leads_to(
    current: &Checkpoint,
    party: &place::Whereabouts,
    exit: &SceneWayOut,
) -> Result<Value, df_session::submission::RepositoryError> {
    Ok(
        match place::destination(current.state(), &party.home, &party.here, exit)? {
            Destination::Home => json!({
                "status": "home",
                "place_id": party.home,
                "name": place::name_of(current.state(), &party.home)?,
            }),
            Destination::Created(known) => json!({
                "status": "created",
                "place_id": known.place_id,
                "name": known.name,
            }),
            Destination::Create { named } => json!({
                "status": "new",
                "place_id": named,
            }),
            Destination::Story { place_id } => json!({
                "status": "story",
                "place_id": place_id,
            }),
        },
    )
}

impl Service {
    /// The current place, its exits and every place the run knows, for the dev CLI.
    pub(super) fn place_report(&self) -> Result<Value, tonic::Status> {
        let current = self.updates.borrow().clone();
        report(&current).map_err(actor::unavailable)
    }

    /// A hero's freeform act that Jev read as a move: when it succeeded and Jev finds an exit the
    /// line goes through, the party moves there. `None` leaves the act to the ordinary narration:
    /// nothing moved, so nothing is claimed.
    pub(super) async fn leave_by_freeform(
        &self,
        credential: [u8; 32],
        request: &rpc::RawDialogueRequest,
        scene: &pack::Scene,
        hero: &str,
        hero_index: u32,
        resolution: &Resolution,
    ) -> Option<DialogueReply> {
        if resolution.act != Some(ActKind::Move) {
            return None;
        }
        let Verdict::Performed(performance) = &resolution.verdict else {
            return None;
        };
        if !performance.succeeded {
            return None;
        }
        let current = self.updates.borrow().clone();
        let snapshot = journey::living_scene::snapshot(current.state(), scene)
            .ok()
            .flatten()?;
        let exits: Vec<&SceneWayOut> = snapshot
            .model
            .ways_out
            .iter()
            .filter(|way| way.open)
            .collect();
        let short = scene
            .setting
            .as_ref()
            .map(|setting| setting.short.as_str())
            .unwrap_or_default();
        let (_, jev) = self.npc_voice.hero_identity_models();
        let from = scene
            .setting
            .as_ref()
            .map(|setting| setting.place_id.clone())
            .unwrap_or_default();
        let chosen =
            match place_creation::judge_exit(jev, &request.text, hero, &scene.title, short, &exits)
                .await
            {
                Ok(chosen) => chosen,
                Err(reason) => {
                    place::audit_judged(
                        &from,
                        "unavailable",
                        &format!(" reason={}", reason.label()),
                    );
                    return None;
                }
            };
        let exit = chosen.and_then(|index| exits.get(index).copied());
        place::audit_judged(&from, exit.map_or("none", |exit| exit.id.as_str()), "");
        let exit = exit?;
        let operation = df_api::operation_id(request.input_id.as_ref()).ok()?;
        let moved = match self
            .move_party_through(credential, operation, &exit.id)
            .await
        {
            Ok(moved) => moved,
            Err(MoveError::Refused(_)) | Err(MoveError::Status(_)) => return None,
        };
        let roll = performance.check.as_ref().map(|check| check.row());
        freeform_action::publish_outcome(&self.table_log, hero_index, roll, &moved.line);
        Some(DialogueReply::Raw(
            rpc::DialogueResponse {
                input_id: request.input_id.clone(),
                disposition: rpc::DialogueDisposition::Action as i32,
                revision: Some(wire::revision(moved.revision)),
                npc_reply: moved.line,
                ..Default::default()
            },
            None,
        ))
    }

    /// Moves the party through `exit_id` of the place it stands in. The place is created first
    /// when the run does not have it yet; the owner then repeats every check and commits.
    pub(super) async fn move_party_through(
        &self,
        credential: [u8; 32],
        operation: OperationId,
        exit_id: &str,
    ) -> Result<Box<Moved>, MoveError> {
        let current = self.updates.borrow().clone();
        let state = current.state();
        let scene = beat_script_runtime::scene(state).map_err(state_failure)?;
        let snapshot = journey::living_scene::snapshot(state, &scene)
            .map_err(state_failure)?
            .ok_or(PlaceRefusal::NoExit)?;
        let party = place::whereabouts(state, &scene).map_err(state_failure)?;
        let exit = snapshot
            .model
            .ways_out
            .iter()
            .find(|way| way.id == exit_id)
            .cloned()
            .ok_or(PlaceRefusal::NoExit)?;
        if !exit.open {
            audit_refusal(&party.here, &exit.id, PlaceRefusal::Closed.label());
            return Err(PlaceRefusal::Closed.into());
        }
        let (to, created) = match place::destination(state, &party.home, &party.here, &exit)
            .map_err(state_failure)?
        {
            Destination::Home => (party.home.clone(), None),
            Destination::Created(known) => (known.place_id, None),
            Destination::Story { .. } => {
                audit_refusal(&party.here, &exit.id, PlaceRefusal::StoryPlace.label());
                return Err(PlaceRefusal::StoryPlace.into());
            }
            Destination::Create { named } => {
                if place::created(state).map_err(state_failure)?.len()
                    >= place::MAXIMUM_CREATED_PLACES
                {
                    audit_refusal(&party.here, &exit.id, PlaceRefusal::RunFull.label());
                    return Err(PlaceRefusal::RunFull.into());
                }
                let request = self
                    .creation_request(&current, &scene, &snapshot.model, &exit, named)
                    .await
                    .map_err(state_failure)?;
                let (storyteller, _) = self.npc_voice.hero_identity_models();
                let made = tokio::time::timeout(
                    CREATION_DEADLINE,
                    place_creation::create(storyteller, &request),
                )
                .await;
                let record = match made {
                    Ok(Ok(record)) => record,
                    Ok(Err(error)) => {
                        audit_refusal(&party.here, &exit.id, error.label());
                        return Err(PlaceRefusal::CreationUnavailable.into());
                    }
                    Err(_) => {
                        audit_refusal(&party.here, &exit.id, "creation_deadline");
                        return Err(PlaceRefusal::CreationUnavailable.into());
                    }
                };
                (record.place_id.clone(), Some(record))
            }
        };
        self.commit_move(
            credential,
            MoveProposal {
                operation,
                from: party.here,
                exit: exit.id,
                to,
                created,
            },
        )
        .await
    }

    /// Moves the party straight to a place the run already has (development only: no exit is
    /// used, so nothing is created).
    pub(super) async fn move_party_to_known(
        &self,
        credential: [u8; 32],
        operation: OperationId,
        place_id: &str,
    ) -> Result<Box<Moved>, MoveError> {
        let current = self.updates.borrow().clone();
        let scene = beat_script_runtime::scene(current.state()).map_err(state_failure)?;
        let party = place::whereabouts(current.state(), &scene).map_err(state_failure)?;
        self.commit_move(
            credential,
            MoveProposal {
                operation,
                from: party.here,
                exit: DEV_EXIT.to_owned(),
                to: place_id.to_owned(),
                created: None,
            },
        )
        .await
    }

    async fn commit_move(
        &self,
        credential: [u8; 32],
        proposal: MoveProposal,
    ) -> Result<Box<Moved>, MoveError> {
        let (reply, wait) = oneshot::channel();
        self.actor
            .try_submit(actor::Call::PlaceMove {
                credential,
                proposal,
                reply,
            })
            .map_err(|_| tonic::Status::resource_exhausted("gameplay actor queue full"))?;
        let outcome = tokio::time::timeout(timing::active().reply, wait)
            .await
            .map_err(|_| tonic::Status::deadline_exceeded("the change of place timed out"))?
            .map_err(|_| tonic::Status::unavailable("gameplay actor stopped"))??;
        match outcome {
            MoveOutcome::Moved(moved) => Ok(moved),
            MoveOutcome::Refused(reason) => Err(MoveError::Refused(reason)),
        }
    }

    /// What the storyteller is given to create the place beyond `exit`: only public things.
    async fn creation_request(
        &self,
        current: &Checkpoint,
        scene: &pack::Scene,
        model: &SceneModel,
        exit: &SceneWayOut,
        named: Option<String>,
    ) -> Result<CreationRequest, df_session::submission::RepositoryError> {
        let state = current.state();
        let setting = scene
            .setting
            .as_ref()
            .ok_or(df_session::submission::RepositoryError::InvalidCandidate)?;
        let heroes: Vec<String> = state
            .characters
            .iter()
            .map(|character| journey::name(state, character.entity))
            .collect::<Result<_, _>>()?;
        let origin_people: Vec<(String, String)> = scene
            .cast
            .iter()
            .map(|member| {
                let role = model
                    .people
                    .iter()
                    .find(|person| person.id == member.id)
                    .map_or_else(String::new, |person| person.role.clone());
                (member.name.clone(), role)
            })
            .collect();
        let statements: Vec<String> = scene
            .cast
            .iter()
            .filter_map(|member| model.people.iter().find(|person| person.id == member.id))
            .flat_map(|person| {
                person
                    .knowledge
                    .iter()
                    .map(|fact| format!("{}: {}", person.name, fact.statement))
            })
            .take(STATEMENT_LIMIT)
            .collect();
        let mut taken_ids: Vec<String> = place::story_places()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        taken_ids.extend(place::created(state)?.into_keys());
        let scene_label = format!("{:?}", scene.key);
        let game_names: Vec<String> = origin_people.iter().map(|(name, _)| name.clone()).collect();
        let canon = self
            .npc_voice
            .local_canon(&scene_label, &heroes, &game_names);
        let recent_table = self
            .table_log
            .recent_public(RECENT_LINES)
            .into_iter()
            .map(|(speaker, text)| {
                format!("{speaker}: {text}")
                    .chars()
                    .take(RECENT_LINE_CHARACTERS)
                    .collect()
            })
            .collect();
        let recalled_memory = match memory::current() {
            Some(story) => {
                let text = format!("{} {} {}", exit.name, exit.beyond, setting.name);
                let entities = [
                    setting.place_id.as_str(),
                    exit.id.as_str(),
                    setting.name.as_str(),
                ];
                let asked = recall::RecallRequest::new(
                    recall::Purpose::Storyteller,
                    Audience::Public,
                    &text,
                )
                .scene(&scene_label)
                .entities(&entities);
                tokio::time::timeout(Duration::from_secs(4), story.recall(&asked))
                    .await
                    .map(|pack| pack.rendered)
                    .unwrap_or_default()
            }
            None => String::new(),
        };
        Ok(CreationRequest {
            adventure: pack::scene(pack::SceneKey::Opening).title,
            origin_id: setting.place_id.clone(),
            origin_name: setting.name.clone(),
            origin_short: setting.short.clone(),
            origin_narration: scene.narration.chars().take(300).collect(),
            time_of_day: model.time_of_day.clone(),
            weather: model.weather.clone(),
            origin_people,
            statements,
            heroes,
            exit: exit.clone(),
            named,
            taken_ids,
            canon,
            recent_table,
            recalled_memory,
        })
    }
}

/// What `dev place go` was told to go to.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum DevTarget {
    /// An exit of the current place, by id.
    Exit(String),
    /// A place the run has, by id.
    Known(String),
}

/// Reads a `dev place go` argument against the current place's exits, then the places the run
/// has. This reads a developer's argument, never a player's words.
pub(super) fn resolve_target(current: &Checkpoint, wanted: &str) -> Result<DevTarget, String> {
    let unavailable = |_| "the current place is unavailable".to_owned();
    let state = current.state();
    let scene = beat_script_runtime::scene(state).map_err(unavailable)?;
    let snapshot = journey::living_scene::snapshot(state, &scene)
        .map_err(unavailable)?
        .ok_or_else(|| "the current place has no scene model".to_owned())?;
    let wanted = wanted.trim();
    let same = |text: &str| text.eq_ignore_ascii_case(wanted);
    if let Some(exit) = snapshot.model.ways_out.iter().find(|exit| {
        same(&exit.id) || same(&exit.name) || exit.aliases.iter().any(|alias| same(alias))
    }) {
        return Ok(DevTarget::Exit(exit.id.clone()));
    }
    let mut known = place::story_places();
    known.extend(
        place::created(state)
            .map_err(unavailable)?
            .into_iter()
            .map(|(id, record)| (id, record.name)),
    );
    if let Some((id, _)) = known.iter().find(|(id, name)| same(id) || same(name)) {
        return Ok(DevTarget::Known(id.clone()));
    }
    Err(format!(
        "no exit or place matches {wanted:?}; exits: {}; places: {}",
        snapshot
            .model
            .ways_out
            .iter()
            .map(|exit| exit.id.as_str())
            .collect::<Vec<_>>()
            .join(", "),
        known
            .iter()
            .map(|(id, _)| id.as_str())
            .collect::<Vec<_>>()
            .join(", "),
    ))
}

/// The report `dev place` prints.
pub(super) fn report(
    current: &Checkpoint,
) -> Result<Value, df_session::submission::RepositoryError> {
    let state = current.state();
    let scene = beat_script_runtime::scene(state)?;
    let party = place::whereabouts(state, &scene)?;
    let snapshot = journey::living_scene::snapshot(state, &scene)?;
    let created = place::created(state)?;
    let setting = scene
        .setting
        .as_ref()
        .ok_or(df_session::submission::RepositoryError::InvalidCandidate)?;
    let exits = snapshot
        .as_ref()
        .map(|snapshot| {
            snapshot
                .model
                .ways_out
                .iter()
                .map(|exit| {
                    Ok(json!({
                        "id": exit.id,
                        "name": exit.name,
                        "position": exit.position,
                        "beyond": exit.beyond,
                        "open": exit.open,
                        "leads_to": leads_to(current, &party, exit)?,
                    }))
                })
                .collect::<Result<Vec<_>, df_session::submission::RepositoryError>>()
        })
        .transpose()?
        .unwrap_or_default();
    let mut known: Vec<Value> = place::story_places()
        .into_iter()
        .map(|(id, name)| {
            json!({
                "id": id,
                "name": name,
                "origin": "story",
                "current": id == party.here,
            })
        })
        .collect();
    for (id, record) in &created {
        known.push(json!({
            "id": id,
            "name": record.name,
            "origin": "created",
            "kind": record.kind,
            "people": record.people.iter().map(|person| json!({"name": person.name, "role": person.role})).collect::<Vec<_>>(),
            "current": *id == party.here,
        }));
    }
    let moves: Vec<Value> = place::entries(state)?
        .into_iter()
        .map(|(revision, entry)| {
            json!({
                "revision": {"epoch": revision.epoch().get(), "sequence": revision.sequence()},
                "hero": entry.hero,
                "from": entry.change.from,
                "to": entry.change.to,
                "exit": entry.change.exit,
                "beat": entry.change.beat,
                "created": entry.change.created.is_some(),
            })
        })
        .collect();
    Ok(json!({
        "place": {
            "id": setting.place_id,
            "name": setting.name,
            "short": setting.short,
            "kind": setting.kind,
            "exterior": setting.exterior,
            "created": created.contains_key(&setting.place_id),
            "home": setting.place_id == party.home,
            "beat": party.beat,
            "title": scene.title,
            "people": scene.cast.iter().map(|member| member.name.clone()).collect::<Vec<_>>(),
        },
        "exits": exits,
        "known": known,
        "moves": moves,
    }))
}

#[cfg(test)]
#[path = "place_tests.rs"]
mod tests;
