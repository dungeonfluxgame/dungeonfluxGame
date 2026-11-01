//! Creator portraits use the mounted durable image service and fresh account authority.
#[path = "creator_portrait_policy.rs"]
mod policy;
use super::{
    imagery::{self, Imagery},
    startup_configuration as config,
};
use axum::{
    Router,
    extract::{Path as RoutePath, State},
    http::{HeaderMap, StatusCode, Uri, header},
    response::{IntoResponse, Response},
    routing::get,
};
use df_imagery::{
    Audience, CreatorClassification, CreatorRoute, Dimensions, ExecutionMode, Freshness,
    GenerateSpec, ImageContentRating, ImageDemandView, ImageExecutionState, ImageOperation,
    ImageOwner, ImagePipeline, ImageServiceError, OperationHandle, OperationIdentity,
    OutputProfile, PrivateImageText, Purpose, PurposedUse, RasterOutputSpec, RequestOutcome,
    TextPrompt, Urgency, VariantRole,
};
use df_protocol::common as rpc;
use df_types::OperationId;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
use tonic::{Request, Status};
pub(super) fn frozen_metadata<T>(uncertain: Option<T>, committed: Option<T>, fresh: T) -> T {
    policy::frozen_metadata(uncertain, committed, fresh)
}
const PREPARING_PORTRAIT: &str = "Preparing your portrait…";
const CREATING_PORTRAIT: &str = "Creating your portrait…";
const READY_PORTRAIT: &str = "Portrait ready";
const MAXIMUM_PORTRAIT_BYTES: u64 = 6 * 1024 * 1024;
const CREATOR_CONTRACT: &str = "creator-content-v3-jev-latest";
#[derive(Clone)]
pub(super) struct Portraits {
    imagery: Option<Imagery>,
    requests: Arc<Mutex<BTreeMap<String, Entry>>>,
}
#[derive(Clone)]
struct Entry {
    diagnostic_operation: u64,
    fingerprint: Vec<u8>,
    revision: u64,
    handle: OperationHandle,
    imagery: Imagery,
    binding: [u8; 32],
    appearance: String,
    sequence: usize,
    grant: [u8; 32],
}
impl Portraits {
    /// The mounted host owns providers, workers, credentials and durable accounting.
    pub(super) fn new(imagery: Option<Imagery>) -> Self {
        Self {
            imagery,
            requests: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }
    #[cfg(test)]
    pub(super) fn disabled() -> Self {
        Self::new(None)
    }
}
impl Entry {
    async fn read(&self) -> Result<ImageDemandView, ImageServiceError> {
        let context = self
            .imagery
            .consumer_context()
            .await
            .map_err(|_| ImageServiceError::Unavailable)?;
        if context.owner != self.handle.owner {
            return Err(ImageServiceError::NotAuthorizedOrNotFound);
        }
        context.service.read(&context.principal, &self.handle).await
    }
    fn key(&self) -> imagery::ImageKey {
        imagery::central::alias_for_operation(self.handle.operation)
    }
    async fn ready_reference(
        &self,
    ) -> Result<
        Option<(
            imagery::ReferenceImage,
            super::image_observability::PublishedObservation,
        )>,
        imagery::ImageFailure,
    > {
        let view = self
            .read()
            .await
            .map_err(|_| imagery::ImageFailure::Refused)?;
        if !ready(&view) {
            return Ok(None);
        }
        let result = view
            .result
            .as_ref()
            .ok_or(imagery::ImageFailure::BadOutput)?;
        let ImageOwner::Draft { account } = self.handle.owner else {
            return Err(imagery::ImageFailure::Refused);
        };
        let reference = self
            .imagery
            .published_reference(result, Audience::OwnerPrivate(account))
            .await?;
        Ok(Some((
            reference,
            super::image_observability::PublishedObservation {
                operation: view.handle.operation,
                job: view.job,
                asset: result.asset,
                config_revision: None,
            },
        )))
    }
}
fn ready(view: &ImageDemandView) -> bool {
    view.execution == ImageExecutionState::Ready
        && view.result.is_some()
        && matches!(
            view.assessment.map(|a| a.route),
            Some(CreatorRoute::General | CreatorRoute::Bfl)
        )
}
#[derive(Clone, Debug)]
pub(super) struct Attachment {
    pub(super) key: String,
    pub(super) digest: [u8; 32],
    pub(super) bytes: u64,
    pub(super) revision: u64,
}
impl Attachment {
    pub(super) fn choices(
        self,
    ) -> Result<
        Vec<(df_types::RevisionLabel, df_types::RevisionLabel)>,
        df_session::submission::RepositoryError,
    > {
        [
            ("key", self.key),
            ("digest", super::journey::hex(&self.digest)),
            ("bytes", self.bytes.to_string()),
            ("revision", self.revision.to_string()),
        ]
        .into_iter()
        .map(|(field, value)| {
            Ok((
                super::model::label(&format!("identity-portrait:{field}"))?,
                super::model::label(&value)?,
            ))
        })
        .collect()
    }
}
impl Portraits {
    pub(super) async fn accepted(
        &self,
        binding: [u8; 32],
        character: Option<&rpc::CharacterSelection>,
    ) -> Result<Option<Attachment>, Status> {
        let Some(character) = character else {
            return Ok(None);
        };
        let Some(identity) = character.identity.clone() else {
            return Ok(None);
        };
        let identity = super::hero_identity::Identity::from_rpc(identity).map_err(|error| {
            super::actor::record_ingress_refusal(
                rpc::GameplayActionKind::CreateCharacter as i32,
                "creator-portrait-attachment-identity",
                &format!("{error:?}"),
                tonic::Code::InvalidArgument,
            );
            Status::invalid_argument("Review your public appearance")
        })?;
        let species = character
            .choices
            .iter()
            .find(|choice| choice.group_id == "species")
            .map(|choice| choice.option_id.as_str())
            .unwrap_or("");
        let selected = |group: &str| {
            character
                .choices
                .iter()
                .find(|choice| choice.group_id == group)
                .map(|choice| choice.option_id.as_str())
                .unwrap_or("")
        };
        let (public, _) = super::creator_catalog::portrait_context(
            &identity,
            species,
            selected("class"),
            selected("background"),
        )
        .map_err(|error| {
            super::actor::record_ingress_refusal(
                rpc::GameplayActionKind::CreateCharacter as i32,
                "creator-portrait-attachment-design",
                &format!("{error:?}"),
                tonic::Code::InvalidArgument,
            );
            Status::invalid_argument("Review your character design")
        })?;
        let (public, _) = super::creator_catalog::with_scenery(
            public,
            String::new(),
            &character.portrait_scenery,
        )
        .map_err(|error| {
            super::actor::record_ingress_refusal(
                rpc::GameplayActionKind::CreateCharacter as i32,
                "creator-portrait-attachment-scenery",
                &format!("{error:?}"),
                tonic::Code::InvalidArgument,
            );
            Status::invalid_argument("Review your public portrait scenery")
        })?;

        let mut candidates = self
            .requests
            .lock()
            .map_err(|_| Status::unavailable("Portrait status unavailable"))?
            .values()
            .filter(|entry| entry.binding == binding && entry.appearance == public)
            .cloned()
            .collect::<Vec<_>>();
        candidates.sort_unstable_by_key(|entry| std::cmp::Reverse(entry.sequence));
        for entry in candidates {
            let Ok(Some((reference, _))) = entry.ready_reference().await else {
                continue;
            };
            let bytes = reference.bytes.len() as u64;
            if bytes == 0 || bytes > MAXIMUM_PORTRAIT_BYTES {
                continue;
            }
            super::actor::record_audit_line(
                "gameplay.creator_portrait_attach",
                format!(
                    "CREATOR_PORTRAIT event=attach outcome=ready reason=attached operation_hash={} bytes={bytes} seat={}",
                    entry.diagnostic_operation,
                    df_observe::seat_hash(&binding)
                ),
            );
            return Ok(Some(Attachment {
                key: entry.key().as_str().to_owned(),
                digest: Sha256::digest(&reference.bytes).into(),
                bytes,
                revision: entry.revision,
            }));
        }
        super::actor::record_audit_line(
            "gameplay.creator_portrait_attach",
            format!(
                "CREATOR_PORTRAIT event=attach outcome=none reason=not_ready operation_hash=none bytes=0 seat={}",
                df_observe::seat_hash(&binding)
            ),
        );
        Ok(None)
    }
}
/// Current host/catalog authority precedes the exact committed reference pin comparison.
pub(super) async fn approved_reference(
    imagery: &Imagery,
    asset: &df_model::checkpoint::AssetReference,
) -> Result<imagery::ReferenceImage, imagery::ImageFailure> {
    if asset.kind != df_model::checkpoint::AssetKind::Image
        || asset.byte_length == 0
        || asset.byte_length > MAXIMUM_PORTRAIT_BYTES
    {
        return Err(imagery::ImageFailure::InvalidRequest);
    }
    let key = imagery::ImageKey::parse(asset.key.as_str())
        .ok_or(imagery::ImageFailure::InvalidRequest)?;
    let reference = imagery.reference(&key).await?;
    if reference.bytes.len() as u64 != asset.byte_length
        || <[u8; 32]>::from(Sha256::digest(&reference.bytes)) != asset.digest.0
    {
        return Err(imagery::ImageFailure::BadOutput);
    }
    Ok(reference)
}
struct PreparedPortrait {
    identity: super::hero_identity::ValidatedIdentity,
    public: String,
    design: String,
    fingerprint: Vec<u8>,
}

fn prepare_portrait(body: &rpc::HeroPortraitRequest) -> Result<PreparedPortrait, Status> {
    let identity = super::hero_identity::Identity::from_rpc(
        body.identity
            .clone()
            .ok_or_else(|| Status::invalid_argument("Public appearance is required"))?,
    )
    .map_err(|_| Status::invalid_argument("Review your public appearance details"))?;
    let (public, design) = super::creator_catalog::portrait_context(
        &identity,
        &body.species,
        &body.character_class,
        &body.background,
    )
    .map_err(|_| {
        Status::invalid_argument("Choose an ancestry, class and background from the core catalog")
    })?;
    let (public, design) = super::creator_catalog::with_scenery(public, design, &body.scenery)
        .map_err(|_| {
            Status::invalid_argument(
                "Scenery needs up to 360 characters / 768 bytes of public setting details",
            )
        })?;
    // A browser revision is presentation bookkeeping, not a different paid design.
    let fingerprint = Sha256::digest(public.as_bytes()).to_vec();
    Ok(PreparedPortrait {
        identity,
        public,
        design,
        fingerprint,
    })
}

fn portrait_message_key(message: &str) -> &'static str {
    match message {
        PREPARING_PORTRAIT | "preparing" => "preparing",
        CREATING_PORTRAIT | "creating" => "creating",
        READY_PORTRAIT | "ready" => "ready",
        "classified" => "classified",
        "image-bytes" => "image-bytes",
        _ => {
            for label in [
                "classifier_unavailable",
                "alternate_unavailable",
                "rating_disallows_appearance",
                "classification_uncertain",
                "content_requires_revision",
                "bfl_submit_failed",
                "bfl_poll_failed",
                "bfl_delivery_failed",
                "bfl_download_failed",
                "refused",
                "deadline",
                "interrupted",
            ] {
                if message == label || message == failure_message(label) {
                    return label;
                }
            }
            "unknown-result"
        }
    }
}

fn portrait_event_scoped(
    audit: &df_observe::AuditContext,
    operation_hash: &str,
    event: &'static str,
    outcome: &str,
    revision: u64,
    bytes: usize,
    message: &str,
) {
    let _scope = df_observe::audit_scope(audit.clone());
    portrait_event(operation_hash, event, outcome, revision, bytes, message);
}

fn portrait_event(
    operation_hash: &str,
    event: &'static str,
    outcome: &str,
    revision: u64,
    bytes: usize,
    message: &str,
) {
    let message_key = portrait_message_key(message);
    let message_hash = format!("{:x}", Sha256::digest(message.as_bytes()));
    super::actor::record_audit_line(
        "gameplay.creator_portrait",
        format!(
            "CREATOR_PORTRAIT event={event} operation_hash={operation_hash} outcome={outcome} draft_revision={revision} bytes={bytes} message_key={message_key} message_hash={message_hash}"
        ),
    );
}

pub(super) async fn handle(
    service: &super::Service,
    request: Request<rpc::HeroPortraitRequest>,
    dispatch: bool,
) -> Result<rpc::HeroPortraitResponse, Status> {
    let audit = super::service_audit_context(service, super::credential(&request).ok().as_ref());
    let operation_hash = request
        .get_ref()
        .operation_id
        .as_ref()
        .and_then(|id| id.value.as_deref())
        .map(crate::client_diagnostics::operation_hash)
        .map_or_else(|| "unbound".to_owned(), |value| value.to_string());
    let revision = request.get_ref().draft_revision;
    if dispatch {
        let _scope = df_observe::audit_scope(audit.clone());
        let client_operation_hash = crate::client_diagnostics::operation_hash(
            request
                .get_ref()
                .operation_id
                .as_ref()
                .and_then(|id| id.value.as_deref())
                .unwrap_or_default(),
        );
        super::actor::record_audit_line(
            "gameplay.creator_portrait_binding",
            format!(
                "CREATOR_PORTRAIT event=operation-binding operation_hash={operation_hash} client_operation_hash={client_operation_hash} draft_revision={revision}"
            ),
        );
    }
    let result = handle_inner(service, request, dispatch).await;
    match &result {
        Ok(response) => portrait_event_scoped(
            &audit,
            &operation_hash,
            if dispatch {
                "request-answer"
            } else {
                "poll-answer"
            },
            &response.state,
            revision,
            0,
            &response.message,
        ),
        Err(status) => portrait_event_scoped(
            &audit,
            &operation_hash,
            if dispatch {
                "request-answer"
            } else {
                "poll-answer"
            },
            if status.code() == tonic::Code::NotFound {
                "unknown"
            } else {
                "failed"
            },
            revision,
            0,
            &format!("rpc-{:?}", status.code()),
        ),
    }
    result
}

fn operation_id(
    owner: &ImageOwner,
    operation: &[u8],
    revision: u64,
) -> Result<OperationId, Status> {
    let mut digest = Sha256::new();
    digest.update(b"df-gameplay/creator-portrait/2");
    digest.update(df_imagery::owner_scope_bytes(owner));
    digest.update(operation);
    digest.update(revision.to_be_bytes());
    OperationId::from_bytes(&digest.finalize()[..16])
        .map_err(|_| Status::invalid_argument("Portrait operation is invalid"))
}
fn creator_account_status(error: imagery::ImageFailure) -> Status {
    match error {
        imagery::ImageFailure::AuthorityCapacity => {
            Status::unavailable("Portrait service is at capacity. Try again.")
        }
        _ => Status::permission_denied("Creator account unavailable"),
    }
}
fn service_status(error: ImageServiceError) -> Status {
    match error {
        ImageServiceError::NotAuthorizedOrNotFound => {
            Status::not_found("Portrait request is unavailable")
        }
        ImageServiceError::Conflict => {
            Status::already_exists("This operation belongs to a different appearance")
        }
        ImageServiceError::InvalidRequest(_) => {
            Status::invalid_argument("Review your public portrait description")
        }
        ImageServiceError::InvalidRecord
        | ImageServiceError::Unavailable
        | ImageServiceError::Unknown { .. } => Status::unavailable("Portrait status unavailable"),
    }
}
fn service_request(
    context: &imagery::central::ConsumerContext,
    body: &rpc::HeroPortraitRequest,
    prepared: &PreparedPortrait,
    operation: OperationId,
    operation_key: &str,
    regenerate: bool,
) -> Result<df_imagery::ImageRequest, Status> {
    let ImageOwner::Draft { account } = context.owner else {
        return Err(Status::permission_denied("Creator account unavailable"));
    };
    let route = context
        .config
        .analysis
        .classification
        .clone()
        .ok_or_else(|| {
            Status::failed_precondition(
                "Portrait generation is unavailable. Ask the host to check setup",
            )
        })?;
    let binding = format!("{CREATOR_CONTRACT}|{operation_key}");
    let assembled = |rating| {
        imagery::creator_portrait(
            rating,
            imagery::HeroVisualIdentity {
                identity: &prepared.identity,
                species: &body.species,
                held_gear: &[],
            },
            &binding,
            &prepared.design,
        )
        .map_err(|_| Status::invalid_argument("Portrait description is too long"))
    };
    let rating = config::content_rating();
    let general = assembled(config::ContentRating::Restrained)?;
    let rated = assembled(rating)?;
    let prompt = TextPrompt::new(
        rated.as_str(),
        imagery::ImageKind::CanonicalCharacter.template_revision(),
    )
    .and_then(|prompt| {
        prompt.with_pipeline(ImagePipeline {
            revision: 1,
            moderation: context
                .config
                .purpose(Purpose::CreatorPortrait)
                .moderation_override,
            rating: if rating == config::ContentRating::Mature {
                ImageContentRating::Mature
            } else {
                ImageContentRating::Restrained
            },
            creator: Some(CreatorClassification {
                route,
                contract_revision: 3,
                public_appearance: PrivateImageText::new(&prepared.public),
                general_prompt: Some(PrivateImageText::new(general.as_str())),
            }),
            inspection: None,
        })
    })
    .map_err(|_| Status::invalid_argument("Portrait description is too long"))?;
    let dimensions = Dimensions::new(1024, 1536)
        .map_err(|_| Status::unavailable("Portrait output unavailable"))?;
    let delivered = RasterOutputSpec::new(
        dimensions,
        df_imagery::AlphaHandling::Opaque,
        df_imagery::Encoding::Jpeg {
            quality: context.config.quality.jpeg_quality,
        },
    )
    .map_err(|_| Status::unavailable("Portrait output unavailable"))?;
    let mut request = df_imagery::ImageRequest {
        identity: OperationIdentity {
            owner: context.owner.clone(),
            operation,
            fingerprint: [0; 32],
        },
        purpose: PurposedUse {
            purpose: Purpose::CreatorPortrait,
            role: VariantRole::Primary,
        },
        operation: ImageOperation::Generate(GenerateSpec {
            prompt,
            references: Vec::new(),
        }),
        execution: ExecutionMode::Live,
        freshness: if regenerate {
            Freshness::Regenerate { nonce: operation }
        } else {
            Freshness::ReuseCompatible
        },
        urgency: Urgency::Visible,
        audience: Audience::OwnerPrivate(account),
        expected_source_revision: body.draft_revision,
        output: OutputProfile::provider(dimensions, delivered),
    };
    request.identity.fingerprint = context
        .service
        .canonical_fingerprint(&request)
        .map_err(service_status)?;
    Ok(request)
}
fn presentation(view: &ImageDemandView) -> (&'static str, String) {
    use ImageExecutionState as State;
    if ready(view) {
        return ("ready", READY_PORTRAIT.to_owned());
    }
    match view.execution {
        State::NeedsRevision => (
            "failed",
            failure_message(
                if view
                    .assessment
                    .is_some_and(|a| a.category == df_imagery::CreatorCategory::Unsupported)
                {
                    "content_requires_revision"
                } else {
                    "rating_disallows_appearance"
                },
            )
            .to_owned(),
        ),
        State::Uncertain => (
            "unknown",
            failure_message("classification_uncertain").to_owned(),
        ),
        State::Unknown | State::Ready | State::Completed => {
            ("unknown", failure_message("interrupted").to_owned())
        }
        State::Expired => ("unknown", failure_message("deadline").to_owned()),
        State::Failed { .. } | State::Withdrawn => {
            ("failed", failure_message("refused").to_owned())
        }
        State::Planning | State::Queued | State::Running | State::Publishing => (
            "pending",
            if view.assessment.is_some() {
                CREATING_PORTRAIT
            } else {
                PREPARING_PORTRAIT
            }
            .to_owned(),
        ),
    }
}
async fn handle_inner(
    service: &super::Service,
    request: Request<rpc::HeroPortraitRequest>,
    dispatch: bool,
) -> Result<rpc::HeroPortraitResponse, Status> {
    let audit = super::service_audit_context(service, super::credential(&request).ok().as_ref());
    let binding = super::credential(&request)?;
    // Regeneration is an explicit action; ordinary Generate/Confirm reuses a design.
    let regenerate = request
        .metadata()
        .get("x-df-portrait-regenerate")
        .is_some_and(|value| value == "true");
    let body = request.into_inner();
    let expected = body
        .observed_revision
        .ok_or_else(|| Status::invalid_argument("Current hero options are required"))?;
    let scope = rpc::WatchViewRequest {
        session_id: body.session_id.clone(),
        run_id: body.run_id.clone(),
        client_binding_id: body.client_binding_id.clone(),
        after_revision: Some(expected),
    };
    let operation = body
        .operation_id
        .clone()
        .and_then(|operation| operation.value);
    if !policy::operation_valid(operation.as_deref()) || body.draft_revision == 0 {
        return Err(Status::invalid_argument(
            "Portrait operation and draft revision are required",
        ));
    }
    let operation =
        operation.ok_or_else(|| Status::invalid_argument("Portrait operation is required"))?;

    let prepared = prepare_portrait(&body)?;
    let fingerprint = prepared.fingerprint.clone();
    let operation_key = format!(
        "{:x}",
        Sha256::digest([binding.as_slice(), &operation].concat())
    );
    let portraits = &service.portraits;
    let retained = {
        let requests = portraits
            .requests
            .lock()
            .map_err(|_| Status::unavailable("Portrait status unavailable"))?;
        requests
            .get(&operation_key)
            .is_some_and(|entry| entry.binding == binding && entry.fingerprint == fingerprint)
            || (dispatch
                && !regenerate
                && requests
                    .values()
                    .any(|entry| entry.binding == binding && entry.fingerprint == fingerprint))
    };
    if retained {
        // The table may have advanced while the phone slept. Authenticate the current
        // seat/run through the actor, but do not require its old presentation revision.
        let (reply, wait) = tokio::sync::oneshot::channel();
        service
            .actor
            .try_submit(super::actor::Call::View {
                credential: binding,
                request: scope,
                reply,
            })
            .map_err(|_| Status::resource_exhausted("The table is busy. Try again."))?;
        let (_, view) = wait
            .await
            .map_err(|_| Status::unavailable("The table is reconnecting."))??;
        if !matches!(view.audience, Some(rpc::view_message::Audience::Player(_))) {
            return Err(Status::permission_denied(
                "Join as a player to make a hero.",
            ));
        }
    } else {
        super::hero_identity::check_preview_scope(service, binding, &scope, &expected).await?;
    }
    let imagery = portraits.imagery.as_ref().ok_or_else(|| {
        Status::failed_precondition(
            "Portrait generation is unavailable. Ask the host to check setup",
        )
    })?;

    let scoped = imagery
        .for_local_creator(binding)
        .await
        .map_err(creator_account_status)?;
    let existing = {
        let mut requests = portraits
            .requests
            .lock()
            .map_err(|_| Status::unavailable("Portrait status unavailable"))?;
        reuse_design(
            &mut requests,
            &operation_key,
            binding,
            &fingerprint,
            dispatch && !regenerate,
        );
        match policy::admit(
            requests
                .get(&operation_key)
                .map(|e| e.fingerprint.as_slice()),
            &fingerprint,
            dispatch,
        ) {
            policy::Admission::Conflict => {
                return Err(Status::already_exists(
                    "This operation belongs to a different appearance",
                ));
            }
            policy::Admission::Unknown => {
                return Err(Status::not_found(
                    "Portrait request is unknown; no new request was sent",
                ));
            }
            policy::Admission::Replay => requests.get(&operation_key).cloned(),
            policy::Admission::New => None,
        }
    };
    let (entry, view) = if let Some(entry) = existing {
        let view = entry.read().await.map_err(service_status)?;
        (entry, Some(view))
    } else {
        portrait_event_scoped(
            &audit,
            &crate::client_diagnostics::operation_hash(&operation).to_string(),
            "request-admitted",
            "new",
            body.draft_revision,
            0,
            "preparing",
        );
        let context = scoped
            .consumer_context()
            .await
            .map_err(|_| Status::unavailable("Portrait setup unavailable"))?;
        let operation_id = operation_id(&context.owner, &operation, body.draft_revision)?;
        let request = service_request(
            &context,
            &body,
            &prepared,
            operation_id,
            &operation_key,
            regenerate,
        )?;
        let handle = OperationHandle {
            owner: context.owner.clone(),
            operation: operation_id,
        };
        let view = match context.service.request(&context.principal, &request).await {
            Ok(
                RequestOutcome::Recorded(view)
                | RequestOutcome::Replayed(view)
                | RequestOutcome::Reused(view),
            ) => Some(view),
            Ok(RequestOutcome::ReusedArtifact(_)) => {
                return Err(Status::unavailable("Portrait assessment is unavailable"));
            }
            Ok(RequestOutcome::Refused(_)) => {
                return Err(Status::failed_precondition(
                    "Portrait generation is unavailable. Ask the host to check setup",
                ));
            }
            Err(ImageServiceError::Unknown { handle: uncertain }) if uncertain == handle => None,
            Err(error) => return Err(service_status(error)),
        };
        let handle = match &view {
            Some(view) if view.handle.owner == context.owner => view.handle.clone(),
            Some(_) => return Err(Status::permission_denied("Portrait account unavailable")),
            None => handle,
        };
        let mut requests = portraits
            .requests
            .lock()
            .map_err(|_| Status::unavailable("Portrait status unavailable"))?;
        let sequence = requests.len();
        let entry = requests
            .entry(operation_key.clone())
            .or_insert(Entry {
                diagnostic_operation: crate::client_diagnostics::operation_hash(&operation),
                fingerprint,
                revision: body.draft_revision,
                handle,
                imagery: scoped,
                binding,
                appearance: prepared.public,
                sequence,
                grant: new_grant()?,
            })
            .clone();
        (entry, view)
    };
    let (state, message) = view
        .as_ref()
        .map(presentation)
        .unwrap_or(("unknown", failure_message("interrupted").to_owned()));
    let mut uri = if state == "ready" {
        format!(
            "/gameplay/creator-portrait/{}",
            super::journey::hex(&entry.grant)
        )
    } else {
        String::new()
    };
    super::media_grant::sign_gameplay_url(
        &mut uri,
        "/gameplay/creator-portrait/",
        super::media_grant::PrincipalAudience::Seat,
        &binding,
    );
    Ok(rpc::HeroPortraitResponse {
        draft_revision: body.draft_revision,
        state: state.into(),
        image_uri: uri,
        message,
    })
}
/// A 256-bit capability for one draft. Random, never derived from the image or the seat.
fn new_grant() -> Result<[u8; 32], Status> {
    let mut grant = [0; 32];
    super::model::fill_os_entropy(&mut grant)
        .map_err(|_| Status::unavailable("Portrait status unavailable"))?;
    Ok(grant)
}

fn reuse_design(
    requests: &mut BTreeMap<String, Entry>,
    operation: &str,
    binding: [u8; 32],
    fingerprint: &[u8],
    reuse: bool,
) {
    if !reuse || requests.contains_key(operation) {
        return;
    }
    // Alias the same resource, including failed/unknown outcomes. Only Regenerate may
    // deliberately buy another attempt after an uncertain result.
    if let Some(entry) = requests
        .values()
        .filter(|entry| entry.binding == binding && entry.fingerprint == fingerprint)
        .max_by_key(|entry| entry.sequence)
        .cloned()
    {
        portrait_event(
            &entry.diagnostic_operation.to_string(),
            "request-reused",
            "existing",
            entry.revision,
            0,
            "preparing",
        );
        requests.insert(operation.to_owned(), entry);
    }
}

fn failure_message(label: &str) -> &'static str {
    match label {
        "classifier_unavailable" => {
            "Your portrait could not be prepared. No image request was sent; your previous portrait is kept."
        }
        "alternate_unavailable" => {
            "This portrait cannot be created right now. Ask the host to check setup; your draft is kept."
        }
        "rating_disallows_appearance" => {
            "This appearance exceeds the campaign's restrained content setting. Edit the public appearance; no image request was sent."
        }
        "classification_uncertain" => {
            "Your portrait needs a clearer public description. Clarify the public description; no image request was sent and your previous portrait is kept."
        }
        "content_requires_revision" => {
            "Revise the appearance: sexualized content, extreme gore, or mature content involving minors, uncertain ages or real people is unsupported. Your draft and previous portrait are kept."
        }
        "bfl_submit_failed" => {
            "The portrait request could not be started. Your draft and previous portrait are kept. Try again when ready."
        }
        "bfl_poll_failed" => {
            "The portrait status could not be checked. The portrait result is unknown; your draft is kept and no new image request was sent."
        }
        "bfl_delivery_failed" => {
            "The portrait image could not be retrieved. Your draft is kept; no retry was sent."
        }
        "bfl_download_failed" => {
            "The portrait image could not be downloaded. Your draft and previous portrait are kept. Try again when ready."
        }
        "refused" => {
            "This appearance could not be generated. Your draft and previous portrait are kept. Edit the public description before trying again."
        }
        "deadline" | "interrupted" => {
            "The portrait result is unknown. Your previous portrait is kept and no automatic retry was sent."
        }
        _ => "The portrait could not be completed. Your appearance and previous portrait are kept.",
    }
}

pub(super) fn router(service: super::Service) -> Router {
    Router::new()
        .route("/gameplay/creator-portrait/{key}", get(serve))
        .with_state(service)
}
async fn serve(
    State(service): State<super::Service>,
    RoutePath(key): RoutePath<String>,
    headers: HeaderMap,
    uri: Uri,
) -> Response {
    let audit = super::service_audit_context(&service, None);
    let approved = service
        .updates
        .borrow()
        .state()
        .continuity
        .canonical_packs
        .iter()
        .flat_map(|p| &p.identities)
        .flat_map(|i| &i.appearances)
        .find(|a| a.key.as_str() == key)
        .cloned();
    if let Some(asset) = approved {
        let response = match approved_reference(&service.imagery, &asset).await {
            Ok(reference) => portrait_image(reference.bytes, reference.format),
            Err(_) => refuse_image(&audit, "approved-source-unavailable", "committed-asset"),
        };
        return super::media_grant::public_media_response(response);
    }
    let Some(grant) = super::media_grant::verify_request(
        &headers,
        &uri,
        &key,
        super::media_grant::Audience::Portrait,
        &[super::media_grant::PrincipalAudience::Seat],
    ) else {
        return super::media_grant::concealed_not_found();
    };
    super::media_grant::secure_media_response(
        serve_draft(&service.portraits, &audit, &key).await,
        grant.remaining,
    )
}
async fn serve_draft(
    portraits: &Portraits,
    audit: &df_observe::AuditContext,
    grant: &str,
) -> Response {
    let Some(presented) = super::parse_binding(grant) else {
        return refuse_image(audit, "unknown-capability", "none");
    };
    let entry = portraits.requests.lock().ok().and_then(|requests| {
        requests.values().fold(None, |found, entry| {
            if same_grant(&entry.grant, &presented) {
                Some(entry.clone())
            } else {
                found
            }
        })
    });
    let Some(entry) = entry else {
        return refuse_image(audit, "unknown-capability", "none");
    };
    let operation = entry.diagnostic_operation.to_string();
    match entry.ready_reference().await {
        Ok(Some((reference, observation)))
            if !reference.bytes.is_empty()
                && reference.bytes.len() as u64 <= MAXIMUM_PORTRAIT_BYTES =>
        {
            if let Some(observer) = entry.imagery.observer() {
                super::image_observability::delivery_authorized(
                    observer.as_ref(),
                    observation.context(),
                    super::image_host::delivery_id(),
                );
            }
            portrait_event_scoped(
                audit,
                &operation,
                "image-served",
                "ready",
                entry.revision,
                reference.bytes.len(),
                "image-bytes",
            );
            portrait_image(reference.bytes, reference.format)
        }
        _ => refuse_image(audit, "current-source-unavailable", &operation),
    }
}
fn refuse_image(audit: &df_observe::AuditContext, reason: &str, operation_hash: &str) -> Response {
    let _scope = df_observe::audit_scope(audit.clone());
    super::actor::record_audit_line(
        "gameplay.creator_portrait_image",
        format!(
            "CREATOR_PORTRAIT event=image-served outcome=not_found reason={reason} operation_hash={operation_hash}"
        ),
    );
    StatusCode::NOT_FOUND.into_response()
}

/// Compares two capabilities over every byte with no early exit, so the time taken does not show
/// how long a prefix matched.
fn same_grant(issued: &[u8; 32], presented: &[u8; 32]) -> bool {
    issued
        .iter()
        .zip(presented)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

fn portrait_image(bytes: impl IntoResponse, format: df_providers::ImageFormat) -> Response {
    let mime = match format {
        df_providers::ImageFormat::Png => "image/png",
        df_providers::ImageFormat::Jpeg => "image/jpeg",
        df_providers::ImageFormat::Webp => "image/webp",
    };
    (
        [
            (header::CONTENT_TYPE, mime),
            (header::CACHE_CONTROL, "private, no-store"),
        ],
        bytes,
    )
        .into_response()
}
#[cfg(test)]
#[path = "creator_portrait_route_tests.rs"]
mod route_tests;
#[cfg(test)]
mod catalog_tests {
    use super::*;
    #[test]
    fn portrait_reuse_matches_design_across_reload_revisions_and_private_story_edits() {
        let mut body = rpc::HeroPortraitRequest {
            species: "dwarf".to_owned(),
            character_class: "fighter-1".to_owned(),
            background: "soldier".to_owned(),
            draft_revision: 1,
            identity: Some(
                super::super::hero_identity::Identity::from_menus(&[])
                    .unwrap()
                    .to_rpc(),
            ),
            ..Default::default()
        };
        let first = prepare_portrait(&body).unwrap();
        body.draft_revision = 24;
        body.identity.as_mut().unwrap().backstory =
            "A private story unrelated to the portrait.".to_owned();
        let restored = prepare_portrait(&body).unwrap();
        assert_eq!(first.fingerprint, restored.fingerprint);
        body.identity
            .as_mut()
            .unwrap()
            .appearance
            .as_mut()
            .unwrap()
            .public_description = "Violet skin and silver horns".to_owned();
        assert_ne!(
            first.fingerprint,
            prepare_portrait(&body).unwrap().fingerprint
        );
    }

    #[test]
    fn creator_authority_capacity_is_retryable_and_authentication_refusal_stays_denied() {
        assert_eq!(
            creator_account_status(imagery::ImageFailure::AuthorityCapacity).code(),
            tonic::Code::Unavailable
        );
        assert_eq!(
            creator_account_status(imagery::ImageFailure::Refused).code(),
            tonic::Code::PermissionDenied
        );
    }

    #[test]
    fn portrait_poll_state_records_keep_operation_state_and_message_key_without_text() {
        for (state, message, expected_key) in [
            ("pending", PREPARING_PORTRAIT, "preparing"),
            ("pending", CREATING_PORTRAIT, "creating"),
            ("ready", READY_PORTRAIT, "ready"),
            (
                "failed",
                failure_message("classifier_unavailable"),
                "classifier_unavailable",
            ),
            (
                "unknown",
                "The portrait result is unknown. No automatic retry was sent.",
                "unknown-result",
            ),
        ] {
            portrait_event(
                "synthetic-operation-hash",
                "poll-answer",
                state,
                7,
                0,
                message,
            );
            let lines = super::super::actor::audit_lines_containing(
                "CREATOR_PORTRAIT event=poll-answer operation_hash=synthetic-operation-hash",
            );
            assert!(lines.iter().any(|line| line.contains(&format!(
                "outcome={state} draft_revision=7 bytes=0 message_key={expected_key}"
            ))));
            assert!(lines.iter().all(|line| !line.contains(message)));
        }
    }

    #[test]
    fn creator_catalog_real_request_preparation_freezes_full_design_and_rejects_unknown_ids() {
        let identity = super::super::hero_identity::Identity::from_menus(&[])
            .unwrap()
            .to_rpc();
        let base = rpc::HeroPortraitRequest {
            species: "dwarf".to_owned(),
            character_class: "fighter-1".to_owned(),
            background: "soldier".to_owned(),
            identity: Some(identity),
            draft_revision: 7,
            ..Default::default()
        };
        let mut catalog = ["species", "class", "background"]
            .into_iter()
            .map(|id| rpc::JourneyGroup {
                group_id: id.to_owned(),
                ..Default::default()
            })
            .collect::<Vec<_>>();
        super::super::creator_catalog::expand_groups(&mut catalog);
        for species in &catalog[0].options {
            for class in &catalog[1].options {
                for background in &catalog[2].options {
                    let request = rpc::HeroPortraitRequest {
                        species: species.option_id.clone(),
                        character_class: class.option_id.clone(),
                        background: background.option_id.clone(),
                        ..base.clone()
                    };
                    assert!(prepare_portrait(&request).is_ok());
                }
            }
        }
        let prepared = prepare_portrait(&base).unwrap();
        let wizard = prepare_portrait(&rpc::HeroPortraitRequest {
            species: "aasimar".to_owned(),
            character_class: "wizard-1".to_owned(),
            background: "sage".to_owned(),
            ..base.clone()
        })
        .unwrap();
        let prompt = imagery::creator_portrait(
            config::ContentRating::Restrained,
            imagery::HeroVisualIdentity {
                identity: &wizard.identity,
                species: "aasimar",
                held_gear: &[],
            },
            "test-boundary",
            &wizard.design,
        )
        .unwrap();
        assert!(prompt.as_str().contains("class styling: Wizard"));
        assert!(prompt.as_str().contains("background styling: Sage"));
        assert!(prompt.as_str().contains("Aasimar"));
        let dragonborn = prepare_portrait(&rpc::HeroPortraitRequest {
            species: "dragonborn".to_owned(),
            character_class: "ranger-1".to_owned(),
            background: "guide".to_owned(),
            ..base.clone()
        })
        .unwrap();
        let dragon_prompt = imagery::creator_portrait(
            config::ContentRating::Restrained,
            imagery::HeroVisualIdentity {
                identity: &dragonborn.identity,
                species: "dragonborn",
                held_gear: &[],
            },
            "dragonborn-boundary",
            &dragonborn.design,
        )
        .unwrap();
        assert!(dragon_prompt.as_str().contains("projecting dragon muzzle"));
        assert!(dragon_prompt.as_str().contains("substantial reptilian jaw"));
        assert!(dragon_prompt.as_str().contains("practical creature heads"));
        assert!(
            dragon_prompt
                .as_str()
                .contains("Explicit custom appearance details take precedence")
        );
        assert!(dragon_prompt.as_str().contains("class styling: Ranger"));
        assert!(dragon_prompt.as_str().contains("background styling: Guide"));
        let (acceptance_public, _) = super::super::creator_catalog::portrait_context(
            &dragonborn.identity,
            "dragonborn",
            "ranger-1",
            "guide",
        )
        .unwrap();
        let (acceptance_public, _) =
            super::super::creator_catalog::with_scenery(acceptance_public, String::new(), "")
                .unwrap();
        assert_eq!(acceptance_public, dragonborn.public);
        assert!(acceptance_public.contains("projecting dragon muzzle"));
        let legacy = rpc::HeroPortraitRequest {
            character_class: String::new(),
            background: String::new(),
            ..base.clone()
        };
        assert_eq!(
            prepare_portrait(&legacy).unwrap().fingerprint,
            prepared.fingerprint
        );
        for (species, class, background) in [
            ("aasimar", "fighter-1", "soldier"),
            ("dwarf", "wizard-1", "soldier"),
            ("dwarf", "fighter-1", "sage"),
        ] {
            let changed = rpc::HeroPortraitRequest {
                species: species.to_owned(),
                character_class: class.to_owned(),
                background: background.to_owned(),
                ..base.clone()
            };
            let next = prepare_portrait(&changed).unwrap();
            assert_ne!(next.public, prepared.public);
            assert_eq!(
                policy::admit(Some(&prepared.fingerprint), &next.fingerprint, true),
                policy::Admission::Conflict
            );
            assert_eq!(
                policy::admit(Some(&prepared.fingerprint), &next.fingerprint, false),
                policy::Admission::Conflict
            );
            let (acceptance_public, _) = super::super::creator_catalog::portrait_context(
                &next.identity,
                species,
                class,
                background,
            )
            .unwrap();
            let (acceptance_public, _) =
                super::super::creator_catalog::with_scenery(acceptance_public, String::new(), "")
                    .unwrap();
            assert_eq!(acceptance_public, next.public);
            assert_ne!(acceptance_public, prepared.public);
        }
        for (species, class, background) in [
            ("unknown", "fighter-1", "soldier"),
            ("aasimar", "unknown", "sage"),
            ("elf", "wizard-1", "unknown"),
        ] {
            let request = rpc::HeroPortraitRequest {
                species: species.to_owned(),
                character_class: class.to_owned(),
                background: background.to_owned(),
                ..base.clone()
            };
            assert_eq!(
                prepare_portrait(&request).err().unwrap().code(),
                tonic::Code::InvalidArgument
            );
        }
    }

    #[test]
    fn creator_catalog_scenery_is_checked_bound_to_requests_and_neutral_in_player_status() {
        let identity = super::super::hero_identity::Identity::from_menus(&[])
            .unwrap()
            .to_rpc();
        let base = rpc::HeroPortraitRequest {
            species: "dragonborn".to_owned(),
            character_class: "ranger-1".to_owned(),
            background: "guide".to_owned(),
            identity: Some(identity),
            draft_revision: 3,
            ..Default::default()
        };
        let plain = prepare_portrait(&base).unwrap();
        let explicit = rpc::HeroPortraitRequest {
            scenery: super::super::creator_catalog::scenery_options()[0]
                .description
                .clone(),
            ..base.clone()
        };
        assert_eq!(
            plain.fingerprint,
            prepare_portrait(&explicit).unwrap().fingerprint
        );
        let scenery = "Ancient ruins beside a waterfall  \r\n薄い霧";
        let request = rpc::HeroPortraitRequest {
            scenery: scenery.to_owned(),
            ..base.clone()
        };
        let prepared = prepare_portrait(&request).unwrap();
        assert_ne!(plain.fingerprint, prepared.fingerprint);
        assert_eq!(
            policy::admit(Some(&plain.fingerprint), &prepared.fingerprint, true),
            policy::Admission::Conflict
        );
        let (appearance, design) = super::super::creator_catalog::portrait_context(
            &prepared.identity,
            "dragonborn",
            "ranger-1",
            "guide",
        )
        .unwrap();
        let (accepted_public, _) =
            super::super::creator_catalog::with_scenery(appearance, design, scenery).unwrap();
        assert_eq!(accepted_public, prepared.public);
        let scene_json: serde_json::Value =
            serde_json::from_str(prepared.public.lines().last().unwrap()).unwrap();
        assert_eq!(scene_json["public_portrait_scenery"], scenery);
        let prompt = imagery::creator_portrait(
            config::ContentRating::Restrained,
            imagery::HeroVisualIdentity {
                identity: &prepared.identity,
                species: "dragonborn",
                held_gear: &[],
            },
            "scenery-boundary",
            &prepared.design,
        )
        .unwrap();
        assert!(prompt.as_str().contains("waterfall"));
        assert!(
            !prompt
                .as_str()
                .contains("against a continuous plain charcoal-grey")
        );
        for maximum in ["x".repeat(360), "界".repeat(256)] {
            assert!(
                prepare_portrait(&rpc::HeroPortraitRequest {
                    scenery: maximum,
                    ..base.clone()
                })
                .is_ok()
            );
        }
        for invalid in ["x".repeat(361), "界".repeat(257), "setting\0".to_owned()] {
            let invalid = rpc::HeroPortraitRequest {
                scenery: invalid,
                ..base.clone()
            };
            assert_eq!(
                prepare_portrait(&invalid).err().unwrap().code(),
                tonic::Code::InvalidArgument
            );
        }
        for note in [
            PREPARING_PORTRAIT,
            CREATING_PORTRAIT,
            READY_PORTRAIT,
            failure_message("classifier_unavailable"),
            failure_message("alternate_unavailable"),
            failure_message("bfl_submit_failed"),
            failure_message("bfl_poll_failed"),
            failure_message("bfl_delivery_failed"),
            failure_message("bfl_download_failed"),
        ] {
            for internal in ["Jev", "BFL", "FLUX", "OpenAI", "API_KEY", "anatomy detail"] {
                assert!(!note.contains(internal));
            }
        }
    }

    #[test]
    fn a_capability_matches_only_when_every_byte_matches() {
        let issued = [0x5a; 32];
        assert!(same_grant(&issued, &[0x5a; 32]));
        let mut last_byte_differs = issued;
        last_byte_differs[31] ^= 1;
        assert!(!same_grant(&issued, &last_byte_differs));
        let mut first_byte_differs = issued;
        first_byte_differs[0] ^= 0x80;
        assert!(!same_grant(&issued, &first_byte_differs));
    }
}
