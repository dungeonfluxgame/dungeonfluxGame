//! Isolated policy staging over canonical checkpoints; no durable commit or rules execution.
use df_model::checkpoint::{
    Basis, Checkpoint, CheckpointError, CheckpointLimits, CheckpointPins, ContentReference, FactId,
    GameState, PendingResolution, ReferenceInventory,
};
use df_narrative::{
    BeatSelectionError, BeatSelectionLimits, CheckpointBeatRequest, CheckpointProgressError,
    ProgressLimits, ThreadCheckpointRequest, ThreadConsequenceSelection, ThreadProgressEvidence,
    stage_checkpoint_beat_selection, stage_checkpoint_thread_progress,
};
use df_types::RevisionLabel;
use df_world::{
    AdmittedEnvironmentalChange, AdmittedScheduleDestination, DueSelection, DueSelectionError,
    DueSelectionLimits, DueSelectionRequest, EnvironmentalDeltaError, EnvironmentalDeltaLimits,
    ScheduleAdvancementError, ScheduleAdvancementLimits, select_due_events,
    stage_environmental_delta, stage_schedule_advancement,
};

/// Explicit caller-selected work and allocation bounds. These are not calibrated defaults.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DirectorLimits {
    pub maximum_checkpoint_bytes: usize,
    pub maximum_pass_bytes: usize,
    pub maximum_relationships: usize,
    pub world: DueSelectionLimits,
}

/// Already source-qualified policy snapshots supplied by the owning pure directors.
///
/// Interaction is based on the same immutable checkpoint as world selection. Narrative is
/// based on the selected interaction snapshot, after any separately owned knowledge stage.
/// None means an explicitly inapplicable/no-intervention stage. Structural checkpoint validation
/// does not prove NPC motivation, authored beat prerequisites or rules legality; the adapters
/// must qualify those policies before supplying these snapshots.
pub struct DirectorCandidates {
    pub interaction: Option<Checkpoint>,
    pub narrative: Option<Checkpoint>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DirectorStage {
    Interaction,
    Narrative,
}

#[derive(Debug, Eq, PartialEq)]
pub enum DirectorError {
    InvalidLimits,
    Capacity,
    Binding(CheckpointError),
    ProposalBinding {
        stage: DirectorStage,
        error: CheckpointError,
    },
    AuthorityConflict(DirectorStage),
    DuplicateRelationship(DirectorStage),
    World(DueSelectionError),
    Schedule(ScheduleAdvancementError),
    Environment(EnvironmentalDeltaError),
    Narrative(CheckpointProgressError),
    BeatSelection(BeatSelectionError),
    InvalidCandidate(CheckpointError),
    WorldStateConflict,
}

/// Owned detached policy state, bound to the original canonical checkpoint.
///
/// This is not a model Transition or a durable decision. Facts, draws, resources and intents
/// remain unchanged. The schedule entrypoint can stage admitted location/time/cursor changes;
/// the policy-only entrypoint leaves those records unchanged. Both retain `world()` as proposal data. The engine's complete transition adapter must qualify source outcomes and construct
/// their ordered decision/fact records; the session then rechecks its owner fence/revision and
/// commits atomically. No apply, publish or effect-executor method is exposed here.
#[derive(Debug, Eq, PartialEq)]
pub struct StagedDirectors<'a> {
    world: DueSelection<'a>,
    candidate: Checkpoint,
    narrative_policy: Option<RevisionLabel>,
    narrative_evidence: Vec<ThreadProgressEvidence>,
    beat_selection: Option<ContentReference>,
    beat_causes: Vec<FactId>,
}

impl StagedDirectors<'_> {
    pub fn basis(&self) -> Basis {
        self.candidate.basis()
    }

    pub fn pins(&self) -> &CheckpointPins {
        self.candidate.pins()
    }

    pub fn world(&self) -> &DueSelection<'_> {
        &self.world
    }

    /// Private server-side policy data, never an audience projection or provider prompt.
    pub fn candidate(&self) -> &Checkpoint {
        &self.candidate
    }

    /// Transfers the detached candidate to its owning transition producer.
    /// Policy/evidence must be qualified in that producer's durable decision contract.
    pub fn into_candidate(self) -> Checkpoint {
        self.candidate
    }

    /// Retained admitted narrative policy for owner revalidation and replay qualification.
    pub fn narrative_policy(&self) -> Option<&RevisionLabel> {
        self.narrative_policy.as_ref()
    }

    /// Private causal records; the audience owner must authorize before projection.
    pub fn narrative_evidence(&self) -> &[ThreadProgressEvidence] {
        &self.narrative_evidence
    }

    /// Exact admitted route; private proposal evidence for owner revalidation.
    pub fn beat_selection(&self) -> Option<&ContentReference> {
        self.beat_selection.as_ref()
    }

    /// Ordered source facts that justified this route; this is not a durable receipt.
    pub fn beat_causes(&self) -> &[FactId] {
        &self.beat_causes
    }
}

/// Unresolved due-event consequences stop dependent policy composition. The selected prefix
/// remains owned data for the world/rules authority, with explicit backlog and no applied cursor.
#[derive(Debug, Eq, PartialEq)]
pub enum DirectorStaging<'a> {
    RulesPending(&'a [PendingResolution]),
    WorldPending(Box<DueSelection<'a>>),
    MissingEnvironmentalProducer,
    Staged(Box<StagedDirectors<'a>>),
}

/// Selects bounded world work and atomically stages only compatible policy snapshots.
///
/// World and interaction share the immutable admission basis; neither consumes the other's
/// tentative state. Narrative may consume the selected interaction state but cannot overwrite it.
/// Any unresolved rules continuation or world event stops before policy staging. This bounded
/// entrypoint has no operation-specific continuation selector: it conservatively preserves all
/// canonical pending windows for the outer rules/session owner instead of skipping one.
///
/// Binding/refusal/capacity errors return no partial policy state and never mutate `current`.
/// Input order cannot resolve duplicate directional relationships. All owned checkpoint capacities
/// are admitted before traversal or cloning. The pass budget conservatively includes input
/// checkpoints, three bounded scratch checkpoints and maximum world output; allocator metadata
/// and native elapsed deadlines remain the caller's responsibility.
pub fn compose_director_candidates<'a>(
    current: &'a Checkpoint,
    admitted_pins: &CheckpointPins,
    world_request: DueSelectionRequest<'_>,
    candidates: DirectorCandidates,
    limits: DirectorLimits,
) -> Result<DirectorStaging<'a>, DirectorError> {
    if limits.maximum_checkpoint_bytes == 0
        || limits.maximum_pass_bytes == 0
        || limits.maximum_relationships == 0
    {
        return Err(DirectorError::InvalidLimits);
    }
    let mut input_bytes = checkpoint_bytes(current, limits)?;
    for candidate in [&candidates.interaction, &candidates.narrative]
        .into_iter()
        .flatten()
    {
        input_bytes = input_bytes
            .checked_add(checkpoint_bytes(candidate, limits)?)
            .ok_or(DirectorError::Capacity)?;
    }
    let pass_bytes = limits
        .maximum_checkpoint_bytes
        .checked_mul(3)
        .and_then(|scratch| input_bytes.checked_add(scratch))
        .and_then(|bytes| bytes.checked_add(limits.world.output_bytes))
        .ok_or(DirectorError::Capacity)?;
    if pass_bytes > limits.maximum_pass_bytes {
        return Err(DirectorError::Capacity);
    }
    current
        .validate_resume(world_request.expected_basis, admitted_pins)
        .map_err(DirectorError::Binding)?;
    if !current.state().pending.is_empty() {
        return Ok(DirectorStaging::RulesPending(&current.state().pending));
    }
    unique_relationships(current.state(), limits, DirectorStage::Interaction)?;
    let world = select_due_events(current, admitted_pins, world_request, limits.world)
        .map_err(DirectorError::World)?;
    if !world.events.is_empty() || world.remaining_due() != 0 {
        return Ok(DirectorStaging::WorldPending(Box::new(world)));
    }
    let mut selected = current.clone();
    if let Some(interaction) = candidates.interaction {
        validate_proposal(&interaction, &selected, DirectorStage::Interaction, limits)?;
        selected = interaction;
    }
    if let Some(narrative) = candidates.narrative {
        validate_proposal(&narrative, &selected, DirectorStage::Narrative, limits)?;
        selected = narrative;
    }
    Ok(DirectorStaging::Staged(Box::new(StagedDirectors {
        world,
        candidate: selected,
        narrative_policy: None,
        narrative_evidence: vec![],
        beat_selection: None,
        beat_causes: vec![],
    })))
}

fn checkpoint_bytes(
    checkpoint: &Checkpoint,
    limits: DirectorLimits,
) -> Result<usize, DirectorError> {
    let bytes = checkpoint.retained_bytes().ok_or(DirectorError::Capacity)?;
    if bytes > limits.maximum_checkpoint_bytes {
        return Err(DirectorError::Capacity);
    }
    Ok(bytes)
}

fn unique_relationships(
    state: &GameState,
    limits: DirectorLimits,
    stage: DirectorStage,
) -> Result<(), DirectorError> {
    if state.relationships.len() > limits.maximum_relationships {
        return Err(DirectorError::Capacity);
    }
    for (index, relationship) in state.relationships.iter().enumerate() {
        if state.relationships.iter().take(index).any(|earlier| {
            earlier.subject == relationship.subject && earlier.object == relationship.object
        }) {
            return Err(DirectorError::DuplicateRelationship(stage));
        }
    }
    Ok(())
}

fn validate_proposal(
    proposed: &Checkpoint,
    selected: &Checkpoint,
    stage: DirectorStage,
    limits: DirectorLimits,
) -> Result<(), DirectorError> {
    proposed
        .validate_resume(selected.basis(), selected.pins())
        .map_err(|error| DirectorError::ProposalBinding { stage, error })?;
    unique_relationships(proposed.state(), limits, stage)?;
    let mut protected = proposed.state().clone();
    match stage {
        DirectorStage::Interaction => {
            protected.relationships = selected.state().relationships.clone();
            protected.conversations = selected.state().conversations.clone();
            protected.obligations = selected.state().obligations.clone();
        }
        DirectorStage::Narrative => protected.narrative = selected.state().narrative.clone(),
    }
    if protected != *selected.state() {
        return Err(DirectorError::AuthorityConflict(stage));
    }
    Ok(())
}

/// Additional explicit limits for the real World schedule and canonical checkpoint adapters.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScheduleDirectorLimits {
    pub directors: DirectorLimits,
    pub schedule: ScheduleAdvancementLimits,
    pub checkpoint: CheckpointLimits,
}

/// Allocation bounds for schedule composition and the real narrative reducer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScheduleThreadProgressLimits {
    pub directors: ScheduleDirectorLimits,
    pub narrative: ProgressLimits,
}

/// Stages World/environment and interaction before evaluating admitted thread consequences.
///
/// All stages use the canonical Model checkpoint. Narrative consumes the selected sibling state
/// without changing facts, knowledge, draws, decisions, effects, rules outcomes or open alternatives
/// beyond explicitly admitted Resolve selections. Pending prerequisites preserve their typed
/// outcome. A narrative refusal discards the whole detached candidate, including tentative World
/// work. Retained evidence/policy remain private server data, requiring owner revalidation and an
/// atomic session commit before publication; this operation cannot commit or dispatch anything.
pub fn compose_schedule_thread_progress<'a>(
    current: &'a Checkpoint,
    admitted_pins: &CheckpointPins,
    schedule: ScheduleDirectorRequest<'_>,
    interaction: Option<Checkpoint>,
    narrative: ThreadCheckpointRequest<'_>,
    limits: ScheduleThreadProgressLimits,
) -> Result<DirectorStaging<'a>, DirectorError> {
    let composition_limits = ScheduleDirectorLimits {
        directors: thread_composition_limits(
            &narrative,
            limits.directors.directors,
            limits.narrative,
        )?,
        ..limits.directors
    };
    let inventory = ReferenceInventory {
        rules: narrative.inventory.rules,
        content: narrative.inventory.content,
        resources: narrative.inventory.resources,
        assets: narrative.inventory.assets,
    };
    let composed = compose_schedule_candidates(
        current,
        admitted_pins,
        schedule,
        DirectorCandidates {
            interaction,
            narrative: None,
        },
        inventory,
        composition_limits,
    )?;
    finish_thread_progress(composed, narrative, limits.narrative)
}

/// Evaluates an explicitly selected authored route after World/environment and interaction.
/// Pending prerequisites remain pending. A rejected route discards all detached sibling changes;
/// it never chooses another route, grants knowledge, resolves rules or commits a decision.
/// Exact policy, selection and ordered causes remain private for the transition/session owner.
pub fn compose_schedule_beat_selection<'a>(
    current: &'a Checkpoint,
    admitted_pins: &CheckpointPins,
    schedule: ScheduleDirectorRequest<'_>,
    interaction: Option<Checkpoint>,
    narrative: CheckpointBeatRequest<'_>,
    limits: ScheduleDirectorLimits,
    beat_limits: BeatSelectionLimits,
) -> Result<DirectorStaging<'a>, DirectorError> {
    if narrative.checkpoint_limits.maximum_retained_bytes
        > limits.directors.maximum_checkpoint_bytes
    {
        return Err(DirectorError::InvalidLimits);
    }
    if narrative.alternatives.len() > beat_limits.alternatives {
        return Err(DirectorError::Capacity);
    }
    // Composition reserves its checkpoint scratch; reserve the separately retained route receipt.
    let receipt_bytes = std::mem::size_of::<Option<RevisionLabel>>()
        .checked_add(narrative.policy.retained_heap_bytes())
        .and_then(|bytes| bytes.checked_add(std::mem::size_of::<Option<ContentReference>>()))
        .and_then(|bytes| bytes.checked_add(narrative.selection.package.retained_heap_bytes()))
        .and_then(|bytes| bytes.checked_add(narrative.selection.entry.retained_heap_bytes()))
        .and_then(|bytes| bytes.checked_add(std::mem::size_of::<Vec<FactId>>()))
        .and_then(|bytes| {
            beat_limits
                .records
                .checked_mul(std::mem::size_of::<FactId>())
                .and_then(|causes| bytes.checked_add(causes))
        })
        .ok_or(DirectorError::Capacity)?;
    let mut composition_limits = limits;
    composition_limits.directors.maximum_pass_bytes = composition_limits
        .directors
        .maximum_pass_bytes
        .checked_sub(receipt_bytes)
        .ok_or(DirectorError::Capacity)?;
    let inventory = ReferenceInventory {
        rules: narrative.inventory.rules,
        content: narrative.inventory.content,
        resources: narrative.inventory.resources,
        assets: narrative.inventory.assets,
    };
    let composed = compose_schedule_candidates(
        current,
        admitted_pins,
        schedule,
        DirectorCandidates {
            interaction,
            narrative: None,
        },
        inventory,
        composition_limits,
    )?;
    let DirectorStaging::Staged(mut staged) = composed else {
        return Ok(composed);
    };
    // Bound the additional evidence traversal as well as the selector's own traversal.
    let evidence_work = narrative
        .alternatives
        .len()
        .checked_add(beat_limits.records)
        .ok_or(DirectorError::BeatSelection(BeatSelectionError::Capacity))?;
    let work = beat_limits
        .work
        .checked_sub(evidence_work)
        .ok_or(DirectorError::BeatSelection(BeatSelectionError::Capacity))?;
    let policy = narrative.policy;
    let selection = narrative.selection;
    let alternatives = narrative.alternatives;
    let candidate = stage_checkpoint_beat_selection(
        &staged.candidate,
        narrative,
        BeatSelectionLimits {
            work,
            ..beat_limits
        },
    )
    .map_err(DirectorError::BeatSelection)?;
    let selected = alternatives
        .iter()
        .find(|alternative| alternative.selection == selection)
        .ok_or(DirectorError::BeatSelection(BeatSelectionError::Selection))?;
    staged.candidate = candidate;
    staged.narrative_policy = Some(policy.clone());
    staged.beat_selection = Some(selection.clone());
    staged.beat_causes = selected.causes.iter().map(|cause| cause.fact).collect();
    Ok(DirectorStaging::Staged(staged))
}

/// Bounds for the policy-only World/interaction and canonical narrative composition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ThreadDirectorLimits {
    pub directors: DirectorLimits,
    pub narrative: ProgressLimits,
}

/// Evaluates real admitted thread consequences after policy-only sibling selection.
/// Pending rules/World work stops Narrative. Unlike schedule composition, this entrypoint does
/// not stage time, movement or a catch-up cursor. It retains private policy/evidence and publishes
/// nothing; the same immutable-basis, source, bounded-work and atomic-refusal contract applies.
pub fn compose_director_thread_progress<'a>(
    current: &'a Checkpoint,
    admitted_pins: &CheckpointPins,
    world: DueSelectionRequest<'_>,
    interaction: Option<Checkpoint>,
    narrative: ThreadCheckpointRequest<'_>,
    limits: ThreadDirectorLimits,
) -> Result<DirectorStaging<'a>, DirectorError> {
    let bounds = thread_composition_limits(&narrative, limits.directors, limits.narrative)?;
    let composed = compose_director_candidates(
        current,
        admitted_pins,
        world,
        DirectorCandidates {
            interaction,
            narrative: None,
        },
        bounds,
    )?;
    finish_thread_progress(composed, narrative, limits.narrative)
}

fn thread_composition_limits(
    narrative: &ThreadCheckpointRequest<'_>,
    mut bounds: DirectorLimits,
    limits: ProgressLimits,
) -> Result<DirectorLimits, DirectorError> {
    if narrative.checkpoint_limits.maximum_retained_bytes > bounds.maximum_checkpoint_bytes {
        return Err(DirectorError::InvalidLimits);
    }
    if narrative.selections.len() > limits.consequences {
        return Err(DirectorError::Capacity);
    }
    // Reserve causal output and borrowed consequence scratch before cloning sibling state.
    // Composition already reserves three checkpoint-sized working copies for reducer staging.
    let mut narrative_bytes =
        std::mem::size_of::<RevisionLabel>() + narrative.policy.retained_heap_bytes();
    for selection in narrative.selections {
        narrative_bytes = narrative_bytes
            .checked_add(std::mem::size_of::<ThreadProgressEvidence>())
            .and_then(|bytes| {
                bytes.checked_add(std::mem::size_of::<ThreadConsequenceSelection<'_>>())
            })
            .and_then(|bytes| bytes.checked_add(selection.thread.package.retained_heap_bytes()))
            .and_then(|bytes| bytes.checked_add(selection.thread.entry.retained_heap_bytes()))
            .ok_or(DirectorError::Capacity)?;
    }
    bounds.maximum_pass_bytes = bounds
        .maximum_pass_bytes
        .checked_sub(narrative_bytes)
        .ok_or(DirectorError::Capacity)?;
    Ok(bounds)
}

fn finish_thread_progress<'a>(
    composed: DirectorStaging<'a>,
    narrative: ThreadCheckpointRequest<'_>,
    limits: ProgressLimits,
) -> Result<DirectorStaging<'a>, DirectorError> {
    let DirectorStaging::Staged(mut staged) = composed else {
        return Ok(composed);
    };
    let proposed = stage_checkpoint_thread_progress(&staged.candidate, narrative, limits)
        .map_err(DirectorError::Narrative)?;
    staged.candidate = proposed.checkpoint;
    staged.narrative_policy = Some(proposed.policy);
    staged.narrative_evidence = proposed.evidence;
    Ok(DirectorStaging::Staged(staged))
}

/// Explicit applicable world work supplied by the engine's source-qualified caller.
/// Required environmental work cannot disappear when its producer is absent: None returns
/// MissingEnvironmentalProducer. Admitted records still pass the actual World validator;
/// this request grants no source legality, geometry-byte verification or commit authority.
pub enum EnvironmentalRequest<'a> {
    NotApplicable,
    Required {
        admitted_changes: Option<&'a [AdmittedEnvironmentalChange<'a>]>,
        limits: EnvironmentalDeltaLimits,
    },
}

/// One bounded world composition input; all records refer to the unchanged original checkpoint.
pub struct ScheduleDirectorRequest<'a> {
    pub accepted_time: Option<DueSelectionRequest<'a>>,
    pub destinations: &'a [AdmittedScheduleDestination<'a>],
    pub environmental: EnvironmentalRequest<'a>,
}

/// Composes actual admitted schedule movement with sibling interaction and downstream narrative.
///
/// The World owner selects and validates exact current event/entity/destination mappings. The
/// interaction snapshot is checked against the original immutable basis, then its owned policy
/// fields are merged with the selected World records; neither sibling overwrites the other.
/// Narrative is checked against this merged snapshot. An unmapped selected event stops the pass
/// as WorldPending: the engine cannot advance its cursor past a missing consequence. A bounded
/// fully mapped prefix may carry a remaining catch-up backlog; it remains explicit in world().
/// Paused backlog remains WorldPending. Source-qualified time admission, destination legality,
/// narrative/interaction policies and operation retries remain the owning callers' responsibility.
///
/// State is reconstructed by the actual Model validator with trusted inventory and unchanged
/// schema/basis/pins. No fact, decision, draw, resource, effect or source outcome is synthesized.
/// The complete Transition/session adapter must atomically commit admitted world changes and
/// their actual ordered fact/decision records before exposing any of these staged records.
pub fn compose_schedule_candidates<'a>(
    current: &'a Checkpoint,
    admitted_pins: &CheckpointPins,
    request: ScheduleDirectorRequest<'_>,
    candidates: DirectorCandidates,
    inventory: ReferenceInventory<'_>,
    limits: ScheduleDirectorLimits,
) -> Result<DirectorStaging<'a>, DirectorError> {
    let ScheduleDirectorRequest {
        accepted_time,
        destinations,
        environmental,
    } = request;
    match environmental {
        EnvironmentalRequest::NotApplicable => compose_world_candidates(
            current,
            admitted_pins,
            WorldCompositionRequest {
                accepted_time,
                destinations,
                environment: EnvironmentalWork::NotApplicable,
            },
            candidates,
            inventory,
            limits,
        ),
        EnvironmentalRequest::Required {
            admitted_changes,
            limits: environmental_limits,
        } => compose_environmental_schedule_candidates(
            current,
            admitted_pins,
            EnvironmentalScheduleRequest {
                accepted_time,
                destinations,
                admitted_changes,
                environmental_limits,
            },
            candidates,
            inventory,
            limits,
        ),
    }
}

/// Private engine input over the World owner's exact existing source-admitted records.
/// No current GameInput variant admits an environmental replacement. A caller lacking that
/// source/legal producer supplies None and receives typed pending, never an empty successful batch.
struct EnvironmentalScheduleRequest<'a> {
    pub accepted_time: Option<DueSelectionRequest<'a>>,
    pub destinations: &'a [AdmittedScheduleDestination<'a>],
    pub admitted_changes: Option<&'a [AdmittedEnvironmentalChange<'a>]>,
    pub environmental_limits: EnvironmentalDeltaLimits,
}

/// Stages source-admitted environmental records through the actual World validator together
/// with due/schedule state in one isolated candidate. Geometry checks bind exact current model
/// references; actual asset bytes, spatial legality and environmental mechanics are not inferred.
/// Causes must already be current-revision accepted facts in the original checkpoint. The
/// schedule/time proposal does not create new environmental causes, locations or geometry.
/// Missing producer/rules/world work suspends; a last environmental/narrative/constructor failure
/// discards all uncommitted location/time/cursor/policy/environmental changes.
fn compose_environmental_schedule_candidates<'a>(
    current: &'a Checkpoint,
    admitted_pins: &CheckpointPins,
    request: EnvironmentalScheduleRequest<'_>,
    candidates: DirectorCandidates,
    inventory: ReferenceInventory<'_>,
    limits: ScheduleDirectorLimits,
) -> Result<DirectorStaging<'a>, DirectorError> {
    let environment = match request.admitted_changes {
        Some(changes) => EnvironmentalWork::Admitted {
            changes,
            limits: request.environmental_limits,
        },
        None => EnvironmentalWork::MissingProducer,
    };
    compose_world_candidates(
        current,
        admitted_pins,
        WorldCompositionRequest {
            accepted_time: request.accepted_time,
            destinations: request.destinations,
            environment,
        },
        candidates,
        inventory,
        limits,
    )
}

enum EnvironmentalWork<'a> {
    NotApplicable,
    MissingProducer,
    Admitted {
        changes: &'a [AdmittedEnvironmentalChange<'a>],
        limits: EnvironmentalDeltaLimits,
    },
}

struct WorldCompositionRequest<'a> {
    accepted_time: Option<DueSelectionRequest<'a>>,
    destinations: &'a [AdmittedScheduleDestination<'a>],
    environment: EnvironmentalWork<'a>,
}

fn compose_world_candidates<'a>(
    current: &'a Checkpoint,
    admitted_pins: &CheckpointPins,
    request: WorldCompositionRequest<'_>,
    candidates: DirectorCandidates,
    inventory: ReferenceInventory<'_>,
    limits: ScheduleDirectorLimits,
) -> Result<DirectorStaging<'a>, DirectorError> {
    let bounds = limits.directors;
    if bounds.maximum_checkpoint_bytes == 0
        || bounds.maximum_pass_bytes == 0
        || bounds.maximum_relationships == 0
        || limits.schedule.selection != bounds.world
    {
        return Err(DirectorError::InvalidLimits);
    }
    let mut bytes = checkpoint_bytes(current, bounds)?;
    for proposal in [&candidates.interaction, &candidates.narrative]
        .into_iter()
        .flatten()
    {
        bytes = bytes
            .checked_add(checkpoint_bytes(proposal, bounds)?)
            .ok_or(DirectorError::Capacity)?;
    }
    let bytes = bounds
        .maximum_checkpoint_bytes
        .checked_mul(3)
        .and_then(|scratch| bytes.checked_add(scratch))
        .and_then(|bytes| bytes.checked_add(limits.schedule.output_bytes))
        .and_then(|bytes| match &request.environment {
            EnvironmentalWork::Admitted { limits, .. } => bytes.checked_add(limits.output_bytes),
            EnvironmentalWork::NotApplicable | EnvironmentalWork::MissingProducer => Some(bytes),
        })
        .ok_or(DirectorError::Capacity)?;
    if bytes > bounds.maximum_pass_bytes {
        return Err(DirectorError::Capacity);
    }
    let expected = request
        .accepted_time
        .as_ref()
        .map(|request| request.expected_basis)
        .ok_or(DirectorError::Schedule(
            ScheduleAdvancementError::TimeNotAccepted,
        ))?;
    current
        .validate_resume(expected, admitted_pins)
        .map_err(DirectorError::Binding)?;
    if !current.state().pending.is_empty() {
        return Ok(DirectorStaging::RulesPending(&current.state().pending));
    }
    if matches!(request.environment, EnvironmentalWork::MissingProducer) {
        return Ok(DirectorStaging::MissingEnvironmentalProducer);
    }
    unique_relationships(current.state(), bounds, DirectorStage::Interaction)?;
    let advanced = stage_schedule_advancement(
        current,
        admitted_pins,
        request.accepted_time,
        request.destinations,
        limits.schedule,
    )
    .map_err(DirectorError::Schedule)?;
    if advanced.due.events.iter().any(|event| {
        !advanced
            .movements
            .iter()
            .any(|movement| movement.event == event.id)
    }) || (advanced.due.events.is_empty() && advanced.due.remaining_due() != 0)
    {
        return Ok(DirectorStaging::WorldPending(Box::new(advanced.due)));
    }
    let environmental = match request.environment {
        EnvironmentalWork::Admitted { changes, limits } => Some(
            stage_environmental_delta(current, admitted_pins, expected, Some(changes), limits)
                .map_err(DirectorError::Environment)?,
        ),
        EnvironmentalWork::NotApplicable => None,
        EnvironmentalWork::MissingProducer => {
            return Ok(DirectorStaging::MissingEnvironmentalProducer);
        }
    };
    let mut state = current.state().clone();
    if let Some(interaction) = candidates.interaction {
        validate_proposal(&interaction, current, DirectorStage::Interaction, bounds)?;
        state.relationships = interaction.state().relationships.clone();
        state.conversations = interaction.state().conversations.clone();
        state.obligations = interaction.state().obligations.clone();
    }
    for movement in &advanced.movements {
        let entity = state
            .entities
            .iter_mut()
            .find(|entity| entity.id == movement.entity)
            .ok_or(DirectorError::WorldStateConflict)?;
        if entity.location != movement.before {
            return Err(DirectorError::WorldStateConflict);
        }
        entity.location = Some(movement.after);
    }
    if let Some(environmental) = environmental {
        for replacement in environmental.replacements {
            let environment = state
                .continuity
                .environment
                .iter_mut()
                .find(|environment| environment.location == replacement.location)
                .ok_or(DirectorError::WorldStateConflict)?;
            *environment = replacement;
        }
    }
    state.logical_time = advanced.due.proposed_time;
    state.continuity.catch_up = Some(advanced.due.cursor.clone());
    let mut selected = Checkpoint::new(
        current.schema(),
        current.basis(),
        current.pins().clone(),
        state,
        inventory,
        limits.checkpoint,
    )
    .map_err(DirectorError::InvalidCandidate)?;
    checkpoint_bytes(&selected, bounds)?;
    if let Some(narrative) = candidates.narrative {
        validate_proposal(&narrative, &selected, DirectorStage::Narrative, bounds)?;
        selected = narrative;
    }
    Ok(DirectorStaging::Staged(Box::new(StagedDirectors {
        world: advanced.due,
        candidate: selected,
        narrative_policy: None,
        narrative_evidence: vec![],
        beat_selection: None,
        beat_causes: vec![],
    })))
}
