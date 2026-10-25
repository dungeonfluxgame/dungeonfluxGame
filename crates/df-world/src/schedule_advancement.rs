use std::mem::size_of;

use df_model::checkpoint::{Checkpoint, CheckpointPins, ContentReference, EntityId, RecordId};

use crate::due_events::{
    DueSelection, DueSelectionError, DueSelectionLimits, DueSelectionRequest, select_due_events,
};

const MAXIMUM_SCHEDULE_RECORDS: usize = 4096;

/// Destination policy already admitted by the content/rules/engine owner.
/// The canonical ScheduledEvent carries no destination; this mapping does not
/// infer one from an entry name, authorize movement or define geometry.
pub struct AdmittedScheduleDestination<'a> {
    pub event: RecordId,
    pub definition: &'a ContentReference,
    pub entity: EntityId,
    pub expected_location: Option<EntityId>,
    pub destination: EntityId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScheduleAdvancementLimits {
    pub selection: DueSelectionLimits,
    pub entities: usize,
    pub destinations: usize,
    pub movements: usize,
    pub output_bytes: usize,
}

/// An ordered candidate location change; publication follows the session commit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScheduledLocationChange {
    pub event: RecordId,
    pub entity: EntityId,
    pub before: Option<EntityId>,
    pub after: EntityId,
}

#[derive(Debug, Eq, PartialEq)]
pub struct ScheduleAdvancement<'a> {
    pub due: DueSelection<'a>,
    pub movements: Vec<ScheduledLocationChange>,
    pub accounted_output_bytes: usize,
}

#[derive(Debug, Eq, PartialEq)]
pub enum ScheduleAdvancementError {
    TimeNotAccepted,
    InvalidLimits,
    Capacity,
    AllocationCapacity,
    UnknownEvent,
    DuplicateDestination,
    StaleDestination,
    UnknownEntity,
    StaleLocation,
    DueSelection(DueSelectionError),
}

/// Stages admitted schedule destinations against an immutable canonical checkpoint.
///
/// Some(request) means the engine has admitted a logical time advance or same-tick
/// catch-up continuation; None refuses unaccepted time. This is a pure contract,
/// not an authentication boundary. No wall/presentation time is accepted or read.
/// The existing due selector owns time/pin/cursor checks, pause and prefix bounds.
/// Unmapped events stay selected for other engine consequence adapters; no effect
/// or fact is created here. Repeated entity moves follow the selected event order
/// and each checks the preceding candidate location. Any refusal discards the
/// whole proposal, including the candidate cursor. The engine/session owner must
/// commit time, every selected consequence and cursor atomically, recheck basis
/// and admitted mappings, and deduplicate operation/continuation IDs.
///
/// Each input dimension is capped at 4096. Retained output accounting includes
/// the due selector's accounting and movement vector capacity, conservatively
/// counting its inline due value again; serialization is a separate owner gate.
pub fn stage_schedule_advancement<'a>(
    checkpoint: &'a Checkpoint,
    admitted_pins: &CheckpointPins,
    accepted_time: Option<DueSelectionRequest<'_>>,
    destinations: &[AdmittedScheduleDestination<'_>],
    limits: ScheduleAdvancementLimits,
) -> Result<ScheduleAdvancement<'a>, ScheduleAdvancementError> {
    let request = accepted_time.ok_or(ScheduleAdvancementError::TimeNotAccepted)?;
    if limits.entities == 0
        || limits.entities > MAXIMUM_SCHEDULE_RECORDS
        || limits.destinations == 0
        || limits.destinations > MAXIMUM_SCHEDULE_RECORDS
        || limits.movements == 0
        || limits.movements > limits.destinations
        || limits.output_bytes == 0
    {
        return Err(ScheduleAdvancementError::InvalidLimits);
    }
    let state = checkpoint.state();
    if state.entities.len() > limits.entities || destinations.len() > limits.destinations {
        return Err(ScheduleAdvancementError::Capacity);
    }
    let due = select_due_events(checkpoint, admitted_pins, request, limits.selection)
        .map_err(ScheduleAdvancementError::DueSelection)?;
    for (index, destination) in destinations.iter().enumerate() {
        if destinations
            .iter()
            .skip(index + 1)
            .any(|other| other.event == destination.event)
        {
            return Err(ScheduleAdvancementError::DuplicateDestination);
        }
        let event = state
            .schedules
            .iter()
            .find(|event| event.id == destination.event)
            .ok_or(ScheduleAdvancementError::UnknownEvent)?;
        if event.entity != destination.entity || event.definition != *destination.definition {
            return Err(ScheduleAdvancementError::StaleDestination);
        }
        if !state
            .entities
            .iter()
            .any(|entity| entity.id == destination.destination)
        {
            return Err(ScheduleAdvancementError::UnknownEntity);
        }
    }

    let movement_count = due
        .events
        .iter()
        .filter(|event| {
            destinations
                .iter()
                .any(|destination| destination.event == event.id)
        })
        .count();
    if movement_count > limits.movements {
        return Err(ScheduleAdvancementError::Capacity);
    }
    let mut movements: Vec<ScheduledLocationChange> = Vec::new();
    movements
        .try_reserve_exact(movement_count)
        .map_err(|_| ScheduleAdvancementError::AllocationCapacity)?;
    let accounted_output_bytes = size_of::<ScheduledLocationChange>()
        .checked_mul(movements.capacity())
        .and_then(|bytes| bytes.checked_add(size_of::<ScheduleAdvancement<'_>>()))
        .and_then(|bytes| bytes.checked_add(due.accounted_output_bytes))
        .ok_or(ScheduleAdvancementError::Capacity)?;
    if accounted_output_bytes > limits.output_bytes {
        return Err(ScheduleAdvancementError::Capacity);
    }
    for event in &due.events {
        let Some(destination) = destinations
            .iter()
            .find(|destination| destination.event == event.id)
        else {
            continue;
        };
        let entity = state
            .entities
            .iter()
            .find(|entity| entity.id == destination.entity)
            .ok_or(ScheduleAdvancementError::UnknownEntity)?;
        let before = movements
            .iter()
            .rev()
            .find(|movement| movement.entity == entity.id)
            .map_or(entity.location, |movement| Some(movement.after));
        if before != destination.expected_location {
            return Err(ScheduleAdvancementError::StaleLocation);
        }
        movements.push(ScheduledLocationChange {
            event: event.id,
            entity: entity.id,
            before,
            after: destination.destination,
        });
    }
    Ok(ScheduleAdvancement {
        due,
        movements,
        accounted_output_bytes,
    })
}
