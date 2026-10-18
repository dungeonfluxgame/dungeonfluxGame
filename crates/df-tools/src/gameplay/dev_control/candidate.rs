//! Development candidate checkpoints: restores and overrides built from the
//! current checkpoint without the engine.
//!
//! Every builder takes the owner's current checkpoint, produces exactly one
//! new decision for a fresh operation at exactly the next revision, and keeps
//! the history chains (decisions, facts, draws, intents) extended — the shape
//! the owner's `validate_candidate` gate demands. Anything else (session,
//! run, pins, mode) is carried over untouched, so a dev commit is
//! indistinguishable in form from a player commit; its provenance lives in the
//! new decision's `semantic_output`. Provider work is never staged. Battle orders retain their actual rules draws and the
//! canonical DrawAccepted facts required by the ordinary owner accounting gate.

use df_model::checkpoint::{
    AcceptedDecision, Basis, CHECKPOINT_SCHEMA, Checkpoint, CheckpointLimits, GameState,
    ReferenceInventory,
};
use df_types::{MemberId, OperationId};

/// A built candidate plus its human before/after for the table log, the
/// override log and the JSON answer.
pub(crate) struct Built {
    pub candidate: Checkpoint,
    pub before: String,
    pub after: String,
}

/// What one dev commit changes. Restores bring back a saved moment's full state;
/// overrides mutate one field of the current content.
pub(crate) enum Change {
    /// Bring back a saved game state (history, named snapshot or redo entry):
    /// the saved moment's FULL state — content and chains alike — plus the new
    /// dev decision. Never old content mixed with newer chains: that mixture
    /// commits but fails every view projection (devctl-a1, rev 18 refutation).
    Restore {
        saved: Box<Checkpoint>,
        what: String,
    },
    /// Set a hero's hit points, validated against their resource bounds.
    HitPoints { member: MemberId, value: i64 },
    /// Set the narrative (provider) budget, validated against the pack cap.
    Budget { value: u64, cap: u64 },
    /// One NPC fight turn already resolved by the rules: its state, draws and receipt.
    NpcTurn {
        turn: Box<super::super::journey::tactical::StagedNpcTurn>,
    },
}

/// Builds the candidate for one dev commit. `inventory`/`limits` are the
/// run's admitted references (cloned at startup); `policy` is the current
/// last decision's source policy so the new decision stays admitted.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build(
    current: &Checkpoint,
    change: &Change,
    next: df_types::SessionRevision,
    operation: OperationId,
    policy: df_types::RevisionLabel,
    note: String,
    inventory: ReferenceInventory<'_>,
    limits: CheckpointLimits,
) -> Result<Built, String> {
    if let Change::NpcTurn { turn } = change
        && turn.ordinary
    {
        let candidate =
            super::super::journey::tactical::ordinary_npc_candidate(current, turn, next, operation)
                .map_err(|error| format!("ordinary NPC candidate: {error:?}"))?;
        return Ok(Built {
            candidate,
            before: turn.before.clone(),
            after: turn.after.clone(),
        });
    }
    let mut state = match change {
        Change::Restore { saved, .. } => restore_state(current, saved)?,
        Change::NpcTurn { turn } => turn.state.clone(),
        _ => current.state().clone(),
    };
    let (before, after) = match change {
        Change::Restore { what, .. } => (describe_revision(current), format!("restored {what}")),
        Change::HitPoints { member, value } => apply_hit_points(&mut state, *member, *value)?,
        Change::Budget { value, cap } => apply_budget(&mut state, *value, *cap)?,
        Change::NpcTurn { turn } => (turn.before.clone(), turn.after.clone()),
    };
    let decision = match change {
        Change::NpcTurn { turn } => {
            // A fight turn is a thread decision: its receipt carries the combat outcomes the
            // view reads, and its draws are the ones the rules consumed.
            let facts = super::super::journey::tactical::development_draw_facts(
                current,
                &mut state,
                next,
                operation,
                &turn.draws,
            )
            .map_err(|error| format!("development draw accounting: {error:?}"))?;
            state.draws.extend(turn.draws.iter().cloned());
            AcceptedDecision {
                operation,
                revision: next,
                facts,
                draws: turn.draws.iter().map(|draw| draw.ordinal).collect(),
                effects: Vec::new(),
                source_policy: super::super::model::rule()
                    .map_err(|_| "development policy unavailable")?
                    .catalog,
                semantic_output: Some(
                    super::super::journey::tactical::development_encoded(
                        current,
                        &{
                            use prost::Message;
                            df_protocol::common::AcceptedAction::decode(
                                super::super::journey::unhex(&turn.receipt)
                                    .map_err(|_| "development receipt invalid")?
                                    .as_slice(),
                            )
                            .map_err(|_| "development receipt invalid")?
                        },
                        &turn.after,
                        turn.restoration.as_ref(),
                    )
                    .map_err(|error| format!("development receipt: {error:?}"))?,
                ),
            }
        }
        Change::Budget { .. } => AcceptedDecision {
            operation,
            revision: next,
            facts: Vec::new(),
            draws: Vec::new(),
            effects: Vec::new(),
            source_policy: super::super::model::rule()
                .map_err(|_| "development policy unavailable")?
                .catalog,
            semantic_output: Some(
                super::super::journey::tactical::development_encoded(
                    current,
                    &df_protocol::common::AcceptedAction {
                        phase: super::super::journey::phase(current)
                            .map_err(|_| "development phase unavailable")?
                            as i32,
                        ..Default::default()
                    },
                    &note,
                    None,
                )
                .map_err(|error| format!("development budget receipt: {error:?}"))?,
            ),
        },
        _ => AcceptedDecision {
            operation,
            revision: next,
            facts: Vec::new(),
            draws: Vec::new(),
            effects: Vec::new(),
            source_policy: policy,
            semantic_output: Some(note),
        },
    };
    state.decisions.push(decision);
    let basis = Basis {
        session: current.basis().session,
        run: current.basis().run,
        revision: next,
    };
    let resources = super::super::journey::resources_for(&state)
        .map_err(|error| format!("development resource bounds: {error:?}"))?;
    let inventory = ReferenceInventory {
        resources: &resources,
        ..inventory
    };
    let candidate = Checkpoint::new(
        CHECKPOINT_SCHEMA,
        basis,
        current.pins().clone(),
        state,
        inventory,
        limits,
    )
    .map_err(|error| format!("dev candidate rejected by checkpoint validation: {error:?}"))?;
    Ok(Built {
        candidate,
        before,
        after,
    })
}

/// The restored game state: the saved state's everything — content AND chains
/// (decisions, facts, draws truncated to the saved point). The single new dev
/// decision is appended by [`build`]; the owner rewind validation then checks
/// the restored chains stand in a prefix relationship with the current ones.
/// Outstanding async provider work is not history, so the current intents are
/// kept: dropping them orphans completions and restaging them repeats provider
/// calls. The table log and the social sidecar travel beside the checkpoint
/// (see [`super::history::SavedMoment`]) and are rewound on the owner thread.
fn restore_state(current: &Checkpoint, saved: &Checkpoint) -> Result<GameState, String> {
    if saved.basis().session != current.basis().session || saved.basis().run != current.basis().run
    {
        return Err("saved checkpoint is from a different session or run".to_owned());
    }
    let mut state = saved.state().clone();
    state.intents = current.state().intents.clone();
    Ok(state)
}

fn describe_revision(current: &Checkpoint) -> String {
    let revision = current.basis().revision;
    format!(
        "revision {}:{}",
        revision.epoch().get(),
        revision.sequence()
    )
}

fn hero_entity(
    state: &GameState,
    member: MemberId,
) -> Result<df_model::checkpoint::EntityId, String> {
    state
        .characters
        .iter()
        .find(|character| character.owner == member)
        .map(|character| character.entity)
        .ok_or_else(|| "this seat has no hero yet; create one first".to_owned())
}

fn apply_hit_points(
    state: &mut GameState,
    member: MemberId,
    value: i64,
) -> Result<(String, String), String> {
    let entity = hero_entity(state, member)?;
    let hit_points = super::super::model::label("hit-points").map_err(|_| "label unavailable")?;
    let resource = state
        .resources
        .iter_mut()
        .find(|resource| resource.owner == entity && resource.resource == hit_points)
        .ok_or_else(|| "hero has no hit-point resource".to_owned())?;
    if value < resource.minimum || value > resource.maximum {
        return Err(format!(
            "hit points must be {}..={} (maximum {})",
            resource.minimum, resource.maximum, resource.maximum
        ));
    }
    let before = format!("{} HP", resource.value);
    resource.value = value;
    Ok((before, format!("{value} HP")))
}

fn apply_budget(state: &mut GameState, value: u64, cap: u64) -> Result<(String, String), String> {
    if value > cap {
        return Err(format!("budget must be 0..={cap} (pack cap)"));
    }
    let before = format!("budget {}", state.narrative.remaining_budget);
    state.narrative.remaining_budget = value;
    Ok((before, format!("budget {value}")))
}
