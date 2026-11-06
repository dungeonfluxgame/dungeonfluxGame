use crate::command::{self, Command, CommandError, split_words};

use super::play::{
    LOBBY_USAGE, LobbyCommand, PLAY_USAGE, PlayCommand, SCENE_START_USAGE, WIRE_USAGE,
};
use super::{EngineCommand, VERBS, verb_usages};
use crate::engine::machine_io::WireIntentKind;
use crate::engine::request::AudienceSpec;

fn parsed(line: &str) -> Result<Command, CommandError> {
    command::parse(&split_words(line).expect("the line splits"))
}

fn play(line: &str) -> PlayCommand {
    match parsed(line) {
        Ok(Command::Engine(EngineCommand::Play(command))) => command,
        other => panic!("'{line}' should be a play command, got {other:?}"),
    }
}

#[test]
fn the_lobby_verbs_are_the_taps_a_phone_sends_and_refuse_the_rest() {
    assert_eq!(
        play("lobby join 3"),
        PlayCommand::Lobby(LobbyCommand::Act {
            action: "join",
            seat: 3
        })
    );
    for action in ["leave", "ready", "unready", "start"] {
        assert!(matches!(
            play(&format!("lobby {action} 1")),
            PlayCommand::Lobby(LobbyCommand::Act { .. })
        ));
    }
    assert_eq!(play("lobby show"), PlayCommand::Lobby(LobbyCommand::Show));
    for bad in ["lobby", "lobby join", "lobby dance 1", "lobby show 1"] {
        assert_eq!(parsed(bad), Err(CommandError::Usage(LOBBY_USAGE)), "{bad}");
    }
    assert!(matches!(
        parsed("lobby join first"),
        Err(CommandError::BadCount { .. })
    ));
}

#[test]
fn scene_start_takes_an_optional_seat() {
    assert_eq!(play("scene start"), PlayCommand::SceneStart { seat: None });
    assert_eq!(
        play("scene start 2"),
        PlayCommand::SceneStart { seat: Some(2) }
    );
    assert_eq!(
        parsed("scene start 1 2"),
        Err(CommandError::Usage(SCENE_START_USAGE))
    );
}

#[test]
fn the_wire_verbs_name_an_audience_or_an_intent_and_the_play_verbs_a_seat() {
    assert_eq!(
        play("wire view tv"),
        PlayCommand::WireView(AudienceSpec::Tv)
    );
    assert_eq!(
        play("wire view 2"),
        PlayCommand::WireView(AudienceSpec::Seat { seat: 2 })
    );
    let PlayCommand::WireSend(body) = play("wire send 1 --op 9 --tap ready") else {
        panic!("a wire send");
    };
    assert_eq!(body.seat, 1);
    assert_eq!(body.operation, Some(9));
    assert_eq!(
        body.intent,
        WireIntentKind::Tap {
            offer: "ready".to_owned()
        }
    );
    let PlayCommand::WireSend(body) = play("wire send 1 --say hello there") else {
        panic!("a wire send");
    };
    assert_eq!(
        body.intent,
        WireIntentKind::Say {
            text: "hello there".to_owned()
        }
    );
    assert!(matches!(
        play("wire send 2 --pause"),
        PlayCommand::WireSend(_)
    ));
    assert_eq!(parsed("wire"), Err(CommandError::Usage(WIRE_USAGE)));
    assert_eq!(parsed("wire send 1"), Err(CommandError::Usage(WIRE_USAGE)));
    assert_eq!(play("play publish"), PlayCommand::Publish { seat: None });
    assert_eq!(
        play("play publish 3"),
        PlayCommand::Publish { seat: Some(3) }
    );
    assert_eq!(play("play remove"), PlayCommand::Remove);
    assert_eq!(parsed("play"), Err(CommandError::Usage(PLAY_USAGE)));
}

#[test]
fn the_new_verbs_are_registered_listed_and_allowed_in_a_script() {
    for verb in ["lobby", "wire", "play"] {
        assert!(VERBS.contains(&verb), "{verb} is not in VERBS");
    }
    let usages = verb_usages();
    for usage in [LOBBY_USAGE, SCENE_START_USAGE, WIRE_USAGE, PLAY_USAGE] {
        assert!(usages.iter().any(|listed| listed == usage), "{usage}");
    }
    for line in ["lobby join 1", "scene start", "wire view tv", "play remove"] {
        assert!(
            command::parse_script_line(&split_words(line).expect("splits")).is_ok(),
            "a script may run '{line}'"
        );
    }
}

// ---- #1755: the dev verbs fail by name when the engine's answers lack what they read ----

use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::dev::DevError;

use super::play::{LIVE_FIXTURE, run};
use crate::engine::routes::{AUDIENCE_PARAMETER, path};

/// A host that answers each request target with a fixed body and everything else with `{}`, so a
/// verb can be shown what a real engine never serves.
async fn serve(answers: Vec<(String, String)>) -> u16 {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("a port");
    let port = listener.local_addr().expect("an address").port();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let answers = answers.clone();
            tokio::spawn(async move {
                let mut buffer = vec![0_u8; 16_384];
                let mut length = 0;
                while let Ok(read) = stream.read(&mut buffer[length..]).await {
                    length += read;
                    let head_done = buffer[..length].windows(4).any(|part| part == b"\r\n\r\n");
                    if read == 0 || head_done || length == buffer.len() {
                        break;
                    }
                }
                let head = String::from_utf8_lossy(&buffer[..length]).into_owned();
                let target = head.split_whitespace().nth(1).unwrap_or_default();
                let body = answers
                    .iter()
                    .find(|(wanted, _)| wanted == target)
                    .map_or_else(|| "{}".to_owned(), |(_, body)| body.clone());
                let answer = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: \
                     {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                // The test has what it needs once the verb has failed; a closed socket is fine.
                let _ = stream.write_all(answer.as_bytes()).await;
                let _ = stream.shutdown().await;
            });
        }
    });
    port
}

fn done(value: &Value) -> String {
    json!({"outcome": "done", "value": value}).to_string()
}

fn dump(machine: &str, state: &str, slot: &Value) -> String {
    done(&json!({
        "revision": 3, "commit": 3, "machine": machine, "state": state, "schema_version": 1,
        "slot": slot,
        "table": {
            "seat_capacity": 6, "seats": [1, 2], "active_scene": null, "active_beat": null,
            "last_hand_off": null, "scenes": [], "cast": 0, "world_time_s": 0,
            "status": "running", "digest": "0000000000000000"
        }
    }))
}

fn projection_address(who: &str) -> String {
    format!("{}?{AUDIENCE_PARAMETER}={who}", path::PROJECTION)
}

fn rejection(result: Result<String, DevError>) -> String {
    match result {
        Err(DevError::Rejected(message)) => message,
        other => panic!("a rejection naming the missing field, got {other:?}"),
    }
}

#[tokio::test]
async fn scene_start_and_lobby_show_fail_naming_the_ready_list_a_dump_lacks() {
    let port = serve(vec![
        (
            path::DUMP.to_owned(),
            dump("lobby", "gathering", &json!({"phase": "gathering"})),
        ),
        (
            projection_address("tv"),
            done(&json!({"readout": "0 of 2 seats are ready."})),
        ),
    ])
    .await;

    let started = rejection(run(port, PlayCommand::SceneStart { seat: None }).await);
    assert!(started.contains("ready"), "{started}");
    let shown = rejection(run(port, PlayCommand::Lobby(LobbyCommand::Show)).await);
    assert!(shown.contains("ready"), "{shown}");
}

#[tokio::test]
async fn lobby_show_fails_naming_the_readout_a_projection_lacks_and_reads_a_started_lobby() {
    let port = serve(vec![
        (
            path::DUMP.to_owned(),
            dump(
                "lobby",
                "gathering",
                &json!({"phase": "gathering", "ready": [1]}),
            ),
        ),
        (projection_address("tv"), done(&json!({"view": {}}))),
    ])
    .await;
    let shown = rejection(run(port, PlayCommand::Lobby(LobbyCommand::Show)).await);
    assert!(shown.contains("readout"), "{shown}");

    let started = serve(vec![
        (
            path::DUMP.to_owned(),
            dump("explore", "presenting", &json!({"anything": 1})),
        ),
        (
            projection_address("tv"),
            done(&json!({"readout": "The party is at the gate."})),
        ),
    ])
    .await;
    let text = run(started, PlayCommand::Lobby(LobbyCommand::Show))
        .await
        .expect("a table past the lobby is shown without a ready list");
    assert!(text.contains("the table is in explore"), "{text}");
    assert!(text.contains("not asked"), "{text}");
}

#[tokio::test]
async fn play_publish_fails_naming_the_view_a_projection_lacks_and_never_writes_a_null_fixture() {
    let dumped = (
        path::DUMP.to_owned(),
        dump(
            "lobby",
            "gathering",
            &json!({"phase": "gathering", "ready": []}),
        ),
    );
    let broken_tv = serve(vec![
        dumped.clone(),
        (projection_address("tv"), done(&json!({"readout": "x"}))),
        (
            projection_address("1"),
            done(&json!({"view": {"hero": "A"}})),
        ),
    ])
    .await;
    let message = rejection(run(broken_tv, PlayCommand::Publish { seat: None }).await);
    assert!(
        message.contains("shared screen") && message.contains("view"),
        "{message}"
    );

    let broken_phone = serve(vec![
        dumped.clone(),
        (
            projection_address("tv"),
            done(&json!({"view": {"place": "gate"}})),
        ),
        (projection_address("1"), done(&json!({"readout": "y"}))),
    ])
    .await;
    let message = rejection(run(broken_phone, PlayCommand::Publish { seat: None }).await);
    assert!(
        message.contains("seat 1") && message.contains("view"),
        "{message}"
    );

    let whole = serve(vec![
        dumped,
        (
            projection_address("tv"),
            done(&json!({"view": {"place": "gate"}})),
        ),
        (
            projection_address("1"),
            done(&json!({"view": {"hero": "A"}})),
        ),
    ])
    .await;
    let written = run(whole, PlayCommand::Publish { seat: None })
        .await
        .expect("two real views are published");
    assert!(written.contains(LIVE_FIXTURE), "{written}");
}
