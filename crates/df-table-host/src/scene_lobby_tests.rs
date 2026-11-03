//! The `scene lobby`, `scene loading`, `scene roleplay` and `scene pill` verbs: parsed into
//! typed changes, applied by the host as numbered changes, drawn on top of the fixture and
//! recorded in the audit log with the component and the field, never the free text.

use df_playground::fixture::Fixture;
use df_playground::lobby_change::{LobbyLine, PillPart, ScreenEdit};
use df_playground::lobby_model::{LogoStyle, PartyCount, SeatState, SummaryIcon};
use df_playground::scene::SceneChange;

use crate::client::get;
use crate::command::{Command, CommandError, parse, split_words, usage_text};
use crate::dev::{DevError, run_dev};
use crate::test_support::{Running, STARTER_FILES, start};
use crate::wire::{ControlsReply, EventsReply};

const FIXTURE: &str = r#"{
    "name": "One",
    "tv": {"place": "Here", "status": {"kind": "ok"}, "elements": [], "caption": null},
    "phone": {"hero": "Pip", "status": {"kind": "ok"}}
}"#;

fn edit(line: &str) -> Result<ScreenEdit, CommandError> {
    let words = split_words(line).expect("the line splits");
    match parse(&words)? {
        Command::Scene(SceneChange::Screen { edit }) => Ok(edit),
        other => panic!("expected a screen change, got {other:?}"),
    }
}

async fn run(running: &Running, line: &str) -> Result<String, DevError> {
    run_dev(running.port, &split_words(line).expect("the line splits")).await
}

async fn with_fixture(running: &Running) {
    running.write_asset("fixtures/one.json", FIXTURE);
    get(running.port, "/playground/controls.json")
        .await
        .expect("answers");
}

async fn drawn(running: &Running) -> Fixture {
    let response = get(running.port, "/playground/controls.json")
        .await
        .expect("answers");
    let reply: ControlsReply = serde_json::from_str(&response.body).expect("controls");
    let mut fixture = Fixture::parse(FIXTURE).expect("the fixture reads");
    reply.scene.apply_to(&mut fixture);
    fixture
}

async fn audit_lines(running: &Running) -> Vec<String> {
    let events = get(running.port, "/events.json").await.expect("answers");
    let events: EventsReply = serde_json::from_str(&events.body).expect("events");
    events.events.into_iter().map(|line| line.text).collect()
}

#[test]
fn the_lobby_verbs_read_into_typed_edits() {
    let seat = edit(
        "scene lobby seat 2 state=creating name=Alyssa class=Mage step=race progress=40 host=false portrait=lobby-mage.webp",
    )
    .expect("reads");
    let ScreenEdit::LobbySeat { number, seat } = seat else {
        panic!("a seat edit");
    };
    assert_eq!(
        (number, seat.state, seat.step, seat.progress_percent),
        (2, Some(SeatState::Creating), Some(1), Some(40))
    );
    assert_eq!(
        (seat.name.as_deref(), seat.host, seat.portrait.as_deref()),
        (Some("Alyssa"), Some(false), Some("lobby-mage.webp"))
    );
    assert_eq!(
        edit("scene lobby set title The Ashen Citadel Awaits"),
        Ok(ScreenEdit::LobbyLine {
            field: LobbyLine::Title,
            text: "The Ashen Citadel Awaits".into()
        })
    );
    assert_eq!(edit("scene lobby clear"), Ok(ScreenEdit::LobbyClear {}));
    assert_eq!(
        edit("scene lobby legend on"),
        Ok(ScreenEdit::LobbyLegend { shown: true })
    );
    let invite = edit("scene lobby invite code=AC7F-K8M2 link=dungeonflux.com/join/AC7F-K8M2 joined=3 capacity=6 icon=people")
        .expect("reads");
    let ScreenEdit::LobbyInvite { invite } = invite else {
        panic!("an invite edit");
    };
    assert_eq!(
        (invite.joined, invite.capacity, invite.icon),
        (Some(3), Some(6), Some(SummaryIcon::People))
    );
    let ScreenEdit::LobbyBegin { begin } =
        edit("scene lobby begin enabled=true label=\"Begin Adventure\"").expect("reads")
    else {
        panic!("a begin edit");
    };
    assert_eq!(
        (begin.enabled, begin.label.as_deref()),
        (Some(true), Some("Begin Adventure"))
    );
}

#[test]
fn the_lobby_verbs_refuse_what_is_not_of_the_right_kind_with_the_usage() {
    assert!(matches!(edit("scene lobby"), Err(CommandError::Usage(_))));
    assert!(matches!(
        edit("scene lobby seat"),
        Err(CommandError::Usage(_))
    ));
    assert!(matches!(
        edit("scene lobby seat two"),
        Err(CommandError::BadCount { .. })
    ));
    assert!(matches!(
        edit("scene lobby seat 1 state=asleep"),
        Err(CommandError::BadChoice { .. })
    ));
    assert!(matches!(
        edit("scene lobby seat 1 step=fourth"),
        Err(CommandError::BadChoice { .. })
    ));
    assert!(matches!(
        edit("scene lobby seat 1 colour=red"),
        Err(CommandError::BadChoice { .. })
    ));
    assert!(matches!(
        edit("scene lobby seat 1 name=A name=B"),
        Err(CommandError::DuplicateKey(_))
    ));
    assert!(matches!(
        edit("scene lobby invite joined=many"),
        Err(CommandError::BadChoice { .. })
    ));
    assert!(matches!(
        edit("scene lobby set heading Words"),
        Err(CommandError::BadChoice { .. })
    ));
    assert!(matches!(
        edit("scene lobby legend maybe"),
        Err(CommandError::BadChoice { .. })
    ));
}

#[test]
fn the_pill_verbs_read_into_typed_edits() {
    let ScreenEdit::PillSet { pill } = edit("scene pill set party 4/6").expect("reads") else {
        panic!("a pill edit");
    };
    assert_eq!(
        pill.party,
        Some(PartyCount {
            joined: 4,
            capacity: 6
        })
    );
    let ScreenEdit::PillSet { pill } =
        edit(r#"scene pill set session "The Ember Roads""#).expect("reads")
    else {
        panic!("a pill edit");
    };
    assert_eq!(pill.session.as_deref(), Some("The Ember Roads"));
    let ScreenEdit::PillSet { pill } = edit("scene pill set logo inline").expect("reads") else {
        panic!("a pill edit");
    };
    assert_eq!(pill.logo, Some(LogoStyle::Inline));
    assert_eq!(
        edit("scene pill clear rate"),
        Ok(ScreenEdit::PillClear {
            part: PillPart::Rate
        })
    );
    assert!(matches!(
        edit("scene pill set party four"),
        Err(CommandError::BadChoice { .. })
    ));
    assert!(matches!(
        edit("scene pill set price 9"),
        Err(CommandError::BadChoice { .. })
    ));
    assert!(matches!(
        edit("scene pill clear price"),
        Err(CommandError::BadChoice { .. })
    ));
    assert!(matches!(edit("scene pill"), Err(CommandError::Usage(_))));
}

#[test]
fn the_roleplay_and_loading_verbs_read_into_typed_edits() {
    let ScreenEdit::RoleplaySet { roleplay } = edit(
        r#"scene roleplay set title="1. Arrive in Town" icon=shield speaker=Kael line="So this is Valemere." tone=hero tail-x=40 caption="The party arrives.""#,
    )
    .expect("reads") else {
        panic!("a roleplay edit");
    };
    assert_eq!(
        (roleplay.title.as_deref(), roleplay.speaker.as_deref()),
        (Some("1. Arrive in Town"), Some("Kael"))
    );
    assert_eq!(
        (
            roleplay.tail_x_percent,
            roleplay.tone.is_some(),
            roleplay.icon.is_some()
        ),
        (Some(40), true, true)
    );
    assert_eq!(
        edit("scene roleplay clear"),
        Ok(ScreenEdit::RoleplayClear {})
    );
    assert!(matches!(
        edit("scene roleplay set icon=banana"),
        Err(CommandError::BadChoice { .. })
    ));
    assert!(matches!(
        edit("scene roleplay set tone=loud"),
        Err(CommandError::BadChoice { .. })
    ));
    let ScreenEdit::LoadingSet { loading } =
        edit("scene loading set percent=76 narrator=Vaelin").expect("reads")
    else {
        panic!("a loading edit");
    };
    assert_eq!(
        (loading.percent, loading.narrator_name.as_deref()),
        (Some(76), Some("Vaelin"))
    );
    assert!(matches!(
        edit("scene loading seat 2 ready=true"),
        Ok(ScreenEdit::LoadingSeat { number: 2, .. })
    ));
    assert_eq!(edit("scene loading clear"), Ok(ScreenEdit::LoadingClear {}));
}

#[test]
fn help_lists_the_lobby_loading_roleplay_and_pill_verbs() {
    let help = usage_text();
    for text in [
        "scene lobby",
        "scene loading",
        "scene roleplay",
        "scene pill",
    ] {
        assert!(help.contains(text), "{text}");
    }
}

#[tokio::test]
async fn lobby_changes_are_applied_numbered_drawn_and_audited_without_their_text() {
    let running = start(&STARTER_FILES).await;
    with_fixture(&running).await;
    let said = run(&running, "scene lobby set title \"A Secret Title\"")
        .await
        .expect("applies");
    assert!(said.starts_with("ok (revision "), "{said}");
    run(
        &running,
        "scene lobby seat 1 state=locked name=Cam class=Paladin host=true",
    )
    .await
    .expect("applies");
    run(
        &running,
        "scene lobby seat 2 state=creating name=Alyssa progress=70",
    )
    .await
    .expect("applies");
    run(
        &running,
        "scene lobby invite code=AC7F-K8M2 joined=2 capacity=6",
    )
    .await
    .expect("applies");
    run(
        &running,
        "scene lobby begin enabled=false note=\"Wait for all players to be ready\"",
    )
    .await
    .expect("applies");
    let lobby = drawn(&running).await.tv.lobby.expect("a lobby");
    assert_eq!(lobby.title, "A Secret Title");
    assert_eq!(
        (lobby.seats.len(), lobby.seats[0].state, lobby.seats[0].host),
        (2, SeatState::LockedIn, true)
    );
    assert_eq!(lobby.seats[1].progress_percent, Some(70));
    assert_eq!(
        lobby
            .invite
            .as_ref()
            .map(|invite| (invite.joined, invite.capacity)),
        Some((2, 6))
    );
    assert!(!lobby.begin.expect("a begin button").enabled);
    run(&running, "scene lobby begin enabled=true")
        .await
        .expect("applies");
    assert!(
        drawn(&running)
            .await
            .tv
            .lobby
            .expect("a lobby")
            .begin
            .expect("begin")
            .enabled
    );
    let lines = audit_lines(&running).await;
    assert!(
        lines.iter().any(|line| line
            .starts_with("PLAYGROUND screen component=lobby field=seat seat=2 state=creating")),
        "{lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("component=lobby field=title chars=14")),
        "{lines:?}"
    );
    assert!(
        lines
            .iter()
            .all(|line| !line.contains("Secret") && !line.contains("Alyssa")),
        "{lines:?}"
    );
}

#[tokio::test]
async fn a_refused_lobby_change_says_why_and_changes_nothing() {
    let running = start(&STARTER_FILES).await;
    with_fixture(&running).await;
    let before = drawn(&running).await;
    for (line, wanted) in [
        ("scene lobby seat 1 progress=101", "percent"),
        ("scene lobby seat 1 step=3", "step"),
        ("scene lobby seat 0 state=open", "numbered"),
        (
            "scene lobby invite joined=7 capacity=6",
            "cannot have joined",
        ),
        (
            "scene lobby seat 1 portrait=no-such-picture.webp",
            "no-such-picture.webp",
        ),
    ] {
        let refused = run(&running, line).await.expect_err("is refused");
        assert!(refused.to_string().contains(wanted), "{line}: {refused}");
    }
    assert_eq!(drawn(&running).await, before);
}

#[tokio::test]
async fn pills_loading_and_roleplay_changes_are_drawn_and_reset_returns_to_the_file() {
    let running = start(&STARTER_FILES).await;
    with_fixture(&running).await;
    run(&running, r#"scene pill set session "The Ember Roads""#)
        .await
        .expect("applies");
    run(&running, "scene pill set party 4/6")
        .await
        .expect("applies");
    run(&running, "scene pill set balance $28.40")
        .await
        .expect("applies");
    run(&running, "scene pill set rate $3.90/hr")
        .await
        .expect("applies");
    run(&running, "scene pill clear rate")
        .await
        .expect("applies");
    let bar = drawn(&running).await.tv.pills.expect("pills");
    assert_eq!(
        (bar.session.as_deref(), bar.balance.as_deref(), bar.rate),
        (Some("The Ember Roads"), Some("$28.40"), None)
    );
    assert_eq!(
        bar.party,
        Some(PartyCount {
            joined: 4,
            capacity: 6
        })
    );
    run(
        &running,
        "scene loading set title=\"The Ashen Citadel\" percent=40",
    )
    .await
    .expect("applies");
    run(&running, "scene loading seat 1 name=Cam ready=true")
        .await
        .expect("applies");
    let loading = drawn(&running).await.tv.loading.expect("loading");
    assert_eq!((loading.percent, loading.seats[0].ready), (40, true));
    run(&running, "scene roleplay set title=\"1. Arrive in Town\" icon=shield speaker=Kael line=\"So this is Valemere.\"").await.expect("applies");
    let card = drawn(&running).await.tv.roleplay.expect("a roleplay card");
    assert_eq!(card.bubbles[0].speaker, "Kael");
    let refused = run(&running, "scene pill set party 7/6")
        .await
        .expect_err("is refused");
    assert!(
        refused.to_string().contains("cannot have joined"),
        "{refused}"
    );
    let lines = audit_lines(&running).await;
    assert!(
        lines
            .iter()
            .any(|line| line.contains("component=pill field=set party=4/6")),
        "{lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("component=roleplay field=set")),
        "{lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("component=loading field=set title=set percent=40")),
        "{lines:?}"
    );
    run(&running, "scene reset").await.expect("resets");
    let after = drawn(&running).await;
    assert!(after.tv.pills.is_none() && after.tv.loading.is_none() && after.tv.roleplay.is_none());
    let response = get(running.port, "/playground/controls.json")
        .await
        .expect("answers");
    let reply: ControlsReply = serde_json::from_str(&response.body).expect("controls");
    assert!(reply.scene.screens.lobby.is_none() && reply.scene.screens.pills.is_none());
}
