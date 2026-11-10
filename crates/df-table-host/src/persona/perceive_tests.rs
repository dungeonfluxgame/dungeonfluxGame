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
    // The character's own region is also named once on its own, and the instruction says that the
    // figure there is them and is told in the first person.
    assert_eq!(
        facts["now"]["you"],
        json!({"region": region_of(CHARACTER), "number": 0})
    );
    let instruction = host.instructions_of(0);
    for part in [
        "now.you gives the region of the scan that is them",
        "is the character's own body",
        "in the first person (I, my), never as a stranger would",
        "that region's id and subject self",
    ] {
        assert!(instruction.contains(part), "{part}");
    }
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
    // A viewer is in no region; a character in the picture is told which region is them.
    assert!(host.facts_of(0)["now"].get("you").is_none());
    assert_eq!(
        host.facts_of(1)["now"]["you"]["region"],
        json!(region_of("anna"))
    );
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
        "see: a well, mossy | region=- item=well salience=70 subject=place"
    );
    assert_eq!(lines[2], "thought: That well wants painting.");
    assert_eq!(lines[3], "attention: item:well");
    assert_eq!(lines[4], "feeling: joy +12");
    assert_eq!(lines[5], "wants: paint it; rest");
    assert_eq!(lines[6], "kept: thought-1-0");
}

#[test]
fn a_thing_added_to_the_place_is_written_with_its_name_for_the_runner_and_one_left_out_is_not() {
    let perceived: Perceived = serde_json::from_value(json!({
        "who": "danny", "mode": "glance", "picture": "plate.png", "vision": true,
        "viewpoint": {"from": "pictured", "region": "r0000000a"}, "regions": 1,
        "sees": [{"what": "a door", "region": null, "item": "wooden_door", "salience": 70}],
        "new_things": [
            {"id": "wooden_door", "name": "weathered wooden door",
             "look": "Grey\ntimber, black iron latch.", "added": true},
            {"id": "sky", "name": "sky", "look": "Clouds.", "added": false,
             "why_not": "a vista"}
        ],
        "thinking_about": "That door.", "attention": {"on": "item", "id": "wooden_door"},
        "feeling": null, "wants": [], "leak_suspect": false, "thought": null, "duration_ms": 5
    }))
    .unwrap();
    let text = format_perceived(&perceived);
    let things: Vec<&str> = text
        .lines()
        .filter(|line| line.starts_with("thing "))
        .collect();
    assert_eq!(
        things,
        ["thing wooden_door (weathered wooden door): Grey timber, black iron latch."]
    );
}

#[tokio::test]
async fn a_sight_at_the_characters_own_region_is_themselves_whatever_subject_the_model_gave() {
    let mut mirror = plain_answer(
        vec![{
            let mut man = sight("a bearded man beside the easel", "person", None, 70);
            man["region"] = json!(region_of(CHARACTER));
            man
        }],
        json!({"region": null, "item": null}),
    );
    mirror["new_things"] = json!([]);
    let host = host_of(&[CHARACTER], vec![mirror]).await;
    let seen = done(perceive(&host, json!({"who": CHARACTER})).await);
    assert_eq!(seen.sees[0].subject.key(), "self");
    assert!(!format_perceived(&seen).contains("subject=person"));
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

#[tokio::test]
async fn an_entry_may_name_the_new_thing_proposed_for_it_as_its_item_and_its_attention() {
    // The natural answer: the entry names the thing it proposes, and attention is on it.
    let mut said = wall_answer("old-wall");
    said["sees"][0]["item"] = json!("old-wall");
    said["attention"] = json!({"region": null, "item": "old-wall"});
    said["new_things"][0]["attended"] = json!(false);
    let host = host_of(&[CHARACTER], vec![said]).await;
    designed_road(&host);
    let seen = done(perceive(&host, json!({"who": CHARACTER})).await);
    assert_eq!(host.calls(), 1, "no retry");
    assert!(seen.new_things[0].added);
    assert_eq!(seen.sees[0].item.as_deref(), Some("old-wall"));
    assert_eq!(
        seen.attention,
        Attended::Item {
            id: "old-wall".to_owned()
        }
    );
    assert_eq!(ids_here(&host), ["milestone", "old-wall"]);
}

#[tokio::test]
async fn an_entry_that_names_the_place_thing_it_proposes_again_adds_nothing_and_is_not_refused() {
    // The model names the existing handcart and proposes it too: it is already there.
    let mut said = wall_answer("milestone");
    said["sees"][0]["item"] = json!("milestone");
    said["new_things"][0]["attended"] = json!(false);
    let host = host_of(&[CHARACTER], vec![said]).await;
    designed_road(&host);
    let seen = done(perceive(&host, json!({"who": CHARACTER})).await);
    assert_eq!(host.calls(), 1, "no retry");
    assert!(seen.new_things.is_empty());
    assert_eq!(seen.sees[0].item.as_deref(), Some("milestone"));
    assert_eq!(ids_here(&host), ["milestone"]);
    assert!(host.lines_starting_with("WORLD thing_added").is_empty());
}

#[tokio::test]
async fn a_thing_that_could_not_be_added_leaves_no_item_the_place_does_not_have() {
    let mut said = wall_answer("old-wall");
    said["sees"][0]["item"] = json!("old-wall");
    said["attention"] = json!({"region": null, "item": "old-wall"});
    said["new_things"][0]["attended"] = json!(false);
    let host = host_of(&[CHARACTER], vec![said]).await;
    // No active place: nothing can be added.
    let seen = done(perceive(&host, json!({"who": CHARACTER})).await);
    assert!(!seen.new_things[0].added);
    assert_eq!(seen.sees[0].item, None);
    assert_eq!(seen.attention, Attended::None);
}

#[tokio::test]
async fn the_facts_carry_the_last_six_thoughts_and_the_instruction_asks_for_what_is_new() {
    let answers: Vec<Value> = (0..8)
        .map(|number| answer(CHARACTER, &format!("Thought number {number}.")))
        .collect();
    let host = host_of(&[CHARACTER], answers).await;
    for _ in 0..8 {
        done(perceive(&host, json!({"who": CHARACTER})).await);
    }
    let earlier = host.facts_of(7)["now"]["earlier_thoughts"].clone();
    let earlier: Vec<&str> = earlier
        .as_array()
        .unwrap()
        .iter()
        .map(|thought| thought.as_str().unwrap().split('\n').next().unwrap())
        .collect();
    assert_eq!(
        earlier,
        [
            "Thought number 1.",
            "Thought number 2.",
            "Thought number 3.",
            "Thought number 4.",
            "Thought number 5.",
            "Thought number 6."
        ]
    );
    let instruction = host.instructions_of(7);
    assert!(
        instruction.contains(
            "say what is NEW in this picture compared with them; never reuse their wording, \
             their opening words or their sentence shape, and begin differently each time"
        ),
        "{instruction}"
    );
}

/// Runs `said` in a host with a designed road and answers what the perception came to.
async fn perceived_with(cast: &[&str], said: Value) -> (ScriptedHost, Perceived) {
    let host = host_of(cast, vec![said]).await;
    designed_road(&host);
    let seen = done(perceive(&host, json!({"who": CHARACTER})).await);
    assert_eq!(host.calls(), 1, "a bad proposal costs no retry");
    (host, seen)
}

#[tokio::test]
async fn a_proposal_for_a_person_is_dropped_and_the_rest_of_the_answer_is_kept() {
    // The sees entry has a region, so it is a person; the sight and thought stay.
    let mut said = wall_answer("wren-thing");
    said["sees"][0]["region"] = json!(region_of("wren"));
    said["sees"][0]["item"] = json!("wren-thing");
    said["attention"] = json!({"region": null, "item": "wren-thing"});
    said["new_things"][0]["attended"] = json!(false);
    let (host, seen) = perceived_with(&[CHARACTER, "wren"], said).await;
    assert_eq!(seen.thinking_about, "That wall wants painting.");
    assert_eq!(seen.sees.len(), 1);
    assert_eq!(seen.sees[0].region, Some(region_of("wren")));
    // The provisional item and attention that pointed at the dropped thing are cleared.
    assert_eq!(seen.sees[0].item, None);
    assert_eq!(seen.attention, Attended::None);
    assert_eq!(seen.new_things.len(), 1);
    assert!(!seen.new_things[0].added);
    let why = seen.new_things[0].why_not.as_deref().unwrap();
    assert!(
        why.contains("is a person") && why.contains("remove its new_things entry"),
        "{why}"
    );
    assert_eq!(ids_here(&host), ["milestone"]);
    assert_eq!(
        host.lines_starting_with("PERSONA thing_unadded"),
        ["PERSONA thing_unadded who=innkeeper error=new_thing_for_person"]
    );
    assert!(seen.thought.is_some());
}

#[tokio::test]
async fn a_bad_proposal_is_dropped_with_what_to_change_and_the_answer_is_kept() {
    let case = |edit: &dyn Fn(&mut Value)| {
        let mut said = wall_answer("wall-a");
        edit(&mut said);
        said
    };
    // (the answer, the key it is left out for, words its reason holds)
    let cases = [
        (
            case(&|said| said["new_things"][0]["id"] = json!("milestone")),
            "bad_new_thing",
            "choose a different id",
        ),
        (
            case(&|said| said["new_things"][0]["id"] = json!("Not An Id")),
            "bad_new_thing",
            "is not an id",
        ),
        (
            case(&|said| said["sees"][0]["item"] = json!("milestone")),
            "bad_new_thing",
            "set the sees entry's item to null",
        ),
        (
            case(&|said| said["new_things"][0]["see"] = json!(4)),
            "bad_new_thing",
            "which is not there",
        ),
        (
            case(&|said| said["new_things"][0]["look"] = json!("  ")),
            "bad_new_thing",
            "needs a name and a look",
        ),
        (
            case(&|said| {
                said["attention"] = json!({"region": null, "item": "milestone"});
            }),
            "bad_new_thing",
            "only one new thing may be attended to",
        ),
    ];
    for (said, key, words) in cases {
        let (host, seen) = perceived_with(&[CHARACTER], said).await;
        assert_eq!(seen.thinking_about, "That wall wants painting.");
        assert_eq!(seen.sees.len(), 1);
        assert!(!seen.new_things[0].added);
        let why = seen.new_things[0].why_not.as_deref().unwrap();
        assert!(why.contains(words), "{words} in {why}");
        assert_eq!(
            host.lines_starting_with("PERSONA thing_unadded"),
            [format!("PERSONA thing_unadded who=innkeeper error={key}")]
        );
        assert_eq!(ids_here(&host), ["milestone"], "{words}");
        assert!(host.lines_starting_with("WORLD thing_added").is_empty());
    }
}

#[tokio::test]
async fn a_good_proposal_next_to_a_bad_one_is_still_added() {
    let mut said = wall_answer("old-wall");
    said["sees"] = json!([
        {"what": "an old pale wall", "region": null, "item": null, "salience": 80},
        {"what": "a gate", "region": null, "item": null, "salience": 60}
    ]);
    said["new_things"] = json!([
        {"see": 0, "id": "old-wall", "name": "old wall", "look": "Pale stone.", "attended": true},
        {"see": 1, "id": "milestone", "name": "gate", "look": "Oak.", "attended": false}
    ]);
    let (host, seen) = perceived_with(&[CHARACTER], said).await;
    assert_eq!(seen.new_things.len(), 2);
    let bad = seen
        .new_things
        .iter()
        .find(|thing| thing.id == "milestone")
        .unwrap();
    let good = seen
        .new_things
        .iter()
        .find(|thing| thing.id == "old-wall")
        .unwrap();
    assert!(good.added && !bad.added);
    assert_eq!(seen.sees[0].item.as_deref(), Some("old-wall"));
    assert_eq!(seen.sees[1].item, None);
    assert_eq!(
        seen.attention,
        Attended::Item {
            id: "old-wall".to_owned()
        }
    );
    assert_eq!(ids_here(&host), ["milestone", "old-wall"]);
    assert_eq!(host.lines_starting_with("PERSONA thing_unadded").len(), 1);
    assert_eq!(host.lines_starting_with("WORLD thing_added").len(), 1);
}

#[tokio::test]
async fn a_bad_core_still_asks_again_with_the_reason_whatever_the_proposals_hold() {
    // An unknown item in sees is the core of the answer: refused, asked once more with why.
    let mut core = wall_answer("old-wall");
    core["sees"][0]["item"] = json!("lute");
    let host = host_of(&[CHARACTER], vec![core, wall_answer("old-wall")]).await;
    designed_road(&host);
    let seen = done(perceive(&host, json!({"who": CHARACTER})).await);
    assert_eq!(host.calls(), 2);
    let reason = host.facts_of(1)["now"]["retry_reason"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(reason.contains("lute"), "{reason}");
    assert!(seen.new_things[0].added);
    // And if it stays bad, the whole answer is refused and nothing is added.
    let mut stays = wall_answer("old-wall");
    stays["sees"][0]["region"] = json!("rffffffff");
    assert_eq!(
        thing_cause_of(stays, &[CHARACTER]).await,
        PerceiveCause::UnknownRegion {
            id: "rffffffff".to_owned()
        }
    );
    let mut empty = wall_answer("old-wall");
    empty["thinking_about"] = json!(" ");
    assert_eq!(
        thing_cause_of(empty, &[CHARACTER]).await,
        PerceiveCause::EmptyThought
    );
}

#[tokio::test]
async fn a_vista_is_dropped_and_an_object_is_kept() {
    let mut said = wall_answer("old-wall");
    said["sees"] = json!([
        {"what": "a distant town on the hill", "region": null, "item": null, "salience": 60},
        {"what": "an old pale wall", "region": null, "item": null, "salience": 80},
        {"what": "a trough", "region": null, "item": null, "salience": 50}
    ]);
    said["new_things"] = json!([
        {"see": 0, "id": "distant-town", "name": "distant town", "look": "Roofs far off.",
         "attended": false, "kind": "vista"},
        {"see": 1, "id": "old-wall", "name": "old wall", "look": "Pale stone.",
         "attended": true, "kind": "object"},
        {"see": 2, "id": "trough", "name": "trough", "look": "Hollowed oak.",
         "attended": false, "kind": null}
    ]);
    let (host, seen) = perceived_with(&[CHARACTER], said).await;
    // The sight and thought are kept whole; the vista is no thing; the objects are.
    assert_eq!(seen.sees.len(), 3);
    assert_eq!(seen.thinking_about, "That wall wants painting.");
    let by_id = |id: &str| seen.new_things.iter().find(|thing| thing.id == id).unwrap();
    assert!(!by_id("distant-town").added);
    assert!(by_id("old-wall").added && by_id("trough").added);
    assert!(
        by_id("distant-town")
            .why_not
            .as_deref()
            .unwrap()
            .contains("a vista")
    );
    assert_eq!(seen.sees[0].item, None);
    assert_eq!(ids_here(&host), ["milestone", "old-wall", "trough"]);
    assert_eq!(
        host.lines_starting_with("PERSONA thing_unadded"),
        ["PERSONA thing_unadded who=innkeeper error=vista"]
    );
    // A kind that is neither is a bad proposal, dropped the same way.
    let mut odd = wall_answer("old-wall");
    odd["new_things"][0]["kind"] = json!("scenery");
    let (_, seen) = perceived_with(&[CHARACTER], odd).await;
    assert!(!seen.new_things[0].added);
    assert!(
        seen.new_things[0]
            .why_not
            .as_deref()
            .unwrap()
            .contains("neither object nor vista")
    );
}

/// A frame judge over the same host and a script of judgements; answers what it was asked.
async fn frame_judge_over(
    host: &ScriptedHost,
    answers: usize,
) -> (u16, Arc<std::sync::Mutex<Vec<df_ai::text::TextRequest>>>) {
    use df_ai::text::TextOutcome;
    use df_ai::text_fixture::ScriptedBackend;

    use crate::text::{BackendChoice, TextRuntime};

    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let asked = Arc::new(std::sync::Mutex::new(Vec::new()));
    let judgement = json!({
        "changed_percent": 10, "changes": [], "next_picture": "The road, as it was.",
        "shot": "wide", "inspected_item": null, "new_look": false
    });
    let script = ScriptedBackend::new(
        (0..answers).map(|_| TextOutcome::Answered(judgement.clone())),
        calls,
    )
    .recording(Arc::clone(&asked));
    let recorder = Arc::clone(&host.running.state);
    let text = TextRuntime::new(
        BackendChoice::Scripted(Arc::new(script), None),
        Arc::new(move |line| recorder.record_line(line)),
    );
    let app = super::frame::router_with(Arc::clone(&host.running.state), Arc::new(text));
    let listener = crate::server::bind(0).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        // The server runs until the test's runtime stops; its end is the test's end.
        let _ = axum::serve(listener, app).await;
    });
    (port, asked)
}

async fn judged(port: u16) -> Value {
    let body = json!({
        "picture": "A road with an old wall.",
        "lines": [format!("{CHARACTER}: I'll step up to that wall.")],
        "speakers": [
            {"id": CHARACTER, "display_name": "Marta", "job": "Role: an innkeeper"},
            {"id": "wren", "display_name": "Wren", "job": "Role: a passer-by"}
        ]
    });
    post_json(port, "/dev/persona/frame", &body).await.unwrap()
}

#[tokio::test]
async fn a_speakers_attention_reaches_the_frame_judge_for_them_only_and_expires_with_the_place() {
    // The speaker perceives the road and attends to the old wall.
    let mut said = wall_answer("old-wall");
    said["sees"][0]["item"] = json!("old-wall");
    said["attention"] = json!({"region": null, "item": "old-wall"});
    said["new_things"] = json!([]);
    let mut nothing = said.clone();
    nothing["sees"][0]["item"] = json!(null);
    nothing["attention"] = json!({"region": null, "item": null});
    let host = host_of(&[CHARACTER, "wren"], vec![said, nothing]).await;
    host.running.write_asset(
        "world/state.json",
        &json!({
            "place": "road",
            "designed": {
                "id": "road", "name": "A road", "description": "A dusty road.", "stalls": [],
                "things": [
                    {"id": "milestone", "name": "milestone", "look": "Worn grey stone."},
                    {"id": "old-wall", "name": "old wall", "look": "Pale stone."}
                ]
            }
        })
        .to_string(),
    );
    done(perceive(&host, json!({"who": CHARACTER})).await);
    let (port, asked) = frame_judge_over(&host, 4).await;
    // At the place they perceived: the speaker's attention, and nobody else's.
    judged(port).await;
    assert_eq!(
        asked.lock().unwrap()[0].facts["attending"],
        json!([{"speaker": CHARACTER, "item": "old-wall"}])
    );
    assert!(
        asked.lock().unwrap()[0]
            .instructions
            .contains("attending lists")
    );
    // Another place: it expires with the place, though the same thing is there.
    host.running.write_asset(
        "world/state.json",
        &json!({
            "place": "lane",
            "designed": {
                "id": "lane", "name": "A lane", "description": "A lane.", "stalls": [],
                "things": [{"id": "old-wall", "name": "old wall", "look": "Pale stone."}]
            }
        })
        .to_string(),
    );
    judged(port).await;
    assert_eq!(asked.lock().unwrap()[1].facts["attending"], json!([]));
    // Back at the road the thought was had at, it holds again.
    host.running.write_asset(
        "world/state.json",
        &json!({
            "place": "road",
            "designed": {
                "id": "road", "name": "A road", "description": "A dusty road.", "stalls": [],
                "things": [{"id": "old-wall", "name": "old wall", "look": "Pale stone."}]
            }
        })
        .to_string(),
    );
    judged(port).await;
    assert_eq!(
        asked.lock().unwrap()[2].facts["attending"],
        json!([{"speaker": CHARACTER, "item": "old-wall"}])
    );
    // A newer perception that attends to nothing ends it.
    done(perceive(&host, json!({"who": CHARACTER})).await);
    judged(port).await;
    assert_eq!(asked.lock().unwrap()[3].facts["attending"], json!([]));
}

/// A road designed in play with two things in sight.
fn road_of_two(host: &ScriptedHost) {
    host.running.write_asset(
        "world/state.json",
        &json!({
            "place": "road",
            "designed": {
                "id": "road", "name": "A road", "description": "A dusty road.", "stalls": [],
                "things": [
                    {"id": "milestone", "name": "milestone", "look": "Worn grey stone."},
                    {"id": "cottage", "name": "cottage", "look": "Dark timber and thatch."}
                ]
            }
        })
        .to_string(),
    );
}

/// The character carries a brush.
fn carrying_a_brush(host: &ScriptedHost) {
    host.running.write_asset(
        "world/inventory/innkeeper.json",
        &json!({
            "schema": 1, "owner": CHARACTER, "kind": "character",
            "items": [{"id": "brush", "name": "brush", "look": "Splayed bristles.",
                       "use": "kept", "count": 1}]
        })
        .to_string(),
    );
}

fn sight(what: &str, subject: &str, item: Option<&str>, salience: u32) -> Value {
    json!({"what": what, "region": null, "item": item, "salience": salience, "subject": subject})
}

fn plain_answer(sees: Vec<Value>, attention: Value) -> Value {
    json!({
        "sees": sees, "thinking_about": "A thought.", "attention": attention,
        "feeling": null, "wants": [], "new_things": [],
        "leaks_private_side": false, "leaked_excerpt": null
    })
}

#[tokio::test]
async fn the_character_themselves_is_never_the_attention_and_the_place_is_chosen_instead() {
    // Attention on their own region and on a sight marked self: cleared, then the place's most
    // salient thing is chosen.
    let mut own = plain_answer(
        vec![
            {
                let mut coat = sight("my dark coat feels heavy", "self", None, 90);
                coat["region"] = json!(region_of(CHARACTER));
                coat
            },
            sight("a worn milestone", "place", Some("milestone"), 30),
            sight("a dark cottage", "place", Some("cottage"), 70),
        ],
        json!({"region": region_of(CHARACTER), "item": null}),
    );
    own["new_things"] = json!([]);
    let host = host_of(&[CHARACTER], vec![own]).await;
    road_of_two(&host);
    let seen = done(perceive(&host, json!({"who": CHARACTER})).await);
    assert_eq!(
        seen.attention,
        Attended::Item {
            id: "cottage".to_owned()
        }
    );
    assert_eq!(
        host.lines_starting_with("PERSONA attention_cleared"),
        ["PERSONA attention_cleared who=innkeeper why=self"]
    );
    assert_eq!(seen.sees[0].subject.key(), "self");
    // The thought keeps the sights, not the coat.
    let kept = crate::world::memory::recent(&host.running.state, CHARACTER, "thought", 1)
        .await
        .unwrap();
    assert!(
        !kept[0].description.contains("dark coat"),
        "{}",
        kept[0].description
    );
    assert!(kept[0].description.contains("dark cottage"));
    // A sight marked self by item, with the attention on that item, is cleared the same way.
    let by_item = plain_answer(
        vec![sight("my hat", "self", Some("milestone"), 80)],
        json!({"region": null, "item": "milestone"}),
    );
    let host = host_of(&[CHARACTER], vec![by_item]).await;
    road_of_two(&host);
    let seen = done(perceive(&host, json!({"who": CHARACTER})).await);
    assert_eq!(seen.attention, Attended::None);
    assert!(
        !host
            .lines_starting_with("PERSONA attention_cleared who=innkeeper why=self")
            .is_empty()
    );
}

#[tokio::test]
async fn what_the_character_carries_is_no_sight_no_attention_and_no_new_thing() {
    // Attention on a carried item: cleared; a carried sight is no place thing and is no proposal.
    let mut held = plain_answer(
        vec![
            sight("I hold a brush", "carried", Some("brush"), 95),
            sight("a worn milestone", "place", Some("milestone"), 20),
        ],
        json!({"region": null, "item": "brush"}),
    );
    held["new_things"] = json!([
        {"see": 0, "id": "my-brush", "name": "brush", "look": "Bristles.",
         "attended": false, "kind": "object"}
    ]);
    let host = host_of(&[CHARACTER], vec![held]).await;
    road_of_two(&host);
    carrying_a_brush(&host);
    let seen = done(perceive(&host, json!({"who": CHARACTER})).await);
    assert_eq!(host.calls(), 1);
    // Never the brush; the milestone is chosen instead.
    assert_eq!(
        seen.attention,
        Attended::Item {
            id: "milestone".to_owned()
        }
    );
    assert_eq!(
        host.lines_starting_with("PERSONA attention_cleared"),
        ["PERSONA attention_cleared who=innkeeper why=carried"]
    );
    assert!(!seen.new_things[0].added);
    assert!(
        seen.new_things[0]
            .why_not
            .as_deref()
            .unwrap()
            .contains("what they carry")
    );
    assert_eq!(ids_here(&host), ["milestone", "cottage"]);
    // With only carried sights and self, the attention stays nothing.
    let only_mine = plain_answer(
        vec![
            sight("I hold a brush", "carried", Some("brush"), 95),
            sight("my coat", "self", None, 90),
        ],
        json!({"region": null, "item": null}),
    );
    let host = host_of(&[CHARACTER], vec![only_mine]).await;
    road_of_two(&host);
    carrying_a_brush(&host);
    let seen = done(perceive(&host, json!({"who": CHARACTER})).await);
    assert_eq!(seen.attention, Attended::None);
}

#[tokio::test]
async fn a_place_thing_is_chosen_when_nothing_else_is_and_a_person_is_no_thing() {
    // No attention given: the most salient thing of the place is chosen (the first of equals).
    let said = plain_answer(
        vec![
            sight("a worn milestone", "place", Some("milestone"), 40),
            sight("a dark cottage", "place", Some("cottage"), 75),
            sight("a stranger", "person", None, 99),
        ],
        json!({"region": null, "item": null}),
    );
    let host = host_of(&[CHARACTER], vec![said]).await;
    road_of_two(&host);
    let seen = done(perceive(&host, json!({"who": CHARACTER})).await);
    assert_eq!(
        seen.attention,
        Attended::Item {
            id: "cottage".to_owned()
        }
    );
    assert!(
        host.lines_starting_with("PERSONA attention_cleared")
            .is_empty()
    );
    // A proposal for a person (by subject, with no region) is dropped as a person.
    let mut stranger = plain_answer(
        vec![sight("a stranger on the road", "person", None, 90)],
        json!({"region": null, "item": null}),
    );
    stranger["new_things"] = json!([
        {"see": 0, "id": "stranger", "name": "stranger", "look": "A traveller.",
         "attended": false, "kind": "object"}
    ]);
    let host = host_of(&[CHARACTER], vec![stranger]).await;
    road_of_two(&host);
    let seen = done(perceive(&host, json!({"who": CHARACTER})).await);
    assert!(!seen.new_things[0].added);
    assert_eq!(
        host.lines_starting_with("PERSONA thing_unadded"),
        ["PERSONA thing_unadded who=innkeeper error=new_thing_for_person"]
    );
    // A thing just added is attended to when nothing else is, the strongest first.
    let mut added = plain_answer(
        vec![
            sight("a low wall", "place", None, 40),
            sight("a stout gate", "place", None, 85),
        ],
        json!({"region": null, "item": null}),
    );
    added["new_things"] = json!([
        {"see": 0, "id": "low-wall", "name": "low wall", "look": "Stone.",
         "attended": false, "kind": "object"},
        {"see": 1, "id": "gate", "name": "gate", "look": "Oak.",
         "attended": false, "kind": "object"}
    ]);
    let host = host_of(&[CHARACTER], vec![added]).await;
    road_of_two(&host);
    let seen = done(perceive(&host, json!({"who": CHARACTER})).await);
    assert_eq!(
        seen.attention,
        Attended::Item {
            id: "gate".to_owned()
        }
    );
}

#[tokio::test]
async fn the_instruction_keeps_the_character_and_what_they_carry_out_of_the_sights() {
    let host = host_of(
        &[CHARACTER],
        vec![plain_answer(vec![], json!({"region": null, "item": null}))],
    )
    .await;
    done(perceive(&host, json!({"who": CHARACTER})).await);
    let instruction = host.instructions_of(0);
    for part in [
        "does not list themselves, what they wear or what they carry as sights",
        "the sights are the place and the others in it",
        "self and carried entries never become what they attend to",
        "choose the one that draws them most",
    ] {
        assert!(instruction.contains(part), "{part}");
    }
}
