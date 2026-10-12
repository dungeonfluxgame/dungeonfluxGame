//! Immutable checkpoint ownership and structural validation. No rules execution or I/O.
//! Catalog correctness, standard-2024 coverage, restore authority and codecs are separate gates.
use df_types::{
    BuildIdentity, IdentityError, MemberId, OperationId, RevisionLabel, RunId, SessionId,
    SessionRevision,
};
use std::collections::BTreeSet;

/// Current domain schema. The persistence owner separately versions its codec.
pub const CHECKPOINT_SCHEMA: u16 = 2;

/// Supplied identities bind records; none grants membership, restore, or source access.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Basis {
    pub session: SessionId,
    pub run: RunId,
    pub revision: SessionRevision,
}

// New semantic roles reuse the canonical nonzero-byte validator, without cross-role conversion.
macro_rules! identity {
    ($($name:ident),+ $(,)?) => {$ (
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(OperationId);
        impl $name {
            pub fn from_bytes(bytes: &[u8]) -> Result<Self, IdentityError> {
                OperationId::from_bytes(bytes).map(Self)
            }
            pub fn as_bytes(&self) -> &[u8; 16] { self.0.as_bytes() }
        }
    )+ };
}
identity!(
    EntityId,
    FactId,
    ResolutionId,
    WindowId,
    JobId,
    TimerId,
    EffectId,
    RecordId
);

/// Digest bytes supplied by the trusted boundary. Possession grants no rights.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ContentDigest(pub [u8; 32]);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RulesMode {
    Standard2024,
    DisclosedCustom,
}

/// Exact source/data/handler identity, never a mutable current-catalog pointer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RulesPins {
    pub mode: RulesMode,
    pub ruleset: RevisionLabel,
    pub catalog: RevisionLabel,
    pub catalog_digest: ContentDigest,
    pub source_manifest: RevisionLabel,
    pub source_manifest_digest: ContentDigest,
    pub handler: RevisionLabel,
    pub handler_digest: ContentDigest,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContentPins {
    pub content: RevisionLabel,
    pub content_digest: ContentDigest,
    pub package: RevisionLabel,
    pub package_digest: ContentDigest,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckpointPins {
    pub rules: RulesPins,
    pub content: ContentPins,
    pub build: BuildIdentity,
}

/// A source clause locator. Catalog pin equality alone is not source qualification.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuleReference {
    pub catalog: RevisionLabel,
    pub source: RevisionLabel,
    pub entry: RevisionLabel,
    pub clause: RevisionLabel,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContentReference {
    pub package: RevisionLabel,
    pub entry: RevisionLabel,
}

/// Trusted already-admitted reference inventory; checkpoint bytes cannot admit sources.
#[derive(Clone, Copy)]
pub struct ReferenceInventory<'a> {
    pub rules: &'a [RuleReference],
    pub content: &'a [ContentReference],
    pub resources: &'a [ResourceConstraint],
    pub assets: &'a [AssetReference],
}

/// Trusted source-validated current resource bounds supplied by the rules owner.
/// The checkpoint cannot raise its own cap by editing persisted interval fields.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceConstraint {
    pub owner: EntityId,
    pub resource: RevisionLabel,
    pub minimum: i64,
    pub maximum: i64,
    pub source: RuleReference,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecutionMode {
    Live,
    PreparedOnly,
    Replay,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LogicalTime {
    pub ticks: u64,
    pub ticks_per_second: u32,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AudienceScope {
    Shared,
    Members(Vec<MemberId>),
    Host,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MembershipLink {
    pub member: MemberId,
    pub character: Option<EntityId>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Position {
    pub x: i64,
    pub y: i64,
    pub z: i64,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorldEntity {
    pub id: EntityId,
    pub definition: ContentReference,
    pub location: Option<EntityId>,
    pub position: Option<Position>,
    pub identity_revision: RevisionLabel,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CharacterState {
    pub entity: EntityId,
    pub build: ContentReference,
    pub owner: MemberId,
    pub choices: Vec<AcceptedChoice>,
}

/// Source-supplied interval; no universal hit-point/slot/currency cap is invented here.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceState {
    pub owner: EntityId,
    pub resource: RevisionLabel,
    pub value: i64,
    pub minimum: i64,
    pub maximum: i64,
    pub source: RuleReference,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InventoryItem {
    pub item: EntityId,
    pub owner: EntityId,
    pub quantity: u64,
    pub source: RuleReference,
    pub origin: ContentReference,
    pub attunement_owner: Option<EntityId>,
}

/// Actual observed die result, never a future seed or request to generate a draw.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActualDraw {
    pub operation: OperationId,
    pub ordinal: u32,
    pub resolution: ResolutionId,
    pub window: WindowId,
    pub sides: u32,
    pub value: u32,
    pub source: RuleReference,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcceptedChoice {
    pub participant: MemberId,
    pub offer: RevisionLabel,
    pub selected: RevisionLabel,
    pub source: RuleReference,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceSpend {
    pub owner: EntityId,
    pub resource: RevisionLabel,
    pub amount: u64,
    pub source: RuleReference,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScopedRuling {
    pub adjudicator: MemberId,
    pub selected: RevisionLabel,
    pub source: RuleReference,
    pub audience: AudienceScope,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TriggerPhase {
    BeforeDraw,
    AfterDraw,
    BeforeConsequence,
    AfterConsequence,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolutionWindow {
    pub id: WindowId,
    pub phase: TriggerPhase,
    pub causal_fact: FactId,
    pub source: RuleReference,
    pub timer: Option<TimerId>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OfferedResponse {
    pub participant: MemberId,
    pub offer: RevisionLabel,
    pub options: Vec<RevisionLabel>,
    pub source: RuleReference,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PendingInput {
    Choice {
        remaining: Vec<OfferedResponse>,
    },
    Roll {
        participant: MemberId,
        sides: Vec<u32>,
        source: RuleReference,
    },
    Reaction {
        remaining: Vec<OfferedResponse>,
    },
    Ruling {
        permitted: Vec<OfferedResponse>,
        source: RuleReference,
    },
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingResolution {
    pub id: ResolutionId,
    pub basis: Basis,
    pub continuation: RevisionLabel,
    pub window: ResolutionWindow,
    pub next: PendingInput,
    pub choices: Vec<AcceptedChoice>,
    pub draw_ordinals: Vec<(OperationId, u32)>,
    pub spent: Vec<ResourceSpend>,
    pub rulings: Vec<ScopedRuling>,
}

/// Typed committed facts retain order, causal linkage and authorized disclosure metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FactValue {
    EntityCreated {
        entity: EntityId,
        definition: ContentReference,
    },
    EntityMoved {
        entity: EntityId,
        destination: EntityId,
        position: Position,
    },
    ResourceChanged {
        entity: EntityId,
        resource: RevisionLabel,
        before: i64,
        after: i64,
        source: RuleReference,
    },
    DrawAccepted {
        operation: OperationId,
        ordinal: u32,
    },
    ChoiceAccepted {
        resolution: ResolutionId,
        window: WindowId,
        choice: AcceptedChoice,
    },
    RulingAccepted {
        resolution: ResolutionId,
        window: WindowId,
        ruling: ScopedRuling,
    },
    TimeAdvanced {
        before: LogicalTime,
        after: LogicalTime,
    },
    ContentEvent {
        definition: ContentReference,
        subjects: Vec<EntityId>,
    },
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GameFact {
    pub id: FactId,
    pub revision: SessionRevision,
    pub operation: OperationId,
    pub ordinal: u32,
    pub cause: Option<FactId>,
    pub audience: AudienceScope,
    pub value: FactValue,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcceptedDecision {
    pub operation: OperationId,
    pub revision: SessionRevision,
    pub facts: Vec<FactId>,
    pub draws: Vec<u32>,
    pub effects: Vec<EffectId>,
    pub source_policy: RevisionLabel,
    pub semantic_output: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EffectKind {
    RunAi,
    RunMedia,
    LoadMemoryCandidates,
    ArmTimer,
    CancelJob,
    CancelTimer,
    PublishPresentation,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DurableStatus {
    Pending,
    Claimed,
    SentUnknown,
    Completed,
    Failed,
    Cancelled,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DurableIntent {
    pub id: EffectId,
    pub basis: Basis,
    pub operation: OperationId,
    pub slot: u32,
    pub kind: EffectKind,
    pub job: Option<JobId>,
    pub timer: Option<TimerId>,
    pub generation: u64,
    pub status: DurableStatus,
    pub definition: ContentReference,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OwnedTimer {
    pub id: TimerId,
    pub basis: Basis,
    pub generation: u64,
    pub due: LogicalTime,
    pub source: RuleReference,
    pub status: DurableStatus,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActiveEffect {
    pub id: RecordId,
    pub source: RuleReference,
    pub origin: FactId,
    pub source_entity: Option<EntityId>,
    pub targets: Vec<EntityId>,
    pub starts: LogicalTime,
    pub expires: Option<LogicalTime>,
    pub concentration_owner: Option<EntityId>,
    pub choices: Vec<AcceptedChoice>,
}

/// Beliefs remain attributed claims, distinct from canonical truth.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttributedClaim {
    pub id: RecordId,
    pub holder: EntityId,
    pub subject: EntityId,
    pub claim: String,
    pub evidence: Vec<FactId>,
    pub audience: AudienceScope,
    pub source: ContentReference,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KnowledgeGrant {
    pub observer: MemberId,
    pub fact: FactId,
    pub source: FactId,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryEpisode {
    pub id: RecordId,
    pub holder: EntityId,
    pub source_facts: Vec<FactId>,
    pub retained_text: String,
    pub audience: AudienceScope,
    pub source_revision: SessionRevision,
}

/// Pure persistent state records. They carry admitted content policy, never execute scripts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScheduledEvent {
    pub id: RecordId,
    pub entity: EntityId,
    pub due: LogicalTime,
    pub definition: ContentReference,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ThreatClock {
    pub id: RecordId,
    pub definition: ContentReference,
    pub progress: u64,
    pub capacity: u64,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Relationship {
    pub subject: EntityId,
    pub object: EntityId,
    pub policy: ContentReference,
    /// Authored categorical projection; it remains separate from all seven axes.
    pub state: RevisionLabel,
    pub trust: RelationshipAxisState,
    pub affection: RelationshipAxisState,
    pub respect: RelationshipAxisState,
    pub fear: RelationshipAxisState,
    pub suspicion: RelationshipAxisState,
    pub debt: RelationshipAxisState,
    pub familiarity: RelationshipAxisState,
}

/// One opaque authored policy value and its independent causal origin.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelationshipAxisState {
    pub value: RevisionLabel,
    pub provenance: RelationshipAxisProvenance,
}

/// Baselines are explicitly authored; changes point to an accepted decision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RelationshipAxisProvenance {
    AuthoredBaseline {
        source: ContentReference,
    },
    AcceptedFact {
        source: ContentReference,
        fact: FactId,
        source_policy: RevisionLabel,
        witness: Option<RecordId>,
    },
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConversationState {
    pub id: RecordId,
    pub participants: Vec<EntityId>,
    pub topic: ContentReference,
    pub accepted_facts: Vec<FactId>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Obligation {
    pub id: RecordId,
    pub obligor: EntityId,
    pub beneficiary: EntityId,
    pub definition: ContentReference,
    pub terms: ContentReference,
    pub due: Option<LogicalTime>,
    pub agreement: ObligationAgreement,
    pub status: ObligationStatus,
    /// At most one lifecycle action is accepted before a terminal state.
    pub transition: Option<ObligationTransition>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObligationAgreement {
    pub source: ContentReference,
    pub fact: FactId,
    pub source_policy: RevisionLabel,
    pub at: LogicalTime,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObligationStatus {
    Active,
    Fulfilled,
    Broken,
    Expired,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObligationAction {
    Fulfill,
    Break,
    Expire,
    Cancel,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObligationTransition {
    pub action: ObligationAction,
    pub source: ContentReference,
    pub fact: FactId,
    pub source_policy: RevisionLabel,
    pub at: LogicalTime,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NarrativeState {
    pub definition: ContentReference,
    pub active_beats: Vec<ContentReference>,
    pub completed_beats: Vec<ContentReference>,
    pub open_threads: Vec<ContentReference>,
    pub accepted_facts: Vec<FactId>,
    pub remaining_budget: u64,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TacticalSnapshot {
    pub schema: u16,
    pub source_revision: RevisionLabel,
    pub scene_digest: ContentDigest,
    pub scene: Vec<u8>,
    pub layout: RevisionLabel,
    pub actors: Vec<(u32, EntityId)>,
    pub state: Vec<u8>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EncounterState {
    pub id: RecordId,
    pub definition: ContentReference,
    pub participants: Vec<EntityId>,
    pub turn_order: Vec<EntityId>,
    pub active_turn: Option<EntityId>,
    pub objectives: Vec<ContentReference>,
    pub combat_policy: ContentReference,
    pub tactical: Option<TacticalSnapshot>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActivityWindow {
    pub member: MemberId,
    pub started: LogicalTime,
    pub ends: LogicalTime,
    pub observed_facts: Vec<FactId>,
    pub spotlight_opt_in: bool,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TempoState {
    pub policy: ContentReference,
    pub presentation_ticks: u64,
    pub intensity: i64,
    pub inertia: i64,
    pub fatigue: Vec<(ContentReference, u64)>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PresentationDemand {
    pub id: RecordId,
    pub definition: ContentReference,
    pub audience: AudienceScope,
    pub causal_facts: Vec<FactId>,
    pub source_revision: SessionRevision,
}

/// Owned concrete state, passed by value into a checkpoint and borrowed immutably afterwards.
/// Legal mechanics and current access remain caller-owned; this type preserves their supplied records.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GameState {
    pub mode: ExecutionMode,
    pub logical_time: LogicalTime,
    pub members: Vec<MembershipLink>,
    pub entities: Vec<WorldEntity>,
    pub characters: Vec<CharacterState>,
    pub resources: Vec<ResourceState>,
    pub inventory: Vec<InventoryItem>,
    pub facts: Vec<GameFact>,
    pub draws: Vec<ActualDraw>,
    pub decisions: Vec<AcceptedDecision>,
    pub pending: Vec<PendingResolution>,
    pub intents: Vec<DurableIntent>,
    pub timers: Vec<OwnedTimer>,
    pub active_effects: Vec<ActiveEffect>,
    pub knowledge: Vec<KnowledgeGrant>,
    pub beliefs: Vec<AttributedClaim>,
    pub memories: Vec<MemoryEpisode>,
    pub schedules: Vec<ScheduledEvent>,
    pub threats: Vec<ThreatClock>,
    pub relationships: Vec<Relationship>,
    pub conversations: Vec<ConversationState>,
    pub obligations: Vec<Obligation>,
    pub narrative: NarrativeState,
    pub encounters: Vec<EncounterState>,
    pub activity: Vec<ActivityWindow>,
    pub tempo: TempoState,
    pub presentation: Vec<PresentationDemand>,
    pub continuity: ContinuityState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CheckpointError {
    UnsupportedSchema,
    WrongSession,
    WrongRun,
    StaleBasis,
    RulesMismatch,
    ContentMismatch,
    BuildMismatch,
    InvalidReference,
    DuplicateIdentity,
    InvalidResource,
    InvalidTime,
    InvalidDraw,
    InvalidOrder,
    InvalidContinuation,
    InvalidIntent,
    Capacity,
    RedactedCheckpoint,
    UnavailableCheckpoint,
}

/// Caller-admitted bounds are explicit, positive and checked before structural traversal.
/// Device/load qualification must freeze actual production values; this API supplies no defaults.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CheckpointLimits {
    pub maximum_records: usize,
    pub maximum_text_bytes: usize,
    pub maximum_total_text_bytes: usize,
    pub maximum_retained_bytes: usize,
}

/// Immutable detached checkpoint. Constructor rejection returns no usable checkpoint.
#[derive(Clone, Eq, PartialEq)]
pub struct Checkpoint {
    schema: u16,
    basis: Basis,
    pins: CheckpointPins,
    state: GameState,
}
impl std::fmt::Debug for Checkpoint {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Checkpoint")
            .field("schema", &self.schema)
            .field("basis", &self.basis)
            .finish_non_exhaustive()
    }
}
impl Checkpoint {
    pub fn new(
        schema: u16,
        basis: Basis,
        pins: CheckpointPins,
        state: GameState,
        inventory: ReferenceInventory<'_>,
        limits: CheckpointLimits,
    ) -> Result<Self, CheckpointError> {
        if schema != CHECKPOINT_SCHEMA {
            return Err(CheckpointError::UnsupportedSchema);
        }
        validate_state(&state, basis, &pins, inventory, limits)?;
        let checkpoint = Self {
            schema,
            basis,
            pins,
            state,
        };
        let retained = checkpoint
            .retained_bytes()
            .ok_or(CheckpointError::Capacity)?;
        require(
            retained <= limits.maximum_retained_bytes,
            CheckpointError::Capacity,
        )?;
        Ok(checkpoint)
    }
    pub fn schema(&self) -> u16 {
        self.schema
    }
    pub fn basis(&self) -> Basis {
        self.basis
    }
    pub fn pins(&self) -> &CheckpointPins {
        &self.pins
    }
    pub fn state(&self) -> &GameState {
        &self.state
    }

    /// Borrows an exact retained fact and the accepted decision that produced it.
    /// Seed facts may exist without a decision and return None. This lookup does not
    /// recheck current source/access admission or classify an outcome as successful.
    pub fn committed_outcome(&self, id: FactId) -> Option<(&GameFact, &AcceptedDecision)> {
        committed_fact(&self.state, id)
    }
    pub fn validate_resume(
        &self,
        expected: Basis,
        admitted: &CheckpointPins,
    ) -> Result<&Self, CheckpointError> {
        if self.schema != CHECKPOINT_SCHEMA {
            return Err(CheckpointError::UnsupportedSchema);
        }
        if self.basis.session != expected.session {
            return Err(CheckpointError::WrongSession);
        }
        if self.basis.run != expected.run {
            return Err(CheckpointError::WrongRun);
        }
        if self.basis.revision != expected.revision {
            return Err(CheckpointError::StaleBasis);
        }
        if self.pins.rules != admitted.rules {
            return Err(CheckpointError::RulesMismatch);
        }
        if self.pins.content != admitted.content {
            return Err(CheckpointError::ContentMismatch);
        }
        if self.pins.build != admitted.build {
            return Err(CheckpointError::BuildMismatch);
        }
        if !self.state.continuity.recovery.redacted_records.is_empty() {
            return Err(CheckpointError::RedactedCheckpoint);
        }
        if !self
            .state
            .continuity
            .recovery
            .unavailable_sources
            .is_empty()
        {
            return Err(CheckpointError::UnavailableCheckpoint);
        }
        Ok(self)
    }

    /// Recheck a retained candidate against current inventory and narrower limits before cloning.
    /// Source admission still belongs to the caller; this checks canonical structural lineage.
    pub fn validate_admitted(
        &self,
        expected: Basis,
        admitted: &CheckpointPins,
        inventory: ReferenceInventory<'_>,
        limits: CheckpointLimits,
    ) -> Result<&Self, CheckpointError> {
        self.validate_resume(expected, admitted)?;
        validate_state(&self.state, self.basis, &self.pins, inventory, limits)?;
        let retained = self.retained_bytes().ok_or(CheckpointError::Capacity)?;
        require(
            retained <= limits.maximum_retained_bytes,
            CheckpointError::Capacity,
        )?;
        Ok(self)
    }
}

fn committed_fact(state: &GameState, id: FactId) -> Option<(&GameFact, &AcceptedDecision)> {
    let record = state.facts.iter().find(|record| record.id == id)?;
    let decision = state.decisions.iter().find(|decision| {
        decision.operation == record.operation
            && decision.revision == record.revision
            && decision.facts.contains(&id)
    })?;
    Some((record, decision))
}

fn accepted_fact<'a>(
    state: &'a GameState,
    id: FactId,
    source_policy: &RevisionLabel,
) -> Result<&'a GameFact, CheckpointError> {
    let (record, decision) = committed_fact(state, id).ok_or(CheckpointError::InvalidReference)?;
    require(
        decision.source_policy == *source_policy,
        CheckpointError::InvalidReference,
    )?;
    Ok(record)
}

fn validate_state(
    state: &GameState,
    basis: Basis,
    pins: &CheckpointPins,
    inventory: ReferenceInventory<'_>,
    limits: CheckpointLimits,
) -> Result<(), CheckpointError> {
    if limits.maximum_records == 0
        || limits.maximum_text_bytes == 0
        || limits.maximum_total_text_bytes == 0
        || limits.maximum_retained_bytes == 0
    {
        return Err(CheckpointError::Capacity);
    }
    let mut records = 0usize;
    let mut texts = 0usize;
    macro_rules! count {
        ($items:expr) => {{
            records = records
                .checked_add($items.len())
                .ok_or(CheckpointError::Capacity)?;
            if records > limits.maximum_records {
                return Err(CheckpointError::Capacity);
            }
        }};
    }
    macro_rules! text {
        ($value:expr) => {{
            let value = $value;
            if value.len() > limits.maximum_text_bytes {
                return Err(CheckpointError::Capacity);
            }
            texts = texts
                .checked_add(value.len())
                .ok_or(CheckpointError::Capacity)?;
            if texts > limits.maximum_total_text_bytes {
                return Err(CheckpointError::Capacity);
            }
        }};
    }
    count!(&state.members);
    count!(&state.entities);
    count!(&state.characters);
    count!(&state.resources);
    count!(&state.inventory);
    count!(&state.facts);
    count!(&state.draws);
    count!(&state.decisions);
    count!(&state.pending);
    count!(&state.intents);
    count!(&state.timers);
    count!(&state.active_effects);
    count!(&state.knowledge);
    count!(&state.beliefs);
    count!(&state.memories);
    count!(&state.schedules);
    count!(&state.threats);
    count!(&state.relationships);
    count!(&state.conversations);
    count!(&state.obligations);
    count!(&state.encounters);
    count!(&state.activity);
    count!(&state.presentation);
    let members = unique(state.members.iter().map(|x| x.member))?;
    let entities = unique(state.entities.iter().map(|x| x.id))?;
    let facts = unique(state.facts.iter().map(|x| x.id))?;
    unique(state.pending.iter().map(|x| x.id))?;
    unique(state.intents.iter().map(|x| x.id))?;
    let timers = unique(state.timers.iter().map(|x| x.id))?;
    unique(state.pending.iter().map(|x| x.window.id))?;
    unique(state.active_effects.iter().map(|x| x.id))?;
    unique(state.beliefs.iter().map(|x| x.id))?;
    unique(state.memories.iter().map(|x| x.id))?;
    unique(state.schedules.iter().map(|x| x.id))?;
    unique(state.threats.iter().map(|x| x.id))?;
    unique(state.conversations.iter().map(|x| x.id))?;
    unique(state.obligations.iter().map(|x| x.id))?;
    unique(state.relationships.iter().map(|x| (x.subject, x.object)))?;
    unique(state.encounters.iter().map(|x| x.id))?;
    unique(state.presentation.iter().map(|x| x.id))?;
    unique(state.inventory.iter().map(|x| x.item))?;
    unique(state.characters.iter().map(|x| x.entity))?;
    unique(
        state
            .resources
            .iter()
            .map(|x| (x.owner, x.resource.as_str())),
    )?;
    let content = |reference: &ContentReference| {
        if reference.package != pins.content.package || !inventory.content.contains(reference) {
            Err(CheckpointError::InvalidReference)
        } else {
            Ok(())
        }
    };
    let rule = |reference: &RuleReference| {
        if reference.catalog != pins.rules.catalog || !inventory.rules.contains(reference) {
            Err(CheckpointError::InvalidReference)
        } else {
            Ok(())
        }
    };
    let entity = |id: EntityId| require(entities.contains(&id), CheckpointError::InvalidReference);
    let member = |id: MemberId| require(members.contains(&id), CheckpointError::InvalidReference);
    let fact = |id: FactId| require(facts.contains(&id), CheckpointError::InvalidReference);
    let audience = |scope: &AudienceScope, records: &mut usize| -> Result<(), CheckpointError> {
        if let AudienceScope::Members(ids) = scope {
            count_records(records, ids.len(), limits.maximum_records)?;
            unique(ids.iter().copied())?;
            for id in ids {
                member(*id)?;
            }
        }
        Ok(())
    };
    let choice = |x: &AcceptedChoice| -> Result<(), CheckpointError> {
        member(x.participant)?;
        rule(&x.source)
    };
    let ruling = |x: &ScopedRuling, records: &mut usize| -> Result<(), CheckpointError> {
        member(x.adjudicator)?;
        rule(&x.source)?;
        audience(&x.audience, records)
    };
    valid_time(state.logical_time)?;
    for x in &state.members {
        if let Some(id) = x.character {
            entity(id)?;
        }
    }
    for x in &state.entities {
        content(&x.definition)?;
        if let Some(id) = x.location {
            entity(id)?;
        }
    }
    for x in &state.characters {
        entity(x.entity)?;
        member(x.owner)?;
        content(&x.build)?;
        count!(&x.choices);
        for selected in &x.choices {
            choice(selected)?;
        }
        require(
            state
                .members
                .iter()
                .any(|link| link.member == x.owner && link.character == Some(x.entity)),
            CheckpointError::InvalidReference,
        )?;
    }
    for x in &state.resources {
        entity(x.owner)?;
        rule(&x.source)?;
        let trusted = inventory
            .resources
            .iter()
            .find(|admitted| {
                admitted.owner == x.owner
                    && admitted.resource == x.resource
                    && admitted.source == x.source
            })
            .ok_or(CheckpointError::InvalidResource)?;
        require(
            trusted.minimum <= trusted.maximum
                && x.minimum == trusted.minimum
                && x.maximum == trusted.maximum
                && x.value >= trusted.minimum
                && x.value <= trusted.maximum,
            CheckpointError::InvalidResource,
        )?;
    }
    for x in &state.inventory {
        entity(x.item)?;
        entity(x.owner)?;
        rule(&x.source)?;
        content(&x.origin)?;
        require(x.quantity > 0, CheckpointError::InvalidResource)?;
        if let Some(id) = x.attunement_owner {
            entity(id)?;
        }
    }
    let draws = unique(state.draws.iter().map(|x| (x.operation, x.ordinal)))?;
    let mut last_draw = std::collections::BTreeMap::new();
    for x in &state.draws {
        rule(&x.source)?;
        require(
            x.sides > 0 && x.value > 0 && x.value <= x.sides,
            CheckpointError::InvalidDraw,
        )?;
        ordered(&mut last_draw, x.operation, x.ordinal)?;
    }
    let mut seen_facts = BTreeSet::new();
    let mut last_fact = std::collections::BTreeMap::new();
    let mut last_revision = None;
    for x in &state.facts {
        require(x.revision <= basis.revision, CheckpointError::StaleBasis)?;
        if let Some(previous) = last_revision {
            require(previous <= x.revision, CheckpointError::InvalidOrder)?;
        }
        last_revision = Some(x.revision);
        ordered(&mut last_fact, x.operation, x.ordinal)?;
        if let Some(cause) = x.cause {
            require(
                seen_facts.contains(&cause),
                CheckpointError::InvalidReference,
            )?;
        }
        seen_facts.insert(x.id);
        audience(&x.audience, &mut records)?;
        match &x.value {
            FactValue::EntityCreated {
                entity: id,
                definition,
            } => {
                entity(*id)?;
                content(definition)?;
            }
            FactValue::EntityMoved {
                entity: id,
                destination,
                ..
            } => {
                entity(*id)?;
                entity(*destination)?;
            }
            FactValue::ResourceChanged {
                entity: id, source, ..
            } => {
                entity(*id)?;
                rule(source)?;
            }
            FactValue::DrawAccepted { operation, ordinal } => {
                require(
                    *operation == x.operation && draws.contains(&(*operation, *ordinal)),
                    CheckpointError::InvalidReference,
                )?;
            }
            FactValue::ChoiceAccepted {
                choice: selected, ..
            } => choice(selected)?,
            FactValue::RulingAccepted {
                ruling: selected, ..
            } => ruling(selected, &mut records)?,
            FactValue::TimeAdvanced { before, after } => {
                valid_time(*before)?;
                valid_time(*after)?;
                require(
                    before.ticks_per_second == after.ticks_per_second
                        && before.ticks <= after.ticks,
                    CheckpointError::InvalidTime,
                )?;
            }
            FactValue::ContentEvent {
                definition,
                subjects,
            } => {
                content(definition)?;
                count!(subjects);
                for id in subjects {
                    entity(*id)?;
                }
            }
        }
    }
    unique(state.decisions.iter().map(|x| x.operation))?;
    for x in &state.decisions {
        require(x.revision <= basis.revision, CheckpointError::StaleBasis)?;
        count!(&x.facts);
        count!(&x.draws);
        count!(&x.effects);
        unique(x.facts.iter().copied())?;
        unique(x.draws.iter().copied())?;
        unique(x.effects.iter().copied())?;
        for id in &x.facts {
            require(
                state.facts.iter().any(|record| {
                    record.id == *id
                        && record.operation == x.operation
                        && record.revision == x.revision
                }),
                CheckpointError::InvalidReference,
            )?;
        }
        for ordinal in &x.draws {
            require(
                draws.contains(&(x.operation, *ordinal)),
                CheckpointError::InvalidReference,
            )?;
        }
        for id in &x.effects {
            require(
                state
                    .intents
                    .iter()
                    .any(|record| record.id == *id && record.operation == x.operation),
                CheckpointError::InvalidReference,
            )?;
        }
        if let Some(output) = &x.semantic_output {
            text!(output);
        }
    }
    for x in &state.pending {
        require(x.basis == basis, CheckpointError::StaleBasis)?;
        fact(x.window.causal_fact)?;
        rule(&x.window.source)?;
        if let Some(id) = x.window.timer {
            require(timers.contains(&id), CheckpointError::InvalidReference)?;
        }
        count!(&x.choices);
        count!(&x.draw_ordinals);
        count!(&x.spent);
        count!(&x.rulings);
        for selected in &x.choices {
            choice(selected)?;
        }
        for id in &x.draw_ordinals {
            require(
                state
                    .draws
                    .iter()
                    .any(|draw| (draw.operation, draw.ordinal) == *id && draw.resolution == x.id),
                CheckpointError::InvalidReference,
            )?;
        }
        for spent in &x.spent {
            entity(spent.owner)?;
            rule(&spent.source)?;
            require(
                state.resources.iter().any(|resource| {
                    resource.owner == spent.owner && resource.resource == spent.resource
                }),
                CheckpointError::InvalidReference,
            )?;
        }
        for selected in &x.rulings {
            ruling(selected, &mut records)?;
        }
        match &x.next {
            PendingInput::Choice { remaining }
            | PendingInput::Reaction { remaining }
            | PendingInput::Ruling {
                permitted: remaining,
                ..
            } => {
                require(!remaining.is_empty(), CheckpointError::InvalidContinuation)?;
                count!(remaining);
                for offer in remaining {
                    member(offer.participant)?;
                    rule(&offer.source)?;
                    require(
                        !offer.options.is_empty(),
                        CheckpointError::InvalidContinuation,
                    )?;
                    count!(&offer.options);
                    unique(offer.options.iter().map(RevisionLabel::as_str))?;
                }
                if let PendingInput::Ruling { source, .. } = &x.next {
                    rule(source)?;
                }
            }
            PendingInput::Roll {
                participant,
                sides,
                source,
            } => {
                member(*participant)?;
                rule(source)?;
                count!(sides);
                require(
                    !sides.is_empty() && sides.iter().all(|value| *value > 0),
                    CheckpointError::InvalidDraw,
                )?;
            }
        }
    }
    for x in &state.intents {
        require(
            x.basis.session == basis.session
                && x.basis.run == basis.run
                && x.basis.revision.epoch() == basis.revision.epoch()
                && x.basis.revision <= basis.revision,
            CheckpointError::StaleBasis,
        )?;
        require(x.generation > 0, CheckpointError::InvalidIntent)?;
        content(&x.definition)?;
        match x.kind {
            EffectKind::RunAi
            | EffectKind::RunMedia
            | EffectKind::LoadMemoryCandidates
            | EffectKind::CancelJob => {
                require(
                    x.job.is_some() && x.timer.is_none(),
                    CheckpointError::InvalidIntent,
                )?;
            }
            EffectKind::ArmTimer | EffectKind::CancelTimer => {
                require(
                    x.job.is_none() && x.timer.is_some_and(|id| timers.contains(&id)),
                    CheckpointError::InvalidIntent,
                )?;
            }
            EffectKind::PublishPresentation => {
                require(
                    x.job.is_none() && x.timer.is_none(),
                    CheckpointError::InvalidIntent,
                )?;
            }
        }
    }
    unique(state.intents.iter().map(|x| (x.operation, x.slot)))?;
    for x in &state.timers {
        require(
            x.basis.session == basis.session
                && x.basis.run == basis.run
                && x.basis.revision.epoch() == basis.revision.epoch()
                && x.basis.revision <= basis.revision,
            CheckpointError::StaleBasis,
        )?;
        require(x.generation > 0, CheckpointError::InvalidIntent)?;
        valid_time(x.due)?;
        rule(&x.source)?;
    }
    for x in &state.active_effects {
        rule(&x.source)?;
        fact(x.origin)?;
        valid_time(x.starts)?;
        if let Some(id) = x.source_entity {
            entity(id)?;
        }
        if let Some(id) = x.concentration_owner {
            entity(id)?;
        }
        if let Some(expires) = x.expires {
            valid_time(expires)?;
            require(
                expires.ticks_per_second == x.starts.ticks_per_second
                    && expires.ticks >= x.starts.ticks,
                CheckpointError::InvalidTime,
            )?;
        }
        count!(&x.targets);
        count!(&x.choices);
        for id in &x.targets {
            entity(*id)?;
        }
        for selected in &x.choices {
            choice(selected)?;
        }
    }
    for x in &state.knowledge {
        member(x.observer)?;
        fact(x.fact)?;
        fact(x.source)?;
    }
    for x in &state.beliefs {
        entity(x.holder)?;
        entity(x.subject)?;
        content(&x.source)?;
        audience(&x.audience, &mut records)?;
        count!(&x.evidence);
        for id in &x.evidence {
            fact(*id)?;
        }
        text!(&x.claim);
    }
    for x in &state.memories {
        entity(x.holder)?;
        audience(&x.audience, &mut records)?;
        count!(&x.source_facts);
        for id in &x.source_facts {
            fact(*id)?;
        }
        text!(&x.retained_text);
        require(
            x.source_revision <= basis.revision,
            CheckpointError::StaleBasis,
        )?;
    }
    for x in &state.schedules {
        entity(x.entity)?;
        content(&x.definition)?;
        valid_time(x.due)?;
    }
    for x in &state.threats {
        content(&x.definition)?;
        require(
            x.capacity > 0 && x.progress <= x.capacity,
            CheckpointError::InvalidResource,
        )?;
    }
    for x in &state.relationships {
        entity(x.subject)?;
        entity(x.object)?;
        content(&x.policy)?;
        for axis in [
            &x.trust,
            &x.affection,
            &x.respect,
            &x.fear,
            &x.suspicion,
            &x.debt,
            &x.familiarity,
        ] {
            match &axis.provenance {
                RelationshipAxisProvenance::AuthoredBaseline { source } => content(source)?,
                RelationshipAxisProvenance::AcceptedFact {
                    source,
                    fact,
                    source_policy,
                    witness,
                } => {
                    content(source)?;
                    accepted_fact(state, *fact, source_policy)?;
                    if let Some(witness_id) = witness {
                        let witness = state
                            .continuity
                            .witnesses
                            .iter()
                            .find(|witness| witness.id == *witness_id)
                            .ok_or(CheckpointError::InvalidReference)?;
                        require(
                            witness.fact == *fact && witness.observer == x.subject,
                            CheckpointError::InvalidReference,
                        )?;
                    }
                }
            }
        }
    }
    for x in &state.conversations {
        content(&x.topic)?;
        count!(&x.participants);
        count!(&x.accepted_facts);
        for id in &x.participants {
            entity(*id)?;
        }
        for id in &x.accepted_facts {
            fact(*id)?;
        }
    }
    for x in &state.obligations {
        entity(x.obligor)?;
        entity(x.beneficiary)?;
        content(&x.definition)?;
        content(&x.terms)?;
        content(&x.agreement.source)?;
        valid_time(x.agreement.at)?;
        require(
            x.agreement.at.ticks_per_second == state.logical_time.ticks_per_second
                && x.agreement.at.ticks <= state.logical_time.ticks,
            CheckpointError::InvalidTime,
        )?;
        if let Some(due) = x.due {
            require(
                due.ticks_per_second == x.agreement.at.ticks_per_second
                    && due.ticks >= x.agreement.at.ticks,
                CheckpointError::InvalidTime,
            )?;
        }
        let agreement = accepted_fact(state, x.agreement.fact, &x.agreement.source_policy)?;
        require(
            matches!(
                &agreement.value,
                FactValue::ContentEvent { definition, subjects }
                    if definition == &x.agreement.source
                        && subjects.as_slice() == [x.obligor, x.beneficiary]
            ),
            CheckpointError::InvalidReference,
        )?;
        if let Some(due) = x.due {
            valid_time(due)?;
        }
        match (&x.status, &x.transition) {
            (ObligationStatus::Active, None) => {}
            (ObligationStatus::Active, Some(_)) | (_, None) => {
                return Err(CheckpointError::InvalidReference);
            }
            (status, Some(transition)) => {
                content(&transition.source)?;
                require(
                    transition.fact != x.agreement.fact,
                    CheckpointError::InvalidReference,
                )?;
                valid_time(transition.at)?;
                require(
                    transition.at.ticks_per_second == state.logical_time.ticks_per_second
                        && transition.at.ticks <= state.logical_time.ticks
                        && transition.at.ticks >= x.agreement.at.ticks,
                    CheckpointError::InvalidTime,
                )?;
                if transition.action == ObligationAction::Expire {
                    let due = x.due.ok_or(CheckpointError::InvalidTime)?;
                    require(
                        due.ticks_per_second == transition.at.ticks_per_second
                            && transition.at.ticks >= due.ticks,
                        CheckpointError::InvalidTime,
                    )?;
                }
                let accepted = accepted_fact(state, transition.fact, &transition.source_policy)?;
                require(
                    matches!(
                        &accepted.value,
                        FactValue::ContentEvent { definition, subjects }
                            if definition == &transition.source
                                && subjects.as_slice() == [x.obligor, x.beneficiary]
                    ),
                    CheckpointError::InvalidReference,
                )?;
                let matching_action = matches!(
                    (*status, transition.action),
                    (ObligationStatus::Fulfilled, ObligationAction::Fulfill)
                        | (ObligationStatus::Broken, ObligationAction::Break)
                        | (ObligationStatus::Expired, ObligationAction::Expire)
                        | (ObligationStatus::Cancelled, ObligationAction::Cancel)
                );
                require(matching_action, CheckpointError::InvalidReference)?;
            }
        }
    }
    content(&state.narrative.definition)?;
    count!(&state.narrative.active_beats);
    count!(&state.narrative.completed_beats);
    count!(&state.narrative.open_threads);
    count!(&state.narrative.accepted_facts);
    for x in state
        .narrative
        .active_beats
        .iter()
        .chain(&state.narrative.completed_beats)
        .chain(&state.narrative.open_threads)
    {
        content(x)?;
    }
    for id in &state.narrative.accepted_facts {
        fact(*id)?;
    }
    for x in &state.encounters {
        if let Some(tactical) = &x.tactical {
            require(tactical.schema == 1, CheckpointError::UnsupportedSchema)?;
            count!(&tactical.actors);
            require(
                !tactical.scene.is_empty()
                    && tactical.scene.len() <= 1024 * 1024
                    && !tactical.state.is_empty()
                    && tactical.state.len() <= 1024 * 1024
                    && tactical.actors.len() == x.participants.len(),
                CheckpointError::Capacity,
            )?;
            unique(tactical.actors.iter().map(|(id, _)| *id))?;
            unique(tactical.actors.iter().map(|(_, entity)| *entity))?;
            require(
                tactical
                    .actors
                    .iter()
                    .all(|(id, entity)| *id != 0 && x.participants.contains(entity)),
                CheckpointError::InvalidReference,
            )?;
        }
        content(&x.definition)?;
        content(&x.combat_policy)?;
        count!(&x.participants);
        count!(&x.turn_order);
        count!(&x.objectives);
        let participants = unique(x.participants.iter().copied())?;
        unique(x.turn_order.iter().copied())?;
        for id in &x.participants {
            entity(*id)?;
        }
        for id in &x.turn_order {
            require(participants.contains(id), CheckpointError::InvalidReference)?;
        }
        if let Some(id) = x.active_turn {
            require(
                x.turn_order.contains(&id),
                CheckpointError::InvalidReference,
            )?;
        }
        for definition in &x.objectives {
            content(definition)?;
        }
    }
    for x in &state.activity {
        member(x.member)?;
        valid_time(x.started)?;
        valid_time(x.ends)?;
        count!(&x.observed_facts);
        require(
            x.started.ticks_per_second == x.ends.ticks_per_second
                && x.started.ticks <= x.ends.ticks,
            CheckpointError::InvalidTime,
        )?;
        for id in &x.observed_facts {
            fact(*id)?;
        }
    }
    content(&state.tempo.policy)?;
    count!(&state.tempo.fatigue);
    for (definition, _) in &state.tempo.fatigue {
        content(definition)?;
    }
    for x in &state.presentation {
        content(&x.definition)?;
        audience(&x.audience, &mut records)?;
        count!(&x.causal_facts);
        require(
            x.source_revision <= basis.revision,
            CheckpointError::StaleBasis,
        )?;
        for id in &x.causal_facts {
            fact(*id)?;
        }
    }
    let mut progress = ValidationProgress {
        records,
        text_bytes: texts,
    };
    validate_continuity(
        &state.continuity,
        state,
        basis,
        pins,
        &inventory,
        limits,
        &mut progress,
    )?;
    Ok(())
}
fn unique<T: Ord>(values: impl Iterator<Item = T>) -> Result<BTreeSet<T>, CheckpointError> {
    let mut seen = BTreeSet::new();
    for value in values {
        if !seen.insert(value) {
            return Err(CheckpointError::DuplicateIdentity);
        }
    }
    Ok(seen)
}
fn require(condition: bool, error: CheckpointError) -> Result<(), CheckpointError> {
    if condition { Ok(()) } else { Err(error) }
}
fn valid_time(time: LogicalTime) -> Result<(), CheckpointError> {
    require(time.ticks_per_second > 0, CheckpointError::InvalidTime)
}
fn count_records(records: &mut usize, count: usize, maximum: usize) -> Result<(), CheckpointError> {
    *records = records
        .checked_add(count)
        .ok_or(CheckpointError::Capacity)?;
    require(*records <= maximum, CheckpointError::Capacity)
}
fn ordered<T: Ord>(
    last: &mut std::collections::BTreeMap<T, u32>,
    key: T,
    ordinal: u32,
) -> Result<(), CheckpointError> {
    if let Some(previous) = last.insert(key, ordinal) {
        require(
            previous.checked_add(1) == Some(ordinal),
            CheckpointError::InvalidOrder,
        )
    } else {
        require(ordinal == 0, CheckpointError::InvalidOrder)
    }
}

// Capacity traversal uses the canonical primitive allocation accessors.
// Private traversal counts live allocation capacities, including nested vectors and label strings.
use df_types::BuildRevision;
use std::mem::size_of;
trait RetainedHeap {
    fn retained_heap(&self) -> Option<usize>;
}
impl<T: RetainedHeap> RetainedHeap for Vec<T> {
    fn retained_heap(&self) -> Option<usize> {
        self.iter().try_fold(
            self.capacity().checked_mul(size_of::<T>())?,
            |sum, value| sum.checked_add(value.retained_heap()?),
        )
    }
}
impl<T: RetainedHeap> RetainedHeap for Option<T> {
    fn retained_heap(&self) -> Option<usize> {
        match self {
            Some(value) => value.retained_heap(),
            None => Some(0),
        }
    }
}
impl RetainedHeap for String {
    fn retained_heap(&self) -> Option<usize> {
        Some(self.capacity())
    }
}
impl RetainedHeap for RevisionLabel {
    fn retained_heap(&self) -> Option<usize> {
        // The primitive owns its allocation; string length would undercount spare capacity.
        Some(self.retained_heap_bytes())
    }
}
impl RetainedHeap for BuildIdentity {
    fn retained_heap(&self) -> Option<usize> {
        [
            BuildRevision::Source,
            BuildRevision::Native,
            BuildRevision::Wasm,
            BuildRevision::Configuration,
            BuildRevision::Content,
        ]
        .into_iter()
        .try_fold(0usize, |sum, key| {
            sum.checked_add(self.revision(key).retained_heap()?)
        })
    }
}
impl<A: RetainedHeap, B: RetainedHeap> RetainedHeap for (A, B) {
    fn retained_heap(&self) -> Option<usize> {
        self.0.retained_heap()?.checked_add(self.1.retained_heap()?)
    }
}
macro_rules! no_heap {
    ($($kind:ty),+ $(,)?) => {$ (
        impl RetainedHeap for $kind {
            fn retained_heap(&self) -> Option<usize> { Some(0) }
        }
    )+ };
}
no_heap!(
    bool,
    u8,
    u16,
    u32,
    u64,
    i64,
    SessionId,
    MemberId,
    OperationId,
    RunId,
    SessionRevision,
    Basis,
    ContentDigest,
    RulesMode,
    ExecutionMode,
    LogicalTime,
    Position,
    EntityId,
    FactId,
    ResolutionId,
    WindowId,
    JobId,
    TimerId,
    EffectId,
    RecordId,
    MembershipLink,
    KnowledgeGrant,
    EffectKind,
    DurableStatus,
    TriggerPhase
);
macro_rules! record_heap {
    ($kind:ty; $($field:ident),+ $(,)?) => {
        impl RetainedHeap for $kind {
            fn retained_heap(&self) -> Option<usize> {
                let mut sum = 0usize;
                $(sum = sum.checked_add(self.$field.retained_heap()?)?;)+
                Some(sum)
            }
        }
    };
}
record_heap!(RulesPins; ruleset, catalog, source_manifest, handler);
record_heap!(ContentPins; content, package);
record_heap!(CheckpointPins; rules, content, build);
record_heap!(RuleReference; catalog, source, entry, clause);
record_heap!(ContentReference; package, entry);
record_heap!(WorldEntity; definition, identity_revision);
record_heap!(CharacterState; build, choices);
record_heap!(ResourceState; resource, source);
record_heap!(ResourceConstraint; resource, source);
record_heap!(InventoryItem; source, origin);
record_heap!(ActualDraw; source);
record_heap!(AcceptedChoice; offer, selected, source);
record_heap!(ResourceSpend; resource, source);
record_heap!(ScopedRuling; selected, source, audience);
record_heap!(ResolutionWindow; source);
record_heap!(OfferedResponse; offer, options, source);
record_heap!(PendingResolution; continuation, window, next, choices, draw_ordinals, spent, rulings);
record_heap!(GameFact; audience, value);
record_heap!(AcceptedDecision; facts, draws, effects, source_policy, semantic_output);
record_heap!(DurableIntent; definition);
record_heap!(OwnedTimer; source);
record_heap!(ActiveEffect; source, targets, choices);
record_heap!(AttributedClaim; claim, evidence, audience, source);
record_heap!(MemoryEpisode; source_facts, retained_text, audience);
record_heap!(ScheduledEvent; definition);
record_heap!(ThreatClock; definition);
impl RetainedHeap for RelationshipAxisProvenance {
    fn retained_heap(&self) -> Option<usize> {
        match self {
            Self::AuthoredBaseline { source } => source.retained_heap(),
            Self::AcceptedFact {
                source,
                source_policy,
                ..
            } => source
                .retained_heap()?
                .checked_add(source_policy.retained_heap()?),
        }
    }
}
record_heap!(RelationshipAxisState; value, provenance);
record_heap!(Relationship; policy, state, trust, affection, respect, fear, suspicion, debt,
    familiarity);
record_heap!(ConversationState; participants, topic, accepted_facts);
record_heap!(ObligationAgreement; source, source_policy);
record_heap!(ObligationTransition; source, source_policy);
record_heap!(Obligation; definition, terms, agreement, transition);
record_heap!(NarrativeState; definition, active_beats, completed_beats, open_threads, accepted_facts);
record_heap!(TacticalSnapshot; source_revision, scene, layout, actors, state);
record_heap!(EncounterState; definition, participants, turn_order, objectives, combat_policy, tactical);
record_heap!(ActivityWindow; observed_facts);
record_heap!(TempoState; policy, fatigue);
record_heap!(PresentationDemand; definition, audience, causal_facts);
record_heap!(GameState; members, entities, characters, resources, inventory, facts, draws, decisions,
    pending, intents, timers, active_effects, knowledge, beliefs, memories, schedules, threats,
    relationships, conversations, obligations, narrative, encounters, activity, tempo, presentation, continuity);
record_heap!(Checkpoint; pins, state);
impl RetainedHeap for AudienceScope {
    fn retained_heap(&self) -> Option<usize> {
        match self {
            Self::Members(ids) => ids.retained_heap(),
            Self::Shared | Self::Host => Some(0),
        }
    }
}
impl RetainedHeap for PendingInput {
    fn retained_heap(&self) -> Option<usize> {
        match self {
            Self::Choice { remaining } | Self::Reaction { remaining } => remaining.retained_heap(),
            Self::Roll { sides, source, .. } => {
                sides.retained_heap()?.checked_add(source.retained_heap()?)
            }
            Self::Ruling { permitted, source } => permitted
                .retained_heap()?
                .checked_add(source.retained_heap()?),
        }
    }
}
impl RetainedHeap for FactValue {
    fn retained_heap(&self) -> Option<usize> {
        match self {
            Self::EntityCreated { definition, .. } => definition.retained_heap(),
            Self::EntityMoved { .. } | Self::DrawAccepted { .. } | Self::TimeAdvanced { .. } => {
                Some(0)
            }
            Self::ResourceChanged {
                resource, source, ..
            } => resource
                .retained_heap()?
                .checked_add(source.retained_heap()?),
            Self::ChoiceAccepted { choice, .. } => choice.retained_heap(),
            Self::RulingAccepted { ruling, .. } => ruling.retained_heap(),
            Self::ContentEvent {
                definition,
                subjects,
            } => definition
                .retained_heap()?
                .checked_add(subjects.retained_heap()?),
        }
    }
}
impl Checkpoint {
    /// Inline object plus all retained Rust allocation capacities; excludes allocator metadata.
    /// Checked arithmetic returns None on overflow, which owner admission must reject.
    pub fn retained_bytes(&self) -> Option<usize> {
        size_of::<Self>().checked_add(self.retained_heap()?)
    }
    pub fn retained_heap_bytes(&self) -> Option<usize> {
        self.retained_heap()
    }
}

use df_types::{ClientBindingId, LocaleTag, RecoveryEpoch};

/// Source-defined character creation is pending until its legal offers are accepted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CreationPhase {
    Selecting,
    AwaitingValidation,
    Accepted,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CharacterDraft {
    pub entity: EntityId,
    pub member: MemberId,
    pub ancestry: Option<ContentReference>,
    pub background: Option<ContentReference>,
    pub classes: Vec<ContentReference>,
    pub choices: Vec<AcceptedChoice>,
    pub phase: CreationPhase,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SimulationTier {
    Active,
    Scheduled,
    Dormant,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EntitySimulation {
    pub entity: EntityId,
    pub tier: SimulationTier,
    pub policy: ContentReference,
    pub last_advanced: LogicalTime,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatchUpCursor {
    pub last_processed: Option<RecordId>,
    pub pending_events: Vec<RecordId>,
    pub policy: ContentReference,
    pub time: LogicalTime,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EnvironmentalState {
    pub location: EntityId,
    pub definition: ContentReference,
    pub change_facts: Vec<FactId>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TravelLeg {
    pub id: RecordId,
    pub travelers: Vec<EntityId>,
    pub from: EntityId,
    pub destination: EntityId,
    pub route: ContentReference,
    pub starts: LogicalTime,
    pub arrives: LogicalTime,
    pub source: RuleReference,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WitnessRecord {
    pub id: RecordId,
    pub observer: EntityId,
    pub fact: FactId,
    pub perceived_at: LogicalTime,
    pub source: ContentReference,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RumorTransmission {
    pub id: RecordId,
    pub claim: RecordId,
    pub sender: EntityId,
    pub recipient: EntityId,
    pub evidence: Vec<FactId>,
    pub policy: ContentReference,
    pub remaining_hops: u32,
    pub audience: AudienceScope,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JournalEntry {
    pub id: RecordId,
    pub source_facts: Vec<FactId>,
    pub attributed_claims: Vec<RecordId>,
    pub audience: AudienceScope,
    pub text: String,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemorySummary {
    pub id: RecordId,
    pub episodes: Vec<RecordId>,
    pub derived_claims: Vec<RecordId>,
    pub source_digest: ContentDigest,
    pub source_revision: SessionRevision,
    pub summarizer: RevisionLabel,
    pub model: RevisionLabel,
    pub policy: ContentReference,
    pub audience: AudienceScope,
    pub text: String,
    pub incomplete: bool,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetrievalPurpose {
    NpcContext,
    PlayerRecall,
    RecapHistory,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RetrievalRequest {
    pub id: RecordId,
    pub basis: Basis,
    pub observer: MemberId,
    pub purpose: RetrievalPurpose,
    pub topics: Vec<ContentReference>,
    pub entities: Vec<EntityId>,
    pub from: LogicalTime,
    pub through: LogicalTime,
    /// Aggregate count of episode, fact, attributed-claim and snippet entries.
    pub maximum_items: u32,
    /// Aggregate UTF-8 snippet bytes retained in the selected result.
    pub maximum_bytes: u64,
    /// The admitted native tokenizer enforces this bound before candidate acceptance.
    pub maximum_tokens: u64,
    pub access_generation: u64,
    pub index_generation: u64,
    pub source_digest: ContentDigest,
    pub policy: ContentReference,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RetrievedMemory {
    pub request: RecordId,
    pub episodes: Vec<RecordId>,
    pub facts: Vec<FactId>,
    pub attributed_claims: Vec<RecordId>,
    pub snippets: Vec<String>,
    pub source_revision: SessionRevision,
    pub index_generation: u64,
    pub access_generation: u64,
    pub incomplete: bool,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConsolidationCandidate {
    pub id: RecordId,
    pub job: JobId,
    pub basis: Basis,
    pub episodes: Vec<RecordId>,
    pub summaries: Vec<RecordId>,
    pub source_digest: ContentDigest,
    pub source_revision: SessionRevision,
    pub access_generation: u64,
    pub index_generation: u64,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SecretPolicy {
    pub holder: EntityId,
    pub claims: Vec<RecordId>,
    pub policy: ContentReference,
    pub permitted_audience: AudienceScope,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NpcState {
    pub entity: EntityId,
    pub role: ContentReference,
    pub personality: ContentReference,
    pub motivations: Vec<ContentReference>,
    pub goals: Vec<ContentReference>,
    pub needs: Vec<ContentReference>,
    pub fears: Vec<ContentReference>,
    pub known_facts: Vec<FactId>,
    pub beliefs: Vec<RecordId>,
    pub secrets: Vec<SecretPolicy>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CharacterHook {
    pub id: RecordId,
    pub member: MemberId,
    pub definition: ContentReference,
    pub source_facts: Vec<FactId>,
    pub consent_generation: u64,
    pub audience: AudienceScope,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoryArc {
    pub id: RecordId,
    pub definition: ContentReference,
    pub phase: ContentReference,
    pub source_facts: Vec<FactId>,
    pub active_hooks: Vec<RecordId>,
}

/// Membership, observed presence, capture and output routing remain distinct authority records.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemotePlayPolicy {
    pub definition: ContentReference,
    pub maximum_participants: u32,
    pub maximum_connections: u32,
    pub maximum_capture_leases: u32,
    pub device_matrix: RevisionLabel,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PresenceKind {
    Connected,
    Sleeping,
    Disconnected,
    VoluntaryAfk,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParticipantPresence {
    pub member: MemberId,
    pub bindings: Vec<ClientBindingId>,
    pub state: PresenceKind,
    pub policy: ContentReference,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AudioDestination {
    PublicRoom(ClientBindingId),
    PrivateListener {
        member: MemberId,
        binding: ClientBindingId,
    },
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AudioOutputLease {
    pub id: RecordId,
    pub destination: AudioDestination,
    pub generation: u64,
    pub audience: AudienceScope,
    pub device_unlocked: bool,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CaptureLease {
    pub id: RecordId,
    pub member: MemberId,
    pub binding: ClientBindingId,
    pub generation: u64,
    pub permitted_offer: Option<WindowId>,
    pub format: RevisionLabel,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AudioTopology {
    pub policy: ContentReference,
    pub outputs: Vec<AudioOutputLease>,
    pub captures: Vec<CaptureLease>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContextualPrivateOffer {
    pub id: RecordId,
    pub member: MemberId,
    pub resolution: ResolutionId,
    pub window: WindowId,
    pub facts: Vec<FactId>,
    pub policy: ContentReference,
    pub access_generation: u64,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KnowledgeCue {
    pub id: RecordId,
    pub source_facts: Vec<FactId>,
    pub audience: AudienceScope,
    pub definition: ContentReference,
}

/// Bytes are immutable df-assets-owned objects; model retains references and lifecycle meaning.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AssetKind {
    Image,
    Audio,
    Video,
    TacticalGeometry,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssetReference {
    pub key: RevisionLabel,
    pub digest: ContentDigest,
    pub byte_length: u64,
    pub kind: AssetKind,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssetRequestKey {
    pub schema: u16,
    pub source: ContentDigest,
    pub moment: RecordId,
    pub identity: RevisionLabel,
    pub style: RevisionLabel,
    pub voice: Option<RevisionLabel>,
    pub provider: RevisionLabel,
    pub model: RevisionLabel,
    pub format: RevisionLabel,
    pub references: Vec<AssetReference>,
    pub audience: AudienceScope,
    pub parameters: RevisionLabel,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DemandPriority {
    InteractionCritical,
    SoonLikely,
    Optional,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NarrativeMoment {
    pub id: RecordId,
    pub location: EntityId,
    pub characters: Vec<EntityId>,
    pub facts: Vec<FactId>,
    pub attributed_claims: Vec<RecordId>,
    pub audience: AudienceScope,
    pub semantic_focus: ContentReference,
    pub identity_revision: RevisionLabel,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssetDemand {
    pub id: RecordId,
    pub basis: Basis,
    pub key: AssetRequestKey,
    pub priority: DemandPriority,
    pub mode: ExecutionMode,
    pub expires: LogicalTime,
    pub budget_reservation: RevisionLabel,
    pub maximum_bytes: u64,
    pub policy: ContentReference,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AssetLifecycle {
    Missing,
    Queued,
    Generating,
    Ready,
    Failed,
    Cancelled,
    Stale,
    Superseded,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssetJobState {
    pub job: JobId,
    pub demand: RecordId,
    pub generation: u64,
    pub state: AssetLifecycle,
    pub published: Option<AssetReference>,
    pub dispatch: DurableStatus,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssetDependency {
    pub asset: AssetReference,
    pub prerequisites: Vec<AssetReference>,
}
/// Accepted cosmetic descriptions only; they never establish rules or equipment statistics.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CharacterAppearance {
    pub features: String,
    pub outfit: String,
    pub outfit_revision: RevisionLabel,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EntityIdentityRevision {
    pub entity: EntityId,
    pub revision: RevisionLabel,
    pub character_appearance: Option<CharacterAppearance>,
    pub source_facts: Vec<FactId>,
    pub appearances: Vec<AssetReference>,
    pub voice: Option<AssetReference>,
    pub sound: Vec<AssetReference>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VisualBible {
    pub revision: RevisionLabel,
    pub definition: ContentReference,
    pub palette: Vec<RevisionLabel>,
    pub style: String,
    pub references: Vec<AssetReference>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalPack {
    pub revision: RevisionLabel,
    pub digest: ContentDigest,
    pub bible: VisualBible,
    pub identities: Vec<EntityIdentityRevision>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PerformanceHint {
    pub definition: ContentReference,
    pub voice: Option<AssetReference>,
    pub emphasis_facts: Vec<FactId>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShotPlan {
    pub id: RecordId,
    pub moment: RecordId,
    pub subjects: Vec<EntityId>,
    pub audience: AudienceScope,
    pub duration_ticks: u64,
    pub definition: ContentReference,
    pub references: Vec<AssetReference>,
    pub performance: PerformanceHint,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrefetchPolicy {
    pub definition: ContentReference,
    pub maximum_candidates: u32,
    pub maximum_branches: u32,
    pub maximum_bytes: u64,
    pub maximum_duration_ticks: u64,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SceneIdentityRevision {
    pub scene: EntityId,
    pub revision: RevisionLabel,
    pub source_facts: Vec<FactId>,
    pub geometry: AssetReference,
    pub canonical_pack: RevisionLabel,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ItemOrigin {
    pub item: EntityId,
    pub award_operation: OperationId,
    pub source_fact: FactId,
    pub definition: ContentReference,
    pub identity_revision: RevisionLabel,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BookendKind {
    Recap,
    SpeculativeTrailer,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BookendSpec {
    pub id: RecordId,
    pub basis: Basis,
    pub kind: BookendKind,
    pub from: SessionRevision,
    pub through: SessionRevision,
    pub audience: AudienceScope,
    pub locale: LocaleTag,
    pub policy: ContentReference,
    pub maximum_shots: u32,
    pub maximum_duration_ticks: u64,
    pub maximum_text_bytes: u64,
    pub maximum_asset_bytes: u64,
    pub expires: LogicalTime,
    pub mode: ExecutionMode,
    pub budget_reservation: RevisionLabel,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FactSelection {
    pub facts: Vec<FactId>,
    pub attributed_claims: Vec<RecordId>,
    pub audience: AudienceScope,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BookendPlan {
    pub spec: BookendSpec,
    pub selection: FactSelection,
    pub shots: Vec<ShotPlan>,
    pub captions: Vec<String>,
    pub fallback: AssetReference,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExportGrant {
    pub id: RecordId,
    pub bookend: RecordId,
    pub recipients: Vec<MemberId>,
    pub selection: FactSelection,
    pub assets: Vec<AssetReference>,
    pub rights: Vec<RevisionLabel>,
    pub source_revision: SessionRevision,
    pub access_generation: u64,
    pub suppression_generation: u64,
    pub expires: LogicalTime,
    pub revoked: bool,
    pub downloaded_copy_retraction_supported: bool,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CriticalCueEligibility {
    pub id: RecordId,
    pub fact: FactId,
    pub resolution: ResolutionId,
    pub audience: AudienceScope,
    pub ready_assets: Vec<AssetReference>,
    pub policy: ContentReference,
    pub maximum_duration_ticks: u64,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContentCandidate {
    pub id: RecordId,
    pub basis: Basis,
    pub generator: RevisionLabel,
    pub model: RevisionLabel,
    pub policy: ContentReference,
    pub template: ContentReference,
    pub source: RuleReference,
    pub origin: FactId,
    pub audience: AudienceScope,
    pub bounded_parameters: Vec<(RevisionLabel, i64)>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContentValidation {
    StandardCompatible,
    NeedsCustomApproval,
    UnsupportedMechanic,
    SourceGap,
    Rejected,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContentAdmission {
    pub candidate: RecordId,
    pub result: ContentValidation,
    pub definition: ContentReference,
    pub definition_digest: ContentDigest,
    pub source: RuleReference,
    pub handler: RevisionLabel,
    pub ruleset: RevisionLabel,
    pub approval: Option<MemberId>,
    pub disclosed_to: Vec<MemberId>,
    pub operation: OperationId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RestoreKind {
    DebugFork,
    Disaster,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RestoreOrigin {
    pub kind: RestoreKind,
    pub checkpoint_digest: ContentDigest,
    pub origin: Basis,
    pub process_generation: u64,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LostGameRange {
    pub from: SessionRevision,
    pub through: SessionRevision,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveryState {
    pub origin: Option<RestoreOrigin>,
    pub retired_epochs: Vec<RecoveryEpoch>,
    pub lost_ranges: Vec<LostGameRange>,
    pub suppression_generation: u64,
    pub redacted_records: Vec<RecordId>,
    pub unavailable_sources: Vec<RevisionLabel>,
}

/// Every family is persisted once here; native owners still authenticate, fence and project it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContinuityState {
    pub creation: Vec<CharacterDraft>,
    pub simulation: Vec<EntitySimulation>,
    pub catch_up: Option<CatchUpCursor>,
    pub environment: Vec<EnvironmentalState>,
    pub travel: Vec<TravelLeg>,
    pub witnesses: Vec<WitnessRecord>,
    pub rumors: Vec<RumorTransmission>,
    pub journal: Vec<JournalEntry>,
    pub summaries: Vec<MemorySummary>,
    pub retrieval: Vec<RetrievalRequest>,
    pub retrieved: Vec<RetrievedMemory>,
    pub consolidation: Vec<ConsolidationCandidate>,
    pub npcs: Vec<NpcState>,
    pub hooks: Vec<CharacterHook>,
    pub arcs: Vec<StoryArc>,
    pub remote: Option<RemotePlayPolicy>,
    pub presence: Vec<ParticipantPresence>,
    pub audio: Option<AudioTopology>,
    pub private_offers: Vec<ContextualPrivateOffer>,
    pub knowledge_cues: Vec<KnowledgeCue>,
    pub moments: Vec<NarrativeMoment>,
    pub demands: Vec<AssetDemand>,
    pub asset_jobs: Vec<AssetJobState>,
    pub asset_dependencies: Vec<AssetDependency>,
    pub canonical_packs: Vec<CanonicalPack>,
    pub shots: Vec<ShotPlan>,
    pub prefetch: Option<PrefetchPolicy>,
    pub scenes: Vec<SceneIdentityRevision>,
    pub item_origins: Vec<ItemOrigin>,
    pub bookends: Vec<BookendPlan>,
    pub exports: Vec<ExportGrant>,
    pub critical_cues: Vec<CriticalCueEligibility>,
    pub content_candidates: Vec<ContentCandidate>,
    pub content_admissions: Vec<ContentAdmission>,
    pub recovery: RecoveryState,
}

struct ValidationProgress {
    records: usize,
    text_bytes: usize,
}

fn validate_continuity(
    all: &ContinuityState,
    state: &GameState,
    basis: Basis,
    pins: &CheckpointPins,
    inventory: &ReferenceInventory<'_>,
    limits: CheckpointLimits,
    progress: &mut ValidationProgress,
) -> Result<(), CheckpointError> {
    macro_rules! count {
        ($items:expr) => {{
            progress.records = progress
                .records
                .checked_add($items.len())
                .ok_or(CheckpointError::Capacity)?;
            require(
                progress.records <= limits.maximum_records,
                CheckpointError::Capacity,
            )?;
        }};
    }
    macro_rules! text {
        ($value:expr) => {{
            let value = $value;
            require(
                value.len() <= limits.maximum_text_bytes,
                CheckpointError::Capacity,
            )?;
            progress.text_bytes = progress
                .text_bytes
                .checked_add(value.len())
                .ok_or(CheckpointError::Capacity)?;
            require(
                progress.text_bytes <= limits.maximum_total_text_bytes,
                CheckpointError::Capacity,
            )?;
        }};
    }
    macro_rules! top { ($($field:ident),+ $(,)?) => { $(count!(&all.$field);)+ }; }
    top!(
        creation,
        simulation,
        environment,
        travel,
        witnesses,
        rumors,
        journal,
        summaries,
        retrieval,
        retrieved,
        consolidation,
        npcs,
        hooks,
        arcs,
        presence,
        private_offers,
        knowledge_cues,
        moments,
        demands,
        asset_jobs,
        asset_dependencies,
        canonical_packs,
        shots,
        scenes,
        item_origins,
        bookends,
        exports,
        critical_cues,
        content_candidates,
        content_admissions
    );
    let entities = unique(state.entities.iter().map(|x| x.id))?;
    let members = unique(state.members.iter().map(|x| x.member))?;
    let facts = unique(state.facts.iter().map(|x| x.id))?;
    let episodes = unique(state.memories.iter().map(|x| x.id))?;
    unique(state.schedules.iter().map(|x| x.id))?;
    let claims = unique(state.beliefs.iter().map(|x| x.id))?;
    let summaries = unique(all.summaries.iter().map(|x| x.id))?;
    unique(all.retrieval.iter().map(|x| x.id))?;
    let moments = unique(all.moments.iter().map(|x| x.id))?;
    let demands = unique(all.demands.iter().map(|x| x.id))?;
    let hooks = unique(all.hooks.iter().map(|x| x.id))?;
    let candidates = unique(all.content_candidates.iter().map(|x| x.id))?;
    let bookends = unique(all.bookends.iter().map(|x| x.spec.id))?;
    let entity = |id| require(entities.contains(&id), CheckpointError::InvalidReference);
    let member = |id| require(members.contains(&id), CheckpointError::InvalidReference);
    let fact = |id| require(facts.contains(&id), CheckpointError::InvalidReference);
    let claim = |id| require(claims.contains(&id), CheckpointError::InvalidReference);
    let episode = |id| require(episodes.contains(&id), CheckpointError::InvalidReference);
    let content = |x: &ContentReference| {
        require(
            x.package == pins.content.package && inventory.content.contains(x),
            CheckpointError::InvalidReference,
        )
    };
    let rule = |x: &RuleReference| {
        require(
            x.catalog == pins.rules.catalog && inventory.rules.contains(x),
            CheckpointError::InvalidReference,
        )
    };
    let asset = |x: &AssetReference| {
        require(
            x.byte_length > 0 && inventory.assets.contains(x),
            CheckpointError::InvalidReference,
        )
    };
    let current_basis = |x: Basis| {
        require(
            x.session == basis.session && x.run == basis.run && x.revision <= basis.revision,
            CheckpointError::StaleBasis,
        )
    };
    let audience = |x: &AudienceScope, records: &mut usize| -> Result<(), CheckpointError> {
        if let AudienceScope::Members(ids) = x {
            count_records(records, ids.len(), limits.maximum_records)?;
            unique(ids.iter().copied())?;
            for id in ids {
                member(*id)?;
            }
        }
        Ok(())
    };
    let time_range = |from: LogicalTime, through: LogicalTime| -> Result<(), CheckpointError> {
        valid_time(from)?;
        valid_time(through)?;
        require(
            from.ticks_per_second == through.ticks_per_second && from.ticks <= through.ticks,
            CheckpointError::InvalidTime,
        )
    };
    unique(all.creation.iter().map(|x| x.entity))?;
    unique(all.simulation.iter().map(|x| x.entity))?;
    unique(all.travel.iter().map(|x| x.id))?;
    unique(all.witnesses.iter().map(|x| x.id))?;
    unique(all.rumors.iter().map(|x| x.id))?;
    unique(all.journal.iter().map(|x| x.id))?;
    unique(all.consolidation.iter().map(|x| x.id))?;
    unique(all.npcs.iter().map(|x| x.entity))?;
    unique(all.arcs.iter().map(|x| x.id))?;
    unique(all.presence.iter().map(|x| x.member))?;
    unique(all.private_offers.iter().map(|x| x.id))?;
    unique(all.knowledge_cues.iter().map(|x| x.id))?;
    unique(all.asset_jobs.iter().map(|x| x.job))?;
    unique(all.shots.iter().map(|x| x.id))?;
    unique(all.exports.iter().map(|x| x.id))?;
    unique(all.critical_cues.iter().map(|x| x.id))?;
    unique(all.asset_dependencies.iter().map(|x| x.asset.key.as_str()))?;
    for x in &all.creation {
        entity(x.entity)?;
        member(x.member)?;
        if let Some(value) = &x.ancestry {
            content(value)?;
        }
        if let Some(value) = &x.background {
            content(value)?;
        }
        count!(&x.classes);
        count!(&x.choices);
        for value in &x.classes {
            content(value)?;
        }
        for value in &x.choices {
            member(value.participant)?;
            rule(&value.source)?;
        }
    }
    for x in &all.simulation {
        entity(x.entity)?;
        content(&x.policy)?;
        valid_time(x.last_advanced)?;
    }
    if let Some(x) = &all.catch_up {
        content(&x.policy)?;
        valid_time(x.time)?;
        count!(&x.pending_events);
        unique(x.pending_events.iter().copied())?;
        for id in &x.pending_events {
            require(
                state.schedules.iter().any(|event| event.id == *id),
                CheckpointError::InvalidReference,
            )?;
        }
    }
    for x in &all.environment {
        entity(x.location)?;
        content(&x.definition)?;
        count!(&x.change_facts);
        for id in &x.change_facts {
            fact(*id)?;
        }
    }
    for x in &all.travel {
        entity(x.from)?;
        entity(x.destination)?;
        content(&x.route)?;
        rule(&x.source)?;
        count!(&x.travelers);
        for id in &x.travelers {
            entity(*id)?;
        }
        time_range(x.starts, x.arrives)?;
    }
    for x in &all.witnesses {
        entity(x.observer)?;
        fact(x.fact)?;
        valid_time(x.perceived_at)?;
        content(&x.source)?;
    }
    for x in &all.rumors {
        claim(x.claim)?;
        entity(x.sender)?;
        entity(x.recipient)?;
        content(&x.policy)?;
        audience(&x.audience, &mut progress.records)?;
        count!(&x.evidence);
        for id in &x.evidence {
            fact(*id)?;
        }
    }
    for x in &all.journal {
        audience(&x.audience, &mut progress.records)?;
        count!(&x.source_facts);
        count!(&x.attributed_claims);
        for id in &x.source_facts {
            fact(*id)?;
        }
        for id in &x.attributed_claims {
            claim(*id)?;
        }
        text!(&x.text);
    }
    for x in &all.summaries {
        require(
            x.source_revision <= basis.revision,
            CheckpointError::StaleBasis,
        )?;
        require(
            x.source_digest == pins.content.content_digest,
            CheckpointError::ContentMismatch,
        )?;
        content(&x.policy)?;
        audience(&x.audience, &mut progress.records)?;
        count!(&x.episodes);
        count!(&x.derived_claims);
        for id in &x.episodes {
            episode(*id)?;
        }
        for id in &x.derived_claims {
            claim(*id)?;
        }
        text!(&x.text);
    }
    for x in &all.retrieval {
        current_basis(x.basis)?;
        member(x.observer)?;
        content(&x.policy)?;
        time_range(x.from, x.through)?;
        require(
            x.maximum_items > 0
                && x.maximum_bytes > 0
                && x.maximum_tokens > 0
                && x.access_generation > 0,
            CheckpointError::Capacity,
        )?;
        require(
            x.source_digest == pins.content.content_digest,
            CheckpointError::ContentMismatch,
        )?;
        count!(&x.topics);
        count!(&x.entities);
        for value in &x.topics {
            content(value)?;
        }
        for id in &x.entities {
            entity(*id)?;
        }
    }
    for x in &all.retrieved {
        let request = all
            .retrieval
            .iter()
            .find(|request| request.id == x.request)
            .ok_or(CheckpointError::InvalidReference)?;
        require(
            x.source_revision == request.basis.revision,
            CheckpointError::StaleBasis,
        )?;
        require(
            x.access_generation == request.access_generation
                && x.index_generation == request.index_generation,
            CheckpointError::InvalidReference,
        )?;
        let items = x
            .episodes
            .len()
            .checked_add(x.facts.len())
            .and_then(|count| count.checked_add(x.attributed_claims.len()))
            .and_then(|count| count.checked_add(x.snippets.len()))
            .ok_or(CheckpointError::Capacity)?;
        require(
            u64::try_from(items).map_err(|_| CheckpointError::Capacity)?
                <= u64::from(request.maximum_items),
            CheckpointError::Capacity,
        )?;
        let bytes = x.snippets.iter().try_fold(0u64, |sum, text| {
            let length = u64::try_from(text.len()).map_err(|_| CheckpointError::Capacity)?;
            sum.checked_add(length).ok_or(CheckpointError::Capacity)
        })?;
        require(bytes <= request.maximum_bytes, CheckpointError::Capacity)?;
        count!(&x.episodes);
        count!(&x.facts);
        count!(&x.attributed_claims);
        count!(&x.snippets);
        for id in &x.episodes {
            episode(*id)?;
        }
        for id in &x.facts {
            fact(*id)?;
        }
        for id in &x.attributed_claims {
            claim(*id)?;
        }
        for value in &x.snippets {
            text!(value);
        }
    }
    for x in &all.consolidation {
        current_basis(x.basis)?;
        require(
            x.source_revision <= basis.revision,
            CheckpointError::StaleBasis,
        )?;
        require(
            x.source_digest == pins.content.content_digest,
            CheckpointError::ContentMismatch,
        )?;
        count!(&x.episodes);
        count!(&x.summaries);
        for id in &x.episodes {
            episode(*id)?;
        }
        for id in &x.summaries {
            require(summaries.contains(id), CheckpointError::InvalidReference)?;
        }
    }
    unique(all.npcs.iter().map(|x| x.entity))?;
    for x in &all.npcs {
        entity(x.entity)?;
        content(&x.role)?;
        content(&x.personality)?;
        count!(&x.motivations);
        count!(&x.goals);
        count!(&x.needs);
        count!(&x.fears);
        count!(&x.known_facts);
        count!(&x.beliefs);
        count!(&x.secrets);
        for (index, value) in x.motivations.iter().enumerate() {
            content(value)?;
            require(
                !x.motivations[..index].contains(value),
                CheckpointError::DuplicateIdentity,
            )?;
        }
        for (index, value) in x.goals.iter().enumerate() {
            content(value)?;
            require(
                !x.goals[..index].contains(value),
                CheckpointError::DuplicateIdentity,
            )?;
        }
        for (index, value) in x.needs.iter().enumerate() {
            content(value)?;
            require(
                !x.needs[..index].contains(value),
                CheckpointError::DuplicateIdentity,
            )?;
        }
        for (index, value) in x.fears.iter().enumerate() {
            content(value)?;
            require(
                !x.fears[..index].contains(value),
                CheckpointError::DuplicateIdentity,
            )?;
        }
        for id in &x.known_facts {
            fact(*id)?;
        }
        for id in &x.beliefs {
            claim(*id)?;
        }
        for secret in &x.secrets {
            entity(secret.holder)?;
            content(&secret.policy)?;
            audience(&secret.permitted_audience, &mut progress.records)?;
            count!(&secret.claims);
            for id in &secret.claims {
                claim(*id)?;
            }
        }
    }
    for x in &all.hooks {
        member(x.member)?;
        content(&x.definition)?;
        audience(&x.audience, &mut progress.records)?;
        require(x.consent_generation > 0, CheckpointError::InvalidReference)?;
        count!(&x.source_facts);
        for id in &x.source_facts {
            fact(*id)?;
        }
    }
    for x in &all.arcs {
        content(&x.definition)?;
        content(&x.phase)?;
        count!(&x.source_facts);
        count!(&x.active_hooks);
        for id in &x.source_facts {
            fact(*id)?;
        }
        for id in &x.active_hooks {
            require(hooks.contains(id), CheckpointError::InvalidReference)?;
        }
    }
    if let Some(x) = &all.remote {
        content(&x.definition)?;
        require(
            x.maximum_participants > 0
                && x.maximum_connections > 0
                && state.members.len() <= x.maximum_participants as usize,
            CheckpointError::Capacity,
        )?;
    }
    let bindings = unique(all.presence.iter().flat_map(|x| x.bindings.iter().copied()))?;
    for x in &all.presence {
        member(x.member)?;
        content(&x.policy)?;
        count!(&x.bindings);
    }
    if let Some(x) = &all.audio {
        content(&x.policy)?;
        count!(&x.outputs);
        count!(&x.captures);
        unique(x.outputs.iter().map(|x| x.id))?;
        unique(x.captures.iter().map(|x| x.id))?;
        for output in &x.outputs {
            require(output.generation > 0, CheckpointError::InvalidIntent)?;
            audience(&output.audience, &mut progress.records)?;
            match output.destination {
                AudioDestination::PublicRoom(id) => {
                    require(
                        bindings.contains(&id) && output.audience == AudienceScope::Shared,
                        CheckpointError::InvalidReference,
                    )?;
                }
                AudioDestination::PrivateListener {
                    member: id,
                    binding,
                } => {
                    member(id)?;
                    require(
                        all.presence.iter().any(|presence| {
                            presence.member == id && presence.bindings.contains(&binding)
                        }) && output.device_unlocked
                            && matches!(&output.audience, AudienceScope::Members(ids) if ids.contains(&id)
                            && ids.len() == 1),
                        CheckpointError::InvalidReference,
                    )?;
                }
            }
        }
        for capture in &x.captures {
            member(capture.member)?;
            require(
                capture.generation > 0
                    && all.presence.iter().any(|presence| {
                        presence.member == capture.member
                            && presence.bindings.contains(&capture.binding)
                    }),
                CheckpointError::InvalidIntent,
            )?;
            if let Some(id) = capture.permitted_offer {
                require(
                    state.pending.iter().any(|pending| pending.window.id == id),
                    CheckpointError::InvalidReference,
                )?;
            }
        }
    }
    for x in &all.private_offers {
        member(x.member)?;
        content(&x.policy)?;
        count!(&x.facts);
        for id in &x.facts {
            fact(*id)?;
        }
        require(
            x.access_generation > 0
                && state
                    .pending
                    .iter()
                    .any(|pending| pending.id == x.resolution && pending.window.id == x.window),
            CheckpointError::InvalidReference,
        )?;
    }
    for x in &all.knowledge_cues {
        audience(&x.audience, &mut progress.records)?;
        content(&x.definition)?;
        count!(&x.source_facts);
        for id in &x.source_facts {
            fact(*id)?;
        }
    }
    for x in &all.moments {
        entity(x.location)?;
        content(&x.semantic_focus)?;
        audience(&x.audience, &mut progress.records)?;
        count!(&x.characters);
        count!(&x.facts);
        count!(&x.attributed_claims);
        for id in &x.characters {
            entity(*id)?;
        }
        for id in &x.facts {
            fact(*id)?;
        }
        for id in &x.attributed_claims {
            claim(*id)?;
        }
    }
    let request_key = |key: &AssetRequestKey, records: &mut usize| -> Result<(), CheckpointError> {
        require(
            key.schema == CHECKPOINT_SCHEMA && moments.contains(&key.moment),
            CheckpointError::InvalidReference,
        )?;
        require(
            key.source == pins.content.content_digest,
            CheckpointError::ContentMismatch,
        )?;
        audience(&key.audience, records)?;
        for reference in &key.references {
            asset(reference)?;
        }
        Ok(())
    };
    for x in &all.demands {
        current_basis(x.basis)?;
        request_key(&x.key, &mut progress.records)?;
        content(&x.policy)?;
        valid_time(x.expires)?;
        // Retained demand mode is its admission policy; the default applies to new jobs.
        require(x.maximum_bytes > 0, CheckpointError::Capacity)?;
        count!(&x.key.references);
    }
    for x in &all.asset_jobs {
        require(
            demands.contains(&x.demand) && x.generation > 0,
            CheckpointError::InvalidIntent,
        )?;
        if let Some(value) = &x.published {
            asset(value)?;
        }
        require(
            !matches!(x.state, AssetLifecycle::Ready) || x.published.is_some(),
            CheckpointError::InvalidIntent,
        )?;
    }
    for x in &all.asset_dependencies {
        asset(&x.asset)?;
        count!(&x.prerequisites);
        for value in &x.prerequisites {
            asset(value)?;
        }
    }
    // Dependencies are immutable references; any cycle is rejected before canonical admission.
    for start in &all.asset_dependencies {
        let mut pending = vec![&start.asset];
        let mut visited = Vec::new();
        while let Some(current) = pending.pop() {
            if visited.contains(&current) {
                continue;
            }
            visited.push(current);
            if let Some(record) = all.asset_dependencies.iter().find(|x| &x.asset == current) {
                for next in &record.prerequisites {
                    require(next != &start.asset, CheckpointError::InvalidReference)?;
                    pending.push(next);
                }
            }
        }
    }
    for x in &all.canonical_packs {
        content(&x.bible.definition)?;
        count!(&x.bible.palette);
        count!(&x.bible.references);
        count!(&x.identities);
        text!(&x.bible.style);
        for value in &x.bible.references {
            asset(value)?;
        }
        for identity in &x.identities {
            entity(identity.entity)?;
            count!(&identity.source_facts);
            count!(&identity.appearances);
            count!(&identity.sound);
            if let Some(appearance) = &identity.character_appearance {
                require(
                    !appearance.features.trim().is_empty() && !appearance.outfit.trim().is_empty(),
                    CheckpointError::InvalidReference,
                )?;
                text!(&appearance.features);
                text!(&appearance.outfit);
            }
            for id in &identity.source_facts {
                fact(*id)?;
            }
            for value in identity.appearances.iter().chain(&identity.sound) {
                asset(value)?;
            }
            if let Some(value) = &identity.voice {
                asset(value)?;
            }
        }
    }
    let shot = |x: &ShotPlan, records: &mut usize| -> Result<(), CheckpointError> {
        require(
            moments.contains(&x.moment) && x.duration_ticks > 0,
            CheckpointError::InvalidReference,
        )?;
        audience(&x.audience, records)?;
        content(&x.definition)?;
        content(&x.performance.definition)?;
        for id in &x.subjects {
            entity(*id)?;
        }
        for value in &x.references {
            asset(value)?;
        }
        for id in &x.performance.emphasis_facts {
            fact(*id)?;
        }
        if let Some(value) = &x.performance.voice {
            asset(value)?;
        }
        Ok(())
    };
    for x in &all.shots {
        shot(x, &mut progress.records)?;
        count!(&x.subjects);
        count!(&x.references);
        count!(&x.performance.emphasis_facts);
    }
    if let Some(x) = &all.prefetch {
        content(&x.definition)?;
        require(
            x.maximum_candidates > 0
                && x.maximum_branches > 0
                && x.maximum_bytes > 0
                && x.maximum_duration_ticks > 0,
            CheckpointError::Capacity,
        )?;
    }
    for x in &all.scenes {
        entity(x.scene)?;
        asset(&x.geometry)?;
        count!(&x.source_facts);
        require(
            all.canonical_packs
                .iter()
                .any(|pack| pack.revision == x.canonical_pack),
            CheckpointError::InvalidReference,
        )?;
        for id in &x.source_facts {
            fact(*id)?;
        }
    }
    for x in &all.item_origins {
        entity(x.item)?;
        fact(x.source_fact)?;
        content(&x.definition)?;
    }
    let selection = |x: &FactSelection, records: &mut usize| -> Result<(), CheckpointError> {
        audience(&x.audience, records)?;
        for id in &x.facts {
            fact(*id)?;
        }
        for id in &x.attributed_claims {
            claim(*id)?;
        }
        Ok(())
    };
    for x in &all.bookends {
        current_basis(x.spec.basis)?;
        content(&x.spec.policy)?;
        audience(&x.spec.audience, &mut progress.records)?;
        valid_time(x.spec.expires)?;
        selection(&x.selection, &mut progress.records)?;
        asset(&x.fallback)?;
        require(
            x.spec.from <= x.spec.through
                && x.spec.through <= basis.revision
                && x.spec.maximum_shots > 0
                && x.spec.maximum_duration_ticks > 0
                && x.spec.maximum_text_bytes > 0
                && x.spec.maximum_asset_bytes > 0
                && x.spec.audience == x.selection.audience,
            CheckpointError::InvalidReference,
        )?;
        require(
            x.shots.len() <= x.spec.maximum_shots as usize,
            CheckpointError::Capacity,
        )?;
        count!(&x.selection.facts);
        count!(&x.selection.attributed_claims);
        count!(&x.shots);
        count!(&x.captions);
        for value in &x.shots {
            shot(value, &mut progress.records)?;
            require(
                value.audience == x.spec.audience,
                CheckpointError::InvalidReference,
            )?;
            count!(&value.subjects);
            count!(&value.references);
            count!(&value.performance.emphasis_facts);
        }
        for value in &x.captions {
            text!(value);
        }
    }
    for x in &all.exports {
        require(
            bookends.contains(&x.bookend)
                && x.access_generation > 0
                && x.source_revision <= basis.revision
                && !x.downloaded_copy_retraction_supported,
            CheckpointError::InvalidReference,
        )?;
        selection(&x.selection, &mut progress.records)?;
        valid_time(x.expires)?;
        count!(&x.recipients);
        count!(&x.assets);
        count!(&x.rights);
        count!(&x.selection.facts);
        count!(&x.selection.attributed_claims);
        require(
            !x.recipients.is_empty() && !x.rights.is_empty(),
            CheckpointError::InvalidReference,
        )?;
        for id in &x.recipients {
            member(*id)?;
        }
        for value in &x.assets {
            asset(value)?;
        }
    }
    for x in &all.critical_cues {
        fact(x.fact)?;
        audience(&x.audience, &mut progress.records)?;
        content(&x.policy)?;
        count!(&x.ready_assets);
        require(x.maximum_duration_ticks > 0, CheckpointError::Capacity)?;
        for value in &x.ready_assets {
            asset(value)?;
        }
    }
    for x in &all.content_candidates {
        current_basis(x.basis)?;
        content(&x.policy)?;
        content(&x.template)?;
        rule(&x.source)?;
        fact(x.origin)?;
        audience(&x.audience, &mut progress.records)?;
        count!(&x.bounded_parameters);
    }
    for x in &all.content_admissions {
        require(
            candidates.contains(&x.candidate),
            CheckpointError::InvalidReference,
        )?;
        rule(&x.source)?;
        count!(&x.disclosed_to);
        for id in &x.disclosed_to {
            member(*id)?;
        }
        if let Some(id) = x.approval {
            member(id)?;
        }
        if x.result == ContentValidation::StandardCompatible {
            content(&x.definition)?;
            require(
                x.handler == pins.rules.handler && x.ruleset == pins.rules.ruleset,
                CheckpointError::RulesMismatch,
            )?;
        }
    }
    count!(&all.recovery.retired_epochs);
    count!(&all.recovery.lost_ranges);
    count!(&all.recovery.redacted_records);
    count!(&all.recovery.unavailable_sources);
    unique(all.recovery.retired_epochs.iter().copied())?;
    for epoch in &all.recovery.retired_epochs {
        require(*epoch < basis.revision.epoch(), CheckpointError::StaleBasis)?;
    }
    for range in &all.recovery.lost_ranges {
        require(
            range.from <= range.through && range.through < basis.revision,
            CheckpointError::StaleBasis,
        )?;
    }
    if let Some(origin) = &all.recovery.origin {
        require(
            origin.process_generation > 0,
            CheckpointError::InvalidIntent,
        )?;
        match origin.kind {
            RestoreKind::DebugFork => {
                require(origin.origin.run != basis.run, CheckpointError::WrongRun)?
            }
            RestoreKind::Disaster => require(
                origin.origin.revision.epoch() < basis.revision.epoch()
                    && all
                        .recovery
                        .retired_epochs
                        .contains(&origin.origin.revision.epoch()),
                CheckpointError::StaleBasis,
            )?,
        }
    }
    Ok(())
}

impl RetainedHeap for LocaleTag {
    fn retained_heap(&self) -> Option<usize> {
        Some(self.retained_heap_bytes())
    }
}
no_heap!(
    ClientBindingId,
    RecoveryEpoch,
    CreationPhase,
    SimulationTier,
    RetrievalPurpose,
    PresenceKind,
    AudioDestination,
    AssetKind,
    DemandPriority,
    AssetLifecycle,
    BookendKind,
    ContentValidation,
    RestoreKind,
    RestoreOrigin,
    LostGameRange
);
record_heap!(CharacterDraft; ancestry, background, classes, choices);
record_heap!(EntitySimulation; policy);
record_heap!(CatchUpCursor; pending_events, policy);
record_heap!(EnvironmentalState; definition, change_facts);
record_heap!(TravelLeg; travelers, route, source);
record_heap!(WitnessRecord; source);
record_heap!(RumorTransmission; evidence, policy, audience);
record_heap!(JournalEntry; source_facts, attributed_claims, audience, text);
record_heap!(MemorySummary; episodes, derived_claims, summarizer, model, policy, audience, text);
record_heap!(RetrievalRequest; topics, entities, policy);
record_heap!(RetrievedMemory; episodes, facts, attributed_claims, snippets);
record_heap!(ConsolidationCandidate; episodes, summaries);
record_heap!(SecretPolicy; claims, policy, permitted_audience);
record_heap!(NpcState; role, personality, motivations, goals, needs, fears, known_facts, beliefs,
    secrets);
record_heap!(CharacterHook; definition, source_facts, audience);
record_heap!(StoryArc; definition, phase, source_facts, active_hooks);
record_heap!(RemotePlayPolicy; definition, device_matrix);
record_heap!(ParticipantPresence; bindings, policy);
record_heap!(AudioOutputLease; audience);
record_heap!(CaptureLease; format);
record_heap!(AudioTopology; policy, outputs, captures);
record_heap!(ContextualPrivateOffer; facts, policy);
record_heap!(KnowledgeCue; source_facts, audience, definition);
record_heap!(AssetReference; key);
record_heap!(AssetRequestKey; identity, style, voice, provider, model, format, references, audience, parameters);
record_heap!(NarrativeMoment; characters, facts, attributed_claims, audience, semantic_focus, identity_revision);
record_heap!(AssetDemand; key, budget_reservation, policy);
record_heap!(AssetJobState; published);
record_heap!(AssetDependency; asset, prerequisites);
record_heap!(CharacterAppearance; features, outfit, outfit_revision);
record_heap!(EntityIdentityRevision; revision, character_appearance, source_facts, appearances, voice, sound);
record_heap!(VisualBible; revision, definition, palette, style, references);
record_heap!(CanonicalPack; revision, bible, identities);
record_heap!(PerformanceHint; definition, voice, emphasis_facts);
record_heap!(ShotPlan; subjects, audience, definition, references, performance);
record_heap!(PrefetchPolicy; definition);
record_heap!(SceneIdentityRevision; revision, source_facts, geometry, canonical_pack);
record_heap!(ItemOrigin; definition, identity_revision);
record_heap!(BookendSpec; audience, locale, policy, budget_reservation);
record_heap!(FactSelection; facts, attributed_claims, audience);
record_heap!(BookendPlan; spec, selection, shots, captions, fallback);
record_heap!(ExportGrant; recipients, selection, assets, rights);
record_heap!(CriticalCueEligibility; audience, ready_assets, policy);
record_heap!(ContentCandidate; generator, model, policy, template, source, audience, bounded_parameters);
record_heap!(ContentAdmission; definition, source, handler, ruleset, disclosed_to);
record_heap!(RecoveryState; retired_epochs, lost_ranges, redacted_records, unavailable_sources);
record_heap!(ContinuityState; creation, simulation, catch_up, environment, travel, witnesses, rumors,
    journal, summaries, retrieval, retrieved, consolidation, npcs, hooks, arcs, remote, presence,
    audio, private_offers, knowledge_cues, moments, demands, asset_jobs, asset_dependencies,
    canonical_packs, shots, prefetch, scenes, item_origins, bookends, exports, critical_cues,
    content_candidates, content_admissions, recovery);

/// Source-labelled commands contain input data only; the session authenticates the sender,
/// and rules validate mechanics before engine candidates can be committed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GameCommand {
    SelectChoice {
        resolution: ResolutionId,
        window: WindowId,
        offer: RevisionLabel,
        option: RevisionLabel,
    },
    /// Requests the pending server-owned draw; client data cannot choose its outcome.
    /// ```compile_fail
    /// use df_model::checkpoint::{GameCommand, ResolutionId, WindowId};
    /// let request = GameCommand::SubmitRoll {
    ///     resolution: ResolutionId::from_bytes(&[1; 16]).unwrap(),
    ///     window: WindowId::from_bytes(&[2; 16]).unwrap(),
    ///     values: vec![20],
    /// };
    /// ```
    SubmitRoll {
        resolution: ResolutionId,
        window: WindowId,
    },
    SelectReaction {
        resolution: ResolutionId,
        window: WindowId,
        offer: RevisionLabel,
        option: RevisionLabel,
    },
    ProposeAction {
        actor: EntityId,
        action: ContentReference,
        targets: Vec<EntityId>,
        choices: Vec<(RevisionLabel, RevisionLabel)>,
    },
    SubmitCharacterDraft {
        draft: CharacterDraft,
    },
    Speak {
        speaker: EntityId,
        text: String,
        conversation: Option<RecordId>,
    },
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HostCommand {
    ResolveRuling {
        resolution: ResolutionId,
        window: WindowId,
        offer: RevisionLabel,
        option: RevisionLabel,
    },
    SetPause {
        paused: bool,
        policy: ContentReference,
    },
    RequestCheckpoint,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandInput {
    pub basis: Basis,
    pub observed_revision: SessionRevision,
    pub operation: OperationId,
    pub member: MemberId,
    pub command: GameCommand,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostInput {
    pub basis: Basis,
    pub operation: OperationId,
    pub host: MemberId,
    pub command: HostCommand,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeFailure {
    Unavailable,
    Deadline,
    Cancelled,
    Capacity,
    Stale,
    Rejected,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum JobOutcome {
    Ai {
        semantic_output: String,
        policy: ContentReference,
        model: RevisionLabel,
    },
    Media {
        asset: AssetReference,
        demand: RecordId,
    },
    MemoryCandidates {
        records: Vec<RetrievedMemory>,
    },
    JobCancelled {
        target: JobId,
    },
    TimerCancelled {
        target: TimerId,
    },
    Failed(NativeFailure),
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JobCompletion {
    pub basis: Basis,
    pub operation: OperationId,
    pub job: JobId,
    pub generation: u64,
    pub outcome: JobOutcome,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimerExpiry {
    pub basis: Basis,
    pub timer: TimerId,
    pub generation: u64,
    pub observed_time: LogicalTime,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PresentationObservation {
    Started,
    Finished,
    Skipped,
    Failed,
    Expired,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PresentationReport {
    pub basis: Basis,
    pub binding: ClientBindingId,
    pub cue: RecordId,
    pub observation: PresentationObservation,
    pub presentation_ticks: u64,
}
/// Native completion/timer variants are never accepted by the client DTO mapper.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GameInput {
    Game(CommandInput),
    Host(HostInput),
    Job(JobCompletion),
    Timer(TimerExpiry),
    Presentation(PresentationReport),
}
impl GameInput {
    /// Counts retained capacities for owner queue admission; not sender-reported bytes.
    pub fn retained_bytes(&self) -> Option<usize> {
        size_of::<Self>().checked_add(self.retained_heap()?)
    }
    pub fn retained_heap_bytes(&self) -> Option<usize> {
        self.retained_heap()
    }
}
no_heap!(
    NativeFailure,
    TimerExpiry,
    PresentationObservation,
    PresentationReport
);
record_heap!(CommandInput; command);
record_heap!(HostInput; command);
record_heap!(JobCompletion; outcome);
impl RetainedHeap for GameCommand {
    fn retained_heap(&self) -> Option<usize> {
        match self {
            Self::SelectChoice { offer, option, .. }
            | Self::SelectReaction { offer, option, .. } => {
                offer.retained_heap()?.checked_add(option.retained_heap()?)
            }
            Self::SubmitRoll { .. } => Some(0),
            Self::ProposeAction {
                action,
                targets,
                choices,
                ..
            } => action
                .retained_heap()?
                .checked_add(targets.retained_heap()?)?
                .checked_add(choices.retained_heap()?),
            Self::SubmitCharacterDraft { draft } => draft.retained_heap(),
            Self::Speak { text, .. } => text.retained_heap(),
        }
    }
}
impl RetainedHeap for HostCommand {
    fn retained_heap(&self) -> Option<usize> {
        match self {
            Self::ResolveRuling { offer, option, .. } => {
                offer.retained_heap()?.checked_add(option.retained_heap()?)
            }
            Self::SetPause { policy, .. } => policy.retained_heap(),
            Self::RequestCheckpoint => Some(0),
        }
    }
}
impl RetainedHeap for JobOutcome {
    fn retained_heap(&self) -> Option<usize> {
        match self {
            Self::Ai {
                semantic_output,
                policy,
                model,
            } => semantic_output
                .retained_heap()?
                .checked_add(policy.retained_heap()?)?
                .checked_add(model.retained_heap()?),
            Self::Media { asset, .. } => asset.retained_heap(),
            Self::MemoryCandidates { records } => records.retained_heap(),
            Self::JobCancelled { .. } | Self::TimerCancelled { .. } | Self::Failed(_) => Some(0),
        }
    }
}
impl RetainedHeap for GameInput {
    fn retained_heap(&self) -> Option<usize> {
        match self {
            Self::Game(value) => value.retained_heap(),
            Self::Host(value) => value.retained_heap(),
            Self::Job(value) => value.retained_heap(),
            Self::Timer(value) => value.retained_heap(),
            Self::Presentation(value) => value.retained_heap(),
        }
    }
}
