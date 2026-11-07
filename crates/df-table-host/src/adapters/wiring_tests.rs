//! The adapters inside the real owner actor: the explore check runs on the production rules owner
//! and the recording dice, and a replay supplies the same dice again.

use std::sync::Arc;

use df_table::config::Settings;
use df_table::outcome::OutcomeKind;
use df_table::ports::{ActionId, Dice, RulesOwner};
use df_table::testkit::{ManualClock, ScriptedInterpreter};

use crate::engine::adapters::{Adapters, BrokerSide, DevControls, DevKnowledge, DevNarrator};
use crate::engine::audit::LogBrokerAudit;
use crate::engine::commit::ScriptedCommitStore;
use crate::engine::handle::EngineHandle;
use crate::engine::report::TurnReport;
use crate::engine::request::{InputBody, InputRequest};
use crate::engine::setup::{EngineConfig, dev_machines};
use crate::event_log::EventLog;

use super::backend::{RulesBackend, dev_ports};
use super::dice::{RecordedDice, ReplayDice};

const SEED: u64 = 31_337;

fn tap(seat: u64, action: &str) -> InputRequest {
    InputRequest {
        input: InputBody::Tap {
            seat,
            action: action.to_owned(),
        },
        operation: None,
        operation_id: None,
        view: None,
        machine: None,
    }
}

fn config() -> EngineConfig {
    let large = serde_json::json!({
        "seat_pending_limit": 1_000,
        "seat_rate_burst": 1_000_000,
        "seat_rate_sustained_per_s": 1_000_000,
    });
    let configuration = large.as_object().cloned().unwrap_or_default();
    EngineConfig {
        settings: Settings::load(&configuration)
            .expect("the settings load")
            .settings,
        ..EngineConfig::dev()
    }
}

fn start_with(rules: Arc<dyn RulesOwner>, dice: Arc<dyn Dice>) -> EngineHandle {
    let clock = Arc::new(ManualClock::default());
    let log = Arc::new(EventLog::default());
    let adapters = Adapters {
        broker: BrokerSide::dev(
            clock.clone(),
            Arc::new(LogBrokerAudit::new(Arc::clone(&log))),
        )
        .expect("the broker builds"),
        clock,
        rules,
        interpreter: Arc::new(ScriptedInterpreter::default()),
        narrator: Arc::new(DevNarrator),
        dice,
        knowledge: Arc::new(DevKnowledge),
        controls: DevControls::default(),
    };
    EngineHandle::start(
        config(),
        adapters,
        Arc::new(ScriptedCommitStore::default()),
        log,
    )
    .expect("the owner starts")
}

fn check_action() -> ActionId {
    dev_machines(EngineConfig::dev().first_scene)
        .expect("the dev machines build")
        .check
}

/// Plays the lobby to the explore machine and makes the one check, returning the check's turn.
async fn play_to_the_check(handle: &EngineHandle) -> TurnReport {
    for (seat, action) in [
        (1, "join"),
        (2, "join"),
        (1, "ready"),
        (2, "ready"),
        (1, "start"),
    ] {
        handle.submit(tap(seat, action)).await.expect("answered");
    }
    let checked = handle
        .submit(tap(1, check_action().as_str()))
        .await
        .expect("answered");
    checked.turns.into_iter().next().expect("the check ran")
}

#[tokio::test]
async fn the_explore_check_resolves_on_the_df_rules_adapter_and_its_d20_is_recorded_for_that_turn()
{
    let rules = dev_ports(RulesBackend::DfRules, &check_action(), SEED)
        .expect("builds")
        .rules;
    let recording = Arc::new(RecordedDice::seeded(SEED));
    let handle = start_with(rules, recording.clone());

    let check = play_to_the_check(&handle).await;

    assert_eq!(check.outcome, OutcomeKind::Applied, "{check:?}");
    assert_eq!(
        (check.state_before.as_str(), check.state_after.as_str()),
        ("presenting", "resolved")
    );
    let records = recording.records();
    assert_eq!(records.len(), 1, "one d20 for the one check");
    assert_eq!(records[0].draw.sides().get(), 20);
    assert_eq!(records[0].correlation.turn_seq.get(), check.turn_seq);
}

#[tokio::test]
async fn a_replay_of_the_recorded_draws_runs_the_same_turns_without_rolling() {
    let rules = dev_ports(RulesBackend::DfRules, &check_action(), SEED)
        .expect("builds")
        .rules;
    let recording = Arc::new(RecordedDice::seeded(SEED));
    let first = start_with(Arc::clone(&rules), recording.clone());
    let first_check = play_to_the_check(&first).await;
    let records = recording.records();
    assert_eq!(records.len(), 1, "one d20 was drawn for the one check");
    assert_eq!(records[0].draw.sides().get(), 20);
    assert_eq!(records[0].correlation.turn_seq.get(), first_check.turn_seq);

    let replay = Arc::new(ReplayDice::new(records));
    let second = start_with(rules, replay.clone());
    let second_check = play_to_the_check(&second).await;

    assert_eq!(second_check.outcome, first_check.outcome);
    assert_eq!(second_check.readout, first_check.readout);
    assert_eq!(
        replay.remaining(),
        0,
        "the replay supplied the recorded draw"
    );
}

#[tokio::test]
async fn the_scripted_backend_still_runs_the_same_check_on_the_h0_fakes() {
    let ports = dev_ports(RulesBackend::Scripted, &check_action(), SEED).expect("builds");
    let handle = start_with(ports.rules, ports.dice);

    let check = play_to_the_check(&handle).await;

    assert_eq!(check.outcome, OutcomeKind::Applied, "{check:?}");
}
