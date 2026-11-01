//! Transaction-taking statements for the durable image execution journal
//! (migration 0040), split by responsibility: shared locks and parsing in
//! `common`, enqueue plus operator bounds and gates in `enqueue`, claims,
//! queue scan and release in `claim`, dispatch send/outcome work in
//! `dispatch`, refetch selection in `cursor`, identity resolution in
//! `resolve`, and plain reads in `reads`.
//!
//! Every function takes the caller's transaction and performs one atomic
//! call at READ COMMITTED; row locks carry the guarantees. No statement
//! sleeps, retries, spends money, or touches finance, catalog or provider
//! state on its own.
//!
//! Lock the job first to serialize stage, dispatch, cursor and checkpoint changes.
//! Gate checks retain a shared gate lock until commit; operator updates take the
//! gate before capacity bounds. Finance, fence and quota/catalog locks precede
//! imagery locks. Failed commits are Unknown and never authorize a resend.

pub(crate) mod claim;
pub(crate) mod common;
pub(crate) mod cursor;
pub(crate) mod dispatch;
pub(crate) mod enqueue;
pub(crate) mod reads;
pub(crate) mod resolve;

pub(crate) use claim::{claim_stage_in, release_stage_in, scan_queue_in};
pub(crate) use common::record_execution_failure;
pub(crate) use cursor::request_refetch_in;
pub(crate) use dispatch::{
    cancel_dispatch_in, complete_dispatch_in, mark_sent_in, mirror_settlement_in,
    prepare_dispatch_in, read_output_checkpoint_in, read_supplier_checkpoint_in,
    record_supplier_checkpoint_in, record_supplier_in,
};
pub(crate) use enqueue::{
    advance_execution_gate_in, enqueue_stage_in, read_execution_gate_in, set_capacity_bounds_in,
    set_execution_gate_in,
};
pub(crate) use reads::{
    read_capacity_bounds_in, read_child_stage_in, read_current_dispatch_in, read_dispatch_in,
    read_stage_in,
};
pub(crate) use resolve::resolve_execution_in;

mod recovery;
pub(crate) use recovery::{
    claim_recovery_in, finish_recovery_in, scan_recovery_in, validate_recovery_in,
};

mod decision;
pub(crate) use decision::{
    read_decision_checkpoint_in, read_decision_observation_in, read_decision_reference_in,
    read_decision_reference_unlocked_in, read_stage_basis_in, record_decision_observation_in,
};

mod candidate;
pub(crate) use candidate::{
    read_candidate_checkpoint_in, read_candidate_observation_in, read_candidate_reference_in,
    record_candidate_observation_in,
};

mod reconciliation;
pub(crate) use reconciliation::prepare_reconciliation_in;

mod decline;
mod failure;
pub(crate) use decline::{
    read_decline_reference_in, record_decline_observation_in, requeue_declined_in,
};

mod scheduling;
pub(crate) use scheduling::record_urgency_in;

mod sent;
pub(crate) use sent::{mark_hosted_sent_in, mark_local_sent_in};

mod deferral;
pub(crate) use deferral::{defer_queued_in, has_stage_finance};

mod local_inspection;
pub(crate) use local_inspection::{read_local_inspection_in, record_local_inspection_in};

mod terminal;
pub(crate) use terminal::{check_output_open_in, close_terminal_in, validate_terminal_tail_in};

mod unsent_projection;
pub(crate) use unsent_projection::acknowledge_unsent_projection_in;

mod unsent;
pub(crate) use unsent::scan_unsent_prepared_in;

mod queue;
pub(crate) use queue::scan_queue_after_in;

mod unclaimed;
pub(crate) use unclaimed::scan_unsent_terminal_in;
