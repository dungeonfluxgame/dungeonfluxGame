//! Gameplay image intent and authorized delivery over the mounted durable ImageService.
//! Shared owners execute suppliers, pixels, retries, cost accounting and publication.
pub(in crate::gameplay) mod analysis;
mod artifact;
mod authorized_service;
mod canonical_prompt;
mod cast_sheet;
pub(in crate::gameplay) mod central;
pub(in crate::gameplay) mod figure_masks;
mod legacy_manifest;
mod mural_prompt;
pub(super) mod narrative_focus;
mod party_sheet;
mod photo_check;
mod photo_edit;
mod plate_prompt;
mod prompt;
mod quota_notice;
mod references;
mod scene_prompt;
mod service_request;
#[cfg(test)]
pub(in crate::gameplay) mod test_support;
#[cfg(test)]
pub(super) use canonical_prompt::canonical_room;
pub(super) use canonical_prompt::{
    RoomArchitectureContext, canonical_cast_member, canonical_character, canonical_npc,
    canonical_room_content, canonical_room_with_context,
};
pub(super) use scene_prompt::{graph_edit, item_picture, model_plate, state_edit, story_beat_edit};

use super::startup_configuration::{ContentRating, ImageryOff, ImagerySettings};
#[allow(
    unused_imports,
    reason = "the investigation task (a follow-up) is the first caller"
)]
pub(super) use artifact::{ArtifactSubject, SubjectRefusal, close_up_request};
use axum::{
    Router,
    extract::{Path as RoutePath, State},
    http::{HeaderMap, StatusCode, Uri, header},
    response::{IntoResponse, Response},
    routing::get,
};
use bytes::Bytes;
use df_providers::{ImageFormat, ImageQuality, LocatedHead};
use legacy_manifest::{
    legacy_file_name, legacy_stored_formats, manifest_records, manifest_tail, read_cache_file,
};
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
use sha2::{Digest, Sha256};
use std::{
    io,
    path::Path,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};
#[cfg(test)]
use std::{
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
// PLAY-V09: the scene plate's party sheet and prompts.
pub(super) use cast_sheet::{CastIdentity, cast_sheet};
pub(super) use mural_prompt::party_mural;
pub(super) use party_sheet::party_sheet;
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
pub(super) use quota_notice::{QuotaNoticeRegistration, register_quota_notice};
pub(super) use references::{AdventureManifest, ReferenceImage, ReferenceStore};
pub(in crate::gameplay) use references::{
    ApprovedPortraitSource, CommittedPortraitProof, DevelopmentSavedPhotoSource,
    TrustedReferenceOrigin,
};

/// Words for a medium other than photography. No image or video prompt names one, even to forbid
/// it: naming another medium steers the model toward it (owner, 2026-10-17).
#[cfg(test)]
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
const CACHE_CONTROL: &str = "private, max-age=86400, immutable";
const MAXIMUM_ISSUED_KEYS: usize = 65_536;
#[cfg(test)]
const MAXIMUM_IMAGE_BYTES: u64 = 6 * 1024 * 1024;
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

/// Frozen template family retained by neutral service requests and cost attribution.
pub(in crate::gameplay) const SCENE_REFRESH_TEMPLATE_REVISION: u32 = 0xDF01_0003;

impl ImageKind {
    pub(in crate::gameplay) const fn template_revision(self) -> u32 {
        match self {
            Self::CanonicalCharacter => 0xDF01_0001,
            Self::CanonicalRoom => 0xDF01_0002,
            Self::SceneRefresh => SCENE_REFRESH_TEMPLATE_REVISION,
            Self::CastExpression => 0xDF01_0004,
            Self::ArtifactCloseUp => 0xDF01_0005,
            Self::ScenePlate => 0xDF01_0006,
            Self::ScenePlateEdit => 0xDF01_0007,
        }
    }

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

/// The provider's smallest accepted picture (`df_providers` image edit validation): sides are
/// multiples of 16 and the area is at least this many pixels.
const PROVIDER_MINIMUM_PIXELS: u64 = 655_360;

/// The smallest provider-accepted size with the same shape as `output`.
fn smallest_like(output: OutputSpec) -> OutputSpec {
    let area = f64::from(output.width) * f64::from(output.height);
    let scale = (PROVIDER_MINIMUM_PIXELS as f64 / area).sqrt().min(1.0);
    let side = |length: u32| ((f64::from(length) * scale / 16.0).ceil() as u32 * 16).max(16);
    let (mut width, mut height) = (side(output.width), side(output.height));
    while u64::from(width) * u64::from(height) < PROVIDER_MINIMUM_PIXELS {
        if width <= height {
            width += 16;
        } else {
            height += 16;
        }
    }
    OutputSpec {
        width,
        height,
        transparent: output.transparent,
    }
}

/// The quality and size a kind is drawn at. Dev play-tests draw every kind at low quality and the
/// smallest size the provider accepts in its shape (owner 2026-10-24: the cheapest dev setting);
/// production draws everything at the configured quality and full size.
pub(in crate::gameplay) fn drawn_for(
    profile: super::startup_configuration::ImageryProfile,
    configured: ImageQuality,
    _kind: ImageKind,
    output: OutputSpec,
) -> (ImageQuality, OutputSpec) {
    use super::startup_configuration::ImageryProfile;
    match profile {
        ImageryProfile::Production => (configured, output),
        ImageryProfile::Dev => (ImageQuality::Low, smallest_like(output)),
    }
}

#[cfg(test)]
mod drawn_tests {
    use super::*;
    use crate::gameplay::startup_configuration::ImageryProfile;

    #[test]
    fn the_dev_profile_draws_every_kind_low_at_the_smallest_accepted_size() {
        let dev = |kind, output| drawn_for(ImageryProfile::Dev, ImageQuality::High, kind, output);
        let size = |spec: OutputSpec| (spec.width, spec.height);
        let (quality, plate) = dev(ImageKind::ScenePlate, OutputSpec::PLATE);
        assert_eq!((quality, size(plate)), (ImageQuality::Low, (1088, 608)));
        assert_eq!(
            size(dev(ImageKind::CanonicalRoom, OutputSpec::BACKDROP).1),
            (1088, 608)
        );
        assert_eq!(
            size(dev(ImageKind::ArtifactCloseUp, OutputSpec::CLOSE_UP).1),
            (944, 704)
        );
        let (quality, cutout) = dev(ImageKind::CanonicalCharacter, OutputSpec::CUTOUT);
        assert_eq!((quality, size(cutout)), (ImageQuality::Low, (672, 992)));
        assert!(cutout.transparent);
        for spec in [
            OutputSpec::PLATE,
            OutputSpec::BACKDROP,
            OutputSpec::CLOSE_UP,
            OutputSpec::CUTOUT,
        ] {
            let small = smallest_like(spec);
            assert!(u64::from(small.width) * u64::from(small.height) >= PROVIDER_MINIMUM_PIXELS);
            assert!(small.width.is_multiple_of(16) && small.height.is_multiple_of(16));
        }
        for kind in [
            ImageKind::ScenePlate,
            ImageKind::CanonicalRoom,
            ImageKind::ArtifactCloseUp,
        ] {
            assert_eq!(
                drawn_for(
                    ImageryProfile::Production,
                    ImageQuality::High,
                    kind,
                    OutputSpec::PLATE
                ),
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
    /// Current host authority registrations are full; no authorization was evicted.
    AuthorityCapacity,
    DeadlineExceeded,
    /// The provider's moderation declined it. It is never reworded or rerouted.
    Refused,
    /// The provider failed, rejected the credential or was rate limited.
    Provider,
    RateLimited,
    /// The provider's account has no credit or quota left (a structured 429). Waiting does not
    /// clear it, so it is never retried, and it is not remembered as a permanent failure.
    QuotaExhausted,
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
            Self::AuthorityCapacity => "authority_capacity",
            Self::DeadlineExceeded => "deadline_exceeded",
            Self::Refused => "refused",
            Self::Provider => "provider_failed",
            Self::RateLimited => "rate_limited",
            Self::QuotaExhausted => "quota_exhausted",
            Self::BflSubmit => "bfl_submit_failed",
            Self::BflPoll => "bfl_poll_failed",
            Self::BflDelivery => "bfl_delivery_failed",
            Self::BflDownload => "bfl_download_failed",
            Self::BadOutput => "bad_output",
            Self::Interrupted => "interrupted",
        }
    }
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

/// The 32 bytes of a lowercase hexadecimal key.
fn key_digest(key: &str) -> Option<[u8; 32]> {
    let bytes = key.as_bytes();
    if bytes.len() != 64 {
        return None;
    }
    let (pairs, _) = bytes.as_chunks::<2>();
    let mut digest = [0_u8; 32];
    for (slot, pair) in digest.iter_mut().zip(pairs) {
        *slot = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
    }
    Some(digest)
}

/// Accepted photographic evidence, retained beside its immutable image key across restarts.
#[derive(serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(in crate::gameplay) struct PhotoEvidence {
    pub(in crate::gameplay) place: String,
    pub(in crate::gameplay) source_hash: String,
    pub(in crate::gameplay) visible_features: Vec<super::journey::living_scene::VisibleFeature>,
}

struct Inner {
    central: Option<Arc<central::CentralImages>>,
    disabled_reason: ImageryOff,
    hub: super::resources::Hub,
}
/// Gameplay intent and authorized delivery through the mounted durable image service.
#[derive(Clone)]
pub(super) struct Imagery {
    inner: Arc<Inner>,
}
fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
/// A read-only display indication from actual live host quota state.
pub(super) fn operational_notice() -> Option<&'static str> {
    quota_notice::operational_notice()
}
impl Imagery {
    pub(in crate::gameplay) fn from_central(
        central: central::CentralImages,
        hub: super::resources::Hub,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                central: Some(Arc::new(central)),
                disabled_reason: ImageryOff::Disabled,
                hub,
            }),
        }
    }

    pub(in crate::gameplay) async fn pin_reference(
        &self,
        reference: &ReferenceImage,
    ) -> Result<df_imagery::SourceRef, ImageFailure> {
        self.inner
            .central
            .as_ref()
            .ok_or(ImageFailure::Off)?
            .pin_reference(reference)
            .await
    }

    pub(in crate::gameplay) async fn reference(
        &self,
        key: &ImageKey,
    ) -> Result<ReferenceImage, ImageFailure> {
        self.inner
            .central
            .as_ref()
            .ok_or(ImageFailure::Off)?
            .reference(key)
            .await
    }

    pub(in crate::gameplay) async fn for_local_creator(
        &self,
        credential: [u8; 32],
    ) -> Result<Self, ImageFailure> {
        let central = self
            .inner
            .central
            .as_ref()
            .ok_or(ImageFailure::Off)?
            .for_local_creator(credential)
            .await?;
        Ok(Self::from_central(central, self.inner.hub.clone()))
    }

    pub(in crate::gameplay) fn observer(
        &self,
    ) -> Option<Arc<dyn df_imagery::telemetry::ImageObserver>> {
        self.inner
            .central
            .as_ref()
            .map(|central| central.observer())
    }

    pub(in crate::gameplay) async fn displayed_observation(
        &self,
        identity: u64,
    ) -> Result<Option<super::image_observability::PublishedObservation>, ImageFailure> {
        match &self.inner.central {
            Some(central) => central.displayed_observation(identity).await,
            None => Ok(None),
        }
    }

    pub(in crate::gameplay) async fn committed_portrait(
        &self,
        asset: &df_model::checkpoint::AssetReference,
        proof: Arc<dyn CommittedPortraitProof>,
    ) -> Result<ReferenceImage, ImageFailure> {
        self.inner
            .central
            .as_ref()
            .ok_or(ImageFailure::Off)?
            .committed_portrait(asset, proof)
            .await
    }

    pub(in crate::gameplay) fn reviewed_input_limit(&self) -> Result<u64, ImageFailure> {
        self.inner
            .central
            .as_ref()
            .ok_or(ImageFailure::Off)
            .map(|central| central.reviewed_input_limit())
    }

    pub(in crate::gameplay) async fn consumer_context(
        &self,
    ) -> Result<central::ConsumerContext, ImageFailure> {
        self.inner
            .central
            .as_ref()
            .ok_or(ImageFailure::Off)?
            .consumer_context()
            .await
    }

    pub(in crate::gameplay) async fn published_reference(
        &self,
        result: &df_imagery::publication::PublishedImage,
        audience: df_imagery::Audience,
    ) -> Result<ReferenceImage, ImageFailure> {
        self.inner
            .central
            .as_ref()
            .ok_or(ImageFailure::Off)?
            .published_reference(result, audience)
            .await
    }

    pub(in crate::gameplay) async fn usage_for_reference(
        &self,
        reference: &ReferenceImage,
    ) -> Result<Option<df_imagery::runtime::ImageUsageSummary>, ImageFailure> {
        self.inner
            .central
            .as_ref()
            .ok_or(ImageFailure::Off)?
            .usage_for_reference(reference)
            .await
    }

    #[cfg(test)]
    pub(in crate::gameplay) async fn read_transform(
        &self,
        subject: &str,
        purpose: df_imagery::PurposedUse,
        spec: df_imagery::TransformSpec,
        output: df_imagery::RasterOutputSpec,
    ) -> Result<Option<ReferenceImage>, ImageFailure> {
        self.inner
            .central
            .as_ref()
            .ok_or(ImageFailure::Off)?
            .read_transform(subject, purpose, spec, output)
            .await
    }

    pub(in crate::gameplay) async fn transform(
        &self,
        subject: &str,
        purpose: df_imagery::PurposedUse,
        spec: df_imagery::TransformSpec,
        output: df_imagery::RasterOutputSpec,
    ) -> Result<ReferenceImage, ImageFailure> {
        self.inner
            .central
            .as_ref()
            .ok_or(ImageFailure::Off)?
            .transform(subject, purpose, spec, output)
            .await
    }

    pub(in crate::gameplay) async fn frame(
        &self,
        subject: &str,
        prompt: &str,
        references: &[ReferenceImage],
        output: OutputSpec,
        freshness: df_imagery::Freshness,
    ) -> Result<ReferenceImage, ImageFailure> {
        self.inner
            .central
            .as_ref()
            .ok_or(ImageFailure::Off)?
            .frame(subject, prompt, references, output, freshness)
            .await
    }

    /// Disabled harnesses retain their hub; enabled work requires an actual service mount.
    pub(super) fn disabled(reason: ImageryOff) -> Self {
        Self::start_in_hub(
            ImagerySettings::Off(reason),
            None,
            super::resources::Hub::new(),
        )
        .expect("disabled imagery cannot require a mount")
    }
    pub(super) fn start_in_hub(
        settings: ImagerySettings,
        _manifest: Option<&Path>,
        hub: super::resources::Hub,
    ) -> Result<Self, io::Error> {
        match settings {
            ImagerySettings::Off(reason) => Ok(Self {
                inner: Arc::new(Inner {
                    central: None,
                    disabled_reason: reason,
                    hub,
                }),
            }),
            ImagerySettings::On(_) => Err(io::Error::other(
                "enabled imagery requires the mounted image service",
            )),
        }
    }
    pub(super) fn startup_line(&self) -> String {
        match &self.inner.central {
            Some(central) => format!(
                "IMAGERY {} service=durable",
                if central.is_on() { "on" } else { "off" }
            ),
            None => format!("IMAGERY off ({})", self.inner.disabled_reason.reason()),
        }
    }
    pub(super) fn is_on(&self) -> bool {
        self.inner
            .central
            .as_ref()
            .is_some_and(|central| central.is_on())
    }
    pub(super) fn flux_prompts(&self) -> bool {
        self.inner
            .central
            .as_ref()
            .is_some_and(|central| central.flux_prompts())
    }
    pub(super) fn key_for(&self, request: &ImageRequest) -> ImageKey {
        self.inner.central.as_ref().map_or_else(
            || request_key(request, "off", "off", "off"),
            |central| central.key_for(request),
        )
    }
    pub(super) fn hub(&self) -> &super::resources::Hub {
        &self.inner.hub
    }
    pub(super) fn model_label(&self) -> String {
        self.inner
            .central
            .as_ref()
            .and_then(|central| central.model_label().ok())
            .unwrap_or_else(|| "off".to_owned())
    }
    pub(super) async fn locate_heads(
        &self,
        key: &ImageKey,
        face: Option<&ReferenceImage>,
        targets: &[(&str, &str)],
    ) -> Result<Vec<LocatedHead>, ImageFailure> {
        analysis::heads(self, key, face, targets)
            .await
            .map(|observed| observed.value)
    }
    pub(in crate::gameplay) async fn photo_evidence(
        &self,
        key: &ImageKey,
    ) -> Result<Option<PhotoEvidence>, ImageFailure> {
        self.inner
            .central
            .as_ref()
            .ok_or(ImageFailure::Off)?
            .photo_evidence(key)
            .await
    }
    pub(super) async fn plate_staging(&self, key: &ImageKey) -> Vec<StagedPerson> {
        let Some(central) = &self.inner.central else {
            return Vec::new();
        };
        match central.plate_staging(key).await {
            Ok(people) => people,
            Err(failure) => {
                super::actor::record_audit_line(
                    "gameplay.imagery.staging_unavailable",
                    format!(
                        "PLATE staging key={} outcome={}",
                        key.as_str(),
                        failure.label()
                    ),
                );
                Vec::new()
            }
        }
    }
    pub(in crate::gameplay) async fn read_presentation_annotation(
        &self,
        key: &ImageKey,
        kind: super::image_references::PresentationAnnotation,
    ) -> Result<Option<Vec<u8>>, ImageFailure> {
        self.inner
            .central
            .as_ref()
            .ok_or(ImageFailure::Off)?
            .read_annotation(key, kind)
            .await
    }
    pub(in crate::gameplay) async fn write_presentation_annotation(
        &self,
        key: &ImageKey,
        kind: super::image_references::PresentationAnnotation,
        note: Vec<u8>,
    ) -> Result<(), ImageFailure> {
        self.inner
            .central
            .as_ref()
            .ok_or(ImageFailure::Off)?
            .write_annotation(key, kind, note)
            .await
    }
    pub(super) async fn read_note(&self, key: &ImageKey) -> Option<Vec<u8>> {
        match self
            .read_presentation_annotation(
                key,
                super::image_references::PresentationAnnotation::Layout,
            )
            .await
        {
            Ok(note) => note,
            Err(failure) => {
                super::actor::record_audit_line(
                    "gameplay.imagery.layout_unavailable",
                    format!(
                        "PLATE layout key={} outcome={}",
                        key.as_str(),
                        failure.label()
                    ),
                );
                None
            }
        }
    }
    pub(super) async fn write_note(&self, key: &ImageKey, note: Vec<u8>) {
        if let Err(failure) = self
            .write_presentation_annotation(
                key,
                super::image_references::PresentationAnnotation::Layout,
                note,
            )
            .await
        {
            super::actor::record_audit_line(
                "gameplay.imagery.layout_unavailable",
                format!(
                    "PLATE layout key={} outcome={}",
                    key.as_str(),
                    failure.label()
                ),
            );
        }
    }
    pub(super) async fn generate(
        &self,
        subject: &str,
        request: ImageRequest,
    ) -> Result<ImageKey, ImageFailure> {
        let priority = ImagePriority::for_kind(request.kind);
        self.generate_with_priority(subject, request, priority)
            .await
    }
    pub(in crate::gameplay) async fn generate_with_priority(
        &self,
        subject: &str,
        request: ImageRequest,
        priority: ImagePriority,
    ) -> Result<ImageKey, ImageFailure> {
        let central = self.inner.central.as_ref().ok_or(ImageFailure::Off)?;
        let (line, fields, attributes) =
            image_request_diagnostics(&central.key_for(&request), &request);
        super::actor::record_audit_diagnostic(
            "gameplay.imagery.render_requested",
            line,
            fields,
            &attributes,
        );
        central.generate(subject, &request, priority).await
    }
    /// Domain-specific purpose and freshness stay inside the admitted, frozen service request.
    pub(in crate::gameplay) async fn generate_for_purpose(
        &self,
        subject: &str,
        request: ImageRequest,
        purpose: df_imagery::Purpose,
        priority: ImagePriority,
        freshness: df_imagery::Freshness,
    ) -> Result<ImageKey, ImageFailure> {
        self.inner
            .central
            .as_ref()
            .ok_or(ImageFailure::Off)?
            .generate_for_purpose(subject, &request, purpose, priority, freshness)
            .await
    }
    /// Only a currently trusted photograph enters the actual catalog import flow.
    pub(super) async fn register_reviewed(
        &self,
        _subject: &str,
        reference: ReferenceImage,
    ) -> Result<ImageKey, ImageFailure> {
        self.inner
            .central
            .as_ref()
            .ok_or(ImageFailure::Off)?
            .register_reference(&reference)
            .await
    }
    #[cfg(test)]
    async fn render(&self, request: ImageRequest) -> Result<ImageKey, ImageFailure> {
        self.generate("render", request).await
    }
    pub(super) async fn image(&self, key: &str) -> Lookup {
        let Some(central) = &self.inner.central else {
            return Lookup::Unregistered;
        };
        let Some(key) = ImageKey::parse(key) else {
            return Lookup::Unregistered;
        };
        match central.served_reference(&key).await {
            Ok(reference) => Lookup::Image {
                bytes: reference.bytes,
                format: reference.format,
            },
            Err(ImageFailure::Refused | ImageFailure::InvalidRequest | ImageFailure::Off) => {
                Lookup::Unregistered
            }
            Err(_) => Lookup::NotReady,
        }
    }
    pub(super) async fn peek(&self, request: &ImageRequest) -> Option<ImageKey> {
        let central = self.inner.central.as_ref()?;
        let key = central.key_for(request);
        central.reference(&key).await.ok()?;
        Some(key)
    }
}
/// The only fields a span carries: the kind, the model and a count. Never a prompt or an image.
#[cfg(test)]
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
    uri: Uri,
) -> Response {
    let Some(grant) = super::media_grant::verify_request(
        &headers,
        &uri,
        &key,
        super::media_grant::Audience::Imagery,
        &[
            super::media_grant::PrincipalAudience::Seat,
            super::media_grant::PrincipalAudience::Display,
        ],
    ) else {
        return super::media_grant::concealed_not_found();
    };
    let Some(bytes) = imagery.focus_mask(&key, index).await else {
        return super::media_grant::concealed_not_found();
    };
    let etag = format!("\"{:x}\"", Sha256::digest(&bytes));
    if headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        == Some(etag.as_str())
    {
        return super::media_grant::secure_media_response(
            (
                StatusCode::NOT_MODIFIED,
                [
                    (header::ETAG, etag),
                    (header::CACHE_CONTROL, CACHE_CONTROL.to_owned()),
                ],
            )
                .into_response(),
            grant.remaining,
        );
    }
    super::media_grant::secure_media_response(
        (
            [
                (header::CONTENT_TYPE, "image/png".to_owned()),
                (header::CACHE_CONTROL, CACHE_CONTROL.to_owned()),
                (header::ETAG, etag),
            ],
            bytes,
        )
            .into_response(),
        grant.remaining,
    )
}

pub(super) async fn serve_image(
    State(imagery): State<Imagery>,
    RoutePath(key): RoutePath<String>,
    headers: HeaderMap,
    uri: Uri,
) -> Response {
    let Some(grant) = super::media_grant::verify_request(
        &headers,
        &uri,
        &key,
        super::media_grant::Audience::Imagery,
        &[
            super::media_grant::PrincipalAudience::Seat,
            super::media_grant::PrincipalAudience::Display,
        ],
    ) else {
        return super::media_grant::concealed_not_found();
    };
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
    super::media_grant::secure_media_response(response, grant.remaining)
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

#[cfg(test)]
mod session_delivery_tests;
