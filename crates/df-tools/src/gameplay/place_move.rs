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

use std::time::Duration;

use df_model::checkpoint::Checkpoint;
use df_protocol::common as rpc;
use df_types::OperationId;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
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
    place::{self, DEV_EXIT, Destination, JOURNEY_EXIT, KnownPlace, LandmarkDraft, PlaceRefusal},
    place_creation::{self, CreationRequest, Heading, HeadingChoices, ServiceOffer, TravelBeat},
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
    let Some(snapshot) = journey::living_scene::snapshot(state, &scene)? else {
        return Ok(vec![]);
    };
    let party = place::whereabouts(state, &scene)?;
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

/// A ride the hero hailed or hired, ready to go once any fare is paid.
pub(super) struct Ride {
    prepared: PreparedMove,
    pub(super) fare: Option<RideFare>,
    hero: String,
    hero_index: u32,
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
        let snapshot = journey::living_scene::snapshot(current.state(), &scene)?
            .ok_or(df_session::submission::RepositoryError::InvalidCandidate)?;
        let setting = scene
            .setting
            .as_mut()
            .ok_or(df_session::submission::RepositoryError::InvalidCandidate)?;
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
        self.prepare_free_roam_neighbours(&current, &scene, &record)
            .await;
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
        let snapshot = journey::living_scene::snapshot(state, scene)
            .ok()
            .flatten()?;
        let party = place::whereabouts(state, scene).ok()?;
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
                return Some(DialogueReply::Raw(
                    rpc::DialogueResponse {
                        input_id: request.input_id.clone(),
                        disposition: rpc::DialogueDisposition::Rejected as i32,
                        revision: Some(wire::revision(current.basis().revision)),
                        npc_reply: line.to_owned(),
                        ..Default::default()
                    },
                    None,
                ));
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
        let operation = df_api::operation_id(request.input_id.as_ref()).ok()?;
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
                return Some(DialogueReply::Raw(
                    rpc::DialogueResponse {
                        input_id: request.input_id.clone(),
                        disposition: rpc::DialogueDisposition::Rejected as i32,
                        revision: Some(wire::revision(self.updates.borrow().basis().revision)),
                        npc_reply: line.to_owned(),
                        ..Default::default()
                    },
                    None,
                ));
            }
            Err(MoveError::Status(status)) => {
                actor::record_unavailable("freeform-travel", &format!("{:?}", status.code()));
                return None;
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

    /// A line that hails or hires a ride to another place, as Jev reads it: the destination is
    /// chosen and, when new, made before any fare is paid, so a paid fare is never left without a
    /// journey. The fare is the listed paid service Jev finds is this ride, when there is one. `None`
    /// when the line is no ride, names nowhere to go, or the destination cannot be made.
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
        Some(Ride {
            prepared,
            fare,
            hero,
            hero_index,
        })
    }

    /// Takes a ride the hero hailed: commits the journey, then tells the table the hero's line
    /// (unless an interaction already showed it), the travel beat and the arrival, in that order.
    pub(super) async fn take_ride(
        &self,
        credential: [u8; 32],
        request: &rpc::RawDialogueRequest,
        ride: Ride,
        line_shown: bool,
    ) -> Option<DialogueReply> {
        let from_name = ride.prepared.from_name.clone();
        let to_short = ride.prepared.to_short.clone();
        let moved = self.commit_prepared(credential, ride.prepared).await.ok()?;
        if !line_shown {
            freeform_action::publish_hero_line(
                &self.table_log,
                &ride.hero,
                ride.hero_index,
                &request.text,
            );
        }
        let (storyteller, _) = self.npc_voice.hero_identity_models();
        let beat = place_creation::narrate_travel(
            storyteller,
            &TravelBeat {
                hero: &ride.hero,
                words: &request.text,
                from: &from_name,
                to: &moved.place_name,
                to_short: &to_short,
                ride: true,
                hired: ride
                    .fare
                    .as_ref()
                    .map(|fare| (fare.name.as_str(), fare.provider.as_str())),
            },
        )
        .await;
        freeform_action::publish_outcome(
            &self.table_log,
            ride.hero_index,
            None::<super::table_log::RollRow>,
            &beat,
        );
        freeform_action::publish_outcome(
            &self.table_log,
            ride.hero_index,
            None::<super::table_log::RollRow>,
            &moved.line,
        );
        Some(DialogueReply::Raw(
            rpc::DialogueResponse {
                input_id: request.input_id.clone(),
                disposition: rpc::DialogueDisposition::Action as i32,
                revision: Some(wire::revision(moved.revision)),
                npc_reply: format!("{beat} {}", moved.line),
                ..Default::default()
            },
            None,
        ))
    }

    /// After a listed paid service of the place was bought by a hero's line: when Jev finds that
    /// buying it is taking a ride now (a carriage hire), the journey starts at once and the table
    /// is told the travel beat and the arrival. Returns what the table was told and the committed
    /// revision, or `None` when the service is no ride or the journey could not be made (the
    /// purchase stands either way: it was committed before this was asked).
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the service reply calls it once the TRAVEL request lands"
        )
    )]
    pub(super) async fn ride_after_service(
        &self,
        credential: [u8; 32],
        request: &rpc::RawDialogueRequest,
        scene: &pack::Scene,
        service_id: &str,
    ) -> Option<(String, df_types::SessionRevision)> {
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
        if !place_creation::judge_ride(jev, &request.text, &hero, &choices.place_name, Some(&offer))
            .await
            .ok()?
        {
            return None;
        }
        let target = self.heading(&request.text, &hero, &choices).await?;
        let operation = following_operation(df_api::operation_id(request.input_id.as_ref()).ok()?)?;
        let prepared = self
            .prepare_move(operation, target, &request.text)
            .await
            .ok()?;
        let ride = Ride {
            prepared,
            fare: Some(RideFare {
                service_id: service.id.clone(),
                price_cp: service.price_cp,
                name: service.name.clone(),
                provider,
            }),
            hero,
            hero_index,
        };
        match self.take_ride(credential, request, ride, true).await? {
            DialogueReply::Raw(response, _) => {
                Some((response.npc_reply, self.updates.borrow().basis().revision))
            }
            _ => None,
        }
    }

    /// Moves the party through `exit_id` of the place it stands in. The place is created first
    /// when the run does not have it yet; the owner then repeats every check and commits.
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
        if !matches!(&target, MoveTarget::Spot(_)) {
            self.prepare_free_roam_current().await?;
        }
        let prepared = self.prepare_move(operation, target, goal).await?;
        self.commit_prepared(credential, prepared).await
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
        let prepared = tokio::time::timeout(
            CREATION_DEADLINE,
            self.prepare_move(operation, MoveTarget::Exit(exit.clone()), ""),
        )
        .await
        .map_err(|_| {
            audit_refusal(&source, &exit, "arena_creation_deadline");
            MoveError::Refused(PlaceRefusal::CreationUnavailable)
        })??;
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
                let ready = super::adventure::free_roam::prepared(&current)
                    .into_iter()
                    .find(|ready| {
                        ready.from == party.here
                            && (ready.exit == exit_id
                                || landmark.as_ref().is_some_and(|landmark| {
                                    place::name_key(&landmark.name)
                                        == place::name_key(&ready.place.name)
                                }))
                    });
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
                let (storyteller, _) = self.npc_voice.hero_identity_models();
                let made = match ready {
                    Some(ready) => Ok(Ok(ready.place)),
                    None => {
                        tokio::time::timeout(CREATION_DEADLINE, async {
                            if free_roam.is_some() || spot.is_some() {
                                place_creation::create_silent(storyteller, &request).await
                            } else {
                                place_creation::create(storyteller, &request).await
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
            if spot.is_none() {
                self.prepare_free_roam_neighbours(&current, &scene, record)
                    .await;
            }
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
        prepared: PreparedMove,
    ) -> Result<Box<Moved>, MoveError> {
        if let Some(reason) = travel_blocker(&self.updates.borrow()).map_err(state_failure)? {
            return Err(tonic::Status::failed_precondition(reason).into());
        }
        self.commit_move(
            credential,
            MoveProposal {
                operation: prepared.operation,
                expected_basis: prepared.expected_basis,
                from: prepared.from,
                exit: prepared.exit,
                to: prepared.to,
                created: prepared.created,
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
            MoveOutcome::Moved(mut moved) => {
                self.voice
                    .register_people(self.updates.borrow().state())
                    .map_err(actor::unavailable)?;
                if super::adventure::free_roam::world().is_some() {
                    moved.line = moved.narration.clone();
                }
                Ok(moved)
            }
            MoveOutcome::Refused(reason) => Err(MoveError::Refused(reason)),
        }
    }

    /// Bounded owned look-ahead before arrival, never an authoritative move or a detached job.
    async fn prepare_free_roam_neighbours(
        &self,
        current: &Checkpoint,
        base: &pack::Scene,
        origin: &place::PlaceRecord,
    ) {
        let Some(world) = super::adventure::free_roam::world() else {
            return;
        };
        let Ok(scene) = destination_scene(current, origin, base) else {
            return;
        };
        let Some(model) = scene
            .setting
            .as_ref()
            .and_then(|setting| setting.model.as_ref())
        else {
            return;
        };
        let home = match place::home_id(&pack::scene(pack::SceneKey::Opening)) {
            Ok(home) => home,
            Err(_) => {
                audit_refusal(&origin.place_id, "ahead", "look_ahead_home");
                return;
            }
        };
        for exit in model
            .ways_out
            .iter()
            .filter(|exit| exit.open && exit.id != place::WAY_BACK)
            .take(2)
        {
            match place::destination(current.state(), &home, &origin.place_id, exit) {
                Ok(Destination::Created(record)) => {
                    self.prepare_place_media(
                        destination_scene(current, &record, base),
                        super::living_scenes::PlateAheadSource::Travel,
                    );
                    continue;
                }
                Ok(Destination::Home) => {
                    self.prepare_place_media(
                        destination_scene(current, &world.gate, base),
                        super::living_scenes::PlateAheadSource::Travel,
                    );
                    continue;
                }
                Ok(Destination::Story { .. }) => continue,
                Ok(Destination::Create { .. }) => {}
                Err(_) => {
                    audit_refusal(&origin.place_id, &exit.id, "look_ahead_state");
                    continue;
                }
            }
            let ahead = super::adventure::free_roam::prepared(current);
            if let Some(ready) = ahead
                .iter()
                .find(|ready| ready.from == origin.place_id && ready.exit == exit.id)
            {
                self.prepare_place_media(
                    destination_scene(current, &ready.place, base),
                    super::living_scenes::PlateAheadSource::Generated,
                );
                continue;
            }
            let request = self
                .creation_request(current, &scene, model, Some(exit), None, "", None)
                .await;
            let mut request = match request {
                Ok(request) => request,
                Err(_) => {
                    audit_refusal(&origin.place_id, &exit.id, "look_ahead_state");
                    continue;
                }
            };
            request.adventure = world.city.description();
            request.canon["city"] = json!(world.city);
            request
                .place_names
                .extend(ahead.iter().map(|ready| ready.place.name.clone()));
            request.place_names.push(origin.name.clone());
            request
                .taken_ids
                .extend(ahead.iter().map(|ready| ready.place.place_id.clone()));
            request.taken_ids.push(origin.place_id.clone());
            request.people_names.extend(
                ahead
                    .iter()
                    .flat_map(|ready| ready.place.people.iter().map(|person| person.name.clone())),
            );
            let (storyteller, _) = self.npc_voice.hero_identity_models();
            match tokio::time::timeout(
                CREATION_DEADLINE,
                place_creation::create_silent(storyteller, &request),
            )
            .await
            {
                Ok(Ok(record)) => {
                    self.prepare_place_media(
                        destination_scene(current, &record, base),
                        super::living_scenes::PlateAheadSource::Generated,
                    );
                    actor::record_audit_line(
                        "gameplay.free_roam",
                        format!(
                            "FREE_ROAM look_ahead from={} exit={} place={} outcome=ready",
                            origin.place_id, exit.id, record.place_id
                        ),
                    );
                    super::adventure::free_roam::remember(
                        current,
                        super::adventure::free_roam::PreparedPlace {
                            from: origin.place_id.clone(),
                            exit: exit.id.clone(),
                            place: record,
                        },
                    );
                }
                Ok(Err(error)) => audit_refusal(&origin.place_id, &exit.id, error.label()),
                Err(_) => audit_refusal(&origin.place_id, &exit.id, "look_ahead_deadline"),
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
        "city": super::adventure::free_roam::world().map(|world| world.city),
        "prepared_ahead": super::adventure::free_roam::prepared(&current).into_iter().map(|ready| json!({
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
