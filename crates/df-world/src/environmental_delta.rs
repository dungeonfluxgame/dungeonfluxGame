use std::mem::size_of;

use df_model::checkpoint::{
    AssetKind, Basis, Checkpoint, CheckpointError, CheckpointPins, EnvironmentalState, FactId,
    FactValue, SceneIdentityRevision,
};

const MAX_INPUT_RECORDS: usize = 4096;

/// Exact source/rules-admitted replacement and its current committed geometry basis.
/// Admission belongs to the engine/content/rules owners; this value is not authentication.
/// No environmental mechanics, coordinates or geometry edits are inferred here.
pub struct AdmittedEnvironmentalChange<'a> {
    pub expected: &'a EnvironmentalState,
    pub replacement: &'a EnvironmentalState,
    pub geometry: &'a SceneIdentityRevision,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EnvironmentalDeltaLimits {
    pub input_records: usize,
    pub changes: usize,
    pub output_bytes: usize,
}

/// Owned replacement records, in canonical location-ID order, for an atomic owner commit.
#[derive(Debug, Eq, PartialEq)]
pub struct EnvironmentalDelta<'a> {
    pub basis: Basis,
    pub pins: &'a CheckpointPins,
    pub replacements: Vec<EnvironmentalState>,
    pub accounted_output_bytes: usize,
}

#[derive(Debug, Eq, PartialEq)]
pub enum EnvironmentalDeltaError {
    NotAdmitted,
    InvalidLimits,
    InputCapacity,
    OutputCapacity,
    AllocationCapacity,
    DuplicateLocation,
    DuplicateFact,
    UnknownLocation,
    UnknownEnvironment,
    AmbiguousEnvironment,
    StaleEnvironment,
    StaleSource,
    UnknownGeometry,
    AmbiguousGeometry,
    StaleGeometry,
    UnsupportedGeometry,
    MissingCause,
    UnknownFact,
    InvalidFactOrder,
    StaleFact,
    InvalidFactSource,
    FactNotAccepted,
    Checkpoint(CheckpointError),
}

/// Validates all changes before returning any proposal; a refusal publishes nothing.
/// `None` refuses missing source/rules admission; an admitted empty batch is no change.
///
/// The checkpoint's full pins and session/run/revision must match current owner admission.
/// Each replacement preserves the location and exact existing fact-history prefix, then
/// appends current-revision, accepted ContentEvent facts for its exact definition and
/// location in canonical fact order. History cannot be rewritten or replayed twice.
/// Geometry is the exact current SceneIdentityRevision with a tactical geometry reference;
/// this validates its source basis, not asset bytes, spatial legality or physics.
///
/// All traversed record collections, including nested fact references and subjects, share
/// a positive bound capped at 4096. Output counts retained Rust allocation capacities;
/// encoded size, execution deadlines and production capacity remain native-owner gates.
/// The engine/session must recheck admission, basis and geometry before the fenced atomic
/// commit and deduplicate stable operation IDs. This seam never advances time, changes
/// facts, mutates the checkpoint, performs I/O or grants source/audience authority.
pub fn stage_environmental_delta<'a>(
    checkpoint: &'a Checkpoint,
    admitted_pins: &CheckpointPins,
    expected_basis: Basis,
    admitted_changes: Option<&[AdmittedEnvironmentalChange<'_>]>,
    limits: EnvironmentalDeltaLimits,
) -> Result<EnvironmentalDelta<'a>, EnvironmentalDeltaError> {
    let changes = admitted_changes.ok_or(EnvironmentalDeltaError::NotAdmitted)?;
    if limits.input_records == 0
        || limits.input_records > MAX_INPUT_RECORDS
        || limits.changes == 0
        || limits.changes > limits.input_records
        || limits.output_bytes == 0
    {
        return Err(EnvironmentalDeltaError::InvalidLimits);
    }
    let state = checkpoint.state();
    let mut input_record_count = 0usize;
    let mut count_input_records = |length: usize| -> Result<(), EnvironmentalDeltaError> {
        input_record_count = input_record_count
            .checked_add(length)
            .filter(|total| *total <= limits.input_records)
            .ok_or(EnvironmentalDeltaError::InputCapacity)?;
        Ok(())
    };
    count_input_records(state.entities.len())?;
    count_input_records(state.facts.len())?;
    count_input_records(state.decisions.len())?;
    count_input_records(state.continuity.environment.len())?;
    count_input_records(state.continuity.scenes.len())?;
    count_input_records(changes.len())?;
    if changes.len() > limits.changes {
        return Err(EnvironmentalDeltaError::InputCapacity);
    }
    for fact in &state.facts {
        if let FactValue::ContentEvent { subjects, .. } = &fact.value {
            count_input_records(subjects.len())?;
        }
    }
    for decision in &state.decisions {
        count_input_records(decision.facts.len())?;
    }
    for environment in &state.continuity.environment {
        count_input_records(environment.change_facts.len())?;
    }
    for scene in &state.continuity.scenes {
        count_input_records(scene.source_facts.len())?;
    }
    for change in changes {
        count_input_records(change.expected.change_facts.len())?;
        count_input_records(change.replacement.change_facts.len())?;
        count_input_records(change.geometry.source_facts.len())?;
    }
    checkpoint
        .validate_resume(expected_basis, admitted_pins)
        .map_err(EnvironmentalDeltaError::Checkpoint)?;

    for (index, change) in changes.iter().enumerate() {
        let expected = change.expected;
        let replacement = change.replacement;
        if changes
            .iter()
            .skip(index + 1)
            .any(|other| other.expected.location == expected.location)
        {
            return Err(EnvironmentalDeltaError::DuplicateLocation);
        }
        if expected.location != replacement.location
            || !state
                .entities
                .iter()
                .any(|entity| entity.id == expected.location)
        {
            return Err(EnvironmentalDeltaError::UnknownLocation);
        }
        let mut current = state
            .continuity
            .environment
            .iter()
            .filter(|environment| environment.location == expected.location);
        let current_environment = current
            .next()
            .ok_or(EnvironmentalDeltaError::UnknownEnvironment)?;
        if current.next().is_some() {
            return Err(EnvironmentalDeltaError::AmbiguousEnvironment);
        }
        if current_environment != expected {
            return Err(EnvironmentalDeltaError::StaleEnvironment);
        }
        if replacement.definition.package != checkpoint.pins().content.package {
            return Err(EnvironmentalDeltaError::StaleSource);
        }
        let mut scenes = state
            .continuity
            .scenes
            .iter()
            .filter(|scene| scene.scene == expected.location);
        let scene = scenes
            .next()
            .ok_or(EnvironmentalDeltaError::UnknownGeometry)?;
        if scenes.next().is_some() {
            return Err(EnvironmentalDeltaError::AmbiguousGeometry);
        }
        if scene != change.geometry {
            return Err(EnvironmentalDeltaError::StaleGeometry);
        }
        if scene.geometry.kind != AssetKind::TacticalGeometry {
            return Err(EnvironmentalDeltaError::UnsupportedGeometry);
        }
        if !replacement.change_facts.starts_with(&expected.change_facts) {
            return Err(EnvironmentalDeltaError::StaleEnvironment);
        }
        let mut previous_index = None;
        for (index, id) in replacement.change_facts.iter().enumerate() {
            if replacement
                .change_facts
                .iter()
                .take(index)
                .any(|previous| previous == id)
            {
                return Err(EnvironmentalDeltaError::DuplicateFact);
            }
            let (fact_index, fact) = state
                .facts
                .iter()
                .enumerate()
                .find(|(_, fact)| fact.id == *id)
                .ok_or(EnvironmentalDeltaError::UnknownFact)?;
            if previous_index.is_some_and(|previous| fact_index <= previous) {
                return Err(EnvironmentalDeltaError::InvalidFactOrder);
            }
            previous_index = Some(fact_index);
            if index < expected.change_facts.len() {
                continue;
            }
            if fact.revision != checkpoint.basis().revision {
                return Err(EnvironmentalDeltaError::StaleFact);
            }
            if !matches!(&fact.value, FactValue::ContentEvent { definition, subjects }
                if definition == &replacement.definition && subjects.contains(&replacement.location))
            {
                return Err(EnvironmentalDeltaError::InvalidFactSource);
            }
            if !state.decisions.iter().any(|decision| {
                decision.operation == fact.operation
                    && decision.revision == fact.revision
                    && decision.facts.contains(id)
            }) {
                return Err(EnvironmentalDeltaError::FactNotAccepted);
            }
        }
        if replacement != expected && replacement.change_facts.len() == expected.change_facts.len()
        {
            return Err(EnvironmentalDeltaError::MissingCause);
        }
    }

    let changed_count = changes
        .iter()
        .filter(|change| change.expected != change.replacement)
        .count();
    let mut replacements = Vec::new();
    replacements
        .try_reserve_exact(changed_count)
        .map_err(|_| EnvironmentalDeltaError::AllocationCapacity)?;
    let mut output_bytes = size_of::<EnvironmentalState>()
        .checked_mul(replacements.capacity())
        .and_then(|output_bytes| output_bytes.checked_add(size_of::<EnvironmentalDelta<'_>>()))
        .ok_or(EnvironmentalDeltaError::OutputCapacity)?;
    if output_bytes > limits.output_bytes {
        return Err(EnvironmentalDeltaError::OutputCapacity);
    }
    for change in changes
        .iter()
        .filter(|change| change.expected != change.replacement)
    {
        let replacement = change.replacement;
        let reference_bytes = replacement
            .definition
            .package
            .retained_heap_bytes()
            .checked_add(replacement.definition.entry.retained_heap_bytes())
            .ok_or(EnvironmentalDeltaError::OutputCapacity)?;
        let fact_bytes = size_of::<FactId>()
            .checked_mul(replacement.change_facts.len())
            .and_then(|bytes| bytes.checked_add(reference_bytes))
            .ok_or(EnvironmentalDeltaError::OutputCapacity)?;
        if fact_bytes > limits.output_bytes - output_bytes {
            return Err(EnvironmentalDeltaError::OutputCapacity);
        }
        let definition = replacement.definition.clone();
        let mut change_facts = Vec::new();
        change_facts
            .try_reserve_exact(replacement.change_facts.len())
            .map_err(|_| EnvironmentalDeltaError::AllocationCapacity)?;
        let retained_bytes = size_of::<FactId>()
            .checked_mul(change_facts.capacity())
            .and_then(|bytes| bytes.checked_add(definition.package.retained_heap_bytes()))
            .and_then(|bytes| bytes.checked_add(definition.entry.retained_heap_bytes()))
            .ok_or(EnvironmentalDeltaError::OutputCapacity)?;
        if retained_bytes > limits.output_bytes - output_bytes {
            return Err(EnvironmentalDeltaError::OutputCapacity);
        }
        change_facts.extend_from_slice(&replacement.change_facts);
        output_bytes += retained_bytes;
        replacements.push(EnvironmentalState {
            location: replacement.location,
            definition,
            change_facts,
        });
    }
    replacements.sort_unstable_by_key(|replacement| replacement.location);
    Ok(EnvironmentalDelta {
        basis: checkpoint.basis(),
        pins: checkpoint.pins(),
        replacements,
        accounted_output_bytes: output_bytes,
    })
}
