//! Explicit bounded local room admission through generated RPC and the same game actor.
use df_model::checkpoint::{CommandInput, GameCommand, GameInput};
use df_persistence::local_demo_scope::{LocalRejectedLookup, PLAYER};
use df_protocol::common as rpc;
use df_session::inbox::{AdmissionSequence, Reducer};
use df_session::submission::{
    DurableOwner, OwnedInput, PublicationOwner, RepositoryError, SessionEngine, SessionRepository,
    SubmissionOutcome,
};
use prost::Message;
use sha2::{Digest, Sha256};
use tonic::{Request, Response, Status};

use super::{Service, actor, journey, model, wire};
fn refused(code: rpc::RejectionCode) -> rpc::JoinRoomResponse {
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
    code == super::journey::room_code()
}

/// The shell remains shared with qualification. Only its validated ASCII room binding changes.
pub(super) async fn player_page() -> axum::response::Html<String> {
    let axum::response::Html(body) = super::player().await;
    axum::response::Html(body.replace(
        &format!("data-room=\"{}\"", super::journey::ROOM_CODE),
        &format!("data-room=\"{}\"", super::journey::room_code()),
    ))
}

#[cfg(test)]
mod admission_tests;

impl actor::Actor {
    pub(super) fn join(
        &mut self,
        request: rpc::JoinRoomRequest,
    ) -> Result<rpc::JoinRoomResponse, Status> {
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
                    if journey::participants(self.owner.checkpoint()).len() >= 2 {
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
        self.owner.reduce(AdmissionSequence(0), item);
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
        // Membership grant acknowledgement is the final visibility gate. A receipt
        // or reload alone must not reopen watches after an ambiguous Join.
        if recovering {
            self.fenced = false;
            self.recovery_wakeup
                .send_replace(self.owner.checkpoint().clone());
        }
        if let Some(joins) = self.qualification_joins.as_mut()
            && joins.len() < 2
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
            return Err(Status::invalid_argument("bounded join identity required"));
        }
        let (reply, wait) = tokio::sync::oneshot::channel();
        self.actor
            .try_submit(actor::Call::Join {
                request: body,
                reply,
            })
            .map_err(|_| Status::resource_exhausted("room actor queue full"))?;
        let response = tokio::time::timeout(std::time::Duration::from_secs(20), wait)
            .await
            .map_err(|_| Status::deadline_exceeded("retain your original room join proof"))?
            .map_err(|_| Status::unavailable("room owner stopped"))??;
        Ok(Response::new(response))
    }
}
