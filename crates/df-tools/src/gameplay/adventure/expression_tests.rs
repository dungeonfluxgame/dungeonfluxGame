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
        // A draft that names where it happens ("A Stand at the market") must keep that place.
        if let Some((_, place)) = draft.split_once(" at ") {
            return Some(format!("Challenge at {place}"));
        }
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

fn rated(rating: ContentRating) -> GenerateOptions {
    GenerateOptions {
        rating,
        ..GenerateOptions::default()
    }
}

/// Answers every slot with its draft, except `target`, which gets `extra` and then its
/// must-mention terms so the slot's own checks pass.
fn model_with_line(target: &'static str, extra: &'static str) -> Reply {
    Reply::Compute(Arc::new(move |prompt| {
        let (_, facts) = parse(prompt);
        let slots: Vec<Value> = serde_json::from_str(
            prompt
                .lines()
                .find_map(|line| line.strip_prefix("SLOTS: "))
                .unwrap(),
        )
        .unwrap();
        let answers: serde_json::Map<String, Value> = slots
            .iter()
            .map(|slot| {
                let name = slot["name"].as_str().unwrap();
                let text = if name == target {
                    let terms: Vec<&str> = slot["must_mention"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter_map(Value::as_str)
                        .collect();
                    format!("{extra} {}.", terms.join(" and "))
                } else {
                    facts["drafts"][name].as_str().unwrap().to_owned()
                };
                (name.to_owned(), Value::String(text))
            })
            .collect();
        Value::Object(answers)
    }))
}

async fn express_rated(
    rating: ContentRating,
    script: impl Fn() -> Reply,
    cache_name: &str,
) -> (
    GeneratedAdventure,
    Result<(GeneratedAdventure, ModelExpression), Declined>,
    Vec<Value>,
) {
    let endpoint = FakeEndpoint::start((0..8).map(|_| script()).collect());
    let cache = cache_directory(cache_name);
    let tables = tables();
    let baseline = generate(&tables, SEED, rated(rating), &ForcedEntries::new()).unwrap();
    let storyteller = teller(endpoint.address, 200_000, Duration::from_secs(5), &cache);
    let outcome = express(
        &tables,
        SEED,
        rated(rating),
        &ForcedEntries::new(),
        &baseline,
        &storyteller,
    )
    .await;
    let bodies = endpoint.bodies();
    let _ = std::fs::remove_dir_all(cache);
    (baseline, outcome, bodies)
}

fn opening_line(adventure: &GeneratedAdventure) -> String {
    adventure
        .pack
        .scenes
        .iter()
        .find(|scene| scene.key == super::super::super::pack::SceneKey::Opening)
        .unwrap()
        .dialogue
        .clone()
}

#[test]
fn the_craft_guidance_differs_between_ratings_only_in_the_rating_sentence() {
    let (restrained, mature) = (
        guidance(ContentRating::Restrained),
        guidance(ContentRating::Mature),
    );
    let (before, after) = (craft_before_rating!(), craft_after_rating!());
    for text in [restrained, mature] {
        assert!(text.starts_with(before) && text.ends_with(after));
    }
    let middle = |text: &str| text[before.len()..text.len() - after.len()].to_owned();
    assert_eq!(
        middle(restrained),
        "Heroic fantasy for a living-room table: tense, never gory."
    );
    assert!(middle(mature).starts_with(mature_themes!()));
    assert!(mature.contains("mature themes are allowed"));
    assert!(!mature.contains("never gory") && !mature.contains("living-room"));
    // Line by line, exactly the continuity line changes; every craft rule and the secret rule
    // are the same words.
    let differing: Vec<&str> = restrained
        .lines()
        .zip(mature.lines())
        .filter(|(restrained, mature)| restrained != mature)
        .map(|(restrained, _)| restrained.split(':').next().unwrap())
        .collect();
    assert_eq!(restrained.lines().count(), mature.lines().count());
    assert_eq!(differing, ["CONTINUITY"]);
    for text in [restrained, mature] {
        assert!(text.contains("SECRET: never write a slot's forbidden terms or hint at them."));
        assert!(text.contains("STORY, NOT RULES: narration is pure story."));
    }
}

#[test]
fn the_word_lists_drop_gore_and_strong_language_when_mature_and_keep_every_rules_word() {
    let (restrained, mature) = (
        forbidden_words(ContentRating::Restrained),
        forbidden_words(ContentRating::Mature),
    );
    for word in RESTRAINED_GORE.iter().chain(&RESTRAINED_PROFANITY) {
        assert!(restrained.contains(word), "restrained refuses {word}");
        assert!(!mature.contains(word), "mature allows {word}");
    }
    for word in [
        "damage",
        "hit points",
        "d20",
        "dc",
        "killed",
        "dies",
        "okay",
        "internet",
    ] {
        assert!(
            restrained.contains(&word) && mature.contains(&word),
            "{word}"
        );
    }
    let shared: Vec<&&str> = restrained
        .iter()
        .filter(|word| mature.contains(word))
        .collect();
    assert_eq!(
        shared.len(),
        mature.len(),
        "mature is restrained minus the two groups"
    );
    // The restrained list keeps today's words and order, with the strong language appended.
    assert_eq!(
        &restrained[..FORBIDDEN_RULES_WORDS.len()],
        FORBIDDEN_RULES_WORDS
    );
}

#[test]
fn the_model_is_told_the_rating_tone_and_the_always_on_limits_in_both_ratings() {
    let facts_of = |rating| {
        let baseline = generate(&tables(), SEED, rated(rating), &ForcedEntries::new()).unwrap();
        let planned = plan(&baseline).unwrap();
        group_facts(&baseline, &planned, true, None)
    };
    let (restrained, mature) = (
        facts_of(ContentRating::Restrained),
        facts_of(ContentRating::Mature),
    );
    assert_eq!(restrained["tone"], "heroic fantasy for a living-room table");
    assert!(mature["tone"].as_str().unwrap().contains("adult table"));
    assert!(restrained["forbidden"].to_string().contains("entrails"));
    assert!(!mature["forbidden"].to_string().contains("entrails"));
    for facts in [&restrained, &mature] {
        for person in ["patron", "contact", "enemy"] {
            let person = &facts["cast"][person];
            assert_eq!(person["based_on_real_person"], false);
            assert_ne!(person["age_group"], "child");
            assert_ne!(person["age_group"], "teen");
        }
    }
    let prompt = build_prompt(&mature);
    assert!(prompt.contains("never write sexual content involving a minor"));
}

fn build_prompt(facts: &Value) -> String {
    super::super::super::storyteller::build_prompt(
        &super::super::super::storyteller::Expression {
            purpose: "adventure",
            facts,
            slots: &[Slot::new("room_title", 32)],
            style: "style",
            profile: super::super::super::storyteller::CallProfile::short_reply(
                ContentRating::Restrained,
            ),
        },
        None,
    )
}

#[test]
fn a_young_cast_member_is_named_a_young_adult_in_the_facts() {
    let tables = tables();
    let mut young = 0;
    for seed in 0..200 {
        let baseline = generate(
            &tables,
            seed,
            rated(ContentRating::Mature),
            &ForcedEntries::new(),
        )
        .unwrap();
        let planned = plan(&baseline).unwrap();
        let facts = group_facts(&baseline, &planned, false, None);
        for (key, person) in [
            ("patron", "patron"),
            ("contact", "contact"),
            ("enemy", "ant"),
        ] {
            let age = &baseline.facts[&format!("{person}.age")];
            let told = &facts["cast"][key];
            let expected = match age.as_str() {
                "young" => "young adult",
                other => other,
            };
            assert_eq!(told["age_group"], expected, "seed {seed} {key}");
            assert_eq!(told["sexual_content_allowed"], true);
            young += usize::from(age == "young");
        }
    }
    assert!(young > 0, "some cast member is young");
}

#[tokio::test(flavor = "current_thread")]
async fn a_gory_and_profane_line_is_kept_when_mature_and_refused_when_restrained() {
    let crude = "I will spill your entrails, damn you.";
    let (_, mature, _) = express_rated(
        ContentRating::Mature,
        || model_with_line("opening_line", crude),
        "rating-crude-mature",
    )
    .await;
    let (generated, model) = mature.unwrap();
    assert!(model.model_slots.contains(&"opening_line".to_owned()));
    assert!(opening_line(&generated).contains(crude));

    let (_, restrained, _) = express_rated(
        ContentRating::Restrained,
        || model_with_line("opening_line", crude),
        "rating-crude-restrained",
    )
    .await;
    let (generated, model) = restrained.unwrap();
    assert!(model.pattern_slots.contains(&"opening_line".to_owned()));
    assert!(
        model
            .refused
            .contains(&"opening_line: forbidden_term".to_owned()),
        "{:?}",
        model.refused
    );
    assert!(!opening_line(&generated).contains("entrails"));
}

#[tokio::test(flavor = "current_thread")]
async fn a_slot_that_gives_away_the_secret_is_refused_the_same_way_in_both_ratings() {
    let tables = tables();
    let secret = generate(
        &tables,
        SEED,
        rated(ContentRating::Mature),
        &ForcedEntries::new(),
    )
    .unwrap()
    .pack
    .secret_terms[0]
        .clone();
    let leak: &'static str = Box::leak(format!("It is all about the {secret}.").into_boxed_str());
    for rating in [ContentRating::Mature, ContentRating::Restrained] {
        let (_, outcome, bodies) = express_rated(
            rating,
            || model_with_line("opening_line", leak),
            &format!("rating-secret-{rating:?}"),
        )
        .await;
        let (generated, model) = outcome.unwrap();
        assert!(
            model
                .refused
                .contains(&"opening_line: forbidden_term".to_owned()),
            "{rating:?}: {:?}",
            model.refused
        );
        assert!(!opening_line(&generated).contains(&secret), "{rating:?}");
        // Refused once, asked once more with the same rules, never reworded to get around it.
        let asks = bodies
            .iter()
            .filter(|body| {
                body["input"]
                    .as_str()
                    .unwrap()
                    .contains("\"name\":\"opening_line\"")
            })
            .count();
        assert_eq!(asks, 2, "{rating:?}");
    }
}
