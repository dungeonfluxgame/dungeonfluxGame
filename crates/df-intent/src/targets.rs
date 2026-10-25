//! Exact target resolution of canonical World affordance results; proposals never execute.

use df_model::affordance::{AffordanceSet, TargetResolution, TargetSelection};
use df_model::checkpoint::{Checkpoint, CheckpointError, ReferenceInventory};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TargetResolutionLimits {
    pub input_records: usize,
    pub candidates: usize,
    pub output_bytes: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TargetResolutionError {
    Snapshot(CheckpointError),
    Capacity,
    AllocationCapacity,
    InvalidAffordance,
    AmbiguousSource,
    ConflictingTargetPolicy,
}

/// Revalidates the complete lookup basis and canonical records before resolving exact identity.
/// Source-admitted World results and independently authorized actor/input context are required.
/// Unspecified multiple targets return clarification, including when names would look identical.
/// An explicit unsupported identity and a missing admitted mechanic remain separate outcomes.
/// No confidence, display label, collection order, roll or model output authorizes selection.
pub fn resolve_target<'a>(
    current: &Checkpoint,
    set: &'a AffordanceSet<'_>,
    selection: TargetSelection,
    inventory: ReferenceInventory<'_>,
    limits: TargetResolutionLimits,
) -> Result<TargetResolution<'a>, TargetResolutionError> {
    current
        .validate_resume(set.basis, set.pins)
        .map_err(TargetResolutionError::Snapshot)?;
    let record_count = current
        .state()
        .entities
        .len()
        .checked_add(set.matches.len())
        .and_then(|record_count| record_count.checked_add(inventory.rules.len()))
        .and_then(|record_count| record_count.checked_add(inventory.content.len()))
        .ok_or(TargetResolutionError::Capacity)?;
    if limits.input_records == 0
        || limits.candidates == 0
        || limits.output_bytes == 0
        || record_count > limits.input_records
        || set.matches.len() > limits.candidates
        || set
            .retained_bytes()
            .is_none_or(|bytes| bytes > limits.output_bytes)
    {
        return Err(TargetResolutionError::Capacity);
    }
    if !current
        .state()
        .entities
        .iter()
        .any(|entity| entity.id == set.actor)
        || set.action.package != current.pins().content.package
        || !inventory.content.contains(&set.action)
    {
        return Err(TargetResolutionError::InvalidAffordance);
    }
    for (index, record) in set.matches.iter().enumerate() {
        if record.actor != set.actor
            || record.action != set.action
            || record.source.catalog != current.pins().rules.catalog
            || !inventory.rules.contains(&record.source)
        {
            return Err(TargetResolutionError::InvalidAffordance);
        }
        if let Some(target) = &record.target
            && (!current
                .state()
                .entities
                .iter()
                .any(|entity| entity == target)
                || !inventory.content.contains(&target.definition))
        {
            return Err(TargetResolutionError::InvalidAffordance);
        }
        for previous in set.matches.iter().take(index) {
            if previous.target.as_ref().map(|target| target.id)
                == record.target.as_ref().map(|target| target.id)
            {
                return Err(TargetResolutionError::AmbiguousSource);
            }
            if previous.target.is_some() != record.target.is_some() {
                return Err(TargetResolutionError::ConflictingTargetPolicy);
            }
        }
    }
    if set.matches.is_empty() {
        return Ok(TargetResolution::NeedsRuling);
    }
    if let TargetSelection::Explicit(target) = selection {
        return Ok(
            match set.matches.iter().find(|entry| {
                entry
                    .target
                    .as_ref()
                    .is_some_and(|entity| entity.id == target)
            }) {
                Some(entry) => TargetResolution::Selected(entry),
                None => TargetResolution::UnsupportedTarget,
            },
        );
    }
    let mut entries = set.matches.iter();
    let first = entries
        .next()
        .ok_or(TargetResolutionError::InvalidAffordance)?;
    if entries.next().is_none() {
        return Ok(TargetResolution::Selected(first));
    }
    let candidate_bytes = set
        .matches
        .len()
        .checked_mul(std::mem::size_of::<df_model::checkpoint::EntityId>())
        .ok_or(TargetResolutionError::Capacity)?;
    if candidate_bytes > limits.output_bytes {
        return Err(TargetResolutionError::Capacity);
    }
    let mut candidates = Vec::new();
    candidates
        .try_reserve_exact(set.matches.len())
        .map_err(|_| TargetResolutionError::AllocationCapacity)?;
    for entry in &set.matches {
        candidates.push(
            entry
                .target
                .as_ref()
                .ok_or(TargetResolutionError::ConflictingTargetPolicy)?
                .id,
        );
    }
    candidates.sort_by_key(|id| *id.as_bytes());
    Ok(TargetResolution::NeedsClarification(candidates))
}
