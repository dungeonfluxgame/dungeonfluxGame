//! Canonical optional reference-sheet staging after registered character acceptance.
//! Native creation rules, durable commits, current-rights publication and providers are separate owners.
use crate::command_entry::{CommandEntryContext, CommandRejection, decide_registered_command};
use df_model::checkpoint::{
    AssetDemand, AssetJobState, AssetKind, AssetLifecycle, AssetReference, AssetRequestKey,
    AudienceScope, Basis, CHECKPOINT_SCHEMA, CanonicalPack, CharacterAppearance, Checkpoint,
    CheckpointError, CheckpointLimits, CheckpointPins, ContentReference, CreationPhase,
    DemandPriority, DurableIntent, DurableStatus, EffectKind, EntityId, EntityIdentityRevision,
    FactId, GameCommand, GameInput, GameState, JobCompletion, JobId, JobOutcome, NarrativeMoment,
    NativeFailure, RecordId, ReferenceInventory, RuleReference, VisualBible,
};
use df_rules::{DispatchRegistry, RulesCommandHandler, RulesCommandInput};
use df_types::{OperationId, RevisionLabel};
use std::mem::size_of;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReferenceSheetView {
    FrontFullBody,
    SideFullBody,
    BackFullBody,
    FaceCloseUp,
}

/// Required reusable sheet composition. These are descriptive until the native adapter maps them.
pub const REFERENCE_SHEET_VIEWS: [ReferenceSheetView; 4] = [
    ReferenceSheetView::FrontFullBody,
    ReferenceSheetView::SideFullBody,
    ReferenceSheetView::BackFullBody,
    ReferenceSheetView::FaceCloseUp,
];

/// Native-owned approved sheet specification, never a client upload or a provider request DTO.
/// `key.parameters` pins the accepted outfit revision; `policy`/`definition` pins the four-view brief.
/// Cosmetic features/outfit and art direction come from the accepted canonical identity/bible,
/// not from ancestry, class, inventory statistics, secrets or arbitrary checkpoint text.
pub struct CharacterReferencePlan {
    pub moment: NarrativeMoment,
    pub demand: AssetDemand,
    pub intent: DurableIntent,
}

#[derive(Clone, Copy, Debug)]
pub struct ReferenceSheetLimits {
    pub checkpoint: CheckpointLimits,
    pub maximum_owned_bytes: usize,
}

pub struct CharacterReferenceContext<'a> {
    pub command: CommandEntryContext<'a>,
    pub reference_limits: ReferenceSheetLimits,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReferenceSheetError {
    Checkpoint(CheckpointError),
    NotCharacterCreation,
    InvalidAcceptedCreation,
    PlanBinding,
    AppearanceUnavailable,
    IdentityChanged,
    AudienceUnavailable,
    MissingJob,
    StaleCompletion,
    InvalidTransition,
    OutputUnavailable,
    Capacity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReferenceSheetAdmission {
    Queued,
    AlreadyQueued,
    Unavailable(ReferenceSheetError),
}

pub struct CharacterReferenceStaging {
    pub checkpoint: Checkpoint,
    pub reference: ReferenceSheetAdmission,
}

/// Actual registered acceptance hook. Optional media refusal cannot reject a legal character.
/// Session must resolve prior durable operation receipts before calling the rules registry.
/// A source-qualified creation producer must return an accepted draft and matching character;
/// this adapter does not create such a producer or infer lawful acceptance from a phase label.
pub fn decide_registered_character_reference<H: RulesCommandHandler>(
    input: RulesCommandInput<'_>,
    current: &Checkpoint,
    context: CharacterReferenceContext<'_>,
    registry: &DispatchRegistry<'_, H>,
    selector: &RevisionLabel,
    source: &RuleReference,
    plan: &CharacterReferencePlan,
) -> Result<CharacterReferenceStaging, CommandRejection<H::Rejection>> {
    let inventory = ReferenceInventory {
        rules: context.command.inventory.rules,
        content: context.command.inventory.content,
        resources: context.command.inventory.resources,
        assets: context.command.inventory.assets,
    };
    let accepted =
        decide_registered_command(input, current, context.command, registry, selector, source)?;
    Ok(stage_accepted_character_reference(
        current,
        accepted,
        input.command,
        plan,
        inventory,
        context.reference_limits,
    ))
}

/// Adds one optional canonical demand to the already accepted next-revision staging result.
/// Consumes the owned accepted checkpoint so optional failure needs no fallback allocation.
/// A retry with the same declaration preserves its existing lifecycle; changed IDs are refused.
/// This is not a substitute for the registered source-handler acceptance guard above.
pub fn stage_accepted_character_reference(
    current: &Checkpoint,
    accepted: Checkpoint,
    command: &GameInput,
    plan: &CharacterReferencePlan,
    inventory: ReferenceInventory<'_>,
    limits: ReferenceSheetLimits,
) -> CharacterReferenceStaging {
    match stage_reference(current, &accepted, command, plan, inventory, limits) {
        Ok(Some(checkpoint)) => CharacterReferenceStaging {
            checkpoint,
            reference: ReferenceSheetAdmission::Queued,
        },
        Ok(None) => CharacterReferenceStaging {
            checkpoint: accepted,
            reference: ReferenceSheetAdmission::AlreadyQueued,
        },
        Err(error) => CharacterReferenceStaging {
            checkpoint: accepted,
            reference: ReferenceSheetAdmission::Unavailable(error),
        },
    }
}

fn stage_reference(
    current: &Checkpoint,
    accepted: &Checkpoint,
    command: &GameInput,
    plan: &CharacterReferencePlan,
    inventory: ReferenceInventory<'_>,
    limits: ReferenceSheetLimits,
) -> Result<Option<Checkpoint>, ReferenceSheetError> {
    preflight(&[current, accepted], 0, limits)?;
    if [
        plan.moment.characters.len(),
        plan.moment.facts.len(),
        plan.moment.attributed_claims.len(),
        plan.demand.key.references.len(),
    ]
    .into_iter()
    .any(|count| count > limits.checkpoint.maximum_records)
    {
        return Err(ReferenceSheetError::Capacity);
    }
    preflight(&[current, accepted], plan_bytes(plan)?, limits)?;
    let GameInput::Game(input) = command else {
        return Err(ReferenceSheetError::NotCharacterCreation);
    };
    let GameCommand::SubmitCharacterDraft { draft } = &input.command else {
        return Err(ReferenceSheetError::NotCharacterCreation);
    };
    let next = Basis {
        revision: current
            .basis()
            .revision
            .next_sequence()
            .map_err(|_| ReferenceSheetError::Capacity)?,
        ..current.basis()
    };
    current
        .validate_resume(current.basis(), accepted.pins())
        .map_err(ReferenceSheetError::Checkpoint)?;
    accepted
        .validate_resume(next, current.pins())
        .map_err(ReferenceSheetError::Checkpoint)?;
    if input.basis.session != current.basis().session
        || input.basis.run != current.basis().run
        || input.basis.revision.epoch() != current.basis().revision.epoch()
        || input.basis.revision > current.basis().revision
        || input.member != draft.member
        || draft.phase == CreationPhase::Accepted
        || current
            .state()
            .decisions
            .iter()
            .any(|decision| decision.operation == input.operation)
    {
        return Err(ReferenceSheetError::InvalidAcceptedCreation);
    }
    let saved = accepted
        .state()
        .continuity
        .creation
        .iter()
        .find(|record| record.entity == draft.entity)
        .ok_or(ReferenceSheetError::InvalidAcceptedCreation)?;
    let character = accepted
        .state()
        .characters
        .iter()
        .find(|record| record.entity == draft.entity)
        .ok_or(ReferenceSheetError::InvalidAcceptedCreation)?;
    let decision = accepted
        .state()
        .decisions
        .iter()
        .find(|record| record.operation == input.operation)
        .ok_or(ReferenceSheetError::InvalidAcceptedCreation)?;
    if saved.phase != CreationPhase::Accepted
        || saved.member != input.member
        || character.owner != input.member
        || character.choices != saved.choices
        || saved.ancestry != draft.ancestry
        || saved.background != draft.background
        || saved.classes != draft.classes
        || saved.choices != draft.choices
        || decision.revision != next.revision
        || current
            .state()
            .continuity
            .creation
            .iter()
            .any(|record| record.entity == draft.entity && record.phase == CreationPhase::Accepted)
        || !accepted
            .state()
            .members
            .iter()
            .any(|member| member.member == input.member && member.character == Some(draft.entity))
    {
        return Err(ReferenceSheetError::InvalidAcceptedCreation);
    }
    let (_, identity) = identity_for(
        accepted,
        draft.entity,
        &plan.demand.key.identity,
        &plan.demand.key.style,
    )?;
    let appearance = identity
        .character_appearance
        .as_ref()
        .ok_or(ReferenceSheetError::AppearanceUnavailable)?;
    if plan.moment.characters.as_slice() != [draft.entity]
        || !plan.moment.facts.is_empty()
        || !plan.moment.attributed_claims.is_empty()
        || plan.moment.identity_revision != identity.revision
        || plan.moment.semantic_focus != plan.demand.policy
        || plan.moment.audience != plan.demand.key.audience
        || !member_audience(&plan.demand.key.audience, input.member)
        || plan.demand.key.parameters != appearance.outfit_revision
        || plan.demand.key.schema != CHECKPOINT_SCHEMA
        || plan.demand.key.source != accepted.pins().content.content_digest
        || plan.demand.key.moment != plan.moment.id
        || plan.demand.key.voice.is_some()
        || plan.demand.basis != next
        || plan.demand.mode != accepted.state().mode
        || plan.demand.priority != DemandPriority::Optional
        || plan.demand.maximum_bytes == 0
        || plan.demand.expires.ticks_per_second != accepted.state().logical_time.ticks_per_second
        || plan.demand.expires.ticks <= accepted.state().logical_time.ticks
        || plan.intent.basis != next
        || plan.intent.operation != input.operation
        || plan.intent.kind != EffectKind::RunMedia
        || plan.intent.job.is_none()
        || plan.intent.timer.is_some()
        || plan.intent.generation == 0
        || plan.intent.status != DurableStatus::Pending
        || plan.intent.definition != plan.demand.policy
    {
        return Err(ReferenceSheetError::PlanBinding);
    }
    let (bible, _) = identity_for(
        accepted,
        draft.entity,
        &identity.revision,
        &plan.demand.key.style,
    )?;
    if plan.demand.key.references.iter().any(|reference| {
        !inventory.assets.contains(reference)
            || (!identity.appearances.contains(reference) && !bible.references.contains(reference))
    }) {
        return Err(ReferenceSheetError::PlanBinding);
    }
    let existing =
        accepted.state().intents.iter().find(|intent| {
            intent.operation == input.operation && intent.kind == EffectKind::RunMedia
        });
    if let Some(existing) = existing {
        let same_intent = existing.id == plan.intent.id
            && existing.basis == plan.intent.basis
            && existing.job == plan.intent.job
            && existing.slot == plan.intent.slot
            && existing.generation == plan.intent.generation
            && existing.definition == plan.intent.definition;
        let same_records = accepted.state().continuity.moments.contains(&plan.moment)
            && accepted.state().continuity.demands.contains(&plan.demand)
            && accepted.state().continuity.asset_jobs.iter().any(|job| {
                Some(job.job) == plan.intent.job
                    && job.demand == plan.demand.id
                    && job.generation == plan.intent.generation
            })
            && decision.effects.contains(&plan.intent.id);
        return if same_intent && same_records {
            Ok(None)
        } else {
            Err(ReferenceSheetError::PlanBinding)
        };
    }
    let mut state = accepted.state().clone();
    let decision = state
        .decisions
        .iter_mut()
        .find(|record| record.operation == input.operation)
        .ok_or(ReferenceSheetError::InvalidAcceptedCreation)?;
    decision.effects.push(plan.intent.id);
    state.intents.push(plan.intent.clone());
    state.continuity.moments.push(plan.moment.clone());
    state.continuity.demands.push(plan.demand.clone());
    state.continuity.asset_jobs.push(AssetJobState {
        job: plan.intent.job.ok_or(ReferenceSheetError::PlanBinding)?,
        demand: plan.demand.id,
        generation: plan.intent.generation,
        state: AssetLifecycle::Queued,
        published: None,
        dispatch: DurableStatus::Pending,
    });
    finish(accepted, next, state, inventory, limits).map(Some)
}

pub enum ReferenceSheetEvent<'a> {
    Started {
        basis: Basis,
        operation: OperationId,
        job: JobId,
        generation: u64,
    },
    Completed {
        completion: &'a JobCompletion,
        canonical_pack: Option<&'a CanonicalPack>,
    },
}

/// Stages observed native job state without executing or committing it. Inventory is supplied by
/// the trusted complete-byte/current-rights publication owner, not evidence reconstructed here.
/// Image decoding/format/four-view qualification and budget reconciliation remain native gates.
/// Ambiguous failure after start retains SentUnknown dispatch liability; it never requeues a job.
pub fn stage_character_reference_event(
    current: &Checkpoint,
    expected_basis: Basis,
    admitted_pins: &CheckpointPins,
    event: ReferenceSheetEvent<'_>,
    inventory: ReferenceInventory<'_>,
    limits: ReferenceSheetLimits,
) -> Result<Checkpoint, ReferenceSheetError> {
    preflight(&[current], 0, limits)?;
    let extra = match &event {
        ReferenceSheetEvent::Completed {
            canonical_pack: Some(pack),
            ..
        } => {
            if [
                pack.identities.len(),
                pack.bible.references.len(),
                pack.bible.palette.len(),
            ]
            .into_iter()
            .any(|count| count > limits.checkpoint.maximum_records)
                || pack.bible.style.len() > limits.checkpoint.maximum_text_bytes
            {
                return Err(ReferenceSheetError::Capacity);
            }
            for identity in &pack.identities {
                if [
                    identity.appearances.len(),
                    identity.sound.len(),
                    identity.source_facts.len(),
                ]
                .into_iter()
                .any(|count| count > limits.checkpoint.maximum_records)
                    || identity
                        .character_appearance
                        .as_ref()
                        .is_some_and(|appearance| {
                            appearance.features.len() > limits.checkpoint.maximum_text_bytes
                                || appearance.outfit.len() > limits.checkpoint.maximum_text_bytes
                        })
                {
                    return Err(ReferenceSheetError::Capacity);
                }
            }
            pack_bytes(pack)?
        }
        _ => 0,
    };
    preflight(&[current], extra, limits)?;
    current
        .validate_resume(expected_basis, admitted_pins)
        .map_err(ReferenceSheetError::Checkpoint)?;
    let (basis, operation, job_id, generation) = match &event {
        ReferenceSheetEvent::Started {
            basis,
            operation,
            job,
            generation,
        } => (*basis, *operation, *job, *generation),
        ReferenceSheetEvent::Completed { completion, .. } => (
            completion.basis,
            completion.operation,
            completion.job,
            completion.generation,
        ),
    };
    let job = current
        .state()
        .continuity
        .asset_jobs
        .iter()
        .find(|job| job.job == job_id)
        .ok_or(ReferenceSheetError::MissingJob)?;
    let binding = reference_binding(current, job_id)?;
    let demand = binding.demand;
    let intent = binding.intent;
    let identity = binding.identity;
    let bible = binding.bible;
    let character = identity.entity;
    if generation == 0
        || generation != job.generation
        || generation != intent.generation
        || basis != demand.basis
        || basis != intent.basis
        || operation != intent.operation
    {
        return Err(ReferenceSheetError::StaleCompletion);
    }
    let (state_value, dispatch, published, pack) = match event {
        ReferenceSheetEvent::Started { .. } => {
            if job.state == AssetLifecycle::Generating && job.dispatch == DurableStatus::SentUnknown
            {
                return Ok(current.clone());
            }
            if job.state != AssetLifecycle::Queued {
                return Err(ReferenceSheetError::InvalidTransition);
            }
            (
                AssetLifecycle::Generating,
                DurableStatus::SentUnknown,
                None,
                None,
            )
        }
        ReferenceSheetEvent::Completed {
            completion,
            canonical_pack,
        } => match &completion.outcome {
            JobOutcome::Media {
                asset,
                demand: completed_demand,
            } => {
                if *completed_demand != demand.id
                    || asset.kind != AssetKind::Image
                    || asset.byte_length == 0
                    || asset.byte_length > demand.maximum_bytes
                    || !inventory.assets.contains(asset)
                {
                    return Err(ReferenceSheetError::OutputUnavailable);
                }
                let pack = canonical_pack.ok_or(ReferenceSheetError::OutputUnavailable)?;
                if job.state == AssetLifecycle::Ready
                    && job.published.as_ref() == Some(asset)
                    && current.state().continuity.canonical_packs.contains(pack)
                {
                    return Ok(current.clone());
                }
                if demand.expires.ticks_per_second != current.state().logical_time.ticks_per_second
                    || demand.expires.ticks <= current.state().logical_time.ticks
                {
                    return Err(ReferenceSheetError::OutputUnavailable);
                }
                let [published_identity] = pack.identities.as_slice() else {
                    return Err(ReferenceSheetError::OutputUnavailable);
                };
                if &pack.bible != bible
                    || published_identity.entity != character
                    || published_identity.revision != identity.revision
                    || published_identity.character_appearance != identity.character_appearance
                    || published_identity.source_facts != identity.source_facts
                    || published_identity.voice != identity.voice
                    || published_identity.sound != identity.sound
                    || !published_identity
                        .appearances
                        .starts_with(&identity.appearances)
                    || published_identity.appearances.last() != Some(asset)
                    || published_identity.appearances.len() != identity.appearances.len() + 1
                {
                    return Err(ReferenceSheetError::OutputUnavailable);
                }
                if current
                    .state()
                    .continuity
                    .canonical_packs
                    .iter()
                    .any(|prior| prior.revision == pack.revision)
                    || !matches!(
                        job.state,
                        AssetLifecycle::Queued | AssetLifecycle::Generating
                    )
                {
                    return Err(ReferenceSheetError::InvalidTransition);
                }
                (
                    AssetLifecycle::Ready,
                    DurableStatus::Completed,
                    Some(asset.clone()),
                    Some(pack),
                )
            }
            JobOutcome::Failed(failure) => {
                if canonical_pack.is_some() {
                    return Err(ReferenceSheetError::OutputUnavailable);
                }
                let state = match failure {
                    NativeFailure::Cancelled => AssetLifecycle::Cancelled,
                    NativeFailure::Stale => AssetLifecycle::Stale,
                    _ => AssetLifecycle::Failed,
                };
                if job.state == state && job.published.is_none() {
                    return Ok(current.clone());
                }
                if !matches!(
                    job.state,
                    AssetLifecycle::Queued | AssetLifecycle::Generating
                ) {
                    return Err(ReferenceSheetError::InvalidTransition);
                }
                let dispatch = if job.dispatch == DurableStatus::SentUnknown {
                    DurableStatus::SentUnknown
                } else if *failure == NativeFailure::Cancelled {
                    DurableStatus::Cancelled
                } else {
                    DurableStatus::Failed
                };
                (state, dispatch, None, None)
            }
            _ => return Err(ReferenceSheetError::InvalidTransition),
        },
    };
    let mut state = current.state().clone();
    let job = state
        .continuity
        .asset_jobs
        .iter_mut()
        .find(|job| job.job == job_id)
        .ok_or(ReferenceSheetError::MissingJob)?;
    job.state = state_value;
    job.dispatch = dispatch;
    job.published = published;
    let intent = state
        .intents
        .iter_mut()
        .find(|intent| intent.job == Some(job_id) && intent.kind == EffectKind::RunMedia)
        .ok_or(ReferenceSheetError::MissingJob)?;
    intent.status = dispatch;
    if let Some(pack) = pack {
        state.continuity.canonical_packs.push(pack.clone());
    }
    let next = Basis {
        revision: current
            .basis()
            .revision
            .next_sequence()
            .map_err(|_| ReferenceSheetError::Capacity)?,
        ..current.basis()
    };
    finish(current, next, state, inventory, limits)
}

/// Prepares a later scene request using its actual committed permitted moment. Only the original
/// member audience is supported until the production owner supplies an explicit wider grant.
/// This returns a request key, not a generated/rendered scene, authorization or publication receipt.
pub fn character_reference_for_scene(
    current: &Checkpoint,
    character: EntityId,
    identity: &RevisionLabel,
    outfit: &RevisionLabel,
    style: &RevisionLabel,
    scene_key: &AssetRequestKey,
    limits: ReferenceSheetLimits,
) -> Result<AssetRequestKey, ReferenceSheetError> {
    preflight(&[current], 0, limits)?;
    if scene_key.references.len() > limits.checkpoint.maximum_records {
        return Err(ReferenceSheetError::Capacity);
    }
    preflight(&[current], key_bytes(scene_key)?, limits)?;
    current
        .validate_resume(current.basis(), current.pins())
        .map_err(ReferenceSheetError::Checkpoint)?;
    let moment = current
        .state()
        .continuity
        .moments
        .iter()
        .find(|moment| moment.id == scene_key.moment)
        .ok_or(ReferenceSheetError::PlanBinding)?;
    if scene_key.schema != CHECKPOINT_SCHEMA
        || scene_key.source != current.pins().content.content_digest
        || &scene_key.style != style
        || scene_key.identity != moment.identity_revision
        || !moment.characters.contains(&character)
        || moment.audience != scene_key.audience
    {
        return Err(ReferenceSheetError::PlanBinding);
    }
    let (_, canonical) = identity_for(current, character, identity, style)?;
    if canonical
        .character_appearance
        .as_ref()
        .is_none_or(|appearance| &appearance.outfit_revision != outfit)
    {
        return Err(ReferenceSheetError::IdentityChanged);
    }
    let mut matching_reference = None;
    for job in &current.state().continuity.asset_jobs {
        let Some(demand) = current
            .state()
            .continuity
            .demands
            .iter()
            .find(|demand| demand.id == job.demand)
        else {
            continue;
        };
        let Some(origin) = current
            .state()
            .continuity
            .moments
            .iter()
            .find(|moment| moment.id == demand.key.moment)
        else {
            continue;
        };
        if origin.characters.as_slice() != [character]
            || &demand.key.identity != identity
            || &demand.key.parameters != outfit
            || &demand.key.style != style
        {
            continue;
        }
        if demand.key.audience != scene_key.audience {
            return Err(ReferenceSheetError::AudienceUnavailable);
        }
        if job.state != AssetLifecycle::Ready {
            return Err(ReferenceSheetError::OutputUnavailable);
        }
        let binding = reference_binding(current, job.job)?;
        if binding.identity.entity != character {
            return Err(ReferenceSheetError::PlanBinding);
        }
        let reference = job
            .published
            .as_ref()
            .ok_or(ReferenceSheetError::OutputUnavailable)?;
        if !canonical.appearances.contains(reference) {
            return Err(ReferenceSheetError::OutputUnavailable);
        }
        if matching_reference.is_some() {
            return Err(ReferenceSheetError::PlanBinding);
        }
        matching_reference = Some(reference);
    }
    let reference = matching_reference.ok_or(ReferenceSheetError::OutputUnavailable)?;
    let mut key = scene_key.clone();
    if !key.references.contains(reference) {
        key.references.push(reference.clone());
    }
    if key_bytes(&key)? > limits.maximum_owned_bytes {
        return Err(ReferenceSheetError::Capacity);
    }
    Ok(key)
}

/// Borrowed dispatch specification resolved from the actual canonical queued/generating job.
/// Passing it to an adapter is request preparation, never authority to spend or dispatch.
pub struct ReferenceSheetSpecification<'a> {
    pub appearance: &'a CharacterAppearance,
    pub visual_bible: &'a VisualBible,
    pub views: &'static [ReferenceSheetView; 4],
    pub demand: &'a AssetDemand,
}

pub fn reference_sheet_specification(
    current: &Checkpoint,
    job: JobId,
    limits: ReferenceSheetLimits,
) -> Result<ReferenceSheetSpecification<'_>, ReferenceSheetError> {
    preflight(&[current], 0, limits)?;
    current
        .validate_resume(current.basis(), current.pins())
        .map_err(ReferenceSheetError::Checkpoint)?;
    let binding = reference_binding(current, job)?;
    if !matches!(
        binding.job.state,
        AssetLifecycle::Queued | AssetLifecycle::Generating
    ) || binding.demand.expires.ticks_per_second != current.state().logical_time.ticks_per_second
        || binding.demand.expires.ticks <= current.state().logical_time.ticks
    {
        return Err(ReferenceSheetError::InvalidTransition);
    }
    Ok(ReferenceSheetSpecification {
        appearance: binding
            .identity
            .character_appearance
            .as_ref()
            .ok_or(ReferenceSheetError::AppearanceUnavailable)?,
        visual_bible: binding.bible,
        views: &REFERENCE_SHEET_VIEWS,
        demand: binding.demand,
    })
}

struct ReferenceBinding<'a> {
    job: &'a AssetJobState,
    demand: &'a AssetDemand,
    intent: &'a DurableIntent,
    bible: &'a VisualBible,
    identity: &'a EntityIdentityRevision,
}

fn member_audience(audience: &AudienceScope, member: df_types::MemberId) -> bool {
    matches!(audience, AudienceScope::Members(ids) if ids.as_slice() == [member])
}

fn reference_binding(
    current: &Checkpoint,
    job_id: JobId,
) -> Result<ReferenceBinding<'_>, ReferenceSheetError> {
    let job = current
        .state()
        .continuity
        .asset_jobs
        .iter()
        .find(|job| job.job == job_id)
        .ok_or(ReferenceSheetError::MissingJob)?;
    let demand = current
        .state()
        .continuity
        .demands
        .iter()
        .find(|demand| demand.id == job.demand)
        .ok_or(ReferenceSheetError::MissingJob)?;
    let intent = current
        .state()
        .intents
        .iter()
        .find(|intent| intent.job == Some(job_id) && intent.kind == EffectKind::RunMedia)
        .ok_or(ReferenceSheetError::MissingJob)?;
    let moment = current
        .state()
        .continuity
        .moments
        .iter()
        .find(|moment| moment.id == demand.key.moment)
        .ok_or(ReferenceSheetError::MissingJob)?;
    let character = match moment.characters.as_slice() {
        [entity] => *entity,
        _ => return Err(ReferenceSheetError::PlanBinding),
    };
    if job.generation == 0
        || job.generation != intent.generation
        || demand.basis != intent.basis
        || job.dispatch != intent.status
        || intent.definition != demand.policy
        || demand.basis.session != current.basis().session
        || demand.basis.run != current.basis().run
        || demand.basis.revision.epoch() != current.basis().revision.epoch()
        || demand.basis.revision > current.basis().revision
        || demand.key.schema != current.schema()
        || demand.key.source != current.pins().content.content_digest
        || demand.priority != DemandPriority::Optional
        || !current.state().decisions.iter().any(|decision| {
            decision.operation == intent.operation
                && decision.revision == demand.basis.revision
                && decision.effects.contains(&intent.id)
        })
    {
        return Err(ReferenceSheetError::StaleCompletion);
    }
    let character_state = current
        .state()
        .characters
        .iter()
        .find(|record| record.entity == character)
        .ok_or(ReferenceSheetError::InvalidAcceptedCreation)?;
    // Creation choices establish origin acceptance; later mechanical choices do not
    // change the captured cosmetic identity or the current owner/audience fence.
    if !current.state().continuity.creation.iter().any(|record| {
        record.entity == character
            && record.member == character_state.owner
            && record.phase == CreationPhase::Accepted
    }) || !current
        .state()
        .members
        .iter()
        .any(|member| member.member == character_state.owner && member.character == Some(character))
        || !member_audience(&demand.key.audience, character_state.owner)
    {
        return Err(ReferenceSheetError::AudienceUnavailable);
    }
    let (bible, identity) =
        identity_for(current, character, &demand.key.identity, &demand.key.style)?;
    let appearance = identity
        .character_appearance
        .as_ref()
        .ok_or(ReferenceSheetError::AppearanceUnavailable)?;
    if appearance.outfit_revision != demand.key.parameters
        || moment.identity_revision != identity.revision
        || moment.audience != demand.key.audience
        || moment.semantic_focus != demand.policy
        || !moment.facts.is_empty()
        || !moment.attributed_claims.is_empty()
    {
        return Err(ReferenceSheetError::IdentityChanged);
    }
    Ok(ReferenceBinding {
        job,
        demand,
        intent,
        bible,
        identity,
    })
}

fn identity_for<'a>(
    current: &'a Checkpoint,
    entity: EntityId,
    revision: &RevisionLabel,
    style: &RevisionLabel,
) -> Result<(&'a VisualBible, &'a EntityIdentityRevision), ReferenceSheetError> {
    if !current
        .state()
        .entities
        .iter()
        .any(|record| record.id == entity && &record.identity_revision == revision)
    {
        return Err(ReferenceSheetError::IdentityChanged);
    }
    let mut selected: Option<(&VisualBible, &EntityIdentityRevision)> = None;
    for pack in &current.state().continuity.canonical_packs {
        if &pack.bible.revision != style {
            continue;
        }
        for identity in &pack.identities {
            if identity.entity == entity && &identity.revision == revision {
                if let Some((bible, prior)) = selected
                    && (bible != &pack.bible
                        || prior.character_appearance != identity.character_appearance)
                {
                    return Err(ReferenceSheetError::IdentityChanged);
                }
                selected = Some((&pack.bible, identity));
            }
        }
    }
    selected.ok_or(ReferenceSheetError::AppearanceUnavailable)
}

fn finish(
    current: &Checkpoint,
    basis: Basis,
    state: GameState,
    inventory: ReferenceInventory<'_>,
    limits: ReferenceSheetLimits,
) -> Result<Checkpoint, ReferenceSheetError> {
    let output = Checkpoint::new(
        current.schema(),
        basis,
        current.pins().clone(),
        state,
        inventory,
        limits.checkpoint,
    )
    .map_err(ReferenceSheetError::Checkpoint)?;
    preflight(&[current, &output], 0, limits)?;
    Ok(output)
}

fn preflight(
    checkpoints: &[&Checkpoint],
    extra: usize,
    limits: ReferenceSheetLimits,
) -> Result<(), ReferenceSheetError> {
    if limits.maximum_owned_bytes == 0
        || limits.checkpoint.maximum_records == 0
        || limits.checkpoint.maximum_text_bytes == 0
        || limits.checkpoint.maximum_total_text_bytes == 0
    {
        return Err(ReferenceSheetError::Capacity);
    }
    let mut owned_bytes = extra.checked_mul(3).ok_or(ReferenceSheetError::Capacity)?;
    for checkpoint in checkpoints {
        let state = checkpoint.state();
        let mut records = 0usize;
        for count in [
            state.entities.len(),
            state.characters.len(),
            state.members.len(),
            state.decisions.len(),
            state.intents.len(),
            state.continuity.moments.len(),
            state.continuity.demands.len(),
            state.continuity.asset_jobs.len(),
            state.continuity.canonical_packs.len(),
            state.continuity.creation.len(),
        ] {
            records = records
                .checked_add(count)
                .filter(|count| *count <= limits.checkpoint.maximum_records)
                .ok_or(ReferenceSheetError::Capacity)?;
        }
        for pack in &state.continuity.canonical_packs {
            records = records
                .checked_add(pack.identities.len())
                .filter(|count| *count <= limits.checkpoint.maximum_records)
                .ok_or(ReferenceSheetError::Capacity)?;
        }
        let bytes = checkpoint
            .retained_bytes()
            .ok_or(ReferenceSheetError::Capacity)?;
        if bytes > limits.checkpoint.maximum_retained_bytes {
            return Err(ReferenceSheetError::Capacity);
        }
        owned_bytes = owned_bytes
            .checked_add(bytes.checked_mul(3).ok_or(ReferenceSheetError::Capacity)?)
            .ok_or(ReferenceSheetError::Capacity)?;
    }
    if owned_bytes > limits.maximum_owned_bytes {
        return Err(ReferenceSheetError::Capacity);
    }
    Ok(())
}
fn label_bytes(label: &RevisionLabel) -> usize {
    label.retained_heap_bytes()
}
fn content_bytes(content: &ContentReference) -> Result<usize, ReferenceSheetError> {
    label_bytes(&content.package)
        .checked_add(label_bytes(&content.entry))
        .ok_or(ReferenceSheetError::Capacity)
}
fn audience_bytes(audience: &AudienceScope) -> Result<usize, ReferenceSheetError> {
    match audience {
        AudienceScope::Members(ids) => ids
            .capacity()
            .checked_mul(size_of::<df_types::MemberId>())
            .ok_or(ReferenceSheetError::Capacity),
        _ => Ok(0),
    }
}
fn asset_bytes(asset: &AssetReference) -> Result<usize, ReferenceSheetError> {
    size_of::<AssetReference>()
        .checked_add(label_bytes(&asset.key))
        .ok_or(ReferenceSheetError::Capacity)
}
fn key_bytes(key: &AssetRequestKey) -> Result<usize, ReferenceSheetError> {
    let mut total = size_of::<AssetRequestKey>();
    for label in [
        &key.identity,
        &key.style,
        &key.provider,
        &key.model,
        &key.format,
        &key.parameters,
    ] {
        total = total
            .checked_add(label_bytes(label))
            .ok_or(ReferenceSheetError::Capacity)?;
    }
    if let Some(voice) = &key.voice {
        total = total
            .checked_add(label_bytes(voice))
            .ok_or(ReferenceSheetError::Capacity)?;
    }
    total = total
        .checked_add(audience_bytes(&key.audience)?)
        .and_then(|bytes| {
            bytes.checked_add(
                key.references
                    .capacity()
                    .checked_mul(size_of::<AssetReference>())?,
            )
        })
        .ok_or(ReferenceSheetError::Capacity)?;
    for reference in &key.references {
        total = total
            .checked_add(label_bytes(&reference.key))
            .ok_or(ReferenceSheetError::Capacity)?;
    }
    Ok(total)
}
fn plan_bytes(plan: &CharacterReferencePlan) -> Result<usize, ReferenceSheetError> {
    let mut total = size_of::<CharacterReferencePlan>();
    for amount in [
        key_bytes(&plan.demand.key)?,
        audience_bytes(&plan.moment.audience)?,
        content_bytes(&plan.moment.semantic_focus)?,
        content_bytes(&plan.demand.policy)?,
        content_bytes(&plan.intent.definition)?,
        label_bytes(&plan.moment.identity_revision),
        label_bytes(&plan.demand.budget_reservation),
    ] {
        total = total
            .checked_add(amount)
            .ok_or(ReferenceSheetError::Capacity)?;
    }
    for (capacity, width) in [
        (plan.moment.characters.capacity(), size_of::<EntityId>()),
        (plan.moment.facts.capacity(), size_of::<FactId>()),
        (
            plan.moment.attributed_claims.capacity(),
            size_of::<RecordId>(),
        ),
    ] {
        total = total
            .checked_add(
                capacity
                    .checked_mul(width)
                    .ok_or(ReferenceSheetError::Capacity)?,
            )
            .ok_or(ReferenceSheetError::Capacity)?;
    }
    Ok(total)
}
fn pack_bytes(pack: &CanonicalPack) -> Result<usize, ReferenceSheetError> {
    let mut total = size_of::<CanonicalPack>();
    for amount in [
        label_bytes(&pack.revision),
        label_bytes(&pack.bible.revision),
        content_bytes(&pack.bible.definition)?,
        pack.bible.style.capacity(),
    ] {
        total = total
            .checked_add(amount)
            .ok_or(ReferenceSheetError::Capacity)?;
    }
    for (capacity, width) in [
        (pack.bible.palette.capacity(), size_of::<RevisionLabel>()),
        (
            pack.bible.references.capacity(),
            size_of::<AssetReference>(),
        ),
        (
            pack.identities.capacity(),
            size_of::<EntityIdentityRevision>(),
        ),
    ] {
        total = total
            .checked_add(
                capacity
                    .checked_mul(width)
                    .ok_or(ReferenceSheetError::Capacity)?,
            )
            .ok_or(ReferenceSheetError::Capacity)?;
    }
    for label in &pack.bible.palette {
        total = total
            .checked_add(label_bytes(label))
            .ok_or(ReferenceSheetError::Capacity)?;
    }
    for asset in &pack.bible.references {
        total = total
            .checked_add(label_bytes(&asset.key))
            .ok_or(ReferenceSheetError::Capacity)?;
    }
    for identity in &pack.identities {
        total = total
            .checked_add(label_bytes(&identity.revision))
            .ok_or(ReferenceSheetError::Capacity)?;
        if let Some(appearance) = &identity.character_appearance {
            for amount in [
                appearance.features.capacity(),
                appearance.outfit.capacity(),
                label_bytes(&appearance.outfit_revision),
            ] {
                total = total
                    .checked_add(amount)
                    .ok_or(ReferenceSheetError::Capacity)?;
            }
        }
        for (capacity, width) in [
            (identity.source_facts.capacity(), size_of::<FactId>()),
            (identity.appearances.capacity(), size_of::<AssetReference>()),
            (identity.sound.capacity(), size_of::<AssetReference>()),
        ] {
            total = total
                .checked_add(
                    capacity
                        .checked_mul(width)
                        .ok_or(ReferenceSheetError::Capacity)?,
                )
                .ok_or(ReferenceSheetError::Capacity)?;
        }
        for reference in identity
            .appearances
            .iter()
            .chain(&identity.sound)
            .chain(identity.voice.iter())
        {
            total = total
                .checked_add(asset_bytes(reference)?)
                .ok_or(ReferenceSheetError::Capacity)?;
        }
    }
    Ok(total)
}
