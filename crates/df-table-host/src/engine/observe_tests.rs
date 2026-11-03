//! The event and settings verbs through a running host (the command line, the routes, the guard,
//! on an ephemeral port) and through the owner directly where the test needs a sink or a replayer
//! of its own.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use df_table::testkit::ManualClock;
use tempfile::TempDir;

use crate::command::split_words;
use crate::dev::run_dev;
use crate::event_log::EventLog;
use crate::observe::{FileSpool, HostEventSink, RecordOutlet};
use crate::repl::{LineResult, run_line};
use crate::test_support::{STARTER_FILES, post_request, raw_exchange, start};

use super::adapters::BrokerSide;
use super::broker_fields::SinkBrokerAudit;
use super::commit::ScriptedCommitStore;
use super::extras::Extras;
use super::handle::EngineHandle;
use super::limits::COMMAND_MAILBOX_CAPACITY;
use super::replay::{ReplayRefusal, Replayer};
use super::report::EngineError;
use super::report_observe::ReplayReport;
use super::request::{ClassFilter, ConfigControl, HistoryRequest, ReplayRequest};
use super::routes::path;
use super::setup::EngineStartError;
use super::tests_support::{RigOptions, adapters, config, lines, rig, tap, with_operation};

async fn dev(port: u16, line: &str) -> String {
    let words = split_words(line).expect("the line splits");
    run_dev(port, &words)
        .await
        .unwrap_or_else(|error| panic!("'{line}' failed: {error}"))
}

async fn refused(port: u16, line: &str) -> String {
    let words = split_words(line).expect("the line splits");
    run_dev(port, &words)
        .await
        .expect_err(&format!("'{line}' should be refused"))
        .to_string()
}

const FIVE: &str = "00000000000000000000000000000005";

#[tokio::test]
async fn history_shows_the_newest_events_with_their_number_class_and_line_and_the_streams_health() {
    let host = start(&STARTER_FILES).await;
    dev(host.port, "input 1 --op 5 --tap join").await;

    let all = dev(host.port, "history --limit 1000").await;
    assert!(all.contains("events retained"), "{all}");
    assert!(all.contains("INPUT received table=1"), "{all}");
    assert!(all.contains("TURN applied table=1"), "{all}");
    assert!(all.contains("ENGINE input sender=seat:1"), "{all}");
    assert!(all.contains("event stream:"), "{all}");
    assert!(all.contains("events at level verbose"), "{all}");

    let input = dev(host.port, "history --area input --class a").await;
    assert!(input.contains("INPUT received"), "{input}");
    assert!(!input.contains("TURN applied"), "{input}");
    assert!(!input.contains("STEP finished"), "{input}");

    let verbose = dev(host.port, "history --class b --limit 1000").await;
    assert!(verbose.contains(" B "), "{verbose}");
    assert!(verbose.contains("STEP finished"), "{verbose}");
    assert!(!verbose.contains("TURN applied"), "{verbose}");

    let newest = dev(host.port, "history --limit 2").await;
    assert!(newest.contains("showing the newest 2 of the"), "{newest}");
}

#[tokio::test]
async fn trace_shows_every_event_of_one_input_and_what_the_queue_did_with_it() {
    let host = start(&STARTER_FILES).await;
    dev(host.port, "input 1 --op 5 --tap join").await;
    dev(host.port, "input 2 --op 6 --tap join").await;

    let by_number = dev(host.port, "trace 5").await;
    assert!(
        by_number.contains(&format!("operation {FIVE}: ")),
        "{by_number}"
    );
    for expected in [
        "INPUT received",
        "INPUT served",
        "TURN applied",
        "ENGINE input sender=seat:1",
        "what the queue did:",
        "turn 1 seat:1 tap",
    ] {
        assert!(by_number.contains(expected), "{expected}: {by_number}");
    }
    assert!(
        !by_number.contains("seat:2"),
        "another input's events are not in the trace: {by_number}"
    );
    let by_digits = dev(host.port, &format!("trace {FIVE}")).await;
    assert!(by_digits.contains("INPUT received"), "{by_digits}");

    let unknown = dev(host.port, "trace 99").await;
    assert!(
        unknown.contains("no event of operation 00000000000000000000000000000063 is retained"),
        "{unknown}"
    );
    assert!(
        unknown.contains("the queue remembers nothing of it"),
        "{unknown}"
    );
    let bad = refused(host.port, "trace banana").await;
    assert!(
        bad.contains("an operation is a number or 32 hexadecimal digits"),
        "{bad}"
    );
}

#[tokio::test]
async fn config_lists_every_setting_with_its_spec_and_set_changes_what_the_next_input_uses() {
    let host = start(&STARTER_FILES).await;

    let shown = dev(host.port, "config show").await;
    assert!(
        shown.contains(
            "narration_late_s = 8 (default 8, 1 to 1000000 seconds; launcher_settings; cost effect none; changes the next input)"
        ),
        "{shown}"
    );
    assert!(
        shown.contains("seat_rate_burst = 3")
            && shown.contains("read at start, cannot change while running"),
        "{shown}"
    );
    assert_eq!(
        shown.lines().count(),
        8,
        "one line for each of the eight settings"
    );

    let changed = dev(host.port, "config set held_nudge_s 45").await;
    assert!(
        changed.starts_with("changed held_nudge_s from 30 to 45"),
        "{changed}"
    );
    assert!(changed.contains("held_nudge_s = 45"), "{changed}");
    assert!(
        dev(host.port, "config show")
            .await
            .contains("held_nudge_s = 45")
    );
    let events = dev(host.port, "events").await;
    assert!(
        events.contains("ENGINE config verb=set setting=held_nudge_s value=45 driven=true"),
        "{events}"
    );
}

#[tokio::test]
async fn config_refuses_an_unknown_setting_a_value_outside_its_spec_and_a_setting_read_at_start() {
    let host = start(&STARTER_FILES).await;

    let unknown = refused(host.port, "config set sparkle 1").await;
    assert!(
        unknown.contains("there is no setting 'sparkle'"),
        "{unknown}"
    );
    let range = refused(host.port, "config set held_nudge_s 0").await;
    assert!(
        range.contains("held_nudge_s was refused: out_of_range"),
        "{range}"
    );
    let restart = refused(host.port, "config set seat_rate_burst 5").await;
    assert!(
        restart.contains("read once when the owner starts"),
        "{restart}"
    );
    // narration_late_s may not fall below narration_batch_s (4): the pair is refused whole.
    let related = refused(host.port, "config set narration_late_s 2").await;
    assert!(
        related.contains("inconsistent_with_related_setting"),
        "{related}"
    );

    let shown = dev(host.port, "config show").await;
    assert!(shown.contains("held_nudge_s = 30"), "{shown}");
    assert!(shown.contains("narration_late_s = 8"), "{shown}");
    assert!(shown.contains("seat_rate_burst = 3"), "{shown}");
    let events = dev(host.port, "events").await;
    assert!(
        events.contains("ENGINE config_refused cause=unknown_setting"),
        "{events}"
    );
    assert!(
        events.contains("ENGINE config_refused setting=held_nudge_s cause=out_of_range"),
        "{events}"
    );
    assert!(
        !events.contains("sparkle"),
        "a name from the request never reaches a line: {events}"
    );
}

#[tokio::test]
async fn replay_says_it_is_not_built_yet_and_the_route_answers_501_with_the_typed_error() {
    let host = start(&STARTER_FILES).await;

    let answer = refused(host.port, "replay session --until 3 --diff").await;
    assert!(answer.contains("replay is not built yet"), "{answer}");
    assert!(answer.contains("C7's replay"), "{answer}");

    let request = post_request(path::REPLAY, r#"{"log":"session"}"#, "");
    let raw = raw_exchange(host.port, &request).await;
    assert!(raw.starts_with("HTTP/1.1 501"), "{raw}");
    assert!(raw.contains("\"error\":\"not_supported_yet\""), "{raw}");
    let events = dev(host.port, "events").await;
    assert!(
        events.contains("ENGINE replay_refused cause=not_supported_yet"),
        "{events}"
    );
}

#[tokio::test]
async fn describe_lists_the_new_verbs_and_the_h16_verbs_that_are_not_built() {
    let host = start(&STARTER_FILES).await;

    let described = dev(host.port, "describe").await;

    assert!(described.contains("history [--limit N]"), "{described}");
    assert!(described.contains("trace <operation>"), "{described}");
    assert!(
        described.contains("config show | config set"),
        "{described}"
    );
    assert!(
        described.contains("H.16 verbs not built yet:"),
        "{described}"
    );
    assert!(
        described.contains("the verb set of C7's inspect, and what this host has of it:"),
        "{described}"
    );
    assert!(
        described.contains("trace (reads, operator): built"),
        "{described}"
    );
    assert!(
        described.contains("replay (reads, operator): seam only"),
        "{described}"
    );
    assert!(
        described.contains("sim run (drives, operator): not built"),
        "{described}"
    );
    assert!(
        described.contains("sim run (waits for: the sim runner (H.17))"),
        "{described}"
    );
}

#[tokio::test]
async fn health_shows_the_event_stream_with_its_lag_and_its_loss() {
    let host = start(&STARTER_FILES).await;
    dev(host.port, "input 1 --tap join").await;

    let health = dev(host.port, "health").await;

    assert!(health.contains("4: events at level verbose"), "{health}");
    assert!(health.contains("ingestion lag 0 ms"), "{health}");
    assert!(health.contains("0 class A and 0 class B lost"), "{health}");
    assert!(health.contains("complete so far"), "{health}");
    assert!(health.contains("5: outlet memory:"), "{health}");
}

#[tokio::test]
async fn a_bad_request_to_the_new_routes_is_a_typed_400() {
    let host = start(&STARTER_FILES).await;

    for (verb, address) in [
        ("GET", format!("{}?limit=many", path::HISTORY)),
        ("GET", path::TRACE.to_owned()),
        ("GET", format!("{}?operation=zero", path::TRACE)),
    ] {
        let request =
            format!("{verb} {address} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
        let raw = raw_exchange(host.port, &request).await;
        assert!(raw.starts_with("HTTP/1.1 400"), "{address}: {raw}");
    }
    let bad_body = post_request(
        path::CONFIG,
        r#"{"control":"set","name":"held_nudge_s"}"#,
        "",
    );
    assert!(
        raw_exchange(host.port, &bad_body)
            .await
            .starts_with("HTTP/1.1 400")
    );
}

// The owner directly, with a sink and a replayer of the test's own.

fn extras_over(log: &Arc<EventLog>, replayer: Arc<dyn Replayer>) -> Extras {
    let sink = HostEventSink::new(Arc::clone(log), Arc::new(ManualClock::starting_at(1_000)));
    Extras::new(Arc::new(sink), replayer)
}

fn start_with(extras: Extras, log: &Arc<EventLog>) -> EngineHandle {
    let clock = Arc::new(ManualClock::default());
    EngineHandle::start_with_extras(
        config(&RigOptions::default()),
        adapters(clock, log),
        Arc::new(ScriptedCommitStore::scripted(Vec::new())),
        extras,
        COMMAND_MAILBOX_CAPACITY,
    )
    .expect("the owner starts")
}

#[derive(Default)]
struct RecordingReplayer {
    asked: Mutex<Vec<ReplayRequest>>,
}

impl Replayer for RecordingReplayer {
    fn replay(&self, request: &ReplayRequest) -> Result<ReplayReport, ReplayRefusal> {
        self.asked.lock().expect("the lock").push(request.clone());
        if request.log != "demo" {
            return Err(ReplayRefusal::UnknownLog {
                log: request.log.clone(),
            });
        }
        Ok(ReplayReport {
            log: request.log.clone(),
            turns_replayed: 3,
            final_state_hash: "00000000000000ab".to_owned(),
            matches_recorded: true,
            first_difference_turn: None,
            differences: Vec::new(),
        })
    }
}

#[tokio::test]
async fn a_replayer_behind_the_seam_is_called_with_the_request_and_audited() {
    let log = Arc::new(EventLog::default());
    let replayer = Arc::new(RecordingReplayer::default());
    let handle = start_with(extras_over(&log, replayer.clone()), &log);

    let request = ReplayRequest {
        log: "demo".to_owned(),
        until: Some(3),
        diff: true,
    };
    let report = handle.replay(request.clone()).await.expect("a replay");
    assert_eq!(report.turns_replayed, 3);
    assert_eq!(*replayer.asked.lock().expect("the lock"), vec![request]);
    assert!(
        lines(&log)
            .iter()
            .any(|line| line == "ENGINE replay turns=3")
    );

    let unknown = handle
        .replay(ReplayRequest {
            log: "nope".to_owned(),
            until: None,
            diff: false,
        })
        .await
        .expect_err("no such log");
    assert_eq!(
        unknown,
        EngineError::UnknownLog {
            log: "nope".to_owned()
        }
    );
    assert!(
        lines(&log)
            .iter()
            .any(|line| line == "ENGINE replay_refused cause=unknown_log")
    );

    let blank = handle
        .replay(ReplayRequest {
            log: "  ".to_owned(),
            until: None,
            diff: false,
        })
        .await
        .expect_err("a replay names its log");
    assert!(matches!(blank, EngineError::BadRequest { .. }), "{blank:?}");
    assert_eq!(
        replayer.asked.lock().expect("the lock").len(),
        2,
        "a blank name never reaches it"
    );
}

#[tokio::test]
async fn the_line_log_and_the_history_are_made_from_the_same_events_and_correlate_by_operation() {
    let rig = rig();
    rig.handle
        .submit(with_operation(tap(1, "join"), 5))
        .await
        .expect("an input");

    let history = rig
        .handle
        .history(HistoryRequest {
            limit: Some(10_000),
            ..HistoryRequest::default()
        })
        .await
        .expect("a history");
    let logged = lines(&rig.log);
    assert!(!history.events.is_empty());
    for event in &history.events {
        assert!(
            logged.contains(&event.line),
            "the record's line is a line of the log: {}",
            event.line
        );
    }
    let ids: Vec<u64> = history.events.iter().map(|event| event.audit_id).collect();
    assert!(
        ids.windows(2).all(|pair| pair[1] == pair[0] + 1),
        "ids run with no gap: {ids:?}"
    );

    let trace = rig.handle.trace("5".to_owned()).await.expect("a trace");
    assert!(trace.events.len() >= 3, "{:?}", trace.events);
    assert!(
        trace
            .events
            .iter()
            .all(|event| event.operation.as_deref() == Some(FIVE))
    );
    assert_eq!(trace.turns.len(), 1);
    assert_eq!(trace.submissions.len(), 1);
    assert!(trace.events.iter().any(|event| event.turn_seq == Some(1)));
}

#[tokio::test]
async fn a_class_filter_and_a_limit_apply_to_the_history_and_the_stream_health_comes_with_it() {
    let rig = rig();
    rig.handle.submit(tap(1, "join")).await.expect("an input");

    let verbose = rig
        .handle
        .history(HistoryRequest {
            class: Some(ClassFilter::Verbose),
            limit: Some(5),
            area: None,
        })
        .await
        .expect("a history");
    assert!(verbose.events.iter().all(|event| event.class == 'B'));
    assert!(verbose.matched >= verbose.events.len());
    assert!(verbose.stream.seen >= verbose.stream.verbose);
    assert!(!verbose.stream.degraded);
    let health = rig.handle.health().await.expect("a health");
    assert_eq!(health.events.outlets[0].name, "memory");
}

#[tokio::test]
async fn an_input_reaches_a_file_spool_as_json_lines_for_both_the_table_and_the_engine() {
    let directory = TempDir::new().expect("a directory");
    let path = directory.path().join("events.jsonl");
    let spool = Arc::new(FileSpool::open(&path).expect("a spool"));
    let log = Arc::new(EventLog::default());
    let sink = HostEventSink::new(Arc::clone(&log), Arc::new(ManualClock::starting_at(1_000)))
        .with_outlet(Arc::clone(&spool) as Arc<dyn RecordOutlet>);
    let extras = Extras::new(Arc::new(sink), Arc::new(super::replay::PendingReplay));
    let handle = start_with(extras, &log);

    handle
        .submit(with_operation(tap(1, "join"), 5))
        .await
        .expect("an input");
    assert!(spool.settle(Duration::from_secs(10)));

    let text = std::fs::read_to_string(&path).expect("the spool file");
    let records: Vec<serde_json::Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).expect("JSON"))
        .collect();
    let areas: Vec<&str> = records
        .iter()
        .filter_map(|record| record["area"].as_str())
        .collect();
    for expected in ["INPUT", "TURN", "ENGINE", "COMMIT"] {
        assert!(
            areas.contains(&expected),
            "{expected} is not in the spool: {areas:?}"
        );
    }
    let one_input = records
        .iter()
        .filter(|record| record["correlation"]["operation"] == FIVE)
        .count();
    assert!(
        one_input >= 3,
        "the input's records carry its operation: {one_input}"
    );
    let health = handle.health().await.expect("a health");
    let spooled = &health.events.outlets[1];
    assert_eq!(spooled.name, "spool");
    assert_eq!(spooled.lost_always, 0);
    assert_eq!(spooled.accepted, spooled.confirmed + spooled.pending);
}

#[test]
fn a_spool_that_cannot_be_opened_stops_the_owner_from_starting_with_a_typed_error() {
    let directory = TempDir::new().expect("a directory");
    let in_the_way = directory.path().join("file");
    std::fs::write(&in_the_way, b"x").expect("a file");

    let result = Extras::with_spool_directory(Arc::new(EventLog::default()), Some(in_the_way));

    match result {
        Err(EngineStartError::Spool { reason }) => assert!(!reason.is_empty()),
        Err(other) => panic!("a different error: {other}"),
        Ok(_) => panic!("the spool should not open"),
    }
}

#[tokio::test]
async fn config_through_the_owner_reports_the_change_and_keeps_the_rest() {
    let rig = rig();

    let report = rig
        .handle
        .config(ConfigControl::Set {
            name: "ruling_nudge_s".to_owned(),
            value: 25,
        })
        .await
        .expect("a change");

    let change = report.changed.expect("the change is reported");
    assert_eq!(
        (change.name.as_str(), change.from, change.to),
        ("ruling_nudge_s", 20, 25)
    );
    let value_of = |name: &str| {
        report
            .settings
            .iter()
            .find(|s| s.name == name)
            .map(|s| s.value)
    };
    assert_eq!(value_of("ruling_nudge_s"), Some(25));
    assert_eq!(value_of("held_nudge_s"), Some(30));
}

#[tokio::test]
async fn a_broker_request_is_a_record_with_its_operation_and_trace_finds_the_brokers_side_of_it() {
    let log = Arc::new(EventLog::default());
    let extras = extras_over(&log, Arc::new(super::replay::PendingReplay));
    let clock = Arc::new(ManualClock::default());
    let mut wired = adapters(clock.clone(), &log);
    wired.broker = BrokerSide::dev(
        clock,
        Arc::new(SinkBrokerAudit::new(Arc::clone(&extras.sink))),
    )
    .expect("the broker builds");
    let handle = EngineHandle::start_with_extras(
        config(&RigOptions::default()),
        wired,
        Arc::new(ScriptedCommitStore::scripted(Vec::new())),
        extras,
        COMMAND_MAILBOX_CAPACITY,
    )
    .expect("the owner starts");

    handle
        .broker_request("plate".to_owned())
        .await
        .expect("a request");

    let history = handle
        .history(HistoryRequest {
            area: Some("RESOURCE".to_owned()),
            limit: Some(100),
            class: Some(ClassFilter::Always),
        })
        .await
        .expect("a history");
    let requested = history
        .events
        .iter()
        .find(|event| event.line.starts_with("RESOURCE request purpose=plate"))
        .expect("the request is recorded");
    let operation = requested
        .operation
        .clone()
        .expect("a request names its operation");
    assert!(
        requested
            .line
            .contains(&format!("operation_id={operation}"))
    );
    assert!(
        lines(&log).contains(&requested.line),
        "the same line is in the log"
    );
    let trace = handle.trace(operation).await.expect("a trace");
    assert!(
        trace
            .events
            .iter()
            .any(|event| event.line == requested.line),
        "trace joins the broker's request to its operation"
    );
}

#[tokio::test]
async fn a_script_and_the_repl_run_the_event_and_settings_verbs_like_any_other_command() {
    let host = start(&STARTER_FILES).await;
    let directory = TempDir::new().expect("a directory");
    let script = directory.path().join("observe.txt");
    std::fs::write(
        &script,
        "input 1 --op 5 --tap join
trace 5
config set held_nudge_s 40
config show
history --limit 3
",
    )
    .expect("writes the script");

    let ran = dev(host.port, &format!("script {}", script.display())).await;
    assert!(ran.contains("ran 5 commands, all ok"), "{ran}");

    let LineResult::Ok(text) = run_line(host.port, "trace 5").await else {
        panic!("the repl runs trace");
    };
    assert!(text.contains("INPUT received"), "{text}");
    assert!(matches!(
        run_line(host.port, "history --limit").await,
        LineResult::Failed(message) if message.starts_with("usage: history")
    ));
    assert!(matches!(
        run_line(host.port, "config set held_nudge_s 0").await,
        LineResult::Failed(message) if message.contains("out_of_range")
    ));
}
