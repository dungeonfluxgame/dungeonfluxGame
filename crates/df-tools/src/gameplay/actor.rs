#[path = "place_actor.rs"]
pub(super) mod place_actor;
#[path = "tempo.rs"]
pub(super) mod tempo;

use df_model::checkpoint::{
    Checkpoint, CommandInput, EffectId, GameCommand, GameInput, ReferenceInventory,
};
use df_observe::OperationContext;
use df_persistence::local_demo_scope::{
    LocalDemoAuthority, LocalDemoRole, LocalDemoScopeIssuer, LocalRejectedLookup,
};
use df_persistence::{NativeCodecLimits, NativeScope, PostgresRepository};
use df_protocol::common as rpc;
use df_session::inbox::{ActorInput, AdmissionSequence, Reducer};
use df_session::submission::{
    DeliveryError, DurableOwner, OwnedInput, PublicationOwner, RepositoryError, SubmissionOutcome,
};
use prost::Message;
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};
use tokio::sync::{oneshot, watch};

use super::{
    model, notebook,
    npc_event_classifier::{EventKind, Proposal},
    refusal_detail,
    table_settings::{self, TableSettings},
    wire,
};
use crate::gameplay::investigation::the_phrase;

#[derive(Clone)]
pub(super) struct QualificationSnapshot {
    pub actions: Vec<QualificationInput>,
    pub joins: Vec<QualificationJoin>,
    pub checkpoint: Checkpoint,
    pub codec: NativeCodecLimits,
    pub calls_remaining: u16,
    #[cfg(test)]
    pub fenced: bool,
    #[cfg(test)]
    pub uncertain: bool,
}
pub(super) type QualificationJoin = (rpc::JoinRoomRequest, [u8; 32], [u8; 16]);
pub(super) type QualificationInput = ([u8; 32], rpc::SubmitActionRequest);
fn retain_qualification_input(
    inputs: &mut Vec<QualificationInput>,
    credential: [u8; 32],
    request: &rpc::SubmitActionRequest,
) {
    if inputs.len() < 7
        && matches!(
            rpc::GameplayActionKind::try_from(request.action_kind),
            Ok(rpc::GameplayActionKind::CreateCharacter
                | rpc::GameplayActionKind::BeginStory
                | rpc::GameplayActionKind::AskCourier
                | rpc::GameplayActionKind::DefendCourier
                | rpc::GameplayActionKind::GreatswordAttack)
        )
        && !inputs.iter().any(|(saved_credential, saved)| {
            *saved_credential == credential && saved.action_kind == request.action_kind
        })
    {
        inputs.push((credential, request.clone()));
    }
}

/// Whether a submission was accepted and applied now, not replayed from an earlier one.
fn accepted_now(result: &Result<rpc::SubmitActionResponse, tonic::Status>) -> bool {
    matches!(
        result,
        Ok(rpc::SubmitActionResponse {
            outcome: Some(rpc::submit_action_response::Outcome::CommittedDecision(receipt)),
        }) if !receipt.replayed
            && matches!(receipt.outcome, Some(rpc::decision_receipt::Outcome::Accepted(_)))
    )
}

/// Publishes the checkpoint-derived scene-fight close after its owning action commits.
/// A replay observes the already-closed state as both `before` and `after`, so it cannot
/// append the same public line again.
pub(in crate::gameplay) fn publish_fight_close(
    table_log: &super::table_log::TableLog,
    before: &df_model::checkpoint::GameState,
    after: &df_model::checkpoint::GameState,
) {
    if !super::journey::scene_fight::open(before) || super::journey::scene_fight::open(after) {
        return;
    }
    match super::journey::scene_fight::aftermath_narration(after) {
        // One row per line: the outcome first, then each fate, never one wall of text.
        Ok(line) if !line.is_empty() => {
            table_log.push_lines(
                "Narrator",
                rpc::TableRole::Narrator,
                0,
                rpc::TableKind::Action,
                &line,
            );
        }
        Ok(_) => {}
        Err(error) => record_unavailable("fight-close-table-line", &format!("{error:?}")),
    }
}

fn observation_table_line(
    hero: &str,
    scene: &super::pack::Scene,
    check: &rpc::AbilityCheckResult,
    choices: &[rpc::JourneyChoice],
) -> Option<String> {
    let parts = check.source_revision.splitn(4, ':').collect::<Vec<_>>();
    let [_, _, skill, detail] = parts.as_slice() else {
        return None;
    };
    let (ability, skill_name) = match *skill {
        "perception" => ("Wisdom", "Perception"),
        "insight" => ("Wisdom", "Insight"),
        "survival" => ("Wisdom", "Survival"),
        "medicine" => ("Wisdom", "Medicine"),
        "investigation" => ("Intelligence", "Investigation"),
        "arcana" => ("Intelligence", "Arcana"),
        "history" => ("Intelligence", "History"),
        "nature" => ("Intelligence", "Nature"),
        "religion" => ("Intelligence", "Religion"),
        "stealth" => ("Dexterity", "Stealth"),
        _ => return None,
    };
    let subject = if let Some(id) = detail.strip_prefix("watch-") {
        scene
            .cast
            .iter()
            .find(|person| person.id == id)?
            .name
            .clone()
    } else if let Some(thing) = choices
        .iter()
        .find(|choice| choice.group_id == "attempt-thing")
        .map(|choice| choice.option_id.trim())
        .filter(|thing| !thing.is_empty())
    {
        the_phrase(&thing.replace('-', " "))
    } else {
        let label = detail
            .strip_prefix("finding-")
            .unwrap_or(if *detail == "visible" {
                "the area"
            } else {
                detail
            });
        the_phrase(&label.replace('-', " "))
    };
    let modifier = check.ability_modifier + i32::try_from(check.proficiency_bonus).ok()?;
    let verb = match *skill {
        "insight" => "watches",
        "survival" => "tracks",
        "arcana" | "history" | "investigation" | "nature" | "religion" => "studies",
        "stealth" => "scouts",
        _ => "searches",
    };
    Some(format!(
        "{hero} {verb} {subject}: {ability} ({skill_name}) {} + {modifier} = {}.",
        check.die, check.total
    ))
}

fn watch_scope_identity(
    basis: df_model::checkpoint::Basis,
    role: LocalDemoRole,
    principal: [u8; 16],
    request: &rpc::WatchViewRequest,
) -> Result<df_types::MemberId, tonic::Status> {
    let session = df_api::session_id(request.session_id.as_ref())
        .map_err(|_| tonic::Status::invalid_argument("required watch scope invalid"))?;
    let run = df_api::run_id(request.run_id.as_ref())
        .map_err(|_| tonic::Status::invalid_argument("required watch scope invalid"))?;
    let binding = df_api::client_binding_id(request.client_binding_id.as_ref())
        .map_err(|_| tonic::Status::invalid_argument("required watch binding invalid"))?;
    let expected_binding = match role {
        LocalDemoRole::Player => principal,
        LocalDemoRole::Display => [0x72; 16],
    };
    if session != basis.session || run != basis.run || binding.as_bytes() != &expected_binding {
        return Err(tonic::Status::permission_denied("local watch scope denied"));
    }
    if let Some(revision) = request.after_revision.as_ref() {
        let observed = df_api::session_revision(Some(revision))
            .map_err(|_| tonic::Status::invalid_argument("watch revision invalid"))?;
        if observed.epoch() != basis.revision.epoch() || observed > basis.revision {
            return Err(tonic::Status::failed_precondition(
                "watch recovery required",
            ));
        }
    }
    if principal == df_persistence::local_demo_scope::PLAYER {
        return Err(tonic::Status::permission_denied(
            "bootstrap scope cannot watch",
        ));
    }
    let member = df_types::MemberId::from_bytes(&principal)
        .map_err(|_| tonic::Status::permission_denied("binding invalid"))?;
    Ok(member)
}

pub(super) enum Call {
    PublicSceneAsset {
        reply: oneshot::Sender<Result<String, tonic::Status>>,
    },
    AwaySeats {
        room_code: String,
        reply: oneshot::Sender<Result<Vec<super::room::AwayHero>, tonic::Status>>,
    },
    Reclaim {
        request: super::room::ReclaimRequest,
        reply: oneshot::Sender<
            Result<Result<rpc::JoinedRoom, super::room::ReclaimRefusal>, tonic::Status>,
        >,
    },
    QualificationInputs {
        reply: oneshot::Sender<Result<QualificationSnapshot, tonic::Status>>,
    },
    Join {
        request: rpc::JoinRoomRequest,
        reply: oneshot::Sender<Result<rpc::JoinRoomResponse, tonic::Status>>,
    },
    Submit {
        credential: [u8; 32],
        request: rpc::SubmitActionRequest,
        reply: oneshot::Sender<Result<rpc::SubmitActionResponse, tonic::Status>>,
    },
    /// A Jev classification of a committed NPC reply. The actor validates the proposal against
    /// the current scene and revision before creating one durable scene journal entry.
    NpcEvent {
        credential: [u8; 32],
        proposal: Proposal,
        reply: oneshot::Sender<Result<NpcEventOutcome, tonic::Status>>,
    },
    /// A change of place (see `place`): the owner repeats every check and commits the journal
    /// entry, or says why it did not.
    PlaceMove {
        credential: [u8; 32],
        proposal: place_actor::MoveProposal,
        reply: oneshot::Sender<Result<place_actor::MoveOutcome, tonic::Status>>,
    },
    Dialogue {
        credential: [u8; 32],
        request: super::dialogue::DialogueRequest,
        reply: oneshot::Sender<Result<super::dialogue::DialogueReply, tonic::Status>>,
    },
    /// Read-only telemetry authorization; does not publish or refresh a player's activity.
    TelemetryIdentity {
        credential: [u8; 32],
        request: rpc::WatchViewRequest,
        reply: oneshot::Sender<Result<LocalDemoRole, tonic::Status>>,
    },
    View {
        credential: [u8; 32],
        request: rpc::WatchViewRequest,
        reply: oneshot::Sender<Result<(LocalDemoRole, rpc::ViewMessage), tonic::Status>>,
    },
    /// A hero's speech intent (typing or talking). Presentation only: it resolves the
    /// caller's seat and updates the in-memory table log; it never commits, never moves a
    /// revision and is served before any heartbeat or committed-job dispatch.
    Presence {
        credential: [u8; 32],
        kind: rpc::ComposeKind,
        reply: oneshot::Sender<Result<(), tonic::Status>>,
    },
    /// The shared display's presentation report: the last combat beat sequence it has
    /// finished presenting. Presentation only, like Presence: it updates the in-memory
    /// pacing gate in the table log, never commits and never moves a revision. Only the
    /// display credential's reports count; any other caller's are ignored.
    Presented {
        credential: [u8; 32],
        sequence: u64,
        display_instance: u64,
        reply: oneshot::Sender<Result<u64, tonic::Status>>,
    },
    /// Which current room member a player binding belongs to (speech input).
    SpeechMember {
        credential: [u8; 32],
        reply: oneshot::Sender<Result<df_types::MemberId, tonic::Status>>,
    },
    /// A development control jump (DEVCTL-001 milestones 2/3): restore,
    /// back/forward or a validated state override. Reduced on the owner thread
    /// through the same compare-and-swap, receipt and publication path as
    /// player decisions; only the development surface constructs it.
    Dev {
        credential: [u8; 32],
        order: Box<super::dev_control::commands::DevOrder>,
        reply: oneshot::Sender<Result<super::dev_control::commands::DevResult, tonic::Status>>,
    },
    DevDirect {
        credential: [u8; 32],
        request: rpc::WatchViewRequest,
        order: super::dev_control::direct::Order,
        reply: oneshot::Sender<Result<super::dev_control::direct::Admission, tonic::Status>>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct NpcEventOutcome {
    pub(super) status: &'static str,
    pub(super) revision: String,
    pub(super) operation_id: Option<String>,
    pub(super) source_line_id: Option<String>,
    pub(super) item_id: Option<String>,
    pub(super) holder_id: Option<String>,
    pub(super) owner_id: Option<String>,
    pub(super) item_detail: Option<String>,
}

impl NpcEventOutcome {
    fn status(status: &'static str, revision: String) -> Self {
        Self {
            status,
            revision,
            operation_id: None,
            source_line_id: None,
            item_id: None,
            holder_id: None,
            owner_id: None,
            item_detail: None,
        }
    }
}

fn revision_key(revision: df_types::SessionRevision) -> String {
    format!("{}:{}", revision.epoch().get(), revision.sequence())
}

fn record_npc_event_status(
    proposal: &Proposal,
    status: &'static str,
    reason: &'static str,
    current_revision: &str,
) {
    record_audit_diagnostic(
        "gameplay.npc.reply_event",
        format!(
            "NPC_EVENT status={status} reason={reason} kind={} actor={} item={} source_line_id={}",
            proposal.kind.label(),
            proposal.actor_id,
            proposal.item_id.as_deref().unwrap_or("none"),
            proposal.source_line_id,
        ),
        serde_json::Map::from_iter([
            ("status".into(), serde_json::json!(status)),
            ("reason".into(), serde_json::json!(reason)),
            ("kind".into(), serde_json::json!(proposal.kind.label())),
            ("actor_id".into(), serde_json::json!(proposal.actor_id)),
            ("npc_id".into(), serde_json::json!(proposal.npc_id)),
            ("item_id".into(), serde_json::json!(proposal.item_id)),
            (
                "source_line_id".into(),
                serde_json::json!(proposal.source_line_id),
            ),
            (
                "source_line_sequence".into(),
                serde_json::json!(proposal.source_line_sequence),
            ),
            (
                "source_revision".into(),
                serde_json::json!(proposal.revision),
            ),
            (
                "current_revision".into(),
                serde_json::json!(current_revision),
            ),
        ]),
        &[
            ("gameplay.npc.reply_event.status", status.into()),
            ("gameplay.npc.reply_event.reason", reason.into()),
        ],
    );
}
impl ActorInput for Call {
    fn retained_heap_bytes(&self) -> Option<usize> {
        fn bytes(value: &Option<Vec<u8>>) -> usize {
            value.as_ref().map_or(0, Vec::capacity)
        }
        match self {
            Self::AwaySeats { room_code, .. } => Some(room_code.capacity()),
            Self::Reclaim { request, .. } => Some(request.room_code.capacity()),
            Self::Join { request, .. } => request
                .room_code
                .capacity()
                .checked_add(request.join_secret.capacity())?
                .checked_add(
                    request
                        .operation_id
                        .as_ref()
                        .map_or(0, |id| bytes(&id.value)),
                ),
            Self::PublicSceneAsset { .. }
            | Self::QualificationInputs { .. }
            | Self::SpeechMember { .. }
            | Self::NpcEvent { .. }
            | Self::Presence { .. }
            | Self::Presented { .. } => Some(0),
            Self::PlaceMove { proposal, .. } => Some(proposal.retained_bytes()),
            Self::Dev { order, .. } => order.retained_heap_bytes(),
            Self::DevDirect { order, request, .. } => order
                .text
                .as_ref()
                .map_or(0, String::capacity)
                .checked_add(request.session_id.as_ref().map_or(0, |id| bytes(&id.value)))?
                .checked_add(request.run_id.as_ref().map_or(0, |id| bytes(&id.value)))?
                .checked_add(
                    request
                        .client_binding_id
                        .as_ref()
                        .map_or(0, |id| bytes(&id.value)),
                ),
            Self::Dialogue { request, .. } => Some(request.retained_heap_bytes()),
            Self::Submit { request, .. } => {
                let character_bytes = if let Some(character) = &request.character {
                    character.choices.iter().try_fold(
                        character.name.capacity().checked_add(
                            character
                                .choices
                                .capacity()
                                .checked_mul(std::mem::size_of::<rpc::JourneyChoice>())?,
                        )?,
                        |total, choice| {
                            total
                                .checked_add(choice.group_id.capacity())?
                                .checked_add(choice.option_id.capacity())
                        },
                    )?
                } else {
                    0
                };
                request
                    .offer_id
                    .capacity()
                    .checked_add(request.session_id.as_ref().map_or(0, |id| bytes(&id.value)))?
                    .checked_add(request.run_id.as_ref().map_or(0, |id| bytes(&id.value)))?
                    .checked_add(
                        request
                            .operation_id
                            .as_ref()
                            .map_or(0, |id| bytes(&id.value)),
                    )?
                    .checked_add(character_bytes)
            }
            Self::View { request, .. } | Self::TelemetryIdentity { request, .. } => request
                .session_id
                .as_ref()
                .map_or(0, |id| bytes(&id.value))
                .checked_add(request.run_id.as_ref().map_or(0, |id| bytes(&id.value)))?
                .checked_add(
                    request
                        .client_binding_id
                        .as_ref()
                        .map_or(0, |id| bytes(&id.value)),
                ),
        }
    }
}

pub(super) struct Publication {
    updates: watch::Sender<Checkpoint>,
    intents: std::sync::mpsc::SyncSender<()>,
    campaign: Option<super::adventure::campaign_completion::CompletionObserver>,
    /// Development history bundle (DEVCTL-001 milestone 2): every committed
    /// checkpoint travels with the actor-side state the checkpoint does not
    /// carry, so a rewind restores all of it or none. Set only when the
    /// development surface is on; otherwise no history is kept.
    dev: Option<super::dev_control::Bundle>,
}
impl Publication {
    pub(super) fn new(updates: watch::Sender<Checkpoint>) -> (Self, std::sync::mpsc::Receiver<()>) {
        let (intents, receiver) = std::sync::mpsc::sync_channel(1);
        (
            Self {
                updates,
                intents,
                campaign: None,
                dev: None,
            },
            receiver,
        )
    }
    pub(super) fn with_campaign(
        mut self,
        observer: Option<super::adventure::campaign_completion::CompletionObserver>,
    ) -> Self {
        self.campaign = observer;
        self
    }
    pub(super) fn with_history(mut self, bundle: super::dev_control::Bundle) -> Self {
        self.dev = Some(bundle);
        self
    }
    pub(super) fn publish(&mut self, checkpoint: &Checkpoint) {
        self.updates.send_replace(checkpoint.clone());
    }
    pub(super) fn wake(&mut self) -> Result<(), DeliveryError> {
        match self.intents.try_send(()) {
            Ok(()) | Err(std::sync::mpsc::TrySendError::Full(())) => Ok(()),
            Err(std::sync::mpsc::TrySendError::Disconnected(())) => Err(DeliveryError::Unavailable),
        }
    }
}
impl PublicationOwner<NativeScope<LocalDemoAuthority>> for Publication {
    fn publish_committed(
        &mut self,
        _: &NativeScope<LocalDemoAuthority>,
        checkpoint: &Checkpoint,
    ) -> Result<(), DeliveryError> {
        self.publish(checkpoint);
        if let Some(observer) = &self.campaign {
            observer.observe(checkpoint);
        }
        // Development history keeps every committed moment (bounded ring)
        // so back/forward steps count decisions, player and dev alike. The
        // checkpoint travels with the actor-side state it does not carry, so
        // a rewind restores all of it or none.
        if let Some(bundle) = &self.dev
            && let Ok(mut history) = bundle.history.lock()
        {
            history.record(super::dev_control::history::SavedMoment {
                checkpoint: checkpoint.clone(),
                table_floor: bundle.table.position(),
                social: bundle.social.snapshot(),
                notebook: bundle.notebook.snapshot(),
            });
        }
        Ok(())
    }
    fn wake_committed_intents(
        &mut self,
        _: &NativeScope<LocalDemoAuthority>,
    ) -> Result<(), DeliveryError> {
        self.wake()
    }
}
pub(super) struct Engine(pub Option<std::sync::Arc<std::sync::atomic::AtomicUsize>>);
impl df_session::submission::SessionEngine<NativeScope<LocalDemoAuthority>> for Engine {
    fn decide(
        &mut self,
        current: &Checkpoint,
        scope: &NativeScope<LocalDemoAuthority>,
        input: &GameInput,
    ) -> Result<Checkpoint, RepositoryError> {
        if let GameInput::Job(completion) = input {
            df_session::submission::OperationScope::validate_input(scope, input)?;
            // This native executor runs only after DurableOwner's receipt lookup.
            // Its finite prepared recording is owned by the actor, with no task,
            // provider, clock, or dependence on the originating RPC receiver.
            let intent = current
                .state()
                .intents
                .iter()
                .find(|intent| intent.job == Some(completion.job))
                .ok_or_else(|| {
                    refusal_detail::note("job-intent-missing");
                    RepositoryError::InvalidCandidate
                })?;
            let admitted = super::courier_ai::execute(current, intent).map_err(|_| {
                refusal_detail::note("job-execute");
                RepositoryError::InvalidCandidate
            })?;
            if &admitted != completion {
                return Err(RepositoryError::InputBinding);
            }
        }
        if let Some(counter) = &self.0 {
            counter
                .fetch_update(
                    std::sync::atomic::Ordering::SeqCst,
                    std::sync::atomic::Ordering::SeqCst,
                    |value| value.checked_add(1),
                )
                .map_err(|_| RepositoryError::Capacity)?;
        }
        df_session::submission::SessionEngine::decide(
            &mut model::HarborEngine,
            current,
            scope,
            input,
        )
        .map(with_reaction_window)
    }
    fn validate_recovery(&mut self, checkpoint: &Checkpoint) -> Result<(), RepositoryError> {
        df_session::submission::SessionEngine::<NativeScope<LocalDemoAuthority>>::validate_recovery(
            &mut model::HarborEngine,
            checkpoint,
        )
    }
}
/// Opens the answer window of a hero reaction offer the decision opened, at the owner's clock,
/// before the decision commits; the rules stay clock-free. A failure leaves the candidate as
/// it is: the owner then times the offer from when it first sees it.
fn with_reaction_window(candidate: Checkpoint) -> Checkpoint {
    use super::journey::tactical;
    let stamped = tactical::unix_now_ms()
        .and_then(|now| tactical::stamp_reaction_window(&candidate, now))
        .and_then(|stamped| {
            let opened = match &stamped {
                Some(stamped) => tactical::pending_reaction(stamped)?,
                None => None,
            };
            Ok((stamped, opened))
        });
    match stamped {
        Ok((Some(stamped), opened)) => {
            if let Some(pending) = opened
                && let Some(window) = pending.window
            {
                record_audit_line(
                    "gameplay.reaction_window",
                    format!(
                        "REACTION window hero={:?} provoker={:?} action={} opened_ms={} deadline_ms={}",
                        pending.reactor_name,
                        pending.provoker_name,
                        pending.provocation_name(),
                        window.opened_unix_ms,
                        window.deadline_unix_ms
                    ),
                );
            }
            stamped
        }
        Ok((None, _)) => candidate,
        Err(error) => {
            record_unavailable("reaction-window", &format!("{error:?}"));
            candidate
        }
    }
}
pub(super) type Owner = DurableOwner<PostgresRepository<LocalDemoAuthority>, Engine, Publication>;
const COMPLETION_ATTEMPTS: u8 = 8;
const COMPLETION_RETRY_LIFETIME: Duration = Duration::from_secs(30);

pub(super) struct CompletionRetry {
    intent: EffectId,
    started: Instant,
    attempts: u8,
    next: Option<Instant>,
}
#[cfg(test)]
pub(super) type CompletionObserver =
    std::sync::mpsc::SyncSender<(Result<(), RepositoryError>, u16)>;
impl CompletionRetry {
    fn ready(&mut self, now: Instant) -> bool {
        if now.saturating_duration_since(self.started) >= COMPLETION_RETRY_LIFETIME {
            self.next = None;
        }
        self.next.is_some_and(|due| now >= due)
    }

    fn failed(&mut self, now: Instant, error: RepositoryError) -> &'static str {
        self.attempts = self.attempts.saturating_add(1);
        self.next = None;
        if !matches!(
            error,
            RepositoryError::Unavailable
                | RepositoryError::UnresolvedCommit
                | RepositoryError::RevisionConflict
        ) {
            return "completion_refused_retained_pending";
        }
        if self.attempts >= COMPLETION_ATTEMPTS
            || now.saturating_duration_since(self.started) >= COMPLETION_RETRY_LIFETIME
        {
            return "completion_retry_exhausted_retained_pending";
        }
        let delay = Duration::from_millis(250 * (1u64 << (self.attempts - 1).min(3)));
        self.next = now.checked_add(delay).filter(|next| {
            next.saturating_duration_since(self.started) <= COMPLETION_RETRY_LIFETIME
        });
        if self.next.is_some() {
            "completion_retry_scheduled"
        } else {
            "completion_retry_exhausted_retained_pending"
        }
    }
}
/// The operation a fence was last at risk from, kept so the owner can retry it by itself.
#[derive(Clone)]
pub(super) enum Attempt {
    /// A player's (or the room's) input, with the envelope it was issued under.
    Input {
        credential: [u8; 32],
        fingerprint: [u8; 32],
        input: Box<GameInput>,
    },
    /// The committed courier job's completion, which rebuilds its own scope.
    Completion,
}

/// What the owner knows about the health of the session (see `session_recovery`).
pub(super) struct SessionState {
    pub(super) attempt: Option<Attempt>,
    /// How long the last commit ran, in milliseconds.
    pub(super) commit_millis: u64,
    /// Set while the session is fenced.
    pub(super) fence: Option<super::session_recovery::Health>,
    /// Keeps the owner lease and access grants alive while nobody is acting.
    pub(super) heartbeat: super::session_recovery::Heartbeat,
    pub(super) seat_clock: super::room::SeatClock,
    /// Times hero reaction offers and declines one whose window runs out (P6-80).
    pub(super) reaction: ReactionTimer,
}

impl Default for SessionState {
    fn default() -> Self {
        Self {
            seat_clock: super::room::SeatClock::default(),
            reaction: ReactionTimer::default(),
            attempt: None,
            commit_millis: 0,
            fence: None,
            heartbeat: super::session_recovery::Heartbeat::for_lease(
                df_persistence::local_demo_scope::lease_seconds(),
            ),
        }
    }
}

/// How long the owner waits before it tries again an expiry decline it could not send.
const REACTION_EXPIRY_RETRY: Duration = Duration::from_secs(5);

/// The owner's clock for hero reaction offers (P6-80). The window itself is recorded in the
/// checkpoint when the offer opens; this keeps only what the owner needs to act on it.
#[derive(Default)]
pub(super) struct ReactionTimer {
    /// The latest live player credential of each member, so that a decline for a hero whose
    /// window ran out goes through the same submit path as the hero's own answer.
    credentials: std::collections::HashMap<[u8; 16], [u8; 32]>,
    /// The window of an offer whose checkpoint records none (one written before windows
    /// existed, or a development candidate): it runs from when this owner first saw the offer.
    fallback: Option<(df_model::checkpoint::RecordId, i64)>,
    /// The earliest next attempt after an expiry decline that could not be sent.
    retry_after: Option<(df_model::checkpoint::RecordId, Instant)>,
    /// The offer whose expiry decline is being submitted right now.
    expiring: Option<super::journey::tactical::PendingReaction>,
}
impl ReactionTimer {
    /// The most members whose credentials are kept; a room never seats more.
    const CREDENTIALS: usize = 16;

    fn remember(&mut self, member: df_types::MemberId, credential: [u8; 32]) {
        if self.credentials.len() < Self::CREDENTIALS
            || self.credentials.contains_key(member.as_bytes())
        {
            self.credentials.insert(*member.as_bytes(), credential);
        }
    }
    /// The deadline of this offer: the recorded one, else the window this owner started.
    fn deadline(&self, pending: &super::journey::tactical::PendingReaction) -> Option<i64> {
        pending
            .window
            .map(|window| window.deadline_unix_ms)
            .or_else(|| {
                self.fallback
                    .filter(|(reaction, _)| *reaction == pending.reaction)
                    .map(|(_, deadline)| deadline)
            })
    }
    /// When the owner must next look at the pending offer: its deadline, no earlier than a
    /// scheduled retry, or at once for an offer without a window yet.
    fn wake_at(
        &self,
        pending: Option<&super::journey::tactical::PendingReaction>,
        now: Instant,
        now_unix_ms: i64,
    ) -> Option<Instant> {
        let pending = pending?;
        let Some(deadline) = self.deadline(pending) else {
            return Some(now);
        };
        let wait = u64::try_from(deadline.saturating_sub(now_unix_ms)).unwrap_or_default();
        let due = now + Duration::from_millis(wait);
        Some(match self.retry_after {
            Some((reaction, retry)) if reaction == pending.reaction => due.max(retry),
            _ => due,
        })
    }
    /// The deadline of an offer whose window has run out and whose decline may be sent now.
    /// An offer without a recorded window starts its window here, at `now_unix_ms`.
    fn due(
        &mut self,
        pending: Option<&super::journey::tactical::PendingReaction>,
        now: Instant,
        now_unix_ms: i64,
    ) -> Option<i64> {
        let current = pending.map(|pending| pending.reaction);
        self.fallback = self
            .fallback
            .filter(|(reaction, _)| Some(*reaction) == current);
        self.retry_after = self
            .retry_after
            .filter(|(reaction, _)| Some(*reaction) == current);
        let pending = pending?;
        let Some(deadline) = self.deadline(pending) else {
            let deadline = now_unix_ms.saturating_add(df_model::checkpoint::REACTION_WINDOW_MILLIS);
            self.fallback = Some((pending.reaction, deadline));
            record_audit_line(
                "gameplay.reaction_window",
                format!(
                    "REACTION window hero={:?} provoker={:?} deadline_ms={deadline} source=owner-start",
                    pending.reactor_name, pending.provoker_name
                ),
            );
            return None;
        };
        let waiting = self.retry_after.is_some_and(|(_, retry)| now < retry);
        (now_unix_ms >= deadline && !waiting).then_some(deadline)
    }
    fn retry_later(&mut self, reaction: df_model::checkpoint::RecordId, now: Instant) {
        self.retry_after = Some((reaction, now + REACTION_EXPIRY_RETRY));
    }
}

/// The hero's own table line of a decline the owner sent because the window ran out: the hero
/// let the provoking combatant pass. The other lines of the resolution are unchanged.
fn expired_reaction_lines(
    mut lines: Vec<super::table_log::ActionLine>,
    hero: &str,
    provoker: &str,
) -> Vec<super::table_log::ActionLine> {
    if let Some(line) = lines.iter_mut().find(|line| line.speaker == hero) {
        line.text = format!(
            "lets {provoker} pass without an opportunity attack: no answer within {} seconds",
            df_model::checkpoint::REACTION_WINDOW_MILLIS / 1000
        );
    }
    lines
}

pub(super) struct Actor {
    pub session: SessionState,
    pub owner: Owner,
    /// The public table log; an accepted action and a typed line are appended here.
    pub table_log: std::sync::Arc<super::table_log::TableLog>,
    /// The shared and per-hero notebook, persisted beside the adventure record.
    pub notebook: std::sync::Arc<notebook::NotebookLedger>,
    /// Each NPC's backstory and attitude toward each hero.
    pub social: std::sync::Arc<super::social::SocialLedger>,
    /// The room's table settings, as stored and carried in every view.
    pub table_settings: TableSettings,
    pub dialogue: super::dialogue::DialogueState,
    pub tempo: Option<tempo::TempoOwner>,
    pub bootstrap_credential: [u8; 32],
    pub issuer: LocalDemoScopeIssuer,
    pub codec: NativeCodecLimits,
    pub fenced: bool,
    pub recovery_wakeup: watch::Sender<Checkpoint>,
    pub campaign_observer: Option<super::adventure::campaign_completion::CompletionObserver>,
    pub intent_notifications: std::sync::mpsc::Receiver<()>,
    pub calls_remaining: u16,
    /// Plain play without `DF_GAMEPLAY_CALL_BUDGET`: calls are neither refused nor counted down.
    pub calls_unbounded: bool,
    pub qualification_inputs: Option<Vec<QualificationInput>>,
    pub qualification_joins: Option<Vec<QualificationJoin>>,
    pub completion_retry: Option<CompletionRetry>,
    /// Development back/forward ring for refreshing a rewind's history entry
    /// (DEVCTL-001 milestone 2). Set only when the surface is on.
    pub dev_history: Option<std::sync::Arc<std::sync::Mutex<super::dev_control::history::History>>>,
    #[cfg(test)]
    pub completion_observer: Option<CompletionObserver>,
}
/// The rejection a phone is shown when this command was definitively refused
/// against the current state (nothing committed, the owner keeps serving), or
/// `None` when the failure is an authority problem the phone must retry after
/// recovery. A definitive refusal must never become `Unavailable`: the phone
/// would keep its operation and show the lost-connection state for a command
/// the game has already answered.
fn command_refusal_code(error: RepositoryError) -> Option<rpc::RejectionCode> {
    match error {
        RepositoryError::InvalidCandidate | RepositoryError::Capacity => {
            Some(rpc::RejectionCode::CapabilityUnavailable)
        }
        RepositoryError::InputBinding => Some(rpc::RejectionCode::InvalidSelection),
        RepositoryError::OperationConflict => Some(rpc::RejectionCode::OperationConflict),
        _ => None,
    }
}

pub(super) fn unavailable(error: RepositoryError) -> tonic::Status {
    match error {
        RepositoryError::Unauthorized => {
            tonic::Status::permission_denied("local gameplay binding denied")
        }
        _ => {
            record_unavailable("authority", &format!("{error:?}"));
            authority_unavailable()
        }
    }
}

fn authority_unavailable() -> tonic::Status {
    tonic::Status::unavailable("gameplay authority unavailable; retain your operation")
}

/// One `UNAVAILABLE` line on stdout and the same fact on the shared observability path, for every
/// place the phone is told to retain its operation or reconnect. `site` names the server path.
pub(super) fn record_unavailable(site: &'static str, reason: &str) {
    let step = refusal_detail::step_or_unknown();
    println!("UNAVAILABLE site={site} reason={reason} step={step}");
    let context = OperationContext {
        trace_parent: String::new(),
        build: crate::BUILD_ID.to_owned(),
    };
    let mut span = df_observe::begin(&context, "gameplay.unavailable");
    span.event(
        "gameplay.unavailable",
        &[
            ("unavailable.site", site.to_owned()),
            ("unavailable.reason", reason.to_owned()),
            ("unavailable.step", step.to_owned()),
        ],
    );
    span.finish_unmeasured("unavailable");
}

/// One diagnostic per refused or failed gameplay submit: one `REFUSAL` line on stdout and the same
/// fact on the shared observability path. It names the action kind, the internal reason and the
/// status the phone receives; it never carries the submitted payload.
fn record_refusal(action_kind: i32, reason: &str, status: &str) {
    let kind = rpc::GameplayActionKind::try_from(action_kind)
        .map_or_else(|_| action_kind.to_string(), |kind| format!("{kind:?}"));
    let step = refusal_detail::step_or_unknown();
    println!("REFUSAL action={kind} reason={reason} status={status} step={step}");
    let context = OperationContext {
        trace_parent: String::new(),
        build: crate::BUILD_ID.to_owned(),
    };
    let mut span = df_observe::begin(&context, "gameplay.action_refused");
    span.event(
        "gameplay.action_refused",
        &[
            ("action.kind", kind),
            ("refusal.reason", reason.to_owned()),
            ("refusal.step", step.to_owned()),
        ],
    );
    span.finish_unmeasured(status);
}

/// Every audit line written during a test run, so a test can check that a subsystem wrote its line.
#[cfg(test)]
static AUDIT_TAP: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// The audit lines written so far in this test run that contain `needle`.
#[cfg(test)]
pub(super) fn audit_lines_containing(needle: &str) -> Vec<String> {
    AUDIT_TAP
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
        .filter(|line| line.contains(needle))
        .cloned()
        .collect()
}

pub(super) fn record_audit_line(event: &'static str, line: String) {
    #[cfg(test)]
    AUDIT_TAP
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push(line.clone());
    super::dev_control::timeline::record(event, &line);
    println!("{line}");
    let context = OperationContext {
        trace_parent: String::new(),
        build: crate::BUILD_ID.to_owned(),
    };
    let mut span = df_observe::begin(&context, event);
    let field = if event == "gameplay.combat_outcome" {
        "combat.line"
    } else {
        "audit.line"
    };
    span.event(event, &[(field, line)]);
    span.finish_unmeasured("recorded");
}

pub(super) fn record_audit_diagnostic(
    event: &'static str,
    line: String,
    fields: serde_json::Map<String, serde_json::Value>,
    attributes: &[(&'static str, String)],
) {
    super::dev_control::timeline::record_structured(event, &line, fields);
    println!("{line}");
    let context = OperationContext {
        trace_parent: String::new(),
        build: crate::BUILD_ID.to_owned(),
    };
    let mut span = df_observe::begin(&context, event);
    span.event(event, attributes);
    span.finish_unmeasured("recorded");
}

pub(super) fn record_enemy_turn(
    actor: &str,
    operation: &str,
    run: &str,
    revision: u64,
    policy_revision: &str,
    action: &str,
    reason: &str,
    detail: &str,
) {
    let line = enemy_turn_line(
        actor,
        operation,
        run,
        revision,
        policy_revision,
        action,
        reason,
        detail,
    );
    println!("{line}");
    let context = OperationContext {
        trace_parent: String::new(),
        build: crate::BUILD_ID.to_owned(),
    };
    let mut span = df_observe::begin(&context, "gameplay.enemy_turn");
    span.event(
        "gameplay.enemy_turn",
        &[
            ("combat.actor", actor.to_owned()),
            ("operation.id", operation.to_owned()),
            ("session.run", run.to_owned()),
            ("session.revision", revision.to_string()),
            ("combat.policy_revision", policy_revision.to_owned()),
            ("combat.action", action.to_owned()),
            ("combat.reason", reason.to_owned()),
            ("combat.action_detail", detail.to_owned()),
        ],
    );
    span.finish_unmeasured("recorded");
}

fn enemy_turn_line(
    actor: &str,
    operation: &str,
    run: &str,
    revision: u64,
    policy_revision: &str,
    action: &str,
    reason: &str,
    detail: &str,
) -> String {
    format!(
        "COMBAT turn actor={actor:?} operation={operation} run={run} revision={revision} policy={policy_revision:?} action={action} reason={reason:?} detail={detail:?}"
    )
}

fn record_session_machine_refusal(checkpoint: &Checkpoint, request: &rpc::SubmitActionRequest) {
    let Some(step) = refusal_detail::take() else {
        return;
    };
    if !matches!(
        step,
        "session-machine"
            | "session-machine-rest"
            | "session-machine-outcome"
            | "new-adventure-machine"
    ) {
        refusal_detail::note(step);
        return;
    }
    if let (Ok(kind), Ok(state)) = (
        rpc::GameplayActionKind::try_from(request.action_kind),
        super::journey::session_machine::SessionState::of(checkpoint),
    ) && let Some(event) = super::journey::session_machine::Event::for_kind(state, kind)
    {
        record_session_state_diagnostic(
            "gameplay.session_machine_refused",
            "failed",
            "refused",
            Some(step),
            Some(state),
            None,
            event,
            kind,
            request,
            Some(checkpoint.basis().revision),
            None,
            checkpoint,
        );
    }
    refusal_detail::note(step);
}

#[allow(clippy::too_many_arguments)]
fn record_session_state_diagnostic(
    event_name: &'static str,
    stage: &'static str,
    status: &'static str,
    failure_category: Option<&str>,
    from_state: Option<super::journey::session_machine::SessionState>,
    to_state: Option<super::journey::session_machine::SessionState>,
    machine_event: super::journey::session_machine::Event,
    kind: rpc::GameplayActionKind,
    request: &rpc::SubmitActionRequest,
    revision_before: Option<df_types::SessionRevision>,
    revision_after: Option<df_types::SessionRevision>,
    checkpoint_for_scene: &Checkpoint,
) {
    let session_bytes = request
        .session_id
        .as_ref()
        .and_then(|value| value.value.as_deref())
        .unwrap_or_default();
    let run_bytes = request
        .run_id
        .as_ref()
        .and_then(|value| value.value.as_deref())
        .unwrap_or_default();
    let operation_bytes = request
        .operation_id
        .as_ref()
        .and_then(|value| value.value.as_deref())
        .unwrap_or_default();
    let room_hash = (!session_bytes.is_empty()).then(|| diagnostic_hash("room", session_bytes));
    let run_hash = (!run_bytes.is_empty()).then(|| diagnostic_hash("run", run_bytes));
    let operation_hash =
        (!operation_bytes.is_empty()).then(|| diagnostic_hash("operation", operation_bytes));

    let mut resource_hasher = Sha256::new();
    resource_hasher.update(b"df-session-transition-resource-v1\0");
    resource_hasher.update(session_bytes);
    resource_hasher.update([0]);
    resource_hasher.update(run_bytes);
    resource_hasher.update([0]);
    resource_hasher.update(operation_bytes);
    resource_hasher.update([0]);
    resource_hasher.update(format!("{}:{}", kind as i32, event_label(machine_event)));
    if let Some(revision) = revision_before {
        resource_hasher.update(revision.epoch().get().to_be_bytes());
        resource_hasher.update(revision.sequence().to_be_bytes());
    }
    let resource_id = format!(
        "sha256:{}",
        resource_hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );
    let scene_id = scene_key(checkpoint_for_scene);
    let from = from_state.map(state_label);
    let to = to_state.map(state_label);
    let revision_before = revision_before.map(revision_label);
    let revision_after = revision_after.map(revision_label);
    let action_kind = audit_kind_name(kind);
    let machine_event = event_label(machine_event);
    let line = format!(
        "STATE {stage} status={status} resource_id={resource_id} room_id_hash={} run_id_hash={} revision_before={} revision_after={} from_state={} to_state={} event={machine_event} action_kind={action_kind} scene_id={} failure_category={}",
        room_hash.as_deref().unwrap_or("none"),
        run_hash.as_deref().unwrap_or("none"),
        revision_before.as_deref().unwrap_or("none"),
        revision_after.as_deref().unwrap_or("none"),
        from.unwrap_or("none"),
        to.unwrap_or("none"),
        scene_id.unwrap_or("none"),
        failure_category.unwrap_or("none"),
    );
    let fields = serde_json::Map::from_iter([
        ("resource_type".to_owned(), serde_json::json!("state")),
        ("resource_id".to_owned(), serde_json::json!(resource_id)),
        ("room_id_hash".to_owned(), serde_json::json!(room_hash)),
        ("run_id_hash".to_owned(), serde_json::json!(run_hash)),
        (
            "operation_id_hash".to_owned(),
            serde_json::json!(operation_hash),
        ),
        ("scene_id".to_owned(), serde_json::json!(scene_id)),
        ("stage".to_owned(), serde_json::json!(stage)),
        ("status".to_owned(), serde_json::json!(status)),
        (
            "failure_category".to_owned(),
            serde_json::json!(failure_category),
        ),
        ("from_state".to_owned(), serde_json::json!(from)),
        ("to_state".to_owned(), serde_json::json!(to)),
        ("machine_event".to_owned(), serde_json::json!(machine_event)),
        ("action_kind".to_owned(), serde_json::json!(action_kind)),
        (
            "revision_before".to_owned(),
            serde_json::json!(revision_before),
        ),
        (
            "revision_after".to_owned(),
            serde_json::json!(revision_after),
        ),
    ]);
    let attributes = [
        ("gameplay.resource.type", "state".to_owned()),
        ("gameplay.resource.id", resource_id),
        (
            "gameplay.session.room_id_hash",
            room_hash.unwrap_or_else(|| "none".to_owned()),
        ),
        (
            "gameplay.session.run_id_hash",
            run_hash.unwrap_or_else(|| "none".to_owned()),
        ),
        (
            "gameplay.session.operation_id_hash",
            operation_hash.unwrap_or_else(|| "none".to_owned()),
        ),
        (
            "gameplay.session.scene_id",
            scene_id.unwrap_or("none").to_owned(),
        ),
        ("gameplay.resource.stage", stage.to_owned()),
        ("gameplay.resource.status", status.to_owned()),
        (
            "gameplay.resource.failure_category",
            failure_category.unwrap_or("none").to_owned(),
        ),
        (
            "gameplay.session.from_state",
            from.unwrap_or("none").to_owned(),
        ),
        ("gameplay.session.to_state", to.unwrap_or("none").to_owned()),
        ("gameplay.session.machine_event", machine_event.to_owned()),
        ("gameplay.session.action_kind", action_kind),
        (
            "gameplay.session.revision_before",
            revision_before.unwrap_or_else(|| "none".to_owned()),
        ),
        (
            "gameplay.session.revision_after",
            revision_after.unwrap_or_else(|| "none".to_owned()),
        ),
    ];
    record_audit_diagnostic(event_name, line, fields, &attributes);
}

fn diagnostic_hash(kind: &str, bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"df-session-diagnostic-id-v1\0");
    hasher.update(kind.as_bytes());
    hasher.update([0]);
    hasher.update(bytes);
    format!(
        "sha256:{}",
        hasher
            .finalize()
            .iter()
            .take(16)
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

fn state_label(state: super::journey::session_machine::SessionState) -> &'static str {
    use super::journey::session_machine::SessionState as State;
    match state {
        State::Lobby => "lobby",
        State::Story => "story",
        State::Conversation => "conversation",
        State::Fight => "fight",
        State::Aftermath => "aftermath",
        State::Ended => "ended",
        State::Preparing => "preparing",
    }
}

fn event_label(event: super::journey::session_machine::Event) -> &'static str {
    use super::journey::session_machine::Event;
    match event {
        Event::CreateHero => "create_hero",
        Event::Begin => "begin",
        Event::StoryChoice => "story_choice",
        Event::ScriptStep => "script_step",
        Event::StartFight => "start_fight",
        Event::CombatAction => "combat_action",
        Event::EndTurn => "end_turn",
        Event::Rest => "rest",
        Event::Resolve => "resolve",
        Event::NewAdventure => "new_adventure",
    }
}

fn revision_label(revision: df_types::SessionRevision) -> String {
    format!("{}:{}", revision.epoch().get(), revision.sequence())
}

fn scene_key(checkpoint: &Checkpoint) -> Option<&'static str> {
    let scene = super::beat_script_runtime::scene(checkpoint.state()).ok()?;
    Some(match scene.key {
        super::pack::SceneKey::Room => "room",
        super::pack::SceneKey::Opening => "opening",
        super::pack::SceneKey::DialogueSeal => "dialogue_seal",
        super::pack::SceneKey::DialogueEscort => "dialogue_escort",
        super::pack::SceneKey::Combat => "combat",
        super::pack::SceneKey::Inn => "inn",
        super::pack::SceneKey::Victory => "victory",
        super::pack::SceneKey::Defeat => "defeat",
    })
}

fn audit_kind_name(kind: rpc::GameplayActionKind) -> String {
    let mut name = String::new();
    for character in format!("{kind:?}").chars() {
        if character.is_ascii_uppercase() && !name.is_empty() {
            name.push('_');
        }
        name.push(character.to_ascii_lowercase());
    }
    name
}

fn audit_phase_name(phase: rpc::JourneyPhase) -> &'static str {
    match phase {
        rpc::JourneyPhase::Unspecified => "unspecified",
        rpc::JourneyPhase::Room => "room",
        rpc::JourneyPhase::CharacterCreation => "character_creation",
        rpc::JourneyPhase::Opening => "opening",
        rpc::JourneyPhase::Dialogue => "dialogue",
        rpc::JourneyPhase::Combat => "combat",
        rpc::JourneyPhase::Complete => "complete",
    }
}

fn audit_combat_view(
    checkpoint: &Checkpoint,
    member: df_types::MemberId,
) -> Option<rpc::CombatView> {
    let view = wire::journey_view(checkpoint, LocalDemoRole::Player, member).ok()?;
    let Some(rpc::view_message::Audience::Player(player)) = view.audience else {
        return None;
    };
    player.journey?.combat
}

fn audit_hero(checkpoint: &Checkpoint, member: df_types::MemberId) -> Option<String> {
    let view = wire::journey_view(checkpoint, LocalDemoRole::Player, member).ok()?;
    let Some(rpc::view_message::Audience::Player(player)) = view.audience else {
        return None;
    };
    player
        .journey?
        .own_character
        .map(|character| character.name)
}

fn after_grant_reconnect<T>(
    reconnected: Result<(), RepositoryError>,
    dispatch: impl FnOnce() -> Result<T, tonic::Status>,
) -> Result<T, tonic::Status> {
    reconnected.map_err(unavailable)?;
    dispatch()
}
// The same fence transition is used by native handlers and controlled owner tests.
pub(super) fn fence_checkpoint(
    fenced: &mut bool,
    recovery_wakeup: &watch::Sender<Checkpoint>,
    checkpoint: &Checkpoint,
) {
    *fenced = true;
    recovery_wakeup.send_replace(checkpoint.clone());
}

/// What every screen is told when the saved state cannot be recovered. The browser shows this
/// text (see `SERVER_LOST` in the client).
pub(super) const SESSION_LOST_MESSAGE: &str =
    "The game server lost its saved state and must be restarted";

pub(super) fn session_lost_status() -> tonic::Status {
    tonic::Status::data_loss(SESSION_LOST_MESSAGE)
}

pub(super) fn require_readable(fenced: bool) -> Result<(), tonic::Status> {
    if fenced {
        record_unavailable("owner-fenced", "the owner is fenced until recovery");
        return Err(tonic::Status::unavailable(
            "gameplay owner requires recovery",
        ));
    }
    Ok(())
}

impl Actor {
    fn observe_campaign(&self) {
        if let Some(observer) = &self.campaign_observer {
            observer.observe(self.owner.checkpoint());
        }
    }
    /// The next time the owner has work of its own: a completion retry or a recovery attempt.
    pub(super) fn next_completion_wake(&self) -> Option<Instant> {
        let completion = self.completion_retry.as_ref().and_then(|retry| retry.next);
        let recovery = if self.fenced {
            match &self.session.fence {
                Some(fence) => fence.next_attempt(),
                None => Some(Instant::now()),
            }
        } else {
            None
        };
        let heartbeat =
            heartbeat_wake(self.session_lost(), &self.session.heartbeat, Instant::now());
        [
            completion,
            recovery,
            heartbeat,
            (!self.session_lost()).then_some(self.session.seat_clock.next_poll),
            self.reaction_wake(),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    /// When the pending hero reaction offer's window runs out, if one is pending.
    fn reaction_wake(&self) -> Option<Instant> {
        if self.fenced || self.session_lost() {
            return None;
        }
        let pending = super::journey::tactical::pending_reaction(self.owner.checkpoint()).ok()?;
        let now_unix_ms = super::journey::tactical::unix_now_ms().ok()?;
        self.session
            .reaction
            .wake_at(pending.as_ref(), Instant::now(), now_unix_ms)
    }

    /// Declines the pending hero reaction offer whose window has run out, through the same
    /// admission and commit path as the hero's own "Let it go": the interrupted turn resumes.
    /// An answer that committed first leaves nothing pending here, so this does nothing.
    fn expire_reaction_if_due(&mut self) {
        use super::journey::tactical;
        if self.fenced || self.session_lost() {
            return;
        }
        let now = Instant::now();
        let read = tactical::pending_reaction(self.owner.checkpoint())
            .and_then(|pending| tactical::unix_now_ms().map(|now_unix_ms| (pending, now_unix_ms)));
        let (pending, now_unix_ms) = match read {
            Ok(read) => read,
            Err(error) => {
                record_unavailable("reaction-window", &format!("{error:?}"));
                return;
            }
        };
        let Some(deadline) = self
            .session
            .reaction
            .due(pending.as_ref(), now, now_unix_ms)
        else {
            return;
        };
        let Some(pending) = pending else {
            return;
        };
        let expiry_line = |answer: &str| {
            format!(
                "REACTION expired hero={:?} provoker={:?} deadline_ms={deadline} late_ms={} answer={answer}",
                pending.reactor_name,
                pending.provoker_name,
                now_unix_ms.saturating_sub(deadline)
            )
        };
        let Some(&credential) = self
            .session
            .reaction
            .credentials
            .get(pending.member.as_bytes())
        else {
            self.session.reaction.retry_later(pending.reaction, now);
            record_audit_line(
                "gameplay.reaction_expired",
                expiry_line("waiting reason=no-seat-credential"),
            );
            return;
        };
        let request =
            match tactical::expired_reaction_decline(self.owner.checkpoint(), &pending, deadline) {
                Ok(request) => request,
                Err(error) => {
                    self.session.reaction.retry_later(pending.reaction, now);
                    record_unavailable("reaction-expiry", &format!("{error:?}"));
                    return;
                }
            };
        // The ordinary call path repairs a closed grant connection first; so does this one.
        if let Err(error) = self.issuer.reconnect_if_closed() {
            self.session.reaction.retry_later(pending.reaction, now);
            record_audit_line(
                "gameplay.reaction_expired",
                expiry_line(&format!("waiting reason=reconnect-{error:?}")),
            );
            return;
        }
        let _cleared = refusal_detail::take();
        self.session.reaction.expiring = Some(pending.clone());
        let result = self.submit(credential, request);
        self.session.reaction.expiring = None;
        let answer = if accepted_now(&result) {
            "declined".to_owned()
        } else {
            self.session.reaction.retry_later(pending.reaction, now);
            match &result {
                Ok(response) => format!("refused outcome={:?}", response.outcome),
                Err(status) => format!("unavailable status={:?}", status.message()),
            }
        };
        record_audit_line("gameplay.reaction_expired", expiry_line(&answer));
    }

    /// Renews the session lease and grants when the heartbeat is due. Called at every wake and
    /// before every call, so a call after a long pause (a suspended machine) renews first.
    /// Ownership taken over by another process is the one failure that cannot heal.
    fn heartbeat_if_due(&mut self) {
        let now = Instant::now();
        if self.session_lost() || !self.session.heartbeat.due(now) {
            return;
        }
        match self.issuer.renew_owner() {
            Ok(()) => {
                if self.session.heartbeat.renewed(now) {
                    self.session_event("SESSION lease renewed", "session.lease_renewed");
                }
            }
            Err(error) => match super::session_recovery::classify(error) {
                super::session_recovery::Resolution::Lost(_)
                | super::session_recovery::Resolution::Retry(RepositoryError::RevisionConflict) => {
                    self.mark_lost(error);
                }
                _ => {
                    if self.session.heartbeat.failed(now) {
                        self.session_event(
                            &format!("SESSION lease renewal failing ({error:?}); retrying"),
                            "session.lease_failing",
                        );
                    }
                }
            },
        }
    }

    /// The saved state is gone for good: fence the session, say so, and tell the watchers.
    fn mark_lost(&mut self, error: RepositoryError) {
        self.fenced = true;
        if !self.session_lost() {
            self.session.fence = Some(super::session_recovery::Health::lost(Instant::now()));
            self.session_event(
                &format!(
                    "SESSION lost ({error:?}): the saved state cannot be recovered; restart the server"
                ),
                "session.lost",
            );
            self.recovery_wakeup
                .send_replace(self.owner.checkpoint().clone());
        }
    }

    pub(super) fn wake_completion(&mut self) {
        self.heartbeat_if_due();
        self.heal_session();
        self.poll_away_seats(Instant::now());
        if self
            .completion_retry
            .as_ref()
            .and_then(|retry| retry.next)
            .is_some_and(|due| Instant::now() >= due)
        {
            self.run_committed_intents();
        }
        self.expire_reaction_if_due();
    }

    /// Whether the saved state was lost and only a restart can help.
    pub(super) fn session_lost(&self) -> bool {
        self.session
            .fence
            .as_ref()
            .is_some_and(super::session_recovery::Health::is_lost)
    }

    /// While fenced, retries the retained operation when it is due, with a backoff and for as
    /// long as the process lives, and says so when the session fences, recovers or is lost.
    fn heal_session(&mut self) {
        if !self.fenced {
            if self.session.fence.take().is_some() {
                self.session_event("SESSION recovered", "session.recovered");
            }
            return;
        }
        let now = Instant::now();
        if self.session.fence.is_none() {
            let health = super::session_recovery::Health::fenced(now, self.session.commit_millis);
            let line = health.fenced_line();
            self.session.fence = Some(health);
            self.session_event(&line, "session.fenced");
        }
        let due = self
            .session
            .fence
            .as_ref()
            .and_then(super::session_recovery::Health::next_attempt)
            .is_some_and(|due| now >= due);
        if !due {
            return;
        }
        let resolution = self.attempt_recovery();
        let now = Instant::now();
        if let Some(fence) = self.session.fence.as_mut() {
            fence.attempted(now, &resolution);
        }
        match resolution {
            super::session_recovery::Resolution::Recovered => {
                let (down, attempts) = self
                    .session
                    .fence
                    .take()
                    .map_or((Duration::ZERO, 0), |fence| {
                        (fence.down_for(now), fence.attempts())
                    });
                self.fenced = false;
                self.observe_campaign();
                self.recovery_wakeup
                    .send_replace(self.owner.checkpoint().clone());
                self.session_event(
                    &format!(
                        "SESSION recovered (after {} ms, {attempts} attempts)",
                        down.as_millis()
                    ),
                    "session.recovered",
                );
            }
            super::session_recovery::Resolution::Lost(error) => {
                self.session_event(
                    &format!(
                        "SESSION lost ({error:?}): the saved state cannot be recovered; restart the server"
                    ),
                    "session.lost",
                );
                // Wake the watchers so they are told, instead of waiting on a stream.
                self.recovery_wakeup
                    .send_replace(self.owner.checkpoint().clone());
            }
            super::session_recovery::Resolution::Retry(_) => {}
        }
    }

    fn attempt_recovery(&mut self) -> super::session_recovery::Resolution {
        use super::session_recovery::{Resolution, classify, recover};
        let context = OperationContext {
            trace_parent: String::new(),
            build: crate::BUILD_ID.to_owned(),
        };
        if let Err(error) = self.issuer.reconnect_if_closed() {
            return classify(error);
        }
        // An expired lease or grant is repaired first: it is what a fence after a long idle is.
        if let Err(error) = self.issuer.renew_owner() {
            return match classify(error) {
                Resolution::Retry(RepositoryError::RevisionConflict) => Resolution::Lost(error),
                other => other,
            };
        }
        self.session.heartbeat.renewed(Instant::now());
        match self.session.attempt.clone() {
            Some(Attempt::Input {
                credential,
                fingerprint,
                input,
            }) => {
                let scope = match model::random().and_then(|random| {
                    model::pins().and_then(|pins| {
                        self.issuer
                            .issue(&credential, random, fingerprint, (*input).clone(), pins)
                    })
                }) {
                    Ok(scope) => scope,
                    Err(error) => return classify(error),
                };
                recover(&mut self.owner, scope, &input, &context)
            }
            Some(Attempt::Completion) => match self.complete_courier(&context) {
                Ok(()) if self.owner.is_current() => Resolution::Recovered,
                Ok(()) => Resolution::Retry(RepositoryError::UnresolvedCommit),
                Err(error) => classify(error),
            },
            None => Resolution::Retry(RepositoryError::Unavailable),
        }
    }

    /// One `SESSION` line on stdout and the same fact on the shared observability path.
    fn session_event(&self, line: &str, event: &'static str) {
        println!("{line}");
        let context = OperationContext {
            trace_parent: String::new(),
            build: crate::BUILD_ID.to_owned(),
        };
        let mut span = df_observe::begin(&context, "gameplay.session");
        span.event(event, &[("session.detail", line.to_owned())]);
        span.finish_unmeasured(event);
    }
    /// Called on startup and at input boundaries by this same serialization owner.
    /// Notification loss cannot erase the backlog: work comes from a validated
    /// durable reload, while terminal state comes from the committed checkpoint.
    pub(super) fn run_committed_intents(&mut self) {
        let pending = self
            .owner
            .checkpoint()
            .state()
            .intents
            .iter()
            .find(|intent| {
                intent.kind == df_model::checkpoint::EffectKind::RunAi
                    && intent.status == df_model::checkpoint::DurableStatus::Pending
            })
            .map(|intent| intent.id);
        let now = Instant::now();
        if let Some(intent) = pending {
            if self
                .completion_retry
                .as_ref()
                .is_none_or(|retry| retry.intent != intent)
            {
                self.completion_retry = Some(CompletionRetry {
                    intent,
                    started: now,
                    attempts: 0,
                    next: Some(now),
                });
            }
            // Player/view traffic cannot bypass spacing or restart a refused/exhausted
            // intent's lifetime. A different durable intent gets its own finite budget.
            if self
                .completion_retry
                .as_mut()
                .is_some_and(|retry| !retry.ready(now))
            {
                return;
            }
        }
        let context = OperationContext {
            trace_parent: String::new(),
            build: crate::BUILD_ID.to_owned(),
        };
        let mut span = df_observe::begin(&context, "gameplay.courier_intents");
        let result = self.complete_courier(&context);
        let outcome = match (result, pending) {
            (Err(error), Some(_)) => {
                let now = Instant::now();
                match self.completion_retry.as_mut() {
                    Some(retry) => retry.failed(now, error),
                    None => "completion_retry_exhausted_retained_pending",
                }
            }
            (Err(_), None) => "durable_completion_pending",
            (Ok(()), _) => {
                self.completion_retry = None;
                "durable_work_checked"
            }
        };
        #[cfg(test)]
        if let Some(observer) = &self.completion_observer
            && observer.try_send((result, self.calls_remaining)).is_err()
        {
            span.finish_unmeasured("qualification_completion_observation_lost");
            return;
        }
        span.finish_unmeasured(outcome);
    }

    fn complete_courier(&mut self, context: &OperationContext) -> Result<(), RepositoryError> {
        match self.intent_notifications.try_recv() {
            Ok(()) | Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                return Err(RepositoryError::Unavailable);
            }
        }
        let Some(intent) = self
            .owner
            .checkpoint()
            .state()
            .intents
            .iter()
            .find(|intent| {
                intent.kind == df_model::checkpoint::EffectKind::RunAi
                    && intent.status == df_model::checkpoint::DurableStatus::Pending
            })
            .cloned()
        else {
            return Ok(());
        };
        let member =
            super::courier_ai::recipient(self.owner.checkpoint(), &intent).map_err(|_| {
                refusal_detail::note("completion-recipient");
                RepositoryError::InvalidCandidate
            })?;
        let completion = super::courier_ai::expected_completion(&intent).map_err(|_| {
            refusal_detail::note("completion-expected");
            RepositoryError::InvalidCandidate
        })?;
        let operation = super::courier_ai::completion_operation(&intent).map_err(|_| {
            refusal_detail::note("completion-operation");
            RepositoryError::InvalidCandidate
        })?;
        let fingerprint = super::courier_ai::completion_fingerprint(&completion).map_err(|_| {
            refusal_detail::note("completion-fingerprint");
            RepositoryError::InvalidCandidate
        })?;
        if !self.fenced {
            self.session.attempt = Some(Attempt::Completion);
        }
        self.issuer.reconnect_if_closed()?;
        let scope = self.issuer.issue_completion(
            self.owner.checkpoint(),
            completion.clone(),
            member,
            operation,
            (model::random()?, fingerprint),
        )?;
        let input = GameInput::Job(completion);
        if self.owner.has_uncertain_operation() {
            if !self.owner.matches_uncertain_retry(&scope, &input)? {
                return Err(RepositoryError::UnresolvedCommit);
            }
        } else {
            self.owner.reload_current(&scope, context)?;
            if !self
                .owner
                .checkpoint()
                .state()
                .intents
                .iter()
                .any(|record| {
                    record.id == intent.id
                        && record.status == df_model::checkpoint::DurableStatus::Pending
                })
            {
                return Ok(());
            }
        }
        let (item, receipt) = OwnedInput::new(
            OperationContext {
                trace_parent: context.trace_parent.clone(),
                build: context.build.clone(),
            },
            scope,
            input,
        );
        self.owner.reduce(AdmissionSequence(0), item);
        let outcome = receipt
            .try_recv()
            .map_err(|_| RepositoryError::Unavailable)?;
        if !self.owner.is_current() {
            self.fence();
        }
        match outcome {
            SubmissionOutcome::Confirmed(_) if self.owner.is_current() => {
                self.fenced = false;
                // Receipt recovery reloads canonical state without redispatching a
                // committed effect. Publish that restored state to existing watches.
                self.observe_campaign();
                self.recovery_wakeup.send_if_modified(|published| {
                    if published == self.owner.checkpoint() {
                        false
                    } else {
                        *published = self.owner.checkpoint().clone();
                        true
                    }
                });
                Ok(())
            }
            SubmissionOutcome::Refused(error) => Err(error),
            SubmissionOutcome::Confirmed(_)
            | SubmissionOutcome::LookupRequired
            | SubmissionOutcome::ExpiredOrIndeterminate => Err(RepositoryError::UnresolvedCommit),
            SubmissionOutcome::OperationConflict => Err(RepositoryError::InputBinding),
        }
    }
    pub(super) fn fence(&mut self) {
        fence_checkpoint(
            &mut self.fenced,
            &self.recovery_wakeup,
            self.owner.checkpoint(),
        );
    }
    /// A seated player's change to the room's table settings: validated, stored, then carried in
    /// every view. It moves no game revision and touches no rule.
    fn set_table_setting(
        &mut self,
        member: df_types::MemberId,
        request: &rpc::SubmitActionRequest,
    ) -> Result<rpc::SubmitActionResponse, tonic::Status> {
        require_readable(self.fenced)?;
        let current = self.owner.checkpoint().basis();
        let scope_invalid = || tonic::Status::invalid_argument("required action scope invalid");
        let session =
            df_api::session_id(request.session_id.as_ref()).map_err(|_| scope_invalid())?;
        let run = df_api::run_id(request.run_id.as_ref()).map_err(|_| scope_invalid())?;
        let operation =
            df_api::operation_id(request.operation_id.as_ref()).map_err(|_| scope_invalid())?;
        if session != current.session || run != current.run {
            return Err(tonic::Status::permission_denied(
                "local gameplay scope denied",
            ));
        }
        if !table_settings::may_change(self.owner.checkpoint(), member) {
            return Err(tonic::Status::permission_denied(
                "only a seated player can change the table settings",
            ));
        }
        let change = request
            .table_setting
            .as_ref()
            .ok_or_else(|| tonic::Status::invalid_argument("table setting required"))?;
        let settings = self.table_settings.changed(change)?;
        self.issuer
            .store_table_settings(settings.stored())
            .map_err(unavailable)?;
        self.table_settings = settings;
        self.table_log.settings_changed();
        Ok(wire::committed(wire::receipt(
            current,
            operation,
            rpc::decision_receipt::Outcome::Accepted(Box::default()),
            false,
        )))
    }

    /// Admit the small set of NPC reply events that can have an immediate, observable world
    /// effect. Jev only supplies a proposal; this method is the sole authority that checks the
    /// exact basis, current public source line, canonical speaker, target and recipient before
    /// committing it.
    fn npc_event(
        &mut self,
        credential: &[u8; 32],
        proposal: Proposal,
    ) -> Result<NpcEventOutcome, tonic::Status> {
        if self.session_lost() {
            return Err(session_lost_status());
        }
        require_readable(self.fenced)?;
        if self.issuer.authenticate(credential).map_err(unavailable)? != LocalDemoRole::Player {
            return Err(tonic::Status::permission_denied(
                "NPC event needs a seated player binding",
            ));
        }
        let member = self.dialogue_member(credential)?;
        let current = self.owner.checkpoint().clone();
        let scene = super::beat_script_runtime::scene(current.state()).map_err(unavailable)?;
        let snapshot = super::journey::living_scene::snapshot(current.state(), &scene)
            .map_err(unavailable)?
            .ok_or_else(|| tonic::Status::failed_precondition("scene snapshot unavailable"))?;
        let Some(canonical_actor) = snapshot
            .model
            .people
            .iter()
            .find(|person| person.id == proposal.actor_id)
        else {
            record_npc_event_status(
                &proposal,
                "ignored",
                "unknown_speaker",
                &revision_key(current.basis().revision),
            );
            return Ok(NpcEventOutcome::status(
                "ignored_unknown_speaker",
                revision_key(current.basis().revision),
            ));
        };
        let dossier_ids: Vec<String> = [
            rpc::SceneRole::Patron,
            rpc::SceneRole::Contact,
            rpc::SceneRole::Antagonist,
            rpc::SceneRole::Bystander,
        ]
        .into_iter()
        .filter_map(|role| self.social.dossier_of(role, &canonical_actor.id))
        .map(|dossier| dossier.npc_id.clone())
        .collect();
        let dossier_ids: Vec<&str> = dossier_ids.iter().map(String::as_str).collect();
        let dossier_matches_actor = super::npc_event_classifier::binds_speaker(
            &dossier_ids,
            &canonical_actor.id,
            &proposal.npc_id,
        );
        if !dossier_matches_actor {
            record_npc_event_status(
                &proposal,
                "ignored",
                "wrong_speaker_binding",
                &revision_key(current.basis().revision),
            );
            return Ok(NpcEventOutcome::status(
                "ignored_wrong_speaker",
                revision_key(current.basis().revision),
            ));
        }

        let expected_scene_identity = format!("{}|{}", scene.title, scene.asset);
        let expected_source_line_id = Proposal::source_line_id(
            &proposal.actor_id,
            &proposal.npc_id,
            &proposal.scene_identity,
            &proposal.revision,
            &proposal.typed_words,
            &proposal.reply_text,
        );
        if proposal.scene_identity != expected_scene_identity
            || proposal.source_line_id != expected_source_line_id
        {
            record_npc_event_status(
                &proposal,
                "ignored",
                "source_identity_mismatch",
                &revision_key(current.basis().revision),
            );
            return Ok(NpcEventOutcome::status(
                "ignored_invalid_source",
                revision_key(current.basis().revision),
            ));
        }
        let digest = Sha256::digest(proposal.source_line_id.as_bytes());
        let mut operation_bytes = [0_u8; 16];
        operation_bytes.copy_from_slice(&digest[..16]);
        let operation = df_types::OperationId::from_bytes(&operation_bytes)
            .map_err(|_| tonic::Status::failed_precondition("NPC event operation invalid"))?;
        if current
            .state()
            .decisions
            .iter()
            .any(|decision| decision.operation == operation)
        {
            record_npc_event_status(
                &proposal,
                "duplicate",
                "operation_already_journaled",
                &revision_key(current.basis().revision),
            );
            return Ok(NpcEventOutcome::status(
                "duplicate",
                revision_key(current.basis().revision),
            ));
        }
        let current_revision = revision_key(current.basis().revision);
        if proposal.revision != current_revision {
            record_npc_event_status(&proposal, "stale", "revision_changed", &current_revision);
            return Ok(NpcEventOutcome::status("stale", current_revision));
        }
        let Some(source_sequence) = proposal.source_line_sequence else {
            record_npc_event_status(
                &proposal,
                "ignored",
                "missing_source_sequence",
                &current_revision,
            );
            return Ok(NpcEventOutcome::status(
                "ignored_line_not_current",
                current_revision.clone(),
            ));
        };
        let Some(source_text) = self.table_log.latest_npc_speech_text(
            source_sequence,
            &proposal.actor_id,
            &canonical_actor.name,
            &proposal.reply_text,
        ) else {
            record_npc_event_status(
                &proposal,
                "ignored",
                "source_line_not_latest_public_speech",
                &current_revision,
            );
            return Ok(NpcEventOutcome::status(
                "ignored_line_not_current",
                current_revision.clone(),
            ));
        };
        if let Some(quote) = proposal.trade_quote.as_ref() {
            let hero = super::journey::player_entity(member, &current).map_err(unavailable)?;
            let hero_id = super::journey::name(current.state(), hero).map_err(unavailable)?;
            let mut terms = super::journey::living_scene::merchant_trade_terms(&current, &hero_id)
                .map_err(unavailable)?;
            if quote.direction == super::journey::living_scene::SceneTradeDirection::HeroSells {
                let place_id = scene
                    .setting
                    .as_ref()
                    .map(|setting| setting.place_id.as_str())
                    .ok_or_else(|| {
                        tonic::Status::failed_precondition("merchant place unavailable")
                    })?;
                super::journey::merchant_bargaining::apply_committed_buyback_context(
                    &current,
                    &mut terms,
                    place_id,
                    &quote.item_id,
                    &hero_id,
                    &proposal.actor_id,
                )
                .map_err(unavailable)?;
            }
            // The judgement that created this candidate was made by the service before the
            // proposal existed; the actor repeats only the deterministic checks.
            let validated = match super::journey::merchant_bargaining::revalidate_quote(
                &terms,
                &hero_id,
                &proposal.actor_id,
                quote.direction,
                &quote.item_id,
                &source_text,
            ) {
                Ok(validated) => validated,
                Err(reason) => {
                    record_npc_event_status(
                        &proposal,
                        "rejected",
                        reason.label(),
                        &current_revision,
                    );
                    return Ok(NpcEventOutcome::status("rejected", current_revision));
                }
            };
            if &validated != quote {
                record_npc_event_status(
                    &proposal,
                    "rejected",
                    "candidate_mismatch",
                    &current_revision,
                );
                return Ok(NpcEventOutcome::status("rejected", current_revision));
            }
            let next =
                current.basis().revision.next_sequence().map_err(|_| {
                    tonic::Status::failed_precondition("revision sequence exhausted")
                })?;
            let candidate_checkpoint = match super::journey::living_scene::commit_npc_trade_quote(
                &current,
                super::journey::living_scene::NpcTradeQuoteProposal {
                    operation,
                    expected_basis: current.basis(),
                    actor_id: proposal.actor_id.clone(),
                    customer_id: hero_id.clone(),
                    direction: quote.direction,
                    item_id: quote.item_id.clone(),
                    offer_id: quote.offer_id.clone(),
                    price_cp: quote.price_cp,
                    source: super::journey::living_scene::NpcInventorySource {
                        source_line_id: proposal.source_line_id.clone(),
                        source_revision: current.basis().revision,
                        source_line_sequence: source_sequence,
                        source_actor_id: proposal.actor_id.clone(),
                        source_text,
                    },
                },
            ) {
                Ok(candidate) => candidate,
                Err(error) => {
                    record_npc_event_status(
                        &proposal,
                        "rejected",
                        "inventory_quote_validation_rejected",
                        &current_revision,
                    );
                    return Err(unavailable(error));
                }
            };
            if candidate_checkpoint.basis().revision != next {
                record_npc_event_status(
                    &proposal,
                    "rejected",
                    "candidate_revision_not_single_step",
                    &current_revision,
                );
                return Err(tonic::Status::failed_precondition(
                    "merchant quote revision did not advance exactly once",
                ));
            }
            let speaker = current
                .state()
                .characters
                .iter()
                .find(|character| character.owner == member)
                .map(|character| character.entity)
                .ok_or_else(|| tonic::Status::failed_precondition("seat has no hero"))?;
            let input = GameInput::Game(CommandInput {
                basis: current.basis(),
                observed_revision: current.basis().revision,
                operation,
                member,
                command: GameCommand::Speak {
                    speaker,
                    text: proposal.source_line_id.clone(),
                    conversation: None,
                },
            });
            let fingerprint: [u8; 32] = Sha256::digest(proposal.source_line_id.as_bytes()).into();
            let scope = self
                .issuer
                .issue(
                    credential,
                    model::random().map_err(unavailable)?,
                    fingerprint,
                    input,
                    model::pins().map_err(unavailable)?,
                )
                .map_err(unavailable)?;
            let operation_context = OperationContext {
                trace_parent: String::new(),
                build: crate::BUILD_ID.to_owned(),
            };
            match self.owner.commit_owner_candidate(
                &scope,
                &candidate_checkpoint,
                &operation_context,
            ) {
                SubmissionOutcome::Confirmed(receipt) => {
                    super::journey::living_scene::publish(&candidate_checkpoint)
                        .map_err(unavailable)?;
                    let direction = match quote.direction {
                        super::journey::merchant_bargaining::TradeDirection::HeroBuys => {
                            "hero_buys"
                        }
                        super::journey::merchant_bargaining::TradeDirection::HeroSells => {
                            "hero_sells"
                        }
                    };
                    record_audit_diagnostic(
                        "gameplay.merchant.quote",
                        format!(
                            "MERCHANT_QUOTE status=issued direction={direction} item={} price_cp={}",
                            quote.item_id, quote.price_cp
                        ),
                        serde_json::Map::from_iter([
                            ("status".into(), serde_json::json!("issued")),
                            ("direction".into(), serde_json::json!(direction)),
                            ("item_id".into(), serde_json::json!(quote.item_id)),
                            ("price_cp".into(), serde_json::json!(quote.price_cp)),
                            (
                                "source_line_id".into(),
                                serde_json::json!(proposal.source_line_id),
                            ),
                            (
                                "source_line_sequence".into(),
                                serde_json::json!(source_sequence),
                            ),
                            (
                                "committed_revision".into(),
                                serde_json::json!(revision_key(receipt.basis().revision)),
                            ),
                        ]),
                        &[
                            ("gameplay.merchant.quote.status", "issued".into()),
                            ("gameplay.merchant.quote.direction", direction.into()),
                        ],
                    );
                    if let Some(line) =
                        super::journey::living_scene::last_line(candidate_checkpoint.state())
                            .map_err(unavailable)?
                    {
                        self.table_log.push_actor(
                            &proposal.actor_id,
                            &canonical_actor.name,
                            rpc::TableRole::Npc,
                            0,
                            rpc::TableKind::Action,
                            &line,
                        );
                    }
                    return Ok(NpcEventOutcome {
                        status: "committed",
                        revision: revision_key(receipt.basis().revision),
                        operation_id: Some(super::journey::hex(operation.as_bytes())),
                        source_line_id: Some(proposal.source_line_id),
                        item_id: Some(quote.item_id.clone()),
                        holder_id: None,
                        owner_id: None,
                        item_detail: None,
                    });
                }
                SubmissionOutcome::Refused(error) => {
                    record_npc_event_status(
                        &proposal,
                        "rejected",
                        "owner_refused_candidate",
                        &current_revision,
                    );
                    return Err(unavailable(error));
                }
                SubmissionOutcome::OperationConflict => {
                    record_npc_event_status(
                        &proposal,
                        "conflict",
                        "owner_operation_conflict",
                        &current_revision,
                    );
                    return Err(tonic::Status::aborted("merchant quote conflict"));
                }
                SubmissionOutcome::LookupRequired | SubmissionOutcome::ExpiredOrIndeterminate => {
                    self.fenced = true;
                    record_npc_event_status(
                        &proposal,
                        "unavailable",
                        "owner_commit_outcome_indeterminate",
                        &current_revision,
                    );
                    return Err(tonic::Status::unavailable(
                        "merchant quote commit outcome unavailable",
                    ));
                }
            }
        }
        if !matches!(
            proposal.kind,
            EventKind::IntentToTake | EventKind::IntentToGive
        ) {
            let status = match proposal.kind {
                EventKind::IntentToBuy => "awaiting_quote_acceptance",
                EventKind::Conditional => "conditional_no_commit",
                EventKind::Refusal => "refusal_no_commit",
                EventKind::None => "ignored",
                EventKind::Reveal | EventKind::Offer => "proposal_only",
                EventKind::IntentToTake | EventKind::IntentToGive => unreachable!(),
            };
            let reason = match proposal.kind {
                EventKind::IntentToBuy => "explicit_quote_acceptance_required",
                EventKind::Conditional => "conditional_narrative_only",
                EventKind::Refusal => "refusal_narrative_only",
                EventKind::None => "no_event_proposed",
                EventKind::Reveal | EventKind::Offer => "narrative_proposal_only",
                EventKind::IntentToTake | EventKind::IntentToGive => unreachable!(),
            };
            record_npc_event_status(&proposal, status, reason, &current_revision);
            return Ok(NpcEventOutcome::status(status, current_revision));
        }
        let Some(item_id) = proposal.item_id.clone() else {
            record_npc_event_status(
                &proposal,
                "ignored",
                "missing_canonical_item",
                &current_revision,
            );
            return Ok(NpcEventOutcome::status(
                "ignored_missing_item",
                current_revision,
            ));
        };
        if let Err(reason) = super::npc_event_classifier::validate_item_event(
            proposal.kind,
            &item_id,
            &proposal.typed_words,
            &proposal.reply_text,
            &snapshot,
        ) {
            record_npc_event_status(&proposal, "rejected", reason.label(), &current_revision);
            return Ok(NpcEventOutcome::status("rejected", current_revision));
        }
        let action = match proposal.kind {
            EventKind::IntentToTake => super::journey::living_scene::NpcInventoryAction::Take,
            EventKind::IntentToGive => {
                let hero = super::journey::player_entity(member, &current).map_err(unavailable)?;
                let hero_id = super::journey::name(current.state(), hero).map_err(unavailable)?;
                if proposal.recipient_id.as_deref() != Some(hero_id.as_str()) {
                    record_npc_event_status(
                        &proposal,
                        "rejected",
                        "recipient_not_authenticated_hero",
                        &current_revision,
                    );
                    return Ok(NpcEventOutcome::status(
                        "ignored_wrong_recipient",
                        current_revision,
                    ));
                }
                super::journey::living_scene::NpcInventoryAction::Give {
                    recipient_id: hero_id,
                    transfer_ownership: false,
                    owner_consent_id: None,
                }
            }
            _ => unreachable!("only inventory events reach the inventory adapter"),
        };
        let next = current
            .basis()
            .revision
            .next_sequence()
            .map_err(|_| tonic::Status::failed_precondition("revision sequence exhausted"))?;
        let candidate = match super::journey::living_scene::commit_npc_inventory_action(
            &current,
            super::journey::living_scene::NpcInventoryProposal {
                operation,
                expected_basis: current.basis(),
                actor_id: proposal.actor_id.clone(),
                target_id: item_id.clone(),
                action,
                source: Some(super::journey::living_scene::NpcInventorySource {
                    source_line_id: proposal.source_line_id.clone(),
                    source_line_sequence: source_sequence,
                    source_revision: current.basis().revision,
                    source_actor_id: proposal.actor_id.clone(),
                    source_text,
                }),
            },
        ) {
            Ok(candidate) => candidate,
            Err(error) => {
                record_npc_event_status(
                    &proposal,
                    "rejected",
                    "inventory_owner_validation_rejected",
                    &current_revision,
                );
                return Err(unavailable(error));
            }
        };
        if candidate.basis().revision != next {
            record_npc_event_status(
                &proposal,
                "rejected",
                "candidate_revision_not_single_step",
                &current_revision,
            );
            return Err(tonic::Status::failed_precondition(
                "NPC event revision did not advance exactly once",
            ));
        }
        let speaker = current
            .state()
            .characters
            .iter()
            .find(|character| character.owner == member)
            .map(|character| character.entity)
            .ok_or_else(|| tonic::Status::failed_precondition("seat has no hero"))?;
        let input = GameInput::Game(CommandInput {
            basis: current.basis(),
            observed_revision: current.basis().revision,
            operation,
            member,
            command: GameCommand::Speak {
                speaker,
                text: proposal.source_line_id.clone(),
                conversation: None,
            },
        });
        let fingerprint: [u8; 32] = Sha256::digest(proposal.source_line_id.as_bytes()).into();
        let scope = self
            .issuer
            .issue(
                credential,
                model::random().map_err(unavailable)?,
                fingerprint,
                input,
                model::pins().map_err(unavailable)?,
            )
            .map_err(unavailable)?;
        let context = OperationContext {
            trace_parent: String::new(),
            build: crate::BUILD_ID.to_owned(),
        };
        match self
            .owner
            .commit_owner_candidate(&scope, &candidate, &context)
        {
            SubmissionOutcome::Confirmed(receipt) => {
                super::journey::living_scene::publish(&candidate).map_err(unavailable)?;
                let committed_scene =
                    super::beat_script_runtime::scene(candidate.state()).map_err(unavailable)?;
                let committed_snapshot =
                    super::journey::living_scene::snapshot(candidate.state(), &committed_scene)
                        .map_err(unavailable)?
                        .ok_or_else(|| {
                            tonic::Status::failed_precondition("scene snapshot unavailable")
                        })?;
                let committed_thing = committed_snapshot
                    .model
                    .things
                    .iter()
                    .find(|thing| thing.id == item_id);
                let item_detail = committed_thing.map(|thing| thing.detail.clone());
                let holder_id = committed_thing.and_then(|thing| thing.holder_id.clone());
                let owner_id = committed_thing.and_then(|thing| thing.owner_id.clone());
                let committed_line = super::journey::living_scene::last_line(candidate.state())
                    .map_err(unavailable)?;
                let operation_id = super::journey::hex(operation.as_bytes());
                record_audit_diagnostic(
                    "gameplay.npc.reply_event",
                    format!(
                        "NPC_EVENT status=committed kind={} source_line_id={}",
                        proposal.kind.label(),
                        proposal.source_line_id
                    ),
                    serde_json::Map::from_iter([
                        ("status".into(), serde_json::json!("committed")),
                        ("kind".into(), serde_json::json!(proposal.kind.label())),
                        ("operation_id".into(), serde_json::json!(operation_id)),
                        (
                            "source_line_id".into(),
                            serde_json::json!(proposal.source_line_id),
                        ),
                        ("item_id".into(), serde_json::json!(item_id)),
                        ("holder_id".into(), serde_json::json!(holder_id)),
                        ("owner_id".into(), serde_json::json!(owner_id)),
                        (
                            "source_line_sequence".into(),
                            serde_json::json!(source_sequence),
                        ),
                        (
                            "source_revision".into(),
                            serde_json::json!(proposal.revision),
                        ),
                        (
                            "committed_revision".into(),
                            serde_json::json!(revision_key(receipt.basis().revision)),
                        ),
                    ]),
                    &[
                        ("gameplay.npc.reply_event.status", "committed".into()),
                        (
                            "gameplay.npc.reply_event.operation_id",
                            operation_id.clone(),
                        ),
                    ],
                );
                if let Some(line) = committed_line {
                    self.table_log.push_actor(
                        &proposal.actor_id,
                        &canonical_actor.name,
                        rpc::TableRole::Npc,
                        0,
                        rpc::TableKind::Action,
                        &line,
                    );
                }
                Ok(NpcEventOutcome {
                    status: "committed",
                    revision: revision_key(receipt.basis().revision),
                    operation_id: Some(operation_id),
                    source_line_id: Some(proposal.source_line_id),
                    item_id: Some(item_id),
                    holder_id,
                    owner_id,
                    item_detail,
                })
            }
            SubmissionOutcome::Refused(error) => {
                record_npc_event_status(
                    &proposal,
                    "rejected",
                    "owner_refused_candidate",
                    &current_revision,
                );
                Err(unavailable(error))
            }
            SubmissionOutcome::OperationConflict => {
                record_npc_event_status(
                    &proposal,
                    "conflict",
                    "owner_operation_conflict",
                    &current_revision,
                );
                Err(tonic::Status::aborted("NPC event conflict"))
            }
            SubmissionOutcome::LookupRequired | SubmissionOutcome::ExpiredOrIndeterminate => {
                record_npc_event_status(
                    &proposal,
                    "unavailable",
                    "owner_commit_outcome_indeterminate",
                    &current_revision,
                );
                Err(tonic::Status::unavailable(
                    "NPC event commit outcome unavailable",
                ))
            }
        }
    }

    /// One development jump (DEVCTL-001 milestones 2/3), reduced on the owner
    /// thread: builds the candidate from the owner's own current checkpoint
    /// and commits it through the same compare-and-swap, receipt and
    /// publication path as player decisions, so the database, the checkpoint
    /// and every connected screen stay consistent. Refused while fenced or
    /// when the seat has no hero yet; the commit is scoped as table operation
    /// through the given admitted seat, and the scope binds a spoken marker
    /// carrying the dev note (never submitted as a player action).
    pub(super) fn dev(
        &mut self,
        credential: &[u8; 32],
        order: super::dev_control::commands::DevOrder,
    ) -> Result<super::dev_control::commands::DevResult, tonic::Status> {
        use super::dev_control::{candidate, commands};
        if self.session_lost() {
            return Err(session_lost_status());
        }
        require_readable(self.fenced)?;
        if self.issuer.authenticate(credential).map_err(unavailable)? != LocalDemoRole::Player {
            return Err(tonic::Status::permission_denied(
                "development jumps need a seated player binding",
            ));
        }
        let principal = self.issuer.principal(credential).map_err(unavailable)?;
        let member = df_types::MemberId::from_bytes(&principal)
            .map_err(|_| tonic::Status::permission_denied("member binding invalid"))?;
        let current = self.owner.checkpoint().clone();
        let next = current
            .basis()
            .revision
            .next_sequence()
            .map_err(|_| tonic::Status::failed_precondition("revision sequence exhausted"))?;
        let restore_moment = match &order.job {
            commands::DevJob::Restore { moment, .. } => Some(moment.clone()),
            _ => None,
        };
        // A rewind marker carries the dev policy no view projection reads, so
        // replays walk exactly the saved moment's game decisions and skip the
        // marker itself; anything else keeps its author's admitted policy.
        let policy = if restore_moment.is_some() {
            df_types::RevisionLabel::new(Some(df_session::submission::DEV_REWIND_POLICY))
                .map_err(|_| tonic::Status::failed_precondition("dev policy refused"))?
        } else {
            current
                .state()
                .decisions
                .last()
                .map(|decision| decision.source_policy.clone())
                .ok_or_else(|| tonic::Status::failed_precondition("no decisions committed yet"))?
        };
        let operation = loop {
            let bytes = model::random::<16>().map_err(unavailable)?;
            if let Ok(operation) = df_types::OperationId::from_bytes(&bytes) {
                break operation;
            }
        };
        let change = match &order.job {
            commands::DevJob::Restore { moment, what } => candidate::Change::Restore {
                saved: Box::new(moment.checkpoint.clone()),
                what: what.clone(),
            },
            commands::DevJob::HitPoints { member, value } => {
                let member = df_types::MemberId::from_bytes(member)
                    .map_err(|_| tonic::Status::invalid_argument("hero member invalid"))?;
                candidate::Change::HitPoints {
                    member,
                    value: *value,
                }
            }
            commands::DevJob::Budget { value, cap } => candidate::Change::Budget {
                value: *value,
                cap: *cap,
            },
            commands::DevJob::NpcTurn { npc, act } => candidate::Change::NpcTurn {
                turn: Box::new(
                    super::journey::tactical::npc_turn(&current, npc, act, operation)
                        .map_err(tonic::Status::failed_precondition)?,
                ),
            },
        };
        let inventory = ReferenceInventory {
            rules: &order.inventory.rules,
            content: &order.inventory.content,
            resources: &order.inventory.resources,
            assets: &order.inventory.assets,
        };
        let built = candidate::build(
            &current,
            &change,
            next,
            operation,
            policy,
            order.job.note(),
            inventory,
            order.inventory.limits,
        )
        .map_err(tonic::Status::failed_precondition)?;
        let speaker = current
            .state()
            .characters
            .iter()
            .find(|character| character.owner == member)
            .map(|character| character.entity)
            .ok_or_else(|| {
                tonic::Status::failed_precondition("this seat has no hero yet; create one first")
            })?;
        let input = GameInput::Game(CommandInput {
            basis: current.basis(),
            observed_revision: current.basis().revision,
            operation,
            member,
            command: GameCommand::Speak {
                speaker,
                text: order.job.note(),
                conversation: None,
            },
        });
        let fingerprint: [u8; 32] = Sha256::digest(order.job.note().as_bytes()).into();
        let scope = self
            .issuer
            .issue(
                credential,
                model::random().map_err(unavailable)?,
                fingerprint,
                input,
                model::pins().map_err(unavailable)?,
            )
            .map_err(unavailable)?;
        let context = OperationContext {
            trace_parent: String::new(),
            build: crate::BUILD_ID.to_owned(),
        };
        let is_restore = restore_moment.is_some();
        let committed = if is_restore {
            self.owner
                .commit_owner_rewind(&scope, &built.candidate, &context)
        } else {
            self.owner
                .commit_owner_candidate(&scope, &built.candidate, &context)
        };
        match committed {
            SubmissionOutcome::Confirmed(receipt) => {
                if !self.owner.is_current() {
                    self.fence();
                }
                // A rewind restores the actor-side state the checkpoint does
                // not carry on the same owner critical section, right after
                // the commit it belongs to (never before: a failed commit
                // must not move the table or the ledger). The history entry
                // the publication hook just recorded is then refreshed with
                // the rewound state, so back/forward from here stays exact.
                if let Some(moment) = restore_moment {
                    self.table_log.rewind_to(moment.table_floor);
                    if let Err(error) = self.social.restore_snapshot(&moment.social) {
                        self.fence();
                        record_unavailable("authority", &format!("social rewind: {error}"));
                        return Err(authority_unavailable());
                    }
                    if let Err(error) = self.notebook.restore_snapshot(&moment.notebook) {
                        self.fence();
                        record_unavailable("authority", &format!("notebook rewind: {error}"));
                        return Err(authority_unavailable());
                    }
                }
                if let candidate::Change::NpcTurn { turn } = &change {
                    match super::journey::unhex(&turn.receipt)
                        .ok()
                        .and_then(|bytes| rpc::AcceptedAction::decode(bytes.as_slice()).ok())
                    {
                        Some(accepted) => self.record_kill_shots(&current, &accepted.combat),
                        None => {
                            record_unavailable("kill-shot-npc-receipt", "invalid committed receipt")
                        }
                    }
                }
                let revision = receipt.basis().revision;
                self.table_log.push(
                    "table",
                    rpc::TableRole::Narrator,
                    0,
                    rpc::TableKind::Action,
                    &format!(
                        "Development {}: {} → {}",
                        order.job.note(),
                        built.before,
                        built.after
                    ),
                );
                // The recorded moment keeps the commit's own lines: the
                // publication hook runs before table lines are pushed.
                self.refresh_dev_moment();
                Ok(commands::DevResult {
                    epoch: revision.epoch().get(),
                    sequence: revision.sequence(),
                    before: built.before,
                    after: built.after,
                })
            }
            SubmissionOutcome::OperationConflict => Err(tonic::Status::failed_precondition(
                "development operation conflict; retry the command",
            )),
            SubmissionOutcome::ExpiredOrIndeterminate => Err(tonic::Status::failed_precondition(
                "development operation expired; retry the command",
            )),
            SubmissionOutcome::LookupRequired => {
                self.fence();
                Err(unavailable(RepositoryError::UnresolvedCommit))
            }
            SubmissionOutcome::Refused(error) => Err(match error {
                RepositoryError::StaleEpoch
                | RepositoryError::StaleFence
                | RepositoryError::ExpiredOwner
                | RepositoryError::RevisionConflict => {
                    self.fence();
                    unavailable(error)
                }
                RepositoryError::InvalidCandidate | RepositoryError::InputBinding => {
                    tonic::Status::failed_precondition(format!(
                        "development candidate refused: {error:?}"
                    ))
                }
                _ => unavailable(error),
            }),
        }
    }

    fn validate_trade_quote_receipt(
        &self,
        member: df_types::MemberId,
        request: &rpc::SubmitActionRequest,
    ) -> Result<(), tonic::Status> {
        let options = request
            .action_choices
            .iter()
            .filter(|choice| choice.group_id == "scene-option")
            .map(|choice| choice.option_id.as_str())
            .collect::<Vec<_>>();
        let quote_ids = request
            .action_choices
            .iter()
            .filter(|choice| choice.group_id == "scene-quote-id")
            .map(|choice| choice.option_id.as_str())
            .collect::<Vec<_>>();
        let is_trade = options.contains(&"trade");
        if !is_trade && quote_ids.is_empty() {
            return Ok(());
        }
        if options.as_slice() != ["trade"] || quote_ids.len() != 1 {
            return Err(tonic::Status::failed_precondition(
                "merchant trade needs one live quote acceptance",
            ));
        }

        let current = self.owner.checkpoint();
        let hero = super::journey::name(
            current.state(),
            super::journey::player_entity(member, current).map_err(unavailable)?,
        )
        .map_err(unavailable)?;
        let quote = super::journey::living_scene::pending_merchant_quote(current, &hero)
            .map_err(unavailable)?
            .filter(|quote| quote.quote_id == quote_ids[0])
            .ok_or_else(|| tonic::Status::failed_precondition("merchant quote is not pending"))?;
        let merchant_id = match quote.direction {
            super::journey::living_scene::SceneTradeDirection::HeroBuys => &quote.seller_id,
            super::journey::living_scene::SceneTradeDirection::HeroSells => &quote.buyer_id,
        };
        if quote.source_actor_id != *merchant_id
            || quote.source_line_sequence == 0
            || quote.source_line_id.is_empty()
            || quote.source_revision.is_empty()
        {
            return Err(tonic::Status::failed_precondition(
                "merchant quote source binding is invalid",
            ));
        }
        Ok(())
    }

    pub(super) fn submit(
        &mut self,
        credential: [u8; 32],
        request: rpc::SubmitActionRequest,
    ) -> Result<rpc::SubmitActionResponse, tonic::Status> {
        if self.session_lost() {
            return Err(session_lost_status());
        }
        if self.issuer.authenticate(&credential).map_err(unavailable)? != LocalDemoRole::Player {
            return Err(tonic::Status::permission_denied(
                "display bindings cannot submit player actions",
            ));
        }

        // An expiry decline is the owner's, not a sign that the silent hero is back.
        if self.session.reaction.expiring.is_none() {
            self.seat_activity(&credential)?;
        }
        let principal = self.issuer.principal(&credential).map_err(unavailable)?;
        if principal == df_persistence::local_demo_scope::PLAYER {
            return Err(tonic::Status::permission_denied(
                "bootstrap scope is not a player client",
            ));
        }
        let member = df_types::MemberId::from_bytes(&principal)
            .map_err(|_| tonic::Status::permission_denied("member binding invalid"))?;
        self.session.reaction.remember(member, credential);
        self.validate_trade_quote_receipt(member, &request)?;
        if rpc::GameplayActionKind::try_from(request.action_kind)
            == Ok(rpc::GameplayActionKind::SetTableSetting)
        {
            return self.set_table_setting(member, &request);
        }
        // A request refused for its shape never reaches the game, so it is logged here: without
        // this line a phone's "Outcome unknown" has no trace on the server.
        let input = wire::journey_input(&request, member, self.owner.checkpoint()).inspect_err(
            |status| {
                record_audit_line(
                    "gameplay.request_invalid",
                    format!(
                        "REQUEST invalid action_kind={} choices={} reason={:?}",
                        request.action_kind,
                        request.action_choices.len(),
                        status.message()
                    ),
                );
            },
        )?;
        let GameInput::Game(command) = &input else {
            return Err(tonic::Status::internal("canonical command unavailable"));
        };
        let operation = command.operation;
        let current = self.owner.checkpoint().basis();
        if command.basis.session != current.session {
            return Err(tonic::Status::permission_denied(
                "local gameplay scope denied",
            ));
        }
        // Fingerprint the exact generated request; retrying requires the same original envelope.
        let fingerprint: [u8; 32] = Sha256::digest(request.encode_to_vec()).into();
        let scope = self
            .issuer
            .issue(
                &credential,
                model::random().map_err(unavailable)?,
                fingerprint,
                input.clone(),
                model::pins().map_err(unavailable)?,
            )
            .map_err(unavailable)?;
        let context = OperationContext {
            trace_parent: String::new(),
            build: crate::BUILD_ID.to_owned(),
        };
        // Kept so that, if this commit's outcome becomes unknown, the owner can retry it itself.
        if !self.fenced {
            self.session.attempt = Some(Attempt::Input {
                credential,
                fingerprint,
                input: Box::new(input.clone()),
            });
        }
        let recovering = self.fenced;
        let replaying_uncertain = recovering && self.owner.has_uncertain_operation();
        if recovering {
            if self.owner.has_uncertain_operation() {
                if !self
                    .owner
                    .matches_uncertain_retry(&scope, &input)
                    .map_err(unavailable)?
                {
                    record_unavailable("uncertain-retry", "a different operation is retained");
                    return Err(tonic::Status::unavailable(
                        "exact retained operation required for recovery",
                    ));
                }
            } else {
                // Known CAS/acknowledged duplicate failures have no unknown write:
                // current database authorization plus validated reload may resume.
                self.owner
                    .reload_current(&scope, &context)
                    .map_err(unavailable)?;
                self.fenced = false;
                self.observe_campaign();
                self.recovery_wakeup
                    .send_replace(self.owner.checkpoint().clone());
            }
        } else {
            let retained = self
                .issuer
                .rejected_operation(&scope, current, None, self.codec)
                .map_err(|error| {
                    if error == RepositoryError::UnresolvedCommit {
                        self.fence();
                    }
                    unavailable(error)
                })?;
            match retained {
                LocalRejectedLookup::Committed(bytes) => {
                    let mut receipt = rpc::DecisionReceipt::decode(bytes.as_slice())
                        .map_err(|_| tonic::Status::internal("retained receipt invalid"))?;
                    receipt.replayed = true;
                    return Ok(wire::committed(receipt));
                }
                LocalRejectedLookup::Conflict => {
                    return Ok(wire::observation(rpc::RejectionCode::OperationConflict));
                }
                LocalRejectedLookup::Expired => {
                    return Ok(wire::observation(rpc::RejectionCode::OperationExpired));
                }
                LocalRejectedLookup::NotRecorded => {
                    let checkpoint = self.owner.checkpoint();
                    let kind = rpc::GameplayActionKind::try_from(request.action_kind)
                        .map_err(|_| tonic::Status::invalid_argument("unknown action"))?;
                    let rejection = if command.basis.run != current.run
                        || command.basis.revision.epoch() != current.revision.epoch()
                        || command.observed_revision > current.revision
                        || (super::journey::tactical::present(checkpoint.state())
                            && command.observed_revision != current.revision)
                    {
                        Some(rpc::RejectionCode::StaleOffer)
                    } else if wire::unrelated_payload(&request, kind)
                        || (kind == rpc::GameplayActionKind::CreateCharacter
                            && request.character.as_ref().is_none_or(|character| {
                                let selections = character
                                    .choices
                                    .iter()
                                    .filter(|choice| {
                                        super::hero_identity::is_character_pack_validation_choice(
                                            &choice.group_id,
                                        )
                                    })
                                    .map(|choice| {
                                        (choice.group_id.as_str(), choice.option_id.as_str())
                                    })
                                    .collect::<Vec<_>>();
                                super::pack::validate_character(&character.name, &selections)
                                    .is_err()
                            }))
                    {
                        Some(rpc::RejectionCode::InvalidSelection)
                    } else {
                        // An offer check that definitively refuses this command
                        // is a refusal the phone can show, recorded like any
                        // other rejection below; only an authority failure is
                        // told to retain its operation and retry.
                        match wire::action_offer_rejection(
                            checkpoint,
                            member,
                            kind,
                            &request.offer_id,
                            &request.action_choices,
                        ) {
                            Ok(rejection) => rejection,
                            Err(error) => {
                                let code = command_refusal_code(error)
                                    .ok_or_else(|| unavailable(error))?;
                                record_refusal(
                                    request.action_kind,
                                    &format!("{error:?}"),
                                    "refused",
                                );
                                Some(code)
                            }
                        }
                    };
                    if let Some(code) = rejection {
                        record_session_machine_refusal(self.owner.checkpoint(), &request);
                        let receipt = wire::rejected(current, operation, code);
                        let bytes = receipt.encode_to_vec();
                        let recorded = self
                            .issuer
                            .rejected_operation(&scope, current, Some(&bytes), self.codec)
                            .map_err(|error| {
                                if error == RepositoryError::UnresolvedCommit {
                                    self.fence();
                                }
                                unavailable(error)
                            })?;
                        return match recorded {
                            LocalRejectedLookup::Committed(bytes) => {
                                rpc::DecisionReceipt::decode(bytes.as_slice())
                                    .map(wire::committed)
                                    .map_err(|_| {
                                        tonic::Status::internal("committed rejection invalid")
                                    })
                            }
                            _ => {
                                record_unavailable("rejection-commit", "requires recovery");
                                Err(tonic::Status::unavailable(
                                    "rejection commit requires recovery",
                                ))
                            }
                        };
                    }
                }
                LocalRejectedLookup::Accepted => {}
            }
        }
        let prior_revision = self.owner.checkpoint().basis().revision;
        let before = self.owner.checkpoint().clone();
        let (item, receipt) = OwnedInput::new(context, scope, input);
        let reservation = self.table_log.reserve();
        let commit_started = Instant::now();
        self.owner.reduce(AdmissionSequence(0), item);
        self.session.commit_millis =
            u64::try_from(commit_started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let outcome = receipt
            .try_recv()
            .map_err(|_| tonic::Status::internal("actor did not produce its outcome"))?;
        if !self.owner.is_current() {
            self.fence();
        }
        match outcome {
            SubmissionOutcome::Confirmed(committed) => {
                if recovering && self.owner.is_current() {
                    self.fenced = false;
                    self.observe_campaign();
                    self.recovery_wakeup
                        .send_replace(self.owner.checkpoint().clone());
                }
                let accepted = super::journey::accepted(committed.decision()).map_err(|error| {
                    record_refusal(request.action_kind, &format!("{error:?}"), "unavailable");
                    authority_unavailable()
                })?;
                let replayed = replaying_uncertain || committed.basis().revision <= prior_revision;
                if !replayed {
                    if let Err(error) =
                        super::journey::living_scene::publish(self.owner.checkpoint())
                    {
                        record_unavailable("scene-publication", &format!("{error:?}"));
                    }
                    self.record_table_action(member, &request, &accepted, &before, reservation);
                    // The recorded moment keeps the commit's own lines: the
                    // publication hook runs before table lines are pushed.
                    self.refresh_dev_moment();
                }
                if committed.basis().revision > prior_revision
                    && self.owner.is_current()
                    && let Some(tempo) = self.tempo.as_mut()
                {
                    tempo::observe(tempo.accepted(
                        self.owner.checkpoint(),
                        &accepted,
                        request.action_kind,
                        Instant::now(),
                    ));
                }
                if let Some(inputs) = self.qualification_inputs.as_mut() {
                    retain_qualification_input(inputs, credential, &request);
                }
                Ok(wire::committed(wire::receipt(
                    committed.basis(),
                    operation,
                    rpc::decision_receipt::Outcome::Accepted(Box::new(accepted)),
                    replaying_uncertain || committed.basis().revision <= prior_revision,
                )))
            }
            SubmissionOutcome::OperationConflict => {
                Ok(wire::observation(rpc::RejectionCode::OperationConflict))
            }
            SubmissionOutcome::ExpiredOrIndeterminate => {
                record_refusal(
                    request.action_kind,
                    "ExpiredOrIndeterminate",
                    "lookup-required",
                );
                self.fence();
                Err(tonic::Status::unavailable("operation lookup required"))
            }
            SubmissionOutcome::LookupRequired => {
                record_refusal(request.action_kind, "LookupRequired", "lookup-required");
                self.fence();
                Err(tonic::Status::unavailable("operation lookup required"))
            }
            // The owner stayed current, so no uncertain commit was recorded: the command was
            // refused before anything reached durable storage. Nothing applies, so a retry of
            // this operation is refused the same way and the player can choose again.
            SubmissionOutcome::Refused(error)
                if self.owner.is_current() && command_refusal_code(error).is_some() =>
            {
                record_session_machine_refusal(self.owner.checkpoint(), &request);
                record_refusal(request.action_kind, &format!("{error:?}"), "refused");
                Ok(wire::committed(wire::rejected(
                    current,
                    operation,
                    command_refusal_code(error)
                        .unwrap_or(rpc::RejectionCode::CapabilityUnavailable),
                )))
            }
            SubmissionOutcome::Refused(error) => {
                if error == RepositoryError::UnresolvedCommit {
                    self.fence();
                }
                if error == RepositoryError::Unauthorized {
                    return Err(unavailable(error));
                }
                record_session_machine_refusal(self.owner.checkpoint(), &request);
                record_refusal(request.action_kind, &format!("{error:?}"), "unavailable");
                Err(authority_unavailable())
            }
        }
    }
    /// Publishes an accepted action as public table talk: the acting hero's name and a short
    /// phrase from the offer's label and visible result, then what the enemy did in answer.
    /// The test fight setting (`DF_GAMEPLAY_TEST_FIGHT`): the opening's first choice is followed
    /// at once by the courier defence, so it leads straight to a tactical fight. The follow-up is
    /// an ordinary submission of the same member through the same checks, one revision after the
    /// choice; a refusal is reported, never hidden.
    fn begin_test_fight(
        &mut self,
        credential: [u8; 32],
        asked: &rpc::SubmitActionRequest,
        result: &Result<rpc::SubmitActionResponse, tonic::Status>,
    ) {
        let first_choice = matches!(
            rpc::GameplayActionKind::try_from(asked.action_kind),
            Ok(rpc::GameplayActionKind::AskCourier | rpc::GameplayActionKind::EscortCourier)
        );
        if !super::journey::tactical::test_fight_enabled() || !first_choice || !accepted_now(result)
        {
            return;
        }
        match self.test_fight_request(&credential, asked) {
            Ok(request) => {
                let kind = request.action_kind;
                let followed = self.submit(credential, request);
                match &followed {
                    Err(status) => record_refusal(kind, "test-fight-follow-up", status.message()),
                    Ok(_) if !accepted_now(&followed) => {
                        record_refusal(kind, "test-fight-follow-up", "not accepted");
                    }
                    Ok(_) => {}
                }
            }
            Err(reason) => record_unavailable("test-fight", reason),
        }
    }

    fn test_fight_request(
        &self,
        credential: &[u8; 32],
        asked: &rpc::SubmitActionRequest,
    ) -> Result<rpc::SubmitActionRequest, &'static str> {
        let principal = self
            .issuer
            .principal(credential)
            .map_err(|_| "member unavailable")?;
        let member =
            df_types::MemberId::from_bytes(&principal).map_err(|_| "member binding invalid")?;
        let checkpoint = self.owner.checkpoint();
        let offer_id = super::beat_script_runtime::view_offers(checkpoint, member)
            .map_err(|_| "offers unavailable")?
            .into_iter()
            .find(|(kind, ..)| *kind == rpc::GameplayActionKind::DefendCourier)
            .map(|(.., id)| id)
            .ok_or("the courier defence is not offered")?;
        let basis = checkpoint.basis();
        let mut hasher = Sha256::new();
        hasher.update(
            asked
                .operation_id
                .as_ref()
                .and_then(|id| id.value.as_deref())
                .unwrap_or_default(),
        );
        hasher.update(b"test-fight");
        let operation: [u8; 32] = hasher.finalize().into();
        Ok(rpc::SubmitActionRequest {
            session_id: Some(rpc::SessionId {
                value: Some(basis.session.as_bytes().to_vec()),
            }),
            run_id: Some(rpc::RunId {
                value: Some(basis.run.as_bytes().to_vec()),
            }),
            operation_id: Some(rpc::OperationId {
                value: Some(operation[..16].to_vec()),
            }),
            observed_revision: Some(wire::revision(basis.revision)),
            offer_id,
            action_kind: rpc::GameplayActionKind::DefendCourier as i32,
            ..Default::default()
        })
    }

    /// Refreshes the newest history entry's actor-side state after a commit's
    /// own table lines land (DEVCTL-001 milestone 2). No-op unless the
    /// development surface is on.
    fn refresh_dev_moment(&self) {
        if let Some(history) = &self.dev_history
            && let Ok(mut history) = history.lock()
        {
            history.refresh_latest_aux(
                self.table_log.position(),
                self.social.snapshot(),
                self.notebook.snapshot(),
            );
        }
    }

    /// Cosmetic work starts only after the authoritative owner confirmed this decision.
    fn record_kill_shots(&self, before: &Checkpoint, outcomes: &[rpc::CombatOutcome]) {
        use super::home_video::kill_shot;
        if outcomes.is_empty() {
            return;
        }
        let after = self.owner.checkpoint();
        let scene = match super::beat_script_runtime::scene(before.state()) {
            Ok(scene) => scene,
            Err(error) => {
                record_unavailable("kill-shot-place", &format!("{error:?}"));
                return;
            }
        };
        // Receipts leave sequence at zero. The TV projection counts every
        // committed combat outcome, including moves and saves, from decisions.
        // Read that same history directly because a finished fight has no live
        // player combat view.
        let committed_history = after
            .state()
            .decisions
            .iter()
            .filter(|decision| decision.source_policy.as_str() == super::journey::THREAD_POLICY)
            .map(super::journey::accepted)
            .collect::<Result<Vec<_>, _>>();
        let Ok(committed_history) = committed_history else {
            record_unavailable("kill-shot-sequence", "committed combat history unavailable");
            return;
        };
        let history = committed_history
            .iter()
            .flat_map(|accepted| accepted.combat.iter().cloned())
            .collect::<Vec<_>>();
        let Some(sequenced_outcomes) = kill_shot::sequence_committed_outcomes(&history, outcomes)
        else {
            record_unavailable("kill-shot-sequence", "committed combat history incomplete");
            return;
        };
        for (index, (outcome, sequenced)) in outcomes.iter().zip(&sequenced_outcomes).enumerate() {
            if !matches!(
                rpc::OutcomeKind::try_from(outcome.kind),
                Ok(rpc::OutcomeKind::Attack | rpc::OutcomeKind::OpportunityAttack)
            ) {
                continue;
            }
            let hero = after
                .state()
                .characters
                .iter()
                .find(|hero| hero.entity.as_bytes().as_slice() == outcome.target_id.as_slice());
            let hero_dead = match hero {
                Some(hero) => {
                    match super::journey::tactical::hero_is_dead(after.state(), hero.entity) {
                        Ok(dead) => Some(kill_shot::hero_death_at_blow(
                            outcome,
                            &outcomes[index + 1..],
                            dead,
                        )),
                        Err(error) => {
                            record_unavailable("kill-shot-hero-health", &format!("{error:?}"));
                            continue;
                        }
                    }
                }
                None => None,
            };
            let blow = kill_shot::committed_blow(sequenced, &scene.title, hero_dead);
            match kill_shot::decide(&blow, after.basis().revision.epoch().get()) {
                Ok(request) => super::home_video::request_kill_shot_still(request, before, outcome),
                Err(reason) => record_audit_line(
                    "gameplay.kill_shot",
                    kill_shot::audit_line("skipped", &blow.killer, &blow.victim, 0, reason.label()),
                ),
            }
        }
    }

    fn record_table_action(
        &self,
        member: df_types::MemberId,
        request: &rpc::SubmitActionRequest,
        accepted: &rpc::AcceptedAction,
        before: &Checkpoint,
        reservation: super::table_log::TableLogReservation,
    ) {
        self.record_kill_shots(before, &accepted.combat);
        self.record_accepted_audit(member, request, accepted, before);
        let Ok(view) = wire::journey_view(self.owner.checkpoint(), LocalDemoRole::Player, member)
        else {
            return;
        };
        let Some(rpc::view_message::Audience::Player(player)) = view.audience else {
            return;
        };
        let Some(journey) = player.journey else {
            return;
        };
        let Some(hero) = journey.own_character.map(|character| character.name) else {
            return;
        };
        let Ok(kind) = rpc::GameplayActionKind::try_from(request.action_kind) else {
            return;
        };
        let Some(index) = journey
            .party
            .iter()
            .position(|party_member| party_member.name == hero)
            .and_then(|position| u32::try_from(position).ok())
        else {
            return;
        };
        match super::journey::living_scene::last_line(self.owner.checkpoint().state()) {
            Ok(Some(line)) => {
                let check = match super::journey::living_scene::last_check_line(
                    self.owner.checkpoint().state(),
                ) {
                    Ok(check) => check,
                    Err(error) => {
                        record_unavailable("scene-interaction-check", &format!("{error:?}"));
                        None
                    }
                };
                self.table_log
                    .push_scene_outcome(reservation, index, check, &line);
                record_audit_line(
                    "gameplay.scene_interaction",
                    format!("SCENE actor={hero:?} result={line:?}"),
                );
                return;
            }
            Err(error) => record_unavailable("scene-interaction-log", &format!("{error:?}")),
            Ok(None) => {}
        }
        if kind == rpc::GameplayActionKind::ExamineHarborSeal
            && let Some(check) = accepted.check.as_ref()
        {
            let Ok(scene) = super::beat_script_runtime::scene(before.state()) else {
                return;
            };
            let observation = super::investigation::observation(&scene, check);
            let Some(table_line) = observation
                .as_ref()
                .map(|observation| observation.roll_line(&hero, check))
                .or_else(|| observation_table_line(&hero, &scene, check, &request.action_choices))
            else {
                record_unavailable("search-audit", "check result cannot be displayed");
                return;
            };
            self.table_log.push_check(
                reservation,
                index,
                super::table_log::ability_check_row(table_line, &hero, check),
            );
            if let Some(finding) = super::beat_script_runtime::observation_finding(&scene, check) {
                let Ok(scene_name) = notebook::scene_name(scene.key) else {
                    record_unavailable("notebook-save", "finding scene invalid");
                    return;
                };
                let text = format!("{} (found by {hero})", finding.sentence);
                match self
                    .notebook
                    .add_clue(member, &hero, &finding.id, &text, &scene_name)
                {
                    Ok(Some(order)) if finding.private => {
                        self.log_notebook_entry(&notebook::EntryView {
                            kind: notebook::Kind::Clue,
                            title: finding.id.clone(),
                            text,
                            scene: scene_name,
                            order,
                            owner: Some(hero.clone()),
                            author: Some(hero.clone()),
                        });
                    }
                    Ok(Some(order)) => match self.notebook.share(member, order) {
                        Ok(Some(entry)) => self.log_notebook_entry(&entry),
                        Ok(None) => {
                            record_unavailable("notebook-share", "new clue was not shareable")
                        }
                        Err(error) => record_unavailable("notebook-share", &error.to_string()),
                    },
                    Ok(None) => {}
                    Err(error) => record_unavailable("notebook-save", &error.to_string()),
                }
                if !finding.private {
                    self.table_log.push(
                        "Narrator",
                        rpc::TableRole::Narrator,
                        0,
                        rpc::TableKind::Action,
                        &finding.sentence,
                    );
                }
            } else if let Some(observation) = observation {
                self.table_log.push(
                    "Narrator",
                    rpc::TableRole::Narrator,
                    0,
                    rpc::TableKind::Action,
                    &observation.public_view().fallback(),
                );
            } else if check.succeeded {
                self.table_log.push(
                    "Narrator",
                    rpc::TableRole::Narrator,
                    0,
                    rpc::TableKind::Action,
                    &format!(
                        "You see {}: {} Nothing is out of place.",
                        scene.title, scene.narration
                    ),
                );
            } else {
                self.table_log.push(
                    "Narrator",
                    rpc::TableRole::Narrator,
                    0,
                    rpc::TableKind::Action,
                    "You find nothing unusual.",
                );
            }
            return;
        }
        let task_status = match kind {
            rpc::GameplayActionKind::EscortCourier => Some("accepted"),
            rpc::GameplayActionKind::ResolveErrand => Some("done"),
            _ => None,
        };
        if let Some(status) = task_status
            && let Err(error) = self.notebook.mark_task_status(status)
        {
            record_unavailable("notebook-save", &error.to_string());
        }
        let weapon = super::pack::hero_weapon_item().replace('-', " ");
        let destination = super::pack::destination_label();
        let label = match super::beat_script_runtime::action_label(&request.offer_id, kind) {
            Ok(label) => label,
            Err(_) => return,
        };
        let after = self.owner.checkpoint();
        let action_lines =
            if kind == rpc::GameplayActionKind::ShortRest && !request.action_choices.is_empty() {
                match rest_hit_die_details(before, &after, member) {
                    Ok(Some(details)) => {
                        vec![super::table_log::ActionLine {
                            roll: None,
                            speaker: hero.clone(),
                            text: short_rest_die_action_text(
                                details.die,
                                details.modifier,
                                details.rolled,
                                details.hit_points,
                                details.maximum,
                            ),
                        }]
                    }
                    Ok(None) => Vec::new(),
                    Err(error) => {
                        record_unavailable("short-rest-table-log", &format!("{error:?}"));
                        Vec::new()
                    }
                }
            } else {
                super::table_log::action_lines(
                    kind,
                    &hero,
                    &accepted.combat,
                    &weapon,
                    &destination,
                    &label,
                )
            };
        // A party member's line is the hero's (a death save is the hero's roll); anyone else
        // who acted is the table's non-player voice.
        let role_of = |speaker: &str| -> (rpc::TableRole, u32) {
            if speaker == "Narrator" {
                return (rpc::TableRole::Narrator, 0);
            }
            match journey
                .party
                .iter()
                .position(|member| member.name == speaker)
                .and_then(|position| u32::try_from(position).ok())
            {
                Some(position) => (rpc::TableRole::Hero, position),
                None => (rpc::TableRole::Npc, 0),
            }
        };
        let action_lines = match &self.session.reaction.expiring {
            Some(pending) if kind == rpc::GameplayActionKind::DeclineReaction => {
                expired_reaction_lines(action_lines, &hero, &pending.provoker_name)
            }
            _ => action_lines,
        };
        let mut reservation = Some(reservation);
        for line in action_lines {
            let (role, hero_index) = role_of(&line.speaker);
            self.table_log
                .push_action_line(reservation.take(), &line, role, hero_index);
        }
        if !super::journey::rewards::paid(before.state())
            && super::journey::rewards::paid(after.state())
            && let Some(reward) = super::pack::with_content(|pack| pack.reward.clone())
        {
            let line = format!(
                "Each hero receives {} GP for the completed errand.",
                reward.gold_per_hero
            );
            self.table_log.push(
                "Narrator",
                rpc::TableRole::Narrator,
                0,
                rpc::TableKind::Action,
                &line,
            );
            record_audit_line(
                "gameplay.reward",
                format!(
                    "REWARD status=paid gold_per_hero={} heroes={}",
                    reward.gold_per_hero,
                    after.state().characters.len()
                ),
            );
            if let Some(favour) = reward.favour {
                if let Err(error) =
                    self.notebook
                        .add_note(member, &hero, &favour, "The completed errand")
                {
                    record_unavailable("reward-favour-note", &error.to_string());
                }
            }
        } else {
            match (
                super::beat_script_runtime::ended(after.state()),
                super::beat_script_runtime::ended(before.state()),
            ) {
                (Ok(true), Ok(false)) => match super::journey::combat_victory(after.state()) {
                    Ok(false) => {
                        self.table_log.push(
                            "Narrator",
                            rpc::TableRole::Narrator,
                            0,
                            rpc::TableKind::Action,
                            "The errand was lost. No reward is paid.",
                        );
                    }
                    Ok(true) => {}
                    Err(error) => record_unavailable("reward-outcome", &format!("{error:?}")),
                },
                (Ok(_), Ok(_)) => {}
                (Err(error), _) | (_, Err(error)) => {
                    record_unavailable("reward-ending", &format!("{error:?}"))
                }
            }
        }
        publish_fight_close(&self.table_log, before.state(), after.state());
    }

    fn record_say_audit(&mut self, credential: &[u8; 32], request: &rpc::RawDialogueRequest) {
        let Ok(member) = self.dialogue_member(credential) else {
            return;
        };
        let Ok(view) = wire::journey_view(self.owner.checkpoint(), LocalDemoRole::Player, member)
        else {
            return;
        };
        let Some(rpc::view_message::Audience::Player(player)) = view.audience else {
            return;
        };
        let Some(journey) = player.journey else {
            return;
        };
        let Some(actor) = journey.own_character.map(|character| character.name) else {
            return;
        };
        record_audit_line(
            "gameplay.story_say",
            super::table_log::story_say_audit_line(
                &actor,
                request.text.chars().count(),
                &journey.speaker,
            ),
        );
    }

    fn record_accepted_audit(
        &self,
        member: df_types::MemberId,
        request: &rpc::SubmitActionRequest,
        accepted: &rpc::AcceptedAction,
        before: &Checkpoint,
    ) {
        let after = self.owner.checkpoint();
        let Ok(kind) = rpc::GameplayActionKind::try_from(request.action_kind) else {
            return;
        };
        let before_phase = super::journey::phase(before).ok();
        let after_phase = super::journey::phase(after).ok();
        let before_combat = audit_combat_view(before, member);
        let after_combat = audit_combat_view(after, member);
        let hero = audit_hero(after, member);
        if let Some(actor) = hero.as_deref() {
            if kind == rpc::GameplayActionKind::ExamineHarborSeal
                && let Some(check) = accepted.check.as_ref()
            {
                let parts = check.source_revision.splitn(4, ':').collect::<Vec<_>>();
                if parts.len() != 4 {
                    record_unavailable("search-audit", "check source invalid");
                    return;
                }
                let Ok(scene) = super::beat_script_runtime::scene(before.state()) else {
                    record_unavailable("search-audit", "check scene invalid");
                    return;
                };
                let finding = super::beat_script_runtime::observation_finding(&scene, check);
                let skill = parts[2];
                let Ok(proficiency) = i32::try_from(check.proficiency_bonus) else {
                    record_unavailable("search-audit", "proficiency overflow");
                    return;
                };
                record_audit_line(
                    "gameplay.search_check",
                    format!(
                        "CHECK actor={actor:?} skill={skill} die={} modifier={} total={} dc={} outcome={} finding={}",
                        check.die,
                        check.ability_modifier + proficiency,
                        check.total,
                        check.difficulty_class,
                        if check.succeeded {
                            "success"
                        } else {
                            "failure"
                        },
                        finding.map_or("none", |finding| finding.id.as_str()),
                    ),
                );
                if let Some(finding) = finding {
                    record_audit_line(
                        "gameplay.finding_revealed",
                        format!("FINDING actor={actor:?} id={}", finding.id),
                    );
                }
            }
            let state_audit = if let (Ok(state_before), Ok(state_after)) = (
                super::journey::session_machine::SessionState::of(before),
                super::journey::session_machine::SessionState::of(after),
            ) {
                super::journey::session_machine::Event::for_kind(state_before, kind)
                    .map(|event| (event, state_before, state_after))
            } else {
                None
            };
            if let Some((event, from_state, to_state)) = &state_audit
                && *event != super::journey::session_machine::Event::StoryChoice
            {
                record_session_state_diagnostic(
                    "gameplay.session_machine_transition",
                    "projected",
                    "projected",
                    None,
                    Some(*from_state),
                    Some(*to_state),
                    *event,
                    kind,
                    request,
                    Some(before.basis().revision),
                    Some(after.basis().revision),
                    after,
                );
            }
            match kind {
                rpc::GameplayActionKind::SecondWind => {
                    if let Some(entity) = after
                        .state()
                        .members
                        .iter()
                        .find(|link| link.member == member)
                        .and_then(|link| link.character)
                        && let (Ok(before_hp), Ok(hit_points), Ok(uses_left)) = (
                            super::journey::value(before.state(), entity, "hit-points"),
                            super::journey::value(after.state(), entity, "hit-points"),
                            super::journey::value(after.state(), entity, "second-wind"),
                        )
                    {
                        record_audit_line(
                            "gameplay.second_wind",
                            super::table_log::second_wind_audit_line(
                                actor,
                                hit_points.saturating_sub(before_hp),
                                hit_points,
                                uses_left,
                            ),
                        );
                    }
                }
                rpc::GameplayActionKind::EndTurn => record_audit_line(
                    "gameplay.turn_end",
                    super::table_log::end_turn_audit_line(actor),
                ),
                rpc::GameplayActionKind::Dash
                | rpc::GameplayActionKind::Dodge
                | rpc::GameplayActionKind::Disengage => record_audit_line(
                    "gameplay.combat_action",
                    super::table_log::combat_action_audit_line(
                        &audit_kind_name(kind).replace('_', "-"),
                        actor,
                    ),
                ),
                rpc::GameplayActionKind::ShortRest if request.action_choices.is_empty() => {
                    record_audit_line(
                        "gameplay.story_rest",
                        super::table_log::story_rest_audit_line(actor),
                    )
                }
                rpc::GameplayActionKind::ShortRest => {
                    match rest_hit_die_details(before, after, member) {
                        Ok(Some(details)) => record_audit_line(
                            "gameplay.story_rest_hit_die",
                            super::table_log::story_rest_hit_die_audit_line(
                                actor,
                                details.die,
                                details.modifier,
                                details.healed,
                                details.hit_points,
                            ),
                        ),
                        Ok(None) => {}
                        Err(error) => {
                            record_unavailable("short-rest-hit-die-audit", &format!("{error:?}"));
                        }
                    }
                }
                rpc::GameplayActionKind::GreatswordAttack
                | rpc::GameplayActionKind::Move
                | rpc::GameplayActionKind::OpportunityAttack
                | rpc::GameplayActionKind::DeclineReaction
                | rpc::GameplayActionKind::Stand
                | rpc::GameplayActionKind::PickUpWeapon
                | rpc::GameplayActionKind::CreateCharacter
                | rpc::GameplayActionKind::Unspecified
                | rpc::GameplayActionKind::SetTableSetting => {}
                _ => {
                    if let (Some(phase_before), Some(phase_after)) = (before_phase, after_phase)
                        && let Ok(label) =
                            super::beat_script_runtime::action_label(&request.offer_id, kind)
                    {
                        record_audit_line(
                            "gameplay.story_choice",
                            super::table_log::story_choice_audit_line(
                                actor,
                                &audit_kind_name(kind),
                                &label,
                                audit_phase_name(phase_before),
                                audit_phase_name(phase_after),
                            ),
                        );
                    }
                }
            }
            if let Some((
                super::journey::session_machine::Event::StoryChoice,
                from_state,
                to_state,
            )) = state_audit
            {
                record_session_state_diagnostic(
                    "gameplay.session_machine_transition",
                    "projected",
                    "projected",
                    None,
                    Some(from_state),
                    Some(to_state),
                    super::journey::session_machine::Event::StoryChoice,
                    kind,
                    request,
                    Some(before.basis().revision),
                    Some(after.basis().revision),
                    after,
                );
            }
        }

        if before_phase != Some(rpc::JourneyPhase::Combat)
            && after_phase == Some(rpc::JourneyPhase::Combat)
            && let Ok(rolls) = super::journey::initiative_rolls(after)
        {
            record_audit_line(
                "gameplay.combat_start",
                super::table_log::initiative_audit_line(&rolls),
            );
        }
        for (event, line) in super::table_log::combat_audit_lines(
            &accepted.combat,
            before_combat.as_ref(),
            after_combat.as_ref(),
        ) {
            record_audit_line(event, line);
        }

        if before_phase == Some(rpc::JourneyPhase::Combat)
            && after_phase == Some(rpc::JourneyPhase::Complete)
        {
            let rounds = before_combat.as_ref().map_or(0, |combat| combat.round);
            let result = after.state().encounters.first().and_then(|encounter| {
                let victory = encounter
                    .objectives
                    .iter()
                    .any(|objective| objective.entry.as_str() == "combat-victory");
                let defeat = encounter
                    .objectives
                    .iter()
                    .any(|objective| objective.entry.as_str() == "combat-defeat");
                fight_end_outcome(victory, defeat)
            });
            if let Some(result) = result {
                record_audit_line(
                    "gameplay.fight_end",
                    super::table_log::fight_end_audit_line(result, rounds),
                );
            } else {
                record_unavailable("fight-end-audit", "terminal encounter outcome absent");
            }
        }
    }
    fn telemetry_identity(
        &self,
        credential: [u8; 32],
        request: rpc::WatchViewRequest,
    ) -> Result<LocalDemoRole, tonic::Status> {
        if self.session_lost() {
            return Err(session_lost_status());
        }
        require_readable(self.fenced)?;
        let role = self.issuer.authenticate(&credential).map_err(unavailable)?;
        let principal = self.issuer.principal(&credential).map_err(unavailable)?;
        watch_scope_identity(self.owner.checkpoint().basis(), role, principal, &request)?;
        Ok(role)
    }
    fn view(
        &mut self,
        credential: [u8; 32],
        request: rpc::WatchViewRequest,
    ) -> Result<(LocalDemoRole, rpc::ViewMessage), tonic::Status> {
        if self.session_lost() {
            return Err(session_lost_status());
        }
        require_readable(self.fenced)?;
        let role = self.issuer.authenticate(&credential).map_err(unavailable)?;
        let principal = self.issuer.principal(&credential).map_err(unavailable)?;
        let current = self.owner.checkpoint();
        let member = watch_scope_identity(current.basis(), role, principal, &request)?;
        if role == LocalDemoRole::Player {
            self.session.reaction.remember(member, credential);
        }
        let scene = super::beat_script_runtime::scene(current.state()).map_err(unavailable)?;
        if let Some(snapshot) =
            super::journey::living_scene::snapshot(current.state(), &scene).map_err(unavailable)?
        {
            self.social
                .sync_room(&format!("{:?}", scene.key), snapshot)
                .map_err(|error| {
                    record_unavailable("scene-social-projection", &error.to_string());
                    tonic::Status::unavailable("scene state unavailable")
                })?;
        }
        super::journey::living_scene::publish(current).map_err(unavailable)?;
        let mut view = wire::journey_view(current, role, member).map_err(unavailable)?;
        self.table_settings.attach(&mut view);
        self.social.attach(&mut view, member);
        if role == LocalDemoRole::Player
            && let Some(rpc::view_message::Audience::Player(player)) = view.audience.as_ref()
            && let (Some(journey), Some(hero)) = (
                player.journey.as_ref(),
                player
                    .journey
                    .as_ref()
                    .and_then(|journey| journey.own_character.as_ref())
                    .map(|character| character.name.as_str()),
            )
        {
            let scene = super::beat_script_runtime::scene(current.state()).map_err(unavailable)?;
            let entries = self
                .notebook
                .observe_scene(
                    member,
                    hero,
                    &scene,
                    &player.private_clue,
                    journey.adventure_ended,
                )
                .map_err(|error| {
                    record_unavailable("notebook-save", &error.to_string());
                    tonic::Status::unavailable("notebook state unavailable")
                })?;
            for entry in entries {
                self.log_notebook_entry(&entry);
            }
        }
        if let Some(reward) = super::pack::with_content(|pack| pack.reward.clone()) {
            self.notebook
                .sync_task_reward(
                    reward.gold_per_hero,
                    super::journey::rewards::paid(current.state()),
                )
                .map_err(|error| {
                    record_unavailable("notebook-save", &error.to_string());
                    tonic::Status::unavailable("notebook state unavailable")
                })?;
        }
        let notebook = if role == LocalDemoRole::Display {
            self.notebook.public_entries()
        } else {
            self.notebook.visible_to(member)
        };
        wire::attach_notebook(&mut view, &notebook)
            .map_err(|_| tonic::Status::internal("notebook view invalid"))?;
        self.table_log.attach(&mut view);
        if let Some(tempo) = self.tempo.as_mut() {
            tempo::observe(tempo.decorate(&mut view, current, Instant::now()));
        }
        self.seat_activity(&credential)?;
        Ok((role, view))
    }

    fn log_notebook_entry(&self, entry: &notebook::EntryView) {
        self.table_log.push(
            "table",
            rpc::TableRole::Narrator,
            0,
            rpc::TableKind::Action,
            &format!(
                "NOTE added kind={} title={:?} scene={}",
                entry.kind.label(),
                entry.title,
                entry.scene
            ),
        );
    }
}
impl Actor {
    fn public_scene_asset(&self) -> Result<String, tonic::Status> {
        if self.session_lost() {
            return Err(session_lost_status());
        }
        require_readable(self.fenced)?;
        wire::public_scene_asset(self.owner.checkpoint()).map_err(unavailable)
    }

    /// Which hero seat the caller holds, for a speech-intent mark.
    fn presence(
        &mut self,
        credential: &[u8; 32],
        kind: rpc::ComposeKind,
    ) -> Result<(), tonic::Status> {
        if self.session_lost() {
            return Err(session_lost_status());
        }
        let member = self.dialogue_member(credential)?;
        let view = wire::journey_view(self.owner.checkpoint(), LocalDemoRole::Player, member)
            .map_err(unavailable)?;
        let Some(rpc::view_message::Audience::Player(player)) = view.audience else {
            return Err(tonic::Status::failed_precondition("player view required"));
        };
        let journey = player
            .journey
            .ok_or_else(|| tonic::Status::failed_precondition("journey required"))?;
        let hero = journey
            .own_character
            .map(|character| character.name)
            .ok_or_else(|| tonic::Status::failed_precondition("hero not created yet"))?;
        let index = journey
            .party
            .iter()
            .position(|party_member| party_member.name == hero)
            .and_then(|position| u32::try_from(position).ok())
            .ok_or_else(|| tonic::Status::failed_precondition("hero seat unknown"))?;
        self.seat_activity(credential)?;
        self.table_log.set_composing(index, kind);
        Ok(())
    }

    /// A presentation report from the shared display. Only the display credential's
    /// reports advance the pacing gate; a player credential's report is ignored (it
    /// learns the current released sequence and nothing changes).
    fn presented(
        &mut self,
        credential: &[u8; 32],
        sequence: u64,
        display_instance: u64,
    ) -> Result<u64, tonic::Status> {
        if self.session_lost() {
            return Err(session_lost_status());
        }
        let role = self.issuer.authenticate(credential).map_err(unavailable)?;
        if role != LocalDemoRole::Display {
            return Ok(self.table_log.released_sequence());
        }
        Ok(self
            .table_log
            .report_presented_at(display_instance, sequence, Instant::now()))
    }
}

impl Reducer<Call> for Actor {
    fn reduce(&mut self, _: AdmissionSequence, call: Call) {
        // One command's refusal detail must never be printed for another: every call
        // this owner reduces starts from a cleared slot (see `refusal_detail`).
        let _cleared = refusal_detail::take();
        // The unauthenticated player shell may learn only the public scene picture name.
        // Reading it is cheap presentation work and must not consume the game call budget.
        let call = match call {
            Call::PublicSceneAsset { reply } => {
                let _delivered = reply.send(self.public_scene_asset());
                return;
            }
            other => other,
        };
        // A speech-intent mark is served first and alone: no budget, heartbeat, grant
        // reconnect or committed-job dispatch, so it can neither fence nor delay the session.
        let call = match call {
            Call::TelemetryIdentity {
                credential,
                request,
                reply,
            } => {
                let _delivered = reply.send(self.telemetry_identity(credential, request));
                return;
            }
            Call::Presence {
                credential,
                kind,
                reply,
            } => {
                let _delivered = reply.send(self.presence(&credential, kind));
                return;
            }
            Call::Presented {
                credential,
                sequence,
                display_instance,
                reply,
            } => {
                let _delivered =
                    reply.send(self.presented(&credential, sequence, display_instance));
                return;
            }
            other => other,
        };
        // Dialogue admission must not dispatch previously committed work.
        // Accepted jobs retain their ordinary non-dialogue dispatch/poll path.
        let dispatch_committed_jobs = !matches!(&call, Call::Dialogue { .. });
        self.heartbeat_if_due();
        if dispatch_committed_jobs {
            self.run_committed_intents();
        }
        if !self.calls_unbounded && self.calls_remaining == 0 {
            match call {
                Call::PublicSceneAsset { reply } => {
                    let _ = reply.send(self.public_scene_asset());
                }
                Call::QualificationInputs { reply } => {
                    let _ = reply.send(Err(tonic::Status::resource_exhausted(
                        "qualification budget exhausted",
                    )));
                }
                Call::AwaySeats { reply, .. } => {
                    let _ = reply.send(Err(tonic::Status::resource_exhausted(
                        "finite room budget exhausted",
                    )));
                }
                Call::Reclaim { reply, .. } => {
                    let _ = reply.send(Err(tonic::Status::resource_exhausted(
                        "finite room budget exhausted",
                    )));
                }
                Call::Join { reply, .. } => {
                    let _ = reply.send(Err(tonic::Status::resource_exhausted(
                        "finite room budget exhausted",
                    )));
                }
                Call::Submit { reply, .. } => {
                    let _ = reply.send(Err(tonic::Status::resource_exhausted(
                        "finite demonstration call budget exhausted",
                    )));
                }
                Call::NpcEvent { reply, .. } => {
                    let _ = reply.send(Err(tonic::Status::resource_exhausted(
                        "finite demonstration call budget exhausted",
                    )));
                }
                Call::PlaceMove { reply, .. } => {
                    let _ = reply.send(Err(tonic::Status::resource_exhausted(
                        "finite demonstration call budget exhausted",
                    )));
                }
                Call::Dialogue { reply, .. } => {
                    let _ = reply.send(Err(tonic::Status::resource_exhausted(
                        "finite dialogue budget exhausted",
                    )));
                }
                Call::View { reply, .. } => {
                    let _ = reply.send(Err(tonic::Status::resource_exhausted(
                        "finite demonstration call budget exhausted",
                    )));
                }
                Call::SpeechMember { reply, .. } => {
                    let _ = reply.send(Err(tonic::Status::resource_exhausted(
                        "finite demonstration call budget exhausted",
                    )));
                }
                Call::Dev { reply, .. } => {
                    let _ = reply.send(Err(tonic::Status::resource_exhausted(
                        "finite demonstration call budget exhausted",
                    )));
                }
                Call::DevDirect { reply, .. } => {
                    let _ = reply.send(Err(tonic::Status::resource_exhausted(
                        "finite demonstration call budget exhausted",
                    )));
                }
                Call::Presence { .. } | Call::Presented { .. } | Call::TelemetryIdentity { .. } => {
                }
            }
            return;
        }
        if !self.calls_unbounded {
            self.calls_remaining -= 1;
        }
        let context = OperationContext {
            trace_parent: String::new(),
            build: crate::BUILD_ID.to_owned(),
        };
        let mut span = df_observe::begin(&context, "gameplay.local_demo_call");
        // Charge the ordinary call first. Reconnection repairs only its owned
        // transport; the original handler still checks current authority and fences.
        let reconnected = match &call {
            Call::QualificationInputs { .. } => Ok(()),
            _ => self.issuer.reconnect_if_closed(),
        };
        let delivered = match call {
            Call::PublicSceneAsset { reply } => reply.send(self.public_scene_asset()).is_ok(),
            Call::QualificationInputs { reply } => reply
                .send(
                    self.qualification_inputs
                        .clone()
                        .map(|actions| QualificationSnapshot {
                            actions,
                            joins: self.qualification_joins.clone().unwrap_or_default(),
                            checkpoint: self.owner.checkpoint().clone(),
                            codec: self.codec,
                            calls_remaining: self.calls_remaining,
                            #[cfg(test)]
                            fenced: self.fenced,
                            #[cfg(test)]
                            uncertain: self.owner.has_uncertain_operation(),
                        })
                        .ok_or_else(|| tonic::Status::permission_denied("qualification disabled")),
                )
                .is_ok(),
            Call::AwaySeats { room_code, reply } => reply
                .send(after_grant_reconnect(reconnected, || {
                    self.away_seats(&room_code)
                }))
                .is_ok(),
            Call::Reclaim { request, reply } => reply
                .send(after_grant_reconnect(reconnected, || {
                    self.reclaim_seat(request)
                }))
                .is_ok(),
            Call::Join { request, reply } => reply
                .send(after_grant_reconnect(reconnected, || self.join(request)))
                .is_ok(),
            Call::Dialogue {
                credential,
                request,
                reply,
            } => {
                let say = match &request {
                    super::dialogue::DialogueRequest::Raw(bytes)
                    | super::dialogue::DialogueRequest::RawTarget { bytes, .. } => {
                        rpc::RawDialogueRequest::decode(bytes.as_slice()).ok()
                    }
                    super::dialogue::DialogueRequest::Confirm(_)
                    | super::dialogue::DialogueRequest::SyncNotes
                    | super::dialogue::DialogueRequest::PendingMerchantQuote
                    | super::dialogue::DialogueRequest::PartyHeroes
                    | super::dialogue::DialogueRequest::CloseMerchantQuote { .. }
                    | super::dialogue::DialogueRequest::Cancel(_) => None,
                };
                let result = after_grant_reconnect(reconnected, || {
                    self.dialogue_member(&credential)?;
                    self.seat_activity(&credential)?;
                    self.handle_dialogue(credential, request)
                });
                if let (Some(request), Ok(super::dialogue::DialogueReply::Raw(_, _))) =
                    (say.as_ref(), &result)
                {
                    self.record_say_audit(&credential, request);
                }
                reply.send(result).is_ok()
            }
            Call::Submit {
                credential,
                request,
                reply,
            } => {
                let asked = request.clone();
                let result =
                    after_grant_reconnect(reconnected, || self.submit(credential, request));
                self.begin_test_fight(credential, &asked, &result);
                reply.send(result).is_ok()
            }
            Call::NpcEvent {
                credential,
                proposal,
                reply,
            } => reply
                .send(after_grant_reconnect(reconnected, || {
                    self.npc_event(&credential, proposal)
                }))
                .is_ok(),
            Call::PlaceMove {
                credential,
                proposal,
                reply,
            } => reply
                .send(after_grant_reconnect(reconnected, || {
                    self.place_move(&credential, proposal)
                }))
                .is_ok(),
            Call::View {
                credential,
                request,
                reply,
            } => reply
                .send(after_grant_reconnect(reconnected, || {
                    self.view(credential, request)
                }))
                .is_ok(),
            Call::SpeechMember { credential, reply } => reply
                .send(after_grant_reconnect(reconnected, || {
                    let member = self.dialogue_member(&credential)?;
                    self.seat_activity(&credential)?;
                    Ok(member)
                }))
                .is_ok(),
            Call::Dev {
                credential,
                order,
                reply,
            } => reply
                .send(after_grant_reconnect(reconnected, || {
                    self.dev(&credential, *order)
                }))
                .is_ok(),
            Call::DevDirect {
                credential,
                request,
                order,
                reply,
            } => reply
                .send(after_grant_reconnect(reconnected, || {
                    let history = self.dev_history.clone().ok_or_else(|| {
                        tonic::Status::permission_denied("development control is off")
                    })?;
                    let (_, view) = self.view(credential, request)?;
                    let mut history = history.lock().map_err(|_| {
                        tonic::Status::unavailable("development direction store unavailable")
                    })?;
                    super::dev_control::direct::admit(&mut history, &view, order)
                }))
                .is_ok(),
            Call::Presence { .. } | Call::Presented { .. } | Call::TelemetryIdentity { .. } => true,
        };
        span.finish_unmeasured(if delivered {
            "actor_outcome_delivered"
        } else {
            "caller_gone_outcome_retained"
        });
        if dispatch_committed_jobs {
            self.run_committed_intents();
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RestHitDieDetails {
    die: u32,
    modifier: i32,
    rolled: u32,
    healed: u32,
    hit_points: u32,
    maximum: u32,
}

fn rest_hit_die_details(
    before: &Checkpoint,
    after: &Checkpoint,
    member: df_types::MemberId,
) -> Result<Option<RestHitDieDetails>, RepositoryError> {
    let Some(who) = after
        .state()
        .members
        .iter()
        .find(|link| link.member == member)
        .and_then(|link| link.character)
    else {
        refusal_detail::note("short-rest-hit-die-member");
        return Err(RepositoryError::InvalidCandidate);
    };
    let before_dice =
        super::journey::value(before.state(), who, "hit-point-dice").map_err(|error| {
            refusal_detail::note("short-rest-hit-die-state");
            error
        })?;
    let after_dice =
        super::journey::value(after.state(), who, "hit-point-dice").map_err(|error| {
            refusal_detail::note("short-rest-hit-die-state");
            error
        })?;
    if !hit_point_die_was_spent(before_dice, after_dice) {
        return Ok(None);
    }
    let die =
        super::journey::value(after.state(), who, "short-rest-die-roll").map_err(|error| {
            refusal_detail::note("short-rest-hit-die-state");
            error
        })?;
    if die == 0 {
        return Ok(None);
    }
    let before_hit_points =
        super::journey::value(before.state(), who, "hit-points").map_err(|error| {
            refusal_detail::note("short-rest-hit-die-state");
            error
        })?;
    let hit_points = super::journey::value(after.state(), who, "hit-points").map_err(|error| {
        refusal_detail::note("short-rest-hit-die-state");
        error
    })?;
    let maximum = super::journey::maximum_hit_points(after.state(), who).map_err(|error| {
        refusal_detail::note("short-rest-hit-die-state");
        error
    })?;
    let constitution = u8::try_from(
        super::journey::value(after.state(), who, "constitution").map_err(|error| {
            refusal_detail::note("short-rest-hit-die-state");
            error
        })?,
    )
    .map_err(|_| {
        refusal_detail::note("short-rest-hit-die-constitution");
        RepositoryError::InvalidCandidate
    })?;
    let modifier = df_rules::local_journey::ability_modifier(constitution);
    let (expected_hit_points, healed) =
        df_rules::local_journey::heal_hit_point_die(die, modifier, before_hit_points, maximum)
            .map_err(|_| {
                refusal_detail::note("short-rest-hit-die-result");
                RepositoryError::InvalidCandidate
            })?;
    if expected_hit_points != hit_points {
        refusal_detail::note("short-rest-hit-die-result");
        return Err(RepositoryError::InvalidCandidate);
    }
    let rolled = u32::try_from((i64::from(die) + i64::from(modifier)).max(1)).map_err(|_| {
        refusal_detail::note("short-rest-hit-die-total");
        RepositoryError::InvalidCandidate
    })?;
    Ok(Some(RestHitDieDetails {
        die,
        modifier,
        rolled,
        healed,
        hit_points,
        maximum,
    }))
}

fn hit_point_die_was_spent(before: u32, after: u32) -> bool {
    after < before
}

fn short_rest_die_action_text(
    die: u32,
    modifier: i32,
    rolled: u32,
    hit_points: u32,
    maximum: u32,
) -> String {
    format!(
        "spends 1 Hit Point Die: {die} {} {} = {rolled}, now {hit_points} of {maximum}",
        if modifier < 0 { "-" } else { "+" },
        modifier.unsigned_abs()
    )
}

fn fight_end_outcome(victory: bool, defeat: bool) -> Option<&'static str> {
    match (victory, defeat) {
        (true, false) => Some("victory"),
        (false, true) => Some("defeat"),
        _ => None,
    }
}

fn heartbeat_wake(
    session_lost: bool,
    heartbeat: &super::session_recovery::Heartbeat,
    now: Instant,
) -> Option<Instant> {
    (!session_lost).then(|| heartbeat.next_due(now))
}

#[cfg(test)]
mod qualification_tests {
    use super::*;

    #[test]
    fn fight_end_audit_uses_the_committed_encounter_outcome() {
        assert_eq!(fight_end_outcome(true, false), Some("victory"));
        assert_eq!(fight_end_outcome(false, true), Some("defeat"));
        assert_eq!(fight_end_outcome(false, false), None);
        assert_eq!(fight_end_outcome(true, true), None);
    }

    #[test]
    fn short_rest_die_log_shows_the_roll_and_result() {
        assert_eq!(
            short_rest_die_action_text(7, 2, 9, 12, 13),
            "spends 1 Hit Point Die: 7 + 2 = 9, now 12 of 13"
        );
    }

    #[test]
    fn keeping_a_hit_point_die_does_not_report_the_previous_roll_again() {
        assert!(!hit_point_die_was_spent(1, 1));
        assert!(hit_point_die_was_spent(2, 1));
    }

    #[test]
    fn lost_session_does_not_schedule_an_expired_heartbeat() {
        let now = Instant::now();
        let mut heartbeat = super::super::session_recovery::Heartbeat::for_lease(30);
        heartbeat.renewed(now);
        let due = heartbeat.next_due(now);

        assert_eq!(heartbeat_wake(true, &heartbeat, due), None);
        assert_eq!(heartbeat_wake(false, &heartbeat, due), Some(due));
    }

    #[test]
    fn only_definitive_refusals_are_answered_as_rejections() {
        // SES-01: a command refused against the current state is answered with
        // a rejection the phone can show; authority failures stay retryable.
        for (error, code) in [
            (
                RepositoryError::InvalidCandidate,
                Some(rpc::RejectionCode::CapabilityUnavailable),
            ),
            (
                RepositoryError::Capacity,
                Some(rpc::RejectionCode::CapabilityUnavailable),
            ),
            (
                RepositoryError::InputBinding,
                Some(rpc::RejectionCode::InvalidSelection),
            ),
            (
                RepositoryError::OperationConflict,
                Some(rpc::RejectionCode::OperationConflict),
            ),
            (RepositoryError::Unavailable, None),
            (RepositoryError::UnresolvedCommit, None),
            (RepositoryError::RevisionConflict, None),
            (RepositoryError::StaleEpoch, None),
            (RepositoryError::StaleFence, None),
            (RepositoryError::ExpiredOwner, None),
            (RepositoryError::InvalidReceipt, None),
            (RepositoryError::SequenceExhausted, None),
            (RepositoryError::RetiredNamespace, None),
            (RepositoryError::Unauthorized, None),
        ] {
            assert_eq!(command_refusal_code(error), code, "{error:?}");
        }
    }

    #[test]
    fn failed_grant_reconnect_never_dispatches_the_original_call() {
        let calls = std::cell::Cell::new(0);
        let result = after_grant_reconnect(Err(RepositoryError::Unavailable), || {
            calls.set(calls.get() + 1);
            Ok(())
        });
        assert_eq!(result.unwrap_err().code(), tonic::Code::Unavailable);
        assert_eq!(calls.get(), 0);
    }

    #[test]
    fn reconnected_call_dispatches_once_and_preserves_current_authority_refusal() {
        let calls = std::cell::Cell::new(0);
        let result: Result<(), _> = after_grant_reconnect(Ok(()), || {
            calls.set(calls.get() + 1);
            Err(tonic::Status::permission_denied("current grant refused"))
        });
        assert_eq!(result.unwrap_err().code(), tonic::Code::PermissionDenied);
        assert_eq!(calls.get(), 1);
        assert_eq!(after_grant_reconnect(Ok(()), || Ok(7)).unwrap(), 7);
    }

    #[test]
    fn completion_retry_has_controlled_spacing_and_terminal_exhaustion() {
        let start = Instant::now();
        let mut retry = CompletionRetry {
            intent: EffectId::from_bytes(&[1; 16]).unwrap(),
            started: start,
            attempts: 0,
            next: None,
        };
        let mut now = start;
        for attempt in 1..COMPLETION_ATTEMPTS {
            assert_eq!(
                retry.failed(now, RepositoryError::Unavailable),
                "completion_retry_scheduled"
            );
            let due = retry.next.unwrap();
            let delay = Duration::from_millis(250 * (1u64 << (attempt - 1).min(3)));
            assert_eq!(due.duration_since(now), delay);
            assert!(now < due);
            now = due;
        }
        assert_eq!(
            retry.failed(now, RepositoryError::UnresolvedCommit),
            "completion_retry_exhausted_retained_pending"
        );
        assert!(retry.next.is_none());
        assert_eq!(retry.attempts, COMPLETION_ATTEMPTS);
    }

    #[test]
    fn permanent_refusals_and_lifetime_expiry_leave_no_idle_wake() {
        let start = Instant::now();
        for error in [
            RepositoryError::Unauthorized,
            RepositoryError::InputBinding,
            RepositoryError::InvalidCandidate,
            RepositoryError::StaleFence,
            RepositoryError::ExpiredOwner,
            RepositoryError::InvalidReceipt,
        ] {
            let mut retry = CompletionRetry {
                intent: EffectId::from_bytes(&[1; 16]).unwrap(),
                started: start,
                attempts: 0,
                next: Some(start),
            };
            assert_eq!(
                retry.failed(start, error),
                "completion_refused_retained_pending"
            );
            assert!(retry.next.is_none());
        }
        let mut retry = CompletionRetry {
            intent: EffectId::from_bytes(&[1; 16]).unwrap(),
            started: start,
            attempts: 0,
            next: None,
        };
        assert_eq!(
            retry.failed(
                start + COMPLETION_RETRY_LIFETIME,
                RepositoryError::Unavailable
            ),
            "completion_retry_exhausted_retained_pending"
        );
        assert!(retry.next.is_none());
    }

    #[test]
    fn input_boundaries_cannot_bypass_spacing_or_restart_terminal_retry() {
        let start = Instant::now();
        let mut retry = CompletionRetry {
            intent: EffectId::from_bytes(&[1; 16]).unwrap(),
            started: start,
            attempts: 0,
            next: Some(start),
        };
        assert!(retry.ready(start));
        retry.failed(start, RepositoryError::Unavailable);
        for offset in [0, 1, 100, 249] {
            assert!(!retry.ready(start + Duration::from_millis(offset)));
        }
        assert!(retry.ready(start + Duration::from_millis(250)));
        retry.failed(
            start + Duration::from_millis(250),
            RepositoryError::Unauthorized,
        );
        assert!(!retry.ready(start + Duration::from_secs(5)));
        assert!(!retry.ready(start + Duration::from_secs(60)));
        assert_eq!(retry.attempts, 2);

        retry.next = Some(start + Duration::from_secs(29));
        assert!(!retry.ready(start + COMPLETION_RETRY_LIFETIME));
        assert!(retry.next.is_none());
    }

    #[test]
    fn retained_inputs_cover_authored_verifier_and_replay_with_a_fixed_bound() {
        let mut inputs = Vec::new();
        let action = |kind| rpc::SubmitActionRequest {
            action_kind: kind as i32,
            ..Default::default()
        };
        for kind in [
            rpc::GameplayActionKind::CreateCharacter,
            rpc::GameplayActionKind::BeginStory,
            rpc::GameplayActionKind::AskCourier,
            rpc::GameplayActionKind::DefendCourier,
            rpc::GameplayActionKind::GreatswordAttack,
        ] {
            retain_qualification_input(&mut inputs, [1; 32], &action(kind));
            retain_qualification_input(&mut inputs, [1; 32], &action(kind));
        }
        retain_qualification_input(
            &mut inputs,
            [2; 32],
            &action(rpc::GameplayActionKind::CreateCharacter),
        );
        retain_qualification_input(
            &mut inputs,
            [2; 32],
            &action(rpc::GameplayActionKind::GreatswordAttack),
        );
        assert_eq!(inputs.len(), 7);
        for kind in [
            rpc::GameplayActionKind::BeginStory,
            rpc::GameplayActionKind::AskCourier,
            rpc::GameplayActionKind::DefendCourier,
            rpc::GameplayActionKind::GreatswordAttack,
        ] {
            assert!(
                inputs
                    .iter()
                    .any(|(_, request)| request.action_kind == kind as i32)
            );
        }
        for kind in [
            rpc::GameplayActionKind::CreateCharacter,
            rpc::GameplayActionKind::GreatswordAttack,
        ] {
            assert_eq!(
                inputs
                    .iter()
                    .filter(|(_, request)| request.action_kind == kind as i32)
                    .count(),
                2
            );
        }
        let retained = inputs.clone();
        retain_qualification_input(
            &mut inputs,
            [3; 32],
            &action(rpc::GameplayActionKind::BeginStory),
        );
        assert_eq!(inputs, retained);
        let mut empty = Vec::new();
        retain_qualification_input(
            &mut empty,
            [1; 32],
            &action(rpc::GameplayActionKind::EndTurn),
        );
        retain_qualification_input(
            &mut empty,
            [1; 32],
            &rpc::SubmitActionRequest {
                action_kind: i32::MAX,
                ..Default::default()
            },
        );
        assert!(empty.is_empty());
    }
}

#[cfg(test)]
mod enemy_turn_audit_tests {
    use super::enemy_turn_line;

    #[test]
    fn enemy_turn_audit_includes_actor_correlation_policy_and_reason() {
        let line = enemy_turn_line(
            "Bandit 1",
            "010203",
            "aabbcc",
            42,
            "SRD-5.2.1:PLAY-T01:1:enemy-policy-opportunity-v2",
            "dodge",
            "no attack target is currently reachable",
            "Dodge",
        );
        assert!(
            line.starts_with(
                "COMBAT turn actor=\"Bandit 1\" operation=010203 run=aabbcc revision=42"
            )
        );
        assert!(line.contains("policy=\"SRD-5.2.1:PLAY-T01:1:enemy-policy-opportunity-v2\""));
        assert!(line.contains("action=dodge reason=\"no attack target is currently reachable\""));
    }
}

#[cfg(test)]
mod telemetry_identity_tests {
    use super::*;

    #[test]
    fn telemetry_scope_accepts_live_roles_and_refuses_wrong_binding_run_and_revision() {
        let current = crate::gameplay::journey::initial().unwrap();
        let basis = current.basis();
        for (role, principal, binding) in [
            (
                LocalDemoRole::Display,
                df_persistence::local_demo_scope::DISPLAY,
                [0x72; 16],
            ),
            (LocalDemoRole::Player, [0x61; 16], [0x61; 16]),
        ] {
            let request = rpc::WatchViewRequest {
                session_id: Some(rpc::SessionId {
                    value: Some(basis.session.as_bytes().to_vec()),
                }),
                run_id: Some(rpc::RunId {
                    value: Some(basis.run.as_bytes().to_vec()),
                }),
                client_binding_id: Some(rpc::ClientBindingId {
                    value: Some(binding.to_vec()),
                }),
                after_revision: Some(wire::revision(basis.revision)),
            };
            assert_eq!(
                watch_scope_identity(basis, role, principal, &request)
                    .unwrap()
                    .as_bytes(),
                &principal
            );
            let mut wrong = request.clone();
            wrong.client_binding_id.as_mut().unwrap().value = Some(vec![0x99; 16]);
            assert_eq!(
                watch_scope_identity(basis, role, principal, &wrong)
                    .unwrap_err()
                    .code(),
                tonic::Code::PermissionDenied
            );
            wrong = request.clone();
            wrong.run_id.as_mut().unwrap().value = Some(vec![0x99; 16]);
            assert_eq!(
                watch_scope_identity(basis, role, principal, &wrong)
                    .unwrap_err()
                    .code(),
                tonic::Code::PermissionDenied
            );
            wrong = request.clone();
            wrong.after_revision = Some(wire::revision(basis.revision.next_sequence().unwrap()));
            assert_eq!(
                watch_scope_identity(basis, role, principal, &wrong)
                    .unwrap_err()
                    .code(),
                tonic::Code::FailedPrecondition
            );
        }
        assert_eq!(basis, current.basis());
    }
}

#[cfg(test)]
mod reaction_timer_tests {
    use super::super::journey::tactical::PendingReaction;
    use super::*;
    use df_model::checkpoint::{Provocation, REACTION_WINDOW_MILLIS, ReactionWindow, RecordId};

    /// The owner's wall clock at the start of each test: controlled, never read.
    const NOW_MS: i64 = 1_760_000_000_000;

    fn pending(reaction: u8, recorded_deadline: Option<i64>) -> PendingReaction {
        let reaction = RecordId::from_bytes(&[reaction; 16]).unwrap();
        PendingReaction {
            reaction,
            member: df_types::MemberId::from_bytes(&[0x11; 16]).unwrap(),
            reactor_name: "Brakka".to_owned(),
            provoker_name: "Bandit 1".to_owned(),
            provocation: Provocation::Move,
            window: recorded_deadline.map(|deadline| ReactionWindow {
                reaction,
                reactor: 1,
                provoker: 3,
                provocation: Provocation::Move,
                opened_unix_ms: deadline - REACTION_WINDOW_MILLIS,
                deadline_unix_ms: deadline,
            }),
        }
    }

    #[test]
    fn a_recorded_window_wakes_and_expires_at_its_deadline_and_not_before() {
        let mut timer = ReactionTimer::default();
        let start = Instant::now();
        let deadline = NOW_MS + REACTION_WINDOW_MILLIS;
        let offer = pending(1, Some(deadline));
        assert_eq!(
            timer.wake_at(Some(&offer), start, NOW_MS),
            Some(start + Duration::from_millis(17_000))
        );
        let late = start + Duration::from_millis(16_999);
        assert_eq!(timer.due(Some(&offer), late, deadline - 1), None);
        let at = start + Duration::from_millis(17_000);
        assert_eq!(timer.due(Some(&offer), at, deadline), Some(deadline));
        // A reload later than the deadline expires at once, against the same deadline.
        assert_eq!(timer.wake_at(Some(&offer), at, deadline + 60_000), Some(at));
    }

    #[test]
    fn an_offer_answered_before_its_deadline_leaves_nothing_to_expire() {
        let mut timer = ReactionTimer::default();
        let start = Instant::now();
        let deadline = NOW_MS + REACTION_WINDOW_MILLIS;
        let offer = pending(1, Some(deadline));
        assert_eq!(timer.due(Some(&offer), start, deadline - 1), None);
        // The hero's answer committed a millisecond before the deadline: nothing is pending.
        assert_eq!(timer.due(None, start, deadline), None);
        assert_eq!(timer.wake_at(None, start, deadline), None);
        // A new offer, even to the same hero, has its own window.
        let next = pending(2, Some(deadline + 5_000));
        assert_eq!(timer.due(Some(&next), start, deadline), None);
    }

    #[test]
    fn an_offer_without_a_recorded_window_gets_seventeen_seconds_from_when_the_owner_loads_it() {
        let mut timer = ReactionTimer::default();
        let start = Instant::now();
        let offer = pending(1, None);
        // Seen for the first time: wake at once to start the window.
        assert_eq!(timer.wake_at(Some(&offer), start, NOW_MS), Some(start));
        assert_eq!(timer.due(Some(&offer), start, NOW_MS), None);
        assert_eq!(
            timer.wake_at(Some(&offer), start, NOW_MS),
            Some(start + Duration::from_millis(17_000))
        );
        let later = start + Duration::from_millis(17_000);
        assert_eq!(timer.due(Some(&offer), later, NOW_MS + 16_999), None);
        assert_eq!(
            timer.due(Some(&offer), later, NOW_MS + 17_000),
            Some(NOW_MS + 17_000)
        );
        // Another offer drops the started window.
        assert_eq!(
            timer.due(Some(&pending(2, None)), later, NOW_MS + 17_000),
            None
        );
        assert_eq!(timer.deadline(&offer), None);
    }

    #[test]
    fn an_expiry_that_could_not_be_sent_waits_before_trying_again() {
        let mut timer = ReactionTimer::default();
        let start = Instant::now();
        let deadline = NOW_MS;
        let offer = pending(1, Some(deadline));
        assert_eq!(timer.due(Some(&offer), start, deadline), Some(deadline));
        timer.retry_later(offer.reaction, start);
        assert_eq!(
            timer.wake_at(Some(&offer), start, deadline),
            Some(start + REACTION_EXPIRY_RETRY)
        );
        assert_eq!(
            timer.due(Some(&offer), start + Duration::from_secs(4), deadline),
            None
        );
        assert_eq!(
            timer.due(Some(&offer), start + REACTION_EXPIRY_RETRY, deadline),
            Some(deadline)
        );
    }

    #[test]
    fn an_expired_decline_reads_that_the_hero_let_the_enemy_pass() {
        use rpc::GameplayActionKind as K;
        let enemy_move = rpc::CombatOutcome {
            actor_name: "Bandit 1".to_owned(),
            kind: rpc::OutcomeKind::Move as i32,
            path: vec![rpc::GridCell { col: 2, row: 2 }],
            cost_ft: 5,
            ..Default::default()
        };
        let lines = super::super::table_log::action_lines(
            K::DeclineReaction,
            "Brakka",
            &[enemy_move],
            "greatsword",
            "the harbor",
            "Decline opportunity attack",
        );
        assert_eq!(lines[0].text, "declines opportunity attack");
        let expired = expired_reaction_lines(lines, "Brakka", "Bandit 1");
        let told = expired
            .iter()
            .map(|line| (line.speaker.as_str(), line.text.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(
            told,
            [
                (
                    "Brakka",
                    "lets Bandit 1 pass without an opportunity attack: no answer within 17 seconds"
                ),
                ("Bandit 1", "moves 1 square (5 ft)"),
            ]
        );
    }
}
