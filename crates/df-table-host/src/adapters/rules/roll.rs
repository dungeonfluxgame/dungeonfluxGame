//! The roll path of the rules adapter: any `df-rules` roll, from the dice its plan asks the `Dice`
//! port for to the result and its `RULES rolled` line.
//!
//! It owns the mapping between a roll's draw plan and the port's `DrawRequest`s and `Draw`s, and
//! the check that the draws it is given are the dice the plan named. It never draws a die (the
//! caller draws each requested die from the `Dice` port), never decides a target or a mode (the
//! request carries them) and never holds state. A roll that cannot be answered fails with a
//! `PortError` and a `RULES refused` line, a roll with dice still missing is an answer
//! (`RollStep::NeedsDraws`) and writes no line.

use std::num::NonZeroU16;

use df_rules::roll::{DieSize, RollError, RollRequest, RollResult};
use df_table::identity::OperationId;
use df_table::ports::{ActionId, Draw, DrawRequest, PortError};

use super::audit::{RulesRefusal, record, refused_line, rolled_line};

/// What a roll needs next: more dice, or its result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RollStep {
    NeedsDraws(Vec<DrawRequest>),
    Rolled(Box<RollResult>),
}

fn die_request(size: DieSize) -> DrawRequest {
    DrawRequest {
        sides: NonZeroU16::MIN.saturating_add(size.sides() - 1),
    }
}

fn refused(error: RollError) -> RulesRefusal {
    RulesRefusal::RollRefused(error)
}

fn plan(request: &RollRequest) -> Result<Vec<DrawRequest>, RulesRefusal> {
    request
        .draw_plan()
        .map(|sizes| sizes.into_iter().map(die_request).collect())
        .map_err(refused)
}

pub(super) fn step(request: &RollRequest, draws: &[Draw]) -> Result<RollStep, RulesRefusal> {
    let wanted = plan(request)?;
    if draws.len() > wanted.len()
        || draws
            .iter()
            .zip(&wanted)
            .any(|(draw, wanted)| draw.sides() != wanted.sides)
    {
        return Err(RulesRefusal::DrawsUnfit);
    }
    if draws.len() < wanted.len() {
        return Ok(RollStep::NeedsDraws(
            wanted.into_iter().skip(draws.len()).collect(),
        ));
    }
    let faces: Vec<u16> = draws.iter().map(|draw| draw.value()).collect();
    request
        .resolve(&faces)
        .map(|result| RollStep::Rolled(Box::new(result)))
        .map_err(refused)
}

/// The dice a roll needs, in order. A request `df-rules` refuses writes `RULES refused`.
pub fn prepare_roll(
    action: &ActionId,
    operation: OperationId,
    request: &RollRequest,
) -> Result<Vec<DrawRequest>, PortError> {
    plan(request).map_err(|cause| {
        record(operation, refused_line(action, operation, cause));
        cause.port_error()
    })
}

/// Resolves a roll with the dice drawn for it: the result and a `RULES rolled` line, or the dice
/// still missing.
pub fn resolve_roll(
    action: &ActionId,
    operation: OperationId,
    request: &RollRequest,
    draws: &[Draw],
) -> Result<RollStep, PortError> {
    match step(request, draws) {
        Ok(RollStep::Rolled(result)) => {
            record(operation, rolled_line(action, operation, &result));
            Ok(RollStep::Rolled(result))
        }
        Ok(needs @ RollStep::NeedsDraws(_)) => Ok(needs),
        Err(cause) => {
            record(operation, refused_line(action, operation, cause));
            Err(cause.port_error())
        }
    }
}
