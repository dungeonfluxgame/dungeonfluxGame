//! Explicit bounded local room admission through generated RPC and the same game actor.
use df_model::checkpoint::{CommandInput, GameCommand, GameInput};
use df_persistence::local_demo_scope::{LocalRejectedLookup, PLAYER};
use df_protocol::common as rpc;
use df_session::inbox::{AdmissionRefusal, AdmissionSequence, Reducer};
use df_session::submission::{
    DurableOwner, OwnedInput, PublicationOwner, RepositoryError, SessionEngine, SessionRepository,
    SubmissionOutcome,
};
use prost::Message;
use sha2::{Digest, Sha256};
use tonic::{Request, Response, Status};

use super::{Service, actor, journey, model, wire};

pub(super) async fn actor_view(
    actor: &super::InboxHandle<actor::Call>,
    credential: [u8; 32],
    request: rpc::WatchViewRequest,
) -> Result<(super::LocalDemoRole, rpc::ViewMessage), Status> {
    let admission_started = std::time::Instant::now();
    let (reply, wait) = tokio::sync::oneshot::channel();
    let reply_deadline = tokio::time::Instant::now() + super::timing::active().reply;
    let mut pending = actor::Call::View {
        credential,
        request,
        reply,
    };
    let wait = loop {
        match actor.try_submit_since(pending, admission_started) {
            Ok(_) => break wait,
            Err(refused)
                if matches!(
                    refused.reason,
                    AdmissionRefusal::ItemCapacity | AdmissionRefusal::ByteCapacity
                ) =>
            {
                pending = refused.input;
                let now = tokio::time::Instant::now();
                if now >= reply_deadline {
                    actor::record_audit_line(
                        "gameplay.view_admission",
                        format!(
                            "VIEW admission_refused reason=queue-full waited_ms={}",
                            admission_started.elapsed().as_millis()
                        ),
                    );
                    return Err(Status::resource_exhausted(
                        "gameplay actor queue remained full",
                    ));
                }
                tokio::time::sleep_until(
                    reply_deadline.min(now + std::time::Duration::from_millis(10)),
                )
                .await;
                if tokio::time::Instant::now() >= reply_deadline {
                    actor::record_audit_line(
                        "gameplay.view_admission",
                        format!(
                            "VIEW admission_refused reason=queue-full waited_ms={}",
                            admission_started.elapsed().as_millis()
                        ),
                    );
                    return Err(Status::resource_exhausted(
                        "gameplay actor queue remained full",
                    ));
                }
            }
            Err(refused) => {
                actor::record_audit_line(
                    "gameplay.view_admission",
                    format!(
                        "VIEW admission_refused reason={:?} waited_ms={}",
                        refused.reason,
                        admission_started.elapsed().as_millis()
                    ),
                );
                return Err(match refused.reason {
                    AdmissionRefusal::Closed => Status::unavailable("gameplay actor stopped"),
                    AdmissionRefusal::ItemCapacity | AdmissionRefusal::ByteCapacity => {
                        Status::resource_exhausted("gameplay actor queue remained full")
                    }
                    AdmissionRefusal::Oversize => {
                        Status::resource_exhausted("gameplay view exceeds actor queue capacity")
                    }
                    AdmissionRefusal::RetainedBytesOverflow => {
                        Status::resource_exhausted("gameplay view admission size overflow")
                    }
                    AdmissionRefusal::SequenceExhausted => {
                        Status::resource_exhausted("gameplay actor sequence exhausted")
                    }
                    AdmissionRefusal::AllocationUnavailable => {
                        Status::resource_exhausted("gameplay actor queue allocation unavailable")
                    }
                    AdmissionRefusal::Poisoned => Status::internal("gameplay actor queue poisoned"),
                });
            }
        }
    };
    tokio::time::timeout_at(reply_deadline, wait)
        .await
        .map_err(|_| Status::deadline_exceeded("gameplay view deadline"))?
        .map_err(|_| Status::unavailable("gameplay actor stopped"))?
}

mod reclaim;
pub(in crate::gameplay) use reclaim::{AwayHero, ReclaimRefusal, ReclaimRequest, SeatClock};
fn rpc_failure(call: &'static str, step: &'static str, status: Status) -> Status {
    actor::record_audit_line(
        "seat.rpc_refused",
        format!(
            "SEAT rpc_refused call={call} code={:?} step={step}",
            status.code()
        ),
    );
    status
}
fn refused(code: rpc::RejectionCode) -> rpc::JoinRoomResponse {
    let step = match code {
        rpc::RejectionCode::InvalidSelection => "room-code",
        rpc::RejectionCode::StaleOffer => "phase",
        rpc::RejectionCode::ResourceMissing => "capacity",
        rpc::RejectionCode::OperationConflict => "conflict",
        rpc::RejectionCode::OperationExpired => "expired",
        rpc::RejectionCode::RecoveryRequired => "recovery",
        _ => "grant",
    };
    actor::record_audit_line(
        "seat.join_refused",
        format!("SEAT join_refused code={code:?} step={step}"),
    );
    rpc::JoinRoomResponse {
        outcome: Some(rpc::join_room_response::Outcome::Refused(
            rpc::JoinRefusal { code: code as i32 },
        )),
    }
}
// Exact uncertain input is lookup-only until DurableOwner validates its retained receipt
// and reload. Known stale state instead requires an authorized reload before admission.
fn prepare_join_recovery<R, E, P>(
    owner: &mut DurableOwner<R, E, P>,
    scope: &R::Scope,
    input: &GameInput,
    context: &df_observe::OperationContext,
) -> Result<(), RepositoryError>
where
    R: SessionRepository,
    E: SessionEngine<R::Scope>,
    P: PublicationOwner<R::Scope>,
{
    if owner.has_uncertain_operation() {
        if !owner.matches_uncertain_retry(scope, input)? {
            return Err(RepositoryError::UnresolvedCommit);
        }
        Ok(())
    } else {
        owner.reload_current(scope, context)
    }
}

// A receipt proves the durable decision, but cannot authorize a grant from a stale cache.
fn admit_join_outcome(
    owner_current: bool,
    outcome: Result<SubmissionOutcome, std::sync::mpsc::TryRecvError>,
    checkpoint: &df_model::checkpoint::Checkpoint,
    fenced: &mut bool,
    recovery_wakeup: &tokio::sync::watch::Sender<df_model::checkpoint::Checkpoint>,
) -> Result<SubmissionOutcome, Status> {
    // Fence and wake watches before interpreting even a missing/confirmed reply.
    if !owner_current {
        actor::fence_checkpoint(fenced, recovery_wakeup, checkpoint);
    }
    let outcome = outcome.map_err(|_| Status::internal("room owner reply missing"))?;
    if matches!(
        &outcome,
        SubmissionOutcome::Refused(df_session::submission::RepositoryError::UnresolvedCommit)
    ) {
        actor::fence_checkpoint(fenced, recovery_wakeup, checkpoint);
    }
    if !owner_current && matches!(&outcome, SubmissionOutcome::Confirmed(_)) {
        return Ok(SubmissionOutcome::LookupRequired);
    }
    Ok(outcome)
}

fn join_lookup_error(
    error: df_session::submission::RepositoryError,
    checkpoint: &df_model::checkpoint::Checkpoint,
    fenced: &mut bool,
    recovery_wakeup: &tokio::sync::watch::Sender<df_model::checkpoint::Checkpoint>,
) -> Status {
    if error == df_session::submission::RepositoryError::UnresolvedCommit {
        actor::fence_checkpoint(fenced, recovery_wakeup, checkpoint);
    }
    actor::unavailable(error)
}

pub(super) fn matches_room_code(code: &str) -> bool {
    super::room_code::matches(&super::journey::room_code(), code)
}

/// The shell remains shared with qualification. Only its validated ASCII room binding and the
/// run's public identity change.
pub(super) async fn player_page(
    actor: super::InboxHandle<super::actor::Call>,
    run_identity: std::sync::Arc<str>,
) -> axum::response::Html<String> {
    let scene_asset = public_scene_asset(&actor).await;
    player_page_with_scene_asset(&run_identity, scene_asset.as_deref()).await
}

async fn public_scene_asset(actor: &super::InboxHandle<super::actor::Call>) -> Option<String> {
    let (reply, wait) = tokio::sync::oneshot::channel();
    actor
        .try_submit(super::actor::Call::PublicSceneAsset { reply })
        .ok()?;
    scene_asset_reply(wait, std::time::Duration::from_millis(250)).await
}

async fn scene_asset_reply(
    wait: tokio::sync::oneshot::Receiver<Result<String, tonic::Status>>,
    timeout: std::time::Duration,
) -> Option<String> {
    tokio::time::timeout(timeout, wait).await.ok()?.ok()?.ok()
}

/// The player shell for the run named `run_identity` (empty when the run publishes none).
#[cfg(test)]
pub(super) async fn player_page_for(run_identity: &str) -> axum::response::Html<String> {
    player_page_with_scene_asset(run_identity, None).await
}

async fn player_page_with_scene_asset(
    run_identity: &str,
    scene_asset: Option<&str>,
) -> axum::response::Html<String> {
    let axum::response::Html(body) = super::player(scene_asset).await;
    let run = if run_identity.is_empty() {
        String::new()
    } else {
        format!(" data-run=\"{run_identity}\"")
    };
    axum::response::Html(body.replace(
        &format!("data-room=\"{}\"", super::journey::ROOM_CODE),
        &format!("data-room=\"{}\"{run}", super::journey::room_code()),
    ))
}

#[cfg(test)]
mod player_page_tests {
    use super::{player_page_with_scene_asset, scene_asset_reply};

    #[tokio::test]
    async fn player_page_uses_the_lobby_scene_asset_when_the_actor_answers() {
        let (reply, wait) = tokio::sync::oneshot::channel();
        reply
            .send(Ok("assets/adventures/rookhill-gate.webp".to_owned()))
            .expect("reply receiver remains open");
        let scene_asset = scene_asset_reply(wait, std::time::Duration::from_secs(1)).await;
        let axum::response::Html(page) =
            player_page_with_scene_asset("run", scene_asset.as_deref()).await;

        assert!(page.contains("data-join-scene-asset=\"assets/adventures/rookhill-gate.webp\""));
    }

    #[tokio::test]
    async fn player_page_uses_the_opening_asset_when_the_actor_does_not_answer() {
        let (_reply, wait) = tokio::sync::oneshot::channel();
        let scene_asset = scene_asset_reply(wait, std::time::Duration::ZERO).await;
        let opening = super::super::pack::scene(super::super::pack::SceneKey::Opening).asset;
        let axum::response::Html(page) =
            player_page_with_scene_asset("run", scene_asset.as_deref()).await;

        assert!(page.contains(&format!("data-join-scene-asset=\"{opening}\"")));
    }
}

#[cfg(test)]
mod admission_tests;
#[cfg(test)]
mod view_retry_tests;

impl actor::Actor {
    pub(super) fn join(
        &mut self,
        request: rpc::JoinRoomRequest,
    ) -> Result<rpc::JoinRoomResponse, Status> {
        let join_started = std::time::Instant::now();
        let op = request
            .operation_id
            .as_ref()
            .and_then(|id| id.value.as_deref())
            .map(crate::client_diagnostics::operation_hash);
        let join_result = self.join_inner(request);
        if let Err(status) = &join_result {
            actor::record_audit_line(
                "seat.join_refused",
                format!(
                    "SEAT join_refused code={:?} step=admission op={} ms={}",
                    status.code(),
                    op.map_or_else(|| "none".to_owned(), |value| value.to_string()),
                    join_started.elapsed().as_millis()
                ),
            );
        }
        join_result
    }
    fn join_inner(
        &mut self,
        request: rpc::JoinRoomRequest,
    ) -> Result<rpc::JoinRoomResponse, Status> {
        if self.session_lost() {
            return Err(actor::session_lost_status());
        }
        if request.room_code.len() > 32
            || request.join_secret.len() != 32
            || request.join_secret.iter().all(|byte| *byte == 0)
        {
            return Err(Status::invalid_argument("bounded join proof required"));
        }
        let operation = df_api::operation_id(request.operation_id.as_ref())
            .map_err(|_| Status::invalid_argument("join operation required"))?;
        let basis = model::basis().map_err(actor::unavailable)?;
        let input = GameInput::Game(CommandInput {
            basis,
            operation,
            member: journey::bootstrap_member().map_err(actor::unavailable)?,
            observed_revision: basis.revision,
            command: GameCommand::ProposeAction {
                actor: journey::room_entity().map_err(actor::unavailable)?,
                action: model::content("join-room").map_err(actor::unavailable)?,
                targets: Vec::new(),
                choices: Vec::new(),
            },
        });
        let fingerprint: [u8; 32] = Sha256::digest(request.encode_to_vec()).into();
        let scope = self
            .issuer
            .issue(
                &self.bootstrap_credential,
                model::random().map_err(actor::unavailable)?,
                fingerprint,
                input.clone(),
                model::pins().map_err(actor::unavailable)?,
            )
            .map_err(actor::unavailable)?;
        let context = df_observe::OperationContext {
            trace_parent: String::new(),
            build: crate::BUILD_ID.to_owned(),
        };
        if !self.fenced {
            self.session.attempt = Some(actor::Attempt::Input {
                credential: self.bootstrap_credential,
                fingerprint,
                input: Box::new(input.clone()),
            });
        }
        let recovering = self.fenced;
        let replaying_uncertain = recovering && self.owner.has_uncertain_operation();
        if recovering {
            match prepare_join_recovery(&mut self.owner, &scope, &input, &context) {
                Ok(()) => {}
                Err(RepositoryError::UnresolvedCommit) => {
                    return Ok(refused(rpc::RejectionCode::RecoveryRequired));
                }
                Err(error) => return Err(actor::unavailable(error)),
            }
        }
        // The exact unresolved key reaches DurableOwner's lookup directly. Fresh
        // room/capacity admission cannot reject a decision already committed.
        if !replaying_uncertain {
            // Lookup always precedes room/capacity/phase admission, and is fingerprint-bound.
            match self
                .issuer
                .rejected_operation(&scope, self.owner.checkpoint().basis(), None, self.codec)
                .map_err(|error| {
                    join_lookup_error(
                        error,
                        self.owner.checkpoint(),
                        &mut self.fenced,
                        &self.recovery_wakeup,
                    )
                })? {
                LocalRejectedLookup::Accepted => {}
                LocalRejectedLookup::NotRecorded => {
                    if !matches_room_code(&request.room_code) {
                        return Ok(refused(rpc::RejectionCode::InvalidSelection));
                    }
                    if journey::phase(self.owner.checkpoint()).map_err(actor::unavailable)?
                        != rpc::JourneyPhase::Room
                    {
                        return Ok(refused(rpc::RejectionCode::StaleOffer));
                    }
                    if journey::participants(self.owner.checkpoint()).len()
                        >= journey::MAXIMUM_PARTY
                    {
                        return Ok(refused(rpc::RejectionCode::ResourceMissing));
                    }
                }
                LocalRejectedLookup::Conflict => {
                    return Ok(refused(rpc::RejectionCode::OperationConflict));
                }
                LocalRejectedLookup::Expired => {
                    return Ok(refused(rpc::RejectionCode::OperationExpired));
                }
                LocalRejectedLookup::Committed(_) => {
                    return Err(Status::internal("unexpected room receipt"));
                }
            }
        }
        let prior = self.owner.checkpoint().basis().revision;
        let (item, receiver) = OwnedInput::new(context, scope, input.clone());
        let commit_started = std::time::Instant::now();
        self.owner.reduce(AdmissionSequence(0), item);
        self.session.commit_millis =
            u64::try_from(commit_started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let receipt = match admit_join_outcome(
            self.owner.is_current(),
            receiver.try_recv(),
            self.owner.checkpoint(),
            &mut self.fenced,
            &self.recovery_wakeup,
        )? {
            SubmissionOutcome::Confirmed(receipt) => receipt,
            SubmissionOutcome::OperationConflict => {
                return Ok(refused(rpc::RejectionCode::OperationConflict));
            }
            SubmissionOutcome::ExpiredOrIndeterminate => {
                return Ok(refused(rpc::RejectionCode::OperationExpired));
            }
            SubmissionOutcome::LookupRequired => {
                self.fence();
                return Ok(refused(rpc::RejectionCode::RecoveryRequired));
            }
            SubmissionOutcome::Refused(error) => return Err(actor::unavailable(error)),
        };
        let (member, entity) =
            journey::decode_join(receipt.decision()).map_err(actor::unavailable)?;
        if member == PLAYER {
            return Err(Status::internal("room member invalid"));
        }
        let proof = self
            .issuer
            .issue(
                &self.bootstrap_credential,
                model::random().map_err(actor::unavailable)?,
                fingerprint,
                input,
                model::pins().map_err(actor::unavailable)?,
            )
            .map_err(actor::unavailable)?;
        let grant = self.issuer.complete_room_join(
            &proof,
            self.owner.checkpoint(),
            member,
            entity,
            model::random().map_err(actor::unavailable)?,
            self.codec,
        );
        let grant = match grant {
            Ok(grant) => grant,
            Err(df_session::submission::RepositoryError::UnresolvedCommit) => {
                self.fence();
                return Ok(refused(rpc::RejectionCode::RecoveryRequired));
            }
            Err(error) => return Err(actor::unavailable(error)),
        };
        let now = self
            .session
            .seat_clock
            .millis(std::time::Instant::now())
            .map_err(actor::unavailable)?;
        self.issuer
            .register_room_seat(member, model::random().map_err(actor::unavailable)?, now)
            .map_err(actor::unavailable)?;
        self.seat_activity(&grant)?;
        // Membership grant acknowledgement is the final visibility gate. A receipt
        // or reload alone must not reopen watches after an ambiguous Join.
        if recovering {
            self.fenced = false;
            self.recovery_wakeup
                .send_replace(self.owner.checkpoint().clone());
        }
        if let Some(joins) = self.qualification_joins.as_mut()
            && joins.len() < journey::MAXIMUM_PARTY
            && !joins
                .iter()
                .any(|(saved, _, _)| saved.operation_id == request.operation_id)
        {
            joins.push((request.clone(), grant, member));
        }
        let committed = wire::receipt(
            receipt.basis(),
            operation,
            rpc::decision_receipt::Outcome::Accepted(Box::new(rpc::AcceptedAction {
                phase: rpc::JourneyPhase::Room as i32,
                ..Default::default()
            })),
            replaying_uncertain || receipt.basis().revision <= prior,
        );
        let revision = receipt.basis().revision;
        actor::record_audit_line(
            "seat.joined",
            format!(
                "SEAT joined seat={} op={} rev={}:{} party={}/{} replayed={} ms={}",
                df_observe::seat_hash(&grant),
                crate::client_diagnostics::operation_hash(operation.as_bytes()),
                revision.epoch().get(),
                revision.sequence(),
                journey::participants(self.owner.checkpoint()).len(),
                journey::MAXIMUM_PARTY,
                replaying_uncertain || receipt.basis().revision <= prior,
                commit_started.elapsed().as_millis(),
            ),
        );
        Ok(rpc::JoinRoomResponse {
            outcome: Some(rpc::join_room_response::Outcome::Joined(Box::new(
                rpc::JoinedRoom {
                    receipt: Some(committed),
                    local_binding: journey::hex(&grant),
                    client_binding_id: Some(rpc::ClientBindingId {
                        value: Some(member.to_vec()),
                    }),
                    session_id: Some(rpc::SessionId {
                        value: Some(receipt.basis().session.as_bytes().to_vec()),
                    }),
                    run_id: Some(rpc::RunId {
                        value: Some(receipt.basis().run.as_bytes().to_vec()),
                    }),
                },
            ))),
        })
    }
}
#[tonic::async_trait]
impl rpc::room_service_server::RoomService for Service {
    async fn current_adventure(
        &self,
        _: Request<rpc::CurrentAdventureRequest>,
    ) -> Result<Response<rpc::CurrentAdventureResponse>, Status> {
        Ok(Response::new(rpc::CurrentAdventureResponse {
            room_code: journey::room_code(),
            run_identity: super::run_restart::identity()
                .map_err(|_| Status::internal("run identity unavailable"))?,
            preparing: journey::new_adventure_requested(&self.updates.borrow()),
        }))
    }

    async fn reclaim(
        &self,
        request: Request<rpc::ReclaimRoomRequest>,
    ) -> Result<Response<rpc::ReclaimRoomResponse>, Status> {
        let body = request.into_inner();
        if body.room_code.len() > 32 || !matches!(body.seat_token.len(), 0 | 32) {
            return Err(rpc_failure(
                "reclaim",
                "shape",
                Status::invalid_argument("bounded reclaim identity required"),
            ));
        }
        let refused = |reason: rpc::SeatReclaimRefusal| {
            actor::record_audit_line(
                "seat.rpc_refused",
                format!("SEAT rpc_refused call=reclaim code={reason:?} step=seat-admission"),
            );
            Response::new(rpc::ReclaimRoomResponse {
                outcome: Some(rpc::reclaim_room_response::Outcome::Refused(reason as i32)),
                away_heroes: Vec::new(),
            })
        };
        if body.seat_token.is_empty() {
            if body.operation_id.is_some() {
                return Err(rpc_failure(
                    "reclaim",
                    "read-operation",
                    Status::invalid_argument("seat read has no operation"),
                ));
            }
            if !matches_room_code(&body.room_code) {
                return Ok(refused(rpc::SeatReclaimRefusal::SeatRoomCodeWrong));
            }
            let (reply, wait) = tokio::sync::oneshot::channel();
            self.actor
                .try_submit(actor::Call::AwaySeats {
                    room_code: body.room_code,
                    reply,
                })
                .map_err(|_| {
                    rpc_failure(
                        "room",
                        "actor-or-proof",
                        Status::resource_exhausted("room actor queue full"),
                    )
                })?;
            let away = tokio::time::timeout(super::timing::active().reply, wait)
                .await
                .map_err(|_| {
                    rpc_failure(
                        "room",
                        "actor-or-proof",
                        Status::deadline_exceeded("away seat read deadline"),
                    )
                })?
                .map_err(|_| {
                    rpc_failure(
                        "room",
                        "actor-or-proof",
                        Status::unavailable("room owner stopped"),
                    )
                })??;
            return Ok(Response::new(rpc::ReclaimRoomResponse {
                outcome: None,
                away_heroes: away
                    .into_iter()
                    .map(|hero| rpc::AwayHero {
                        name: hero.name,
                        seat_token: hero.token.to_vec(),
                    })
                    .collect(),
            }));
        }
        let operation = df_api::operation_id(body.operation_id.as_ref()).map_err(|_| {
            rpc_failure(
                "room",
                "actor-or-proof",
                Status::invalid_argument("reclaim operation required"),
            )
        })?;
        let token = body.seat_token.as_slice().try_into().map_err(|_| {
            rpc_failure(
                "room",
                "actor-or-proof",
                Status::invalid_argument("reclaim token required"),
            )
        })?;
        let (reply, wait) = tokio::sync::oneshot::channel();
        self.actor
            .try_submit(actor::Call::Reclaim {
                request: ReclaimRequest {
                    room_code: body.room_code,
                    operation,
                    token,
                },
                reply,
            })
            .map_err(|_| {
                rpc_failure(
                    "room",
                    "actor-or-proof",
                    Status::resource_exhausted("room actor queue full"),
                )
            })?;
        let answer = tokio::time::timeout(super::timing::active().reply, wait)
            .await
            .map_err(|_| {
                rpc_failure(
                    "room",
                    "actor-or-proof",
                    Status::deadline_exceeded("retain your original reclaim proof"),
                )
            })?
            .map_err(|_| {
                rpc_failure(
                    "room",
                    "actor-or-proof",
                    Status::unavailable("room owner stopped"),
                )
            })??;
        Ok(match answer {
            Ok(joined) => Response::new(rpc::ReclaimRoomResponse {
                outcome: Some(rpc::reclaim_room_response::Outcome::Joined(Box::new(
                    joined,
                ))),
                away_heroes: Vec::new(),
            }),
            Err(reason) => refused(match reason {
                ReclaimRefusal::WrongRoomCode => rpc::SeatReclaimRefusal::SeatRoomCodeWrong,
                ReclaimRefusal::Seat(
                    df_persistence::local_demo_scope::SeatReclaimRefusal::NotAway,
                ) => rpc::SeatReclaimRefusal::SeatNotAway,
                ReclaimRefusal::Seat(
                    df_persistence::local_demo_scope::SeatReclaimRefusal::UnknownToken,
                ) => rpc::SeatReclaimRefusal::SeatTokenUnknown,
                ReclaimRefusal::Seat(
                    df_persistence::local_demo_scope::SeatReclaimRefusal::OperationConflict,
                ) => rpc::SeatReclaimRefusal::SeatOperationConflict,
            }),
        })
    }

    async fn join(
        &self,
        request: Request<rpc::JoinRoomRequest>,
    ) -> Result<Response<rpc::JoinRoomResponse>, Status> {
        let body = request.into_inner();
        if body.room_code.len() > 32
            || body.join_secret.len() != 32
            || body
                .operation_id
                .as_ref()
                .and_then(|id| id.value.as_ref())
                .is_none_or(|bytes| bytes.len() != 16)
        {
            return Err(rpc_failure(
                "join",
                "shape",
                Status::invalid_argument("bounded join identity required"),
            ));
        }
        let (reply, wait) = tokio::sync::oneshot::channel();
        self.actor
            .try_submit(actor::Call::Join {
                request: body,
                reply,
            })
            .map_err(|_| {
                rpc_failure(
                    "room",
                    "actor-or-proof",
                    Status::resource_exhausted("room actor queue full"),
                )
            })?;
        let response = tokio::time::timeout(super::timing::active().reply, wait)
            .await
            .map_err(|_| {
                rpc_failure(
                    "room",
                    "actor-or-proof",
                    Status::deadline_exceeded("retain your original room join proof"),
                )
            })?
            .map_err(|_| {
                rpc_failure(
                    "room",
                    "actor-or-proof",
                    Status::unavailable("room owner stopped"),
                )
            })??;
        Ok(Response::new(response))
    }
}

#[cfg(test)]
mod audit_tests {
    use super::*;
    #[test]
    fn capacity_refusal_keeps_the_response_and_records_its_admission_step() {
        let refusal_response = refused(rpc::RejectionCode::ResourceMissing);
        assert!(
            matches!(refusal_response.outcome, Some(rpc::join_room_response::Outcome::Refused(rpc::JoinRefusal { code })) if code == rpc::RejectionCode::ResourceMissing as i32)
        );
        assert!(
            !actor::audit_lines_containing("SEAT join_refused code=ResourceMissing step=capacity")
                .is_empty()
        );
    }
}
