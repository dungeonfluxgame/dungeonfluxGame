//! `MediaRequestKey` (planning/asset-storage-research.md 3.4, 6.4): the one dedupe key of the
//! catalog. SHA-256 over the schema label and a fixed sequence of fields, each a big-endian `u32`
//! length followed by its bytes, ending with the full output audience. Length prefixes make the
//! encoding injective: no two different field lists encode to the same bytes, unlike today's
//! newline-joined `df-imagery/1` text.

use crate::audience::Audience;
use sha2::{Digest, Sha256};
use std::fmt;

/// The schema label hashed first. A change of fields is a new label, so old keys never collide.
pub const MEDIA_REQUEST_SCHEMA: &str = "df-imagery/2";

/// The inputs of one generated picture.
#[derive(Clone, Copy, Debug)]
pub struct MediaRequest<'a> {
    pub kind: &'a str,
    pub model: &'a str,
    pub quality: &'a str,
    pub rating: &'a str,
    pub width_px: u32,
    pub height_px: u32,
    pub is_transparent: bool,
    /// Reference image digests in request order.
    pub references: &'a [&'a str],
    pub prompt: &'a str,
    /// The public scene snapshot a scene plate is evidence for, when the kind has one.
    pub scene_evidence: Option<&'a str>,
    pub audience: &'a Audience,
}

/// SHA-256 of the canonical encoding.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct MediaRequestKey([u8; 32]);

impl MediaRequestKey {
    pub fn derive(request: &MediaRequest<'_>) -> Self {
        let mut encoded = Vec::with_capacity(256 + request.prompt.len());
        field(&mut encoded, MEDIA_REQUEST_SCHEMA.as_bytes());
        field(&mut encoded, request.kind.as_bytes());
        field(&mut encoded, request.model.as_bytes());
        field(&mut encoded, request.quality.as_bytes());
        field(&mut encoded, request.rating.as_bytes());
        field(&mut encoded, &request.width_px.to_be_bytes());
        field(&mut encoded, &request.height_px.to_be_bytes());
        field(&mut encoded, &[u8::from(request.is_transparent)]);
        field(
            &mut encoded,
            &length(request.references.len()).to_be_bytes(),
        );
        for reference in request.references {
            field(&mut encoded, reference.as_bytes());
        }
        field(&mut encoded, request.prompt.as_bytes());
        match request.scene_evidence {
            Some(evidence) => {
                field(&mut encoded, &[1]);
                field(&mut encoded, evidence.as_bytes());
            }
            None => field(&mut encoded, &[0]),
        }
        let mut audience = Vec::with_capacity(64);
        request.audience.encode_into(&mut audience);
        field(&mut encoded, &audience);
        Self(Sha256::digest(&encoded).into())
    }

    /// SHA-256 over `df-imagery/3` and the recipe fields in declaration order, each a
    /// big-endian `u32` length followed by its bytes, with a one-byte tag before each optional
    /// field and a `u32` count before the ordered references, ending with the full output
    /// audience. Queue position, deadlines and concurrency limits are not hashed. Unlike the
    /// legacy [`MediaRequestKey::derive`], an unrepresentable length fails instead of
    /// saturating, so two different recipes can never share a key through truncation.
    pub fn derive_recipe(request: &MediaRecipeRequest<'_>) -> Result<Self, MediaKeyError> {
        let mut hasher = Sha256::new();
        feed(&mut hasher, "schema", MEDIA_RECIPE_SCHEMA.as_bytes())?;
        feed(
            &mut hasher,
            "operation_kind",
            request.operation_kind.as_bytes(),
        )?;
        feed(&mut hasher, "purpose", request.purpose.as_bytes())?;
        feed(&mut hasher, "owner_scope", request.owner_scope.as_bytes())?;
        feed(
            &mut hasher,
            "backend_profile",
            request.backend_profile.as_bytes(),
        )?;
        feed(&mut hasher, "model", request.model.as_bytes())?;
        feed(&mut hasher, "quality", request.quality.as_bytes())?;
        feed(&mut hasher, "rating", request.rating.as_bytes())?;
        feed_u32(&mut hasher, request.width_px);
        feed_u32(&mut hasher, request.height_px);
        feed(
            &mut hasher,
            "output_format",
            request.output_format.as_bytes(),
        )?;
        feed(&mut hasher, "alpha_policy", request.alpha_policy.as_bytes())?;
        feed_u32(&mut hasher, request.template_revision);
        feed_u32(&mut hasher, request.route_revision);
        feed_u32(&mut hasher, request.acceptance_revision);
        feed_u32(&mut hasher, request.executor_revision);
        match request.source_binding {
            Some(binding) => {
                feed_tag(&mut hasher, 1);
                feed(&mut hasher, "source_binding", binding.as_bytes())?;
            }
            None => feed_tag(&mut hasher, 0),
        }
        let count = u32::try_from(request.references.len()).map_err(|_| {
            MediaKeyError::TooManyReferences {
                count: request.references.len(),
            }
        })?;
        feed_u32(&mut hasher, count);
        for reference in request.references {
            feed(&mut hasher, "reference", reference.as_bytes())?;
        }
        feed(&mut hasher, "prompt", request.prompt.as_bytes())?;
        feed(&mut hasher, "pipeline_bytes", request.pipeline_bytes)?;
        match request.regeneration_nonce {
            Some(nonce) => {
                feed_tag(&mut hasher, 1);
                feed(&mut hasher, "regeneration_nonce", nonce.as_bytes())?;
            }
            None => feed_tag(&mut hasher, 0),
        }
        let mut audience = Vec::with_capacity(64);
        request.audience.encode_into(&mut audience);
        feed(&mut hasher, "audience", &audience)?;
        Ok(Self(hasher.finalize().into()))
    }

    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// The recipe schema label hashed first by [`MediaRequestKey::derive_recipe`]. A change of
/// fields is a new label, so recipe keys never collide with `df-imagery/2` legacy keys.
pub const MEDIA_RECIPE_SCHEMA: &str = "df-imagery/3";

/// The frozen inputs of one generated or transformed picture (planning/image-generation-
/// architecture.md "Identity, cache and durable attempts",
/// planning/image-transformations-architecture.md "Identity, ownership and publication").
///
/// `pipeline_bytes` holds the ordered canonical pipeline bytes produced by `df-imagery`
/// (ordered normalized operations plus encoder and color settings), never a `Debug` string:
/// debug formatting is not canonical and would merge distinct pipelines into one key. Queue
/// position, deadlines and concurrency limits are execution state, not pixel identity, and stay
/// out of the key.
#[derive(Clone, Copy)]
pub struct MediaRecipeRequest<'a> {
    pub operation_kind: &'a str,
    pub purpose: &'a str,
    pub owner_scope: &'a str,
    pub backend_profile: &'a str,
    pub model: &'a str,
    pub quality: &'a str,
    pub rating: &'a str,
    pub width_px: u32,
    pub height_px: u32,
    pub output_format: &'a str,
    pub alpha_policy: &'a str,
    pub template_revision: u32,
    pub route_revision: u32,
    pub acceptance_revision: u32,
    pub executor_revision: u32,
    pub source_binding: Option<&'a str>,
    /// Reference image digests in request order.
    pub references: &'a [&'a str],
    pub prompt: &'a str,
    /// Ordered canonical pipeline bytes from `df-imagery`; see the struct docs.
    pub pipeline_bytes: &'a [u8],
    /// Present only for an explicit regeneration; `None` reuses a compatible result.
    pub regeneration_nonce: Option<&'a str>,
    pub audience: &'a Audience,
}

// The prompt, source binding, pipeline bytes and regeneration nonce can carry private
// generation basis, so `Debug` names those fields without printing their contents.
impl fmt::Debug for MediaRecipeRequest<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MediaRecipeRequest")
            .field("operation_kind", &self.operation_kind)
            .field("purpose", &self.purpose)
            .field("owner_scope", &self.owner_scope)
            .field("backend_profile", &self.backend_profile)
            .field("model", &self.model)
            .field("quality", &self.quality)
            .field("rating", &self.rating)
            .field("width_px", &self.width_px)
            .field("height_px", &self.height_px)
            .field("output_format", &self.output_format)
            .field("alpha_policy", &self.alpha_policy)
            .field("template_revision", &self.template_revision)
            .field("route_revision", &self.route_revision)
            .field("acceptance_revision", &self.acceptance_revision)
            .field("executor_revision", &self.executor_revision)
            .field("source_binding_len", &self.source_binding.map(str::len))
            .field("reference_count", &self.references.len())
            .field("prompt_len", &self.prompt.len())
            .field("pipeline_bytes_len", &self.pipeline_bytes.len())
            .field(
                "regeneration_nonce_len",
                &self.regeneration_nonce.map(str::len),
            )
            .field("audience", &self.audience)
            .finish()
    }
}

/// Why a recipe request was refused a key.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MediaKeyError {
    /// A field whose byte length does not fit the `u32` length prefix.
    FieldTooLong { field: &'static str, len: usize },
    /// More references than the `u32` count prefix can hold.
    TooManyReferences { count: usize },
}

impl fmt::Display for MediaKeyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FieldTooLong { field, len } => {
                write!(
                    formatter,
                    "{field} of {len} bytes does not fit the key encoding"
                )
            }
            Self::TooManyReferences { count } => {
                write!(formatter, "{count} references do not fit the key encoding")
            }
        }
    }
}

impl std::error::Error for MediaKeyError {}

/// A length beyond `u32::MAX` cannot occur for bounded prompts (32,000 characters) and reference
/// lists; saturation only keeps the encoding total.
fn length(len: usize) -> u32 {
    u32::try_from(len).unwrap_or(u32::MAX)
}

fn field(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&length(bytes.len()).to_be_bytes());
    out.extend_from_slice(bytes);
}

/// A length that does not fit the `u32` prefix fails with the offending field named.
fn checked_len(field: &'static str, len: usize) -> Result<u32, MediaKeyError> {
    u32::try_from(len).map_err(|_| MediaKeyError::FieldTooLong { field, len })
}

/// [`field`] for recipe keys: streams one length-prefixed field into the digest and
/// fails instead of saturating an unrepresentable length, so no caller allocates first.
fn feed(hasher: &mut Sha256, field_name: &'static str, bytes: &[u8]) -> Result<(), MediaKeyError> {
    let len = checked_len(field_name, bytes.len())?;
    hasher.update(len.to_be_bytes());
    hasher.update(bytes);
    Ok(())
}

/// One length-prefixed `u32` word: the prefix is the constant word length.
fn feed_u32(hasher: &mut Sha256, value: u32) {
    hasher.update(4_u32.to_be_bytes());
    hasher.update(value.to_be_bytes());
}

/// One length-prefixed option tag byte: the prefix is the constant tag length.
fn feed_tag(hasher: &mut Sha256, tag: u8) {
    hasher.update(1_u32.to_be_bytes());
    hasher.update([tag]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audience::MemberSet;
    use df_types::{CampaignId, MemberId};

    fn id(byte: u8) -> [u8; 16] {
        let mut bytes = [0_u8; 16];
        bytes[15] = byte;
        bytes
    }

    fn shared() -> Audience {
        Audience::CampaignShared(CampaignId::from_bytes(&id(1)).unwrap())
    }

    fn request<'a>(audience: &'a Audience, model: &'a str, quality: &'a str) -> MediaRequest<'a> {
        MediaRequest {
            kind: "scene_plate",
            model,
            quality,
            rating: "mature",
            width_px: 2560,
            height_px: 1440,
            is_transparent: false,
            references: &["aa", "bb"],
            prompt: "a tavern at dusk",
            scene_evidence: None,
            audience,
        }
    }

    #[test]
    fn fields_that_concatenate_to_the_same_text_hash_differently() {
        let audience = shared();
        assert_ne!(
            MediaRequestKey::derive(&request(&audience, "ab", "c")),
            MediaRequestKey::derive(&request(&audience, "a", "bc"))
        );
    }

    #[test]
    fn the_same_request_always_derives_the_same_key() {
        let audience = shared();
        assert_eq!(
            MediaRequestKey::derive(&request(&audience, "m", "q")),
            MediaRequestKey::derive(&request(&audience, "m", "q"))
        );
    }

    #[test]
    fn the_audience_is_part_of_the_key_down_to_the_member_ids() {
        let table = shared();
        let campaign = CampaignId::from_bytes(&id(1)).unwrap();
        let one = Audience::Members(
            campaign,
            MemberSet::new(vec![MemberId::from_bytes(&id(5)).unwrap()]).unwrap(),
        );
        let other = Audience::Members(
            campaign,
            MemberSet::new(vec![MemberId::from_bytes(&id(6)).unwrap()]).unwrap(),
        );
        let keys = [
            MediaRequestKey::derive(&request(&table, "m", "q")),
            MediaRequestKey::derive(&request(&one, "m", "q")),
            MediaRequestKey::derive(&request(&other, "m", "q")),
        ];
        assert_ne!(keys[0], keys[1]);
        assert_ne!(keys[1], keys[2]);
    }

    #[test]
    fn moving_a_reference_into_the_prompt_changes_the_key() {
        let audience = shared();
        let mut moved = request(&audience, "m", "q");
        moved.references = &["aa"];
        let prompt = format!("{}bb", moved.prompt);
        moved.prompt = &prompt;
        assert_ne!(
            MediaRequestKey::derive(&request(&audience, "m", "q")),
            MediaRequestKey::derive(&moved)
        );
    }

    fn recipe(audience: &Audience) -> MediaRecipeRequest<'_> {
        MediaRecipeRequest {
            operation_kind: "generate",
            purpose: "scene_plate",
            owner_scope: "campaign",
            backend_profile: "general",
            model: "m",
            quality: "q",
            rating: "mature",
            width_px: 2560,
            height_px: 1440,
            output_format: "png",
            alpha_policy: "opaque",
            template_revision: 1,
            route_revision: 2,
            acceptance_revision: 3,
            executor_revision: 4,
            source_binding: None,
            references: &["aa", "bb"],
            prompt: "a tavern at dusk",
            pipeline_bytes: &[7, 8, 9],
            regeneration_nonce: None,
            audience,
        }
    }

    fn recipe_key(request: &MediaRecipeRequest<'_>) -> MediaRequestKey {
        MediaRequestKey::derive_recipe(request).expect("bounded recipe fixture")
    }

    #[test]
    fn v2_encoding_matches_the_documented_scheme_byte_for_byte() {
        let audience = shared();
        let derived = MediaRequestKey::derive(&request(&audience, "m", "q"));
        let mut expected = Vec::new();
        let mut raw = |bytes: &[u8]| {
            let len = u32::try_from(bytes.len()).expect("fixture fits");
            expected.extend_from_slice(&len.to_be_bytes());
            expected.extend_from_slice(bytes);
        };
        raw(MEDIA_REQUEST_SCHEMA.as_bytes());
        raw(b"scene_plate");
        raw(b"m");
        raw(b"q");
        raw(b"mature");
        raw(&2560_u32.to_be_bytes());
        raw(&1440_u32.to_be_bytes());
        raw(&[0]);
        raw(&2_u32.to_be_bytes());
        raw(b"aa");
        raw(b"bb");
        raw(b"a tavern at dusk");
        raw(&[0]);
        let mut audience_bytes = Vec::new();
        audience.encode_into(&mut audience_bytes);
        raw(&audience_bytes);
        assert_eq!(
            derived,
            MediaRequestKey::from_bytes(Sha256::digest(&expected).into())
        );
    }

    #[test]
    fn recipe_keys_use_a_distinct_schema_from_legacy_keys() {
        let audience = shared();
        assert_eq!(
            recipe_key(&recipe(&audience)),
            recipe_key(&recipe(&audience))
        );
        assert_ne!(
            MediaRequestKey::derive(&request(&audience, "m", "q")),
            recipe_key(&recipe(&audience))
        );
    }

    #[test]
    fn every_recipe_field_contributes_to_the_key() {
        let audience = shared();
        let base = recipe(&audience);
        let base_key = recipe_key(&base);
        let mut changed = base;
        changed.operation_kind = "edit";
        assert_ne!(base_key, recipe_key(&changed));
        changed = base;
        changed.purpose = "creator_portrait";
        assert_ne!(base_key, recipe_key(&changed));
        changed = base;
        changed.owner_scope = "account";
        assert_ne!(base_key, recipe_key(&changed));
        changed = base;
        changed.backend_profile = "portrait";
        assert_ne!(base_key, recipe_key(&changed));
        changed = base;
        changed.model = "other";
        assert_ne!(base_key, recipe_key(&changed));
        changed = base;
        changed.quality = "high";
        assert_ne!(base_key, recipe_key(&changed));
        changed = base;
        changed.rating = "general";
        assert_ne!(base_key, recipe_key(&changed));
        changed = base;
        changed.width_px = 1280;
        assert_ne!(base_key, recipe_key(&changed));
        changed = base;
        changed.height_px = 720;
        assert_ne!(base_key, recipe_key(&changed));
        changed = base;
        changed.output_format = "jpeg";
        assert_ne!(base_key, recipe_key(&changed));
        changed = base;
        changed.alpha_policy = "matte";
        assert_ne!(base_key, recipe_key(&changed));
        changed = base;
        changed.template_revision = 9;
        assert_ne!(base_key, recipe_key(&changed));
        changed = base;
        changed.route_revision = 9;
        assert_ne!(base_key, recipe_key(&changed));
        changed = base;
        changed.acceptance_revision = 9;
        assert_ne!(base_key, recipe_key(&changed));
        changed = base;
        changed.executor_revision = 9;
        assert_ne!(base_key, recipe_key(&changed));
        changed = base;
        changed.source_binding = Some("binding");
        assert_ne!(base_key, recipe_key(&changed));
        changed = base;
        changed.references = &["aa"];
        assert_ne!(base_key, recipe_key(&changed));
        changed = base;
        let prompt = "a tavern at dawn".to_string();
        changed.prompt = &prompt;
        assert_ne!(base_key, recipe_key(&changed));
        changed = base;
        changed.pipeline_bytes = &[9, 8, 7];
        assert_ne!(base_key, recipe_key(&changed));
        changed = base;
        changed.regeneration_nonce = Some("again");
        assert_ne!(base_key, recipe_key(&changed));
        changed = base;
        let campaign = CampaignId::from_bytes(&id(1)).unwrap();
        let private = Audience::Members(
            campaign,
            MemberSet::new(vec![MemberId::from_bytes(&id(5)).unwrap()]).unwrap(),
        );
        changed.audience = &private;
        assert_ne!(base_key, recipe_key(&changed));
    }

    #[test]
    fn recipe_boundaries_resist_concatenation_and_option_shifts() {
        let audience = shared();
        let base = recipe(&audience);
        let base_key = recipe_key(&base);
        let mut first = base;
        first.operation_kind = "ab";
        first.purpose = "c";
        let mut second = base;
        second.operation_kind = "a";
        second.purpose = "bc";
        assert_ne!(recipe_key(&first), recipe_key(&second));
        let mut third = base;
        third.references = &["ab", "c"];
        let mut fourth = base;
        fourth.references = &["a", "bc"];
        assert_ne!(recipe_key(&third), recipe_key(&fourth));
        let mut absent = base;
        absent.source_binding = None;
        let mut empty = base;
        empty.source_binding = Some("");
        assert_ne!(recipe_key(&absent), recipe_key(&empty));
        let mut no_nonce = base;
        no_nonce.regeneration_nonce = None;
        let mut blank_nonce = base;
        blank_nonce.regeneration_nonce = Some("");
        assert_ne!(recipe_key(&no_nonce), recipe_key(&blank_nonce));
        let mut moved = base;
        moved.references = &["aa"];
        let prompt = format!("{}bb", moved.prompt);
        moved.prompt = &prompt;
        assert_ne!(base_key, recipe_key(&moved));
    }

    #[test]
    fn reference_and_pipeline_order_are_part_of_the_key() {
        let audience = shared();
        let base = recipe(&audience);
        let base_key = recipe_key(&base);
        let mut swapped = base;
        swapped.references = &["bb", "aa"];
        assert_ne!(base_key, recipe_key(&swapped));
        let mut reversed = base;
        reversed.pipeline_bytes = &[9, 8, 7];
        assert_ne!(base_key, recipe_key(&reversed));
    }

    #[test]
    fn the_recipe_key_separates_audiences() {
        let table = shared();
        let campaign = CampaignId::from_bytes(&id(1)).unwrap();
        let one = Audience::Members(
            campaign,
            MemberSet::new(vec![MemberId::from_bytes(&id(5)).unwrap()]).unwrap(),
        );
        let other = Audience::Members(
            campaign,
            MemberSet::new(vec![MemberId::from_bytes(&id(6)).unwrap()]).unwrap(),
        );
        let host = Audience::Host(campaign);
        let keys = [
            recipe_key(&recipe(&table)),
            recipe_key(&recipe(&one)),
            recipe_key(&recipe(&other)),
            recipe_key(&recipe(&host)),
        ];
        assert_ne!(keys[0], keys[1]);
        assert_ne!(keys[1], keys[2]);
        assert_ne!(keys[0], keys[3]);
    }

    #[test]
    fn recipe_debug_hides_the_private_generation_basis() {
        let audience = shared();
        let mut request = recipe(&audience);
        request.source_binding = Some("secret-source-binding");
        request.regeneration_nonce = Some("secret-nonce");
        let shown = format!("{request:?}");
        for secret in ["a tavern at dusk", "secret-source-binding", "secret-nonce"] {
            assert!(
                !shown.contains(secret),
                "debug leaks private basis: {shown}"
            );
        }
        assert!(
            !shown.contains("[7, 8, 9]"),
            "debug leaks pipeline bytes: {shown}"
        );
    }

    #[test]
    fn unrepresentable_lengths_fail_with_the_field_named() {
        assert_eq!(checked_len("prompt", 0), Ok(0));
        assert_eq!(checked_len("prompt", 4), Ok(4));
        assert!(
            MediaKeyError::FieldTooLong {
                field: "prompt",
                len: 5
            }
            .to_string()
            .contains("prompt")
        );
        assert!(
            MediaKeyError::TooManyReferences { count: 5 }
                .to_string()
                .contains("references")
        );
    }

    #[cfg(target_pointer_width = "64")]
    #[test]
    fn lengths_beyond_the_u32_prefix_fail_instead_of_saturating() {
        let too_long =
            usize::try_from(u64::from(u32::MAX) + 1).expect("one past u32::MAX fits on 64-bit");
        assert_eq!(
            checked_len("prompt", too_long),
            Err(MediaKeyError::FieldTooLong {
                field: "prompt",
                len: too_long
            })
        );
        let error = MediaKeyError::TooManyReferences { count: too_long };
        assert!(error.to_string().contains("references"));
        assert!(
            MediaKeyError::FieldTooLong {
                field: "prompt",
                len: too_long
            }
            .to_string()
            .contains("prompt")
        );
    }

    #[test]
    fn source_bindings_with_different_values_hash_differently() {
        let audience = shared();
        let mut first = recipe(&audience);
        first.source_binding = Some("alpha");
        let mut second = recipe(&audience);
        second.source_binding = Some("beta");
        assert_ne!(recipe_key(&first), recipe_key(&second));
    }

    #[test]
    fn regeneration_nonces_with_different_values_hash_differently() {
        let audience = shared();
        let mut first = recipe(&audience);
        first.regeneration_nonce = Some("first");
        let mut second = recipe(&audience);
        second.regeneration_nonce = Some("second");
        assert_ne!(recipe_key(&first), recipe_key(&second));
    }

    #[test]
    fn recipe_encoding_matches_the_documented_scheme_byte_for_byte() {
        let audience = shared();
        let derived = recipe_key(&recipe(&audience));
        let mut expected = Vec::new();
        let mut raw = |bytes: &[u8]| {
            let len = u32::try_from(bytes.len()).expect("fixture fits");
            expected.extend_from_slice(&len.to_be_bytes());
            expected.extend_from_slice(bytes);
        };
        raw(MEDIA_RECIPE_SCHEMA.as_bytes());
        raw(b"generate");
        raw(b"scene_plate");
        raw(b"campaign");
        raw(b"general");
        raw(b"m");
        raw(b"q");
        raw(b"mature");
        raw(&2560_u32.to_be_bytes());
        raw(&1440_u32.to_be_bytes());
        raw(b"png");
        raw(b"opaque");
        raw(&1_u32.to_be_bytes());
        raw(&2_u32.to_be_bytes());
        raw(&3_u32.to_be_bytes());
        raw(&4_u32.to_be_bytes());
        raw(&[0]);
        raw(&2_u32.to_be_bytes());
        raw(b"aa");
        raw(b"bb");
        raw(b"a tavern at dusk");
        raw(&[7, 8, 9]);
        raw(&[0]);
        let mut audience_bytes = Vec::new();
        audience.encode_into(&mut audience_bytes);
        raw(&audience_bytes);
        assert_eq!(
            derived,
            MediaRequestKey::from_bytes(Sha256::digest(&expected).into())
        );
    }

    #[test]
    fn recipe_debug_hides_reference_digests_and_alternate_secrets() {
        let audience = shared();
        let mut request = recipe(&audience);
        let secrets = [
            "alternate-secret-reference",
            "a cellar at midnight",
            "alternate-binding-secret",
            "alternate-nonce-secret",
        ];
        request.references = &["alternate-secret-reference"];
        let prompt = secrets[1].to_string();
        request.prompt = &prompt;
        request.source_binding = Some(secrets[2]);
        request.pipeline_bytes = &[1, 2, 3, 4];
        request.regeneration_nonce = Some(secrets[3]);
        let shown = format!("{request:?}");
        for secret in secrets {
            assert!(
                !shown.contains(secret),
                "debug leaks private basis: {shown}"
            );
        }
        assert!(
            !shown.contains("[1, 2, 3, 4]"),
            "debug leaks pipeline bytes: {shown}"
        );
    }

    #[test]
    fn v2_historical_digest_is_pinned() {
        let audience = shared();
        assert_eq!(
            *MediaRequestKey::derive(&request(&audience, "m", "q")).as_bytes(),
            [
                0x63, 0xc8, 0x93, 0x46, 0xc5, 0x63, 0xb1, 0x65, 0xe2, 0x46, 0xf7, 0x75, 0x9a, 0x5e,
                0xdf, 0x1d, 0x89, 0x6d, 0x1c, 0x74, 0x87, 0xe0, 0xf0, 0xc4, 0x48, 0x53, 0x55, 0x2b,
                0x28, 0x9c, 0x9d, 0x33
            ]
        );
    }

    #[test]
    fn v3_historical_digest_is_pinned() {
        let audience = shared();
        assert_eq!(
            *recipe_key(&recipe(&audience)).as_bytes(),
            [
                0x2e, 0x29, 0x73, 0x25, 0x72, 0xf2, 0x07, 0xd1, 0x97, 0xb0, 0x1c, 0xd6, 0x9c, 0x24,
                0xbf, 0xd1, 0xac, 0x63, 0x37, 0x44, 0x89, 0x7c, 0x60, 0xc5, 0x01, 0xa1, 0x84, 0x17,
                0xd0, 0xde, 0xc3, 0x3a
            ]
        );
    }
}
