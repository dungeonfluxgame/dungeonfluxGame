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
    model, refusal_detail,
    table_settings::{self, TableSettings},
    wire,
};

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
    Dialogue {
        credential: [u8; 32],
        request: super::dialogue::DialogueRequest,
        reply: oneshot::Sender<Result<super::dialogue::DialogueReply, tonic::Status>>,
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
            | Self::Presence { .. }
            | Self::Presented { .. } => Some(0),
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
            Self::View { request, .. } => request
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
    }
    fn validate_recovery(&mut self, checkpoint: &Checkpoint) -> Result<(), RepositoryError> {
        df_session::submission::SessionEngine::<NativeScope<LocalDemoAuthority>>::validate_recovery(
            &mut model::HarborEngine,
            checkpoint,
        )
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
}

impl Default for SessionState {
    fn default() -> Self {
        Self {
            seat_clock: super::room::SeatClock::default(),
            attempt: None,
            commit_millis: 0,
            fence: None,
            heartbeat: super::session_recovery::Heartbeat::for_lease(
                df_persistence::local_demo_scope::lease_seconds(),
            ),
        }
    }
}

pub(super) struct Actor {
    pub session: SessionState,
    pub owner: Owner,
    /// The public table log; an accepted action and a typed line are appended here.
    pub table_log: std::sync::Arc<super::table_log::TableLog>,
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
fn record_unavailable(site: &'static str, reason: &str) {
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

pub(super) fn record_audit_line(event: &'static str, line: String) {
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
        record_audit_line(
            "gameplay.session_machine_refused",
            format!("STATE refused state={state:?} event={event:?} reason={step}"),
        );
    }
    refusal_detail::note(step);
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
        ]
        .into_iter()
        .flatten()
        .min()
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

        self.seat_activity(&credential)?;
        let principal = self.issuer.principal(&credential).map_err(unavailable)?;
        if principal == df_persistence::local_demo_scope::PLAYER {
            return Err(tonic::Status::permission_denied(
                "bootstrap scope is not a player client",
            ));
        }
        let member = df_types::MemberId::from_bytes(&principal)
            .map_err(|_| tonic::Status::permission_denied("member binding invalid"))?;
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
            history.refresh_latest_aux(self.table_log.position(), self.social.snapshot());
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
        let index = journey
            .party
            .iter()
            .position(|party_member| party_member.name == hero)
            .and_then(|position| u32::try_from(position).ok())
            .unwrap_or(0);
        let Ok(kind) = rpc::GameplayActionKind::try_from(request.action_kind) else {
            return;
        };
        let weapon = super::pack::hero_weapon_item().replace('-', " ");
        let destination = super::pack::destination_label();
        let label = match super::beat_script_runtime::action_label(&request.offer_id, kind) {
            Ok(label) => label,
            Err(_) => return,
        };
        let mut lines = super::table_log::action_lines(
            kind,
            &hero,
            &accepted.combat,
            &weapon,
            &destination,
            &label,
        )
        .into_iter();
        if let Some(line) = lines.next() {
            let (role, hero_index) = if line.speaker == hero {
                (rpc::TableRole::Hero, index)
            } else {
                (rpc::TableRole::Npc, 0)
            };
            self.table_log.push_reserved(
                reservation,
                &line.speaker,
                role,
                hero_index,
                rpc::TableKind::Action,
                &line.text,
            );
        }
        for line in lines {
            let (role, hero_index) = if line.speaker == hero {
                (rpc::TableRole::Hero, index)
            } else {
                (rpc::TableRole::Npc, 0)
            };
            self.table_log.push(
                &line.speaker,
                role,
                hero_index,
                rpc::TableKind::Action,
                &line.text,
            );
        }
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
            let state_audit = if let (Ok(state_before), Ok(state_after)) = (
                super::journey::session_machine::SessionState::of(before),
                super::journey::session_machine::SessionState::of(after),
            ) {
                super::journey::session_machine::Event::for_kind(state_before, kind).map(|event| {
                    (
                        event,
                        format!(
                            "STATE {state_before:?} -> {state_after:?} event={event:?} actor={actor:?}"
                        ),
                    )
                })
            } else {
                None
            };
            if let Some((event, line)) = &state_audit
                && *event != super::journey::session_machine::Event::StoryChoice
            {
                record_audit_line("gameplay.session_machine_transition", line.clone());
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
                rpc::GameplayActionKind::ShortRest => record_audit_line(
                    "gameplay.story_rest",
                    super::table_log::story_rest_audit_line(actor),
                ),
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
            if let Some((super::journey::session_machine::Event::StoryChoice, line)) = state_audit {
                record_audit_line("gameplay.session_machine_transition", line);
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
            let hero_ids = after
                .state()
                .characters
                .iter()
                .map(|character| character.entity.as_bytes())
                .collect::<Vec<_>>();
            let defeat = accepted.combat.iter().any(|outcome| {
                let hero_target = hero_ids
                    .iter()
                    .any(|hero_id| *hero_id == outcome.target_id.as_slice());
                let hero_actor = hero_ids
                    .iter()
                    .any(|hero_id| *hero_id == outcome.actor_id.as_slice());
                (outcome.knocked_out && hero_target)
                    || (hero_actor
                        && rpc::DeathSaveResult::try_from(outcome.death_save_result)
                            == Ok(rpc::DeathSaveResult::Died))
            });
            record_audit_line(
                "gameplay.fight_end",
                super::table_log::fight_end_audit_line(
                    if defeat { "defeat" } else { "victory" },
                    rounds,
                ),
            );
        }
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
        let session = df_api::session_id(request.session_id.as_ref())
            .map_err(|_| tonic::Status::invalid_argument("required watch scope invalid"))?;
        let run = df_api::run_id(request.run_id.as_ref())
            .map_err(|_| tonic::Status::invalid_argument("required watch scope invalid"))?;
        let binding = df_api::client_binding_id(request.client_binding_id.as_ref())
            .map_err(|_| tonic::Status::invalid_argument("required watch binding invalid"))?;
        let expected_binding = match role {
            LocalDemoRole::Player => self.issuer.principal(&credential).map_err(unavailable)?,
            LocalDemoRole::Display => [0x72; 16],
        };
        let current = self.owner.checkpoint();
        if session != current.basis().session
            || run != current.basis().run
            || binding.as_bytes() != &expected_binding
        {
            return Err(tonic::Status::permission_denied("local watch scope denied"));
        }
        if let Some(revision) = request.after_revision.as_ref() {
            let observed = df_api::session_revision(Some(revision))
                .map_err(|_| tonic::Status::invalid_argument("watch revision invalid"))?;
            if observed.epoch() != current.basis().revision.epoch()
                || observed > current.basis().revision
            {
                return Err(tonic::Status::failed_precondition(
                    "watch recovery required",
                ));
            }
        }
        let principal = self.issuer.principal(&credential).map_err(unavailable)?;
        if principal == df_persistence::local_demo_scope::PLAYER {
            return Err(tonic::Status::permission_denied(
                "bootstrap scope cannot watch",
            ));
        }
        let member = df_types::MemberId::from_bytes(&principal)
            .map_err(|_| tonic::Status::permission_denied("binding invalid"))?;
        let mut view = wire::journey_view(current, role, member).map_err(unavailable)?;
        self.table_settings.attach(&mut view);
        self.table_log.attach(&mut view);
        self.social.attach(&mut view, member);
        if let Some(tempo) = self.tempo.as_mut() {
            tempo::observe(tempo.decorate(&mut view, current, Instant::now()));
        }
        self.seat_activity(&credential)?;
        Ok((role, view))
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
                Call::Presence { .. } | Call::Presented { .. } => {}
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
                    super::dialogue::DialogueRequest::Raw(bytes) => {
                        rpc::RawDialogueRequest::decode(bytes.as_slice()).ok()
                    }
                    super::dialogue::DialogueRequest::Confirm(_)
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
            Call::Presence { .. } | Call::Presented { .. } => true,
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
