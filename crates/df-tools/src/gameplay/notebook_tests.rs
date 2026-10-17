use super::notebook::{
    Kind, MAXIMUM_ENTRIES, MAXIMUM_NOTES_PER_PLAYER, MAXIMUM_SCENE_BYTES, MAXIMUM_TEXT_BYTES,
    MAXIMUM_TITLE_BYTES, NotebookError, NotebookLedger,
};
use df_types::MemberId;
use std::{fs, path::PathBuf, time::SystemTime};

// SRD 5.2.1: no page applies; these tests cover narrative storage and access, not game rules.
fn member(value: u8) -> MemberId {
    MemberId::from_bytes(&[value; 16]).unwrap()
}

fn test_directory() -> PathBuf {
    let tick = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    PathBuf::from("artifacts/tmp/campaign/luna6/V03")
        .join(format!("notebook-{}-{tick}", std::process::id()))
}

// SRD 5.2.1 p. N/A: this test covers narrative privacy and persistence, not a game rule.
#[test]
fn private_clue_is_owner_only_until_shared_and_survives_restart() {
    let directory = test_directory();
    fs::create_dir_all(&directory).unwrap();
    let notebook = NotebookLedger::open("seed", Vec::new(), Vec::new(), Some(&directory)).unwrap();
    let order = notebook
        .add_clue(
            member(1),
            "Brakka",
            "Private clue",
            "Keep Tomas warned.",
            "opening",
        )
        .unwrap()
        .unwrap();
    let second = notebook
        .add_clue(
            member(1),
            "Brakka",
            "Private clue",
            "The map shows a flooded alley.",
            "opening",
        )
        .unwrap()
        .unwrap();
    assert_ne!(order, second);

    assert_eq!(notebook.visible_to(member(1)).len(), 2);
    assert!(notebook.visible_to(member(2)).is_empty());
    assert!(notebook.share(member(2), order).unwrap().is_none());
    let shared = notebook.share(member(1), order).unwrap().unwrap();
    assert!(shared.order > order);

    let resumed = NotebookLedger::open("seed", Vec::new(), Vec::new(), Some(&directory)).unwrap();
    assert_eq!(resumed.visible_to(member(1)).len(), 2);
    assert_eq!(resumed.visible_to(member(2)).len(), 1);
    assert_eq!(resumed.visible_to(member(1))[0].order, shared.order);
    assert_eq!(resumed.visible_to(member(2))[0].owner, None);
    assert_eq!(resumed.visible_to(member(2))[0].scene, "opening");
    assert_ne!(resumed.visible_to(member(2))[0].order, order);
    fs::remove_dir_all(directory).unwrap();
}

// SRD 5.2.1 p. N/A: this test covers saved narrative notes, not a game rule.
#[test]
fn missing_notebook_file_starts_empty_and_party_notes_are_shared() {
    let directory = test_directory();
    fs::create_dir_all(&directory).unwrap();
    let notebook =
        NotebookLedger::open("old-stored-game", Vec::new(), Vec::new(), Some(&directory)).unwrap();
    assert!(notebook.visible_to(member(1)).is_empty());

    let order = notebook
        .add_note(member(1), "Brakka", "Tomas is a mapmaker.", "opening")
        .unwrap();
    let visible = notebook.visible_to(member(2));
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].kind, Kind::Note);
    assert_eq!(visible[0].order, order);
    assert_eq!(visible[0].author.as_deref(), Some("Brakka"));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn permanent_grievance_is_private_idempotent_and_persistent() {
    let directory = test_directory();
    fs::create_dir_all(&directory).unwrap();
    let notebook = NotebookLedger::open("seed", Vec::new(), Vec::new(), Some(&directory)).unwrap();

    let order = notebook
        .record_never_forgives(
            member(1),
            "Brakka",
            "Ulric",
            "Brakka threatened to break his shield.",
            "opening",
        )
        .unwrap()
        .unwrap();
    assert!(
        notebook
            .record_never_forgives(
                member(1),
                "Brakka",
                "Ulric",
                "Brakka threatened to break his shield.",
                "opening",
            )
            .unwrap()
            .is_none()
    );
    assert!(notebook.visible_to(member(2)).is_empty());
    assert!(notebook.public_entries().is_empty());

    let resumed = NotebookLedger::open("seed", Vec::new(), Vec::new(), Some(&directory)).unwrap();
    let visible = resumed.visible_to(member(1));
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].kind, Kind::Note);
    assert_eq!(visible[0].order, order);
    assert_eq!(visible[0].author.as_deref(), Some("Brakka"));
    assert!(visible[0].text.contains("will never forgive"));
    fs::remove_dir_all(directory).unwrap();
}

// SRD 5.2.1 p. N/A: the text bound is a product contract, not a game rule.
#[test]
fn notes_enforce_spoken_line_bytes_and_per_player_count() {
    let notebook = NotebookLedger::none();
    assert_eq!(
        notebook.add_note(
            member(1),
            "Brakka",
            &"a".repeat(MAXIMUM_TEXT_BYTES + 1),
            "opening"
        ),
        Err(NotebookError::Bound("text"))
    );
    for index in 0..MAXIMUM_NOTES_PER_PLAYER {
        notebook
            .add_note(member(1), "Brakka", &format!("note {index}"), "opening")
            .unwrap();
    }
    assert_eq!(
        notebook.add_note(member(1), "Brakka", "one too many", "opening"),
        Err(NotebookError::Bound("notes per player"))
    );
    assert!(
        notebook
            .add_note(member(2), "Sigrun", "another hero can write", "opening")
            .is_ok()
    );
}

// SRD 5.2.1 p. N/A: these are storage bounds, not game rules.
#[test]
fn entries_enforce_all_field_and_total_count_bounds() {
    let notebook = NotebookLedger::none();
    assert_eq!(
        notebook.add_fact(
            Kind::Person,
            &"T".repeat(MAXIMUM_TITLE_BYTES + 1),
            "A person.",
            "opening",
        ),
        Err(NotebookError::Bound("title"))
    );
    assert_eq!(
        notebook.add_fact(
            Kind::Person,
            "Tomas",
            "A person.",
            &"s".repeat(MAXIMUM_SCENE_BYTES + 1),
        ),
        Err(NotebookError::Bound("scene"))
    );
    for index in 0..MAXIMUM_ENTRIES {
        notebook
            .add_fact(
                Kind::Person,
                &format!("Person {index}"),
                "A person.",
                "opening",
            )
            .unwrap();
    }
    assert_eq!(
        notebook.add_fact(Kind::Place, "Too many", "A place.", "opening"),
        Err(NotebookError::Bound("entries"))
    );
}

// SRD 5.2.1 p. N/A: order and scene are narrative-memory metadata, not game rules.
#[test]
fn newest_visible_entries_come_first_and_each_retains_its_scene() {
    let notebook = NotebookLedger::none();
    let first = notebook
        .add_fact(
            Kind::Person,
            "Tomas Greaves",
            "Mapmaker at the tea house.",
            "opening",
        )
        .unwrap()
        .unwrap();
    let second = notebook
        .add_fact(
            Kind::Place,
            "Tin Street Tea House",
            "Tomas makes maps here.",
            "opening",
        )
        .unwrap()
        .unwrap();
    let visible = notebook.visible_to(member(1));
    assert_eq!(
        visible.iter().map(|entry| entry.order).collect::<Vec<_>>(),
        [second, first]
    );
    assert_eq!(visible[0].scene, "opening");
}

// SRD 5.2.1 p. N/A: this test follows the recorded conversation and asserts no game rule.
#[test]
fn recorded_errand_facts_and_private_clue_become_party_notebook_entries() {
    let directory = test_directory();
    fs::create_dir_all(&directory).unwrap();
    let mut inn_sheet = fact_sheet();
    inn_sheet.scene = Some("inn".to_owned());
    inn_sheet.facts.insert(
        "reward.line".to_owned(),
        "A silver ring if word arrives before dawn.".to_owned(),
    );
    let notebook = NotebookLedger::open(
        "seed",
        Vec::new(),
        vec![fact_sheet(), inn_sheet],
        Some(&directory),
    )
    .unwrap();
    let opening = scene(super::pack::SceneKey::Opening);
    notebook
        .observe_scene(member(1), "Brakka", &opening, "", false)
        .unwrap();

    let opening_entries = notebook.public_entries();
    assert!(opening_entries.iter().any(|entry| {
        entry.kind == Kind::Person
            && entry.title == "Jorun Greaves"
            && entry.text.contains("ferry clerk")
    }));
    assert!(opening_entries.iter().any(|entry| {
        entry.kind == Kind::Person
            && entry.title == "Tomas Greaves"
            && entry.text.contains("mapmaker")
            && entry.text.contains("Tin Street Tea House")
    }));
    assert!(
        opening_entries
            .iter()
            .any(|entry| { entry.kind == Kind::Place && entry.title == "Tin Street Tea House" })
    );
    assert!(
        opening_entries
            .iter()
            .any(|entry| { entry.kind == Kind::Task && entry.text.contains("before dawn") })
    );
    notebook
        .observe_scene(
            member(1),
            "Brakka",
            &scene(super::pack::SceneKey::Inn),
            "",
            false,
        )
        .unwrap();
    assert!(notebook.public_entries().iter().any(|entry| {
        entry.kind == Kind::Task
            && entry
                .text
                .contains("Next hook: A silver ring if word arrives before dawn.")
    }));

    let given_order = notebook
        .public_entries()
        .into_iter()
        .find(|entry| entry.kind == Kind::Task)
        .unwrap()
        .order;
    notebook.mark_task_status("accepted").unwrap();
    let accepted_order = notebook
        .public_entries()
        .into_iter()
        .find(|entry| entry.kind == Kind::Task)
        .unwrap()
        .order;
    assert_eq!(accepted_order, given_order);
    assert!(
        notebook
            .public_entries()
            .iter()
            .any(|entry| entry.kind == Kind::Task && entry.text.starts_with("Status: given\n"))
    );
    let choice_scene = scene(super::pack::SceneKey::DialogueSeal);
    notebook
        .observe_scene(
            member(1),
            "Brakka",
            &choice_scene,
            "Jorun whispers that the warning is for Tomas Greaves at the Tin Street Tea House.",
            false,
        )
        .unwrap();
    let clue = notebook
        .visible_to(member(1))
        .into_iter()
        .find(|entry| entry.kind == Kind::Clue)
        .unwrap();
    assert!(
        notebook
            .visible_to(member(2))
            .iter()
            .all(|entry| entry.kind != Kind::Clue)
    );
    let shared = notebook.share(member(1), clue.order).unwrap().unwrap();
    assert!(
        notebook
            .visible_to(member(1))
            .iter()
            .all(|entry| entry.order != clue.order)
    );
    assert!(
        notebook
            .visible_to(member(2))
            .iter()
            .any(|entry| entry.kind == Kind::Clue && entry.order == shared.order)
    );

    notebook
        .add_note(
            member(1),
            "Brakka",
            "Tomas is a mapmaker at the tea house; Kelbin wants the letter.",
            "dialogue_seal",
        )
        .unwrap();
    assert!(
        notebook
            .visible_to(member(2))
            .iter()
            .any(|entry| entry.kind == Kind::Note)
    );
    fs::remove_dir_all(directory).unwrap();
}

// SRD 5.2.1 p. N/A: task status is narrative memory, not a game rule.
#[test]
fn task_status_updates_through_given_done_and_failed() {
    let notebook = NotebookLedger::none();
    notebook
        .add_fact(
            Kind::Task,
            "The errand",
            "Jorun asked the party to warn Tomas.",
            "opening",
        )
        .unwrap();

    for (status, recorded) in [
        ("accepted", "given"),
        ("done", "done"),
        ("failed", "failed"),
    ] {
        notebook.mark_task_status(status).unwrap();
        let task = notebook
            .public_entries()
            .into_iter()
            .find(|entry| entry.kind == Kind::Task)
            .unwrap();
        assert!(task.text.starts_with(&format!("Status: {recorded}\n")));
    }
}

// SRD 5.2.1 p. N/A: notebook entries are narrative memory, not a game rule.
#[test]
fn notebook_entries_use_scene_titles_and_keep_a_private_clue_once_across_scenes() {
    let notebook = NotebookLedger::none();
    let mut opening = scene(super::pack::SceneKey::Opening);
    opening.title = "At the Ferry Desk".to_owned();
    notebook
        .observe_scene(
            member(1),
            "Brakka",
            &opening,
            "The key is beneath the bell.",
            false,
        )
        .unwrap();

    let mut clue_scene = scene(super::pack::SceneKey::DialogueSeal);
    clue_scene.title = "The Sealed Warning".to_owned();
    notebook
        .observe_scene(
            member(1),
            "Brakka",
            &clue_scene,
            "The key is beneath the bell.",
            false,
        )
        .unwrap();

    let entries = notebook.visible_to(member(1));
    let clues = entries
        .iter()
        .filter(|entry| entry.kind == Kind::Clue)
        .collect::<Vec<_>>();
    assert_eq!(clues.len(), 1);
    assert_eq!(clues[0].scene, "At the Ferry Desk");
    let task = entries
        .iter()
        .find(|entry| entry.kind == Kind::Task)
        .unwrap();
    assert_eq!(task.scene, "At the Ferry Desk");
}

// SRD 5.2.1 p. N/A: this test checks notebook fact identity, not a game rule.
#[test]
fn a_location_short_name_does_not_create_a_second_place_entry() {
    let mut sheet = fact_sheet();
    sheet
        .facts
        .insert("loc.name".to_owned(), "the Lamplit Market".to_owned());
    sheet
        .facts
        .insert("loc.short".to_owned(), "the market".to_owned());
    sheet
        .facts
        .insert("place_short".to_owned(), "the market".to_owned());
    sheet.facts.insert(
        "place_detail".to_owned(),
        "You leave your meeting place for the market.".to_owned(),
    );
    sheet.must_mention = vec!["the Lamplit Market".to_owned(), "the market".to_owned()];
    let notebook = NotebookLedger::open("seed", Vec::new(), vec![sheet], None).unwrap();

    notebook
        .observe_scene(
            member(1),
            "Brakka",
            &scene(super::pack::SceneKey::Opening),
            "",
            false,
        )
        .unwrap();

    let places = notebook
        .public_entries()
        .into_iter()
        .filter(|entry| entry.kind == Kind::Place)
        .collect::<Vec<_>>();
    assert_eq!(places.len(), 1);
    assert_eq!(places[0].title, "the Lamplit Market");
    assert_eq!(places[0].text, "Also known as the market.");
}

// SRD 5.2.1 p. N/A: task status records narrative outcome, not a game rule.
#[test]
fn a_defeat_marks_the_errand_failed_even_after_a_done_action() {
    let notebook = NotebookLedger::none();
    notebook
        .observe_scene(
            member(1),
            "Brakka",
            &scene(super::pack::SceneKey::Opening),
            "",
            false,
        )
        .unwrap();
    notebook.mark_task_status("done").unwrap();

    let mut defeat = scene(super::pack::SceneKey::Defeat);
    defeat.title = "The Road Goes Dark".to_owned();
    notebook
        .observe_scene(member(1), "Brakka", &defeat, "", true)
        .unwrap();

    let task = notebook
        .public_entries()
        .into_iter()
        .find(|entry| entry.kind == Kind::Task)
        .unwrap();
    assert!(task.text.starts_with("Status: failed\n"));
    assert_eq!(task.scene, "Jorun's Quiet Warning");
}

// SRD 5.2.1 p. N/A: this test checks person classification, not a game rule.
#[test]
fn a_beast_antagonist_is_not_added_to_the_people_list() {
    let mut sheet = fact_sheet();
    sheet
        .facts
        .insert("ant.full".to_owned(), "The Shaggy Giant Weasel".to_owned());
    sheet
        .facts
        .insert("ant.short".to_owned(), "The Shaggy Giant Weasel".to_owned());
    sheet
        .facts
        .insert("ant.group".to_owned(), "beast".to_owned());
    sheet
        .must_mention
        .push("The Shaggy Giant Weasel".to_owned());
    let mut opening = scene(super::pack::SceneKey::Opening);
    opening.cast.push(super::pack::SceneCastMember {
        id: "giant-weasel".to_owned(),
        name: "Shaggy Giant Weasel".to_owned(),
        role: super::pack::CastRole::Antagonist,
        sprite: String::new(),
        portrait: String::new(),
        voice: String::new(),
        gender: None,
        age: None,
    });
    let notebook = NotebookLedger::open("seed", Vec::new(), vec![sheet], None).unwrap();

    notebook
        .observe_scene(member(1), "Brakka", &opening, "", false)
        .unwrap();

    assert!(notebook.public_entries().iter().all(|entry| {
        entry.kind != Kind::Person || !entry.title.to_lowercase().contains("weasel")
    }));
}

fn scene(key: super::pack::SceneKey) -> super::pack::Scene {
    super::pack::Scene {
        source_variant_id: None,
        key,
        title: "Jorun's Quiet Warning".to_owned(),
        narration: String::new(),
        rules_note: String::new(),
        cast: vec![super::pack::SceneCastMember {
            id: "ferry-clerk".to_owned(),
            name: "Jorun Greaves".to_owned(),
            role: super::pack::CastRole::Patron,
            sprite: String::new(),
            portrait: String::new(),
            voice: String::new(),
            gender: None,
            age: None,
        }],
        narration_rested: None,
        speaker: "Jorun Greaves".to_owned(),
        dialogue: "Please warn Tomas before dawn. If the letter never reaches him, we may lose our only chance. I can't leave the ferry desk.".to_owned(),
        asset: String::new(),
        setting: None,
        speaker_portrait: String::new(),
        findings: Vec::new(),
    }
}

fn fact_sheet() -> super::adventure::FactSheet {
    super::adventure::FactSheet {
        scene: Some("opening".to_owned()),
        beat: "opening".to_owned(),
        slot: "dialogue".to_owned(),
        speaker: "Jorun Greaves".to_owned(),
        voice: String::new(),
        delivery: String::new(),
        facts: std::collections::BTreeMap::from([
            ("patron.full".to_owned(), "Jorun Greaves".to_owned()),
            ("patron.first".to_owned(), "Jorun".to_owned()),
            ("patron.role".to_owned(), "ferry clerk".to_owned()),
            ("setting.meeting.name".to_owned(), "Ferry Desk".to_owned()),
            ("contact.full".to_owned(), "Tomas Greaves".to_owned()),
            ("contact.first".to_owned(), "Tomas".to_owned()),
            ("contact.role".to_owned(), "mapmaker".to_owned()),
            (
                "setting.destination.name".to_owned(),
                "Tin Street Tea House".to_owned(),
            ),
            ("loc.name".to_owned(), "Tin Street Tea House".to_owned()),
            ("place_short".to_owned(), "Tin Street Tea House".to_owned()),
            (
                "place_detail".to_owned(),
                "Tomas makes maps here.".to_owned(),
            ),
        ]),
        must_mention: vec![
            "Jorun".to_owned(),
            "Tomas".to_owned(),
            "Tin Street Tea House".to_owned(),
        ],
        forbidden: Vec::new(),
        bounds: 256,
        baseline: String::new(),
        tone: "restrained".to_owned(),
    }
}

// SRD 5.2.1 p. N/A: place aliases identify the same authored place.
#[test]
fn the_current_setting_short_name_and_destination_are_one_place() {
    let adventure = super::npc_dossier_tests::adventure(0x97);
    let mut setting = adventure.settings.destination.clone();
    setting.name = "the Binding Workshop".to_owned();
    setting.short = "the bindery".to_owned();
    let mut current = scene(super::pack::SceneKey::Opening);
    current.setting = Some(setting);
    let mut sheet = fact_sheet();
    sheet
        .facts
        .insert("place_short".to_owned(), "the bindery".to_owned());
    sheet
        .facts
        .insert("dest.name".to_owned(), "the Binding Workshop".to_owned());
    sheet.must_mention = vec!["the bindery".to_owned(), "the Binding Workshop".to_owned()];
    let notebook = NotebookLedger::open("seed", Vec::new(), vec![sheet], None).unwrap();
    notebook
        .observe_scene(member(1), "Brakka", &current, "", false)
        .unwrap();
    let places = notebook
        .public_entries()
        .into_iter()
        .filter(|entry| entry.kind == Kind::Place)
        .collect::<Vec<_>>();
    assert_eq!(places.len(), 1);
    assert_eq!(places[0].title, "the Binding Workshop");
}

// SRD 5.2.1 p. N/A: a future story hook is not a payment.
#[test]
fn the_next_hook_is_not_recorded_as_a_paid_reward() {
    let mut inn = fact_sheet();
    inn.scene = Some("inn".to_owned());
    inn.facts.insert(
        "reward.line".to_owned(),
        "There is talk of trouble farther along.".to_owned(),
    );
    let notebook = NotebookLedger::open("seed", Vec::new(), vec![fact_sheet(), inn], None).unwrap();
    notebook
        .observe_scene(
            member(1),
            "Brakka",
            &scene(super::pack::SceneKey::Opening),
            "",
            false,
        )
        .unwrap();
    notebook
        .observe_scene(
            member(1),
            "Brakka",
            &scene(super::pack::SceneKey::Inn),
            "",
            false,
        )
        .unwrap();
    let task = notebook
        .public_entries()
        .into_iter()
        .find(|entry| entry.kind == Kind::Task)
        .unwrap();
    assert!(
        task.text
            .contains("Next hook: There is talk of trouble farther along.")
    );
    assert!(!task.text.lines().any(|line| line.starts_with("Reward:")));
}

// SRD 5.2.1 p. N/A: payment status mirrors the committed payment marker.
#[test]
fn the_canonical_coin_reward_is_offered_then_paid_once_and_survives_reload() {
    let directory = test_directory();
    fs::create_dir_all(&directory).unwrap();
    let notebook = NotebookLedger::open("seed", Vec::new(), Vec::new(), Some(&directory)).unwrap();
    notebook
        .add_fact(
            Kind::Task,
            "The errand",
            "Status: done\nWarn Tomas.\nNext hook: Trouble farther along.",
            "opening",
        )
        .unwrap();
    assert!(notebook.sync_task_reward(14, false).unwrap());
    let task = notebook
        .public_entries()
        .into_iter()
        .find(|entry| entry.kind == Kind::Task)
        .unwrap();
    assert!(
        task.text
            .contains("Reward: 14 GP per hero (offered; not paid).")
    );
    assert!(notebook.sync_task_reward(14, true).unwrap());
    let paid = notebook
        .public_entries()
        .into_iter()
        .find(|entry| entry.kind == Kind::Task)
        .unwrap();
    assert!(paid.text.contains("Reward: 14 GP per hero (paid)."));
    assert!(paid.text.contains("Next hook: Trouble farther along."));
    assert!(!notebook.sync_task_reward(14, true).unwrap());
    assert_eq!(notebook.public_entries()[0].order, paid.order);
    let resumed = NotebookLedger::open("seed", Vec::new(), Vec::new(), Some(&directory)).unwrap();
    assert!(!resumed.sync_task_reward(14, true).unwrap());
    assert_eq!(resumed.public_entries()[0], paid);
    fs::remove_dir_all(directory).unwrap();
}

// SRD 5.2.1 p. N/A: refresh corrects an existing authored place alias.
#[test]
fn an_existing_place_alias_is_merged_on_scene_refresh() {
    let adventure = super::npc_dossier_tests::adventure(0x97);
    let mut setting = adventure.settings.destination.clone();
    setting.name = "the Binding Workshop".to_owned();
    setting.short = "the bindery".to_owned();
    let mut current = scene(super::pack::SceneKey::Opening);
    current.setting = Some(setting);
    let notebook = NotebookLedger::none();
    notebook
        .add_fact(
            Kind::Place,
            "the bindery",
            "Books are bound here.",
            "opening",
        )
        .unwrap();
    notebook
        .add_fact(
            Kind::Place,
            "the Binding Workshop",
            "Tomas works here.",
            "opening",
        )
        .unwrap();
    notebook
        .observe_scene(member(1), "Brakka", &current, "", false)
        .unwrap();
    let places = notebook
        .public_entries()
        .into_iter()
        .filter(|entry| entry.kind == Kind::Place)
        .collect::<Vec<_>>();
    assert_eq!(places.len(), 1);
    assert_eq!(places[0].title, "the Binding Workshop");
    assert_eq!(places[0].text, "Tomas works here.");
}

// SRD 5.2.1 p. N/A: this test covers the shared party notebook projection, not a game rule.
#[test]
fn a_party_note_from_one_member_reaches_every_members_projected_notebook() {
    use df_protocol::common as rpc;

    let notebook = NotebookLedger::none();
    notebook
        .add_note(
            member(1),
            "Brakka",
            "Marek is out of traveler's clothes.",
            "opening",
        )
        .unwrap();
    assert!(
        notebook
            .public_entries()
            .iter()
            .any(|entry| entry.kind == Kind::Note)
    );
    for viewer in [member(1), member(2)] {
        let mut view = rpc::ViewMessage {
            audience: Some(rpc::view_message::Audience::Player(
                rpc::PlayerGameplayView {
                    journey: Some(rpc::JourneyView::default()),
                    ..Default::default()
                },
            )),
            ..Default::default()
        };
        super::wire::attach_notebook(&mut view, &notebook.visible_to(viewer)).unwrap();
        let Some(rpc::view_message::Audience::Player(player)) = view.audience else {
            panic!("a player view is expected");
        };
        let notes = player
            .journey
            .unwrap()
            .notebook
            .into_iter()
            .filter(|entry| entry.kind == rpc::NotebookKind::Note as i32)
            .collect::<Vec<_>>();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].title, "Party note");
        assert_eq!(notes[0].text, "Marek is out of traveler's clothes.");
        assert_eq!(notes[0].author.as_deref(), Some("Brakka"));
        assert_eq!(notes[0].owner, None);
    }
}
