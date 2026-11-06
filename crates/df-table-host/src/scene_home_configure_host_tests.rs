//! The `scene home configure` verbs end to end: sent to a running host, applied as numbered
//! changes, drawn on top of the fixture, recorded in the audit log with the component and the
//! field and never the free text, refused with a reason that leaves the panel as it was, and the
//! settings-panel verb sequence run in order.

use df_playground::fixture::Fixture;
use df_playground::home::configure::{ConfigureData, Row};

use crate::client::get;
use crate::command::split_words;
use crate::dev::{DevError, run_dev};
use crate::test_support::{Running, STARTER_FILES, start};
use crate::wire::{ControlsReply, EventsReply};

const FIXTURE: &str = r#"{
    "name": "One",
    "tv": {"place": "Here", "status": {"kind": "ok"}, "elements": [], "caption": null},
    "phone": {"hero": "Pip", "status": {"kind": "ok"}}
}"#;

async fn run(running: &Running, line: &str) -> Result<String, DevError> {
    run_dev(running.port, &split_words(line).expect("the line splits")).await
}

async fn with_fixture(running: &Running) {
    running.write_asset("fixtures/ui_home.json", FIXTURE);
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

async fn panel(running: &Running) -> Option<ConfigureData> {
    drawn(running).await.tv.home.and_then(|home| home.configure)
}

async fn audit_lines(running: &Running) -> Vec<String> {
    let events = get(running.port, "/events.json").await.expect("answers");
    let events: EventsReply = serde_json::from_str(&events.body).expect("events");
    events.events.into_iter().map(|line| line.text).collect()
}

/// The settings-panel sequence the landing tour carried before the landing page became three
/// components: the panel arrives empty, then the preset selector, the six cards, one change at a
/// time to every kind of field, the remote over the cards, and the cards and rows coming and going.
/// It lives here so the verbs keep their end-to-end coverage. The banner pictures are left out,
/// which a host started for a test has no library for.
const SETTINGS_SEQUENCE: &str = r#"
scene home configure panel title="Configure Session" currency="$" layout=tiles
scene home configure preset label="Presets:" value="Balanced Adventure" options="Balanced Adventure,Rules Purist,Story First,Combat Heavy,Quick Play"
scene home configure card-add world kind=select label="World / Setting" icon=globe
scene home configure row-add world setting kind=select label="World / Setting" value="Arcane Sci-Fi" options="Medieval Fantasy,Arcane Sci-Fi,Steampunk,Post-Apocalyptic,Techno-Fantasy,Custom"
scene home configure card-add tone kind=select label="Session Tone" icon=masks accent=violet
scene home configure row-add tone tone kind=select label="Session Tone" value=Heroic options="Heroic,Dark,Mysterious,Whimsical,Serious,Chaotic"
scene home configure card-add party kind=select label=Party icon=party accent=violet
scene home configure row-add party players kind=select label=Party value="4 Players (Local)" options="1 Player (Local),2 Players (Local),3 Players (Local),4 Players (Local),5 Players (Local),6 Players (Local)"
scene home configure row-add party session_length kind=select label="Session Length" value="~3-4 Hours" options="~1-2 Hours,~2-3 Hours,~3-4 Hours,~4+ Hours"
scene home configure card-add rules kind=sliders label="Rules & Gameplay" icon=swords
scene home configure row-add rules rules_fidelity kind=slider label="Rules Fidelity" level=80
scene home configure row-add rules combat_intensity kind=slider label="Combat Intensity" level=60
scene home configure row-add rules narrative_freedom kind=slider label="Narrative Freedom" level=70
scene home configure row-add rules exploration_depth kind=slider label="Exploration Depth" level=75
scene home configure row-add rules puzzle_complexity kind=slider label="Puzzle Complexity" level=50
scene home configure row-add rules difficulty kind=slider label=Difficulty level=65 unit=steps steps="Relaxed,Moderate,Challenging,Punishing"
scene home configure card-add narration kind=mixed label="AI & Narration" icon=core
scene home configure row-add narration narrator_voice kind=select label="Narrator Voice" value="Aria (Epic)" options="Aria (Epic),Vaelin (Epic),Brakka (Gruff)"
scene home configure row-add narration imagination kind=slider label="Imagination Level" level=80
scene home configure row-add narration response_speed kind=select label="Response Speed" value=Balanced options="Fast,Balanced,Thoughtful"
scene home configure row-add narration narration_style kind=select label="Narration Style" value=Cinematic options="Cinematic,Storybook,Terse"
scene home configure row-add narration memory_depth kind=slider label="Memory Depth" level=70 unit=steps steps="Short,Standard,Long,Extended"
scene home configure card-add budget kind=budget label="Session Budget" icon=coins accent=green
scene home configure row-add budget balance kind=stat label=Balance money=28.40 highlight=true
scene home configure row-add budget spend_rate kind=stat label="Spend Rate" per-hour=3.90 info=true
scene home configure row-add budget estimate kind=stat label="Estimated (2-3 hrs)" range=7.80-11.70
scene home configure row world setting value=Steampunk
scene home configure row world setting value="Arcane Sci-Fi"
scene home configure row tone tone value=Dark
scene home configure row tone tone value=Heroic
scene home configure row rules rules_fidelity level=30
scene home configure row rules rules_fidelity level=100
scene home configure row rules rules_fidelity level=80
scene home configure row narration narrator_voice value="Vaelin (Epic)"
scene home configure row narration narrator_voice value="Aria (Epic)"
scene home configure row budget balance money=21.50
scene home configure row budget balance money=28.40
scene home configure preset value="Story First"
scene home configure preset value="Balanced Adventure"
scene home configure card world label="World"
scene home configure card world label="World / Setting" accent=green
scene home configure card world accent=cyan
scene home configure card party icon=dice
scene home configure card party icon=party
scene home configure row budget balance no-value=true
scene home configure row budget balance money=28.40
scene home focus configure:preset
scene home focus configure:2
scene home focus configure:4
scene home focus configure:6
scene home focus none
scene home configure card-move budget 1
scene home configure card-move budget 6
scene home configure row-remove rules rules_fidelity
scene home configure row-add rules rules_fidelity kind=slider label="Rules Fidelity" level=80 at=1
scene home configure row-move rules narrative_freedom 1
scene home configure row-move rules narrative_freedom 3
scene home configure card-add extras kind=sliders label=Extras icon=sparkles
scene home configure row-add extras luck kind=slider label=Luck level=40
scene home configure card-remove extras
scene home configure panel title="Tonight's Table"
scene home configure panel title="Configure Session"
scene home configure preset-remove
scene home configure preset value="Balanced Adventure" label="Presets:"
"#;

/// The lines of the settings sequence, one verb each.
fn settings_lines() -> Vec<String> {
    SETTINGS_SEQUENCE
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

#[tokio::test]
async fn panel_changes_are_applied_numbered_drawn_and_audited_without_their_text() {
    let running = start(&STARTER_FILES).await;
    with_fixture(&running).await;
    let said = run(
        &running,
        "scene home configure panel title=\"A Secret Title\" layout=tiles",
    )
    .await
    .expect("makes the panel");
    assert!(said.contains("revision"), "{said}");
    run(
        &running,
        "scene home configure card-add rules kind=sliders label=\"Secret Rules\" icon=swords",
    )
    .await
    .expect("adds a card");
    run(
        &running,
        "scene home configure row-add rules fidelity kind=slider label=\"Secret Fidelity\" level=80",
    )
    .await
    .expect("adds a row");
    run(&running, "scene home configure row rules fidelity level=35")
        .await
        .expect("changes a row");
    let data = panel(&running).await.expect("a panel");
    assert_eq!(data.title, "A Secret Title");
    let Some(Row::Slider(slider)) = data.card("rules").and_then(|card| card.rows.first()) else {
        panic!("a slider row");
    };
    assert_eq!((slider.value, slider.read_out()), (35, "35%".to_owned()));
    let lines = audit_lines(&running).await;
    let configure: Vec<&String> = lines
        .iter()
        .filter(|line| line.contains("component=configure"))
        .collect();
    assert!(
        configure.iter().any(|line| line.contains("field=row")
            && line.contains("card=rules")
            && line.contains("row=fidelity")
            && line.contains("level=35")),
        "{configure:?}"
    );
    assert!(
        configure.iter().any(|line| line.contains("field=card_add")),
        "{configure:?}"
    );
    assert!(
        lines.iter().all(|line| !line.contains("Secret")),
        "an audit line never carries the words"
    );
}

#[tokio::test]
async fn a_refused_panel_change_says_why_and_changes_nothing() {
    let running = start(&STARTER_FILES).await;
    with_fixture(&running).await;
    let refused = run(&running, "scene home configure row rules fidelity level=1")
        .await
        .expect_err("there is no panel");
    assert!(
        refused.to_string().contains("no configure panel"),
        "{refused}"
    );
    run(
        &running,
        "scene home configure card-add world kind=select label=World",
    )
    .await
    .expect("makes the panel with a card");
    run(
        &running,
        "scene home configure row-add world setting kind=select label=World value=Arcane options=Arcane,Steampunk",
    )
    .await
    .expect("adds a row");
    run(
        &running,
        "scene home configure card-add rules kind=sliders label=Rules",
    )
    .await
    .expect("adds a card");
    run(
        &running,
        "scene home configure row-add rules fidelity kind=slider label=Fidelity level=80",
    )
    .await
    .expect("adds a row");
    let before = panel(&running).await;
    for (line, expected) in [
        (
            "scene home configure row rules fidelity level=120",
            "percent from 0 to 100",
        ),
        (
            "scene home configure row rules nothing level=1",
            "has no row 'nothing'",
        ),
        (
            "scene home configure row nothing fidelity level=1",
            "there is no card 'nothing'",
        ),
        (
            "scene home configure row rules fidelity value=High",
            "does not apply",
        ),
        (
            "scene home configure row world setting value=Dream",
            "not one of the options",
        ),
        (
            "scene home configure card world banner=missing.webp",
            "there is no picture 'missing.webp'",
        ),
        (
            "scene home configure card-add world kind=select label=Again",
            "already there",
        ),
        (
            "scene home configure card-add odd!key kind=select label=Odd",
            "is not a key",
        ),
        (
            "scene home configure card-move world 9",
            "a position is from 1 to 2",
        ),
        (
            "scene home configure row-add rules cost kind=stat label=Cost",
            "cannot hold",
        ),
        (
            "scene home configure row-add rules luck kind=slider label=Luck unit=steps",
            "at least 2",
        ),
        (
            "scene home configure row-remove rules nothing",
            "has no row",
        ),
        (
            "scene home configure card-remove nothing",
            "there is no card",
        ),
        ("scene home configure panel title=\" \"", "cannot be empty"),
        (
            "scene home configure row rules fidelity",
            "scene home configure",
        ),
    ] {
        let refused = run(&running, line).await.expect_err("is refused");
        assert!(refused.to_string().contains(expected), "{line}: {refused}");
    }
    assert_eq!(panel(&running).await, before);
}

#[tokio::test]
async fn the_settings_panel_sequence_builds_the_panel_in_order() {
    let running = start(&STARTER_FILES).await;
    with_fixture(&running).await;
    let lines = settings_lines();
    assert!(
        lines.len() > 60,
        "the sequence has the whole settings section: {}",
        lines.len()
    );
    let directory = tempfile::TempDir::new().expect("makes a directory");
    let path = directory.path().join("settings.txt");
    let script: String = lines.iter().map(|line| format!("{line}\n")).collect();
    std::fs::write(&path, script).expect("writes the script");
    let words = ["script".to_owned(), path.to_string_lossy().into_owned()];
    let said = run_dev(running.port, &words).await.expect("runs");
    assert!(said.ends_with("all ok"), "{said}");
    let home = drawn(&running).await.tv.home.expect("a landing page");
    assert_eq!(home.focus, None, "the sequence ends on the default focus");
    let data = home.configure.expect("a panel");
    data.check().expect("the panel is in range");
    let keys: Vec<&str> = data.cards.iter().map(|card| card.key.as_str()).collect();
    assert_eq!(
        keys,
        ["world", "tone", "party", "rules", "narration", "budget"]
    );
    let rows: Vec<usize> = data.cards.iter().map(|card| card.rows.len()).collect();
    assert_eq!(rows, [1, 1, 2, 6, 5, 3]);
    let presets = data.presets.as_ref().expect("the preset selector is back");
    assert_eq!(presets.value, "Balanced Adventure");
    // The budget card draws its three amounts, and nothing else on the card is made up.
    let budget = data.card("budget").expect("a card");
    assert_eq!(budget.drawn_rows().len(), 3);
}
