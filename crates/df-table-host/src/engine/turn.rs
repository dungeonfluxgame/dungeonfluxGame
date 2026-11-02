//! One served input, run through the real DM pipeline of C3 over the dev adapters.
//!
//! This is the adapter between the owner and `df_table::pipeline`: it builds the pipeline over the
//! owner's ports for one batch, runs each served input against the batch's copy of the table, and
//! hands the owner what the pipeline staged. The pipeline holds no table state and commits
//! nothing: its events of table changes (`committed_events`) and its views are held in the
//! `TurnResult` until the owner has written the batch's checkpoint, and the owner releases them
//! then and not before (the S1 group commit). It owns no rule of its own: what a line means, what
//! the rules say, which dice are drawn and what an input ends as are the pipeline's.
//!
//! What stays here is small: the pipeline's construction (a typed error, never a panic), the
//! machine and state names a report prints, and the catalog-based list of what a seat can do now
//! (`viable`), which stands in for the action registry of O.5.

use df_table::config::Settings;
use df_table::identity::SeatId;
use df_table::machine::MachineInput;
use df_table::owner_queue::ServedInput;
use df_table::pipeline::{
    Pipeline, PipelineError, PipelineLimits, PipelineTable, Ports, TurnResult,
};
use df_table::ports::{ActionId, EventSink};
use df_table::registry::Registry;

use super::adapters::Adapters;
use super::setup::ActionCatalog;

/// What a turn reads besides the table.
pub(super) struct TurnContext<'a> {
    pub(super) registry: &'a Registry,
    pub(super) catalog: &'a ActionCatalog,
    pub(super) settings: &'a Settings,
    pub(super) limits: &'a PipelineLimits,
    pub(super) adapters: &'a Adapters,
    pub(super) sink: &'a dyn EventSink,
}

impl TurnContext<'_> {
    /// Gives `act` the pipeline over the owner's ports.
    pub(super) fn with_pipeline<R>(
        &self,
        act: impl FnOnce(&Pipeline<'_>) -> R,
    ) -> Result<R, PipelineError> {
        let adapters = self.adapters;
        let ports = Ports {
            rules: &*adapters.rules,
            interpreter: &*adapters.interpreter,
            narrator: &*adapters.narrator,
            broker: &*adapters.broker.hub,
            clock: &*adapters.clock,
            dice: &*adapters.dice,
            knowledge: &*adapters.knowledge,
            sink: self.sink,
        };
        let pipeline =
            Pipeline::new(ports, self.registry, self.settings)?.with_limits(self.limits.clone());
        Ok(act(&pipeline))
    }
}

/// Runs one served input against `table`, in memory. Nothing it did is visible outside `table`
/// until the owner commits and releases the result.
pub(super) fn run_turn(
    ctx: &TurnContext<'_>,
    table: &mut PipelineTable,
    served: &ServedInput,
) -> Result<TurnResult, PipelineError> {
    ctx.with_pipeline(|pipeline| pipeline.run(table, &served.envelope, served.correlation()))
}

/// The names of the machine that runs now and its state, or `none` when no machine runs.
pub(super) fn machine_state(ctx: &TurnContext<'_>, table: &PipelineTable) -> (String, String) {
    let snapshot = table.facts().snapshot();
    let Some(active) = snapshot.focus_machine() else {
        return ("none".to_owned(), "none".to_owned());
    };
    let state = ctx
        .registry
        .machine(active.machine())
        .and_then(|machine| machine.state_name(active.slot()).ok())
        .map_or_else(|| "unknown".to_owned(), |state| state.to_string());
    (active.machine().to_string(), state)
}

/// The actions the machine that runs now would accept from `seat`.
pub(super) fn viable_actions(
    ctx: &TurnContext<'_>,
    table: &PipelineTable,
    seat: SeatId,
) -> Vec<ActionId> {
    let snapshot = table.facts().snapshot();
    let Some(active) = snapshot.focus_machine() else {
        return Vec::new();
    };
    let Some(machine) = ctx.registry.machine(active.machine()) else {
        return Vec::new();
    };
    ctx.catalog
        .actions(active.machine())
        .iter()
        .filter(|action| {
            let input = MachineInput::Intent {
                seat,
                action: (*action).clone(),
            };
            machine.accepts(table.facts(), &input, active.slot())
        })
        .cloned()
        .collect()
}
