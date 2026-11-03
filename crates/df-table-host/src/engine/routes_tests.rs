//! The dev engine through a running host: the routes, the guard, the command line, the repl and a
//! script, on an ephemeral port.

use crate::client;
use crate::command::split_words;
use crate::dev::run_dev;
use crate::repl::{LineResult, run_line};
use crate::test_support::{STARTER_FILES, post_request, raw_exchange, start};
use crate::wire::EventsReply;

use super::report::{EngineDump, EngineReply};
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
            "POST {} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: text/plain\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}",
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
    assert_eq!(bad_body.status, 400);
    let reply: EngineReply<serde_json::Value> =
        serde_json::from_str(&bad_body.body).expect("a reply");
    assert!(matches!(reply, EngineReply::Rejected { .. }));

    let zero = client::post_json(
        host.port,
        path::INPUT,
        r#"{"input":{"kind":"pause","seat":1},"operation":0}"#,
    )
    .await
    .expect("answered");
    assert_eq!(zero.status, 400);
    assert!(
        zero.body.contains("an operation id is not zero"),
        "{}",
        zero.body
    );

    let no_seat = client::get(host.port, path::VIABLE)
        .await
        .expect("answered");
    assert_eq!(no_seat.status, 400);
    let unknown = client::get(host.port, &format!("{}?seat=99", path::VIABLE))
        .await
        .expect("answered");
    assert!(unknown.body.contains("unknown_seat"), "{}", unknown.body);
    let audience = client::get(
        host.port,
        &format!("{}?audience=everyone", path::PROJECTION),
    )
    .await
    .expect("answered");
    assert_eq!(audience.status, 400);
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
