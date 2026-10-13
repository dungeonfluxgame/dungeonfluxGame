//! Rule checks on the transcript of one NPC conversation, made without any model (PLAY-N06).
//!
//! A transcript is the `conversation.json` that `npc_talk_live_tests` writes for each scripted
//! conversation: the hero's words, the reply, and the exact facts the reply was written from. The
//! checks read only that record, so they run on a recorded transcript, on a live one as it is
//! played, and on the output of any future build. They support a person's reading of the
//! transcripts and never replace it.

use serde_json::Value;

use super::speech_text::{
    REPEAT_MIN_WORDS, REPEATED_CLAUSE_WORDS, content_sequence, content_words, coverage, is_denial,
    is_flat_dodge, normalised, sentence_key, sentences, words,
};

/// The most sentences a spoken reply may have.
pub(super) const MAXIMUM_SENTENCES: usize = 3;
/// The longest reply, in characters, that fits a speech bubble and a few seconds of speech.
pub(super) const MAXIMUM_REPLY_CHARACTERS: usize = 300;

/// What one rule found wrong with one exchange (or, for a whole conversation, `exchange` 0).
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Finding {
    pub(super) check: &'static str,
    pub(super) exchange: usize,
    pub(super) detail: String,
}

/// One exchange of a recorded conversation.
#[derive(Clone, Debug)]
pub(super) struct Exchange {
    pub(super) hero_words: String,
    pub(super) reply: String,
    pub(super) facts: Value,
    pub(super) from_model: bool,
}

/// A recorded conversation with what the checks need to know about its speaker and listener.
#[derive(Clone, Debug)]
pub(super) struct Transcript {
    pub(super) name: String,
    /// The adventure the conversation took place in, and who spoke.
    pub(super) seed: String,
    pub(super) speaker: String,
    pub(super) holds_private_clue: bool,
    pub(super) clue_in_play: bool,
    /// The speaker is expected to decline in character, so a refusal is not a denial.
    pub(super) deflects: bool,
    pub(super) clue_only_terms: Vec<String>,
    pub(super) never_terms: Vec<String>,
    /// The speaker's tic, opener and closer, which may shape the voice but are never said.
    pub(super) style_phrases: Vec<String>,
    /// The speaker's local lore, sayings and rumours.
    pub(super) colour_lines: Vec<String>,
    /// Every line the dossier lets the speaker say freely about themselves and the errand.
    pub(super) own_lines: Vec<String>,
    /// The standing answer to romance, repeated word for word every time by design: the
    /// clause-reuse check never flags it. Older records predate it.
    pub(super) courtship: String,
    pub(super) exchanges: Vec<Exchange>,
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

impl Transcript {
    /// Reads a `conversation.json`; `None` when it is not one.
    pub(super) fn from_json(record: &Value) -> Option<Self> {
        let speaker = record.get("speaker")?;
        let exchanges = record
            .get("exchanges")?
            .as_array()?
            .iter()
            .map(|exchange| {
                Some(Exchange {
                    hero_words: exchange.get("hero_words")?.as_str()?.to_owned(),
                    reply: exchange.get("reply")?.as_str()?.to_owned(),
                    facts: exchange.get("facts")?.clone(),
                    from_model: exchange.get("from_model")?.as_bool()?,
                })
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Self {
            name: record.get("conversation")?.as_str()?.to_owned(),
            seed: format!(
                "{}/{}",
                record.get("seed")?.as_str()?,
                record.get("rating")?.as_str()?
            ),
            speaker: speaker.get("name")?.as_str()?.to_owned(),
            holds_private_clue: record.get("holds_private_clue")?.as_bool()?,
            clue_in_play: record.get("clue_in_play")?.as_bool()?,
            deflects: record
                .get("deflects")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            clue_only_terms: strings(record.get("clue_only_terms")?),
            never_terms: strings(record.get("never_terms")?),
            style_phrases: strings(speaker.get("style_phrases")?),
            colour_lines: strings(speaker.get("colour_lines")?),
            own_lines: strings(speaker.get("own_lines")?),
            courtship: speaker
                .get("courtship")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            exchanges,
        })
    }
}

/// Whether the facts of an exchange offered the speaker something that answers the hero.
fn answer_offered(facts: &Value) -> bool {
    [
        "answer_from",
        "new_material",
        "you_told_this_hero_in_private",
    ]
    .iter()
    .any(|key| match facts.get(key) {
        Some(Value::Array(items)) => !items.is_empty(),
        Some(Value::String(text)) => !text.is_empty(),
        Some(Value::Null) | None => false,
        Some(_) => true,
    })
}

/// Whether the hero asked about the speaker themselves.
pub(super) fn asks_about_speaker(words_said: &str) -> bool {
    let spoken = format!(" {} ", normalised(words_said));
    [
        " who are you",
        " your ",
        " yourself",
        " you do ",
        " about you",
        " why do you",
        " do you ",
        " are you ",
        " what do you want",
        " did you ",
        " have you ",
    ]
    .iter()
    .any(|phrase| spoken.contains(phrase))
}

/// Whether the hero asked about the place, the road, the neighbourhood or the danger, which
/// local lore and rumour answer.
fn asks_about_place_or_danger(words_said: &str) -> bool {
    let spoken = normalised(words_said);
    [
        "trouble",
        "danger",
        "watch for",
        "look out",
        "stand in our way",
        "stop us",
        "place",
        "town",
        "village",
        "around here",
        "the road",
        "this road",
        "area",
        "local",
    ]
    .iter()
    .any(|phrase| spoken.contains(phrase))
}

fn contains_term(text: &str, term: &str) -> bool {
    let text = normalised(text);
    let term = normalised(term);
    let term = term.trim();
    !term.is_empty()
        && text.match_indices(term).any(|(start, matched)| {
            let before = text[..start].chars().next_back();
            let after = text[start + matched.len()..].chars().next();
            before.is_none_or(|character| !character.is_alphanumeric())
                && after.is_none_or(|character| !character.is_alphanumeric())
        })
}

fn finding(check: &'static str, exchange: usize, detail: impl Into<String>) -> Finding {
    Finding {
        check,
        exchange,
        detail: detail.into(),
    }
}

/// Phrases by which a reply reads the phone's menu aloud instead of answering in character.
const MENU_PATTERNS: [&str; 5] = [
    "ask me what troubles me",
    "ask what troubles me",
    "offer your help",
    "ask me about this place",
    "what to click",
];

/// What the exchange's menu offered, as the recorded facts kept it.
fn menu_labels(facts: &Value) -> Vec<String> {
    facts
        .get("offered_choices")
        .map(strings)
        .unwrap_or_default()
        .into_iter()
        .filter(|label| words(label).len() >= 2)
        .collect()
}

/// The menu wording the reply recites, if any: a recorded choice label or a stock menu
/// phrase. A person talking names no menus.
fn menu_recital(reply: &str, facts: &Value) -> Option<String> {
    for label in menu_labels(facts) {
        if contains_term(reply, &label) {
            return Some(label);
        }
    }
    MENU_PATTERNS
        .iter()
        .find(|pattern| contains_term(reply, pattern))
        .map(|pattern| (*pattern).to_owned())
}

/// Terms of address that wear out: at most about one reply in four may use one.
const TERMS_OF_ADDRESS: [&str; 4] = ["friend", "chief", "mate", "stranger"];
/// The content-word clauses of `text`, minus the standing courtship answer, which is
/// repeated word for word every time by design.
fn fresh_clauses(text: &str, courted: &std::collections::BTreeSet<String>) -> Vec<String> {
    let mut clauses = Vec::new();
    for part in sentences(text) {
        let sequence = content_sequence(&part);
        if sequence.len() < REPEATED_CLAUSE_WORDS {
            continue;
        }
        for window in sequence.windows(REPEATED_CLAUSE_WORDS) {
            let clause = window.join(" ");
            if !courted.contains(&clause) {
                clauses.push(clause);
            }
        }
    }
    clauses
}

/// Runs every check on one conversation.
pub(super) fn check(transcript: &Transcript) -> Vec<Finding> {
    let mut found = Vec::new();
    let mut seen: Vec<(usize, String)> = Vec::new();
    let mut admissions = 0;
    let mut heard: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let courted: std::collections::BTreeSet<String> = content_sequence(&transcript.courtship)
        .windows(REPEATED_CLAUSE_WORDS)
        .map(|window| window.join(" "))
        .collect();
    let mut addressed: [usize; TERMS_OF_ADDRESS.len()] = [0; TERMS_OF_ADDRESS.len()];
    for (index, exchange) in transcript.exchanges.iter().enumerate() {
        let number = index + 1;
        let reply = &exchange.reply;
        let parts = sentences(reply);
        if exchange.reply.trim().is_empty() {
            found.push(finding("no_reply", number, "the speaker said nothing"));
            continue;
        }
        if parts.len() > MAXIMUM_SENTENCES {
            found.push(finding(
                "too_many_sentences",
                number,
                format!("{} sentences", parts.len()),
            ));
        }
        if reply.chars().count() > MAXIMUM_REPLY_CHARACTERS {
            found.push(finding(
                "too_long",
                number,
                format!("{} characters", reply.chars().count()),
            ));
        }
        if reply.contains(';') {
            found.push(finding(
                "literary_punctuation",
                number,
                "a semicolon is writing, not talking",
            ));
        }
        if reply.contains(':') {
            found.push(finding(
                "literary_punctuation",
                number,
                "a colon-led list is writing, not talking",
            ));
        }
        if let Some(menu) = menu_recital(reply, &exchange.facts) {
            found.push(finding("menu_recital", number, menu));
        }
        if let Some(first) = parts.first() {
            if is_denial(first) && answer_offered(&exchange.facts) && !transcript.deflects {
                found.push(finding(
                    "denial_with_an_answer_offered",
                    number,
                    first.clone(),
                ));
            }
            // A restatement echoes the question's words and adds almost nothing of its own.
            let asked = content_words(&exchange.hero_words);
            let added = content_words(first).difference(&asked).count();
            if asked.len() >= 4 && coverage(&exchange.hero_words, first) >= 0.75 && added <= 1 {
                found.push(finding("restates_the_question", number, first.clone()));
            }
        }
        admissions += parts.iter().filter(|part| is_denial(part)).count().min(1);
        // A flat dodge refuses without saying what is known, guessing, or naming who would
        // know: in character, specific and once per topic is owed instead.
        for part in &parts {
            if is_flat_dodge(part) {
                found.push(finding("flat_refusal", number, part.clone()));
            }
        }
        // No clause from the speaker's own earlier replies, courtship aside.
        let clauses = fresh_clauses(reply, &courted);
        if let Some(clause) = clauses.iter().find(|clause| heard.contains(*clause)) {
            found.push(finding("repeated_clause", number, clause.clone()));
        }
        for clause in clauses {
            heard.insert(clause);
        }
        for (index, term) in TERMS_OF_ADDRESS.iter().enumerate() {
            if contains_term(reply, term) {
                addressed[index] += 1;
            }
        }
        for part in &parts {
            if words(part).len() >= REPEAT_MIN_WORDS {
                let key = sentence_key(part);
                if let Some((first_time, _)) = seen.iter().find(|(_, earlier)| *earlier == key) {
                    found.push(finding(
                        "repeated_sentence",
                        number,
                        format!("{part} (first said in exchange {first_time})"),
                    ));
                } else {
                    seen.push((number, key));
                }
            }
        }
        for phrase in &transcript.style_phrases {
            let quoted = phrase
                .split('\'')
                .nth(1)
                .filter(|inner| !inner.trim().is_empty())
                .unwrap_or(phrase);
            if words(quoted).len() >= 2 && contains_term(reply, quoted.trim_end_matches('.')) {
                found.push(finding(
                    "says_a_speech_style_phrase",
                    number,
                    quoted.to_owned(),
                ));
            }
        }
        for line in &transcript.colour_lines {
            if !asks_about_place_or_danger(&exchange.hero_words) && coverage(line, reply) >= 0.6 {
                found.push(finding("local_lore_or_saying", number, line.clone()));
            }
        }
        if !asks_about_speaker(&exchange.hero_words) {
            for line in &transcript.own_lines {
                let is_errand = transcript.colour_lines.contains(line);
                if !is_errand
                    && words(line).len() >= 8
                    && coverage(line, reply) >= 0.7
                    && is_autobiography(line)
                {
                    found.push(finding("autobiography", number, line.clone()));
                }
            }
        }
        for term in &transcript.never_terms {
            if contains_term(reply, term) {
                found.push(finding("said_a_never_term", number, term.clone()));
            }
        }
        if transcript.clue_in_play && !transcript.holds_private_clue {
            for term in &transcript.clue_only_terms {
                if contains_term(reply, term) {
                    found.push(finding(
                        "leaked_the_private_clue",
                        number,
                        format!("said \"{term}\" to a hero who was not told"),
                    ));
                }
            }
        }
    }
    if admissions > 1 {
        found.push(finding(
            "says_i_dont_know_more_than_once",
            0,
            format!("{admissions} replies"),
        ));
    }
    // A term of address wears out: at most about one reply in four may use one.
    let replies = transcript.exchanges.len();
    let allowed = (replies + 3) / 4;
    for (index, term) in TERMS_OF_ADDRESS.iter().enumerate() {
        if addressed[index] > allowed {
            found.push(finding(
                "overused_address",
                0,
                format!("{term} in {} of {replies} replies", addressed[index]),
            ));
        }
    }
    if transcript.holds_private_clue
        && !transcript.exchanges.iter().any(|exchange| {
            transcript
                .clue_only_terms
                .iter()
                .any(|term| contains_term(&exchange.reply, term))
        })
    {
        found.push(finding(
            "never_spoke_of_the_private_clue",
            0,
            "the hero who was told asked about it and never heard it discussed",
        ));
    }
    found
}

/// Lines about the speaker's own past and character, as opposed to the errand: the dossier's
/// origin, trade, event, ideal, bond and want lines read in the first person about "I was" or
/// "I want".
fn is_autobiography(line: &str) -> bool {
    let lowered = normalised(line);
    [
        "i was ",
        "i walked",
        "i carried",
        "i try",
        "i do the",
        "i want",
        "i have a ",
        "i fear",
        "i sang",
        "i mucked",
        "i found",
        "i promised",
        "i owe",
        "everything i know",
    ]
    .iter()
    .any(|start| lowered.starts_with(start))
}

/// The sentences that two different speakers of one adventure both say, as `(sentence, speakers)`.
/// A line two speakers share makes them sound interchangeable; one speaker repeating a line in
/// another conversation of the same adventure is not counted here.
pub(super) fn shared_sentences(transcripts: &[&Transcript]) -> Vec<(String, Vec<String>)> {
    let mut by_sentence: Vec<(String, String, Vec<String>)> = Vec::new();
    for transcript in transcripts {
        for exchange in &transcript.exchanges {
            for part in sentences(&exchange.reply) {
                if words(&part).len() < REPEAT_MIN_WORDS {
                    continue;
                }
                let key = sentence_key(&part);
                match by_sentence
                    .iter_mut()
                    .find(|(known, seed, _)| *known == key && *seed == transcript.seed)
                {
                    Some((_, _, speakers)) => {
                        if !speakers.contains(&transcript.speaker) {
                            speakers.push(transcript.speaker.clone());
                        }
                    }
                    None => by_sentence.push((
                        key,
                        transcript.seed.clone(),
                        vec![transcript.speaker.clone()],
                    )),
                }
            }
        }
    }
    by_sentence
        .into_iter()
        .filter(|(_, _, speakers)| speakers.len() > 1)
        .map(|(sentence, _, speakers)| (sentence, speakers))
        .collect()
}

#[cfg(test)]
#[path = "npc_talk_rules_tests.rs"]
mod tests;
