//! Pure selection of existing, audience-permitted identity media. No issuance, mutation or I/O.
use df_model::checkpoint::{
    AssetKind, AssetReference, AudienceScope, Basis, CanonicalPack, Checkpoint, CheckpointError,
    CheckpointLimits, CheckpointPins, ContentDigest, EntityId, EntityIdentityRevision,
    ExecutionMode, FactId, GameState, ItemOrigin, ReferenceInventory, SceneIdentityRevision,
    VisualBible,
};
use df_types::RevisionLabel;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdentityTarget {
    Scene(EntityId),
    Item(EntityId),
    Npc(EntityId),
}
impl IdentityTarget {
    fn entity(self) -> EntityId {
        match self {
            Self::Scene(entity) | Self::Item(entity) | Self::Npc(entity) => entity,
        }
    }
}

/// Consumer policy labels, without numerical quality or provider capability claims.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreparedTier {
    Video,
    Still,
    Illustration,
    FlatScene,
    Audio,
}
impl PreparedTier {
    fn kind(self) -> AssetKind {
        match self {
            Self::Video => AssetKind::Video,
            Self::Still | Self::Illustration => AssetKind::Image,
            Self::FlatScene => AssetKind::TacticalGeometry,
            Self::Audio => AssetKind::Audio,
        }
    }
}

/// Exact immutable request binding. Facts contain every canonical provenance fact, without
/// duplicates or extra facts. Scene and item records pin geometry and award/definition lineage.
/// The session owner supplies the current pins, basis and authorized audience independently.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ContinuityBinding<'a> {
    pub basis: Basis,
    pub source: ContentDigest,
    pub mode: ExecutionMode,
    pub target: IdentityTarget,
    pub revision: &'a RevisionLabel,
    pub pack_revision: &'a RevisionLabel,
    pub pack_digest: ContentDigest,
    pub bible_revision: &'a RevisionLabel,
    pub facts: &'a [FactId],
    pub audience: &'a AudienceScope,
    pub scene: Option<&'a SceneIdentityRevision>,
    pub item: Option<&'a ItemOrigin>,
}

pub struct ContinuityContext<'a> {
    pub current: &'a Checkpoint,
    pub expected: Basis,
    pub pins: &'a CheckpointPins,
    pub inventory: ReferenceInventory<'a>,
}

/// Request-scoped offers supplied after audience projection. Every offer must match the request;
/// a stale offer is refused even when another tier would otherwise be usable.
#[derive(Clone, Copy, Debug)]
pub struct PreparedRepresentation<'a> {
    pub binding: ContinuityBinding<'a>,
    pub tier: PreparedTier,
    pub asset: &'a AssetReference,
}

/// Trusted input from the existing byte/access owner after complete digest/rights verification.
/// Constructing this value grants no access or publication authority. Media checks exact reference
/// and complete nonempty length; it does not replace the asset owner's digest verification.
#[derive(Clone, Copy)]
pub struct AdmittedPreparedBytes<'a> {
    pub asset: &'a AssetReference,
    pub bytes: &'a [u8],
}

#[derive(Clone, Copy, Debug)]
pub struct ContinuityLimits {
    pub checkpoint: CheckpointLimits,
    pub maximum_prepared_records: usize,
    pub maximum_prepared_bytes: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContinuityError {
    InvalidLimits,
    Capacity,
    MissingPolicy,
    Duplicate,
    Checkpoint(CheckpointError),
    StaleBinding,
    MissingIdentity,
    AmbiguousIdentity,
    PackMismatch,
    MissingFacts,
    UncommittedFact,
    AudienceUnavailable,
    PreparedMismatch,
    UnsupportedTier,
    ReferenceUnavailable,
    BytesUnavailable,
    Unavailable,
}

/// Borrowed selection, not a ready/publication receipt. Retains the same bible, canonical pack,
/// identity facts, geometry/award lineage and byte ownership across explicitly ordered fallback.
pub struct ContinuitySelection<'a> {
    pub binding: ContinuityBinding<'a>,
    pub pack: &'a CanonicalPack,
    pub bible: &'a VisualBible,
    pub identity: Option<&'a EntityIdentityRevision>,
    pub representation: PreparedRepresentation<'a>,
    pub bytes: &'a [u8],
    pub fallback: bool,
}
impl std::fmt::Debug for ContinuitySelection<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ContinuitySelection")
            .field("tier", &self.representation.tier)
            .field("fallback", &self.fallback)
            .field("byte_length", &self.bytes.len())
            .finish_non_exhaustive()
    }
}

/// Selects the first available admitted tier. The caller supplies a finite explicit order.
/// Missing prepared/replay/live entries return unavailability; there is no provider branch.
pub fn select_continuity<'a>(
    context: ContinuityContext<'a>,
    binding: ContinuityBinding<'a>,
    tiers: &[PreparedTier],
    prepared: &[PreparedRepresentation<'a>],
    admitted_bytes: &[AdmittedPreparedBytes<'a>],
    limits: ContinuityLimits,
) -> Result<ContinuitySelection<'a>, ContinuityError> {
    preflight(&context, &binding, tiers, prepared, admitted_bytes, limits)?;
    context
        .current
        .validate_admitted(
            context.expected,
            context.pins,
            context.inventory,
            limits.checkpoint,
        )
        .map_err(ContinuityError::Checkpoint)?;
    if binding.basis != context.expected
        || binding.source != context.pins.content.content_digest
        || binding.mode != context.current.state().mode
    {
        return Err(ContinuityError::StaleBinding);
    }
    let state = context.current.state();
    validate_audience(state, binding.audience)?;
    let pack = unique(
        state
            .continuity
            .canonical_packs
            .iter()
            .filter(|pack| &pack.revision == binding.pack_revision),
    )?;
    if pack.digest != binding.pack_digest || &pack.bible.revision != binding.bible_revision {
        return Err(ContinuityError::PackMismatch);
    }
    let entity = unique(
        state
            .entities
            .iter()
            .filter(|entity| entity.id == binding.target.entity()),
    )?;
    let identity = match binding.target {
        IdentityTarget::Scene(id) => {
            let scene = unique(
                state
                    .continuity
                    .scenes
                    .iter()
                    .filter(|scene| scene.scene == id),
            )?;
            if binding.scene != Some(scene)
                || binding.item.is_some()
                || &scene.revision != binding.revision
                || &scene.canonical_pack != binding.pack_revision
            {
                return Err(ContinuityError::StaleBinding);
            }
            // Location versions and entity appearance versions are independent canonical records.
            let mut identities = pack
                .identities
                .iter()
                .filter(|identity| identity.entity == id);
            let identity = identities.next();
            if identities.next().is_some() {
                return Err(ContinuityError::AmbiguousIdentity);
            }
            if identity.is_some_and(|identity| identity.revision != entity.identity_revision) {
                return Err(ContinuityError::StaleBinding);
            }
            identity
        }
        IdentityTarget::Item(id) => {
            let item = unique(
                state
                    .continuity
                    .item_origins
                    .iter()
                    .filter(|item| item.item == id),
            )?;
            if binding.item != Some(item)
                || binding.scene.is_some()
                || &item.identity_revision != binding.revision
                || item.definition != entity.definition
            {
                return Err(ContinuityError::StaleBinding);
            }
            let fact = unique(
                state
                    .facts
                    .iter()
                    .filter(|fact| fact.id == item.source_fact),
            )?;
            if fact.operation != item.award_operation {
                return Err(ContinuityError::UncommittedFact);
            }
            Some(entity_identity(
                pack,
                entity.id,
                &entity.identity_revision,
                binding.revision,
            )?)
        }
        IdentityTarget::Npc(id) => {
            unique(state.continuity.npcs.iter().filter(|npc| npc.entity == id))?;
            if binding.item.is_some() || binding.scene.is_some() {
                return Err(ContinuityError::StaleBinding);
            }
            Some(entity_identity(
                pack,
                entity.id,
                &entity.identity_revision,
                binding.revision,
            )?)
        }
    };
    validate_facts(state, &binding, identity)?;
    // Validate all offers before selecting, so malformed lower tiers cannot hide behind a hit.
    for (index, offer) in prepared.iter().enumerate() {
        if offer.binding != binding {
            return Err(ContinuityError::PreparedMismatch);
        }
        if prepared
            .iter()
            .take(index)
            .any(|prior| prior.tier == offer.tier)
        {
            return Err(ContinuityError::Duplicate);
        }
        if !tiers.contains(&offer.tier)
            || offer.asset.kind != offer.tier.kind()
            || (offer.tier == PreparedTier::FlatScene
                && !matches!(binding.target, IdentityTarget::Scene(_)))
        {
            return Err(ContinuityError::UnsupportedTier);
        }
        let is_canonical = identity.is_some_and(|identity| {
            identity.appearances.contains(offer.asset)
                || identity.sound.contains(offer.asset)
                || identity.voice.as_ref() == Some(offer.asset)
        }) || (matches!(binding.target, IdentityTarget::Scene(_))
            && pack.bible.references.contains(offer.asset))
            || binding
                .scene
                .is_some_and(|scene| &scene.geometry == offer.asset);
        if !is_canonical || !context.inventory.assets.contains(offer.asset) {
            return Err(ContinuityError::ReferenceUnavailable);
        }
        let bytes = admitted_bytes
            .iter()
            .find(|bytes| bytes.asset == offer.asset)
            .ok_or(ContinuityError::BytesUnavailable)?;
        if bytes.bytes.is_empty()
            || u64::try_from(bytes.bytes.len()).ok() != Some(offer.asset.byte_length)
        {
            return Err(ContinuityError::BytesUnavailable);
        }
    }
    for (index, tier) in tiers.iter().enumerate() {
        if let Some(offer) = prepared.iter().find(|offer| &offer.tier == tier) {
            let bytes = admitted_bytes
                .iter()
                .find(|bytes| bytes.asset == offer.asset)
                .ok_or(ContinuityError::BytesUnavailable)?;
            return Ok(ContinuitySelection {
                binding,
                pack,
                bible: &pack.bible,
                identity,
                representation: *offer,
                bytes: bytes.bytes,
                fallback: index != 0,
            });
        }
    }
    Err(ContinuityError::Unavailable)
}

fn unique<'a, T>(mut records: impl Iterator<Item = &'a T>) -> Result<&'a T, ContinuityError> {
    let record = records.next().ok_or(ContinuityError::MissingIdentity)?;
    if records.next().is_some() {
        return Err(ContinuityError::AmbiguousIdentity);
    }
    Ok(record)
}

fn entity_identity<'a>(
    pack: &'a CanonicalPack,
    entity: EntityId,
    current_revision: &RevisionLabel,
    requested_revision: &RevisionLabel,
) -> Result<&'a EntityIdentityRevision, ContinuityError> {
    if current_revision != requested_revision {
        return Err(ContinuityError::StaleBinding);
    }
    let identity = unique(
        pack.identities
            .iter()
            .filter(|identity| identity.entity == entity),
    )?;
    if &identity.revision != requested_revision {
        return Err(ContinuityError::StaleBinding);
    }
    Ok(identity)
}

fn validate_audience(state: &GameState, audience: &AudienceScope) -> Result<(), ContinuityError> {
    if let AudienceScope::Members(members) = audience
        && (members.is_empty()
            || members.iter().enumerate().any(|(index, member)| {
                members.iter().take(index).any(|prior| prior == member)
                    || !state.members.iter().any(|record| &record.member == member)
            }))
    {
        return Err(ContinuityError::AudienceUnavailable);
    }
    Ok(())
}

fn validate_facts(
    state: &GameState,
    binding: &ContinuityBinding<'_>,
    identity: Option<&EntityIdentityRevision>,
) -> Result<(), ContinuityError> {
    let identity_facts = identity.map_or(&[][..], |identity| identity.source_facts.as_slice());
    let scene_facts = binding
        .scene
        .map_or(&[][..], |scene| scene.source_facts.as_slice());
    let item_source_fact = binding.item.map(|item| item.source_fact);
    for facts in [identity_facts, scene_facts] {
        if facts
            .iter()
            .enumerate()
            .any(|(index, fact)| facts.iter().take(index).any(|prior| prior == fact))
        {
            return Err(ContinuityError::Duplicate);
        }
    }
    let is_required_fact = |id: &FactId| {
        identity_facts.contains(id) || scene_facts.contains(id) || item_source_fact == Some(*id)
    };
    if binding.facts.iter().any(|id| !is_required_fact(id))
        || identity_facts
            .iter()
            .chain(scene_facts)
            .any(|id| !binding.facts.contains(id))
        || item_source_fact.is_some_and(|id| !binding.facts.contains(&id))
    {
        return Err(ContinuityError::MissingFacts);
    }
    for id in binding.facts {
        let fact = unique(state.facts.iter().filter(|fact| fact.id == *id))?;
        if !state.decisions.iter().any(|decision| {
            decision.operation == fact.operation
                && decision.revision == fact.revision
                && decision.facts.contains(id)
        }) {
            return Err(ContinuityError::UncommittedFact);
        }
        if fact.audience != AudienceScope::Shared && &fact.audience != binding.audience {
            return Err(ContinuityError::AudienceUnavailable);
        }
    }
    Ok(())
}

fn preflight(
    context: &ContinuityContext<'_>,
    binding: &ContinuityBinding<'_>,
    tiers: &[PreparedTier],
    prepared: &[PreparedRepresentation<'_>],
    admitted_bytes: &[AdmittedPreparedBytes<'_>],
    limits: ContinuityLimits,
) -> Result<(), ContinuityError> {
    let checkpoint = limits.checkpoint;
    if checkpoint.maximum_records == 0
        || checkpoint.maximum_records > 4096
        || checkpoint.maximum_text_bytes == 0
        || checkpoint.maximum_text_bytes > 4096
        || checkpoint.maximum_total_text_bytes == 0
        || checkpoint.maximum_total_text_bytes > 1024 * 1024
        || checkpoint.maximum_retained_bytes == 0
        || checkpoint.maximum_retained_bytes > 16 * 1024 * 1024
        || limits.maximum_prepared_records == 0
        || limits.maximum_prepared_records > 256
        || limits.maximum_prepared_bytes == 0
        || limits.maximum_prepared_bytes > 16 * 1024 * 1024
    {
        return Err(ContinuityError::InvalidLimits);
    }
    let inventory = context.inventory;
    if [
        inventory.rules.len(),
        inventory.content.len(),
        inventory.resources.len(),
        inventory.assets.len(),
        binding.facts.len(),
    ]
    .into_iter()
    .try_fold(0usize, |total, count| total.checked_add(count))
    .is_none_or(|count| count > checkpoint.maximum_records)
        || prepared.len() > limits.maximum_prepared_records
        || admitted_bytes.len() > limits.maximum_prepared_records
        || context
            .current
            .retained_bytes()
            .is_none_or(|bytes| bytes > checkpoint.maximum_retained_bytes)
    {
        return Err(ContinuityError::Capacity);
    }
    if tiers.is_empty() {
        return Err(ContinuityError::MissingPolicy);
    }
    if tiers.len() > 5 {
        return Err(ContinuityError::Capacity);
    }
    if tiers
        .iter()
        .enumerate()
        .any(|(index, tier)| tiers.iter().take(index).any(|prior| prior == tier))
        || binding
            .facts
            .iter()
            .enumerate()
            .any(|(index, fact)| binding.facts.iter().take(index).any(|prior| prior == fact))
    {
        return Err(ContinuityError::Duplicate);
    }
    for value in std::iter::once(binding).chain(prepared.iter().map(|offer| &offer.binding)) {
        if value.facts.len() > checkpoint.maximum_records
            || value
                .scene
                .is_some_and(|scene| scene.source_facts.len() > checkpoint.maximum_records)
            || matches!(value.audience, AudienceScope::Members(members) if members.len() > checkpoint.maximum_records)
        {
            return Err(ContinuityError::Capacity);
        }
    }
    let mut total_admitted_bytes = 0usize;
    for (index, entry) in admitted_bytes.iter().enumerate() {
        if admitted_bytes
            .iter()
            .take(index)
            .any(|prior| prior.asset == entry.asset)
        {
            return Err(ContinuityError::Duplicate);
        }
        total_admitted_bytes = total_admitted_bytes
            .checked_add(entry.bytes.len())
            .ok_or(ContinuityError::Capacity)?;
        if total_admitted_bytes > limits.maximum_prepared_bytes {
            return Err(ContinuityError::Capacity);
        }
    }
    Ok(())
}
