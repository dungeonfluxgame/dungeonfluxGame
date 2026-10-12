//! The transcript rules, shown on made-up replies and on recorded live conversations.

use std::path::PathBuf;

use serde_json::{Value, json};

use super::*;

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
        exchanges,
    }
}

fn checks_of(findings: &[Finding]) -> Vec<&'static str> {
    findings.iter().map(|finding| finding.check).collect()
}

fn with_answer() -> Value {
    json!({"answer_from": ["The iron coffer goes to Tomas."]})
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
fn admitting_ignorance_in_more_than_one_reply_is_found() {
    let nothing = json!({"answer_from": []});
    let twice = transcript(vec![
        exchange("Who is she?", "I don't know who she is.", nothing.clone()),
        exchange("And him?", "I have no idea about him either.", nothing),
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
    assert_eq!(checks_of(&findings), ["repeated_sentence"]);
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
        "denial_with_an_answer_offered",
        "says_a_speech_style_phrase",
        "local_lore_or_saying",
        "autobiography",
        "repeated_sentence",
    ] {
        assert!(found.contains(&check), "{check} was not found: {found:?}");
    }
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
    for (name, conversation) in &after {
        for finding in check(conversation) {
            if matches!(finding.check, "autobiography" | "local_lore_or_saying") {
                soft += 1;
            } else {
                panic!("{name}: {finding:?}");
            }
        }
        fallbacks += conversation
            .exchanges
            .iter()
            .filter(|exchange| !exchange.from_model)
            .count();
    }
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
