//! The dev engine through a running host: the routes, the guard, the command line, the repl and a
//! script, on an ephemeral port.

use axum::http::StatusCode;

use crate::client;
use crate::command::split_words;
use crate::dev::run_dev;
use crate::dev_http::BodyCause;
use crate::repl::{LineResult, run_line};
use crate::test_support::{STARTER_FILES, post_request, raw_exchange, start};
use crate::wire::EventsReply;

use super::report::{EngineDump, EngineError, EngineReply};
use super::routes::path;

async fn dev(port: u16, line: &str) -> String {
    let words = split_words(line).expect("the line splits");
    run_dev(port, &words)
        .await
        .unwrap_or_else(|error| panic!("'{line}' failed: {error}"))
}

#[tokio::test]
async fn an_input_from_the_command_line_reaches_the_machine_and_the_projection_shows_it() {
    let host = start(&STARTER_FILES).await;

    let answer = dev(host.port, "input 1 --tap join").await;
    assert!(answer.starts_with("1: "), "{answer}");
    assert!(answer.contains("admitted to the player lane"), "{answer}");
    assert!(answer.contains("2: turn 1 seat:1 tap"), "{answer}");
    assert!(
        answer.contains(
            "applied action=join, lobby:gathering -> lobby:gathering, 1 effects, committed as 1"
        ),
        "{answer}"
    );

    let tv = dev(host.port, "projection show tv").await;
    let view: serde_json::Value = serde_json::from_str(&tv).expect("a view");
    assert_eq!(view["viewer"]["kind"], "shared");
    assert_eq!(view["machine_id"], "lobby");
    let dump: EngineDump = serde_json::from_str(&dev(host.port, "dump").await).expect("a dump");
    assert_eq!((dump.machine.as_str(), dump.commit), ("lobby", 1));
    let viable = dev(host.port, "viable 1").await;
    assert!(
        viable.contains("seat 1 can do, in lobby (gathering):"),
        "{viable}"
    );
    assert!(viable.contains(": ready"), "{viable}");
}

#[tokio::test]
async fn the_numbered_answers_name_duplicates_and_rate_limits_and_queue_show_lists_them() {
    let host = start(&STARTER_FILES).await;
    dev(host.port, "input 1 --op 5 --tap join").await;

    let duplicate = dev(host.port, "input 1 --op 5 --tap join").await;
    assert!(duplicate.contains("refused (duplicate)"), "{duplicate}");
    assert!(
        duplicate.contains("the first copy is finished"),
        "{duplicate}"
    );

    let shown = dev(host.port, "queue show").await;
    assert!(shown.contains("serving: running"), "{shown}");
    assert!(
        shown.contains("lanes, in the order they are served:\n1: seat_controls 0\n2: engine 0"),
        "{shown}"
    );
    assert!(shown.contains("refused (duplicate)"), "{shown}");
    assert!(shown.contains("commit: per_batch policy"), "{shown}");
    assert!(shown.contains("turn 1 seat:1 tap"), "{shown}");
}

#[tokio::test]
async fn hold_admits_without_serving_and_step_serves_in_lane_order() {
    let host = start(&STARTER_FILES).await;
    dev(host.port, "queue hold").await;
    dev(host.port, "input 2 --tap join").await;
    dev(host.port, "input engine --timer nudge").await;
    dev(host.port, "input 3 --pause").await;

    let waiting = dev(host.port, "queue show").await;
    assert!(waiting.contains("serving: held"), "{waiting}");
    assert!(waiting.contains("1: seat_controls 1"), "{waiting}");

    let served = dev(host.port, "queue step 3").await;
    let order: Vec<&str> = served
        .lines()
        .map(|line| {
            ["seat_controls", "timers_and_results", "player"]
                .into_iter()
                .find(|lane| line.contains(&format!("from the {lane} lane")))
                .unwrap_or("?")
        })
        .collect();
    assert_eq!(
        order,
        ["seat_controls", "timers_and_results", "player"],
        "{served}"
    );
}

#[tokio::test]
async fn describe_lists_the_registry_and_force_moves_the_table() {
    let host = start(&STARTER_FILES).await;

    let described = dev(host.port, "describe").await;
    // The host runs the first playable path: the lobby, the story scene and the conversation.
    assert!(
        described.contains("1: dialogue (group, enabled)"),
        "{described}"
    );
    assert!(
        described.contains("2: explore (group, enabled)"),
        "{described}"
    );
    assert!(
        described.contains("3: lobby (table, enabled, active)"),
        "{described}"
    );
    assert!(
        described.contains("lanes, in the order they are served: seat_controls, engine"),
        "{described}"
    );
    assert!(
        described.contains("projection show <tv|seat-number>"),
        "{described}"
    );

    let forced = dev(host.port, "force lobby started phase=started scene=1").await;
    assert!(
        forced.contains("the table is now in lobby (started) at revision 1"),
        "{forced}"
    );
    let words = split_words("force explore presenting").expect("splits");
    let refused = run_dev(host.port, &words).await.expect_err("no scene yet");
    assert!(
        refused.to_string().contains("the change was refused"),
        "{refused}"
    );
}

#[tokio::test]
async fn a_script_and_the_repl_run_engine_verbs_like_any_other_command() {
    let host = start(&STARTER_FILES).await;
    let directory = tempfile::TempDir::new().expect("a directory");
    let script = directory.path().join("lobby.txt");
    std::fs::write(
        &script,
        "# seat one joins and is ready\ninput 1 --tap join\ninput 1 --tap ready\nviable 1\n",
    )
    .expect("writes the script");

    let ran = dev(host.port, &format!("script {}", script.display())).await;
    assert!(ran.contains("ran 3 commands, all ok"), "{ran}");

    let LineResult::Ok(text) = run_line(host.port, "dump").await else {
        panic!("the repl runs dump");
    };
    assert!(text.contains("\"commit\": 2"), "{text}");
    assert!(matches!(
        run_line(host.port, "input 1").await,
        LineResult::Failed(message) if message.starts_with("usage: input")
    ));
}

#[tokio::test]
async fn a_typo_on_a_late_script_line_runs_nothing() {
    let host = start(&STARTER_FILES).await;
    let directory = tempfile::TempDir::new().expect("a directory");
    let script = directory.path().join("typo.txt");
    std::fs::write(&script, "input 1 --tap join\nqueue sideways\n").expect("writes the script");
    let words = split_words(&format!("script {}", script.display())).expect("splits");

    let error = run_dev(host.port, &words).await.expect_err("refused");

    assert!(error.to_string().contains("usage: queue"), "{error}");
    let dump: EngineDump = serde_json::from_str(&dev(host.port, "dump").await).expect("a dump");
    assert_eq!(dump.commit, 0);
}

#[tokio::test]
async fn the_routes_are_behind_the_hosts_guard() {
    let host = start(&STARTER_FILES).await;
    let foreign = raw_exchange(
        host.port,
        &format!(
            "GET {} HTTP/1.1\r\nHost: example.com\r\nConnection: close\r\n\r\n",
            path::DUMP
        ),
    )
    .await;
    assert!(foreign.starts_with("HTTP/1.1 403"), "{foreign}");

    let not_json = raw_exchange(
        host.port,
        &format!(
            "POST {} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: text/plain\r\nContent-Length: \
             2\r\nConnection: close\r\n\r\n{{}}",
            path::INPUT
        ),
    )
    .await;
    assert!(not_json.starts_with("HTTP/1.1 415"), "{not_json}");

    let fetched_cross_site = raw_exchange(
        host.port,
        &post_request(
            path::INPUT,
            r#"{"input":{"kind":"pause","seat":1}}"#,
            "Sec-Fetch-Site: cross-site\r\n",
        ),
    )
    .await;
    assert!(
        fetched_cross_site.starts_with("HTTP/1.1 403"),
        "{fetched_cross_site}"
    );

    for loopback in ["localhost", "[::1]", "127.0.0.1"] {
        let allowed = raw_exchange(
            host.port,
            &format!(
                "GET {} HTTP/1.1\r\nHost: {loopback}:{}\r\nConnection: close\r\n\r\n",
                path::DUMP,
                host.port
            ),
        )
        .await;
        assert!(allowed.starts_with("HTTP/1.1 200"), "{loopback}: {allowed}");
    }

    let cross_site = raw_exchange(
        host.port,
        &post_request(
            path::INPUT,
            r#"{"input":{"kind":"pause","seat":1}}"#,
            "Origin: http://example.com\r\n",
        ),
    )
    .await;
    assert!(cross_site.starts_with("HTTP/1.1 403"), "{cross_site}");
    let dump: EngineDump = serde_json::from_str(&dev(host.port, "dump").await).expect("a dump");
    assert_eq!(
        dump.commit, 0,
        "nothing the guard refused reached the owner"
    );
}

#[tokio::test]
async fn a_bad_body_or_query_is_a_typed_refusal_not_a_failure() {
    let host = start(&STARTER_FILES).await;

    let bad_body = client::post_json(host.port, path::INPUT, r#"{"input":{"kind":"tap"}}"#)
        .await
        .expect("answered");
    assert_eq!(bad_body.status, StatusCode::BAD_REQUEST);
    let error = rejected_with(&bad_body.body);
    assert!(
        matches!(&error, EngineError::InvalidBody(body) if body.cause == BodyCause::Data),
        "{error:?}"
    );

    let zero = client::post_json(
        host.port,
        path::INPUT,
        r#"{"input":{"kind":"pause","seat":1},"operation":0}"#,
    )
    .await
    .expect("answered");
    assert_eq!(zero.status, StatusCode::BAD_REQUEST);
    assert_eq!(rejected_with(&zero.body), EngineError::ZeroOperation);

    let no_seat = client::get(host.port, path::VIABLE)
        .await
        .expect("answered");
    assert_eq!(no_seat.status, StatusCode::BAD_REQUEST);
    let error = rejected_with(&no_seat.body);
    assert!(matches!(error, EngineError::InvalidQuery(_)), "{error:?}");
    let unknown = client::get(host.port, &format!("{}?seat=99", path::VIABLE))
        .await
        .expect("answered");
    assert_eq!(
        rejected_with(&unknown.body),
        EngineError::UnknownSeat { seat: 99 }
    );
    let audience = client::get(
        host.port,
        &format!("{}?audience=everyone", path::PROJECTION),
    )
    .await
    .expect("answered");
    assert_eq!(audience.status, StatusCode::BAD_REQUEST);
    let error = rejected_with(&audience.body);
    assert!(matches!(error, EngineError::InvalidQuery(_)), "{error:?}");
}

/// The typed error of a refusal the routes answered.
fn rejected_with(body: &str) -> EngineError {
    match serde_json::from_str::<EngineReply<serde_json::Value>>(body) {
        Ok(EngineReply::Rejected { error, .. }) => error,
        other => panic!("a typed refusal, got {other:?} from {body}"),
    }
}

#[tokio::test]
async fn every_dev_call_and_commit_is_a_line_in_the_hosts_event_log() {
    let host = start(&STARTER_FILES).await;
    dev(host.port, "input 1 --tap join").await;
    dev(host.port, "viable 1").await;
    dev(host.port, "dump").await;
    dev(host.port, "projection show tv").await;
    dev(host.port, "queue show").await;
    dev(host.port, "describe").await;

    let response = client::get(host.port, "/events.json")
        .await
        .expect("answered");
    let events: EventsReply = serde_json::from_str(&response.body).expect("events");
    let text: Vec<&str> = events
        .events
        .iter()
        .map(|line| line.text.as_str())
        .collect();
    for needle in [
        "ENGINE input sender=seat:1 kind=tap",
        "COMMIT applied commit=1",
        "ENGINE viable seat=1",
        "ENGINE dump revision=1",
        "ENGINE projection audience=tv revision=1",
        "ENGINE queue verb=show",
        "ENGINE describe",
    ] {
        assert!(
            text.iter().any(|line| line.contains(needle)),
            "no event line has '{needle}'"
        );
    }
}

#[tokio::test]
async fn the_broker_verbs_show_its_queues_its_ledger_and_a_request() {
    let host = start(&STARTER_FILES).await;

    let requested = dev(host.port, "broker request portrait").await;
    assert!(
        requested.contains("1: requested portrait: handle 1, pending"),
        "{requested}"
    );
    let queues = dev(host.port, "broker queues").await;
    assert!(
        queues.contains("classes, in the order they are served:"),
        "{queues}"
    );
    assert!(queues.contains("background: 0 waiting"), "{queues}");
    assert!(queues.contains("sent"), "{queues}");
    let ledger = dev(host.port, "broker ledger").await;
    assert!(
        ledger.contains(
            "portrait: 1 requests, 1 calls sent, 0 micro-dollars spent, 1 unpriced calls"
        ),
        "{ledger}"
    );
    assert!(ledger.contains("recorded, never limited"), "{ledger}");
}

#[tokio::test]
async fn chaos_commit_with_the_largest_count_is_one_run_and_answers_at_once() {
    let host = start(&STARTER_FILES).await;

    // One scripted write per count would hold four billion entries on the owner thread.
    let chaos = dev(host.port, &format!("chaos commit refused {}", u32::MAX)).await;

    assert!(
        chaos.contains(&format!("{} scripted writes wait", u32::MAX)),
        "{chaos}"
    );
    let again = dev(host.port, "chaos commit unavailable 2").await;
    assert!(
        again.contains(&format!("{} scripted writes wait", u64::from(u32::MAX) + 2)),
        "{again}"
    );
}

#[tokio::test]
async fn health_cost_clock_dice_and_chaos_are_dev_verbs_with_answers_and_log_lines() {
    let host = start(&STARTER_FILES).await;

    let health = dev(host.port, "health").await;
    assert!(health.contains("running"), "{health}");
    let cost = dev(host.port, "cost").await;
    assert!(cost.contains("recorded, never limited"), "{cost}");
    let advanced = dev(host.port, "clock advance 5000").await;
    assert!(advanced.contains("the clock reads 5"), "{advanced}");
    let frozen = dev(host.port, "clock freeze").await;
    assert!(frozen.contains("frozen"), "{frozen}");
    let resumed = dev(host.port, "clock resume").await;
    assert!(!resumed.contains("frozen: true"), "{resumed}");
    let dice = dev(host.port, "dice force 20 1").await;
    assert!(dice.contains('2'), "{dice}");
    let seeded = dev(host.port, "dice seed 9").await;
    assert!(seeded.contains('9'), "{seeded}");
    let chaos = dev(host.port, "chaos commit refused 2").await;
    assert!(chaos.contains("refused"), "{chaos}");
    let down = dev(host.port, "chaos interpreter down").await;
    assert!(down.to_lowercase().contains("down"), "{down}");
    let up = dev(host.port, "chaos interpreter up").await;
    assert!(!up.is_empty());

    let log = dev(host.port, "events").await;
    for needle in [
        "ENGINE health",
        "ENGINE clock",
        "ENGINE dice",
        "ENGINE chaos",
    ] {
        assert!(log.contains(needle), "no '{needle}' in the log: {log}");
    }
}

#[tokio::test]
async fn a_forced_name_with_a_newline_through_the_route_is_one_refused_line() {
    let host = start(&STARTER_FILES).await;
    let body = r#"{"machine":"lobby\nCOMMIT applied commit=77","state":"started"}"#;

    let reply = raw_exchange(host.port, &post_request(path::FORCE, body, "")).await;

    assert!(reply.starts_with("HTTP/1.1 4"), "{reply}");
    let log = dev(host.port, "events").await;
    assert!(!log.contains("commit=77"), "{log}");
}

#[tokio::test]
async fn a_body_with_an_unknown_field_is_refused_typed() {
    let host = start(&STARTER_FILES).await;

    let answer = client::post_json(
        host.port,
        path::CLOCK,
        r#"{"control":"advance","ms":5,"by":1}"#,
    )
    .await
    .expect("answered");

    assert_eq!(answer.status, StatusCode::BAD_REQUEST);
    let error = rejected_with(&answer.body);
    assert!(
        matches!(&error, EngineError::InvalidBody(body) if body.cause == BodyCause::Data),
        "{error:?}"
    );
}

// ---- Route logging and the reads that reach into the owner (wave 3, U2) ----

/// Every line of the host's log that starts with `prefix`.
fn lines_starting(host: &crate::test_support::Running, prefix: &str) -> Vec<String> {
    super::fixtures::lines(host.state.log())
        .into_iter()
        .filter(|line| line.starts_with(prefix))
        .collect()
}

/// Serves `router` alone on an ephemeral port, for a test that needs a router the host does not
/// build (an owner that never started, a small body limit).
async fn serve_alone(router: axum::Router) -> u16 {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("binds an ephemeral port");
    let port = listener.local_addr().expect("has an address").port();
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    port
}

#[tokio::test]
async fn a_bad_body_writes_one_refused_line_with_its_verb_its_error_and_its_duration() {
    let host = start(&STARTER_FILES).await;

    let answer = client::post_json(host.port, path::INPUT, r#"{"input":{"kind":"tap"}}"#)
        .await
        .expect("answered");

    assert_eq!(answer.status, StatusCode::BAD_REQUEST);
    let refused = lines_starting(&host, "ENGINE refused verb=input ");
    let [line] = refused.as_slice() else {
        panic!("one refused line: {refused:?}");
    };
    let duration = line
        .strip_prefix("ENGINE refused verb=input error=invalid_body duration_ms=")
        .unwrap_or_else(|| panic!("the refused line's shape: {line}"));
    assert!(duration.parse::<u64>().is_ok(), "{line}");
}

#[tokio::test]
async fn the_engine_writes_its_start_line_with_its_machines_spool_and_level() {
    let host = start(&STARTER_FILES).await;
    // The server builds its router on its own task: an answer means it has.
    dev(host.port, "health").await;

    let started = lines_starting(&host, "ENGINE started ");

    let [line] = started.as_slice() else {
        panic!("one start line: {started:?}");
    };
    assert!(line.contains(" machines=3 "), "{line}");
    assert!(line.contains(" spool=off "), "{line}");
    assert!(line.ends_with(" level=verbose"), "{line}");
}

#[tokio::test]
async fn an_owner_that_never_started_refuses_typed_with_its_cause_and_logs_both() {
    let host = start(&STARTER_FILES).await;
    let router = super::routes::router_not_started(
        std::sync::Arc::clone(&host.state),
        super::setup::EngineStartError::Thread(std::io::ErrorKind::Other),
    );
    let port = serve_alone(router).await;

    let input = client::post_json(port, path::INPUT, r#"{"input":{"kind":"pause","seat":1}}"#)
        .await
        .expect("answered");
    let health = client::get(port, path::HEALTH).await.expect("answered");

    assert_eq!(input.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(health.status, StatusCode::SERVICE_UNAVAILABLE);
    let error = rejected_with(&health.body);
    assert!(
        matches!(
            error,
            EngineError::NotStarted {
                cause: super::setup::StartCause::Thread,
                ..
            }
        ),
        "{error:?}"
    );
    assert!(error.to_string().contains("(thread)"), "{error}");
    assert_eq!(
        lines_starting(&host, "ENGINE start_failed "),
        ["ENGINE start_failed cause=thread"]
    );
    for verb in ["input", "health"] {
        let refused = lines_starting(&host, &format!("ENGINE refused verb={verb} "));
        assert_eq!(refused.len(), 1, "{refused:?}");
        assert!(
            refused[0].contains(" error=not_started duration_ms="),
            "{refused:?}"
        );
    }
    let printed = run_dev(port, &split_words("health").expect("splits"))
        .await
        .expect_err("refused")
        .to_string();
    assert!(
        printed.contains("refused (not_started)") && printed.contains("(thread)"),
        "{printed}"
    );
}

#[tokio::test]
async fn a_body_over_the_limit_is_answered_413_typed_and_logged() {
    let host = start(&STARTER_FILES).await;
    let router = super::routes::router(std::sync::Arc::clone(&host.state))
        .layer(axum::extract::DefaultBodyLimit::max(32));
    let port = serve_alone(router).await;
    let body = format!(
        r#"{{"input":{{"kind":"say","seat":1,"text":"{}"}}}}"#,
        "a".repeat(200)
    );

    let reply = raw_exchange(port, &post_request(path::INPUT, &body, "")).await;

    assert!(reply.starts_with("HTTP/1.1 413"), "{reply}");
    assert!(reply.contains(r#""error":"unreadable_body""#), "{reply}");
    assert!(reply.contains(r#""cause":"too_large""#), "{reply}");
    let refused = lines_starting(&host, "ENGINE refused verb=input ");
    assert_eq!(refused.len(), 1, "{refused:?}");
    assert!(
        refused[0].contains(" error=unreadable_body duration_ms="),
        "{refused:?}"
    );
}

#[tokio::test]
async fn inspect_serves_every_report_and_a_seat_sees_the_pending_count_redacted() {
    let host = start(&STARTER_FILES).await;

    for report in super::request::InspectName::ALL {
        let answer = dev(host.port, &format!("inspect {}", report.key())).await;
        assert!(
            answer.starts_with(&format!("1: {} as operator sees it", report.key())),
            "{answer}"
        );
    }
    let seat = dev(host.port, "inspect pending --seat 1").await;
    assert!(seat.starts_with("1: pending as seat:1 sees it"), "{seat}");
    assert!(seat.contains("\"visibility\": \"redacted\""), "{seat}");
    let host_view = dev(host.port, "inspect requests --host").await;
    assert!(
        host_view.contains("\"visibility\": \"shown\""),
        "{host_view}"
    );
    assert_eq!(
        lines_starting(&host, "ENGINE inspect report=pending audience=seat:1").len(),
        1
    );

    let unknown = client::get(host.port, &path::inspect_path("everything"))
        .await
        .expect("answered");
    assert_eq!(unknown.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        rejected_with(&unknown.body),
        EngineError::UnknownReport {
            report: "everything".to_owned()
        }
    );
    let stranger = client::get(
        host.port,
        &format!("{}?audience=seat:99", path::inspect_path("pending")),
    )
    .await
    .expect("answered");
    assert_eq!(
        rejected_with(&stranger.body),
        EngineError::UnknownSeat { seat: 99 }
    );
    assert_eq!(
        lines_starting(&host, "ENGINE refused verb=inspect ").len(),
        2
    );
}

#[tokio::test]
async fn dump_names_the_table_or_one_machine_and_refuses_one_off_the_table() {
    use super::report::reach::DumpReport;

    let host = start(&STARTER_FILES).await;

    let table = dev(host.port, "dump table").await;
    let lobby = dev(host.port, "dump lobby").await;
    let refused = run_dev(host.port, &split_words("dump dialogue").expect("splits"))
        .await
        .expect_err("the dialogue is not on the table yet")
        .to_string();

    let table: DumpReport = serde_json::from_str(&table).expect("a table dump");
    assert!(matches!(table, DumpReport::Table { .. }), "{table:?}");
    let lobby: DumpReport = serde_json::from_str(&lobby).expect("a machine dump");
    let DumpReport::Machine { places, .. } = lobby else {
        panic!("a machine dump: {lobby:?}");
    };
    assert_eq!(places.len(), 1);
    assert!(places[0].focus && !places[0].stacked, "{places:?}");
    assert_eq!(places[0].machine, "lobby");
    assert!(refused.contains("machine_not_on_table"), "{refused}");
    for of in ["of=table", "of=lobby"] {
        assert_eq!(
            super::fixtures::count_lines(host.state.log(), of),
            1,
            "no dump line {of}"
        );
    }
}

#[tokio::test]
async fn chaos_with_no_words_and_broker_queues_read_what_is_in_force() {
    let host = start(&STARTER_FILES).await;
    dev(host.port, "chaos commit refused 2").await;
    dev(host.port, "chaos interpreter down").await;

    let standing = dev(host.port, "chaos").await;
    let queues = dev(host.port, "broker queues").await;

    assert!(
        standing.contains("1: commit: 2 scripted writes wait"),
        "{standing}"
    );
    assert!(standing.contains("2: interpreter: down"), "{standing}");
    assert!(
        queues.contains("limits: ") && queues.contains("most waiting: live "),
        "{queues}"
    );
    assert_eq!(
        lines_starting(&host, "ENGINE chaos verb=show in_force=2").len(),
        1
    );
}

#[tokio::test]
async fn the_event_level_is_read_and_set_through_its_route_with_a_host_line() {
    let host = start(&STARTER_FILES).await;

    let before = client::get(host.port, path::EVENTS_LEVEL)
        .await
        .expect("answered");
    let set = client::post_json(host.port, path::EVENTS_LEVEL, r#"{"level":"always"}"#)
        .await
        .expect("answered");
    let after = client::get(host.port, path::EVENTS_LEVEL)
        .await
        .expect("answered");
    let refused = client::post_json(host.port, path::EVENTS_LEVEL, r#"{"level":"loud"}"#)
        .await
        .expect("answered");

    let level = |body: &str| match serde_json::from_str::<EngineReply<serde_json::Value>>(body) {
        Ok(EngineReply::Done { value }) => value["level"].clone(),
        other => panic!("a level: {other:?}"),
    };
    assert_eq!(level(&before.body), "verbose");
    assert_eq!(level(&set.body), "always");
    assert_eq!(level(&after.body), "always");
    assert_eq!(refused.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        lines_starting(&host, "HOST sink_level "),
        ["HOST sink_level from=verbose to=always driven=true"]
    );
    assert_eq!(
        lines_starting(
            &host,
            "ENGINE refused verb=events_level error=invalid_body "
        )
        .len(),
        1
    );
    let health = dev(host.port, "health").await;
    assert!(
        health.contains("events at level always"),
        "the engine's sink follows: {health}"
    );
}

#[tokio::test]
async fn a_wire_intent_through_the_route_writes_its_line_and_no_projection_line() {
    let host = start(&STARTER_FILES).await;

    let joined = client::post_json(
        host.port,
        path::WIRE_INTENT,
        r#"{"seat":1,"intent":{"kind":"tap","offer":"join"},"operation":41}"#,
    )
    .await
    .expect("answered");
    let zero = client::post_json(
        host.port,
        path::WIRE_INTENT,
        r#"{"seat":1,"intent":{"kind":"pause"},"operation":0}"#,
    )
    .await
    .expect("answered");
    let view = client::get(host.port, &format!("{}?audience=tv", path::WIRE_VIEW))
        .await
        .expect("answered");

    assert_eq!(joined.status, StatusCode::OK, "{}", joined.body);
    assert_eq!(zero.status, StatusCode::BAD_REQUEST);
    assert_eq!(view.status, StatusCode::OK);
    assert_eq!(
        lines_starting(&host, "ENGINE wire_intent "),
        [
            format!(
                "ENGINE wire_intent seat=1 operation={:032x} outcome=accepted code=-",
                41
            ),
            "ENGINE wire_intent seat=1 operation=- outcome=refused code=zero_operation".to_owned(),
        ]
    );
    assert!(
        lines_starting(&host, "ENGINE projection ").is_empty(),
        "the intent's own view read is not a projection"
    );
    let served = lines_starting(&host, "ENGINE wire_view ");
    let [line] = served.as_slice() else {
        panic!("one served view: {served:?}");
    };
    assert!(
        line.starts_with("ENGINE wire_view audience=tv revision=1 bytes="),
        "{line}"
    );
}
