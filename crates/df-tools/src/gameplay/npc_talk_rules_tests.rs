//! The transcript rules, shown on made-up replies and on recorded live conversations.

use std::path::PathBuf;

use serde_json::{Value, json};

use super::*;

#[test]
fn influence_uses_the_srd_dc_and_attitude_roll_mode() {
    assert_eq!(
        influence_check_plan(
            InfluenceWillingness::Willing,
            Attitude::Friendly,
            18,
            Approach::Persuasion,
        ),
        None
    );
    assert_eq!(
        influence_check_plan(
            InfluenceWillingness::Unwilling,
            Attitude::Hostile,
            18,
            Approach::Intimidation,
        ),
        None
    );
    assert_eq!(
        influence_check_plan(
            InfluenceWillingness::Hesitant,
            Attitude::Friendly,
            18,
            Approach::Deception,
        ),
        Some(InfluenceCheckPlan {
            skill: InfluenceSkill::Deception,
            difficulty_class: 18,
            roll_mode: InfluenceRollMode::Advantage,
        })
    );
    assert_eq!(
        influence_check_plan(
            InfluenceWillingness::Hesitant,
            Attitude::Indifferent,
            10,
            Approach::Flirting,
        ),
        Some(InfluenceCheckPlan {
            skill: InfluenceSkill::Performance,
            difficulty_class: 15,
            roll_mode: InfluenceRollMode::Normal,
        })
    );
    assert_eq!(
        influence_check_plan(
            InfluenceWillingness::Hesitant,
            Attitude::Hostile,
            15,
            Approach::Persuasion,
        )
        .unwrap()
        .roll_mode,
        InfluenceRollMode::Disadvantage
    );
    assert_eq!(
        INFLUENCE_FAILURE_WAIT,
        std::time::Duration::from_secs(86_400)
    );
}

#[test]
fn influence_advantage_and_disadvantage_select_the_correct_d20_face() {
    let plan = |roll_mode| InfluenceCheckPlan {
        skill: InfluenceSkill::Persuasion,
        difficulty_class: 15,
        roll_mode,
    };
    let resolve = |roll_mode| resolve_influence_check(plan(roll_mode), [5, 17], 2, 0).unwrap();

    assert_eq!(resolve(InfluenceRollMode::Advantage).result.die, 17);
    assert_eq!(resolve(InfluenceRollMode::Normal).result.die, 5);
    assert_eq!(resolve(InfluenceRollMode::Disadvantage).result.die, 5);
}

fn exchange(hero_words: &str, reply: &str, facts: Value) -> Exchange {
    Exchange {
        hero_words: hero_words.to_owned(),
        reply: reply.to_owned(),
        facts,
        from_model: true,
    }
}

fn transcript(exchanges: Vec<Exchange>) -> Transcript {
    Transcript {
        name: "test".to_owned(),
        seed: "0x1/mature".to_owned(),
        speaker: "Kelbin".to_owned(),
        holds_private_clue: false,
        clue_in_play: false,
        deflects: false,
        clue_only_terms: Vec::new(),
        never_terms: Vec::new(),
        style_phrases: Vec::new(),
        colour_lines: Vec::new(),
        own_lines: Vec::new(),
        courtship: String::new(),
        exchanges,
    }
}

fn checks_of(findings: &[Finding]) -> Vec<&'static str> {
    findings.iter().map(|finding| finding.check).collect()
}

fn with_answer() -> Value {
    json!({"answer_from": ["The iron coffer goes to Tomas."]})
}

fn live_reply_is_rejected(reply: &str, said: &[String], answer_offered: bool) -> bool {
    super::super::npc_dossier::spoken_rules(answer_offered, said, false)
        .iter()
        .any(|rule| rule.violation(reply).is_some())
}

#[test]
fn live_reply_validation_rejects_a_flat_refusal() {
    assert!(live_reply_is_rejected(
        "I have nothing more I can tell you.",
        &[],
        false,
    ));
}

#[test]
fn live_reply_validation_rejects_a_repeated_phrase() {
    let said = vec!["There is a quicker way to finish the journey.".to_owned()];

    assert!(live_reply_is_rejected(
        "There is a quicker way to finish the journey, through a gap most travelers miss.",
        &said,
        false,
    ));
}

#[test]
fn live_reply_validation_rejects_refusing_to_recommend_a_choice() {
    assert!(live_reply_is_rejected(
        "I cannot say which you should choose.",
        &[],
        false,
    ));
}

#[test]
fn sentences_split_at_terminal_marks_and_keep_the_rest() {
    assert_eq!(
        sentences("The coffer goes to Tomas. Who sent it? Nobody! Go"),
        ["The coffer goes to Tomas.", "Who sent it?", "Nobody!", "Go"]
    );
}

#[test]
fn a_denial_is_found_wherever_it_stands_in_the_first_sentence() {
    for denial in [
        "To be exact, I do not know what is in the iron coffer.",
        "I'm afraid I don\u{2019}t know who has been watching her door.",
        "I can't tell you whether the seed-grain is here.",
        "I know nothing of that.",
    ] {
        assert!(is_denial(denial), "{denial}");
    }
    for answer in [
        "The coffer goes to Tomas at the Waystation.",
        "Ask after Tomas by name and the door will open.",
    ] {
        assert!(!is_denial(answer), "{answer}");
    }
}

#[test]
fn a_reply_that_opens_with_a_denial_when_an_answer_was_offered_is_found() {
    let denying = transcript(vec![exchange(
        "What is in the iron coffer?",
        "To be exact, I do not know what is in the iron coffer. Will you carry it?",
        with_answer(),
    )]);
    assert_eq!(
        checks_of(&check(&denying)),
        ["denial_with_an_answer_offered"]
    );

    let nothing_offered = transcript(vec![exchange(
        "What is in the iron coffer?",
        "I do not know what is in the iron coffer, but Tomas will.",
        json!({"answer_from": []}),
    )]);
    assert!(check(&nothing_offered).is_empty());

    let mut declining = transcript(vec![exchange(
        "What did you whisper to Brakka?",
        "I can't say what passed between us, friend.",
        with_answer(),
    )]);
    declining.deflects = true;
    assert!(check(&declining).is_empty());
}

#[test]
fn admitting_ignorance_on_the_same_question_twice_is_found() {
    let nothing = json!({"answer_from": []});
    let twice = transcript(vec![
        exchange("Who is she?", "I don't know who she is.", nothing.clone()),
        exchange("Who is she?", "I have no idea about her either.", nothing),
    ]);
    assert_eq!(
        checks_of(&check(&twice)),
        ["says_i_dont_know_more_than_once"]
    );
}

#[test]
fn a_sentence_said_twice_in_one_conversation_is_found() {
    let repeated = transcript(vec![
        exchange(
            "Who are you?",
            "I carried letters for a grain merchant.",
            json!({}),
        ),
        exchange(
            "And before?",
            "I carried letters for a grain merchant. Then the roads closed.",
            json!({}),
        ),
    ]);
    let findings = check(&repeated);
    // A repeated sentence repeats its clauses too: both findings fire.
    assert_eq!(
        checks_of(&findings),
        ["repeated_clause", "repeated_sentence"]
    );
    assert_eq!(findings[0].exchange, 2);
}

#[test]
fn a_long_reply_with_many_sentences_is_found() {
    let long = "The road is long and the pass is cold. The guard is wary of every stranger who comes. \
                The gate is shut from dusk until the first grey light of morning. Nobody sleeps well \
                in the watch house these nights. Tomas waits for you at the end of it all, as he \
                promised he would, and he will not wait forever. The lamps along the wall stay lit \
                until the dawn bell has rung twice over the valley.";
    let findings = check(&transcript(vec![exchange("Where?", long, json!({}))]));
    assert_eq!(checks_of(&findings), ["too_many_sentences", "too_long"]);
}

#[test]
fn a_stock_phrase_from_the_speech_style_is_found_as_a_whole_phrase() {
    let mut spoken = transcript(vec![exchange(
        "Where does it go?",
        "To be exact, it goes to Tomas.",
        json!({}),
    )]);
    spoken.style_phrases = vec![
        "begins corrections with 'To be exact.'".to_owned(),
        "Right.".to_owned(),
    ];
    assert_eq!(checks_of(&check(&spoken)), ["says_a_speech_style_phrase"]);
    spoken.exchanges[0].reply = "It goes to Tomas, who is exact about everything.".to_owned();
    assert!(check(&spoken).is_empty());
}

#[test]
fn local_lore_and_autobiography_are_found_unless_the_hero_asked_for_them() {
    let lore = "Muleteers sing as they cross so the ones ahead will hear them in the cloud.";
    let life = "I walked alone to the nearest great city at fifteen, on a very lucky hay cart.";
    let mut told = transcript(vec![exchange(
        "Where do we go next?",
        &format!("{lore} {life}"),
        json!({}),
    )]);
    told.colour_lines = vec![lore.to_owned()];
    told.own_lines = vec![life.to_owned()];
    assert_eq!(
        checks_of(&check(&told)),
        ["local_lore_or_saying", "autobiography"]
    );
    told.exchanges[0].hero_words =
        "What is this place like, and tell me about your past?".to_owned();
    assert!(check(&told).is_empty(), "{:?}", check(&told));
}

#[test]
fn a_reply_that_restates_the_question_is_found() {
    let said = transcript(vec![exchange(
        "What is in the iron coffer, and who would stop us on the road?",
        "What is in the iron coffer, and who would stop us on the road, you ask.",
        json!({}),
    )]);
    assert_eq!(checks_of(&check(&said)), ["restates_the_question"]);
}

#[test]
fn the_private_clue_matrix_lets_the_told_hero_hear_it_and_nobody_else() {
    let terms = vec!["quicker way".to_owned(), "Whitlock".to_owned()];
    let leaking = |holds: bool, reply: &str| {
        let mut conversation =
            transcript(vec![exchange("What did you say?", reply, with_answer())]);
        conversation.holds_private_clue = holds;
        conversation.clue_in_play = true;
        conversation.clue_only_terms = terms.clone();
        check(&conversation)
    };
    // The hero who was told hears it discussed.
    assert!(leaking(true, "There is a quicker way, past the pass.").is_empty());
    // One who was told but never hears it discussed is a failure of the speaker.
    assert_eq!(
        checks_of(&leaking(true, "Mind the road, friend.")),
        ["never_spoke_of_the_private_clue"]
    );
    // One who was not told never hears it.
    assert_eq!(
        checks_of(&leaking(false, "Ask for Whitlock at the waystation.")),
        ["leaked_the_private_clue"]
    );
    assert!(leaking(false, "I keep that to myself, friend.").is_empty());
}

#[test]
fn a_sentence_two_speakers_of_one_adventure_share_is_found() {
    let line = "Muleteers sing as they cross so the ones ahead will hear them.";
    let mut first = transcript(vec![exchange("a", line, json!({}))]);
    first.speaker = "Kelbin".to_owned();
    let mut second = transcript(vec![exchange("b", line, json!({}))]);
    second.speaker = "Tomas".to_owned();
    let shared = shared_sentences(&[&first, &second]);
    assert_eq!(shared.len(), 1);
    assert_eq!(shared[0].1, ["Kelbin", "Tomas"]);

    // The same speaker saying it in two conversations, or two adventures sharing a table line,
    // is not two speakers sounding alike.
    second.speaker = "Kelbin".to_owned();
    assert!(shared_sentences(&[&first, &second]).is_empty());
    second.speaker = "Tomas".to_owned();
    second.seed = "0x2/mature".to_owned();
    assert!(shared_sentences(&[&first, &second]).is_empty());
}

fn fixtures() -> Vec<(String, Transcript)> {
    let directory =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/gameplay/fixtures/npc-talk");
    let mut found: Vec<(String, Transcript)> = std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .map(|path| {
            let name = path.file_stem().unwrap().to_string_lossy().into_owned();
            let record: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            (
                name,
                Transcript::from_json(&record).expect("a recorded conversation"),
            )
        })
        .collect();
    found.sort_by(|a, b| a.0.cmp(&b.0));
    found
}

#[test]
fn the_recorded_conversations_from_before_the_fix_break_the_rules_the_player_noticed() {
    let mut found: Vec<&'static str> = Vec::new();
    let recorded = fixtures();
    for (name, conversation) in recorded
        .iter()
        .filter(|(name, _)| name.starts_with("before-"))
    {
        let findings = check(conversation);
        println!("{name}: {:?}", checks_of(&findings));
        found.extend(findings.iter().map(|finding| finding.check));
    }
    for check in [
        "says_a_speech_style_phrase",
        "local_lore_or_saying",
        "autobiography",
        "repeated_sentence",
    ] {
        assert!(found.contains(&check), "{check} was not found: {found:?}");
    }
}

#[test]
fn a_prepared_admission_is_not_treated_as_a_known_answer_that_cannot_be_denied() {
    let honest = transcript(vec![exchange(
        "Who is that lady?",
        "I don't know who that lady is.",
        json!({"answer_from": ["I don't know who that lady is."]}),
    )]);
    assert!(check(&honest).is_empty());
}

#[test]
fn the_recorded_conversations_after_the_fix_hold_every_hard_rule() {
    let recorded = fixtures();
    let after: Vec<&(String, Transcript)> = recorded
        .iter()
        .filter(|(name, _)| name.starts_with("after-"))
        .collect();
    assert!(after.len() >= 6, "{} recorded conversations", after.len());
    // The two soft rules (a life story line, local lore) may fire once or twice across the whole
    // set, since a reader judges them; every other rule must hold in every conversation.
    let mut soft = 0;
    let mut fallbacks = 0;
    let mut hard: Vec<String> = Vec::new();
    let mut live_rejections: Vec<(String, &'static str)> = Vec::new();
    for (name, conversation) in &after {
        for finding in check(conversation) {
            if matches!(finding.check, "autobiography" | "local_lore_or_saying") {
                soft += 1;
            } else {
                let exchange = &conversation.exchanges[finding.exchange - 1];
                let said: Vec<String> = conversation.exchanges[..finding.exchange - 1]
                    .iter()
                    .map(|exchange| exchange.reply.clone())
                    .collect();
                if live_reply_is_rejected(
                    &exchange.reply,
                    &said,
                    super::answer_offered(&exchange.facts),
                ) {
                    live_rejections.push((name.clone(), finding.check));
                } else {
                    hard.push(format!("{name}: {finding:?}"));
                }
            }
        }
        fallbacks += conversation
            .exchanges
            .iter()
            .filter(|exchange| !exchange.from_model)
            .count();
    }
    assert!(hard.is_empty(), "{hard:?}");
    live_rejections.sort();
    assert_eq!(
        live_rejections,
        [
            ("after-patron-earnest-mature".to_owned(), "flat_refusal"),
            ("after-patron-earnest-restrained".to_owned(), "flat_refusal"),
            ("after-witness-holder-mature".to_owned(), "repeated_clause"),
            ("after-witness-outsider-mature".to_owned(), "flat_refusal"),
            (
                "after-witness-outsider-mature".to_owned(),
                "repeated_clause"
            ),
        ],
        "known recorded violations must remain rejected by live validation"
    );
    assert!(soft <= 2, "{soft} soft findings");
    assert!(
        fallbacks <= 1,
        "{fallbacks} replies fell back to a prepared line"
    );
    let borrowed: Vec<&Transcript> = after.iter().map(|(_, conversation)| conversation).collect();
    let shared = shared_sentences(&borrowed);
    assert!(shared.is_empty(), "{shared:?}");
}

/// Reads every `conversation.json` under `DF_N06_CHECK` and prints what the rules find in each,
/// so a live run can be judged without a model. Fails when any rule finds something and
/// `DF_N06_STRICT` is set.
#[test]
#[ignore = "reads the transcripts of a live run named by DF_N06_CHECK"]
fn check_the_transcripts_of_a_live_run() {
    let root = PathBuf::from(std::env::var("DF_N06_CHECK").unwrap());
    let mut directories: Vec<PathBuf> = std::fs::read_dir(&root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.join("conversation.json").exists())
        .collect();
    directories.sort();
    let mut conversations = Vec::new();
    let mut total = 0;
    for directory in &directories {
        let record: Value =
            serde_json::from_slice(&std::fs::read(directory.join("conversation.json")).unwrap())
                .unwrap();
        let conversation = Transcript::from_json(&record).expect("a recorded conversation");
        let findings = check(&conversation);
        total += findings.len();
        println!("RULES {}: {} finding(s)", conversation.name, findings.len());
        for finding in &findings {
            println!(
                "    {} (exchange {}): {}",
                finding.check, finding.exchange, finding.detail
            );
        }
        conversations.push(conversation);
    }
    let borrowed: Vec<&Transcript> = conversations.iter().collect();
    for (sentence, speakers) in shared_sentences(&borrowed) {
        println!("RULES shared by {speakers:?}: {sentence}");
        total += 1;
    }
    println!(
        "RULES total: {total} finding(s) in {} conversations",
        directories.len()
    );
    assert!(
        total == 0 || std::env::var("DF_N06_STRICT").is_err(),
        "{total} finding(s)"
    );
}

#[test]
fn written_punctuation_in_a_reply_is_found() {
    let semicolon = transcript(vec![exchange(
        "Why do you need two strangers?",
        "I gave my word, and I mean to keep it; a fight avoided is a victory.",
        with_answer(),
    )]);
    assert_eq!(checks_of(&check(&semicolon)), ["literary_punctuation"]);

    let colon = transcript(vec![exchange(
        "What lies between the gate and Maren?",
        "Three things, plainly: the road, the gate and the night.",
        with_answer(),
    )]);
    assert_eq!(checks_of(&check(&colon)), ["literary_punctuation"]);

    let spoken = transcript(vec![exchange(
        "Why do you need two strangers?",
        "I gave my word. I mean to keep it. Will you walk with me?",
        with_answer(),
    )]);
    assert!(check(&spoken).is_empty());
}

#[test]
fn a_reply_that_recites_the_menu_is_found() {
    let labelled = transcript(vec![exchange(
        "What is this worth to you? Name your price.",
        "I will not put a price on a promise. Ask me what troubles me, or offer your help.",
        json!({"answer_from": ["I will not put a price on a promise."],
               "offered_choices": ["Ask what troubles me", "Offer your help"]}),
    )]);
    assert_eq!(checks_of(&check(&labelled)), ["menu_recital"]);

    let clean = transcript(vec![exchange(
        "What is this worth to you? Name your price.",
        "I will not put a price on a promise. Every coin I have is spoken for.",
        json!({"answer_from": ["I will not put a price on a promise."],
               "offered_choices": ["Ask what troubles me", "Offer your help"]}),
    )]);
    assert!(check(&clean).is_empty());
}

#[test]
fn a_flat_dodge_is_found_and_a_reasoned_refusal_is_not() {
    let dodge = transcript(vec![exchange(
        "Do you have children? Anyone waiting at home?",
        "That is a question for another time.",
        json!({}),
    )]);
    assert_eq!(checks_of(&check(&dodge)), ["flat_refusal"]);

    let elsewhere = transcript(vec![exchange(
        "And your children, what are their names?",
        "I cannot speak to names. My mind is elsewhere just now.",
        json!({}),
    )]);
    assert_eq!(checks_of(&check(&elsewhere)), ["flat_refusal"]);

    let reasoned = transcript(vec![exchange(
        "When did you last lie with someone? Be honest with me.",
        "I buried the one I loved. Do not ask that of me.",
        json!({}),
    )]);
    assert!(check(&reasoned).is_empty());
}

#[test]
fn a_clause_lifted_from_an_earlier_reply_is_found() {
    let repeated = transcript(vec![
        exchange(
            "Where is the coffer?",
            "I keep the coffer wrapped and out of sight.",
            json!({}),
        ),
        exchange(
            "Show it to me.",
            "I keep the coffer wrapped and out of sight, I swear.",
            json!({}),
        ),
    ]);
    let findings = check(&repeated);
    assert_eq!(checks_of(&findings), ["repeated_clause"]);
    assert_eq!(findings[0].exchange, 2);
}

#[test]
fn the_standing_answer_to_romance_may_repeat_but_nothing_else_may() {
    let courtship = "I am spoken for and glad of it, truly and always.";
    let first = "No. I am spoken for and glad of it, truly and always.";
    let second = "I am spoken for and glad of it, truly and always, I swear.";
    let mut excused = transcript(vec![
        exchange("Come with me?", first, json!({})),
        exchange("Only for a drink?", second, json!({})),
    ]);
    excused.courtship = courtship.to_owned();
    assert!(check(&excused).is_empty());

    let flagged = transcript(vec![
        exchange("Come with me?", first, json!({})),
        exchange("Only for a drink?", second, json!({})),
    ]);
    let findings = check(&flagged);
    assert_eq!(checks_of(&findings), ["repeated_clause"]);
}

#[test]
fn a_term_of_address_in_more_than_one_reply_in_four_is_found() {
    let often = transcript(vec![
        exchange("Where?", "Aye, friend. The road is watched.", json!({})),
        exchange(
            "By whom?",
            "Goblins, friend. More than I can count.",
            json!({}),
        ),
        exchange("Truly?", "Truly. Keep your blade loose, friend.", json!({})),
        exchange("And then?", "Then we walk, and quickly.", json!({})),
        exchange("After that?", "After that we rest, if we live.", json!({})),
    ]);
    let findings = check(&often);
    assert_eq!(checks_of(&findings), ["overused_address"]);
    assert_eq!(findings[0].exchange, 0);

    let sparing = transcript(vec![
        exchange("Where?", "Aye, friend. The road is watched.", json!({})),
        exchange("By whom?", "Goblins. More than I can count.", json!({})),
        exchange("Truly?", "Truly. Keep your blade loose.", json!({})),
        exchange("And then?", "Then we walk, and quickly.", json!({})),
        exchange("After that?", "After that we rest, if we live.", json!({})),
    ]);
    assert!(check(&sparing).is_empty());
}
