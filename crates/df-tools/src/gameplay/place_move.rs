//! Moving the party between places: the service side.
//!
//! `leave_by_freeform` is the hero-line route: a freeform act Jev read as a move, once it has
//! succeeded, is put to Jev again as a closed choice of where it goes: an open exit of the place,
//! a place the run has, a landmark of this place, elsewhere (a destination the hero names), or
//! nowhere. `hailed_ride` and `take_ride` are the ride route: a line that hails or hires a ride
//! (a passing carriage and its driver) becomes a journey with a short narrated travel beat, paid
//! through a listed service of the place when Jev finds one is that ride. `move_party_*` are the
//! one path every move takes (a hero line, a ride, `dev place go`): they resolve where the move
//! leads, create the place when the run does not have it yet, and ask the session owner to commit
//! the move. The table is told about a move only from the committed result.

use std::{
    sync::{Arc, Mutex, Weak},
    time::{Duration, Instant},
};

use df_model::checkpoint::Checkpoint;
use df_protocol::common as rpc;
use df_types::OperationId;
use futures::{StreamExt, stream::FuturesUnordered};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::{oneshot, watch};

#[cfg(test)]
use super::place::DEV_EXIT;
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
    place::{self, Destination, JOURNEY_EXIT, KnownPlace, LandmarkDraft, PlaceRefusal},
    place_creation::{self, CreationRequest, Heading, HeadingChoices, ServiceOffer, TravelBeat},
    timing, wire,
};

/// How long the whole creation of a place may take: two storyteller attempts at most.
const CREATION_DEADLINE: Duration = Duration::from_secs(90);
/// Recent public table lines given to the storyteller.
const RECENT_LINES: usize = 10;
const RECENT_LINE_CHARACTERS: usize = 200;
const STATEMENT_LIMIT: usize = 8;
/// Said when a chosen move could not be recorded: the party is where it was. In the fiction, as
/// the route-ahead refusal is.
const MOVE_NOT_RECORDED: &str =
    "You look for a route ahead, but cannot make it out yet. Your party stays together here.";
/// Added after a paid ride could not start. Only the purse fact; the owner decides the refund
/// (issue-triage TRV-01).
const FARE_KEPT: &str = "The coins you paid are already spent.";

tokio::task_local! {
    /// The input boundary scopes this across heading judgement and the resulting move.
    pub(super) static HERO_LINE_STARTED: Instant;
}

fn move_started() -> Instant {
    HERO_LINE_STARTED
        .try_with(|started| *started)
        .unwrap_or_else(|_| Instant::now())
}

#[derive(Clone)]
enum AheadOutcome {
    Pending,
    Ready(Arc<place::PlaceRecord>),
    Failed,
    Cancelled,
}

struct AheadExit {
    id: String,
    outcome: watch::Sender<AheadOutcome>,
}

struct AheadBatch {
    origin: String,
    basis: df_model::checkpoint::Basis,
    token: Arc<()>,
    started: Instant,
    operation: Option<u64>,
    exits: Vec<AheadExit>,
    task: tokio::task::JoinHandle<()>,
}

fn audit_ahead(from: &str, exit: &str, outcome: &str, started: Instant, operation: Option<u64>) {
    let correlation = operation
        .map(|operation| format!(" operation_hash={operation}"))
        .unwrap_or_default();
    actor::record_audit_line(
        "gameplay.free_roam",
        format!(
            "FREE_ROAM look_ahead from={from} exit={exit} outcome={outcome} ms={}{correlation}",
            started.elapsed().as_millis()
        ),
    );
}

impl AheadBatch {
    fn cancel_pending(&self) {
        for exit in &self.exits {
            if matches!(*exit.outcome.borrow(), AheadOutcome::Pending) {
                exit.outcome.send_replace(AheadOutcome::Cancelled);
                audit_ahead(
                    &self.origin,
                    &exit.id,
                    "cancelled",
                    self.started,
                    self.operation,
                );
            }
        }
    }
}

impl Drop for AheadBatch {
    fn drop(&mut self) {
        self.cancel_pending();
        self.task.abort();
    }
}

/// Shared by service clones; the final owner aborts its retained task on drop.
#[derive(Default)]
pub(super) struct LookAhead {
    batch: Mutex<Option<AheadBatch>>,
    #[cfg(test)]
    generator: Mutex<Option<NeighbourGenerator>>,
}

#[cfg(test)]
type NeighbourGenerator = Arc<
    dyn Fn(
            &CreationRequest,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<place::PlaceRecord, place_creation::CreationError>,
                    > + Send,
            >,
        > + Send
        + Sync,
>;

impl LookAhead {
    pub(super) fn cancel(&self) {
        self.lock().take();
    }

    #[cfg(test)]
    pub(super) async fn wait(&self) {
        let waits: Vec<_> = self
            .lock()
            .as_ref()
            .map(|batch| {
                batch
                    .exits
                    .iter()
                    .map(|exit| exit.outcome.subscribe())
                    .collect()
            })
            .unwrap_or_default();
        tokio::time::timeout(CREATION_DEADLINE, async {
            for mut outcome in waits {
                while matches!(*outcome.borrow_and_update(), AheadOutcome::Pending) {
                    outcome.changed().await.unwrap();
                }
                assert!(matches!(*outcome.borrow(), AheadOutcome::Ready(_)));
            }
        })
        .await
        .unwrap();
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<AheadBatch>> {
        self.batch
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

struct MoveCancellation {
    started: Instant,
    operation: OperationId,
    active: bool,
}

impl Drop for MoveCancellation {
    fn drop(&mut self) {
        if self.active {
            actor::record_audit_line(
                "gameplay.place",
                format!(
                    "PLACE move outcome=cancelled ms={} operation_hash={}",
                    self.started.elapsed().as_millis(),
                    df_observe::operation_hash(self.operation.as_bytes())
                ),
            );
        }
    }
}

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

/// The key that tells one ride's audit lines from another's: the hash of the line's own input.
fn audit_key(request: &rpc::RawDialogueRequest) -> u64 {
    let input = request
        .input_id
        .as_ref()
        .and_then(|id| id.value.as_deref())
        .unwrap_or_default();
    df_observe::operation_hash(input)
}

/// A paid fare whose journey did not start: the coins stay spent, and the audit says why.
fn audit_fare_kept(service_id: &str, price_cp: u32, reason: &str, key: u64) {
    actor::record_audit_line(
        "gameplay.place",
        format!(
            "PLACE ride outcome=fare_kept service={service_id} price_cp={price_cp} reason={reason} key={key}"
        ),
    );
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

/// Where a move goes, once Jev has chosen: through an exit, or on a journey to a place the run
/// has, to a landmark of this place, or to a new place the hero's words name.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum MoveTarget {
    Exit(String),
    Known(String),
    Landmark(LandmarkDraft),
    /// A model-selected named spot within the current place; direction carries its visible look.
    Spot(LandmarkDraft),
    New,
}

/// The current fiction's travel restriction. The authoritative owner repeats this at admission.
pub(super) fn travel_blocker(
    current: &Checkpoint,
) -> Result<Option<&'static str>, df_session::submission::RepositoryError> {
    let state = current.state();
    if journey::phase(current)? == rpc::JourneyPhase::Room {
        return Ok(Some(
            "Bring your heroes into the story before setting off together.",
        ));
    }
    if beat_script_runtime::ended(state)? || beat_script_runtime::closing(state)? {
        return Ok(Some(
            "Finish this chapter before beginning another journey.",
        ));
    }
    if state
        .encounters
        .iter()
        .any(|encounter| encounter.active_turn.is_some())
    {
        return Ok(Some("You cannot set off while the fight blocks your way."));
    }
    super::beat_script::with_script(|script| {
        let beat = script.current(state)?;
        if script.is_canonical()
            && matches!(
                beat.state.durable.as_str(),
                "opening" | "courier-answer-seal" | "courier-answer-escort"
            )
            && !beat_script_runtime::opening_threat_resolved(state)?
        {
            return Ok(Some(
                "The enforcer bars the courier's way, his hand on his scimitar. Settle the threat before your party can leave together.",
            ));
        }
        Ok(None)
    })
}

/// One truthful phone destination. The caller binds the selection to the observed revision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct TravelOption {
    pub(super) id: String,
    pub(super) label: String,
    pub(super) target: MoveTarget,
}

pub(super) fn travel_options(
    current: &Checkpoint,
) -> Result<Vec<TravelOption>, df_session::submission::RepositoryError> {
    if travel_blocker(current)?.is_some() {
        return Ok(vec![]);
    }
    let state = current.state();
    let scene = beat_script_runtime::scene(state)?;
    let Some(mut snapshot) = journey::living_scene::snapshot(state, &scene)? else {
        return Ok(vec![]);
    };
    let party = place::whereabouts(state, &scene)?;
    place::name_bound_exits(state, &party.here, &mut snapshot.model.ways_out)?;
    let mut options = Vec::new();
    let mut reached = Vec::new();
    for exit in snapshot.model.ways_out.iter().filter(|exit| exit.open) {
        match place::destination(state, &party.home, &party.here, exit)? {
            Destination::Story { .. } => continue,
            Destination::Home => reached.push(party.home.clone()),
            Destination::Created(known) => reached.push(known.place_id),
            Destination::Create { .. } => {}
        }
        options.push(TravelOption {
            id: format!("exit:{}", exit.id),
            label: format!("Travel via {}", exit.name),
            target: MoveTarget::Exit(exit.id.clone()),
        });
    }
    for (index, landmark) in place::landmarks_here(state, &party.here)?
        .into_iter()
        .enumerate()
    {
        options.push(TravelOption {
            id: format!("landmark:{index}"),
            label: format!("Travel to {}", landmark.name),
            target: MoveTarget::Landmark(landmark),
        });
    }
    for known in place::reachable(state, &party)? {
        if reached.contains(&known.id) {
            continue;
        }
        options.push(TravelOption {
            id: format!("known:{}", known.id),
            label: format!("Travel to {}", known.name),
            target: MoveTarget::Known(known.id),
        });
    }
    Ok(options)
}

/// A move that is ready to commit: its destination is resolved and, when it is new, made.
pub(super) struct PreparedMove {
    started: Instant,
    cancellation: Option<MoveCancellation>,
    operation: OperationId,
    expected_basis: df_model::checkpoint::Basis,
    from: String,
    from_name: String,
    exit: String,
    to: String,
    to_short: String,
    created: Option<place::PlaceRecord>,
}

/// The paid service a ride goes through, when the place lists one that is the ride.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct RideFare {
    pub(super) service_id: String,
    pub(super) price_cp: u32,
    pub(super) name: String,
    pub(super) provider: String,
}

/// A ride the hero hailed or hired, ready to go once any fare is paid. A ride that the travel
/// restriction already blocks has no fare and no destination: it is answered, never bought.
pub(super) struct Ride {
    journey: RideJourney,
    pub(super) fare: Option<RideFare>,
    hero: String,
    hero_index: u32,
}

enum RideJourney {
    Ready(Box<PreparedMove>),
    /// The restriction in force, as the table is told it.
    Blocked(Refusal),
}

/// What a refused ride tells the table, and the reason its audit line gives.
#[derive(Clone, Copy)]
struct Refusal {
    line: &'static str,
    reason: &'static str,
}

/// What a ride did to the party's journey. Either way the table has been told the `line`; only
/// `Travelled` is travel narration.
pub(super) enum RideOutcome {
    Travelled {
        line: String,
        revision: df_types::SessionRevision,
    },
    /// The journey did not start: the refusal, not travel narration.
    NotTaken {
        line: String,
        revision: df_types::SessionRevision,
    },
}

/// A journey the party made: where it arrived, the owner's arrival line, and the revision it made.
struct Arrival {
    place_name: String,
    line: String,
    revision: df_types::SessionRevision,
}

/// Why a prepared journey did not commit. A blocked journey is refused before the owner sees it.
enum CommitFailure {
    Blocked(&'static str),
    Move(MoveError),
}

impl From<CommitFailure> for MoveError {
    fn from(failure: CommitFailure) -> Self {
        match failure {
            CommitFailure::Blocked(reason) => tonic::Status::failed_precondition(reason).into(),
            CommitFailure::Move(error) => error,
        }
    }
}

/// What Jev chooses among for a line in the place the party stands in, with each exit's id.
struct Choices {
    exits: Vec<SceneWayOut>,
    known: Vec<KnownPlace>,
    landmarks: Vec<LandmarkDraft>,
    place_name: String,
    place_short: String,
    here: String,
}

impl Choices {
    /// The move target a heading names; `None` when the party stays.
    fn target(&self, heading: Heading) -> Option<MoveTarget> {
        match heading {
            Heading::Exit(index) => self
                .exits
                .get(index)
                .map(|exit| MoveTarget::Exit(exit.id.clone())),
            Heading::Known(index) => self
                .known
                .get(index)
                .map(|place| MoveTarget::Known(place.id.clone())),
            Heading::Landmark(index) => {
                self.landmarks.get(index).cloned().map(MoveTarget::Landmark)
            }
            Heading::Elsewhere => Some(MoveTarget::New),
            Heading::Stay => None,
        }
    }
}

/// A second operation id derived from a line's own, for a move that follows a payment the line
/// already committed under its id. The same line always derives the same id, so a retry repeats.
fn following_operation(operation: OperationId) -> Option<OperationId> {
    let digest = Sha256::digest([b"ride:".as_slice(), operation.as_bytes()].concat());
    OperationId::from_bytes(&digest[..16]).ok()
}

/// The gate's published scene has no synthetic way back to the outside approach.
pub(in crate::gameplay) fn destination_scene(
    current: &Checkpoint,
    record: &place::PlaceRecord,
    base: &pack::Scene,
) -> Result<pack::Scene, df_session::submission::RepositoryError> {
    let free_roam = super::adventure::free_roam::world();
    let mut scene = if free_roam
        .as_ref()
        .is_some_and(|world| world.gate.place_id == record.place_id)
    {
        pack::scene(pack::SceneKey::Opening)
    } else {
        place::scene_of(record, base)?
    };
    if free_roam.is_some() {
        let mut snapshot = journey::living_scene::snapshot(current.state(), &scene)?
            .ok_or(df_session::submission::RepositoryError::InvalidCandidate)?;
        let setting = scene
            .setting
            .as_mut()
            .ok_or(df_session::submission::RepositoryError::InvalidCandidate)?;
        place::name_bound_exits(
            current.state(),
            &setting.place_id,
            &mut snapshot.model.ways_out,
        )?;
        setting.model = Some(snapshot.model);
    }
    Ok(scene)
}

impl Service {
    /// A Jev travel intent: choose a destination, commit it once, then speak one arrival line.
    pub(super) async fn travel_by_line(
        &self,
        credential: [u8; 32],
        request: &rpc::RawDialogueRequest,
        scene: &pack::Scene,
    ) -> Result<DialogueReply, tonic::Status> {
        let expected = self.updates.borrow().basis();
        if df_api::session_id(request.session_id.as_ref()).ok() != Some(expected.session)
            || df_api::run_id(request.run_id.as_ref()).ok() != Some(expected.run)
            || df_api::session_revision(request.observed_revision.as_ref()).ok()
                != Some(expected.revision)
            || request.finality != rpc::DialogueFinality::Final as i32
            || request.context == rpc::DialogueContext::Meta as i32
        {
            return Err(tonic::Status::failed_precondition(
                "the departure's session or scene changed",
            ));
        }
        let (hero, hero_index) = self
            .speaking_hero(credential)
            .await
            .ok_or_else(|| tonic::Status::failed_precondition("travel needs a seated hero"))?;
        let current = self.updates.borrow().clone();
        let refusal = travel_blocker(&current).map_err(actor::unavailable)?;
        let response = |line: String, disposition: rpc::DialogueDisposition, revision| {
            DialogueReply::Raw(
                rpc::DialogueResponse {
                    input_id: request.input_id.clone(),
                    disposition: disposition as i32,
                    revision: Some(wire::revision(revision)),
                    npc_reply: line,
                    ..Default::default()
                },
                None,
            )
        };
        if let Some(line) = refusal {
            freeform_action::publish_hero_line(&self.table_log, &hero, hero_index, &request.text);
            freeform_action::publish_outcome(
                &self.table_log,
                hero_index,
                None::<super::table_log::RollRow>,
                line,
            );
            return Ok(response(
                line.to_owned(),
                rpc::DialogueDisposition::Rejected,
                self.updates.borrow().basis().revision,
            ));
        }
        let choices = self
            .choices(scene)
            .ok_or_else(|| tonic::Status::unavailable("the current routes are unavailable"))?;
        let target = match self.heading(&request.text, &hero, &choices).await {
            Some(target) => target,
            None => {
                if let Some(response) = self
                    .local_reference_call(credential, request, scene, false)
                    .await?
                {
                    return Ok(DialogueReply::Raw(response, None));
                }
                let line = "You stay together here; no way onward has been chosen.";
                freeform_action::publish_hero_line(
                    &self.table_log,
                    &hero,
                    hero_index,
                    &request.text,
                );
                freeform_action::publish_outcome(
                    &self.table_log,
                    hero_index,
                    None::<super::table_log::RollRow>,
                    line,
                );
                return Ok(response(
                    line.into(),
                    rpc::DialogueDisposition::Rejected,
                    self.updates.borrow().basis().revision,
                ));
            }
        };
        let operation = df_api::operation_id(request.input_id.as_ref())
            .map_err(|_| tonic::Status::invalid_argument("travel input identity required"))?;
        if self.updates.borrow().basis() != expected {
            return Err(tonic::Status::aborted(
                "the departure's scene changed while choosing a route",
            ));
        }
        let moved = match self
            .move_party_toward(credential, operation, target, &request.text)
            .await
        {
            Ok(moved) => moved,
            Err(MoveError::Status(error)) => return Err(error),
            Err(MoveError::Refused(reason)) => {
                let line = super::actor::place_actor::travel_refusal_line(reason);
                freeform_action::publish_hero_line(
                    &self.table_log,
                    &hero,
                    hero_index,
                    &request.text,
                );
                freeform_action::publish_outcome(
                    &self.table_log,
                    hero_index,
                    None::<super::table_log::RollRow>,
                    line,
                );
                return Ok(response(
                    line.to_owned(),
                    rpc::DialogueDisposition::Rejected,
                    self.updates.borrow().basis().revision,
                ));
            }
        };
        freeform_action::publish_hero_line(&self.table_log, &hero, hero_index, &request.text);
        freeform_action::publish_outcome(
            &self.table_log,
            hero_index,
            None::<super::table_log::RollRow>,
            &moved.line,
        );
        Ok(response(
            moved.line,
            rpc::DialogueDisposition::Action,
            moved.revision,
        ))
    }

    /// The current place, its exits and every place the run knows, for the dev CLI.
    pub(super) fn place_report(&self) -> Result<Value, tonic::Status> {
        let current = self.updates.borrow().clone();
        report(&current).map_err(actor::unavailable)
    }

    /// Queues future media through the existing plate and place-music owners.
    fn prepare_place_media(
        &self,
        scene: Result<pack::Scene, df_session::submission::RepositoryError>,
        source: super::living_scenes::PlateAheadSource,
    ) {
        if super::adventure::free_roam::world().is_some()
            && let Ok(scene) = &scene
        {
            let place_id = scene
                .setting
                .as_ref()
                .map_or("none", |setting| setting.place_id.as_str());
            let outcome = match super::place_music::installed() {
                Some(music) => match music.prepare_scene(scene) {
                    Ok(Some(plan)) => format!("outcome=planned plan_state={}", plan.state),
                    Ok(None) => "outcome=no_plan".to_owned(),
                    Err(error) => format!("outcome=projection_failed reason={error:?}"),
                },
                None => "outcome=disabled".to_owned(),
            };
            actor::record_audit_line(
                "gameplay.free_roam",
                format!("FREE_ROAM ahead_media place={place_id} kind=music_ambience {outcome}"),
            );
        }
        self.living.plan_plate(scene, source);
    }

    /// Restores speculative neighbours for the current saved place without making a move.
    /// Startup can call this after constructing the service; travel retries an interrupted warm-up.
    pub(super) async fn prepare_free_roam_current(&self) -> Result<(), tonic::Status> {
        let Some(world) = super::adventure::free_roam::world() else {
            return Ok(());
        };
        let current = self.updates.borrow().clone();
        let scene = beat_script_runtime::scene(current.state()).map_err(actor::unavailable)?;
        let setting = scene
            .setting
            .as_ref()
            .ok_or_else(|| tonic::Status::unavailable("free-roam current place absent"))?;
        let record = if setting.place_id == world.gate.place_id {
            world.gate
        } else {
            place::created(current.state())
                .map_err(actor::unavailable)?
                .remove(&setting.place_id)
                .ok_or_else(|| {
                    tonic::Status::unavailable("free-roam current place record absent")
                })?
        };
        self.start_free_roam_neighbours(current, scene, record, move_started(), None);
        Ok(())
    }

    /// The speaking hero of a seat and their table index.
    pub(super) async fn speaking_hero(&self, credential: [u8; 32]) -> Option<(String, u32)> {
        let (reply, wait) = oneshot::channel();
        self.actor
            .try_submit(actor::Call::SpeechMember { credential, reply })
            .ok()?;
        let member = tokio::time::timeout(timing::active().reply, wait)
            .await
            .ok()?
            .ok()?
            .ok()?;
        let current = self.updates.borrow().clone();
        let state = current.state();
        let entity = journey::player_entity(member, &current).ok()?;
        let name = journey::name(state, entity).ok()?;
        let index = state
            .characters
            .iter()
            .position(|character| character.entity == entity)
            .and_then(|position| u32::try_from(position).ok())?;
        Some((name, index))
    }

    /// What Jev chooses among in the place the party stands in now: its open exits, the places
    /// the run has that no open exit already leads to, and its landmarks not yet made.
    fn choices(&self, scene: &pack::Scene) -> Option<Choices> {
        let current = self.updates.borrow().clone();
        let state = current.state();
        let mut snapshot = journey::living_scene::snapshot(state, scene)
            .ok()
            .flatten()?;
        let party = place::whereabouts(state, scene).ok()?;
        place::name_bound_exits(state, &party.here, &mut snapshot.model.ways_out).ok()?;
        let exits: Vec<SceneWayOut> = snapshot
            .model
            .ways_out
            .iter()
            .filter(|way| way.open)
            .cloned()
            .collect();
        let reached: Vec<String> = exits
            .iter()
            .filter_map(|exit| {
                match place::destination(state, &party.home, &party.here, exit).ok()? {
                    Destination::Home => Some(party.home.clone()),
                    Destination::Created(known) => Some(known.place_id),
                    _ => None,
                }
            })
            .collect();
        let known: Vec<KnownPlace> = place::reachable(state, &party)
            .ok()?
            .into_iter()
            .filter(|place| !reached.contains(&place.id))
            .collect();
        let landmarks = place::landmarks_here(state, &party.here).ok()?;
        let setting = scene.setting.as_ref()?;
        Some(Choices {
            exits,
            known,
            landmarks,
            place_name: setting.name.clone(),
            place_short: setting.short.clone(),
            here: party.here,
        })
    }

    /// Jev's choice of where the line takes the party, audited as `PLACE judged`.
    async fn heading(&self, words: &str, hero: &str, choices: &Choices) -> Option<MoveTarget> {
        let (_, jev) = self.npc_voice.hero_identity_models();
        let exits: Vec<&SceneWayOut> = choices.exits.iter().collect();
        let listed = HeadingChoices {
            exits: &exits,
            known: &choices.known,
            landmarks: &choices.landmarks,
        };
        let heading = match place_creation::judge_heading(
            jev,
            words,
            hero,
            &choices.place_name,
            &choices.place_short,
            &listed,
        )
        .await
        {
            Ok(heading) => heading,
            Err(reason) => {
                place::audit_judged(
                    &choices.here,
                    "unavailable",
                    &format!(" reason={}", reason.label()),
                );
                return None;
            }
        };
        place::audit_judged(&choices.here, &heading.label(&listed), "");
        choices.target(heading)
    }

    /// A hero's freeform act that Jev read as a move: when it succeeded and Jev finds where the
    /// line goes (an exit, a place the run has, a landmark, or a destination the hero names), the
    /// party moves there. `None` leaves the act to the ordinary narration: nothing moved, so
    /// nothing is claimed.
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
        match travel_blocker(&current) {
            Ok(Some(line)) => {
                freeform_action::publish_outcome(
                    &self.table_log,
                    hero_index,
                    None::<super::table_log::RollRow>,
                    line,
                );
                return Some(self.rejected_reply(request, line));
            }
            Err(error) => {
                actor::record_unavailable("travel-blocker", &format!("{error:?}"));
                return None;
            }
            Ok(None) => {}
        }
        let choices = self.choices(scene)?;
        let target = match self.heading(&request.text, hero, &choices).await {
            Some(target) => target,
            None => {
                return match self
                    .local_reference_call(credential, request, scene, true)
                    .await
                {
                    Ok(Some(response)) => Some(DialogueReply::Raw(response, None)),
                    Ok(None) => None,
                    Err(error) => {
                        actor::record_unavailable("local-spot-move", error.message());
                        None
                    }
                };
            }
        };
        let operation = match df_api::operation_id(request.input_id.as_ref()) {
            Ok(operation) => operation,
            Err(error) => {
                actor::record_unavailable("freeform-travel", &format!("{error:?}"));
                return Some(self.move_not_recorded(request, hero_index));
            }
        };
        let moved = match self
            .move_party_toward(credential, operation, target, &request.text)
            .await
        {
            Ok(moved) => moved,
            Err(MoveError::Refused(reason)) => {
                // A refused destination is told in the fiction; returning None here would let the
                // ordinary success narration claim the party left.
                let line = super::actor::place_actor::travel_refusal_line(reason);
                freeform_action::publish_outcome(
                    &self.table_log,
                    hero_index,
                    None::<super::table_log::RollRow>,
                    line,
                );
                return Some(self.rejected_reply(request, line));
            }
            Err(MoveError::Status(status)) => {
                actor::record_unavailable(
                    "freeform-travel",
                    &format!("{:?}: {}", status.code(), status.message()),
                );
                return Some(self.move_not_recorded(request, hero_index));
            }
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

    /// A line that hails or hires a ride to another place, as Jev reads it. Jev judges the line
    /// first, so ordinary chatter is never answered as a ride. Then the travel restriction is
    /// checked before the fare is judged, and again after the destination is made, so a restriction
    /// that starts in between buys nothing. The result is a `Ride` with a fare when the line is a
    /// ride the restriction allows, or a `Ride` with no fare and a `Blocked` journey when it does
    /// not. `None` when the line is no ride, names nowhere to go, or the destination cannot be made;
    /// no fare is bought in any `None` case.
    pub(super) async fn hailed_ride(
        &self,
        credential: [u8; 32],
        request: &rpc::RawDialogueRequest,
        scene: &pack::Scene,
    ) -> Option<Ride> {
        let (hero, hero_index) = self.speaking_hero(credential).await?;
        let choices = self.choices(scene)?;
        let (_, jev) = self.npc_voice.hero_identity_models();
        match place_creation::judge_ride(jev, &request.text, &hero, &choices.place_name, None).await
        {
            Ok(true) => {}
            Ok(false) => return None,
            Err(reason) => {
                actor::record_audit_line(
                    "gameplay.place",
                    format!(
                        "PLACE ride from={} source=unavailable reason={}",
                        choices.here,
                        reason.label()
                    ),
                );
                return None;
            }
        }
        if let Some(blocked) = self.blocked_ride(hero.clone(), hero_index) {
            return Some(blocked);
        }
        let current = self.updates.borrow().clone();
        let model = journey::living_scene::snapshot(current.state(), scene)
            .ok()
            .flatten()?
            .model;
        let provider_name = |id: &str| {
            model
                .people
                .iter()
                .find(|person| person.id == id)
                .map_or_else(|| id.to_owned(), |person| person.name.clone())
        };
        let providers: Vec<String> = model
            .services
            .iter()
            .map(|service| provider_name(&service.provider_id))
            .collect();
        let offers: Vec<ServiceOffer<'_>> = model
            .services
            .iter()
            .zip(&providers)
            .map(|(service, provider)| ServiceOffer {
                name: &service.name,
                description: &service.description,
                provider,
            })
            .collect();
        let fare = match place_creation::judge_ride_service(jev, &request.text, &offers).await {
            Ok(Some(index)) => model.services.get(index).map(|service| RideFare {
                service_id: service.id.clone(),
                price_cp: service.price_cp,
                name: service.name.clone(),
                provider: provider_name(&service.provider_id),
            }),
            Ok(None) | Err(_) => None,
        };
        actor::record_audit_line(
            "gameplay.place",
            format!(
                "PLACE ride from={} source=jev fare={}",
                choices.here,
                fare.as_ref()
                    .map_or("none", |fare| fare.service_id.as_str())
            ),
        );
        let target = self.heading(&request.text, &hero, &choices).await?;
        let operation = df_api::operation_id(request.input_id.as_ref()).ok()?;
        // A paid fare commits under the line's own operation; the move follows under its own.
        let operation = if fare.is_some() {
            following_operation(operation)?
        } else {
            operation
        };
        let prepared = self
            .prepare_move(operation, target, &request.text)
            .await
            .ok()?;
        if let Some(blocked) = self.blocked_ride(hero.clone(), hero_index) {
            drop(prepared);
            return Some(blocked);
        }
        Some(Ride {
            journey: RideJourney::Ready(Box::new(prepared)),
            fare,
            hero,
            hero_index,
        })
    }

    /// The restriction in force now, as the ride it refuses: no fare and no journey. A restriction
    /// that cannot be read refuses too, since nothing is bought on an unknown state; that case is
    /// audited as unavailable. `None` when the party may set off.
    pub(super) fn blocked_ride(&self, hero: String, hero_index: u32) -> Option<Ride> {
        let refusal = match travel_blocker(&self.updates.borrow()) {
            Ok(None) => return None,
            Ok(Some(line)) => Refusal {
                line,
                reason: "travel_blocked",
            },
            Err(error) => {
                actor::record_unavailable("ride-restriction", &format!("{error:?}"));
                Refusal {
                    line: MOVE_NOT_RECORDED,
                    reason: "restriction_unreadable",
                }
            }
        };
        Some(Ride {
            journey: RideJourney::Blocked(refusal),
            fare: None,
            hero,
            hero_index,
        })
    }

    /// The last check before a fare is bought: a ride whose restriction now holds is returned as
    /// the blocked ride, and the purchase does not happen. Called by the dialogue boundary.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "called by the dialogue boundary before the purchase; the dialogue.rs change is filed for N"
        )
    )]
    pub(super) fn recheck_before_fare(&self, ride: Ride) -> Ride {
        let Ride {
            journey,
            fare,
            hero,
            hero_index,
        } = ride;
        match self.blocked_ride(hero.clone(), hero_index) {
            Some(blocked) => blocked,
            None => Ride {
                journey,
                fare,
                hero,
                hero_index,
            },
        }
    }

    /// The rejected answer to a line whose move did not happen. Nothing moved, so the answer
    /// cannot be mistaken for the ordinary narration of a move.
    fn rejected_reply(&self, request: &rpc::RawDialogueRequest, line: &str) -> DialogueReply {
        DialogueReply::Raw(
            rpc::DialogueResponse {
                input_id: request.input_id.clone(),
                disposition: rpc::DialogueDisposition::Rejected as i32,
                revision: Some(wire::revision(self.updates.borrow().basis().revision)),
                npc_reply: line.to_owned(),
                ..Default::default()
            },
            None,
        )
    }

    /// A chosen move that could not be recorded: the table hears it and the party stays put.
    fn move_not_recorded(
        &self,
        request: &rpc::RawDialogueRequest,
        hero_index: u32,
    ) -> DialogueReply {
        freeform_action::publish_outcome(
            &self.table_log,
            hero_index,
            None::<super::table_log::RollRow>,
            MOVE_NOT_RECORDED,
        );
        self.rejected_reply(request, MOVE_NOT_RECORDED)
    }

    /// Takes a ride the hero hailed or hired and says which way it went. The table hears the hero's
    /// line (unless an interaction already showed it), then either the travel beat and the arrival
    /// or the refusal. A fare paid is said to stand. Arrival is decided from the committed state,
    /// never from a revision change alone.
    pub(super) async fn take_ride_outcome(
        &self,
        credential: [u8; 32],
        request: &rpc::RawDialogueRequest,
        ride: Ride,
        line_shown: bool,
    ) -> RideOutcome {
        let key = audit_key(request);
        let Ride {
            journey,
            fare,
            hero,
            hero_index,
        } = ride;
        if !line_shown {
            freeform_action::publish_hero_line(&self.table_log, &hero, hero_index, &request.text);
        }
        let prepared = match journey {
            RideJourney::Ready(prepared) => *prepared,
            RideJourney::Blocked(refusal) => {
                return self.ride_not_taken(
                    hero_index,
                    fare.as_ref(),
                    refusal.line,
                    refusal.reason,
                    key,
                );
            }
        };
        let from_name = prepared.from_name.clone();
        let to_short = prepared.to_short.clone();
        let to = prepared.to.clone();
        let started = prepared.started;
        let operation = df_observe::operation_hash(prepared.operation.as_bytes());
        let arrival = match self.commit_prepared(credential, prepared).await {
            Ok(moved) => {
                let moved = *moved;
                Arrival {
                    place_name: moved.place_name,
                    line: moved.line,
                    revision: moved.revision,
                }
            }
            Err(CommitFailure::Blocked(reason)) => {
                return self.ride_not_taken(
                    hero_index,
                    fare.as_ref(),
                    reason,
                    "travel_blocked",
                    key,
                );
            }
            Err(CommitFailure::Move(MoveError::Refused(reason))) => {
                return self.ride_not_taken(
                    hero_index,
                    fare.as_ref(),
                    actor::place_actor::travel_refusal_line(reason),
                    reason.label(),
                    key,
                );
            }
            Err(CommitFailure::Move(MoveError::Status(status))) => {
                let code = format!("{:?}", status.code());
                actor::record_unavailable("ride-travel", &format!("{code}: {}", status.message()));
                // The reply was lost; the committed state says whether the party moved.
                match self.arrived(&to) {
                    Ok(false) => {
                        return self.ride_not_taken(
                            hero_index,
                            fare.as_ref(),
                            MOVE_NOT_RECORDED,
                            &code,
                            key,
                        );
                    }
                    Err(error) => {
                        actor::record_unavailable("ride-arrival", &format!("{error:?}"));
                        return self.ride_not_taken(
                            hero_index,
                            fare.as_ref(),
                            MOVE_NOT_RECORDED,
                            "arrival_unreadable",
                            key,
                        );
                    }
                    Ok(true) => {}
                }
                let state = self.updates.borrow();
                let place_name = match place::name_of(state.state(), &to) {
                    Ok(place_name) => place_name,
                    Err(error) => {
                        drop(state);
                        actor::record_unavailable("ride-arrival", &format!("{error:?}"));
                        return self.ride_not_taken(
                            hero_index,
                            fare.as_ref(),
                            MOVE_NOT_RECORDED,
                            "arrival_unnamed",
                            key,
                        );
                    }
                };
                let revision = state.basis().revision;
                drop(state);
                // The owner committed the move, so the work a move starts must still run.
                self.start_after_move(&to, started, operation);
                actor::record_audit_line(
                    "gameplay.place",
                    format!(
                        "PLACE ride outcome=arrived_after_lost_reply key={key} operation_hash={operation}"
                    ),
                );
                Arrival {
                    line: place::journey_line(&hero, &place_name),
                    place_name,
                    revision,
                }
            }
        };
        let (storyteller, _) = self.npc_voice.hero_identity_models();
        let beat = place_creation::narrate_travel(
            storyteller,
            &TravelBeat {
                hero: &hero,
                words: &request.text,
                from: &from_name,
                to: &arrival.place_name,
                to_short: &to_short,
                ride: true,
                hired: fare
                    .as_ref()
                    .map(|fare| (fare.name.as_str(), fare.provider.as_str())),
            },
        )
        .await;
        freeform_action::publish_outcome(
            &self.table_log,
            hero_index,
            None::<super::table_log::RollRow>,
            &beat,
        );
        freeform_action::publish_outcome(
            &self.table_log,
            hero_index,
            None::<super::table_log::RollRow>,
            &arrival.line,
        );
        RideOutcome::Travelled {
            line: format!("{beat} {}", arrival.line),
            revision: arrival.revision,
        }
    }

    /// Whether the party stands at `to` in the committed state. Only a committed move from the
    /// prepared origin brings it there, so unlike a revision change this cannot come from another
    /// hero's action.
    fn arrived(&self, to: &str) -> Result<bool, df_session::submission::RepositoryError> {
        let current = self.updates.borrow().clone();
        let scene = beat_script_runtime::scene(current.state())?;
        Ok(place::whereabouts(current.state(), &scene)?.here == to)
    }

    /// The journey did not start: the refusal is audited, keyed to its input, and told to the table.
    /// A paid fare is said to stand, and the purse fact is audited with it.
    fn ride_not_taken(
        &self,
        hero_index: u32,
        fare: Option<&RideFare>,
        refusal: &str,
        reason: &str,
        key: u64,
    ) -> RideOutcome {
        actor::record_audit_line(
            "gameplay.place",
            format!("PLACE ride outcome=not_taken reason={reason} key={key}"),
        );
        let line = match fare {
            Some(fare) => {
                audit_fare_kept(&fare.service_id, fare.price_cp, reason, key);
                format!("{refusal} {FARE_KEPT}")
            }
            None => refusal.to_owned(),
        };
        freeform_action::publish_outcome(
            &self.table_log,
            hero_index,
            None::<super::table_log::RollRow>,
            &line,
        );
        RideOutcome::NotTaken {
            line,
            revision: self.updates.borrow().basis().revision,
        }
    }

    /// The reply the dialogue boundary answers a hailed ride with: travel is an action, a journey
    /// that did not start is rejected, and its text is the one the table was told.
    pub(super) async fn take_ride(
        &self,
        credential: [u8; 32],
        request: &rpc::RawDialogueRequest,
        ride: Ride,
        line_shown: bool,
    ) -> Option<DialogueReply> {
        let (disposition, line, revision) = match self
            .take_ride_outcome(credential, request, ride, line_shown)
            .await
        {
            RideOutcome::Travelled { line, revision } => {
                (rpc::DialogueDisposition::Action, line, revision)
            }
            RideOutcome::NotTaken { line, revision } => {
                (rpc::DialogueDisposition::Rejected, line, revision)
            }
        };
        Some(DialogueReply::Raw(
            rpc::DialogueResponse {
                input_id: request.input_id.clone(),
                disposition: disposition as i32,
                revision: Some(wire::revision(revision)),
                npc_reply: line,
                ..Default::default()
            },
            None,
        ))
    }

    /// After a listed paid service of the place is bought by a hero's line, when Jev finds that
    /// buying it is taking a ride now: the ride is judged and prepared before any purchase, so the
    /// purchase is the only step left and nothing after payment can end silently. `None` only
    /// before any purchase: the line is no ride, or no route is judged or prepared.
    /// A restriction is checked after the destination is made, as for a hailed ride. The purchase
    /// itself is the caller's step: it must re-check with `recheck_before_fare` first (TODO(TRV-01)).
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "TODO(TRV-01): wired when the service reply buys between prepare and take"
        )
    )]
    pub(super) async fn prepare_service_ride(
        &self,
        credential: [u8; 32],
        request: &rpc::RawDialogueRequest,
        scene: &pack::Scene,
        service_id: &str,
    ) -> Option<Ride> {
        let (hero, hero_index) = self.speaking_hero(credential).await?;
        let choices = self.choices(scene)?;
        let current = self.updates.borrow().clone();
        let model = journey::living_scene::snapshot(current.state(), scene)
            .ok()
            .flatten()?
            .model;
        let service = model
            .services
            .iter()
            .find(|service| service.id == service_id)?;
        let provider = model
            .people
            .iter()
            .find(|person| person.id == service.provider_id)
            .map_or_else(|| service.provider_id.clone(), |person| person.name.clone());
        let (_, jev) = self.npc_voice.hero_identity_models();
        let offer = ServiceOffer {
            name: &service.name,
            description: &service.description,
            provider: &provider,
        };
        match place_creation::judge_ride(
            jev,
            &request.text,
            &hero,
            &choices.place_name,
            Some(&offer),
        )
        .await
        {
            Ok(true) => {}
            Ok(false) => return None,
            Err(reason) => {
                actor::record_audit_line(
                    "gameplay.place",
                    format!(
                        "PLACE ride service={service_id} source=unavailable reason={}",
                        reason.label()
                    ),
                );
                return None;
            }
        }
        let target = self.heading(&request.text, &hero, &choices).await?;
        let operation = following_operation(df_api::operation_id(request.input_id.as_ref()).ok()?)?;
        let prepared = self
            .prepare_move(operation, target, &request.text)
            .await
            .ok()?;
        if let Some(blocked) = self.blocked_ride(hero.clone(), hero_index) {
            drop(prepared);
            return Some(blocked);
        }
        Some(Ride {
            journey: RideJourney::Ready(Box::new(prepared)),
            fare: Some(RideFare {
                service_id: service.id.clone(),
                price_cp: service.price_cp,
                name: service.name.clone(),
                provider,
            }),
            hero,
            hero_index,
        })
    }

    /// Moves the party through `exit_id` of the place it stands in. The place is created first
    /// when the run does not have it yet; the owner then repeats every check and commits.
    #[cfg(test)]
    pub(super) async fn move_party_through(
        &self,
        credential: [u8; 32],
        operation: OperationId,
        exit_id: &str,
    ) -> Result<Box<Moved>, MoveError> {
        self.move_party_toward(
            credential,
            operation,
            MoveTarget::Exit(exit_id.to_owned()),
            "",
        )
        .await
    }

    /// Moves the party to `target`: through an exit, or on a journey. `goal` is the hero's own
    /// line, given to the storyteller as quoted data when the destination is made.
    pub(super) async fn move_party_toward(
        &self,
        credential: [u8; 32],
        operation: OperationId,
        target: MoveTarget,
        goal: &str,
    ) -> Result<Box<Moved>, MoveError> {
        if let Some(reason) = travel_blocker(&self.updates.borrow()).map_err(state_failure)? {
            return Err(tonic::Status::failed_precondition(reason).into());
        }
        let prepared = self.prepare_move(operation, target, goal).await?;
        self.commit_prepared(credential, prepared)
            .await
            .map_err(MoveError::from)
    }

    /// Prepares the forecast's ordinary exit destination without committing a separate move.
    /// The assault owner validates and journals the retained destination with the fight.
    pub(super) async fn prepare_fight_arena(
        &self,
        current: &Checkpoint,
        operation: OperationId,
    ) -> Result<(), MoveError> {
        if super::adventure::free_roam::world().is_some() {
            return Ok(());
        }
        // Forecast and opening share one owned gate; waiting for preparation never commits
        // movement and cancellation releases the guard automatically.
        let _preparation = super::fight_foresight::preparation_guard().await;
        let scene = beat_script_runtime::scene(current.state()).map_err(state_failure)?;
        let source = place::whereabouts(current.state(), &scene)
            .map_err(state_failure)?
            .here;
        let Some(exit) = super::fight_foresight::proposed_exit(&source) else {
            return Ok(());
        };
        let source_matches = |latest: &Checkpoint| {
            if latest.basis().run != current.basis().run {
                return false;
            }
            let Ok(scene) = beat_script_runtime::scene(latest.state()) else {
                return false;
            };
            let Ok(Some(snapshot)) = journey::living_scene::snapshot(latest.state(), &scene) else {
                return false;
            };
            scene
                .setting
                .as_ref()
                .is_some_and(|setting| setting.place_id == source)
                && snapshot
                    .model
                    .ways_out
                    .iter()
                    .any(|way| way.open && way.id == exit)
                && super::fight_foresight::proposed_exit(&source).as_deref() == Some(exit.as_str())
        };
        if !source_matches(&self.updates.borrow()) {
            return Err(tonic::Status::failed_precondition("fight source changed").into());
        }
        if let Some(record) = super::fight_foresight::proposed_destination(&source) {
            self.living.prepare_arena_photograph(&record)?;
            return Ok(());
        }
        let mut prepared = tokio::time::timeout(
            CREATION_DEADLINE,
            self.prepare_move(operation, MoveTarget::Exit(exit.clone()), ""),
        )
        .await
        .map_err(|_| {
            audit_refusal(&source, &exit, "arena_creation_deadline");
            MoveError::Refused(PlaceRefusal::CreationUnavailable)
        })??;
        if let Some(cancellation) = &mut prepared.cancellation {
            cancellation.active = false;
        }
        if !source_matches(&self.updates.borrow()) || prepared.from != source {
            return Err(tonic::Status::failed_precondition("fight source changed").into());
        }
        if let Some(record) = prepared.created {
            if !super::fight_foresight::retain_destination(&source, record.clone()) {
                audit_refusal(&source, &exit, "arena_destination_not_retained");
                return Err(PlaceRefusal::CreationUnavailable.into());
            }
            self.living.prepare_arena_photograph(&record)?;
        }
        Ok(())
    }

    /// Resolves where a move goes and makes the place when it is new; nothing is committed here.
    async fn prepare_move(
        &self,
        operation: OperationId,
        target: MoveTarget,
        goal: &str,
    ) -> Result<PreparedMove, MoveError> {
        let started = move_started();
        let mut cancellation = MoveCancellation {
            started,
            operation,
            active: true,
        };
        let result = self
            .prepare_move_inner(operation, target, goal, started)
            .await;
        match result {
            Ok(mut prepared) => {
                prepared.cancellation = Some(cancellation);
                Ok(prepared)
            }
            Err(error) => {
                cancellation.active = false;
                Err(error)
            }
        }
    }

    async fn prepare_move_inner(
        &self,
        operation: OperationId,
        target: MoveTarget,
        goal: &str,
        started: Instant,
    ) -> Result<PreparedMove, MoveError> {
        let current = self.updates.borrow().clone();
        let state = current.state();
        let scene = beat_script_runtime::scene(state).map_err(state_failure)?;
        let snapshot = journey::living_scene::snapshot(state, &scene)
            .map_err(state_failure)?
            .ok_or(PlaceRefusal::NoExit)?;
        let party = place::whereabouts(state, &scene).map_err(state_failure)?;
        let from_name = scene
            .setting
            .as_ref()
            .map_or_else(|| scene.title.clone(), |setting| setting.name.clone());
        let spot = match &target {
            MoveTarget::Spot(spot) => Some(spot.clone()),
            _ => None,
        };
        let (exit, destination, landmark) = match target {
            MoveTarget::Exit(exit_id) => {
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
                let destination = place::destination(state, &party.home, &party.here, &exit)
                    .map_err(state_failure)?;
                (Some(exit), destination, None)
            }
            MoveTarget::Known(place_id) => {
                match place::destination_of(state, &party.home, &place_id).map_err(state_failure)? {
                    // A journey by name reaches only a place the run has; it makes nothing.
                    Destination::Create { .. } => return Err(PlaceRefusal::InvalidPlace.into()),
                    destination => (None, destination, None),
                }
            }
            MoveTarget::Landmark(landmark) => {
                (None, Destination::Create { named: None }, Some(landmark))
            }
            MoveTarget::Spot(spot) => {
                let existing = place::spots_here(state, &party.here)
                    .map_err(state_failure)?
                    .into_iter()
                    .find(|known| place::name_key(&known.name) == place::name_key(&spot.name));
                let destination = match existing {
                    Some(known) => place::destination_of(state, &party.home, &known.id)
                        .map_err(state_failure)?,
                    None => Destination::Create { named: None },
                };
                (None, destination, Some(spot))
            }
            MoveTarget::New => (None, Destination::Create { named: None }, None),
        };
        let exit_id = spot.as_ref().map_or_else(
            || {
                exit.as_ref()
                    .map_or_else(|| JOURNEY_EXIT.to_owned(), |exit| exit.id.clone())
            },
            |spot| place::spot_exit(&spot.name),
        );
        let (to, created) = match destination {
            Destination::Home => (party.home.clone(), None),
            Destination::Created(known) => (known.place_id, None),
            Destination::Story { .. } => {
                audit_refusal(&party.here, &exit_id, PlaceRefusal::StoryPlace.label());
                return Err(PlaceRefusal::StoryPlace.into());
            }
            Destination::Create { named } => {
                if place::created(state).map_err(state_failure)?.len()
                    >= place::MAXIMUM_CREATED_PLACES
                {
                    audit_refusal(&party.here, &exit_id, PlaceRefusal::RunFull.label());
                    return Err(PlaceRefusal::RunFull.into());
                }
                let mut ready = super::adventure::free_roam::prepared(&current)
                    .into_iter()
                    .find(|ready| {
                        ready.from == party.here
                            && (ready.exit == exit_id
                                || landmark.as_ref().is_some_and(|landmark| {
                                    place::name_key(&landmark.name)
                                        == place::name_key(&ready.place.name)
                                }))
                    });
                if ready.is_none()
                    && let Some(record) = self
                        .await_free_roam_destination(&current, &party.here, &exit_id)
                        .await?
                {
                    ready = Some(super::adventure::free_roam::PreparedPlace {
                        from: party.here.clone(),
                        exit: exit_id.clone(),
                        place: record,
                    });
                }
                let mut request = self
                    .creation_request(
                        &current,
                        &scene,
                        &snapshot.model,
                        exit.as_ref(),
                        named,
                        goal,
                        landmark,
                    )
                    .await
                    .map_err(state_failure)?;
                if let Some(spot) = &spot {
                    if let Some(origin) = place::created(state)
                        .map_err(state_failure)?
                        .get(&party.here)
                        .cloned()
                        .or_else(|| {
                            super::adventure::free_roam::world()
                                .filter(|world| world.gate.place_id == party.here)
                                .map(|world| world.gate)
                        })
                        && let Some(stall) = origin.stalls.iter().find(|stall| {
                            place::name_key(&stall.name) == place::name_key(&spot.name)
                        })
                        && let Some(keeper) = origin
                            .people
                            .iter()
                            .find(|person| person.name == stall.keeper)
                    {
                        request.canon["local_stall"] = json!({"stall":stall,"keeper":keeper,
                            "wares":origin.wares.iter().filter(|ware| ware.seller == keeper.name).collect::<Vec<_>>()});
                    }
                    request.canon["local_spot"] = json!({
                        "name": spot.name,
                        "look": spot.direction,
                        "parent_place": party.here,
                        "parent_name": from_name,
                        "arrival_question": goal.chars().take(500).collect::<String>(),
                        "picture_evidence": journey::living_scene::picture_features(state, &party.here)
                            .map_err(state_failure)?.into_iter().filter(|record| {
                                record.seen_in_picture.iter().any(|feature| {
                                    place::name_key(&feature.name) == place::name_key(&spot.name)
                                })
                            }).collect::<Vec<_>>(),
                    });
                }
                let free_roam = super::adventure::free_roam::world();
                let ahead = super::adventure::free_roam::prepared(&current);
                if let Some(world) = &free_roam {
                    request.adventure = world.city.description();
                    request.canon["city"] = json!(world.city);
                    request
                        .place_names
                        .extend(ahead.iter().map(|ready| ready.place.name.clone()));
                    request.people_names.extend(ahead.iter().flat_map(|ready| {
                        ready.place.people.iter().map(|person| person.name.clone())
                    }));
                    request
                        .taken_ids
                        .extend(ahead.iter().map(|ready| ready.place.place_id.clone()));
                }
                let (storyteller, persona_model) = self.npc_voice.hero_identity_models();
                let made = match ready {
                    Some(ready) => Ok(Ok(ready.place)),
                    None => {
                        tokio::time::timeout(CREATION_DEADLINE, async {
                            if free_roam.is_some() || spot.is_some() {
                                place_creation::create_silent(storyteller, persona_model, &request)
                                    .await
                            } else {
                                place_creation::create(storyteller, persona_model, &request).await
                            }
                        })
                        .await
                    }
                };
                let record = match made {
                    Ok(Ok(record)) => record,
                    Ok(Err(error)) => {
                        audit_refusal(&party.here, &exit_id, error.label());
                        return Err(PlaceRefusal::CreationUnavailable.into());
                    }
                    Err(_) => {
                        audit_refusal(&party.here, &exit_id, "creation_deadline");
                        return Err(PlaceRefusal::CreationUnavailable.into());
                    }
                };
                (record.place_id.clone(), Some(record))
            }
        };
        let known_places = place::created(state).map_err(state_failure)?;
        let destination_record = created
            .clone()
            .or_else(|| known_places.get(&to).cloned())
            .or_else(|| {
                super::adventure::free_roam::world()
                    .filter(|world| world.gate.place_id == to)
                    .map(|world| world.gate)
            });
        if let Some(record) = &destination_record {
            self.prepare_place_media(
                destination_scene(&current, record, &scene),
                super::living_scenes::PlateAheadSource::Generated,
            );
        }
        let to_short = match &created {
            Some(record) => record.short.clone(),
            None => place::created(state)
                .map_err(state_failure)?
                .remove(&to)
                .map(|record| record.short)
                .unwrap_or_default(),
        };
        Ok(PreparedMove {
            started,
            cancellation: None,
            operation,
            expected_basis: current.basis(),
            from: party.here,
            from_name,
            exit: exit_id,
            to,
            to_short,
            created,
        })
    }

    async fn commit_prepared(
        &self,
        credential: [u8; 32],
        mut prepared: PreparedMove,
    ) -> Result<Box<Moved>, CommitFailure> {
        let blocker = travel_blocker(&self.updates.borrow())
            .map_err(|error| CommitFailure::Move(state_failure(error)))?;
        if let Some(reason) = blocker {
            if let Some(cancellation) = &mut prepared.cancellation {
                cancellation.active = false;
            }
            return Err(CommitFailure::Blocked(reason));
        }
        let started = prepared.started;
        let mut cancellation = prepared.cancellation.take();
        let result = self
            .commit_move(
                credential,
                started,
                MoveProposal {
                    operation: prepared.operation,
                    expected_basis: prepared.expected_basis,
                    from: prepared.from,
                    exit: prepared.exit,
                    to: prepared.to,
                    created: prepared.created,
                },
            )
            .await;
        if let Some(cancellation) = &mut cancellation {
            cancellation.active = false;
        }
        result.map_err(CommitFailure::Move)
    }

    /// Moves the party straight to a place the run already has (development only: no exit is
    /// used, so nothing is created).
    #[cfg(test)]
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
            move_started(),
            MoveProposal {
                operation,
                expected_basis: current.basis(),
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
        started: Instant,
        proposal: MoveProposal,
    ) -> Result<Box<Moved>, MoveError> {
        let operation = df_observe::operation_hash(proposal.operation.as_bytes());
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
            MoveOutcome::Moved(mut moved) => {
                // The owner has already committed this move: a failed voice registration is its own
                // audit line, and the caller decides the arrival from the committed revision.
                if let Err(error) = self.voice.register_people(self.updates.borrow().state()) {
                    actor::record_unavailable("register-people", &format!("{error:?}"));
                    return Err(actor::unavailable(error).into());
                }
                if super::adventure::free_roam::world().is_some() {
                    moved.line = moved.narration.clone();
                }
                self.start_after_move(&moved.to, started, operation);
                Ok(moved)
            }
            MoveOutcome::Refused(reason) => Err(MoveError::Refused(reason)),
        }
    }

    fn start_after_move(&self, to: &str, started: Instant, operation: u64) {
        // Replacement cancels the old origin even when the new place has no onward exits.
        self.look_ahead.lock().take();
        let current = self.updates.borrow().clone();
        let Ok(scene) = beat_script_runtime::scene(current.state()) else {
            return;
        };
        let record = place::created(current.state())
            .ok()
            .and_then(|mut places| places.remove(to))
            .or_else(|| {
                super::adventure::free_roam::world()
                    .filter(|world| world.gate.place_id == to)
                    .map(|world| world.gate)
            });
        if let Some(record) = record {
            self.start_free_roam_neighbours(current, scene, record, started, Some(operation));
        }
    }

    async fn await_free_roam_destination(
        &self,
        current: &Checkpoint,
        from: &str,
        exit: &str,
    ) -> Result<Option<place::PlaceRecord>, MoveError> {
        let waiting = self.look_ahead.lock().as_ref().and_then(|batch| {
            (batch.origin == from
                && batch.basis.session == current.basis().session
                && batch.basis.run == current.basis().run)
                .then(|| {
                    batch
                        .exits
                        .iter()
                        .find(|entry| entry.id == exit)
                        .map(|entry| entry.outcome.subscribe())
                })
                .flatten()
        });
        let Some(mut waiting) = waiting else {
            return Ok(None);
        };
        tokio::time::timeout(CREATION_DEADLINE, async {
            loop {
                match waiting.borrow_and_update().clone() {
                    AheadOutcome::Ready(record) => return Ok(Some(record.as_ref().clone())),
                    AheadOutcome::Failed | AheadOutcome::Cancelled => {
                        return Err(MoveError::Refused(PlaceRefusal::CreationUnavailable));
                    }
                    AheadOutcome::Pending => {}
                }
                waiting
                    .changed()
                    .await
                    .map_err(|_| MoveError::Refused(PlaceRefusal::CreationUnavailable))?;
            }
        })
        .await
        .map_err(|_| MoveError::Refused(PlaceRefusal::CreationUnavailable))?
    }

    /// The owner retains the whole warm-up, including memory recall before generation.
    fn start_free_roam_neighbours(
        &self,
        current: Checkpoint,
        base: pack::Scene,
        origin: place::PlaceRecord,
        started: Instant,
        operation: Option<u64>,
    ) {
        if super::adventure::free_roam::world().is_none() {
            return;
        }
        let Ok(scene) = destination_scene(&current, &origin, &base) else {
            return;
        };
        let Some(model) = scene
            .setting
            .as_ref()
            .and_then(|setting| setting.model.as_ref())
        else {
            return;
        };
        let exits: Vec<_> = model
            .ways_out
            .iter()
            .filter(|exit| exit.open && exit.id != place::WAY_BACK)
            .take(2)
            .cloned()
            .collect();
        let mut owner = self.look_ahead.lock();
        if owner.as_ref().is_some_and(|batch| {
            batch.origin == origin.place_id
                && batch.basis.session == current.basis().session
                && batch.basis.run == current.basis().run
        }) {
            if let Some(batch) = owner.as_ref() {
                for exit in &batch.exits {
                    if let AheadOutcome::Ready(record) = &*exit.outcome.borrow() {
                        super::adventure::free_roam::remember(
                            &current,
                            super::adventure::free_roam::PreparedPlace {
                                from: origin.place_id.clone(),
                                exit: exit.id.clone(),
                                place: record.as_ref().clone(),
                            },
                        );
                    }
                }
            }
            return;
        }
        owner.take();
        let token = Arc::new(());
        let weak = Arc::downgrade(&self.look_ahead);
        // The worker owns data clones, but never the Arc that retains its own handle.
        let mut worker = self.clone();
        worker.look_ahead = Arc::default();
        let statuses: Vec<_> = exits
            .iter()
            .map(|exit| AheadExit {
                id: exit.id.clone(),
                outcome: watch::channel(AheadOutcome::Pending).0,
            })
            .collect();
        let destinations: Vec<_> = exits
            .into_iter()
            .zip(statuses.iter().map(|entry| entry.outcome.clone()))
            .collect();
        let job_token = token.clone();
        let job_origin = origin.clone();
        let basis = current.basis();
        let mut updates = self.updates.clone();
        let task = tokio::spawn(async move {
            let work = async {
                let mut pending: FuturesUnordered<_> = destinations
                    .into_iter()
                    .map(|(exit, outcome)| {
                        worker.generate_free_roam_neighbour(
                            &current,
                            &scene,
                            &job_origin,
                            exit,
                            outcome,
                            &weak,
                            &job_token,
                            started,
                            operation,
                        )
                    })
                    .collect();
                while pending.next().await.is_some() {}
            };
            tokio::select! {
                () = work => {}
                () = async { while updates.changed().await.is_ok() {} } => {
                    if let Some(owner) = weak.upgrade() {
                        let batch = owner.lock();
                        if let Some(batch) = batch.as_ref().filter(|batch| Arc::ptr_eq(&batch.token, &job_token)) {
                            batch.cancel_pending();
                        }
                    }
                }
            }
        });
        *owner = Some(AheadBatch {
            origin: origin.place_id,
            basis,
            token,
            started,
            operation,
            exits: statuses,
            task,
        });
    }

    #[allow(clippy::too_many_arguments)]
    async fn generate_free_roam_neighbour(
        &self,
        current: &Checkpoint,
        scene: &pack::Scene,
        origin: &place::PlaceRecord,
        exit: SceneWayOut,
        outcome: watch::Sender<AheadOutcome>,
        owner: &Weak<LookAhead>,
        token: &Arc<()>,
        started: Instant,
        operation: Option<u64>,
    ) {
        let mut cancellation = outcome.subscribe();
        let generation = tokio::time::timeout(CREATION_DEADLINE, async {
            let world = super::adventure::free_roam::world().ok_or(())?;
            let home = place::home_id(&pack::scene(pack::SceneKey::Opening)).map_err(|_| ())?;
            let destination = place::destination(current.state(), &home, &origin.place_id, &exit)
                .map_err(|_| ())?;
            let named = match &destination {
                Destination::Create { named } => named.clone(),
                _ => None,
            };
            let existing = match destination {
                Destination::Created(record) => Some(*record),
                Destination::Home => Some(world.gate),
                Destination::Story { .. } => return Err(()),
                Destination::Create { .. } => super::adventure::free_roam::prepared(current)
                    .into_iter()
                    .find(|ready| ready.from == origin.place_id && ready.exit == exit.id)
                    .map(|ready| ready.place),
            };
            if let Some(record) = existing {
                return Ok(record);
            }
            let model = scene
                .setting
                .as_ref()
                .and_then(|setting| setting.model.as_ref())
                .ok_or(())?;
            let mut request = self
                .creation_request(current, scene, model, Some(&exit), named, "", None)
                .await
                .map_err(|_| ())?;
            request.adventure = world.city.description();
            request.canon["city"] = json!(world.city);
            request.place_names.push(origin.name.clone());
            request.taken_ids.push(origin.place_id.clone());
            let (storyteller, persona_model) = self.npc_voice.hero_identity_models();
            for _ in 0..2 {
                let ahead = super::adventure::free_roam::prepared(current);
                request
                    .place_names
                    .extend(ahead.iter().map(|ready| ready.place.name.clone()));
                request
                    .taken_ids
                    .extend(ahead.iter().map(|ready| ready.place.place_id.clone()));
                request.people_names.extend(
                    ahead.iter().flat_map(|ready| {
                        ready.place.people.iter().map(|person| person.name.clone())
                    }),
                );
                #[cfg(test)]
                let generator = owner.upgrade().and_then(|owner| {
                    owner
                        .generator
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .clone()
                });
                #[cfg(test)]
                let made = match generator {
                    Some(generator) => generator(&request).await,
                    None => {
                        place_creation::create_silent(storyteller, persona_model, &request).await
                    }
                };
                #[cfg(not(test))]
                let made =
                    place_creation::create_silent(storyteller, persona_model, &request).await;
                let record = made.map_err(|_| ())?;
                let Some(owner) = owner.upgrade() else {
                    return Err(());
                };
                let batch = owner.lock();
                if !batch
                    .as_ref()
                    .is_some_and(|batch| Arc::ptr_eq(&batch.token, token))
                {
                    return Err(());
                }
                if !matches!(*outcome.borrow(), AheadOutcome::Pending) {
                    return Err(());
                }
                let latest = self.updates.borrow().clone();
                if latest.basis().session != current.basis().session
                    || latest.basis().run != current.basis().run
                {
                    return Err(());
                }
                if beat_script_runtime::scene(latest.state())
                    .ok()
                    .and_then(|scene| scene.setting)
                    .is_none_or(|setting| setting.place_id != origin.place_id)
                {
                    outcome.send_replace(AheadOutcome::Cancelled);
                    audit_ahead(&origin.place_id, &exit.id, "cancelled", started, operation);
                    return Err(());
                }
                if super::adventure::free_roam::remember(
                    current,
                    super::adventure::free_roam::PreparedPlace {
                        from: origin.place_id.clone(),
                        exit: exit.id.clone(),
                        place: record.clone(),
                    },
                ) {
                    outcome.send_replace(AheadOutcome::Ready(Arc::new(record.clone())));
                    audit_ahead(&origin.place_id, &exit.id, "ready", started, operation);
                    return Ok(record);
                }
                // The other concurrent result reserved this name/id first; regenerate against it.
            }
            Err(())
        });
        let result = tokio::select! {
            result = generation => result,
            () = async {
                loop {
                    if matches!(*cancellation.borrow_and_update(), AheadOutcome::Cancelled) { break; }
                    if cancellation.changed().await.is_err() { break; }
                }
            } => return,
        };
        match result {
            Ok(Ok(record)) => {
                self.prepare_place_media(
                    destination_scene(current, &record, scene),
                    super::living_scenes::PlateAheadSource::Generated,
                );
                if matches!(*outcome.borrow(), AheadOutcome::Pending) {
                    outcome.send_replace(AheadOutcome::Ready(Arc::new(record)));
                    audit_ahead(&origin.place_id, &exit.id, "ready", started, operation);
                }
            }
            _ => {
                if matches!(*outcome.borrow(), AheadOutcome::Pending) {
                    outcome.send_replace(AheadOutcome::Failed);
                    audit_ahead(&origin.place_id, &exit.id, "failed", started, operation);
                }
            }
        }
    }

    /// What the storyteller is given to create the place beyond `exit`, or at the end of a
    /// journey when there is no exit: only public things, the hero's own line as quoted data, and
    /// every place and person the run already has, so none is made twice.
    #[allow(clippy::too_many_arguments)]
    async fn creation_request(
        &self,
        current: &Checkpoint,
        scene: &pack::Scene,
        model: &SceneModel,
        exit: Option<&SceneWayOut>,
        named: Option<String>,
        goal: &str,
        landmark: Option<LandmarkDraft>,
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
        let place_names = place::names_in_use(state)?;
        let people_names = place::people_in_use(state, &heroes)?;
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
        let heading_text = match (exit, &landmark) {
            (Some(exit), _) => format!("{} {}", exit.name, exit.beyond),
            (None, Some(landmark)) => format!("{} {}", landmark.name, landmark.direction),
            (None, None) => goal.chars().take(200).collect(),
        };
        let recalled_memory = match memory::current() {
            Some(story) => {
                let text = format!("{heading_text} {}", setting.name);
                let exit_id = exit.map_or(JOURNEY_EXIT, |exit| exit.id.as_str());
                let entities = [setting.place_id.as_str(), exit_id, setting.name.as_str()];
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
            run_seed: format!("{:?}", current.basis().run),
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
            exit: exit.cloned(),
            goal: goal.to_owned(),
            landmark,
            named,
            taken_ids,
            place_names,
            people_names,
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
    let mut snapshot = journey::living_scene::snapshot(state, &scene)
        .map_err(unavailable)?
        .ok_or_else(|| "the current place has no scene model".to_owned())?;
    let party = place::whereabouts(state, &scene).map_err(unavailable)?;
    place::name_bound_exits(state, &party.here, &mut snapshot.model.ways_out)
        .map_err(unavailable)?;
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
        "city": super::adventure::free_roam::world().map(|world| world.city),
        "prepared_ahead": super::adventure::free_roam::prepared(current).into_iter().map(|ready| json!({
            "from": ready.from, "exit": ready.exit, "id": ready.place.place_id, "name": ready.place.name,
        })).collect::<Vec<_>>(),
        "exits": exits,
        "known": known,
        "moves": moves,
    }))
}

#[cfg(test)]
#[path = "place_tests.rs"]
mod tests;
