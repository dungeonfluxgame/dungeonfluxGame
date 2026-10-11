//! Current observer projection of canonical witness provenance.
//!
//! This view pins audience selection to the admitted checkpoint Basis. The
//! witnessed event retains its own operation/revision/ordinal and perception time;
//! neither that provenance nor a witness grants disclosure or reveal authority.
use df_model::checkpoint::{Basis, Checkpoint, CheckpointPins, GameFact, RecordId, WitnessRecord};

use crate::perception::{ObserverScope, PerceptionError, PerceptionLimits, perceive};

/// Resolve a canonical witness only when its fact is in the current observer's
/// audience. The authenticated owner supplies the observer and must revalidate
/// this Basis before later publication. Missing or hidden records return no proof.
///
/// The scan budget includes the witness lookup, the perception fact/member scan,
/// and the selected-fact lookup. Reserve the full lookup bounds before selecting;
/// capacity or validation errors never return a partial provenance tuple.
pub fn perceive_witnessed_fact<'a>(
    current: &'a Checkpoint,
    expected: Basis,
    admitted: &CheckpointPins,
    observer: ObserverScope,
    witness_id: RecordId,
    limits: PerceptionLimits,
) -> Result<Option<(Basis, &'a WitnessRecord, &'a GameFact)>, PerceptionError> {
    current
        .validate_resume(expected, admitted)
        .map_err(PerceptionError::Checkpoint)?;
    let state = current.state();
    let remaining_scans = limits
        .maximum_scan_records
        .checked_sub(state.continuity.witnesses.len())
        .and_then(|remaining| remaining.checked_sub(state.facts.len()))
        .ok_or(PerceptionError::ScanCapacity)?;
    let perceived = perceive(
        current,
        expected,
        admitted,
        observer,
        PerceptionLimits {
            maximum_scan_records: remaining_scans,
            ..limits
        },
    )?;
    let Some(witness) = state
        .continuity
        .witnesses
        .iter()
        .find(|witness| witness.id == witness_id)
    else {
        return Ok(None);
    };
    let Some(fact) = perceived
        .facts()
        .iter()
        .copied()
        .find(|fact| fact.id == witness.fact)
    else {
        return Ok(None);
    };
    Ok(Some((perceived.basis(), witness, fact)))
}
