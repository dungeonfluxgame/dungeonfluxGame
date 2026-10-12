//! Raw dialogue remains inert; only a separate native confirmed choice reaches Session.
use std::collections::BTreeMap;
use std::mem::size_of;

use df_intent::dialogue::{DialogueLimits, classify_dialogue, confirm_final_input};
use df_model::intent::{DiscourseContext, InputFinality, IntentDisposition, RawInputRef};
use df_persistence::local_demo_scope::{LocalDemoRole, PLAYER};
use df_protocol::common as rpc;
use df_types::{MemberId, OperationId};
use prost::Message;
use sha2::{Digest, Sha256};
use tokio::sync::oneshot;
use tonic::{Request, Response, Status};

use super::{
    Service, actor, credential, journey, model,
    npc_reply::{NpcReplyContext, StoryFacts},
    wire,
};

const MAXIMUM_MESSAGE_BYTES: usize = 8192;
const MAXIMUM_TEXT_BYTES: usize = 4096;
const MAXIMUM_PENDING_MEMBERS: usize = 32;
const MAXIMUM_PENDING_BYTES: usize = 64 * 1024 * 1024;

pub(super) enum DialogueRequest {
    Raw(Vec<u8>),
    Confirm(Vec<u8>),
    Cancel(Vec<u8>),
}

impl DialogueRequest {
    pub(super) fn retained_heap_bytes(&self) -> usize {
        match self {
            Self::Raw(bytes) | Self::Confirm(bytes) | Self::Cancel(bytes) => bytes.capacity(),
        }
    }
}

pub(super) enum DialogueReply {
    /// The response, plus what a spoken in-character reply needs when the line qualifies.
    Raw(rpc::DialogueResponse, Option<NpcReplyContext>),
    Confirmed(rpc::SubmitActionResponse),
    Cancelled(rpc::DialogueResponse),
}

struct PendingConfirmation {
    input: RawInputRef,
    token: [u8; 32],
    offer: String,
    kind: rpc::GameplayActionKind,
    selection: Option<([u8; 32], OperationId)>,
    charge: usize,
}

/// Owned by the single native Actor. Raw text is never retained here.
#[derive(Default)]
pub(super) struct DialogueState {
    pending: BTreeMap<MemberId, PendingConfirmation>,
}

fn encode<M: Message>(request: &M) -> Result<Vec<u8>, Status> {
    let length = request.encoded_len();
    if length > MAXIMUM_MESSAGE_BYTES {
        return Err(Status::resource_exhausted(
            "dialogue message bound exceeded",
        ));
    }
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(length)
        .map_err(|_| Status::resource_exhausted("dialogue admission capacity unavailable"))?;
    request
        .encode(&mut bytes)
        .map_err(|_| Status::invalid_argument("dialogue encoding invalid"))?;
    Ok(bytes)
}

impl Service {
    async fn dialogue_call(
        &self,
        credential: [u8; 32],
        request: DialogueRequest,
    ) -> Result<DialogueReply, Status> {
        let (reply, wait) = oneshot::channel();
        self.actor
            .try_submit(actor::Call::Dialogue {
                credential,
                request,
                reply,
            })
            .map_err(|_| Status::resource_exhausted("dialogue actor queue full"))?;
        tokio::time::timeout(std::time::Duration::from_secs(5), wait)
            .await
            .map_err(|_| Status::deadline_exceeded("retain the confirmed operation for lookup"))?
            .map_err(|_| Status::unavailable("dialogue actor stopped"))?
    }
}

#[tonic::async_trait]
impl rpc::dialogue_service_server::DialogueService for Service {
    async fn submit(
        &self,
        request: Request<rpc::RawDialogueRequest>,
    ) -> Result<Response<rpc::DialogueResponse>, Status> {
        let credential = credential(&request)?;
        let request = request.into_inner();
        if request.text.len() > MAXIMUM_TEXT_BYTES || request.offer_id.len() > 96 {
            return Err(Status::resource_exhausted(
                "bounded dialogue fields required",
            ));
        }
        match self
            .dialogue_call(credential, DialogueRequest::Raw(encode(&request)?))
            .await?
        {
            DialogueReply::Raw(mut response, npc) => {
                // The model call runs here, off the actor thread; on any failure the
                // response is exactly the classification the actor produced.
                if let Some(context) = npc
                    && let Some(reply) = self
                        .npc_voice
                        .reply_in_public(context, &self.table_log)
                        .await
                {
                    response.npc_reply = reply;
                }
                Ok(Response::new(response))
            }
            _ => Err(Status::internal("dialogue response binding invalid")),
        }
    }

    async fn confirm(
        &self,
        request: Request<rpc::ConfirmDialogueRequest>,
    ) -> Result<Response<rpc::SubmitActionResponse>, Status> {
        let credential = credential(&request)?;
        let request = request.into_inner();
        if request.confirmation_token.len() != 32 {
            return Err(Status::invalid_argument(
                "exact confirmation token required",
            ));
        }
        match self
            .dialogue_call(credential, DialogueRequest::Confirm(encode(&request)?))
            .await?
        {
            DialogueReply::Confirmed(response) => Ok(Response::new(response)),
            _ => Err(Status::internal("confirmation response binding invalid")),
        }
    }

    async fn cancel(
        &self,
        request: Request<rpc::CancelDialogueRequest>,
    ) -> Result<Response<rpc::DialogueResponse>, Status> {
        let credential = credential(&request)?;
        let request = request.into_inner();
        if request.confirmation_token.len() != 32 {
            return Err(Status::invalid_argument(
                "exact confirmation token required",
            ));
        }
        match self
            .dialogue_call(credential, DialogueRequest::Cancel(encode(&request)?))
            .await?
        {
            DialogueReply::Cancelled(response) => Ok(Response::new(response)),
            _ => Err(Status::internal("cancellation response binding invalid")),
        }
    }
}

fn context(value: i32) -> Result<DiscourseContext, Status> {
    Ok(match rpc::DialogueContext::try_from(value) {
        Ok(rpc::DialogueContext::Unspecified) => DiscourseContext::Unspecified,
        Ok(rpc::DialogueContext::Question) => DiscourseContext::Question,
        Ok(rpc::DialogueContext::Social) => DiscourseContext::Social,
        Ok(rpc::DialogueContext::PlanOnly) => DiscourseContext::PlanOnly,
        Ok(rpc::DialogueContext::Meta) => DiscourseContext::Meta,
        Ok(rpc::DialogueContext::Joke) => DiscourseContext::Joke,
        Ok(rpc::DialogueContext::Clarify) => DiscourseContext::Clarify,
        Err(_) => return Err(Status::invalid_argument("unsupported dialogue context")),
    })
}

fn finality(value: i32) -> Result<InputFinality, Status> {
    match rpc::DialogueFinality::try_from(value) {
        Ok(rpc::DialogueFinality::Partial) => Ok(InputFinality::Partial),
        Ok(rpc::DialogueFinality::Final) => Ok(InputFinality::Final),
        _ => Err(Status::invalid_argument("explicit input finality required")),
    }
}

fn disposition(value: IntentDisposition) -> rpc::DialogueDisposition {
    match value {
        IntentDisposition::Action => rpc::DialogueDisposition::Action,
        IntentDisposition::Question => rpc::DialogueDisposition::Question,
        IntentDisposition::Social => rpc::DialogueDisposition::Social,
        IntentDisposition::PlanOnly => rpc::DialogueDisposition::PlanOnly,
        IntentDisposition::Meta => rpc::DialogueDisposition::Meta,
        IntentDisposition::Joke => rpc::DialogueDisposition::Joke,
        IntentDisposition::Clarify => rpc::DialogueDisposition::Clarify,
        IntentDisposition::Rejected => rpc::DialogueDisposition::Rejected,
    }
}

impl actor::Actor {
    fn dialogue_member(&mut self, credential: &[u8; 32]) -> Result<MemberId, Status> {
        if self
            .issuer
            .authenticate(credential)
            .map_err(actor::unavailable)?
            != LocalDemoRole::Player
        {
            return Err(Status::permission_denied(
                "player dialogue binding required",
            ));
        }
        let principal = self
            .issuer
            .principal(credential)
            .map_err(actor::unavailable)?;
        if principal == PLAYER {
            return Err(Status::permission_denied(
                "bootstrap is not a dialogue player",
            ));
        }
        let member = MemberId::from_bytes(&principal)
            .map_err(|_| Status::permission_denied("dialogue member binding invalid"))?;
        if !self
            .owner
            .checkpoint()
            .state()
            .members
            .iter()
            .any(|link| link.member == member)
        {
            return Err(Status::permission_denied(
                "current dialogue membership required",
            ));
        }
        Ok(member)
    }

    pub(super) fn handle_dialogue(
        &mut self,
        credential: [u8; 32],
        request: DialogueRequest,
    ) -> Result<DialogueReply, Status> {
        match request {
            DialogueRequest::Raw(bytes) => {
                let request = rpc::RawDialogueRequest::decode(bytes.as_slice())
                    .map_err(|_| Status::invalid_argument("dialogue input invalid"))?;
                self.raw_dialogue(credential, request)
                    .map(|(response, npc)| DialogueReply::Raw(response, npc))
            }
            DialogueRequest::Confirm(bytes) => {
                let request = rpc::ConfirmDialogueRequest::decode(bytes.as_slice())
                    .map_err(|_| Status::invalid_argument("confirmation input invalid"))?;
                self.confirm_dialogue(credential, request)
                    .map(DialogueReply::Confirmed)
            }
            DialogueRequest::Cancel(bytes) => {
                let request = rpc::CancelDialogueRequest::decode(bytes.as_slice())
                    .map_err(|_| Status::invalid_argument("cancellation input invalid"))?;
                self.cancel_dialogue(credential, request)
                    .map(DialogueReply::Cancelled)
            }
        }
    }

    fn raw_dialogue(
        &mut self,
        credential: [u8; 32],
        request: rpc::RawDialogueRequest,
    ) -> Result<(rpc::DialogueResponse, Option<NpcReplyContext>), Status> {
        let member = self.dialogue_member(&credential)?;
        if self.fenced || self.owner.has_uncertain_operation() {
            return Err(Status::unavailable(
                "resolve the retained operation before confirmation",
            ));
        }
        let invalid = || Status::invalid_argument("dialogue scope invalid");
        let input_id = df_api::operation_id(request.input_id.as_ref()).map_err(|_| invalid())?;
        let session = df_api::session_id(request.session_id.as_ref()).map_err(|_| invalid())?;
        let run = df_api::run_id(request.run_id.as_ref()).map_err(|_| invalid())?;
        let observed =
            df_api::session_revision(request.observed_revision.as_ref()).map_err(|_| invalid())?;
        let current = self.owner.checkpoint();
        let basis = current.basis();
        if session != basis.session || run != basis.run || observed != basis.revision {
            return Err(Status::failed_precondition("dialogue basis changed"));
        }
        let finality = finality(request.finality)?;
        let declared = context(request.context)?;
        let reference = RawInputRef {
            input: input_id,
            member,
            basis,
            pins: current.pins().clone(),
            finality,
        };
        let result = classify_dialogue(
            &request.text,
            declared,
            &reference,
            current,
            member,
            DialogueLimits {
                maximum_text_bytes: MAXIMUM_TEXT_BYTES,
            },
        )
        .map_err(|_| Status::invalid_argument("dialogue input rejected"))?;
        let mut token = Vec::new();
        self.dialogue.pending.remove(&member);
        if result == IntentDisposition::Clarify
            && finality == InputFinality::Final
            && !request.offer_id.is_empty()
        {
            let kind = journey::offered(current, member)
                .map_err(actor::unavailable)?
                .into_iter()
                .map(|(kind, _)| kind)
                .find(|kind| journey::offer_id(current, *kind) == request.offer_id)
                .ok_or_else(|| Status::failed_precondition("current offered choice required"))?;
            let charge = current
                .retained_bytes()
                .and_then(|bytes| bytes.checked_add(size_of::<PendingConfirmation>()))
                .and_then(|bytes| bytes.checked_add(request.offer_id.capacity()))
                .ok_or_else(|| Status::resource_exhausted("confirmation capacity overflow"))?;
            let retained = self
                .dialogue
                .pending
                .values()
                .try_fold(charge, |sum, pending| sum.checked_add(pending.charge));
            if self.dialogue.pending.len() >= MAXIMUM_PENDING_MEMBERS
                || retained.is_none_or(|bytes| bytes > MAXIMUM_PENDING_BYTES)
            {
                return Err(Status::resource_exhausted("confirmation capacity exceeded"));
            }
            let proof = model::random::<32>().map_err(actor::unavailable)?;
            token = proof.to_vec();
            self.dialogue.pending.insert(
                member,
                PendingConfirmation {
                    input: reference,
                    token: proof,
                    offer: request.offer_id,
                    kind,
                    selection: None,
                    charge,
                },
            );
        }
        // Conversation addressed to the scene's speaker is answered in character; a pending
        // confirmation ticket means the player is clarifying an action, which is not.
        let npc = (token.is_empty()
            && finality == InputFinality::Final
            && matches!(
                result,
                IntentDisposition::Question
                    | IntentDisposition::Social
                    | IntentDisposition::Joke
                    | IntentDisposition::Clarify
            ))
        .then(|| self.npc_reply_context(member, &request.text))
        .flatten();
        // Typed words to the scene's speaker are table talk, answered or not.
        if let Some(context) = &npc {
            context.publish_typed_line(&self.table_log);
        }
        Ok((
            rpc::DialogueResponse {
                input_id: request.input_id,
                disposition: disposition(result) as i32,
                confirmation_required: !token.is_empty(),
                confirmation_token: token,
                revision: Some(wire::revision(basis.revision)),
                npc_reply: String::new(),
            },
            npc,
        ))
    }

    /// The facts for the speaker's reply, taken from this member's own view. Failing to build
    /// them only means this line gets today's response without a spoken reply.
    fn npc_reply_context(&self, member: MemberId, typed: &str) -> Option<NpcReplyContext> {
        let view =
            wire::journey_view(self.owner.checkpoint(), LocalDemoRole::Player, member).ok()?;
        let story = super::pack::with_content(StoryFacts::from_pack);
        NpcReplyContext::from_view(&view, member, typed, &story)
    }

    fn confirm_dialogue(
        &mut self,
        credential: [u8; 32],
        request: rpc::ConfirmDialogueRequest,
    ) -> Result<rpc::SubmitActionResponse, Status> {
        let member = self.dialogue_member(&credential)?;
        let input_id = df_api::operation_id(request.input_id.as_ref())
            .map_err(|_| Status::invalid_argument("confirmation input identity required"))?;
        let action = request
            .action
            .ok_or_else(|| Status::invalid_argument("separate typed choice required"))?;
        if action.offer_id.len() > 96
            || action.character.as_ref().is_some_and(|character| {
                character.name.len() > 48
                    || character.choices.len() > 16
                    || character
                        .choices
                        .iter()
                        .any(|choice| choice.group_id.len() > 64 || choice.option_id.len() > 64)
            })
        {
            return Err(Status::invalid_argument("bounded typed choice required"));
        }
        let operation = df_api::operation_id(action.operation_id.as_ref())
            .map_err(|_| Status::invalid_argument("confirmed operation identity required"))?;
        if operation == input_id {
            return Err(Status::invalid_argument(
                "confirmation is a separate user operation",
            ));
        }
        let fingerprint: [u8; 32] = Sha256::digest(action.encode_to_vec()).into();
        let pending = self
            .dialogue
            .pending
            .get_mut(&member)
            .ok_or_else(|| Status::failed_precondition("confirmation expired or cancelled"))?;
        if pending.input.input != input_id
            || request.confirmation_token.as_slice() != pending.token.as_slice()
            || action.offer_id != pending.offer
            || action.action_kind != pending.kind as i32
        {
            return Err(Status::permission_denied("confirmation binding mismatch"));
        }
        if let Some(selection) = pending.selection {
            if selection != (fingerprint, operation) {
                return Err(Status::failed_precondition(
                    "exact confirmed operation retry required",
                ));
            }
            // A previously admitted exact retry reaches native lookup before
            // current offer/basis checks. Session owns uncertainty and receipts.
        } else {
            confirm_final_input(&pending.input, self.owner.checkpoint(), member)
                .map_err(|_| Status::failed_precondition("final input or source basis changed"))?;
            if self.fenced || self.owner.has_uncertain_operation() {
                return Err(Status::unavailable("resolve the retained operation first"));
            }
            let current = self.owner.checkpoint();
            if !journey::offered(current, member)
                .map_err(actor::unavailable)?
                .iter()
                .any(|(kind, _)| *kind == pending.kind)
                || journey::offer_id(current, pending.kind) != pending.offer
            {
                return Err(Status::failed_precondition("confirmed offer changed"));
            }
            let command = wire::journey_input(&action, member, current)?;
            let df_model::checkpoint::GameInput::Game(command) = command else {
                return Err(Status::invalid_argument("explicit typed choice required"));
            };
            if command.basis != pending.input.basis
                || command.observed_revision != pending.input.basis.revision
            {
                return Err(Status::failed_precondition(
                    "confirmed selection basis changed",
                ));
            }
            pending.selection = Some((fingerprint, operation));
        }
        self.submit(credential, action)
    }

    fn cancel_dialogue(
        &mut self,
        credential: [u8; 32],
        request: rpc::CancelDialogueRequest,
    ) -> Result<rpc::DialogueResponse, Status> {
        let member = self.dialogue_member(&credential)?;
        let input_id = df_api::operation_id(request.input_id.as_ref())
            .map_err(|_| Status::invalid_argument("cancellation input identity required"))?;
        let pending = self
            .dialogue
            .pending
            .get(&member)
            .ok_or_else(|| Status::failed_precondition("confirmation expired or cancelled"))?;
        if pending.input.input != input_id
            || request.confirmation_token.as_slice() != pending.token.as_slice()
        {
            return Err(Status::permission_denied("cancellation binding mismatch"));
        }
        if pending.selection.is_some() {
            return Err(Status::failed_precondition(
                "confirmed operation cannot be cancelled; retry exact confirmation",
            ));
        }
        self.dialogue.pending.remove(&member);
        Ok(rpc::DialogueResponse {
            input_id: request.input_id,
            disposition: rpc::DialogueDisposition::Rejected as i32,
            confirmation_token: Vec::new(),
            confirmation_required: false,
            revision: Some(wire::revision(self.owner.checkpoint().basis().revision)),
            npc_reply: String::new(),
        })
    }
}
