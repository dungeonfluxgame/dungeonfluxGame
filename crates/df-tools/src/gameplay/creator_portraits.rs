//! Explicit creator portraits, owned by the normal run resource worker.
#[path = "creator_portrait_policy.rs"]
mod policy;
use super::{
    creator_classification::{self, Classifier, Route},
    imagery::{self, ImageKind, ImageRequest, Imagery, OutputSpec},
    resources::{
        CreatorPortraitSpec, Generated, Hub, ResourceFailure, ResourceKey, ResourceStatus,
    },
    startup_configuration as config,
};
use axum::{
    Router,
    extract::{Path as RoutePath, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use df_protocol::common as rpc;
pub(super) fn frozen_metadata<T>(uncertain: Option<T>, committed: Option<T>, fresh: T) -> T {
    policy::frozen_metadata(uncertain, committed, fresh)
}
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};
use tonic::{Request, Status};

const PREPARING_PORTRAIT: &str = "Preparing your portrait…";
const CREATING_PORTRAIT: &str = "Creating your portrait…";
const READY_PORTRAIT: &str = "Portrait ready";

#[derive(Clone)]
pub(super) struct Portraits {
    imagery: Option<Imagery>,
    alternate: Option<Imagery>,
    classifier: Option<Arc<Classifier>>,
    requests: Arc<Mutex<BTreeMap<String, Entry>>>,
    cache: std::path::PathBuf,
}
struct Entry {
    fingerprint: Vec<u8>,
    revision: u64,
    resource: ResourceKey,
    binding: [u8; 32],
    appearance: String,
    sequence: usize,
    progress: Arc<Mutex<String>>,
}
impl Portraits {
    #[cfg(test)]
    pub(super) fn disabled() -> Self {
        Self {
            imagery: None,
            alternate: None,
            classifier: None,
            requests: Arc::new(Mutex::new(BTreeMap::new())),
            cache: std::path::PathBuf::from("artifacts/tmp/disabled-creator-portraits"),
        }
    }

    pub(super) fn start(hub: Hub, current: &Path) -> Result<Self, io::Error> {
        let enabled = match crate::gameplay::media_policy::gates()
            .input(crate::gameplay::media_policy::Capability::CreatorPortraits)
            .ok()
            .as_deref()
        {
            None | Some("" | "off") => false,
            Some("on") => true,
            Some(_) => {
                return Err(io::Error::other(
                    "DF_GAMEPLAY_CREATOR_PORTRAITS must be on or off",
                ));
            }
        };
        let mut alternate = None;
        let mut classifier = None;
        let imagery = if enabled {
            let mut inputs = config::ImageryInputs::from_environment();
            inputs.credential = std::env::var("DF_GAMEPLAY_CREATOR_OPENAI_KEY");
            inputs.mode = Ok("on".into());
            inputs.quality = Ok("medium".into());
            inputs.deadline_seconds = Ok("170".into());
            let config::ImagerySettings::On(mut settings) =
                config::imagery_settings(inputs, current, config::content_rating())?
            else {
                return Err(io::Error::other(
                    "Creator portraits require an OpenAI credential",
                ));
            };
            settings.deadline = Duration::from_secs(180);
            let jev = config::approach_settings(
                std::env::var("DF_GAMEPLAY_CREATOR_JEV_API_KEY"),
                std::env::var("DF_GAMEPLAY_CREATOR_JEV_BASE_URL"),
            )?
            .ok_or_else(|| {
                io::Error::other("Creator portraits require the dedicated Jev credential")
            })?;
            classifier = Some(Arc::new(Classifier::new(
                &jev.credential,
                &jev.base_url,
                Arc::clone(hub.ledger()),
            )?));
            let provider = df_providers::NativeImageEditProvider::new(
                df_providers::ImageEditConfig::live(),
                settings.credential.expose(),
                settings.deadline,
            )
            .map_err(|_| io::Error::other("Creator portrait provider configuration refused"))?;
            let manifest = current
                .join("artifacts/tmp/creator-portraits")
                .join(format!("run-{}", std::process::id()));
            std::fs::create_dir_all(&manifest)?;
            if let Ok(key) = std::env::var("DF_GAMEPLAY_CREATOR_BFL_KEY") {
                if !key.is_empty() {
                    let provider = df_providers::NativeBflImageProvider::new(
                        df_providers::BflImageConfig::live(),
                        &key,
                        settings.deadline,
                    )
                    .map_err(|_| {
                        io::Error::other("Alternate portrait provider configuration refused")
                    })?;
                    let mut inputs = config::ImageryInputs::from_environment();
                    inputs.credential = std::env::var("DF_GAMEPLAY_CREATOR_OPENAI_KEY");
                    inputs.mode = Ok("on".into());
                    inputs.quality = Ok("medium".into());
                    inputs.deadline_seconds = Ok("170".into());
                    let config::ImagerySettings::On(mut alternate_settings) =
                        config::imagery_settings(inputs, current, config::content_rating())?
                    else {
                        return Err(io::Error::other("Alternate portrait configuration refused"));
                    };
                    alternate_settings.deadline = settings.deadline;
                    alternate = Some(Imagery::new_in_hub(
                        alternate_settings,
                        imagery::ImageRoute::Bfl(provider),
                        Some(&manifest),
                        hub.clone(),
                        true,
                    )?);
                }
            }
            hub.set_limits(
                super::resources::ResourceKind::CreatorPortrait,
                super::resources::Limits {
                    timeout: Duration::from_secs(210),
                    attempts: 1,
                },
            );
            let imagery = Imagery::new_in_hub(
                settings,
                imagery::ImageRoute::OpenAi(provider),
                Some(&manifest),
                hub.clone(),
                true,
            )?;
            hub.set_limits(
                super::resources::ResourceKind::CreatorPortrait,
                super::resources::Limits {
                    timeout: Duration::from_secs(210),
                    attempts: 1,
                },
            );
            Some(imagery)
        } else {
            None
        };
        let cache = std::env::var("DF_GAMEPLAY_IMAGERY_CACHE")
            .ok()
            .filter(|value| !value.is_empty())
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| current.join("artifacts/tmp/play/imagery-cache"));
        Ok(Self {
            cache: if cache.is_absolute() {
                cache
            } else {
                current.join(cache)
            },
            imagery,
            alternate,
            classifier,
            requests: Arc::new(Mutex::new(BTreeMap::new())),
        })
    }
}

#[derive(Clone, Debug)]
pub(super) struct Attachment {
    pub(super) key: String,
    pub(super) digest: [u8; 32],
    pub(super) bytes: u64,
    pub(super) revision: u64,
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
        let identity = super::hero_identity::Identity::from_rpc(identity)
            .map_err(|_| Status::invalid_argument("Review your public appearance"))?;
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
        .map_err(|_| Status::invalid_argument("Review your character design"))?;
        let (public, _) = super::creator_catalog::with_scenery(
            public,
            String::new(),
            &character.portrait_scenery,
        )
        .map_err(|_| Status::invalid_argument("Review your public portrait scenery"))?;
        let candidate = {
            let requests = self
                .requests
                .lock()
                .map_err(|_| Status::unavailable("Portrait status unavailable"))?;
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
                                Some((generated.handle, entry.revision))
                            }
                            _ => None,
                        }
                    })
                })
        };
        let Some((key, revision)) = candidate else {
            return Ok(None);
        };
        match self.image(&key).await {
            imagery::Lookup::Image { bytes, .. }
                if !bytes.is_empty() && bytes.len() <= 6 * 1024 * 1024 =>
            {
                Ok(Some(Attachment {
                    key,
                    digest: Sha256::digest(&bytes).into(),
                    bytes: bytes.len() as u64,
                    revision,
                }))
            }
            // A vanished cache must not prevent the actor from resolving an exact retry.
            _ => Ok(None),
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
        || asset.byte_length > 6 * 1024 * 1024
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
        file.take(6 * 1024 * 1024 + 1)
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
    let fingerprint = Sha256::digest(format!(
        "{}|{}|{}",
        body.species, body.draft_revision, public
    ))
    .to_vec();
    Ok(PreparedPortrait {
        identity,
        public,
        design,
        fingerprint,
    })
}

pub(super) async fn handle(
    service: &super::Service,
    request: Request<rpc::HeroPortraitRequest>,
    dispatch: bool,
) -> Result<rpc::HeroPortraitResponse, Status> {
    let binding = super::credential(&request)?;
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
    super::hero_identity::check_preview_scope(service, binding, &scope, &expected).await?;
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
    let imagery = portraits.imagery.as_ref().ok_or_else(|| {
        Status::failed_precondition(
            "Portrait generation is unavailable. Ask the host to check setup",
        )
    })?;
    let mut requests = portraits
        .requests
        .lock()
        .map_err(|_| Status::unavailable("Portrait status unavailable"))?;
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
            let operation = operation_key.clone();
            let primary = imagery.clone();
            let alternate = portraits.alternate.clone();
            // A frozen operation owns one classifier attempt and exactly one selected image route.
            let versioned_input = format!(
                "{}|{}|{}|{}",
                creator_classification::VERSION,
                operation_key,
                public,
                alternate.is_some()
            );
            let spec = CreatorPortraitSpec::new(
                &format!("creator:{operation_key}"),
                &versioned_input,
                df_render::art_contract::Neutral::Nothing,
            );
            let resource = ResourceKey::of(&spec);
            let progress = Arc::new(Mutex::new(PREPARING_PORTRAIT.to_owned()));
            let job_progress = progress.clone();
            imagery.hub().get_or_request(
                &spec,
                Arc::new(move || {
                    let (classifier, source, operation, primary, alternate, request, general_request) = (
                        classifier.clone(),
                        source.clone(),
                        operation.clone(),
                        primary.clone(),
                        alternate.clone(),
                        request.clone(),
                        general_request.clone(),
                    );
                    let progress = job_progress.clone();
                    Box::pin(async move {
                        let assessment = classifier
                            .classify(&source, &operation)
                            .await
                            .map_err(|_| ResourceFailure::Permanent("classifier_unavailable"))?;
                        let route = assessment
                            .route_for_rating(
                                config::content_rating() == config::ContentRating::Mature,
                            )
                            .map_err(|_| {
                                ResourceFailure::Permanent("rating_disallows_appearance")
                            })?;
                        let (chosen, provider, request) = match route {
                            Route::OpenAi => (primary, "OpenAI", general_request),
                            Route::Bfl => (
                                alternate
                                    .ok_or(ResourceFailure::Permanent("alternate_unavailable"))?,
                                "BFL FLUX 3",
                                if assessment.requires_mature_prompt() { request } else { general_request },
                            ),
                            Route::Revise => {
                                return Err(ResourceFailure::Permanent(
                                    "content_requires_revision",
                                ));
                            }
                            Route::Uncertain => return Err(ResourceFailure::Permanent("classification_uncertain")),
                        };
                        let appearance_hash = format!("{:x}",Sha256::digest(source.as_bytes()));
                        let model = chosen.model_label();
                        let request_hash = format!("{:x}",Sha256::digest(request.prompt.as_str().as_bytes()));
                        super::actor::record_audit_diagnostic("gameplay.creator_portrait_route",
                            format!("CREATOR_ROUTE classifier=jev-latest category={} provider={provider} model={model} operation={operation} appearance_hash={appearance_hash} request_hash={request_hash}",assessment.category()),
                            serde_json::Map::from_iter([
                                ("request_hash".into(),serde_json::json!(request_hash)),("classifier".into(),serde_json::json!("jev-latest")),("category".into(),serde_json::json!(assessment.category())),("provider".into(),serde_json::json!(provider)),("model".into(),serde_json::json!(model)),("operation".into(),serde_json::json!(operation)),("appearance_hash".into(),serde_json::json!(appearance_hash))
                            ]),&[]);
                        if let Ok(mut message) = progress.lock() {
                            *message = CREATING_PORTRAIT.to_owned();
                        }
                        let key = chosen
                            .render_creator(request)
                            .await
                            .map_err(|failure| ResourceFailure::Permanent(failure.label()))?;
                        let mut generated = Generated::new(key.as_str());
                        generated.note =
                            READY_PORTRAIT.to_owned();
                        Ok(generated)
                    })
                }),
            );
            let sequence = requests.len();
            requests.insert(
                operation_key.clone(),
                Entry {
                    fingerprint,
                    binding,
                    appearance: public,
                    sequence,
                    progress,
                    revision: body.draft_revision,
                    resource,
                },
            );
        }
    }
    let entry = &requests[&operation_key];
    let (state, uri, message) = match imagery.hub().status(&entry.resource) {
        ResourceStatus::Ready(generated) => (
            "ready",
            format!("/gameplay/creator-portrait/{}", generated.handle),
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
    Ok(rpc::HeroPortraitResponse {
        draft_revision: entry.revision,
        state: state.into(),
        image_uri: uri,
        message,
    })
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
) -> Response {
    if key.len() != 64
        || !key
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    let accepted = service
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
    let Some(asset) = accepted else {
        if let imagery::Lookup::Image { bytes, format: _ } = service.portraits.image(&key).await {
            return (
                [
                    (header::CONTENT_TYPE, "image/jpeg"),
                    (header::CACHE_CONTROL, "private, max-age=86400, immutable"),
                ],
                bytes,
            )
                .into_response();
        }
        return StatusCode::NOT_FOUND.into_response();
    };
    if asset.byte_length > 6 * 1024 * 1024 {
        return StatusCode::NOT_FOUND.into_response();
    }
    let path = service.portraits.cache.join(format!("{key}.jpg"));
    match tokio::task::spawn_blocking(move || {
        use std::io::Read;
        let file = std::fs::File::open(path)?;
        if file.metadata()?.len() != asset.byte_length {
            return Err(std::io::Error::other("Portrait cache size mismatch"));
        }
        let mut bytes = Vec::new();
        file.take(6 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
        Ok::<_, std::io::Error>(bytes)
    })
    .await
    {
        Ok(Ok(bytes))
            if bytes.len() as u64 == asset.byte_length
                && <[u8; 32]>::from(Sha256::digest(&bytes)) == asset.digest.0 =>
        {
            (
                [
                    (header::CONTENT_TYPE, "image/jpeg"),
                    (header::CACHE_CONTROL, "private, max-age=86400, immutable"),
                ],
                bytes,
            )
                .into_response()
        }
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

#[cfg(test)]
mod catalog_tests {
    use super::*;

    #[tokio::test]
    async fn approved_reference_checks_digest_length_key_and_kind_before_use() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../artifacts/tmp/lanes/HS5/approved-reference");
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
}
