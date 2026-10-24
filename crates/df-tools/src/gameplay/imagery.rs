//! The runtime image service: server-generated art for the shared display.
//!
//! One typed request ([`ImageRequest`]) serves every caller: a re-framed scene backdrop, a
//! character's changed expression, and a close-up of an object. A request names its reference
//! images (the base backdrop and/or character art from the served asset set), a prompt built
//! only from the server's closed vocabulary (see `prompt`), and the output size. The result is
//! an image the display loads at `GET /gameplay/imagery/<key>`, where the key is the SHA-256 of
//! the kind, the references' hashes, the prompt, the model and the parameters.
//!
//! Like the voice lines, only registered keys are served: a key the server never registered is
//! a 404. Results live in an on-disk cache keyed the same way, so a request never costs twice,
//! not even across runs. There is no spend cap: every provider call is recorded in the hub's
//! spend ledger (a call that was sent is never refunded); separate pools reserve display capacity;
//! identical requests share one flight. Every
//! failure is typed and means "keep showing what is on screen": nothing in the game waits for
//! an image, and a provider refusal is a failure, never retried with another wording or another
//! model. Generated images are never written under `assets/` and never committed; each is
//! recorded in a run-local manifest (`imagery.jsonl`) with its provenance.
mod artifact;
mod canonical_prompt;
mod cast_sheet;
pub(in crate::gameplay) mod figure_masks;
mod mural_prompt;
pub(super) mod narrative_focus;
mod party_sheet;
mod photo_check;
mod photo_edit;
mod plate_prompt;
mod prompt;
mod references;
mod scene_prompt;
#[cfg(test)]
pub(super) use canonical_prompt::canonical_room;
pub(super) use canonical_prompt::{
    RoomArchitectureContext, canonical_cast_member, canonical_character, canonical_npc,
    canonical_room_content, canonical_room_with_context,
};
pub(super) use scene_prompt::{graph_edit, item_picture, model_plate, state_edit, story_beat_edit};

use std::{
    collections::{HashMap, VecDeque},
    io,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use axum::{
    Router,
    extract::{Path as RoutePath, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use bytes::Bytes;
use df_observe::OperationContext;
use df_providers::{
    HeadLocateConfig, ImageBackground, ImageEdit, ImageEditConfig, ImageEditError, ImageEditModel,
    ImageFormat, ImageModeration, ImageQuality, ImageUsage, LocateTarget, LocatedHead,
    NativeHeadLocator, NativeHttpRefusal, NativeImageEditProvider, NativePlateJudge,
    PlateJudgeConfig, ReferenceImage as ProviderReference,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::{Semaphore, watch};

#[allow(
    unused_imports,
    reason = "the investigation task (a follow-up) is the first caller"
)]
pub(super) use artifact::{ArtifactSubject, SubjectRefusal, close_up_request};
#[allow(
    unused_imports,
    reason = "T02 stage rendering must pass each checked hero identity into these prompts"
)]
pub(super) use prompt::{
    AssembledPrompt, CharacterFacts, CharacterRole, FILM_LOOK, HeroVisualIdentity, LifeStage,
    PlaceFacts, StagedPerson, cast_expression, creator_portrait, hero_cutout, hero_expression,
    hero_portrait, scene_refresh,
};
#[allow(
    unused_imports,
    reason = "the investigation task (a follow-up) is the first caller"
)]
pub(super) use prompt::{PromptRefusal, VettedFact};
// PLAY-V09: the scene plate's party sheet and prompts.
pub(super) use cast_sheet::{CastIdentity, cast_sheet};
pub(super) use mural_prompt::party_mural;
use party_sheet::SheetError;
pub(super) use party_sheet::{fit_reference, party_sheet, vision_picture};
pub(super) use photo_edit::harbor_photo_edit;
#[cfg(test)]
pub(super) use plate_prompt::{FLUX_MAXIMUM_PROMPT_CHARACTERS, flux_scene_plate};
pub(super) use plate_prompt::{
    FluxCast, FluxPerson, PlateReferences, SceneParty, flux_scene_plate_for_party,
    hero_locator_phrase, plate_moment, scene_plate,
};
#[allow(
    unused_imports,
    reason = "T02 stage rendering must pass each checked hero identity into these prompts"
)]
pub(super) use plate_prompt::{hero_locator_phrase_for_identity, scene_plate_for_heroes};
pub(super) use references::{ReferenceImage, ReferenceStore};

use super::startup_configuration::{
    ContentRating, ImageryConfiguration, ImageryOff, ImagerySettings,
};

/// Words for a medium other than photography. No image or video prompt names one, even to forbid
/// it: naming another medium steers the model toward it (owner, 2026-10-17).
pub(super) const NON_PHOTOGRAPHIC_WORDS: &[&str] = &[
    "painting",
    "painted",
    "painterly",
    "illustration",
    "illustrated",
    "illustrative",
    "drawing",
    "sketch",
    "cartoon",
    "anime",
    "CGI",
    "3D render",
    "concept art",
    "game art",
    "matte painting",
    "key art",
    "artwork",
];

const ROUTE: &str = "/gameplay/imagery";
const MANIFEST_NAME: &str = "imagery.jsonl";
/// Most recent registered keys kept; older ones can no longer be requested.
const MAXIMUM_REGISTERED: usize = 512;
const MAXIMUM_IMAGE_BYTES: u64 = 6 * 1024 * 1024;
/// Current-place canonical work has its own OpenAI/FLUX image-call allocation.
pub(super) const MAXIMUM_IN_FLIGHT: usize = 3;
/// Reserved OpenAI/FLUX image-call allocation for visible plates and photo checks.
const FOREGROUND_IN_FLIGHT: usize = 3;
/// OpenAI/FLUX background image-call allocation; supplier HTTP 429 enforces account rate limits.
pub(in crate::gameplay) const BACKGROUND_IN_FLIGHT: usize = 6;
/// Independent OpenAI vision-call allocation: head measurements never queue behind image edits.
const HEADS_IN_FLIGHT: usize = 2;
const RATE_LIMIT_BACKOFF: Duration = Duration::from_secs(1);
const MAXIMUM_RATE_LIMIT_RETRIES: u32 = 3;
const MAXIMUM_MANIFEST_BYTES: u64 = 4 * 1024 * 1024;
/// How long one head-location call may take.
const HEAD_LOCATE_DEADLINE: Duration = Duration::from_secs(25);
const CACHE_CONTROL: &str = "private, max-age=86400, immutable";

#[derive(Clone, Copy, Debug)]
pub(in crate::gameplay) enum ImagePriority {
    Foreground,
    Presence,
    Background,
}

impl ImagePriority {
    fn for_kind(kind: ImageKind) -> Self {
        match kind {
            ImageKind::ScenePlate | ImageKind::ScenePlateEdit | ImageKind::SceneRefresh => {
                Self::Foreground
            }
            ImageKind::CanonicalCharacter
            | ImageKind::CanonicalRoom
            | ImageKind::CastExpression => Self::Presence,
            ImageKind::ArtifactCloseUp => Self::Background,
        }
    }
}

/// What a result is for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ImageKind {
    CanonicalCharacter,
    CanonicalRoom,
    /// The current backdrop re-framed and re-lit; no people.
    SceneRefresh,
    /// One character's cut-out with another expression; transparent.
    CastExpression,
    /// A close-up photograph of one object in this place.
    ArtifactCloseUp,
    /// PLAY-V09: one composed film still of a story scene, heroes and speaker in the place.
    ScenePlate,
    /// PLAY-V09: an existing plate edited for a lasting change of the speaker's face.
    ScenePlateEdit,
}

impl ImageKind {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::CanonicalCharacter => "canonical_character",
            Self::CanonicalRoom => "canonical_room",
            Self::SceneRefresh => "scene_refresh",
            Self::CastExpression => "cast_expression",
            Self::ArtifactCloseUp => "artifact_close_up",
            Self::ScenePlate => "scene_plate",
            Self::ScenePlateEdit => "scene_plate_edit",
        }
    }
}

/// The quality and size a kind is drawn at. Dev play-tests draw plates at backdrop size and medium
/// quality, rooms and close-ups at low quality, and people at medium so identity references hold;
/// production draws everything at the configured quality and full size.
fn drawn_for(
    profile: super::startup_configuration::ImageryProfile,
    configured: ImageQuality,
    kind: ImageKind,
    output: OutputSpec,
) -> (ImageQuality, OutputSpec) {
    use super::startup_configuration::ImageryProfile;
    match profile {
        ImageryProfile::Production => (configured, output),
        ImageryProfile::Dev => match kind {
            ImageKind::ScenePlate | ImageKind::ScenePlateEdit => (
                ImageQuality::Medium,
                if output == OutputSpec::PLATE {
                    OutputSpec::BACKDROP
                } else {
                    output
                },
            ),
            ImageKind::CanonicalRoom | ImageKind::ArtifactCloseUp | ImageKind::SceneRefresh => {
                (ImageQuality::Low, output)
            }
            ImageKind::CanonicalCharacter | ImageKind::CastExpression => {
                (ImageQuality::Medium, output)
            }
        },
    }
}

#[cfg(test)]
mod drawn_tests {
    use super::*;
    use crate::gameplay::startup_configuration::ImageryProfile;

    #[test]
    fn the_dev_profile_draws_cheaper_plates_rooms_and_close_ups_and_keeps_people() {
        let dev = |kind, output| drawn_for(ImageryProfile::Dev, ImageQuality::High, kind, output);
        assert_eq!(
            dev(ImageKind::ScenePlate, OutputSpec::PLATE),
            (ImageQuality::Medium, OutputSpec::BACKDROP)
        );
        assert_eq!(
            dev(ImageKind::ScenePlateEdit, OutputSpec::PLATE),
            (ImageQuality::Medium, OutputSpec::BACKDROP)
        );
        assert_eq!(
            dev(ImageKind::CanonicalRoom, OutputSpec::BACKDROP),
            (ImageQuality::Low, OutputSpec::BACKDROP)
        );
        assert_eq!(
            dev(ImageKind::ArtifactCloseUp, OutputSpec::CLOSE_UP),
            (ImageQuality::Low, OutputSpec::CLOSE_UP)
        );
        assert_eq!(
            dev(ImageKind::CanonicalCharacter, OutputSpec::CUTOUT),
            (ImageQuality::Medium, OutputSpec::CUTOUT)
        );
        for kind in [ImageKind::ScenePlate, ImageKind::CanonicalRoom, ImageKind::ArtifactCloseUp] {
            assert_eq!(
                drawn_for(ImageryProfile::Production, ImageQuality::High, kind, OutputSpec::PLATE),
                (ImageQuality::High, OutputSpec::PLATE),
                "production draws every kind as configured"
            );
        }
    }
}

/// Output size and transparency of a result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct OutputSpec {
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) transparent: bool,
}

impl OutputSpec {
    /// A 16:9 backdrop.
    pub(super) const BACKDROP: Self = Self {
        width: 1536,
        height: 864,
        transparent: false,
    };
    /// A 2:3 transparent cut-out, the aspect of the generated sprites.
    pub(super) const CUTOUT: Self = Self {
        width: 1024,
        height: 1536,
        transparent: true,
    };
    /// PLAY-V09: a 16:9 composed scene plate (heroes are small in it, so it is drawn large).
    pub(super) const PLATE: Self = Self {
        width: 2560,
        height: 1440,
        transparent: false,
    };
    /// A 4:3 photographic close-up.
    pub(super) const CLOSE_UP: Self = Self {
        width: 1024,
        height: 768,
        transparent: false,
    };

    /// A cut-out is a WebP with alpha (a PNG of the same picture is several times larger); a
    /// backdrop or close-up is a JPEG.
    fn format(self) -> ImageFormat {
        if self.transparent {
            ImageFormat::Webp
        } else {
            ImageFormat::Jpeg
        }
    }
}

/// One request to the image service.
#[derive(Clone)]
pub(super) struct ImageRequest {
    pub(super) kind: ImageKind,
    pub(super) references: Vec<ReferenceImage>,
    pub(super) prompt: AssembledPrompt,
    pub(super) output: OutputSpec,
}

/// Cards depict the object itself. Custody, hero instance IDs and the current plate are not
/// visual inputs, so identical catalog gear shares one flight and a persistent cache entry.
pub(in crate::gameplay) fn item_image_request(
    rating: ContentRating,
    thing: &super::adventure::scene_model::SceneThing,
    observed_detail: Option<&str>,
) -> Result<ImageRequest, ImageFailure> {
    // StartingPossession serializes custody into its picture description. Remove only that
    // exact server-authored suffix; preserve custom descriptions and visible alterations.
    let mut visual = thing.clone();
    if let Some(owner) = &thing.owner_id {
        let custody = format!(", {} by {owner}; not in hand", thing.position);
        if let Some(description) = thing.picture_description.strip_suffix(&custody) {
            visual.picture_description = description.to_owned();
        }
    }
    let prompt = item_picture(rating, &visual, observed_detail)?.with_canonical_binding(&format!(
        "Object source identity: {}.",
        thing.provenance.source_id,
    ));
    Ok(ImageRequest {
        kind: ImageKind::ArtifactCloseUp,
        references: Vec::new(),
        prompt,
        output: OutputSpec::CLOSE_UP,
    })
}

/// The identity of a result: lowercase hexadecimal SHA-256, safe as one URL path segment.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) struct ImageKey(String);

impl ImageKey {
    /// `Some` only for exactly 64 lowercase hexadecimal digits.
    pub(super) fn parse(text: &str) -> Option<Self> {
        (text.len() == 64
            && text
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f')))
        .then(|| Self(text.to_owned()))
    }

    pub(super) fn as_str(&self) -> &str {
        &self.0
    }

    /// The path the display loads the image from.
    pub(super) fn path(&self) -> String {
        format!("{ROUTE}/{}", self.0)
    }
}

/// Why a request produced no image. Every variant means "keep what is on screen".
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ImageFailure {
    /// Imagery is off.
    Off,
    /// The request failed local validation; nothing was charged or sent.
    InvalidRequest,
    DeadlineExceeded,
    /// The provider's moderation declined it. It is never reworded or rerouted.
    Refused,
    /// The provider failed, rejected the credential or was rate limited.
    Provider,
    RateLimited,
    BflSubmit,
    BflPoll,
    BflDelivery,
    BflDownload,
    /// The provider answered with an image that is not what was asked for.
    BadOutput,
    /// The flight ended without reporting.
    Interrupted,
}

impl ImageFailure {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::InvalidRequest => "invalid_request",
            Self::DeadlineExceeded => "deadline_exceeded",
            Self::Refused => "refused",
            Self::Provider => "provider_failed",
            Self::RateLimited => "rate_limited",
            Self::BflSubmit => "bfl_submit_failed",
            Self::BflPoll => "bfl_poll_failed",
            Self::BflDelivery => "bfl_delivery_failed",
            Self::BflDownload => "bfl_download_failed",
            Self::BadOutput => "bad_output",
            Self::Interrupted => "interrupted",
        }
    }

    /// The failure a hub failure label names (a failure the hub remembered is served without
    /// running the request again, so the original is gone and only its label remains).
    fn from_label(label: &str) -> Option<Self> {
        [
            Self::Off,
            Self::InvalidRequest,
            Self::DeadlineExceeded,
            Self::Refused,
            Self::Provider,
            Self::RateLimited,
            Self::BflSubmit,
            Self::BflPoll,
            Self::BflDelivery,
            Self::BflDownload,
            Self::BadOutput,
            Self::Interrupted,
        ]
        .into_iter()
        .find(|failure| failure.label() == label)
    }

    /// A failure that the same request would repeat: the hub remembers it as failed (until it is
    /// invalidated or regenerated) and it is never paid for twice.
    fn is_permanent(self) -> bool {
        matches!(self, Self::Refused | Self::BadOutput | Self::InvalidRequest)
    }
}

type Outcome = Result<(), ImageFailure>;

#[derive(Clone, Copy, Default)]
struct RequestTiming {
    wait: Duration,
    provider: Duration,
}

#[derive(Clone, Copy)]
struct FlightResult {
    outcome: Outcome,
    timing: RequestTiming,
}

/// The result of asking for the bytes of one key.
#[derive(Debug, Eq, PartialEq)]
pub(super) enum Lookup {
    Image {
        bytes: Bytes,
        format: ImageFormat,
    },
    /// The server never registered this key.
    Unregistered,
    /// Registered, but no image exists (yet).
    NotReady,
}

#[derive(Clone, Copy, Debug)]
struct Registered {
    format: ImageFormat,
}

#[derive(Default)]
struct Registry {
    order: VecDeque<String>,
    by_key: HashMap<String, Registered>,
}

impl Registry {
    fn register(&mut self, key: &ImageKey, registered: Registered) {
        if self.by_key.contains_key(key.as_str()) {
            self.order.retain(|known| known != key.as_str());
        } else if self.order.len() >= MAXIMUM_REGISTERED
            && let Some(oldest) = self.order.pop_front()
        {
            self.by_key.remove(&oldest);
        }
        self.order.push_back(key.0.clone());
        self.by_key.insert(key.0.clone(), registered);
    }
}

/// Image calls are recorded in the hub's spend ledger (`Unit::ImageCall`); nothing caps them.
const IMAGE_UNIT: super::resources::Unit = super::resources::Unit::ImageCall;

/// The resource kinds an image request can be.
const IMAGE_KINDS: [super::resources::ResourceKind; 4] = [
    super::resources::ResourceKind::Depiction,
    super::resources::ResourceKind::ScenePlate,
    super::resources::ResourceKind::PlateEdit,
    super::resources::ResourceKind::ItemCloseUp,
];

/// Counters for tests; they contain no content.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct Stats {
    /// Provider calls charged.
    pub(super) calls: u32,
    pub(super) cache_hits: u32,
    /// Cost of the calls that reported usage, in millionths of a US dollar.
    pub(super) micro_dollars: u64,
}

struct Engine {
    provider: ImageRoute,
    model: ImageEditModel,
    quality: ImageQuality,
    profile: super::startup_configuration::ImageryProfile,
    rating: ContentRating,
    deadline: Duration,
    cache_dir: PathBuf,
    manifest: Option<PathBuf>,
    ledger: Arc<super::resources::SpendLedger>,
    calls: AtomicU32,
    cache_hits: AtomicU32,
    micro_dollars: AtomicU64,
    presence_permits: Semaphore,
    permits: Semaphore,
    background_permits: Semaphore,
    head_permits: Semaphore,
    in_flight: Mutex<HashMap<String, watch::Receiver<Option<FlightResult>>>>,
    /// PLAY-V09: finds the speaker's head in a finished plate; absent in tests and when the
    /// production provider could not be built.
    locator: std::sync::OnceLock<NativeHeadLocator>,
    figure_masks: figure_masks::FigureMaskCache,
    /// ART-01: judges each new plate and empty room before it is stored; absent in tests and
    /// when the production judge could not be built.
    judge: std::sync::OnceLock<NativePlateJudge>,
}

/// Bounds one photo-quality judgement of a finished image.
const PHOTO_CHECK_DEADLINE: Duration = Duration::from_secs(30);
/// The edit adapter's limit on one reference image.
const REFERENCE_BYTES_LIMIT: usize = 4 * 1024 * 1024;

enum Mode {
    Off(ImageryOff),
    On {
        engine: Arc<Engine>,
        summary: String,
    },
}

struct Inner {
    registry: Mutex<Registry>,
    mode: Mode,
    /// The resource generation hub: the one place a generated image is asked for and remembered.
    hub: super::resources::Hub,
}

/// Accepted photographic evidence, retained beside its immutable image key across restarts.
#[derive(serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(in crate::gameplay) struct PhotoEvidence {
    pub(in crate::gameplay) place: String,
    pub(in crate::gameplay) source_hash: String,
    pub(in crate::gameplay) visible_features: Vec<super::journey::living_scene::VisibleFeature>,
}

/// Registry of renderable results plus, when enabled, the cached provider calls.
#[derive(Clone)]
pub(super) struct Imagery {
    inner: Arc<Inner>,
}

/// The guarded data are plain collections and counters that stay consistent when a holder
/// panics, so a poisoned lock is still safe to use.
fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Serialize, Deserialize)]
struct ManifestLine {
    key: String,
    kind: String,
    model: String,
    prompt: String,
    references: Vec<String>,
    created_unix: u64,
    bytes: u64,
    width: u32,
    height: u32,
    transparent: bool,
    micro_dollars: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    supplier_credits: Option<String>,
    /// PLAY-V09: how long the provider call took, in milliseconds; absent in older manifests.
    #[serde(default)]
    elapsed_ms: u64,
}

/// OpenAI is the default image route. FLUX remains an explicit development opt-in;
/// merely configuring a fal credential never changes the selected image provider.
pub(super) enum ImageRoute {
    OpenAi(NativeImageEditProvider),
    Flux(df_providers::NativeFalImageProvider),
    Bfl(df_providers::NativeBflImageProvider),
}

impl From<NativeImageEditProvider> for ImageRoute {
    fn from(provider: NativeImageEditProvider) -> Self {
        Self::OpenAi(provider)
    }
}

impl ImageRoute {
    /// `DF_GAMEPLAY_IMAGERY_PROVIDER` is `flux` (needs `FAL_KEY`), `openai`, or unset.
    /// Unset and empty choose OpenAI; the imagery configuration admits its credential first.
    fn configured(configuration: &ImageryConfiguration) -> Result<Self, io::Error> {
        let fal_key = std::env::var("FAL_KEY").ok().filter(|key| !key.is_empty());
        let flux = match super::media_policy::image_provider(
            std::env::var("DF_GAMEPLAY_IMAGERY_PROVIDER"),
            fal_key.is_some(),
        )? {
            super::media_policy::ImageProvider::OpenAi => None,
            super::media_policy::ImageProvider::Flux => fal_key,
        };
        match flux {
            Some(key) => df_providers::NativeFalImageProvider::new(
                df_providers::FalImageConfig::live(),
                &key,
                configuration.deadline,
            )
            .map(Self::Flux)
            .map_err(|_| io::Error::other("imagery provider configuration refused")),
            None => NativeImageEditProvider::new(
                ImageEditConfig::live(),
                configuration.credential.expose(),
                configuration.deadline,
            )
            .map(Self::OpenAi)
            .map_err(|_| io::Error::other("imagery provider configuration refused")),
        }
    }

    fn label(&self, model: ImageEditModel) -> &'static str {
        match self {
            Self::OpenAi(_) => model.model_id(),
            Self::Flux(_) => "flux-1.1-ultra-raw+kontext-max",
            Self::Bfl(_) => "flux-3-image:latest:1.5k:grounding-off:safety-2",
        }
    }

    async fn edit(
        &self,
        edit: &ImageEdit<'_>,
        deadline: Duration,
    ) -> Result<(df_providers::GeneratedImage, Option<String>), ImageFailure> {
        match self {
            Self::OpenAi(provider) => provider
                .edit(edit, deadline)
                .await
                .map(|image| (image, None))
                .map_err(failure_of),
            Self::Flux(provider) => provider
                .edit(edit, deadline)
                .await
                .map(|image| (image, None))
                .map_err(|error| match error {
                    // This adapter erases submit/poll stage. Repeating a possibly accepted
                    // paid job is unsafe; fal must retry 429 at its HTTP exchange boundary.
                    ImageEditError::Provider(429) => ImageFailure::Provider,
                    error => failure_of(error),
                }),
            Self::Bfl(provider) => {
                provider
                    .edit_diagnosed(edit, deadline)
                    .await
                    .map_err(|failure| {
                        let (error_class, http_status) = match failure.error {
                    ImageEditError::InvalidRequest => ("invalid_request", None),
                    ImageEditError::Refused => ("moderation_refused", None),
                    ImageEditError::Unauthorized => ("unauthorized", None),
                    ImageEditError::Provider(status) => ("http_status", Some(status)),
                    ImageEditError::Transport(_) => ("transport", None),
                    ImageEditError::Output => ("output_contract", None),
                };
                let http_status = failure.http_status.or(http_status);
                let request_hash = format!("{:x}",Sha256::digest(edit.prompt.as_bytes()));
                super::actor::record_audit_diagnostic(
                    "gameplay.creator_bfl_boundary",
                    format!("BFL_BOUNDARY provider=BFL stage={} error_class={error_class} error={:?} http_status={http_status:?} delivery_host={:?} admission_reason={:?} request_hash={request_hash}",failure.stage,failure.error,failure.delivery_host,failure.admission_reason),
                    serde_json::Map::from_iter([
                        ("provider".into(),serde_json::json!("BFL")),
                        ("delivery_host".into(),serde_json::json!(failure.delivery_host)),
                        ("admission_reason".into(),serde_json::json!(failure.admission_reason)),
                        ("stage".into(),serde_json::json!(failure.stage)),
                        ("error_class".into(),serde_json::json!(error_class)),
                        ("http_status".into(),serde_json::json!(http_status)),
                        ("request_hash".into(),serde_json::json!(request_hash)),
                        ("error".into(),serde_json::json!(format!("{:?}",failure.error))),
                    ]), &[]);
                match failure.error {
                            // Only a rejected submit is safe to resubmit. A poll failure may
                            // already own a paid job and must retain its existing job identity.
                            ImageEditError::Provider(429)
                                if matches!(failure.stage, "submit" | "submit_response") =>
                            {
                                ImageFailure::RateLimited
                            }
                            ImageEditError::Refused
                            | ImageEditError::Transport(
                                NativeHttpRefusal::Deadline | NativeHttpRefusal::Cancelled,
                            ) => failure_of(failure.error),
                            _ => match failure.stage {
                                "submit" | "submit_response" | "validation" => {
                                    ImageFailure::BflSubmit
                                }
                                "poll" | "poll_response" => ImageFailure::BflPoll,
                                "delivery_admission" => ImageFailure::BflDelivery,
                                "download" => ImageFailure::BflDownload,
                                _ => ImageFailure::BadOutput,
                            },
                        }
                    })
            }
        }
    }
}

impl Imagery {
    /// An image service that registers nothing and never renders.
    pub(super) fn disabled(reason: ImageryOff) -> Self {
        Self {
            inner: Arc::new(Inner {
                registry: Mutex::new(Registry::default()),
                mode: Mode::Off(reason),
                hub: super::resources::Hub::new(),
            }),
        }
    }

    /// Builds the production OpenAI image-edit route from validated settings. The run-local
    /// manifest is written to `manifest_directory` when given.
    pub(super) fn start(
        settings: ImagerySettings,
        manifest_directory: Option<&Path>,
    ) -> Result<Self, io::Error> {
        let configuration = match settings {
            ImagerySettings::Off(reason) => return Ok(Self::disabled(reason)),
            ImagerySettings::On(configuration) => configuration,
        };
        let provider = ImageRoute::configured(&configuration)?;
        let locator = NativeHeadLocator::new(
            HeadLocateConfig::live(),
            configuration.credential.expose(),
            configuration.deadline,
        )
        .ok();
        let judge = NativePlateJudge::new(
            PlateJudgeConfig::live(),
            configuration.credential.expose(),
            PHOTO_CHECK_DEADLINE,
        )
        .ok();
        let imagery = Self::new(configuration, provider, manifest_directory)?;
        if let Mode::On { engine, .. } = &imagery.inner.mode {
            engine.figure_masks.configure();
            // Set once, here; a second call cannot happen.
            if let Some(locator) = locator {
                let _first = engine.locator.set(locator);
            }
            if let Some(judge) = judge {
                let _first = engine.judge.set(judge);
            }
        }
        Ok(imagery)
    }

    /// PLAY-V09: where the heads of `targets` (`id`, fixed description) are in the finished image
    /// `key`, as fractions of the picture. `face` is a picture of the first target's face (any
    /// image the server holds), which tells the right person from the many in the scene. Costs a
    /// fraction of a cent, never charges the image budget, and every failure is typed: the caller
    /// keeps the layout it predicted.
    pub(super) async fn locate_heads(
        &self,
        key: &ImageKey,
        face: Option<&ReferenceImage>,
        targets: &[(&str, &str)],
    ) -> Result<Vec<LocatedHead>, ImageFailure> {
        let Mode::On { engine, .. } = &self.inner.mode else {
            return Err(ImageFailure::Off);
        };
        let Some(locator) = engine.locator.get() else {
            return Err(ImageFailure::Off);
        };
        let registered = locked(&self.inner.registry)
            .by_key
            .get(key.as_str())
            .copied();
        let format = registered.ok_or(ImageFailure::InvalidRequest)?.format;
        let (bytes, stored) = engine
            .read_cached(key.as_str(), format)
            .await
            .ok_or(ImageFailure::InvalidRequest)?;
        let face = face.map(|face| (face.bytes.clone(), face.format));
        let (picture, face) = tokio::task::spawn_blocking(move || {
            let picture = vision_picture(&bytes, stored, 1024)?;
            let face = face
                .map(|(bytes, format)| vision_picture(&bytes, format, 384))
                .transpose()?;
            Ok::<_, SheetError>((picture, face))
        })
        .await
        .map_err(|_| ImageFailure::Interrupted)?
        .map_err(|_| ImageFailure::BadOutput)?;
        let targets: Vec<LocateTarget<'_>> = targets
            .iter()
            .map(|(id, description)| LocateTarget { id, description })
            .collect();
        let queued = std::time::Instant::now();
        let _permit = engine
            .head_permits
            .acquire()
            .await
            .map_err(|_| ImageFailure::Interrupted)?;
        let wait = queued.elapsed();
        // The deadline begins at provider dispatch, after the independent head queue.
        let started = std::time::Instant::now();
        let mut retries = 0;
        let mut provider_elapsed = Duration::ZERO;
        let result = loop {
            let remaining = HEAD_LOCATE_DEADLINE.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                break Err(ImageFailure::DeadlineExceeded);
            }
            let dispatched = std::time::Instant::now();
            let result = locator
                .locate(&picture, face.as_deref(), &targets, remaining)
                .await;
            let elapsed = dispatched.elapsed();
            provider_elapsed += elapsed;
            if matches!(result, Err(df_providers::HeadLocateError::Provider(429)))
                && retries < MAXIMUM_RATE_LIMIT_RETRIES
            {
                record_provider_timing(
                    "head_retry",
                    key,
                    Duration::ZERO,
                    elapsed,
                    Some(ImageFailure::RateLimited),
                );
                tokio::time::sleep((RATE_LIMIT_BACKOFF * (1 << retries)).min(remaining)).await;
                retries += 1;
                continue;
            }
            break result.map_err(|error| match error {
                df_providers::HeadLocateError::InvalidRequest => ImageFailure::InvalidRequest,
                df_providers::HeadLocateError::Output => ImageFailure::BadOutput,
                df_providers::HeadLocateError::Provider(429) => ImageFailure::RateLimited,
                df_providers::HeadLocateError::Transport(NativeHttpRefusal::Deadline) => {
                    ImageFailure::DeadlineExceeded
                }
                df_providers::HeadLocateError::Unauthorized
                | df_providers::HeadLocateError::Provider(_)
                | df_providers::HeadLocateError::Transport(_) => ImageFailure::Provider,
            });
        };
        record_provider_timing(
            "head_location",
            key,
            wait,
            provider_elapsed,
            result.as_ref().err().copied(),
        );
        result
    }

    pub(in crate::gameplay) async fn photo_evidence(
        &self,
        key: &ImageKey,
    ) -> Result<Option<PhotoEvidence>, ImageFailure> {
        let Mode::On { engine, .. } = &self.inner.mode else {
            return Ok(None);
        };
        let path = engine
            .cache_dir
            .join(format!("{}.features.json", key.as_str()));
        let bytes = tokio::task::spawn_blocking(move || read_cache_file(&path))
            .await
            .map_err(|_| ImageFailure::Interrupted)?;
        bytes
            .map(|bytes| serde_json::from_slice(&bytes).map_err(|_| ImageFailure::BadOutput))
            .transpose()
    }

    /// PLAY-V09: a small note kept beside the cached image `key` (its measured layout), so a
    /// later server run does not ask again. `None` when there is none (or imagery is off).
    pub(super) async fn read_note(&self, key: &ImageKey) -> Option<Vec<u8>> {
        let Mode::On { engine, .. } = &self.inner.mode else {
            return None;
        };
        let path = engine
            .cache_dir
            .join(format!("{}.layout.json", key.as_str()));
        tokio::task::spawn_blocking(move || read_cache_file(&path).map(|bytes| bytes.to_vec()))
            .await
            .ok()
            .flatten()
    }

    /// Only generated requests with an explicit spatial instruction have this per-image record.
    pub(super) async fn plate_staging(&self, key: &ImageKey) -> Vec<StagedPerson> {
        let Mode::On { engine, .. } = &self.inner.mode else {
            return Vec::new();
        };
        let path = engine
            .cache_dir
            .join(format!("{}.staging-v1.json", key.as_str()));
        let note = tokio::task::spawn_blocking(move || read_cache_file(&path)).await;
        let Ok(Some(bytes)) = note else {
            return Vec::new();
        };
        match serde_json::from_slice::<Vec<StagedPerson>>(&bytes) {
            Ok(people) if !people.is_empty() && people.len() <= 24 => people,
            _ => Vec::new(),
        }
    }

    async fn keep_plate_staging(&self, key: &ImageKey, people: Vec<StagedPerson>) {
        if people.is_empty() {
            return;
        }
        let Mode::On { engine, .. } = &self.inner.mode else {
            return;
        };
        let target = engine
            .cache_dir
            .join(format!("{}.staging-v1.json", key.as_str()));
        let staging = engine.cache_dir.join(format!(
            ".{}.staging.{}.tmp",
            key.as_str(),
            std::process::id()
        ));
        let Ok(note) = serde_json::to_vec(&people) else {
            return;
        };
        let kept =
            tokio::task::spawn_blocking(move || write_cache_file(&staging, &target, &note)).await;
        if !matches!(kept, Ok(Ok(()))) {
            crate::gameplay::actor::record_audit_line(
                "gameplay.imagery.staging_record_failed",
                format!("PLATE staging key={} outcome=unavailable", key.as_str()),
            );
        }
    }

    /// Keeps a note beside the cached image `key`. A failed write only means it is asked again.
    pub(super) async fn write_note(&self, key: &ImageKey, note: Vec<u8>) {
        let Mode::On { engine, .. } = &self.inner.mode else {
            return;
        };
        let target = engine
            .cache_dir
            .join(format!("{}.layout.json", key.as_str()));
        let staging = engine.cache_dir.join(format!(
            ".{}.layout.{}.tmp",
            key.as_str(),
            std::process::id()
        ));
        let _kept =
            tokio::task::spawn_blocking(move || write_cache_file(&staging, &target, &note)).await;
    }

    /// Uses `provider`; tests pass one pointed at a loopback fake.
    pub(super) fn new(
        configuration: ImageryConfiguration,
        provider: impl Into<ImageRoute>,
        manifest_directory: Option<&Path>,
    ) -> Result<Self, io::Error> {
        Self::new_in_hub(
            configuration,
            provider.into(),
            manifest_directory,
            super::resources::Hub::new(),
            false,
        )
    }

    pub(super) fn new_in_hub(
        configuration: ImageryConfiguration,
        provider: ImageRoute,
        manifest_directory: Option<&Path>,
        hub: super::resources::Hub,
        creator_only: bool,
    ) -> Result<Self, io::Error> {
        std::fs::create_dir_all(&configuration.cache_dir)?;
        let summary = format!(
            "IMAGERY on model={} interval={}s rating={} quality={} profile={}",
            provider.label(configuration.model),
            configuration.interval.as_secs(),
            configuration.rating.label(),
            match configuration.quality {
                ImageQuality::Low => "low",
                ImageQuality::Medium => "medium",
                ImageQuality::High => "high",
            },
            configuration.profile.label(),
        );
        let manifest = manifest_directory.map(|directory| directory.join(MANIFEST_NAME));
        let engine = Engine {
            provider,
            model: configuration.model,
            quality: configuration.quality,
            profile: configuration.profile,
            rating: configuration.rating,
            deadline: configuration.deadline,
            cache_dir: configuration.cache_dir,
            manifest,
            ledger: Arc::clone(hub.ledger()),
            calls: AtomicU32::new(0),
            cache_hits: AtomicU32::new(0),
            micro_dollars: AtomicU64::new(0),
            presence_permits: Semaphore::new(MAXIMUM_IN_FLIGHT),
            permits: Semaphore::new(FOREGROUND_IN_FLIGHT),
            background_permits: Semaphore::new(BACKGROUND_IN_FLIGHT),
            head_permits: Semaphore::new(HEADS_IN_FLIGHT),
            in_flight: Mutex::new(HashMap::new()),
            locator: std::sync::OnceLock::new(),
            figure_masks: figure_masks::FigureMaskCache::default(),
            judge: std::sync::OnceLock::new(),
        };
        let mut registry = Registry::default();
        engine.register_from_manifest(&mut registry);
        let engine = Arc::new(engine);
        // Two bounded permit waits and edits, with up to two 30-second photo checks.
        let wait = engine.deadline * 4 + Duration::from_secs(70);
        for kind in if creator_only {
            vec![super::resources::ResourceKind::CreatorPortrait]
        } else {
            IMAGE_KINDS.to_vec()
        } {
            hub.set_limits(
                kind,
                super::resources::Limits {
                    timeout: wait,
                    attempts: 2,
                },
            );
            let checked = Arc::clone(&engine);
            hub.set_validator(
                kind,
                Arc::new(move |generated| checked.cached_now(&generated.handle)),
            );
        }
        Ok(Self {
            inner: Arc::new(Inner {
                registry: Mutex::new(registry),
                mode: Mode::On { engine, summary },
                hub,
            }),
        })
    }

    /// The startup line; it never contains the credential.
    pub(super) fn startup_line(&self) -> String {
        match &self.inner.mode {
            Mode::Off(reason) => format!("IMAGERY off ({})", reason.reason()),
            Mode::On { summary, .. } => summary.clone(),
        }
    }

    pub(super) fn is_on(&self) -> bool {
        matches!(self.inner.mode, Mode::On { .. })
    }

    /// Whether the active route is FLUX. FLUX reads the people and the shot first, and a
    /// Kontext edit keeps its input, so plates on that route are composed afresh when the shot or
    /// a face changes; the OpenAI route keeps the layout and the edit chain it was built for.
    pub(super) fn flux_prompts(&self) -> bool {
        match &self.inner.mode {
            Mode::On { engine, .. } => matches!(engine.provider, ImageRoute::Flux(_)),
            Mode::Off(_) => false,
        }
    }

    #[cfg(test)]
    pub(super) fn stats(&self) -> Stats {
        match &self.inner.mode {
            Mode::Off(_) => Stats::default(),
            Mode::On { engine, .. } => Stats {
                calls: engine.calls.load(Ordering::SeqCst),
                cache_hits: engine.cache_hits.load(Ordering::SeqCst),
                micro_dollars: engine.micro_dollars.load(Ordering::SeqCst),
            },
        }
    }

    /// The key a request would be stored under.
    pub(super) fn key_for(&self, request: &ImageRequest) -> ImageKey {
        let (model, quality, rating) = match &self.inner.mode {
            Mode::On { engine, .. } => (
                engine.provider.label(engine.model),
                engine.key_quality(request.kind),
                engine.rating.label(),
            ),
            Mode::Off(_) => ("off", "off", "off"),
        };
        request_key(request, model, quality, rating)
    }

    /// The resource generation hub that owns every generated image's request, state and audit.
    pub(super) fn hub(&self) -> &super::resources::Hub {
        &self.inner.hub
    }

    pub(super) fn model_label(&self) -> &'static str {
        match &self.inner.mode {
            Mode::On { engine, .. } => engine.provider.label(engine.model),
            Mode::Off(_) => "off",
        }
    }

    /// Called only by the owned creator pipeline after its classification chooses a route.
    pub(super) async fn render_creator(
        &self,
        request: ImageRequest,
    ) -> Result<ImageKey, ImageFailure> {
        self.render(request).await
    }

    /// The one way to get an image made: describes the request as a typed resource, runs it
    /// through the hub (state, timeout, request cap, `RESOURCE` audit lines) and returns the key
    /// to load it by. `subject` names what the picture is about (`character:ant-wolf`,
    /// `place:salt-quay`, `plate:<scene>`, `item:<id>`). Every consumer calls this; the render
    /// below is the hub's own generator and is private to this module.
    pub(super) async fn generate(
        &self,
        subject: &str,
        request: ImageRequest,
    ) -> Result<ImageKey, ImageFailure> {
        let priority = ImagePriority::for_kind(request.kind);
        self.generate_with_priority(subject, request, priority)
            .await
    }

    /// Look-ahead callers explicitly use Background; current-place canonical work uses Presence.
    pub(in crate::gameplay) async fn generate_with_priority(
        &self,
        subject: &str,
        request: ImageRequest,
        priority: ImagePriority,
    ) -> Result<ImageKey, ImageFailure> {
        use super::resources::{DepictionSpec, ItemCloseUpSpec, PlateEditSpec, PlateSpec};
        use df_render::art_contract::Neutral;
        let staging_order = request.prompt.staging_order().to_vec();
        let key = self.key_for(&request);
        let key = key.as_str();
        let result = match request.kind {
            ImageKind::ScenePlate => {
                self.run(
                    PlateSpec::new(subject, key, Neutral::Nothing),
                    request,
                    priority,
                )
                .await
            }
            ImageKind::ScenePlateEdit => {
                self.run(
                    PlateEditSpec::new(subject, key, Neutral::Nothing),
                    request,
                    priority,
                )
                .await
            }
            ImageKind::ArtifactCloseUp => {
                self.run(
                    ItemCloseUpSpec::new(subject, key, Neutral::Block),
                    request,
                    priority,
                )
                .await
            }
            ImageKind::CanonicalCharacter | ImageKind::CastExpression => {
                self.run(
                    DepictionSpec::new(subject, key, Neutral::Silhouette),
                    request,
                    priority,
                )
                .await
            }
            ImageKind::CanonicalRoom | ImageKind::SceneRefresh => {
                self.run(
                    DepictionSpec::new(subject, key, Neutral::Nothing),
                    request,
                    priority,
                )
                .await
            }
        };
        if let Ok(key) = &result {
            self.keep_plate_staging(key, staging_order).await;
        }
        result
    }

    /// Register reviewed local bytes for key-based consumers without invoking a provider.
    /// The caller obtains the photograph through ReferenceStore's reviewed-asset boundary.
    pub(super) async fn register_reviewed(
        &self,
        subject: &str,
        reference: ReferenceImage,
    ) -> Result<ImageKey, ImageFailure> {
        use super::resources::{DepictionSpec, Generated, Generation, ResourceFailure};
        let key = ImageKey(format!(
            "{:x}",
            Sha256::digest(format!("reviewed-photograph-v1|{}", reference.sha256).as_bytes())
        ));
        let spec = DepictionSpec::new(
            subject,
            key.as_str(),
            df_render::art_contract::Neutral::Nothing,
        );
        let imagery = self.clone();
        let registered_key = key.clone();
        let generation: Generation = Box::pin(async move {
            let Mode::On { engine, .. } = &imagery.inner.mode else {
                return Err(ResourceFailure::Permanent(ImageFailure::Off.label()));
            };
            engine
                .store(&registered_key, reference.format, reference.bytes)
                .await
                .map_err(|_| ResourceFailure::Transient(ImageFailure::Provider.label()))?;
            locked(&imagery.inner.registry).register(
                &registered_key,
                Registered {
                    format: reference.format,
                },
            );
            Ok(Generated::new(registered_key.as_str()))
        });
        self.hub()
            .render(&spec, generation)
            .await
            .map_err(|failure| match failure {
                ResourceFailure::TimedOut => ImageFailure::DeadlineExceeded,
                _ => ImageFailure::from_label(failure.label()).unwrap_or(ImageFailure::Interrupted),
            })?;
        // A hub cache hit must also re-register the key in this process's serving registry.
        locked(&self.inner.registry).register(
            &key,
            Registered {
                format: reference.format,
            },
        );
        Ok(key)
    }

    async fn run<S: super::resources::ResourceSpec>(
        &self,
        spec: S,
        request: ImageRequest,
        priority: ImagePriority,
    ) -> Result<ImageKey, ImageFailure> {
        use super::resources::{Generated, Generation, ResourceFailure};
        let started = std::time::Instant::now();
        let kind = request.kind;
        let key = self.key_for(&request);
        let rendered = Arc::new(AtomicBool::new(false));
        let dispatched = Arc::clone(&rendered);
        let original = Arc::new(Mutex::new(None));
        let slot = Arc::clone(&original);
        let imagery = self.clone();
        let generation: Generation = Box::pin(async move {
            dispatched.store(true, Ordering::SeqCst);
            match imagery.render_with_priority(request, priority).await {
                Ok(key) => Ok(Generated::new(key.as_str())),
                Err(failure) => {
                    *locked(&slot) = Some(failure);
                    Err(if failure.is_permanent() {
                        ResourceFailure::Permanent(failure.label())
                    } else {
                        ResourceFailure::Transient(failure.label())
                    })
                }
            }
        });
        let result = match self.hub().render(&spec, generation).await {
            Ok(_) => Ok(key.clone()),
            Err(ResourceFailure::TimedOut) => Err(ImageFailure::DeadlineExceeded),
            Err(failure) => Err(locked(&original)
                .take()
                .or_else(|| ImageFailure::from_label(failure.label()))
                .unwrap_or(ImageFailure::Interrupted)),
        };
        // Hub cache hits and coalesced callers make no provider call of their own.
        if !rendered.load(Ordering::SeqCst) {
            record_image_result(
                kind,
                &key,
                started.elapsed(),
                RequestTiming::default(),
                result.as_ref(),
            );
        }
        result
    }

    /// Renders `request`, or finds its cached result, and returns the key to load it by. The
    /// key is registered before any work so the route can serve it as soon as it exists. Only the
    /// hub (through `generate`) calls this.
    async fn render(&self, request: ImageRequest) -> Result<ImageKey, ImageFailure> {
        let priority = ImagePriority::for_kind(request.kind);
        self.render_with_priority(request, priority).await
    }

    async fn render_with_priority(
        &self,
        request: ImageRequest,
        priority: ImagePriority,
    ) -> Result<ImageKey, ImageFailure> {
        let started = std::time::Instant::now();
        let key = self.key_for(&request);
        let kind = request.kind;
        let (line, fields, attributes) = image_request_diagnostics(&key, &request);
        crate::gameplay::actor::record_audit_diagnostic(
            "gameplay.imagery.render_requested",
            line,
            fields,
            &attributes,
        );
        let Mode::On { engine, .. } = &self.inner.mode else {
            record_image_result(
                kind,
                &key,
                started.elapsed(),
                RequestTiming::default(),
                Err(&ImageFailure::Off),
            );
            return Err(ImageFailure::Off);
        };
        let format = request.output.format();
        locked(&self.inner.registry).register(&key, Registered { format });
        let flight = engine.image(&key, request, priority).await;
        let result = flight.outcome.map(|()| key.clone());
        record_image_result(
            kind,
            &key,
            started.elapsed(),
            flight.timing,
            result.as_ref().map_err(|e| e),
        );
        result
    }

    /// The bytes of a registered key, read from the cache.
    pub(super) async fn image(&self, key: &str) -> Lookup {
        let registered = locked(&self.inner.registry).by_key.get(key).copied();
        let Some(registered) = registered else {
            return Lookup::Unregistered;
        };
        let Mode::On { engine, .. } = &self.inner.mode else {
            return Lookup::NotReady;
        };
        match engine.read_cached(key, registered.format).await {
            Some((bytes, format)) => Lookup::Image { bytes, format },
            None => Lookup::NotReady,
        }
    }

    /// The key of `request` if its image is already cached; never calls the provider and never
    /// charges. The only way a fight learns what it may show.
    pub(super) async fn peek(&self, request: &ImageRequest) -> Option<ImageKey> {
        let Mode::On { engine, .. } = &self.inner.mode else {
            return None;
        };
        let key = self.key_for(request);
        let format = request.output.format();
        engine.cache_has(&key, format).await?;
        locked(&self.inner.registry).register(&key, Registered { format });
        Some(key)
    }
}

impl Engine {
    fn permits(&self, priority: ImagePriority) -> &Semaphore {
        match priority {
            ImagePriority::Foreground => &self.permits,
            ImagePriority::Presence => &self.presence_permits,
            ImagePriority::Background => &self.background_permits,
        }
    }

    fn quality_label(&self) -> &'static str {
        match self.quality {
            ImageQuality::Low => "low",
            ImageQuality::Medium => "medium",
            ImageQuality::High => "high",
        }
    }

    /// The quality and size this kind is drawn at under the configured profile.
    fn drawn(&self, kind: ImageKind, output: OutputSpec) -> (ImageQuality, OutputSpec) {
        drawn_for(self.profile, self.quality, kind, output)
    }

    /// The quality part of a cache key: production keys are unchanged, and a dev picture never
    /// stands in for a production one.
    fn key_quality(&self, kind: ImageKind) -> &'static str {
        use super::startup_configuration::ImageryProfile;
        match (self.profile, self.drawn(kind, OutputSpec::PLATE).0) {
            (ImageryProfile::Production, _) => self.quality_label(),
            (ImageryProfile::Dev, ImageQuality::Low) => "dev-low",
            (ImageryProfile::Dev, ImageQuality::Medium) => "dev-medium",
            (ImageryProfile::Dev, ImageQuality::High) => "dev-high",
        }
    }

    fn file_name(key: &ImageKey, format: ImageFormat) -> String {
        let extension = match format {
            ImageFormat::Png => "png",
            ImageFormat::Webp => "webp",
            ImageFormat::Jpeg => "jpg",
        };
        format!("{}.{extension}", key.as_str())
    }

    /// The formats an image of `format` may be found in: a cut-out cached before WebP output
    /// is a PNG of the same request.
    fn stored_formats(format: ImageFormat) -> &'static [ImageFormat] {
        match format {
            ImageFormat::Webp => &[ImageFormat::Webp, ImageFormat::Png],
            ImageFormat::Png => &[ImageFormat::Png],
            ImageFormat::Jpeg => &[ImageFormat::Jpeg],
        }
    }

    async fn image(
        self: &Arc<Self>,
        key: &ImageKey,
        request: ImageRequest,
        priority: ImagePriority,
    ) -> FlightResult {
        let format = request.output.format();
        if self.cache_has(key, format).await.is_some() {
            self.cache_hits.fetch_add(1, Ordering::SeqCst);
            return FlightResult {
                outcome: Ok(()),
                timing: RequestTiming::default(),
            };
        }
        let mut receiver = {
            let mut flights = locked(&self.in_flight);
            match flights.get(key.as_str()) {
                Some(receiver) => receiver.clone(),
                None => {
                    let (sender, receiver) = watch::channel(None);
                    flights.insert(key.0.clone(), receiver.clone());
                    tokio::spawn(Arc::clone(self).fly(key.clone(), request, priority, sender));
                    receiver
                }
            }
        };
        let interrupted = FlightResult {
            outcome: Err(ImageFailure::Interrupted),
            timing: RequestTiming::default(),
        };
        match receiver.wait_for(Option::is_some).await {
            Ok(finished) => finished.unwrap_or(interrupted),
            Err(_) => interrupted,
        }
    }

    /// The one task of a key. Callers that go away do not cancel it: its charge is already
    /// spent, so its image is finished and cached for the next request.
    async fn fly(
        self: Arc<Self>,
        key: ImageKey,
        request: ImageRequest,
        priority: ImagePriority,
        sender: watch::Sender<Option<FlightResult>>,
    ) {
        let context = OperationContext {
            trace_parent: String::new(),
            build: crate::BUILD_ID.to_owned(),
        };
        let mut span = df_observe::begin(&context, "gameplay.imagery_render");
        span.event(
            "gameplay.imagery.render",
            &span_fields(
                request.kind,
                self.provider.label(self.model),
                request.references.len(),
            ),
        );
        let mut timing = RequestTiming::default();
        let outcome = self
            .retire_flight(&key, self.produce(&key, request, priority, &mut timing))
            .await;
        span.finish_unmeasured(match outcome {
            Ok(()) => "rendered",
            Err(failure) => failure.label(),
        });
        // No receiver left means every requester went away; the cache holds the result.
        sender.send_replace(Some(FlightResult { outcome, timing }));
    }

    async fn retire_flight(
        &self,
        key: &ImageKey,
        producing: impl std::future::Future<Output = Outcome>,
    ) -> Outcome {
        // Even an adapter panic retires this key so subsequent callers can retry.
        use futures::FutureExt;
        let outcome = std::panic::AssertUnwindSafe(producing)
            .catch_unwind()
            .await
            .unwrap_or(Err(ImageFailure::Interrupted));
        locked(&self.in_flight).remove(key.as_str());
        outcome
    }

    async fn produce(
        &self,
        key: &ImageKey,
        request: ImageRequest,
        priority: ImagePriority,
        timing: &mut RequestTiming,
    ) -> Outcome {
        let format = request.output.format();
        // A flight that finished between the caller's cache check and this one wrote it.
        if self.cache_has(key, format).await.is_some() {
            self.cache_hits.fetch_add(1, Ordering::SeqCst);
            return Ok(());
        }
        // A reference over the adapter's limit is sent smaller rather than refusing the edit.
        let mut fitted = Vec::with_capacity(request.references.len());
        for reference in &request.references {
            let bytes = if reference.bytes.len() > REFERENCE_BYTES_LIMIT {
                let (source, format) = (reference.bytes.clone(), reference.format);
                tokio::task::spawn_blocking(move || {
                    fit_reference(&source, format, REFERENCE_BYTES_LIMIT)
                })
                .await
                .map_err(|_| ImageFailure::Interrupted)?
                .map_err(|_| ImageFailure::InvalidRequest)?
                .map_or_else(|| reference.bytes.clone(), Bytes::from)
            } else {
                reference.bytes.clone()
            };
            fitted.push((reference.format, bytes));
        }
        let references: Vec<ProviderReference<'_>> = fitted
            .iter()
            .map(|(format, bytes)| ProviderReference {
                format: *format,
                bytes,
            })
            .collect();
        let (quality, drawn) = self.drawn(request.kind, request.output);
        let edit = ImageEdit {
            model: self.model,
            prompt: request.prompt.as_str(),
            references: &references,
            width: drawn.width,
            height: drawn.height,
            quality,
            background: if request.output.transparent {
                ImageBackground::Transparent
            } else {
                ImageBackground::Opaque
            },
            output: format,
            moderation: match self.rating {
                ContentRating::Mature => ImageModeration::Low,
                ContentRating::Restrained => ImageModeration::Auto,
            },
        };
        edit.validate().map_err(|_| ImageFailure::InvalidRequest)?;
        let started = std::time::Instant::now();
        let (mut stored_format, mut bytes, mut micro_dollars, supplier_credits) =
            self.generate(&edit, format, key, priority, timing).await?;
        // ART-01: a plate or empty room is judged before it is stored; one that fails is made
        // once more with the defect named, and a second failure stores nothing.
        let gate = match request.kind {
            ImageKind::ScenePlate | ImageKind::ScenePlateEdit => {
                Some(photo_check::PhotoCheckKind::ScenePlate)
            }
            ImageKind::CanonicalRoom => Some(photo_check::PhotoCheckKind::CanonicalRoom),
            _ => None,
        };
        let scene_party = request
            .prompt
            .scene_party()
            .map(|party| {
                request
                    .references
                    .get(party.sheet_image() - 1)
                    .map(|sheet| (party, sheet))
                    .ok_or(ImageFailure::InvalidRequest)
            })
            .transpose()?;
        let mut visible_features = Vec::new();
        if let (Some(kind), Some(judge)) = (gate, self.judge.get()) {
            let (failure, features) = scheduled_photo_failure(
                self,
                key,
                timing,
                PhotoCheckInput {
                    judge,
                    kind,
                    bytes: &bytes,
                    format: stored_format,
                    prompt: request.prompt.as_str(),
                    scene_party,
                    hand_custody: request.prompt.scene_hand_custody(),
                },
            )
            .await;
            visible_features = features;
            if let Some(failure) = failure {
                let corrected = format!("{} {}", failure.correction(), request.prompt.as_str());
                let retry = ImageEdit {
                    prompt: &corrected,
                    ..edit
                };
                retry.validate().map_err(|_| ImageFailure::BadOutput)?;
                let (retry_format, retry_bytes, retry_cost, _) =
                    self.generate(&retry, format, key, priority, timing).await?;
                micro_dollars = micro_dollars.saturating_add(retry_cost);
                let (failure, features) = scheduled_photo_failure(
                    self,
                    key,
                    timing,
                    PhotoCheckInput {
                        judge,
                        kind,
                        bytes: &retry_bytes,
                        format: retry_format,
                        prompt: &corrected,
                        scene_party,
                        hand_custody: request.prompt.scene_hand_custody(),
                    },
                )
                .await;
                match failure {
                    None => visible_features = features,
                    Some(photo_check::PhotoCheckFailure::VisibleText) => {
                        record_photo_check(kind, "kept", "visible_text", scene_party);
                        visible_features.clear();
                    }
                    Some(failure) => {
                        record_photo_check(kind, "fallback", failure.label(), scene_party);
                        return Err(ImageFailure::BadOutput);
                    }
                }
                (stored_format, bytes) = (retry_format, retry_bytes);
            }
        }
        // Keep evidence with the exact admitted key, including corrected regenerations.
        if !visible_features.is_empty() {
            let context =
                serde_json::from_str::<serde_json::Value>(request.prompt.scene_hand_custody()).ok();
            if let Some(context) = context
                && let (Some(place), Some(source_hash)) =
                    (context["place"].as_str(), context["source_hash"].as_str())
            {
                let evidence = PhotoEvidence {
                    place: place.into(),
                    source_hash: source_hash.into(),
                    visible_features,
                };
                let note = serde_json::to_vec(&evidence).map_err(|_| ImageFailure::BadOutput)?;
                let target = self
                    .cache_dir
                    .join(format!("{}.features.json", key.as_str()));
                let staging = self.cache_dir.join(format!(
                    ".{}.features.{}.tmp",
                    key.as_str(),
                    std::process::id()
                ));
                tokio::task::spawn_blocking(move || write_cache_file(&staging, &target, &note))
                    .await
                    .map_err(|_| ImageFailure::Interrupted)?
                    .map_err(|_| ImageFailure::Provider)?;
            } else {
                record_photo_check(
                    photo_check::PhotoCheckKind::ScenePlate,
                    "features_unbound",
                    "source_unavailable",
                    scene_party,
                );
            }
        }
        let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let stored = self.store(key, stored_format, bytes.clone()).await.is_ok();
        if stored {
            self.record(
                key,
                &request,
                bytes.len() as u64,
                micro_dollars,
                supplier_credits,
                elapsed_ms,
            )
            .await;
        }
        // A result that could not be cached is still served from memory for this key's first
        // requester only through the cache, so a failed write is a failed render.
        stored.then_some(()).ok_or(ImageFailure::Provider)
    }

    /// One provider call for `edit`: the bytes in a format the request accepts and their cost.
    async fn generate(
        &self,
        edit: &ImageEdit<'_>,
        format: ImageFormat,
        key: &ImageKey,
        priority: ImagePriority,
        timing: &mut RequestTiming,
    ) -> Result<(ImageFormat, Bytes, u64, Option<String>), ImageFailure> {
        let mut retries = 0;
        let (mut generated, supplier_credits) = loop {
            let queued = std::time::Instant::now();
            let permit =
                tokio::time::timeout(self.deadline, self.permits(priority).acquire()).await;
            let wait = queued.elapsed();
            timing.wait += wait;
            let permit = permit
                .map_err(|_| ImageFailure::DeadlineExceeded)?
                .map_err(|_| ImageFailure::Interrupted)?;
            self.charge();
            let started = std::time::Instant::now();
            let result = self.provider.edit(edit, self.deadline).await;
            let elapsed = started.elapsed();
            timing.provider += elapsed;
            drop(permit);
            record_provider_timing("image", key, wait, elapsed, result.as_ref().err().copied());
            match result {
                Err(ImageFailure::RateLimited) if retries < MAXIMUM_RATE_LIMIT_RETRIES => {
                    tokio::time::sleep(RATE_LIMIT_BACKOFF * (1 << retries)).await;
                    retries += 1;
                }
                result => break result?,
            }
        };
        if matches!(self.provider, ImageRoute::Bfl(_)) && format == ImageFormat::Jpeg {
            let source = generated.bytes;
            let (width, height) = (edit.width, edit.height);
            let source_format = match generated.format {
                ImageFormat::Png => image::ImageFormat::Png,
                ImageFormat::Jpeg => image::ImageFormat::Jpeg,
                _ => return Err(ImageFailure::BadOutput),
            };
            generated.bytes = tokio::task::spawn_blocking(move || {
                let image = image::load_from_memory_with_format(&source, source_format)
                    .map_err(|_| ImageFailure::BadOutput)?;
                let image =
                    image.resize_exact(width, height, image::imageops::FilterType::Lanczos3);
                let mut bytes = Vec::new();
                image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 88)
                    .encode_image(&image.to_rgb8())
                    .map_err(|_| ImageFailure::BadOutput)?;
                Ok::<_, ImageFailure>(bytes)
            })
            .await
            .map_err(|_| ImageFailure::Interrupted)??;
            generated.format = ImageFormat::Jpeg;
            generated.has_alpha_channel = false;
            generated.width = width;
            generated.height = height;
        }
        if generated.bytes.is_empty() || generated.bytes.len() as u64 > MAXIMUM_IMAGE_BYTES {
            return Err(ImageFailure::BadOutput);
        }
        let micro_dollars = usage_cost(generated.usage);
        self.micro_dollars
            .fetch_add(micro_dollars, Ordering::SeqCst);
        // FLUX may answer a WebP request with a PNG; it is kept in a format the request accepts.
        if !Self::stored_formats(format).contains(&generated.format) {
            return Err(ImageFailure::BadOutput);
        }
        Ok((
            generated.format,
            Bytes::from(generated.bytes),
            micro_dollars,
            supplier_credits,
        ))
    }

    /// Spend is committed before the call and kept whatever the outcome: a call whose result is
    /// unknown may still have been billed.
    fn charge(&self) {
        self.ledger.reserve(IMAGE_UNIT, 1).commit(1);
        self.calls.fetch_add(1, Ordering::SeqCst);
    }

    /// Whether an image of `key` is on disk in any format, without waiting: the hub's
    /// freshness check of a ready entry.
    fn cached_now(&self, key: &str) -> bool {
        let Some(key) = ImageKey::parse(key) else {
            return false;
        };
        [ImageFormat::Webp, ImageFormat::Png, ImageFormat::Jpeg]
            .into_iter()
            .any(|format| {
                std::fs::symlink_metadata(self.cache_dir.join(Self::file_name(&key, format)))
                    .is_ok_and(|metadata| {
                        metadata.is_file() && (1..=MAXIMUM_IMAGE_BYTES).contains(&metadata.len())
                    })
            })
    }

    /// The format an image of `format` is cached in, if it is cached.
    async fn cache_has(&self, key: &ImageKey, format: ImageFormat) -> Option<ImageFormat> {
        let paths: Vec<(ImageFormat, PathBuf)> = Self::stored_formats(format)
            .iter()
            .map(|found| (*found, self.cache_dir.join(Self::file_name(key, *found))))
            .collect();
        tokio::task::spawn_blocking(move || {
            paths.into_iter().find_map(|(found, path)| {
                std::fs::symlink_metadata(&path)
                    .is_ok_and(|metadata| {
                        metadata.is_file() && (1..=MAXIMUM_IMAGE_BYTES).contains(&metadata.len())
                    })
                    .then_some(found)
            })
        })
        .await
        .ok()
        .flatten()
    }

    async fn read_cached(&self, key: &str, format: ImageFormat) -> Option<(Bytes, ImageFormat)> {
        let key = ImageKey::parse(key)?;
        let paths: Vec<(ImageFormat, PathBuf)> = Self::stored_formats(format)
            .iter()
            .map(|found| (*found, self.cache_dir.join(Self::file_name(&key, *found))))
            .collect();
        // A failed blocking task is treated as a cache miss.
        tokio::task::spawn_blocking(move || {
            paths
                .into_iter()
                .find_map(|(found, path)| read_cache_file(&path).map(|bytes| (bytes, found)))
        })
        .await
        .ok()
        .flatten()
    }

    /// Writes a temporary file beside the target and renames it into place so a reader never
    /// sees a partial file.
    async fn store(&self, key: &ImageKey, format: ImageFormat, bytes: Bytes) -> io::Result<()> {
        let name = Self::file_name(key, format);
        let target = self.cache_dir.join(&name);
        let staging = self
            .cache_dir
            .join(format!(".{name}.{}.tmp", std::process::id()));
        tokio::task::spawn_blocking(move || write_cache_file(&staging, &target, &bytes))
            .await
            .map_err(|_| io::Error::other("imagery cache write task failed"))?
    }

    /// Appends the provenance of one image to the run-local manifest. A failed append is
    /// reported to diagnostics and never fails the render.
    async fn record(
        &self,
        key: &ImageKey,
        request: &ImageRequest,
        bytes: u64,
        micro_dollars: u64,
        supplier_credits: Option<String>,
        elapsed_ms: u64,
    ) {
        let Some(path) = self.manifest.clone() else {
            return;
        };
        let line = ManifestLine {
            key: key.0.clone(),
            kind: request.kind.label().to_owned(),
            model: self.provider.label(self.model).to_owned(),
            prompt: request.prompt.as_str().to_owned(),
            references: request
                .references
                .iter()
                .map(|reference| reference.sha256.clone())
                .collect(),
            created_unix: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_secs()),
            bytes,
            // The size actually drawn under the profile, not the kind's nominal size.
            width: self.drawn(request.kind, request.output).1.width,
            height: self.drawn(request.kind, request.output).1.height,
            transparent: request.output.transparent,
            micro_dollars: (!matches!(self.provider, ImageRoute::Bfl(_))).then_some(micro_dollars),
            supplier_credits,
            elapsed_ms,
        };
        let appended = tokio::task::spawn_blocking(move || append_manifest(&path, &line)).await;
        if !matches!(appended, Ok(Ok(()))) {
            let context = OperationContext {
                trace_parent: String::new(),
                build: crate::BUILD_ID.to_owned(),
            };
            let mut span = df_observe::begin(&context, "gameplay.imagery_manifest");
            span.finish_unmeasured("append_failed");
        }
    }

    /// Re-registers the keys an earlier run of this adventure generated and cached, so a display
    /// that reconnects after a restart can still load the art it was shown.
    fn register_from_manifest(&self, registry: &mut Registry) {
        let Some(path) = &self.manifest else {
            return;
        };
        let Some(text) = read_bounded_text(path) else {
            return;
        };
        for line in text.lines() {
            let Ok(entry) = serde_json::from_str::<ManifestLine>(line) else {
                continue;
            };
            let Some(key) = ImageKey::parse(&entry.key) else {
                continue;
            };
            let format = if entry.transparent {
                ImageFormat::Webp
            } else {
                ImageFormat::Jpeg
            };
            let present = Self::stored_formats(format).iter().any(|found| {
                std::fs::symlink_metadata(self.cache_dir.join(Self::file_name(&key, *found)))
                    .is_ok_and(|metadata| metadata.is_file())
            });
            if present {
                registry.register(&key, Registered { format });
            }
        }
    }
}

/// ART-01: the defect a finished image shows, or `None` when it passes. A judge that cannot answer
/// lets the image through: the check is a quality gate, and the audit line records the gap.
struct PhotoCheckInput<'a> {
    judge: &'a NativePlateJudge,
    kind: photo_check::PhotoCheckKind,
    bytes: &'a [u8],
    format: ImageFormat,
    prompt: &'a str,
    scene_party: Option<(SceneParty, &'a ReferenceImage)>,
    hand_custody: &'a str,
}

async fn scheduled_photo_failure(
    engine: &Engine,
    key: &ImageKey,
    timing: &mut RequestTiming,
    input: PhotoCheckInput<'_>,
) -> (
    Option<photo_check::PhotoCheckFailure>,
    Vec<super::journey::living_scene::VisibleFeature>,
) {
    let mut retries = 0;
    let reading = loop {
        let queued = std::time::Instant::now();
        let permit = engine.permits.acquire().await;
        let wait = queued.elapsed();
        timing.wait += wait;
        let Ok(permit) = permit else {
            record_provider_timing(
                "photo_check",
                key,
                wait,
                Duration::ZERO,
                Some(ImageFailure::Interrupted),
            );
            return (None, Vec::new());
        };
        let started = std::time::Instant::now();
        let reading = read_photo(&input).await;
        let elapsed = started.elapsed();
        timing.provider += elapsed;
        drop(permit);
        let rate_limited = matches!(
            reading,
            Err(photo_check::PhotoCheckError::Judge(
                df_providers::PlateJudgeError::Provider(429)
            ))
        );
        record_provider_timing(
            "photo_check",
            key,
            wait,
            elapsed,
            reading.as_ref().err().map(|_| {
                if rate_limited {
                    ImageFailure::RateLimited
                } else {
                    ImageFailure::Provider
                }
            }),
        );
        if rate_limited && retries < MAXIMUM_RATE_LIMIT_RETRIES {
            tokio::time::sleep(RATE_LIMIT_BACKOFF * (1 << retries)).await;
            retries += 1;
            continue;
        }
        break reading;
    };
    photo_outcome(input, reading).await
}

async fn read_photo(
    input: &PhotoCheckInput<'_>,
) -> Result<photo_check::ScenePhotoReading, photo_check::PhotoCheckError> {
    let PhotoCheckInput {
        judge,
        kind,
        bytes,
        format,
        scene_party,
        hand_custody,
        ..
    } = *input;
    match (kind, scene_party) {
        (photo_check::PhotoCheckKind::ScenePlate, Some((party, sheet))) => {
            photo_check::read_scene_photo(
                judge,
                bytes,
                format,
                party,
                sheet,
                hand_custody,
                PHOTO_CHECK_DEADLINE,
            )
            .await
        }
        _ => photo_check::check_photo(judge, kind, bytes, format, PHOTO_CHECK_DEADLINE)
            .await
            .map(|outcome| photo_check::ScenePhotoReading {
                outcome,
                visible_features: Vec::new(),
            }),
    }
}

#[cfg(test)]
async fn photo_failure(
    judge: &NativePlateJudge,
    kind: photo_check::PhotoCheckKind,
    bytes: &[u8],
    format: ImageFormat,
    prompt: &str,
    scene_party: Option<(SceneParty, &ReferenceImage)>,
    hand_custody: &str,
) -> (
    Option<photo_check::PhotoCheckFailure>,
    Vec<super::journey::living_scene::VisibleFeature>,
) {
    let input = PhotoCheckInput {
        judge,
        kind,
        bytes,
        format,
        prompt,
        scene_party,
        hand_custody,
    };
    let reading = read_photo(&input).await;
    photo_outcome(input, reading).await
}

async fn photo_outcome(
    input: PhotoCheckInput<'_>,
    reading: Result<photo_check::ScenePhotoReading, photo_check::PhotoCheckError>,
) -> (
    Option<photo_check::PhotoCheckFailure>,
    Vec<super::journey::living_scene::VisibleFeature>,
) {
    let PhotoCheckInput {
        kind,
        bytes,
        format,
        prompt,
        scene_party,
        ..
    } = input;
    match reading {
        Ok(photo_check::ScenePhotoReading {
            outcome: photo_check::PhotoCheckOutcome::Pass,
            visible_features,
        }) => {
            record_photo_check(kind, "pass", "none", scene_party);
            (None, visible_features)
        }
        Ok(photo_check::ScenePhotoReading {
            outcome: photo_check::PhotoCheckOutcome::Fail(failure),
            ..
        }) => {
            photo_check::retain_rejected_candidate(kind, bytes, format, failure, prompt).await;
            (Some(failure), Vec::new())
        }
        Err(error) => {
            record_photo_check(kind, "unjudged", error.label(), scene_party);
            (None, Vec::new())
        }
    }
}

fn record_photo_check(
    kind: photo_check::PhotoCheckKind,
    result: &str,
    reason: &str,
    scene_party: Option<(SceneParty, &ReferenceImage)>,
) {
    let mut line = format!(
        "PHOTO_CHECK kind={} result={result} reason={reason}",
        kind.label()
    );
    let mut fields = serde_json::Map::from_iter([
        ("kind".to_owned(), serde_json::json!(kind.label())),
        ("result".to_owned(), serde_json::json!(result)),
        ("reason".to_owned(), serde_json::json!(reason)),
    ]);
    if let Some((party, _)) = scene_party {
        line.push_str(&format!(" heroes={}", party.heroes()));
        fields.insert("heroes".to_owned(), serde_json::json!(party.heroes()));
    }
    super::actor::record_audit_diagnostic("gameplay.imagery.photo_check", line, fields, &[]);
}

fn failure_of(error: ImageEditError) -> ImageFailure {
    match error {
        ImageEditError::InvalidRequest => ImageFailure::InvalidRequest,
        ImageEditError::Refused => ImageFailure::Refused,
        ImageEditError::Provider(429) => ImageFailure::RateLimited,
        ImageEditError::Unauthorized | ImageEditError::Provider(_) => ImageFailure::Provider,
        ImageEditError::Transport(NativeHttpRefusal::Deadline) => ImageFailure::DeadlineExceeded,
        ImageEditError::Transport(NativeHttpRefusal::Unknown | NativeHttpRefusal::Cancelled) => {
            ImageFailure::Interrupted
        }
        ImageEditError::Transport(_) => ImageFailure::Provider,
        ImageEditError::Output => ImageFailure::BadOutput,
    }
}

fn usage_cost(usage: ImageUsage) -> u64 {
    usage.micro_dollars()
}

/// The only fields a span carries: the kind, the model and a count. Never a prompt or an image.
fn span_fields(
    kind: ImageKind,
    model: &'static str,
    references: usize,
) -> [(&'static str, String); 3] {
    [
        ("gameplay.imagery.kind", kind.label().to_owned()),
        ("gameplay.imagery.model", model.to_owned()),
        ("gameplay.imagery.references", references.to_string()),
    ]
}

fn image_request_diagnostics(
    key: &ImageKey,
    request: &ImageRequest,
) -> (
    String,
    serde_json::Map<String, serde_json::Value>,
    Vec<(&'static str, String)>,
) {
    let kind = request.kind.label();
    let key = key.as_str();
    let prompt_char_count = request.prompt.character_count();
    let reference_byte_lengths: Vec<usize> = request
        .references
        .iter()
        .map(|reference| reference.bytes.len())
        .collect();
    let reference_sha256: Vec<&str> = request
        .references
        .iter()
        .map(|reference| reference.sha256.as_str())
        .collect();
    let prompt_sha256 = format!("{:x}", Sha256::digest(request.prompt.as_str().as_bytes()));
    let reference_bytes_total = reference_byte_lengths.iter().sum::<usize>();
    let reference_count = reference_byte_lengths.len();
    let line = format!(
        "IMAGERY request kind={kind} id={key} prompt_chars={prompt_char_count} references={reference_count} reference_bytes={reference_bytes_total}"
    );
    let fields = serde_json::Map::from_iter([
        (
            "request_schema".to_owned(),
            serde_json::json!(REQUEST_SCHEMA),
        ),
        ("prompt_sha256".to_owned(), serde_json::json!(prompt_sha256)),
        (
            "reference_sha256".to_owned(),
            serde_json::json!(reference_sha256),
        ),
        ("kind".to_owned(), serde_json::json!(kind)),
        ("image_key".to_owned(), serde_json::json!(key)),
        (
            "prompt_char_count".to_owned(),
            serde_json::json!(prompt_char_count),
        ),
        (
            "reference_count".to_owned(),
            serde_json::json!(reference_count),
        ),
        (
            "reference_byte_lengths".to_owned(),
            serde_json::json!(reference_byte_lengths),
        ),
        (
            "reference_bytes_total".to_owned(),
            serde_json::json!(reference_bytes_total),
        ),
    ]);
    let attributes = vec![
        ("gameplay.imagery.request_schema", REQUEST_SCHEMA.to_owned()),
        ("gameplay.imagery.prompt_sha256", prompt_sha256),
        (
            "gameplay.imagery.reference_sha256",
            reference_sha256.join(","),
        ),
        ("gameplay.imagery.kind", kind.to_owned()),
        ("gameplay.imagery.image_key", key.to_owned()),
        (
            "gameplay.imagery.prompt_char_count",
            prompt_char_count.to_string(),
        ),
        (
            "gameplay.imagery.reference_count",
            reference_count.to_string(),
        ),
        (
            "gameplay.imagery.reference_byte_lengths",
            reference_byte_lengths
                .iter()
                .map(usize::to_string)
                .collect::<Vec<_>>()
                .join(","),
        ),
        (
            "gameplay.imagery.reference_bytes_total",
            reference_bytes_total.to_string(),
        ),
    ];
    (line, fields, attributes)
}

fn record_provider_timing(
    stage: &str,
    key: &ImageKey,
    wait: Duration,
    elapsed: Duration,
    failure: Option<ImageFailure>,
) {
    let failure = failure.map(ImageFailure::label);
    super::actor::record_audit_diagnostic(
        "gameplay.imagery.provider_timing",
        format!(
            "IMAGERY provider stage={stage} id={} wait_ms={} ms={} failure_category={}",
            key.as_str(),
            wait.as_millis(),
            elapsed.as_millis(),
            failure.unwrap_or("none"),
        ),
        serde_json::Map::from_iter([
            ("stage".into(), serde_json::json!(stage)),
            ("image_key".into(), serde_json::json!(key.as_str())),
            ("wait_ms".into(), serde_json::json!(wait.as_millis())),
            ("provider_ms".into(), serde_json::json!(elapsed.as_millis())),
            ("failure_category".into(), serde_json::json!(failure)),
        ]),
        &[("gameplay.imagery.image_key", key.as_str().to_owned())],
    );
}

fn record_image_result(
    kind: ImageKind,
    key: &ImageKey,
    elapsed: Duration,
    timing: RequestTiming,
    result: Result<&ImageKey, &ImageFailure>,
) {
    let (status, failure_category) = match result {
        Ok(_) => ("ready", None),
        Err(failure) => ("failed", Some(failure.label())),
    };
    let elapsed_ms = elapsed.as_millis();
    let line = format!(
        "IMAGERY {status} kind={} id={} wait_ms={} ms={} total_ms={elapsed_ms} failure_category={}",
        kind.label(),
        key.as_str(),
        timing.wait.as_millis(),
        timing.provider.as_millis(),
        failure_category.unwrap_or("none"),
    );
    let fields = serde_json::Map::from_iter([
        ("kind".to_owned(), serde_json::json!(kind.label())),
        ("image_key".to_owned(), serde_json::json!(key.as_str())),
        ("status".to_owned(), serde_json::json!(status)),
        (
            "failure_category".to_owned(),
            serde_json::json!(failure_category),
        ),
        ("elapsed_ms".to_owned(), serde_json::json!(elapsed_ms)),
        (
            "wait_ms".to_owned(),
            serde_json::json!(timing.wait.as_millis()),
        ),
        (
            "provider_ms".to_owned(),
            serde_json::json!(timing.provider.as_millis()),
        ),
    ]);
    let attributes = [
        ("gameplay.imagery.kind", kind.label().to_owned()),
        ("gameplay.imagery.image_key", key.as_str().to_owned()),
        ("gameplay.imagery.status", status.to_owned()),
        (
            "gameplay.imagery.failure_category",
            failure_category.unwrap_or("none").to_owned(),
        ),
        ("gameplay.imagery.elapsed_ms", elapsed_ms.to_string()),
    ];
    let event = match status {
        "ready" => "gameplay.imagery.render_ready",
        _ => "gameplay.imagery.render_failed",
    };
    crate::gameplay::actor::record_audit_diagnostic(event, line, fields, &attributes);
}

const REQUEST_SCHEMA: &str = "df-imagery/1";

fn request_key(request: &ImageRequest, model: &str, quality: &str, rating: &str) -> ImageKey {
    let mut hasher = Sha256::new();
    let mut part = |text: &str| {
        hasher.update(text.as_bytes());
        hasher.update(b"\n");
    };
    part(REQUEST_SCHEMA);
    part(request.kind.label());
    part(model);
    part(quality);
    part(rating);
    part(&format!(
        "{}x{} transparent={}",
        request.output.width, request.output.height, request.output.transparent
    ));
    for reference in &request.references {
        part(&reference.sha256);
    }
    part(request.prompt.as_str());
    if matches!(
        request.kind,
        ImageKind::ScenePlate | ImageKind::ScenePlateEdit
    ) {
        // Evidence belongs to this public source snapshot, including cache hits and restarts.
        part("scene-picture-evidence-1");
        part(request.prompt.scene_hand_custody());
    }
    ImageKey(
        hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
    )
}

fn read_cache_file(path: &Path) -> Option<Bytes> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAXIMUM_IMAGE_BYTES {
        return None;
    }
    std::fs::read(path).ok().map(Bytes::from)
}

fn write_cache_file(staging: &Path, target: &Path, bytes: &[u8]) -> io::Result<()> {
    if bytes.is_empty() {
        return Err(io::Error::other("an empty image is never cached"));
    }
    let written = std::fs::write(staging, bytes).and_then(|()| std::fs::rename(staging, target));
    if written.is_err() {
        // Best effort: a leftover staging file is the only residue of a failed write.
        let _removed = std::fs::remove_file(staging);
    }
    written
}

fn append_manifest(path: &Path, line: &ManifestLine) -> io::Result<()> {
    use std::io::Write;
    let mut text = serde_json::to_string(line).map_err(io::Error::other)?;
    text.push('\n');
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?
        .write_all(text.as_bytes())
}

fn read_bounded_text(path: &Path) -> Option<String> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > MAXIMUM_MANIFEST_BYTES {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

/// `GET /gameplay/imagery/{key}`.
pub(super) fn router(imagery: Imagery) -> Router {
    Router::new()
        .route("/gameplay/imagery/{key}", get(serve_image))
        .route(
            "/gameplay/imagery/{key}/figure/{index}",
            get(serve_focus_mask),
        )
        .with_state(imagery)
}

async fn serve_focus_mask(
    State(imagery): State<Imagery>,
    RoutePath((key, index)): RoutePath<(String, u32)>,
    headers: HeaderMap,
) -> Response {
    let Some(bytes) = imagery.focus_mask(&key, index).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let etag = format!("\"{key}-figure-{index}-v1\"");
    if headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        == Some(etag.as_str())
    {
        return (
            StatusCode::NOT_MODIFIED,
            [
                (header::ETAG, etag),
                (header::CACHE_CONTROL, CACHE_CONTROL.to_owned()),
            ],
        )
            .into_response();
    }
    (
        [
            (header::CONTENT_TYPE, "image/png".to_owned()),
            (header::CACHE_CONTROL, CACHE_CONTROL.to_owned()),
            (header::ETAG, etag),
        ],
        bytes,
    )
        .into_response()
}

pub(super) async fn serve_image(
    State(imagery): State<Imagery>,
    RoutePath(key): RoutePath<String>,
    headers: HeaderMap,
) -> Response {
    let started = std::time::Instant::now();
    let resource_id = safe_image_resource_id(&key);
    let browser_asset_id =
        df_observe::presentation::local_identity(&format!("/gameplay/imagery/{key}"));
    let (response, status_category, cache_result, byte_length, failure_category) =
        match imagery.image(&key).await {
            Lookup::Image { bytes, format } => {
                let etag = format!("\"{key}\"");
                let content_type = match format {
                    ImageFormat::Png => "image/png",
                    ImageFormat::Webp => "image/webp",
                    ImageFormat::Jpeg => "image/jpeg",
                };
                if headers
                    .get(header::IF_NONE_MATCH)
                    .and_then(|value| value.to_str().ok())
                    .is_some_and(|value| value == etag)
                {
                    (
                        (
                            StatusCode::NOT_MODIFIED,
                            [
                                (header::ETAG, etag),
                                (header::CACHE_CONTROL, CACHE_CONTROL.to_owned()),
                            ],
                        )
                            .into_response(),
                        "not_modified",
                        "client_revalidated",
                        0,
                        None,
                    )
                } else {
                    let byte_length = bytes.len();
                    (
                        (
                            [
                                (header::CONTENT_TYPE, content_type.to_owned()),
                                (header::CACHE_CONTROL, CACHE_CONTROL.to_owned()),
                                (header::ETAG, etag),
                            ],
                            bytes,
                        )
                            .into_response(),
                        "ready",
                        "server_hit",
                        byte_length,
                        None,
                    )
                }
            }
            Lookup::Unregistered => (
                StatusCode::NOT_FOUND.into_response(),
                "unregistered",
                "not_applicable",
                0,
                Some("unregistered"),
            ),
            Lookup::NotReady => (
                StatusCode::SERVICE_UNAVAILABLE.into_response(),
                "not_ready",
                "miss",
                0,
                Some("not_ready"),
            ),
        };
    record_media_delivery(
        "image",
        &resource_id,
        browser_asset_id,
        status_category,
        failure_category,
        response.status().as_u16(),
        byte_length,
        cache_result,
        started.elapsed().as_millis(),
    );
    response
}

fn safe_image_resource_id(key: &str) -> String {
    if key.len() == 64
        && key
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        key.to_owned()
    } else {
        df_observe::presentation::local_identity(key).to_string()
    }
}

#[allow(clippy::too_many_arguments)]
fn record_media_delivery(
    resource_type: &'static str,
    resource_id: &str,
    browser_asset_id: u64,
    status: &'static str,
    failure_category: Option<&'static str>,
    http_status: u16,
    byte_length: usize,
    cache_result: &'static str,
    elapsed_ms: u128,
) {
    let line = format!(
        "MEDIA_DELIVERY type={resource_type} resource_id={resource_id} browser_asset_id={browser_asset_id} status={status} http_status={http_status} bytes={byte_length} cache_result={cache_result} elapsed_ms={elapsed_ms} failure_category={}",
        failure_category.unwrap_or("none"),
    );
    let fields = serde_json::Map::from_iter([
        ("resource_type".to_owned(), serde_json::json!(resource_type)),
        ("resource_id".to_owned(), serde_json::json!(resource_id)),
        (
            "browser_asset_id".to_owned(),
            serde_json::json!(browser_asset_id),
        ),
        (
            "stage".to_owned(),
            serde_json::json!(if failure_category.is_some() {
                "failed"
            } else {
                "delivered"
            }),
        ),
        (
            "status".to_owned(),
            serde_json::json!(if failure_category.is_some() {
                "failed"
            } else {
                "delivered"
            }),
        ),
        ("status_category".to_owned(), serde_json::json!(status)),
        (
            "failure_category".to_owned(),
            serde_json::json!(failure_category),
        ),
        ("provider".to_owned(), serde_json::json!("image_cache")),
        ("http_status".to_owned(), serde_json::json!(http_status)),
        ("byte_length".to_owned(), serde_json::json!(byte_length)),
        ("cache_result".to_owned(), serde_json::json!(cache_result)),
        ("elapsed_ms".to_owned(), serde_json::json!(elapsed_ms)),
    ]);
    let attributes = [
        ("gameplay.resource.type", resource_type.to_owned()),
        ("gameplay.resource.id", resource_id.to_owned()),
        (
            "gameplay.media.browser_asset_id",
            browser_asset_id.to_string(),
        ),
        (
            "gameplay.resource.stage",
            if failure_category.is_some() {
                "failed".to_owned()
            } else {
                "delivered".to_owned()
            },
        ),
        (
            "gameplay.resource.status",
            if failure_category.is_some() {
                "failed".to_owned()
            } else {
                "delivered".to_owned()
            },
        ),
        ("gameplay.media.status_category", status.to_owned()),
        (
            "gameplay.resource.failure_category",
            failure_category.unwrap_or("none").to_owned(),
        ),
        ("gameplay.media.provider", "image_cache".to_owned()),
        ("gameplay.media.http_status", http_status.to_string()),
        ("gameplay.resource.byte_length", byte_length.to_string()),
        ("gameplay.media.cache_result", cache_result.to_owned()),
        ("gameplay.resource.elapsed_ms", elapsed_ms.to_string()),
    ];
    crate::gameplay::actor::record_audit_diagnostic(
        "gameplay.media.delivery",
        line,
        fields,
        &attributes,
    );
}

#[cfg(test)]
#[path = "imagery_tests.rs"]
pub(super) mod tests;

#[cfg(test)]
#[path = "imagery_live_tests.rs"]
mod live_tests;
