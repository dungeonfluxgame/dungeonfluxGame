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

/// Distinct titles that are about this adventure's place, as the section 7 title rule wants.
fn title_for(scene: &str, facts: &Value) -> String {
    let place = facts["location"]["name"].as_str().unwrap();
    let word = match scene {
        "room" => "Gathering",
        "opening" => "Request",
        "dialogue_seal" => "Question",
        "dialogue_escort" => "Promise",
        "combat" => "Challenge",
        "inn" => "Arrival",
        "victory" => "Reprieve",
        _ => "Setback",
    };
    format!("{place} {word}")
}

fn rewrites(name: &str, draft: &str, facts: &Value) -> Option<String> {
    let place = facts["location"]["name"].as_str().unwrap();
    let patron = facts["cast"]["patron"]["name"].as_str().unwrap();
    if let Some(scene) = name.strip_suffix("_title") {
        return Some(title_for(scene, facts));
    }
    match name {
        "begin_label" => Some(format!("Slip into {place}")),
        // Digits are not a storyteller rule, so this reaches the section 7 semantic check.
        "ask_label" => Some(format!("Ask {patron} about 12 things")),
        // A rules word: the storyteller refuses it, retries once, and refuses it again.
        "help_label" => Some(format!("Attack beside {patron}")),
        // The secret may only be told in the private clue and the closing lines.
        "opening_narration" => Some(format!("{draft} It is my fault.")),
        _ => None,
    }
}

/// Plenty of answers: each group is asked once, a refused slot is asked again, and the lint may
/// ask a third time.
fn answers() -> Vec<Reply> {
    (0..8).map(|_| model_that(rewrites)).collect()
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
    let endpoint = FakeEndpoint::start(answers());
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
    assert!(model.model_slots.contains(&"combat_title".to_owned()));
    for refused in ["ask_label", "help_label", "opening_narration"] {
        assert!(
            model.pattern_slots.contains(&refused.to_owned()),
            "{refused}"
        );
    }
    assert!(model.refused.contains(&"ask_label: lint_digits".to_owned()));
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
        140 * u64::try_from(endpoint.connections()).unwrap()
    );
    // The rules sentences and the rest of the pack are exactly the pattern adventure's.
    let rules = |adventure: &GeneratedAdventure| {
        adventure
            .pack
            .scenes
            .iter()
            .map(|scene| (scene.rules_note.clone(), scene.narration_rested.clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(rules(&generated), rules(&baseline));
    assert_eq!(generated.pack.secret_terms, baseline.pack.secret_terms);
    assert_eq!(generated.tuning, baseline.tuning);
    assert_eq!(generated.rolls, baseline.rolls);
    let _ = std::fs::remove_dir_all(cache);
}

#[tokio::test(flavor = "current_thread")]
async fn the_final_adventure_reloads_and_records_the_model_slots() {
    let endpoint = FakeEndpoint::start(answers());
    let cache = cache_directory("expression-record");

    let (_, outcome) = express_with(&endpoint, &cache).await;

    let (generated, model) = outcome.unwrap();
    let documents = generated
        .documents_expressed(true, Some(model.clone()))
        .unwrap();
    let (loaded, stored) = record::reload(&documents).unwrap();
    assert_eq!(loaded.pack, generated.pack);
    assert_eq!(stored.prose_summary(), model.summary());
    assert!(model.summary().starts_with("prose=model slots="));
    let record_text = String::from_utf8(documents.record).unwrap();
    assert!(record_text.contains("\"source\": \"model\""));
    assert!(record_text.contains("\"model\": \"gpt-test-model\""));
    assert!(record_text.contains("\"secret_terms\""));
    let _ = std::fs::remove_dir_all(cache);
}

#[tokio::test(flavor = "current_thread")]
async fn the_requests_carry_low_reasoning_a_large_cap_and_tell_the_secret_only_where_it_belongs() {
    let endpoint = FakeEndpoint::start(answers());
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
    assert!(voices[0].contains("never write a slot's forbidden terms"));
    assert!(voices[0].contains("STORY, NOT RULES"));
    // The invitation to the players is not narration, and the rules wording is never sent.
    assert!(!scenes[0].contains("Share the room"));
    assert!(
        !inputs
            .iter()
            .any(|input| input.contains("no surprise, cover"))
    );
    // The lint asked again for the label with digits, and said what to fix.
    let reasks: Vec<&&str> = inputs
        .iter()
        .filter(|input| input.contains("rewrite_notes"))
        .collect();
    assert!(
        reasks
            .iter()
            .any(|input| input.contains("contains digits; write numbers in words"))
    );
    for input in reasks {
        if !input.contains("\"secret\":") {
            assert!(!input.contains(&secret), "a rewrite without the secret");
        }
    }
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

fn sheet_of<'a>(baseline: &'a GeneratedAdventure, slot: &str) -> &'a FactSheet {
    baseline
        .fact_sheet
        .iter()
        .find(|sheet| sheet.slot == slot)
        .unwrap()
}

#[test]
fn the_semantic_check_refuses_digits_markup_and_names_the_facts_do_not_hold() {
    let tables = tables();
    let baseline = baseline(&tables);
    let known = known_words(&baseline);
    let specific = specific_words(&baseline);
    let narration = sheet_of(&baseline, "narration");
    let patron = baseline.facts["patron.first"].clone();
    let check = |text: &str| semantic_problem(narration, text, &known, &specific);
    assert_eq!(
        check(&format!("Rain falls. {patron} waits by the lamp.")),
        None
    );
    assert_eq!(check("Rain falls on 3 roofs."), Some("digits"));
    assert_eq!(check("Rain *falls* softly."), Some("markup"));
    assert_eq!(
        check("Rain falls. Then Zorblax the Unseen waits by the lamp."),
        Some("new_name")
    );
    assert_eq!(check("Rain falls; the lamp sways."), Some("punctuation"));
    assert_eq!(check("Rain falls. Who waits?"), Some("punctuation"));
    assert_eq!(
        check("He says \"wait\" and waits."),
        Some("quotation_marks")
    );
}

#[test]
fn a_spoken_line_may_not_be_narration_and_narration_may_not_quote_a_line() {
    let tables = tables();
    let baseline = baseline(&tables);
    let known = known_words(&baseline);
    let specific = specific_words(&baseline);
    let line = baseline
        .fact_sheet
        .iter()
        .find(|sheet| sheet.slot == "line" && !sheet.speaker.is_empty())
        .unwrap();
    let speaker = line.speaker.clone();
    let check = |text: &str| semantic_problem(line, text, &known, &specific);
    assert_eq!(check("I need your help, and I need it tonight."), None);
    assert_eq!(
        check(&format!("{speaker} lowers a shaking hand and waits.")),
        Some("narration_in_line")
    );
    assert_eq!(
        check("The clash falls quiet as the dust settles over the stones."),
        Some("narration_in_line")
    );
    assert_eq!(
        check("I said \"wait\" and I meant it."),
        Some("quotation_marks")
    );
    let narration = sheet_of(&baseline, "narration");
    assert_eq!(
        semantic_problem(
            narration,
            "He growls, \"Give it here.\" Then he waits.",
            &known,
            &specific
        ),
        Some("quotation_marks")
    );
}

#[test]
fn a_title_must_be_specific_short_and_not_a_stock_phrase() {
    let tables = tables();
    let baseline = baseline(&tables);
    let known = known_words(&baseline);
    let specific = specific_words(&baseline);
    let title = sheet_of(&baseline, "title");
    let check = |text: &str| semantic_problem(title, text, &known, &specific);
    assert_eq!(check("The Lamplit Market Bell"), None);
    assert_eq!(check("A Moment of Quiet"), Some("title"));
    assert_eq!(check("Safe at Last"), Some("title"));
    assert_eq!(check("Lamplit"), Some("title"));
    assert_eq!(
        check("The Long Wait for a Message at the Market"),
        Some("title")
    );
    assert_eq!(check("A Quiet Evening Alone"), Some("title"));
    assert_eq!(check(&title.baseline.clone()), Some("title"));
}

#[test]
fn the_lint_finds_stock_phrases_long_sentences_and_repeats_across_slots() {
    let tables = tables();
    let baseline = baseline(&tables);
    let planned = plan(&baseline).unwrap();
    let known = known_words(&baseline);
    let specific = specific_words(&baseline);
    let texts = |pairs: &[(&str, &str)]| -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(name, text)| ((*name).to_owned(), (*text).to_owned()))
            .collect()
    };
    let long = "Over the market the lanterns sway above the crowd while the hawkers shout over \
                the sizzling pans and somebody drops a tray of bells and a child laughs at \
                the noise tonight.";
    let found = find_problems(
        &planned,
        &texts(&[
            ("opening_narration", "Now, then. Rain falls on the awnings."),
            ("room_narration", long),
            (
                "combat_narration",
                "Rain falls on the old awnings again. The crowd parts.",
            ),
            (
                "inn_narration",
                "Rain falls on the old awnings again. Softly.",
            ),
            ("victory_narration", "The lanterns sway in the damp air."),
            ("defeat_narration", "The lanterns sway in the damp air."),
            ("dialogue_seal_narration", "Ilsa steps close and waits."),
            ("dialogue_escort_narration", "Ilsa steps close and waits."),
        ]),
        &known,
        &specific,
    );
    assert_eq!(found["opening_narration"], "stock_phrase");
    assert_eq!(found["room_narration"], "long_sentence");
    assert_eq!(found["inn_narration"], "repeated_sentence");
    assert!(
        !found.contains_key("defeat_narration") && !found.contains_key("victory_narration"),
        "slots of which only one is played may share sentences"
    );
    assert!(
        !found.contains_key("dialogue_escort_narration"),
        "the two answers to the patron are alternatives"
    );
    let found = find_problems(
        &planned,
        &texts(&[
            ("opening_line", "The lamps are burning low and I fear it."),
            ("combat_line", "The lamps are all that I wanted."),
        ]),
        &known,
        &specific,
    );
    assert_eq!(found["combat_line"], "same_opening");
}

#[test]
fn narration_and_rules_notes_are_separate_end_to_end() {
    let tables = tables();
    let baseline = baseline(&tables);
    let rules_words = ["HP", "Short Rest", "feet", "Prone", "campaign time"];
    for scene in &baseline.pack.scenes {
        for word in rules_words {
            assert!(
                !scene.narration.contains(word),
                "{:?} narration has {word}",
                scene.key
            );
        }
    }
    let note = |key| {
        baseline
            .pack
            .scenes
            .iter()
            .find(|scene| scene.key == key)
            .unwrap()
            .rules_note
            .clone()
    };
    assert!(note(super::super::super::pack::SceneKey::Combat).contains("5 feet"));
    assert!(note(super::super::super::pack::SceneKey::Victory).contains("Short Rest"));
    assert!(note(super::super::super::pack::SceneKey::Defeat).contains("unconscious"));
    assert!(note(super::super::super::pack::SceneKey::Room).is_empty());
    for sheet in &baseline.fact_sheet {
        assert!(!sheet.baseline.contains("5 feet"));
        assert!(!sheet.baseline.contains("Short Rest"));
    }
}
