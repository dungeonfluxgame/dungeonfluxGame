//! Events for the sink's tests: one of each shape the sink treats differently.

use df_table::element::{ElementId, KindName};
use df_table::event::{FailureCause, PipelineStep, TableEvent};
use df_table::identity::{MachineId, SceneId, TableId, TableScope};
use df_table::pipeline::events::PipelineEvent;
use df_table::ports::PortName;
use df_table::private_data::LengthClass;
use df_table::testkit::correlation;

pub fn scope() -> TableScope {
    TableScope::of_table(TableId::new(1))
        .in_scene(SceneId::new(2))
        .in_machine(MachineId::new("explore").expect("a valid machine"))
}

/// Class A, correlated: an input arrived as the `turn`th of the run.
pub fn received(turn: u64) -> TableEvent {
    TableEvent::InputReceived {
        scope: scope(),
        correlation: correlation(turn),
        length: LengthClass::Short,
        evidence: None,
    }
}

/// Class B, correlated: a stage finished.
pub fn step_finished(turn: u64) -> TableEvent {
    TableEvent::StepFinished {
        scope: scope(),
        correlation: correlation(turn),
        step: PipelineStep::Check,
        outcome: None,
        ms: 3,
    }
}

/// Class A, not correlated: a canvas change.
pub fn spawned() -> TableEvent {
    TableEvent::ElementSpawned {
        element: ElementId::new("pip").expect("a valid element"),
        kind: KindName::new("character").expect("a valid kind"),
        revision: 4,
    }
}

/// Class A, correlated, from a nested family.
pub fn dice_drawn(turn: u64) -> TableEvent {
    TableEvent::Pipeline(PipelineEvent::DiceDrawn {
        scope: scope(),
        correlation: correlation(turn),
        draw: 0,
        sides: 20,
        value: 12,
    })
}

/// Class A, an error: a port could not answer.
pub fn boundary_failed(turn: u64) -> TableEvent {
    TableEvent::BoundaryFailed {
        scope: scope(),
        correlation: correlation(turn),
        port: PortName::Dice,
        cause: FailureCause::Unavailable,
        ms: 9,
    }
}

/// The operation id of `correlation(turn)` as 32 hexadecimal digits.
pub fn operation_of(turn: u64) -> String {
    correlation(turn).operation_id.to_string()
}
