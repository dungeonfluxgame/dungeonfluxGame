//! Perception over a scripted text backend: the facts each character is asked from (the picture
//! attached, the scan with strangers as handles, bounds), the checks of every answer field, the
//! retry, the state path of a feeling, the scene call over many characters, the vision-off path,
//! the kept thoughts and the command line.

use std::sync::Arc;

use df_content::persona::Persona;
use df_scene_layout::region_id;
use serde_json::{Value, json};

use super::fixtures::{CHARACTER, ScriptedHost, scripted_host_cast};
use super::perceive::cli::{PerceiveCommand, format_perceived, parse_perceive, parse_thoughts};
use super::perceive::request::{
    Attended, Mode, PerceiveCause, PerceiveReply, PerceiveRouteError, Perceived, SceneOutcome,
    ScenePerceived, Thoughts, Viewpoint,
};
use super::perceive::routes::path;
use super::state::load_state;
use super::storage::write_persona;
use crate::dev_client::{get_json, post_json};
use crate::dev_http::DevReply;
use crate::speech::fixtures::FakePort;

fn words(text: &str) -> Vec<String> {
    text.split_whitespace().map(str::to_owned).collect()
}

/// A character of the scene with a look in her job, so a stranger can be said by it.
fn person(id: &str, display: &str, job: &str) -> Persona {
    serde_json::from_value(json!({
        "schema": 1, "id": id, "display_name": display, "job": job,
        "personality": ["watchful"],
        "speech_style": "Plain and quick.",
        "backstory": "Has walked these roads for years.",
        "private_backstory": "Owes a debt to the guild.",
        "secrets": [{
            "id": "debt", "text": "Owes the guild a great deal.", "sensitivity": "high",
            "reveal_conditions": ["asked about money by someone she trusts"], "known_by": []
        }]
    }))
    .unwrap()
}

fn region_of(name: &str) -> String {
    region_id(name.as_bytes()).as_str().to_owned()
}

/// A valid answer: one thing seen (the region of `seen`), attention on it, a feeling.
fn answer(seen: &str, thought: &str) -> Value {
    json!({
        "sees": [{
            "what": format!("{seen} by the road"),
            "region": region_of(seen), "item": null, "salience": 80
        }],
        "thinking_about": thought,
        "attention": {"region": region_of(seen), "item": null},
        "feeling": {"emotion": "joy", "delta": 20},
        "wants": ["say hello"],
        "new_things": [],
        "leaks_private_side": false, "leaked_excerpt": null
    })
}

/// A host showing `cast`, with a persona stored for each of `with_personas`.
async fn host_with(cast: &[&str], with_personas: &[&str], answers: Vec<Value>) -> ScriptedHost {
    let host = scripted_host_cast(cast, answers, Arc::new(FakePort::unconfigured())).await;
    for name in with_personas {
        write_persona(
            &host.directory(),
            &person(
                name,
                &format!("Display {name}"),
                &format!("Role: a figure like {name}"),
            ),
        )
        .unwrap();
    }
    host
}

async fn host_of(cast: &[&str], answers: Vec<Value>) -> ScriptedHost {
    host_with(cast, cast, answers).await
}

async fn perceive(host: &ScriptedHost, body: Value) -> PerceiveReply<Perceived> {
    post_json(host.port, path::PERSONA_PERCEIVE, &body)
        .await
        .unwrap()
}

fn done<T: std::fmt::Debug>(reply: PerceiveReply<T>) -> T {
    match reply {
        DevReply::Done { value } => value,
        DevReply::Rejected { error, message } => panic!("refused: {error:?} {message}"),
    }
}

fn refused<T: std::fmt::Debug>(reply: PerceiveReply<T>) -> PerceiveRouteError {
    match reply {
        DevReply::Done { value } => panic!("not refused: {value:?}"),
        DevReply::Rejected { error, .. } => error,
    }
}

/// The cause an answer is refused for, asked twice (the retry) with the same bad answer.
async fn cause_of(bad: Value) -> PerceiveCause {
    let host = host_of(&[CHARACTER], vec![bad.clone(), bad]).await;
    match refused(perceive(&host, json!({"who": CHARACTER})).await) {
        PerceiveRouteError::AnswerRefused { cause, .. } => cause,
        other => panic!("not an answer refusal: {other:?}"),
    }
}

#[tokio::test]
async fn the_model_is_given_the_picture_and_the_scan_and_the_answer_is_kept_as_a_thought() {
    let host = host_of(
        &[CHARACTER, "wren"],
        vec![answer("wren", "She looks tired.")],
    )
    .await;
    let seen = done(
        perceive(
            &host,
            json!({"who": CHARACTER, "recent": ["wren: Morning."], "mode": "study"}),
        )
        .await,
    );
    assert!(seen.vision);
    assert_eq!(seen.picture.as_deref(), Some("plate.png"));
    assert_eq!(seen.regions, 2);
    assert_eq!(seen.mode, Mode::Study);
    assert_eq!(
        seen.viewpoint,
        Viewpoint::Pictured {
            region: region_of(CHARACTER)
        }
    );
    assert_eq!(
        seen.attention,
        Attended::Region {
            id: region_of("wren")
        }
    );
    assert_eq!(seen.thinking_about, "She looks tired.");
    assert_eq!(seen.wants, vec!["say hello".to_owned()]);
    // The picture itself went to the model, labelled.
    let pictures = host
        .asked
        .lock()
        .unwrap()
        .iter()
        .map(|asked| asked.images.clone())
        .collect::<Vec<_>>();
    assert_eq!(pictures.len(), 1);
    assert_eq!(pictures[0].len(), 1);
    assert_eq!(pictures[0][0].bytes, b"plate.png");
    assert_eq!(pictures[0][0].label, "The screen now");
    let facts = host.facts_of(0);
    assert_eq!(facts["now"]["picture"]["attached"], json!(true));
    assert_eq!(facts["now"]["viewpoint"], json!("in_picture"));
    let scan = facts["now"]["scan"].as_array().unwrap();
    assert_eq!(scan[0]["who"], json!("you"));
    assert_eq!(scan[0]["id"], json!(region_of(CHARACTER)));
    // The thought is a memory of kind thought, owned by the character.
    let kept = crate::world::memory::recent(&host.running.state, CHARACTER, "thought", 10)
        .await
        .unwrap();
    assert_eq!(kept.len(), 1);
    assert_eq!(Some(&kept[0].id), seen.thought.as_ref());
    assert!(kept[0].description.starts_with("She looks tired."));
    assert_eq!(kept[0].pictures, vec!["plate.png".to_owned()]);
    let line = host
        .lines_starting_with("PERSONA perceived who=")
        .pop()
        .expect("an audit line");
    for part in [
        "who=innkeeper",
        "vision=true",
        "regions=2",
        "attention=region:",
        "feeling=joy:+20",
        "viewpoint=pictured",
    ] {
        assert!(line.contains(part), "{part} in {line}");
    }
    assert!(!line.contains("She looks tired"), "{line}");
}

#[tokio::test]
async fn a_stranger_is_a_handle_with_a_look_and_comes_back_as_the_look() {
    let mut said = answer("wren_smith", "I think person-1 is waiting.");
    said["sees"][0]["what"] = json!("person-1 stands at the gate");
    let host = host_of(&[CHARACTER, "wren_smith"], vec![said]).await;
    write_persona(
        &host.directory(),
        &person("wren_smith", "Wren Smith", "Courier: a tall woman in grey"),
    )
    .unwrap();
    let seen = done(perceive(&host, json!({"who": CHARACTER})).await);
    let facts = serde_json::to_string(&host.facts_of(0)).unwrap();
    assert!(!facts.contains("wren_smith"), "{facts}");
    assert!(!facts.contains("Wren Smith"), "{facts}");
    assert!(!facts.contains("Wren"), "{facts}");
    assert!(facts.contains("person-1"));
    assert_eq!(
        seen.sees[0].what,
        "the tall woman in grey stands at the gate"
    );
    assert_eq!(
        seen.thinking_about,
        "I think the tall woman in grey is waiting."
    );
}

#[tokio::test]
async fn a_name_the_character_has_learned_is_the_id_they_call_the_person_by() {
    let mut said = answer("wren_smith", "Wren is here.");
    said["sees"][0]["what"] = json!("wren_smith at the gate");
    let host = host_of(&[CHARACTER, "wren_smith"], vec![said]).await;
    host.running.write_asset(
        "persona-state/innkeeper.json",
        r#"{"id": "innkeeper", "known_names": ["wren_smith"]}"#,
    );
    let seen = done(perceive(&host, json!({"who": CHARACTER})).await);
    let facts = serde_json::to_string(&host.facts_of(0)).unwrap();
    assert!(facts.contains("wren_smith"), "{facts}");
    let scan = host.facts_of(0)["now"]["scan"].clone();
    assert_eq!(scan[1]["who"], json!("wren_smith"));
    assert_eq!(seen.sees[0].what, "wren_smith at the gate");
}

#[tokio::test]
async fn every_field_of_an_answer_is_checked_and_a_refusal_changes_nothing() {
    let with = |edit: &dyn Fn(&mut Value)| {
        let mut said = answer(CHARACTER, "A thought.");
        edit(&mut said);
        said
    };
    assert_eq!(
        cause_of(with(&|said| said["sees"][0]["region"] = json!("rffffffff"))).await,
        PerceiveCause::UnknownRegion {
            id: "rffffffff".to_owned()
        }
    );
    assert_eq!(
        cause_of(with(&|said| said["sees"][0]["item"] = json!("lute"))).await,
        PerceiveCause::UnknownItem {
            id: "lute".to_owned()
        }
    );
    assert_eq!(
        cause_of(with(&|said| said["feeling"]["emotion"] = json!("wrath"))).await,
        PerceiveCause::UnknownEmotion {
            emotion: "wrath".to_owned()
        }
    );
    assert!(matches!(
        cause_of(with(&|said| said["attention"]["item"] = json!("lute"))).await,
        PerceiveCause::BadAttention { .. }
    ));
    assert!(matches!(
        cause_of(with(
            &|said| said["attention"] = json!({"region": "rffffffff", "item": null})
        ))
        .await,
        PerceiveCause::BadAttention { .. }
    ));
    assert_eq!(
        cause_of(with(&|said| said["sees"][0]["salience"] = json!(101))).await,
        PerceiveCause::BadSalience { salience: 101 }
    );
    assert_eq!(
        cause_of(with(&|said| said["thinking_about"] = json!("  "))).await,
        PerceiveCause::EmptyThought
    );
    // A stranger with no persona has no look to be said by, so a handle for them is refused.
    let mut handled = answer(CHARACTER, "A thought.");
    handled["wants"] = json!(["ask person-1 in"]);
    let host = host_with(
        &[CHARACTER, "mute"],
        &[CHARACTER],
        vec![handled.clone(), handled],
    )
    .await;
    match refused(perceive(&host, json!({"who": CHARACTER})).await) {
        PerceiveRouteError::AnswerRefused { cause, .. } => {
            assert_eq!(cause, PerceiveCause::HandleInText);
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        cause_of(with(&|said| {
            said.as_object_mut().unwrap().remove("wants");
        }))
        .await,
        PerceiveCause::Malformed { .. }
    ));
    // Nothing changed: no thought, no state, no state_applied line.
    let host = host_of(
        &[CHARACTER],
        vec![
            with(&|said| said["sees"][0]["region"] = json!("rffffffff")),
            with(&|said| said["sees"][0]["region"] = json!("rffffffff")),
        ],
    )
    .await;
    let error = refused(perceive(&host, json!({"who": CHARACTER})).await);
    assert_eq!(error.key(), "answer_refused");
    assert!(
        crate::world::memory::recent(&host.running.state, CHARACTER, "thought", 10)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        host.lines_starting_with("PERSONA state_applied").is_empty(),
        "{:?}",
        host.persona_lines()
    );
    let refusal = host.lines_starting_with("PERSONA refused verb=perceive");
    assert_eq!(refusal.len(), 1);
    assert!(refusal[0].contains("id=innkeeper"));
    assert!(refusal[0].contains("error=answer_refused"));
    assert!(refusal[0].contains("cause=unknown_region"));
}

#[tokio::test]
async fn a_refused_answer_is_asked_again_once_with_why() {
    let mut bad = answer(CHARACTER, "First try.");
    bad["sees"][0]["item"] = json!("lute");
    let host = host_of(&[CHARACTER], vec![bad, answer(CHARACTER, "Second try.")]).await;
    let seen = done(perceive(&host, json!({"who": CHARACTER})).await);
    assert_eq!(seen.thinking_about, "Second try.");
    assert_eq!(host.calls(), 2);
    assert!(host.facts_of(0)["now"]["retry_reason"].is_null());
    let reason = host.facts_of(1)["now"]["retry_reason"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(reason.contains("lute"), "{reason}");
    assert!(
        host.lines_starting_with("PERSONA perceived who=")
            .pop()
            .unwrap()
            .contains("retried=true")
    );
}

#[tokio::test]
async fn a_feeling_goes_into_the_state_only_through_the_state_path() {
    let host = host_of(&[CHARACTER], vec![answer(CHARACTER, "A fine morning.")]).await;
    let before = load_state(&host.running.state, CHARACTER).level("joy");
    let seen = done(perceive(&host, json!({"who": CHARACTER})).await);
    assert_eq!(seen.feeling.as_ref().map(|feeling| feeling.delta), Some(20));
    assert!(load_state(&host.running.state, CHARACTER).level("joy") > before);
    assert_eq!(host.lines_starting_with("PERSONA state_applied").len(), 1);
}

#[tokio::test]
async fn without_vision_the_answer_is_marked_and_the_picture_is_not_sent() {
    let host = host_of(
        &[CHARACTER, "wren"],
        vec![
            answer("wren", "From the words only."),
            answer("wren", "With the picture."),
        ],
    )
    .await;
    let seen = done(
        perceive(
            &host,
            json!({"who": CHARACTER, "vision": false, "scene": "A road, a gate and a woman."}),
        )
        .await,
    );
    assert!(!seen.vision);
    assert!(host.asked.lock().unwrap()[0].images.is_empty());
    let facts = host.facts_of(0);
    assert_eq!(facts["now"]["picture"]["attached"], json!(false));
    assert_eq!(
        facts["now"]["picture"]["scene"],
        json!("A road, a gate and a woman.")
    );
    assert!(
        host.lines_starting_with("PERSONA perceived who=")
            .pop()
            .unwrap()
            .contains("vision=false")
    );
    // With the picture, the scene words are not sent.
    let seen = done(
        perceive(
            &host,
            json!({"who": CHARACTER, "scene": "A road, a gate and a woman."}),
        )
        .await,
    );
    assert!(seen.vision);
    assert!(host.facts_of(1)["now"]["picture"]["scene"].is_null());
    // A picture that is not in the library cannot be seen, and is not named by a thought.
    let mut nothing = answer(CHARACTER, "Nothing to see.");
    nothing["sees"] = json!([]);
    nothing["attention"] = json!({"region": null, "item": null});
    let host = host_of(&[CHARACTER], vec![nothing]).await;
    let seen = done(perceive(&host, json!({"who": CHARACTER, "picture": "gone.png"})).await);
    assert!(!seen.vision);
    assert_eq!(seen.picture, None);
    assert!(host.asked.lock().unwrap()[0].images.is_empty());
    assert!(seen.thought.is_some());
}

#[tokio::test]
async fn a_focus_must_be_a_region_or_an_item_in_sight() {
    let host = host_of(
        &[CHARACTER, "wren"],
        vec![answer("wren", "Looking at her.")],
    )
    .await;
    let focus = region_of("wren");
    done(perceive(&host, json!({"who": CHARACTER, "focus": focus})).await);
    assert_eq!(host.facts_of(0)["now"]["focus"], json!({"region": focus}));
    let error = refused(perceive(&host, json!({"who": CHARACTER, "focus": "lute"})).await);
    assert!(matches!(
        error,
        PerceiveRouteError::BadRequest {
            cause: PerceiveCause::UnknownFocus { .. }
        }
    ));
    let error = refused(perceive(&host, json!({"who": "ghost"})).await);
    assert!(matches!(
        error,
        PerceiveRouteError::BadRequest {
            cause: PerceiveCause::Persona { .. }
        }
    ));
}

/// A scan of `names`, seated across the picture in rows so every box is valid.
fn crowd_layout(names: &[String]) -> df_scene_layout::SceneLayout {
    use df_scene_layout::{PictureBinding, RegionGeometry, SceneLayout, SceneRegion};
    let binding = PictureBinding {
        name: "plate.png".to_owned(),
        sha256: df_imagery::oneshot_segment::picture_sha256(b"plate.png"),
        width_px: 100,
        height_px: 50,
    };
    let regions = names
        .iter()
        .zip(0_u16..)
        .map(|(name, seat)| {
            let left = (seat % 40) * 20;
            let top = (seat / 40) * 100;
            let geometry = RegionGeometry {
                left_permille: left,
                top_permille: top,
                right_permille: left + 15,
                bottom_permille: top + 90,
                head_x_permille: left + 7,
                head_y_permille: top,
            };
            SceneRegion::new(region_id(name.as_bytes()), geometry, 900)
                .unwrap()
                .with_label(Some(name.clone()))
        })
        .collect();
    SceneLayout::new(binding, "sam-3.1", "person", regions)
}

#[tokio::test]
async fn the_facts_of_a_big_scene_are_bounded() {
    let mut names: Vec<String> = (0..100).map(|number| format!("extra{number}")).collect();
    names.push(CHARACTER.to_owned());
    let host = host_with(
        &[CHARACTER],
        &[CHARACTER],
        vec![answer(CHARACTER, "So many people.")],
    )
    .await;
    crate::locate::write_layout(
        &host.running.playground_path("assets/layouts"),
        "plate.png",
        &crowd_layout(&names),
    )
    .unwrap();
    let things: Vec<Value> = (0..80)
        .map(|number| json!({"id": format!("thing{number}"), "name": "thing", "look": "a thing"}))
        .collect();
    host.running.write_asset(
        "world/bible.json",
        &json!({
            "items": [], "voice_packs": [],
            "places": [{"id": "square", "name": "Square", "description": "A square.",
                        "stalls": [], "things": things}]
        })
        .to_string(),
    );
    host.running
        .write_asset("world/state.json", r#"{"place": "square"}"#);
    let recent: Vec<String> = (0..100)
        .map(|number| format!("extra0: line {number}"))
        .collect();
    let seen = done(perceive(&host, json!({"who": CHARACTER, "recent": recent})).await);
    assert_eq!(seen.regions, 48);
    let facts = host.facts_of(0);
    let scan = facts["now"]["scan"].as_array().unwrap();
    assert_eq!(scan.len(), 48);
    // The character's own region is always listed, though it is the last of the scan.
    assert!(scan.iter().any(|region| region["who"] == json!("you")));
    assert_eq!(facts["now"]["picture"]["regions_left_out"], json!(53));
    assert_eq!(facts["now"]["recent"].as_array().unwrap().len(), 30);
    assert_eq!(facts["now"]["recent_lines_left_out"], json!(70));
    assert_eq!(facts["now"]["items_here"].as_array().unwrap().len(), 48);
    // The other people are handles, never ids.
    assert!(!facts.to_string().contains("extra5\""), "{facts}");
}

#[tokio::test]
async fn a_scene_call_asks_each_character_from_their_own_point_of_view_with_no_leaks() {
    let cast = ["anna", "bram", "cora"];
    let host = host_of(
        &cast,
        vec![
            answer("bram", "ANNA-THOUGHT bram is odd."),
            answer("cora", "BRAM-THOUGHT cora is odd."),
            answer("anna", "CORA-THOUGHT anna is odd."),
        ],
    )
    .await;
    let reply: PerceiveReply<ScenePerceived> = post_json(
        host.port,
        path::PERSONA_PERCEIVE_SCENE,
        &json!({"parallel": 1}),
    )
    .await
    .unwrap();
    let scene = done(reply);
    assert!(scene.vision);
    assert_eq!(scene.picture.as_deref(), Some("plate.png"));
    let who: Vec<&str> = scene.results.iter().map(|row| row.who.as_str()).collect();
    assert_eq!(who, ["anna", "bram", "cora"]);
    for row in &scene.results {
        assert!(
            matches!(row.outcome, SceneOutcome::Perceived { .. }),
            "{row:?}"
        );
    }
    // Each call was its own: their own persona in the facts, their own region as "you", and no
    // other character's thought in any of them.
    let asked = host.asked.lock().unwrap().clone();
    assert_eq!(asked.len(), 3);
    for (index, name) in cast.iter().enumerate() {
        let facts = &asked[index].facts;
        assert_eq!(facts["context"]["character"]["id"], json!(name));
        let you: Vec<&Value> = facts["now"]["scan"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|region| region["who"] == json!("you"))
            .collect();
        assert_eq!(you.len(), 1);
        assert_eq!(you[0]["id"], json!(region_of(name)));
        let text = facts.to_string();
        for thought in ["ANNA-THOUGHT", "BRAM-THOUGHT", "CORA-THOUGHT"] {
            assert!(!text.contains(thought), "{name}'s call holds {thought}");
        }
    }
    for name in cast {
        let kept = crate::world::memory::recent(&host.running.state, name, "thought", 10)
            .await
            .unwrap();
        assert_eq!(kept.len(), 1, "{name}");
    }
    let line = host
        .lines_starting_with("PERSONA perceived_scene")
        .pop()
        .unwrap();
    assert!(
        line.contains("characters=3 perceived=3 refused=0 parallel=1"),
        "{line}"
    );
}

#[tokio::test]
async fn a_scene_call_over_a_named_list_marks_a_viewer_and_one_refusal_stops_no_one() {
    let host = host_of(
        &["anna", "bram"],
        vec![answer("anna", "A first."), answer("bram", "A second.")],
    )
    .await;
    // dora is not in the picture; ghost has no persona.
    write_persona(
        &host.directory(),
        &person("dora", "Dora", "Role: a passer-by"),
    )
    .unwrap();
    let reply: PerceiveReply<ScenePerceived> = post_json(
        host.port,
        path::PERSONA_PERCEIVE_SCENE,
        &json!({"who": ["dora", "ghost", "dora", "anna"], "parallel": 1}),
    )
    .await
    .unwrap();
    let scene = done(reply);
    assert_eq!(scene.results.len(), 3);
    match &scene.results[0].outcome {
        SceneOutcome::Perceived { perceived } => {
            assert_eq!(perceived.viewpoint, Viewpoint::Viewer);
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        &scene.results[1].outcome,
        SceneOutcome::Refused {
            error: PerceiveRouteError::BadRequest {
                cause: PerceiveCause::Persona { .. }
            }
        }
    ));
    match &scene.results[2].outcome {
        SceneOutcome::Perceived { perceived } => {
            assert!(matches!(perceived.viewpoint, Viewpoint::Pictured { .. }));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(host.facts_of(0)["now"]["viewpoint"], json!("viewer"));
    // Nobody to ask, too many and a bad parallel are refused whole.
    for (body, key) in [
        (json!({"parallel": 0}), "invalid_parallel"),
        (json!({"parallel": 9}), "invalid_parallel"),
        (
            json!({"who": ["ghost"], "picture": "plate.png", "parallel": 2}),
            "",
        ),
    ] {
        let reply: PerceiveReply<ScenePerceived> =
            post_json(host.port, path::PERSONA_PERCEIVE_SCENE, &body)
                .await
                .unwrap();
        if key.is_empty() {
            let scene = done(reply);
            assert!(matches!(
                scene.results[0].outcome,
                SceneOutcome::Refused { .. }
            ));
        } else {
            let PerceiveRouteError::BadRequest { cause } = refused(reply) else {
                panic!("not a bad request");
            };
            assert_eq!(cause.key(), key);
        }
    }
}

#[tokio::test]
async fn a_scene_call_with_no_one_in_the_cast_is_refused() {
    let host = scripted_host_cast(
        &["nobody_has_a_persona"],
        vec![],
        Arc::new(FakePort::unconfigured()),
    )
    .await;
    let reply: PerceiveReply<ScenePerceived> =
        post_json(host.port, path::PERSONA_PERCEIVE_SCENE, &json!({}))
            .await
            .unwrap();
    assert!(matches!(
        refused(reply),
        PerceiveRouteError::BadRequest {
            cause: PerceiveCause::NoCharacters
        }
    ));
}

#[tokio::test]
async fn the_thoughts_are_read_back_newest_last_and_a_secret_thought_is_tagged_private() {
    let mut leaking = answer(CHARACTER, "I owe the guild.");
    leaking["leaks_private_side"] = json!(true);
    leaking["leaked_excerpt"] = json!("owe the guild");
    let host = host_of(
        &[CHARACTER],
        vec![
            answer(CHARACTER, "First."),
            leaking,
            answer(CHARACTER, "Third."),
        ],
    )
    .await;
    for _ in 0..3 {
        done(perceive(&host, json!({"who": CHARACTER})).await);
    }
    let all: PerceiveReply<Thoughts> = get_json(
        host.port,
        &format!("{}?who={CHARACTER}", path::PERSONA_THOUGHTS),
    )
    .await
    .unwrap();
    let all = done(all);
    let texts: Vec<&str> = all
        .thoughts
        .iter()
        .map(|thought| thought.text.split('\n').next().unwrap())
        .collect();
    assert_eq!(texts, ["First.", "I owe the guild.", "Third."]);
    let newest: PerceiveReply<Thoughts> = get_json(
        host.port,
        &format!("{}?who={CHARACTER}&limit=1", path::PERSONA_THOUGHTS),
    )
    .await
    .unwrap();
    assert_eq!(done(newest).thoughts.len(), 1);
    let bad: PerceiveReply<Thoughts> = get_json(
        host.port,
        &format!("{}?who={CHARACTER}&limit=0", path::PERSONA_THOUGHTS),
    )
    .await
    .unwrap();
    assert!(matches!(
        refused(bad),
        PerceiveRouteError::BadRequest {
            cause: PerceiveCause::InvalidLimit { .. }
        }
    ));
    let none: PerceiveReply<Thoughts> =
        get_json(host.port, &format!("{}?who=wren", path::PERSONA_THOUGHTS))
            .await
            .unwrap();
    assert!(done(none).thoughts.is_empty());
    let kept = crate::world::memory::recent(&host.running.state, CHARACTER, "thought", 10)
        .await
        .unwrap();
    assert!(kept[1].tags.contains(&"private".to_owned()));
    assert!(!kept[0].tags.contains(&"private".to_owned()));
}

#[tokio::test]
async fn a_perception_is_reminded_of_the_thoughts_before() {
    let host = host_of(
        &[CHARACTER],
        vec![answer(CHARACTER, "First."), answer(CHARACTER, "Second.")],
    )
    .await;
    done(perceive(&host, json!({"who": CHARACTER})).await);
    done(perceive(&host, json!({"who": CHARACTER})).await);
    let earlier = host.facts_of(1)["now"]["earlier_thoughts"].clone();
    assert_eq!(earlier.as_array().unwrap().len(), 1);
    assert!(earlier[0].as_str().unwrap().starts_with("First."));
}

#[test]
fn the_verbs_parse_and_the_words_are_reserved() {
    let PerceiveCommand::One(one) = parse_perceive(&words(
        "danny mode=study picture=a.png focus=well recent=danny:hi vision=false model=m",
    ))
    .unwrap() else {
        panic!("not one");
    };
    assert_eq!(one.who, "danny");
    assert_eq!(one.mode, Mode::Study);
    assert_eq!(one.picture.as_deref(), Some("a.png"));
    assert_eq!(one.focus.as_deref(), Some("well"));
    assert_eq!(one.recent, vec!["danny:hi".to_owned()]);
    assert!(!one.vision);
    let PerceiveCommand::Scene(scene) =
        parse_perceive(&words("scene who=anna,bram parallel=2 mode=glance")).unwrap()
    else {
        panic!("not a scene");
    };
    assert_eq!(scene.who, ["anna", "bram"]);
    assert_eq!(scene.parallel, Some(2));
    assert!(scene.vision);
    let PerceiveCommand::Thoughts { who, limit } = parse_thoughts(&words("danny limit=3")).unwrap()
    else {
        panic!("not thoughts");
    };
    assert_eq!((who.as_str(), limit), ("danny", Some(3)));
    for bad in [
        "",
        "danny mode=wild",
        "danny vision=maybe",
        "danny mode=study mode=glance",
        "danny stray words",
        "scene parallel=many",
        "scene who=Bad Id",
    ] {
        assert!(parse_perceive(&words(bad)).is_err(), "{bad:?}");
    }
    assert!(parse_thoughts(&words("")).is_err());
    assert!(parse_thoughts(&words("danny limit=x")).is_err());
    assert!(super::cli::parse(&words("perceive danny")).is_ok());
    assert!(super::cli::parse(&words("thoughts danny")).is_ok());
    assert!(super::cli::PERSONA_USAGE.contains(super::perceive::cli::PERSONA_THOUGHTS_USAGE));
    assert!(super::cli::PERSONA_USAGE.contains("persona perceive scene"));
    for word in ["perceive", "scene", "thoughts"] {
        assert!(super::storage::RESERVED_IDS.contains(&word), "{word}");
    }
}

#[test]
fn the_answer_is_written_as_lines_the_runner_reads() {
    let perceived: Perceived = serde_json::from_value(json!({
        "who": "danny", "mode": "glance", "picture": "plate.png", "vision": true,
        "viewpoint": {"from": "pictured", "region": "r0000000a"}, "regions": 1,
        "sees": [{"what": "a well,\nmossy", "region": null, "item": "well", "salience": 70}],
        "new_things": [],
        "thinking_about": "That well\nwants painting.", "attention": {"on": "item", "id": "well"},
        "feeling": {"emotion": "joy", "delta": 12}, "wants": ["paint it", "rest"],
        "leak_suspect": false, "thought": "thought-1-0", "duration_ms": 5
    }))
    .unwrap();
    let text = format_perceived(&perceived);
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[0].starts_with("danny perceives (mode=glance vision=true picture=plate.png"));
    assert_eq!(
        lines[1],
        "see: a well, mossy | region=- item=well salience=70"
    );
    assert_eq!(lines[2], "thought: That well wants painting.");
    assert_eq!(lines[3], "attention: item:well");
    assert_eq!(lines[4], "feeling: joy +12");
    assert_eq!(lines[5], "wants: paint it; rest");
    assert_eq!(lines[6], "kept: thought-1-0");
}

/// A road designed in play, with one thing in sight: the active place new things are added to.
fn designed_road(host: &ScriptedHost) {
    host.running.write_asset(
        "world/state.json",
        &json!({
            "place": "road",
            "designed": {
                "id": "road", "name": "A road", "description": "A dusty road.",
                "stalls": [],
                "things": [{"id": "milestone", "name": "milestone", "look": "Worn grey stone."}]
            }
        })
        .to_string(),
    );
}

fn ids_here(host: &ScriptedHost) -> Vec<String> {
    crate::world::place_items(&host.running.state)
        .into_iter()
        .map(|item| item.id)
        .collect()
}

/// An answer in which the character sees a wall (no region, no item) and proposes it as a thing.
fn wall_answer(id: &str) -> Value {
    json!({
        "sees": [{"what": "an old pale wall in the evening light", "region": null,
                  "item": null, "salience": 85}],
        "thinking_about": "That wall wants painting.",
        "attention": {"region": null, "item": null},
        "feeling": null,
        "wants": [],
        "new_things": [{"see": 0, "id": id, "name": "old wall",
                        "look": "Pale lime-washed stone, flaking, with a darker streak.",
                        "attended": true}],
        "leaks_private_side": false, "leaked_excerpt": null
    })
}

/// The cause an answer is refused for in a host with a designed road, asked twice.
async fn thing_cause_of(bad: Value, cast: &[&str]) -> PerceiveCause {
    let host = host_of(cast, vec![bad.clone(), bad]).await;
    designed_road(&host);
    match refused(perceive(&host, json!({"who": CHARACTER})).await) {
        PerceiveRouteError::AnswerRefused { cause, .. } => cause,
        other => panic!("not an answer refusal: {other:?}"),
    }
}

#[tokio::test]
async fn a_new_thing_the_character_notices_becomes_a_thing_of_the_place_and_is_offered_next() {
    let mut again = wall_answer("unused");
    again["sees"][0]["item"] = json!("old-wall");
    again["new_things"] = json!([]);
    again["attention"] = json!({"region": null, "item": "old-wall"});
    let host = host_of(&[CHARACTER], vec![wall_answer("old-wall"), again]).await;
    designed_road(&host);
    assert_eq!(ids_here(&host), ["milestone"]);
    let seen = done(perceive(&host, json!({"who": CHARACTER})).await);
    assert_eq!(seen.new_things.len(), 1);
    assert!(seen.new_things[0].added);
    assert_eq!(seen.sees[0].item.as_deref(), Some("old-wall"));
    assert_eq!(
        seen.attention,
        Attended::Item {
            id: "old-wall".to_owned()
        }
    );
    // The frame judge's next offer holds it, with its look.
    assert_eq!(ids_here(&host), ["milestone", "old-wall"]);
    let offered = crate::world::place_items(&host.running.state);
    assert!(
        offered[1]
            .look
            .as_deref()
            .unwrap()
            .starts_with("Pale lime-washed stone")
    );
    assert_eq!(
        host.lines_starting_with("WORLD thing_added"),
        ["WORLD thing_added place=road id=old-wall via=perceive"]
    );
    assert!(
        host.lines_starting_with("PERSONA perceived who=")
            .pop()
            .unwrap()
            .contains("new_things=1")
    );
    // The next perception is offered it as an item, and may name it.
    let seen = done(perceive(&host, json!({"who": CHARACTER})).await);
    assert_eq!(seen.sees[0].item.as_deref(), Some("old-wall"));
    assert!(seen.new_things.is_empty());
    let facts = host.facts_of(1);
    let here: Vec<&str> = facts["now"]["items_here"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_str().unwrap())
        .collect();
    assert_eq!(here, ["milestone", "old-wall"]);
}

#[tokio::test]
async fn a_new_thing_that_is_taken_or_not_usable_is_refused_and_adds_nothing() {
    let bad_thing = |cause: PerceiveCause| matches!(cause, PerceiveCause::BadNewThing { .. });
    // An id the place already has, a word that is no id, and an id twice in one answer.
    assert!(bad_thing(
        thing_cause_of(wall_answer("milestone"), &[CHARACTER]).await
    ));
    assert!(bad_thing(
        thing_cause_of(wall_answer("Not An Id"), &[CHARACTER]).await
    ));
    let mut twice = wall_answer("wall-a");
    twice["sees"] = json!([
        {"what": "a wall", "region": null, "item": null, "salience": 80},
        {"what": "a gate", "region": null, "item": null, "salience": 70}
    ]);
    twice["new_things"] = json!([
        {"see": 0, "id": "same", "name": "wall", "look": "Stone.", "attended": false},
        {"see": 1, "id": "same", "name": "gate", "look": "Wood.", "attended": false}
    ]);
    assert!(bad_thing(thing_cause_of(twice, &[CHARACTER]).await));
    // A sees entry that is not there, a blank look, and attention that names something else.
    let mut missing = wall_answer("wall-a");
    missing["new_things"][0]["see"] = json!(4);
    assert!(bad_thing(thing_cause_of(missing, &[CHARACTER]).await));
    let mut blank = wall_answer("wall-a");
    blank["new_things"][0]["look"] = json!("  ");
    assert!(bad_thing(thing_cause_of(blank, &[CHARACTER]).await));
    let mut busy = wall_answer("wall-a");
    busy["attention"] = json!({"region": region_of(CHARACTER), "item": null});
    assert!(bad_thing(thing_cause_of(busy, &[CHARACTER]).await));
    // An unknown region is refused as before, whatever else the answer holds.
    let mut unknown = wall_answer("wall-a");
    unknown["sees"][0]["region"] = json!("rffffffff");
    assert_eq!(
        thing_cause_of(unknown, &[CHARACTER]).await,
        PerceiveCause::UnknownRegion {
            id: "rffffffff".to_owned()
        }
    );
    // Nothing was added by a refused answer.
    let host = host_of(
        &[CHARACTER],
        vec![wall_answer("milestone"), wall_answer("milestone")],
    )
    .await;
    designed_road(&host);
    refused(perceive(&host, json!({"who": CHARACTER})).await);
    assert_eq!(ids_here(&host), ["milestone"]);
    assert!(host.lines_starting_with("WORLD thing_added").is_empty());
}

#[tokio::test]
async fn a_person_is_never_offered_as_a_thing() {
    // A sees entry with a region is a person of the scan; it cannot become a thing.
    let mut person = wall_answer("wren-thing");
    person["sees"][0]["region"] = json!(region_of("wren"));
    person["new_things"][0]["attended"] = json!(false);
    assert_eq!(
        thing_cause_of(person, &[CHARACTER, "wren"]).await,
        PerceiveCause::NewThingForPerson { see: 0 }
    );
}

#[tokio::test]
async fn a_place_that_cannot_take_things_is_left_alone_and_the_proposal_is_reported_unadded() {
    // The active place is one of the bible's: the host never writes it.
    let host = host_of(&[CHARACTER], vec![wall_answer("old-wall")]).await;
    host.running.write_asset(
        "world/bible.json",
        &json!({
            "items": [], "voice_packs": [],
            "places": [{"id": "square", "name": "Square", "description": "A square.",
                        "stalls": [], "things": []}]
        })
        .to_string(),
    );
    host.running
        .write_asset("world/state.json", r#"{"place": "square"}"#);
    let seen = done(perceive(&host, json!({"who": CHARACTER})).await);
    assert_eq!(seen.new_things.len(), 1);
    assert!(!seen.new_things[0].added);
    assert_eq!(seen.sees[0].item, None);
    assert_eq!(seen.attention, Attended::None);
    assert!(ids_here(&host).is_empty());
    assert!(!host.lines_starting_with("PERSONA thing_unadded").is_empty());
    assert!(host.lines_starting_with("WORLD thing_added").is_empty());
}

#[tokio::test]
async fn the_world_refuses_a_thing_twice_and_adds_all_or_none() {
    let host = host_of(&[CHARACTER], vec![]).await;
    designed_road(&host);
    let thing = |id: &str| df_content::world_bible::Thing {
        id: id.to_owned(),
        name: "wall".to_owned(),
        look: "Stone.".to_owned(),
        picture: None,
    };
    let state = &host.running.state;
    assert_eq!(
        crate::world::add_things(state, "test", vec![thing("wall")]).await,
        Ok("road".to_owned())
    );
    assert!(
        crate::world::add_things(state, "test", vec![thing("wall")])
            .await
            .is_err()
    );
    assert!(
        crate::world::add_things(state, "test", vec![thing("gate"), thing("wall")])
            .await
            .is_err()
    );
    assert_eq!(ids_here(&host), ["milestone", "wall"]);
}
