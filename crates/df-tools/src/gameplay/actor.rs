#[path = "tempo.rs"]
pub(super) mod tempo;

use df_model::checkpoint::{Checkpoint, EffectId, GameInput};
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

use super::{model, wire};

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

pub(super) enum Call {
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
    /// A hero's speech intent (typing or holding to talk). Presentation only: it resolves the
    /// caller's seat and updates the in-memory table log; it never commits, never moves a
    /// revision and is served before any heartbeat or committed-job dispatch.
    Presence {
        credential: [u8; 32],
        kind: rpc::ComposeKind,
        reply: oneshot::Sender<Result<(), tonic::Status>>,
    },
    /// Which current room member a player binding belongs to (speech input).
    SpeechMember {
        credential: [u8; 32],
        reply: oneshot::Sender<Result<df_types::MemberId, tonic::Status>>,
    },
}
impl ActorInput for Call {
    fn retained_heap_bytes(&self) -> Option<usize> {
        fn bytes(value: &Option<Vec<u8>>) -> usize {
            value.as_ref().map_or(0, Vec::capacity)
        }
        match self {
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
            Self::QualificationInputs { .. }
            | Self::SpeechMember { .. }
            | Self::Presence { .. } => Some(0),
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
}
impl Publication {
    pub(super) fn new(updates: watch::Sender<Checkpoint>) -> (Self, std::sync::mpsc::Receiver<()>) {
        let (intents, receiver) = std::sync::mpsc::sync_channel(1);
        (
            Self {
                updates,
                intents,
                campaign: None,
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
                .ok_or(RepositoryError::InvalidCandidate)?;
            let admitted = super::courier_ai::execute(current, intent)
                .map_err(|_| RepositoryError::InvalidCandidate)?;
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
}

impl Default for SessionState {
    fn default() -> Self {
        Self {
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
    #[cfg(test)]
    pub completion_observer: Option<CompletionObserver>,
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
    println!("UNAVAILABLE site={site} reason={reason}");
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
    println!("REFUSAL action={kind} reason={reason} status={status}");
    let context = OperationContext {
        trace_parent: String::new(),
        build: crate::BUILD_ID.to_owned(),
    };
    let mut span = df_observe::begin(&context, "gameplay.action_refused");
    span.event(
        "gameplay.action_refused",
        &[("action.kind", kind), ("refusal.reason", reason.to_owned())],
    );
    span.finish_unmeasured(status);
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
        let heartbeat = Some(self.session.heartbeat.next_due(Instant::now()));
        [completion, recovery, heartbeat]
            .into_iter()
            .flatten()
            .min()
    }

    /// Renews the session lease and grants when the heartbeat is due. Called at every wake and
    /// before every call, so a call after a long pause (a suspended machine) renews first.
    /// Ownership taken over by another process is the one failure that cannot heal.
    fn heartbeat_if_due(&mut self) {
        let now = Instant::now();
        if !self.session.heartbeat.due(now) {
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
        let member = super::courier_ai::recipient(self.owner.checkpoint(), &intent)
            .map_err(|_| RepositoryError::InvalidCandidate)?;
        let completion = super::courier_ai::expected_completion(&intent)
            .map_err(|_| RepositoryError::InvalidCandidate)?;
        let operation = super::courier_ai::completion_operation(&intent)
            .map_err(|_| RepositoryError::InvalidCandidate)?;
        let fingerprint = super::courier_ai::completion_fingerprint(&completion)
            .map_err(|_| RepositoryError::InvalidCandidate)?;
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
        let principal = self.issuer.principal(&credential).map_err(unavailable)?;
        if principal == df_persistence::local_demo_scope::PLAYER {
            return Err(tonic::Status::permission_denied(
                "bootstrap scope is not a player client",
            ));
        }
        let member = df_types::MemberId::from_bytes(&principal)
            .map_err(|_| tonic::Status::permission_denied("member binding invalid"))?;
        let input = wire::journey_input(&request, member, self.owner.checkpoint())?;
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
                        wire::action_offer_rejection(
                            checkpoint,
                            member,
                            kind,
                            &request.offer_id,
                            &request.action_choices,
                        )
                        .map_err(unavailable)?
                    };
                    if let Some(code) = rejection {
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
        let (item, receipt) = OwnedInput::new(context, scope, input);
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
                    self.record_table_action(member, &request, &accepted);
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
            // The owner stayed current, so no uncertain commit was recorded: the candidate was
            // refused before anything reached durable storage. Nothing applies, so a retry of
            // this operation is refused the same way and the player can choose again.
            SubmissionOutcome::Refused(
                error @ (RepositoryError::InvalidCandidate | RepositoryError::Capacity),
            ) if self.owner.is_current() => {
                record_refusal(request.action_kind, &format!("{error:?}"), "refused");
                Ok(wire::committed(wire::rejected(
                    current,
                    operation,
                    rpc::RejectionCode::CapabilityUnavailable,
                )))
            }
            SubmissionOutcome::Refused(error) => {
                if error == RepositoryError::UnresolvedCommit {
                    self.fence();
                }
                if error == RepositoryError::Unauthorized {
                    return Err(unavailable(error));
                }
                record_refusal(request.action_kind, &format!("{error:?}"), "unavailable");
                Err(authority_unavailable())
            }
        }
    }
    /// Publishes an accepted action as public table talk: the acting hero's name and a short
    /// phrase from the offer's label and visible result, then what the enemy did in answer.
    fn record_table_action(
        &self,
        member: df_types::MemberId,
        request: &rpc::SubmitActionRequest,
        accepted: &rpc::AcceptedAction,
    ) {
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
        for line in super::table_log::action_lines(
            kind,
            &hero,
            &accepted.combat,
            &weapon,
            &destination,
            &label,
        ) {
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
        self.table_log.attach(&mut view);
        self.social.attach(&mut view, member);
        if let Some(tempo) = self.tempo.as_mut() {
            tempo::observe(tempo.decorate(&mut view, current, Instant::now()));
        }
        Ok((role, view))
    }
}
impl Actor {
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
        self.table_log.set_composing(index, kind);
        Ok(())
    }
}

impl Reducer<Call> for Actor {
    fn reduce(&mut self, _: AdmissionSequence, call: Call) {
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
                Call::QualificationInputs { reply } => {
                    let _ = reply.send(Err(tonic::Status::resource_exhausted(
                        "qualification budget exhausted",
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
                Call::Presence { .. } => {}
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
            Call::Join { request, reply } => reply
                .send(after_grant_reconnect(reconnected, || self.join(request)))
                .is_ok(),
            Call::Dialogue {
                credential,
                request,
                reply,
            } => reply
                .send(after_grant_reconnect(reconnected, || {
                    self.handle_dialogue(credential, request)
                }))
                .is_ok(),
            Call::Submit {
                credential,
                request,
                reply,
            } => reply
                .send(after_grant_reconnect(reconnected, || {
                    self.submit(credential, request)
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
                    self.dialogue_member(&credential)
                }))
                .is_ok(),
            Call::Presence { .. } => true,
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

#[cfg(test)]
mod qualification_tests {
    use super::*;

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
