use super::super::super::storyteller::tests::{FakeEndpoint, Reply, cache_directory, teller};
use super::super::{generate, record};
use super::*;
use std::{path::Path, sync::Arc};

const SEED: u64 = 0x3c6e_f378_dd81_de97;

fn tables() -> Tables {
    Tables::load(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../content/adventure-tables")
            .as_path(),
    )
    .unwrap()
}

fn baseline(tables: &Tables) -> GeneratedAdventure {
    generate(
        tables,
        SEED,
        GenerateOptions::default(),
        &ForcedEntries::new(),
    )
    .unwrap()
}

/// The slot names and facts a request's prompt carries.
fn parse(prompt: &str) -> (Vec<String>, Value) {
    let line = |prefix: &str| {
        prompt
            .lines()
            .find_map(|line| line.strip_prefix(prefix))
            .unwrap()
            .to_owned()
    };
    let slots: Vec<Value> = serde_json::from_str(&line("SLOTS: ")).unwrap();
    let names = slots
        .iter()
        .map(|slot| slot["name"].as_str().unwrap().to_owned())
        .collect();
    (names, serde_json::from_str(&line("FACTS: ")).unwrap())
}

/// A model that returns each requested slot's draft, except where `rewrite` says otherwise.
fn model_that(
    rewrite: impl Fn(&str, &str, &Value) -> Option<String> + Send + Sync + 'static,
) -> Reply {
    Reply::Compute(Arc::new(move |prompt| {
        let (names, facts) = parse(prompt);
        let answers: serde_json::Map<String, Value> = names
            .into_iter()
            .map(|name| {
                let draft = facts["drafts"][name.as_str()].as_str().unwrap();
                let text = rewrite(&name, draft, &facts).unwrap_or_else(|| draft.to_owned());
                (name, Value::String(text))
            })
            .collect();
        Value::Object(answers)
    }))
}

fn rewrites(name: &str, draft: &str, facts: &Value) -> Option<String> {
    let place = facts["location"]["name"].as_str().unwrap();
    let patron = facts["cast"]["patron"]["name"].as_str().unwrap();
    match name {
        "begin_label" => Some(format!("Slip into {place}")),
        // Digits are not a storyteller rule, so this reaches the section 7 semantic check.
        "ask_label" => Some(format!("Ask {patron} about 12 things")),
        // A rules word: the storyteller refuses it, retries once, and refuses it again.
        "help_label" => Some(format!("Attack beside {patron}")),
        // The secret may only be told in the private clue and the closing lines.
        "opening_narration" => Some(format!("{draft} It is my fault.")),
        _ => {
            let _ = draft;
            None
        }
    }
}

/// Each group is asked once, and the group whose slot is refused is asked a second time.
fn four_answers() -> Vec<Reply> {
    (0..4).map(|_| model_that(rewrites)).collect()
}

async fn express_with(
    endpoint: &FakeEndpoint,
    cache: &Path,
) -> (
    GeneratedAdventure,
    Result<(GeneratedAdventure, ModelExpression), Declined>,
) {
    let tables = tables();
    let baseline = baseline(&tables);
    let storyteller = teller(endpoint.address, 200_000, Duration::from_secs(5), cache);
    let outcome = express(
        &tables,
        SEED,
        GenerateOptions::default(),
        &ForcedEntries::new(),
        &baseline,
        &storyteller,
    )
    .await;
    (baseline, outcome)
}

#[tokio::test(flavor = "current_thread")]
async fn accepted_slots_take_the_model_text_and_refused_slots_keep_their_pattern_text() {
    let endpoint = FakeEndpoint::start(four_answers());
    let cache = cache_directory("expression-mixed");

    let (baseline, outcome) = express_with(&endpoint, &cache).await;

    let (generated, model) = outcome.unwrap();
    let place = baseline.facts["loc.name"].clone();
    assert_eq!(
        generated.pack.offer_labels.begin_story,
        format!("Slip into {place}")
    );
    assert_eq!(
        generated.pack.offer_labels.ask_courier,
        baseline.pack.offer_labels.ask_courier
    );
    assert_eq!(
        generated.pack.offer_labels.escort_courier,
        baseline.pack.offer_labels.escort_courier
    );
    assert!(model.model_slots.contains(&"begin_label".to_owned()));
    for refused in ["ask_label", "help_label", "opening_narration"] {
        assert!(
            model.pattern_slots.contains(&refused.to_owned()),
            "{refused}"
        );
    }
    assert!(model.refused.contains(&"ask_label: semantic".to_owned()));
    assert!(
        model
            .refused
            .contains(&"help_label: forbidden_term".to_owned())
    );
    assert!(
        model
            .refused
            .contains(&"opening_narration: forbidden_term".to_owned())
    );
    assert_eq!(model.model, "gpt-test-model");
    assert_eq!(
        model.tokens,
        140 * 4,
        "two groups, and the voices retried once"
    );
    // The mechanical sentences and the rest of the pack are exactly the pattern adventure's.
    let narrations = |adventure: &GeneratedAdventure| {
        adventure
            .pack
            .scenes
            .iter()
            .map(|scene| (scene.narration.clone(), scene.narration_rested.clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(narrations(&generated), narrations(&baseline));
    assert_eq!(generated.pack.secret_terms, baseline.pack.secret_terms);
    assert_eq!(generated.tuning, baseline.tuning);
    assert_eq!(generated.rolls, baseline.rolls);
    let _ = std::fs::remove_dir_all(cache);
}

#[tokio::test(flavor = "current_thread")]
async fn the_final_adventure_reloads_and_records_the_model_slots() {
    let endpoint = FakeEndpoint::start(four_answers());
    let cache = cache_directory("expression-record");

    let (_, outcome) = express_with(&endpoint, &cache).await;

    let (generated, model) = outcome.unwrap();
    let documents = generated
        .documents_expressed(true, Some(model.clone()))
        .unwrap();
    let (loaded, stored) = record::reload(&documents).unwrap();
    assert_eq!(loaded.pack, generated.pack);
    assert_eq!(stored.prose_summary(), model.summary());
    assert_eq!(
        model.summary(),
        format!(
            "prose=model slots={}/{} tokens=560",
            model.model_slots.len(),
            model.model_slots.len() + model.pattern_slots.len()
        )
    );
    let record_text = String::from_utf8(documents.record).unwrap();
    assert!(record_text.contains("\"source\": \"model\""));
    assert!(record_text.contains("\"model\": \"gpt-test-model\""));
    assert!(record_text.contains("\"secret_terms\""));
    let _ = std::fs::remove_dir_all(cache);
}

#[tokio::test(flavor = "current_thread")]
async fn the_requests_carry_low_reasoning_a_large_cap_and_tell_the_secret_only_where_it_belongs() {
    let endpoint = FakeEndpoint::start(four_answers());
    let cache = cache_directory("expression-request");

    let (baseline, outcome) = express_with(&endpoint, &cache).await;

    outcome.unwrap();
    let bodies = endpoint.bodies();
    assert!(bodies.len() >= 2);
    for body in &bodies {
        assert_eq!(body["reasoning"], json!({"effort": "low"}));
        assert_eq!(body["max_output_tokens"], 2500);
    }
    let secret = baseline.facts["secret.fact"].clone();
    let inputs: Vec<&str> = bodies
        .iter()
        .map(|body| body["input"].as_str().unwrap())
        .collect();
    let scenes: Vec<&&str> = inputs
        .iter()
        .filter(|input| input.contains("\"name\":\"room_title\""))
        .collect();
    let voices: Vec<&&str> = inputs
        .iter()
        .filter(|input| input.contains("\"name\":\"clue\""))
        .collect();
    assert_eq!(scenes.len(), 1);
    assert!(!voices.is_empty());
    assert!(
        !scenes[0].contains(&secret),
        "the scene group is never told the secret"
    );
    assert!(voices[0].contains(&secret));
    assert!(voices[0].contains("Never write a slot's forbidden terms"));
    let _ = std::fs::remove_dir_all(cache);
}

#[tokio::test(flavor = "current_thread")]
async fn a_failing_or_off_storyteller_leaves_the_pattern_adventure_to_the_caller() {
    let endpoint = FakeEndpoint::start(vec![
        Reply::Status(500, json!({"error": {}})),
        Reply::Status(500, json!({"error": {}})),
    ]);
    let cache = cache_directory("expression-failure");

    let (_, outcome) = express_with(&endpoint, &cache).await;

    assert!(matches!(outcome, Err(Declined::Storyteller(_))));
    let tables = tables();
    let baseline = baseline(&tables);
    let outcome = express(
        &tables,
        SEED,
        GenerateOptions::default(),
        &ForcedEntries::new(),
        &baseline,
        &Storyteller::disabled(),
    )
    .await;
    assert!(matches!(
        outcome,
        Err(Declined::Storyteller(StorytellerError::Off))
    ));
    assert_eq!(outcome.err().unwrap().to_string(), "storyteller off");
    let _ = std::fs::remove_dir_all(cache);
}

#[test]
fn the_semantic_check_refuses_digits_markup_and_names_the_facts_do_not_hold() {
    let tables = tables();
    let baseline = baseline(&tables);
    let known = known_words(&baseline);
    let narration = baseline
        .fact_sheet
        .iter()
        .find(|sheet| sheet.slot == "narration")
        .unwrap();
    let patron = baseline.facts["patron.first"].clone();
    assert!(!semantic_problem(
        narration,
        &format!("Rain falls. {patron} waits by the lamp."),
        &known
    ));
    assert!(semantic_problem(
        narration,
        "Rain falls on 3 roofs.",
        &known
    ));
    assert!(semantic_problem(narration, "Rain *falls* softly.", &known));
    assert!(semantic_problem(
        narration,
        "Rain falls. Then Zorblax the Unseen waits by the lamp.",
        &known
    ));
}
