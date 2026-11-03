//! The `scene home` verbs of the landing page's brand mark, title block, bell, navigation rail and
//! action bar: parsed into typed changes, applied by the host as numbered changes, drawn on top
//! of the fixture, recorded in the audit log with the component and the field and never the free
//! text, and the shipped placement script run end to end.

use df_playground::fixture::Fixture;
use df_playground::home_change::{HomeEdit, NavEdit};
use df_playground::home_chrome::BrandSize;
use df_playground::home_glyph::{HomeGlyph, HomeIcon};
use df_playground::home_model::HomePart;
use df_playground::home_nav::ActionVariant;
use df_playground::overlay::Edit;
use df_playground::scene::SceneChange;

use crate::client::get;
use crate::command::{Command, CommandError, parse, split_words, usage_text};
use crate::dev::{DevError, run_dev};
use crate::test_support::{Running, STARTER_FILES, start};
use crate::wire::{ControlsReply, EventsReply};

const TOUR: &str = include_str!("../../../development/playground-scripts/landing-tour.txt");

const FIXTURE: &str = r#"{
    "name": "One",
    "tv": {"place": "Here", "status": {"kind": "ok"}, "elements": [], "caption": null,
           "pills": {"balance": "$28.40"}},
    "phone": {"hero": "Pip", "status": {"kind": "ok"}}
}"#;

fn edit(line: &str) -> Result<HomeEdit, CommandError> {
    let words = split_words(line).expect("the line splits");
    match parse(&words)? {
        Command::Scene(SceneChange::Home { edit }) => Ok(edit),
        other => panic!("expected a landing page change, got {other:?}"),
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
fn the_brand_title_and_bell_verbs_read_into_typed_edits() {
    let HomeEdit::Brand { brand } =
        edit("scene home brand wordmark=Flux size=display star=off").expect("reads")
    else {
        panic!("a brand edit");
    };
    assert_eq!(brand.wordmark.as_deref(), Some("Flux"));
    assert_eq!(brand.size, Some(BrandSize::Display));
    assert_eq!(brand.star, Some(false));
    let HomeEdit::Title { title } =
        edit(r#"scene home title text=DungeonFlux tagline="Forge a world." rule=on"#)
            .expect("reads")
    else {
        panic!("a title edit");
    };
    assert_eq!(title.title.as_deref(), Some("DungeonFlux"));
    assert_eq!(title.tagline, Edit::Set("Forge a world.".to_owned()));
    assert_eq!(title.rule, Some(true));
    let HomeEdit::Title { title } = edit("scene home title tagline=none").expect("reads") else {
        panic!("a title edit");
    };
    assert_eq!(title.tagline, Edit::Clear);
    let HomeEdit::Top { top } = edit("scene home top bell=on unread=3").expect("reads") else {
        panic!("a top edit");
    };
    assert_eq!((top.bell, top.unread), (Some(true), Edit::Set(3)));
    let HomeEdit::Top { top } = edit("scene home top unread=none").expect("reads") else {
        panic!("a top edit");
    };
    assert_eq!(top.unread, Edit::Clear);
    assert!(matches!(
        edit("scene home brand size=huge"),
        Err(CommandError::BadChoice { .. })
    ));
    assert!(matches!(
        edit("scene home top unread=many"),
        Err(CommandError::BadChoice { .. })
    ));
    assert!(matches!(
        edit("scene home title colour=red"),
        Err(CommandError::BadChoice { .. })
    ));
    assert!(matches!(
        edit("scene home brand star=on star=off"),
        Err(CommandError::DuplicateKey(_))
    ));
}

#[test]
fn the_navigation_verbs_are_numbered_from_one() {
    let HomeEdit::Nav { number, item } =
        edit("scene home nav 2 label=Campaigns glyph=campaigns active=true badge=2")
            .expect("reads")
    else {
        panic!("a nav edit");
    };
    assert_eq!(number, 2);
    assert_eq!(item.label.as_deref(), Some("Campaigns"));
    assert_eq!(item.glyph, Some(HomeIcon::Page(HomeGlyph::Campaigns)));
    assert_eq!(item.active, Some(true));
    assert_eq!(item.badge, Edit::Set("2".to_owned()));
    assert_eq!(
        edit("scene home nav remove 3"),
        Ok(HomeEdit::NavRemove { number: 3 })
    );
    assert_eq!(
        edit("scene home nav 1 badge=none"),
        Ok(HomeEdit::Nav {
            number: 1,
            item: NavEdit {
                badge: Edit::Clear,
                ..NavEdit::default()
            }
        })
    );
    assert!(matches!(
        edit("scene home nav first label=Home"),
        Err(CommandError::BadCount { .. })
    ));
    assert!(matches!(
        edit("scene home nav 1 glyph=banana"),
        Err(CommandError::BadChoice { .. })
    ));
    assert!(matches!(
        edit("scene home nav 1 active=maybe"),
        Err(CommandError::BadChoice { .. })
    ));
    assert!(matches!(
        edit("scene home nav"),
        Err(CommandError::Usage(_))
    ));
}

#[test]
fn the_action_verbs_are_numbered_from_one() {
    let HomeEdit::Action { number, button } = edit(
        r#"scene home actions 1 label="Continue Campaign" subtitle="The Ashen Citadel" glyph=play variant=primary enabled=true"#,
    )
    .expect("reads") else {
        panic!("an action edit");
    };
    assert_eq!(number, 1);
    assert_eq!(button.label.as_deref(), Some("Continue Campaign"));
    assert_eq!(button.subtitle, Edit::Set("The Ashen Citadel".to_owned()));
    assert_eq!(button.glyph, Edit::Set(HomeIcon::Page(HomeGlyph::Play)));
    assert_eq!(button.variant, Some(ActionVariant::Primary));
    let HomeEdit::Action { button, .. } =
        edit("scene home actions 2 glyph=none subtitle=none").expect("reads")
    else {
        panic!("an action edit");
    };
    assert_eq!((button.glyph, button.subtitle), (Edit::Clear, Edit::Clear));
    assert_eq!(
        edit("scene home actions remove 4"),
        Ok(HomeEdit::ActionRemove { number: 4 })
    );
    assert!(matches!(
        edit("scene home actions 1 variant=gold"),
        Err(CommandError::BadChoice { .. })
    ));
}

#[test]
fn the_focus_verb_names_an_element_and_none_returns_to_the_default() {
    assert_eq!(
        edit("scene home focus actions:2"),
        Ok(HomeEdit::Focus {
            id: Edit::Set("actions:2".to_owned())
        })
    );
    assert_eq!(
        edit("scene home focus none"),
        Ok(HomeEdit::Focus { id: Edit::Clear })
    );
    assert!(matches!(
        edit("scene home focus"),
        Err(CommandError::Usage(_))
    ));
    assert!(matches!(
        edit("scene home focus a b"),
        Err(CommandError::Usage(_))
    ));
}

#[test]
fn remove_and_clear_name_a_part_and_the_usage_lists_them() {
    assert_eq!(
        edit("scene home remove campaigns"),
        Ok(HomeEdit::Remove {
            part: HomePart::Campaigns
        })
    );
    assert_eq!(edit("scene home clear"), Ok(HomeEdit::Clear {}));
    let Err(CommandError::BadChoice { allowed, .. }) = edit("scene home remove sidebar") else {
        panic!("an unknown part is refused with the choices");
    };
    assert!(
        allowed.contains("brand") && allowed.contains("actions"),
        "{allowed}"
    );
    assert!(matches!(
        edit("scene home"),
        Err(CommandError::Usage(text)) if text.contains("nav <n>")
    ));
    assert!(matches!(
        edit("scene home clear everything"),
        Err(CommandError::Usage(_))
    ));
    let help = usage_text();
    assert!(
        help.contains("scene home ..."),
        "the help lists the landing page verbs"
    );
}

#[tokio::test]
async fn landing_page_changes_are_applied_numbered_drawn_and_audited_without_their_text() {
    let running = start(&STARTER_FILES).await;
    with_fixture(&running).await;
    let said = run(&running, "scene home title text=\"A Secret Title\"")
        .await
        .expect("sets the title");
    assert!(said.contains("revision"), "{said}");
    run(
        &running,
        "scene home nav 1 label=Home glyph=home active=true",
    )
    .await
    .expect("adds item 1");
    run(&running, "scene home nav 2 label=Campaigns glyph=campaigns")
        .await
        .expect("adds item 2");
    run(&running, "scene home nav 2 active=true badge=4")
        .await
        .expect("activates item 2");
    run(
        &running,
        "scene home actions 1 label=\"Quick Launch\" glyph=bolt",
    )
    .await
    .expect("adds a button");
    run(&running, "scene home top bell=on unread=120")
        .await
        .expect("a bell");
    let home = drawn(&running).await.tv.home.expect("a landing page");
    assert_eq!(home.title.expect("title").title, "A Secret Title");
    let rail = home.nav.expect("nav");
    assert_eq!(
        rail.items
            .iter()
            .map(|item| (item.label.as_str(), item.active))
            .collect::<Vec<_>>(),
        vec![("Home", false), ("Campaigns", true)]
    );
    assert_eq!(rail.items[1].badge.as_deref(), Some("4"));
    assert_eq!(home.actions.expect("actions").buttons.len(), 1);
    assert_eq!(home.top.expect("top").unread, Some(120));
    let lines = audit_lines(&running).await;
    let home_lines: Vec<&String> = lines.iter().filter(|line| line.contains("home")).collect();
    assert!(
        home_lines
            .iter()
            .any(|line| line.contains("component=nav") && line.contains("field=item")),
        "{home_lines:?}"
    );
    assert!(
        lines.iter().all(|line| !line.contains("Secret Title")),
        "an audit line never carries the words"
    );
    // The scene's pills are the scene's own: the landing page draws them, it does not copy them.
    assert_eq!(
        drawn(&running)
            .await
            .tv
            .pills
            .expect("pills")
            .balance
            .as_deref(),
        Some("$28.40")
    );
}

#[tokio::test]
async fn a_refused_landing_page_change_says_why_and_changes_nothing() {
    let running = start(&STARTER_FILES).await;
    with_fixture(&running).await;
    run(&running, "scene home nav 1 label=Home glyph=home")
        .await
        .expect("adds item 1");
    let before = drawn(&running).await;
    for (line, expected) in [
        ("scene home nav 5 label=Far", "next free number is 2"),
        ("scene home nav 2", "cannot be empty"),
        ("scene home brand wordmark=\" \"", "cannot be empty"),
        ("scene home actions remove 1", "no action bar"),
        ("scene home nav remove 9", "no navigation item number 9"),
        ("scene home focus \"Not An Id\"", "is not a focus id"),
    ] {
        let refused = run(&running, line).await.expect_err("is refused");
        assert!(refused.to_string().contains(expected), "{line}: {refused}");
    }
    assert_eq!(drawn(&running).await, before);
}

#[tokio::test]
async fn the_shipped_landing_script_assembles_the_page_in_order() {
    let running = start(&STARTER_FILES).await;
    running.write_asset("fixtures/ui_home.json", FIXTURE);
    get(running.port, "/playground/controls.json")
        .await
        .expect("answers");
    // The pauses are real time in the shipped script; the check runs the same lines without them.
    // Only the lines of the components this file covers: the other parts of the shipped script
    // (the adventure card, the campaign strip) are run by their own tests.
    let mine = |line: &&str| {
        let line = line.trim();
        line.starts_with("playground go")
            || line == "scene reset"
            || line.starts_with("scene pill")
            || line == "scene home clear"
            || ["brand", "title", "top", "nav", "actions", "focus", "remove"]
                .iter()
                .any(|verb| line.starts_with(&format!("scene home {verb}")))
    };
    let quick: String = TOUR
        .lines()
        .filter(mine)
        .map(|line| format!("{line}\n"))
        .collect();
    let directory = tempfile::TempDir::new().expect("makes a directory");
    let path = directory.path().join("tour.txt");
    std::fs::write(&path, quick).expect("writes the script");
    let words = ["script".to_owned(), path.to_string_lossy().into_owned()];
    let said = run_dev(running.port, &words).await.expect("runs");
    assert!(said.ends_with("all ok"), "{said}");
    let scene = drawn(&running).await.tv;
    let home = scene.home.expect("a landing page");
    assert_eq!(home.brand.expect("brand").wordmark, "DungeonFlux");
    assert_eq!(
        home.title.expect("title").tagline.as_deref(),
        Some("Forge a world. Gather your party. Begin the legend.")
    );
    let rail = home.nav.expect("nav");
    assert_eq!(rail.items.len(), 6);
    assert_eq!(rail.items.iter().filter(|item| item.active).count(), 1);
    assert_eq!(home.actions.expect("actions").buttons.len(), 4);
    let top = home.top.expect("top");
    assert_eq!((top.bell, top.unread), (true, Some(3)));
    assert_eq!(home.focus, None, "the script ends on the default focus");
    let pills = scene.pills.expect("pills");
    assert_eq!(pills.balance.as_deref(), Some("$28.40"));
    assert_eq!(pills.rate.as_deref(), Some("$3.90/hr"));
    assert!(pills.profile);
}
