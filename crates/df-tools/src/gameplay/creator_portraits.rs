//! Explicit creator portraits, owned by the normal run resource worker.
#[path = "creator_portrait_policy.rs"]
mod policy;
use super::{
    creator_classification::{self, Classifier, Route},
    imagery::{self, ImageKind, ImageRequest, Imagery, OutputSpec},
    resources::{CreatorPortraitSpec, Generated, ResourceFailure, ResourceKey, ResourceStatus},
    startup_configuration as config,
};
use axum::{
    Router,
    extract::{Path as RoutePath, State},
    http::{HeaderMap, StatusCode, Uri, header},
    response::{IntoResponse, Response},
    routing::get,
};
use df_protocol::common as rpc;
pub(super) fn frozen_metadata<T>(uncertain: Option<T>, committed: Option<T>, fresh: T) -> T {
    policy::frozen_metadata(uncertain, committed, fresh)
}
#[cfg(test)]
use super::resources::Hub;
use sha2::{Digest, Sha256};
#[cfg(test)]
use std::time::Duration;
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Arc, Mutex},
};
use tonic::{Request, Status};

const PREPARING_PORTRAIT: &str = "Preparing your portrait…";
const CREATING_PORTRAIT: &str = "Creating your portrait…";
const READY_PORTRAIT: &str = "Portrait ready";
/// The largest portrait image served or attached, drafts and approved appearances alike.
const MAXIMUM_PORTRAIT_BYTES: u64 = 6 * 1024 * 1024;

#[derive(Clone)]
pub(super) struct Portraits {
    imagery: Option<Imagery>,
    alternate: Option<Imagery>,
    classifier: Option<Arc<Classifier>>,
    requests: Arc<Mutex<BTreeMap<String, Entry>>>,
    cache: std::path::PathBuf,
}
#[derive(Clone)]
struct Entry {
    diagnostic_operation: u64,
    fingerprint: Vec<u8>,
    revision: u64,
    resource: ResourceKey,
    binding: [u8; 32],
    appearance: String,
    sequence: usize,
    progress: Arc<Mutex<String>>,
    /// The capability that opens this draft's image; only the owning seat's answer carries it.
    grant: [u8; 32],
}
impl Portraits {
    /// Receives the already admitted providers from the gameplay composition root.
    pub(super) fn new(
        imagery: Option<Imagery>,
        alternate: Option<Imagery>,
        classifier: Option<Arc<Classifier>>,
        cache: std::path::PathBuf,
    ) -> Self {
        Self {
            imagery,
            alternate,
            classifier,
            requests: Arc::new(Mutex::new(BTreeMap::new())),
            cache,
        }
    }

    #[cfg(test)]
    pub(super) fn disabled() -> Self {
        Self::new(
            None,
            None,
            None,
            std::path::Path::new("artifacts/tmp/disabled-creator-portraits").to_path_buf(),
        )
    }
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
        let candidate = {
            let requests = self.requests.lock().map_err(|_| {
                super::actor::record_ingress_refusal(
                    rpc::GameplayActionKind::CreateCharacter as i32,
                    "creator-portrait-attachment-state",
                    "RequestMapPoisoned",
                    tonic::Code::Unavailable,
                );
                Status::unavailable("Portrait status unavailable")
            })?;
            requests
                .values()
                .filter(|entry| entry.binding == binding && entry.appearance == public)
                .filter(|entry| {
                    self.imagery.as_ref().is_some_and(|imagery| {
                        matches!(
                            imagery.hub().status(&entry.resource),
                            ResourceStatus::Ready(_)
                        )
                    })
                })
                .max_by_key(|entry| entry.sequence)
                .and_then(|entry| {
                    self.imagery.as_ref().and_then(|imagery| {
                        match imagery.hub().status(&entry.resource) {
                            ResourceStatus::Ready(generated) => {
                                Some((generated.handle, entry.revision, entry.diagnostic_operation))
                            }
                            _ => None,
                        }
                    })
                })
        };
        let Some((key, revision, operation_hash)) = candidate else {
            super::actor::record_audit_line(
                "gameplay.creator_portrait_attach",
                format!(
                    "CREATOR_PORTRAIT event=attach outcome=none reason=not_ready operation_hash=none bytes=0 seat={}",
                    df_observe::seat_hash(&binding)
                ),
            );
            return Ok(None);
        };
        match self.image(&key).await {
            imagery::Lookup::Image { bytes, .. }
                if !bytes.is_empty() && bytes.len() as u64 <= MAXIMUM_PORTRAIT_BYTES =>
            {
                super::actor::record_audit_line(
                    "gameplay.creator_portrait_attach",
                    format!(
                        "CREATOR_PORTRAIT event=attach outcome=ready reason=attached operation_hash={operation_hash} bytes={} seat={}",
                        bytes.len(),
                        df_observe::seat_hash(&binding)
                    ),
                );
                Ok(Some(Attachment {
                    key,
                    digest: Sha256::digest(&bytes).into(),
                    bytes: bytes.len() as u64,
                    revision,
                }))
            }
            // A vanished cache must not prevent the actor from resolving an exact retry.
            result => {
                let (reason, bytes) = match result {
                    imagery::Lookup::Image { bytes, .. } => ("oversize", bytes.len()),
                    _ => ("cache_miss", 0),
                };
                super::actor::record_audit_line(
                    "gameplay.creator_portrait_attach",
                    format!(
                        "CREATOR_PORTRAIT event=attach outcome=none reason={reason} operation_hash={operation_hash} bytes={bytes} seat={}",
                        df_observe::seat_hash(&binding)
                    ),
                );
                Ok(None)
            }
        }
    }
}

impl Portraits {
    async fn image(&self, key: &str) -> imagery::Lookup {
        for imagery in [&self.imagery, &self.alternate].into_iter().flatten() {
            let result = imagery.image(key).await;
            if matches!(result, imagery::Lookup::Image { .. }) {
                return result;
            }
        }
        imagery::Lookup::NotReady
    }
}

/// Reads only a committed approved image reference, with the accepted length and digest.
/// No creator request, provider call or raw camera source is consulted here.
pub(super) async fn approved_reference(
    asset: &df_model::checkpoint::AssetReference,
) -> Result<imagery::ReferenceImage, imagery::ImageFailure> {
    let root = std::env::var("DF_GAMEPLAY_IMAGERY_CACHE")
        .ok()
        .filter(|value| !value.is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("artifacts/tmp/play/imagery-cache"));
    approved_reference_at(asset, &root).await
}

async fn approved_reference_at(
    asset: &df_model::checkpoint::AssetReference,
    root: &Path,
) -> Result<imagery::ReferenceImage, imagery::ImageFailure> {
    use imagery::ImageFailure;
    let key = asset.key.as_str();
    if key.len() != 64
        || !key.bytes().all(|byte| byte.is_ascii_hexdigit())
        || asset.kind != df_model::checkpoint::AssetKind::Image
        || asset.byte_length == 0
        || asset.byte_length > MAXIMUM_PORTRAIT_BYTES
    {
        return Err(ImageFailure::InvalidRequest);
    }
    let path = root.join(format!("{key}.jpg"));
    let asset = asset.clone();
    tokio::task::spawn_blocking(move || {
        use std::io::Read;
        let file = std::fs::File::open(path).map_err(|_| ImageFailure::InvalidRequest)?;
        if file
            .metadata()
            .map_err(|_| ImageFailure::InvalidRequest)?
            .len()
            != asset.byte_length
        {
            return Err(ImageFailure::InvalidRequest);
        }
        let mut bytes = Vec::new();
        file.take(MAXIMUM_PORTRAIT_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| ImageFailure::InvalidRequest)?;
        if bytes.len() as u64 != asset.byte_length
            || <[u8; 32]>::from(Sha256::digest(&bytes)) != asset.digest.0
        {
            return Err(ImageFailure::InvalidRequest);
        }
        Ok(imagery::ReferenceImage::new(
            df_providers::ImageFormat::Jpeg,
            bytes.into(),
        ))
    })
    .await
    .map_err(|_| ImageFailure::Interrupted)?
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

/// A single admitted creator operation. Keeping this unit separate makes the actual
/// classifier-to-provider dispatch testable with the same adapters used in play.
struct PortraitJob {
    audit: df_observe::AuditContext,
    classifier: Arc<Classifier>,
    source: String,
    operation: String,
    primary: Imagery,
    alternate: Option<Imagery>,
    request: ImageRequest,
    general_request: ImageRequest,
    progress: Arc<Mutex<String>>,
}
impl PortraitJob {
    async fn run(self) -> Result<Generated, ResourceFailure> {
        let Self {
            audit,
            classifier,
            source,
            operation,
            primary,
            alternate,
            request,
            general_request,
            progress,
        } = self;
        portrait_event_scoped(
            &audit,
            &operation,
            "classification-start",
            "pending",
            0,
            0,
            "preparing",
        );
        let assessment = classifier
            .classify(&source, &operation)
            .await
            .map_err(|_| {
                portrait_event_scoped(
                    &audit,
                    &operation,
                    "classification-finish",
                    "failed",
                    0,
                    0,
                    "classifier_unavailable",
                );
                ResourceFailure::Permanent("classifier_unavailable")
            })?;
        portrait_event_scoped(
            &audit,
            &operation,
            "classification-finish",
            assessment.category(),
            0,
            0,
            "classified",
        );
        let route = assessment
            .route_for_rating(config::content_rating() == config::ContentRating::Mature)
            .map_err(|_| {
                portrait_event_scoped(
                    &audit,
                    &operation,
                    "route",
                    "failed",
                    0,
                    0,
                    "rating_disallows_appearance",
                );
                ResourceFailure::Permanent("rating_disallows_appearance")
            })?;
        let (chosen, provider, request) = match route {
            Route::General => {
                let provider = primary.model_label();
                (primary, provider, general_request)
            }
            Route::Bfl => (
                alternate.ok_or_else(|| {
                    portrait_event_scoped(
                        &audit,
                        &operation,
                        "route",
                        "failed",
                        0,
                        0,
                        "alternate_unavailable",
                    );
                    ResourceFailure::Permanent("alternate_unavailable")
                })?,
                "BFL FLUX 3",
                if assessment.requires_mature_prompt() {
                    request
                } else {
                    general_request
                },
            ),
            Route::Revise => {
                portrait_event_scoped(
                    &audit,
                    &operation,
                    "route",
                    "refused",
                    0,
                    0,
                    "content_requires_revision",
                );
                return Err(ResourceFailure::Permanent("content_requires_revision"));
            }
            Route::Uncertain => {
                portrait_event_scoped(
                    &audit,
                    &operation,
                    "route",
                    "unknown",
                    0,
                    0,
                    "classification_uncertain",
                );
                return Err(ResourceFailure::Permanent("classification_uncertain"));
            }
        };
        let appearance_hash = format!("{:x}", Sha256::digest(source.as_bytes()));
        let model = chosen.model_label();
        let request_hash = format!("{:x}", Sha256::digest(request.prompt.as_str().as_bytes()));
        {
            let _scope = df_observe::audit_scope(audit.clone());
            super::actor::record_audit_diagnostic(
                "gameplay.creator_portrait_route",
                format!(
                    "CREATOR_ROUTE classifier=jev-latest category={} provider={provider} model={model} operation={operation} appearance_hash={appearance_hash} request_hash={request_hash}",
                    assessment.category()
                ),
                serde_json::Map::from_iter([
                    ("request_hash".into(), serde_json::json!(request_hash)),
                    ("classifier".into(), serde_json::json!("jev-latest")),
                    ("category".into(), serde_json::json!(assessment.category())),
                    ("provider".into(), serde_json::json!(provider)),
                    ("model".into(), serde_json::json!(model)),
                    ("operation".into(), serde_json::json!(operation)),
                    ("appearance_hash".into(), serde_json::json!(appearance_hash)),
                ]),
                &[],
            );
        }
        if let Ok(mut message) = progress.lock() {
            *message = CREATING_PORTRAIT.to_owned();
        }
        portrait_event_scoped(
            &audit,
            &operation,
            "provider-start",
            "pending",
            0,
            0,
            "creating",
        );
        let started = std::time::Instant::now();
        let result = chosen.render_creator(request).await;
        {
            let _scope = df_observe::audit_scope(audit.clone());
            super::actor::record_audit_line(
                "gameplay.creator_portrait_provider",
                format!(
                    "CREATOR_PORTRAIT event=provider-finish operation_hash={operation} outcome={} elapsed_ms={}",
                    result
                        .as_ref()
                        .map_or_else(|failure| failure.label(), |_| "ready"),
                    started.elapsed().as_millis()
                ),
            );
        }
        let key = result.map_err(|failure| ResourceFailure::Permanent(failure.label()))?;
        let mut generated = Generated::new(key.as_str());
        generated.note = READY_PORTRAIT.to_owned();
        Ok(generated)
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
    let operation = operation.expect("validated nonzero operation");

    let PreparedPortrait {
        identity,
        public,
        design,
        fingerprint,
    } = prepare_portrait(&body)?;
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
            .map(|entry| entry.fingerprint.as_slice()),
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
        policy::Admission::Replay => {}
        policy::Admission::New => {
            portrait_event_scoped(
                &audit,
                &crate::client_diagnostics::operation_hash(&operation).to_string(),
                "request-admitted",
                "new",
                body.draft_revision,
                0,
                "preparing",
            );
            let portrait_binding = format!("{}|{operation_key}", creator_classification::VERSION);
            let general_prompt = imagery::creator_portrait(
                config::ContentRating::Restrained,
                imagery::HeroVisualIdentity {
                    identity: &identity,
                    species: &body.species,
                    held_gear: &[],
                },
                &portrait_binding,
                &design,
            )
            .map_err(|_| Status::invalid_argument("Portrait description is too long"))?;
            let prompt = imagery::creator_portrait(
                config::content_rating(),
                imagery::HeroVisualIdentity {
                    identity: &identity,
                    species: &body.species,
                    held_gear: &[],
                },
                &portrait_binding,
                &design,
            )
            .map_err(|_| Status::invalid_argument("Portrait description is too long"))?;
            let request = ImageRequest {
                kind: ImageKind::CanonicalCharacter,
                references: vec![],
                prompt,
                output: OutputSpec {
                    width: 1024,
                    height: 1536,
                    transparent: false,
                },
            };
            let general_request = ImageRequest {
                prompt: general_prompt,
                ..request.clone()
            };
            let classifier = portraits
                .classifier
                .as_ref()
                .ok_or_else(|| {
                    Status::failed_precondition(
                        "Portrait generation is unavailable. Try again after the host checks setup",
                    )
                })?
                .clone();
            let source = public.clone();
            let diagnostic_operation = crate::client_diagnostics::operation_hash(&operation);
            let operation = diagnostic_operation.to_string();
            let primary = imagery.clone();
            let alternate = portraits.alternate.clone();
            // A frozen operation owns one classifier attempt and exactly one selected image route.
            let versioned_input = format!(
                "{}|{}|{}|{}|{}|{}",
                creator_classification::VERSION,
                operation_key,
                public,
                alternate.is_some(),
                primary.model_label(),
                alternate.as_ref().map_or("off", Imagery::model_label)
            );
            let spec = CreatorPortraitSpec::new(
                &format!("creator:{operation_key}"),
                &versioned_input,
                df_render::art_contract::Neutral::Nothing,
            );
            let resource = ResourceKey::of(&spec);
            let grant = new_grant()?;
            let progress = Arc::new(Mutex::new(PREPARING_PORTRAIT.to_owned()));
            let job_progress = progress.clone();
            imagery.hub().get_or_request(
                &spec,
                Arc::new(move || {
                    let audit = audit.clone();
                    let (
                        classifier,
                        source,
                        operation,
                        primary,
                        alternate,
                        request,
                        general_request,
                    ) = (
                        classifier.clone(),
                        source.clone(),
                        operation.clone(),
                        primary.clone(),
                        alternate.clone(),
                        request.clone(),
                        general_request.clone(),
                    );
                    let progress = job_progress.clone();
                    Box::pin(
                        PortraitJob {
                            audit,
                            classifier,
                            source,
                            operation,
                            primary,
                            alternate,
                            request,
                            general_request,
                            progress,
                        }
                        .run(),
                    )
                }),
            );
            let sequence = requests.len();
            requests.insert(
                operation_key.clone(),
                Entry {
                    diagnostic_operation,
                    fingerprint,
                    binding,
                    appearance: public,
                    sequence,
                    progress,
                    revision: body.draft_revision,
                    resource,
                    grant,
                },
            );
        }
    }
    let entry = &requests[&operation_key];
    let (state, mut uri, message) = match imagery.hub().status(&entry.resource) {
        ResourceStatus::Ready(generated) => (
            "ready",
            format!(
                "/gameplay/creator-portrait/{}",
                super::journey::hex(&entry.grant)
            ),
            generated.note,
        ),
        ResourceStatus::Pending(_) => (
            "pending",
            String::new(),
            entry.progress.lock().map(|message| message.clone()).unwrap_or_else(|_| "Creating your portrait...".to_owned()),
        ),
        ResourceStatus::Failed(failure, _) if matches!(failure.label(), "deadline_exceeded"|"interrupted"|"timed_out") => (
            "unknown", String::new(), "The portrait result is unknown. Your previous portrait is kept and no automatic retry was sent.".to_owned(),
        ),
        ResourceStatus::Failed(failure, _) => (
            "failed",
            String::new(),
            failure_message(failure.label()).to_owned(),
        ),
        ResourceStatus::Fallback(_) => (
            "unknown",
            String::new(),
            "The portrait result is unknown. No automatic retry was sent.".to_owned(),
        ),
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
        .flat_map(|pack| &pack.identities)
        .flat_map(|identity| &identity.appearances)
        .find(|asset| asset.key.as_str() == key)
        .cloned();
    if approved.is_some() {
        return super::media_grant::public_media_response(
            respond(&service.portraits, &audit, &key, approved).await,
        );
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
        respond(&service.portraits, &audit, &key, approved).await,
        grant.remaining,
    )
}

/// Answers one portrait request. An approved appearance is public to the room (the shared display
/// loads it by its content key). Anything else is a draft, served only to its capability.
async fn respond(
    portraits: &Portraits,
    audit: &df_observe::AuditContext,
    key: &str,
    approved: Option<df_model::checkpoint::AssetReference>,
) -> Response {
    if key.len() != 64
        || !key
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        let _scope = df_observe::audit_scope(audit.clone());
        super::actor::record_audit_line("gameplay.creator_portrait_image", "CREATOR_PORTRAIT event=image-served outcome=not_found reason=invalid-key operation_hash=none".to_owned());
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(asset) = approved else {
        return serve_draft(portraits, audit, key).await;
    };
    let operation_hash = portraits
        .requests
        .lock()
        .ok()
        .and_then(|requests| {
            requests.iter().find_map(|(_operation, entry)| {
                portraits.imagery.as_ref().and_then(|imagery| {
                    match imagery.hub().status(&entry.resource) {
                        ResourceStatus::Ready(generated) if generated.handle == key => {
                            Some(entry.diagnostic_operation.to_string())
                        }
                        _ => None,
                    }
                })
            })
        })
        .unwrap_or_else(|| "committed-asset".to_owned());
    if asset.byte_length > MAXIMUM_PORTRAIT_BYTES {
        let _scope = df_observe::audit_scope(audit.clone());
        super::actor::record_audit_line("gameplay.creator_portrait_image", "CREATOR_PORTRAIT event=image-served outcome=not_found reason=oversize operation_hash=none".to_owned());
        return StatusCode::NOT_FOUND.into_response();
    }
    let path = portraits.cache.join(format!("{key}.jpg"));
    match tokio::task::spawn_blocking(move || {
        use std::io::Read;
        let file = std::fs::File::open(path)?;
        if file.metadata()?.len() != asset.byte_length {
            return Err(std::io::Error::other("Portrait cache size mismatch"));
        }
        let mut bytes = Vec::new();
        file.take(MAXIMUM_PORTRAIT_BYTES + 1)
            .read_to_end(&mut bytes)?;
        Ok::<_, std::io::Error>(bytes)
    })
    .await
    {
        Ok(Ok(bytes))
            if bytes.len() as u64 == asset.byte_length
                && <[u8; 32]>::from(Sha256::digest(&bytes)) == asset.digest.0 =>
        {
            portrait_event_scoped(
                audit,
                &operation_hash,
                "image-served",
                "ready",
                0,
                bytes.len(),
                "image-bytes",
            );
            portrait_image(bytes)
        }
        _ => {
            let _scope = df_observe::audit_scope(audit.clone());
            super::actor::record_audit_line(
                "gameplay.creator_portrait_image",
                format!(
                    "CREATOR_PORTRAIT event=image-served outcome=not_found reason=cache-integrity operation_hash={operation_hash}"
                ),
            );
            StatusCode::NOT_FOUND.into_response()
        }
    }
}

/// A draft is private to the seat it was issued to. Its content key is never a way in: the
/// capability in the path is the only one, because a browser image request cannot carry the
/// seat credential.
async fn serve_draft(
    portraits: &Portraits,
    audit: &df_observe::AuditContext,
    grant: &str,
) -> Response {
    let Some(presented) = super::parse_binding(grant) else {
        return refuse_image(audit, "unknown-capability", "none");
    };
    let issued = portraits.requests.lock().ok().and_then(|requests| {
        // Every entry is compared, so the scan does not reveal which draft matched.
        let entry = requests.values().fold(None, |found, entry| {
            if same_grant(&entry.grant, &presented) {
                Some(entry)
            } else {
                found
            }
        })?;
        match portraits.imagery.as_ref()?.hub().status(&entry.resource) {
            ResourceStatus::Ready(generated) => {
                Some((generated.handle, entry.diagnostic_operation))
            }
            _ => None,
        }
    });
    let Some((handle, operation)) = issued else {
        return refuse_image(audit, "unknown-capability", "none");
    };
    let operation_hash = operation.to_string();
    match portraits.image(&handle).await {
        imagery::Lookup::Image { bytes, format: _ }
            if bytes.len() as u64 > MAXIMUM_PORTRAIT_BYTES =>
        {
            refuse_image(audit, "oversize", &operation_hash)
        }
        imagery::Lookup::Image { bytes, format: _ } => {
            portrait_event_scoped(
                audit,
                &operation_hash,
                "image-served",
                "ready",
                0,
                bytes.len(),
                "image-bytes",
            );
            portrait_image(bytes)
        }
        _ => refuse_image(audit, "cache-miss", &operation_hash),
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

fn portrait_image(bytes: impl IntoResponse) -> Response {
    (
        [
            (header::CONTENT_TYPE, "image/jpeg"),
            (header::CACHE_CONTROL, "private, max-age=86400, immutable"),
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

    #[tokio::test]
    async fn ready_current_design_attaches_the_same_checked_portrait_without_another_image_call() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
            "../../artifacts/tmp/lanes/HS8/accepted-portrait-{}",
            std::process::id()
        ));
        let fixture = super::super::imagery::tests::DirectionFixture::new(&root);
        let identity = super::super::hero_identity::Identity::from_menus(&[])
            .unwrap()
            .to_rpc();
        let body = rpc::HeroPortraitRequest {
            identity: Some(identity.clone()),
            species: "dwarf".to_owned(),
            character_class: "fighter-1".to_owned(),
            background: "soldier".to_owned(),
            draft_revision: 7,
            ..Default::default()
        };
        let prepared = prepare_portrait(&body).unwrap();
        let request = imagery::creator_portrait(
            config::ContentRating::Restrained,
            imagery::HeroVisualIdentity {
                identity: &prepared.identity,
                species: &body.species,
                held_gear: &[],
            },
            "HS8-explicit-portrait-fixture",
            &prepared.design,
        )
        .unwrap();
        let key = fixture
            .imagery
            .render_creator(ImageRequest {
                kind: ImageKind::CanonicalCharacter,
                references: vec![],
                prompt: request,
                output: OutputSpec {
                    width: 1024,
                    height: 1536,
                    transparent: false,
                },
            })
            .await
            .unwrap();
        let spec = CreatorPortraitSpec::new(
            "accepted-hero",
            "HS8-portrait",
            df_render::art_contract::Neutral::Nothing,
        );
        let handle = key.as_str().to_owned();
        fixture.imagery.hub().get_or_request(
            &spec,
            Arc::new(move || {
                let handle = handle.clone();
                Box::pin(async move { Ok(Generated::new(handle)) })
            }),
        );
        let mut portraits = Portraits::disabled();
        portraits.imagery = Some(fixture.imagery.clone());
        portraits.requests.lock().unwrap().insert(
            "operation".to_owned(),
            Entry {
                diagnostic_operation: 1,
                fingerprint: prepared.fingerprint,
                appearance: prepared.public,
                binding: [1; 32],
                revision: 7,
                resource: ResourceKey::of(&spec),
                sequence: 0,
                progress: Arc::new(Mutex::new(CREATING_PORTRAIT.to_owned())),
                grant: [3; 32],
            },
        );
        let mut selection = rpc::CharacterSelection {
            name: "Vesna".to_owned(),
            identity: Some(identity),
            choices: [
                ("species", "dwarf"),
                ("class", "fighter-1"),
                ("background", "soldier"),
            ]
            .into_iter()
            .map(|(group, option)| rpc::JourneyChoice {
                group_id: group.to_owned(),
                option_id: option.to_owned(),
            })
            .collect(),
            ..Default::default()
        };
        // Creation without a completed resource succeeds without an attachment.
        assert!(
            portraits
                .accepted([1; 32], Some(&selection))
                .await
                .unwrap()
                .is_none()
        );
        for job in fixture.imagery.hub().take_jobs(1) {
            fixture.imagery.hub().complete(job.execute().await);
        }
        let attached = portraits
            .accepted([1; 32], Some(&selection))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(attached.key, key.as_str());
        assert_eq!(attached.revision, 7);
        assert!(attached.bytes > 0);
        assert!(
            portraits
                .accepted([2; 32], Some(&selection))
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(
            portraits
                .accepted([1; 32], Some(&selection))
                .await
                .unwrap()
                .unwrap()
                .digest,
            attached.digest
        );
        selection
            .identity
            .as_mut()
            .unwrap()
            .appearance
            .as_mut()
            .unwrap()
            .public_description = "A changed look".to_owned();
        assert!(
            portraits
                .accepted([1; 32], Some(&selection))
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(fixture.calls(), 1);
    }

    #[tokio::test]
    async fn portrait_reuse_returns_one_resource_in_flight_ready_failed_and_unknown_without_retry()
    {
        use std::sync::atomic::{AtomicUsize, Ordering};
        for outcome in [
            Ok(Generated::new("a".repeat(64))),
            Err(ResourceFailure::Permanent("refused")),
            Err(ResourceFailure::Interrupted),
        ] {
            let hub = Hub::new();
            hub.set_limits(
                super::super::resources::ResourceKind::CreatorPortrait,
                super::super::resources::Limits {
                    timeout: Duration::from_secs(1),
                    attempts: 1,
                },
            );
            let expected = outcome.clone();
            let calls = Arc::new(AtomicUsize::new(0));
            let counted = calls.clone();
            let spec = CreatorPortraitSpec::new(
                "synthetic-hero",
                "unchanged-design",
                df_render::art_contract::Neutral::Nothing,
            );
            hub.get_or_request(
                &spec,
                Arc::new(move || {
                    counted.fetch_add(1, Ordering::SeqCst);
                    let outcome = outcome.clone();
                    Box::pin(async move { outcome })
                }),
            );
            let resource = ResourceKey::of(&spec);
            let mut requests = BTreeMap::from([(
                "original".to_owned(),
                Entry {
                    diagnostic_operation: 1,
                    fingerprint: vec![8; 32],
                    revision: 1,
                    resource: resource.clone(),
                    binding: [1; 32],
                    appearance: "unchanged-design".to_owned(),
                    sequence: 0,
                    progress: Arc::new(Mutex::new(PREPARING_PORTRAIT.to_owned())),
                    grant: [4; 32],
                },
            )]);
            reuse_design(&mut requests, "reloaded", [1; 32], &[8; 32], true);
            assert_eq!(requests["reloaded"].resource, resource);
            let jobs = hub.take_jobs(5);
            assert_eq!(jobs.len(), 1);
            assert!(hub.take_jobs(5).is_empty());
            for job in jobs {
                hub.complete(job.execute().await);
            }
            match (hub.status(&resource), expected) {
                (ResourceStatus::Ready(actual), Ok(expected)) => {
                    assert_eq!(actual.handle, expected.handle)
                }
                (ResourceStatus::Failed(actual, _), Err(expected)) => assert_eq!(actual, expected),
                (actual, expected) => {
                    panic!("unexpected resource result {actual:?} for {expected:?}")
                }
            }
            reuse_design(&mut requests, "after-ready", [1; 32], &[8; 32], true);
            assert_eq!(requests["after-ready"].resource, resource);
            reuse_design(
                &mut requests,
                "explicit-regenerate",
                [1; 32],
                &[8; 32],
                false,
            );
            assert!(!requests.contains_key("explicit-regenerate"));
            reuse_design(&mut requests, "other-seat", [2; 32], &[8; 32], true);
            reuse_design(&mut requests, "edited-design", [1; 32], &[9; 32], true);
            assert!(!requests.contains_key("other-seat"));
            assert!(!requests.contains_key("edited-design"));
            assert_eq!(calls.load(Ordering::SeqCst), 1);
            assert!(hub.take_jobs(5).is_empty());
        }
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

    #[tokio::test]
    async fn approved_reference_checks_digest_length_key_and_kind_before_use() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../artifacts/tmp/lanes/HS8/approved-reference");
        std::fs::create_dir_all(&root).unwrap();
        let bytes = b"approved generated portrait fixture";
        let mut asset = df_model::checkpoint::AssetReference {
            key: super::super::model::label(&"5".repeat(64)).unwrap(),
            digest: df_model::checkpoint::ContentDigest(Sha256::digest(bytes).into()),
            byte_length: bytes.len() as u64,
            kind: df_model::checkpoint::AssetKind::Image,
        };
        std::fs::write(root.join(format!("{}.jpg", asset.key.as_str())), bytes).unwrap();
        assert_eq!(
            approved_reference_at(&asset, &root)
                .await
                .unwrap()
                .bytes
                .as_ref(),
            bytes
        );
        asset.digest.0[0] ^= 1;
        assert!(approved_reference_at(&asset, &root).await.is_err());
        asset.digest.0[0] ^= 1;
        asset.byte_length += 1;
        assert!(approved_reference_at(&asset, &root).await.is_err());
        asset.byte_length -= 1;
        asset.kind = df_model::checkpoint::AssetKind::Audio;
        assert!(approved_reference_at(&asset, &root).await.is_err());
        asset.kind = df_model::checkpoint::AssetKind::Image;
        asset.key = super::super::model::label("unapproved-key").unwrap();
        assert!(approved_reference_at(&asset, &root).await.is_err());
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

    /// A draft whose image is ready in the creator cache: owned by `owner`, opened by `grant`.
    /// Returns the content key the cache knows the image by.
    async fn ready_draft(
        portraits: &Portraits,
        fixture: &super::super::imagery::tests::DirectionFixture,
        label: &str,
        owner: [u8; 32],
        grant: [u8; 32],
    ) -> String {
        let identity = super::super::hero_identity::Identity::from_menus(&[])
            .unwrap()
            .to_rpc();
        let body = rpc::HeroPortraitRequest {
            identity: Some(identity),
            species: "dwarf".to_owned(),
            character_class: "fighter-1".to_owned(),
            background: "soldier".to_owned(),
            draft_revision: 7,
            ..Default::default()
        };
        let prepared = prepare_portrait(&body).unwrap();
        let prompt = imagery::creator_portrait(
            config::ContentRating::Restrained,
            imagery::HeroVisualIdentity {
                identity: &prepared.identity,
                species: &body.species,
                held_gear: &[],
            },
            label,
            &prepared.design,
        )
        .unwrap();
        let key = fixture
            .imagery
            .render_creator(ImageRequest {
                kind: ImageKind::CanonicalCharacter,
                references: vec![],
                prompt,
                output: OutputSpec {
                    width: 1024,
                    height: 1536,
                    transparent: false,
                },
            })
            .await
            .unwrap();
        let spec = CreatorPortraitSpec::new(
            &format!("draft:{label}"),
            label,
            df_render::art_contract::Neutral::Nothing,
        );
        let handle = key.as_str().to_owned();
        fixture.imagery.hub().get_or_request(
            &spec,
            Arc::new(move || {
                let handle = handle.clone();
                Box::pin(async move { Ok(Generated::new(handle)) })
            }),
        );
        for job in fixture.imagery.hub().take_jobs(1) {
            fixture.imagery.hub().complete(job.execute().await);
        }
        let mut requests = portraits.requests.lock().unwrap();
        let sequence = requests.len();
        requests.insert(
            label.to_owned(),
            Entry {
                diagnostic_operation: 1,
                fingerprint: vec![9; 32],
                revision: 7,
                resource: ResourceKey::of(&spec),
                binding: owner,
                appearance: prepared.public,
                sequence,
                progress: Arc::new(Mutex::new(READY_PORTRAIT.to_owned())),
                grant,
            },
        );
        key.as_str().to_owned()
    }

    async fn served_bytes(response: Response) -> Vec<u8> {
        assert_eq!(response.status(), StatusCode::OK);
        axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec()
    }

    #[tokio::test]
    async fn a_draft_portrait_is_not_served_by_its_content_key_or_an_unissued_capability() {
        let root = std::env::temp_dir().join(format!("df-draft-by-key-{}", std::process::id()));
        let _removed = std::fs::remove_dir_all(&root);
        let fixture = super::super::imagery::tests::DirectionFixture::new(&root);
        let mut portraits = Portraits::disabled();
        portraits.imagery = Some(fixture.imagery.clone());
        let content_key = ready_draft(&portraits, &fixture, "draft-a", [1; 32], [0x11; 32]).await;
        let audit = df_observe::AuditContext::default();
        assert_eq!(
            respond(&portraits, &audit, &content_key, None)
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            respond(&portraits, &audit, &"0".repeat(64), None)
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
        let _removed = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn each_draft_opens_only_for_the_capability_issued_to_its_own_seat() {
        let root = std::env::temp_dir().join(format!("df-draft-by-grant-{}", std::process::id()));
        let _removed = std::fs::remove_dir_all(&root);
        let fixture = super::super::imagery::tests::DirectionFixture::new(&root);
        let mut portraits = Portraits::disabled();
        portraits.imagery = Some(fixture.imagery.clone());
        let first_grant = [0x11; 32];
        let second_grant = [0x22; 32];
        let first_key = ready_draft(&portraits, &fixture, "draft-a", [1; 32], first_grant).await;
        let second_key = ready_draft(&portraits, &fixture, "draft-b", [2; 32], second_grant).await;
        // The fake provider answers every prompt with one picture; give the second draft its own.
        std::fs::write(
            root.join(format!("{second_key}.jpg")),
            b"second draft picture",
        )
        .unwrap();
        let audit = df_observe::AuditContext::default();
        let first = served_bytes(
            respond(
                &portraits,
                &audit,
                &super::super::journey::hex(&first_grant),
                None,
            )
            .await,
        )
        .await;
        let second = served_bytes(
            respond(
                &portraits,
                &audit,
                &super::super::journey::hex(&second_grant),
                None,
            )
            .await,
        )
        .await;
        let cached = |key: String| {
            let imagery = fixture.imagery.clone();
            async move {
                match imagery.image(&key).await {
                    imagery::Lookup::Image { bytes, .. } => bytes.to_vec(),
                    _ => panic!("the draft image is cached"),
                }
            }
        };
        assert_eq!(first, cached(first_key).await);
        assert_eq!(second, cached(second_key).await);
        assert_ne!(first, second);
        assert_eq!(
            respond(
                &portraits,
                &audit,
                &super::super::journey::hex(&[0x33; 32]),
                None,
            )
            .await
            .status(),
            StatusCode::NOT_FOUND
        );
        let _removed = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn an_approved_portrait_stays_public_by_its_content_key() {
        let root = std::env::temp_dir().join(format!("df-approved-by-key-{}", std::process::id()));
        let _removed = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let key = "5".repeat(64);
        let bytes: &[u8] = b"approved portrait fixture";
        std::fs::write(root.join(format!("{key}.jpg")), bytes).unwrap();
        let asset = df_model::checkpoint::AssetReference {
            key: super::super::model::label(&key).unwrap(),
            digest: df_model::checkpoint::ContentDigest(Sha256::digest(bytes).into()),
            byte_length: bytes.len() as u64,
            kind: df_model::checkpoint::AssetKind::Image,
        };
        let mut portraits = Portraits::disabled();
        portraits.cache = root.clone();
        let audit = df_observe::AuditContext::default();
        assert_eq!(
            served_bytes(respond(&portraits, &audit, &key, Some(asset)).await).await,
            bytes
        );
        let _removed = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn the_public_imagery_route_does_not_serve_a_draft_from_the_shared_cache() {
        use axum::body::Body;
        use axum::http::Request;
        use tonic::codegen::Service as _;
        let root =
            std::env::temp_dir().join(format!("df-draft-public-route-{}", std::process::id()));
        let _removed = std::fs::remove_dir_all(&root);
        let creator = super::super::imagery::tests::DirectionFixture::new(&root);
        let mut portraits = Portraits::disabled();
        portraits.imagery = Some(creator.imagery.clone());
        let content_key = ready_draft(&portraits, &creator, "draft-a", [1; 32], [0x11; 32]).await;
        // Both routes get a valid seat grant, so a 404 below means the draft is not public, not a missing grant.
        let seat = [0x22; 32];
        let request = |key: &str| {
            let expiry = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
                + 60;
            let grant = super::super::media_grant::sign(
                key,
                super::super::media_grant::Audience::Imagery,
                super::super::media_grant::PrincipalAudience::Seat,
                &seat,
                expiry,
            )
            .unwrap();
            let bearer: String = seat.iter().map(|byte| format!("{byte:02x}")).collect();
            Request::builder()
                .uri(format!("/gameplay/imagery/{key}?g={grant}"))
                .header(
                    axum::http::header::AUTHORIZATION,
                    format!("Bearer {bearer}"),
                )
                .body(Body::empty())
                .unwrap()
        };
        // The display's own image service reads the same cache directory with its own registry.
        let display = super::super::imagery::tests::DirectionFixture::photographic(&root);
        let mut creator_route = super::super::imagery::router(creator.imagery.clone());
        assert_eq!(
            creator_route
                .call(request(&content_key))
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
        let mut display_route = super::super::imagery::router(display.imagery.clone());
        assert_eq!(
            display_route
                .call(request(&content_key))
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );
        let _removed = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn a_draft_is_served_up_to_the_portrait_bound_and_refused_one_byte_beyond_it() {
        let root = std::env::temp_dir().join(format!("df-draft-bound-{}", std::process::id()));
        let _removed = std::fs::remove_dir_all(&root);
        let fixture = super::super::imagery::tests::DirectionFixture::new(&root);
        let mut portraits = Portraits::disabled();
        portraits.imagery = Some(fixture.imagery.clone());
        let grant = [0x44; 32];
        let key = ready_draft(&portraits, &fixture, "draft-a", [1; 32], grant).await;
        let path = root.join(format!("{key}.jpg"));
        let capability = super::super::journey::hex(&grant);
        let audit = df_observe::AuditContext::default();
        let largest = usize::try_from(MAXIMUM_PORTRAIT_BYTES).unwrap();
        std::fs::write(&path, vec![7_u8; largest]).unwrap();
        assert_eq!(
            served_bytes(respond(&portraits, &audit, &capability, None).await)
                .await
                .len(),
            largest
        );
        std::fs::write(&path, vec![7_u8; largest + 1]).unwrap();
        assert_eq!(
            respond(&portraits, &audit, &capability, None)
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
        let _removed = std::fs::remove_dir_all(&root);
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
